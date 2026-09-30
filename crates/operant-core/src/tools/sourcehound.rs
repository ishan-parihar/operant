//! sourcehound integration — web search / scrape / extract / crawl over MCP.
//!
//! [sourcehound](https://github.com/ishan-parihar/sourcehound) is a Rust
//! intelligence-gathering binary that serves an [rmcp] MCP server over stdio
//! (`sourcehound mcp`) alongside its CLI. operant already speaks the MCP
//! *client* protocol in [`crate::mcp`] (HTTP + stdio + SSE), so the
//! integration is a subprocess speaking MCP — not a cargo dependency on
//! their library (their `[patch.crates-io]` never reaches consumers, and the
//! library pulls an unconditional render stack).
//!
//! ## Resolution
//!
//! [`find_sourcehound_binary`] follows the `aft_bridge.rs` shape: the
//! `SOURCEHOUND_BINARY` env override, then `sourcehound` on PATH. A missing
//! binary is a documented install hint, not a silent fetch — but an
//! *installed* binary is kept current by
//! [`crate::tools::sourcehound_update`], which polls the release tags (via
//! the `gh` CLI, authenticated for the private repo) once per session and
//! replaces the binary in the background on a verified newer asset.
//!
//! ## Tool names
//!
//! Read from sourcehound's `src/tools/registry.rs` (`web` group) and
//! `src/server.rs` `#[tool(name = …)]` attributes — see [`TOOL_SEARCH`],
//! [`TOOL_SCRAPE`], [`TOOL_EXTRACT`], [`TOOL_CRAWL`].
//!
//! All tools degrade to a helpful error when the `sourcehound` binary is
//! missing: [`Sourcehound::is_available`] returns `false`, so the registry
//! never advertises them, and a direct call returns
//! [`SOURCEHOUND_INSTALL_HINT`].
//!
//! [rmcp]: https://modelcontextprotocol.io

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::mcp::McpStdioClient;
use crate::schema::ToolSchema;
use crate::security::ssrf_verdict;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Binary name looked up on `PATH`.
pub const BINARY_NAME: &str = "sourcehound";

/// Environment variable holding an explicit path to the sourcehound binary.
pub const BINARY_ENV: &str = "SOURCEHOUND_BINARY";

/// Subcommand that starts the stdio MCP server.
const MCP_SUBCOMMAND: &str = "mcp";

/// sourcehound MCP tool: multi-engine web search (sourcehound `web.search`).
pub const TOOL_SEARCH: &str = "web.search";

/// sourcehound MCP tool: scrape a URL to markdown (sourcehound `web.scrape`).
pub const TOOL_SCRAPE: &str = "web.scrape";

/// sourcehound MCP tool: extract main content from a URL (sourcehound
/// `web.extract`).
pub const TOOL_EXTRACT: &str = "web.extract";

/// sourcehound MCP tool: crawl a site to markdown pages (sourcehound
/// `web.crawl`).
pub const TOOL_CRAWL: &str = "web.crawl";

/// Installation hint surfaced when the sourcehound binary is missing.
pub const SOURCEHOUND_INSTALL_HINT: &str = "sourcehound binary not found. Install it with:\n  cargo install --git https://github.com/ishan-parihar/sourcehound sourcehound\n(or point SOURCEHOUND_BINARY at an existing binary).";

/// sourcehound only advertises the tool groups its settings allow-list names
/// (`tools.groups` in `~/.config/sourcehound-mcp/settings.yml`). A default
/// install that allow-lists only `cloakctl` therefore has no `web.*` tools at
/// all, and every web call fails with a bare "unknown tool" from the server.
pub const SOURCEHOUND_GROUP_HINT: &str = "sourcehound is running but the `web` tool group is not enabled. Add \"web\" to `tools.groups` in ~/.config/sourcehound-mcp/settings.yml (or drop the `groups` allow-list) and restart the sourcehound process.";

/// Resolve the sourcehound binary: `SOURCEHOUND_BINARY` override, then PATH.
///
/// A configured path that does not exist is skipped with a warning rather
/// than fatal — the same fallback `aft_bridge.rs` uses.
pub fn find_sourcehound_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(BINARY_ENV).map(PathBuf::from)
        && path.exists()
    {
        return Some(path);
    } else if std::env::var_os(BINARY_ENV).is_some() {
        tracing::warn!(
            env = BINARY_ENV,
            "configured sourcehound binary does not exist — falling back to PATH"
        );
    }
    which::which(BINARY_NAME).ok()
}

