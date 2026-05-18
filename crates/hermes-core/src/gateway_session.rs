//! Persistent SQLite-backed session store for the Hermes gateway.
//!
//! Mirrors the in-memory [`SessionStore`](crate::gateway::SessionStore) API
//! but persists all sessions to a SQLite database file.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json;
use uuid::Uuid;

use crate::error::Error;
use crate::gateway::PlatformSession;

/// A SQLite-backed session store that mirrors the in-memory [`SessionStore`]
/// API but persists session data to disk.
pub struct PersistentSessionStore {
    conn: Arc<Mutex<Connection>>,
}

impl PersistentSessionStore {
    /// Open (or create) the session database at `db_path`.
    pub fn open(db_path: &str) -> Result<Self, Error> {
        let conn = Connection::open(db_path)
            .map_err(|e| Error::Agent(format!("Failed to open session DB: {}", e)))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS gateway_sessions (
                session_id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                platform_user_id TEXT NOT NULL,
                platform_channel_id TEXT NOT NULL,
                hermes_session_id TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                last_active TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_platform ON gateway_sessions(platform);
            CREATE INDEX IF NOT EXISTS idx_sessions_hermes ON gateway_sessions(hermes_session_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_lookup ON gateway_sessions(platform, platform_user_id, platform_channel_id);",
        )
        .map_err(|e| Error::Agent(format!("Failed to init session DB: {}", e)))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create a new session and return it.
    pub fn create_session(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
    ) -> Result<PlatformSession, Error> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let session = PlatformSession {
            session_id: Uuid::new_v4().to_string(),
            platform: platform.to_string(),
            platform_user_id: user_id.to_string(),
            platform_channel_id: channel_id.to_string(),
            hermes_session_id: String::new(),
            created_at: now.clone(),
            last_active: now,
            metadata: HashMap::new(),
        };
        conn.execute(
            "INSERT INTO gateway_sessions (session_id, platform, platform_user_id, platform_channel_id, hermes_session_id, created_at, last_active, metadata)
             VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, '{}')",
            params![
                session.session_id,
                session.platform,
                session.platform_user_id,
                session.platform_channel_id,
                session.created_at,
                session.last_active
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session: {}", e)))?;
        Ok(session)
    }

    /// Get a session by its ID.
    pub fn get_session(&self, session_id: &str) -> Option<PlatformSession> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT session_id, platform, platform_user_id, platform_channel_id, hermes_session_id, created_at, last_active, metadata
             FROM gateway_sessions WHERE session_id = ?1",
            params![session_id],
            |row| row_to_session(row),
        );
        result.ok()
    }

    /// Find a session matching platform + user + channel.
    pub fn find_session(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
    ) -> Option<PlatformSession> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT session_id, platform, platform_user_id, platform_channel_id, hermes_session_id, created_at, last_active, metadata
             FROM gateway_sessions WHERE platform = ?1 AND platform_user_id = ?2 AND platform_channel_id = ?3
             LIMIT 1",
            params![platform, user_id, channel_id],
            |row| row_to_session(row),
        );
        result.ok()
    }

    /// Update the `last_active` timestamp for a session.
    pub fn update_activity(&self, session_id: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute(
                "UPDATE gateway_sessions SET last_active = ?1 WHERE session_id = ?2",
                params![Utc::now().to_rfc3339(), session_id],
            )
            .map_err(|e| Error::Agent(format!("Failed to update activity: {}", e)))?;
        if affected == 0 {
            Err(Error::Agent(format!("Session not found: {}", session_id)))
        } else {
            Ok(())
        }
    }

    /// Remove a session.
    pub fn close_session(&self, session_id: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM gateway_sessions WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to close session: {}", e)))?;
        Ok(())
    }

    /// List all active sessions, optionally filtered by platform.
    pub fn list_active_sessions(&self, platform: Option<&str>) -> Vec<PlatformSession> {
        let conn = self.conn.lock().unwrap();
        if let Some(p) = platform {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, platform, platform_user_id, platform_channel_id, hermes_session_id, created_at, last_active, metadata
                     FROM gateway_sessions WHERE platform = ?1 ORDER BY last_active DESC",
                )
                .unwrap();
            stmt.query_map(params![p], |row| row_to_session(row))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, platform, platform_user_id, platform_channel_id, hermes_session_id, created_at, last_active, metadata
                     FROM gateway_sessions ORDER BY last_active DESC",
                )
                .unwrap();
            stmt.query_map([], |row| row_to_session(row))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }
    }

    /// Total number of sessions in the database.
    pub fn get_session_count(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM gateway_sessions", [], |row| {
            row.get(0)
        })
        .unwrap_or(0)
    }

    /// Find a session by its Hermes session ID.
    pub fn get_hermes_session(&self, hermes_session_id: &str) -> Option<PlatformSession> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT session_id, platform, platform_user_id, platform_channel_id, hermes_session_id, created_at, last_active, metadata
             FROM gateway_sessions WHERE hermes_session_id = ?1 LIMIT 1",
            params![hermes_session_id],
            |row| row_to_session(row),
        );
        result.ok()
    }

    /// Find or create a shared session keyed by channel_id only (for group chats).
    pub fn find_or_create_shared_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<PlatformSession, Error> {
        if let Some(s) = self.find_session(platform, "__shared__", channel_id) {
            let _ = self.update_activity(&s.session_id);
            return Ok(s);
        }
        self.create_session(platform, "__shared__", channel_id)
    }
}

// ---------------------------------------------------------------------------
// Helper: map a SQLite row to a PlatformSession
// ---------------------------------------------------------------------------

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlatformSession> {
    let metadata_str: String = row.get(7)?;
    let metadata: HashMap<String, String> = serde_json::from_str(&metadata_str).unwrap_or_default();
    Ok(PlatformSession {
        session_id: row.get(0)?,
        platform: row.get(1)?,
        platform_user_id: row.get(2)?,
        platform_channel_id: row.get(3)?,
        hermes_session_id: row.get(4)?,
        created_at: row.get(5)?,
        last_active: row.get(6)?,
        metadata,
    })
}
