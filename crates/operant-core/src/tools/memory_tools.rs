//! Memory operation tools
//!
//! Tools for storing, searching, and recalling memories.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::memory_provider::MemoryProvider;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

use std::sync::LazyLock;

/// (plan 11C3) Global handle to the live memory-wire provider — the ONE
/// memory backend. Set once at agent construction by the CLI so the memory
/// tools write into the same store the provider recalls from. When unset
/// (unit tests, standalone use) the tools report that memory is not
/// attached instead of silently writing to a fallback nobody reads.
pub(crate) static ACTIVE_MEMORY_PROVIDER: LazyLock<
    RwLock<Option<std::sync::Arc<crate::memory_wire::MemoryWireProvider>>>,
> = LazyLock::new(|| RwLock::new(None));

/// Register the active provider for the memory tools.
pub async fn set_active_memory_provider(
    provider: std::sync::Arc<crate::memory_wire::MemoryWireProvider>,
) {
    *ACTIVE_MEMORY_PROVIDER.write().await = Some(provider);
}

/// (plan 11C3) A snapshot of the active provider handle (distillation and
/// other background writers).
pub async fn active_memory_provider()
-> Option<std::sync::Arc<crate::memory_wire::MemoryWireProvider>> {
    ACTIVE_MEMORY_PROVIDER.read().await.clone()
}

/// Test hook: detach the active provider so cross-module tests don't leak
/// their handle into each other.
pub async fn clear_active_memory_provider_for_tests() {
    *ACTIVE_MEMORY_PROVIDER.write().await = None;
}

/// Cross-module test lock: every test that attaches or clears the ACTIVE
/// provider holds this (the tools' TEST_GUARD covers only this module —
/// distillation.rs's test needs the same serialization).
pub static PROVIDER_TEST_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Tool for storing a memory
pub struct MemoryStoreTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryStoreArgs {
    key: String,
    content: String,
    block_type: Option<String>,
    tags: Option<Vec<String>>,
}

/// Bounds on one `memory_store` write (KB-12). A write over a bound is refused
/// with the bound named, so the model can retry shorter, instead of storing it.
const MAX_STORE_KEY_CHARS: usize = 128;
const MAX_STORE_CONTENT_BYTES: usize = 8_000;
const MAX_STORE_TAGS: usize = 16;
const MAX_STORE_TAG_CHARS: usize = 64;

/// The first bound `args` exceeds, as the message the tool returns.
fn store_bound_violation(args: &MemoryStoreArgs) -> Option<String> {
    if args.key.chars().count() > MAX_STORE_KEY_CHARS {
        return Some(format!(
            "key is longer than {MAX_STORE_KEY_CHARS} characters"
        ));
    }
    if args.content.len() > MAX_STORE_CONTENT_BYTES {
        return Some(format!(
            "content is longer than {MAX_STORE_CONTENT_BYTES} bytes"
        ));
    }
    let tags = args.tags.as_deref().unwrap_or_default();
    if tags.len() > MAX_STORE_TAGS {
        return Some(format!("more than {MAX_STORE_TAGS} tags"));
    }
    if tags
        .iter()
        .any(|tag| tag.chars().count() > MAX_STORE_TAG_CHARS)
    {
        return Some(format!(
            "a tag is longer than {MAX_STORE_TAG_CHARS} characters"
        ));
    }
    None
}

