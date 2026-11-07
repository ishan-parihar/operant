use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

static BACKEND_MAP: LazyLock<HashMap<&'static str, (&'static [&'static str], &'static str)>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "web_search",
            (
                // The only search engine left in the tree (plan 11C1):
                // `build_search_candidates` returns exactly one provider.
                // The four deleted engines were advertised here as
                // switchable long after they were gone (RT-302).
                &["sourcehound"][..],
                // ponytail: static claim of the config default
                // (tools.web.preferred_provider = "sourcehound", config.rs);
                // a live read of the per-session override adds config plumbing
                // this introspection tool has never needed.
                "sourcehound",
            ),
        );
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
}

#[async_trait]
impl OperantTool for ToolBackendTool {
    fn name(&self) -> &str {
        "tool_backend"
    }

    fn description(&self) -> &str {
        "Report which backends a tool supports (READ-ONLY — it does not switch \
         anything). Known tools with backends include web_search (sourcehound), \
         code_execution (local, docker), terminal (local, docker, ssh), and \
         vision (openai, anthropic)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ToolBackendArgs>(
            "tool_backend",
            "Report the backends a tool supports",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: ToolBackendArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("tool_backend", format!("Invalid arguments: {}", e));
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
        assert_eq!(
            v["available_backends"],
            serde_json::json!(["sourcehound"]),
            "only the engine that is still in the tree may be advertised"
        );
        assert_eq!(v["current_backend"], "sourcehound");
    }

    /// RT-302: the tool must not advertise a switch it does not perform.
    ///
    /// The description said it "optionally switches the active backend" and
    /// the schema exposed a `backend` parameter — but the body never read
    /// it (there is no writer anywhere in the crate), and the map still
    /// advertised four search engines that plan 11C1 deleted from the tree.
    /// A model that read the schema was told it could scope a tool it
    /// could not.
    #[tokio::test]
    async fn tool_backend_is_query_only_and_lists_no_deleted_provider() {
        // The schema exposes no `backend` property.
        let schema = ToolSchema::from_type::<ToolBackendArgs>("tool_backend", "test");
        let json = serde_json::to_value(&schema).expect("schema json");
        let properties = json["parameters"]["properties"]
            .as_object()
            .expect("the schema's properties");
        assert!(
            !properties.contains_key("backend"),
            "the tool cannot switch a backend, so the schema must not offer one: {json}"
        );

        // The description does not promise a switch.
        assert!(
            !ToolBackendTool.description().contains("switches"),
            "the description must not promise a switch: {}",
            ToolBackendTool.description()
        );

        // The deleted engines are not advertised.
        let tool = ToolBackendTool;
        let result = tool
            .execute(
                serde_json::json!({"toolName": "web_search"}),
                ToolContext::default(),
            )
            .await;
        let payload: Value = serde_json::from_str(&result.content).expect("json");
        assert_eq!(
            payload["available_backends"],
            serde_json::json!(["sourcehound"]),
            "only the engine that is still in the tree may be advertised"
        );
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
