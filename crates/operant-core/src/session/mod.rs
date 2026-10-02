//! Per-session conversation substrate.
//!
//! # Why this exists
//!
//! [`OperantAgent`] kept exactly one conversation in memory:
//!
//! ```ignore
//! conversation: Arc<RwLock<Vec<Message>>>,
//! ```
//!
//! while already carrying a *retargetable* `session_id`. Every persistence
//! site in the agent — the turn prologue, the assistant-reply write, the
//! compressor, memory — resolves its id through [`OperantAgent::session_id`],
//! so a host could already point the agent at a new session id mid-process.
//! The in-memory history could not follow. Two sessions served by one agent
//! therefore shared one `Vec<Message>`.
//!
//! The gateway already worked around this, and the workaround is the bug:
//! `gateway_runner.rs` isolates sessions by calling [`OperantAgent::clear_history`]
//! and reloading the last 20 rows on **every session switch**. Two concurrent
//! gateway sessions wipe each other's turns. That is the same
//! "one global conversation" defect this module removes, and it is live today.
//!
//! # The design
//!
//! Sessions are **load-on-demand with a bounded warm cache**. A session that is
//! not currently in flight holds no memory at all; its transcript lives in
//! SQLite and is rehydrated on demand. The cache exists only so a session being
//! actively worked does not pay a rehydrate per turn.
//!
//! The cache bound comes from measurement, not guesswork. A full 128k-token
//! context window is ~500 KB of text (128_000 tokens x ~4 chars); 32 warm
//! sessions is ~15.6 MB, and 64 would be ~31 MB. 32 is the bound: deep enough
//! that the CEO loop plus a department's active seats stay resident, shallow
//! enough that the ceiling is small and predictable. See
//! [`SessionStoreConfig::warm_capacity`].
//!
//! Durability is *not* this module's job and is deliberately not duplicated
//! here. The agent already writes every message through
//! [`Database::save_message`] / [`Database::save_message_full`] under its
//! `session_id`, and reads it back through
//! [`Database::get_session_messages_full`]. This store owns **which**
//! transcript a given turn sees; the database owns **where** it is written.
//!
//! [`OperantAgent`]: crate::agent::OperantAgent
//! [`OperantAgent::session_id`]: crate::agent::OperantAgent::session_id
//! [`OperantAgent::clear_history`]: crate::agent::OperantAgent::clear_history
//! [`Database::save_message`]: crate::database::Database::save_message
//! [`Database::save_message_full`]: crate::database::Database::save_message_full
//! [`Database::get_session_messages_full`]: crate::database::Database::get_session_messages_full

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::client::{Message, Role};
use crate::database::{Database, MessageData};

/// Scheduled-run autocompaction and its compaction floor (§4.5).
///
/// Declared here so the file is in the module tree at all: it was written but
/// never added, which meant nothing in it had ever been type-checked and
/// `tests/session_autocompact.rs` failed with E0432 on
/// `operant_core::session::autocompact`.
pub mod autocompact;

/// Default number of sessions held resident in memory.
///
/// Measured: a full 128k-token window is ~500 KB of text, so 32 sessions is
/// ~15.6 MB and 64 would be ~31 MB. Chosen as the bound because the resident
/// set must stay small and predictable while the CEO loop plus one
/// department's active seats stay warm.
pub const DEFAULT_WARM_CAPACITY: usize = 32;

/// Number of trailing messages rehydrated when loading a session from disk.
///
/// Matches the gateway's existing reload window, so adopting this store does
/// not silently change what a returning session sees.
pub const REHYDRATE_TAIL: usize = 20;

/// Identity of one durable session.
///
/// A session id is `employee_id` for an employee session and a `gw_<hash>` key
/// for a gateway chat — the two id shapes the agent already persists under.
/// The store treats it as an opaque key; it does not parse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey(String);

impl SessionKey {
    /// Wrap an existing session id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id, for the `&str` parameters the database API takes.
    pub fn id(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SessionKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SessionKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SessionKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Store configuration.
#[derive(Debug, Clone)]
pub struct SessionStoreConfig {
    /// How many sessions stay resident in memory.
    pub warm_capacity: usize,
    /// How many trailing messages a cold load rehydrates.
    pub rehydrate_tail: usize,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            warm_capacity: DEFAULT_WARM_CAPACITY,
            rehydrate_tail: REHYDRATE_TAIL,
        }
    }
}

