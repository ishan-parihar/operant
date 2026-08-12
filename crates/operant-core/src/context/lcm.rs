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
use crate::error::{Error, Result};

/// Configuration for the lossless context engine.
#[derive(Debug, Clone)]
pub struct LcmConfig {
    /// SQLite database path for the DAG (default `~/.operant/lcm.db`).
    pub db_path: PathBuf,
    /// Fresh-tail (D0) token budget kept verbatim by `assemble()`.
    pub tail_tokens: usize,
}

impl Default for LcmConfig {
    fn default() -> Self {
        Self {
            db_path: crate::platform::operant_home().join("lcm.db"),
            tail_tokens: 12_000,
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
}

impl std::fmt::Debug for LcmContextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcmContextEngine")
            .field("db_path", &self.db_path)
            .field("tail_tokens", &self.tail_tokens)
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
                 USING fts5(content, session_id UNINDEXED);",
        )
        .map_err(|e| Error::Agent(format!("lcm: schema bootstrap failed: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            db_path: config.db_path,
            tail_tokens: config.tail_tokens.max(1),
        })
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
}

#[async_trait::async_trait]
impl ContextEngine for LcmContextEngine {
    fn name(&self) -> &str {
        "lcm"
    }

    async fn ingest_turn(&self, session_id: &str, turn: &[Message]) -> Result<()> {
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

        // 2. Fast path: everything fits — return unchanged (lossless).
        if crate::context_management::estimate_total_tokens(&base) <= budget_tokens {
            return Ok(base);
        }

        // 3. D0 fresh tail kept verbatim; older messages compacted into a
        //    placeholder (P1 fills it with real day/week/month LLM rollups).
        let tail_budget = self.tail_tokens.min(budget_tokens);
        let (mut kept, compacted_count, compacted_tokens) = split_for_lcm(base, tail_budget);

        let placeholder = Message::system(format!(
            "[LCM lossless context: {} earlier message(s) (~{} tokens) are preserved verbatim in the DAG at {}. Use the lcm_recall tool to retrieve any of them — nothing was deleted.]",
            compacted_count,
            compacted_tokens,
            self.db_path.display()
        ));
        kept.insert(1, placeholder);
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // FTS5 phrase: quote + escape inner quotes. Lower bm25 = better.
        let fts_query = format!("\"{}\"", query.trim().replace('"', "\"\""));
        // `session_id` filters to one session so other sessions' history never
        // leaks into this context; NULL recalls across all sessions.
        let sql = "SELECT n.id, n.role, n.content, n.created_at, bm25(nodes_fts)
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
                Ok(RecallHit {
                    node_id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    score: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("lcm: recall query failed: {e}")))?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(|e| Error::Agent(format!("lcm: recall row failed: {e}")))?);
        }
        Ok(hits)
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
        assert_eq!(out.len(), base.len());
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
}
