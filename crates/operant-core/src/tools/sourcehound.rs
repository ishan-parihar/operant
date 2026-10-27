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

/// sourcehound supports two MCP `tools/list` surfaces and only lists the
/// tool groups its settings allow-list names (`tools.groups` in
/// `settings.yml`). The **v1** surface lists every granular name; the **v2**
/// surface (sourcehound 1.8.0's default) lists only the 23 per-intent
/// composites (`search`, `read`, `crawl`, ...) plus `cloakctl.call`. A group
/// allow-list that omits `web` means the server has no web tool at all and
/// answers every call with a bare "unknown tool".
///
/// The path below is sourcehound's DEFAULT layout, verified against the live
/// 1.8.0 installation rather than assumed: `sourcehound status` prints
/// `Config: ~/.config/sourcehound` and `settings.yml` sits inside it. The
/// `-mcp` directory name this hint used to carry is real in the binary (its
/// `strings` output names both `sourcehound-mcp` and `sourcehound`), but it
/// only materializes under `SOURCEHOUND_CONFIG_DIR=<dir>/sourcehound-mcp` — so
/// naming it as the default sent operators looking for a file that did not
/// exist, which is the RT-195 residual. `XDG_CONFIG_HOME` redirects the base
/// too, so the hint names the env vars in the order sourcehound consults them.
pub const SOURCEHOUND_GROUP_HINT: &str = "sourcehound is running but does not offer this tool. Enable its tool group: add \"web\" to `tools.groups` in ~/.config/sourcehound/settings.yml (or drop the `groups` allow-list) and restart the sourcehound process. If you redirected the config, it lives under `SOURCEHOUND_CONFIG_DIR` (or a `sourcehound-mcp` subdir of `XDG_CONFIG_HOME`) — run `sourcehound status` to print the path this install actually uses.";

