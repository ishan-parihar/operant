#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! Memory subsystem: backends, embeddings, consolidation, retrieval.
//!
//! ## Reserved Key Prefixes
//!
//! The following key prefixes are reserved for the auto-save system. Any memory
//! stored under these keys will be **excluded from context assembly** by all
//! three context-building paths (`build_context`, `DefaultMemoryLoader`, and
//! `should_skip_memory_context_entry`). Do not use these prefixes for semantic
//! memories that should surface in agent context.
//!
//! | Prefix | Purpose | Detection function |
//! |---|---|---|
//! | `assistant_resp` / `assistant_resp_*` | Model-authored assistant summaries (untrusted context) | [`is_assistant_autosave_key`] |
//! | `user_msg` / `user_msg_*` | Raw per-turn user messages (consolidation queue) | [`is_user_autosave_key`] |
//!
//! Channel-scoped variants (e.g. `telegram_user_msg_*`, `discord_*`) are
//! **not** filtered — they use different prefixes and are handled separately.

/// Opening delimiter for recalled memory injected into provider context.
pub const MEMORY_CONTEXT_OPEN: &str = "[Memory context]";
/// Closing delimiter for recalled memory injected into provider context.
pub const MEMORY_CONTEXT_CLOSE: &str = "[/Memory context]";

/// agentmemory REST backend (hybrid semantic memory via the local server).
pub mod agentmemory;
/// Audit-trail decorator backend (logs all operations to SQLite).
#[allow(unused_imports)]
/// Memory backend kind / profile metadata and classification.
pub mod backend;
/// Line-based markdown chunking for long-document ingestion.
pub mod chunker;
/// Conflict detection and resolution between overlapping memories.
pub mod conflict;
/// LLM-driven consolidation of turns into history entries + core facts.
pub mod consolidation;
/// Memory decay / forgetfulness scheduling.
pub mod decay;
/// Embedding providers and factory.
pub mod embeddings;
/// Typed error type and context helpers.
pub mod error;
/// Heuristic importance scoring for memory entries.
pub mod importance;
/// Knowledge-graph node/edge types.
pub mod knowledge_graph;
#[cfg(feature = "memory-postgres")]
/// Postgres-backed knowledge graph (feature-gated).
pub mod knowledge_graph_pg;
/// Lucid-memory bridge backend.
pub mod lucid;
/// Markdown-file backend.
pub mod markdown;
/// Namespace-scoped wrapper around another backend.
pub mod namespaced;
/// No-op backend (memory disabled).
pub mod none;
/// Policy enforcer for memory operations.
pub mod policy;
#[cfg(feature = "memory-postgres")]
/// PostgreSQL backend (feature-gated).
pub mod postgres;
/// Qdrant vector-database backend.
pub mod qdrant;
/// Multi-stage retrieval pipeline.
pub mod retrieval;
/// Snapshot export / hydration (`MEMORY_SNAPSHOT.md`).
pub mod snapshot;
/// SQLite backend with vector search.
pub mod sqlite;
/// Core `Memory` trait and shared types.
pub mod traits;
/// Vector math: cosine similarity, serialization, hybrid merge.
pub mod vector;

#[allow(unused_imports)]
pub use agentmemory::AgentMemory;
#[allow(unused_imports)]
pub use backend::{
    MemoryBackendKind, MemoryBackendProfile, classify_memory_backend, default_memory_backend_key,
    memory_backend_profile, selectable_memory_backends,
};
pub use error::{Error, Result};
pub use lucid::LucidMemory;
pub use markdown::MarkdownMemory;
pub use namespaced::NamespacedMemory;
pub use none::NoneMemory;
#[allow(unused_imports)]
pub use policy::PolicyEnforcer;
#[cfg(feature = "memory-postgres")]
#[allow(unused_imports)]
pub use postgres::PostgresMemory;
pub use qdrant::QdrantMemory;
#[allow(unused_imports)]
pub use retrieval::{RetrievalConfig, RetrievalPipeline};
pub use sqlite::SqliteMemory;
pub use traits::Memory;
#[allow(unused_imports)]
pub use traits::{
    ExportFilter, MemoryCategory, MemoryEntry, MemoryResult, ProceduralMessage,
    is_recent_recall_query, normalize_recent_recall_query,
};

#[cfg(test)]
mod tests {

    // Regression guard for issue #3083: when default_provider is "gemini"
    // (api_key = gemini key) but embedding_provider is "cohere", the
    // embedding provider's own env var (COHERE_API_KEY) must take precedence
    // over the caller-supplied key (which belongs to the default provider).
    //
    // Uses COHERE_API_KEY to avoid accidental collision with OPENAI_API_KEY
    // that may be set in the developer environment.
}