/// True when the sourcehound binary is installed (the gate the tool registry
/// uses to decide whether to advertise the web tools).
pub fn is_available() -> bool {
    find_sourcehound_binary().is_some()
}

/// Spawned-once stdio client for one sourcehound server.
type SharedClient = Arc<Mutex<Option<Arc<McpStdioClient>>>>;

/// A lazily-spawned sourcehound MCP server.
///
/// Construction is free — the child process is spawned (and the MCP
/// `initialize` handshake run) on the first [`Sourcehound::call`], and reused
/// for every call after that. Cloning is cheap: the state is shared.
#[derive(Clone)]
pub struct Sourcehound {
    command: String,
    args: Vec<String>,
    /// `Some` once the handshake succeeded. Held behind a mutex so two
    /// concurrent first-calls spawn exactly one child.
    client: SharedClient,
    /// Tool names the server advertised at connect time, or `None` when its
    /// `tools/list` came back empty or unreadable.
    advertised: Arc<Mutex<Option<HashSet<String>>>>,
}

impl Sourcehound {
    /// Build a handle for an explicit command + arguments. Used by the
    /// process-wide [`Sourcehound::global`] and by tests that point the same
    /// call path at a mock MCP server.
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
            client: Arc::new(Mutex::new(None)),
            advertised: Arc::new(Mutex::new(None)),
        }
    }

    /// The process-wide handle against the resolved binary. Resolution happens
    /// once; the subprocess spawns on first use.
    pub fn global() -> Self {
        static GLOBAL: OnceLock<Sourcehound> = OnceLock::new();
        GLOBAL
            .get_or_init(|| {
                let command = find_sourcehound_binary()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| BINARY_NAME.to_string());
                Self::new(command, vec![MCP_SUBCOMMAND.to_string()])
            })
            .clone()
    }

    /// True when a real sourcehound binary backs this handle. A handle built
    /// from an explicit command (tests) is always considered available.
    pub fn is_available(&self) -> bool {
        self.command != BINARY_NAME || is_available()
    }

    /// Call one sourcehound MCP tool and return its structured payload.
    ///
    /// `value` is whatever the tool declared as `structuredContent`; when the
    /// server omits it (older build, or a tool with no output schema) the
    /// first text content block is returned, parsed as JSON when possible.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<Value> {
        // First call per session kicks the background auto-update poll; the
        // current call always uses the installed binary, update or not.
        crate::tools::sourcehound_update::kick_update_check();
        let client = self.client().await?;
        // A sourcehound started with a `tools.groups` allow-list that omits
        // `web` answers every web call with a bare "unknown tool". Catch it
        // here so the caller gets the one-line settings fix instead.
        let missing_group = match self.advertised.lock().await.as_ref() {
            Some(advertised) => !advertised.is_empty() && !advertised.contains(tool),
            None => false,
        };
        if missing_group {
            return Err(Error::Agent(format!(
                "sourcehound does not advertise `{tool}`.\n{SOURCEHOUND_GROUP_HINT}"
            )));
        }
        let raw = client
            .call_tool(tool, arguments)
            .await
            .map_err(|e| Error::Agent(format!("sourcehound `{tool}` call failed: {e}")))?;
        unwrap_tool_result(tool, &raw)
    }

    /// Spawn (once) and return the connected stdio client.
    async fn client(&self) -> Result<Arc<McpStdioClient>> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref() {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(McpStdioClient::new(
            self.command.clone(),
            self.args.clone(),
            HashMap::new(),
        ));
        if let Err(e) = client.connect().await {
            return Err(Error::Agent(format!(
                "{SOURCEHOUND_INSTALL_HINT}\n(spawned `{}`: {e})",
                self.command
            )));
        }
        // `connect()` already ran tools/list and cached the definitions on the
        // client; mirror the names so `call` can tell "group disabled" from
        // "server errored".
        let advertised = client
            .get_tools()
            .await
            .iter()
            .map(|t| t.definition().name.clone())
            .collect();
        *self.advertised.lock().await = Some(advertised);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }
}

