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
