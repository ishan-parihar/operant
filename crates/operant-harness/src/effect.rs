//! Reversible effects — every registration returns an undo handle.
//!
//! Mirrors Cordis's rule that prompt sections, tool schemas, adapters and
//! listeners are installed through disposables so teardown unwinds them
//! predictably. The kernel stores effects per provider and unwinds them in
//! LIFO order on unmount or transactional rollback.

use std::future::Future;
use std::pin::Pin;

/// A boxed, sendable, owned-output future used by undo closures.
pub type BoxUndoFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Undo closure type. Must be bounded work: no network calls, no unbounded
/// awaits (enforced by review convention; see plan 016 risks table).
pub type UndoFn = Box<dyn FnOnce() -> BoxUndoFuture + Send + Sync>;

/// One reversible registration owned by a provider.
pub struct Effect {
    label: String,
    undo: Option<UndoFn>,
}

impl Effect {
    /// Build an effect from a synchronous-ish async undo closure.
    pub fn new(
        label: impl Into<String>,
        undo: impl FnOnce() -> BoxUndoFuture + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            undo: Some(Box::new(undo)),
        }
    }

    /// An effect that records nothing to undo (e.g. pure observation).
    pub fn noop(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            undo: None,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Consume the effect, running its undo closure (if any).
    pub(crate) async fn unwind(mut self) {
        if let Some(undo) = self.undo.take() {
            undo().await;
        }
    }
}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effect")
            .field("label", &self.label)
            .field("has_undo", &self.undo.is_some())
            .finish()
    }
}

/// Unwind effects in LIFO order. Errors are contained: one failing undo never
/// prevents the rest from running (Cordis fiber-hardening semantics).
pub(crate) async fn unwind_lifo(effects: Vec<Effect>) {
    for effect in effects.into_iter().rev() {
        tracing::debug!(effect = %effect.label(), "unwinding effect");
        effect.unwind().await;
    }
}
