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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many concurrent background delegations are allowed (hermes
/// `_DEFAULT_MAX_ASYNC_CHILDREN = 3` parity).
pub const DEFAULT_MAX_ASYNC_CHILDREN: usize = 3;
/// How many terminal (completed/failed) records to retain before pruning.
pub const MAX_RETAINED_TERMINAL: usize = 50;

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
    /// The session the child actually ran in.
    ///
    /// MEASURED 2026-11-14 (audit finding 10): a delegated child mints its own
    /// `sess_<uuid>` and nothing joined it to this record, so the operator could
    /// see that a delegation had run and could not open what it did. `serde(default)`
    /// so a ledger written before this field still loads.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The session that dispatched this delegation — the parent edge.
    ///
    /// MEASURED 2026-11-14 (audit finding 2): the record carried the child's
    /// session but not the parent's, so `records_snapshot` set
    /// `parent_id: None` for every subagent row and the delegation tree could
    /// not be rebuilt. Both references carry this edge as primary structure —
    /// jcode walks `report_back_to_session_id` for ancestry, codex writes a
    /// graph edge at spawn. Without it the hub shows a flat list in which a
    /// worker is indistinguishable from a top-level session, and nothing can
    /// answer "what did this session spawn?".
    ///
    /// `serde(default)` for the same reason as `session_id`: a ledger written
    /// before the edge existed still loads.
    #[serde(default)]
    pub parent_session_id: Option<String>,
}

fn registry() -> &'static Mutex<HashMap<String, AsyncDelegationRecord>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, AsyncDelegationRecord>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Where the ledger persists. `None` until a process opts in.
///
/// MEASURED 2026-11-14 (audit finding 3): the registry is a process-local
/// `OnceLock<Mutex<HashMap>>` and a restart forgot every delegation. A record
/// that was `Pending` when the process died then stayed `Pending` for the life
/// of the new one — the hub's `Working` count and the background-task widget
/// kept counting work whose child had died with the old process, and no surface
/// could tell the difference. Persistence is opt-in so a test process (and any
/// embedding that does not want a sidecar) never touches the operator's disk.
static LEDGER_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Opt in to a durable ledger at `path`, and recover whatever the previous
/// process left there. Returns true only for the process that WON the path, so
/// a second agent in the same process cannot re-run recovery over live records.
pub fn enable_persistence(path: PathBuf) -> bool {
    if LEDGER_PATH.set(path).is_err() {
        return false;
    }
    recover_interrupted();
    true
}

/// Rewrite the ledger from the in-memory registry. Best-effort: a failure must
/// never block a delegation, the same rule the approval allowlist follows.
fn persist() {
    let Some(path) = LEDGER_PATH.get() else {
        return;
    };
    let Ok(json) = serde_json::to_string_pretty(&list_records()) else {
        return;
    };
    let _ = crate::fs_secrets::write_secret_file(path, json.as_bytes());
}

/// Adopt the previous process's ledger and settle whatever it left running.
///
/// A record that was `Pending` is a record whose child was a tokio task in a
/// process that no longer exists. Keeping it `Pending` is the lie; the honest
/// transition is `Failed` with the reason, so the row stops claiming work is in
/// progress and the parent can re-dispatch if it still wants the work.
///
/// Returns how many records were settled, so a caller can say so once.
pub fn recover_interrupted() -> usize {
    let Some(path) = LEDGER_PATH.get() else {
        return 0;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0; // no ledger, or unreadable: nothing to recover
    };
    let Ok(records) = serde_json::from_str::<Vec<AsyncDelegationRecord>>(&text) else {
        return 0; // a corrupt ledger must not stop a process from starting
    };
    let mut recovered: Vec<AsyncDelegationRecord> = records
        .into_iter()
        .map(|mut rec| {
            if rec.status == AsyncDelegationStatus::Pending {
                rec.status = AsyncDelegationStatus::Failed;
                rec.completed_time = Some(now_secs());
                rec.error = Some(
                    "the process restarted while this delegation was running; its child died \
                     with that process, so the work must be re-dispatched if it is still wanted"
                        .to_string(),
                );
            }
            rec
        })
        .collect();
    // Retention still applies across a restart: a ledger that only grows is the
    // same unbounded-map defect persistence would otherwise introduce.
    if recovered.len() > MAX_RETAINED_TERMINAL {
        recovered.sort_by_key(|r| std::cmp::Reverse(r.completed_time.unwrap_or(r.dispatch_time)));
        recovered.truncate(MAX_RETAINED_TERMINAL);
    }
    if let Ok(mut map) = registry().lock() {
        *map = recovered
            .into_iter()
            .map(|rec| (rec.delegation_id.clone(), rec))
            .collect();
    }
    persist();
    LEDGER_PATH.get().map_or(0, |_| count_recovered())
}