#[async_trait]
impl OperantTool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store a fact in long-term memory (the memory-wire store) so future sessions can recall it. Useful for remembering facts, preferences, or user information. `key` and `blockType` are kept as tags; the same store backs memory_save and memory_smart_search."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<MemoryStoreArgs>("memory_store", "Store information in memory")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        // KB-3: an unattended review does not write persistent memory. The user
        // saves a memory in a turn, where the approval prompt applies.
        if crate::write_origin::is_background_review() {
            return ToolResult::error(
                "memory_store",
                "Refused: background review does not write memory; the user saves it in a turn.",
            );
        }
        let args: MemoryStoreArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("memory_store", format!("Invalid arguments: {}", e));
            }
        };
        if let Some(violation) = store_bound_violation(&args) {
            return ToolResult::error("memory_store", format!("Refused: {violation}."));
        }

        // W6 (audit O-22, invariant 4): a Holon run never writes the
        // personal memory store — the organization's memory is Holon's.
        // The fact lands as a `learnings` annotation for the result; the
        // kernel's own memory carries it from there.
        if let Some(run_dir) = crate::org::holon_learning_sink() {
            crate::org::record_learning_annotation(
                &run_dir,
                "learnings",
                serde_json::json!({
                    "tool": "memory_store",
                    "key": args.key,
                    "content": args.content,
                    "kind": args.block_type,
                }),
            );
            return ToolResult::success(
                "memory_store",
                "Recorded as a learning annotation for this run's result (learnings) — a Holon run does not write the personal memory store; the kernel's memory is the organization's.",
            );
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // (plan 11C3) one backend: the memory-wire provider. The key and
        // type ride as tags so they stay queryable.
        let Some(provider) = ACTIVE_MEMORY_PROVIDER.read().await.clone() else {
            return ToolResult::error(
                "memory_store",
                "No memory backend attached in this session.".to_string(),
            );
        };
        let mut tags = args.tags.clone().unwrap_or_default();
        tags.push(format!("key:{}", args.key));
        if let Some(block_type) = args.block_type.clone() {
            tags.push(format!("type:{block_type}"));
        }
        match provider.retain_fact(&args.content, &tags) {
            Some(_) => ToolResult::success(
                "memory_store",
                serde_json::json!({
                    "key": args.key,
                    "stored": true,
                    "persistent": provider.is_persistent(),
                    "timestamp": now
                }),
            ),
            None => ToolResult::error(
                "memory_store",
                "Memory backend failed to store the fact (see logs).".to_string(),
            ),
        }
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
        "Search long-term memory (the memory-wire store) for facts matching a query, ranked by relevance. Finds everything saved via memory_store or memory_save."
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

        // (plan 11C3) one backend: semantic recall over memory-wire.
        let Some(provider) = ACTIVE_MEMORY_PROVIDER.read().await.clone() else {
            return ToolResult::error(
                "memory_search",
                "No memory backend attached in this session.".to_string(),
            );
        };
        let hits = provider.recall_text(&args.query, max_results);
        let results: Vec<serde_json::Value> = hits
            .lines()
            .filter(|l| l.starts_with("- "))
            .take(max_results)
            .map(|l| {
                serde_json::json!({
                    "content": l.trim_start_matches("- "),
                    "relevance": 1.0
                })
            })
            .collect();
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
        "Recall a remembered fact by the key it was stored under with memory_store (a semantic match on the key tag). For facts saved without a key, use memory_search."
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

        // (plan 11C3) one backend: semantic recall keyed by the asked-for
        // key — memory-wire has no by-id get, and a semantic match on the
        // key is the honest equivalent (the store tag carries key:<id>).
        let Some(provider) = ACTIVE_MEMORY_PROVIDER.read().await.clone() else {
            return ToolResult::error(
                "memory_recall",
                "No memory backend attached in this session.".to_string(),
            );
        };
        let hits = provider.recall_text(&args.key, 5);
        if hits.trim().is_empty() {
            return ToolResult::error(
                "memory_recall",
                format!("Memory with key '{}' not found", args.key),
            );
        }
        ToolResult::success(
            "memory_recall",
            serde_json::json!({
                "key": args.key,
                "content": hits,
                "found": true
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    /// (plan 11C3) no legacy JSON fallback anymore: with no provider
    /// attached, store/recall/search report it instead of silently
    /// writing to a store nobody reads.
    async fn test_memory_store_and_recall() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        clear_active_memory_provider_for_tests().await;
        let store = MemoryStoreTool;
        let recall = MemoryRecallTool;

        let store_result = store
            .execute(
                json!({"key": "test_key", "content": "test content"}),
                ToolContext::default(),
            )
            .await;
        assert!(!store_result.success, "no backend = honest error");
        assert!(store_result.error.unwrap().contains("No memory backend"));

        let recall_result = recall
            .execute(json!({"key": "test_key"}), ToolContext::default())
            .await;
        assert!(!recall_result.success, "no backend = honest error");
    }

    #[tokio::test]
    async fn test_memory_store_missing_args() {
        let tool = MemoryStoreTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_memory_search_empty() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        clear_active_memory_provider_for_tests().await;
        let tool = MemorySearchTool;
        let result = tool
            .execute(json!({"query": "nonexistent"}), ToolContext::default())
            .await;
        assert!(!result.success, "no backend = honest error, not empty list");
    }

    #[tokio::test]
    async fn test_memory_recall_not_found() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        clear_active_memory_provider_for_tests().await;
        let tool = MemoryRecallTool;
        let result = tool
            .execute(json!({"key": "nonexistent_key"}), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("No memory backend"));
    }

    /// (plan 11C3) The tools route through the ONE backend: the memory-wire
    /// provider. A real provider over a temp store; store + recall land in
    /// the same store the provider recalls from.
    #[tokio::test]
    async fn memory_tools_route_through_the_provider() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        let dir = std::env::temp_dir().join(format!("operant-mw-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider =
            std::sync::Arc::new(crate::memory_wire::MemoryWireProvider::new(dir.clone()).unwrap());
        set_active_memory_provider(provider.clone()).await;

        let store = MemoryStoreTool;
        let result = store
            .execute(
                json!({"key": "routed_key", "content": "routed unique content for routing test"}),
                ToolContext::default(),
            )
            .await;
        assert!(
            result.success,
            "store should succeed via the provider: {result:?}"
        );

        let recall = MemoryRecallTool;
        let recall_result = recall
            .execute(json!({"key": "routed_key"}), ToolContext::default())
            .await;
        assert!(
            recall_result.success,
            "recall via the provider: {recall_result:?}"
        );
        let found = crate::memory_wire::MemoryWireProvider::recall_text(
            &provider,
            "routed unique content",
            5,
        );
        assert!(
            !found.trim().is_empty(),
            "the fact is in the shared store: {found:?}"
        );

        *ACTIVE_MEMORY_PROVIDER.write().await = None;
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod store_bounds_tests {
    use super::{MAX_STORE_CONTENT_BYTES, MemoryStoreArgs, store_bound_violation};

    fn args(key: &str, content: &str, tags: Vec<String>) -> MemoryStoreArgs {
        MemoryStoreArgs {
            key: key.to_string(),
            content: content.to_string(),
            block_type: None,
            tags: Some(tags),
        }
    }

    #[test]
    fn an_oversized_write_is_refused_and_a_bounded_one_is_not() {
        // KB-12: an unbounded key, body, tag count or tag is refused by name.
        assert!(store_bound_violation(&args("k", "short", vec![])).is_none());
        let big = "x".repeat(MAX_STORE_CONTENT_BYTES + 1);
        assert!(
            store_bound_violation(&args("k", &big, vec![]))
                .unwrap()
                .contains("content")
        );
        assert!(
            store_bound_violation(&args(&"k".repeat(129), "c", vec![]))
                .unwrap()
                .contains("key")
        );
        assert!(
            store_bound_violation(&args("k", "c", vec!["t".into(); 17]))
                .unwrap()
                .contains("tags")
        );
        assert!(
            store_bound_violation(&args("k", "c", vec!["t".repeat(65)]))
                .unwrap()
                .contains("tag is")
        );
    }
}

#[cfg(test)]
mod background_review_memory_tests {
    use super::{MemoryStoreTool, OperantTool, ToolContext};
    use serde_json::json;

    #[tokio::test]
    async fn a_background_review_cannot_write_memory() {
        // KB-3: the refusal names the review, so a missing provider cannot pass for it.
        let args = json!({"key": "k", "content": "c"});
        let result = crate::write_origin::scope_background_review(
            MemoryStoreTool.execute(args, ToolContext::default()),
        )
        .await;
        assert!(!result.success);
        let refusal = result.error.unwrap_or_default();
        assert!(refusal.contains("background review"), "{refusal}");
    }
}
