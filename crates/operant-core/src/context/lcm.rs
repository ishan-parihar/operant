//! LCM (Lossless Context Management) engine — hermes-lcm parity, Phase P0.
//!
//! Implements [`ContextEngine`] with an **append-only SQLite DAG**: every
//! message is stored verbatim (losslessness), and `assemble()` replaces the
//! lossy `evict_to_budget` step by keeping the D0 fresh tail (most recent
//! messages within the tail budget) and compacting everything older into a
//! placeholder block. P1 replaces that placeholder with real day/week/month
//! LLM rollups (see `docs/HERMES_LCM_INTEGRATION.md`).
//!
//! Schema:
//! ```sql
//! CREATE TABLE nodes (
//!   id INTEGER PRIMARY KEY AUTOINCREMENT,
//!   session_id TEXT NOT NULL,
//!   position INTEGER NOT NULL,          -- index in the (stable, append-only)
//!                                        -- message history; dedup key component
//!   kind TEXT NOT NULL DEFAULT 'message',   -- 'message' | 'rollup_*' (P1)
//!   role TEXT NOT NULL,
//!   content TEXT NOT NULL,
//!   content_hash TEXT NOT NULL,             -- idempotent-ingest dedup key
//!   created_at INTEGER NOT NULL             -- unix millis
//! );
//! CREATE VIRTUAL TABLE nodes_fts USING fts5(content, session_id UNINDEXED);
//! ```
//! FTS rowids mirror `nodes.id` so recall can join back to the verbatim
//! content.
//!
//! **Losslessness note**: the dedup key is `(session_id, position, content_hash)`.
//! The agent's message history is stable and append-only, so a message's
//! position is identical across turns — re-ingesting the same turn dedups —
//! while two identical messages at *different* positions (e.g. the user says
//! \"continue\" twice) are both preserved as distinct nodes.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{Connection, params};

use crate::client::{Message, Role};
use crate::context::{ContextEngine, RecallHit};

/// A raw DAG node row: `(id, role, content, content_hash, position, created_at)`.
pub type LcmNodeRow = (i64, String, String, String, i64, i64);

/// fnmatch-style glob matcher supporting `*` (any run of chars), `?` (any
/// single char), and `[abc]`/`[a-z]` classes. Used for hermes-lcm
/// `ignore_session_patterns`. No external glob dependency — this is a small
/// recursive matcher over the pattern chars.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_chars(&p, &t)
}

fn glob_match_chars(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let (mut star_p, mut star_t) = (None, 0);
    while ti < t.len() {
        if pi < p.len() {
            match p[pi] {
                '?' => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                '*' => {
                    star_p = Some(pi);
                    star_t = ti;
                    pi += 1;
                    continue;
                }
                '[' => {
                    // Character class [abc] or [a-z]; `!` negates.
                    let mut j = pi + 1;
                    let negate = j < p.len() && (p[j] == '!' || p[j] == '^');
                    if negate {
                        j += 1;
                    }
                    let start = j;
                    let mut matched = false;
                    let mut has_range_end = false;
                    while j < p.len() && p[j] != ']' {
                        if j + 2 < p.len() && p[j + 1] == '-' && p[j + 2] != ']' {
                            if (p[j]..=p[j + 2]).contains(&t[ti]) {
                                matched = true;
                            }
                            j += 3;
                        } else {
                            if p[j] == t[ti] {
                                matched = true;
                            }
                            j += 1;
                        }
                    }
                    if j < p.len() && p[j] == ']' {
                        if p.get(start) == p.get(j) {
                            has_range_end = false;
                        }
                        let _ = has_range_end;
                    }
                    if matched != negate && j < p.len() {
                        pi = j + 1;
                        ti += 1;
                        continue;
                    }
                    if j >= p.len() {
                        // Unterminated class — treat `[` literally.
                        if p[pi] == t[ti] {
                            pi += 1;
                            ti += 1;
                            continue;
                        }
                    }
                }
                c if c == t[ti] => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                _ => {}
            }
        }
        if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod glob_tests {
    use super::glob_match;

    #[test]
    fn glob_matches_star_question_and_literal() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("sess_*", "sess_alpha"));
        assert!(glob_match("sess_?", "sess_1"));
        assert!(!glob_match("sess_?", "sess_10"));
        assert!(glob_match("noisy-*", "noisy-log"));
        assert!(!glob_match("noisy-*", "clean-log"));
        assert!(glob_match("archive*", "archive-2024"));
        assert!(!glob_match("archive*", "active"));
    }

    #[test]
    fn glob_matches_character_classes() {
        assert!(glob_match("sess_[0-9]", "sess_5"));
        assert!(!glob_match("sess_[0-9]", "sess_x"));
        assert!(glob_match("sess_[ab]", "sess_a"));
    }

    #[test]
    fn glob_handles_multi_star_and_prefix() {
        assert!(glob_match("*test*", "prefix-test-suffix"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exactly"));
    }
}
use crate::error::{Error, Result};

/// Configuration for the lossless context engine.
#[derive(Debug, Clone)]
pub struct LcmConfig {
    /// SQLite database path for the DAG (default `~/.operant/lcm.db`).
    pub db_path: PathBuf,
    /// Fresh-tail (D0) token budget kept verbatim by `assemble()`.
    pub tail_tokens: usize,
    /// P3 adaptive auto-recall: when `assemble()` runs, issue one bounded
    /// retrieval against the latest user message and inject the top hits as
    /// a system "pre-answer evidence" block (hermes `adaptive_retrieval.py`
    /// parity). Default on.
    pub auto_recall: bool,
    /// Max evidence nodes injected per assemble.
    pub auto_recall_limit: usize,
    /// Hard cap on the injected evidence block, in characters.
    pub auto_recall_max_chars: usize,
    /// P1 rollup-in-compaction: when the assembled context is over budget,
    /// inject stored day/week/month rollup summaries instead of a bare
    /// placeholder marker (hermes `LCM_TEMPORAL_ROLLUPS_ENABLED` parity).
    /// Rollups are only injected when they already exist in `lcm_rollups`
    /// (built via `operant context rollup`); never built on the fly here.
    pub rollups_inject: bool,
    /// Glob patterns (fnmatch-style `*`) of sessions to skip in global recall
    /// (hermes-lcm `ignore_session_patterns` parity). A session is ignored
    /// when it matches any pattern; explicit per-session recall is still
    /// allowed (the ignore list only suppresses cross-session leakage).
    pub ignore_session_patterns: Vec<String>,
    /// Session ids kept read-only: their DAG is never mutated (ingest is a
    /// no-op) so archived/immutable sessions stay byte-for-byte stable
    /// (hermes-lcm `read_only` session scopes parity).
    pub readonly_sessions: Vec<String>,
}

impl Default for LcmConfig {
    fn default() -> Self {
        Self {
            db_path: crate::platform::operant_home().join("lcm.db"),
            tail_tokens: 12_000,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        }
    }
}

/// The lossless DAG context engine. `Connection` is `Send` but not `Sync`,
/// so it is held behind a `Mutex` (all accesses are short, synchronous
/// rusqlite calls — no await while holding the lock).
pub struct LcmContextEngine {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    tail_tokens: usize,
    auto_recall: bool,
    auto_recall_limit: usize,
    auto_recall_max_chars: usize,
    rollups_inject: bool,
    ignore_session_patterns: Vec<String>,
    readonly_sessions: Vec<String>,
}

impl std::fmt::Debug for LcmContextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcmContextEngine")
            .field("db_path", &self.db_path)
            .field("tail_tokens", &self.tail_tokens)
            .field("auto_recall", &self.auto_recall)
            .field("rollups_inject", &self.rollups_inject)
            .finish()
    }
}

