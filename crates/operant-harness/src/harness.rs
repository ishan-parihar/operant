//! The registry — lifecycle, late binding, cascades, transactional replace.
//!
//! Concurrency model: lifecycle operations (`mount`, `unmount`, `replace`)
//! serialize on one write lock and are expected to be rare (boot + explicit
//! agent-driven mutations). Runtime traffic reads claims/state through the
//! read path (`dump`, `claim_owner`, `state_of`). Seam `install` calls run
//! while the write lock is held — implementations MUST NOT call back into
//! [`Harness`] lifecycle methods (documented on [`Seam`]).
//!
//! State note: `Activating`/`Unloading` are transient under the write lock and
//! therefore never observable externally in v1; they exist in the state enum
//! for host-side mirroring and future fine-grained observation.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use serde_json::Value;

use crate::claim::Claim;
use crate::effect::{Effect, unwind_lifo};
use crate::error::HarnessError;
use crate::provider::{ActivateCx, Provider, ProviderState, Seam};
use crate::report::{ClaimInfo, DumpTree, MountReport, ProviderEntryInfo};

/// Kernel-level knobs (host config maps onto this in Phase 2+).
#[derive(Debug, Clone, Copy)]
pub struct KernelOptions {
    /// Emit an info line per lifecycle transition.
    pub audit: bool,
}

impl Default for KernelOptions {
    fn default() -> Self {
        Self { audit: true }
    }
}

struct Entry {
    provider: Arc<dyn Provider>,
    state: ProviderState,
    generation: u64,
    /// Monotonic insertion counter — deterministic ordering everywhere.
    seq: u64,
    config: Value,
    /// Live undo handles; empty unless state == Active.
    effects: Vec<Effect>,
}

#[derive(Default)]
struct Inner {
    providers: HashMap<String, Entry>,
    claims: HashMap<Claim, String>,
    /// Fairness for late-binding rescans (insertion order).
    pending_order: Vec<String>,
    next_seq: u64,
}

impl Inner {
    fn alloc_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    /// Requirements currently unsatisfied by active claims.
    fn missing_requirements(&self, requires: &[Claim]) -> Vec<Claim> {
        requires
            .iter()
            .filter(|r| !self.claims.contains_key(r))
            .cloned()
            .collect()
    }

    fn first_claim_conflict(
        &self,
        provides: &[Claim],
        exempt_owner: Option<&str>,
    ) -> Option<HarnessError> {
        for claim in provides {
            if let Some(owner) = self.claims.get(claim)
                && exempt_owner != Some(owner.as_str())
            {
                return Some(HarnessError::ClaimConflict {
                    claim: claim.clone(),
                    owner: owner.clone(),
                });
            }
        }
        None
    }

    fn make_entry(
        provider: Arc<dyn Provider>,
        state: ProviderState,
        generation: u64,
        seq: u64,
        config: Value,
        effects: Vec<Effect>,
    ) -> Entry {
        Entry {
            provider,
            state,
            generation,
            seq,
            config,
            effects,
        }
    }
}

/// The harness kernel.
pub struct Harness {
    seams: HashMap<String, Arc<dyn Seam>>,
    inner: RwLock<Inner>,
    options: KernelOptions,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new(KernelOptions::default())
    }
}

impl Harness {
    pub fn new(options: KernelOptions) -> Self {
        Self {
            seams: HashMap::new(),
            inner: RwLock::new(Inner::default()),
            options,
        }
    }

    /// Register a seam sink. Must happen before providers that route installs
    /// through it are mounted.
    pub fn add_seam(&mut self, seam: Arc<dyn Seam>) {
        self.seams.insert(seam.name().to_string(), seam);
    }

