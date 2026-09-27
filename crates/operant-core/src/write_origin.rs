//! Write-origin provenance — async-safe tracking for background review context.
//!
//! Mirrors `hermes-agent/tools/skill_provenance.py`. The background review
//! agent fork sets the origin to `"background_review"` so that the skill
//! manager can enforce write guards (no editing protected/hub skills, no
//! creating skills outside the review's scope).
//!
//! Two tiers, read in this order:
//!
//! 1. **Task-local** (`TASK_ORIGIN`): [`scope_background_review`] runs a
//!    future with the origin set for that task only. The production review
//!    daemon uses this — while it runs, a concurrent main-agent turn (a
//!    different Tokio task) still reads the global origin, so its skill
//!    writes are not write-guarded (R39-10 residual, closed iter-338).
//! 2. **Process-global** (`WRITE_ORIGIN`, `Arc<RwLock<String>>`): the
//!    fallback for non-Tokio contexts (sync code, `std::thread` test
//!    harnesses) and the tier tests drive via `set_write_origin` /
//!    [`WriteOriginGuard`].
//!
//! The global keeps `Arc<RwLock<String>>` (not a thread-local) because a
//! Tokio task can be polled on different OS threads by the work-stealing
//! runtime; the task-local tier is runtime-managed and survives migration
//! by construction.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::write_origin::{set_write_origin, reset_write_origin, is_background_review};
//!
//! let token = set_write_origin("background_review");
//! assert!(is_background_review());
//! reset_write_origin(token);
//! assert!(!is_background_review());
//! ```

use std::sync::{Arc, LazyLock, RwLock};

tokio::task_local! {
    /// Task-local origin tier: visible only inside the task whose future was
    /// wrapped by [`scope_background_review`]. Preferred by every read.
    static TASK_ORIGIN: String;
}

/// Shared origin string (process-global fallback tier). `Arc` so it can be
/// cloned into spawned tasks; `RwLock` so reads (the hot path) don't block
/// and writes are rare.
static WRITE_ORIGIN: LazyLock<Arc<RwLock<String>>> =
    LazyLock::new(|| Arc::new(RwLock::new("assistant_tool".to_string())));

/// Run `fut` with the write origin set to `"background_review"` for THAT
/// TASK ONLY — the process-global origin is not touched, so concurrent
/// tasks (a live TUI/gateway turn, another daemon) keep reading their own
/// origin while the review runs. This is the production entry point for the
/// background-review fork (see `agent/prompting.rs`).
pub fn scope_background_review<F>(fut: F) -> impl Future<Output = F::Output>
where
    F: Future + Send,
{
    TASK_ORIGIN.scope("background_review".to_string(), fut)
}

/// Token returned by [`set_write_origin`] for scoped reset.
#[derive(Debug, Clone)]
pub struct WriteOriginToken {
    /// The previous origin value, used for scoped restore.
    previous: String,
}

/// Set the current write origin and return a token for scoped reset.
///
/// The token must be passed to [`reset_write_origin`] when the scoped
/// context ends. This prevents accidental leaking of the review origin
/// into subsequent foreground turns.
pub fn set_write_origin(origin: &str) -> WriteOriginToken {
    let previous = {
        let lock = WRITE_ORIGIN.read().unwrap_or_else(|e| e.into_inner());
        lock.clone()
    };
    {
        let mut lock = WRITE_ORIGIN.write().unwrap_or_else(|e| e.into_inner());
        *lock = origin.to_string();
    }
    WriteOriginToken { previous }
}

/// Reset the write origin to the value before [`set_write_origin`] was called.
pub fn reset_write_origin(token: WriteOriginToken) {
    let mut lock = WRITE_ORIGIN.write().unwrap_or_else(|e| e.into_inner());
    *lock = token.previous;
}

/// Get the current write origin string.
///
/// Task-local tier first (set by [`scope_background_review`]); falls back to
/// the process-global slot when the caller runs outside a scoped Tokio task
/// (sync code, `std::thread`, unscoped tests).
pub fn get_write_origin() -> String {
    TASK_ORIGIN.try_with(Clone::clone).unwrap_or_else(|_| {
        let lock = WRITE_ORIGIN.read().unwrap_or_else(|e| e.into_inner());
        lock.clone()
    })
}

/// Returns `true` when the current execution context is the background review
/// agent fork. This is the primary guard-check used by the skill manager.
pub fn is_background_review() -> bool {
    let lock = WRITE_ORIGIN.read().unwrap_or_else(|e| e.into_inner());
    *lock == "background_review"
}

/// Plan 012 alias: `current_origin()` mirrors hermes `write_approval.current_origin()`.
/// Returns the active origin string (e.g. "user", "background_review", "gateway:telegram").
pub fn current_origin() -> String {
    get_write_origin()
}

/// Plan 012 alias: `is_background()` is the broader non-interactive-origin test.
/// Returns true for origins originating from remote channels or async daemons
/// (background_review, gateway:*, cron_*, code_execution). Interactive
/// `user` and in-process `assistant_tool` origins bypass the gate.
pub fn is_background() -> bool {
    let origin = get_write_origin();
    matches!(
        origin.as_str(),
        "background_review" | "gateway" | "cron" | "code_execution"
    ) || origin.starts_with("gateway:")
        || origin.starts_with("cron_")
}

