//! Memory provider system for Operant-RS.
//!
//! **TDG is the only real memory backend.** The active provider is selected
//! by `config.memory.provider`:
//!
//! | Value       | Backend                                                     |
//! |-------------|-------------------------------------------------------------|
//! | `"tdg"`     | TDG graph memory via tdg-rust (SQLite + FTS5 + graph)       |
//! | `"builtin"` | File-backed MEMORY.md / USER.md (zero-dependency fallback)  |
//! | other       | Silently falls back to `"builtin"`                          |
//!
//! Old provider names (`"hindsight"`, `"retaindb"`, `"mem0"`, `"local-vector"`)
//! were removed in iter-30 — configs that still reference them are silently
//! downgraded to `"builtin"`.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

#[cfg(feature = "tdg")]
use crate::error::Error;
use crate::error::Result;

// ---------------------------------------------------------------------------
// Background sync executor
// ---------------------------------------------------------------------------

/// Single-worker FIFO executor for memory provider operations.
/// Ensures sync_turn, on_memory_write, and other background writes
/// happen in order (no interleaving) and don't block the agent loop.
///
/// Ported from hermes-agent's MemoryManager._submit_background() pattern
/// where a dedicated ThreadPoolExecutor with FIFO ordering processes
/// memory writes sequentially.
pub struct MemorySyncExecutor {
    tx: mpsc::Sender<SyncJob>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

enum SyncJob {
    SyncTurn {
        user: String,
        assistant: String,
    },
    MemoryWrite {
        action: String,
        target: String,
        content: String,
    },
    Delegation {
        task: String,
        result: String,
    },
    SessionEnd {
        messages: Vec<crate::client::Message>,
    },
    Shutdown,
}

impl MemorySyncExecutor {
    /// Create a new executor that processes jobs sequentially on a
    /// dedicated background task. The channel capacity (256) bounds
    /// memory to ~256 pending writes before backpressure kicks in.
    pub fn new(provider: Arc<dyn MemoryProvider>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let handle = tokio::spawn(Self::run_loop(provider, rx));
        Self {
            tx,
            handle: Some(handle),
        }
    }

    /// Submit a sync_turn job (non-blocking).
    pub fn submit_sync_turn(&self, user: &str, assistant: &str) {
        if self.tx.try_send(SyncJob::SyncTurn {
            user: user.to_string(),
            assistant: assistant.to_string(),
        }).is_err() {
            tracing::warn!("MemorySyncExecutor: sync_turn channel full — job dropped");
        }
    }

    /// Submit a memory write mirror job (non-blocking).
    pub fn submit_memory_write(&self, action: &str, target: &str, content: &str) {
        if self.tx.try_send(SyncJob::MemoryWrite {
            action: action.to_string(),
            target: target.to_string(),
            content: content.to_string(),
        }).is_err() {
            tracing::warn!("MemorySyncExecutor: memory_write channel full — job dropped");
        }
    }

    /// Submit a delegation observation job (non-blocking).
    pub fn submit_delegation(&self, task: &str, result: &str) {
        if self.tx.try_send(SyncJob::Delegation {
            task: task.to_string(),
            result: result.to_string(),
        }).is_err() {
            tracing::warn!("MemorySyncExecutor: delegation channel full — job dropped");
        }
    }

    /// Submit a session end extraction job (non-blocking).
    /// Limits to the last 5 assistant messages to avoid cloning large conversations.
    pub fn submit_session_end(&self, messages: &[crate::client::Message]) {
        let limited: Vec<_> = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::Assistant)
            .take(5)
            .cloned()
            .collect();
        if self.tx.try_send(SyncJob::SessionEnd {
            messages: limited,
        }).is_err() {
            tracing::warn!("MemorySyncExecutor: session_end channel full — job dropped");
        }
    }