    pub fn seam_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.seams.keys().cloned().collect();
        names.sort();
        names
    }

    // ── mount ──────────────────────────────────────────────────────────

    /// Mount a provider with null config.
    pub async fn mount(&self, provider: Arc<dyn Provider>) -> Result<MountReport, HarnessError> {
        self.mount_with_config(provider, Value::Null).await
    }

    /// Mount a provider. Requirements unmet ⇒ PENDING (late binding); claim
    /// conflicts rejected BEFORE any activation side effect; activation
    /// failure unwinds partial effects and records the entry Failed.
    ///
    /// Returns every id activated during the call — including Pending entries
    /// rescued by late binding once this mount satisfied their requirements.
    pub async fn mount_with_config(
        &self,
        provider: Arc<dyn Provider>,
        config: Value,
    ) -> Result<MountReport, HarnessError> {
        let mut inner = self.inner.write().await;
        let report = self.mount_locked(&mut inner, provider, config).await?;
        let rescued = self.rescan_pending_locked(&mut inner).await;
        Ok(match report {
            MountReport::Mounted { mut activated } => {
                activated.extend(rescued);
                MountReport::Mounted { activated }
            }
            pending @ MountReport::Pending { .. } => pending,
        })
    }

    async fn mount_locked(
        &self,
        inner: &mut Inner,
        provider: Arc<dyn Provider>,
        config: Value,
    ) -> Result<MountReport, HarnessError> {
        let id = provider.spec().id().to_string();
        if inner.providers.contains_key(&id) {
            return Err(HarnessError::AlreadyMounted(id));
        }
        let requires = provider.spec().requires().to_vec();
        let missing = inner.missing_requirements(&requires);

        if !missing.is_empty() {
            let seq = inner.alloc_seq();
            inner.pending_order.push(id.clone());
            inner.providers.insert(
                id.clone(),
                Inner::make_entry(provider, ProviderState::Pending, 0, seq, config, Vec::new()),
            );
            self.audit(&id, "pending");
            return Ok(MountReport::Pending { missing });
        }

        if let Some(conflict) = inner.first_claim_conflict(provider.spec().provides(), None) {
            return Err(conflict);
        }

        self.activate_locked(inner, provider, config, 1).await?;
        self.audit(&id, "active");
        Ok(MountReport::Mounted {
            activated: vec![id],
        })
    }

    /// Run activation; store claims + effects on success, record Failed
    /// (after containing partial effects) on error.
    async fn activate_locked(
        &self,
        inner: &mut Inner,
        provider: Arc<dyn Provider>,
        config: Value,
        generation: u64,
    ) -> Result<(), HarnessError> {
        let id = provider.spec().id().to_string();
        let seq = inner.alloc_seq();

        let mut effects = Vec::new();
        {
            let mut cx = ActivateCx::new(&id, &config, &self.seams, &mut effects);
            if let Err(err) = provider.activate(&mut cx).await {
                // Contain partial effects; leave a Failed marker for dump().
                unwind_lifo(effects).await;
                inner.providers.insert(
                    id.clone(),
                    Inner::make_entry(
                        provider,
                        ProviderState::Failed,
                        generation,
                        seq,
                        config,
                        Vec::new(),
                    ),
                );
                return Err(HarnessError::ActivationFailed {
                    id,
                    message: err.to_string(),
                });
            }
        }

        for claim in provider.spec().provides() {
            inner.claims.insert(claim.clone(), id.clone());
        }
        inner.providers.insert(
            id.clone(),
            Inner::make_entry(
                provider,
                ProviderState::Active,
                generation,
                seq,
                config,
                effects,
            ),
        );
        Ok(())
    }

    /// Late binding: retry every PENDING entry in insertion order now that
    /// new claims exist. Returns ids activated during this pass.
    async fn rescan_pending_locked(&self, inner: &mut Inner) -> Vec<String> {
        let order = std::mem::take(&mut inner.pending_order);
        let mut activated = Vec::new();
        let mut still_pending = Vec::new();

        for pid in order {
            let Some(entry) = inner.providers.get(&pid) else {
                continue;
            };
            if entry.state != ProviderState::Pending {
                continue;
            }
            let provider = Arc::clone(&entry.provider);
            let config = entry.config.clone();
            let generation = entry.generation + 1;

            let missing = inner.missing_requirements(provider.spec().requires());
            let conflict = inner.first_claim_conflict(provider.spec().provides(), None);
            if missing.is_empty() && conflict.is_none() {
                // Remove the parked entry; activate_locked re-inserts it live.
                inner.providers.remove(&pid);
                if self
                    .activate_locked(inner, provider, config, generation)
                    .await
                    .is_ok()
                {
                    self.audit(&pid, "active (late-bound)");
                    activated.push(pid);
                }
            } else {
                still_pending.push(pid);
            }
        }

        inner.pending_order = still_pending;
        activated
    }

    // ── unmount ────────────────────────────────────────────────────────

    /// Unmount a provider and every active dependent whose requirements it
    /// was satisfying (transitive). Dependents unwind first (highest seq
    /// first). Returns ids actually torn down, in teardown order.
    pub async fn unmount(&self, id: &str) -> Result<Vec<String>, HarnessError> {
        let mut inner = self.inner.write().await;
        if !inner.providers.contains_key(id) {
            return Err(HarnessError::NotFound(id.to_string()));
        }
        let doomed = Self::collect_dependents(&inner, id);
        Ok(self.teardown_many_locked(&mut inner, doomed).await)
    }

    /// Transitive closure of dependents rooted at `root` (includes root).
    fn collect_dependents(inner: &Inner, root: &str) -> Vec<String> {
        let mut doomed: Vec<String> = vec![root.to_string()];
        loop {
            let mut grew = false;
            for (eid, entry) in &inner.providers {
                if doomed.contains(eid) {
                    continue;
                }
                for req in entry.provider.spec().requires() {
                    if let Some(owner) = inner.claims.get(req)
                        && doomed.contains(owner)
                    {
                        doomed.push(eid.clone());
                        grew = true;
                        break;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        doomed
    }

    /// Teardown: Active entries unwind LIFO (dependents first via seq desc);
    /// Pending/Failed entries drop silently. Frees all owned claims.
    async fn teardown_many_locked(&self, inner: &mut Inner, ids: Vec<String>) -> Vec<String> {
        let mut ordered: Vec<(u64, String)> = ids
            .into_iter()
            .filter_map(|eid| inner.providers.get(&eid).map(|e| (e.seq, eid)))
            .collect();
        ordered.sort_by_key(|a| std::cmp::Reverse(a.0)); // youngest dependent first

        let mut unloaded = Vec::new();
        for (_, eid) in ordered {
            let Some(mut entry) = inner.providers.remove(&eid) else {
                continue;
            };
            inner.claims.retain(|_, owner| owner != &eid);
            inner.pending_order.retain(|p| p != &eid);
            if entry.state == ProviderState::Active {
                self.audit(&eid, "unloading");
                unwind_lifo(std::mem::take(&mut entry.effects)).await;
            } else {
                self.audit(&eid, "dropped");
            }
            unloaded.push(eid);
        }
        unloaded
    }

    // ── transactional replace ──────────────────────────────────────────

    /// Hot-swap the implementation behind `next.spec().id()`.
    ///
    /// Transactional: the replacement activates into a STAGING area first.
    /// Any failure leaves the previous instance fully serving. On success the
    /// old effects unwind, claims flip atomically, and the generation counter
    /// increments (ABA guard). Dependent providers survive iff the successor
    /// re-provides the same claims; otherwise they cascade-unload after commit.
    pub async fn replace(&self, next: Arc<dyn Provider>) -> Result<String, HarnessError> {
        let mut inner = self.inner.write().await;
        let target = next.spec().id().to_string();

        let Some(old_entry) = inner.providers.get(&target) else {
            return Err(HarnessError::NotFound(target));
        };

        if old_entry.state != ProviderState::Active {
            // Pending/Failed targets: drop and do a plain mount.
            let config = old_entry.config.clone();
            inner.providers.remove(&target);
            inner.pending_order.retain(|p| p != &target);
            let report = self.mount_locked(&mut inner, next, config).await?;
            let rescued = self.rescan_pending_locked(&mut inner).await;
            return Ok(match report {
                MountReport::Mounted { mut activated } => {
                    activated.extend(rescued);
                    activated.join(",")
                }
                MountReport::Pending { missing } => format!("pending (missing {missing:?})"),
            });
        }

        // Precondition: post-swap requirement satisfaction.
        for req in next.spec().requires() {
            let covered_by_next = next.spec().provides().contains(req);
            let owner_ok = match inner.claims.get(req) {
                Some(owner) => owner != &target,
                None => false,
            };
            if !covered_by_next && !owner_ok {
                return Err(HarnessError::ReplaceBreaksRequirement {
                    id: target.clone(),
                    requirement: req.clone(),
                });
            }
        }
        // Precondition: successor claims must not collide with survivors.
        if let Some(conflict) =
            inner.first_claim_conflict(next.spec().provides(), Some(target.as_str()))
        {
            return Err(conflict);
        }

        let old_generation = old_entry.generation;
        let old_config = old_entry.config.clone();

        // Stage: activate successor WITHOUT touching live claims/effects.
        let staged = match self.stage_activation(&next, &old_config).await {
            Ok(staged) => staged,
            Err(message) => {
                return Err(HarnessError::ActivationFailed {
                    id: target,
                    message,
                });
            }
        };

        // Commit point.
        let Some(mut old_entry) = inner.providers.remove(&target) else {
            return Err(HarnessError::NotFound(target)); // unreachable under lock
        };
        old_entry.state = ProviderState::Unloading;
        unwind_lifo(std::mem::take(&mut old_entry.effects)).await;
        let old_provides = old_entry.provider.spec().provides().to_vec();
        drop(old_entry);

        for claim in old_provides {
            if inner.claims.get(&claim).map(String::as_str) == Some(target.as_str()) {
                inner.claims.remove(&claim);
            }
        }
        for claim in next.spec().provides() {
            inner.claims.insert(claim.clone(), target.clone());
        }
        let seq = inner.alloc_seq();
        inner.providers.insert(
            target.clone(),
            Inner::make_entry(
                next,
                ProviderState::Active,
                old_generation + 1,
                seq,
                old_config,
                staged,
            ),
        );
        self.audit(&target, "replaced");

        // Cascade: anything left with a vanished dependency unwinds AFTER commit.
        let orphaned: Vec<String> = inner
            .providers
            .iter()
            .filter(|(eid, e)| {
                e.state == ProviderState::Active
                    && eid.as_str() != target
                    && e.provider
                        .spec()
                        .requires()
                        .iter()
                        .any(|r| !inner.claims.contains_key(r))
            })
            .map(|(eid, _)| eid.clone())
            .collect();
        if !orphaned.is_empty() {
            self.teardown_many_locked(&mut inner, orphaned).await;
        }

        let rescued = self.rescan_pending_locked(&mut inner).await;
        Ok(if rescued.is_empty() {
            target
        } else {
            format!("{target} (+late-bound {})", rescued.join(","))
        })
    }

    /// Activation into a detached buffer — no claims, no entry mutation.
    async fn stage_activation(
        &self,
        provider: &Arc<dyn Provider>,
        config: &Value,
    ) -> Result<Vec<Effect>, String> {
        let id = provider.spec().id().to_string();
        let mut effects = Vec::new();
        let mut cx = ActivateCx::new(&id, config, &self.seams, &mut effects);
        match provider.activate(&mut cx).await {
            Ok(()) => Ok(effects),
            Err(err) => {
                unwind_lifo(effects).await;
                Err(err.to_string())
            }
        }
    }

    // ── read paths ─────────────────────────────────────────────────────

    pub async fn state_of(&self, id: &str) -> Option<ProviderState> {
        self.inner.read().await.providers.get(id).map(|e| e.state)
    }

    pub async fn generation_of(&self, id: &str) -> Option<u64> {
        self.inner
            .read()
            .await
            .providers
            .get(id)
            .map(|e| e.generation)
    }

    pub async fn claim_owner(&self, claim: &Claim) -> Option<String> {
        self.inner.read().await.claims.get(claim).cloned()
    }

    /// Serializable resolved tree (sorted deterministically).
    pub async fn dump(&self) -> DumpTree {
        let inner = self.inner.read().await;
        let mut providers: Vec<ProviderEntryInfo> = inner
            .providers
            .values()
            .map(|e| ProviderEntryInfo {
                id: e.provider.spec().id().to_string(),
                source: e.provider.spec().source(),
                state: e.state,
                generation: e.generation,
                seq: e.seq,
                provides: e.provider.spec().provides().to_vec(),
                requires: e.provider.spec().requires().to_vec(),
                effects: e.effects.len(),
            })
            .collect();
        providers.sort_by_key(|p| p.seq);

        let mut claims: Vec<ClaimInfo> = inner
            .claims
            .iter()
            .map(|(claim, owner)| ClaimInfo {
                claim: claim.clone(),
                owner: owner.clone(),
            })
            .collect();
        claims.sort_by(|a, b| (&a.claim.seam, &a.claim.key).cmp(&(&b.claim.seam, &b.claim.key)));

        DumpTree {
            semantics_version: crate::HARNESS_SEMANTICS_VERSION,
            providers,
            claims,
        }
    }

    fn audit(&self, id: &str, event: &str) {
        if self.options.audit {
            tracing::info!(provider = %id, event, "[harness]");
        }
    }
}
