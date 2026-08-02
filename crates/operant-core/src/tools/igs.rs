//! IGS (Intelligence Gathering System) integration.
//!
//! IGS is a Rust MCP server + CLI (https://github.com/ishan-parihar/igs-rust)
//! that provides web search, scraping, crawling, and headless-browser
//! automation with zero API keys (DuckDuckGo + Obscura). Operant talks to
//! it over the `igs` CLI with `--format json` output.
//!
//! Install: `curl -sSL https://raw.githubusercontent.com/ishan-parihar/igs-rust/master/scripts/install.sh | bash`
//!
//! Tools exposed:
//! - [`WebScrapeTool`] — `web_scrape`: scrape a URL to markdown
//! - [`WebExtractTool`] — `web_extract`: extract the main content of a URL
//!
//! All tools degrade to a helpful error when the `igs` binary is missing.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::runtime_config;
use crate::error::{Error, Result};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Default timeout for a single `igs` invocation.
const DEFAULT_IGS_TIMEOUT: Duration = Duration::from_secs(60);

/// Installation hint shown when the `igs` binary is missing.
pub const IGS_INSTALL_HINT: &str = "IGS binary not found. Install it with:\n  curl -sSL https://raw.githubusercontent.com/ishan-parihar/igs-rust/master/scripts/install.sh | bash\n(or add an `igs_binary_path` to [tools] in operant.toml).";

/// Resolve the `igs` binary path: config override first, then PATH.
pub fn find_igs_binary() -> Option<PathBuf> {
    if let Some(path) = runtime_config().tools.igs_binary_path.clone() {
        if path.exists() {
            return Some(path);
        }
        tracing::warn!(
            path = %path.display(),
            "configured igs_binary_path does not exist — falling back to PATH"
        );
    }
    which::which("igs").ok()
}

/// Lightweight wrapper around the `igs` CLI.
pub struct IgsCli {
    binary: PathBuf,
    timeout: Duration,
}

impl IgsCli {
    /// Create a CLI wrapper, resolving the binary from config/PATH.
    /// Returns `None` when `igs` is not installed.
    pub fn new() -> Option<Self> {
        let binary = find_igs_binary()?;
        let configured = runtime_config().tools.igs_timeout_secs;
        let secs = if configured > 0 {
            configured.clamp(5, 600)
        } else {
            DEFAULT_IGS_TIMEOUT.as_secs()
        };
        let timeout = Duration::from_secs(secs);
        Some(Self { binary, timeout })
    }

    /// Run `igs <args> --format json` and return the raw stdout.
    pub async fn run_json(&self, args: &[&str]) -> Result<String> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
        full.extend_from_slice(args);
        full.push("--format");
        full.push("json");

        let out = tokio::time::timeout(
            self.timeout,
            tokio::process::Command::new(&self.binary).args(&full).output(),
        )
        .await
        .map_err(|_| Error::Agent(format!("igs command timed out after {:?}", self.timeout)))?
        .map_err(|e| Error::Agent(format!("failed to spawn igs: {e}")))?;

        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if out.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(Error::Agent(format!(
                "igs {} failed: {}",
                args.join(" "),
                if stderr.trim().is_empty() {
                    stdout.chars().take(300).collect::<String>()
                } else {
                    stderr.chars().take(300).collect::<String>()
                }
            )))
        }
    }

    /// Run an igs command and try to extract a structured field from the
    /// JSON output. Falls back to the raw text when the output isn't JSON
    /// or the field is absent.
    pub async fn run_extract(&self, args: &[&str], fields: &[&str]) -> Result<String> {
        let raw = self.run_json(args).await?;
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            for field in fields {
                if let Some(text) = value
                    .get(*field)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    return Ok(text.to_string());
                }
            }
            // Some tools wrap output under data/content arrays — take the
            // first non-empty text field we can find.
            if let Some(text) = first_text_field(&value) {
                return Ok(text);
            }
            return Ok(raw);
        }
        Ok(raw)
    }
}

/// Recursively find the first non-empty string in a JSON value (breadth-ish
/// DFS over object values / array elements). Used for defensive parsing of
/// igs output shapes that aren't fully documented.
fn first_text_field(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(first_text_field),
        Value::Object(map) => map.values().find_map(first_text_field),
        _ => None,
    }
}

/// Scrape a URL to markdown via `igs web scrape`.
pub async fn scrape_url(url: &str) -> Result<String> {
    let cli = IgsCli::new()
        .ok_or_else(|| Error::Agent(IGS_INSTALL_HINT.to_string()))?;
    cli.run_extract(
        &["web", "scrape", "--url", url],
        &["markdown", "content", "text"],
    )
    .await
}

/// Search the web via `igs web search`.
pub async fn web_search_igs(query: &str, limit: usize) -> Result<String> {
    let cli = IgsCli::new()
        .ok_or_else(|| Error::Agent(IGS_INSTALL_HINT.to_string()))?;
    cli.run_extract(
        &[
            "web",
            "search",
            "--query",
            query,
            "--limit",
            &limit.to_string(),
        ],
        &["results"],
    )
    .await
}

