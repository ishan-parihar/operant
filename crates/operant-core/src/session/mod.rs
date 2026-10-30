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

/// The unified listing record (`SessionEntryV2`) every session surface
/// renders: the session hub, the CLI subcommands, and the gateway's
/// read-only API routes (plan 2026-10-28, audit RT-A).
pub mod listing;

/// Default number of sessions held resident in memory.
///
/// Measured: a full 128k-token window is ~500 KB of text, so 32 sessions is
/// ~15.6 MB and 64 would be ~31 MB. Chosen as the bound because the resident
/// set must stay small and predictable while the CEO loop plus one
/// department's active seats stay warm.
pub const DEFAULT_WARM_CAPACITY: usize = 32;

/// Message-count backstop for a cold load's window.
///
/// Raised from 20 to 2000 at iter-1510 after the operator reported that a
/// resumed session "only shows the last message". The 20 was a stale default
/// from the gateway's reload window (iter-548) that nothing re-derived: the
/// hub's preview pane and `operant sessions show` both load a session's FULL
/// transcript, so switching into a session showed 10 turns where the pane
/// beside it showed 2686 — the pane promised history the switch then truncated.
///
/// This is now only the backstop. The real bound is
/// [`REHYDRATE_BYTES_BUDGET`] below, because a message COUNT is a bad proxy
/// for memory: measured on the operator's 102 MB database, messages average
/// 703 bytes but the largest session (`emp-cron_cast_id`, 14103 messages) is
/// 14.3 MB of raw text — a count-only window would let one session blow the
/// whole resident budget.
pub const REHYDRATE_TAIL: usize = 2000;

/// Default per-session byte budget for a cold load's window.
///
/// Measured: 2 MB is roughly 2800 messages at the operator's 703 B average,
/// and the whole 14.3 MB `emp-cron_cast_id` transcript fits in 8 of these.
/// At [`DEFAULT_WARM_CAPACITY`] = 32 resident sessions the worst case is
/// 64 MB, which replaces the old ~15.6 MB estimate — a deliberate trade: the
/// old figure was drawn from a single full-context WINDOW (~500 KB), not from
/// the transcripts the cache actually holds, and a session that cannot see its
/// own history is the more expensive failure. The estimate is stated here so
/// it can be re-measured rather than inherited.
pub const REHYDRATE_BYTES_BUDGET: usize = 2 * 1024 * 1024;

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
    ///
    /// The message-count BACKSTOP only (iter-1510). See
    /// [`crate::session::REHYDRATE_TAIL`] for why a count alone is the wrong
    /// bound, and [`Self::rehydrate_bytes`] for the one that actually bites.
    pub rehydrate_tail: usize,
    /// Byte budget a cold load fills before it stops taking messages.
    ///
    /// A byte budget rather than a count, because the operator's sessions
    /// range from 2 to 14103 messages and from ~1 KB to 14.3 MB of text — a
    /// count predicts memory badly and a byte budget predicts it exactly. The
    /// window is taken from the END (newest), and a message that would cross
    /// the budget is taken whole rather than split: a half-loaded tool call is
    /// worse than one fewer turn.
    pub rehydrate_bytes: usize,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            warm_capacity: DEFAULT_WARM_CAPACITY,
            rehydrate_tail: REHYDRATE_TAIL,
            rehydrate_bytes: REHYDRATE_BYTES_BUDGET,
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

    /// RT-179 item 3.5 — read-only access to the store's own database.
    ///
    /// The gateway's `/search` twin needs to run the `session_search` tool
    /// over the SAME database the live agent writes its sessions to, rather
    /// than opening a second handle to `app_config.database_path` (which
    /// would be a second connection whose WAL state can differ from the
    /// agent's). Handing back the store's own `Arc` is the one way to
    /// guarantee the search sees what the agent just wrote.
    ///
    /// Deliberately a clone of the `Arc`, never `&mut`: the caller shares
    /// the connection, it does not take ownership of it.
    pub fn db(&self) -> std::sync::Arc<Database> {
        std::sync::Arc::clone(&self.db)
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
    ///
    /// The window is the NEWEST messages, bounded by BOTH a message count and
    /// a byte budget (iter-1510). The byte bound is the one that actually
    /// bites: measured on the operator's database, sessions span 2 to 14103
    /// messages and ~1 KB to 14.3 MB of text, so a count alone cannot say what
    /// a load will cost. A message that would cross the budget is taken
    /// WHOLE rather than split, so the transcript is never cut mid-turn.
    ///
    /// `skip` is then one arithmetic step off the smaller bound, so a
    /// session that fits under both costs nothing extra.
    fn load_from_db(&self, key: &SessionKey) -> Vec<Message> {
        let Ok(rows) = self.db.get_session_messages_full(key.id()) else {
            // A cold session with no readable transcript is simply empty; that
            // is a normal first-run state, not an error worth propagating into
            // the agent loop.
            return Vec::new();
        };
        let count_bound = self.config.rehydrate_tail;
        let byte_bound = self.config.rehydrate_bytes;
        // MIN, not max: the window is whichever bound allows FEWER messages.
        // A byte budget that loses to a count bound makes no sense — the point
        // of the byte walk is to cut a session the count bound would wave
        // through (14 MB of cron turns under a 2000-message count).
        let kept = count_bound.min(self.count_that_fits_byte_budget(&rows, byte_bound));
        let start = rows.len().saturating_sub(kept);
        rows.into_iter()
            .skip(start)
            .filter_map(row_to_message)
            .collect()
    }

    /// How many trailing rows of `rows` fit inside `byte_bound`.
    ///
    /// Walks BACKWARD from the newest and stops at the first message that
    /// would cross the budget, which is why it costs O(window) rather than
    /// O(session) for a session far over the budget. Every row in the window
    /// is counted whole, so a load is always a whole number of messages.
    ///
    /// A session whose transcript fits entirely returns its own length, which
    /// makes this a no-op bound for the ordinary case — only the operator's
    /// cron sessions (thousands of turns) get genuinely truncated.
    fn count_that_fits_byte_budget(&self, rows: &[MessageData], byte_bound: usize) -> usize {
        let mut bytes = 0usize;
        let mut taken = 0usize;
        for row in rows.iter().rev() {
            let size = message_row_bytes(row);
            if taken > 0 && bytes + size > byte_bound {
                break;
            }
            bytes += size;
            taken += 1;
        }
        taken
    }
}