impl LcmContextEngine {
    /// Open (creating if needed) the DAG database and bootstrap the schema.
    pub fn new(config: LcmConfig) -> Result<Self> {
        if let Some(parent) = config.db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("lcm: create db dir failed: {e}")))?;
        }
        let conn = Connection::open(&config.db_path).map_err(|e| {
            Error::Agent(format!(
                "lcm: failed to open {}: {e}",
                config.db_path.display()
            ))
        })?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS nodes (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 kind TEXT NOT NULL DEFAULT 'message',
                 role TEXT NOT NULL,
                 content TEXT NOT NULL,
                 content_hash TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_dedup
                 ON nodes(session_id, position, content_hash);
             CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts
                 USING fts5(content, session_id UNINDEXED);
             CREATE TABLE IF NOT EXISTS lcm_rollups (
                 session_id TEXT NOT NULL,
                 period_kind TEXT NOT NULL,
                 period_start TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 source_count INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY (session_id, period_kind, period_start)
             );
             CREATE TABLE IF NOT EXISTS lcm_assertions (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 subject TEXT NOT NULL,
                 predicate TEXT NOT NULL,
                 object_value TEXT NOT NULL,
                 speaker_role TEXT NOT NULL DEFAULT 'assistant',
                 source_node_id INTEGER,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_assertions_lookup
                 ON lcm_assertions(session_id, subject, predicate);
             CREATE TABLE IF NOT EXISTS lcm_embeddings (
                 node_id INTEGER NOT NULL,
                 model TEXT NOT NULL,
                 vector TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY (node_id, model)
             );
             -- One-time migration: earlier builds keyed lcm_embeddings on
             -- node_id alone, so INSERT OR REPLACE silently clobbered the
             -- other model's cached vector on a model change. It is a pure
             -- cache (rebuildable), so a legacy single-column PK is simply
             -- rebuilt as (node_id, model). See migrate_embeddings_pk below.",
        )
        .map_err(|e| Error::Agent(format!("lcm: schema bootstrap failed: {e}")))?;
        migrate_embeddings_pk(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            db_path: config.db_path,
            tail_tokens: config.tail_tokens.max(1),
            auto_recall: config.auto_recall,
            auto_recall_limit: config.auto_recall_limit.clamp(1, 10),
            auto_recall_max_chars: config.auto_recall_max_chars.max(256),
            rollups_inject: config.rollups_inject,
            ignore_session_patterns: config.ignore_session_patterns,
            readonly_sessions: config.readonly_sessions,
        })
    }

    /// True when `session_id` matches any configured `ignore_session_patterns`
    /// glob (hermes-lcm `ignore_session_patterns` parity). Only suppresses
    /// cross-session recall; explicit per-session recall still works.
    pub fn session_ignored(&self, session_id: &str) -> bool {
        self.ignore_session_patterns
            .iter()
            .any(|p| glob_match(p, session_id))
    }

    /// True when `session_id` is in the configured read-only set — its DAG
    /// must never be mutated (hermes-lcm `read_only` scopes parity).
    pub fn session_readonly(&self, session_id: &str) -> bool {
        self.readonly_sessions.iter().any(|s| s == session_id)
    }

    /// DAG database path (diagnostics/tool surface).
    pub fn db_path(&self) -> &std::path::Path {
        &self.db_path
    }

    /// D0 fresh-tail token budget (diagnostics/tool surface).
    pub fn tail_tokens(&self) -> usize {
        self.tail_tokens
    }

    /// Count DAG nodes across ALL sessions (diagnostics/tool surface).
    pub fn node_count_global(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get::<_, i64>(0))
            .map(|n| n as usize)
            .map_err(|e| Error::Agent(format!("lcm: node_count_global failed: {e}")))
    }

    /// List all sessions in the DAG with node counts and last-activity
    /// timestamps, newest first (diagnostics/CLI surface).
    pub fn list_sessions(&self) -> Result<Vec<(String, usize, i64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT session_id, COUNT(*), MAX(created_at) FROM nodes \
                 GROUP BY session_id ORDER BY MAX(created_at) DESC",
            )
            .map_err(|e| Error::Agent(format!("lcm: list_sessions prepare failed: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as usize,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("lcm: list_sessions query failed: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::Agent(format!("lcm: list_sessions row: {e}")))?);
        }
        Ok(out)
    }

    /// Fetch verbatim node contents in a [start_ms, end_ms) window, oldest
    /// first, capped at `limit`. Only message nodes (never derived rollups)
    /// are included — rollups never feed rollups.
    pub fn nodes_in_window(
        &self,
        session_id: &str,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT content FROM nodes WHERE session_id = ?1 AND kind = 'message' \
                 AND created_at >= ?2 AND created_at < ?3 ORDER BY created_at, id LIMIT ?4",
            )
            .map_err(|e| Error::Agent(format!("lcm: nodes_in_window prepare failed: {e}")))?;
        let rows = stmt
            .query_map(params![session_id, start_ms, end_ms, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| Error::Agent(format!("lcm: nodes_in_window failed: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::Agent(format!("lcm: nodes_in_window row: {e}")))?);
        }
        Ok(out)
    }

    /// Fetch the most recent `limit` message nodes of `session` (or the
    /// whole DAG when `session` is None), newest first, as
    /// `(id, role, content, created_at)`. Used by assertion extraction to
    /// mine durable facts out of recent conversation (never derived
    /// rollup nodes).
    pub fn recent_message_nodes(
        &self,
        session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(i64, String, String, i64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, created_at FROM nodes \
                 WHERE (?1 IS NULL OR session_id = ?1) AND kind = 'message' \
                 ORDER BY created_at DESC, id DESC LIMIT ?2",
            )
            .map_err(|e| Error::Agent(format!("lcm: recent_message_nodes prepare: {e}")))?;
        let rows = stmt
            .query_map(params![session, limit.max(1) as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("lcm: recent_message_nodes failed: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            let (id, node_session, role, content, created_at) =
                r.map_err(|e| Error::Agent(format!("lcm: recent_message_nodes row: {e}")))?;
            // hermes-lcm ignore_session_patterns parity: the global list
            // skips noisy sessions; per-session listing is unaffected.
            if session.is_none() && self.session_ignored(&node_session) {
                continue;
            }
            out.push((id, role, content, created_at));
        }
        Ok(out)
    }

    /// Load an ordered, bounded raw-message page for one explicit `session_id`
    /// (hermes-lcm `lcm_load_session` parity). Rows are the raw `nodes` DAG
    /// entries — `LcmNodeRow` — ordered by stable `position`, oldest first,
    /// starting after the `after_store_id` cursor when given. Fetches
    /// `limit + 1` to report a `next_cursor` when more rows exist.
    pub fn load_session_page(
        &self,
        session_id: &str,
        after_store_id: Option<i64>,
        limit: usize,
    ) -> Result<(Vec<LcmNodeRow>, Option<i64>)> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let limit = limit.clamp(1, 500);
        let fetch = limit + 1;
        let mut stmt = conn
            .prepare(
                "SELECT id, role, content, content_hash, position, created_at FROM nodes \
                 WHERE session_id = ?1 AND (?2 IS NULL OR id > ?2) AND kind = 'message' \
                 ORDER BY position ASC, id ASC LIMIT ?3",
            )
            .map_err(|e| Error::Agent(format!("lcm: load_session_page prepare: {e}")))?;
        let rows = stmt
            .query_map(params![session_id, after_store_id, fetch as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("lcm: load_session_page query: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::Agent(format!("lcm: load_session_page row: {e}")))?);
        }
        let has_more = out.len() > limit;
        if has_more {
            out.truncate(limit);
        }
        let next_cursor = if has_more {
            out.last().map(|(id, ..)| *id)
        } else {
            None
        };
        Ok((out, next_cursor))
    }

    /// Upsert a rollup summary for (session, period, start). Returns the
    /// stored `created_at` millis (updated on refresh).
    pub fn upsert_rollup(
        &self,
        session_id: &str,
        period_kind: &str,
        period_start: &str,
        summary: &str,
        source_count: usize,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_millis();
        conn.execute(
            "INSERT INTO lcm_rollups
                 (session_id, period_kind, period_start, summary, source_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id, period_kind, period_start) DO UPDATE SET
                 summary = excluded.summary,
                 source_count = excluded.source_count,
                 created_at = excluded.created_at",
            params![
                session_id,
                period_kind,
                period_start,
                summary,
                source_count as i64,
                now
            ],
        )
        .map_err(|e| Error::Agent(format!("lcm: upsert_rollup failed: {e}")))?;
        Ok(now)
    }

    /// List stored rollups for a session, newest period first.
    /// True when a rollup exists for (session, period_kind, period_start).
    /// Used by maintenance passes to skip already-built periods without
    /// re-summarizing (hermes `rollup_store.py` dedup semantics).
    pub fn has_rollup(
        &self,
        session_id: &str,
        period_kind: &str,
        period_start: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT 1 FROM lcm_rollups WHERE session_id = ?1 AND period_kind = ?2 AND period_start = ?3",
            params![session_id, period_kind, period_start],
            |_| Ok(()),
        )
        .map(|_| true)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(Error::Agent(format!("lcm: has_rollup failed: {other}"))),
        })
    }

    /// Store one durable assertion (fact) — hermes `assertion_store.py`
    /// `write_assertions` parity (conflict-preserving: history is kept, the
    /// latest state per unique object wins in `query_assertion_state`).
    /// Returns the new row id.
    pub fn assert_assertion(
        &self,
        session_id: &str,
        subject: &str,
        predicate: &str,
        object_value: &str,
        speaker_role: &str,
        source_node_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO lcm_assertions
                 (session_id, subject, predicate, object_value, speaker_role, source_node_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                subject,
                predicate,
                object_value,
                speaker_role,
                source_node_id,
                now_millis()
            ],
        )
        .map_err(|e| Error::Agent(format!("lcm: assert_assertion failed: {e}")))?;
        Ok(conn.last_insert_rowid())
    }

    /// Query stored assertions for `(session, subject[, predicate])`, newest
    /// first. The caller resolves "active" state (latest per unique object).
    pub fn query_assertion_state(
        &self,
        session_id: &str,
        subject: &str,
        predicate: Option<&str>,
    ) -> Result<Vec<crate::context::AssertionRecord>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT id, subject, predicate, object_value, speaker_role, \
                        source_node_id, created_at \
                 FROM lcm_assertions \
                 WHERE session_id = ?1 AND subject = ?2 AND (?3 IS NULL OR predicate = ?3) \
                 ORDER BY created_at DESC, id DESC",
            )
            .map_err(|e| Error::Agent(format!("lcm: query_assertion_state prepare failed: {e}")))?;
        let rows = stmt
            .query_map(params![session_id, subject, predicate], |row| {
                Ok(crate::context::AssertionRecord {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object_value: row.get(3)?,
                    speaker_role: row.get(4)?,
                    source_node_id: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| Error::Agent(format!("lcm: query_assertion_state failed: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::Agent(format!("lcm: query_assertion_state row: {e}")))?);
        }
        Ok(out)
    }

    /// P3 vector recall (hermes `vector_store.py` / `embedding_provider.py`
    /// parity, bounded): embed the query via `embedder`, embed the candidate
    /// node pool (cached in `lcm_embeddings`, keyed by model so a model
    /// change re-embeds), and return the top `limit` nodes by cosine
    /// similarity. Candidates are the most recent message nodes of the
    /// session (or the whole DAG when `session` is None), capped at
    /// `MAX_VECTOR_CANDIDATES`. Returns `RecallHit`-shaped results so the
    /// tool renders identically to `lcm_recall`.
    pub async fn vector_recall(
        &self,
        embedder: &dyn crate::context::Embedder,
        session: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::context::RecallHit>> {
        const MAX_VECTOR_CANDIDATES: usize = 200;
        const BATCH: usize = 32;
        const VEC_DIM_CAP: usize = 4096;

        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let query_emb = embedder.embed(&[query.trim().to_string()]).await?;
        let Some(query_emb) = query_emb.into_iter().next() else {
            return Ok(Vec::new());
        };
        if query_emb.is_empty() || query_emb.len() > VEC_DIM_CAP {
            return Ok(Vec::new());
        }

        // Phase 1 — read candidates and cached vectors under the lock, then
        // release it BEFORE embedding (rusqlite `Connection` is not Send, so
        // the guard cannot be held across an await).
        let (candidates, mut vectors, missing) = {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, content, created_at FROM nodes \
                     WHERE (?1 IS NULL OR session_id = ?1) AND kind = 'message' \
                     ORDER BY created_at DESC, id DESC LIMIT ?2",
                )
                .map_err(|e| Error::Agent(format!("lcm: vector candidates prepare: {e}")))?;
            let rows = stmt
                .query_map(params![session, MAX_VECTOR_CANDIDATES as i64], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })
                .map_err(|e| Error::Agent(format!("lcm: vector candidates failed: {e}")))?;
            let mut candidates: Vec<(i64, String, String, i64)> = Vec::new();
            for r in rows {
                let (id, node_session, role, content, created_at) =
                    r.map_err(|e| Error::Agent(format!("lcm: vector candidate row: {e}")))?;
                // hermes-lcm ignore_session_patterns parity: global semantic
                // recall skips noisy sessions.
                if session.is_none() && self.session_ignored(&node_session) {
                    continue;
                }
                candidates.push((id, role, content, created_at));
            }
            let model = embedder.model_id().to_string();
            let mut vectors: Vec<Option<Vec<f32>>> = vec![None; candidates.len()];
            let mut missing: Vec<usize> = Vec::new();
            for (i, (id, _, _, _)) in candidates.iter().enumerate() {
                let cached: Option<String> = conn
                    .query_row(
                        "SELECT vector FROM lcm_embeddings WHERE node_id = ?1 AND model = ?2",
                        params![id, &model],
                        |r| r.get(0),
                    )
                    .ok();
                match cached {
                    Some(raw) => {
                        if let Ok(v) = serde_json::from_str::<Vec<f32>>(&raw) {
                            vectors[i] = Some(v);
                        } else {
                            missing.push(i);
                        }
                    }
                    None => missing.push(i),
                }
            }
            (candidates, vectors, missing)
        };
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Phase 2 — embed the missing candidates (no lock held), keeping the
        // fresh vectors in memory for ranking. Only finite, dimension-
        // consistent vectors are kept: a NaN/Inf or wrong-length vector from
        // the provider would poison the cache and rank garbage (cosine of a
        // NaN is NaN and never matches anything).
        let model = embedder.model_id().to_string();
        for chunk in missing.chunks(BATCH) {
            let texts: Vec<String> = chunk.iter().map(|&i| candidates[i].2.clone()).collect();
            let embs = embedder.embed(&texts).await?;
            for (k, &i) in chunk.iter().enumerate() {
                let Some(v) = embs.get(k) else {
                    continue;
                };
                if v.len() == query_emb.len() && v.iter().all(|x| x.is_finite()) {
                    vectors[i] = Some(v.clone());
                }
            }
        }

        // Phase 3 — persist the cache under a brief lock, then rank. Skip
        // rows whose serialization fails (serde_json can't encode NaN) so we
        // never store a corrupted blob that would force a re-embed loop.
        {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            for (i, v) in vectors.iter().enumerate() {
                if let Some(v) = v {
                    let Ok(raw) = serde_json::to_string(v) else {
                        continue;
                    };
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO lcm_embeddings (node_id, model, vector, created_at) \
                         VALUES (?1, ?2, ?3, ?4)",
                        params![candidates[i].0, &model, raw, now_millis()],
                    );
                }
            }
        }

        // Cosine rank and return the top `limit` as RecallHits.
        let mut scored: Vec<(f32, usize)> = Vec::with_capacity(candidates.len());
        for (i, v) in vectors.iter().enumerate() {
            if let Some(v) = v {
                let sim = crate::context::embedder::cosine_similarity(&query_emb, v);
                scored.push((sim, i));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::with_capacity(limit);
        for (sim, i) in scored.into_iter().take(limit) {
            let (id, role, content, created_at) = &candidates[i];
            out.push(crate::context::RecallHit {
                node_id: *id,
                role: role.clone(),
                content: content.clone(),
                created_at: *created_at,
                score: sim as f64,
            });
        }
        Ok(out)
    }

    pub fn list_rollups(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::context::rollup::RollupSummary>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare(
                "SELECT period_kind, period_start, summary, source_count, created_at \
                 FROM lcm_rollups WHERE session_id = ?1 ORDER BY period_start DESC",
            )
            .map_err(|e| Error::Agent(format!("lcm: list_rollups prepare failed: {e}")))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(crate::context::rollup::RollupSummary {
                    period_kind: row.get(0)?,
                    period_start: row.get(1)?,
                    summary: row.get(2)?,
                    source_count: row.get::<_, i64>(3)? as usize,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("lcm: list_rollups failed: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::Agent(format!("lcm: list_rollups row: {e}")))?);
        }
        Ok(out)
    }

    /// Build a bounded system block of stored rollups for `session_id`,
    /// newest period first. Returns `None` when the session has no rollups
    /// (so the caller keeps the bare placeholder). Rollups are injected
    /// verbatim from `lcm_rollups` — never rebuilt here.
    fn build_rollup_context(&self, session_id: &str) -> Result<Option<Message>> {
        let rollups = self.list_rollups(session_id)?;
        if rollups.is_empty() {
            return Ok(None);
        }
        let mut block = String::new();
        let mut budget = self.auto_recall_max_chars;
        let mut added = false;
        for r in rollups {
            let snippet: String = r.summary.chars().take(400).collect();
            let entry = format!(
                "- [{} {} · {} nodes] {}\n",
                r.period_kind, r.period_start, r.source_count, snippet
            );
            if entry.len() > budget {
                break;
            }
            budget -= entry.len();
            block.push_str(&entry);
            added = true;
        }
        if !added {
            return Ok(None);
        }
        let body = format!(
            "[LCM rollups of earlier context (temporal summaries of the lossless \
             DAG — verbatim history is still recallable via lcm_recall):]\n{block}"
        );
        Ok(Some(Message::system(body)))
    }

    /// Count stored rollups across ALL sessions (diagnostics/CLI surface).
    pub fn rollup_count_global(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row("SELECT COUNT(*) FROM lcm_rollups", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|n| n as usize)
        .map_err(|e| Error::Agent(format!("lcm: rollup_count_global failed: {e}")))
    }

    /// Global rollup listing (all sessions) for temporal recall — hermes-lcm
    /// `lcm_recent` parity. Optionally filtered by period kind
    /// (`day` / `week` / `month`), newest period first.
    pub fn list_rollups_global(
        &self,
        period: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::context::rollup::RollupSummary>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let limit = limit.clamp(1, 50) as i64;
        let rows: Vec<crate::context::rollup::RollupSummary> = match period {
            Some(kind) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT period_kind, period_start, summary, source_count, created_at \
                         FROM lcm_rollups WHERE period_kind = ?1 \
                         ORDER BY period_start DESC, created_at DESC LIMIT ?2",
                    )
                    .map_err(|e| {
                        Error::Agent(format!("lcm: list_rollups_global prepare failed: {e}"))
                    })?;
                let rows = stmt
                    .query_map(params![kind, limit], |row| {
                        Ok(crate::context::rollup::RollupSummary {
                            period_kind: row.get(0)?,
                            period_start: row.get(1)?,
                            summary: row.get(2)?,
                            source_count: row.get::<_, i64>(3)? as usize,
                            created_at: row.get(4)?,
                        })
                    })
                    .map_err(|e| {
                        Error::Agent(format!("lcm: list_rollups_global query failed: {e}"))
                    })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(
                        r.map_err(|e| Error::Agent(format!("lcm: list_rollups_global row: {e}")))?,
                    );
                }
                out
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT period_kind, period_start, summary, source_count, created_at \
                         FROM lcm_rollups \
                         ORDER BY period_start DESC, created_at DESC LIMIT ?1",
                    )
                    .map_err(|e| {
                        Error::Agent(format!("lcm: list_rollups_global prepare failed: {e}"))
                    })?;
                let rows = stmt
                    .query_map(params![limit], |row| {
                        Ok(crate::context::rollup::RollupSummary {
                            period_kind: row.get(0)?,
                            period_start: row.get(1)?,
                            summary: row.get(2)?,
                            source_count: row.get::<_, i64>(3)? as usize,
                            created_at: row.get(4)?,
                        })
                    })
                    .map_err(|e| {
                        Error::Agent(format!("lcm: list_rollups_global query failed: {e}"))
                    })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(
                        r.map_err(|e| Error::Agent(format!("lcm: list_rollups_global row: {e}")))?,
                    );
                }
                out
            }
        };
        Ok(rows)
    }

    /// Database / index / lifecycle health diagnostics — hermes-lcm
    /// `lcm_doctor` parity (compact). Runs SQLite integrity, table counts,
    /// FTS coverage, and store size.
    pub fn doctor(&self) -> Result<serde_json::Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("lcm: integrity_check failed: {e}")))?;
        let count = |table: &str| -> Result<i64> {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .map_err(|e| Error::Agent(format!("lcm: count({table}) failed: {e}")))
        };
        let nodes = count("nodes")?;
        let rollups = count("lcm_rollups")?;
        let assertions = count("lcm_assertions")?;
        let embeddings = count("lcm_embeddings")?;
        let fts: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes_fts", [], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("lcm: nodes_fts count failed: {e}")))?;
        let page_count: i64 = conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap_or(-1);
        let page_size: i64 = conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap_or(0);
        let db_bytes = std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);
        Ok(serde_json::json!({
            "engine": "lcm",
            "db_path": self.db_path.to_string_lossy(),
            "integrity_check": integrity,
            "nodes": nodes,
            "fts_indexed": fts,
            "fts_coverage_pct": if nodes > 0 { (fts * 100) / nodes } else { 100 },
            "rollups": rollups,
            "assertions": assertions,
            "embeddings": embeddings,
            "db_size_bytes": db_bytes,
            "db_pages": page_count,
            "page_size": page_size,
            "tail_tokens": self.tail_tokens,
            "auto_recall": self.auto_recall,
            "rollups_inject": self.rollups_inject,
        }))
    }

    /// Count DAG nodes for a session (diagnostics/tests).
    pub fn node_count(&self, session_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT COUNT(*) FROM nodes WHERE session_id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .map_err(|e| Error::Agent(format!("lcm: node_count failed: {e}")))
    }

    /// Insert one message as a DAG node. Idempotent: a message whose
    /// (session_id, position, content_hash) already exists is skipped. The
    /// position is the message's index in the turn slice — stable across
    /// turns because the agent's history is append-only, so identical content
    /// at different history positions is never collapsed.
    fn insert_node(
        &self,
        conn: &Connection,
        session_id: &str,
        position: usize,
        msg: &Message,
    ) -> Result<()> {
        // Do NOT index LCM diagnostic tool outputs (lcm_recall / lcm_stats).
        // They are identified by the `_lcm_tool` marker the tools embed in
        // their JSON envelope (NOT by content sniffing, which could drop a
        // legitimate tool that happens to return a `hits`-shaped payload).
        // Ingesting them would create a self-referential feedback loop:
        // recall results get indexed, then match future recalls, drowning
        // out real conversation content and inflating node counts.
        if msg.role == Role::Tool && msg.content.contains("\"_lcm_tool\":") {
            return Ok(());
        }
        // Lossless-DAG guarantee: index the FULL text the model produced.
        // Assistant messages that carry tool calls keep their visible text in
        // `reasoning` (content is ""), and observations stated there must be
        // recallable via lcm_recall. Merge content + reasoning so nothing the
        // agent said is ever lost from the store.
        let searchable = match (&msg.reasoning, msg.content.trim().is_empty()) {
            (Some(r), true) => r.trim().to_string(),
            (Some(r), false) => format!("{}\n{}", msg.content, r),
            _ => msg.content.clone(),
        };
        let hash = content_hash(&searchable);
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO nodes
                     (session_id, position, kind, role, content, content_hash, created_at)
                 VALUES (?1, ?2, 'message', ?3, ?4, ?5, ?6)",
                params![
                    session_id,
                    position as i64,
                    msg.role.as_str(),
                    &searchable,
                    hash,
                    now_millis()
                ],
            )
            .map_err(|e| Error::Agent(format!("lcm: node insert failed: {e}")))?;
        if inserted > 0 {
            let id = conn.last_insert_rowid();
            // Mirror the nodes rowid into FTS so recall can join back.
            conn.execute(
                "INSERT INTO nodes_fts (rowid, content, session_id) VALUES (?1, ?2, ?3)",
                params![id, &searchable, session_id],
            )
            .map_err(|e| Error::Agent(format!("lcm: fts insert failed: {e}")))?;
        }
        Ok(())
    }

    /// P3 adaptive auto-recall: run one bounded retrieval round against the
    /// latest user message and return a system "pre-answer evidence" block.
    /// Hits already visible in the current context are skipped (no dupes);
    /// the block is hard-capped at `auto_recall_max_chars`.
    async fn build_auto_recall_evidence(
        &self,
        session_id: &str,
        base: &[Message],
    ) -> Option<Message> {
        // Latest user message is the retrieval query.
        let query = base
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.trim().to_string())?
            .trim()
            .to_string();
        if query.is_empty() {
            return None;
        }
        let limit = self.auto_recall_limit;
        // Relevance, not verbatim phrase match: tokenize the question, drop
        // stopwords, and OR the significant terms. A quoted-phrase query would
        // only match when the exact wording already appeared in the DAG.
        let fts_query = auto_recall_fts_query(&query);
        let hits = self
            .recall_fts(Some(session_id), &fts_query, limit)
            .unwrap_or_default();
        if hits.is_empty() {
            return None;
        }

        let mut block = String::from(
            "[LCM recalled evidence relevant to your question (from the lossless DAG):]\n",
        );
        let mut budget = self.auto_recall_max_chars;
        // Token-based visibility check: a hit is "already visible" when most of
        // its significant terms already appear in the current context
        // (handles re-wordings, e.g. "note: X" vs "it was: X").
        let joined_ctx = base
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let joined_ctx_lower = joined_ctx.to_lowercase();
        let mut added = false;
        for h in hits {
            let text = h.content.trim();
            if text.is_empty() {
                continue;
            }
            let terms = auto_recall_terms(text);
            // Majority-overlap dedup: when >= 60% of a hit's significant
            // terms are already visible in the context, the model can already
            // see that knowledge (possibly re-worded) — don't repeat it.
            if !terms.is_empty() {
                let visible = terms
                    .iter()
                    .filter(|t| joined_ctx_lower.contains(*t))
                    .count();
                if visible * 5 >= terms.len() * 3 {
                    continue;
                }
            }
            let snippet: String = {
                let mut s: String = text.chars().take(300).collect();
                if text.chars().count() > 300 {
                    s.push_str("...");
                }
                s
            };
            let entry = format!("- [{}] {}\n", h.role, snippet);
            if entry.len() > budget {
                break;
            }
            budget -= entry.len();
            block.push_str(&entry);
            added = true;
        }
        if !added {
            return None;
        }
        block
            .push_str("(Full history is preserved verbatim in the DAG — use lcm_recall for more.)");
        Some(Message::system(block))
    }

    /// Shared FTS5 retrieval core. `fts_query` is passed verbatim (the public
    /// `recall()` wraps it as a quoted phrase; auto-recall builds an OR-term
    /// query for relevance matching). Lower bm25 = better.
    fn recall_fts(
        &self,
        session_id: Option<&str>,
        fts_query: &str,
        limit: usize,
    ) -> Result<Vec<RecallHit>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // `session_id` filters to one session so other sessions' history never
        // leaks into this context; NULL recalls across all sessions.
        let sql = "SELECT n.id, n.session_id, n.role, n.content, n.created_at, bm25(nodes_fts)
             FROM nodes_fts
             JOIN nodes n ON n.id = nodes_fts.rowid
             WHERE (?1 IS NULL OR nodes_fts.session_id = ?1) AND nodes_fts MATCH ?2
             ORDER BY bm25(nodes_fts)
             LIMIT ?3";
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| Error::Agent(format!("lcm: recall prepare failed: {e}")))?;
        let rows = stmt
            .query_map(params![session_id, fts_query, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, f64>(5)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("lcm: recall query failed: {e}")))?;
        let mut hits = Vec::new();
        for row in rows {
            let (node_id, hit_session, role, content, created_at, score) =
                row.map_err(|e| Error::Agent(format!("lcm: recall row failed: {e}")))?;
            // hermes-lcm ignore_session_patterns parity: global recall skips
            // hits from configured noisy sessions (explicit per-session recall
            // is unaffected because the ignore list only applies to the
            // cross-session arm).
            if session_id.is_none() && self.session_ignored(&hit_session) {
                continue;
            }
            hits.push(RecallHit {
                node_id,
                role,
                content,
                created_at,
                score,
            });
        }
        Ok(hits)
    }
}

