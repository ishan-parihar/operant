//! Insights tool — wires up `agent::insights::InsightsEngine` as an LLM-callable tool.
//!
//! Allows the agent to query session analytics on demand.
//! Ported from hermes-agent's `agent/insights.py` integration.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent::insights::InsightsEngine;
use crate::database::Database;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Arguments for the `session_insights` tool.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct InsightsArgs {
    /// Number of days to look back (default: 7).
    #[serde(default = "default_days")]
    pub days: u32,
    /// Optional source filter (e.g. "cli", "gateway", "tui").
    #[serde(default)]
    pub source: Option<String>,
}

fn default_days() -> u32 {
    7
}

/// Tool that provides session analytics and usage insights.
pub struct InsightsTool {
    database: std::sync::Arc<Database>,
}

impl InsightsTool {
    pub fn new(database: std::sync::Arc<Database>) -> Self {
        Self { database }
    }
}

#[async_trait]
impl OperantTool for InsightsTool {
    fn name(&self) -> &str {
        "session_insights"
    }

    fn description(&self) -> &str {
        "Query session analytics: token usage, cost estimates, tool usage patterns, \
         activity trends, model/platform breakdowns, and notable sessions."
    }

    fn schema(&self) -> crate::tools::ToolSchema {
        crate::tools::ToolSchema {
            name: "session_insights".to_string(),
            description: "Query session analytics: token usage, cost estimates, tool usage patterns, activity trends, model/platform breakdowns, and notable sessions.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "days": {
                        "type": "integer",
                        "description": "Number of days to look back (default: 7)",
                        "default": 7
                    },
                    "source": {
                        "type": "string",
                        "description": "Optional source filter (e.g. 'cli', 'gateway', 'tui')"
                    }
                },
                "required": []
            }),
            // A native tool: eagerly offered every turn (the trait default).
            exposure: crate::tools::ToolExposure::Direct,
        }
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: InsightsArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error("session_insights", format!("Invalid arguments: {}", e));
            }
        };

        let engine = InsightsEngine::new(&self.database);
        let report = engine.generate(parsed.days, parsed.source.as_deref());
        let formatted = engine.format_gateway(&report);

        ToolResult::success(
            "session_insights",
            json!({
                "formatted": formatted,
                "empty": report.empty,
                // The failure, when the query failed — so the model can tell
                // a broken database from a quiet window (RT-294).
                "error": report.error,
                "days": report.days,
                "total_sessions": report.overview.total_sessions,
                "total_tokens": report.overview.total_tokens,
                "total_cost_usd": report.overview.total_cost_usd,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RT-294: a FAILED insights query must not read as an empty report.
    ///
    /// The engine returned the plain empty report when the session query
    /// failed, so the tool answered `{"empty": true}` — and the model could
    /// not tell a broken database from a week with no activity.
    #[tokio::test]
    async fn a_failed_insights_query_is_not_reported_as_an_empty_report() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db = std::sync::Arc::new(
            crate::database::Database::init(dir.path().join("insights.db")).expect("db"),
        );
        // Break the query the report reads: the table is gone.
        {
            let conn = db.conn();
            conn.execute_batch("DROP TABLE sessions;").expect("drop");
        }
        let engine = InsightsEngine::new(&db);
        let report = engine.generate(7, None);
        assert!(
            report.error.is_some(),
            "the failure must be visible in the report: {report:?}"
        );
        // And the tool's own output carries it.
        let tool = InsightsTool::new(db.clone());
        let result = tool
            .execute(
                serde_json::json!({"days": 7}),
                crate::tools::ToolContext::default(),
            )
            .await;
        let payload: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert!(
            payload["error"].is_string(),
            "the tool's output must carry the failure: {payload}"
        );
    }

    #[test]
    fn test_schema_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let db = std::sync::Arc::new(Database::init(tmp.path().join("test_insights.db")).unwrap());
        let tool = InsightsTool::new(db);
        let schema = tool.schema();
        assert_eq!(schema.name, "session_insights");
        assert!(schema.description.contains("analytics"));
    }
}
