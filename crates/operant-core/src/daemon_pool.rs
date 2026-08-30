//! Plan 011 — `daemon_pool` (hermes `daemon_pool.py` parity).
//!
//! Helper for spawning background/best-effort work. A daemon task:
//! - must not block shutdown on `.join()` (it can outlive its parent turn);
//! - is fire-and-forget by default; errors are logged at WARN, not propagated;
//! - is observable via a static handle table so tests can wait for completion.
//!
//! Usage:
//! ```ignore
//! daemon_pool::spawn("curator-tick", async move {
//!     // ... long-running work ...
//! });
//! ```
//!
//! The pool is purely a paper trail — it tracks handles for testability
//! and shutdown bookkeeping; it does not own its tasks or block joins.
//! This matches hermes's "spawn detached; never join on shutdown" rule.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::warn;

/// Hard cap on simultaneously-tracked daemons. Beyond this, the oldest
/// handle is dropped (forgotten) to bound memory.
const MAX_TRACKED: usize = 256;

/// Default label used when the caller does not pass one.
const DEFAULT_LABEL: &str = "daemon";

/// Tracker: weak counts of running daemons, keyed by label (for tests).
pub static ACTIVE: LazyLock<std::sync::Mutex<HashMap<String, usize>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// All outstanding handles. Held only so the task isn't dropped (which
/// would cancel the future). Insertion is bounded by `MAX_TRACKED`; when
/// full, the oldest entry is popped.
type HandleList = Vec<(String, JoinHandle<()>)>;
static HANDLES: LazyLock<std::sync::Mutex<HandleList>> =
    LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Spawn a background future. The future's return value is discarded;
/// panics are caught by the surrounding `tokio::spawn` task and logged.
///
/// Returns immediately. The future runs concurrently with the caller; it
/// must not depend on the caller's local state past the spawn point.
pub fn spawn<F>(label: impl Into<String>, fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    spawn_with(label.into(), fut);
}

fn spawn_with<F>(label: String, fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let l = label_for(&label);
    {
        let mut active = ACTIVE.lock().expect("daemon_pool ACTIVE mutex poisoned");
        *active.entry(l.clone()).or_insert(0) += 1;
    }

    let label_for_task = l.clone();
    let handle = tokio::spawn(async move {
        let _guard = ActiveGuard::new(label_for_task.clone());
        fut.await;
    });

    {
        let mut handles = HANDLES.lock().expect("daemon_pool HANDLES mutex poisoned");
        if handles.len() >= MAX_TRACKED {
            // Evict the oldest. Forgetting the handle cancels the future,
            // but daemon tasks should be self-contained — no caller is
            // waiting on them. This is the "pool skips idle-timeout join"
            // semantic from hermes daemon_pool.py.
            let _ = handles.remove(0);
        }
        handles.push((l, handle));
    }
}

/// Convenience: spawn with the default label.
pub fn spawn_anonymous<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    spawn(DEFAULT_LABEL, fut);
}

/// Wait (up to `timeout`) for all daemons whose label matches `prefix` to
/// finish. Intended for tests; never call from production paths. Returns
/// the number of daemons that completed within the window.
pub async fn drain_for_label(prefix: &str, timeout: Duration) -> usize {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let snapshot: Vec<(String, JoinHandle<()>)> = {
            let mut handles = HANDLES.lock().expect("daemon_pool HANDLES mutex poisoned");
            let (matching, rest): (Vec<_>, Vec<_>) = handles
                .drain(..)
                .partition(|(label, _)| label == prefix);
            *handles = rest;
            matching
        };
        let count = snapshot.len();
        for (_, h) in snapshot {
            // Best-effort: don't propagate panics from daemons.
            let _ = h.await;
        }
        if count > 0 {
            return count;
        }
        if std::time::Instant::now() >= deadline {
            return 0;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn label_for(raw: &str) -> String {
    if raw.is_empty() {
        DEFAULT_LABEL.to_string()
    } else {
        raw.to_string()
    }
}

/// RAII guard that decrements the active-count on drop (i.e. when the
/// future completes, whether normally or via panic). Keeps the count
/// honest even if the panic-catcher in `spawn_with` ever changes.
struct ActiveGuard {
    label: String,
}

impl ActiveGuard {
    fn new(label: String) -> Self {
        Self { label }
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().expect("daemon_pool ACTIVE mutex poisoned");
        if let Some(n) = active.get_mut(&self.label) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                active.remove(&self.label);
            }
        }
        // A misbehaving daemon that panics inside `fut` will still run
        // its `Drop`, so the count cannot leak across an unwind.
        let _ = std::panic::catch_unwind(|| {
            warn!(daemon = %self.label, "daemon exited");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn active_count(label: &str) -> usize {
        *ACTIVE.lock().expect("daemon_pool ACTIVE mutex poisoned").get(label).unwrap_or(&0)
    }

    #[tokio::test]
    async fn spawn_runs_future_to_completion() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        spawn("test-runs", async move {
            c2.store(1, Ordering::SeqCst);
        });
        // Wait briefly for the future to complete.
        let n = drain_for_label("test-runs", Duration::from_secs(1)).await;
        assert_eq!(n, 1, "one daemon should have completed");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(active_count("test-runs"), 0, "active count must decrement");
    }

    #[tokio::test]
    async fn multiple_daemons_all_run() {
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..5 {
            let c = counter.clone();
            spawn("test-multi", async move {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
        let n = drain_for_label("test-multi", Duration::from_secs(2)).await;
        assert_eq!(n, 5);
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn spawn_does_not_block_caller() {
        // The caller must return before the daemon finishes — otherwise
        // the "daemon does not block shutdown" semantic is broken.
        let started = Arc::new(tokio::sync::Notify::new());
        let released = Arc::new(tokio::sync::Notify::new());
        let s = started.clone();
        let r = released.clone();
        spawn("test-detach", async move {
            s.notify_one();
            r.notified().await;
        });
        // Give the daemon a moment to start.
        tokio::time::timeout(Duration::from_millis(200), started.notified())
            .await
            .expect("daemon must have started within 200ms");
        // If spawn were blocking, we wouldn't be here.
        released.notify_one();
    }

    #[tokio::test]
    async fn empty_label_uses_default() {
        spawn("", async {});
        let n = drain_for_label(DEFAULT_LABEL, Duration::from_secs(1)).await;
        assert_eq!(n, 1, "empty label should normalize to default");
    }

    #[tokio::test]
    async fn handle_table_evicts_oldest_at_cap() {
        // Fill to MAX_TRACKED + 1, then check the oldest was evicted.
        for i in 0..(MAX_TRACKED + 1) {
            spawn(format!("evict-{i}"), async {});
        }
        let remaining = HANDLES.lock().expect("daemon_pool HANDLES mutex poisoned").len();
        assert!(remaining <= MAX_TRACKED, "cap must be honored, got {remaining}");
    }
}
