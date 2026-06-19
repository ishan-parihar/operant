use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct BrowserCdpTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BrowserCdpArgs {
    method: String,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
}

#[async_trait]
impl OperantTool for BrowserCdpTool {
    fn name(&self) -> &str {
        "browser_cdp"
    }

    fn description(&self) -> &str {
        "Send raw Chrome DevTools Protocol commands directly to the browser via WebSocket."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserCdpArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let cdp_url = match std::env::var("BROWSER_CDP_URL") {
            Ok(url) => url,
            Err(_) => {
                return ToolResult::error(self.name(), "BROWSER_CDP_URL not set");
            }
        };

        let parsed: BrowserCdpArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        let target_ws_url = if let Some(ref target_id) = parsed.target_id {
            match resolve_target_ws_url(&cdp_url, target_id).await {
                Ok(url) => url,
                Err(e) => {
                    return ToolResult::error(
                        self.name(),
                        format!("Target resolution failed: {}", e),
                    )
                }
            }
        } else {
            cdp_url.clone()
        };

        let cmd_id = 1u64;
        let command = json!({
            "id": cmd_id,
            "method": parsed.method,
            "params": parsed.params.unwrap_or(json!({})),
        });

        match super::cdp_utils::send_cdp_command(&target_ws_url, &command).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), format!("CDP error: {}", e)),
        }
    }
}

async fn resolve_target_ws_url(cdp_url: &str, target_id: &str) -> Result<String, String> {
    let list_url = cdp_url
        .replace("/devtools/browser/", "/json/")
        .replace("ws://", "http://")
        .replace("wss://", "https://");

    let client = reqwest::Client::new();
    let targets: Vec<Value> = client
        .get(&list_url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    for target in targets.iter() {
        let id = target.get("id").and_then(|v: &Value| v.as_str());
        if id == Some(target_id) {
            return target
                .get("webSocketDebuggerUrl")
                .and_then(|v: &Value| v.as_str())
                .map(|s: &str| s.to_string())
                .ok_or_else(|| format!("Target '{}' has no webSocketDebuggerUrl", target_id));
        }
    }

    Err(format!(
        "Target '{}' not found in browser targets",
        target_id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    #[tokio::test]
    async fn test_browser_cdp_schema() {
        let tool = BrowserCdpTool;
        assert_eq!(tool.name(), "browser_cdp");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "browser_cdp");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_browser_cdp_missing_env() {
        let saved = std::env::var("BROWSER_CDP_URL").ok();
        std::env::remove_var("BROWSER_CDP_URL");

        let tool = BrowserCdpTool;
        let result = tool
            .execute(
                json!({"method": "Target.getTargets"}),
                ToolContext::default(),
            )
            .await;

        if let Some(url) = saved {
            std::env::set_var("BROWSER_CDP_URL", url);
        }
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_browser_cdp_invalid_args() {
        let tool = BrowserCdpTool;
        let result = tool
            .execute(json!("not_an_object"), ToolContext::default())
            .await;

        assert!(!result.success);
    }
}