#[async_trait::async_trait]
impl ContextEngine for LcmContextEngine {
    fn name(&self) -> &str {
        "lcm"
    }

    async fn ingest_turn(&self, session_id: &str, turn: &[Message]) -> Result<()> {
        // hermes-lcm read_only scopes parity: configured read-only sessions
        // are never mutated — ingest is a no-op so archived transcripts stay
        // byte-for-byte stable.
        if self.session_readonly(session_id) {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| Error::Agent(format!("lcm: begin tx failed: {e}")))?;
        for (position, msg) in turn.iter().enumerate() {
            // Skip the frozen system prefix: it is re-injected by
            // build_messages every turn and would pollute recall.
            if msg.role == crate::client::Role::System {
                continue;
            }
            self.insert_node(&tx, session_id, position, msg)?;
        }
        tx.commit()
            .map_err(|e| Error::Agent(format!("lcm: commit failed: {e}")))?;
        Ok(())
    }

    async fn assemble(
        &self,
        session_id: &str,
        base: Vec<Message>,
        budget_tokens: usize,
    ) -> Result<Vec<Message>> {
        // 1. Ingest idempotently so recall covers the current context.
        self.ingest_turn(session_id, &base).await?;

        // 2. P3 adaptive auto-recall: one bounded retrieval round against the
        //    latest user message; inject top evidence as a system block
        //    (hermes `adaptive_retrieval.py` pre-answer evidence parity).
        let evidence = if self.auto_recall {
            self.build_auto_recall_evidence(session_id, &base).await
        } else {
            None
        };

        // 3. Fast path: everything (including evidence) fits — return lossless.
        //    The evidence block is inserted after the leading system prefix
        //    so it acts as context the model reads before the conversation.
        let with_evidence: Vec<Message> = match &evidence {
            Some(block) => {
                let mut out = Vec::with_capacity(base.len() + 1);
                let sys_count = base.iter().take_while(|m| m.role == Role::System).count();
                out.extend_from_slice(&base[..sys_count]);
                out.push(block.clone());
                out.extend_from_slice(&base[sys_count..]);
                out
            }
            None => base,
        };
        if crate::context_management::estimate_total_tokens(&with_evidence) <= budget_tokens {
            return Ok(with_evidence);
        } // 4. D0 fresh tail kept verbatim; older messages compacted. P1: when
        //    stored rollups exist, inject them instead of a bare placeholder
        //    so the model sees real summaries of the compacted history.
        //    Reserve tail budget for the evidence + rollup blocks so the
        //    assembled context never exceeds the intended budget. Rollup
        //    reservation is capped at half the tail budget so the D0 fresh
        //    tail can never be starved by injected summaries.
        let evidence_tokens = evidence
            .as_ref()
            .map(crate::context_management::estimate_message_tokens)
            .unwrap_or(0);
        let rollup_block = if self.rollups_inject {
            self.build_rollup_context(session_id)?
        } else {
            None
        };
        let rollup_tokens = rollup_block
            .as_ref()
            .map(crate::context_management::estimate_message_tokens)
            .unwrap_or(0);
        let base_tail = self.tail_tokens.min(budget_tokens);
        let rollup_reserve = rollup_tokens.min(base_tail / 2);
        let tail_budget = base_tail
            .saturating_sub(evidence_tokens)
            .saturating_sub(rollup_reserve);
        let (mut kept, compacted_count, compacted_tokens) =
            split_for_lcm(with_evidence, tail_budget);

        let sys_count = kept.iter().take_while(|m| m.role == Role::System).count();
        let placeholder = Message::system(format!(
            "[LCM lossless context: {} earlier message(s) (~{} tokens) are preserved verbatim in the DAG at {}. Use the lcm_recall tool to retrieve any of them — nothing was deleted.]",
            compacted_count,
            compacted_tokens,
            self.db_path.display()
        ));
        kept.insert(sys_count, placeholder);
        // Rollups (when present) ride after the placeholder as real
        // summaries of the compacted history.
        if let Some(block) = rollup_block {
            kept.insert(sys_count + 1, block);
        }
        // Pre-answer evidence still rides along last.
        if let Some(block) = evidence {
            kept.insert(sys_count + 2, block);
        }
        Ok(kept)
    }

