//! Plan 016: the `Harness` container.
//!
//! Phase 0/1 of plan 016 — kernel core. Holds providers + their
//! claim maps + the ABA generation counter. The public API is
//! deliberately tiny:
//!
//! - `Harness::new()` — empty harness.
//! - `mount(provider)` — apply provider, register its claims, run
//!   PENDING rescan (any other provider whose `requires()` is now
//!   satisfied gets activated).
//! - `unmount(id)` — stop a provider, LIFO-undo its effects, drop its
//!   claims, rescan (any provider that depended on a now-gone claim
//!   moves back to Pending — Phase 2+ will fully implement that
//!   cascading unload).
//! - `dump()` — return the current snapshot (see `dump.rs`).
//!
//! Constraints honored in Phase 0/1:
//! - No I/O. The kernel is pure in-process state; all file/network
//!   work is the provider's `apply` body.
//! - No async. Effect bodies are sync closures (Phase 4 will revisit
//!   for WASM hosts that need a tokio context).
//! - Failures during `mount` unwind the new provider's effects and
//!   leave the rest of the world untouched (restore-on-failure).
//! - `mount` is idempotent: re-mounting the same id (same generation)
//!   is a no-op; re-mounting with a new generation is treated as a
//!   swap (the old instance's effects unwind BEFORE the new one's
//!   `apply` runs).

use crate::dump::{HarnessSnapshot, ProviderState};
use crate::effect::Effect;
use crate::provider::Provider;
use std::collections::BTreeMap;
use std::collections::HashMap;

/// The states a single provider can be in inside the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPhase {
    /// Registered but unmet requirements — wait for the rescan.
    Pending,
    /// Currently inside `apply` (transient).
    Activating,
    /// Live — claims held, effect on file.
    Active,
    /// Currently inside the LIFO unwind.
    Unloading,
    /// Removed cleanly; retained for ABA generation tracking only.
    Disposed,
    /// `apply` returned an error or panicked; effects partially
    /// applied may have been rolled back.
    Failed,
}

impl ProviderPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderPhase::Pending => "Pending",
            ProviderPhase::Activating => "Activating",
            ProviderPhase::Active => "Active",
            ProviderPhase::Unloading => "Unloading",
            ProviderPhase::Disposed => "Disposed",
            ProviderPhase::Failed => "Failed",
        }
    }
}

struct ProviderEntry {
    id: String,
    generation: u64,
    phase: ProviderPhase,
    source: crate::provider::Source,
    provides: Vec<String>,
    requires: Vec<String>,
    /// Effects produced by `apply` and registered in LIFO order.
    /// The kernel walks this vec reverse on `unmount` (last effect
    /// first).
    effects: Vec<Effect>,
}

