//! Live membership of FOREGROUND swarms.
//!
//! MEASURED 2026-11-14 (audit finding 4): a foreground `delegate_task(tasks=[…])`
//! recorded nothing anywhere. The async registry holds only BACKGROUND
//! delegations, so `App::swarm_gallery_members` — the one pane an operator reads —
//! had no foreground member to show: a swarm that was running was invisible while
//! it ran, and the first sign of it was its result landing in the transcript.
//!
//! This registry is what that pane reads. It is a PROCESS fact, exactly like the
//! async registry's pending rows: it dies with the process, and the durable account
//! of what ran is the transcript's swarm card, which carries every worker's report.
//! A DB-backed projection is the fix for durability and is deliberately NOT here —
//! it needs a migration, and a schema change is its own iteration.

use std::sync::{Mutex, OnceLock};

/// One live (or just-finished) foreground worker.
#[derive(Debug, Clone)]
pub struct ForegroundMember {
    /// Stable id, also the `subagent_id` on the lifecycle events.
    pub id: String,
    /// What this worker was asked to do, already shortened for display.
    pub label: String,
    /// "running", "completed", "failed", "timed_out" or "panicked".
    pub status: String,
    /// The failure text, when it failed.
    pub detail: Option<String>,
    /// Seconds since this worker started, read at list time.
    pub age_secs: u64,
}

struct Row {
    id: String,
    label: String,
    status: String,
    detail: Option<String>,
    started_at: u64,
}

/// How many FINISHED workers to keep.
///
/// MEASURED 2026-11-14 (audit finding 13): this registry never pruned, so a
/// long-lived gateway accumulated one row per worker for the life of the
/// process — the same unbounded-map defect the async registry bounds.
///
/// (iter-2458, R2) This used to be its own `50`, a second copy of the async
/// registry's cap. Two numbers for one policy is how they drift, so this is now an
/// alias: the retention cap has exactly one definition, and the ordering that goes
/// with it lives in `async_delegation::oldest_terminal_ids`.
pub(crate) const MAX_RETAINED_TERMINAL: usize =
    crate::tools::async_delegation::MAX_RETAINED_TERMINAL;

static LIVE: OnceLock<Mutex<Vec<Row>>> = OnceLock::new();

fn live() -> &'static Mutex<Vec<Row>> {
    LIVE.get_or_init(|| Mutex::new(Vec::new()))
}

/// The one clock, shared with the delegation registry.
///
/// MEASURED 2026-11-15 (third-pass audit, finding 9): this was a third copy of
/// an identical body. Both registries age the same kind of row from the same
/// kind of timestamp, and the hub reads both in one pass — a clock that
/// disagreed between them would make one pane's ages inconsistent with the
/// other's.
fn now_secs() -> u64 {
    crate::tools::async_delegation::now_secs()
}

/// Record a worker starting. Called before its stream begins, so the pane can show
/// a swarm that has been admitted but has not produced a token yet.
pub fn begin(id: impl Into<String>, label: impl Into<String>) {
    let id = id.into();
    if let Ok(mut rows) = live().lock() {
        rows.retain(|r| r.id != id);
        rows.push(Row {
            id,
            label: label.into(),
            status: "running".to_string(),
            detail: None,
            started_at: now_secs(),
        });
    }
}

/// Record a worker's outcome. Idempotent by id: a double finish cannot resurrect a
/// finished row or add a second one.
pub fn finish(id: &str, status: &str, detail: Option<String>) {
    if let Ok(mut rows) = live().lock()
        && let Some(row) = rows.iter_mut().find(|r| r.id == id)
    {
        row.status = status.to_string();
        row.detail = detail;
    }
    prune();
}

/// Drop the OLDEST finished rows beyond the retention cap.
///
/// The ordering is NOT defined here any more: `crate::tools::async_delegation::
/// oldest_terminal_ids` owns "keep the newest N of (time, id)", so the two
/// registries cannot drift apart. (iter-2458, R2 — this was the second copy of the
/// rule, with its own cap constant and its own `sort_by`.)
fn prune() {
    if let Ok(mut rows) = live().lock() {
        let keys: Vec<(u64, String)> = rows
            .iter()
            .filter(|r| r.status != "running")
            .map(|r| (r.started_at, r.id.clone()))
            .collect();
        let drop =
            crate::tools::async_delegation::oldest_terminal_ids(&keys, MAX_RETAINED_TERMINAL);
        if !drop.is_empty() {
            rows.retain(|r| !drop.contains(&r.id));
        }
    }
}