/// Run a `igs browser` subcommand (goto, markdown, click, fill, scroll, ...).
pub async fn browser_command(args: &[&str]) -> Result<String> {
    let cli = IgsCli::new()
        .ok_or_else(|| Error::Agent(IGS_INSTALL_HINT.to_string()))?;
    cli.run_extract(args, &["markdown", "content", "text", "result"])
        .await
}

// ---------------------------------------------------------------------------
// web_scrape tool
// ---------------------------------------------------------------------------

/// Scrape a URL to clean markdown (JS-rendered when needed).
pub struct WebScrapeTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebScrapeArgs {
    url: String,
}

#[async_trait]
impl OperantTool for WebScrapeTool {
    fn name(&self) -> &str {
        "web_scrape"
    }

    fn description(&self) -> &str {
        "Scrape a URL to clean, readable markdown. Uses the IGS engine (JS rendering via Obscura when needed). Prefer this over web_fetch when you need page content, not raw HTML. Requires the 'igs' binary (see install hint on error)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebScrapeArgs>("web_scrape", "Scrape URL to markdown")
    }

    fn is_available(&self) -> bool {
        find_igs_binary().is_some()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: WebScrapeArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("web_scrape", format!("Invalid arguments: {e}")),
        };
        if args.url.trim().is_empty() {
            return ToolResult::error("web_scrape", "url is required");
        }
        match scrape_url(&args.url).await {
            Ok(markdown) => ToolResult::success(
                "web_scrape",
                serde_json::json!({
                    "url": args.url,
                    "markdown": markdown,
                }),
            ),
            Err(e) => ToolResult::error("web_scrape", e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// web_extract tool
// ---------------------------------------------------------------------------

/// Extract the main content of a URL (article body / primary text).
pub struct WebExtractTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebExtractArgs {
    url: String,
}

#[async_trait]
impl OperantTool for WebExtractTool {
    fn name(&self) -> &str {
        "web_extract"
    }

    fn description(&self) -> &str {
        "Extract the main content from a URL (article text, product info, etc.) as clean markdown — navigation, ads, and boilerplate removed. Backed by the IGS engine. Requires the 'igs' binary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebExtractArgs>("web_extract", "Extract main content from URL")
    }

    fn is_available(&self) -> bool {
        find_igs_binary().is_some()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: WebExtractArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("web_extract", format!("Invalid arguments: {e}")),
        };
        if args.url.trim().is_empty() {
            return ToolResult::error("web_extract", "url is required");
        }
        // igs web scrape already returns the cleaned main content as
        // markdown — the extract view is the same pipeline.
        match scrape_url(&args.url).await {
            Ok(content) => ToolResult::success(
                "web_extract",
                serde_json::json!({
                    "url": args.url,
                    "content": content,
                }),
            ),
            Err(e) => ToolResult::error("web_extract", e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// IGS browser provider
// ---------------------------------------------------------------------------

/// Browser provider backed by `igs browser` (Obscura headless browser).
///
/// Each command shells out to the `igs` binary; the browser session persists
/// server-side so goto → markdown → click sequences work across calls.
pub struct IgsBrowserProvider;

impl Default for IgsBrowserProvider {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl crate::browser_provider::BrowserProvider for IgsBrowserProvider {
    fn name(&self) -> &str {
        "igs"
    }

    fn is_configured(&self) -> bool {
        find_igs_binary().is_some()
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        // goto, then dump the page as markdown.
        browser_command(&["browser", "goto", "--url", url]).await?;
        browser_command(&["browser", "markdown"]).await
    }

    async fn snapshot(&self) -> Result<String> {
        browser_command(&["browser", "markdown"]).await
    }

    async fn click(&self, selector: &str) -> Result<String> {
        browser_command(&["browser", "click", "--selector", selector]).await
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<String> {
        browser_command(&["browser", "fill", "--selector", selector, "--value", text]).await
    }

    async fn scroll(&self, direction: &str) -> Result<String> {
        browser_command(&["browser", "scroll", "--direction", direction]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_text_field_finds_first_string() {
        let value = serde_json::json!({
            "nested": { "a": [{"b": "hello world"}], "c": 42 }
        });
        assert_eq!(first_text_field(&value).as_deref(), Some("hello world"));
    }

    #[test]
    fn first_text_field_ignores_empty() {
        let value = serde_json::json!({ "x": "  ", "y": "real text" });
        assert_eq!(first_text_field(&value).as_deref(), Some("real text"));
    }

    #[test]
    fn first_text_field_none_on_numbers() {
        let value = serde_json::json!({ "x": [1, 2, 3] });
        assert_eq!(first_text_field(&value), None);
    }

    #[test]
    fn web_scrape_tool_schema_is_valid() {
        let schema = WebScrapeTool.schema();
        assert_eq!(schema.name, "web_scrape");
        assert!(!schema.description.is_empty());
    }

    #[tokio::test]
    async fn web_scrape_rejects_empty_url() {
        let result = WebScrapeTool
            .execute(serde_json::json!({ "url": "" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("url is required"));
    }

    #[tokio::test]
    async fn web_extract_rejects_empty_url() {
        let result = WebExtractTool
            .execute(serde_json::json!({ "url": "" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("url is required"));
    }
}
