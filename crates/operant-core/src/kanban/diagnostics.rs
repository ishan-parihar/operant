use crate::error::Error;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticIssue {
    pub severity: String, // "error", "warning", "info"
    pub category: String, // "stale", "zombie", "orphan", "config"
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub action: String, // human-readable recommendation
}

pub struct KanbanDiagnostics {
    conn: Arc<Mutex<Connection>>,
}

impl KanbanDiagnostics {
    /// Lock the SQLite connection, converting mutex poisoning into a
    /// recoverable error instead of panicking (same pattern as database.rs).
    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("diagnostics db mutex poisoned".to_string()))
    }

    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Run all diagnostic checks and return issues.
    pub fn run_checks(&self) -> Result<Vec<DiagnosticIssue>, Error> {
        let mut issues = Vec::new();
        issues.extend(self.check_stale_tasks()?);
        issues.extend(self.check_zombie_runs()?);
        issues.extend(self.check_orphan_links()?);
        Ok(issues)
    }

    /// Tasks in 'running' status with no heartbeat for >5 minutes
    fn check_stale_tasks(&self) -> Result<Vec<DiagnosticIssue>, Error> {
        let conn = self.lock_conn()?;
        let cutoff = chrono::Utc::now().timestamp() - 300; // 5 min
        let mut stmt = conn.prepare(
            "SELECT id, title, last_heartbeat_at, started_at FROM tasks WHERE status = 'running' AND (last_heartbeat_at IS NULL OR last_heartbeat_at < ?1)"
        ).map_err(|e| Error::Agent(format!("Failed to prepare: {}", e)))?;

        let issues: Vec<DiagnosticIssue> = stmt
            .query_map(params![cutoff], |row| {
                let id: String = row.get(0)?;
                let title: String = row.get(1)?;
                let hb: Option<i64> = row.get(2)?;
                let started: Option<i64> = row.get(3)?;
                // RT-429: the never-heartbeated case reported "-1s" -- a
                // NEGATIVE elapsed time for exactly the case the check
                // exists to flag. The elapsed time is the one since the last
                // heartbeat, or since the task STARTED when it never sent
                // one (the dispatcher stamps started_at when a task goes to
                // 'running'). When even that is unknown, the message says so
                // instead of inventing a number.
                let elapsed = hb
                    .or(started)
                    .map(|t| (chrono::Utc::now().timestamp() - t).max(0));
                let description = match elapsed {
                    Some(secs) => format!(
                        "Task '{}' has been in 'running' status for {}s without heartbeat",
                        id, secs
                    ),
                    None => format!(
                        "Task '{}' has been in 'running' status without heartbeat",
                        id
                    ),
                };
                Ok(DiagnosticIssue {
                    severity: "warning".into(),
                    category: "stale".into(),
                    task_id: id.clone(),
                    title: title.clone(),
                    description,
                    action: format!(
                        "Run `operant kanban reclaim {}` to reset to 'todo' status",
                        id
                    ),
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(issues)
    }

    /// Runs where status='running' but parent task is not running
    fn check_zombie_runs(&self) -> Result<Vec<DiagnosticIssue>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.task_id, t.title FROM task_runs r
             JOIN tasks t ON t.id = r.task_id
             WHERE r.status = 'running' AND t.status != 'running'",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare: {}", e)))?;

        let issues: Vec<DiagnosticIssue> = stmt
            .query_map([], |row| {
                let run_id: i64 = row.get(0)?;
                let task_id: String = row.get(1)?;
                let title: String = row.get(2)?;
                Ok(DiagnosticIssue {
                    severity: "error".into(),
                    category: "zombie".into(),
                    task_id: task_id.clone(),
                    title: title.clone(),
                    description: format!(
                        "Run {} is 'running' but task '{}' status is not 'running'",
                        run_id, task_id
                    ),
                    action: format!(
                        "Run `UPDATE task_runs SET status='orphaned' WHERE id={}`",
                        run_id
                    ),
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(issues)
    }

    /// Links pointing to non-existent tasks
    fn check_orphan_links(&self) -> Result<Vec<DiagnosticIssue>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT parent_id, child_id FROM task_links WHERE
             parent_id NOT IN (SELECT id FROM tasks) OR
             child_id NOT IN (SELECT id FROM tasks)",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare: {}", e)))?;

        let issues: Vec<DiagnosticIssue> = stmt
            .query_map([], |row| {
                let parent_id: String = row.get(0)?;
                let child_id: String = row.get(1)?;
                Ok(DiagnosticIssue {
                    severity: "warning".into(),
                    category: "orphan".into(),
                    task_id: parent_id.clone(),
                    title: "(orphaned link)".into(),
                    description: format!(
                        "Link between '{}' and '{}' references missing task",
                        parent_id, child_id
                    ),
                    action: format!(
                        "Run `operant kanban unlink {} {}` to remove",
                        parent_id, child_id
                    ),
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(issues)
    }
}

#[cfg(test)]
mod rt429_tests {
    use super::*;

    /// RT-429: the stale-task check reported "-1s" for a task that had NEVER
    /// heartbeated — a negative elapsed time for exactly the case the check
    /// exists to flag. BEHAVIORAL: the task is running, started ten minutes
    /// ago, and never heartbeated; the diagnostic must report the elapsed
    /// time since it started, and never a negative one.
    #[test]
    fn a_never_heartbeated_task_reports_a_real_elapsed_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = crate::kanban::db::KanbanDb::init(dir.path().join("operant_kanban.db"))
            .expect("kanban db");
        let started = chrono::Utc::now().timestamp() - 600;
        {
            let conn = db.conn().lock().expect("lock");
            conn.execute(
                "INSERT INTO tasks (id, title, status, created_at, started_at, workspace_kind) VALUES ('t-1', 'stuck task', 'running', ?1, ?1, 'scratch')",
                rusqlite::params![started],
            )
            .expect("insert a running task that never heartbeated");
        }

        let diagnostics = KanbanDiagnostics::new(db.conn().clone());
        let issues = diagnostics.run_checks().expect("diagnostics");
        let stale = issues
            .iter()
            .find(|i| i.task_id == "t-1")
            .expect("the never-heartbeated task must be flagged");
        assert!(
            !stale.description.contains("-1s"),
            "the diagnostic must never report a negative elapsed time: {}",
            stale.description
        );
        assert!(
            stale.description.contains("600s"),
            "the diagnostic must report the elapsed time since the task started: {}",
            stale.description
        );
    }
}
