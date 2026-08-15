//! Async (background) delegation registry — hermes `tools/async_delegation.py`
//! parity.
//!
//! Backs `delegate_task(background=true)`: the parent agent dispatches a
//! subagent that runs on a spawned tokio task and returns a handle
//! immediately, so the user and the model can keep working while the child
//! runs. When the child finishes, the record transitions to
//! completed/failed and an `AgentEvent::AsyncDelegation` is pushed onto the
//! parent's event channel so the CLI/TUI can surface it. The agent polls
//! progress with `delegate_task(query="<id>")`.
//!
//! This module owns ONLY the async lifecycle + records; the actual child
//! build + run is delegated back to `SubAgentTool::call`, so depth limits,
//! tool inheritance, credential config, and result shaping stay in one
//! place (hermes: "the actual child build + run is delegated back to
//! delegate_tool._run_single_child").

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many concurrent background delegations are allowed (hermes
/// `_DEFAULT_MAX_ASYNC_CHILDREN = 3` parity).
pub const DEFAULT_MAX_ASYNC_CHILDREN: usize = 3;
/// How many terminal (completed/failed) records to retain before pruning.
const MAX_RETAINED_TERMINAL: usize = 50;

/// Lifecycle status of a background delegation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsyncDelegationStatus {
    /// Dispatched, child still running.
    Pending,
    /// Child finished and the result was recorded.
    Completed,
    /// Child errored or timed out.
    Failed,
}

/// A single background delegation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncDelegationRecord {
    /// Opaque handle returned to the parent at dispatch time.
    pub delegation_id: String,
    pub status: AsyncDelegationStatus,
    /// The task goal (used in status queries so the parent can re-orient).
    pub goal: String,
    /// Model the child ran on.
    pub model: String,
    /// Unix seconds at dispatch.
    pub dispatch_time: u64,
    pub completed_time: Option<u64>,
    /// Final child answer (Completed only).
    pub result: Option<String>,
    /// Error text (Failed only).
    pub error: Option<String>,
}

fn registry() -> &'static Mutex<HashMap<String, AsyncDelegationRecord>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, AsyncDelegationRecord>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("dlg-{}-{n}", now_secs())
}

fn insert_pending(
    map: &mut HashMap<String, AsyncDelegationRecord>,
    goal: &str,
    model: &str,
) -> String {
    let id = new_id();
    map.insert(
        id.clone(),
        AsyncDelegationRecord {
            delegation_id: id.clone(),
            status: AsyncDelegationStatus::Pending,
            goal: goal.to_string(),
            model: model.to_string(),
            dispatch_time: now_secs(),
            completed_time: None,
            result: None,
            error: None,
        },
    );
    id
}

/// Register a pending background delegation and return its handle id.
pub fn create_record(goal: &str, model: &str) -> String {
    if let Ok(mut map) = registry().lock() {
        insert_pending(&mut map, goal, model)
    } else {
        // Registry poisoned: fall back to an id that simply won't resolve.
        new_id()
    }
}

/// Atomically check the in-flight cap and register a new pending record
/// under ONE lock acquisition, so concurrent dispatches can't both pass a
/// check-then-act race (the count can never exceed `max_pending`).
/// Returns `Some(id)` when a slot was free, `None` when the cap is reached.
pub fn try_create_record(goal: &str, model: &str, max_pending: usize) -> Option<String> {
    let mut map = registry().lock().ok()?;
    let pending = map
        .values()
        .filter(|r| r.status == AsyncDelegationStatus::Pending)
        .count();
    if pending >= max_pending {
        return None;
    }
    Some(insert_pending(&mut map, goal, model))
}

/// Transition a record to completed and stash the child's final answer.
pub fn mark_completed(id: &str, result: &str) {
    if let Ok(mut map) = registry().lock()
        && let Some(rec) = map.get_mut(id)
    {
        rec.status = AsyncDelegationStatus::Completed;
        rec.completed_time = Some(now_secs());
        rec.result = Some(result.to_string());
    }
    prune();
}

/// Transition a record to failed and stash the error text.
pub fn mark_failed(id: &str, error: &str) {
    if let Ok(mut map) = registry().lock()
        && let Some(rec) = map.get_mut(id)
    {
        rec.status = AsyncDelegationStatus::Failed;
        rec.completed_time = Some(now_secs());
        rec.error = Some(error.to_string());
    }
    prune();
}

