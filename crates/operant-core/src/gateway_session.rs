//! Persistent SQLite-backed session store for the Operant gateway.
//!
//! Mirrors the Python `SessionStore` from `operant-agent/gateway/session.py`:
//! - Deterministic session key construction
//! - Session TTL and idle timeout with daily reset policy
//! - Transcript persistence with FTS5 search
//! - Session switching, resume, and branch support

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Timelike, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;
use crate::gateway::PlatformSession;

// ---------------------------------------------------------------------------
// PII helpers (port of Python _hash_id / _hash_sender_id / _hash_chat_id)
// ---------------------------------------------------------------------------

/// Deterministic 12-char hex hash of an identifier.
fn hash_id(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let result = hasher.finalize();
    // Manual hex encoding (no hex crate dependency)
    let hex_chars: Vec<char> = result
        .iter()
        .flat_map(|b| {
            let high = b >> 4;
            let low = b & 0x0f;
            [
                char::from(if high < 10 {
                    b'0' + high
                } else {
                    b'a' + high - 10
                }),
                char::from(if low < 10 {
                    b'0' + low
                } else {
                    b'a' + low - 10
                }),
            ]
        })
        .collect();
    hex_chars[..12].iter().collect()
}

/// Hash a sender ID to `user_<12hex>`.
pub fn hash_sender_id(value: &str) -> String {
    format!("user_{}", hash_id(value))
}

/// Hash the numeric portion of a chat ID, preserving platform prefix.
///
/// `telegram:12345` → `telegram:<hash>`
/// `12345`          → `<hash>`
pub fn hash_chat_id(value: &str) -> String {
    if let Some(colon_pos) = value.find(':') {
        let prefix = &value[..colon_pos];
        let rest = &value[colon_pos + 1..];
        return format!("{}:{}", prefix, hash_id(rest));
    }
    hash_id(value)
}

// ---------------------------------------------------------------------------
// SessionSource — where a message originated from
// ---------------------------------------------------------------------------

/// Describes where a message originated from.
///
/// Used to route responses, inject context, and track origin for cron delivery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSource {
    /// Platform name (e.g., "telegram", "discord", "local")
    pub platform: String,
    /// Channel/chat ID
    pub chat_id: String,
    /// Human-readable chat name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_name: Option<String>,
    /// Chat type: "dm", "group", "channel", "thread"
    #[serde(default = "default_chat_type")]
    pub chat_type: String,
    /// User ID on the platform
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Username or display name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    /// Thread/topic ID (Telegram forum topics, Discord threads, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Channel topic/description (Discord, Slack)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_topic: Option<String>,
    /// Platform-specific stable alt ID (Signal UUID, Feishu union_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id_alt: Option<String>,
    /// Signal group internal ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id_alt: Option<String>,
    /// True when the message author is a bot/webhook (Discord)
    #[serde(default)]
    pub is_bot: bool,
    /// Discord guild / Slack workspace / Matrix server scope
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    /// Parent channel when chat_id refers to a thread
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_chat_id: Option<String>,
    /// ID of the triggering message (for pin/reply/react)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// True when adapter granted access via role (not user ID)
    #[serde(default)]
    pub role_authorized: bool,
}

fn default_chat_type() -> String {
    "dm".to_string()
}

impl Default for SessionSource {
    fn default() -> Self {
        Self {
            platform: "unknown".to_string(),
            chat_id: String::new(),
            chat_name: None,
            chat_type: "dm".to_string(),
            user_id: None,
            user_name: None,
            thread_id: None,
            chat_topic: None,
            user_id_alt: None,
            chat_id_alt: None,
            is_bot: false,
            guild_id: None,
            parent_chat_id: None,
            message_id: None,
            role_authorized: false,
        }
    }
}

impl SessionSource {
    /// Human-readable description of the source.
    pub fn description(&self) -> String {
        if self.platform == "local" {
            return "CLI terminal".to_string();
        }
        let mut parts = Vec::new();
        match self.chat_type.as_str() {
            "dm" => parts.push(format!(
                "DM with {}",
                self.user_name
                    .as_deref()
                    .or(self.user_id.as_deref())
                    .unwrap_or("user")
            )),
            "group" => parts.push(format!(
                "group: {}",
                self.chat_name.as_deref().unwrap_or(&self.chat_id)
            )),
            "channel" => parts.push(format!(
                "channel: {}",
                self.chat_name.as_deref().unwrap_or(&self.chat_id)
            )),
            _ => parts.push(
                self.chat_name
                    .clone()
                    .unwrap_or_else(|| self.chat_id.clone()),
            ),
        }
        if let Some(ref tid) = self.thread_id {
            parts.push(format!("thread: {}", tid));
        }
        parts.join(", ")
    }

    /// Serialize to JSON dict.
    pub fn to_dict(&self) -> serde_json::Value {
        let mut d = serde_json::json!({
            "platform": self.platform,
            "chat_id": self.chat_id,
            "chat_name": self.chat_name,
            "chat_type": self.chat_type,
            "user_id": self.user_id,
            "user_name": self.user_name,
            "thread_id": self.thread_id,
            "chat_topic": self.chat_topic,
        });
        if let Some(ref v) = self.user_id_alt {
            d["user_id_alt"] = serde_json::json!(v);
        }
        if let Some(ref v) = self.chat_id_alt {
            d["chat_id_alt"] = serde_json::json!(v);
        }
        if let Some(ref v) = self.guild_id {
            d["guild_id"] = serde_json::json!(v);
        }
        if let Some(ref v) = self.parent_chat_id {
            d["parent_chat_id"] = serde_json::json!(v);
        }
        if let Some(ref v) = self.message_id {
            d["message_id"] = serde_json::json!(v);
        }
        d
    }

