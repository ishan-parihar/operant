//! Pluggable context engines (hermes-lcm parity).
//!
//! hermes-agent's LCM plugin is a `ContextEngine`: it installs a single
//! deterministic `pre_llm_call` hook and returns the FINAL message list to
//! send. The same shape lives here — an engine that ingests completed
//! turns into a lossless store and assembles the per-call message list
//! instead of the default lossy `evict_to_budget`.
//!
//! Config selection (`agent.context_engine`):
//!   - `"compact"` (default): existing deterministic decay + eviction.
//!   - `"lcm"`: lossless DAG + fresh-tail assembly (see [`lcm`]).
//!
//! See `docs/HERMES_LCM_INTEGRATION.md` for the full design (rollups,
//! adaptive recall, assertions) and the P0–P4 roadmap.

pub mod adaptive;
pub mod lcm;
pub mod rollup;

use crate::client::Message;
use crate::error::Result;

pub use adaptive::{AdaptiveRetrievalRegistry, RetrievalRound};
pub use lcm::{LcmConfig, LcmContextEngine};

/// One stored durable assertion (hermes `assertion_store.py` row parity).
#[derive(Debug, Clone)]
pub struct AssertionRecord {
    /// Row id in `lcm_assertions`.
    pub id: i64,
    /// Subject key (e.g. `project:hermes`, `user`, `assistant:self`).
    pub subject: String,
    /// Predicate key (e.g. `prefers`, `uses`, `deadline`).
    pub predicate: String,
    /// Object value — the fact itself.
    pub object_value: String,
    /// Speaker role recorded with the fact.
    pub speaker_role: String,
    /// DAG node the fact was sourced from, when known.
    pub source_node_id: Option<i64>,
    /// Unix millis when the assertion was stored.
    pub created_at: i64,
}

/// A single recall hit from a context engine's store.
#[derive(Debug, Clone)]
pub struct RecallHit {
    /// Node id in the engine's store (the DAG).
    pub node_id: i64,
    /// Message role (`system`/`user`/`assistant`/`tool`).
    pub role: String,
    /// Verbatim content — losslessness guarantee.
    pub content: String,
    /// Unix millis when the node was created.
    pub created_at: i64,
    /// Retrieval rank (bm25: lower = better match).
    pub score: f64,
}

/// A pluggable context engine (hermes `ContextEngine`/`pre_llm_call`
/// parity). When attached to [`crate::agent::OperantAgent`],
/// `build_messages()` calls [`ContextEngine::assemble`] in place of the
/// lossy eviction step.
#[async_trait::async_trait]
pub trait ContextEngine: Send + Sync {
    /// Engine name for logging/diagnostics ("compact", "lcm", ...).
    fn name(&self) -> &str;

    /// Ingest a completed turn into the engine's store. Must be idempotent
    /// (safe to call every iteration with the full conversation).
    async fn ingest_turn(&self, session_id: &str, turn: &[Message]) -> Result<()>;

    /// Assemble the FINAL message list for the next LLM call within
    /// `budget_tokens`. Called by `build_messages()` instead of
    /// `context_management::evict_to_budget` when an engine is attached.
    async fn assemble(
        &self,
        session_id: &str,
        base: Vec<Message>,
        budget_tokens: usize,
    ) -> Result<Vec<Message>>;

    /// On-demand recall over the engine's store (hybrid FTS, ranked).
    /// Returns verbatim nodes so nothing is ever lost to compaction.
    ///
    /// `session_id`: scope recall to one session (other sessions' history
    /// never leaks into this context); `None` recalls across all sessions.
    async fn recall(
        &self,
        session_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallHit>>;
}