/// Unwrap an MCP `tools/call` result into the tool's payload.
///
/// sourcehound's `format_output` sets `structuredContent` to the serialized
/// output type, so that is the preferred read. A text-only reply is the
/// fallback: parsed as JSON when it parses, otherwise returned as a
/// `{"text": …}` envelope so the caller always gets an object.
fn unwrap_tool_result(tool: &str, raw: &Value) -> Result<Value> {
    if raw.get("isError").and_then(Value::as_bool).unwrap_or(false) {
        return Err(Error::Agent(format!(
            "sourcehound `{tool}` returned an error: {}",
            first_text_block(raw).unwrap_or_else(|| raw.to_string())
        )));
    }
    if let Some(structured) = raw.get("structuredContent")
        && !structured.is_null()
    {
        return Ok(structured.clone());
    }
    match first_text_block(raw) {
        Some(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(serde_json::json!({ "text": text })),
        },
        None => Ok(raw.clone()),
    }
}

/// First `content[i].text` of an MCP tool result.
fn first_text_block(raw: &Value) -> Option<String> {
    raw.get("content")?
        .as_array()?
        .iter()
        .find_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::to_string)
}

/// Read the first non-empty string among `fields` from a sourcehound payload.
fn text_field(value: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        value
            .get(*field)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    })
}

/// Recursively find the first non-empty string in a JSON value.
///
/// A defensive backstop for tool payloads whose text is nested under a
/// `content` / `data` array rather than a named field.
fn first_text_field(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(first_text_field),
        Value::Object(map) => map.values().find_map(first_text_field),
        _ => None,
    }
}

