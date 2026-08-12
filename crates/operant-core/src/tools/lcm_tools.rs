//! LCM (Lossless Context Management) agent tools.
//!
//! When `agent.context_engine = "lcm"`, the DAG engine is attached to the
//! agent AND its two tools are registered so the model can use the lossless
//! store directly:
//!   - `lcm_recall` — FTS5 phrase search over the DAG (session-scoped by
//!     default, global when `session` is omitted). Returns verbatim nodes
//!     so nothing ever lost to compaction is truly gone.
//!   - `lcm_stats` — diagnostics: engine name, DAG path, node count, and
//!     the D0 tail token budget.
//!
//! hermes-lcm parity: recall tools are the "memory tool" half of the
//! ContextEngine plugin (see `docs/HERMES_LCM_INTEGRATION.md` P2).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::context::{ContextEngine, LcmContextEngine};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// FTS recall over the lossless DAG.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmRecallArgs {
    /// Free-text query; matched as an FTS5 phrase (best for verbatim hits).
    pub query: String,
    /// Max hits to return (1..=50, default 5).
    pub limit: Option<usize>,
    /// Scope recall to one session id. Omit for global recall across all
    /// sessions in the DAG.
    pub session: Option<String>,
}

pub struct LcmRecallTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmRecallTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmRecallTool {
    fn name(&self) -> &str {
        "lcm_recall"
    }

    fn description(&self) -> &str {
        "Search the lossless context DAG for earlier conversation content that was compacted out of the active window. Returns verbatim messages ranked by relevance. Use when you need details from earlier in this or a previous session."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmRecallArgs>("lcm_recall", "Recall compacted context verbatim")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmRecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_recall", format!("Invalid arguments: {e}"));
            }
        };
        if args.query.trim().is_empty() {
            return ToolResult::error("lcm_recall", "query is required");
        }
        let limit = args.limit.unwrap_or(5).clamp(1, 50);
        let session = args.session.as_deref();
        match self.engine.recall(session, &args.query, limit).await {
            Ok(hits) => {
                if hits.is_empty() {
                    return ToolResult::success(
                        "lcm_recall",
                        json!({
                            "_lcm_tool": "lcm_recall",
                            "hits": [],
                            "note": "no matching nodes in the DAG",
                        }),
                    );
                }
                // Render at most MAX_RENDERED_HITS so the whole result stays
                // well under the agent's tool-result truncation cap (4096
                // bytes) even when the caller requests a large limit.
                // Full-length content previously overflowed the cap and hid
                // the later (often most relevant) hits from the model.
                const MAX_RENDERED_HITS: usize = 5;
                let rendered = hits.iter().take(MAX_RENDERED_HITS);
                let nodes: Vec<Value> = rendered
                    .map(|h| {
                        let mut content = h.content.clone();
                        if content.chars().count() > 600 {
                            content = content.chars().take(600).collect::<String>();
                            content.push_str("...[truncated]");
                        }
                        json!({
                            "node_id": h.node_id,
                            "role": h.role,
                            "content": content,
                            "created_at": h.created_at,
                            "score": h.score,
                        })
                    })
                    .collect();
                ToolResult::success(
                    "lcm_recall",
                    json!({
                        "_lcm_tool": "lcm_recall",
                        "hits": nodes,
                        "total_hits": hits.len(),
                    }),
                )
            }
            Err(e) => ToolResult::error("lcm_recall", format!("recall failed: {e}")),
        }
    }
}

/// Diagnostics for the lossless context engine.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmStatsArgs {}

pub struct LcmStatsTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmStatsTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmStatsTool {
    fn name(&self) -> &str {
        "lcm_stats"
    }

    fn description(&self) -> &str {
        "Report the lossless context engine status: DAG database path, node count, and the fresh-tail (D0) token budget. Use to confirm the context engine is active and to gauge stored history size."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmStatsArgs>("lcm_stats", "LCM engine status")
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        let db_path = self.engine.db_path().display().to_string();
        let nodes = self.engine.node_count_global().unwrap_or(0);
        ToolResult::success(
            "lcm_stats",
            json!({
                "_lcm_tool": "lcm_stats",
                "engine": "lcm",
                "db_path": db_path,
                "dag_nodes": nodes,
                "tail_tokens": self.engine.tail_tokens(),
            }),
        )
    }
}

/// Register the LCM tools into a registry. Only meaningful when the LCM
/// engine is actually attached (config `agent.context_engine = "lcm"`).
pub async fn register_lcm_tools(
    registry: &crate::tools::ToolRegistry,
    engine: Arc<LcmContextEngine>,
) -> crate::error::Result<()> {
    registry
        .register(LcmRecallTool::new(engine.clone()))
        .await?;
    registry.register(LcmStatsTool::new(engine)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_engine() -> Arc<LcmContextEngine> {
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_tools_test_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: dir.join("lcm-tools-test.db"),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
        })
        .unwrap();
        Arc::new(engine)
    }

    #[tokio::test]
    async fn lcm_recall_returns_verbatim_hits() {
        let engine = test_engine();
        let tool = LcmRecallTool::new(engine.clone());

        // Seed the DAG.
        let turn = vec![
            crate::client::Message::user("the deploy pipeline uses rsync over ssh"),
            crate::client::Message::assistant("rsync is configured in ci.yml"),
        ];
        engine.ingest_turn("test-session", &turn).await.unwrap();

        // Happy path: hit returned verbatim, scoped to the session.
        let out = tool
            .execute(
                serde_json::json!({ "query": "deploy pipeline", "session": "test-session" }),
                ToolContext::default(),
            )
            .await;
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        let hits = parsed["hits"].as_array().unwrap();
        assert!(!hits.is_empty(), "recall should find the seeded node");
        assert!(
            hits.iter()
                .any(|h| h["content"]
                    == serde_json::json!("the deploy pipeline uses rsync over ssh")),
            "hits must be verbatim"
        );
    }

    #[tokio::test]
    async fn lcm_recall_rejects_empty_query() {
        let tool = LcmRecallTool::new(test_engine());
        let out = tool
            .execute(serde_json::json!({ "query": "" }), ToolContext::default())
            .await;
        assert!(!out.success, "empty query must error");
    }
    #[tokio::test]
    async fn lcm_stats_reports_engine_state() {
        let engine = test_engine();
        let tool = LcmStatsTool::new(engine.clone());
        let out = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(parsed["engine"], serde_json::json!("lcm"));
        assert_eq!(
            parsed["tail_tokens"],
            serde_json::json!(engine.tail_tokens())
        );
        assert!(
            parsed["db_path"]
                .as_str()
                .unwrap()
                .contains("lcm-tools-test")
        );
    }

    #[tokio::test]
    async fn cross_connection_engine_sees_committed_nodes() {
        // Production scenario: the agent-attached engine writes (ingest during
        // assemble/eager turn-end) while the registry's tool engine reads
        // (recall) — two LcmContextEngine instances sharing one WAL file.
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_cross_conn_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("lcm-cross.db");

        let writer = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
        })
        .unwrap();
        let reader = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
        })
        .unwrap();

        let turn = vec![crate::client::Message::user(
            "cross-connection freshness marker",
        )];
        writer.ingest_turn("s1", &turn).await.unwrap();

        let hits = reader
            .recall(Some("s1"), "freshness marker", 5)
            .await
            .unwrap();
        assert!(
            hits.iter().any(|h| h.content.contains("cross-connection")),
            "second engine instance must see nodes written by the first (WAL)"
        );
    }
}