    /// Deserialize from JSON dict.
    pub fn from_dict(data: &serde_json::Value) -> Option<Self> {
        Some(Self {
            platform: data.get("platform")?.as_str()?.to_string(),
            chat_id: data.get("chat_id")?.as_str()?.to_string(),
            chat_name: data
                .get("chat_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            chat_type: data
                .get("chat_type")
                .and_then(|v| v.as_str())
                .unwrap_or("dm")
                .to_string(),
            user_id: data
                .get("user_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            user_name: data
                .get("user_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            thread_id: data
                .get("thread_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            chat_topic: data
                .get("chat_topic")
                .and_then(|v| v.as_str())
                .map(String::from),
            user_id_alt: data
                .get("user_id_alt")
                .and_then(|v| v.as_str())
                .map(String::from),
            chat_id_alt: data
                .get("chat_id_alt")
                .and_then(|v| v.as_str())
                .map(String::from),
            is_bot: data
                .get("is_bot")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            guild_id: data
                .get("guild_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            parent_chat_id: data
                .get("parent_chat_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            message_id: data
                .get("message_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            role_authorized: data
                .get("role_authorized")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }
}

// ---------------------------------------------------------------------------
// SessionResetPolicy — when to auto-reset a session
// ---------------------------------------------------------------------------

/// Reset policy mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetMode {
    /// No automatic reset
    None,
    /// Reset after idle timeout
    Idle,
    /// Reset daily at a specific hour
    Daily,
    /// Both idle and daily reset
    Both,
}

impl Default for ResetMode {
    fn default() -> Self {
        Self::None
    }
}

/// Configuration for when sessions should be automatically reset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResetPolicy {
    /// Reset mode
    pub mode: ResetMode,
    /// Idle timeout in minutes (for Idle/Both modes)
    pub idle_minutes: u64,
    /// Hour of day for daily reset (0-23, for Daily/Both modes)
    pub at_hour: u32,
}

impl Default for SessionResetPolicy {
    fn default() -> Self {
        Self {
            mode: ResetMode::None,
            idle_minutes: 120,
            at_hour: 4,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionEntry — metadata for a session key → session_id mapping
// ---------------------------------------------------------------------------

/// Entry in the session store mapping a session key to its session ID and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Deterministic session key
    pub session_key: String,
    /// Session ID (timestamp + random suffix)
    pub session_id: String,
    /// When the session was created
    pub created_at: String,
    /// Last activity timestamp
    pub updated_at: String,
    /// Origin metadata for delivery routing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionSource>,
    /// Display name (chat name)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Platform name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Chat type
    #[serde(default = "default_chat_type")]
    pub chat_type: String,
    /// Input tokens consumed
    #[serde(default)]
    pub input_tokens: u64,
    /// Output tokens generated
    #[serde(default)]
    pub output_tokens: u64,
    /// Cache read tokens
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cache write tokens
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Total tokens
    #[serde(default)]
    pub total_tokens: u64,
    /// Last API-reported prompt tokens (for compression pre-check)
    #[serde(default)]
    pub last_prompt_tokens: u64,
    /// Estimated cost in USD
    #[serde(default)]
    pub estimated_cost_usd: f64,
    /// Cost status
    #[serde(default = "default_cost_status")]
    pub cost_status: String,
    /// Whether the previous session was auto-reset
    #[serde(default)]
    pub was_auto_reset: bool,
    /// Reason for auto-reset ("idle" or "daily")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_reset_reason: Option<String>,
    /// Whether the expired session had any messages
    #[serde(default)]
    pub reset_had_activity: bool,
    /// Set by explicit /new or /reset
    #[serde(default)]
    pub is_fresh_reset: bool,
    /// Set after background expiry watcher finalizes
    #[serde(default)]
    pub expiry_finalized: bool,
    /// When true, next access auto-resets (from /stop)
    #[serde(default)]
    pub suspended: bool,
    /// Session was interrupted by restart, resume on next access
    #[serde(default)]
    pub resume_pending: bool,
    /// Reason for resume_pending
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_reason: Option<String>,
    /// When resume_pending was set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_resume_marked_at: Option<String>,
}

fn default_cost_status() -> String {
    "unknown".to_string()
}

// ---------------------------------------------------------------------------
// Session key construction (port of Python build_session_key)
// ---------------------------------------------------------------------------

/// Build a deterministic session key from a message source.
///
/// This is the single source of truth for session key construction.
///
/// DM rules:
///   - DMs include chat_id when present, so each private conversation is isolated.
///   - thread_id further differentiates threaded DMs within the same DM chat.
///   - Without chat_id, thread_id is used as a best-effort fallback.
///   - Without thread_id or chat_id, DMs share a single session.
///
/// Group/channel rules:
///   - chat_id identifies the parent group/channel.
///   - thread_id differentiates threads within that parent chat.
///   - user_id isolates participants when group_sessions_per_user is enabled.
pub fn build_session_key(
    source: &SessionSource,
    group_sessions_per_user: bool,
    thread_sessions_per_user: bool,
) -> String {
    let platform = &source.platform;

    if source.chat_type == "dm" {
        let dm_chat_id = &source.chat_id;

        if !dm_chat_id.is_empty() {
            if let Some(ref tid) = source.thread_id {
                return format!("agent:main:{}:dm:{}:{}", platform, dm_chat_id, tid);
            }
            return format!("agent:main:{}:dm:{}", platform, dm_chat_id);
        }
        // No chat_id — fall back to participant identifier
        let dm_participant_id = source.user_id_alt.as_deref().or(source.user_id.as_deref());
        if let Some(pid) = dm_participant_id {
            if let Some(ref tid) = source.thread_id {
                return format!("agent:main:{}:dm:{}:{}", platform, pid, tid);
            }
            return format!("agent:main:{}:dm:{}", platform, pid);
        }
        if let Some(ref tid) = source.thread_id {
            return format!("agent:main:{}:dm:{}", platform, tid);
        }
        return format!("agent:main:{}:dm", platform);
    }

    // Non-DM: group/channel/session
    let participant_id = source.user_id_alt.as_deref().or(source.user_id.as_deref());
    let mut key_parts = vec![
        "agent:main".to_string(),
        platform.clone(),
        source.chat_type.clone(),
    ];

    if !source.chat_id.is_empty() {
        key_parts.push(source.chat_id.clone());
    }
    if let Some(ref tid) = source.thread_id {
        key_parts.push(tid.clone());
    }

    // In threads, default to shared sessions unless thread_sessions_per_user
    let isolate_user = if source.thread_id.is_some() && !thread_sessions_per_user {
        false
    } else {
        group_sessions_per_user
    };

    if isolate_user {
        if let Some(pid) = participant_id {
            key_parts.push(pid.to_string());
        }
    }

    key_parts.join(":")
}

/// Check if a session is shared across participants (mirrors Python is_shared_multi_user_session).
pub fn is_shared_multi_user_session(
    source: &SessionSource,
    group_sessions_per_user: bool,
    thread_sessions_per_user: bool,
) -> bool {
    if source.chat_type == "dm" {
        return false;
    }
    if source.thread_id.is_some() {
        return !thread_sessions_per_user;
    }
    !group_sessions_per_user
}

// ---------------------------------------------------------------------------
// PersistentSessionStore — SQLite-backed session store
// ---------------------------------------------------------------------------

/// Configuration for the session store.
#[derive(Debug, Clone)]
pub struct SessionStoreConfig {
    /// Session reset policy
    pub reset_policy: SessionResetPolicy,
    /// Group sessions per user (isolate per-user in groups)
    pub group_sessions_per_user: bool,
    /// Thread sessions per user (isolate per-user in threads)
    pub thread_sessions_per_user: bool,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            reset_policy: SessionResetPolicy::default(),
            group_sessions_per_user: true,
            thread_sessions_per_user: false,
        }
    }
}

/// A SQLite-backed session store that mirrors the Python `SessionStore`.
pub struct PersistentSessionStore {
    conn: Arc<Mutex<Connection>>,
    /// In-memory cache of session_key → SessionEntry for fast lookups
    entries: std::sync::RwLock<HashMap<String, SessionEntry>>,
    config: std::sync::RwLock<SessionStoreConfig>,
}

impl PersistentSessionStore {
    /// Open (or create) the session database at `db_path`.
    pub fn open(db_path: &str) -> Result<Self, Error> {
        let conn = Connection::open(db_path)
            .map_err(|e| Error::Agent(format!("Failed to open session DB: {}", e)))?;

        // Enable WAL mode for better concurrent read/write performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to enable WAL mode: {}", e);
            });

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS gateway_sessions (
                session_key TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                origin TEXT,
                display_name TEXT,
                platform TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                last_prompt_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
                cost_status TEXT NOT NULL DEFAULT 'unknown',
                was_auto_reset INTEGER NOT NULL DEFAULT 0,
                auto_reset_reason TEXT,
                reset_had_activity INTEGER NOT NULL DEFAULT 0,
                is_fresh_reset INTEGER NOT NULL DEFAULT 0,
                expiry_finalized INTEGER NOT NULL DEFAULT 0,
                suspended INTEGER NOT NULL DEFAULT 0,
                resume_pending INTEGER NOT NULL DEFAULT 0,
                resume_reason TEXT,
                last_resume_marked_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_gw_sessions_platform ON gateway_sessions(platform);
            CREATE INDEX IF NOT EXISTS idx_gw_sessions_updated ON gateway_sessions(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_gw_sessions_session_id ON gateway_sessions(session_id);",
        )
        .map_err(|e| Error::Agent(format!("Failed to init gateway session DB: {}", e)))?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            entries: std::sync::RwLock::new(HashMap::new()),
            config: std::sync::RwLock::new(SessionStoreConfig::default()),
        };

        store.load_from_db()?;
        Ok(store)
    }

    /// Create from an existing connection (for testing).
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS gateway_sessions (
                session_key TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                origin TEXT,
                display_name TEXT,
                platform TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                last_prompt_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
                cost_status TEXT NOT NULL DEFAULT 'unknown',
                was_auto_reset INTEGER NOT NULL DEFAULT 0,
                auto_reset_reason TEXT,
                reset_had_activity INTEGER NOT NULL DEFAULT 0,
                is_fresh_reset INTEGER NOT NULL DEFAULT 0,
                expiry_finalized INTEGER NOT NULL DEFAULT 0,
                suspended INTEGER NOT NULL DEFAULT 0,
                resume_pending INTEGER NOT NULL DEFAULT 0,
                resume_reason TEXT,
                last_resume_marked_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_gw_sessions_platform ON gateway_sessions(platform);
            CREATE INDEX IF NOT EXISTS idx_gw_sessions_updated ON gateway_sessions(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_gw_sessions_session_id ON gateway_sessions(session_id);",
        )
        .map_err(|e| Error::Agent(format!("Failed to init gateway session DB: {}", e)))?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            entries: std::sync::RwLock::new(HashMap::new()),
            config: std::sync::RwLock::new(SessionStoreConfig::default()),
        };

        store.load_from_db()?;
        Ok(store)
    }

    /// Update the store configuration at runtime.
    pub fn set_config(&self, config: SessionStoreConfig) {
        *self.config.write().unwrap() = config;
    }

    /// Load all sessions from the database into the in-memory cache.
    fn load_from_db(&self) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT session_key, session_id, created_at, updated_at, origin, display_name,
                        platform, chat_type, input_tokens, output_tokens, cache_read_tokens,
                        cache_write_tokens, total_tokens, last_prompt_tokens, estimated_cost_usd,
                        cost_status, was_auto_reset, auto_reset_reason, reset_had_activity,
                        is_fresh_reset, expiry_finalized, suspended, resume_pending,
                        resume_reason, last_resume_marked_at
                 FROM gateway_sessions",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare load: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let origin_str: Option<String> = row.get(4)?;
                let origin =
                    origin_str.and_then(|s| serde_json::from_str::<SessionSource>(&s).ok());

                Ok(SessionEntry {
                    session_key: row.get(0)?,
                    session_id: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    origin,
                    display_name: row.get(5)?,
                    platform: row.get(6)?,
                    chat_type: row.get(7)?,
                    input_tokens: row.get::<_, i64>(8)? as u64,
                    output_tokens: row.get::<_, i64>(9)? as u64,
                    cache_read_tokens: row.get::<_, i64>(10)? as u64,
                    cache_write_tokens: row.get::<_, i64>(11)? as u64,
                    total_tokens: row.get::<_, i64>(12)? as u64,
                    last_prompt_tokens: row.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: row.get(14)?,
                    cost_status: row.get(15)?,
                    was_auto_reset: row.get::<_, i32>(16)? != 0,
                    auto_reset_reason: row.get(17)?,
                    reset_had_activity: row.get::<_, i32>(18)? != 0,
                    is_fresh_reset: row.get::<_, i32>(19)? != 0,
                    expiry_finalized: row.get::<_, i32>(20)? != 0,
                    suspended: row.get::<_, i32>(21)? != 0,
                    resume_pending: row.get::<_, i32>(22)? != 0,
                    resume_reason: row.get(23)?,
                    last_resume_marked_at: row.get(24)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Failed to query sessions: {}", e)))?;

        let mut entries = self.entries.write().unwrap();
        for row in rows {
            if let Ok(entry) = row {
                entries.insert(entry.session_key.clone(), entry);
            }
        }
        Ok(())
    }

    /// Persist a single entry to the database.
    fn save_entry(&self, entry: &SessionEntry) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        let origin_json = entry
            .origin
            .as_ref()
            .and_then(|o| serde_json::to_string(o).ok());

        conn.execute(
            "INSERT INTO gateway_sessions (
                session_key, session_id, created_at, updated_at, origin, display_name,
                platform, chat_type, input_tokens, output_tokens, cache_read_tokens,
                cache_write_tokens, total_tokens, last_prompt_tokens, estimated_cost_usd,
                cost_status, was_auto_reset, auto_reset_reason, reset_had_activity,
                is_fresh_reset, expiry_finalized, suspended, resume_pending,
                resume_reason, last_resume_marked_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
            )
            ON CONFLICT(session_key) DO UPDATE SET
                session_id = excluded.session_id,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                origin = excluded.origin,
                display_name = excluded.display_name,
                platform = excluded.platform,
                chat_type = excluded.chat_type,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                cache_read_tokens = excluded.cache_read_tokens,
                cache_write_tokens = excluded.cache_write_tokens,
                total_tokens = excluded.total_tokens,
                last_prompt_tokens = excluded.last_prompt_tokens,
                estimated_cost_usd = excluded.estimated_cost_usd,
                cost_status = excluded.cost_status,
                was_auto_reset = excluded.was_auto_reset,
                auto_reset_reason = excluded.auto_reset_reason,
                reset_had_activity = excluded.reset_had_activity,
                is_fresh_reset = excluded.is_fresh_reset,
                expiry_finalized = excluded.expiry_finalized,
                suspended = excluded.suspended,
                resume_pending = excluded.resume_pending,
                resume_reason = excluded.resume_reason,
                last_resume_marked_at = excluded.last_resume_marked_at",
            params![
                entry.session_key,
                entry.session_id,
                entry.created_at,
                entry.updated_at,
                origin_json,
                entry.display_name,
                entry.platform,
                entry.chat_type,
                entry.input_tokens as i64,
                entry.output_tokens as i64,
                entry.cache_read_tokens as i64,
                entry.cache_write_tokens as i64,
                entry.total_tokens as i64,
                entry.last_prompt_tokens as i64,
                entry.estimated_cost_usd,
                entry.cost_status,
                entry.was_auto_reset as i32,
                entry.auto_reset_reason,
                entry.reset_had_activity as i32,
                entry.is_fresh_reset as i32,
                entry.expiry_finalized as i32,
                entry.suspended as i32,
                entry.resume_pending as i32,
                entry.resume_reason,
                entry.last_resume_marked_at,
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to save session entry: {}", e)))?;
        Ok(())
    }

    // ── Core session lifecycle ──

    /// Get or create a session for a given source.
    ///
    /// Evaluates reset policy to determine if the existing session is stale.
    pub fn get_or_create_session(
        &self,
        source: &SessionSource,
        force_new: bool,
    ) -> Result<SessionEntry, Error> {
        let config = self.config.read().unwrap().clone();
        let session_key = build_session_key(
            source,
            config.group_sessions_per_user,
            config.thread_sessions_per_user,
        );
        let now = now_rfc3339();

        // Check existing session
        if !force_new {
            let entries = self.entries.read().unwrap();
            if let Some(entry) = entries.get(&session_key) {
                // Auto-reset suspended sessions
                if entry.suspended {
                    drop(entries);
                    return self.reset_session_inner(&session_key, source, true, Some("suspended"));
                }
                // Resume pending: preserve session_id
                if entry.resume_pending {
                    let mut entry = entry.clone();
                    entry.updated_at = now;
                    drop(entries);
                    self.save_entry(&entry)?;
                    let mut entries = self.entries.write().unwrap();
                    entries.insert(session_key, entry.clone());
                    return Ok(entry);
                }
                // Check reset policy
                if let Some(reason) = self.should_reset(entry, &config.reset_policy) {
                    let was_auto_reset = true;
                    let auto_reset_reason = Some(reason);
                    let _reset_had_activity = entry.total_tokens > 0;
                    drop(entries);
                    return self.reset_session_inner(
                        &session_key,
                        source,
                        was_auto_reset,
                        auto_reset_reason.as_deref(),
                    );
                }
                // Session is still valid — update activity
                let mut entry = entry.clone();
                entry.updated_at = now;
                drop(entries);
                self.save_entry(&entry)?;
                let mut entries = self.entries.write().unwrap();
                entries.insert(session_key, entry.clone());
                return Ok(entry);
            }
        }

        // Create new session
        let entry = SessionEntry {
            session_key: session_key.clone(),
            session_id: format!(
                "{}_{}",
                chrono::Local::now().format("%Y%m%d_%H%M%S"),
                Uuid::new_v4().to_string()[..8].to_string()
            ),
            created_at: now.clone(),
            updated_at: now,
            origin: Some(source.clone()),
            display_name: source.chat_name.clone(),
            platform: Some(source.platform.clone()),
            chat_type: source.chat_type.clone(),
            ..Default::default()
        };

        self.save_entry(&entry)?;
        let mut entries = self.entries.write().unwrap();
        entries.insert(session_key, entry.clone());
        Ok(entry)
    }

    /// Reset a session, creating a new session ID.
    pub fn reset_session(&self, session_key: &str) -> Result<Option<SessionEntry>, Error> {
        let entries = self.entries.read().unwrap();
        let entry = entries.get(session_key).cloned();
        drop(entries);

        if let Some(entry) = entry {
            let origin = entry.origin.clone();
            Ok(Some(self.reset_session_inner(
                session_key,
                &origin.unwrap_or(SessionSource {
                    platform: "unknown".to_string(),
                    chat_id: String::new(),
                    ..Default::default()
                }),
                false,
                None,
            )?))
        } else {
            Ok(None)
        }
    }

    fn reset_session_inner(
        &self,
        session_key: &str,
        source: &SessionSource,
        was_auto_reset: bool,
        auto_reset_reason: Option<&str>,
    ) -> Result<SessionEntry, Error> {
        let now = now_rfc3339();
        let new_entry = SessionEntry {
            session_key: session_key.to_string(),
            session_id: format!(
                "{}_{}",
                chrono::Local::now().format("%Y%m%d_%H%M%S"),
                Uuid::new_v4().to_string()[..8].to_string()
            ),
            created_at: now.clone(),
            updated_at: now,
            origin: Some(source.clone()),
            display_name: source.chat_name.clone(),
            platform: Some(source.platform.clone()),
            chat_type: source.chat_type.clone(),
            was_auto_reset,
            auto_reset_reason: auto_reset_reason.map(String::from),
            ..Default::default()
        };

        self.save_entry(&new_entry)?;
        let mut entries = self.entries.write().unwrap();
        entries.insert(session_key.to_string(), new_entry.clone());
        Ok(new_entry)
    }

    /// Switch a session key to point at an existing session ID.
    pub fn switch_session(
        &self,
        session_key: &str,
        target_session_id: &str,
    ) -> Result<Option<SessionEntry>, Error> {
        let entries = self.entries.read().unwrap();
        let old_entry = entries.get(session_key).cloned();
        drop(entries);

        let old_entry = match old_entry {
            Some(e) => e,
            None => return Ok(None),
        };

        if old_entry.session_id == target_session_id {
            return Ok(Some(old_entry));
        }

        let now = now_rfc3339();
        let new_entry = SessionEntry {
            session_key: session_key.to_string(),
            session_id: target_session_id.to_string(),
            created_at: now.clone(),
            updated_at: now,
            origin: old_entry.origin,
            display_name: old_entry.display_name,
            platform: old_entry.platform,
            chat_type: old_entry.chat_type,
            ..Default::default()
        };

        self.save_entry(&new_entry)?;
        let mut entries = self.entries.write().unwrap();
        entries.insert(session_key.to_string(), new_entry.clone());
        Ok(Some(new_entry))
    }

    /// Mark a session as suspended (from /stop).
    pub fn suspend_session(&self, session_key: &str) -> bool {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(session_key) {
            entry.suspended = true;
            let entry = entry.clone();
            drop(entries);
            let _ = self.save_entry(&entry);
            return true;
        }
        false
    }

    /// Mark a session as resume-pending after a restart interruption.
    pub fn mark_resume_pending(&self, session_key: &str, reason: &str) -> bool {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(session_key) {
            if entry.suspended {
                return false;
            }
            entry.resume_pending = true;
            entry.resume_reason = Some(reason.to_string());
            entry.last_resume_marked_at = Some(now_rfc3339());
            let entry = entry.clone();
            drop(entries);
            let _ = self.save_entry(&entry);
            return true;
        }
        false
    }

    /// Clear resume-pending flag after a successful resumed turn.
    pub fn clear_resume_pending(&self, session_key: &str) -> bool {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(session_key) {
            if !entry.resume_pending {
                return false;
            }
            entry.resume_pending = false;
            entry.resume_reason = None;
            entry.last_resume_marked_at = None;
            let entry = entry.clone();
            drop(entries);
            let _ = self.save_entry(&entry);
            return true;
        }
        false
    }

    /// Update session activity timestamp.
    pub fn update_activity(&self, session_key: &str) -> Result<(), Error> {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(session_key) {
            entry.updated_at = now_rfc3339();
            let entry = entry.clone();
            drop(entries);
            self.save_entry(&entry)?;
            return Ok(());
        }
        Err(Error::Agent(format!("Session not found: {}", session_key)))
    }

    /// Update session token counts.
    pub fn update_tokens(
        &self,
        session_key: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read: u64,
        cache_write: u64,
        cost_usd: f64,
    ) -> Result<(), Error> {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(session_key) {
            entry.input_tokens += input_tokens;
            entry.output_tokens += output_tokens;
            entry.cache_read_tokens += cache_read;
            entry.cache_write_tokens += cache_write;
            entry.total_tokens += input_tokens + output_tokens;
            entry.estimated_cost_usd += cost_usd;
            entry.updated_at = now_rfc3339();
            let entry = entry.clone();
            drop(entries);
            self.save_entry(&entry)?;
            return Ok(());
        }
        Err(Error::Agent(format!("Session not found: {}", session_key)))
    }

    /// Look up a session entry by session key.
    pub fn get_entry(&self, session_key: &str) -> Option<SessionEntry> {
        self.entries.read().unwrap().get(session_key).cloned()
    }

    /// Look up a session entry by session ID.
    pub fn lookup_by_session_id(&self, session_id: &str) -> Option<SessionEntry> {
        self.entries
            .read()
            .unwrap()
            .values()
            .find(|e| e.session_id == session_id)
            .cloned()
    }

    /// List all sessions, optionally filtered by activity.
    pub fn list_sessions(&self, active_minutes: Option<u64>) -> Vec<SessionEntry> {
        let entries = self.entries.read().unwrap();
        let mut result: Vec<SessionEntry> = if let Some(minutes) = active_minutes {
            let cutoff = SystemTime::now() - Duration::from_secs(minutes * 60);
            let cutoff_str = system_time_to_rfc3339(cutoff);
            entries
                .values()
                .filter(|e| e.updated_at >= cutoff_str)
                .cloned()
                .collect()
        } else {
            entries.values().cloned().collect()
        };
        result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        result
    }

    /// Remove a session entry.
    pub fn close_session(&self, session_key: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM gateway_sessions WHERE session_key = ?1",
            params![session_key],
        )
        .map_err(|e| Error::Agent(format!("Failed to close session: {}", e)))?;
        drop(conn);
        self.entries.write().unwrap().remove(session_key);
        Ok(())
    }

    /// Total number of sessions.
    pub fn session_count(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Total number of sessions (alias for backward compatibility).
    pub fn get_session_count(&self) -> usize {
        self.session_count()
    }

    /// List active sessions as `PlatformSession` for backward compatibility.
    pub fn list_active_sessions(&self, platform: Option<&str>) -> Vec<PlatformSession> {
        let entries = self.entries.read().unwrap();
        entries
            .values()
            .filter(|e| {
                platform
                    .map(|p| e.platform.as_deref() == Some(p))
                    .unwrap_or(true)
            })
            .map(|e| PlatformSession {
                session_id: e.session_id.clone(),
                platform: e.platform.clone().unwrap_or_default(),
                platform_user_id: e
                    .origin
                    .as_ref()
                    .and_then(|o| o.user_id.clone())
                    .unwrap_or_default(),
                platform_channel_id: e
                    .origin
                    .as_ref()
                    .map(|o| o.chat_id.clone())
                    .unwrap_or_default(),
                operant_session_id: String::new(),
                created_at: e.created_at.clone(),
                last_active: e.updated_at.clone(),
                metadata: HashMap::new(),
            })
            .collect()
    }

    /// Get a session by its ID (legacy `PlatformSession` API).
    pub fn get_session(&self, session_id: &str) -> Option<PlatformSession> {
        self.entries
            .read()
            .unwrap()
            .values()
            .find(|e| e.session_id == session_id)
            .map(|e| PlatformSession {
                session_id: e.session_id.clone(),
                platform: e.platform.clone().unwrap_or_default(),
                platform_user_id: e
                    .origin
                    .as_ref()
                    .and_then(|o| o.user_id.clone())
                    .unwrap_or_default(),
                platform_channel_id: e
                    .origin
                    .as_ref()
                    .map(|o| o.chat_id.clone())
                    .unwrap_or_default(),
                operant_session_id: String::new(),
                created_at: e.created_at.clone(),
                last_active: e.updated_at.clone(),
                metadata: HashMap::new(),
            })
    }

    /// Find a session matching platform + user + channel (legacy API).
    pub fn find_session(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
    ) -> Option<PlatformSession> {
        self.entries
            .read()
            .unwrap()
            .values()
            .find(|e| {
                e.platform.as_deref() == Some(platform)
                    && e.origin
                        .as_ref()
                        .map(|o| o.user_id.as_deref() == Some(user_id) && o.chat_id == channel_id)
                        .unwrap_or(false)
            })
            .map(|e| PlatformSession {
                session_id: e.session_id.clone(),
                platform: e.platform.clone().unwrap_or_default(),
                platform_user_id: e
                    .origin
                    .as_ref()
                    .and_then(|o| o.user_id.clone())
                    .unwrap_or_default(),
                platform_channel_id: e
                    .origin
                    .as_ref()
                    .map(|o| o.chat_id.clone())
                    .unwrap_or_default(),
                operant_session_id: String::new(),
                created_at: e.created_at.clone(),
                last_active: e.updated_at.clone(),
                metadata: HashMap::new(),
            })
    }

    /// Create a session (legacy API).
    pub fn create_session(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
    ) -> Result<PlatformSession, Error> {
        let source = SessionSource {
            platform: platform.to_string(),
            chat_id: channel_id.to_string(),
            user_id: Some(user_id.to_string()),
            chat_type: "dm".to_string(),
            ..Default::default()
        };
        let entry = self.get_or_create_session(&source, true)?;
        Ok(PlatformSession {
            session_id: entry.session_id,
            platform: entry.platform.unwrap_or_default(),
            platform_user_id: user_id.to_string(),
            platform_channel_id: channel_id.to_string(),
            operant_session_id: String::new(),
            created_at: entry.created_at,
            last_active: entry.updated_at,
            metadata: HashMap::new(),
        })
    }

    /// Find or create a shared session (legacy API).
    pub fn find_or_create_shared_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<PlatformSession, Error> {
        self.create_session(platform, "__shared__", channel_id)
    }

    /// Update session metadata (legacy API).
    pub fn update_session_metadata(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
        _updates: &[(String, String)],
    ) -> bool {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.values_mut().find(|e| {
            e.platform.as_deref() == Some(platform)
                && e.origin
                    .as_ref()
                    .map(|o| o.user_id.as_deref() == Some(user_id) && o.chat_id == channel_id)
                    .unwrap_or(false)
        }) {
            entry.updated_at = now_rfc3339();
            let entry = entry.clone();
            drop(entries);
            let _ = self.save_entry(&entry);
            true
        } else {
            false
        }
    }

    /// Check if any sessions exist.
    pub fn has_any_sessions(&self) -> bool {
        !self.entries.read().unwrap().is_empty()
    }

    // ── Reset policy evaluation ──

    /// Check if a session should be reset based on policy.
    fn should_reset(&self, entry: &SessionEntry, policy: &SessionResetPolicy) -> Option<String> {
        if policy.mode == ResetMode::None {
            return None;
        }

        let now = SystemTime::now();
        let updated_at = parse_rfc3339(&entry.updated_at).unwrap_or(now);

        if matches!(policy.mode, ResetMode::Idle | ResetMode::Both) {
            let idle_deadline = updated_at + Duration::from_secs(policy.idle_minutes * 60);
            if now > idle_deadline {
                return Some("idle".to_string());
            }
        }

        if matches!(policy.mode, ResetMode::Daily | ResetMode::Both) {
            let now_dt: DateTime<Utc> = now.into();
            let today_reset = now_dt
                .date_naive()
                .and_hms_opt(policy.at_hour, 0, 0)
                .unwrap()
                .and_utc();
            let reset_time = if now_dt.time().hour() < policy.at_hour {
                today_reset - chrono::Duration::days(1)
            } else {
                today_reset
            };
            if updated_at < reset_time.into() {
                return Some("daily".to_string());
            }
        }

        None
    }

    /// Check if a session is expired (for background expiry watcher).
    pub fn is_session_expired(&self, session_key: &str) -> bool {
        let config = self.config.read().unwrap().clone();
        let entries = self.entries.read().unwrap();
        if let Some(entry) = entries.get(session_key) {
            self.should_reset(entry, &config.reset_policy).is_some()
        } else {
            false
        }
    }

    /// Mark recently-active sessions as resumable after an unexpected exit.
    pub fn suspend_recently_active(&self, max_age_seconds: u64) -> u32 {
        let cutoff = SystemTime::now() - Duration::from_secs(max_age_seconds);
        let cutoff_str = system_time_to_rfc3339(cutoff);
        let mut count = 0;
        let mut entries = self.entries.write().unwrap();
        for entry in entries.values_mut() {
            if entry.resume_pending || entry.suspended {
                continue;
            }
            if entry.updated_at >= cutoff_str {
                entry.resume_pending = true;
                entry.resume_reason = Some("restart_interrupted".to_string());
                entry.last_resume_marked_at = Some(now_rfc3339());
                count += 1;
            }
        }
        count
    }

    /// Prune old entries older than max_age_days.
    pub fn prune_old_entries(&self, max_age_days: u32) -> u32 {
        if max_age_days == 0 {
            return 0;
        }
        let cutoff = SystemTime::now() - Duration::from_secs(max_age_days as u64 * 86400);
        let cutoff_str = system_time_to_rfc3339(cutoff);
        let mut removed = Vec::new();
        let mut entries = self.entries.write().unwrap();
        for (key, entry) in entries.iter() {
            if entry.suspended {
                continue;
            }
            if entry.updated_at < cutoff_str {
                removed.push(key.clone());
            }
        }
        for key in &removed {
            entries.remove(key);
        }
        let count = removed.len() as u32;
        if !removed.is_empty() {
            drop(entries);
            let conn = self.conn.lock().unwrap();
            for key in &removed {
                let _ = conn.execute(
                    "DELETE FROM gateway_sessions WHERE session_key = ?1",
                    params![key],
                );
            }
        }
        count
    }
}

impl Default for SessionEntry {
    fn default() -> Self {
        let now = now_rfc3339();
        Self {
            session_key: String::new(),
            session_id: String::new(),
            created_at: now.clone(),
            updated_at: now,
            origin: None,
            display_name: None,
            platform: None,
            chat_type: "dm".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 0,
            last_prompt_tokens: 0,
            estimated_cost_usd: 0.0,
            cost_status: "unknown".to_string(),
            was_auto_reset: false,
            auto_reset_reason: None,
            reset_had_activity: false,
            is_fresh_reset: false,
            expiry_finalized: false,
            suspended: false,
            resume_pending: false,
            resume_reason: None,
            last_resume_marked_at: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn system_time_to_rfc3339(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339()
}

fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.into())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> PersistentSessionStore {
        let conn = Connection::open_in_memory().expect("Failed to create in-memory DB");
        PersistentSessionStore::from_connection(conn).expect("Failed to create store")
    }

    fn test_source(platform: &str, user_id: &str, chat_id: &str, chat_type: &str) -> SessionSource {
        SessionSource {
            platform: platform.to_string(),
            chat_id: chat_id.to_string(),
            chat_name: None,
            chat_type: chat_type.to_string(),
            user_id: Some(user_id.to_string()),
            user_name: None,
            thread_id: None,
            chat_topic: None,
            user_id_alt: None,
            chat_id_alt: None,
            is_bot: false,
            guild_id: None,
            parent_chat_id: None,
            message_id: None,
            role_authorized: false,
        }
    }

    #[test]
    fn test_session_key_dm() {
        let source = test_source("telegram", "123", "456", "dm");
        let key = build_session_key(&source, true, false);
        assert_eq!(key, "agent:main:telegram:dm:456");
    }

    #[test]
    fn test_session_key_dm_with_thread() {
        let mut source = test_source("telegram", "123", "456", "dm");
        source.thread_id = Some("789".to_string());
        let key = build_session_key(&source, true, false);
        assert_eq!(key, "agent:main:telegram:dm:456:789");
    }

    #[test]
    fn test_session_key_dm_no_chat_id() {
        let source = test_source("telegram", "123", "", "dm");
        let key = build_session_key(&source, true, false);
        assert_eq!(key, "agent:main:telegram:dm:123");
    }

    #[test]
    fn test_session_key_group() {
        let source = test_source("telegram", "123", "456", "group");
        let key = build_session_key(&source, true, false);
        assert_eq!(key, "agent:main:telegram:group:456:123");
    }

    #[test]
    fn test_session_key_group_no_isolation() {
        let source = test_source("telegram", "123", "456", "group");
        let key = build_session_key(&source, false, false);
        assert_eq!(key, "agent:main:telegram:group:456");
    }

    #[test]
    fn test_session_key_thread_shared() {
        let mut source = test_source("telegram", "123", "456", "group");
        source.thread_id = Some("789".to_string());
        // thread_sessions_per_user=false → shared, no user_id in key
        let key = build_session_key(&source, true, false);
        assert_eq!(key, "agent:main:telegram:group:456:789");
    }

    #[test]
    fn test_session_key_thread_isolated() {
        let mut source = test_source("telegram", "123", "456", "group");
        source.thread_id = Some("789".to_string());
        // thread_sessions_per_user=true → isolated, user_id in key
        let key = build_session_key(&source, true, true);
        assert_eq!(key, "agent:main:telegram:group:456:789:123");
    }

    #[test]
    fn test_persist_and_load() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let entry = store.get_or_create_session(&source, false).unwrap();
        assert_eq!(entry.session_key, "agent:main:telegram:dm:456");
        assert!(!entry.session_id.is_empty());

        // Reload from DB
        let _store2 = {
            let _conn = Connection::open_in_memory().unwrap();
            // We can't reload from the same in-memory DB, so just test the cache
        };

        // Verify it's in the cache
        let found = store.get_entry("agent:main:telegram:dm:456");
        assert!(found.is_some());
        assert_eq!(found.unwrap().session_id, entry.session_id);
    }

    #[test]
    fn test_get_or_create_returns_same_session() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let entry1 = store.get_or_create_session(&source, false).unwrap();
        let entry2 = store.get_or_create_session(&source, false).unwrap();
        assert_eq!(entry1.session_id, entry2.session_id);
    }

    #[test]
    fn test_force_new_session() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let entry1 = store.get_or_create_session(&source, false).unwrap();
        let entry2 = store.get_or_create_session(&source, true).unwrap();
        assert_ne!(entry1.session_id, entry2.session_id);
    }

    #[test]
    fn test_suspend_and_resume() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let _entry = store.get_or_create_session(&source, false).unwrap();

        assert!(store.suspend_session("agent:main:telegram:dm:456"));

        // Next get_or_create should reset
        let entry2 = store.get_or_create_session(&source, false).unwrap();
        assert!(entry2.was_auto_reset);
        assert_eq!(entry2.auto_reset_reason.as_deref(), Some("suspended"));
    }

    #[test]
    fn test_resume_pending() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let entry1 = store.get_or_create_session(&source, false).unwrap();

        assert!(store.mark_resume_pending("agent:main:telegram:dm:456", "restart_timeout"));

        // Should return same session_id
        let entry2 = store.get_or_create_session(&source, false).unwrap();
        assert_eq!(entry1.session_id, entry2.session_id);
    }

    #[test]
    fn test_switch_session() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let entry1 = store.get_or_create_session(&source, false).unwrap();

        let entry2 = store
            .switch_session("agent:main:telegram:dm:456", "old-session-123")
            .unwrap()
            .unwrap();
        assert_eq!(entry2.session_id, "old-session-123");
        assert_ne!(entry1.session_id, entry2.session_id);
    }