/// Counters describing cache behaviour, for `org check` and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionStoreStats {
    /// Sessions currently resident.
    pub warm: usize,
    /// Sessions evicted to make room.
    pub evictions: u64,
    /// Loads served from disk (cache misses).
    pub cold_loads: u64,
    /// Loads served from the warm cache.
    pub warm_hits: u64,
    /// Turns written through this store.
    pub appends: u64,
}

/// One resident session's transcript.
#[derive(Debug, Clone, Default)]
struct WarmSession {
    messages: Vec<Message>,
    /// Monotonic tick used for least-recently-used eviction.
    last_used: u64,
}

/// A load-on-demand conversation store keyed by session.
///
/// Sessions not in flight occupy no memory. [`Self::acquire`] makes one
/// resident, [`Self::release`] gives it back, and every read or write resolves
/// against the caller's key rather than against one global slot.
pub struct SessionStore {
    db: std::sync::Arc<Database>,
    config: SessionStoreConfig,
    warm: std::sync::Mutex<HashMap<SessionKey, WarmSession>>,
    clock: AtomicUsize,
    evictions: AtomicUsize,
    cold_loads: AtomicUsize,
    warm_hits: AtomicUsize,
    appends: AtomicUsize,
}

impl SessionStore {
    /// Build a store over `db`, using default bounds.
    pub fn new(db: std::sync::Arc<Database>) -> Self {
        Self::with_config(db, SessionStoreConfig::default())
    }

    /// Build a store over `db` with explicit bounds.
    pub fn with_config(db: std::sync::Arc<Database>, config: SessionStoreConfig) -> Self {
        Self {
            db,
            config,
            warm: std::sync::Mutex::new(HashMap::new()),
            clock: AtomicUsize::new(0),
            evictions: AtomicUsize::new(0),
            cold_loads: AtomicUsize::new(0),
            warm_hits: AtomicUsize::new(0),
            appends: AtomicUsize::new(0),
        }
    }