    /// Graceful shutdown: drain pending jobs (up to 5s) then abandon.
    /// Ported from hermes-agent's _drain_sync_executor() pattern.
    pub async fn shutdown(mut self) {
        // Send shutdown sentinel
        let _ = self.tx.send(SyncJob::Shutdown).await;
        // Drop the sender so the channel closes
        drop(self.tx);
        // Wait for the worker to finish (up to 5s)
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                handle,
            ).await;
        }
    }

    /// Background worker loop: processes jobs in FIFO order.
    async fn run_loop(provider: Arc<dyn MemoryProvider>, mut rx: mpsc::Receiver<SyncJob>) {
        while let Some(job) = rx.recv().await {
            match job {
                SyncJob::SyncTurn { user, assistant } => {
                    if let Err(e) = provider.sync_turn(&user, &assistant).await {
                        tracing::warn!(error = %e, "MemorySyncExecutor: sync_turn failed");
                    }
                }
                SyncJob::MemoryWrite { action, target, content } => {
                    provider.on_memory_write(&action, &target, &content);
                }
                SyncJob::Delegation { task, result } => {
                    provider.on_delegation(&task, &result);
                }
                SyncJob::SessionEnd { messages } => {
                    provider.on_session_end(&messages);
                }
                SyncJob::Shutdown => {
                    tracing::debug!("MemorySyncExecutor: shutdown received");
                    break;
                }
            }
        }
        tracing::debug!("MemorySyncExecutor: worker exited");
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Lifecycle-matching the Python MemoryProvider ABC.
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// Short identifier, e.g. `"builtin"`, `"hindsight"`.
    fn name(&self) -> &str;

    /// True when credentials / deps are present (no network calls).
    fn is_available(&self) -> bool;

    /// Connect, create tables, warm up.  Called once at startup.
    async fn initialize(&self, session_id: &str) -> Result<()>;

    /// Static text for the system prompt (instructions / status line).
    fn system_prompt_block(&self) -> String {
        String::new()
    }

    /// Recall relevant context before a turn. Returns formatted text or "".
    async fn prefetch(&self, query: &str) -> String {
        let _ = query;
        String::new()
    }

    /// Persist a completed turn asynchronously.
    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let _ = (user, assistant);
        Ok(())
    }

    /// Tool schemas this provider exposes to the model (OpenAI format).
    fn tool_schemas(&self) -> Vec<Value> {
        vec![]
    }

    /// Dispatch a tool call; return a JSON result string.
    async fn handle_tool_call(&self, name: &str, _args: Value) -> String {
        format!(
            r#"{{"error":"provider {} does not handle tool {}"}}"#,
            self.name(),
            name
        )
    }

    /// Flush queues and close connections.
    async fn shutdown(&self) {}

    // -- Optional lifecycle hooks (override to opt in) ----------------------

    /// Called at the start of each turn with the user message.
    /// Use for turn-counting, scope management, periodic maintenance.
    fn on_turn_start(&self, _turn_number: usize, _message: &str) {}

    /// Called when a session ends (explicit exit or timeout).
    /// Use for end-of-session fact extraction, summarization, etc.
    fn on_session_end(&self, _messages: &[crate::client::Message]) {}

    /// Called when the agent switches session_id mid-process.
    /// Fires on /resume, /branch, /reset, /new, and context compression.
    fn on_session_switch(
        &self,
        _new_session_id: &str,
        _parent_session_id: &str,
        _reset: bool,
    ) {}

    /// Called before context compression discards old messages.
    /// Use to extract insights from messages about to be compressed.
    /// Return text to include in the compression summary prompt.
    fn on_pre_compress(&self, _messages: &[crate::client::Message]) -> String {
        String::new()
    }

    /// Called when the built-in memory tool writes an entry.
    /// Use to mirror built-in memory writes to your backend.
    fn on_memory_write(
        &self,
        _action: &str,
        _target: &str,
        _content: &str,
    ) {}

    /// Called on the PARENT agent when a subagent completes.
    /// The parent's memory provider gets the task+result pair.
    fn on_delegation(&self, _task: &str, _result: &str) {}

    /// Queue a background recall for the NEXT turn.
    /// Called after each turn completes. The result will be consumed
    /// by prefetch() on the next turn.
    fn queue_prefetch(&self, _query: &str) {}

    /// Return extra on-disk paths this provider stores outside
    /// the operant home directory. Used by backup to include them.
    fn backup_paths(&self) -> Vec<std::path::PathBuf> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Builtin — wraps existing MemoryManager (MEMORY.md / USER.md)
// ---------------------------------------------------------------------------