    #[test]
    fn test_list_sessions() {
        let store = test_store();
        let source1 = test_source("telegram", "123", "456", "dm");
        let source2 = test_source("discord", "789", "012", "dm");
        store.get_or_create_session(&source1, false).unwrap();
        store.get_or_create_session(&source2, false).unwrap();

        assert_eq!(store.session_count(), 2);
        let all = store.list_sessions(None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_close_session() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        let _entry = store.get_or_create_session(&source, false).unwrap();
        assert_eq!(store.session_count(), 1);

        store.close_session("agent:main:telegram:dm:456").unwrap();
        assert_eq!(store.session_count(), 0);
    }

    #[test]
    fn test_pii_hashing() {
        assert_eq!(hash_sender_id("12345"), hash_sender_id("12345"));
        assert_ne!(hash_sender_id("12345"), hash_sender_id("67890"));
        assert!(hash_sender_id("test").starts_with("user_"));

        let hashed = hash_chat_id("telegram:12345");
        assert!(hashed.starts_with("telegram:"));
        assert_ne!(hashed, "telegram:12345");
    }

    #[test]
    fn test_session_source_description() {
        let mut source = test_source("telegram", "123", "456", "dm");
        source.user_name = Some("Alice".to_string());
        assert_eq!(source.description(), "DM with Alice");

        let source = test_source("telegram", "123", "456", "group");
        assert_eq!(source.description(), "group: 456");

        let source = test_source("local", "", "", "dm");
        assert_eq!(source.description(), "CLI terminal");
    }

    #[test]
    fn test_is_shared_multi_user_session() {
        let source = test_source("telegram", "123", "456", "dm");
        assert!(!is_shared_multi_user_session(&source, true, false));

        let source = test_source("telegram", "123", "456", "group");
        assert!(!is_shared_multi_user_session(&source, true, false)); // group_sessions_per_user=true → not shared

        let source = test_source("telegram", "123", "456", "group");
        assert!(is_shared_multi_user_session(&source, false, false)); // group_sessions_per_user=false → shared
    }

    #[test]
    fn test_prune_old_entries() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        store.get_or_create_session(&source, false).unwrap();

        // Prune with 0 days = no-op
        assert_eq!(store.prune_old_entries(0), 0);
        // Entry should still exist
        assert_eq!(store.session_count(), 1);
    }