/// Scoping guard — sets the origin on creation and resets it on drop.
///
/// # Example
///
/// ```rust,ignore
/// {
///     let _guard = WriteOriginGuard::background_review();
///     assert!(is_background_review());
/// } // _guard drops here, origin resets
/// assert!(!is_background_review());
/// ```
pub struct WriteOriginGuard {
    token: WriteOriginToken,
}

impl WriteOriginGuard {
    /// Create a guard that sets the origin to `"background_review"`.
    pub fn background_review() -> Self {
        let token = set_write_origin("background_review");
        Self { token }
    }

    /// Create a guard with a custom origin string.
    pub fn new(origin: &str) -> Self {
        let token = set_write_origin(origin);
        Self { token }
    }
}

impl Drop for WriteOriginGuard {
    fn drop(&mut self) {
        let token = WriteOriginToken {
            previous: self.token.previous.clone(),
        };
        reset_write_origin(token);
    }
}

#[cfg(test)]
pub(crate) static ORIGIN_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Test-only: take the cross-module lock that serializes every test
/// touching the process-global `WRITE_ORIGIN` slot (write_approval,
/// skills_tool, and this module's own tests). One owner for the rule —
/// per-module test locks would let two tests race the same global.
#[cfg(test)]
pub(crate) fn origin_test_lock() -> std::sync::MutexGuard<'static, ()> {
    ORIGIN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Consolidated test: exercises set/reset, guard scoping, and nested
    /// operations in a single sequential block so parallel test threads
    /// don't race on the global static.
    #[test]
    fn test_write_origin_roundtrip() {
        let _lock = origin_test_lock();
        let before = get_write_origin();

        // ── set / reset ──
        let token = set_write_origin("background_review");
        assert!(is_background_review());
        assert_eq!(get_write_origin(), "background_review");
        reset_write_origin(token);
        assert_eq!(get_write_origin(), before);

        // ── guard scoping ──
        {
            let _guard = WriteOriginGuard::background_review();
            assert!(is_background_review());
        }
        assert_eq!(get_write_origin(), before);

        // ── custom origin guard ──
        {
            let _guard = WriteOriginGuard::new("curator");
            assert_eq!(get_write_origin(), "curator");
            assert!(!is_background_review());
        }
        assert_eq!(get_write_origin(), before);

        // ── nested set/reset ──
        let outer = set_write_origin("background_review");
        assert!(is_background_review());

        let inner = set_write_origin("curator");
        assert_eq!(get_write_origin(), "curator");

        reset_write_origin(inner);
        assert!(is_background_review());

        reset_write_origin(outer);
        assert_eq!(get_write_origin(), before);
    }

    /// The cross-module test lock must actually serialize writers on the
    /// global origin slot: two threads honoring `origin_test_lock()` and
    /// doing set → assert → reset loops must never observe each other's
    /// values. Mutation check (2026-09-27): removing one thread's lock
    /// acquisition makes this fail within a few hundred iterations
    /// ("b" observed while "a" was set); with both locks held it is green
    /// across repeated runs.
    #[test]
    fn origin_test_lock_serializes_concurrent_writers() {
        let a = std::thread::spawn(|| {
            for _ in 0..2_000 {
                let _lock = origin_test_lock();
                let token = set_write_origin("thread_a");
                std::thread::yield_now();
                assert_eq!(get_write_origin(), "thread_a");
                reset_write_origin(token);
            }
        });
        let b = std::thread::spawn(|| {
            for _ in 0..2_000 {
                let _lock = origin_test_lock();
                let token = set_write_origin("thread_b");
                std::thread::yield_now();
                assert_eq!(get_write_origin(), "thread_b");
                reset_write_origin(token);
            }
        });
        a.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
        b.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// Regression test for the R39-10 production race: while a background
    /// review daemon holds the `background_review` origin in ITS task, a
    /// concurrent task (a live TUI/gateway turn) must still read its own
    /// origin. The old process-global set made the concurrent turn read
    /// `background_review` and silently write-guard its skill writes for the
    /// whole review window; the task-local scope keeps them isolated.
    /// Synchronization is deterministic: `entered_rx` guarantees the review
    /// scope is live before the main-task assertion, `release_tx` lets it end.
    #[expect(
        clippy::await_holding_lock,
        reason = "test-only serialization: the lock must span the await window so other modules' origin tests cannot park values mid-assertion; the current_thread test runtime never migrates this task"
    )]
    #[tokio::test]
    async fn task_scoped_origin_does_not_leak_to_concurrent_tasks() {
        let _lock = origin_test_lock();
        // Capture whatever the process-global tier currently holds (other
        // test modules legitimately park values like "user" in it); the
        // invariant under test is that the review scope does NOT change it.
        let main_before = current_origin();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

        let review = tokio::spawn(scope_background_review(async move {
            let _ = entered_tx.send(());
            let _ = release_rx.await;
            current_origin()
        }));

        let _ = entered_rx.await;
        // While the review scope is active in its own task, THIS task must
        // still read exactly what it read before the scope existed — not the
        // review's background_review origin (the old process-global bug).
        assert_eq!(
            current_origin(),
            main_before,
            "concurrent task saw the review's background_review origin"
        );
        let _ = release_tx.send(());

        let scoped_origin = review.await.unwrap_or_default();
        assert_eq!(scoped_origin, "background_review");
    }
}