/// Fetch a record clone (for `delegate_task(query="<id>")` polls).
pub fn get_record(id: &str) -> Option<AsyncDelegationRecord> {
    registry().lock().ok()?.get(id).cloned()
}

/// Number of currently in-flight (pending) delegations — used to enforce
/// the concurrency cap at dispatch time.
pub fn pending_count() -> usize {
    registry()
        .lock()
        .map(|map| {
            map.values()
                .filter(|r| r.status == AsyncDelegationStatus::Pending)
                .count()
        })
        .unwrap_or(0)
}

/// All records, newest dispatch first.
pub fn list_records() -> Vec<AsyncDelegationRecord> {
    let mut records: Vec<AsyncDelegationRecord> = registry()
        .lock()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default();
    records.sort_by(|a, b| {
        b.dispatch_time
            .cmp(&a.dispatch_time)
            .then_with(|| b.delegation_id.cmp(&a.delegation_id))
    });
    records
}

/// Drop the oldest terminal records beyond the retention cap so a long-lived
/// process doesn't accumulate unbounded delegation history (hermes
/// `_MAX_RETAINED_COMPLETED = 50` parity).
fn prune() {
    if let Ok(mut map) = registry().lock() {
        let terminal: Vec<String> = map
            .iter()
            .filter(|(_, r)| r.status != AsyncDelegationStatus::Pending)
            .map(|(id, _)| id.clone())
            .collect();
        if terminal.len() > MAX_RETAINED_TERMINAL {
            let mut sorted: Vec<(&String, u64)> = terminal
                .iter()
                .filter_map(|id| map.get(id).map(|r| (id, r.dispatch_time)))
                .collect();
            sorted.sort_by_key(|(_, time)| *time);
            for (id, _) in sorted.iter().take(terminal.len() - MAX_RETAINED_TERMINAL) {
                map.remove(*id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share the process-global registry and run in parallel — any
    /// absolute-count assertion would race. Serialize them with a lock.
    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn create_then_complete_round_trip() {
        let _guard = test_lock().lock().unwrap();
        let id = create_record("analyze module", "gpt-4.1");
        let rec = get_record(&id).unwrap();
        assert_eq!(rec.status, AsyncDelegationStatus::Pending);
        assert_eq!(rec.goal, "analyze module");
        assert!(rec.result.is_none());

        mark_completed(&id, "{\"summary\":\"done\"}");
        let rec = get_record(&id).unwrap();
        assert_eq!(rec.status, AsyncDelegationStatus::Completed);
        assert_eq!(rec.result.as_deref(), Some("{\"summary\":\"done\"}"));
        assert!(rec.completed_time.is_some());
    }

    #[test]
    fn create_then_fail_records_error() {
        let _guard = test_lock().lock().unwrap();
        let id = create_record("risky task", "gpt-4.1");
        mark_failed(&id, "child timed out");
        let rec = get_record(&id).unwrap();
        assert_eq!(rec.status, AsyncDelegationStatus::Failed);
        assert_eq!(rec.error.as_deref(), Some("child timed out"));
        assert!(rec.result.is_none());
    }

    #[test]
    fn pending_count_reflects_in_flight() {
        let _guard = test_lock().lock().unwrap();
        let before = pending_count();
        let id = create_record("task", "gpt-4.1");
        assert!(pending_count() > before);
        mark_completed(&id, "ok");
        assert_eq!(pending_count(), before);
    }

    #[test]
    fn get_missing_record_returns_none() {
        let _guard = test_lock().lock().unwrap();
        assert!(get_record("dlg-does-not-exist").is_none());
    }

    #[test]
    fn records_list_is_sorted_newest_first() {
        let _guard = test_lock().lock().unwrap();
        let first = create_record("first", "m");
        let second = create_record("second", "m");
        let list = list_records();
        // Records created in the same second share dispatch_time; assert the
        // ordering invariant (newest first) + membership, not a strict pair
        // order that would be flaky on same-second creation.
        let times: Vec<u64> = list.iter().map(|r| r.dispatch_time).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]));
        assert!(list.iter().any(|r| r.delegation_id == first));
        assert!(list.iter().any(|r| r.delegation_id == second));
        // cleanup so other tests' count assertions stay isolated
        mark_completed(&first, "done");
        mark_completed(&second, "done");
    }
}
