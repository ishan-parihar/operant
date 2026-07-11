//! Database persistence layer for Operant-RS
//!
//! Handles session storage, message history, and checkpoint metadata using SQLite.
//! Uses rusqlite with bundled SQLite for simplicity and portability.
//!
//! Schema management follows the declarative reconciliation pattern (Python operant-state.py):
//! DESIRED_SCHEMA_SQL is the single source of truth. On existing databases,
//! reconcile_columns() diffs live columns against DESIRED_SCHEMA_SQL and ADDs
//! any missing ones. This makes column additions a declarative operation.

use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, info, warn};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// Current schema version. Bump when adding data migrations that can't be
/// expressed declaratively (column additions are handled by reconcile_columns).
const SCHEMA_VERSION: i64 = 1;

/// Desired schema SQL — single source of truth for table structure.
/// Column additions are picked up automatically by reconcile_columns().
const DESIRED_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL DEFAULT 'local',
    user_id TEXT,
    model TEXT,
    model_config TEXT,
    system_prompt TEXT,
    parent_session_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    end_reason TEXT,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    cache_write_tokens INTEGER DEFAULT 0,
    reasoning_tokens INTEGER DEFAULT 0,
    cwd TEXT,
    billing_provider TEXT,
    billing_base_url TEXT,
    billing_mode TEXT,
    estimated_cost_usd REAL,
    actual_cost_usd REAL,
    cost_status TEXT,
    cost_source TEXT,
    pricing_version TEXT,
    title TEXT,
    api_call_count INTEGER DEFAULT 0,
    handoff_state TEXT,
    handoff_platform TEXT,
    handoff_error TEXT,
    rewind_count INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT,
    tool_call_id TEXT,
    tool_calls TEXT,
    tool_name TEXT,
    timestamp TEXT NOT NULL,
    token_count INTEGER,
    finish_reason TEXT,
    reasoning TEXT,
    reasoning_content TEXT,
    reasoning_details TEXT,
    codex_reasoning_items TEXT,
    codex_message_items TEXT,
    platform_message_id TEXT,
    observed INTEGER DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS state_meta (
    key TEXT PRIMARY KEY,
    value TEXT
);

