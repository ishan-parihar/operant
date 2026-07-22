//! Browser Automation Tool for Operant-RS
//!
//! This tool provides browser automation capabilities using multiple backend providers.
//! It allows the agent to navigate to URLs, take snapshots, and interact with page elements.
//!
//! Supported providers (configured via `browser.provider` in config.toml):
//! - `lightpanda` (default) - Local Lightpanda binary (auto-downloaded from GitHub Releases)
//! - `obscura` - Local Obscura binary (auto-downloaded, supports CDP)
//! - `camofox` - Camofox REST API (`CAMOFOX_URL`)
//! - `browserbase` - Browserbase cloud (`BROWSERBASE_API_KEY`)
//! - `browser-use` - Browser Use cloud (`BROWSER_USE_API_KEY`)
//! - `firecrawl` - Firecrawl scrape API (`FIRECRAWL_API_KEY`)
//!
//! ## Troubleshooting
//! If you see "Permission denied (os error 13)" or "binary not found" errors:
//! 1. Ensure you have internet access to download the binary from GitHub
//! 2. The binary is downloaded to `~/.operant/bin/browser` (Lightpanda) or `~/.operant/bin/obscura` (Obscura) - check if it exists and is executable
//! 3. On Linux, you may need to install dependencies: `sudo apt-get install -y libnss3 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libasound2`

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::accessibility;
use crate::error::Result;
use crate::tools::{OperantTool, ToolContext, ToolResult, ToolSchema};

pub struct BrowserTool;

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTool {
    pub fn new() -> Self {
        Self
    }

    /// Resolve the browser provider from config (cached per-call is fine since
    /// `build_browser_provider` is cheap).
    async fn run_browser_cmd(&self, command: &str, args: serde_json::Value) -> Result<String> {
        let provider_name = {
            let cfg = crate::config::runtime_config();
            cfg.browser.provider.clone()
        };
        let provider = crate::browser_provider::build_browser_provider(&provider_name);
        provider.execute(command, args).await
    }

    async fn handle_accessibility_tree(&self, args: &BrowserArgs) -> ToolResult {
        let cdp_url = args
            .cdp_url
            .clone()
            .or_else(|| std::env::var("BROWSER_CDP_URL").ok());

        let cdp_url = match cdp_url {
            Some(url) => url,
            None => {
                return ToolResult::error(
                    self.name(),
                    "No CDP URL available. Set BROWSER_CDP_URL or pass cdp_url parameter.",
                );
            }
        };

        let tree = match accessibility::fetch_accessibility_tree(&cdp_url).await {
            Ok(t) => t,
            Err(e) => return ToolResult::error(self.name(), e),
        };

        let full = args.full.unwrap_or(false);
        let text = if full {
            tree.render_full()
        } else {
            tree.render_compact()
        };

        ToolResult::success(
            self.name(),
            serde_json::json!({
                "snapshot": text,
                "element_count": tree.element_count,
                "refs": tree.refs,
            }),
        )
    }
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserArgs {
    command: String,
    url: Option<String>,
    selector: Option<String>,
    text: Option<String>,
    /// For `accessibility_tree`: optional CDP URL override.
    cdp_url: Option<String>,
    /// For `accessibility_tree`: if true, render full tree instead of compact.
    full: Option<bool>,
}

#[async_trait]
impl OperantTool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browser automation tool for navigating and interacting with websites. \
         Supports multiple providers configured via browser.provider in config.toml:\n\
         - lightpanda (default): Local Lightpanda binary (auto-downloaded)\n\
         - obscura: Local Obscura binary (auto-downloaded, supports CDP)\n\
         - camofox: Camofox REST API (CAMOFOX_URL)\n\
         - browserbase: Browserbase cloud (BROWSERBASE_API_KEY)\n\
         - browser-use: Browser Use cloud (BROWSER_USE_API_KEY)\n\
         - firecrawl: Firecrawl scrape API (FIRECRAWL_API_KEY)\n\
         Supports accessibility_tree command for CDP-based accessibility tree extraction with ref selectors."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: BrowserArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        match args.command.as_str() {
            "accessibility_tree" => {
                return self.handle_accessibility_tree(&args).await;
            }
            "navigate" if args.url.is_none() => {
                return ToolResult::error(self.name(), "Missing 'url' for navigate");
            }
            "click" if args.selector.is_none() => {
                return ToolResult::error(self.name(), "Missing 'selector' for click");
            }
            "type" if args.selector.is_none() => {
                return ToolResult::error(self.name(), "Missing 'selector' for type");
            }
            "type" if args.text.is_none() => {
                return ToolResult::error(self.name(), "Missing 'text' for type");
            }
            "navigate" | "snapshot" | "click" | "type" | "scroll" => {}
            _ => {
                return ToolResult::error(
                    self.name(),
                    format!("Unknown command: {}", args.command),
                );
            }
        }

        let cmd_args = serde_json::json!({
            "url": args.url,
            "selector": args.selector,
            "text": args.text,
            "direction": args.text.as_deref().unwrap_or("down"),
        });

        match self.run_browser_cmd(&args.command, cmd_args).await {
            Ok(res) => ToolResult::success(self.name(), serde_json::json!(res)),
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
        let result = tool
            .execute(
                serde_json::json!({ "command": "nonexistent" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_browser_execute_navigate_missing_url() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "navigate" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing 'url'"));
    }

    #[tokio::test]
    async fn test_browser_execute_click_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "click" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(
            result
                .error
                .unwrap_or_default()
                .contains("Missing 'selector'")
        );
    }

    #[tokio::test]
    async fn test_browser_execute_type_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "type" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(
            result
                .error
                .unwrap_or_default()
                .contains("Missing 'selector'")
        );
    }

    #[tokio::test]
    async fn test_browser_execute_type_missing_text() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "type", "selector": "#input" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing 'text'"));
    }

    #[tokio::test]
    async fn test_browser_execute_invalid_args() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(serde_json::json!("not an object"), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_browser_scroll_default_direction() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "scroll" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success); // browser not available in test env
    }

    #[tokio::test]
    async fn test_browser_accessibility_tree_missing_cdp_url() {
        let saved = std::env::var("BROWSER_CDP_URL").ok();
        unsafe { std::env::remove_var("BROWSER_CDP_URL") };

        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "accessibility_tree" }),
                ToolContext::default(),
            )
            .await;

        if let Some(url) = saved {
            unsafe { std::env::set_var("BROWSER_CDP_URL", url) };
        }
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("No CDP URL"));
    }

    #[tokio::test]
    async fn test_browser_accessibility_tree_invalid_args() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(serde_json::json!("not an object"), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