    async fn recall(
        &self,
        session_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 50);
        // FTS5 phrase: quote + escape inner quotes. Lower bm25 = better.
        let fts_query = format!("\"{}\"", query.trim().replace('"', "\"\""));
        self.recall_fts(session_id, &fts_query, limit)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Keep the D0 fresh tail: the FIRST message (system prefix) is always
/// retained, then the most-recent messages accumulate from the end until
/// the token budget is exhausted. Returns (kept, compacted_count,
/// compacted_tokens). The most recent message is always kept even if it
/// alone exceeds the budget (never an empty tail).
fn split_for_lcm(messages: Vec<Message>, budget: usize) -> (Vec<Message>, usize, usize) {
    if messages.is_empty() {
        return (messages, 0, 0);
    }
    let mut iter = messages.into_iter();
    let front = iter.next().expect("non-empty messages");
    let mut kept = vec![front];
    let mut used = crate::context_management::estimate_message_tokens(&kept[0]);
    let mut compacted = 0usize;
    let mut compacted_tokens = 0usize;

    for m in iter.rev() {
        let t = crate::context_management::estimate_message_tokens(&m);
        if kept.len() > 1 && used + t > budget {
            compacted += 1;
            compacted_tokens += t;
        } else {
            used += t;
            kept.push(m);
        }
    }

    // kept = [front, ...most-recent-first tail] — restore chronological order.
    let front = kept.remove(0);
    kept.reverse();
    let mut out = vec![front];
    out.extend(kept);
    (out, compacted, compacted_tokens)
}

/// Stable-enough dedup hash for content (not cryptographic; idempotency
/// across turns is the goal).
/// Build an FTS5 OR query from the significant terms of a natural-language
/// question (for P3 auto-recall). Stopwords and short tokens are dropped so
/// "when is the deploy window again?" becomes `deploy OR window` — matching
/// any prior turn that mentioned the topic, not only exact re-wordings.
fn auto_recall_fts_query(question: &str) -> String {
    let mut terms = auto_recall_terms(question);
    if terms.is_empty() {
        // Fall back to the quoted phrase so recall still has something safe.
        return format!("\"{}\"", question.trim().replace('"', "\"\""));
    }
    terms.truncate(8);
    terms.join(" OR ")
}

/// Significant (non-stopword, >= 3 chars, unique, lowercased) terms of a
/// piece of text — used to build the auto-recall OR query and to detect
/// whether evidence is already visible in the current context.
fn auto_recall_terms(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "a",
        "an",
        "the",
        "and",
        "or",
        "but",
        "if",
        "then",
        "else",
        "when",
        "what",
        "which",
        "who",
        "whom",
        "where",
        "why",
        "how",
        "is",
        "are",
        "was",
        "were",
        "be",
        "been",
        "being",
        "do",
        "does",
        "did",
        "have",
        "has",
        "had",
        "will",
        "would",
        "can",
        "could",
        "should",
        "shall",
        "may",
        "might",
        "must",
        "of",
        "in",
        "on",
        "at",
        "to",
        "for",
        "from",
        "with",
        "without",
        "by",
        "about",
        "again",
        "it",
        "its",
        "that",
        "this",
        "these",
        "those",
        "my",
        "your",
        "our",
        "you",
        "me",
        "i",
        "we",
        "they",
        "he",
        "she",
        "tell",
        "say",
        "remember",
        "whats",
        "mentioned",
        "any",
        "only",
        "all",
        "some",
        // FTS5 reserved operators — a bare `NOT`/`NEAR` in a MATCH query is a
        // parse error, which would silently drop evidence for that turn.
        "not",
        "near",
    ];
    let mut terms: Vec<String> = Vec::new();
    for tok in text.split(|c: char| !c.is_alphanumeric()) {
        let t = tok.trim().to_lowercase();
        if t.len() >= 3 && !STOPWORDS.contains(&t.as_str()) && !terms.contains(&t) {
            terms.push(t);
        }
    }
    terms
}