CREATE TABLE IF NOT EXISTS compression_locks (
    session_id TEXT PRIMARY KEY,
    holder TEXT NOT NULL,
    acquired_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

";

/// Indexes created AFTER reconcile_columns() to avoid referencing columns
/// that may not exist yet on legacy databases.
const DEFERRED_INDEX_SQL: &str = "
CREATE INDEX IF NOT EXISTS idx_sessions_source ON sessions(source);
CREATE INDEX IF NOT EXISTS idx_sessions_source_id ON sessions(source, id);
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_session_active
    ON messages(session_id, active, timestamp);
CREATE INDEX IF NOT EXISTS idx_compression_locks_expires ON compression_locks(expires_at);
";

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
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("Failed to create database directory: {}", e)))?;
        }

        info!("Initializing database at {:?}", path);
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("Failed to open database: {}", e)))?;

        // Enable WAL mode for better concurrent read/write performance
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )
        .unwrap_or_else(|e| {
            warn!(
                "Failed to enable WAL mode, using default journal mode: {}",
                e
            );
        });

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

        // Create tables using desired schema SQL
        conn.execute_batch(DESIRED_SCHEMA_SQL)
            .map_err(|e| Error::Agent(format!("Failed to create tables: {}", e)))?;

        // Reconcile columns — add any missing columns to existing tables
        Self::reconcile_columns(&conn)?;

        // Deferred indexes referencing reconciler-added columns
        conn.execute_batch(DEFERRED_INDEX_SQL)
            .map_err(|e| Error::Agent(format!("Failed to create deferred indexes: {}", e)))?;

        // Additional tables that weren't in original DESIRED_SCHEMA_SQL
        self.create_checkpoints_table(&conn)?;
        self.create_fts_index(&conn)?;
        self.create_session_metadata_table(&conn)?;
        self.create_tools_state_table(&conn)?;
        self.create_session_tags_table(&conn)?;
        self.create_session_events_table(&conn)?;

        // Schema version bookkeeping
        Self::ensure_schema_version(&conn)?;

        debug!("Database migrations completed successfully");
        Ok(())
    }

    /// Parse desired columns from DESIRED_SCHEMA_SQL using an in-memory SQLite.
    /// Returns table_name -> (col_name -> col_type_expr).
    fn parse_desired_columns(schema_sql: &str) -> Result<HashMap<String, HashMap<String, String>>> {
        let ref_conn = Connection::open_in_memory()
            .map_err(|e| Error::Agent(format!("Failed to open in-memory DB: {}", e)))?;

        ref_conn
            .execute_batch(schema_sql)
            .map_err(|e| Error::Agent(format!("Failed to execute schema SQL: {}", e)))?;

        let mut table_columns: HashMap<String, HashMap<String, String>> = HashMap::new();

        let tables: Vec<String> = {
            let mut stmt = ref_conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                )
                .map_err(|e| Error::Agent(format!("Failed to prepare table query: {}", e)))?;
            let rows = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| Error::Agent(format!("Failed to query tables: {}", e)))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        for tbl in tables {
            let mut cols: HashMap<String, String> = HashMap::new();
            let mut stmt = ref_conn
                .prepare(&format!("PRAGMA table_info(\"{}\")", tbl))
                .map_err(|e| {
                    Error::Agent(format!("Failed to prepare PRAGMA for {}: {}", tbl, e))
                })?;
            let rows = stmt
                .query_map([], |row| {
                    // PRAGMA table_info returns (cid, name, type, notnull, dflt_value, pk)
                    let name: String = row.get(1)?;
                    let col_type: String = row.get(2)?;
                    let notnull: bool = row.get(3)?;
                    let dflt_value: Option<String> = row.get(4)?;
                    let pk: bool = row.get(5)?;
                    Ok((name, col_type, notnull, dflt_value, pk))
                })
                .map_err(|e| Error::Agent(format!("Failed to query columns for {}: {}", tbl, e)))?;
            for row in rows {
                let (name, col_type, notnull, dflt_value, pk) =
                    row.map_err(|e| Error::Agent(format!("Row error for {}: {}", tbl, e)))?;
                // Reconstruct type expression for ALTER TABLE ADD COLUMN
                let mut parts = Vec::new();
                if !col_type.is_empty() {
                    parts.push(col_type);
                }
                if notnull && !pk {
                    parts.push("NOT NULL".to_string());
                }
                if let Some(ref default) = dflt_value {
                    parts.push(format!("DEFAULT {}", default));
                }
                cols.insert(name, parts.join(" "));
            }
            table_columns.insert(tbl, cols);
        }

        Ok(table_columns)
    }

    /// Ensure live tables have every column declared in DESIRED_SCHEMA_SQL.
    /// Diff live columns via PRAGMA table_info against the desired schema
    /// and ADD any missing ones.
    fn reconcile_columns(conn: &Connection) -> Result<()> {
        let desired = Self::parse_desired_columns(DESIRED_SCHEMA_SQL)?;

        for (table_name, declared_cols) in &desired {
            // Get current columns from the live table
            let live_cols = match Self::get_live_columns(conn, table_name) {
                Ok(cols) => cols,
                Err(_) => continue, // Table doesn't exist yet (shouldn't happen after CREATE)
            };

            for (col_name, col_type) in declared_cols {
                if !live_cols.contains(col_name) {
                    let safe_name = col_name.replace('"', "\"\"");
                    let sql = format!(
                        "ALTER TABLE \"{}\" ADD COLUMN \"{}\" {}",
                        table_name, safe_name, col_type
                    );
                    match conn.execute_batch(&sql) {
                        Ok(_) => {
                            debug!("reconcile {}.{}: added", table_name, col_name);
                        }
                        Err(e) => {
                            // Expected: "duplicate column name" from a race or re-run
                            debug!("reconcile {}.{}: {}", table_name, col_name, e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Get live columns for a table via PRAGMA table_info.
    fn get_live_columns(
        conn: &Connection,
        table_name: &str,
    ) -> Result<std::collections::HashSet<String>> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{}\")", table_name))
            .map_err(|e| {
                Error::Agent(format!(
                    "PRAGMA table_info failed for {}: {}",
                    table_name, e
                ))
            })?;
        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                Ok(name)
            })
            .map_err(|e| Error::Agent(format!("Query error for {}: {}", table_name, e)))?;
        let mut cols = std::collections::HashSet::new();
        for name in rows.flatten() {
            cols.insert(name);
        }
        Ok(cols)
    }

    /// Ensure schema_version table has the current version.
    fn ensure_schema_version(conn: &Connection) -> Result<()> {
        let current_version: i64 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        if current_version < SCHEMA_VERSION {
            if current_version == 0 {
                conn.execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    params![SCHEMA_VERSION],
                )
                .map_err(|e| Error::Agent(format!("Failed to insert schema version: {}", e)))?;
            } else {
                conn.execute(
                    "UPDATE schema_version SET version = ?1",
                    params![SCHEMA_VERSION],
                )
                .map_err(|e| Error::Agent(format!("Failed to update schema version: {}", e)))?;
            }
        }

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

    fn create_session_metadata_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS session_metadata (
                session_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (session_id, key),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session_metadata table: {}", e)))?;
        Ok(())
    }

    fn create_tools_state_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tools_state (
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                state_json TEXT NOT NULL DEFAULT '{}',
                updated_at TEXT NOT NULL,
                PRIMARY KEY (session_id, tool_name),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create tools_state table: {}", e)))?;
        Ok(())
    }

    fn create_session_tags_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS session_tags (
                session_id TEXT NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (session_id, tag),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session_tags table: {}", e)))?;

        // Index for finding sessions by tag
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_tags_tag ON session_tags(tag)",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session_tags index: {}", e)))?;
        Ok(())
    }

    fn create_session_events_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS session_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_data TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session_events table: {}", e)))?;

        // Index for event queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events(session_id, created_at DESC)",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session_events session index: {}", e)))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_type ON session_events(event_type, created_at DESC)",
            [],
        )
        .map_err(|e| Error::Agent(format!("Failed to create session_events type index: {}", e)))?;
        Ok(())
    }

    // === Session Management ===

    /// Save or update a session with all Python-compatible fields.
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
            "INSERT INTO sessions (id, title, source, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 source = excluded.source,
                 ended_at = excluded.ended_at",
            params![id, title, source, created_at, updated_at],
        )
        .map_err(|e| Error::Agent(format!("Failed to save session: {}", e)))?;
        Ok(())
    }

    /// Update the accumulated actual cost for a session (R3 — cost fidelity).
    pub fn update_session_cost(&self, id: &str, actual_cost_usd: f64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET actual_cost_usd = ?1 WHERE id = ?2",
            params![actual_cost_usd, id],
        )
        .map_err(|e| Error::Agent(format!("Failed to update session cost: {}", e)))?;
        Ok(())
    }

    /// Save a session with full Python-compatible fields.
    pub fn save_session_full(&self, session: &SessionData) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (
                id, source, user_id, model, model_config, system_prompt,
                parent_session_id, started_at, ended_at, end_reason,
                message_count, tool_call_count, input_tokens, output_tokens,
                cache_read_tokens, cache_write_tokens, reasoning_tokens,
                cwd, billing_provider, billing_base_url, billing_mode,
                estimated_cost_usd, actual_cost_usd, cost_status, cost_source,
                pricing_version, title, api_call_count, handoff_state,
                handoff_platform, handoff_error, rewind_count, archived
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30,
                ?31, ?32, ?33
            )
            ON CONFLICT(id) DO UPDATE SET
                source = excluded.source,
                user_id = excluded.user_id,
                model = excluded.model,
                model_config = excluded.model_config,
                system_prompt = excluded.system_prompt,
                parent_session_id = excluded.parent_session_id,
                started_at = excluded.started_at,
                ended_at = excluded.ended_at,
                end_reason = excluded.end_reason,
                message_count = excluded.message_count,
                tool_call_count = excluded.tool_call_count,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                cache_read_tokens = excluded.cache_read_tokens,
                cache_write_tokens = excluded.cache_write_tokens,
                reasoning_tokens = excluded.reasoning_tokens,
                cwd = excluded.cwd,
                billing_provider = excluded.billing_provider,
                billing_base_url = excluded.billing_base_url,
                billing_mode = excluded.billing_mode,
                estimated_cost_usd = excluded.estimated_cost_usd,
                actual_cost_usd = excluded.actual_cost_usd,
                cost_status = excluded.cost_status,
                cost_source = excluded.cost_source,
                pricing_version = excluded.pricing_version,
                title = excluded.title,
                api_call_count = excluded.api_call_count,
                handoff_state = excluded.handoff_state,
                handoff_platform = excluded.handoff_platform,
                handoff_error = excluded.handoff_error,
                rewind_count = excluded.rewind_count,
                archived = excluded.archived",
            params![
                session.id,
                session.source,
                session.user_id,
                session.model,
                session.model_config,
                session.system_prompt,
                session.parent_session_id,
                session.started_at,
                session.ended_at,
                session.end_reason,
                session.message_count,
                session.tool_call_count,
                session.input_tokens,
                session.output_tokens,
                session.cache_read_tokens,
                session.cache_write_tokens,
                session.reasoning_tokens,
                session.cwd,
                session.billing_provider,
                session.billing_base_url,
                session.billing_mode,
                session.estimated_cost_usd,
                session.actual_cost_usd,
                session.cost_status,
                session.cost_source,
                session.pricing_version,
                session.title,
                session.api_call_count,
                session.handoff_state,
                session.handoff_platform,
                session.handoff_error,
                session.rewind_count,
                session.archived,
            ],
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

    /// Save a message with full Python-compatible fields.
    pub fn save_message_full(&self, message: &MessageData) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (
                session_id, role, content, tool_call_id, tool_calls, tool_name,
                timestamp, token_count, finish_reason, reasoning, reasoning_content,
                reasoning_details, codex_reasoning_items, codex_message_items,
                platform_message_id, observed, active
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
            )",
            params![
                message.session_id,
                message.role,
                message.content,
                message.tool_call_id,
                message.tool_calls,
                message.tool_name,
                message.timestamp,
                message.token_count,
                message.finish_reason,
                message.reasoning,
                message.reasoning_content,
                message.reasoning_details,
                message.codex_reasoning_items,
                message.codex_message_items,
                message.platform_message_id,
                message.observed,
                message.active,
            ],
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

    /// Get all messages for a session with full fields, ordered by timestamp.
    pub fn get_session_messages_full(&self, session_id: &str) -> Result<Vec<MessageData>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, tool_call_id, tool_calls, tool_name,
                        timestamp, token_count, finish_reason, reasoning, reasoning_content,
                        reasoning_details, codex_reasoning_items, codex_message_items,
                        platform_message_id, observed, active
                 FROM messages WHERE session_id = ?1 ORDER BY timestamp ASC",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare statement: {}", e)))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(MessageData {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    tool_call_id: row.get(4)?,
                    tool_calls: row.get(5)?,
                    tool_name: row.get(6)?,
                    timestamp: row.get(7)?,
                    token_count: row.get(8)?,
                    finish_reason: row.get(9)?,
                    reasoning: row.get(10)?,
                    reasoning_content: row.get(11)?,
                    reasoning_details: row.get(12)?,
                    codex_reasoning_items: row.get(13)?,
                    codex_message_items: row.get(14)?,
                    platform_message_id: row.get(15)?,
                    observed: row.get(16)?,
                    active: row.get(17)?,
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
                "SELECT s.id, s.title, s.source, s.started_at, s.ended_at, COUNT(m.id) as msg_count,
                        s.actual_cost_usd
                 FROM sessions s
                 LEFT JOIN messages m ON s.id = m.session_id
                 GROUP BY s.id
                 ORDER BY s.ended_at DESC
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
                    actual_cost_usd: row.get(6)?,
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
                "SELECT m.session_id, m.content, s.title, s.ended_at
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
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
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
        conn.execute("DELETE FROM checkpoints WHERE hash = ?1", params![hash])
            .map_err(|e| Error::Agent(format!("Failed to delete checkpoint: {}", e)))?;
        Ok(())
    }

    /// Get database file path (for testing/verification).
    pub fn path(&self) -> Option<PathBuf> {
        self.conn
            .lock()
            .ok()
            .map(|c| PathBuf::from(c.path().unwrap_or("")))
    }

    // === Session Metadata ===

    /// Set a metadata key-value pair for a session.
    pub fn set_session_metadata(&self, session_id: &str, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO session_metadata (session_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id, key) DO UPDATE SET value = excluded.value",
            params![session_id, key, value],
        )
        .map_err(|e| Error::Agent(format!("Failed to set session metadata: {}", e)))?;
        Ok(())
    }

    /// Get a metadata value for a session by key.
    pub fn get_session_metadata(&self, session_id: &str, key: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT value FROM session_metadata WHERE session_id = ?1 AND key = ?2",
            params![session_id, key],
            |row| row.get(0),
        )
        .ok()
    }

    /// Get all metadata for a session.
    pub fn get_all_session_metadata(&self, session_id: &str) -> Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT key, value FROM session_metadata WHERE session_id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare metadata query: {}", e)))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::Agent(format!("Metadata query error: {}", e)))?;
        let mut map = HashMap::new();
        for row in rows {
            let (key, value) =
                row.map_err(|e| Error::Agent(format!("Metadata row error: {}", e)))?;
            map.insert(key, value);
        }
        Ok(map)
    }

    /// Delete a specific metadata key for a session.
    pub fn delete_session_metadata(&self, session_id: &str, key: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM session_metadata WHERE session_id = ?1 AND key = ?2",
            params![session_id, key],
        )
        .map_err(|e| Error::Agent(format!("Failed to delete session metadata: {}", e)))?;
        Ok(())
    }

    // === Tools State ===

    /// Save tool state for a session.
    pub fn set_tool_state(&self, session_id: &str, tool_name: &str, state: &Value) -> Result<()> {
        let state_json = serde_json::to_string(state)?;
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tools_state (session_id, tool_name, state_json, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, tool_name) DO UPDATE SET state_json = excluded.state_json, updated_at = excluded.updated_at",
            params![session_id, tool_name, state_json, now],
        )
        .map_err(|e| Error::Agent(format!("Failed to set tool state: {}", e)))?;
        Ok(())
    }

    /// Get tool state for a session.
    pub fn get_tool_state(&self, session_id: &str, tool_name: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;
        let json: String = conn
            .query_row(
                "SELECT state_json FROM tools_state WHERE session_id = ?1 AND tool_name = ?2",
                params![session_id, tool_name],
                |row| row.get(0),
            )
            .ok()?;
        serde_json::from_str(&json).ok()
    }

    /// Clear tool state for a specific tool in a session.
    pub fn clear_tool_state(&self, session_id: &str, tool_name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tools_state WHERE session_id = ?1 AND tool_name = ?2",
            params![session_id, tool_name],
        )
        .map_err(|e| Error::Agent(format!("Failed to clear tool state: {}", e)))?;
        Ok(())
    }

    /// Clear all tool states for a session.
    pub fn clear_all_tool_states(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tools_state WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to clear all tool states: {}", e)))?;
        Ok(())
    }

    // === Session Tags ===

    /// Add a tag to a session.
    pub fn add_session_tag(&self, session_id: &str, tag: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO session_tags (session_id, tag) VALUES (?1, ?2)",
            params![session_id, tag],
        )
        .map_err(|e| Error::Agent(format!("Failed to add session tag: {}", e)))?;
        Ok(())
    }

    /// Remove a tag from a session.
    pub fn remove_session_tag(&self, session_id: &str, tag: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM session_tags WHERE session_id = ?1 AND tag = ?2",
            params![session_id, tag],
        )
        .map_err(|e| Error::Agent(format!("Failed to remove session tag: {}", e)))?;
        Ok(())
    }

    /// Get all tags for a session.
    pub fn get_session_tags(&self, session_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT tag FROM session_tags WHERE session_id = ?1 ORDER BY tag")
            .map_err(|e| Error::Agent(format!("Failed to prepare tags query: {}", e)))?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("Tags query error: {}", e)))?;
        let mut tags = Vec::new();
        for row in rows {
            tags.push(row.map_err(|e| Error::Agent(format!("Tag row error: {}", e)))?);
        }
        Ok(tags)
    }

    /// Find sessions by tag, returning session summaries.
    pub fn find_sessions_by_tag(&self, tag: &str) -> Result<Vec<SessionSummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.source, s.started_at, s.ended_at, COUNT(m.id) as msg_count
                 FROM sessions s
                 JOIN session_tags st ON s.id = st.session_id
                 LEFT JOIN messages m ON s.id = m.session_id
                 WHERE st.tag = ?1
                 GROUP BY s.id
                 ORDER BY s.ended_at DESC",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare session-by-tag query: {}", e)))?;
        let rows = stmt
            .query_map(params![tag], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    source: row.get(2)?,
                    message_count: row.get::<_, i64>(5)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    tags: Vec::new(),
                })
            })
            .map_err(|e| Error::Agent(format!("Session-by-tag query error: {}", e)))?;
        let mut sessions: Vec<SessionSummary> = Vec::new();
        for row in rows {
            sessions
                .push(row.map_err(|e| Error::Agent(format!("Session-by-tag row error: {}", e)))?);
        }
        self.enrich_session_tags(&conn, &mut sessions)?;
        Ok(sessions)
    }

    // === Events ===

    /// Record an event for a session.
    pub fn record_event(
        &self,
        session_id: &str,
        event_type: &str,
        event_data: &Value,
    ) -> Result<()> {
        let data_json = serde_json::to_string(event_data)?;
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO session_events (session_id, event_type, event_data, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, event_type, data_json, now],
        )
        .map_err(|e| Error::Agent(format!("Failed to record event: {}", e)))?;
        Ok(())
    }

    /// Get events for a session.
    pub fn get_session_events(
        &self,
        session_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<StoredEvent>> {
        let conn = self.conn.lock().unwrap();
        let limit = limit.unwrap_or(50) as i64;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, event_type, event_data, created_at 
                 FROM session_events 
                 WHERE session_id = ?1 
                 ORDER BY created_at DESC 
                 LIMIT ?2",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare event query: {}", e)))?;
        let rows = stmt
            .query_map(params![session_id, limit], |row| {
                let event_data_str: String = row.get(3)?;
                let event_data: Value =
                    serde_json::from_str(&event_data_str).unwrap_or(Value::Null);
                Ok(StoredEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_type: row.get(2)?,
                    event_data,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Event query error: {}", e)))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| Error::Agent(format!("Event row error: {}", e)))?);
        }
        Ok(events)
    }

    /// Get events by type across all sessions.
    pub fn get_events_by_type(
        &self,
        event_type: &str,
        limit: Option<u32>,
    ) -> Result<Vec<StoredEvent>> {
        let conn = self.conn.lock().unwrap();
        let limit = limit.unwrap_or(50) as i64;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, event_type, event_data, created_at 
                 FROM session_events 
                 WHERE event_type = ?1 
                 ORDER BY created_at DESC 
                 LIMIT ?2",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare event-by-type query: {}", e)))?;
        let rows = stmt
            .query_map(params![event_type, limit], |row| {
                let event_data_str: String = row.get(3)?;
                let event_data: Value =
                    serde_json::from_str(&event_data_str).unwrap_or(Value::Null);
                Ok(StoredEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_type: row.get(2)?,
                    event_data,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Event-by-type query error: {}", e)))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| Error::Agent(format!("Event-by-type row error: {}", e)))?);
        }
        Ok(events)
    }

    // === Search Expansion ===

    /// Search messages using FTS5, with LIKE fallback for CJK queries.
    pub fn search_messages_fts(
        &self,
        query: &str,
        session_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<SearchResult>> {
        let conn = self.conn.lock().unwrap();
        let limit = limit.unwrap_or(50) as i64;

        if contains_cjk(query) {
            // CJK charset ─ use LIKE fallback since FTS5 tokenizer handles CJK poorly
            self.search_messages_like(&conn, query, session_id, limit)
        } else {
            // Try FTS5 first, fall back to LIKE on error
            match self.search_messages_fts5(&conn, query, session_id, limit) {
                Ok(results) => Ok(results),
                Err(_) => self.search_messages_like(&conn, query, session_id, limit),
            }
        }
    }

    /// Internal FTS5-based message search.
    fn search_messages_fts5(
        &self,
        conn: &Connection,
        query: &str,
        session_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SearchResult>> {
        let mut stmt = conn
            .prepare(
                "SELECT m.session_id, m.id, m.role, m.content, m.timestamp
                 FROM messages m
                 JOIN messages_fts fts ON m.id = fts.rowid
                 WHERE messages_fts MATCH ?1
                   AND (?2 IS NULL OR m.session_id = ?2)
                 ORDER BY rank
                 LIMIT ?3",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare FTS search: {}", e)))?;
        let rows = stmt
            .query_map(params![query, session_id, limit], |row| {
                Ok(SearchResult {
                    session_id: row.get(0)?,
                    message_id: row.get::<_, i64>(1)?.to_string(),
                    role: row.get(2)?,
                    content: row.get(3)?,
                    score: 1.0,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("FTS search error: {}", e)))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| Error::Agent(format!("FTS search row error: {}", e)))?);
        }
        Ok(results)
    }

    /// LIKE-based message search fallback for CJK and FTS5 failures.
    fn search_messages_like(
        &self,
        conn: &Connection,
        query: &str,
        session_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SearchResult>> {
        let escaped = query
            .replace("\\", "\\\\")
            .replace("%", "\\%")
            .replace("_", "\\_");
        let like_pattern = format!("%{}%", escaped);
        let mut stmt = conn
            .prepare(
                "SELECT m.session_id, m.id, m.role, m.content, m.timestamp
                 FROM messages m
                 WHERE m.content LIKE ?1 ESCAPE '\\'
                   AND (?2 IS NULL OR m.session_id = ?2)
                 ORDER BY m.timestamp DESC
                 LIMIT ?3",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare LIKE search: {}", e)))?;
        let rows = stmt
            .query_map(params![like_pattern, session_id, limit], |row| {
                Ok(SearchResult {
                    session_id: row.get(0)?,
                    message_id: row.get::<_, i64>(1)?.to_string(),
                    role: row.get(2)?,
                    content: row.get(3)?,
                    score: 0.0,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("LIKE search error: {}", e)))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| Error::Agent(format!("LIKE search row error: {}", e)))?);
        }
        Ok(results)
    }

    // === Session Management ===

    /// Update the title of a session.
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET title = ?1, ended_at = ?2 WHERE id = ?3",
            params![title, now, session_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to update session title: {}", e)))?;
        Ok(())
    }

    /// Get total session count.
    pub fn get_session_count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("Failed to count sessions: {}", e)))?;
        Ok(count as u64)
    }

    /// Get recently updated sessions with message counts and tags.
    pub fn get_recent_sessions(&self, limit: u32) -> Result<Vec<SessionSummary>> {
        let conn = self.conn.lock().unwrap();
        let limit = limit as i64;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.source, s.started_at, s.ended_at, COUNT(m.id) as msg_count
                 FROM sessions s
                 LEFT JOIN messages m ON s.id = m.session_id
                 GROUP BY s.id
                 ORDER BY s.ended_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare recent sessions: {}", e)))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    source: row.get(2)?,
                    message_count: row.get::<_, i64>(5)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    tags: Vec::new(),
                })
            })
            .map_err(|e| Error::Agent(format!("Recent sessions query error: {}", e)))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions
                .push(row.map_err(|e| Error::Agent(format!("Recent sessions row error: {}", e)))?);
        }
        self.enrich_session_tags(&conn, &mut sessions)?;
        Ok(sessions)
    }

    /// Get sessions active within the last `since_minutes` minutes.
    pub fn get_active_sessions(&self, since_minutes: u64) -> Result<Vec<SessionSummary>> {
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(since_minutes as i64);
        let cutoff_str = cutoff.to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.source, s.started_at, s.ended_at, COUNT(m.id) as msg_count
                 FROM sessions s
                 LEFT JOIN messages m ON s.id = m.session_id
                 WHERE s.ended_at >= ?1
                 GROUP BY s.id
                 ORDER BY s.ended_at DESC",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare active sessions: {}", e)))?;
        let rows = stmt
            .query_map(params![cutoff_str], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    source: row.get(2)?,
                    message_count: row.get::<_, i64>(5)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    tags: Vec::new(),
                })
            })
            .map_err(|e| Error::Agent(format!("Active sessions query error: {}", e)))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions
                .push(row.map_err(|e| Error::Agent(format!("Active sessions row error: {}", e)))?);
        }
        self.enrich_session_tags(&conn, &mut sessions)?;
        Ok(sessions)
    }

    /// Merge one or more source sessions into a target session.
    /// Moves messages, metadata, tags, and events; deletes source sessions.
    pub fn merge_sessions(&self, target_id: &str, source_ids: &[&str]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        for &source_id in source_ids {
            // Move messages
            conn.execute(
                "UPDATE messages SET session_id = ?1 WHERE session_id = ?2",
                params![target_id, source_id],
            )
            .map_err(|e| Error::Agent(format!("Failed to move messages: {}", e)))?;
            // Copy metadata (skip existing keys in target)
            conn.execute(
                "INSERT OR IGNORE INTO session_metadata (session_id, key, value)
                 SELECT ?1, key, value FROM session_metadata WHERE session_id = ?2",
                params![target_id, source_id],
            )
            .map_err(|e| Error::Agent(format!("Failed to copy metadata: {}", e)))?;
            // Copy tags (skip duplicates)
            conn.execute(
                "INSERT OR IGNORE INTO session_tags (session_id, tag)
                 SELECT ?1, tag FROM session_tags WHERE session_id = ?2",
                params![target_id, source_id],
            )
            .map_err(|e| Error::Agent(format!("Failed to copy tags: {}", e)))?;
            // Move events
            conn.execute(
                "UPDATE session_events SET session_id = ?1 WHERE session_id = ?2",
                params![target_id, source_id],
            )
            .map_err(|e| Error::Agent(format!("Failed to move events: {}", e)))?;
            // Delete source session (CASCADE handles tools_state)
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![source_id])
                .map_err(|e| Error::Agent(format!("Failed to delete source session: {}", e)))?;
        }
        // Update target session timestamp
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
            params![now, target_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to update target session: {}", e)))?;
        Ok(())
    }

    /// Enrich a list of SessionSummary with their tags (in-place).
    fn enrich_session_tags(
        &self,
        conn: &Connection,
        sessions: &mut [SessionSummary],
    ) -> Result<()> {
        for session in sessions.iter_mut() {
            let mut stmt = conn
                .prepare("SELECT tag FROM session_tags WHERE session_id = ?1 ORDER BY tag")
                .map_err(|e| Error::Agent(format!("Failed to prepare enrich tags: {}", e)))?;
            let tags: Vec<String> = stmt
                .query_map(params![session.id], |row| row.get(0))
                .map_err(|e| Error::Agent(format!("Enrich tags query error: {}", e)))?
                .filter_map(|r| r.ok())
                .collect();
            session.tags = tags;
        }
        Ok(())
    }

    // === State Meta ===

    /// Set a state_meta key-value pair.
    pub fn set_state_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO state_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| Error::Agent(format!("Failed to set state_meta: {}", e)))?;
        Ok(())
    }

    /// Get a state_meta value by key.
    pub fn get_state_meta(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT value FROM state_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
    }

    // === Compression Locks ===

    /// Acquire a compression lock for a session.
    pub fn acquire_compression_lock(
        &self,
        session_id: &str,
        holder: &str,
        acquired_at: &str,
        expires_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO compression_locks (session_id, holder, acquired_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, holder, acquired_at, expires_at],
        )
        .map_err(|e| Error::Agent(format!("Failed to acquire compression lock: {}", e)))?;
        Ok(())
    }

    /// Check if a compression lock is held and not expired.
    pub fn is_compression_locked(&self, session_id: &str) -> bool {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let now = chrono::Utc::now().to_rfc3339();
        conn.query_row(
            "SELECT COUNT(*) FROM compression_locks WHERE session_id = ?1 AND expires_at > ?2",
            params![session_id, now],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false)
    }

    /// Release a compression lock for a session.
    pub fn release_compression_lock(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM compression_locks WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to release compression lock: {}", e)))?;
        Ok(())
    }
}