/// Whether a tool result is sourcehound's own "this name does not exist".
///
/// The server answers an unknown tool with a bare `unknown tool: <name>`
/// (`tools/call` succeeds with `isError: true`). That - not the advertised
/// set - is the only authoritative signal that a name is genuinely absent,
/// because it is the server's own verdict rather than operant's inference.
/// Recognized in both the structured and the text-block shape.
fn is_unknown_tool_error(payload: &Value) -> bool {
    let text = payload
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| first_text_block(payload))
        .unwrap_or_default();
    let lower = text.to_ascii_lowercase();
    lower.contains("unknown tool") || lower.contains("tool not found")
}

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
        // Dispatch, then judge. The advertised set cannot decide availability:
        // sourcehound 1.8.0's default (v2) surface lists 23 composite names
        // and none of the granular v1 names operant calls, yet answers them.
        // A local refusal on that basis failed every web and browser tool on
        // a stock install. So dispatch first, and let the server's own
        // "unknown tool" verdict be the only thing that adds the hint - a
        // call that would have worked is never pre-empted.
        let raw = client
            .call_tool(tool, arguments)
            .await
            .map_err(|e| Error::Agent(format!("sourcehound `{tool}` call failed: {e}")))?;
        if raw.get("isError").and_then(Value::as_bool).unwrap_or(false)
            && is_unknown_tool_error(&raw)
        {
            return Err(Error::Agent(format!(
                "sourcehound does not serve `{tool}`.\n{SOURCEHOUND_GROUP_HINT}"
            )));
        }
        unwrap_tool_result(tool, &raw)
    }

    /// The name to send a crawl call under on the wire, and whether the
    /// receiving surface needs `mode: "crawl"`.
    ///
    /// `web.crawl` on the v1 surface, `crawl` on v2 (which folded the four v1
    /// web tools into composites). Resolved per handle from the server's own
    /// advertised set, so a v2 install crawls and a v1 one is unchanged.
    ///
    /// v2's `crawl` composite is dual-mode: `mode` defaults to `map` (sitemap /
    /// link listing). A crawl call that does not send `mode: "crawl"` gets a
    /// sitemap link list back and reports success — the tool returns an empty
    /// page set while the server answered a different question (verified
    /// live against 1.8.0: `crawl {url}` → `links[0]: count: 0`, `crawl
    /// {url, mode:"crawl"}` → `pages[1]{url,title,content,depth}`).
    async fn crawl_wire_surface(&self) -> (&'static str, bool) {
        // Resolve the advertised set FIRST, on this call's own connect — a
        // crawl is frequently the FIRST sourcehound call in a process, and
        // reading the set before the handshake walks the `None` arm (v1
        // name, no mode) into a v2 server's "tool not found" verdict. The
        // first-call path is the live one; the tests that forced the
        // handshake with a probe call masked exactly this ordering bug
        // (live-verified: first web_crawl in a fresh `operant run` failed
        // against stock 1.8.0 while every subsequent call would succeed).
        if let Err(e) = self.client().await {
            // A spawn failure is about to fail the call anyway — surface it
            // from the dispatch rather than from here, where the caller has
            // no error channel. The conservative v1 defaults stand.
            tracing::warn!(
                error = %e,
                "sourcehound handshake failed before crawl; using v1 wire name"
            );
            return (TOOL_CRAWL, false);
        }
        let advertised = self.advertised.lock().await.clone();
        match advertised.as_ref() {
            // The server has not told us what it lists (no spawn yet, or an
            // empty tools/list): try the v1 name, which is what the rest of
            // this module assumes, and let the server correct us.
            None => (TOOL_CRAWL, false),
            // v2: the v1 name is absent and the composite is present. The
            // composite defaults to map mode, so the call must name crawl mode.
            Some(names) if !names.contains(TOOL_CRAWL) && names.contains("crawl") => {
                ("crawl", true)
            }
            // v1, or a server listing neither (the gate will say why).
            Some(_) => (TOOL_CRAWL, false),
        }
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
        .unwrap_or_else(|| toon_results(value, limit));

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

/// Extract search results from a TOON `text` payload — the shape
/// `unwrap_tool_result` falls back to when a sourcehound tool answers in
/// its default TOON format rather than JSON.
///
/// (iter-1210) `format: "json"` now travels with every search call, but a
/// server that ignores the field (or a build without it) still answers in
/// TOON, and the JSON path then finds no `results` array and returns an
/// EMPTY vec. An empty vec reads to the caller as "no results", so the
/// search silently yields nothing while the server answered in full — the
/// defect this fallback closes.
///
/// TOON's search shape is a header line (`results[10]:`) followed by
/// indented entries:
///
/// ```text
/// results[10]:
///   - title: "rustls/tokio-rustls"
///     url: "https://github.com/rustls/tokio-rustls"
///     content: "\u{2b50} 238 stars | ..."
/// ```
///
/// Parsing is deliberately tolerant: values read to end-of-line, quotes
/// stripped, an entry with no recognisable title or url dropped later by
/// `parse_search_results`. TOON truncates long values with a `\u{2026}`, so a
/// recovered snippet is a preview rather than the whole field — better
/// than nothing, and never worse than the empty vec it replaces.
fn toon_results(value: &Value, limit: usize) -> Vec<Value> {
    const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

    let Some(text) = value.get("text").and_then(Value::as_str) else {
        return Vec::new();
    };
    // A pathological payload cannot be worth scanning in full; bound the
    // work so one huge response cannot stall the turn.
    if text.len() > MAX_TEXT_BYTES {
        return Vec::new();
    }
    // Only parse when the payload actually declares a result list. Without
    // this gate, any non-JSON text (an error message, a prose answer) would
    // be fed through the parser and could yield rows that were never
    // results.
    if !text
        .lines()
        .any(|line| line.trim_start().starts_with("results["))
    {
        return Vec::new();
    }

    let mut entries: Vec<Value> = Vec::new();
    let mut current: Option<serde_json::Map<String, Value>> = None;
    let mut in_results = false;
    let mut base_indent: Option<usize> = None;

    for raw_line in text.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }
        let indent = raw_line.len() - raw_line.trim_start().len();
        let trimmed = raw_line.trim_start().trim_end();
        if !in_results {
            if trimmed.starts_with("results[") {
                in_results = true;
                base_indent = Some(indent);
            }
            continue;
        }
        // A sibling field at the header's own indentation (`meta:`,
        // `total:`) ends the list.
        if let Some(base) = base_indent
            && indent == base
            && !trimmed.starts_with("- ")
        {
            break;
        }
        if let Some(body) = trimmed.strip_prefix("- ") {
            if let Some(map) = current.take()
                && !map.is_empty()
            {
                entries.push(Value::Object(map));
                if entries.len() >= limit {
                    break;
                }
            }
            let mut map = serde_json::Map::new();
            // The first field may sit on the dash line itself.
            if let Some((key, val)) = split_toon_field(body) {
                map.insert(key, Value::String(val));
            }
            current = Some(map);
            continue;
        }
        if let Some(map) = current.as_mut()
            && let Some((key, val)) = split_toon_field(trimmed)
        {
            map.insert(key, Value::String(val));
        }
    }
    if let Some(map) = current.take()
        && !map.is_empty()
    {
        entries.push(Value::Object(map));
    }
    entries.truncate(limit);
    entries
}

/// Split one TOON `key: value` field. Returns `None` for a line that is not
/// a field (a nested block header, a bare list marker).
///
/// Values are read to end-of-line because TOON's search rows are one field
/// per line. A leading and trailing quote pair is stripped; TOON emits
/// `content: "\u{2705} 4 score | 5 answers"` exactly in that shape.
fn split_toon_field(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = value.trim();
    // A nested block opens with `key:` and nothing after it — not a field.
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
        .to_string();
    Some((key.to_string(), value))
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
    // (iter-1210) Ask for JSON, not the server's default. sourcehound's
    // default output format is TOON — a compact indented `results[10]:`
    // list, not JSON — so the text block carries no `results` array and
    // `parse_search_results` found none and returned an EMPTY vec. The
    // caller reads an empty vec as "no results" and falls through to the
    // next provider, so a live search silently returned nothing while the
    // server had answered in full (verified against the real binary: the
    // same query answers with 10 results in TOON and, with `format:
    // "json"`, both a `structuredContent` and a JSON `results` array).
    //
    // `format` is an optional parameter on every sourcehound tool schema
    // ("toon|json"), so a server that does not know it must ignore it
    // rather than fail — and the TOON fallback in `parse_search_results`
    // covers that case. Never pass a field the server rejects outright:
    // it would turn every search into an `isError`.
    let arguments = serde_json::json!({
        "query": query,
        "max_results": limit,
        "format": "json",
    });
    let payload = sh.call(TOOL_SEARCH, arguments).await?;
    Ok(parse_search_results(&payload, limit))
}