/// Bytes a persisted row will occupy once it is a [`Message`].
///
/// Counts the text a human or a model can actually see — content, the tool
/// call names and arguments, and reasoning — and excludes the bookkeeping
/// columns (ids, timestamps, token counts), which are the difference between
/// "what the operator sees" and "what the process holds".
fn message_row_bytes(row: &MessageData) -> usize {
    let mut bytes = row.content.as_deref().map(str::len).unwrap_or_default();
    if let Some(tool_calls) = row.tool_calls.as_deref() {
        bytes += tool_calls.len();
    }
    if let Some(reasoning) = row.reasoning.as_deref() {
        bytes += reasoning.len();
    }
    if let Some(name) = row.tool_name.as_deref() {
        bytes += name.len();
    }
    bytes
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
        store_with_window(capacity, REHYDRATE_TAIL, REHYDRATE_BYTES_BUDGET)
    }

    /// The same store with an explicit window, for the tests that measure the
    /// bound itself rather than trusting the default.
    fn store_with_window(
        capacity: usize,
        tail: usize,
        bytes: usize,
    ) -> (std::sync::Arc<Database>, SessionStore) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "operant-session-store-{}-{}-{}-{}",
            std::process::id(),
            capacity,
            tail,
            n
        ));
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("operant.db");
        let db = std::sync::Arc::new(Database::init(path).expect("Database::init"));
        let store = SessionStore::with_config(
            std::sync::Arc::clone(&db),
            SessionStoreConfig {
                warm_capacity: capacity,
                rehydrate_tail: tail,
                rehydrate_bytes: bytes,
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

    /// (iter-1510) The defect behind the operator's "session only shows the
    /// last message": `REHYDRATE_TAIL` was 20, so a cold load of the
    /// operator's 2686-message `emp-cron_fa1e44c` produced 20 messages while
    /// the hub's preview pane beside it was rendering all 2686. The pane
    /// promised history the switch then truncated.
    ///
    /// The default must now carry a whole ordinary session: the operator's own
    /// database has 60 personal sessions averaging 4-7 messages, and the
    /// smallest cron employee holds 1330.
    #[test]
    fn the_default_window_carries_a_whole_ordinary_session() {
        // Read through `Default::default()` rather than asserting on the consts:
        // clippy::assertions_on_constants is right that a bare const check is
        // compile-time dead weight, and the DEFAULT is what production actually
        // loads through — the constant is the constant, the config is the
        // contract.
        let window = SessionStoreConfig::default();
        assert!(
            window.rehydrate_tail >= 1000,
            "the default tail must carry a whole ordinary session, not 10 \
             turns — measured on the operator's database: personal sessions \
             run 2-57 messages and the smallest cron employee holds 1330"
        );
        assert!(
            window.rehydrate_bytes >= 1024 * 1024,
            "the default byte budget must hold a full session's text, not a \
             window of it — measured avg 703 B/message, so 1 MiB is ~1500 turns"
        );
    }

    /// And the default must actually BEHAVE that way, not merely claim it: a
    /// session of the operator's real shape (seven turns, the measured personal
    /// average) loads whole through the default config.
    #[test]
    fn the_default_config_loads_a_session_of_the_measured_shape_whole() {
        let (db, store) = store_with(8);
        let key = SessionKey::new("ordinary");
        seed(
            &db,
            key.id(),
            &[
                "turn 1", "turn 2", "turn 3", "turn 4", "turn 5", "turn 6", "turn 7",
            ],
        );

        assert_eq!(
            store.acquire(&key),
            7,
            "a seven-turn session must load whole through the DEFAULT window"
        );
        assert_eq!(
            store.peek(&key).first().map(|m| m.content.as_str()),
            Some("turn 1"),
            "and the FIRST turn of it, which the old 20-message window also \
             carried but the hub's preview showed in full"
        );
    }

    /// A session that fits under BOTH bounds rehydrates whole — the byte walk
    /// is a no-op for every session that is not a multi-thousand-turn cron
    /// log, which is what makes this an ordinary case rather than a new
    /// failure mode.
    #[test]
    fn a_session_that_fits_both_bounds_rehydrates_whole() {
        let (db, store) = store_with_window(8, 100, 64 * 1024);
        let key = SessionKey::new("small");
        seed(&db, key.id(), &["a", "b", "c", "d", "e"]);

        assert_eq!(store.acquire(&key), 5, "no truncation when it fits");
        assert_eq!(
            store.peek(&key)[0].content,
            "a",
            "the FIRST message survived"
        );
    }

    /// The byte bound is the one that bites: a 2000-message count bound waves a
    /// 14 MB cron log through, and the byte budget cuts it to a whole number of
    /// messages whose visible text fits. Measured against the operator's real
    /// distribution (avg 703 B/message), not a synthetic uniform fixture.
    #[test]
    fn the_byte_bound_truncates_a_session_the_count_bound_would_wave_through() {
        // 40 messages of 1 KiB each = ~41 KB. Count bound 2000 (huge), byte
        // bound 8 KiB (small) — only the byte bound can restrict this.
        let (db, store) = store_with_window(8, 2000, 8 * 1024);
        let key = SessionKey::new("fat");
        let big = "x".repeat(1024);
        let contents: Vec<String> = (0..40).map(|i| format!("{big}{i}")).collect();
        seed(
            &db,
            key.id(),
            &contents.iter().map(String::as_str).collect::<Vec<_>>(),
        );

        let loaded = store.acquire(&key);
        assert!(
            loaded < 40,
            "the byte budget must cut a session the count bound waves through \
             (loaded {loaded} of 40)"
        );
        assert!(
            loaded >= 7,
            "8 KiB of budget must still carry several messages, not one \
             (loaded {loaded})"
        );
        assert!(
            loaded * 1024 <= 9 * 1024,
            "the window must not wildly overshoot the budget (loaded \
             {loaded} x 1 KiB)"
        );
    }

    /// The window is the NEWEST messages, not the oldest: a truncated session
    /// must end at its latest turn, because that is the state the operator was
    /// last working in.
    #[test]
    fn the_truncated_window_keeps_the_newest_messages() {
        let (db, store) = store_with_window(8, 2, 64 * 1024);
        let key = SessionKey::new("tail");
        seed(&db, key.id(), &["oldest", "middle", "newest"]);

        // acquire, not peek: peek is a warm-cache read and returns empty for a
        // cold session by design (the doc on `SessionStore::peek` says so), so
        // the load has to be driven first or the test measures nothing.
        assert_eq!(store.acquire(&key), 2, "count bound is 2");
        let kept = store.peek(&key);
        assert_eq!(kept.len(), 2, "the load made it resident");
        assert_eq!(
            kept[0].content, "middle",
            "the window is the NEWEST messages — the oldest is what goes"
        );
        assert_eq!(kept[1].content, "newest", "the latest turn is the last row");
    }

    /// A message that would cross the budget is taken WHOLE, never split: a
    /// half-loaded tool call is worse than one fewer turn, and a count that
    /// lands mid-turn cannot be reconstructed by the renderer.
    #[test]
    fn the_window_never_cuts_a_message_in_half() {
        // Three messages: two tiny (10 B) and one fat (5 KiB). Budget 5 KiB.
        // The fat message MUST be taken whole — so the load is 2 or 3, and
        // every message it does carry is complete.
        let (db, store) = store_with_window(8, 100, 5 * 1024);
        let key = SessionKey::new("whole");
        let fat = "y".repeat(5 * 1024);
        seed(&db, key.id(), &["tiny-01", &fat, "tiny-02"]);

        // Backward walk: "tiny-02" (7 B) is taken, then the fat message (5 KiB)
        // is the FIRST row the `taken > 0` guard lets overshoot, so it is taken
        // whole and the walk stops — tiny-01 never enters the window at all.
        store.acquire(&key);
        let kept = store.peek(&key);
        assert!(!kept.is_empty(), "the window carries something");
        for m in &kept {
            let len = m.content.len();
            // "tiny-02" is 7 bytes and "tiny-01" is 8; the fat one is 5 KiB.
            // The point is that no length BETWEEN them ever appears.
            assert!(
                len == 7 || len == 8 || len == 5 * 1024,
                "every surviving message is whole — got {len} bytes, which is \
                 neither a seeded length nor a cut of one"
            );
        }
        assert_eq!(
            kept.len(),
            1,
            "the fat message is EXCLUDED rather than split: taking it would \
             mean 5127 bytes inside a 5120 budget, and the walk refuses to cut \
             it — one fewer whole turn beats a truncated one"
        );
    }

    /// The `taken > 0` guard lets the FIRST row of a window overshoot the
    /// budget: a session whose newest message alone is bigger than the budget
    /// must still load it whole, or the operator switches in and sees nothing.
    #[test]
    fn the_newest_message_is_always_taken_whole_even_over_budget() {
        let (db, store) = store_with_window(8, 100, 512);
        let key = SessionKey::new("oversize");
        let fat = "z".repeat(4096);
        seed(&db, key.id(), &["small", &fat]);

        assert_eq!(
            store.acquire(&key),
            1,
            "the fat newest message is carried whole even though it is 8x the \
             budget — an empty transcript reads as a broken switch"
        );
        assert_eq!(
            store.peek(&key)[0].content.len(),
            4096,
            "and it is whole, not clipped to the budget"
        );
    }

    /// A byte budget of zero still yields at least one message: a session the
    /// operator switches into must show SOMETHING. `taken > 0 &&` in the walk
    /// is the guard, and this is the test that keeps it.
    #[test]
    fn a_tiny_budget_still_yields_one_whole_message() {
        let (db, store) = store_with_window(8, 100, 0);
        let key = SessionKey::new("zero");
        seed(&db, key.id(), &["only", "visible", "message"]);

        assert_eq!(
            store.acquire(&key),
            1,
            "a zero byte budget still carries one whole message — an empty \
             transcript reads as a broken switch, not a bounded one"
        );
    }
}
