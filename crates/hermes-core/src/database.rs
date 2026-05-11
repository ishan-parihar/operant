//! Database persistence layer for Hermes-RS
//!
//! Handles session storage, message history, and checkpoint metadata using SQLite.
//! Uses rusqlite with bundled SQLite for simplicity and portability.

use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

use crate::error::{Error, Result};

/// Database manager for persistent storage.
/// Thread-safe via Arc<Mutex<Connection>> pattern.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Initialize a new database at the specified path.
    /// Creates parent directories and runs migrations automatically.
    pub fn init(path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Agent(format!("Failed to create database directory: {}", e))
            })?;
        }

        info!("Initializing database at {:?}", path);
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("Failed to open database: {}", e)))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.run_migrations()?;
        Ok(db)
    }

    /// Create database from the runtime config path.
    pub fn from_config() -> Result<Self> {
        let path = crate::config::runtime_config().database_path.clone();
        Self::init(path)
    }

    /// Run all database migrations.
    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Enable foreign keys
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| Error::Agent(format!("Failed to enable foreign keys: {}", e)))?;

        // Run each migration in order
        self.create_sessions_table(&conn)?;
        self.create_messages_table(&conn)?;
        self.create_checkpoints_table(&conn)?;
        self.create_fts_index(&conn)?;

        debug!("Database migrations completed successfully");
        Ok(())
    }

    fn create_sessions_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                source TEXT NOT NULL DEFAULT 'local',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create sessions table: {}", e)))?;

        // Index for recent sessions query
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC)",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create sessions index: {}", e)))?;

        Ok(())
    }

    fn create_messages_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create messages table: {}", e)))?;

        // Index for session message retrieval
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp)",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create messages index: {}", e)))?;

        Ok(())
    }

    fn create_checkpoints_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS checkpoints (
                hash TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                reason TEXT,
                directory TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create checkpoints table: {}", e)))?;

        // Index for checkpoint lookup by directory
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_dir ON checkpoints(directory, timestamp DESC)",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create checkpoints index: {}", e)))?;

        Ok(())
    }

    fn create_fts_index(&self, conn: &Connection) -> Result<()> {
        // Check if FTS table already exists
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
                [],
                |row| row.get(0),
            )
            .map(|count: i32| count > 0)
            .unwrap_or(false);

        if exists {
            return Ok(());
        }

        // Create FTS5 virtual table for full-text search
        conn.execute(
            "CREATE VIRTUAL TABLE messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='id'
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create FTS table: {}", e)))?;

        // Create triggers to keep FTS index in sync
        conn.execute_batch(
            r#"
            CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
            END;
            "#,
        )
        .map_err(|e| Error::Agent(format!("Failed to create FTS triggers: {}", e)))?;

        Ok(())
    }

    // === Session Management ===

    /// Save or update a session.
    pub fn save_session(
        &self,
        id: &str,
        title: Option<&str>,
        source: &str,
        created_at: &str,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, title, source, created_at, updated_at) 
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET 
                 title = excluded.title, 
                 source = excluded.source,
                 updated_at = excluded.updated_at",
            params![id, title, source, created_at, updated_at],
        )
        .map_err(|e| Error::Agent(format!("Failed to save session: {}", e)))?;
        Ok(())
    }

    /// Save a message to a session.
    pub fn save_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        timestamp: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, timestamp],
        )
        .map_err(|e| Error::Agent(format!("Failed to save message: {}", e)))?;
        Ok(())
    }

    /// Get all messages for a session, ordered by timestamp.
    pub fn get_session_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT role, content, timestamp FROM messages WHERE session_id = ?1 ORDER BY timestamp ASC")
            .map_err(|e| Error::Agent(format!("Failed to prepare statement: {}", e)))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(Message {
                    role: row.get(0)?,
                    content: row.get(1)?,
                    timestamp: row.get(2)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(messages)
    }

    /// List recent sessions (for session_search_tool).
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<DatabaseSession>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.source, s.created_at, s.updated_at, COUNT(m.id) as msg_count 
                 FROM sessions s 
                 LEFT JOIN messages m ON s.id = m.session_id 
                 GROUP BY s.id 
                 ORDER BY s.updated_at DESC 
                 LIMIT ?1",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare list_sessions: {}", e)))?;

        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(DatabaseSession {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    source: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    message_count: row.get::<_, i32>(5)? as usize,
                })
            })
            .map_err(|e| Error::Agent(format!("List sessions query error: {}", e)))?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(sessions)
    }

    /// Search sessions by content query (using FTS5).
    /// Returns session IDs and matching content snippets.
    pub fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<SessionSearchResult>> {
        let conn = self.conn.lock().unwrap();

        // Use FTS5 to find matching messages, join to get session info
        let mut stmt = conn
            .prepare(
                "SELECT m.session_id, m.content, s.title, s.updated_at
                 FROM messages m
                 JOIN messages_fts fts ON m.id = fts.rowid
                 JOIN sessions s ON m.session_id = s.id
                 WHERE messages_fts MATCH ?1 
                 ORDER BY rank 
                 LIMIT ?2",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare search: {}", e)))?;

        let rows = stmt
            .query_map(params![query, limit], |row| {
                Ok(SessionSearchResult {
                    session_id: row.get(0)?,
                    content: row.get(1)?,
                    title: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Search query error: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| Error::Agent(format!("Search row error: {}", e)))?);
        }
        Ok(results)
    }

    /// Delete a session and all its messages.
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![session_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to delete session: {}", e)))?;
        Ok(())
    }

    // === Checkpoint Management ===

    /// Store or update a checkpoint.
    pub fn store_checkpoint(
        &self,
        hash: &str,
        timestamp: &str,
        reason: Option<&str>,
        directory: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO checkpoints (hash, timestamp, reason, directory) 
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(hash) DO UPDATE SET 
                 reason = excluded.reason,
                 timestamp = excluded.timestamp",
            params![hash, timestamp, reason, directory],
        )
        .map_err(|e| Error::Agent(format!("Failed to store checkpoint: {}", e)))?;
        Ok(())
    }

    /// List checkpoints for a directory.
    pub fn list_checkpoints(&self, directory: &str) -> Result<Vec<StoredCheckpoint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT hash, timestamp, reason FROM checkpoints 
                 WHERE directory = ?1 
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare list: {}", e)))?;

        let rows = stmt
            .query_map(params![directory], |row| {
                Ok(StoredCheckpoint {
                    hash: row.get(0)?,
                    timestamp: row.get(1)?,
                    reason: row.get(2).ok(),
                })
            })
            .map_err(|e| Error::Agent(format!("List query error: {}", e)))?;

        let mut checkpoints = Vec::new();
        for row in rows {
            checkpoints.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(checkpoints)
    }

    /// Get a specific checkpoint by hash.
    pub fn get_checkpoint(&self, hash: &str) -> Result<Option<StoredCheckpoint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT hash, timestamp, reason, directory FROM checkpoints WHERE hash = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare get: {}", e)))?;

        let checkpoint = stmt
            .query_row(params![hash], |row| {
                Ok(StoredCheckpoint {
                    hash: row.get(0)?,
                    timestamp: row.get(1)?,
                    reason: row.get(2).ok(),
                })
            })
            .ok();

        Ok(checkpoint)
    }

    /// Delete a checkpoint.
    pub fn delete_checkpoint(&self, hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM checkpoints WHERE hash = ?1",
            params![hash],
        )
        .map_err(|e| Error::Agent(format!("Failed to delete checkpoint: {}", e)))?;
        Ok(())
    }

    /// Get database file path (for testing/verification).
    pub fn path(&self) -> Option<PathBuf> {
        self.conn.lock().ok().map(|c| PathBuf::from(c.path().unwrap_or("")))
    }
}

// === Data Types ===

/// A message stored in the database.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

/// A session from the database (for listing).
#[derive(Debug, Clone)]
pub struct DatabaseSession {
    pub id: String,
    pub title: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

/// A search result from FTS5 query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSearchResult {
    pub session_id: String,
    pub content: String,
    pub title: Option<String>,
    pub updated_at: String,
}

/// A checkpoint stored in the database.
#[derive(Debug, Clone)]
pub struct StoredCheckpoint {
    pub hash: String,
    pub timestamp: String,
    pub reason: Option<String>,
}