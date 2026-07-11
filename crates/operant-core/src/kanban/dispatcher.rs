use crate::error::Error;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

/// Dispatcher engine for spawning and managing task workers.
pub struct Dispatcher {
    db: Arc<Mutex<Connection>>,
}

impl Dispatcher {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { db: conn }
    }

    pub fn pending_tasks(&self, limit: usize) -> Result<Vec<(String, String, Option<i32>)>, Error> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, title, max_runtime_seconds FROM tasks 
             WHERE status IN ('todo', 'ready') 
               AND (current_run_id IS NULL OR current_run_id = 0)
             ORDER BY priority DESC, created_at ASC 
             LIMIT ?1",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare: {}", e)))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(tasks)
    }

    pub fn claim_task(&self, task_id: &str, worker: &str) -> Result<i64, Error> {
        let conn = self.db.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "INSERT INTO task_runs (task_id, profile, status, claim_lock, started_at) 
             VALUES (?1, ?2, 'running', ?3, ?4)",
            params![task_id, "dispatcher", worker, now],
        )
        .map_err(|e| Error::Agent(format!("Failed to create run: {}", e)))?;
        let run_id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE tasks SET status = 'running', current_run_id = ?1, started_at = COALESCE(started_at, ?2) WHERE id = ?3",
            params![run_id, now, task_id],
        ).map_err(|e| Error::Agent(format!("Failed to claim task: {}", e)))?;
        conn.execute(
            "INSERT INTO task_events (task_id, run_id, kind, payload, created_at) VALUES (?1, ?2, 'dispatched', ?3, ?4)",
            params![task_id, run_id, serde_json::json!({"worker": worker}).to_string(), now],
        ).unwrap_or_else(|e| { warn!("Failed to insert dispatched event: {}", e); 0 });
        Ok(run_id)
    }

    pub fn complete_run(
        &self,
        task_id: &str,
        run_id: i64,
        outcome: &str,
        summary: Option<&str>,
    ) -> Result<(), Error> {
        let conn = self.db.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "UPDATE tasks SET status = 'done', completed_at = ?1, current_run_id = NULL WHERE id = ?2",
            params![now, task_id],
        ).map_err(|e| Error::Agent(format!("Failed to complete task: {}", e)))?;
        conn.execute(
            "UPDATE task_runs SET status = 'completed', ended_at = ?1, outcome = ?2, summary = ?3 WHERE id = ?4",
            params![now, outcome, summary, run_id],
        ).map_err(|e| Error::Agent(format!("Failed to update run: {}", e)))?;
        conn.execute(
            "INSERT INTO task_events (task_id, run_id, kind, payload, created_at) VALUES (?1, ?2, 'completed', ?3, ?4)",
            params![task_id, run_id, serde_json::json!({"outcome": outcome}).to_string(), now],
        ).unwrap_or_else(|e| { warn!("Failed to insert completed event: {}", e); 0 });
        Ok(())
    }

    pub fn fail_run(&self, task_id: &str, run_id: i64, error_msg: &str) -> Result<(), Error> {
        let conn = self.db.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "UPDATE tasks SET status = 'todo', current_run_id = NULL, consecutive_failures = consecutive_failures + 1, last_failure_error = ?1 WHERE id = ?2",
            params![error_msg, task_id],
        ).map_err(|e| Error::Agent(format!("Failed to fail task: {}", e)))?;
        conn.execute(
            "UPDATE task_runs SET status = 'failed', ended_at = ?1, error = ?2 WHERE id = ?3",
            params![now, error_msg, run_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to update run: {}", e)))?;
        conn.execute(
            "INSERT INTO task_events (task_id, run_id, kind, payload, created_at) VALUES (?1, ?2, 'failed', ?3, ?4)",
            params![task_id, run_id, serde_json::json!({"error": error_msg}).to_string(), now],
        ).unwrap_or_else(|e| { warn!("Failed to insert failed event: {}", e); 0 });
        Ok(())
    }

    pub fn gc(&self, older_than_days: i64) -> Result<(usize, usize, usize), Error> {
        let conn = self.db.lock().unwrap();
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - older_than_days * 86400;
        let removed_runs = conn.execute(
            "DELETE FROM task_runs WHERE task_id IN (SELECT id FROM tasks WHERE status = 'archived' AND completed_at < ?1)", params![cutoff],
        ).map_err(|e| Error::Agent(format!("GC runs: {}", e)))?;
        let removed_events = conn.execute(
            "DELETE FROM task_events WHERE task_id IN (SELECT id FROM tasks WHERE status = 'archived' AND completed_at < ?1)", params![cutoff],
        ).map_err(|e| Error::Agent(format!("GC events: {}", e)))?;
        let removed_tasks = conn
            .execute(
                "DELETE FROM tasks WHERE status = 'archived' AND completed_at < ?1",
                params![cutoff],
            )
            .map_err(|e| Error::Agent(format!("GC tasks: {}", e)))?;
        Ok((
            removed_tasks as usize,
            removed_runs as usize,
            removed_events as usize,
        ))
    }
}