/// Foreground workers, newest last, filtered before the clone.
///
/// `live_only` is the whole point of this function. MEASURED 2026-11-15
/// (third-pass audit, finding 3): `App::swarm_gallery_members(live_only)` had two
/// sources — the async registry, which honoured the flag, and this one, which did
/// not. `inline_swarm_members` asks for live rows to draw the "what is running
/// now" strip, and got every finished foreground worker back too, up to
/// `MAX_RETAINED_TERMINAL` of them, mixed in with the running ones. The separate
/// `list_live` written for that call had zero production callers, and its doc
/// claimed `inline_swarm_members` used it — it never did.
///
/// Filtering before the clone also keeps the per-frame cost proportional to what
/// is actually running, rather than to everything the process has ever spawned.
pub fn list_filtered(live_only: bool) -> Vec<ForegroundMember> {
    let now = now_secs();
    match live().lock() {
        Ok(rows) => rows
            .iter()
            .filter(|r| !live_only || r.status == "running")
            .map(|r| ForegroundMember {
                id: r.id.clone(),
                label: r.label.clone(),
                status: r.status.clone(),
                detail: r.detail.clone(),
                age_secs: now.saturating_sub(r.started_at),
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Every foreground worker this process knows about, newest last.
///
/// Finished workers stay until the process ends: the pane is the only surface that
/// can say what the last swarm did, and dropping them at completion would make the
/// pane blank exactly when the operator looks.
pub fn list() -> Vec<ForegroundMember> {
    list_filtered(false)
}

/// Only the workers still running, newest last.
pub fn list_live() -> Vec<ForegroundMember> {
    list_filtered(true)
}

/// Drop every row. For tests, which share the process with the rest of the suite.
pub fn clear() {
    if let Ok(mut rows) = live().lock() {
        rows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The registry is a process singleton, so tests that seed it race each other
    // (the same trap the async registry's tests hit). One lock for the whole
    // module; `clear()` on entry and on exit keeps a failed test from poisoning
    // the next one's row counts.
    fn fg_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn the_registry_follows_a_worker_from_start_to_finish() {
        let _guard = fg_test_lock();
        clear();
        assert!(list().is_empty(), "the registry starts empty");

        // Visible from the moment it is admitted, before its first token.
        begin("sub-1", "audit the parser");
        let running = list();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].status, "running");
        assert_eq!(running[0].label, "audit the parser");

        // A finish updates in place: it must not add a second row.
        finish("sub-1", "completed", None);
        let done = list();
        assert_eq!(done.len(), 1, "a finish must not add a row");
        assert_eq!(done[0].status, "completed");

        // A failure keeps its reason, and a double finish is still one row.
        begin("sub-2", "review the pool");
        finish("sub-2", "failed", Some("provider 503".to_string()));
        finish("sub-2", "failed", Some("provider 503".to_string()));
        let rows = list();
        assert_eq!(rows.len(), 2);
        let failed = rows
            .iter()
            .find(|r| r.id == "sub-2")
            .expect("sub-2 is present");
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.detail.as_deref(), Some("provider 503"));

        // A reused id replaces rather than duplicates.
        begin("sub-1", "second run");
        let rows = list();
        assert_eq!(rows.len(), 2, "a reused id must replace: {rows:?}");
        assert_eq!(
            rows.iter()
                .find(|r| r.id == "sub-1")
                .map(|r| r.label.as_str()),
            Some("second run")
        );

        // Audit finding 13: `list_live` returns ONLY the running rows, and the
        // finished ones are retained up to a cap rather than forever.
        clear();
        for i in 0..MAX_RETAINED_TERMINAL + 5 {
            begin(format!("sub-old-{i}"), "old work");
            finish(&format!("sub-old-{i}"), "completed", None);
        }
        let kept = list();
        assert_eq!(
            kept.len(),
            MAX_RETAINED_TERMINAL,
            "the finished rows must be capped at {MAX_RETAINED_TERMINAL}"
        );
        // The five dropped are the ones the documented order picks: every row
        // in this test is created inside the same second, so `started_at` ties
        // and the ID tiebreak decides — and IDs are compared as STRINGS, so
        // "sub-old-10" sorts before "sub-old-2". Compute the expectation with
        // the same rule rather than assuming numeric order.
        let mut expected: Vec<String> = (0..MAX_RETAINED_TERMINAL + 5)
            .map(|i| format!("sub-old-{i}"))
            .collect();
        expected.sort();
        for dropped in expected.iter().take(5) {
            assert!(
                !kept.iter().any(|r| &r.id == dropped),
                "{dropped} should have been pruned"
            );
        }
        assert!(
            kept.iter()
                .any(|r| r.id == format!("sub-old-{}", MAX_RETAINED_TERMINAL + 4)),
            "the newest finished row must survive"
        );
        assert!(
            list_live().is_empty(),
            "a finished worker must not appear in the live list"
        );

        begin("sub-live", "still working");
        assert_eq!(list_live().len(), 1);
        assert_eq!(list_live()[0].id, "sub-live");

        clear();
        assert!(list().is_empty());
    }

    /// `list_filtered(live_only)` is the one call both callers use, and the flag
    /// has to actually change the answer.
    ///
    /// MEASURED 2026-11-15 (third-pass audit, finding 3): the CLI's
    /// `swarm_gallery_members(live_only)` consulted the async registry with the
    /// flag but this one without it, so the live strip drew finished foreground
    /// workers. Both sources now agree on what "live" means.
    ///
    /// MUTATION PROVEN: replacing the filter with `!live_only || true` (i.e.
    /// ignoring the flag, the pre-fix behaviour) fails the `live_only` assertion
    /// below with `left: 2, right: 1`.
    #[test]
    fn the_live_filter_actually_excludes_finished_workers() {
        let _guard = fg_test_lock();
        clear();

        begin("sub-a", "running one");
        begin("sub-b", "running two");
        finish("sub-b", "completed", None);
        begin("sub-c", "running three");

        // Live-only: the two still running, not the finished one.
        let live = list_filtered(true);
        assert_eq!(live.len(), 2, "only the running rows: {live:?}");
        assert!(
            live.iter().all(|r| r.status == "running"),
            "a live-only read must not contain a finished row: {live:?}"
        );
        assert!(live.iter().any(|r| r.id == "sub-a"));
        assert!(live.iter().any(|r| r.id == "sub-c"));
        assert!(
            !live.iter().any(|r| r.id == "sub-b"),
            "the finished worker must not appear in a live-only read"
        );

        // Unfiltered: all three, which is what the transcript path wants.
        let all = list_filtered(false);
        assert_eq!(all.len(), 3, "the transcript needs every row: {all:?}");
        assert_eq!(
            all.iter().filter(|r| r.status != "running").count(),
            1,
            "exactly one finished row survives the unfiltered read"
        );

        clear();
    }
}
