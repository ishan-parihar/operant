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
        let conn = self.conn.lock().unwrap();
        let cutoff = chrono::Utc::now().timestamp() - 300; // 5 min
        let mut stmt = conn.prepare(
            "SELECT id, title, last_heartbeat_at FROM tasks WHERE status = 'running' AND (last_heartbeat_at IS NULL OR last_heartbeat_at < ?1)"
        ).map_err(|e| Error::Agent(format!("Failed to prepare: {}", e)))?;

        let issues: Vec<DiagnosticIssue> = stmt
            .query_map(params![cutoff], |row| {
                let id: String = row.get(0)?;
                let title: String = row.get(1)?;
                let hb: Option<i64> = row.get(2)?;
                let ago = hb.map(|t| chrono::Utc::now().timestamp() - t).unwrap_or(-1);
                Ok(DiagnosticIssue {
                    severity: "warning".into(),
                    category: "stale".into(),
                    task_id: id.clone(),
                    title: title.clone(),
                    description: format!(
                        "Task '{}' has been in 'running' status for {}s without heartbeat",
                        id, ago
                    ),
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