/// How many recovered records this process settled as interrupted.
///
/// Read back out of the registry rather than counted during the map, because the
/// status is what the caller asked about and the registry is the only truth.
fn count_recovered() -> usize {
    registry()
        .lock()
        .map(|map| {
            map.values()
                .filter(|r| {
                    r.status == AsyncDelegationStatus::Failed
                        && r.error
                            .as_deref()
                            .is_some_and(|e| e.starts_with("the process restarted"))
                })
                .count()
        })
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// RAII drain of the process-global delegation registry.
///
/// The registry has no clear in production by design (a running gateway's
/// delegations must survive), but a TEST that seeds it leaves records behind
/// for the rest of the process. This guard empties the registry when it drops,
/// which is what stops one test's seeded subagents from rendering into another
/// test's frame. See `__test_reset_registry` for the RT-207 incident.
///
/// Construction does NOT clear — it clears on DROP — so the seeding test still
/// sees its own records while it runs.
pub struct RegistryDrain;

impl Default for RegistryDrain {
    fn default() -> Self {
        Self
    }
}

impl RegistryDrain {
    pub fn new() -> Self {
        Self
    }
}

impl Drop for RegistryDrain {
    fn drop(&mut self) {
        __test_reset_registry();
    }
}

/// Empty the process-global registry. TEST-ONLY on purpose.
///
/// The registry is `OnceLock<Mutex<HashMap>>` with no clear in production, and
/// tests that seed it leave records behind for the rest of the process. That is
/// not a hypothetical: RT-207 was a real red tip in `operant-cli` where
/// `gateway_commands::subagents_command_tests`
/// (`a_completed_delegation_shows_its_result_and_a_failed_one_its_error` seeds
/// two records via `mark_completed`/`mark_failed`) poisoned the TUI's
/// `highlight_covers_only_the_text_a_copy_carries` and
/// `live_tails_render_once_while_a_tool_is_reserved`, which both panic on
/// `Option::unwrap()` after failing to find their own text in a frame polluted
/// by the leaked subagent rows.
///
/// Mutation-proven: with the registry cleared before the reader, the minimal
/// repro (that one seeding test + the reader) goes GREEN, and the same pair is
/// RED without the clear.
///
/// `#[cfg(test)]` is NOT enough here — the polluting tests live in the
/// `operant-cli` binary's test build, so this must be visible to a binary test
/// target. Gated instead by name and by there being no production caller.
#[doc(hidden)]
pub fn __test_reset_registry() {
    if let Ok(mut g) = registry().lock() {
        g.clear();
    }
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
    parent_session_id: Option<&str>,
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
            session_id: None,
            parent_session_id: parent_session_id.map(str::to_string),
        },
    );
    id
}

/// Register a pending background delegation and return its handle id.
///
/// No parent edge: a caller that knows which session dispatched the work wants
/// [`try_create_record`], which carries it.
pub fn create_record(goal: &str, model: &str) -> String {
    if let Ok(mut map) = registry().lock() {
        insert_pending(&mut map, goal, model, None)
    } else {
        // Registry poisoned: fall back to an id that simply won't resolve.
        new_id()
    }
}