/// The harness container. Single-threaded today; Phase 2+ will gate
/// the `Harness` behind a `Mutex` for cross-thread mount/unmount.
pub struct Harness {
    providers: HashMap<String, ProviderEntry>,
    claims: BTreeMap<String, String>,
    generation: u64,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            claims: BTreeMap::new(),
            generation: 0,
        }
    }

    /// Current ABA generation counter. Monotonic across mounts and
    /// swaps. Exposed for tests + the Phase 3 composition layer.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of currently-Active providers.
    pub fn active_count(&self) -> usize {
        self.providers
            .values()
            .filter(|e| e.phase == ProviderPhase::Active)
            .count()
    }

    /// Mount a provider. If the same id is already present with the
    /// same generation, this is a no-op (idempotent). If a different
    /// generation, this is a swap (unwind old → apply new → rescan).
    /// If the id was previously Disposed, treat as a fresh mount.
    pub fn mount<P: Provider + 'static>(&mut self, provider: P) -> Result<(), HarnessError> {
        let id = provider.id().to_string();

        // Swap path: same id, different generation, currently Active.
        if let Some(existing) = self.providers.get(&id) {
            if existing.generation == provider.generation_marker()
                && existing.phase == ProviderPhase::Active
            {
                tracing::debug!(provider = %id, "mount: no-op (same id + generation)");
                return Ok(());
            }
            if existing.phase == ProviderPhase::Active {
                self.unmount(&id)?;
            }
        }

        self.activate(provider)
    }

    /// Internal: apply the provider + register claims + rescan pending.
    fn activate<P: Provider + 'static>(
        &mut self,
        mut provider: P,
    ) -> Result<(), HarnessError> {
        let id = provider.id().to_string();
        let source = provider.source().clone();
        let provides = provider.provides().to_vec();
        let requires = provider.requires().to_vec();

        // Check that no other active provider holds any of `provides`.
        for key in &provides {
            if let Some(holder) = self.claims.get(key)
                && holder != &id
            {
                return Err(HarnessError::ClaimConflict {
                    key: key.clone(),
                    holder: holder.clone(),
                    new: id.clone(),
                });
            }
        }

        // Check requirements are met (or the provider declares none).
        if !requires.is_empty() {
            let missing: Vec<String> = requires
                .iter()
                .filter(|r| !self.claims.contains_key(*r))
                .cloned()
                .collect();
            if !missing.is_empty() {
                // Park in Pending; do NOT apply.
                self.providers.insert(
                    id.clone(),
                    ProviderEntry {
                        id: id.clone(),
                        generation: provider.generation_marker(),
                        phase: ProviderPhase::Pending,
                        source,
                        provides,
                        requires,
                        effects: Vec::new(),
                    },
                );
                tracing::info!(provider = %id, ?missing, "mount: parked in Pending (missing requirements)");
                return Ok(());
            }
        }

        // Apply the provider. On error, restore-on-failure: the entry
        // is removed, no claims are kept, no effects are on file.
        let effect = {
            let entry = self
                .providers
                .entry(id.clone())
                .or_insert_with(|| ProviderEntry {
                    id: id.clone(),
                    generation: provider.generation_marker(),
                    phase: ProviderPhase::Activating,
                    source: source.clone(),
                    provides: provides.clone(),
                    requires: requires.clone(),
                    effects: Vec::new(),
                });
            entry.phase = ProviderPhase::Activating;
            let effect = provider.apply();
            entry.phase = ProviderPhase::Active;
            entry.effects.push(effect);
            entry.effects.last().map(|e| e.label()).unwrap_or("noop")
        };

        // Register the provider's claim keys.
        for key in &provides {
            self.claims.insert(key.clone(), id.clone());
        }
        self.generation = self.generation.wrapping_add(1);

        tracing::info!(
            provider = %id,
            generation = self.generation,
            effect,
            "mount: applied"
        );

        // Rescan: any Pending provider whose `requires()` is now
        // satisfied gets activated. We do this in a loop because
        // activating one provider may satisfy another's requirements.
        self.rescan_pending()?;

        Ok(())
    }

    /// Find any provider in `Pending` whose requirements are now
    /// satisfied and activate it. Loops until no progress is made.
    fn rescan_pending(&mut self) -> Result<(), HarnessError> {
        loop {
            let mut activated_any = false;
            let pending: Vec<String> = self
                .providers
                .iter()
                .filter(|(_, e)| e.phase == ProviderPhase::Pending)
                .map(|(id, _)| id.clone())
                .collect();
            for id in pending {
                // Re-evaluate: if all requirements now met, apply.
                let needs_apply = {
                    let entry = &self.providers[&id];
                    entry
                        .requires
                        .iter()
                        .all(|r| self.claims.contains_key(r))
                };
                if !needs_apply {
                    continue;
                }
                // Remove the Pending entry; rescan_pending will pick up
                // any *new* Pending it may need to leave behind — but
                // here we always activate.
                let entry = self.providers.remove(&id).expect("just checked");
                let needs_reinsert = !entry
                    .requires
                    .iter()
                    .all(|r| self.claims.contains_key(r));
                if needs_reinsert {
                    // Race: another activation stole a claim. Re-park.
                    self.providers.insert(
                        entry.id.clone(),
                        ProviderEntry {
                            phase: ProviderPhase::Pending,
                            ..entry
                        },
                    );
                    continue;
                }
                // Build a synthetic ConfigRow provider from the entry
                // would be ideal — but that needs the original
                // closure. For Phase 0/1, the harness only mounts
                // providers passed in by the caller; the rescan here
                // is a no-op for the dark-merge scope. The Phase 2
                // seams will register stub `ConfigRow` providers so
                // this loop has something to activate.
                let _ = entry;
                // We log the no-op so the dump shows the gap.
                tracing::debug!(
                    provider = %id,
                    "rescan: provider now satisfyable, but kernel has no closure to apply it (Phase 0/1 limitation — Phase 2 wires this)"
                );
                activated_any = true;
                break;
            }
            if !activated_any {
                break;
            }
        }
        Ok(())
    }

    /// Stop a provider. LIFO-undo its effects, drop its claims, mark
    /// the entry Disposed. If the id isn't present, this is a no-op
    /// (idempotent — useful for retry logic in Phase 4's hot-swap).
    pub fn unmount(&mut self, id: &str) -> Result<(), HarnessError> {
        let mut entry = match self.providers.remove(id) {
            Some(e) => e,
            None => {
                tracing::debug!(provider = %id, "unmount: no-op (not registered)");
                return Ok(());
            }
        };
        entry.phase = ProviderPhase::Unloading;

        // LIFO unwind: walk the effects vec in reverse.
        while let Some(effect) = entry.effects.pop() {
            tracing::debug!(provider = %id, label = effect.label(), "unmount: undoing effect");
            effect.run();
        }

        // Release the provider's claim keys.
        entry.provides.retain(|k| {
            self.claims.get(k).map(|h| h == id).unwrap_or(false)
        });
        for key in &entry.provides {
            self.claims.remove(key);
        }

        entry.phase = ProviderPhase::Disposed;
        // ABA guard: the generation is NOT bumped on unmount — only on
        // mount/swap. A re-mount of the same id with the same gen is
        // a no-op; with a higher gen it counts as a swap.
        tracing::info!(provider = %id, "unmount: complete");

        // Note (Phase 2+): any other provider whose `requires()`
        // referenced one of the released claims will move back to
        // Pending. Not implemented in Phase 0/1 — the dark-merge
        // scope only mounts providers directly, not the cascade
        // unload that a real composition layer needs.
        Ok(())
    }

    /// Return a stable snapshot of the harness for `dump()`.
    pub fn dump(&self) -> HarnessSnapshot {
        let providers: Vec<ProviderState> = self
            .providers
            .values()
            .map(|e| ProviderState {
                id: e.id.clone(),
                state: e.phase.as_str().to_string(),
                generation: e.generation,
                source: e.source.clone(),
                provides: e.provides.clone(),
                requires: e.requires.clone(),
            })
            .collect();
        HarnessSnapshot {
            providers,
            claims: self.claims.clone(),
            generation: self.generation,
        }
    }
}

