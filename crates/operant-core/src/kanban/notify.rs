use crate::error::Error;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifySubscription {
    pub task_id: String,
    pub platform: String,
    pub chat_id: String,
    pub thread_id: String,
    pub user_id: Option<String>,
    pub created_at: i64,
    pub last_event_id: i64,
}

pub struct NotifyManager {
    conn: Arc<Mutex<Connection>>,
}

impl NotifyManager {
    /// Lock the SQLite connection, converting mutex poisoning into a
    /// recoverable error instead of panicking (same pattern as database.rs).
    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("notify db mutex poisoned".to_string()))
    }

    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn subscribe(
        &self,
        task_id: &str,
        platform: &str,
        chat_id: &str,
        user_id: Option<&str>,
    ) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO kanban_notify_subs (task_id, platform, chat_id, thread_id, user_id, created_at, last_event_id)
             VALUES (?1, ?2, ?3, '', ?4, ?5, 0)",
            params![task_id, platform, chat_id, user_id, now],
        ).map_err(|e| Error::Agent(format!("Failed to subscribe: {}", e)))?;
        Ok(())
    }

    pub fn unsubscribe(&self, task_id: &str, platform: &str, chat_id: &str) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "DELETE FROM kanban_notify_subs WHERE task_id = ?1 AND platform = ?2 AND chat_id = ?3",
            params![task_id, platform, chat_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to unsubscribe: {}", e)))?;
        Ok(())
    }

    pub fn list_subscriptions(&self, task_id: &str) -> Result<Vec<NotifySubscription>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_id, platform, chat_id, thread_id, user_id, created_at, last_event_id FROM kanban_notify_subs WHERE task_id = ?1"
        ).map_err(|e| Error::Agent(format!("Failed to prepare: {}", e)))?;

        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(NotifySubscription {
                    task_id: row.get(0)?,
                    platform: row.get(1)?,
                    chat_id: row.get(2)?,
                    thread_id: row.get(3)?,
                    user_id: row.get(4)?,
                    created_at: row.get(5)?,
                    last_event_id: row.get(6)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut subs = Vec::new();
        for row in rows {
            subs.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(subs)
    }

    pub fn notify_subscribers(
        &self,
        task_id: &str,
        event_kind: &str,
        payload: &str,
    ) -> Result<(), Error> {
        let subs = self.list_subscriptions(task_id)?;
        if subs.is_empty() {
            return Ok(());
        }
        for sub in &subs {
            tracing::info!(
                "Notify {} on {} ({}) about '{}': {}",
                sub.task_id,
                sub.platform,
                sub.chat_id,
                event_kind,
                payload
            );
        }
        Ok(())
    }
}