/// Parse a sourcehound `web.search` payload into
/// [`WebSearchResult`](crate::tools::web_providers::WebSearchResult)s.
///
/// sourcehound's `WebSearchOutput.results[]` carries `title`, `url` and
/// `content`; the `data` / `memories` aliases and the `snippet` / `text`
/// field names are accepted so a shape change degrades to fewer fields rather
/// than zero results.
pub fn parse_search_results(
    value: &Value,
    limit: usize,
) -> Vec<crate::tools::web_providers::WebSearchResult> {
    use crate::tools::web_providers::WebSearchResult;

    let items = value
        .get("results")
        .or_else(|| value.get("data"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(items.len().min(limit));
    for item in items.iter().take(limit) {
        let title = text_field(item, &["title", "name"]).unwrap_or_default();
        let url = text_field(item, &["url", "link"]).unwrap_or_default();
        let snippet = text_field(item, &["content", "snippet", "text"]).unwrap_or_default();
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

/// Search the web through sourcehound's `web.search` MCP tool.
///
/// Key-free: sourcehound aggregates DuckDuckGo, Wikipedia, GitHub,
/// HackerNews and StackOverflow. Callers (e.g. `WebSearchTool`) still fall
/// back to the next provider in the chain when this returns an empty vec.
pub async fn web_search_sourcehound(
    query: &str,
    limit: usize,
) -> Result<Vec<crate::tools::web_providers::WebSearchResult>> {
    web_search_via(&Sourcehound::global(), query, limit).await
}

/// [`web_search_sourcehound`] against an explicit handle.
///
/// The split exists so the search half of the re-home is testable on the same
/// MCP call path the provider chain uses — `Sourcehound::global()` resolves a
/// real binary, which a test cannot rely on. Keep this the only place that
/// builds the `web.search` argument object.
async fn web_search_via(
    sh: &Sourcehound,
    query: &str,
    limit: usize,
) -> Result<Vec<crate::tools::web_providers::WebSearchResult>> {
    let arguments = serde_json::json!({
        "query": query,
        "max_results": limit,
    });
    let payload = sh.call(TOOL_SEARCH, arguments).await?;
    Ok(parse_search_results(&payload, limit))
}

/// Scrape a URL to markdown through sourcehound's `web.scrape` MCP tool.
pub async fn scrape_url(url: &str) -> Result<String> {
    let arguments = serde_json::json!({ "url": url });
    let payload = Sourcehound::global().call(TOOL_SCRAPE, arguments).await?;
    text_field(&payload, &["markdown", "content", "text"])
        .or_else(|| first_text_field(&payload))
        .ok_or_else(|| {
            Error::Agent(format!(
                "sourcehound `{TOOL_SCRAPE}` returned no markdown for {url}"
            ))
        })
}

// ---------------------------------------------------------------------------
// web_scrape tool
// ---------------------------------------------------------------------------

/// Scrape a URL to clean markdown (JS-rendered when the engine needs it).
pub struct WebScrapeTool(Sourcehound);

impl WebScrapeTool {
    /// The registered instance, backed by the resolved sourcehound binary.
    pub fn global() -> Self {
        Self(Sourcehound::global())
    }
}

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
        "Scrape a URL to clean, readable markdown. Backed by the sourcehound engine (JS rendering when needed). Prefer this over web_fetch when you need page content, not raw HTML. Requires the 'sourcehound' binary (see install hint on error)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebScrapeArgs>("web_scrape", "Scrape URL to markdown")
    }

    fn is_available(&self) -> bool {
        self.0.is_available()
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
        // URL to the engine. Fail-closed on DNS errors.
        let (safe, block_msg) = ssrf_verdict(&args.url).await;
        if !safe {
            return ToolResult::error("web_scrape", block_msg);
        }
        match self
            .0
            .call(TOOL_SCRAPE, serde_json::json!({ "url": args.url }))
            .await
        {
            Ok(payload) => ToolResult::success("web_scrape", payload),
            Err(e) => ToolResult::error("web_scrape", e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// web_extract tool
// ---------------------------------------------------------------------------

/// Extract the main content of a URL (article body / primary text).
pub struct WebExtractTool(Sourcehound);

impl WebExtractTool {
    /// The registered instance, backed by the resolved sourcehound binary.
    pub fn global() -> Self {
        Self(Sourcehound::global())
    }
}

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
        "Extract the main content from a URL (article text, product info, etc.) as clean markdown — navigation, ads, and boilerplate removed. Backed by the sourcehound engine. Requires the 'sourcehound' binary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebExtractArgs>("web_extract", "Extract main content from URL")
    }

    fn is_available(&self) -> bool {
        self.0.is_available()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: WebExtractArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("web_extract", format!("Invalid arguments: {e}")),
        };
        if args.url.trim().is_empty() {
            return ToolResult::error("web_extract", "url is required");
        }
        // SSRF protection — same fail-closed guard as web_scrape.
        let (safe, block_msg) = ssrf_verdict(&args.url).await;
        if !safe {
            return ToolResult::error("web_extract", block_msg);
        }
        match self
            .0
            .call(TOOL_EXTRACT, serde_json::json!({ "url": args.url }))
            .await
        {
            Ok(payload) => ToolResult::success("web_extract", payload),
            Err(e) => ToolResult::error("web_extract", e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// web_crawl tool
// ---------------------------------------------------------------------------

/// Crawl a site through sourcehound's `web.crawl` MCP tool.
pub struct WebCrawlTool(Sourcehound);

impl WebCrawlTool {
    /// The registered instance, backed by the resolved sourcehound binary.
    pub fn global() -> Self {
        Self(Sourcehound::global())
    }
}

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
         max depth and page count, returning each page as markdown. Backed by \
         the sourcehound engine. Set maxDepth/maxPages to bound the crawl. \
         Requires the 'sourcehound' binary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebCrawlArgs>("web_crawl", "Crawl website to markdown")
    }

    fn is_available(&self) -> bool {
        self.0.is_available()
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
        let arguments = serde_json::json!({
            "url": args.url,
            "max_depth": args.max_depth,
            "max_pages": args.max_pages,
        });
        match self.0.call(TOOL_CRAWL, arguments).await {
            Ok(payload) => ToolResult::success("web_crawl", payload),
            Err(e) => ToolResult::error("web_crawl", e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal MCP stdio server that echoes back the tool name and the
    /// arguments it was called with, as `structuredContent`. Exercising the
    /// tools against this proves the call path reaches the MCP client with
    /// sourcehound's real tool names and argument shapes.
    const ECHO_MCP_SERVER: &str = r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    rid = req.get('id')
    if rid is None:
        continue
    method = req.get('method')
    if method == 'initialize':
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'sourcehound-echo', 'version': '0'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': n, 'description': 'echo', 'inputSchema': {'type': 'object'}} for n in ['web.search', 'web.scrape', 'web.extract', 'web.crawl']]}
    else:
        params = req.get('params') or {}
        name = params.get('name')
        args = params.get('arguments') or {}
        if name == 'web.search':
            payload = {'count': 1, 'echoed_tool': name, 'echoed_args': args, 'results': [{'title': 'Tokio docs', 'url': 'https://tokio.rs', 'content': 'async runtime'}]}
        elif name == 'web.scrape':
            payload = {'success': True, 'url': args.get('url'), 'markdown': '# ' + str(args.get('url'))}
        elif name == 'web.extract':
            payload = {'success': True, 'url': args.get('url'), 'content': 'extracted body'}
        else:
            payload = {'success': True, 'echoed_tool': name, 'echoed_args': args}
        result = {'content': [{'type': 'text', 'text': json.dumps(payload)}], 'structuredContent': payload}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#;

    /// Point a [`Sourcehound`] at a fresh copy of the echo server. The temp
    /// dir is returned so the caller keeps it alive for the test's duration.
    fn echo_sourcehound() -> (Sourcehound, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_echo_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("echo_server.py");
        std::fs::write(&script, ECHO_MCP_SERVER).expect("write echo server");
        (
            Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]),
            dir,
        )
    }

    #[tokio::test]
    async fn web_scrape_routes_through_mcp_to_sourcehound_tool() {
        let (sh, _dir) = echo_sourcehound();
        let result = WebScrapeTool(sh)
            .execute(
                json!({"url": "https://example.com/a"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "scrape failed: {:?}", result.error);
        let content = result.content.to_string();
        assert!(
            content.contains("https://example.com/a"),
            "payload lost the url: {content}"
        );
        assert!(
            content.contains("# https://example.com/a"),
            "scrape did not return web.scrape markdown: {content}"
        );
    }

    #[tokio::test]
    async fn web_extract_routes_through_mcp_to_sourcehound_tool() {
        let (sh, _dir) = echo_sourcehound();
        let result = WebExtractTool(sh)
            .execute(
                json!({"url": "https://example.com/b"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "extract failed: {:?}", result.error);
        assert!(
            result.content.to_string().contains("extracted body"),
            "extract did not return web.extract content: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn web_crawl_forwards_depth_and_page_bounds() {
        let (sh, _dir) = echo_sourcehound();
        let result = WebCrawlTool(sh)
            .execute(
                json!({"url": "https://example.com", "maxDepth": 3, "maxPages": 7}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "crawl failed: {:?}", result.error);
        let content = result.content.to_string();
        assert!(
            content.contains("\"echoed_tool\":\"web.crawl\""),
            "{content}"
        );
        assert!(content.contains("\"max_depth\":3"), "{content}");
        assert!(content.contains("\"max_pages\":7"), "{content}");
    }

    #[tokio::test]
    async fn web_search_routes_through_mcp_to_sourcehound_tool() {
        // The fallback-chain search provider must reach the MCP client as
        // `web.search` with sourcehound's real argument names, and turn the
        // `WebSearchOutput` shape into the chain's `WebSearchResult`.
        let (sh, _dir) = echo_sourcehound();
        let results = web_search_via(&sh, "tokio", 5).await.expect("search");
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].title, "Tokio docs");
        assert_eq!(results[0].url, "https://tokio.rs");
        assert_eq!(results[0].snippet, "async runtime");

        // The mock only takes the `web.search` branch, and echoes what it was
        // called with: prove the wire name and sourcehound's `max_results`
        // argument name, not just the parse.
        let echoed = sh
            .call(
                TOOL_SEARCH,
                serde_json::json!({"query": "tokio", "max_results": 5}),
            )
            .await
            .expect("second search");
        assert_eq!(echoed["echoed_tool"], "web.search");
        assert_eq!(echoed["echoed_args"]["max_results"], 5);
        assert_eq!(echoed["echoed_args"]["query"], "tokio");
    }

    #[tokio::test]
    async fn mcp_error_result_surfaces_as_tool_error() {
        // A server that answers tools/call with isError must not look like a
        // success — the error text has to reach the caller.
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_err_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("err_server.py");
        std::fs::write(
            &script,
            r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    rid = req.get('id')
    if rid is None:
        continue
    if req.get('method') == 'initialize':
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'err', 'version': '0'}}
    elif req.get('method') == 'tools/list':
        result = {'tools': []}
    else:
        result = {'content': [{'type': 'text', 'text': 'boom: query rejected'}], 'isError': True}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#,
        )
        .expect("write err server");
        let sh = Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]);
        let err = sh
            .call(TOOL_SEARCH, json!({"query": "x"}))
            .await
            .expect_err("isError must be an Err");
        assert!(err.to_string().contains("boom"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn tool_absent_from_advertised_set_reports_the_settings_fix() {
        // A sourcehound whose `tools.groups` allow-list omits `web` must
        // produce the actionable settings hint, not a bare "unknown tool".
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_narrow_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("narrow_server.py");
        std::fs::write(
            &script,
            r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    rid = req.get('id')
    if rid is None:
        continue
    if req.get('method') == 'initialize':
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'narrow', 'version': '0'}}
    elif req.get('method') == 'tools/list':
        result = {'tools': [{'name': 'cloakctl.tabs', 'description': 'x', 'inputSchema': {'type': 'object'}}]}
    else:
        result = {'content': [{'type': 'text', 'text': 'unknown tool: ' + str((req.get('params') or {}).get('name'))}], 'isError': True}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#,
        )
        .expect("write narrow server");
        let sh = Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]);
        let err = sh
            .call(TOOL_SCRAPE, json!({"url": "https://example.com"}))
            .await
            .expect_err("an unadvertised tool must not be called");
        let msg = err.to_string();
        assert!(msg.contains("does not advertise"), "unexpected: {msg}");
        assert!(msg.contains("tools.groups"), "unexpected: {msg}");
    }

    #[tokio::test]
    async fn missing_binary_reports_install_hint() {
        // A command that cannot spawn must fail with the install hint, not a
        // bare ENOENT the model cannot act on.
        let sh = Sourcehound::new(
            "operant-no-such-sourcehound-binary",
            vec!["mcp".to_string()],
        );
        let err = sh
            .call(TOOL_SEARCH, json!({"query": "x"}))
            .await
            .expect_err("missing binary must be an Err");
        assert!(
            err.to_string().contains("sourcehound binary not found"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn first_text_field_finds_first_string() {
        let value = json!({ "nested": { "a": [{"b": "hello world"}], "c": 42 } });
        assert_eq!(first_text_field(&value).as_deref(), Some("hello world"));
    }

    #[test]
    fn first_text_field_ignores_empty() {
        let value = json!({ "x": "  ", "y": "real text" });
        assert_eq!(first_text_field(&value).as_deref(), Some("real text"));
    }

    #[test]
    fn first_text_field_none_on_numbers() {
        let value = json!({ "x": [1, 2, 3] });
        assert_eq!(first_text_field(&value), None);
    }

    #[test]
    fn parse_search_results_handles_sourcehound_shape() {
        let value = json!({
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
        let value = json!({ "results": [{"title": "a", "url": "u"}, {"title": "b", "url": "u"}] });
        assert_eq!(parse_search_results(&value, 1).len(), 1);
        assert!(parse_search_results(&json!({ "results": [] }), 5).is_empty());
        assert_eq!(
            parse_search_results(&json!({ "data": [{"title": "x", "url": "y"}] }), 5).len(),
            1
        );
    }

    #[test]
    fn unwrap_tool_result_prefers_structured_content() {
        let raw = json!({
            "content": [{"type": "text", "text": "{\"from\": \"text\"}"}],
            "structuredContent": {"from": "structured"}
        });
        assert_eq!(
            unwrap_tool_result(TOOL_SCRAPE, &raw).expect("unwrap"),
            json!({"from": "structured"})
        );
    }

    #[test]
    fn unwrap_tool_result_falls_back_to_text_block() {
        let raw = json!({ "content": [{"type": "text", "text": "{\"from\": \"text\"}"}] });
        assert_eq!(
            unwrap_tool_result(TOOL_SCRAPE, &raw).expect("unwrap"),
            json!({"from": "text"})
        );
    }

    #[test]
    fn unwrap_tool_result_wraps_non_json_text() {
        let raw = json!({ "content": [{"type": "text", "text": "plain prose"}] });
        assert_eq!(
            unwrap_tool_result(TOOL_SCRAPE, &raw).expect("unwrap"),
            json!({"text": "plain prose"})
        );
    }

    #[tokio::test]
    async fn test_web_scrape_blocks_cloud_metadata() {
        // The SSRF guard fires before the MCP call, so this works even when
        // sourcehound is not installed.
        let result = WebScrapeTool(Sourcehound::new("python3", vec![]))
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
        let result = WebExtractTool(Sourcehound::new("python3", vec![]))
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
    fn web_scrape_tool_schema_is_valid() {
        let schema = WebScrapeTool(Sourcehound::new("python3", vec![])).schema();
        assert_eq!(schema.name, "web_scrape");
        assert!(!schema.description.is_empty());
    }

    #[tokio::test]
    async fn web_scrape_rejects_empty_url() {
        let result = WebScrapeTool(Sourcehound::new("python3", vec![]))
            .execute(json!({ "url": "" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("url is required"));
    }

    #[tokio::test]
    async fn web_extract_rejects_empty_url() {
        let result = WebExtractTool(Sourcehound::new("python3", vec![]))
            .execute(json!({ "url": "" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("url is required"));
    }
}
