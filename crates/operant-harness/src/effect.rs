//! Reversible effects — every registration returns an undo handle.
//!
//! Mirrors Cordis's rule that prompt sections, tool schemas, adapters and
//! listeners are installed through disposables so teardown unwinds them
//! predictably. The kernel stores effects per provider and unwinds them in
//! LIFO order on unmount or transactional rollback.
//!
//! G9 — every `Effect::unwind` is bounded by `UNWIND_TIMEOUT_MS` (default
//! 5_000). A stuck undo (e.g. a network call inside `unregister_tool`)
//! would otherwise wedge the kernel's write lock and stall every
//! subsequent mount/replace. The timeout is per-effect, not global; the
//! host can override via [`set_unwind_timeout`].

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static UNWIND_TIMEOUT_MS: AtomicU64 = AtomicU64::new(5_000);

/// Set the per-effect unwind timeout (milliseconds). Default 5_000.
/// Changes apply to subsequent unwinds; in-flight ones keep their own
/// timer.
pub fn set_unwind_timeout(timeout: Duration) {
    UNWIND_TIMEOUT_MS.store(timeout.as_millis() as u64, Ordering::Relaxed);
}

fn current_timeout() -> Duration {
    Duration::from_millis(UNWIND_TIMEOUT_MS.load(Ordering::Relaxed))
}

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

    /// Consume the effect, running its undo closure (if any). G9 —
    /// bounded by `UNWIND_TIMEOUT_MS`; a stuck undo logs a warning and
    /// the kernel continues with the rest of the LIFO chain.
    pub(crate) async fn unwind(mut self) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        let timeout = current_timeout();
        let label = self.label.clone();
        let timed = tokio::time::timeout(timeout, undo());
        match timed.await {
            Ok(()) => {}
            Err(_) => {
                tracing::warn!(
                    effect = %label,
                    timeout_ms = timeout.as_millis() as u64,
                    "effect unwind exceeded timeout — kernel continues"
                );
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering as AOrd};
    use std::time::Duration;

    #[tokio::test]
    async fn stuck_undo_does_not_block_subsequent_unwinds() {
        // Build two effects. First one stalls forever; second one
        // increments a counter immediately. With G9's timeout the first
        // is bounded and the second still runs.
        let counter = Arc::new(AtomicU32::new(0));
        let counter2 = counter.clone();

        // Effect 1: never completes (suspended forever).
        let stuck = Effect::new("stuck", || {
            Box::pin(async move {
                std::future::pending::<()>().await;
            })
        });

        // Effect 2: completes instantly.
        let fast = Effect::new("fast", move || {
            let c = counter2.clone();
            Box::pin(async move {
                c.fetch_add(1, AOrd::SeqCst);
            })
        });

        set_unwind_timeout(Duration::from_millis(200));

        let start = std::time::Instant::now();
        unwind_lifo(vec![fast, stuck]).await; // LIFO: stuck runs first
        let elapsed = start.elapsed();

        // We must have returned in roughly 200ms (timeout), not blocked
        // forever, and the fast effect must have run (LIFO order: stuck
        // first → timeouts → fast still runs because we continue the
        // chain after the timeout warning).
        // Note: the LIFO chain calls stuck.await first; the timeout
        // fires after 200ms; then fast.await runs synchronously and
        // increments.
        assert!(
            elapsed < Duration::from_secs(2),
            "unwind took {elapsed:?} — timeout did not fire"
        );
        assert_eq!(counter.load(AOrd::SeqCst), 1, "fast effect must run");
    }
}