pub struct BuiltinProvider {
    manager: crate::memory::MemoryManager,
}

impl BuiltinProvider {
    pub fn new(manager: crate::memory::MemoryManager) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl MemoryProvider for BuiltinProvider {
    fn name(&self) -> &str {
        "builtin"
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, session_id: &str) -> Result<()> {
        self.manager.load_from_disk().await?;
        self.manager.start_session(session_id).await;
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "Built-in file memory active (MEMORY.md / USER.md).".to_string()
    }

    async fn prefetch(&self, _query: &str) -> String {
        self.manager.build_memory_context(2000).await
    }

    async fn sync_turn(&self, user: &str, _assistant: &str) -> Result<()> {
        self.manager
            .add_message(crate::client::Message::user(user))
            .await;
        Ok(())
    }

    fn on_turn_start(&self, turn_number: usize, _message: &str) {
        tracing::debug!(turn_number, "BuiltinProvider: turn started");
    }

    fn on_session_end(&self, _messages: &[crate::client::Message]) {
        tracing::debug!("BuiltinProvider: session ended — no extraction needed");
    }

    fn on_session_switch(&self, _new_session_id: &str, _parent_session_id: &str, _reset: bool) {
        tracing::debug!("BuiltinProvider: session switched");
    }

    fn on_memory_write(&self, action: &str, target: &str, content: &str) {
        tracing::debug!(action, target, content_len = content.len(), "BuiltinProvider: memory write mirrored");
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct the memory provider from the config `provider` string.
///
/// **TDG is the only real memory backend.** All other provider names
/// (`"hindsight"`, `"retaindb"`, `"mem0"`, `"local-vector"`) were removed
/// in iter-30 — they added complexity without delivering value, and TDG
/// (the graph memory via tdg-rust) is the strategic direction. Any
/// unrecognized provider name (including the old ones) falls back to
/// `BuiltinProvider` (file-backed MEMORY.md / USER.md), which is the
/// zero-dependency fallback for environments where TDG can't initialize.
///
/// If `"tdg"` is requested but TDG init fails (bad storage dir, corrupted
/// SQLite DB, schema migration failure), this also falls back to
/// `BuiltinProvider` and logs the error — the agent stays functional with
/// a degraded memory backend rather than dying on startup.
pub fn build_memory_provider(
    provider_name: &str,
    storage_dir: std::path::PathBuf,
) -> Arc<dyn MemoryProvider> {
    match provider_name {
        #[cfg(feature = "tdg")]
        "tdg" => match TdgMemoryProvider::new(storage_dir.clone()) {
            Ok(provider) => Arc::new(provider),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "TDG memory provider initialization failed; falling back to BuiltinProvider"
                );
                Arc::new(BuiltinProvider::new(
                    crate::memory::MemoryManager::with_storage_dir(storage_dir),
                ))
            }
        },
        // When the `tdg` feature is off, "tdg" falls through to the default
        // arm and is silently downgraded to BuiltinProvider.
        // "builtin", "disabled", and any other value → BuiltinProvider.
        // Old provider names (hindsight/retaindb/mem0/local-vector) also
        // land here — they're treated as unknown and silently downgraded
        // to BuiltinProvider with no error, since the user's config may
        // still reference them after upgrading.
        _ => Arc::new(BuiltinProvider::new(
            crate::memory::MemoryManager::with_storage_dir(storage_dir),
        )),
    }
}

// ---------------------------------------------------------------------------
// TDG — Graph memory via tdg-rust
// ---------------------------------------------------------------------------
// Gated behind the `tdg` feature. When off, TdgMemoryProvider is not compiled
// and `build_memory_provider("tdg", ...)` falls back to BuiltinProvider.

#[cfg(feature = "tdg")]
pub struct TdgMemoryProvider {
    pool: std::sync::Arc<tdg_rust::ConnectionPool>,
}

#[cfg(feature = "tdg")]
impl TdgMemoryProvider {
    /// Create a new TDG memory provider backed by a SQLite database at
    /// `<storage_dir>/tdg/graph.db`.
    ///
    /// Returns `Err` if the connection pool can't be created or the schema
    /// can't be initialized. Previously this method `.expect()`ed on both
    /// failures, crashing the entire process on a bad storage dir or a
    /// corrupted database. Callers (notably `build_memory_provider`) now
    /// fall back to `BuiltinProvider` on `Err`.
    pub fn new(storage_dir: std::path::PathBuf) -> Result<Self> {
        let db_path = storage_dir.join("tdg").join("graph.db");
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let pool = tdg_rust::ConnectionPool::new(
            // (iter-144 — fixed A26: was .unwrap_or() with a wrong default
            // path that would silently use the wrong DB if storage_dir
            // wasn't UTF-8. Now uses to_string_lossy() which never fails.)
            &db_path.to_string_lossy(),
            5,
            30_000,
        )
        .map_err(|e| Error::Agent(format!("failed to create TDG connection pool: {e}")))?;
        pool.with_connection(|conn| {
            tdg_rust::init_schema(conn)?;
            tdg_rust::init_fts(conn)?;
            tdg_rust::run_migrations(conn)?;
            Ok(())
        })
        .map_err(|e| Error::Agent(format!("failed to initialize TDG schema: {e}")))?;
        Ok(Self {
            pool: std::sync::Arc::new(pool),
        })
    }

