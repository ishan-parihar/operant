//! Live slots for kernel-evolved prompt sections and hooks (audit C1).
//!
//! The harness kernel lets providers install into named seams at boot and
//! change later. Two families had no live consumer: `prompt` and `hook`.
//! The kernel adapters live in `operant-runtime`
//! (`agent::prompt_seam` / `hooks::harness_seam`) and their only
//! constructors are tests — the live CLI path (`operant-core`'s
//! `OperantAgent`, which the agent factories build) has its own prompt
//! pipeline and its own event-based `HookRegistry`, so wiring the runtime
//! slots would have lit a path the CLI never walks.
//!
//! This module provides the same contract natively on the live path:
//!
//! * [`PromptSlot`] — a shared, ordered list of kernel-installed prompt
//!   sections. `prompt.section` providers install a `String` or a
//!   `PromptSection`; the agent appends the rendered sections to its system
//!   prompt at every cache-refresh point (bounded by the provider's own
//!   `max_items`).
//! * [`HookSlot`] — a shared dynamic hook list. `hook` providers install a
//!   `DynamicHook`; the agent registers a bridge handler on its
//!   `HookRegistry` so kernel hooks fire on the live loop's hook events
//!   (AgentStart, AgentEnd, PreTool, PostTool, ...).
//!
//! Both are inert unless a boot path constructs them and hands them to the
//! seams (see `cmd_doctor`-independent wiring in `operant-cli`:
//! `build_harness_host`).

use std::sync::Arc;
use std::sync::RwLock;

use async_trait::async_trait;
use operant_harness::{Effect, HarnessError, Registration, Seam};

use crate::gateway_pipeline::{HookContext, HookEvent};

// ─── prompt slot ───────────────────────────────────────────────────────────

/// A kernel-installed prompt section.
#[derive(Clone)]
pub struct PromptSectionEntry {
    /// Stable id (the kernel claim key), used for replace/remove.
    pub id: String,
    /// Section body, or a provider-rendered closure. The `String` is the
    /// common case (`prompt.section` config rows install literal text).
    pub render: Arc<dyn Fn() -> String + Send + Sync>,
}

impl std::fmt::Debug for PromptSectionEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptSectionEntry")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Shared prompt-section slot. Cheap to clone via [`Arc`].
#[derive(Default)]
pub struct PromptSlot {
    entries: RwLock<Vec<PromptSectionEntry>>,
    /// Maximum number of sections kept. Bounded so a hot loop (every
    /// turn) cannot grow the system prompt without limit; a provider that
    /// installs beyond the cap gets its install dropped (the kernel keeps
    /// the claim either way — swaps/rescans are how a provider is
    /// re-applied after a lower-priority section leaves).
    max_items: usize,
}

/// Default section cap for the live prompt slot.
pub const DEFAULT_PROMPT_SLOT_MAX_ITEMS: usize = 32;

impl PromptSlot {
    /// A slot with the default cap.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            max_items: DEFAULT_PROMPT_SLOT_MAX_ITEMS,
        }
    }

    /// A slot with an explicit cap.
    pub fn with_max_items(max_items: usize) -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            max_items,
        }
    }

    /// Insert or replace by id. Returns false when the cap is reached and
    /// the id is new. An existing id is always replaced in place (the cap
    /// applies to distinct sections, not updates).
    pub fn install(
        &self,
        id: impl Into<String>,
        render: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> bool {
        let id = id.into();
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
            entry.render = render;
            return true;
        }
        if entries.len() >= self.max_items {
            return false;
        }
        entries.push(PromptSectionEntry { id, render });
        true
    }

    /// Remove by id. Returns true when it existed.
    pub fn remove(&self, id: &str) -> bool {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = entries.len();
        entries.retain(|e| e.id != id);
        entries.len() != before
    }

    /// Current section count.
    pub fn len(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// True when no section is installed (the agent appends nothing).
    pub fn is_empty(&self) -> bool {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    }

    /// Render every section, joined with blank lines. Empty when the slot
    /// is empty — the caller appends nothing and the prompt stays
    /// byte-identical to the non-harness path.
    pub fn render(&self) -> String {
        let sections: Vec<String> = self
            .entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|e| (e.render)())
            .filter(|s| !s.trim().is_empty())
            .collect();
        sections.join("\n\n")
    }
}

// ─── hook slot ─────────────────────────────────────────────────────────────

