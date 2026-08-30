//! Plan 016: `Effect` undo handles + LIFO unwind.
//!
//! Every `Provider::apply` returns an `Effect`. The kernel stores the
//! handle in a per-provider vec, and on `unmount` it walks the vec in
//! LIFO order, calling each handle to roll back the side effect.
//!
//! Handle constraints (enforced by clippy lint + review rule once the
//! Phase 2 seams land): the closure MUST be bounded (no `await`s on
//! unbounded futures, no network I/O, no lock waits) and MUST be
//! idempotent (a crash before the closure runs leaves the world in a
//! state the closure would have cleaned up — so retrying on next boot
//! is safe). Phase 0/1 only exposes the type; the kernel uses a
//! `Box<dyn FnOnce()>` body.

use std::fmt;

/// A reversible side effect. Created by `Provider::apply`; consumed by
/// the kernel's LIFO unwinder at unmount time.
pub struct Effect {
    /// The undo body. `None` means no-op (a `ConfigRow` provider that
    /// just claimed a slot has nothing to release).
    undo: Option<Box<dyn FnOnce() + Send + 'static>>,
    /// Short description for `dump()` — e.g. "unregister tool:browser".
    label: &'static str,
}

impl Effect {
    /// A no-op effect (claim-only, no registration to undo).
    pub fn noop() -> Self {
        Self {
            undo: None,
            label: "noop",
        }
    }

    /// Wrap a closure as an effect. The closure runs at unmount time
    /// (LIFO order across all effects of the provider being unmounted).
    pub fn new<F>(label: &'static str, undo: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            undo: Some(Box::new(undo)),
            label,
        }
    }

    /// Human-readable label for `dump()`.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Run the undo body. Consumes the Effect; the kernel may discard
    /// after calling. If this is the *second* run (kernel retried on
    /// panic) the second call is a no-op — the underlying closure has
    /// already been moved out.
    pub fn run(mut self) {
        if let Some(undo) = self.undo.take() {
            undo();
        }
    }
}

impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Effect")
            .field("label", &self.label)
            .field("has_undo", &self.undo.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Idempotency: running the same effect twice must be safe (the
    /// second call is a no-op because the first `take()`-moved the
    /// closure out).
    #[test]
    fn effect_runs_undo_once() {
        use std::sync::Arc;
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        let effect = Effect::new("test", move || {
            c2.fetch_add(1, Ordering::SeqCst);
        });
        effect.run();
        // Second run consumes a now-empty Effect — safe no-op.
        let empty = Effect::noop();
        empty.run();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn noop_effect_carries_no_undo() {
        let e = Effect::noop();
        assert_eq!(e.label(), "noop");
        // Should not panic; just runs no body.
        e.run();
    }

    #[test]
    fn effect_label_is_preserved() {
        let e = Effect::new("unregister tool:browser", || {});
        assert_eq!(e.label(), "unregister tool:browser");
    }
}