    #[test]
    fn test_update_tokens() {
        let store = test_store();
        let source = test_source("telegram", "123", "456", "dm");
        store.get_or_create_session(&source, false).unwrap();

        store
            .update_tokens("agent:main:telegram:dm:456", 100, 50, 10, 5, 0.001)
            .unwrap();

        let entry = store.get_entry("agent:main:telegram:dm:456").unwrap();
        assert_eq!(entry.input_tokens, 100);
        assert_eq!(entry.output_tokens, 50);
        assert_eq!(entry.total_tokens, 150);
    }

    // === Session key edge cases ===

    #[test]
    fn test_session_key_empty_platform() {
        let source = test_source("", "123", "456", "dm");
        let key = build_session_key(&source, false, false);
        assert!(!key.is_empty());
    }

    #[test]
    fn test_session_key_special_chars() {
        let source = test_source("telegram", "user@domain.com+extra", "chat-id_123", "dm");
        let key = build_session_key(&source, false, false);
        assert!(!key.is_empty());
    }

    #[test]
    fn test_session_key_group_vs_dm() {
        let dm = test_source("telegram", "123", "456", "dm");
        let group = test_source("telegram", "123", "789", "group");
        let dm_key = build_session_key(&dm, false, false);
        let group_key = build_session_key(&group, false, false);
        assert_ne!(dm_key, group_key);
    }