/// A kernel-installed hook. The live loop's hook events are event-based
/// (`HookEvent`); a provider implements this trait and its handler is
/// invoked on every matching event.
#[async_trait]
pub trait DynamicHook: Send + Sync {
    /// Handle an event. Returning `None` continues; `Some(reason)` cancels
    /// the turn (only honored for pre-tool events by the bridge).
    async fn on_event(&self, event: &HookEvent, ctx: &HookContext) -> Option<String>;
}

// ─── seams ─────────────────────────────────────────────────────────────────

/// Prompt-section seam: claims are `prompt/<id>`; payloads are `String` or
/// `Arc<dyn Fn() -> String + Send + Sync>` (provider-rendered sections).
pub struct PromptSlotSeam {
    slot: Arc<PromptSlot>,
}

impl PromptSlotSeam {
    pub fn new(slot: Arc<PromptSlot>) -> Self {
        Self { slot }
    }
}

#[async_trait]
impl Seam for PromptSlotSeam {
    fn name(&self) -> &str {
        "prompt"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let id = reg.key.to_string();
        if let Some(render) = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn Fn() -> String + Send + Sync>>())
            .cloned()
        {
            self.slot.install(id.clone(), render);
        } else if let Some(text) = reg.payload.and_then(|p| p.downcast_ref::<String>()) {
            let text = text.clone();
            self.slot
                .install(id.clone(), Arc::new(move || text.clone()));
        } else {
            return Err(HarnessError::CompositionError(format!(
                "prompt seam: unsupported payload for claim '{}' (expected String or render closure)",
                reg.key
            )));
        }
        let slot = Arc::clone(&self.slot);
        Ok(Effect::new(format!("prompt:{id}"), move || {
            Box::pin(async move {
                slot.remove(&id);
            })
        }))
    }
}

/// Hook seam: claims are `hook/<id>`; payloads are `Arc<dyn DynamicHook>`.
pub struct HookSlotSeam {
    slot: Arc<HookSlot>,
}

impl HookSlotSeam {
    pub fn new(slot: Arc<HookSlot>) -> Self {
        Self { slot }
    }
}

#[async_trait]
impl Seam for HookSlotSeam {
    fn name(&self) -> &str {
        "hook"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let id = reg.key.to_string();
        if let Some(hook) = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn DynamicHook>>())
        {
            self.slot.install(id.clone(), Arc::clone(hook));
        } else {
            return Err(HarnessError::CompositionError(format!(
                "hook seam: unsupported payload for claim '{}' (expected Arc<dyn DynamicHook>)",
                reg.key
            )));
        }
        let slot = Arc::clone(&self.slot);
        Ok(Effect::new(format!("hook:{id}"), move || {
            Box::pin(async move {
                slot.remove(&id);
            })
        }))
    }
}

// ─── hook slot ─────────────────────────────────────────────────────────────

type HookEntry = (String, Arc<dyn DynamicHook>);

/// Shared dynamic-hook slot. Cheap to clone via [`Arc`].
#[derive(Default)]
pub struct HookSlot {
    entries: RwLock<Vec<HookEntry>>,
}

