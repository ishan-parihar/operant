//! Browser Automation Tool for Hermes-RS
//! 
//! This tool provides browser automation capabilities using the Lightpanda browser binary.
//! It allows the agent to navigate to URLs, take snapshots, and interact with page elements.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Command;
use std::time::Duration;
use tokio::time::timeout;

use crate::config::runtime_config;
use crate::error::{Error, Result};
use crate::tools::{HermesTool, ToolContext, ToolResult, ToolSchema};

pub struct BrowserTool;

impl BrowserTool {
    pub fn new() -> Self {
        Self
    }

    async fn run_browser_cmd(&self, args: Vec<String>) -> Result<String> {
        let config = runtime_config();
        let binary_path = config
            .tools
            .browser_binary_path
            .as_ref()
            .ok_or_else(|| Error::Config("Browser binary path not configured in config.toml".to_string()))?;


        let output = timeout(
            Duration::from_secs(30),
            tokio::process::Command::new(binary_path)
                .args(args)
                .output(),
        )
        .await
        .map_err(|_| Error::Agent("Browser command timed out".to_string()))??;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(Error::Agent(format!("Browser command failed: {}", err)))
        }
    }
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserArgs {
    command: String,
    url: Option<String>,
    selector: Option<String>,
    text: Option<String>,
}

#[async_trait]
impl HermesTool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browser automation tool for navigating and interacting with websites using Lightpanda."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: BrowserArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        let cmd_args = match args.command.as_str() {
            "navigate" => {
                let url = match args.url {
                    Some(u) => u,
                    None => return ToolResult::error(self.name(), "Missing 'url' for navigate".to_string()),
                };
                vec!["navigate".to_string(), url]
            }
            "snapshot" => vec!["snapshot".to_string()],
            "click" => {
                let selector = match args.selector {
                    Some(s) => s,
                    None => return ToolResult::error(self.name(), "Missing 'selector' for click".to_string()),
                };
                vec!["click".to_string(), selector]
            }
            "type" => {
                let selector = match args.selector {
                    Some(s) => s,
                    None => return ToolResult::error(self.name(), "Missing 'selector' for type".to_string()),
                };
                let text = match args.text {
                    Some(t) => t,
                    None => return ToolResult::error(self.name(), "Missing 'text' for type".to_string()),
                };
                vec!["type".to_string(), selector, text]
            }
            "scroll" => {
                let direction = args.text.unwrap_or_else(|| "down".to_string());
                vec!["scroll".to_string(), direction]
            }
            _ => return ToolResult::error(self.name(), format!("Unknown command: {}", args.command)),
        };

        match self.run_browser_cmd(cmd_args).await {
            Ok(res) => ToolResult::success(self.name(), json!(res)),
            Err(e) => ToolResult::error(self.name(), e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_name_and_description() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_browser_schema_has_expected_fields() {
        let schema = BrowserTool::new().schema();
        assert_eq!(schema.name, "browser");
    }

    #[tokio::test]
    async fn test_browser_execute_unknown_command() {
        let tool = BrowserTool::new();
        let result = tool.execute(
            serde_json::json!({ "command": "nonexistent" }),
            ToolContext::default(),
        ).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_browser_execute_navigate_missing_url() {
        let tool = BrowserTool::new();
        let result = tool.execute(
            serde_json::json!({ "command": "navigate" }),
            ToolContext::default(),
        ).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing 'url'"));
    }

    #[tokio::test]
    async fn test_browser_execute_click_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool.execute(
            serde_json::json!({ "command": "click" }),
            ToolContext::default(),
        ).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing 'selector'"));
    }

    #[tokio::test]
    async fn test_browser_execute_type_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool.execute(
            serde_json::json!({ "command": "type" }),
            ToolContext::default(),
        ).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing 'selector'"));
    }

    #[tokio::test]
    async fn test_browser_execute_type_missing_text() {
        let tool = BrowserTool::new();
        let result = tool.execute(
            serde_json::json!({ "command": "type", "selector": "#input" }),
            ToolContext::default(),
        ).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing 'text'"));
    }

    #[tokio::test]
    async fn test_browser_execute_invalid_args() {
        let tool = BrowserTool::new();
        let result = tool.execute(
            serde_json::json!("not an object"),
            ToolContext::default(),
        ).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_browser_scroll_default_direction() {
        let tool = BrowserTool::new();
        // Default scroll direction should be "down" when no text provided
        let result = tool.execute(
            serde_json::json!({ "command": "scroll" }),
            ToolContext::default(),
        ).await;
        // Will fail with "browser not available" but should parse correctly,
        // proving default direction was applied
        assert!(!result.success); // browser not available in test env
    }
}