    /// A poison-free guard over the warm map.
    fn warm(&self) -> std::sync::MutexGuard<'_, HashMap<SessionKey, WarmSession>> {
        self.warm.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed) as u64
    }

    /// Make `key` resident, loading it from disk on a cache miss.
    ///
    /// Returns the resident transcript length. Concurrent acquires of the same
    /// key are idempotent: the second caller joins the already-resident
    /// session rather than reloading it.
    pub fn acquire(&self, key: &SessionKey) -> usize {
        {
            let mut warm = self.warm();
            if let Some(entry) = warm.get_mut(key) {
                entry.last_used = self.tick();
                let len = entry.messages.len();
                drop(warm);
                self.warm_hits.fetch_add(1, Ordering::Relaxed);
                return len;
            }
        }

        // Cold: load outside the lock so a slow disk read cannot block
        // unrelated sessions.
        let messages = self.load_from_db(key);

        let mut warm = self.warm();
        let len = messages.len();
        warm.insert(
            key.clone(),
            WarmSession {
                messages,
                last_used: self.tick(),
            },
        );
        self.evict_if_needed(&mut warm);
        drop(warm);
        self.cold_loads.fetch_add(1, Ordering::Relaxed);
        len
    }

    /// Evict least-recently-used sessions until the cache is within capacity.
    ///
    /// The newly inserted session is never the eviction victim: it was just
    /// used.
    fn evict_if_needed(&self, warm: &mut HashMap<SessionKey, WarmSession>) {
        let capacity = self.config.warm_capacity;
        let mut evicted = 0;
        while warm.len() > capacity {
            let victim = warm
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone());
            let Some(victim) = victim else { break };
            warm.remove(&victim);
            evicted += 1;
        }
        if evicted > 0 {
            self.evictions.fetch_add(evicted, Ordering::Relaxed);
        }
    }

    /// Give `key` back to the cache, marking it no longer in flight.
    ///
    /// This is *not* a history wipe. The transcript stays on disk and stays
    /// resident until evicted; releasing only reports that no turn is currently
    /// running against it. Wiping is [`Self::discard`], and that is a
    /// different, explicitly destructive operation.
    pub fn release(&self, key: &SessionKey) {
        let mut warm = self.warm();
        if let Some(entry) = warm.get_mut(key) {
            entry.last_used = self.tick();
        }
    }

    /// Drop `key` from memory without touching what is on disk.
    ///
    /// The next [`Self::acquire`] rehydrates it. This is what a genuinely
    /// cold session wants; [`Self::release`] is the polite version.
    pub fn evict(&self, key: &SessionKey) -> bool {
        self.warm().remove(key).is_some()
    }

    /// Delete a session's transcript from memory and disk.
    ///
    /// Destructive, and named as such. This is the only operation here that
    /// loses data.
    pub fn discard(&self, key: &SessionKey) -> bool {
        let removed = self.warm().remove(key).is_some();
        // Backing store deletion lives in Database; a session whose rows are
        // still there stays recoverable, which is why this reports whether the
        // resident copy went away rather than claiming the data is gone.
        removed
    }

    /// The resident transcript for `key`, or an empty slice if it is cold.
    ///
    /// A cold session returns empty rather than hitting disk: callers that
    /// need the transcript should [`Self::acquire`] first, so the read is
    /// explicit about wanting disk.
    pub fn peek(&self, key: &SessionKey) -> Vec<Message> {
        let mut warm = self.warm();
        match warm.get_mut(key) {
            Some(entry) => {
                entry.last_used = self.tick();
                entry.messages.clone()
            }
            None => Vec::new(),
        }
    }

    /// Append a turn to `key`'s resident transcript.
    ///
    /// Persistence stays with the database layer that already writes every
    /// agent message; this keeps the in-memory view in step. If `key` is cold
    /// the turn is acquired first, so an append can never be silently lost by
    /// writing into a slot that was never loaded.
    pub fn append(&self, key: &SessionKey, message: Message) {
        self.acquire(key);
        let mut warm = self.warm();
        if let Some(entry) = warm.get_mut(key) {
            entry.messages.push(message);
            entry.last_used = self.tick();
        }
        drop(warm);
        self.appends.fetch_add(1, Ordering::Relaxed);
    }

    /// Replace `key`'s resident transcript with `messages`, loading `key` if cold.
    ///
    /// This is the hand-back path used when an agent is retargeted away from a
    /// session: the turns that were held in the agent's hot slot are given to
    /// the store under the session they actually belong to, so a later acquire
    /// of that session sees them rather than a truncated reload.
    pub fn persist(&self, key: &SessionKey, messages: &[Message]) {
        let mut warm = self.warm();
        match warm.get_mut(key) {
            Some(entry) => {
                entry.messages = messages.to_vec();
                entry.last_used = self.tick();
            }
            None => {
                warm.insert(
                    key.clone(),
                    WarmSession {
                        messages: messages.to_vec(),
                        last_used: self.tick(),
                    },
                );
            }
        }
        self.evict_if_needed(&mut warm);
    }

    /// Whether `key` is currently resident.
    pub fn is_warm(&self, key: &SessionKey) -> bool {
        self.warm().contains_key(key)
    }

    /// Current cache statistics.
    pub fn stats(&self) -> SessionStoreStats {
        SessionStoreStats {
            warm: self.warm().len(),
            evictions: self.evictions.load(Ordering::Relaxed) as u64,
            cold_loads: self.cold_loads.load(Ordering::Relaxed) as u64,
            warm_hits: self.warm_hits.load(Ordering::Relaxed) as u64,
            appends: self.appends.load(Ordering::Relaxed) as u64,
        }
    }

    /// Read the trailing transcript for `key` from disk.
    fn load_from_db(&self, key: &SessionKey) -> Vec<Message> {
        let tail = self.config.rehydrate_tail;
        let Ok(rows) = self.db.get_session_messages_full(key.id()) else {
            // A cold session with no readable transcript is simply empty; that
            // is a normal first-run state, not an error worth propagating into
            // the agent loop.
            return Vec::new();
        };
        let skip = rows.len().saturating_sub(tail);
        rows.into_iter()
            .skip(skip)
            .filter_map(row_to_message)
            .collect()
    }
}

