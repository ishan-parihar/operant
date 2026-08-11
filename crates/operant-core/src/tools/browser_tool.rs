//! Browser Automation Tool for Operant-RS
//!
//! This tool provides browser automation capabilities using multiple backend providers.
//! It allows the agent to navigate to URLs, take snapshots, and interact with page elements.
//!
//! Supported providers (configured via `browser.provider` in config.toml):
//! - `igs` (default) - IGS headless browser via the `igs` binary (Obscura)
//! - `lightpanda` - Local Lightpanda binary (auto-downloaded from GitHub Releases)
//! - `obscura` - Local Obscura binary (auto-downloaded). Full interactive
//!   automation over CDP: operant spawns the shared binary in `serve` mode
//!   (stealth by default) and drives `Page.navigate` / `Runtime.evaluate` /
//!   `LP.getMarkdown` over a WebSocket, so navigate/snapshot/click/type/scroll
//!   all work — same binary IGS web tools use.
//! - `camofox` - Camofox REST API (`CAMOFOX_URL`)
//! - `browserbase` - Browserbase cloud (`BROWSERBASE_API_KEY`)
//! - `browser-use` - Browser Use cloud (`BROWSER_USE_API_KEY`)
//! - `firecrawl` - Firecrawl scrape API (`FIRECRAWL_API_KEY`)
//!
//! ## Troubleshooting
//! If you see "Permission denied (os error 13)" or "binary not found" errors:
//! 1. Ensure you have internet access to download the binary from GitHub
//! 2. The binary is downloaded to `~/.operant/bin/browser` (Lightpanda) or
//!    `~/.operant/bin/obscura` (Obscura). When IGS is installed the `obscura`
//!    provider reuses IGS's managed copy (`~/.config/igs-mcp/bin/obscura`,
//!    or `$IGS_CONFIG_DIR/bin/obscura`) so browser + IGS web tools share one
//!    binary - check those paths if `browser.provider = "obscura"` fails.
//! 3. On Linux, you may need to install dependencies: `sudo apt-get install -y libnss3 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libasound2`

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::accessibility;
use crate::error::Result;
use crate::security::ssrf_verdict;
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

    /// Handle the cookie commands (`cookies_import` / `cookies_list` /
    /// `cookies_clear`) against the shared Obscura CDP session. Lets the
    /// agent import browser cookies (multi-browser) so authenticated
    /// sessions work without manual login.
    async fn handle_cookie_command(&self, args: &BrowserArgs) -> ToolResult {
        match args.command.as_str() {
            "cookies_list" => {
                let cookies = match crate::obscura_cdp::export_cookies().await {
                    Ok(c) => c,
                    Err(e) => return ToolResult::error(self.name(), format!("CDP error: {e}")),
                };
                return ToolResult::success(
                    self.name(),
                    serde_json::json!({
                        "cookie_count": cookies.len(),
                        "cookies": cookies.iter().map(|c| serde_json::json!({
                            "domain": c.domain,
                            "name": c.name,
                            "secure": c.secure,
                            "httpOnly": c.http_only,
                            "expires": c.expires,
                        })).collect::<Vec<_>>(),
                    }),
                );
            }
            "cookies_clear" => match crate::obscura_cdp::clear_cookies().await {
                Ok(()) => {
                    return ToolResult::success(
                        self.name(),
                        serde_json::json!({ "cleared": true }),
                    );
                }
                Err(e) => return ToolResult::error(self.name(), format!("CDP error: {e}")),
            },
            _ => {}
        }

        // cookies_import
        let source = match args.cookie_file.as_deref() {
            Some(s) => s,
            None => {
                return ToolResult::error(
                    self.name(),
                    "cookies_import requires 'cookie_file' (a cookies.txt/JSON file path or a browser name: chrome, brave, edge, firefox, ...)",
                );
            }
        };

        let path = std::path::Path::new(source);
        let cookies = if path.exists() {
            // File import — auto-detect cookies.txt vs JSON by first char.
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    let trimmed = text.trim_start();
                    if trimmed.starts_with('[') || trimmed.starts_with('{') {
                        crate::cookies::parse_json_cookies(&text)
                    } else {
                        crate::cookies::parse_netscape_cookies_txt(&text)
                    }
                }
                Err(e) => {
                    return ToolResult::error(self.name(), format!("Failed to read {source}: {e}"));
                }
            }
        } else {
            // Browser-name import.
            let found = match crate::cookies::find_browser_cookie_source(source) {
                Some(f) => f,
                None => {
                    return ToolResult::error(
                        self.name(),
                        format!(
                            "No cookie database found for browser '{source}'. Supported: chrome, chromium, brave, edge, vivaldi, opera, firefox"
                        ),
                    );
                }
            };
            let (label, db, local_state) = found;
            if label == "firefox" {
                crate::cookies::read_firefox_cookies(&db)
            } else {
                crate::cookies::read_chromium_cookies(&db, local_state.as_deref())
            }
        };

        if cookies.is_empty() {
            return ToolResult::error(
                self.name(),
                format!(
                    "No cookies parsed from '{source}'. For Chromium browsers with keyring encryption, export a cookies.txt and import that file instead."
                ),
            );
        }

        if args.dry_run.unwrap_or(false) {
            return ToolResult::success(
                self.name(),
                serde_json::json!({
                    "dry_run": true,
                    "cookie_count": cookies.len(),
                    "domains": cookies.iter().map(|c| c.domain.clone()).collect::<std::collections::HashSet<_>>(),
                }),
            );
        }

        match crate::obscura_cdp::import_cookies(&cookies).await {
            Ok(applied) => ToolResult::success(
                self.name(),
                serde_json::json!({
                    "applied": applied,
                    "total": cookies.len(),
                    "message": format!("Applied {applied}/{} cookies to the Obscura session", cookies.len()),
                }),
            ),
            Err(e) => ToolResult::error(self.name(), format!("CDP error: {e}")),
        }
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
    /// For `cookies_import`: path to a cookies.txt / JSON export file, or a
    /// browser name (chrome, brave, edge, firefox, …) to read directly.
    cookie_file: Option<String>,
    /// For `cookies_import` with a browser name: dry-run, don't apply.
    dry_run: Option<bool>,
}

