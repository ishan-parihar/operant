//! Memory operation tools
//!
//! Tools for storing, searching, and recalling memories.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

use std::sync::LazyLock;

fn memory_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".operant")
        .join("memory")
        .join("tool_memories.json")
}

/// Global handle to the live MemoryManager (the injected MEMORY.md store).
///
/// Set once at agent construction by the CLI so the memory tools write into
/// the same store that `build_memory_context` injects into the prompt
/// (hermes parity: one coherent memory surface). When unset (unit tests,
/// standalone use) the tools fall back to the legacy JSON file store.
pub(crate) static ACTIVE_MEMORY_MANAGER: LazyLock<RwLock<Option<crate::memory::MemoryManager>>> =
    LazyLock::new(|| RwLock::new(None));

/// Register the active injected store for the memory tools.
pub async fn set_active_memory_manager(manager: crate::memory::MemoryManager) {
    *ACTIVE_MEMORY_MANAGER.write().await = Some(manager);
}

fn load_store() -> HashMap<String, MemoryEntry> {
    let path = memory_file_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_store(store: &HashMap<String, MemoryEntry>) {
    let path = memory_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(store).unwrap_or_default(),
    );
}

static MEMORY_STORE: LazyLock<Arc<RwLock<HashMap<String, MemoryEntry>>>> =
    LazyLock::new(|| Arc::new(RwLock::new(load_store())));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryEntry {
    content: String,
    block_type: String,
    importance: u8,
    tags: Vec<String>,
    created_at: i64,
}

/// Tool for storing a memory
pub struct MemoryStoreTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryStoreArgs {
    key: String,
    content: String,
    block_type: Option<String>,
    importance: Option<u8>,
    tags: Option<Vec<String>>,
}

#[async_trait]
impl OperantTool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store a piece of information in the builtin memory (MEMORY.md file store). Useful for remembering facts, preferences, or user information. When the agentmemory backend is active, prefer memory_save so the fact also lands in the shared semantic store."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<MemoryStoreArgs>("memory_store", "Store information in memory")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: MemoryStoreArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("memory_store", format!("Invalid arguments: {}", e));
            }
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Route through the injected MemoryManager when one is active so the
        // memory lands in the store that gets injected into the prompt
        // (hermes parity). Fall back to the legacy JSON store otherwise.
        if let Some(manager) = ACTIVE_MEMORY_MANAGER.read().await.clone() {
            let block = crate::memory::MemoryBlock::new(
                args.key.clone(),
                args.block_type
                    .clone()
                    .unwrap_or_else(|| "general".to_string()),
                args.content.clone(),
            )
            .importance(args.importance.unwrap_or(50).min(100))
            .tags(args.tags.clone().unwrap_or_default());
            manager.store(block).await;
            let _ = manager.save_to_disk().await;
            return ToolResult::success(
                "memory_store",
                serde_json::json!({
                    "key": args.key,
                    "stored": true,
                    "timestamp": now
                }),
            );
        }

        let entry = MemoryEntry {
            content: args.content.clone(),
            block_type: args.block_type.unwrap_or_else(|| "general".to_string()),
            importance: args.importance.unwrap_or(50).min(100),
            tags: args.tags.unwrap_or_default(),
            created_at: now,
        };

        let mut store = MEMORY_STORE.write().await;
        store.insert(args.key.clone(), entry);
        save_store(&store);
        drop(store);

        ToolResult::success(
            "memory_store",
            serde_json::json!({
                "key": args.key,
                "stored": true,
                "timestamp": now
            }),
        )
    }
}

/// Tool for searching memories
pub struct MemorySearchTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemorySearchArgs {
    query: String,
    max_results: Option<usize>,
}