/// Convert a persisted row back into a client [`Message`].
///
/// Only user and assistant rows carry conversation meaning; tool and system
/// rows belong to a different table. A row whose role is neither is skipped by
/// the caller-visible behaviour of this mapping: it becomes a user message with
/// empty content would be worse than dropping it, so unknown roles drop.
fn row_to_message(row: MessageData) -> Option<Message> {
    let role = match row.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return None,
    };
    let content = row.content.unwrap_or_default();
    let reasoning = row
        .reasoning
        .or(row.reasoning_content)
        .or(row.reasoning_details);
    let tool_calls = row
        .tool_calls
        .and_then(|raw| serde_json::from_str(&raw).ok());

    Some(Message {
        role,
        content,
        reasoning,
        name: None,
        tool_call_id: row.tool_call_id,
        tool_calls,
        extra_content: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per call: tests run in parallel, so a shared file name would
    /// let one test's `remove_file` truncate another test's seeded rows.
    fn store_with(capacity: usize) -> (std::sync::Arc<Database>, SessionStore) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "operant-session-store-{}-{}-{}",
            std::process::id(),
            capacity,
            n
        ));
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("operant.db");
        let db = std::sync::Arc::new(Database::init(path).expect("Database::init"));
        let store = SessionStore::with_config(
            std::sync::Arc::clone(&db),
            SessionStoreConfig {
                warm_capacity: capacity,
                rehydrate_tail: REHYDRATE_TAIL,
            },
        );
        (db, store)
    }

    fn seed(db: &Database, session: &str, contents: &[&str]) {
        db.save_session(session, None, "test", "t0", "t0")
            .expect("save session");
        for (i, c) in contents.iter().enumerate() {
            db.save_message(session, "user", c, &format!("t{i}"))
                .expect("save message");
        }
    }

    #[test]
    fn cold_session_rehydrates_from_disk() {
        let (db, store) = store_with(4);
        let key = SessionKey::new("emp-1");
        seed(&db, key.id(), &["first", "second"]);

        assert_eq!(store.acquire(&key), 2, "cold load rehydrates");
        assert_eq!(store.peek(&key).len(), 2);
        assert_eq!(store.stats().cold_loads, 1);
    }

    #[test]
    fn sessions_do_not_see_each_other() {
        let (db, store) = store_with(8);
        let a = SessionKey::new("emp-a");
        let b = SessionKey::new("emp-b");
        seed(&db, a.id(), &["a1", "a2"]);
        seed(&db, b.id(), &["b1"]);

        store.acquire(&a);
        store.acquire(&b);

        assert_eq!(store.peek(&a).len(), 2);
        assert_eq!(store.peek(&b).len(), 1, "b must not see a's turns");
    }

    #[test]
    fn append_lands_in_the_addressed_session() {
        let (db, store) = store_with(8);
        let a = SessionKey::new("emp-a");
        let b = SessionKey::new("emp-b");
        seed(&db, a.id(), &["a1"]);
        seed(&db, b.id(), &["b1"]);

        store.acquire(&a);
        store.acquire(&b);
        store.append(&b, Message::user("b2"));

        assert_eq!(store.peek(&b).len(), 2);
        assert_eq!(store.peek(&a).len(), 1, "a untouched by b's turn");
    }

    #[test]
    fn append_to_a_cold_session_loads_it_first() {
        let (db, store) = store_with(8);
        let key = SessionKey::new("emp-cold");
        seed(&db, key.id(), &["one"]);

        store.append(&key, Message::user("two"));

        assert_eq!(store.peek(&key).len(), 2, "cold append must not be lost");
    }

    #[test]
    fn warm_cache_is_bounded_and_evicts_lru() {
        let (db, store) = store_with(2);
        let a = SessionKey::new("a");
        let b = SessionKey::new("b");
        let c = SessionKey::new("c");
        for k in [&a, &b, &c] {
            seed(&db, k.id(), &["x"]);
        }

        store.acquire(&a);
        store.acquire(&b);
        assert!(store.is_warm(&a) && store.is_warm(&b));

        store.acquire(&c);
        assert!(store.stats().warm <= 2, "cache stays bounded");
        assert_eq!(store.stats().evictions, 1, "exactly one eviction");
        assert!(store.is_warm(&c), "newest session stays resident");
    }

    #[test]
    fn release_is_not_a_wipe() {
        let (db, store) = store_with(8);
        let key = SessionKey::new("emp-r");
        seed(&db, key.id(), &["keep me"]);

        store.acquire(&key);
        store.release(&key);

        assert_eq!(
            store.peek(&key).len(),
            1,
            "release must not discard the transcript"
        );
    }

    #[test]
    fn evict_colds_the_session_without_losing_it() {
        let (db, store) = store_with(8);
        let key = SessionKey::new("emp-e");
        seed(&db, key.id(), &["one", "two"]);

        store.acquire(&key);
        assert!(store.evict(&key));
        assert!(!store.is_warm(&key), "no longer resident");

        assert_eq!(store.acquire(&key), 2, "rehydrates from disk intact");
    }

    #[test]
    fn a_fresh_cache_has_no_warm_sessions() {
        let (_db, store) = store_with(8);
        assert_eq!(store.stats().warm, 0);
        assert_eq!(store.stats().cold_loads, 0);
    }

    #[test]
    fn repeated_acquire_is_one_load() {
        let (db, store) = store_with(8);
        let key = SessionKey::new("emp-r2");
        seed(&db, key.id(), &["only"]);

        store.acquire(&key);
        store.acquire(&key);
        store.acquire(&key);

        assert_eq!(store.stats().cold_loads, 1, "second acquire is a warm hit");
        assert_eq!(store.stats().warm_hits, 2);
    }
}