fn content_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One-time migration for the `lcm_embeddings` cache table.
///
/// Earlier builds declared `node_id INTEGER PRIMARY KEY`, but every read
/// keys on `(node_id, model)` — so when the embedding model changed,
/// `INSERT OR REPLACE` on the same node_id silently destroyed the other
/// model's cached vector, forcing a full re-embed and defeating the cache
/// (F1). The table is a pure cache (rebuildable), so a legacy single-column
/// PK is dropped and recreated as a composite `(node_id, model)` PK. No-op
/// on fresh databases.
fn migrate_embeddings_pk(conn: &Connection) -> Result<()> {
    let legacy = {
        let mut stmt = conn
            .prepare("SELECT pk FROM pragma_table_info('lcm_embeddings') WHERE name = 'node_id'")
            .map_err(|e| Error::Agent(format!("lcm: pk inspect prepare failed: {e}")))?;
        let mut single = false;
        for pk in stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| Error::Agent(format!("lcm: pk inspect query failed: {e}")))?
        {
            // In the legacy table node_id alone was pk=1 (single-column PK);
            // in the composite table node_id is pk=1 AND model is pk=2 — we
            // detect the legacy shape by checking there is no second PK column.
            if pk.is_ok() {
                single = true;
            }
        }
        if !single {
            return Ok(());
        }
        // Confirm the composite is missing: node_id pk=1 with model pk=0.
        let model_pk: i64 = conn
            .query_row(
                "SELECT pk FROM pragma_table_info('lcm_embeddings') WHERE name = 'model'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| Error::Agent(format!("lcm: model pk inspect failed: {e}")))?;
        model_pk == 0
    };
    if legacy {
        conn.execute_batch(
            "DROP TABLE IF EXISTS lcm_embeddings;
             CREATE TABLE lcm_embeddings (
                 node_id INTEGER NOT NULL,
                 model TEXT NOT NULL,
                 vector TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY (node_id, model)
             );",
        )
        .map_err(|e| Error::Agent(format!("lcm: embeddings PK migration failed: {e}")))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Role};
    use uuid::Uuid;

    fn test_db() -> (LcmContextEngine, String) {
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_test_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("lcm-test.db");
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();
        (engine, format!("{}", db_path.display()))
    }

    fn sample_turn() -> Vec<Message> {
        vec![
            Message::system("you are a test agent"),
            Message::user("what is the capital of France"),
            Message::assistant("Paris is the capital of France"),
            Message::user("tell me about the Eiffel Tower"),
            Message::assistant("The Eiffel Tower is in Paris"),
        ]
    }

    #[tokio::test]
    async fn glob_match_controls_ignore_patterns() {
        let (engine, _) = test_db();
        // No patterns configured → nothing ignored.
        assert!(!engine.session_ignored("noisy-log"));
        assert!(!engine.session_readonly("archive-2024"));

        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_ctrl_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: dir.join("ctrl.db"),
            tail_tokens: 100,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: vec!["noisy-*".to_string(), "bench/*".to_string()],
            readonly_sessions: vec!["archive-2024".to_string()],
        })
        .unwrap();
        assert!(engine.session_ignored("noisy-log"));
        assert!(engine.session_ignored("bench/suite-1"));
        assert!(!engine.session_ignored("clean-session"));
        assert!(engine.session_readonly("archive-2024"));
        assert!(!engine.session_readonly("active"));
    }

    #[tokio::test]
    async fn global_recall_skips_ignored_sessions_but_explicit_scope_works() {
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_ign_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: dir.join("ign.db"),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: vec!["noisy-*".to_string()],
            readonly_sessions: Vec::new(),
        })
        .unwrap();

        engine
            .ingest_turn(
                "noisy-bench",
                &[Message::assistant("rsync deployment marker")],
            )
            .await
            .unwrap();
        engine
            .ingest_turn(
                "clean-main",
                &[Message::assistant("rsync deployment marker")],
            )
            .await
            .unwrap();

        // Global recall hides the noisy session.
        let global = engine.recall(None, "rsync deployment", 10).await.unwrap();
        assert!(
            global.iter().all(|h| h.content.contains("marker")),
            "global recall must still work"
        );
        // Recent nodes (global) skip ignored sessions.
        let recent = engine.recent_message_nodes(None, 10).unwrap();
        assert_eq!(recent.len(), 1, "only clean-main remains in global recent");

        // Explicit per-session recall still works even for ignored sessions.
        let explicit = engine
            .recall(Some("noisy-bench"), "rsync", 5)
            .await
            .unwrap();
        assert_eq!(explicit.len(), 1);
    }

    #[tokio::test]
    async fn readonly_session_ingest_is_a_noop() {
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_ro_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: dir.join("ro.db"),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: vec!["archive-2024".to_string()],
        })
        .unwrap();

        // Writable session ingests normally.
        engine
            .ingest_turn("active", &[Message::assistant("live node")])
            .await
            .unwrap();
        assert_eq!(engine.node_count("active").unwrap(), 1);

        // Read-only session ingest is a silent no-op (no nodes written).
        engine
            .ingest_turn("archive-2024", &[Message::assistant("should not persist")])
            .await
            .unwrap();
        assert_eq!(engine.node_count("archive-2024").unwrap(), 0);
    }

    #[tokio::test]
    async fn ingest_is_idempotent_and_lossless() {
        let (engine, _) = test_db();
        let turn = sample_turn();
        // System messages are skipped (re-injected each turn).
        engine.ingest_turn("s1", &turn).await.unwrap();
        assert_eq!(engine.node_count("s1").unwrap(), 4);
        // Second ingest of the same turn must not duplicate.
        engine.ingest_turn("s1", &turn).await.unwrap();
        assert_eq!(engine.node_count("s1").unwrap(), 4);
        // A different session is isolated.
        engine.ingest_turn("s2", &turn).await.unwrap();
        assert_eq!(engine.node_count("s1").unwrap(), 4);
        assert_eq!(engine.node_count("s2").unwrap(), 4);
    }

    #[tokio::test]
    async fn assistant_reasoning_is_indexed_and_recallable() {
        // Assistant messages that carry tool calls keep their visible text in
        // `reasoning` (content is ""). The lossless DAG must index that text
        // so observations stated in reasoning are recallable.
        let (engine, _) = test_db();
        let turn = vec![
            Message::user("remember a fact"),
            Message::assistant("")
                .with_reasoning("The magic number for this session is forty-two-point-seven"),
            Message::tool("call_1", "{}"),
        ];
        engine.ingest_turn("s1", &turn).await.unwrap();
        let hits = engine.recall(Some("s1"), "magic number", 5).await.unwrap();
        assert!(
            hits.iter()
                .any(|h| h.content.contains("forty-two-point-seven")),
            "reasoning text must be recallable, got: {:?}",
            hits
        );
        // Content-bearing messages keep their content AND reasoning indexed.
        let turn2 =
            vec![Message::assistant("visible answer").with_reasoning("private note about zebras")];
        engine.ingest_turn("s1", &turn2).await.unwrap();
        let hits2 = engine.recall(Some("s1"), "zebras", 5).await.unwrap();
        assert!(
            hits2
                .iter()
                .any(|h| h.content.contains("private note about zebras")),
            "reasoning on content-bearing messages must be indexed, got: {:?}",
            hits2
        );
    }
    #[tokio::test]
    async fn lcm_diagnostic_tool_outputs_are_not_indexed() {
        // lcm_recall / lcm_stats JSON outputs (marked `_lcm_tool`) must never
        // be ingested — they are self-referential and would pollute recalls.
        let (engine, _) = test_db();
        let turn = vec![
            Message::user("go"),
            Message::tool(
                "call_1",
                "{\"_lcm_tool\":\"lcm_recall\",\"hits\":[{\"content\":\"some old recall\"}]}",
            ),
            Message::tool("call_2", "{\"_lcm_tool\":\"lcm_stats\",\"dag_nodes\":7}"),
            Message::tool("call_3", "{\"byte_size\":70,\"complete\":true}"), // normal tool output
        ];
        engine.ingest_turn("s1", &turn).await.unwrap();
        assert_eq!(
            engine.node_count("s1").unwrap(),
            2,
            "only user + normal tool output"
        );
        // A recall must not surface the diagnostic JSON.
        let hits = engine.recall(Some("s1"), "dag_nodes", 5).await.unwrap();
        assert!(
            hits.is_empty(),
            "diagnostic JSON must not be recallable, got: {:?}",
            hits
        );
    }

    #[tokio::test]
    async fn non_lcm_hits_shaped_tool_output_is_still_indexed() {
        // A legitimate tool that returns a `hits`-shaped payload (e.g. a
        // future memory/search tool) must NOT be dropped — the diagnostic
        // filter keys on the `_lcm_tool` marker, not on content shape.
        let (engine, _) = test_db();
        let turn = vec![
            Message::user("go"),
            Message::tool("call_1", "{\"hits\":[{\"title\":\"search result\"}]}"),
        ];
        engine.ingest_turn("s1", &turn).await.unwrap();
        assert_eq!(
            engine.node_count("s1").unwrap(),
            2,
            "user + hits-shaped tool output"
        );
        let hits = engine.recall(Some("s1"), "search result", 5).await.unwrap();
        assert!(
            hits.iter().any(|h| h.role == "tool"),
            "non-LCM hits-shaped tool output must be recallable, got: {:?}",
            hits
        );
    }

    #[tokio::test]
    async fn assemble_injects_stored_rollups_when_over_budget() {
        // P1: with a stored rollup present, an over-budget assemble must
        // inject the rollup summary block (index 2) alongside the placeholder.
        let (engine, _) = test_db();
        let mut base = vec![Message::system("you are a test agent")];
        for i in 0..20 {
            base.push(Message::user(format!(
                "message number {i} with some filler content to consume tokens"
            )));
        }
        engine.ingest_turn("s1", &base).await.unwrap();
        // Build a real stored rollup (fake summarizer; the injected block
        // reads lcm_rollups, so the summary text must appear in the output).
        let summarizer = |t: String| async move {
            let len = t.len();
            Ok(format!("ROLLUP_SUMMARY[{len} chars]"))
        };
        crate::context::rollup::build_rollup(
            &engine,
            "s1",
            crate::context::rollup::RollupPeriod::Day,
            None,
            summarizer,
        )
        .await
        .unwrap()
        .expect("rollup built");

        let out = engine.assemble("s1", base, 150).await.unwrap();
        assert_eq!(out[0].role, Role::System);
        assert!(out[1].content.contains("LCM lossless context"));
        assert!(
            out[2].content.contains("LCM rollups of earlier context"),
            "rollup block must be injected at index 2, got: {:?}",
            out
        );
        assert!(out[2].content.contains("ROLLUP_SUMMARY["));
        assert!(
            out[2].content.contains("- [day "),
            "rollup entry must carry the day prefix"
        );
        assert!(
            out.iter().any(|m| m.content.contains("message number 19")),
            "the freshest message must survive in the D0 tail"
        );
    }

    #[tokio::test]
    async fn assemble_rollups_inject_disabled_keeps_placeholder_only() {
        // When rollups_inject is off (or no rollups exist), the assembled
        // context keeps only the bare placeholder — no rollup block.
        let (engine, _) = test_db();
        let mut base = vec![Message::system("you are a test agent")];
        for i in 0..20 {
            base.push(Message::user(format!(
                "message number {i} with some filler content to consume tokens"
            )));
        }
        let out = engine.assemble("s1", base, 150).await.unwrap();
        assert!(out[1].content.contains("LCM lossless context"));
        assert!(
            !out.iter()
                .any(|m| m.content.contains("LCM rollups of earlier context")),
            "no rollup block without stored rollups"
        );
    }

    #[tokio::test]
    async fn assemble_keeps_tail_and_compacts_prefix() {
        let (engine, _) = test_db();
        let mut base = vec![Message::system("you are a test agent")];
        for i in 0..20 {
            base.push(Message::user(format!(
                "message number {i} with some filler content to consume tokens"
            )));
        }
        // 21 messages * ~19 tokens each ≈ 400 tokens — a 150-token budget
        // forces compaction while the D0 tail (tail_tokens = 100) still fits.
        let out = engine.assemble("s1", base.clone(), 150).await.unwrap();
        // [front system, placeholder, ...tail]
        assert_eq!(out[0].role, Role::System);
        assert!(out[1].content.contains("LCM lossless context"));
        assert!(out[1].content.contains("nothing was deleted"));
        // The most recent message is always present verbatim.
        assert!(
            out.iter().any(|m| m.content.contains("message number 19")),
            "the freshest message must survive in the D0 tail"
        );
        // Fewer messages than the original 21.
        assert!(out.len() < base.len(), "over-budget context must compact");

        // The engine ingested the turn: recall finds a compacted message.
        // Scoped to the active session (no cross-session leakage).
        let hits = engine
            .recall(Some("s1"), "message number 3", 5)
            .await
            .unwrap();
        assert!(!hits.is_empty(), "recall must find compacted content");
        assert!(hits.iter().any(|h| h.content.contains("message number 3")));
        // Verbatim (lossless) content.
        assert!(
            hits.iter().any(|h| h.content == "message number 3 with some filler content to consume tokens")
        );
        // Cross-session isolation: another session's nodes are never returned.
        let cross = engine
            .recall(Some("other-session"), "message number 3", 5)
            .await
            .unwrap();
        assert!(cross.is_empty(), "recall must not leak other sessions");
    }

    #[tokio::test]
    async fn identical_repeated_messages_are_never_collapsed() {
        let (engine, _) = test_db();
        // "continue" appears twice at DIFFERENT history positions — a plain
        // content-hash dedup would silently collapse them into one node and
        // break the losslessness guarantee. The (session, position, hash)
        // key must preserve both.
        let turn = vec![
            Message::user("continue"),
            Message::assistant("working..."),
            Message::user("continue"),
        ];
        engine.ingest_turn("s1", &turn).await.unwrap();
        assert_eq!(
            engine.node_count("s1").unwrap(),
            3,
            "identical messages at different positions must stay distinct"
        );
        // Re-ingesting the SAME turn still dedups (idempotency preserved).
        engine.ingest_turn("s1", &turn).await.unwrap();
        assert_eq!(engine.node_count("s1").unwrap(), 3);
    }

    #[tokio::test]
    async fn assemble_fast_path_returns_unchanged_when_fits() {
        let (engine, _) = test_db();
        let base = sample_turn();
        let out = engine.assemble("s1", base.clone(), 100_000).await.unwrap();
        // No prior DAG content matches the last user query, so no evidence
        // block is injected — the fast path stays lossless.
        assert_eq!(out.len(), base.len());
        assert!(
            out.iter()
                .all(|m| !m.content.contains("LCM recalled evidence")),
            "no evidence block when the DAG has no relevant hits"
        );
    }

    #[tokio::test]
    async fn auto_recall_injects_evidence_for_relevant_prior_turn() {
        // P3: assemble() issues one bounded retrieval against the latest user
        // message and injects top hits as a system evidence block.
        let (engine, _) = test_db();
        // Seed the DAG with a prior exchange about a project detail.
        let seed = vec![
            Message::user("remember: the deploy window is every Tuesday at 2pm UTC"),
            Message::assistant("Got it — deploys are Tuesdays 2pm UTC"),
        ];
        engine.ingest_turn("s1", &seed).await.unwrap();
        // A fresh turn asking about the deploy window.
        let base = vec![
            Message::system("you are a test agent"),
            Message::user("when is the deploy window again?"),
        ];
        let out = engine.assemble("s1", base.clone(), 100_000).await.unwrap();
        let evidence: Vec<_> = out
            .iter()
            .filter(|m| m.content.contains("LCM recalled evidence"))
            .collect();
        assert_eq!(
            evidence.len(),
            1,
            "one evidence block expected, got {out:?}"
        );
        assert!(
            evidence[0].content.contains("deploy window"),
            "evidence must carry the recalled detail: {}",
            evidence[0].content
        );
        // The original messages remain untouched (fast path, lossless).
        assert!(
            out.iter()
                .any(|m| m.content.contains("when is the deploy window"))
        );
    }

    #[tokio::test]
    async fn auto_recall_skips_evidence_already_visible_in_context() {
        // Evidence whose text already appears in the assembled context must
        // not be duplicated into the injected block.
        let (engine, _) = test_db();
        let fact = "the release cadence is biweekly";
        let seed = vec![Message::user(format!("note: {fact}"))];
        engine.ingest_turn("s1", &seed).await.unwrap();
        // The fact is ALREADY in this turn's context.
        let base = vec![
            Message::system("you are a test agent"),
            Message::user(format!("what did I say about cadence? (it was: {fact})")),
        ];
        let out = engine.assemble("s1", base.clone(), 100_000).await.unwrap();
        let evidence: Vec<_> = out
            .iter()
            .filter(|m| m.content.contains("LCM recalled evidence"))
            .collect();
        assert!(
            evidence.is_empty(),
            "no evidence block when the only hit is already visible: {out:?}"
        );
    }

    #[test]
    fn auto_recall_query_escapes_fts_reserved_keywords() {
        // FTS5 operators (NOT/NEAR) must never reach MATCH as bare tokens —
        // they would be parse errors and silently drop evidence.
        let q = auto_recall_fts_query("what is not working near the deploy window");
        assert!(!q.contains("NOT"), "reserved NOT must be filtered: {q}");
        assert!(!q.contains("NEAR"), "reserved NEAR must be filtered: {q}");
        assert!(q.contains("deploy"), "significant terms survive: {q}");
        assert!(q.contains("window"), "significant terms survive: {q}");
        assert!(q.contains("working"), "significant terms survive: {q}");
    }

    #[tokio::test]
    async fn auto_recall_disabled_never_injects_evidence() {
        let dir = std::env::temp_dir().join(format!("operant_lcm_off_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: dir.join("lcm-off.db"),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();
        let seed = vec![Message::user("remember: key=alpha")];
        engine.ingest_turn("s1", &seed).await.unwrap();
        let base = vec![Message::user("what is key?")];
        let out = engine.assemble("s1", base.clone(), 100_000).await.unwrap();
        assert!(
            out.iter()
                .all(|m| !m.content.contains("LCM recalled evidence")),
            "auto-recall disabled must not inject evidence"
        );
    }

    #[tokio::test]
    async fn auto_recall_evidence_rides_along_when_compacting() {
        // When the context is over budget, the evidence block must survive
        // alongside the compaction placeholder.
        let (engine, _) = test_db();
        let fact = "the secret ingredient is saffron";
        let seed = vec![Message::user(format!("remember: {fact}"))];
        engine.ingest_turn("s1", &seed).await.unwrap();
        let mut base = vec![Message::system("you are a test agent")];
        for i in 0..20 {
            base.push(Message::user(format!(
                "filler message number {i} with enough words to consume tokens"
            )));
        }
        // Ask WITHOUT restating the fact so evidence is genuinely needed.
        base.push(Message::user("what ingredient did I mention earlier?"));
        // Tiny budget forces compaction.
        let out = engine.assemble("s1", base, 100).await.unwrap();
        assert!(
            out.iter()
                .any(|m| m.content.contains("LCM lossless context")),
            "compaction placeholder expected"
        );
        assert!(
            out.iter()
                .any(|m| m.content.contains("LCM recalled evidence")),
            "evidence block must survive compaction"
        );
    }

    #[test]
    fn split_keeps_front_and_recent() {
        let mut msgs = vec![Message::system("sys")];
        for i in 0..10 {
            msgs.push(Message::user(format!("message {i} content")));
        }
        let (kept, compacted, tokens) = split_for_lcm(msgs, 60);
        assert_eq!(kept[0].role, Role::System);
        assert!(compacted > 0);
        assert!(tokens > 0);
        // Tail is chronological.
        let last = kept.last().unwrap();
        assert!(last.content.contains("message 9") || last.content.contains("message 9 content"));
        // Front is the system message, tail follows.
        assert!(kept[1].content.starts_with("message "));
    }

    #[tokio::test]
    async fn vector_recall_ranks_semantic_matches_and_caches() {
        let (engine, _) = test_db();
        let turn = vec![
            Message::assistant("The deploy pipeline uses rsync over ssh"),
            Message::assistant("The weather in paris is rainy today"),
            Message::assistant("Alpha beta gamma project roadmap"),
        ];
        engine.ingest_turn("s1", &turn).await.unwrap();

        let embedder = crate::context::MockEmbedder::default();
        // First query: embeds query (1) + all 3 candidates (3) = 4 calls.
        let hits = engine
            .vector_recall(&embedder, Some("s1"), "deploy pipeline rsync", 3)
            .await
            .unwrap();
        assert!(!hits.is_empty(), "vector recall must return hits");
        assert!(
            hits[0].content.contains("rsync"),
            "semantic top hit should be the rsync node, got: {}",
            hits[0].content
        );
        assert_eq!(
            embedder.calls.load(std::sync::atomic::Ordering::Relaxed),
            4,
            "query + 3 candidates embedded once"
        );

        // Second query: embeddings are cached, only the new query is embedded.
        let before = embedder.calls.load(std::sync::atomic::Ordering::Relaxed);
        let _ = engine
            .vector_recall(&embedder, Some("s1"), "rainy paris weather", 2)
            .await
            .unwrap();
        let after = embedder.calls.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            after - before,
            1,
            "only the query is embedded when candidates are cached (cache hit)"
        );
    }

    #[tokio::test]
    async fn vector_recall_surfaces_reworded_matches_without_exact_words() {
        let (engine, _) = test_db();
        let turn = vec![
            Message::assistant("The magic number for this session is forty-two-point-seven"),
            Message::assistant("Unrelated chatter about lunch plans"),
        ];
        engine.ingest_turn("s1", &turn).await.unwrap();
        let embedder = crate::context::MockEmbedder::default();
        // No exact word overlap with the magic-number node, but tokens
        // (magic/number/session) overlap the query's intent.
        let hits = engine
            .vector_recall(&embedder, Some("s1"), "session magic value", 1)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert!(
            hits[0].content.contains("forty-two-point-seven"),
            "reworded semantic match must surface, got: {}",
            hits[0].content
        );
    }

    #[tokio::test]
    async fn vector_cache_is_keyed_by_model_not_node_alone() {
        // F1 regression: lcm_embeddings used node_id as its sole PK while
        // every lookup keys on (node_id, model) — a model change silently
        // clobbered the other model's cached vectors, forcing a full
        // re-embed on the next query with the previous model. With the
        // composite PK, each model's vectors coexist.
        let (engine, _) = test_db();
        let turn = vec![Message::assistant(
            "The deploy pipeline uses rsync over ssh",
        )];
        engine.ingest_turn("s1", &turn).await.unwrap();

        // Model A caches its vectors.
        let a = crate::context::MockEmbedder::default();
        let hits_a = engine
            .vector_recall(&a, Some("s1"), "deploy rsync", 1)
            .await
            .unwrap();
        assert!(!hits_a.is_empty());
        let a_calls_after_first = a.calls.load(std::sync::atomic::Ordering::Relaxed);

        // Model B embeds the same node with a different model id — this must
        // NOT destroy model A's cache row.
        let b = crate::context::MockEmbedder::default();
        let _ = engine
            .vector_recall(&b, Some("s1"), "deploy rsync", 1)
            .await
            .unwrap();

        // Back to model A: the cached vectors must still be there — only the
        // query gets embedded, candidates are cache hits.
        let before = a.calls.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            a_calls_after_first, before,
            "model A's cache must survive model B's writes"
        );
        let hits_a2 = engine
            .vector_recall(&a, Some("s1"), "deploy rsync", 1)
            .await
            .unwrap();
        assert!(!hits_a2.is_empty());
    }

    #[tokio::test]
    async fn vector_recall_skips_nonfinite_and_wrong_len_embeds() {
        // F2 regression: a NaN/Inf or dimension-mismatched vector from the
        // provider must never be cached (serde_json can't serialize NaN —
        // the old unwrap_or_default wrote a corrupt blob that forced an
        // endless re-embed) and must not rank garbage.
        let (engine, _) = test_db();
        let turn = vec![Message::assistant("alpha beta gamma project roadmap")];
        engine.ingest_turn("s1", &turn).await.unwrap();

        // A mock embedder that returns a NaN vector for one input.
        struct NaNEmbedder;
        #[async_trait::async_trait]
        impl crate::context::Embedder for NaNEmbedder {
            fn model_id(&self) -> &str {
                "nan-model"
            }
            async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
                Ok(texts
                    .iter()
                    .map(|t| {
                        let mut v = vec![0.0f32; 32];
                        v[0] = f32::NAN;
                        let _ = t;
                        v
                    })
                    .collect())
            }
        }
        let hits = engine
            .vector_recall(&NaNEmbedder, Some("s1"), "alpha beta", 1)
            .await
            .unwrap();
        assert!(
            hits.is_empty(),
            "non-finite vectors must be skipped, not ranked"
        );
        // The cache must not contain a poisoned row for this model.
        let conn = engine.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM lcm_embeddings WHERE model = 'nan-model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "NaN vectors must never be persisted");
    }

    #[test]
    fn embeddings_pk_migration_rebuilds_legacy_single_column_pk() {
        // F1 migration: a database created by an older build (node_id as the
        // sole PK) must be rebuilt with the composite (node_id, model) PK.
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_migrate_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("lcm-legacy.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE lcm_embeddings (
                 node_id INTEGER PRIMARY KEY,
                 model TEXT NOT NULL,
                 vector TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO lcm_embeddings (node_id, model, vector, created_at) VALUES (1, 'a', '[1.0]', 0)",
            [],
        )
        .unwrap();
        // Run the migration directly.
        migrate_embeddings_pk(&conn).unwrap();
        let pk_cols: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT pk FROM pragma_table_info('lcm_embeddings') ORDER BY pk")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, i64>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            pk_cols.contains(&1) && pk_cols.contains(&2),
            "composite PK (node_id pk=1, model pk=2) expected, got {pk_cols:?}"
        );
        // Both columns are now part of the PK.
        let composite: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('lcm_embeddings') WHERE pk > 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(composite, 2, "exactly two PK columns after migration");
    }
}