impl HookSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace by id. Returns the previous hook (if replaced).
    pub fn install(
        &self,
        id: impl Into<String>,
        hook: Arc<dyn DynamicHook>,
    ) -> Option<Arc<dyn DynamicHook>> {
        let id = id.into();
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = entries.iter_mut().find(|(eid, _)| *eid == id) {
            let prev = std::mem::replace(&mut entry.1, hook);
            return Some(prev);
        }
        entries.push((id, hook));
        None
    }

    /// Remove by id. Returns true when it existed.
    pub fn remove(&self, id: &str) -> bool {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = entries.len();
        entries.retain(|(eid, _)| eid != id);
        entries.len() != before
    }

    /// Current hook count.
    pub fn len(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// True when no hook is installed.
    pub fn is_empty(&self) -> bool {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    }

    /// Dispatch to every installed hook in insertion order. Returns the
    /// first cancel reason, if any.
    pub async fn dispatch(&self, event: &HookEvent, ctx: &HookContext) -> Option<String> {
        let entries: Vec<Arc<dyn DynamicHook>> = self
            .entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(_, h)| Arc::clone(h))
            .collect();
        for hook in entries {
            if let Some(reason) = hook.on_event(event, ctx).await {
                return Some(reason);
            }
        }
        None
    }

    /// A `HookRegistry` handler that fans every registry event into the
    /// slot. A cancel reason is logged and stops the fan-out for that
    /// event (the core `HookRegistry` has no cancel channel — handlers are
    /// `Fn` futures with no return path).
    pub fn bridge_handler(slot: Arc<HookSlot>) -> crate::gateway_pipeline::HookHandler {
        Arc::new(move |event, ctx| {
            let slot = Arc::clone(&slot);
            Box::pin(async move {
                if let Some(reason) = slot.dispatch(&event, &ctx).await {
                    tracing::warn!(
                        event = ?event,
                        reason = %reason,
                        "kernel hook cancelled the event"
                    );
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::{AtomicUsize, Ordering};

    use operant_harness::{
        ActivateCx, Claim, Harness, KernelOptions, Provider, ProviderSource, ProviderSpec,
    };

    use super::*;

    struct SectionProvider {
        id: String,
        text: String,
        provides: Vec<Claim>,
    }

    impl ProviderSpec for SectionProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn source(&self) -> ProviderSource {
            ProviderSource::Native
        }
        fn provides(&self) -> &[Claim] {
            &self.provides
        }
        fn requires(&self) -> &[Claim] {
            &[]
        }
    }

    #[async_trait]
    impl Provider for SectionProvider {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
            cx.install_with("prompt", "section", &self.text).await?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn prompt_seam_mount_lands_in_slot_and_unmount_clears_it() {
        // C1 end-to-end: a provider mounting through the kernel installs
        // a prompt section in the shared slot; the kernel unmount (the
        // seam effect's undo) removes it.
        let slot = Arc::new(PromptSlot::new());
        let mut harness = Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(PromptSlotSeam::new(Arc::clone(&slot))));

        let provider = SectionProvider {
            id: "sec-a".to_string(),
            text: "kernel says hello".to_string(),
            provides: vec![Claim {
                seam: "prompt".to_string(),
                key: "section".to_string(),
            }],
        };
        harness.mount(Arc::new(provider)).await.unwrap();
        assert_eq!(slot.len(), 1);
        assert!(slot.render().contains("kernel says hello"));

        harness.unmount("sec-a").await.unwrap();
        assert_eq!(slot.len(), 0);
        assert!(slot.render().is_empty());
    }

    struct CountingHook {
        seen: Arc<AtomicUsize>,
        cancel_on: Option<HookEvent>,
    }

    #[async_trait]
    impl DynamicHook for CountingHook {
        async fn on_event(&self, event: &HookEvent, _ctx: &HookContext) -> Option<String> {
            self.seen.fetch_add(1, Ordering::SeqCst);
            if self.cancel_on.as_ref() == Some(event) {
                return Some("nope".to_string());
            }
            None
        }
    }

    #[tokio::test]
    async fn hook_bridge_fires_kernel_hooks_on_live_events() {
        // The live agent's HookRegistry reaches kernel hooks through the
        // bridge; the `Hooks("*")` pattern fans every event.
        let slot = Arc::new(HookSlot::new());
        let seen = Arc::new(AtomicUsize::new(0));
        slot.install(
            "count",
            Arc::new(CountingHook {
                seen: Arc::clone(&seen),
                cancel_on: None,
            }),
        );

        let registry = crate::gateway_pipeline::HookRegistry::new();
        registry
            .register(
                HookEvent::Hooks("*".to_string()),
                HookSlot::bridge_handler(Arc::clone(&slot)),
            )
            .await;

        registry
            .emit(
                HookEvent::AgentStart,
                crate::gateway_pipeline::HookContext::new().with_session("s1"),
            )
            .await;
        registry
            .emit(
                HookEvent::AgentEnd,
                crate::gateway_pipeline::HookContext::new().with_session("s1"),
            )
            .await;
        // `emit` spawns handlers (fire-and-forget by design), so poll for
        // the two bridge invocations instead of asserting on the counter
        // immediately. Bounded so a broken bridge fails the test rather
        // than hanging it.
        for _ in 0..100 {
            if seen.load(Ordering::SeqCst) == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(seen.load(Ordering::SeqCst), 2);

        // A cancel reason propagates out of dispatch (the bridge logs it —
        // the core registry has no cancel channel).
        let cancel = Arc::new(HookSlot::new());
        cancel.install(
            "blocker",
            Arc::new(CountingHook {
                seen: Arc::new(AtomicUsize::new(0)),
                cancel_on: Some(HookEvent::AgentStart),
            }),
        );
        assert_eq!(
            cancel
                .dispatch(
                    &HookEvent::AgentStart,
                    &crate::gateway_pipeline::HookContext::new()
                )
                .await,
            Some("nope".to_string())
        );

        // Slot removal is observable.
        assert!(slot.remove("count"));
        assert_eq!(slot.len(), 0);
    }
}
