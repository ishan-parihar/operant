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
    /// P3 adaptive auto-recall: when `assemble()` runs, issue one bounded
    /// retrieval against the latest user message and inject the top hits as
    /// a system "pre-answer evidence" block (hermes `adaptive_retrieval.py`
    /// parity). Default on.
    pub auto_recall: bool,
    /// Max evidence nodes injected per assemble.
    pub auto_recall_limit: usize,
    /// Hard cap on the injected evidence block, in characters.
    pub auto_recall_max_chars: usize,
}

impl Default for LcmConfig {
    fn default() -> Self {
        Self {
            db_path: crate::platform::operant_home().join("lcm.db"),
            tail_tokens: 12_000,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
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
}

impl std::fmt::Debug for LcmContextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcmContextEngine")
            .field("db_path", &self.db_path)
            .field("tail_tokens", &self.tail_tokens)
            .field("auto_recall", &self.auto_recall)
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
            auto_recall: config.auto_recall,
            auto_recall_limit: config.auto_recall_limit.clamp(1, 10),
            auto_recall_max_chars: config.auto_recall_max_chars.max(256),
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
        } // 4. D0 fresh tail kept verbatim; older messages compacted into a
        //    placeholder (P1 fills it with real day/week/month LLM rollups).
        //    Reserve tail budget for the evidence block so the assembled
        //    context never exceeds the intended budget even when compacting.
        let evidence_tokens = evidence
            .as_ref()
            .map(crate::context_management::estimate_message_tokens)
            .unwrap_or(0);
        let tail_budget = self
            .tail_tokens
            .min(budget_tokens)
            .saturating_sub(evidence_tokens);
        let (mut kept, compacted_count, compacted_tokens) =
            split_for_lcm(with_evidence, tail_budget);

        let placeholder = Message::system(format!(
            "[LCM lossless context: {} earlier message(s) (~{} tokens) are preserved verbatim in the DAG at {}. Use the lcm_recall tool to retrieve any of them — nothing was deleted.]",
            compacted_count,
            compacted_tokens,
            self.db_path.display()
        ));
        kept.insert(1, placeholder);
        // Pre-answer evidence still rides along after the placeholder.
        if let Some(block) = evidence {
            kept.insert(2, block);
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
}