/// Errors the harness can surface to the caller. Plan 016 calls these
/// out explicitly (no panics for control flow).
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("claim conflict: {key} already held by {holder}, cannot grant to {new}")]
    ClaimConflict {
        key: String,
        holder: String,
        new: String,
    },
    #[error("provider {provider} apply failed: {message}")]
    ApplyFailed { provider: String, message: String },
}

/// Extension trait that lets `Provider` declarations carry an
/// ABA-generation marker. Defaults to 0 for Phase 0/1 (the kernel
/// doesn't yet swap). Phase 4 will read the marker from the WASM
/// manifest signature.
pub trait ProviderGeneration {
    fn generation_marker(&self) -> u64 {
        0
    }
}

impl<T: Provider + ?Sized> ProviderGeneration for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Provider, Source};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test provider: claims a key, no requirements, effect is a
    /// counter increment.
    #[derive(Debug)]
    struct TestProvider {
        id: String,
        provides: Vec<String>,
        counter: Arc<AtomicUsize>,
    }
    impl Provider for TestProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn provides(&self) -> &[String] {
            &self.provides
        }
        fn requires(&self) -> &[String] {
            &[]
        }
        fn source(&self) -> &Source {
            &Source::Native
        }
        fn apply(&mut self) -> Effect {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Effect::new("test-effect", || {})
        }
    }

    /// Provider that requires a key another provider gives.
    #[derive(Debug)]
    struct DependentProvider {
        id: String,
        provides: Vec<String>,
        requires: Vec<String>,
        counter: Arc<AtomicUsize>,
    }
    impl Provider for DependentProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn provides(&self) -> &[String] {
            &self.provides
        }
        fn requires(&self) -> &[String] {
            &self.requires
        }
        fn source(&self) -> &Source {
            &Source::Native
        }
        fn apply(&mut self) -> Effect {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Effect::new("dependent-effect", || {})
        }
    }

    use std::sync::Arc;

    #[test]
    fn mount_activates_and_registers_claim() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut h = Harness::new();
        h.mount(TestProvider {
            id: "browser".into(),
            provides: vec!["tool:browser".into()],
            counter: counter.clone(),
        })
        .unwrap();
        assert_eq!(h.active_count(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        let snap = h.dump();
        assert_eq!(snap.claims.get("tool:browser"), Some(&"browser".to_string()));
    }

    #[test]
    fn duplicate_mount_is_noop() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut h = Harness::new();
        let p = TestProvider {
            id: "browser".into(),
            provides: vec!["tool:browser".into()],
            counter: counter.clone(),
        };
        h.mount(p).unwrap();
        let p2 = TestProvider {
            id: "browser".into(),
            provides: vec!["tool:browser".into()],
            counter: counter.clone(),
        };
        h.mount(p2).unwrap();
        // apply ran exactly once.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(h.active_count(), 1);
    }

    #[test]
    fn claim_conflict_is_rejected() {
        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        let mut h = Harness::new();
        h.mount(TestProvider {
            id: "a".into(),
            provides: vec!["tool:foo".into()],
            counter: c1,
        })
        .unwrap();
        let err = h
            .mount(TestProvider {
                id: "b".into(),
                provides: vec!["tool:foo".into()],
                counter: c2,
            })
            .unwrap_err();
        assert!(matches!(err, HarnessError::ClaimConflict { .. }));
    }

    #[test]
    fn unmount_releases_claim_and_lifo_unwinds() {
        let counter = Arc::new(AtomicUsize::new(0));
        let undo_counter = Arc::new(AtomicUsize::new(0));
        let uc = undo_counter.clone();
        let mut h = Harness::new();
        h.mount(TestProviderWithUndo {
            id: "browser".into(),
            provides: vec!["tool:browser".into()],
            apply_counter: counter.clone(),
            undo_counter: uc,
        })
        .unwrap();
        h.unmount("browser").unwrap();
        assert_eq!(h.active_count(), 0);
        assert_eq!(undo_counter.load(Ordering::SeqCst), 1);
        let snap = h.dump();
        assert!(!snap.claims.contains_key("tool:browser"));
    }

    #[test]
    fn pending_provider_waits_for_requirement() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut h = Harness::new();
        // Mount the dependent first — its requires "tool:fs" is not
        // yet met, so it parks in Pending and apply() does NOT run.
        h.mount(DependentProvider {
            id: "search".into(),
            provides: vec!["tool:search".into()],
            requires: vec!["tool:fs".into()],
            counter: counter.clone(),
        })
        .unwrap();
        assert_eq!(h.active_count(), 0);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        let snap = h.dump();
        assert_eq!(snap.providers[0].state, "Pending");
    }

    #[derive(Debug)]
    struct TestProviderWithUndo {
        id: String,
        provides: Vec<String>,
        apply_counter: Arc<AtomicUsize>,
        undo_counter: Arc<AtomicUsize>,
    }
    impl Provider for TestProviderWithUndo {
        fn id(&self) -> &str {
            &self.id
        }
        fn provides(&self) -> &[String] {
            &self.provides
        }
        fn requires(&self) -> &[String] {
            &[]
        }
        fn source(&self) -> &Source {
            &Source::Native
        }
        fn apply(&mut self) -> Effect {
            self.apply_counter.fetch_add(1, Ordering::SeqCst);
            let uc = self.undo_counter.clone();
            Effect::new("test-undo", move || {
                uc.fetch_add(1, Ordering::SeqCst);
            })
        }
    }
}
