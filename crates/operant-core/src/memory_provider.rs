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

use crate::error::{Error, Result};

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

pub struct TdgMemoryProvider {
    pool: std::sync::Arc<tdg_rust::ConnectionPool>,
    storage_dir: std::path::PathBuf,
}

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
            db_path.to_str().unwrap_or("~/.operant/tdg/graph.db"),
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
            storage_dir,
        })
    }
}

#[async_trait]
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
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<String>, String> {
            pool.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, node_type, name, description FROM nodes WHERE valid_to IS NULL AND name LIKE ?1 LIMIT 5"
                )?;
                let pattern = format!("%{}%", query);
                let rows: Vec<String> = stmt
                    .query_map(rusqlite::params![pattern], |row| {
                        let id: String = row.get(0)?;
                        let node_type: String = row.get(1)?;
                        let name: String = row.get(2)?;
                        let desc: String = row.get(3)?;
                        Ok(format!("[{}] {}: {} — {}", node_type, id, name, desc))
                    })?
                    .filter_map(|r| r.ok())
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

    async fn sync_turn(&self, user: &str, _assistant: &str) -> Result<()> {
        let pool = self.pool.clone();
        let user_text = user.to_string();
        let _ = tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            pool.with_connection(|conn| {
                let new_node = tdg_rust::NewNode {
                    node_type: "observation".to_string(),
                    name: user_text.chars().take(100).collect(),
                    description: Some(user_text),
                    properties: None,
                    quadrants: None,
                    drives: None,
                    lifecycle_state: None,
                    teleological_level: None,
                    developmental_stage: Some(0),
                    confidence: Some(0.5),
                    source: Some("operant-session".to_string()),
                    parent_ids: None,
                    agent_id: None,
                    ..Default::default()
                };
                tdg_rust::db::crud::add_node(conn, &new_node)?;
                Ok(())
            })
            .map_err(|e| e.to_string())
        })
        .await
        .ok();
        Ok(())
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
        for old in &["hindsight", "retaindb", "mem0", "local-vector", "local_vector"] {
            let p = build_memory_provider(old, std::path::PathBuf::from("/tmp"));
            assert_eq!(
                p.name(), "builtin",
                "old provider '{}' should fall back to builtin", old
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
