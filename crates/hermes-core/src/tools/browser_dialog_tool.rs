use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use schemars::JsonSchema;

use crate::tools::{HermesTool, ToolContext, ToolResult};
use crate::schema::ToolSchema;

pub struct BrowserDialogTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BrowserDialogArgs {
    action: DialogAction,
    #[serde(default)]
    prompt_text: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum DialogAction {
    Accept,
    Dismiss,
}

#[async_trait]
impl HermesTool for BrowserDialogTool {
    fn name(&self) -> &str {
        "browser_dialog"
    }

    fn description(&self) -> &str {
        "Respond to JavaScript dialog boxes (alert, confirm, prompt) in the browser."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserDialogArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let cdp_url = match std::env::var("BROWSER_CDP_URL") {
            Ok(url) => url,
            Err(_) => {
                return ToolResult::error(self.name(), "BROWSER_CDP_URL environment variable not set.");
            }
        };

        let parsed: BrowserDialogArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        let accept = match parsed.action {
            DialogAction::Accept => true,
            DialogAction::Dismiss => false,
        };

        let mut cdp_params = serde_json::json!({ "accept": accept });
        if let Some(text) = parsed.prompt_text {
            cdp_params["promptText"] = serde_json::Value::String(text);
        }

        let cdp_command = serde_json::json!({
            "id": 1,
            "method": "Page.handleJavaScriptDialog",
            "params": cdp_params,
        });

        match super::cdp_utils::send_cdp_command(&cdp_url, &cdp_command).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), format!("CDP error: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use serde_json::json;

    #[tokio::test]
    async fn test_browser_dialog_schema() {
        let tool = BrowserDialogTool;
        assert_eq!(tool.name(), "browser_dialog");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "browser_dialog");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_browser_dialog_missing_env() {
        let saved = std::env::var("BROWSER_CDP_URL").ok();
        std::env::remove_var("BROWSER_CDP_URL");

        let tool = BrowserDialogTool;
        let result = tool
            .execute(json!({"action": "accept"}), ToolContext::default())
            .await;

        if let Some(url) = saved {
            std::env::set_var("BROWSER_CDP_URL", url);
        }
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_browser_dialog_invalid_action() {
        let tool = BrowserDialogTool;
        let result = tool
            .execute(json!({"action": 123}), ToolContext::default())
            .await;

        assert!(!result.success);
    }
}
