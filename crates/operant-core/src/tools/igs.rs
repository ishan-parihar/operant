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
use crate::security::ssrf_verdict;
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
            tokio::process::Command::new(&self.binary)
                .args(&full)
                .output(),
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
    let cli = IgsCli::new().ok_or_else(|| Error::Agent(IGS_INSTALL_HINT.to_string()))?;
    cli.run_extract(
        &["web", "scrape", "--url", url],
        &["markdown", "content", "text"],
    )
    .await
}

/// Parse an `igs web search --format json` response into structured results.
/// Defensive against shape drift: accepts `results` / `memories` / `data`
/// arrays, and individual items with `title`/`url` + `content`/`snippet`/`text`.
pub fn parse_search_results(
    value: &Value,
    limit: usize,
) -> Vec<crate::tools::web_providers::WebSearchResult> {
    use crate::tools::web_providers::WebSearchResult;

    let items = value
        .get("results")
        .or_else(|| value.get("memories"))
        .or_else(|| value.get("data"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(items.len().min(limit));
    for item in items.iter().take(limit) {
        let title = item
            .get("title")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let url = item
            .get("url")
            .or_else(|| item.get("link"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let snippet = item
            .get("content")
            .or_else(|| item.get("snippet"))
            .or_else(|| item.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !title.trim().is_empty() || !url.trim().is_empty() {
            out.push(WebSearchResult {
                title,
                url,
                snippet,
            });
        }
    }
    out
}

/// Search the web via `igs web search` (structured results).
///
/// Note: IGS >= 1.0 `web search` is key-free (multi-engine: DDG, Wikipedia,
/// GitHub, HN, StackOverflow, YouTube). Callers (e.g. `WebSearchTool`) still
/// fall back to DuckDuckGo when this returns an empty vec or an error.
pub async fn web_search_igs(
    query: &str,
    limit: usize,
) -> Result<Vec<crate::tools::web_providers::WebSearchResult>> {
    let cli = IgsCli::new().ok_or_else(|| Error::Agent(IGS_INSTALL_HINT.to_string()))?;
    let raw = cli
        .run_json(&[
            "web",
            "search",
            "--query",
            query,
            "--max-results",
            &limit.to_string(),
        ])
        .await?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| Error::Agent(format!("igs web search returned non-JSON output: {e}")))?;
    Ok(parse_search_results(&value, limit))
}

/// Run a `igs browser` subcommand (goto, markdown, click, fill, scroll, ...).
pub async fn browser_command(args: &[&str]) -> Result<String> {
    let cli = IgsCli::new().ok_or_else(|| Error::Agent(IGS_INSTALL_HINT.to_string()))?;
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
        // SSRF protection: block private/internal addresses (cloud metadata,
        // localhost, RFC 1918, CGNAT, metadata hostnames) before handing the
        // URL to the IGS engine. Fail-closed on DNS errors.
        let (safe, block_msg) = ssrf_verdict(&args.url).await;
        if !safe {
            return ToolResult::error("web_scrape", block_msg);
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
        // SSRF protection: block private/internal addresses (cloud metadata,
        // localhost, RFC 1918, CGNAT, metadata hostnames). Fail-closed on DNS errors.
        let (safe, block_msg) = ssrf_verdict(&args.url).await;
        if !safe {
            return ToolResult::error("web_extract", block_msg);
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
// web_crawl tool
// ---------------------------------------------------------------------------

/// Crawl a site via `igs web crawl` (Obscura engine, markdown pages).
///
/// IGS >= 1.0 fixed `web crawl` (0.5.x 404'd on its obscura auto-update).
pub struct WebCrawlTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebCrawlArgs {
    url: String,
    /// Maximum link depth (default 2).
    #[serde(default = "default_crawl_depth")]
    max_depth: u32,
    /// Maximum pages to crawl (default 20).
    #[serde(default = "default_crawl_pages")]
    max_pages: u32,
}

fn default_crawl_depth() -> u32 {
    2
}
fn default_crawl_pages() -> u32 {
    20
}

#[async_trait]
impl OperantTool for WebCrawlTool {
    fn name(&self) -> &str {
        "web_crawl"
    }

    fn description(&self) -> &str {
        "Crawl a website starting from a URL, following internal links up to a \
         max depth and page count, returning each page as markdown. Uses the \
         IGS Obscura engine (JS rendering). Set maxDepth/maxPages to bound the \
         crawl. Requires the 'igs' binary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebCrawlArgs>("web_crawl", "Crawl website to markdown")
    }

    fn is_available(&self) -> bool {
        find_igs_binary().is_some()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: WebCrawlArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("web_crawl", format!("Invalid arguments: {e}")),
        };
        if args.url.trim().is_empty() {
            return ToolResult::error("web_crawl", "url is required");
        }
        // SSRF protection — same fail-closed guard as web_scrape/web_extract.
        let (safe, block_msg) = ssrf_verdict(&args.url).await;
        if !safe {
            return ToolResult::error("web_crawl", block_msg);
        }

        let cli = match IgsCli::new() {
            Some(c) => c,
            None => return ToolResult::error("web_crawl", IGS_INSTALL_HINT.to_string()),
        };
        let raw = match cli
            .run_json(&[
                "web",
                "crawl",
                "--url",
                &args.url,
                "--max-depth",
                &args.max_depth.to_string(),
                "--max-pages",
                &args.max_pages.to_string(),
            ])
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::error("web_crawl", e.to_string()),
        };

        match serde_json::from_str::<Value>(&raw) {
            Ok(value) => {
                // Surface the structured crawl result: pages as markdown list.
                let pages = value
                    .get("pages")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                ToolResult::success(
                    "web_crawl",
                    serde_json::json!({
                        "start_url": args.url,
                        "page_count": pages.len(),
                        "pages": pages.iter().map(|p| serde_json::json!({
                            "url": p.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                            "title": p.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                            "markdown": p.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                        })).collect::<Vec<_>>(),
                    }),
                )
            }
            Err(_) => ToolResult::success("web_crawl", serde_json::json!({"raw": raw})),
        }
    }
}

// ---------------------------------------------------------------------------
// IGS browser provider
// ---------------------------------------------------------------------------

/// Browser provider backed by the `igs` binary (Obscura headless browser).
///
/// IMPORTANT (IGS >= 1.0): the `igs browser` CLI subcommands are **stateless**
/// across invocations — every `igs browser <cmd>` spawns a fresh about:blank
/// session, and the `markdown` subcommand is broken (`--dump markdown` is not
/// a valid value). The reliable, tested path is `igs web scrape`, which uses
/// the same shared Obscura engine and returns clean markdown. This provider
/// therefore routes `navigate`/`snapshot` through `web scrape`, tracking the
/// last URL so a snapshot can re-scrape it. Interactive actions (click/fill/
/// scroll) are not supported over the stateless CLI — they return a clear
/// error directing the user to the `obscura` (CDP) provider.
pub struct IgsBrowserProvider {
    last_url: std::sync::Mutex<Option<String>>,
}

impl Default for IgsBrowserProvider {
    fn default() -> Self {
        Self {
            last_url: std::sync::Mutex::new(None),
        }
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
        let markdown = scrape_url(url).await?;
        if let Ok(mut last) = self.last_url.lock() {
            *last = Some(url.to_string());
        }
        Ok(markdown)
    }

    async fn snapshot(&self) -> Result<String> {
        let last = self.last_url.lock().ok().and_then(|g| g.clone());
        match last {
            Some(url) => scrape_url(&url).await,
            None => Err(Error::Agent(
                "No page loaded: call navigate(url) first. \
                 (The IGS browser CLI is stateless across invocations, so the \
                 snapshot re-scrapes the last navigated URL.)"
                    .to_string(),
            )),
        }
    }

    async fn click(&self, _selector: &str) -> Result<String> {
        Err(Error::Agent(
            "The IGS browser CLI is stateless (IGS >= 1.0) — click/fill/scroll \
             are not supported over the CLI. Set browser.provider = \"obscura\" \
             in operant.toml to use the CDP-driven provider for interactive \
             automation."
                .to_string(),
        ))
    }

    async fn type_text(&self, _selector: &str, _text: &str) -> Result<String> {
        Err(Error::Agent(
            "The IGS browser CLI is stateless (IGS >= 1.0) — click/fill/scroll \
             are not supported over the CLI. Set browser.provider = \"obscura\" \
             in operant.toml to use the CDP-driven provider for interactive \
             automation."
                .to_string(),
        ))
    }

    async fn scroll(&self, _direction: &str) -> Result<String> {
        Err(Error::Agent(
            "The IGS browser CLI is stateless (IGS >= 1.0) — click/fill/scroll \
             are not supported over the CLI. Set browser.provider = \"obscura\" \
             in operant.toml to use the CDP-driven provider for interactive \
             automation."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_web_scrape_blocks_cloud_metadata() {
        // The SSRF guard fires before scrape_url touches the igs binary, so
        // this works even when igs is not installed.
        let result = WebScrapeTool
            .execute(
                json!({"url": "http://169.254.169.254/latest/meta-data/"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("SSRF"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_web_extract_blocks_cloud_metadata() {
        let result = WebExtractTool
            .execute(
                json!({"url": "http://169.254.169.254/latest/meta-data/"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("SSRF"), "unexpected error: {err}");
    }

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
    fn parse_search_results_handles_igs_shape() {
        let value = serde_json::json!({
            "results": [
                {"title": "Tokio docs", "url": "https://tokio.rs", "content": "async runtime"},
                {"name": "Only title", "url": "https://example.org"},
                {"title": "", "url": "", "content": "skipped (no id)"}
            ],
            "count": 3
        });
        let results = parse_search_results(&value, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tokio docs");
        assert_eq!(results[0].url, "https://tokio.rs");
        assert_eq!(results[0].snippet, "async runtime");
        assert_eq!(results[1].title, "Only title");
    }

    #[test]
    fn parse_search_results_limits_and_handles_empty() {
        let value = serde_json::json!({ "results": [{"title": "a", "url": "u"}, {"title": "b", "url": "u"}] });
        assert_eq!(parse_search_results(&value, 1).len(), 1);
        assert!(parse_search_results(&serde_json::json!({ "results": [] }), 5).is_empty());
        assert!(
            parse_search_results(
                &serde_json::json!({ "data": [{"title": "x", "url": "y"}] }),
                5
            )
            .len()
                == 1
        );
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
