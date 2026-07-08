use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

use std::collections::HashMap;
use std::sync::LazyLock;

static BACKEND_MAP: LazyLock<HashMap<&'static str, (&'static [&'static str], &'static str)>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("web_search", (&["tavily", "exa", "searxng", "ddg"][..], "tavily"));
    m.insert("code_execution", (&["local", "docker"][..], "local"));
    m.insert("terminal", (&["local", "docker", "ssh"][..], "local"));
    m.insert("vision", (&["openai", "anthropic"][..], "openai"));
    m
});

pub struct ToolBackendTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolBackendArgs {
    tool_name: String,
    backend: Option<String>,
}

#[async_trait]
impl OperantTool for ToolBackendTool {
    fn name(&self) -> &str {
        "tool_backend"
    }

    fn description(&self) -> &str {
        "Query and manage tool backends. Returns available backends for a given tool \
         and optionally switches the active backend. Known tools with backends include \
         web_search (tavily, exa, searxng, ddg), code_execution (local, docker), \
         terminal (local, docker, ssh), and vision (openai, anthropic)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ToolBackendArgs>("tool_backend", "Query and manage tool backends")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: ToolBackendArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("tool_backend", format!("Invalid arguments: {}", e))
            }
        };

        let tool_name = args.tool_name.to_lowercase();

        match BACKEND_MAP.get(tool_name.as_str()) {
            Some(&(backends, current)) => {
                let available: Vec<String> = backends.iter().map(|s| s.to_string()).collect();

                ToolResult::success(
                    "tool_backend",
                    serde_json::json!({
                        "tool": tool_name,
                        "available_backends": available,
                        "current_backend": current
                    }),
                )
            }
            None => ToolResult::success(
                "tool_backend",
                serde_json::json!({
                    "tool": tool_name,
                    "available_backends": [],
                    "current_backend": null
                }),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tool_backend_known() {
        let tool = ToolBackendTool;
        let args = serde_json::json!({
            "toolName": "web_search"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["available_backends"].as_array().unwrap().len(), 4);
        assert_eq!(v["current_backend"], "tavily");
    }

    #[tokio::test]
    async fn test_tool_backend_unknown() {
        let tool = ToolBackendTool;
        let args = serde_json::json!({
            "toolName": "unknown_tool"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["available_backends"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_tool_backend_invalid_args() {
        let tool = ToolBackendTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