// === Data Types ===

/// A message stored in the database (legacy simple format).
#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

/// Full session data matching Python operant_state.py schema.
#[derive(Debug, Clone, Default)]
pub struct SessionData {
    pub id: String,
    pub source: String,
    pub user_id: Option<String>,
    pub model: Option<String>,
    pub model_config: Option<String>,
    pub system_prompt: Option<String>,
    pub parent_session_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub end_reason: Option<String>,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cwd: Option<String>,
    pub billing_provider: Option<String>,
    pub billing_base_url: Option<String>,
    pub billing_mode: Option<String>,
    pub estimated_cost_usd: Option<f64>,
    pub actual_cost_usd: Option<f64>,
    pub cost_status: Option<String>,
    pub cost_source: Option<String>,
    pub pricing_version: Option<String>,
    pub title: Option<String>,
    pub api_call_count: i64,
    pub handoff_state: Option<String>,
    pub handoff_platform: Option<String>,
    pub handoff_error: Option<String>,
    pub rewind_count: i64,
    pub archived: i64,
}

/// Full message data matching Python operant_state.py schema.
#[derive(Debug, Clone, Default)]
pub struct MessageData {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Option<String>,
    pub tool_name: Option<String>,
    pub timestamp: String,
    pub token_count: Option<i64>,
    pub finish_reason: Option<String>,
    pub reasoning: Option<String>,
    pub reasoning_content: Option<String>,
    pub reasoning_details: Option<String>,
    pub codex_reasoning_items: Option<String>,
    pub codex_message_items: Option<String>,
    pub platform_message_id: Option<String>,
    pub observed: Option<i64>,
    pub active: i64,
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
    pub actual_cost_usd: Option<f64>,
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

/// An event recorded for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    pub id: i64,
    pub session_id: String,
    pub event_type: String,
    pub event_data: Value,
    pub created_at: String,
}