/// Scrape a URL to markdown through sourcehound's `web.scrape` MCP tool.
pub async fn scrape_url(url: &str) -> Result<String> {
    // `format: "json"` — the same iter-1210 contract as the search path: the
    // server's TOON default is text operant cannot parse, and a TOON reply
    // here degrades the markdown to whatever `first_text_field` rescues.
    let arguments = serde_json::json!({ "url": url, "format": "json" });
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
        // `format: "json"` — iter-1210's contract on every sourcehound call:
        // the server's default TOON text cannot carry the markdown payload
        // as a parseable object, so ask for the structured form. The SSRF
        // guard runs first, unchanged.
        match self
            .0
            .call(
                TOOL_SCRAPE,
                serde_json::json!({ "url": args.url, "format": "json" }),
            )
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
            .call(
                TOOL_EXTRACT,
                serde_json::json!({ "url": args.url, "format": "json" }),
            )
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
        // v2's `crawl` composite defaults to `mode: "map"` (a sitemap link
        // listing); the tool promises pages, so the call must name crawl
        // mode. `format: "json"` is the same contract as the search path
        // (iter-1210): the server's default TOON text cannot be parsed into
        // a page list, and an unparseable reply reads as an empty crawl.
        let (wire_name, needs_mode) = self.0.crawl_wire_surface().await;
        let mut arguments = serde_json::json!({
            "url": args.url,
            "max_depth": args.max_depth,
            "max_pages": args.max_pages,
            "format": "json",
        });
        if needs_mode && let Some(map) = arguments.as_object_mut() {
            map.insert("mode".to_string(), Value::String("crawl".to_string()));
        }
        match self.0.call(wire_name, arguments).await {
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
            payload = {'success': True, 'url': args.get('url'), 'markdown': '# ' + str(args.get('url')), 'echoed_args': args}
        elif name == 'web.extract':
            payload = {'success': True, 'url': args.get('url'), 'content': 'extracted body', 'echoed_args': args}
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

    /// A sourcehound-shaped server that IGNORES `format` and answers in TOON.
    ///
    /// The real server's default format is TOON — `results[10]:` followed by
    /// indented `- key: value` entries in a plain-text block, with no
    /// `structuredContent`. This is the shape that made a live search return
    /// nothing: `unwrap_tool_result` could not parse it as JSON, so the
    /// payload became `{"text": "..."}` and `parse_search_results` found no
    /// `results` array. The mock reproduces it exactly, including a value
    /// holding a `|` and a `:` and a trailing `...` truncation.
    const TOON_MCP_SERVER: &str = r#"import sys, json
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
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'sourcehound-toon', 'version': '0'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'web.search', 'description': 'toon', 'inputSchema': {'type': 'object'}}]}
    else:
        params = req.get('params') or {}
        toon = ("results[2]:\n"
                "  - title: \"tokio-rs/tokio\"\n"
                "    url: \"https://github.com/tokio-rs/tokio\"\n"
                "    content: \"33497 stars | Rust | A runtime for writing reliable...\"\n"
                "    score: 0.95\n"
                "  - title: \"Tokio docs\"\n"
                "    url: \"https://tokio.rs\"\n"
                "    content: \"An asynchronous runtime for Rust\"\n"
                "meta:\n"
                "  total: 2\n")
        result = {'content': [{'type': 'text', 'text': toon}]}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#;

    fn toon_sourcehound() -> (Sourcehound, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_toon_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("toon_server.py");
        std::fs::write(&script, TOON_MCP_SERVER).expect("write the TOON server");
        (
            Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]),
            dir,
        )
    }

    /// The live-regression test. A search must return results when the server
    /// answers in its default TOON format, because that is what the real
    /// sourcehound does — and an empty vec reads to the caller as "no
    /// results", so the tool silently reported nothing while the server had
    /// answered.
    #[tokio::test]
    async fn a_toon_search_reply_yields_results_not_an_empty_vec() {
        let (sh, _dir) = toon_sourcehound();
        let results = web_search_via(&sh, "rust tokio runtime", 5)
            .await
            .expect("the search must not fail on a TOON reply");
        assert_eq!(
            results.len(),
            2,
            "both TOON entries must be recovered, got: {results:?}"
        );
        assert_eq!(results[0].title, "tokio-rs/tokio");
        assert_eq!(results[0].url, "https://github.com/tokio-rs/tokio");
        assert!(
            results[0].snippet.contains("33497 stars"),
            "the snippet must carry the content field: {:?}",
            results[0].snippet
        );
        assert_eq!(results[1].title, "Tokio docs");
        assert_eq!(results[1].url, "https://tokio.rs");
    }

    /// A sourcehound-shaped server that RECORDS the arguments of every
    /// `web.search` call it receives, so a test can assert on the argument
    /// object the tool actually sent rather than on one the test built
    /// itself. An assertion over a hand-made `sh.call(...)` would pass even
    /// with the production flag removed, which makes it worthless as a
    /// guard — the same trap iter-1207's first mutation check found.
    const RECORDING_MCP_SERVER: &str = r#"import sys, json, os