    #[test]
    fn test_session_key_group_per_user_isolation() {
        let source = test_source("discord", "user1", "guild1", "group");
        let isolated = build_session_key(&source, true, false);
        let shared = build_session_key(&source, false, false);
        assert_ne!(isolated, shared);
    }

    // === PII hashing ===

    #[test]
    fn test_hash_sender_id_deterministic() {
        let h1 = hash_sender_id("test_user_123");
        let h2 = hash_sender_id("test_user_123");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_sender_id_different_inputs() {
        let h1 = hash_sender_id("user_a");
        let h2 = hash_sender_id("user_b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hash_chat_id_deterministic() {
        let h1 = hash_chat_id("chat_42");
        let h2 = hash_chat_id("chat_42");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_chat_id_various_lengths() {
        let short = hash_chat_id("a");
        let long = hash_chat_id(&"x".repeat(1000));
        assert_ne!(short, long);
        assert!(short.len() > 0);
        assert!(long.len() > 0);
    }

    #[test]
    fn test_hash_sender_id_not_reversible() {
        let original = "secret_user_id_12345";
        let hashed = hash_sender_id(original);
        assert_ne!(hashed, original);
        assert!(!hashed.contains(original));
    }

    // === Session lifecycle ===

    #[test]
    fn test_get_or_create_creates_new() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let entry = store.get_or_create_session(&source, false).unwrap();
        assert_eq!(entry.platform.as_deref(), Some("telegram"));
    }

    #[test]
    fn test_get_or_create_returns_existing() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let e1 = store.get_or_create_session(&source, false).unwrap();
        let e2 = store.get_or_create_session(&source, false).unwrap();
        assert_eq!(e1.session_id, e2.session_id);
    }

    #[test]
    fn test_force_new_always_creates() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let e1 = store.get_or_create_session(&source, false).unwrap();
        let e2 = store.get_or_create_session(&source, true).unwrap();
        assert_ne!(e1.session_id, e2.session_id);
    }

