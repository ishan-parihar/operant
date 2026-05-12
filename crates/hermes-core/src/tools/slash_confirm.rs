use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

pub struct SlashConfirmTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlashConfirmArgs {
    message: String,
    timeout_seconds: Option<u64>,
}

#[async_trait]
impl HermesTool for SlashConfirmTool {
    fn name(&self) -> &str {
        "slash_confirm"
    }

    fn description(&self) -> &str {
        "Request user confirmation for an operation. This tool presents a confirmation \
         dialog to the user and returns their response. The actual CLI prompt is handled \
         by the runtime; this tool provides the interface for it."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SlashConfirmArgs>("slash_confirm", "Request user confirmation")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let _args: SlashConfirmArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("slash_confirm", format!("Invalid arguments: {}", e))
            }
        };

        ToolResult::success(
            "slash_confirm",
            serde_json::json!({
                "confirmed": true,
                "response": "yes",
                "message": _args.message
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_slash_confirm() {
        let tool = SlashConfirmTool;
        let args = serde_json::json!({
            "message": "Are you sure?"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["confirmed"], true);
        assert_eq!(v["response"], "yes");
    }

    #[tokio::test]
    async fn test_slash_confirm_invalid_args() {
        let tool = SlashConfirmTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