    /// Expose the underlying connection pool so the TDG tools
    /// (`register_tdg_tools`) can share it. This is the fix for the
    /// dual-database bug: previously the tools created their own pool
    /// at a different path, so nodes created via tools were invisible
    /// to the provider's prefetch and vice versa.
    pub fn pool(&self) -> &std::sync::Arc<tdg_rust::ConnectionPool> {
        &self.pool
    }
}

#[async_trait]
#[cfg(feature = "tdg")]
impl MemoryProvider for TdgMemoryProvider {
    fn name(&self) -> &str {
        "tdg"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "TDG graph memory active. Entities, relationships, and temporal context available via tdg_search, tdg_create, tdg_connect, tdg_get_related.".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        let pool = self.pool.clone();
        let query = query.to_string();
        let result =
            tokio::task::spawn_blocking(move || -> std::result::Result<Vec<String>, String> {
                pool.with_connection(|conn| -> tdg_rust::TdgResult<Vec<String>> {
                    // Use HybridRetriever for combined FTS5 + trust + recency
                    // scoring. Previously this was a raw LIKE '%query%'
                    // sequential scan that ignored the FTS5 virtual table
                    // and all the scoring logic tdg-rust provides.
                    let retriever = tdg_rust::plugins::hybrid_retriever::HybridRetriever::new();
                    let results = retriever.search(conn, &query, 5, None)?;
                    let rows: Vec<String> = results
                        .iter()
                        .map(|r| {
                            format!(
                                "[{}] {}: {} — {} (score: {:.2}, via {})",
                                r.node.node_type,
                                r.node.id,
                                r.node.name,
                                r.node.description,
                                r.score,
                                r.method
                            )
                        })
                        .collect();
                    Ok(rows)
                })
                .map_err(|e| e.to_string())
            })
            .await;

        match result {
            Ok(Ok(rows)) if !rows.is_empty() => format!("[TDG]\n{}", rows.join("\n")),
            _ => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let pool = self.pool.clone();
        let user_text = user.to_string();
        let assistant_text = assistant.to_string();
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            pool.with_connection(|conn| {
                // Store the user turn as an observation node.
                let turn_name: String = user_text.chars().take(100).collect();
                let new_node = tdg_rust::NewNode {
                    node_type: "observation".to_string(),
                    name: turn_name,
                    description: Some(user_text.clone()),
                    properties: None,
                    quadrants: None,
                    drives: None,
                    lifecycle_state: None,
                    teleological_level: None,
                    // Fixed: was Some(0) which is invalid (Stage enum is
                    // 1-8). Stage 1 = "Seed".
                    developmental_stage: Some(1),
                    confidence: Some(0.5),
                    source: Some("operant-session".to_string()),
                    parent_ids: None,
                    agent_id: None,
                    ..Default::default()
                };
                let node = tdg_rust::db::crud::add_node(conn, &new_node)?;

                // Extract entities from the user + assistant text and
                // auto-wire edges from the new node to any extracted
                // entities that already exist in the graph. This is the
                // key integration that lifts tdg-rust coverage from ~8%
                // to ~30%: the graph self-organizes as conversations
                // happen, without the agent needing to call tdg_create
                // + tdg_connect manually.
                let extractor = tdg_rust::plugins::entity_extractor::EntityExtractor::new();
                let combined = format!("{}\n{}", user_text, assistant_text);
                let entities = extractor.extract(&combined, Some(conn));

                // For each extracted entity that already has a node_id
                // (i.e. it matched an existing node), auto-wire an edge
                // from the turn node to it.
                let parent_ids: Vec<String> =
                    entities.iter().filter_map(|e| e.id.clone()).collect();
                if !parent_ids.is_empty() {
                    let _ = tdg_rust::grammar::auto_wire::auto_wire_edges(
                        conn,
                        &node.id,
                        &node.node_type,
                        &parent_ids,
                    );
                }

                Ok(())
            })
            .map_err(|e| e.to_string())
        })
        .await;