#[async_trait]
impl OperantTool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search the builtin memory (MEMORY.md file store) for information matching a query — matches content and tags. Facts saved via memory_save live in the agentmemory semantic store and are found with memory_smart_search, not this tool."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<MemorySearchArgs>("memory_search", "Search memories")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: MemorySearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("memory_search", format!("Invalid arguments: {}", e));
            }
        };

        let max_results = args.max_results.unwrap_or(10).min(50);
        let query_lower = args.query.to_lowercase();

        // Route through the injected MemoryManager when active so search hits
        // the same store that feeds the prompt (hermes parity).
        if let Some(manager) = ACTIVE_MEMORY_MANAGER.read().await.clone() {
            let results: Vec<serde_json::Value> = manager
                .search(&args.query)
                .await
                .into_iter()
                .take(max_results)
                .map(|block| {
                    serde_json::json!({
                        "key": block.id,
                        "content": block.content,
                        "block_type": block.block_type,
                        "importance": block.importance,
                        "tags": block.tags,
                        "created_at": block.created_at,
                        "relevance": if block.content.to_lowercase().contains(&query_lower) { 1.0 } else { 0.5 }
                    })
                })
                .collect();
            return ToolResult::success(
                "memory_search",
                serde_json::json!({
                    "query": args.query,
                    "results": results,
                    "count": results.len()
                }),
            );
        }

        let store = MEMORY_STORE.read().await;
        let mut results = Vec::new();

        for (key, entry) in store.iter() {
            let content_match = entry.content.to_lowercase().contains(&query_lower);
            let tag_match = entry
                .tags
                .iter()
                .any(|t| t.to_lowercase().contains(&query_lower));
            let type_match = entry.block_type.to_lowercase().contains(&query_lower);

            if content_match || tag_match || type_match {
                results.push(serde_json::json!({
                    "key": key,
                    "content": entry.content,
                    "block_type": entry.block_type,
                    "importance": entry.importance,
                    "tags": entry.tags,
                    "created_at": entry.created_at,
                    "relevance": if content_match { 1.0 } else { 0.5 }
                }));

                if results.len() >= max_results {
                    break;
                }
            }
        }

        // Sort by relevance (content match first)
        results.sort_by(|a, b| {
            let relevance_a = a["relevance"].as_f64().unwrap_or(0.0);
            let relevance_b = b["relevance"].as_f64().unwrap_or(0.0);
            relevance_b
                .partial_cmp(&relevance_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        ToolResult::success(
            "memory_search",
            serde_json::json!({
                "query": args.query,
                "results": results,
                "count": results.len()
            }),
        )
    }
}

/// Tool for recalling a specific memory
pub struct MemoryRecallTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryRecallArgs {
    key: String,
}

#[async_trait]
impl OperantTool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Recall a specific memory by its key from the builtin memory (MEMORY.md file store). Facts saved via memory_save (agentmemory) are stored in the semantic store and recalled with memory_smart_search instead."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<MemoryRecallArgs>("memory_recall", "Recall a specific memory")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: MemoryRecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("memory_recall", format!("Invalid arguments: {}", e));
            }
        };

        let store = MEMORY_STORE.read().await;

        // Route through the injected MemoryManager when active (hermes parity).
        if let Some(manager) = ACTIVE_MEMORY_MANAGER.read().await.clone() {
            match manager.get(&args.key).await {
                Some(block) => ToolResult::success(
                    "memory_recall",
                    serde_json::json!({
                        "key": block.id,
                        "content": block.content,
                        "block_type": block.block_type,
                        "importance": block.importance,
                        "tags": block.tags,
                        "created_at": block.created_at,
                        "found": true
                    }),
                ),
                None => ToolResult::error(
                    "memory_recall",
                    format!("Memory with key '{}' not found", args.key),
                ),
            }
        } else {
            match store.get(&args.key) {
                Some(entry) => ToolResult::success(
                    "memory_recall",
                    serde_json::json!({
                        "key": args.key,
                        "content": entry.content,
                        "block_type": entry.block_type,
                        "importance": entry.importance,
                        "tags": entry.tags,
                        "created_at": entry.created_at,
                        "found": true
                    }),
                ),
                None => ToolResult::error(
                    "memory_recall",
                    format!("Memory with key '{}' not found", args.key),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Serializes tests that touch ACTIVE_MEMORY_MANAGER so a routing test
    // doesn't leak its global hook into parallel legacy tests.
    static TEST_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn test_memory_store_schema() {
        let schema = MemoryStoreTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "memory_store");
    }

    #[test]
    fn test_memory_search_schema() {
        let schema = MemorySearchTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "memory_search");
    }

    #[test]
    fn test_memory_recall_schema() {
        let schema = MemoryRecallTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "memory_recall");
    }

    #[tokio::test]
    async fn test_memory_store_and_recall() {
        let _guard = TEST_GUARD.lock().await;
        let store = MemoryStoreTool;
        let recall = MemoryRecallTool;

        // Store a memory
        let store_result = store
            .execute(
                json!({"key": "test_key", "content": "test content"}),
                ToolContext::default(),
            )
            .await;
        assert!(store_result.success);

        // Recall it
        let recall_result = recall
            .execute(json!({"key": "test_key"}), ToolContext::default())
            .await;
        assert!(recall_result.success);
    }

    #[tokio::test]
    async fn test_memory_store_missing_args() {
        let tool = MemoryStoreTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_memory_search_empty() {
        let tool = MemorySearchTool;
        let result = tool
            .execute(json!({"query": "nonexistent"}), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_memory_recall_not_found() {
        let tool = MemoryRecallTool;
        let result = tool
            .execute(json!({"key": "nonexistent_key"}), ToolContext::default())
            .await;
        assert!(!result.success); // Returns error when key not found
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_memory_tools_route_through_active_manager() {
        let _guard = TEST_GUARD.lock().await;
        let dir = std::env::temp_dir().join(format!("operant-mm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let manager = crate::memory::MemoryManager::with_storage_dir(dir.clone());
        manager.load_from_disk().await.unwrap();

        set_active_memory_manager(manager.clone()).await;

        let store = MemoryStoreTool;
        let result = store
            .execute(
                json!({"key": "routed_key", "content": "routed content", "importance": 70}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "store should succeed via active manager");

        let recall = MemoryRecallTool;
        let recall_result = recall
            .execute(json!({"key": "routed_key"}), ToolContext::default())
            .await;
        assert!(
            recall_result.success,
            "recall via active manager: {:?}",
            recall_result
        );

        let got = manager.get("routed_key").await;
        assert!(
            got.is_some(),
            "memory should land in the injected manager store"
        );
        assert_eq!(got.unwrap().importance, 70);

        // reset the hook so other tests keep using the JSON fallback
        *ACTIVE_MEMORY_MANAGER.write().await = None;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