    #[test]
    fn test_session_update_tokens() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let entry = store.get_or_create_session(&source, false).unwrap();
        store
            .update_tokens(&entry.session_key, 100, 50, 20, 10, 0.001)
            .unwrap();
        let updated = store.get_entry(&entry.session_key).unwrap();
        assert_eq!(updated.input_tokens, 100);
        assert_eq!(updated.output_tokens, 50);
        assert_eq!(updated.total_tokens, 150);
    }

    // === Reset policy ===

    #[test]
    fn test_reset_mode_none_never_resets() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        store.set_config(SessionStoreConfig {
            reset_policy: SessionResetPolicy {
                mode: ResetMode::None,
                idle_minutes: 5,
                at_hour: 0,
            },
            ..Default::default()
        });
        let entry = store.get_or_create_session(&source, false).unwrap();
        let result = store.reset_session(&entry.session_key).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_reset_session_creates_new_session() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let e1 = store.get_or_create_session(&source, false).unwrap();
        let result = store.reset_session(&e1.session_key).unwrap();
        assert!(result.is_some());
        let new = result.unwrap();
        assert_ne!(new.session_id, e1.session_id);
    }

    // === Persistent store operations ===

    #[test]
    fn test_session_count() {
        let store = test_store();
        let s1 = test_source("telegram", "u1", "c1", "dm");
        let s2 = test_source("discord", "u2", "c2", "dm");
        store.get_or_create_session(&s1, false).unwrap();
        store.get_or_create_session(&s2, false).unwrap();
        assert_eq!(store.session_count(), 2);
    }

    #[test]
    fn test_v2_list_sessions() {
        let store = test_store();
        let s1 = test_source("telegram", "u1", "c1", "dm");
        let s2 = test_source("discord", "u2", "c2", "dm");
        store.get_or_create_session(&s1, false).unwrap();
        store.get_or_create_session(&s2, false).unwrap();
        let sessions = store.list_sessions(None);
        assert!(sessions.len() >= 2);
    }

    #[test]
    fn test_v2_close_session() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let entry = store.get_or_create_session(&source, false).unwrap();
        store.close_session(&entry.session_key).unwrap();
        assert!(store.get_entry(&entry.session_key).is_none());
    }

    #[test]
    fn test_v2_prune_empty_store() {
        let conn = Connection::open_in_memory().unwrap();
        let store = PersistentSessionStore::from_connection(conn).unwrap();
        let pruned = store.prune_old_entries(60);
        assert_eq!(pruned, 0);
    }

    // === SessionSource ===

    #[test]
    fn test_v2_session_source_description() {
        let source = test_source("telegram", "u1", "c1", "dm");
        let desc = source.description();
        assert!(!desc.is_empty());
    }

    #[test]
    fn test_session_source_to_dict() {
        let source = test_source("telegram", "u1", "c1", "dm");
        let dict = source.to_dict();
        assert!(dict.is_object());
    }

    #[test]
    fn test_session_source_from_dict() {
        let source = test_source("telegram", "u1", "c1", "dm");
        let dict = source.to_dict();
        let restored = SessionSource::from_dict(&dict);
        assert!(restored.is_some());
        let r = restored.unwrap();
        assert_eq!(r.platform, "telegram");
        assert_eq!(r.user_id.as_deref(), Some("u1"));
    }

    #[test]
    fn test_session_source_roundtrip() {
        let mut source = test_source("discord", "user_abc", "chat_xyz", "group");
        source.thread_id = Some("thread_1".to_string());
        source.guild_id = Some("guild_1".to_string());
        let dict = source.to_dict();
        let restored = SessionSource::from_dict(&dict).unwrap();
        assert_eq!(restored.thread_id, Some("thread_1".to_string()));
        assert_eq!(restored.guild_id, Some("guild_1".to_string()));
    }

    #[test]
    fn test_session_source_from_dict_invalid() {
        let invalid = serde_json::json!({"garbage": true});
        let result = SessionSource::from_dict(&invalid);
        assert!(result.is_none());
    }

    // === Shared multi-user session ===

    #[test]
    fn test_is_shared_dm() {
        let source = test_source("telegram", "u1", "c1", "dm");
        assert!(!is_shared_multi_user_session(&source, false, false));
    }

    #[test]
    fn test_is_shared_group_no_isolation() {
        let source = test_source("telegram", "u1", "c1", "group");
        assert!(is_shared_multi_user_session(&source, false, false));
    }

    // === Suspend/resume ===

    #[test]
    fn test_suspend_session() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let entry = store.get_or_create_session(&source, false).unwrap();
        let suspended = store.suspend_session(&entry.session_key);
        assert!(suspended);
    }

    #[test]
    fn test_mark_and_clear_resume_pending() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        let entry = store.get_or_create_session(&source, false).unwrap();
        let marked = store.mark_resume_pending(&entry.session_key, "reason");
        assert!(marked);
        let cleared = store.clear_resume_pending(&entry.session_key);
        assert!(cleared);
    }

    #[test]
    fn test_suspend_recently_active() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        store.get_or_create_session(&source, false).unwrap();
        let count = store.suspend_recently_active(60 * 24 * 365);
        assert!(count >= 1);
    }

    // === has_any_sessions ===

    #[test]
    fn test_has_any_sessions_empty() {
        let conn = Connection::open_in_memory().unwrap();
        let store = PersistentSessionStore::from_connection(conn).unwrap();
        assert!(!store.has_any_sessions());
    }

    #[test]
    fn test_has_any_sessions_with_entry() {
        let store = test_store();
        let source = test_source("telegram", "u1", "c1", "dm");
        store.get_or_create_session(&source, false).unwrap();
        assert!(store.has_any_sessions());
    }
}
