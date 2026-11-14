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

        clear();
        assert!(list().is_empty());
    }
}