/// A result from full-text search on messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub session_id: String,
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub score: f64,
    pub created_at: String,
}

/// Summary of a session with message count and tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub source: String,
    pub message_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
}

// === Retry Logic ===

/// Execute a database operation with retry on SQLITE_BUSY.
///
/// Retries up to 5 times with exponential backoff (50ms, 100ms, 200ms, 400ms, 800ms).
/// Only retries on `rusqlite::Error::SqliteFailure` with `SQLITE_BUSY`.
/// Other errors are returned immediately.
pub fn with_retry<F, T>(f: F) -> Result<T>
where
    F: Fn() -> std::result::Result<T, rusqlite::Error>,
{
    let delays = [50u64, 100, 200, 400, 800];
    let mut last_err = None;

    for attempt in 0..delays.len() {
        match f() {
            Ok(val) => return Ok(val),
            Err(e) => {
                let is_busy = matches!(
                    e,
                    rusqlite::Error::SqliteFailure(ref err, _) if err.code == rusqlite::ffi::ErrorCode::DatabaseBusy
                );

                if !is_busy || attempt == delays.len() - 1 {
                    return Err(Error::Agent(format!("Database error: {}", e)));
                }

                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(delays[attempt]));
            }
        }
    }

    Err(Error::Agent(format!(
        "Database retry exhausted: {:?}",
        last_err
    )))
}