/// Atomically check the in-flight cap and register a new pending record
/// under ONE lock acquisition, so concurrent dispatches can't both pass a
/// check-then-act race (the count can never exceed `max_pending`).
/// Returns `Some(id)` when a slot was free, `None` when the cap is reached.
///
/// `parent_session_id` is the dispatching session — the edge the hub needs to
/// place a worker under its parent rather than among top-level sessions
/// (audit finding 2). `None` when the caller genuinely has no session.
pub fn try_create_record(
    goal: &str,
    model: &str,
    max_pending: usize,
    parent_session_id: Option<&str>,
) -> Option<String> {
    let mut map = registry().lock().ok()?;
    let pending = map
        .values()
        .filter(|r| r.status == AsyncDelegationStatus::Pending)
        .count();
    if pending >= max_pending {
        return None;
    }
    Some(insert_pending(
        &mut map,
        goal,
        model,
        parent_session_id,
    ))
}

/// Join a delegation to the session its child ran in.
///
/// Written before the terminal transition, and separately from it, because the
/// two facts arrive from different places: the id is on the child's answer, the
/// outcome is the task's. Idempotent, and a no-op for an unknown id.
pub fn mark_session(id: &str, session_id: &str) {
    if let Ok(mut map) = registry().lock()
        && let Some(rec) = map.get_mut(id)
    {
        rec.session_id = Some(session_id.to_string());
    }
    persist();
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
    persist();
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
    persist();
}