#[async_trait]
impl OperantTool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browser automation tool for navigating and interacting with websites. \
         Supports multiple providers configured via browser.provider in config.toml:\n\
         - igs (default): IGS headless browser via igs binary (Obscura)\n\
         - lightpanda: Local Lightpanda binary (auto-downloaded)\n\
         - obscura: Local Obscura binary (CDP-driven interactive browser, stealth by default)\n\
         - camofox: Camofox REST API (CAMOFOX_URL)\n\
         - browserbase: Browserbase cloud (BROWSERBASE_API_KEY)\n\
         - browser-use: Browser Use cloud (BROWSER_USE_API_KEY)\n\
         - firecrawl: Firecrawl scrape API (FIRECRAWL_API_KEY)\n\
         Commands: navigate (url), snapshot, click (selector), type (selector, text),\n\
         scroll (text=direction), accessibility_tree. Use snapshot to read a page.\n\
         Cookie commands: cookies_import (cookie_file=<cookies.txt|JSON path> or <browser name>),\n\
         cookies_list, cookies_clear — import cookies from Chrome/Brave/Edge/Firefox so\n\
         authenticated accounts work without manual login.\n\
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
            "cookies_import" | "cookies_list" | "cookies_clear" => {
                return self.handle_cookie_command(&args).await;
            }
            _ => {
                return ToolResult::error(
                    self.name(),
                    format!("Unknown command: {}", args.command),
                );
            }
        }

        // SSRF protection for URL-fetching commands: `navigate` (and
        // `snapshot`, which reloads the current page URL) pass the URL to
        // local browser binaries (lightpanda/obscura/igs) that fetch it
        // directly. Without a guard, the agent could be prompted to
        // navigate to cloud metadata (169.254.169.254), localhost, or
        // internal services. Hermes guards every browser navigation with
        // tools/url_safety.is_safe_url (fail-closed).
        if matches!(args.command.as_str(), "navigate" | "snapshot")
            && let Some(url) = args.url.as_deref()
        {
            let (safe, block_msg) = ssrf_verdict(url).await;
            if !safe {
                return ToolResult::error(self.name(), block_msg);
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
    async fn test_browser_navigate_blocks_cloud_metadata() {
        // SSRF: navigate to the AWS/GCP metadata endpoint must be rejected
        // before any provider (local binary or cloud API) is contacted.
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "command": "navigate",
                    "url": "http://169.254.169.254/latest/meta-data/"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("SSRF"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_browser_navigate_blocks_loopback() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "command": "navigate", "url": "http://127.0.0.1:8080/admin" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("SSRF"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_browser_snapshot_blocks_metadata_hostname() {
        // snapshot re-navigates the current URL — the always-blocked metadata
        // hostname must be caught without DNS resolution.
        let tool = BrowserTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "command": "snapshot",
                    "url": "http://metadata.google.internal/computeMetadata/v1/"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("SSRF"), "unexpected error: {err}");
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
        // scroll must pass argument validation regardless of provider.
        // Success depends on environment: when the `igs` binary is installed
        // the scroll actually executes (and may succeed); otherwise the tool
        // degrades to a graceful "binary not found" error. Either way it must
        // never fail argument validation.
        if !result.success {
            let error = result.error.unwrap_or_default();
            assert!(
                !error.contains("Unknown command") && !error.contains("Missing"),
                "scroll failed argument validation instead of executing: {error}"
            );
        }
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