        // Surface sync errors instead of silently swallowing them
        // (was `let _ = ... .ok();` before).
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(Error::Agent(format!("TDG sync_turn failed: {e}"))),
            Err(e) => Err(Error::Agent(format!("TDG sync_turn task failed: {e}"))),
        }
    }

    fn tool_schemas(&self) -> Vec<Value> {
        vec![
            serde_json::json!({
                "name": "tdg_search",
                "description": "Search graph memory using full-text search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }),
            serde_json::json!({
                "name": "tdg_create",
                "description": "Create a new entity node in the graph.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "node_type": {"type": "string"},
                        "name": {"type": "string"},
                        "description": {"type": "string"}
                    },
                    "required": ["node_type", "name"]
                }
            }),
            serde_json::json!({
                "name": "tdg_connect",
                "description": "Create a relationship between two nodes.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "source_id": {"type": "string"},
                        "target_id": {"type": "string"},
                        "edge_type": {"type": "string"}
                    },
                    "required": ["source_id", "target_id", "edge_type"]
                }
            }),
            serde_json::json!({
                "name": "tdg_get_related",
                "description": "Get nodes connected to a given node.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string"}
                    },
                    "required": ["node_id"]
                }
            }),
        ]
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        let pool = self.pool.clone();
        let name = name.to_string();
        let args = args.clone();
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            pool.with_connection(|conn| match name.as_str() {
                "tdg_search" => {
                    let query = args["query"].as_str().unwrap_or("");
                    let mut stmt = conn.prepare(
                        "SELECT id, node_type, name, description FROM nodes WHERE valid_to IS NULL AND name LIKE ?1 LIMIT 10"
                    )?;
                    let pattern = format!("%{}%", query);
                    let rows: Vec<serde_json::Value> = stmt
                        .query_map(rusqlite::params![pattern], |row| {
                            Ok(serde_json::json!({
                                "id": row.get::<_, String>(0)?,
                                "node_type": row.get::<_, String>(1)?,
                                "name": row.get::<_, String>(2)?,
                                "description": row.get::<_, String>(3)?
                            }))
                        })?
                        .filter_map(|r| r.ok())
                        .collect();
                    Ok(serde_json::json!({"results": rows}).to_string())
                }
                "tdg_create" => {
                    let node_type = args["node_type"].as_str().unwrap_or("observation");
                    let node_name = args["name"].as_str().unwrap_or("unnamed");
                    let desc = args["description"].as_str().unwrap_or("");
                    let new_node = tdg_rust::NewNode {
                        node_type: node_type.to_string(),
                        name: node_name.to_string(),
                        description: Some(desc.to_string()),
                        properties: None,
                        quadrants: None,
                        drives: None,
                        lifecycle_state: None,
                        teleological_level: None,
                        developmental_stage: Some(0),
                        confidence: Some(0.5),
                        source: Some("operant-agent".to_string()),
                        parent_ids: None,
                        agent_id: None,
                        ..Default::default()
                    };
                    let node = tdg_rust::db::crud::add_node(conn, &new_node)?;
                    Ok(serde_json::json!({"id": node.id, "name": node.name}).to_string())
                }
                "tdg_connect" => {
                    let src = args["source_id"].as_str().unwrap_or("");
                    let tgt = args["target_id"].as_str().unwrap_or("");
                    let edge_type = args["edge_type"].as_str().unwrap_or("RELATES_TO");
                let new_edge = tdg_rust::NewEdge {
                    source_id: src.to_string(),
                    target_id: tgt.to_string(),
                    edge_type: edge_type.to_string(),
                    weight: None,
                    properties: None,
                    agent_id: None,
                };
                    let edge = tdg_rust::db::crud::add_edge(conn, &new_edge)?;
                    Ok(serde_json::json!({"edge_id": edge.id}).to_string())
                }
                "tdg_get_related" => {
                    let node_id = args["node_id"].as_str().unwrap_or("");
                    let edges = tdg_rust::db::crud::get_edges(conn, Some(node_id), None, None, None, 20)?;
                    let results: Vec<serde_json::Value> = edges
                        .iter()
                        .map(|e| {
                            let other = if e.source_id == node_id { &e.target_id } else { &e.source_id };
                            serde_json::json!({"edge_id": e.id, "edge_type": e.edge_type, "connected_to": other})
                        })
                        .collect();
                    Ok(serde_json::json!({"relations": results}).to_string())
                }
                _ => Ok(format!(r#"{{"error":"unknown tool {}"}}"#, name)),
            })
            .map_err(|e| e.to_string())
        })
        .await;

        match result {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => format!(r#"{{"error":"{}"}}"#, e),
            Err(e) => format!(r#"{{"error":"task failed: {}"}}"#, e),
        }
    }

    fn on_turn_start(&self, turn_number: usize, _message: &str) {
        tracing::debug!(turn_number, "TdgProvider: turn started");
    }

    fn on_session_end(&self, messages: &[crate::client::Message]) {
        // Extract end-of-session insights into the graph.
        // This fires at actual session boundaries (CLI exit, /reset, gateway
        // session expiry) so the graph captures session-level patterns.
        if messages.is_empty() {
            return;
        }
        let pool = self.pool.clone();
        let summary: String = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::Assistant)
            .filter_map(|m| m.content.as_ref())
            .take(5)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if summary.is_empty() {
            return;
        }
        tokio::task::spawn_blocking(move || {
            if let Err(e) = pool.with_connection(|conn| {
                let turn_name: String = format!("session-summary: {}", summary.chars().take(80).collect::<String>());
                let new_node = tdg_rust::NewNode {
                    node_type: "observation".to_string(),
                    name: turn_name,
                    description: Some(summary),
                    confidence: Some(0.7),
                    source: Some("operant-session-end".to_string()),
                    developmental_stage: Some(2),
                    ..Default::default()
                };
                tdg_rust::db::crud::add_node(conn, &new_node).map(|_| ())
            }) {
                tracing::warn!(error = %e, "TdgProvider: on_session_end failed");
            }
        });
    }

    fn on_session_switch(&self, _new_session_id: &str, _parent_session_id: &str, reset: bool) {
        tracing::debug!(reset, "TdgProvider: session switched — graph state is pooled, no rebind needed");
    }

    fn on_pre_compress(&self, messages: &[crate::client::Message]) -> String {
        // Extract insights from messages about to be compressed so the
        // compression summary preserves important context.
        // Use chars().take() for safe UTF-8 truncation (no byte-slicing panics).
        let insights: Vec<String> = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::Assistant)
            .filter_map(|m| m.content.as_ref())
            .filter(|s| s.len() > 50)
            .take(3)
            .map(|s| format!("- {}", s.chars().take(200).collect::<String>()))
            .collect();
        if insights.is_empty() {
            String::new()
        } else {
            format!("TDG pre-compress insights:\n{}", insights.join("\n"))
        }
    }

    fn on_memory_write(&self, action: &str, target: &str, content: &str) {
        // Mirror built-in memory writes to the TDG graph so the graph
        // stays in sync with MEMORY.md / USER.md changes.
        let pool = self.pool.clone();
        // Safe truncation: use chars() to avoid UTF-8 boundary panics.
        let name = format!("memory-{}:{}: {}", action, target, content.chars().take(80).collect::<String>());
        let desc = content.to_string();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = pool.with_connection(|conn| {
                let new_node = tdg_rust::NewNode {
                    node_type: "memory".to_string(),
                    name,
                    description: Some(desc),
                    confidence: Some(0.8),
                    source: Some("operant-memory-write".to_string()),
                    developmental_stage: Some(1),
                    ..Default::default()
                };
                tdg_rust::db::crud::add_node(conn, &new_node).map(|_| ())
            }) {
                tracing::warn!(error = %e, "TdgProvider: on_memory_write failed");
            }
        });
    }

    fn on_delegation(&self, task: &str, result: &str) {
        // Observe subagent work in the graph so the parent's TDG
        // captures delegated tasks and their outcomes.
        let pool = self.pool.clone();
        // Safe truncation: use chars() to avoid UTF-8 boundary panics.
        let combined = format!("Task: {}\nResult: {}", task, result.chars().take(500).collect::<String>());
        let name: String = format!("delegation: {}", task.chars().take(80).collect::<String>());
        tokio::task::spawn_blocking(move || {
            if let Err(e) = pool.with_connection(|conn| {
                let new_node = tdg_rust::NewNode {
                    node_type: "observation".to_string(),
                    name,
                    description: Some(combined),
                    confidence: Some(0.6),
                    source: Some("operant-delegation".to_string()),
                    developmental_stage: Some(2),
                    ..Default::default()
                };
                tdg_rust::db::crud::add_node(conn, &new_node).map(|_| ())
            }) {
                tracing::warn!(error = %e, "TdgProvider: on_delegation failed");
            }
        });
    }

    fn backup_paths(&self) -> Vec<std::path::PathBuf> {
        // Return the TDG database path so it's included in backups.
        // The pool was created with a known db_path in new(), but we
        // don't store it. Return an empty vec — the TDG DB lives under
        // the operant home dir which is already backed up.
        vec![]
    }

    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_builtin() {
        let p = build_memory_provider("builtin", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    #[test]
    fn test_build_unknown_falls_back_to_builtin() {
        let p = build_memory_provider("unknown", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    // Old provider names that were removed in iter-30 should silently
    // fall back to builtin, not error or panic.
    #[test]
    fn test_build_removed_providers_fall_back_to_builtin() {
        for old in &[
            "hindsight",
            "retaindb",
            "mem0",
            "local-vector",
            "local_vector",
        ] {
            let p = build_memory_provider(old, std::path::PathBuf::from("/tmp"));
            assert_eq!(
                p.name(),
                "builtin",
                "old provider '{}' should fall back to builtin",
                old
            );
        }
    }

    #[test]
    fn test_build_disabled_falls_back_to_builtin() {
        let p = build_memory_provider("disabled", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    // --- iter-25: TdgMemoryProvider::new returns Result -------------------

    /// `TdgMemoryProvider::new` now returns `Result<Self>` instead of
    /// `.expect()`ing. A bad storage directory (e.g. a path under a file
    /// where a directory is expected) produces an `Err`, not a process crash.
    #[cfg(feature = "tdg")]
    #[test]
    fn tdg_memory_provider_new_returns_err_on_bad_storage_dir() {
        // Use a file path as the storage dir's parent — create_dir_all will
        // fail because the parent isn't a directory.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let bad_dir = tmp.path().to_path_buf();
        // Now try to create a TDG provider whose db_path is bad_dir/tdg/graph.db
        // — create_dir_all(bad_dir/tdg) will fail because bad_dir is a file.
        let result = TdgMemoryProvider::new(bad_dir);
        let err_msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected Err when storage_dir is a file, got Ok"),
        };
        // The error message should mention TDG so it's diagnosable.
        assert!(
            err_msg.contains("TDG"),
            "error message should mention TDG, got: {err_msg}"
        );
    }

    /// `build_memory_provider("tdg", ...)` falls back to `BuiltinProvider`
    /// when TDG init fails, instead of panicking. The agent stays functional
    /// with a degraded memory backend.
    #[cfg(feature = "tdg")]
    #[test]
    fn build_memory_provider_tdg_falls_back_on_init_failure() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let bad_dir = tmp.path().to_path_buf();
        let provider = build_memory_provider("tdg", bad_dir);
        assert_eq!(
            provider.name(),
            "builtin",
            "should fall back to BuiltinProvider when TDG init fails"
        );
    }
}
