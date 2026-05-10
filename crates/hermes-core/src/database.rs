//! Database persistence layer for Hermes-RS
//!
//! Handles session storage, message history, and checkpoint metadata using SQLite.

use rusqlite::{params, Connection, Result};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use crate::error::Error;

/// Database manager for persistent storage
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

pub struct DatabaseSession {
    pub id: String,
    pub title: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

impl Database {
    /// Initialize a new database at the specified path
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        info!("Initializing database at {:?}", path);
        
        let conn = Connection::open(path).map_err(|e| Error::Agent(format!("Failed to open database: {}", e)))?;
        
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        
        db.setup_schema()?;
        Ok(db)
    }

    /// Create necessary tables and FTS indices
    fn setup_schema(&self) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        
        // Sessions table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                source TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        ).map_err(|e| Error::Agent(format!("Failed to create sessions table: {}", e)))?;

        // Messages table
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
        ).map_err(|e| Error::Agent(format!("Failed to create messages table: {}", e)))?;

        // Checkpoints table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS checkpoints (
                hash TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                reason TEXT,
                directory TEXT NOT NULL
            )",
            [],
        ).map_err(|e| Error::Agent(format!("Failed to create checkpoints table: {}", e)))?;

        // FTS5 virtual table for session search
        // We use an external content table for the FTS index to keep data normalized
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='id'
            )",
            [],
        ).map_err(|e| Error::Agent(format!("Failed to create FTS table: {}", e)))?;

        // Triggers to keep FTS index in sync with messages table
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
            END",
            [],
        ).map_err(|e| Error::Agent(format!("Failed to create insert trigger: {}", e)))?;

        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
            END",
            [],
        ).map_err(|e| Error::Agent(format!("Failed to create delete trigger: {}", e)))?;

        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
            END",
            [],
        ).map_err(|e| Error::Agent(format!("Failed to create update trigger: {}", e)))?;

        debug!("Database schema initialized successfully");
        Ok(())
    }

    // --- Session Management ---

    pub fn save_session(&self, id: &str, title: Option<&str>, source: &str, created_at: &str, updated_at: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, title, source, created_at, updated_at) 
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET 
                title = excluded.title, 
                source = excluded.source,
                updated_at = excluded.updated_at",
            params![id, title, source, created_at, updated_at],
        ).map_err(|e| Error::Agent(format!("Failed to save session: {}", e)))?;
        Ok(())
    }

    pub fn save_message(&self, session_id: &str, role: &str, content: &str, timestamp: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, timestamp],
        ).map_err(|e| Error::Agent(format!("Failed to save message: {}", e)))?;
        Ok(())
    }

    pub fn get_session_messages(&self, session_id: &str) -> Result<Vec<(String, String, String)>, Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT role, content, timestamp FROM messages WHERE session_id = ?1 ORDER BY timestamp ASC")
            .map_err(|e| Error::Agent(format!("Failed to prepare statement: {}", e)))?;
        
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).map_err(|e| Error::Agent(format!("Query error: {}", e)))?;
        
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(messages)
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<DatabaseSession>, Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.title, s.source, s.created_at, s.updated_at, COUNT(m.id) 
             FROM sessions s 
             LEFT JOIN messages m ON s.id = m.session_id 
             GROUP BY s.id 
             ORDER BY s.updated_at DESC 
             LIMIT ?1"
        ).map_err(|e| Error::Agent(format!("Failed to prepare list_sessions: {}", e)))?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok(DatabaseSession {
                id: row.get(0)?,
                title: row.get(1)?,
                source: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                message_count: row.get(5)?,
            })
        }).map_err(|e| Error::Agent(format!("List sessions query error: {}", e)))?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(sessions)
    }

    pub fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<(String, String)>, Error> {
        let conn = self.conn.lock().unwrap();
        
        // Use FTS5 to find matching messages, then group by session
        let mut stmt = conn.prepare(
            "SELECT session_id, content 
             FROM messages 
             JOIN messages_fts ON messages.id = messages_fts.rowid 
             WHERE messages_fts MATCH ?1 
             ORDER BY rank 
             LIMIT ?2"
        ).map_err(|e| Error::Agent(format!("Failed to prepare search: {}", e)))?;

        let rows = stmt.query_map(params![query, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| Error::Agent(format!("Search query error: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| Error::Agent(format!("Search row error: {}", e)))?);
        }
        Ok(results)
    }

    // --- Checkpoint Management ---

    pub fn store_checkpoint(&self, hash: &str, timestamp: &str, reason: Option<&str>, directory: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO checkpoints (hash, timestamp, reason, directory) 
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(hash) DO UPDATE SET reason = excluded.reason",
            params![hash, timestamp, reason, directory],
        ).map_err(|e| Error::Agent(format!("Failed to store checkpoint: {}", e)))?;
        Ok(())
    }

    pub fn list_checkpoints(&self, directory: &str) -> Result<Vec<(String, String, String)>, Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, timestamp, reason FROM checkpoints WHERE directory = ?1 ORDER BY timestamp DESC"
        ).map_err(|e| Error::Agent(format!("Failed to prepare list: {}", e)))?;

        let rows = stmt.query_map(params![directory], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2).unwrap_or_default()))
        }).map_err(|e| Error::Agent(format!("List query error: {}", e)))?;

        let mut checkpoints = Vec::new();
        for row in rows {
            checkpoints.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(checkpoints)
    }
}