/// Fetch a record clone (for `delegate_task(query="<id>")` polls).
pub fn get_record(id: &str) -> Option<AsyncDelegationRecord> {
    registry().lock().ok()?.get(id).cloned()
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

/// Only the records still pending, newest dispatch first.
///
/// The per-frame callers (`App::swarm_gallery_members(live_only)`, the hub's
/// `Working` count) ask for live rows and used to clone EVERY record first —
/// including each finished child's full answer — then throw the finished ones
/// away (audit finding 13). Filtering before the clone is the whole fix.
pub fn live_records() -> Vec<AsyncDelegationRecord> {
    let mut records: Vec<AsyncDelegationRecord> = registry()
        .lock()
        .map(|map| {
            map.values()
                .filter(|r| r.status == AsyncDelegationStatus::Pending)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    records.sort_by(|a, b| {
        b.dispatch_time
            .cmp(&a.dispatch_time)
            .then_with(|| b.delegation_id.cmp(&a.delegation_id))
    });
    records
}

/// The registry as `SessionEntryV2` subagent rows — the unified listing's
/// join point for live delegations (plan 2026-10-28 iter-C). Pending rows
/// first (the ones an operator would act on), then terminal records within
/// the registry's own retention window.
pub fn records_snapshot() -> Vec<crate::session::listing::SessionEntryV2> {
    use crate::session::listing::{RowKind, RowStatus, SessionEntryV2};

    list_records()
        .into_iter()
        .map(|r| {
            let (status, updated_at) = match r.status {
                AsyncDelegationStatus::Pending => (
                    "pending",
                    chrono::DateTime::from_timestamp(r.dispatch_time as i64, 0)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now),
                ),
                AsyncDelegationStatus::Completed => (
                    "completed",
                    chrono::DateTime::from_timestamp(
                        r.completed_time.unwrap_or(r.dispatch_time) as i64,
                        0,
                    )
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                ),
                AsyncDelegationStatus::Failed => (
                    "failed",
                    chrono::DateTime::from_timestamp(
                        r.completed_time.unwrap_or(r.dispatch_time) as i64,
                        0,
                    )
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                ),
            };
            SessionEntryV2 {
                id: r.delegation_id,
                // Audit finding 2: the parent edge. This was `None` for every
                // row, so a subagent could not be placed under the session that
                // spawned it and the tree could not be rebuilt.
                parent_id: r.parent_session_id,
                title: None,
                updated_at,
                // A delegation's runtime is dispatch -> completion. The registry
                // is process-local and prunes with its worker, so these are the
                // live values rather than persisted ones.
                started_at: Some(
                    chrono::DateTime::from_timestamp(r.dispatch_time as i64, 0)
                        .unwrap_or_else(chrono::Utc::now),
                ),
                ended_at: r.completed_time.map(|t| {
                    chrono::DateTime::from_timestamp(t as i64, 0).unwrap_or_else(chrono::Utc::now)
                }),
                employee_id: None,
                source: "subagent".to_string(),
                message_count: 0,
                cost_usd: 0.0,
                tokens: 0,
                cwd: None,
                // Same-process presence: a pending delegation IS live work.
                status: if r.status == AsyncDelegationStatus::Pending {
                    RowStatus::Live
                } else {
                    RowStatus::Idle
                },
                kind: RowKind::Subagent {
                    goal: r.goal,
                    model: r.model,
                    dispatch_time: r.dispatch_time,
                    status,
                },
            }
        })
        .collect()
}

/// Drop the oldest terminal records beyond the retention cap so a long-lived
/// process doesn't accumulate unbounded delegation history (hermes
/// `_MAX_RETAINED_COMPLETED = 50` parity).
///
/// Ordered by `completed_time`, with the id as tiebreak. `dispatch_time` has
/// one-second resolution and several delegations routinely finish inside the same
/// second, so a dispatch-time sort made the SURVIVOR of a prune nondeterministic —
/// which handle stayed resolvable depended on HashMap iteration order. MEASURED
/// 2026-11-14.
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
                .filter_map(|id| {
                    map.get(id)
                        .map(|r| (id, r.completed_time.unwrap_or(r.dispatch_time)))
                })
                .collect();
            sorted.sort_by_key(|(id, time)| (*time, *id));
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
    fn get_missing_record_returns_none() {
        let _guard = test_lock().lock().unwrap();
        assert!(get_record("dlg-does-not-exist").is_none());
    }

    /// Pruning must keep exactly the newest records by the pruner's own key.
    ///
    /// The key is `(completed_time, id)` — a total order, so the result is
    /// deterministic by construction (MEASURED 2026-11-14: the old sort keyed on
    /// `dispatch_time` alone, and `completed_time` was not consulted at all, so
    /// the survivor of a same-second prune depended on HashMap iteration order).
    #[test]
    fn prune_keeps_the_newest_by_completion_order() {
        let _guard = test_lock().lock().unwrap();
        // Start from an empty registry: records left by other tests would take
        // slots under the cap and change which of OURS survive.
        __test_reset_registry();
        let mut all: Vec<(u64, String)> = Vec::new();
        for i in 0..MAX_RETAINED_TERMINAL + 5 {
            let id = create_record(&format!("job {i}"), "m");
            mark_completed(&id, "ok");
            let finished = get_record(&id).and_then(|r| r.completed_time).unwrap_or(0);
            all.push((finished, id));
        }
        all.sort();

        let mut expected: Vec<String> = all
            .iter()
            .skip(all.len() - MAX_RETAINED_TERMINAL)
            .map(|(_, id)| id.clone())
            .collect();
        expected.sort();
        let mut survivors: Vec<String> = list_records()
            .into_iter()
            .map(|r| r.delegation_id)
            .collect();
        survivors.sort();
        assert_eq!(
            survivors, expected,
            "the newest {MAX_RETAINED_TERMINAL} must survive"
        );
        __test_reset_registry();
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

    /// Audit finding 3: a restart used to forget every delegation, and a record
    /// that was `Pending` when the process died then read as running forever.
    /// Audit finding 10: the row must name the session that ran it, and the
    /// terminal transition must not drop the join.
    #[test]
    fn a_delegation_records_the_session_its_child_ran_in() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let id = create_record("audit the pool", "m");
        assert!(
            get_record(&id).unwrap().session_id.is_none(),
            "a fresh record has no session yet"
        );
        mark_session(&id, "sess_42");
        assert_eq!(
            get_record(&id).unwrap().session_id.as_deref(),
            Some("sess_42")
        );
        mark_completed(&id, "done");
        assert_eq!(
            get_record(&id).unwrap().session_id.as_deref(),
            Some("sess_42"),
            "completing must not drop the join"
        );
        __test_reset_registry();
    }

    /// The field is additive: a ledger written before the join existed must
    /// still load, or the first start after an upgrade would silently discard
    /// every delegation the operator had.
    #[test]
    fn a_ledger_written_before_the_session_join_still_loads() {
        let old = r#"{"delegation_id":"d1","status":"completed","goal":"g","model":"m",
            "dispatch_time":1,"completed_time":2,"result":"r","error":null}"#;
        let rec: AsyncDelegationRecord =
            serde_json::from_str(old).expect("a pre-join ledger must load");
        assert!(rec.session_id.is_none());
        // Audit finding 2: the parent edge is additive too. A ledger written
        // before the edge existed must load, or the first start after this
        // upgrade would discard every delegation the operator had.
        assert!(rec.parent_session_id.is_none());
    }

    /// Audit finding 2: a delegation records which session dispatched it, and
    /// the session-hub row carries it as the parent edge.
    #[test]
    fn a_delegation_records_the_session_that_dispatched_it() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let id = try_create_record("audit the pool", "m", 3, Some("sess_parent42"))
            .expect("a free slot");
        assert_eq!(
            get_record(&id).unwrap().parent_session_id.as_deref(),
            Some("sess_parent42"),
            "the record must name the dispatching session"
        );

        // The hub row is where the operator sees it: without this the row was
        // flat and a worker was indistinguishable from a top-level session.
        let row = records_snapshot()
            .into_iter()
            .find(|r| r.id == id)
            .expect("the delegation has a hub row");
        assert_eq!(
            row.parent_id.as_deref(),
            Some("sess_parent42"),
            "the hub row must carry the parent edge"
        );

        // A caller with no session still works, and says so rather than
        // inventing one.
        let orphan = create_record("no session", "m");
        assert!(get_record(&orphan).unwrap().parent_session_id.is_none());
        __test_reset_registry();
    }

    #[test]
    fn a_ledger_left_by_a_dead_process_settles_its_pending_records() {
        let _guard = test_lock().lock().unwrap();
        let dir = std::env::temp_dir().join(format!("deleg-ledger-{}", std::process::id()));
        let path = dir.join("delegations.json");
        std::fs::create_dir_all(&dir).unwrap();
        let pending = AsyncDelegationRecord {
            delegation_id: "dlg-old-1".to_string(),
            status: AsyncDelegationStatus::Pending,
            goal: "work that died with the process".to_string(),
            model: "m".to_string(),
            dispatch_time: 100,
            completed_time: None,
            result: None,
            error: None,
            session_id: None,
            parent_session_id: Some("dlg-parent-1".to_string()),
        };
        let done = AsyncDelegationRecord {
            delegation_id: "dlg-old-2".to_string(),
            status: AsyncDelegationStatus::Completed,
            goal: "work that finished".to_string(),
            model: "m".to_string(),
            dispatch_time: 100,
            completed_time: Some(110),
            result: Some("ok".to_string()),
            error: None,
            session_id: None,
            parent_session_id: Some("dlg-parent-1".to_string()),
        };
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&[pending, done]).unwrap(),
        )
        .unwrap();

        assert!(enable_persistence(path.clone()));

        let recovered = get_record("dlg-old-1").expect("the pending record is recovered");
        assert_eq!(recovered.status, AsyncDelegationStatus::Failed);
        assert!(
            recovered
                .error
                .as_deref()
                .is_some_and(|e| e.starts_with("the process restarted")),
            "the reason must name the restart: {:?}",
            recovered.error
        );
        assert!(
            recovered.completed_time.is_some(),
            "a settled row must stop claiming it is still running"
        );
        // A finished record is history, not a casualty: it survives untouched.
        assert_eq!(
            get_record("dlg-old-2").unwrap().status,
            AsyncDelegationStatus::Completed
        );

        // Durable, not just in memory: the ledger on disk now says Failed, so a
        // THIRD process would not resurrect the pending row.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("dlg-old-1"),
            "ledger was not rewritten: {text}"
        );
        assert!(
            !text.contains("\"status\": \"pending\""),
            "the settled ledger must not still claim pending"
        );
        // The process-global path stays set for the rest of the test binary;
        // deleting the directory under it would only make later best-effort
        // writes fail, so the per-pid temp dir is left for the OS to reap.
    }
}