# The recording path arrives as the single CLI argument, so two tests (or a
# rerun in the same process) can never write to the same file.
SEEN = sys.argv[1] if len(sys.argv) > 1 else '/tmp/operant_seen.json'
os.makedirs(os.path.dirname(SEEN), exist_ok=True)
def record(payload):
    try:
        data = []
        if os.path.exists(SEEN):
            with open(SEEN) as fh:
                data = json.load(fh)
        data.append(payload)
        with open(SEEN, 'w') as fh:
            json.dump(data, fh)
    except Exception:
        pass
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
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'sourcehound-record', 'version': '0'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'web.search', 'description': 'record', 'inputSchema': {'type': 'object'}}]}
    else:
        params = req.get('params') or {}
        name = params.get('name')
        args = params.get('arguments') or {}
        if name == 'web.search':
            record({'tool': name, 'args': args})
        payload = {'count': 1, 'results': [{'title': 'Tokio docs', 'url': 'https://tokio.rs', 'content': 'async runtime'}]}
        result = {'content': [{'type': 'text', 'text': json.dumps(payload)}], 'structuredContent': payload}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#;

    /// `format: "json"` must travel with the call the TOOL makes.
    ///
    /// The recording server writes every `web.search` argument object it
    /// receives to a file, so this asserts on production traffic. Without
    /// the flag the server answers in TOON and the search silently returns
    /// nothing, so this is the guard on the primary half of the fix.
    #[tokio::test]
    async fn every_search_call_asks_the_server_for_json() {
        let seen_path = std::env::temp_dir().join(format!(
            "operant_sourcehound_seen_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&seen_path);
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_record_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("record_server.py");
        std::fs::write(&script, RECORDING_MCP_SERVER).expect("write the recording server");
        // The path is passed through the environment so the script is a
        // constant (no format! interpolation inside the raw Python).
        let sh = Sourcehound::new(
            "python3",
            vec![
                script.to_string_lossy().to_string(),
                seen_path.to_string_lossy().to_string(),
            ],
        );
        // One call through the real search path.
        let results = web_search_via(&sh, "rust tokio runtime", 3)
            .await
            .expect("search");
        assert_eq!(results.len(), 1, "{results:?}");

        let recorded: Value = serde_json::from_str(
            &std::fs::read_to_string(&seen_path).expect("the server must have recorded the call"),
        )
        .expect("the recording is JSON");
        let calls = recorded.as_array().expect("a list of calls");
        assert_eq!(
            calls.len(),
            1,
            "exactly one search call was made: {calls:?}"
        );
        assert_eq!(
            calls[0]["args"]["format"], "json",
            "the tool must ask for json — sourcehound's default TOON format is unreadable by the parser: {}",
            calls[0]["args"]
        );
        assert_eq!(
            calls[0]["args"]["query"], "rust tokio runtime",
            "the query must reach the server"
        );
        let _ = std::fs::remove_file(&seen_path);
    }

    /// A reachable server that answers with results still parses them, so the
    /// JSON path is pinned too (the guard against the TOON fallback eating a
    /// case it should not own).
    #[test]
    fn parse_search_results_reads_a_json_results_array() {
        let value = json!({
            "results": [
                {"title": "tokio-rs/tokio", "url": "https://github.com/tokio-rs/tokio", "content": "async runtime"},
                {"title": "", "url": "", "content": "no title and no url — dropped"},
                {"name": "Tokio docs", "link": "https://tokio.rs", "snippet": "async runtime for Rust"}
            ],
            "count": 3
        });
        let parsed = parse_search_results(&value, 10);
        assert_eq!(
            parsed.len(),
            2,
            "a row with neither title nor url is dropped"
        );
        assert_eq!(parsed[0].title, "tokio-rs/tokio");
        assert_eq!(
            parsed[1].title, "Tokio docs",
            "the name spelling is accepted"
        );
        assert_eq!(parsed[1].url, "https://tokio.rs");
    }

    /// A `text` payload that is not a TOON result list must not yield rows.
    ///
    /// Without the `results[` gate in `toon_results`, an ordinary prose
    /// answer ("Here are some links: ...") would be scanned and could produce
    /// WebSearchResults from text that was never a result set.
    #[test]
    fn parse_search_results_does_not_invent_rows_from_plain_text() {
        let value = json!({"text": "Here are some links: https://tokio.rs is one of them."});
        assert!(
            parse_search_results(&value, 10).is_empty(),
            "a prose text payload must not become search results"
        );
        let errorish = json!({"text": "results could not be retrieved: upstream timeout"});
        assert!(
            parse_search_results(&errorish, 10).is_empty(),
            "an error message mentioning the word must not become search results"
        );
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
    async fn a_disabled_group_reports_the_settings_fix() {
        // A sourcehound whose `tools.groups` allow-list omits `web` answers
        // every web call with a bare "unknown tool". That verdict - the
        // server's own, not an inference from its tools/list - must reach the
        // operator as the actionable settings hint rather than a dead end.
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
            .expect_err("an unknown tool must not look like a success");
        let msg = err.to_string();
        assert!(msg.contains("does not serve"), "unexpected: {msg}");
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

    // ---- v2 tool surface (sourcehound 1.8.0) -----------------------------

    /// The advertised set a stock sourcehound 1.8.0 returns: 23 v2 composites
    /// and not one granular `web.*` name.
    fn v2_advertised() -> HashSet<String> {
        [
            "archive",
            "catalog.list",
            "cloakctl.call",
            "company.lookup",
            "crawl",
            "data.catalog",
            "data.query",
            "gov.search",
            "index.query",
            "news.fetch",
            "news.search",
            "papers.get",
            "papers.search",
            "patents.search",
            "read",
            "recon.target",
            "results.get",
            "search",
            "security.vuln",
            "social.feed",
            "social.search",
            "video.transcript",
            "workflow.run",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
    }

    /// A mock MCP server that answers every call, advertising the v2 surface.
    /// Proves the gate no longer pre-empts a call the server would serve.
    const V2_ECHO_MCP_SERVER: &str = r#"import sys, json
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
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'v2', 'version': '1.8.0'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': n, 'description': 'x', 'inputSchema': {'type': 'object'}} for n in ['search', 'read', 'crawl', 'cloakctl.call']]}
    else:
        params = req.get('params') or {}
        payload = {'echoed_tool': params.get('name'), 'echoed_args': params.get('arguments') or {}}
        result = {'content': [{'type': 'text', 'text': json.dumps(payload)}], 'structuredContent': payload}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
    "#;

    fn v2_echo_sourcehound() -> (Sourcehound, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_v2_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("v2_server.py");
        std::fs::write(&script, V2_ECHO_MCP_SERVER).expect("write v2 echo server");
        (
            Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]),
            dir,
        )
    }

    #[tokio::test]
    async fn a_v2_server_answers_the_v1_web_tools() {
        // RED without the fix: the old gate read the advertised set, found
        // none of operant's names among the 4 listed, and refused locally.
        let (sh, _dir) = v2_echo_sourcehound();
        for (tool, args) in [
            (TOOL_SEARCH, json!({ "query": "rust" })),
            (TOOL_SCRAPE, json!({ "url": "https://example.com" })),
            (TOOL_EXTRACT, json!({ "url": "https://example.com" })),
            (TOOL_CRAWL, json!({ "url": "https://example.com" })),
        ] {
            let echoed = sh
                .call(tool, args)
                .await
                .unwrap_or_else(|e| panic!("a v2 server must still answer {tool}: {e}"));
            assert_eq!(echoed["echoed_tool"], tool);
        }
    }

    #[tokio::test]
    async fn a_v2_server_answers_the_browser_verbs() {
        // The browser verbs are unlisted on v2 too. The provider already
        // injects the `profile` 1.8.0 now requires, so they must go through.
        let (sh, _dir) = v2_echo_sourcehound();
        for tool in [
            "cloakctl.open",
            "cloakctl.navigate",
            "cloakctl.read",
            "cloakctl.act",
            "cloakctl.cdp",
        ] {
            let echoed = sh
                .call(
                    tool,
                    json!({ "profile": "operant", "url": "https://example.com" }),
                )
                .await
                .unwrap_or_else(|e| panic!("a v2 server must still answer {tool}: {e}"));
            assert_eq!(echoed["echoed_tool"], tool);
        }
    }

    #[test]
    fn an_unlisted_name_does_not_prove_unavailability() {
        // The invariant the outage inverted: a non-empty advertised set that
        // omits the name says nothing about whether the server answers it.
        let advertised = v2_advertised();
        for tool in [TOOL_SEARCH, TOOL_SCRAPE, TOOL_EXTRACT, TOOL_CRAWL] {
            assert!(
                !advertised.contains(tool),
                "sanity: {tool} really is unlisted on v2"
            );
        }
    }

    #[test]
    fn the_servers_own_unknown_tool_verdict_is_recognized() {
        // sourcehound answers an unknown tool with isError + "unknown tool".
        // That verdict - not the advertised set - is what carries the hint,
        // and it has to be read off the RAW result because unwrap_tool_result
        // turns any isError into an Err before this predicate would see it.
        for raw in [
            json!({ "error": "unknown tool: web.crawl", "kind": "not_found", "isError": true }),
            json!({
                "content": [{ "type": "text", "text": "unknown tool: web.scrape" }],
                "isError": true,
            }),
        ] {
            assert!(is_unknown_tool_error(&raw), "must recognize: {raw}");
        }
    }

    #[test]
    fn a_real_error_is_not_an_unknown_tool() {
        // The inverse: a genuine server failure must not read as "unknown
        // tool", or a network or parse error would tell the operator to edit
        // their sourcehound groups. Only the name-absence verdict does that.
        for raw in [
            json!({ "content": [{ "type": "text", "text": "engine timeout" }], "isError": true }),
            json!({ "content": [{ "type": "text", "text": "{\"error\":\"DNS failure\"}" }], "isError": true }),
        ] {
            assert!(
                !is_unknown_tool_error(&raw),
                "a real error must not be mistaken for an absent name: {raw}"
            );
        }
        // And a successful payload never reads as one, whatever it holds.
        let ok = json!({
            "structuredContent": { "success": true, "markdown": "# Example Domain" }
        });
        assert!(!is_unknown_tool_error(&ok));
    }

    #[tokio::test]
    async fn a_v2_server_crawl_reroutes_to_the_composite() {
        // `web.crawl` is the one v1 name that genuinely moved: v2 folds it
        // into `crawl`. The tool must send `crawl` (what the server lists),
        // still report as `web_crawl`, and still forward operant's bounds.
        let (sh, _dir) = v2_echo_sourcehound();
        // Force the handshake so the advertised set is read.
        let _ = sh
            .call(TOOL_SEARCH, json!({ "query": "probe" }))
            .await
            .expect("probe call");
        assert_eq!(sh.crawl_wire_surface().await.0, "crawl");
        let result = WebCrawlTool(sh)
            .execute(
                json!({ "url": "https://example.com", "maxDepth": 3, "maxPages": 7 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "crawl failed: {:?}", result.error);
        let content = result.content.to_string();
        assert!(
            content.contains("\"echoed_tool\":\"crawl\""),
            "crawl must be re-routed to the v2 composite: {content}"
        );
        assert!(content.contains("\"max_depth\":3"), "{content}");
        assert!(content.contains("\"max_pages\":7"), "{content}");
    }

    #[tokio::test]
    async fn a_v1_server_crawls_under_the_original_name() {
        // The re-route must not fire on a v1 server, or a stock v1
        // sourcehound would start failing every crawl.
        let (sh, _dir) = echo_sourcehound();
        let _ = sh
            .call(TOOL_SEARCH, json!({ "query": "probe" }))
            .await
            .expect("probe call");
        assert_eq!(sh.crawl_wire_surface().await.0, TOOL_CRAWL);
        let result = WebCrawlTool(sh)
            .execute(
                json!({ "url": "https://example.com", "maxDepth": 2, "maxPages": 5 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "crawl failed: {:?}", result.error);
        assert!(
            result.content.to_string().contains("web.crawl"),
            "{}",
            result.content
        );
    }

    #[tokio::test]
    async fn first_crawl_call_resolves_the_v2_surface_before_dispatch() {
        // The live first-call ordering bug (found live-verified against
        // stock 1.8.0 after the mode/format fix deployed): `web_crawl` was
        // frequently the FIRST sourcehound call in a process, so the
        // advertised set was still `None` when the surface was resolved,
        // the call went out under the v1 name, and the v2 server answered
        // "tool not found" — the exact outage the re-route exists to
        // prevent. The probe-call preamble in the other tests forced the
        // handshake early and masked this. RED before the ordering fix:
        // this test does NOT probe first.
        let (sh, _dir) = v2_echo_sourcehound();
        let result = WebCrawlTool(sh)
            .execute(
                json!({ "url": "https://example.com", "maxDepth": 1, "maxPages": 2 }),
                ToolContext::default(),
            )
            .await;
        assert!(
            result.success,
            "first-call crawl failed: {:?}",
            result.error
        );
        let echoed: Value = serde_json::from_str(&result.content).expect("crawl payload");
        assert_eq!(
            echoed["echoed_tool"], "crawl",
            "the first crawl in a process must already carry the v2 surface: {echoed}"
        );
        assert_eq!(
            echoed["echoed_args"]["mode"], "crawl",
            "and the mode the composite requires: {echoed}"
        );
    }

    #[tokio::test]
    async fn first_crawl_call_on_a_v1_server_keeps_the_granular_name() {
        // The ordering fix must not break v1: forcing the handshake before
        // resolving the surface is safe on both versions, and the v1
        // surface stays single-mode with no mode key.
        let (sh, _dir) = echo_sourcehound();
        let result = WebCrawlTool(sh)
            .execute(
                json!({ "url": "https://example.com", "maxDepth": 1, "maxPages": 2 }),
                ToolContext::default(),
            )
            .await;
        assert!(
            result.success,
            "first-call crawl failed: {:?}",
            result.error
        );
        let echoed: Value = serde_json::from_str(&result.content).expect("crawl payload");
        assert_eq!(echoed["echoed_tool"], TOOL_CRAWL);
        assert!(
            echoed
                .get("echoed_args")
                .and_then(|a| a.get("mode"))
                .is_none(),
            "a v1 crawl never carries a mode key: {echoed}"
        );
    }

    #[tokio::test]
    async fn a_dead_sourcehound_binary_degrades_to_the_v1_surface_not_a_panic() {
        // The handshake-forcing fix adds a failure path: an unspawnable
        // command must degrade to the conservative v1 defaults (the call
        // then fails with the spawn error from `call`, which is the
        // operator-facing diagnostic) rather than panicking or swallowing
        // the error invisibly.
        let sh = Sourcehound::new("definitely-not-a-real-binary-4782", vec![]);
        let (wire_name, needs_mode) = sh.crawl_wire_surface().await;
        assert_eq!(wire_name, TOOL_CRAWL);
        assert!(!needs_mode);
    }

    #[tokio::test]
    async fn a_v2_crawl_names_crawl_mode_and_asks_for_json() {
        // RT-180 (live-verified against stock 1.8.0): v2's `crawl` composite
        // is dual-mode — `mode` defaults to "map" (sitemap/link listing), so a
        // crawl call that does not send `mode: "crawl"` reports success while
        // answering a different question (an empty link list, no pages). The
        // tool must also ask for `format: "json"` (iter-1210): the TOON
        // default cannot be parsed into a page list, which reads as an empty
        // crawl. RED before the fix: neither key was sent.
        let (sh, _dir) = v2_echo_sourcehound();
        let _ = sh
            .call(TOOL_SEARCH, json!({ "query": "probe" }))
            .await
            .expect("probe call forces the handshake");
        let (wire_name, needs_mode) = sh.crawl_wire_surface().await;
        assert_eq!(wire_name, "crawl");
        assert!(needs_mode, "a v2 composite must be told to crawl, not map");
        let result = WebCrawlTool(sh)
            .execute(
                json!({ "url": "https://example.com", "maxDepth": 1, "maxPages": 2 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "crawl failed: {:?}", result.error);
        let echoed: Value = serde_json::from_str(&result.content).expect("crawl payload");
        assert_eq!(
            echoed["echoed_args"]["mode"], "crawl",
            "the v2 composite defaults to map mode; without mode=crawl the tool returns a sitemap listing, not pages: {echoed}"
        );
        assert_eq!(
            echoed["echoed_args"]["format"], "json",
            "the TOON default is unparseable as a page list: {echoed}"
        );
    }

    #[tokio::test]
    async fn a_v1_crawl_sends_no_mode() {
        // The v1 granular `web.crawl` has no `mode` field; sending one is not
        // fatal (serde tolerates unknown keys) but it must not be sent as a
        // matter of contract: only the v2 composite needs it.
        let (sh, _dir) = echo_sourcehound();
        let _ = sh
            .call(TOOL_SEARCH, json!({ "query": "probe" }))
            .await
            .expect("probe call forces the handshake");
        let (wire_name, needs_mode) = sh.crawl_wire_surface().await;
        assert_eq!(wire_name, TOOL_CRAWL);
        assert!(!needs_mode, "the v1 granular name is single-mode");
        let result = WebCrawlTool(sh)
            .execute(
                json!({ "url": "https://example.com", "maxDepth": 1, "maxPages": 2 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "crawl failed: {:?}", result.error);
        let echoed: Value = serde_json::from_str(&result.content).expect("crawl payload");
        assert!(
            echoed
                .get("echoed_args")
                .and_then(|a| a.get("mode"))
                .is_none(),
            "a v1 crawl must not carry a mode key: {}",
            result.content
        );
        assert_eq!(echoed["echoed_args"]["format"], "json");
    }

    #[tokio::test]
    async fn scrape_and_extract_ask_for_json() {
        // iter-1210 closed the search half; scrape/extract had the same
        // latent defect (TOON default → text rescue instead of the structured
        // payload). Both tools must carry format: "json" on the wire.
        let (sh, _dir) = echo_sourcehound();
        let scrape = WebScrapeTool(sh.clone())
            .execute(
                json!({ "url": "https://example.com" }),
                ToolContext::default(),
            )
            .await;
        assert!(scrape.success, "{:?}", scrape.error);
        let scrape_args: Value = serde_json::from_str(&scrape.content).expect("scrape payload");
        assert_eq!(
            scrape_args["echoed_args"]["format"], "json",
            "web_scrape must ask for the structured payload: {}",
            scrape.content
        );
        let extract = WebExtractTool(sh)
            .execute(
                json!({ "url": "https://example.com" }),
                ToolContext::default(),
            )
            .await;
        assert!(extract.success, "{:?}", extract.error);
        let extract_args: Value = serde_json::from_str(&extract.content).expect("extract payload");
        assert_eq!(
            extract_args["echoed_args"]["format"], "json",
            "web_extract must ask for the structured payload: {}",
            extract.content
        );
    }

    /// A mock shaped EXACTLY like the live defect (RT-181): `cloakctl.cdp`
    /// strips `structuredContent` unless the call's top-level arguments carry
    /// `format: "json"`. With the key, the ws_url is a parseable field;
    /// without it, the reply is TOON text and the ws_url lookup fails — the
    /// exact failure the live binary logged ("published no ws_url for
    /// profile `operant`") on every browser_cdp / accessibility_tree call.
    const CDP_FORMAT_MOCK: &str = r#"import sys, json
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
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'cdp-mock', 'version': '1.8.0'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'cloakctl.cdp', 'description': 'x', 'inputSchema': {'type': 'object'}}]}
    else:
        params = req.get('params') or {}
        args = params.get('arguments') or {}
        wants_json = args.get('format') == 'json'
        if wants_json:
            # The server's dispatch_value keeps structuredContent only for
            # format=json callers — the 1.8.0 behavior verified live.
            payload = {'profile': args.get('profile'), 'engine': 'obscura', 'host': '127.0.0.1', 'port': 44073, 'ws_url': 'ws://127.0.0.1:44073/devtools/browser', 'published': True, 'stopped': False}
            result = {'content': [{'type': 'text', 'text': 'toon-ish envelope'}], 'structuredContent': payload}
        else:
            # TOON text, no structuredContent: unwrap_tool_result wraps it as
            # {"text": ...} and every field lookup misses.
            toon = 'profile: ' + str(args.get('profile')) + '\nws_url: "ws://127.0.0.1:44073/devtools/browser"\npublished: true'
            result = {'content': [{'type': 'text', 'text': toon}]}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#;

    #[tokio::test]
    async fn cloakctl_cdp_needs_format_json_for_a_parseable_ws_url() {
        // RT-181, pinned at the seam where it failed: the ws_url extraction
        // in CdpBrowserSession::start reads `published["ws_url"]` off the
        // UNWRAPPED payload. Without format: "json" the payload is
        // {"text": ...} and the lookup misses — RED exactly as the live
        // gateway log showed. This pins the argument SHAPE; start() itself
        // then needs a live websocket, which a mock cannot honor.
        let dir = std::env::temp_dir().join(format!(
            "operant_sourcehound_cdp_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("cdp_mock.py");
        std::fs::write(&script, CDP_FORMAT_MOCK).expect("write cdp mock");
        let sh = Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]);

        // The fixed call: format: "json" rides the arguments, the structured
        // payload survives, and the ws_url the session needs is readable.
        let published = sh
            .call(
                "cloakctl.cdp",
                json!({ "profile": "operant", "format": "json" }),
            )
            .await
            .expect("the cdp call must succeed");
        let ws_url = published
            .get("ws_url")
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
            .expect("the ws_url must be readable off the structured payload");
        assert_eq!(ws_url, "ws://127.0.0.1:44073/devtools/browser");

        // The pre-fix call shape (no format key): the same server answers in
        // TOON, the ws_url lookup misses, and the error the live gateway
        // logged is reproduced verbatim.
        let toon = sh
            .call("cloakctl.cdp", json!({ "profile": "operant" }))
            .await
            .expect("the call itself succeeds — that is the trap");
        assert!(
            toon.get("ws_url")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
                .is_none(),
            "without format=json the payload must NOT yield a ws_url: {toon}"
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

/// RT-195 residual: the group hint named a config path that does not
/// exist in sourcehound's default layout, so an operator following it
/// went looking for a file that was never there. Verified against the
/// LIVE 1.8.0 install (`sourcehound status` prints
/// `Config: ~/.config/sourcehound`, with `settings.yml` inside it), and
/// the `-mcp` variant confirmed to be a redirect-only name by reading
/// the binary's own strings (`SOURCEHOUND_CONFIG_DIR`, `XDG_CONFIG_HOME`,
/// `sourcehound-mcp`, `sourcehound`).
///
/// Three assertions, because the hint has three jobs: name a path that
/// EXISTS on the default layout, name the redirect, and keep the action
/// word ("web") so the fix is still findable. Fails if the path is
/// reverted or the redirect note is dropped.
#[test]
fn group_hint_names_the_path_the_live_install_actually_uses() {
    // 1. The default path, exactly as `sourcehound status` prints it.
    assert!(
        SOURCEHOUND_GROUP_HINT.contains("~/.config/sourcehound/settings.yml"),
        "the hint must name the live default settings path, got: {SOURCEHOUND_GROUP_HINT}"
    );
    // 1b. ...and it must NOT name the -mcp path as the default. The
    // `~/.config/sourcehound-mcp` spelling may appear only as part of
    // the redirect note, so assert the default spelling is unambiguous.
    let without_redirect = SOURCEHOUND_GROUP_HINT
        .split("If you redirected")
        .next()
        .unwrap_or(SOURCEHOUND_GROUP_HINT);
    assert!(
        !without_redirect.contains("sourcehound-mcp"),
        "the pre-redirect half of the hint must name only the default path, got: {without_redirect}"
    );
    // 2. The redirect is named, so a non-default install is not stranded.
    assert!(
        SOURCEHOUND_GROUP_HINT.contains("SOURCEHOUND_CONFIG_DIR"),
        "the hint must name the redirect env var, got: {SOURCEHOUND_GROUP_HINT}"
    );
    // 3. The actionable half survives: the group name and the file to edit.
    assert!(
        SOURCEHOUND_GROUP_HINT.contains("\"web\"")
            && SOURCEHOUND_GROUP_HINT.contains("tools.groups"),
        "the hint must still name the group to enable, got: {SOURCEHOUND_GROUP_HINT}"
    );
}
