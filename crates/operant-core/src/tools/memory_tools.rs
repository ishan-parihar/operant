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
    /// (iter-1307) Optional when `key` is given: the retired `memory_recall`
    /// shape was key-only, and the fold must accept it. Exactly one of
    /// query/key is required (validated below).
    query: Option<String>,
    /// (plan 2026-10-26 phase 2, RT-192) Optional exact key: performs the
    /// recall-by-key the retired `memory_recall` tool did (the store tag
    /// carries key:<id>). Absent = plain semantic search.
    key: Option<String>,
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

        // (iter-1307) Exactly one of query/key is required: key-only is the
        // retired memory_recall shape (folded, phase 2); neither is a genuine
        // misuse no deserializer can express — validated here instead.
        let Some(search_query) = args
            .key
            .as_deref()
            .filter(|k| !k.trim().is_empty())
            .or(args.query.as_deref().filter(|q| !q.trim().is_empty()))
        else {
            return ToolResult::error(
                "memory_search",
                "Provide either 'query' (semantic search) or 'key' (exact-key recall).".to_string(),
            );
        };

        // (plan 11C3) one backend: semantic recall over memory-wire.
        let Some(provider) = ACTIVE_MEMORY_PROVIDER.read().await.clone() else {
            return ToolResult::error(
                "memory_search",
                "No memory backend attached in this session.".to_string(),
            );
        };
        // (plan 2026-10-26, RT-193) real fused RRF score per hit, not a
        // fabricated constant: the discarded `score` was the audit finding.
        let hits = provider.recall_scored(search_query, max_results);
        let results: Vec<serde_json::Value> = hits
            .iter()
            .filter(|h| !h.memory.content.trim().is_empty())
            .map(|h| {
                serde_json::json!({
                    "content": h.memory.content.trim(),
                    "relevance": h.score
                })
            })
            .collect();
        ToolResult::success(
            "memory_search",
            serde_json::json!({
                "query": args.query.clone().unwrap_or_else(|| search_query.to_string()),
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
        // PIN THE ORIGIN. `PROVIDER_TEST_GUARD` serialises this test against
        // the sibling that installs a provider, but `WRITE_ORIGIN` is a
        // DIFFERENT process-global and this test never touched it — so the
        // origin whichever earlier test left installed was still installed
        // here. A leaked "background_review" makes `store.execute` answer
        // "Refused: background review does not write memory" instead of
        // "No memory backend", so the assertion below failed in a
        // serializer whose order is not fixed: measured as 1 failure in one
        // full-suite run (2458/1) and 0 failures in isolation and in every
        // run since (RT-170's family — the test reading ambient global state
        // rather than a fixture). Same shape and same fix as the sibling's
        // comment at `memory_tools_route_through_the_provider`: take the
        // origin's lock FIRST (so no holder is mid-write), then install a
        // guard whose Drop ends the pin.
        let _origin_lock = crate::write_origin::origin_test_lock().await;
        let _origin = crate::write_origin::WriteOriginGuard::new("user");
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
        // PIN the write origin. `WRITE_ORIGIN` is process-global, and
        // `is_background_review()` reads it when the caller is not inside
        // a scoped tokio task — which is this test. So a test elsewhere
        // that set it to "background_review" made the memory store REFUSE
        // here ("Refused: background review does not write memory"), and
        // the failure appeared in roughly one full-suite run in ten.
        //
        // The origin lock serialises against every other test that
        // touches the global; setting it to "user" makes this test's
        // expectation explicit rather than ambient.
        let _origin_lock = crate::write_origin::origin_test_lock().await;
        // The GUARD, not `set_write_origin`: the token has no `Drop`, so a
        // bare `set_write_origin` would leak "user" into the global —
        // the very leak this fixes.
        let _origin = crate::write_origin::WriteOriginGuard::new("user");
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

    // ── Phase 3 (plan 2026-10-26, RT-193): score survives the wire ──

    /// (plan 2026-10-26, RT-193) A store with one high-relevance and one
    /// low-relevance row must return DISTINCT, ORDERED scores through the
    /// tool layer — the hard-coded relevance:1.0 masked the ranking.
    #[tokio::test]
    async fn memory_search_reports_the_real_fused_score_per_hit() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        let _origin_lock = crate::write_origin::origin_test_lock().await;
        let _origin = crate::write_origin::WriteOriginGuard::new("user");
        let dir = std::env::temp_dir().join(format!("operant-mw-score-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider =
            std::sync::Arc::new(crate::memory_wire::MemoryWireProvider::new(dir.clone()).unwrap());
        set_active_memory_provider(provider.clone()).await;

        // Two facts with very different relevance to the query.
        let store = MemoryStoreTool;
        let a = store
            .execute(
                json!({"key": "koi-pond-depth", "content": "the koi pond depth is exactly two meters"}),
                ToolContext::default(),
            )
            .await;
        assert!(a.success, "store a: {a:?}");
        let b = store
            .execute(
                json!({"key": "unrelated-weather", "content": "sunny weather prevails over the valley today"}),
                ToolContext::default(),
            )
            .await;
        assert!(b.success, "store b: {b:?}");

        let search = MemorySearchTool;
        let result = search
            .execute(
                json!({"query": "koi pond depth", "maxResults": 2}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "search: {result:?}");
        let payload: serde_json::Value =
            serde_json::from_str(&result.content).expect("search payload");
        let results = payload["results"].as_array().expect("results array");
        assert!(!results.is_empty(), "must find the koi fact: {payload}");
        // RED today: every hit carries the fabricated relevance:1.0 —
        // the test demands the REAL fused score (a float that varies
        // between hits and is not constantly 1.0).
        let scores: Vec<f64> = results
            .iter()
            .filter_map(|r| r.get("relevance").and_then(|v| v.as_f64()))
            .collect();
        assert_eq!(scores.len(), results.len(), "every hit carries a score");
        assert!(
            scores.iter().any(|s| *s != 1.0),
            "the relevance must be the REAL fused score, not the fabricated 1.0: {scores:?}"
        );
        if scores.len() > 1 {
            let first = scores[0];
            assert!(
                scores.iter().all(|s| s <= &first),
                "hits arrive in rank order (score descending): {scores:?}"
            );
        }

        *ACTIVE_MEMORY_PROVIDER.write().await = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The prefetch TEXT contract is unchanged (no JSON, same [memory]
    /// header) — scores rank internally but the injected block stays text.
    #[tokio::test]
    async fn prefetch_text_contract_is_unchanged_by_scores() {
        let p = crate::memory_wire::in_memory_for_tests();
        p.sync_turn("koi pond depth", "two meters exactly")
            .await
            .expect("sync_turn");
        let recalled = p.prefetch("koi pond").await;
        assert!(
            recalled.starts_with("[memory]") || recalled.is_empty(),
            "the injected block is text with the same header: {recalled:?}"
        );
        assert!(
            !recalled.contains("\"relevance\""),
            "no JSON in prefetch text"
        );
    }

    // ── Deep-validation findings (2026-10-26, iter-1307): the alias fold
    // was name-only — the MCP pre-check (`contains`) rejected retired
    // names, and the retired argument shape ({key} only, no query) failed
    // deserialization. Both pinned here RED-first. ──

    /// (iter-1307) The registry's `contains` must resolve aliases: the MCP
    /// serve pre-check calls `contains()` before `execute()`, so a retired
    /// name died as "Unknown tool" over the MCP surface even though
    /// `get`/`execute` resolve it. Found by live MCP probe.
    #[tokio::test]
    async fn contains_resolves_retired_names_for_the_mcp_precheck() {
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(5));
        crate::tools::builtin::register_memory_tools(&registry)
            .await
            .unwrap();
        assert!(
            registry.contains("memory_recall").await,
            "MCP pre-check passes for memory_recall"
        );
        assert!(
            registry.contains("memory_save").await,
            "MCP pre-check passes for memory_save"
        );
        assert!(
            registry.contains("memory_smart_search").await,
            "MCP pre-check passes for memory_smart_search"
        );
        assert!(!registry.contains("memory_not_a_thing").await);
    }

    /// (iter-1307) The retired `memory_recall` argument shape was `{key}`
    /// ONLY — the fold must accept it. A key-only call (no query) routes
    /// through the key; neither query nor key is the only invalid shape.
    #[tokio::test]
    async fn memory_search_accepts_key_only_the_recall_shape() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        let _origin_lock = crate::write_origin::origin_test_lock().await;
        let _origin = crate::write_origin::WriteOriginGuard::new("user");
        let dir = std::env::temp_dir().join(format!("operant-mw-keyonly-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider =
            std::sync::Arc::new(crate::memory_wire::MemoryWireProvider::new(dir.clone()).unwrap());
        set_active_memory_provider(provider.clone()).await;

        let store = MemoryStoreTool;
        let stored = store
            .execute(
                json!({"key": "folded-key-probe", "content": "the folded recall probe fact"}),
                ToolContext::default(),
            )
            .await;
        assert!(stored.success, "store: {stored:?}");

        // The retired memory_recall shape: key ONLY, no query field.
        let search = MemorySearchTool;
        let key_only = search
            .execute(json!({"key": "folded-key-probe"}), ToolContext::default())
            .await;
        assert!(
            key_only.success,
            "key-only (the memory_recall argument shape) must work: {key_only:?}"
        );
        assert!(
            key_only.content.contains("folded recall probe"),
            "the key-only call must recall the stored fact: {}",
            key_only.content
        );

        // Neither query nor key: the ONE invalid shape.
        let neither = search.execute(json!({}), ToolContext::default()).await;
        assert!(!neither.success, "neither query nor key must be rejected");

        *ACTIVE_MEMORY_PROVIDER.write().await = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Phase 2 (plan 2026-10-26, RT-192): 5 tools → 2 ─────────────

    /// The consolidated surface registers exactly `memory_store` and
    /// `memory_search` — the twins (`memory_save`, `memory_smart_search`)
    /// and the folded `memory_recall` are gone from the registry.
    #[tokio::test]
    async fn the_registry_holds_exactly_two_memory_tools() {
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(5));
        crate::tools::builtin::register_memory_tools(&registry)
            .await
            .unwrap();
        let mut names: Vec<String> = registry
            .get_schemas_all()
            .await
            .into_iter()
            .map(|s| s.name)
            .collect();
        // (iter-1307) sort: get_schemas_all iterates a HashMap, so the
        // order is seed-random per process — the contract is the SET.
        names.sort();
        assert_eq!(
            names,
            vec!["memory_search".to_string(), "memory_store".to_string()],
            "the memory toolset is exactly two tools: {names:?}"
        );
    }

    /// `memory_search` accepts the optional `key` param and performs the
    /// exact-key recall the old `memory_recall` did (plan 2.2: the key
    /// param routes to `recall_text(key)` — the store tag carries key:<id>).
    #[tokio::test]
    async fn memory_search_recalls_by_key_when_key_is_given() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        let _origin_lock = crate::write_origin::origin_test_lock().await;
        let _origin = crate::write_origin::WriteOriginGuard::new("user");
        let dir = std::env::temp_dir().join(format!("operant-mw-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider =
            std::sync::Arc::new(crate::memory_wire::MemoryWireProvider::new(dir.clone()).unwrap());
        set_active_memory_provider(provider.clone()).await;

        // Store a fact under a key first.
        let store = MemoryStoreTool;
        let stored = store
            .execute(
                json!({"key": "zephyr-catch", "content": "the zephyr catch count is 42"}),
                ToolContext::default(),
            )
            .await;
        assert!(stored.success, "store first: {stored:?}");

        // The folded behavior: memory_search WITH key performs the exact-key
        // recall the old memory_recall performed.
        let search = MemorySearchTool;
        let result = search
            .execute(
                json!({"query": "zephyr", "key": "zephyr-catch"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "key recall: {result:?}");
        assert!(
            result.content.to_string().contains("42"),
            "the keyed fact must come back: {result:?}"
        );

        *ACTIVE_MEMORY_PROVIDER.write().await = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The retired names still EXECUTE through the alias layer (plan 2.4:
    /// one release of aliases): a model calling `memory_recall` gets the
    /// folded behavior with a deprecation note, and `memory_save` /
    /// `memory_smart_search` map to their surviving twins. No schema cost —
    /// the aliases live in the registry's resolve step, not the wire.
    #[tokio::test]
    async fn retired_memory_names_still_execute_through_the_alias_layer() {
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(5));
        crate::tools::builtin::register_memory_tools(&registry)
            .await
            .unwrap();
        // The alias RESOLUTION (not registration) is what keeps old names
        // callable: execute("memory_recall", …) must resolve to the folded
        // memory_search tool.
        let tool = registry.get("memory_recall").await;
        assert!(
            tool.is_some(),
            "the alias layer must resolve memory_recall to the consolidated tool"
        );
        let tool = registry.get("memory_save").await;
        assert!(tool.is_some(), "memory_save must alias memory_store");
        let tool = registry.get("memory_smart_search").await;
        assert!(
            tool.is_some(),
            "memory_smart_search must alias memory_search"
        );
        // And the schema list (the WIRE) carries neither — aliases cost 0.
        let wire_names: Vec<String> = registry
            .get_schemas_all()
            .await
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(!wire_names.contains(&"memory_recall".to_string()));
        assert!(!wire_names.contains(&"memory_save".to_string()));
        assert!(!wire_names.contains(&"memory_smart_search".to_string()));
    }
}

/// Guards the invariant `test_memory_store_and_recall` lost: a memory-tool
/// call with NO provider attached must fail naming the ABSENT BACKEND, and
/// must never fail naming the caller's write origin.
///
/// Why this is a test of its own: both tests above hold
/// `PROVIDER_TEST_GUARD`, which serialises them against each other, but
/// `WRITE_ORIGIN` is a SEPARATE process-global that neither guard order
/// covers for a reader outside the module. A future test that installs a
/// `WriteOriginGuard` and leaves it held across an await — or that sets the
/// origin without the guard, so its Drop never restores the previous value —
/// reintroduces exactly this failure. The assertions name both halves, so
/// the reader is told which global leaked.
#[cfg(test)]
mod absent_backend_error_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn the_absent_backend_is_reported_independently_of_the_write_origin() {
        let _guard = PROVIDER_TEST_GUARD.lock().await;
        let _origin_lock = crate::write_origin::origin_test_lock().await;
        let _origin = crate::write_origin::WriteOriginGuard::new("user");

        clear_active_memory_provider_for_tests().await;
        let store_result = MemoryStoreTool
            .execute(json!({"key": "k", "content": "c"}), ToolContext::default())
            .await;

        assert!(!store_result.success, "no backend = honest error");
        let err = store_result.error.unwrap_or_default();
        assert!(
            err.contains("No memory backend"),
            "the error must name the absent backend, not the caller's origin: {err:?}"
        );
        assert!(
            !err.contains("background review"),
            "the error must not be a write-origin refusal, which would mean a leaked WRITE_ORIGIN global: {err:?}"
        );
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
