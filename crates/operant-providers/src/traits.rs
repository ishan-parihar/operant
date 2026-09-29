pub use operant_api::provider::*;

/// RAII guard that aborts a spawned tokio task when dropped.
///
/// Providers hand the consumer a `Stream` backed by a `tokio::spawn`ed
/// producer task. Binding the task's lifetime to the stream state means that
/// when a caller cancels the stream (timeout, user abort, client disconnect)
/// the producer is cancelled too, instead of reading the response body to
/// completion and holding a connection-pool slot (and provider quota) for a
/// request nobody wants. `AbortHandle::abort` is a no-op once the task has
/// finished naturally, so the happy path is unaffected.
///
/// Carried through `stream::unfold`'s state alongside the receiver:
/// `stream::unfold((rx, guard), |(mut rx, guard)| ...)`.
///
/// See `openrouter::stream_chat` (issue #5822) for the original site.
pub(crate) struct AbortOnDrop(tokio::task::AbortHandle);

impl AbortOnDrop {
    /// Bind `handle` to a new guard. Dropping the guard aborts the task.
    pub(crate) fn new(handle: &tokio::task::JoinHandle<()>) -> Self {
        Self(handle.abort_handle())
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod abort_on_drop_tests {
    use super::AbortOnDrop;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::time::timeout;

    /// The load-bearing property of the guard shape used by every
    /// `tokio::spawn` + `stream::unfold` provider pair: dropping the state
    /// that carries the guard aborts the task and cancels its side effects.
    #[tokio::test]
    async fn dropping_the_guard_aborts_the_producer_task() {
        let finished = Arc::new(AtomicBool::new(false));
        let finished_clone = Arc::clone(&finished);

        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            finished_clone.store(true, Ordering::SeqCst);
        });
        let observer = handle.abort_handle();
        let guard = AbortOnDrop::new(&handle);

        assert!(!observer.is_finished());

        drop(guard);

        let cancelled = timeout(Duration::from_secs(2), async {
            while !observer.is_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(
            cancelled.is_ok(),
            "producer task must be aborted within 2s of the guard being dropped"
        );
        assert!(
            !finished.load(Ordering::SeqCst),
            "aborted task must not run its completion side effect"
        );
    }

    /// A finished task is left alone — `abort` on a completed task is a no-op,
    /// so the happy path (stream drained to the end) is unaffected.
    #[tokio::test]
    async fn guard_drop_after_natural_completion_does_not_panic() {
        let handle = tokio::spawn(async {});
        let guard = AbortOnDrop::new(&handle);
        handle.await.ok();
        drop(guard);
    }
}
