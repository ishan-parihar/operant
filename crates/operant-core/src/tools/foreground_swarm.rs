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
/// process — the same unbounded-map defect the async registry bounds at
/// `async_delegation::MAX_RETAINED_TERMINAL`. Live rows are never pruned: a
/// running worker must stay visible.
const MAX_RETAINED_TERMINAL: usize = 50;

static LIVE: OnceLock<Mutex<Vec<Row>>> = OnceLock::new();

fn live() -> &'static Mutex<Vec<Row>> {
    LIVE.get_or_init(|| Mutex::new(Vec::new()))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
/// Ordered by `started_at`, with the id as a tiebreak so the survivor of a
/// same-second prune does not depend on `Vec` order — the same total-order rule
/// the async pruner follows.
fn prune() {
    if let Ok(mut rows) = live().lock() {
        let terminal: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.status != "running")
            .map(|(i, _)| i)
            .collect();
        if terminal.len() <= MAX_RETAINED_TERMINAL {
            return;
        }
        let mut ordered: Vec<(u64, &String, usize)> = terminal
            .iter()
            .map(|i| (rows[*i].started_at, &rows[*i].id, *i))
            .collect();
        ordered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let drop: std::collections::HashSet<usize> = ordered
            .into_iter()
            .take(terminal.len() - MAX_RETAINED_TERMINAL)
            .map(|(_, _, i)| i)
            .collect();
        let mut i = 0;
        rows.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
    }
}

/// Only the workers still running, newest last.
///
/// The per-frame path (`App::inline_swarm_members`) asks for live rows and used
/// to get every row this process had ever seen, cloned with its label and
/// detail — five times per prepare pass (audit finding 13). Filtering before the
/// clone is the whole fix: the caller throws the finished rows away anyway.
pub fn list_live() -> Vec<ForegroundMember> {
    let now = now_secs();
    match live().lock() {
        Ok(rows) => rows
            .iter()
            .filter(|r| r.status == "running")
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
    let now = now_secs();
    match live().lock() {
        Ok(rows) => rows
            .iter()
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

/// Drop every row. For tests, which share the process with the rest of the suite.
pub fn clear() {
    if let Ok(mut rows) = live().lock() {
        rows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test, not three: the registry is a process singleton, so parallel tests
    // would clear each other's rows (the same trap the async registry's tests hit).
    #[test]
    fn the_registry_follows_a_worker_from_start_to_finish() {
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
}