// === CJK Detection Helper ===

/// Check if text contains CJK (Chinese/Japanese/Korean) characters.
fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        let range = c as u32;
        (0x4E00..=0x9FFF).contains(&range)          // CJK Unified Ideographs
            || (0x3400..=0x4DBF).contains(&range)   // CJK Extension A
            || (0x2E80..=0x2EFF).contains(&range)   // CJK Radicals
            || (0x3000..=0x303F).contains(&range)   // CJK Symbols & Punctuation
            || (0x3040..=0x309F).contains(&range)   // Hiragana
            || (0x30A0..=0x30FF).contains(&range)   // Katakana
            || (0xFF00..=0xFFEF).contains(&range)   // Halfwidth/Fullwidth
            || (0xAC00..=0xD7AF).contains(&range) // Hangul Syllables
    })
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_db() -> Database {
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_test_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&path);
        let db = Database::init(path).expect("Failed to create test database");
        let id = "test-session";
        db.save_session(
            id,
            Some("test"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .expect("Failed to create test session");
        db
    }

    fn test_db_path() -> (Database, PathBuf) {
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("operant_test_dir_{}", counter));
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("test.db");
        let _ = std::fs::remove_file(&path);
        let db = Database::init(path.clone()).expect("Failed to create test database");
        (db, path)
    }

    #[test]
    fn test_metadata_crud() {
        let db = test_db();
        let session_id = "test-session";

        db.set_session_metadata(session_id, "key1", "value1")
            .unwrap();
        db.set_session_metadata(session_id, "key2", "value2")
            .unwrap();
        assert_eq!(
            db.get_session_metadata(session_id, "key1"),
            Some("value1".to_string())
        );
        assert_eq!(db.get_session_metadata(session_id, "nonexistent"), None);
        let all = db.get_all_session_metadata(session_id).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("key1").unwrap(), "value1");
        assert_eq!(all.get("key2").unwrap(), "value2");
        db.delete_session_metadata(session_id, "key1").unwrap();
        assert_eq!(db.get_session_metadata(session_id, "key1"), None);
        let all = db.get_all_session_metadata(session_id).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_tool_state_crud() {
        let db = test_db();
        let session_id = "test-session";

        let state = serde_json::json!({"count": 42, "active": true});

        // Set
        db.set_tool_state(session_id, "code_executor", &state)
            .unwrap();

        // Get
        let retrieved = db.get_tool_state(session_id, "code_executor");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap()["count"], 42);

        // Get nonexistent
        assert!(db.get_tool_state(session_id, "nonexistent").is_none());

        // Update
        let new_state = serde_json::json!({"count": 99});
        db.set_tool_state(session_id, "code_executor", &new_state)
            .unwrap();
        let retrieved = db.get_tool_state(session_id, "code_executor").unwrap();
        assert_eq!(retrieved["count"], 99);

        // Clear single
        db.set_tool_state(session_id, "other_tool", &serde_json::json!({"x": 1}))
            .unwrap();
        db.clear_tool_state(session_id, "other_tool").unwrap();
        assert!(db.get_tool_state(session_id, "other_tool").is_none());
        // First tool still exists
        assert!(db.get_tool_state(session_id, "code_executor").is_some());

        // Clear all
        db.clear_all_tool_states(session_id).unwrap();
        assert!(db.get_tool_state(session_id, "code_executor").is_none());
    }

    #[test]
    fn test_tag_operations() {
        let db = test_db();
        let session_id = "test-session";

        // Add tags
        db.add_session_tag(session_id, "important").unwrap();
        db.add_session_tag(session_id, "archived").unwrap();
        db.add_session_tag(session_id, "important").unwrap(); // duplicate

        // Get tags
        let tags = db.get_session_tags(session_id).unwrap();
        assert_eq!(tags.len(), 2);
        assert!(tags.contains(&"important".to_string()));
        assert!(tags.contains(&"archived".to_string()));

        // Remove tag
        db.remove_session_tag(session_id, "archived").unwrap();
        let tags = db.get_session_tags(session_id).unwrap();
        assert_eq!(tags.len(), 1);

        // Find by tag
        let sessions = db.find_sessions_by_tag("important").unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "test-session");

        // Find by nonexistent tag
        let sessions = db.find_sessions_by_tag("nonexistent").unwrap();
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn test_event_operations() {
        let db = test_db();
        let session_id = "test-session";

        // Record events
        db.record_event(
            session_id,
            "tool_call",
            &serde_json::json!({"tool": "bash"}),
        )
        .unwrap();
        db.record_event(
            session_id,
            "tool_result",
            &serde_json::json!({"exit_code": 0}),
        )
        .unwrap();
        db.record_event(session_id, "error", &serde_json::json!({"msg": "timeout"}))
            .unwrap();

        // Get by session
        let events = db.get_session_events(session_id, None).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "error"); // DESC order
        assert_eq!(events[2].event_type, "tool_call");

        // Verify event data is parsed as JSON
        assert_eq!(events[0].event_data["msg"], "timeout");

        // Get by type
        let tool_events = db.get_events_by_type("tool_call", None).unwrap();
        assert_eq!(tool_events.len(), 1);
        assert_eq!(tool_events[0].event_data["tool"], "bash");

        // Limit
        let limited = db.get_session_events(session_id, Some(1)).unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn test_fts_search() {
        let db = test_db();
        let session_id = "test-session";

        // Add messages with searchable content
        db.save_message(
            session_id,
            "user",
            "Hello, how do I install Python?",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_message(
            session_id,
            "assistant",
            "You can install Python using apt-get",
            "2024-01-01T00:00:01Z",
        )
        .unwrap();
        db.save_message(
            session_id,
            "user",
            "What about Rust installation?",
            "2024-01-01T00:00:02Z",
        )
        .unwrap();

        // Search with FTS5
        let results = db.search_messages_fts("Python", None, None).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.content.contains("Python")));

        // Search with session filter
        let filtered = db
            .search_messages_fts("install", Some(session_id), None)
            .unwrap();
        assert!(!filtered.is_empty());

        // Search with no results
        let empty = db
            .search_messages_fts("xyznonexistent12345", None, None)
            .unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_search_cjk_fallback() {
        let db = test_db();
        let session_id = "test-session";

        db.save_message(
            session_id,
            "user",
            "你好世界，这是一个测试",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_message(
            session_id,
            "user",
            "한글 테스트입니다",
            "2024-01-01T00:00:01Z",
        )
        .unwrap();
        db.save_message(
            session_id,
            "user",
            "普通の日本語テキスト",
            "2024-01-01T00:00:02Z",
        )
        .unwrap();
        db.save_message(
            session_id,
            "user",
            "English message for comparison",
            "2024-01-01T00:00:03Z",
        )
        .unwrap();

        // CJK search should use LIKE fallback
        let results = db.search_messages_fts("测试", None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("测试"));

        // Korean search
        let results = db.search_messages_fts("테스트", None, None).unwrap();
        assert_eq!(results.len(), 1);

        // Japanese search
        let results = db.search_messages_fts("日本語", None, None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_session_management() {
        let db = test_db();
        let session_id = "test-session";

        // Update title
        db.update_session_title(session_id, "New Title").unwrap();
        let sessions = db.get_recent_sessions(10).unwrap();
        let found = sessions.iter().find(|s| s.id == session_id).unwrap();
        assert_eq!(found.title.as_deref(), Some("New Title"));

        // Count
        let count = db.get_session_count().unwrap();
        assert!(count >= 1);
    }

    #[test]
    fn test_merge_sessions() {
        let db = test_db();
        let target_id = "target-session";
        let source_id = "source-session";

        // Create two sessions
        db.save_session(
            target_id,
            Some("target"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_session(
            source_id,
            Some("source"),
            "test",
            "2024-01-01T00:00:01Z",
            "2024-01-01T00:00:01Z",
        )
        .unwrap();

        // Add messages and tags to source
        db.save_message(source_id, "user", "source message", "2024-01-01T00:00:02Z")
            .unwrap();
        db.add_session_tag(source_id, "source-tag").unwrap();

        // Merge
        db.merge_sessions(target_id, &[source_id]).unwrap();

        // Source session should be deleted (test_db creates a default session too)
        let count = db.get_session_count().unwrap();
        assert_eq!(count, 2); // default test session + target remains

        // Messages moved to target
        let msgs = db.get_session_messages(target_id).unwrap();
        assert_eq!(msgs.len(), 1); // original 0 + moved 1
        assert_eq!(msgs[0].content, "source message");

        // Tags merged
        let tags = db.get_session_tags(target_id).unwrap();
        assert!(tags.contains(&"source-tag".to_string()));
    }

    #[test]
    fn test_session_tags_in_summary() {
        let db = test_db();
        let session_id = "test-session";

        db.add_session_tag(session_id, "tag-a").unwrap();
        db.add_session_tag(session_id, "tag-b").unwrap();

        // Check tags appear in recent sessions
        let sessions = db.get_recent_sessions(10).unwrap();
        let s = sessions.iter().find(|s| s.id == session_id).unwrap();
        assert_eq!(s.tags.len(), 2);
        assert!(s.tags.contains(&"tag-a".to_string()));
    }

    #[test]
    fn test_with_retry_ok() {
        // Basic test that with_retry works on a successful operation
        let result = with_retry(|| Ok::<_, rusqlite::Error>(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_contains_cjk() {
        assert!(contains_cjk("测试"));
        assert!(contains_cjk("日本語"));
        assert!(contains_cjk("한글"));
        assert!(!contains_cjk("English"));
        assert!(!contains_cjk(""));
        assert!(contains_cjk("mixed English 测试"));
    }

    #[test]
    fn test_wal_mode_init() {
        let (db, path) = test_db_path();
        assert!(path.exists(), "Database file should exist");
        // Verify the db is functional
        let count = db.get_session_count().unwrap();
        assert_eq!(count, 0);
        // Cleanup temp directory
        let dir = path.parent().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_delete_session_cascade() {
        let db = test_db();
        let session_id = "test-session";

        // Add metadata, tool state, tags, events
        db.set_session_metadata(session_id, "k", "v").unwrap();
        db.set_tool_state(session_id, "tool1", &serde_json::json!({"x": 1}))
            .unwrap();
        db.add_session_tag(session_id, "mytag").unwrap();
        db.record_event(session_id, "test_event", &serde_json::json!({"ok": true}))
            .unwrap();

        // Delete session
        db.delete_session(session_id).unwrap();

        // Verify cascade: metadata, tags, events, tools_state all gone
        let meta = db.get_all_session_metadata(session_id).unwrap();
        assert!(meta.is_empty());
        let tags = db.get_session_tags(session_id).unwrap();
        assert!(tags.is_empty());
        let events = db.get_session_events(session_id, None).unwrap();
        assert!(events.is_empty());
        assert!(db.get_tool_state(session_id, "tool1").is_none());
    }

    // === New Tests for Expanded Schema ===

    #[test]
    fn test_schema_version_tracking() {
        let db = test_db();
        let conn = db.conn.lock().unwrap();
        let version: i64 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_save_and_get_session_full() {
        let db = test_db();
        // Delete the test session first
        db.delete_session("test-session").unwrap();

        let session = SessionData {
            id: "full-session".to_string(),
            source: "test".to_string(),
            user_id: Some("user123".to_string()),
            model: Some("gpt-4".to_string()),
            model_config: Some("{\"temperature\": 0.7}".to_string()),
            system_prompt: Some("You are helpful.".to_string()),
            parent_session_id: None,
            started_at: "2024-01-01T00:00:00Z".to_string(),
            ended_at: Some("2024-01-01T01:00:00Z".to_string()),
            end_reason: Some("completed".to_string()),
            message_count: 10,
            tool_call_count: 5,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cache_write_tokens: 50,
            reasoning_tokens: 200,
            cwd: Some("/home/user/project".to_string()),
            billing_provider: Some("openai".to_string()),
            billing_base_url: None,
            billing_mode: Some("api_key".to_string()),
            estimated_cost_usd: Some(0.05),
            actual_cost_usd: Some(0.048),
            cost_status: Some("final".to_string()),
            cost_source: Some("usage_api".to_string()),
            pricing_version: Some("v1".to_string()),
            title: Some("Test Session".to_string()),
            api_call_count: 15,
            handoff_state: None,
            handoff_platform: None,
            handoff_error: None,
            rewind_count: 0,
            archived: 0,
        };

        db.save_session_full(&session).unwrap();

        // Verify by reading back with raw query
        let conn = db.conn.lock().unwrap();
        let retrieved: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT model, model_config, system_prompt FROM sessions WHERE id = ?1",
                params!["full-session"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(retrieved.0, Some("gpt-4".to_string()));
        assert_eq!(retrieved.1, Some("{\"temperature\": 0.7}".to_string()));
        assert_eq!(retrieved.2, Some("You are helpful.".to_string()));
    }

    #[test]
    fn test_save_and_get_message_full() {
        let db = test_db();
        let session_id = "test-session";

        let message = MessageData {
            id: 0,
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            content: Some("Here is the result.".to_string()),
            tool_call_id: Some("call_abc123".to_string()),
            tool_calls: Some("[{\"name\": \"bash\", \"args\": {\"command\": \"ls\"}}]".to_string()),
            tool_name: Some("bash".to_string()),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            token_count: Some(50),
            finish_reason: Some("tool_calls".to_string()),
            reasoning: Some("Thinking about the problem...".to_string()),
            reasoning_content: None,
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: Some(0),
            active: 1,
        };

        db.save_message_full(&message).unwrap();

        // Get full messages
        let messages = db.get_session_messages_full(session_id).unwrap();
        assert!(!messages.is_empty());
        let msg = &messages[messages.len() - 1];
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_abc123"));
        assert_eq!(msg.tool_name.as_deref(), Some("bash"));
        assert_eq!(msg.token_count, Some(50));
        assert_eq!(msg.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            msg.reasoning.as_deref(),
            Some("Thinking about the problem...")
        );
    }

    #[test]
    fn test_compression_locks() {
        let db = test_db();
        let session_id = "test-session";

        // Initially not locked
        assert!(!db.is_compression_locked(session_id));

        // Acquire lock
        let now = chrono::Utc::now().to_rfc3339();
        let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        db.acquire_compression_lock(session_id, "compressor-1", &now, &expires)
            .unwrap();

        // Now locked
        assert!(db.is_compression_locked(session_id));

        // Release lock
        db.release_compression_lock(session_id).unwrap();
        assert!(!db.is_compression_locked(session_id));
    }

    #[test]
    fn test_state_meta() {
        let db = test_db();

        // Set and get
        db.set_state_meta("last_sync", "2024-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(
            db.get_state_meta("last_sync"),
            Some("2024-01-01T00:00:00Z".to_string())
        );

        // Update
        db.set_state_meta("last_sync", "2024-01-02T00:00:00Z")
            .unwrap();
        assert_eq!(
            db.get_state_meta("last_sync"),
            Some("2024-01-02T00:00:00Z".to_string())
        );

        // Nonexistent
        assert_eq!(db.get_state_meta("nonexistent"), None);
    }

    #[test]
    fn test_reconcile_columns_adds_missing() {
        // Create a DB with old schema, then init with new schema
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_test_reconcile_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&path);

        // Create with minimal schema
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, source TEXT NOT NULL, started_at TEXT NOT NULL, ended_at TEXT NOT NULL);
                 CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, timestamp TEXT NOT NULL);",
            )
            .unwrap();
            conn.close().unwrap();
        }

        // Now init with full schema — should reconcile columns
        let db = Database::init(path.clone()).unwrap();

        // Verify new columns exist by using save_session_full
        let session = SessionData {
            id: "reconcile-test".to_string(),
            source: "test".to_string(),
            model: Some("gpt-4".to_string()),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            ended_at: Some("2024-01-01T01:00:00Z".to_string()),
            ..Default::default()
        };
        db.save_session_full(&session).unwrap();

        // Verify the model was saved
        let model: Option<String> = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT model FROM sessions WHERE id = ?1",
                params!["reconcile-test"],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(model, Some("gpt-4".to_string()));

        // Cleanup
        drop(db);
        std::fs::remove_file(&path).ok();
    }

    // === Checkpoint CRUD ===

    #[test]
    fn test_checkpoint_store_and_list() {
        let db = test_db();
        let dir = std::env::temp_dir().join(format!("ckpt_dir_{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        db.store_checkpoint(
            "abc123hash",
            "2024-06-01T12:00:00Z",
            Some("test checkpoint"),
            dir.to_str().unwrap(),
        )
        .unwrap();
        let list = db.list_checkpoints(dir.to_str().unwrap()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].hash, "abc123hash");
        assert_eq!(list[0].reason.as_deref(), Some("test checkpoint"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_get_by_hash() {
        let db = test_db();
        let dir = std::env::temp_dir().join(format!("ckpt_get_{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        db.store_checkpoint(
            "hash_get",
            "2024-06-01T12:00:00Z",
            Some("reason"),
            dir.to_str().unwrap(),
        )
        .unwrap();
        let ckpt = db.get_checkpoint("hash_get").unwrap();
        assert!(ckpt.is_some());
        assert_eq!(ckpt.unwrap().hash, "hash_get");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_delete() {
        let db = test_db();
        let dir = std::env::temp_dir().join(format!("ckpt_del_{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        db.store_checkpoint(
            "hash_del",
            "2024-06-01T12:00:00Z",
            Some("to delete"),
            dir.to_str().unwrap(),
        )
        .unwrap();
        db.delete_checkpoint("hash_del").unwrap();
        let ckpt = db.get_checkpoint("hash_del").unwrap();
        assert!(ckpt.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_list_empty() {
        let db = test_db();
        let dir = std::env::temp_dir().join(format!("ckpt_empty_{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let list = db.list_checkpoints(dir.to_str().unwrap()).unwrap();
        assert!(list.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_duplicate_overwrite() {
        let db = test_db();
        let dir = std::env::temp_dir().join(format!("ckpt_dup_{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        db.store_checkpoint(
            "same_hash",
            "2024-06-01T12:00:00Z",
            Some("first"),
            dir.to_str().unwrap(),
        )
        .unwrap();
        db.store_checkpoint(
            "same_hash",
            "2024-06-02T12:00:00Z",
            Some("second"),
            dir.to_str().unwrap(),
        )
        .unwrap();
        let list = db.list_checkpoints(dir.to_str().unwrap()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].reason.as_deref(), Some("second"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // === Session-level search ===

    #[test]
    fn test_search_sessions_basic() {
        let db = test_db();
        db.save_session(
            "s1",
            Some("Alpha Project"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_message(
            "s1",
            "user",
            "alpha unique searchable content",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        let results = db.search_sessions("alpha", 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_sessions_no_match() {
        let db = test_db();
        db.save_session(
            "s1",
            Some("Unique Name"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_message("s1", "user", "hello world", "2024-01-01T00:00:00Z")
            .unwrap();
        let results = db.search_sessions("zzz_nonexistent_zzz", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_sessions_limit() {
        let db = test_db();
        for i in 0..10 {
            let id = format!("s{}", i);
            db.save_session(
                &id,
                Some(&format!("Session {}", i)),
                "test",
                "2024-01-01T00:00:00Z",
                "2024-01-01T00:00:00Z",
            )
            .unwrap();
            db.save_message(&id, "user", "alpha searchable", "2024-01-01T00:00:00Z")
                .unwrap();
        }
        let results = db.search_sessions("alpha", 3).unwrap();
        assert!(results.len() <= 3);
    }

    // === Event filtering ===

    #[test]
    fn test_get_events_by_type() {
        let db = test_db();
        db.record_event(
            "test-session",
            "tool_call",
            &serde_json::json!({"tool": "bash"}),
        )
        .unwrap();
        db.record_event(
            "test-session",
            "llm_call",
            &serde_json::json!({"model": "gpt-4"}),
        )
        .unwrap();
        db.record_event(
            "test-session",
            "tool_call",
            &serde_json::json!({"tool": "ls"}),
        )
        .unwrap();
        let tool_events = db.get_events_by_type("tool_call", None).unwrap();
        assert_eq!(tool_events.len(), 2);
        let llm_events = db.get_events_by_type("llm_call", None).unwrap();
        assert_eq!(llm_events.len(), 1);
    }

    #[test]
    fn test_record_event_and_get() {
        let db = test_db();
        db.record_event(
            "test-session",
            "test_event",
            &serde_json::json!({"key": "value"}),
        )
        .unwrap();
        let events = db.get_session_events("test-session", Some(10)).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "test_event");
    }

    // === Session title update ===

    #[test]
    fn test_update_session_title() {
        let db = test_db();
        db.update_session_title("test-session", "New Title")
            .unwrap();
        let sessions = db.list_sessions(10).unwrap();
        let s = sessions.iter().find(|s| s.id == "test-session").unwrap();
        assert_eq!(s.title.as_deref(), Some("New Title"));
    }

    // === Tag operations ===

    #[test]
    fn test_add_remove_tags() {
        let db = test_db();
        db.add_session_tag("test-session", "rust").unwrap();
        db.add_session_tag("test-session", "testing").unwrap();
        let tags = db.get_session_tags("test-session").unwrap();
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"testing".to_string()));
        db.remove_session_tag("test-session", "rust").unwrap();
        let tags = db.get_session_tags("test-session").unwrap();
        assert!(!tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"testing".to_string()));
    }

    #[test]
    fn test_find_sessions_by_tag() {
        let db = test_db();
        db.add_session_tag("test-session", "searchable").unwrap();
        let found = db.find_sessions_by_tag("searchable").unwrap();
        assert!(!found.is_empty());
    }

    #[test]
    fn test_duplicate_tags() {
        let db = test_db();
        db.add_session_tag("test-session", "dup").unwrap();
        db.add_session_tag("test-session", "dup").unwrap();
        let tags = db.get_session_tags("test-session").unwrap();
        assert_eq!(tags.iter().filter(|t| t.as_str() == "dup").count(), 1);
    }

    // === Merge sessions ===

    #[test]
    fn test_merge_sessions_basic() {
        let db = test_db();
        db.save_session(
            "merge_source",
            Some("Source"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_message("merge_source", "user", "hello", "2024-01-01T00:00:00Z")
            .unwrap();
        db.save_message("merge_source", "assistant", "hi", "2024-01-01T00:00:01Z")
            .unwrap();
        db.merge_sessions("test-session", &["merge_source"])
            .unwrap();
        let msgs = db.get_session_messages("test-session").unwrap();
        assert!(msgs.len() >= 2);
    }

    // === Compression lock ===

    #[test]
    fn test_compression_lock_acquire_release() {
        let db = test_db();
        db.acquire_compression_lock(
            "test-session",
            "holder1",
            "2024-01-01T00:00:00Z",
            "2099-01-01T00:00:00Z",
        )
        .unwrap();
        assert!(db.is_compression_locked("test-session"));
        db.release_compression_lock("test-session").unwrap();
        assert!(!db.is_compression_locked("test-session"));
    }

    // === State meta ===

    #[test]
    fn test_set_get_state_meta() {
        let db = test_db();
        db.set_state_meta("last_model", "gpt-4").unwrap();
        let val = db.get_state_meta("last_model");
        assert_eq!(val.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn test_state_meta_overwrite() {
        let db = test_db();
        db.set_state_meta("key", "v1").unwrap();
        db.set_state_meta("key", "v2").unwrap();
        let val = db.get_state_meta("key");
        assert_eq!(val.as_deref(), Some("v2"));
    }

    // === FTS search ===

    #[test]
    fn test_search_messages_fts_basic() {
        let db = test_db();
        db.save_message(
            "test-session",
            "user",
            "the quick brown fox",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        let results = db
            .search_messages_fts("quick brown", None, Some(10))
            .unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_messages_fts_no_match() {
        let db = test_db();
        db.save_message(
            "test-session",
            "user",
            "hello world",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        let results = db
            .search_messages_fts("xyzzy_nonexistent", None, Some(10))
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_messages_fts_with_session_filter() {
        let db = test_db();
        db.save_message(
            "test-session",
            "user",
            "unique_term_alpha",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_session(
            "other",
            Some("Other"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        db.save_message("other", "user", "unique_term_alpha", "2024-01-01T00:00:00Z")
            .unwrap();
        let results = db
            .search_messages_fts("unique_term_alpha", Some("test-session"), Some(10))
            .unwrap();
        assert!(results.iter().all(|r| r.session_id == "test-session"));
    }

    // === Recent sessions ===

    #[test]
    fn test_get_recent_sessions() {
        let db = test_db();
        for i in 0..5 {
            db.save_session(
                &format!("recent_{}", i),
                Some(&format!("Session {}", i)),
                "test",
                "2024-01-01T00:00:00Z",
                "2024-01-01T00:00:00Z",
            )
            .unwrap();
        }
        let recent = db.get_recent_sessions(3).unwrap();
        assert!(recent.len() <= 3);
    }

    #[test]
    fn test_get_recent_sessions_empty() {
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_test_recent_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&path);
        let db = Database::init(path).unwrap();
        let recent = db.get_recent_sessions(10).unwrap();
        assert!(recent.is_empty());
    }

    // === Session count ===

    #[test]
    fn test_get_session_count() {
        let db = test_db();
        let before = db.get_session_count().unwrap();
        db.save_session(
            "count_new",
            Some("Count"),
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
        let after = db.get_session_count().unwrap();
        assert_eq!(after, before + 1);
    }

    // === Reconcile columns ===

    #[test]
    fn test_reconcile_adds_missing_column() {
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_test_reconcile_extra_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER);
                 CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, title TEXT, source TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, timestamp TEXT NOT NULL);",
            ).unwrap();
            conn.close().unwrap();
        }
        let db = Database::init(path.clone()).unwrap();
        let session = SessionData {
            id: "reconcile2".to_string(),
            source: "test".to_string(),
            model: Some("test-model".to_string()),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        db.save_session_full(&session).unwrap();
        let model: Option<String> = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT model FROM sessions WHERE id = ?1",
                params!["reconcile2"],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(model, Some("test-model".to_string()));
        drop(db);
        std::fs::remove_file(&path).ok();
    }

    // === Full session roundtrip ===

    #[test]
    fn test_save_get_session_full_roundtrip() {
        let db = test_db();
        db.delete_session("test-session").unwrap();
        let session = SessionData {
            id: "roundtrip".to_string(),
            source: "rt_test".to_string(),
            user_id: Some("user_rt".to_string()),
            model: Some("gpt-4-rt".to_string()),
            model_config: Some("{\"temp\": 0.5}".to_string()),
            system_prompt: Some("Be helpful".to_string()),
            parent_session_id: None,
            started_at: "2024-06-01T00:00:00Z".to_string(),
            ended_at: Some("2024-06-01T01:00:00Z".to_string()),
            end_reason: Some("completed".to_string()),
            message_count: 25,
            tool_call_count: 12,
            input_tokens: 5000,
            output_tokens: 2500,
            cache_read_tokens: 500,
            cache_write_tokens: 250,
            reasoning_tokens: 1000,
            cwd: Some("/workspace".to_string()),
            billing_provider: Some("openai".to_string()),
            billing_base_url: None,
            billing_mode: Some("api_key".to_string()),
            estimated_cost_usd: Some(0.15),
            actual_cost_usd: Some(0.148),
            cost_status: Some("final".to_string()),
            cost_source: Some("usage_api".to_string()),
            pricing_version: Some("v2".to_string()),
            title: Some("RT Session".to_string()),
            api_call_count: 30,
            handoff_state: None,
            handoff_platform: None,
            handoff_error: None,
            rewind_count: 2,
            archived: 0,
        };
        db.save_session_full(&session).unwrap();
        let conn = db.conn.lock().unwrap();
        let model: Option<String> = conn
            .query_row(
                "SELECT model FROM sessions WHERE id = 'roundtrip'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(model, Some("gpt-4-rt".to_string()));
        let tokens: i64 = conn
            .query_row(
                "SELECT input_tokens FROM sessions WHERE id = 'roundtrip'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tokens, 5000);
    }

    // === Full message roundtrip ===

    #[test]
    fn test_save_get_message_full_roundtrip() {
        let db = test_db();
        let msg = MessageData {
            id: 0,
            session_id: "test-session".to_string(),
            role: "assistant".to_string(),
            content: Some("RT message content".to_string()),
            tool_call_id: Some("call_rt123".to_string()),
            tool_calls: Some("[]".to_string()),
            tool_name: Some("test_tool".to_string()),
            timestamp: "2024-06-01T00:00:00Z".to_string(),
            token_count: Some(100),
            finish_reason: Some("stop".to_string()),
            reasoning: None,
            reasoning_content: Some("thinking...".to_string()),
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: None,
            active: 1,
        };
        db.save_message_full(&msg).unwrap();
        let msgs = db.get_session_messages_full("test-session").unwrap();
        let found = msgs
            .iter()
            .find(|m| m.content.as_deref() == Some("RT message content"));
        assert!(found.is_some());
        let m = found.unwrap();
        assert_eq!(m.tool_call_id.as_deref(), Some("call_rt123"));
        assert_eq!(m.tool_name.as_deref(), Some("test_tool"));
        assert_eq!(m.token_count, Some(100));
    }

    // === Tool state tests ===

    #[test]
    fn test_tool_state_set_get_clear() {
        let db = test_db();
        db.set_tool_state(
            "test-session",
            "my_tool",
            &serde_json::json!({"status": "active"}),
        )
        .unwrap();
        let state = db.get_tool_state("test-session", "my_tool");
        assert!(state.is_some());
        assert_eq!(state.unwrap()["status"], "active");
        db.clear_tool_state("test-session", "my_tool").unwrap();
        assert!(db.get_tool_state("test-session", "my_tool").is_none());
    }

    #[test]
    fn test_clear_all_tool_states() {
        let db = test_db();
        db.set_tool_state("test-session", "t1", &serde_json::json!({"a": 1}))
            .unwrap();
        db.set_tool_state("test-session", "t2", &serde_json::json!({"b": 2}))
            .unwrap();
        db.clear_all_tool_states("test-session").unwrap();
        assert!(db.get_tool_state("test-session", "t1").is_none());
        assert!(db.get_tool_state("test-session", "t2").is_none());
    }

    // === Metadata edge cases ===

    #[test]
    fn test_get_metadata_nonexistent() {
        let db = test_db();
        let val = db.get_session_metadata("test-session", "no_such_key");
        assert!(val.is_none());
    }

    #[test]
    fn test_delete_metadata() {
        let db = test_db();
        db.set_session_metadata("test-session", "del_key", "del_val")
            .unwrap();
        db.delete_session_metadata("test-session", "del_key")
            .unwrap();
        assert!(db.get_session_metadata("test-session", "del_key").is_none());
    }

    // === Path ===

    #[test]
    fn test_database_path() {
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_test_path_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&path);
        let db = Database::init(path.clone()).unwrap();
        assert_eq!(db.path(), Some(path));
    }

    // === Active sessions ===

    #[test]
    fn test_get_active_sessions() {
        let db = test_db();
        let now = chrono::Utc::now().to_rfc3339();
        db.save_session("active_1", Some("Active"), "test", &now, &now)
            .unwrap();
        let active = db.get_active_sessions(60 * 24 * 365).unwrap();
        assert!(!active.is_empty());
    }

    // === Update title edge cases ===

    #[test]
    fn test_update_session_title_empty() {
        let db = test_db();
        db.update_session_title("test-session", "").unwrap();
        let sessions = db.list_sessions(10).unwrap();
        let s = sessions.iter().find(|s| s.id == "test-session").unwrap();
        assert_eq!(s.title.as_deref(), Some(""));
    }
}
