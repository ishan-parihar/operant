//! Web search and fetch tools
//!
//! Tools for searching the web and fetching web content.

use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::config::{WebToolSettings, runtime_config};
use crate::schema::ToolSchema;
use crate::security::ssrf_verdict;
use crate::tools::web_providers::{
    DDGProvider, ExaProvider, IgsSearchProvider, SearXNGProvider, TavilyProvider,
    WebSearchProvider, WebSearchResult,
};
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Tool for searching the web
pub struct WebSearchTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebSearchArgs {
    query: String,
    num_results: Option<usize>,
}

#[async_trait]
impl OperantTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns relevant results with titles and snippets."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebSearchArgs>("web_search", "Search the web")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: WebSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("web_search", format!("Invalid arguments: {}", e)),
        };

        let settings = runtime_config().tools.web;
        let num_results = args
            .num_results
            .unwrap_or(settings.default_results)
            .min(settings.max_results);

        // The IGS engine is preferred when the binary is installed: it
        // aggregates Tavily/Firecrawl + DuckDuckGo with JS rendering. When
        // `igs` is missing (or returns nothing — its upstream needs a key),
        // we fall back to the configured provider / DuckDuckGo.
        let igs_available =
            runtime_config().tools.igs_enabled && crate::tools::igs::find_igs_binary().is_some();

        // hermes `web_search_registry._resolve` semantics: the explicitly
        // configured provider runs first, then the chain falls through the
        // remaining available engines — so a rate-limited/anomaly-blocked
        // DuckDuckGo (or a keyless igs) no longer silently returns 0 results.
        let candidates = build_search_candidates(&settings, igs_available);
        // Bound every provider to `search_timeout_secs`: a stalled engine
        // (e.g. an igs subprocess whose own `igs_timeout_secs` can exceed the
        // agent loop's tool timeout) must fail over to the next candidate
        // instead of killing the whole search with a tool-level timeout.
        let per_provider_timeout = Duration::from_secs(settings.search_timeout_secs.max(1));
        let (results, used_provider, last_error) =
            run_provider_chain(candidates, &args.query, num_results, per_provider_timeout).await;

        if results.is_empty() {
            // Every candidate either errored or returned nothing. Surface an
            // actionable error instead of a silently-empty result set.
            return ToolResult::error(
                "web_search",
                format!(
                    "Search failed: {}",
                    last_error.unwrap_or_else(|| {
                        "all search providers returned no results — the IGS engine \
                     (v1.0.2, key-free) and DuckDuckGo both came up empty; try again \
                     shortly, or configure a Tavily/Exa key in [tools.web] for \
                     additional providers"
                            .to_string()
                    })
                ),
            );
        }

        ToolResult::success(
            "web_search",
            serde_json::json!({
                "query": args.query,
                "num_results": results.len(),
                "results": results,
                "provider": used_provider,
            }),
        )
    }
}

/// Run the ordered provider chain, bounding every provider to
/// `per_provider_timeout`. Returns `(results, used_provider, last_error)`:
/// the first provider that returns non-empty results wins; empty results,
/// errors, and timeouts all fall through to the next candidate.
async fn run_provider_chain(
    candidates: Vec<Box<dyn WebSearchProvider>>,
    query: &str,
    num_results: usize,
    per_provider_timeout: Duration,
) -> (Vec<WebSearchResult>, String, Option<String>) {
    let mut used_provider = String::new();
    let mut results: Vec<WebSearchResult> = Vec::new();
    let mut last_error: Option<String> = None;
    for provider in candidates {
        used_provider = provider.name().to_string();
        match tokio::time::timeout(per_provider_timeout, provider.search(query, num_results)).await
        {
            Ok(Ok(r)) if !r.is_empty() => {
                results = r;
                break;
            }
            Ok(Ok(_)) => {
                tracing::warn!(
                    provider = %used_provider,
                    "web search provider returned no results — trying next"
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    provider = %used_provider,
                    error = %e,
                    "web search provider failed — trying next"
                );
                last_error = Some(e.to_string());
            }
            Err(_elapsed) => {
                tracing::warn!(
                    provider = %used_provider,
                    timeout = ?per_provider_timeout,
                    "web search provider timed out — trying next"
                );
                last_error = Some(format!(
                    "{used_provider} timed out after {per_provider_timeout:?}"
                ));
            }
        }
    }
    (results, used_provider, last_error)
}

/// Build the ordered web-search provider chain (hermes
/// `web_search_registry._resolve` capability fallback).
///
/// 1. The explicitly configured preferred provider runs first
///    (tavily / exa / searxng / duckduckgo / igs / auto).
/// 2. Then the remaining *available* engines in preference order:
///    igs → tavily (key) → exa (key) → duckduckgo → searxng (url).
///
/// Engines that cannot function (no key, no base URL, missing binary) are
/// skipped, and duplicates are deduplicated so a provider is never tried
/// twice. This keeps `web_search` working when any single engine is blocked
/// (DDG anomaly pages / rate limits) or unconfigured.
fn build_search_candidates(
    settings: &WebToolSettings,
    igs_available: bool,
) -> Vec<Box<dyn WebSearchProvider>> {
    let mut out: Vec<Box<dyn WebSearchProvider>> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let ddg = || {
        Box::new(DDGProvider::new(
            settings.search_url.clone(),
            settings.user_agent.clone(),
        )) as Box<dyn WebSearchProvider>
    };
    let igs = || Box::new(IgsSearchProvider) as Box<dyn WebSearchProvider>;
    let tavily = || {
        Box::new(TavilyProvider::new(
            settings.tavily_api_key.clone().unwrap_or_default(),
        )) as Box<dyn WebSearchProvider>
    };
    let exa = || {
        Box::new(ExaProvider::new(
            settings.exa_api_key.clone().unwrap_or_default(),
        )) as Box<dyn WebSearchProvider>
    };
    let searxng = || {
        Box::new(SearXNGProvider::new(
            settings.searxng_base_url.clone().unwrap_or_default(),
        )) as Box<dyn WebSearchProvider>
    };

    let mut push = |p: Box<dyn WebSearchProvider>| {
        let name = p.name().to_string();
        if seen.insert(name.clone()) {
            out.push(p);
        }
    };

    // 1. Explicitly configured provider wins (hermes resolves
    // `web.search_backend` first and only auto-selects when unset).
    match settings
        .preferred_provider
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "tavily" => push(tavily()),
        "exa" => push(exa()),
        "searxng" => push(searxng()),
        "duckduckgo" => push(ddg()),
        // igs / auto / unset: prefer igs when the binary is available.
        _ => {
            if should_prefer_igs(&settings.preferred_provider, igs_available) {
                push(igs());
            } else {
                push(ddg());
            }
        }
    }

    // 2. Capability fallbacks, in preference order, deduplicated.
    if igs_available {
        push(igs());
    }
    if settings
        .tavily_api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty())
    {
        push(tavily());
    }
    if settings
        .exa_api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty())
    {
        push(exa());
    }
    push(ddg());
    if settings
        .searxng_base_url
        .as_deref()
        .is_some_and(|u| !u.trim().is_empty())
    {
        push(searxng());
    }

    out
}

/// Tool for fetching web pages
pub struct WebFetchTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebFetchArgs {
    url: String,
    method: Option<String>,
    headers: Option<std::collections::HashMap<String, String>>,
    body: Option<String>,
    timeout: Option<u64>,
}

#[async_trait]
impl OperantTool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Supports GET and POST requests with custom headers and body."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<WebFetchArgs>("web_fetch", "Fetch URL content")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: WebFetchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("web_fetch", format!("Invalid arguments: {}", e)),
        };
        let settings = runtime_config().tools.web;

        // Validate URL + SSRF check
        match reqwest::Url::parse(&args.url) {
            Ok(url) => {
                if url.scheme() != "http" && url.scheme() != "https" {
                    return ToolResult::error(
                        "web_fetch",
                        "Only HTTP and HTTPS URLs are supported",
                    );
                }
                // SSRF protection: block requests to private/loopback/link-local
                // addresses (e.g. 169.254.169.254 AWS metadata, localhost, 10.x,
                // 192.168.x, 127.x, CGNAT, metadata hostnames). Resolves the
                // hostname and checks each address — fail-closed on DNS errors.
                let (safe, block_msg) = ssrf_verdict(&args.url).await;
                if !safe {
                    return ToolResult::error("web_fetch", block_msg);
                }
            }
            Err(e) => return ToolResult::error("web_fetch", format!("Invalid URL: {}", e)),
        }

        // Redirects are disabled: the SSRF guard validates the initial URL
        // only, and following a redirect could silently land on a private/
        // metadata address (classic SSRF redirect bypass). The 3xx response
        // is returned to the model, which can re-issue against Location.
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                args.timeout.unwrap_or(settings.fetch_timeout_secs),
            ))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "web_fetch",
                    format!("Failed to create HTTP client: {}", e),
                );
            }
        };

        let method = args.method.as_deref().unwrap_or("GET");
        let mut request = match method {
            "GET" => client.get(&args.url),
            "POST" => client.post(&args.url),
            "PUT" => client.put(&args.url),
            "DELETE" => client.delete(&args.url),
            "PATCH" => client.patch(&args.url),
            "HEAD" => client.head(&args.url),
            _ => {
                return ToolResult::error(
                    "web_fetch",
                    format!("Unsupported HTTP method: {}", method),
                );
            }
        };

        // Add custom headers
        if let Some(ref headers) = args.headers {
            for (key, value) in headers {
                request = request.header(key, value);
            }
        }

        // Add body for POST/PUT/PATCH
        if let Some(body) = args.body.filter(|body| !body.is_empty()) {
            request = request.body(body);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let headers: std::collections::HashMap<String, String> = response
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();

                match response.text().await {
                    Ok(body) => {
                        let body_size = body.len();
                        if status.is_success() {
                            ToolResult::success(
                                "web_fetch",
                                serde_json::json!({
                                    "url": args.url,
                                    "method": method,
                                    "status_code": status.as_u16(),
                                    "status_text": status.canonical_reason().unwrap_or(""),
                                    "headers": headers,
                                    "body": body,
                                    "body_size": body_size
                                }),
                            )
                        } else {
                            ToolResult::error(
                                "web_fetch",
                                serde_json::json!({
                                    "url": args.url,
                                    "method": method,
                                    "status_code": status.as_u16(),
                                    "status_text": status.canonical_reason().unwrap_or(""),
                                    "headers": headers,
                                    "body": body,
                                    "body_size": body_size
                                })
                                .to_string(),
                            )
                        }
                    }
                    Err(e) => ToolResult::error(
                        "web_fetch",
                        format!("Failed to read response body: {}", e),
                    ),
                }
            }
            Err(e) => ToolResult::error("web_fetch", format!("Request failed: {}", e)),
        }
    }
}

/// Parse DuckDuckGo Lite HTML results into structured JSON values.
///
/// DDG Lite uses a simple table layout where each result has:
/// - A link in an `<a>` tag with class "result-link"
/// - A snippet in a `<td>` with class "result-snippet"
pub(crate) fn parse_ddg_lite_results(html: &str, max_results: usize) -> Vec<serde_json::Value> {
    let mut results = Vec::new();

    // DDG Lite format: results are in <a class="result-link" href="URL">TITLE</a>
    // followed by <td class="result-snippet">SNIPPET</td>
    let mut pos = 0;
    let html_bytes = html.as_bytes();

    while results.len() < max_results && pos < html.len() {
        // Find next result link
        let link_marker = "class=\"result-link\"";
        let link_pos = match html[pos..].find(link_marker) {
            Some(p) => pos + p,
            None => break,
        };

        // Extract href from the <a> tag
        let href_start = match html[..link_pos].rfind("href=\"") {
            Some(p) => p + 6,
            None => {
                pos = link_pos + link_marker.len();
                continue;
            }
        };
        let href_end = match html[href_start..].find('"') {
            Some(p) => href_start + p,
            None => {
                pos = link_pos + link_marker.len();
                continue;
            }
        };
        let url = html_decode(&html[href_start..href_end]);

        // Extract title (content between > and </a>)
        let title_start = match html[link_pos..].find('>') {
            Some(p) => link_pos + p + 1,
            None => {
                pos = link_pos + link_marker.len();
                continue;
            }
        };
        let title_end = match html[title_start..].find("</a>") {
            Some(p) => title_start + p,
            None => {
                pos = link_pos + link_marker.len();
                continue;
            }
        };
        let title = strip_html_tags(&html[title_start..title_end]);

        // Find snippet after the link
        let snippet_marker = "class=\"result-snippet\"";
        let snippet = if let Some(sp) = html[title_end..].find(snippet_marker) {
            let snippet_pos = title_end + sp;
            let content_start = match html[snippet_pos..].find('>') {
                Some(p) => snippet_pos + p + 1,
                None => snippet_pos,
            };
            let content_end = match html[content_start..].find("</td>") {
                Some(p) => content_start + p,
                None => content_start,
            };
            strip_html_tags(&html[content_start..content_end])
        } else {
            String::new()
        };

        // Skip DDG internal links
        if !url.starts_with("https://duckduckgo.com") && !url.is_empty() && !title.is_empty() {
            results.push(serde_json::json!({
                "title": title.trim(),
                "url": url,
                "snippet": snippet.trim()
            }));
        }

        pos = title_end;
    }

    // If we couldn't parse the Lite format, try a simpler heuristic approach
    if results.is_empty() {
        // Look for any <a href="http..."> patterns
        pos = 0;
        while results.len() < max_results && pos < html.len() {
            let href_marker = "href=\"http";
            let hp = match html[pos..].find(href_marker) {
                Some(p) => pos + p + 6,
                None => break,
            };
            let href_end = match html[hp..].find('"') {
                Some(p) => hp + p,
                None => break,
            };
            let url = html_decode(&html[hp..href_end]);

            // Get link text
            let text_start = match html[href_end..].find('>') {
                Some(p) => href_end + p + 1,
                None => {
                    pos = href_end;
                    continue;
                }
            };
            let text_end = match html[text_start..].find('<') {
                Some(p) => text_start + p,
                None => {
                    pos = href_end;
                    continue;
                }
            };
            let title = html[text_start..text_end].trim().to_string();

            if !url.contains("duckduckgo.com") && title.len() > 3 {
                results.push(serde_json::json!({
                    "title": title,
                    "url": url,
                    "snippet": ""
                }));
            }
            pos = text_end;
        }
    }

    let _ = html_bytes; // used for borrow check
    results
}

/// Strip HTML tags from a string.
fn strip_html_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    html_decode(&result)
}

/// Decode common HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

/// Decide whether to prefer the IGS engine for web search.
///
/// Explicit user config wins: IGS is preferred only when the configured
/// provider is `igs`/`auto`/unset (compared case-insensitively, trimmed).
/// Any explicit non-igs provider (tavily, exa, searxng, …) returns `false`
/// even when the igs binary is installed — mirroring hermes's
/// `web_search_registry._resolve(explicit, capability)`, which honors the
/// configured backend before auto-selecting among available ones.
///
/// `igs_available` still gates the result: even a configured `igs`
/// preference yields `false` when the binary is missing.
fn should_prefer_igs(preferred: &str, igs_available: bool) -> bool {
    if !igs_available {
        return false;
    }
    matches!(
        preferred.trim().to_ascii_lowercase().as_str(),
        "igs" | "auto" | ""
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_should_prefer_igs_matrix() {
        // Explicit non-igs providers must win even when igs is installed.
        for pref in ["tavily", "exa", "searxng", "unknown"] {
            assert!(
                !should_prefer_igs(pref, true),
                "explicit '{pref}' must not be overridden by igs availability"
            );
        }
        // igs / auto / unset prefer igs when available.
        for pref in ["igs", "auto", ""] {
            assert!(should_prefer_igs(pref, true), "'{pref}' should prefer igs");
        }
        // Case + whitespace tolerance.
        assert!(should_prefer_igs("  IGS ", true));
        assert!(should_prefer_igs("Auto", true));
        // Missing binary always wins regardless of preference.
        assert!(!should_prefer_igs("igs", false));
        assert!(!should_prefer_igs("tavily", false));
    }

    fn test_settings(preferred: &str) -> WebToolSettings {
        WebToolSettings {
            preferred_provider: preferred.to_string(),
            ..Default::default()
        }
    }

    fn provider_names(candidates: &[Box<dyn WebSearchProvider>]) -> Vec<String> {
        candidates.iter().map(|p| p.name().to_string()).collect()
    }

    #[test]
    fn search_candidates_preferred_first_then_fallbacks() {
        // duckduckgo preferred: ddg first, igs second, ddg deduped.
        let settings = test_settings("duckduckgo");
        let names = provider_names(&build_search_candidates(&settings, true));
        assert_eq!(names, vec!["duckduckgo", "igs"]);

        // No igs binary: only ddg (deduped, single entry).
        let names = provider_names(&build_search_candidates(&settings, false));
        assert_eq!(names, vec!["duckduckgo"]);
    }

    #[test]
    fn search_candidates_prefers_igs_when_auto() {
        let settings = test_settings("auto");
        let names = provider_names(&build_search_candidates(&settings, true));
        assert_eq!(names, vec!["igs", "duckduckgo"]);

        let names = provider_names(&build_search_candidates(&settings, false));
        assert_eq!(names, vec!["duckduckgo"]);
    }

    #[test]
    fn search_candidates_includes_keyed_providers() {
        let mut settings = test_settings("duckduckgo");
        settings.tavily_api_key = Some("tvly-key".to_string());
        settings.exa_api_key = Some("exa-key".to_string());
        let names = provider_names(&build_search_candidates(&settings, true));
        assert_eq!(names, vec!["duckduckgo", "igs", "tavily", "exa"]);
    }

    #[test]
    fn search_candidates_explicit_tavily_first() {
        let mut settings = test_settings("tavily");
        settings.tavily_api_key = Some("tvly-key".to_string());
        let names = provider_names(&build_search_candidates(&settings, true));
        assert_eq!(names, vec!["tavily", "igs", "duckduckgo"]);
    }

    #[test]
    fn search_candidates_dedupes_across_preferred_and_fallback() {
        let mut settings = test_settings("exa");
        settings.exa_api_key = Some("exa-key".to_string());
        let names = provider_names(&build_search_candidates(&settings, true));
        // exa preferred + exa fallback are the same provider — one entry.
        assert_eq!(names, vec!["exa", "igs", "duckduckgo"]);
    }

    #[test]
    fn test_web_search_schema() {
        let schema = WebSearchTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "web_search");
    }

    #[tokio::test]
    async fn test_provider_chain_fails_over_on_timeout() {
        // A provider that hangs past its budget must not block the chain:
        // the next candidate runs and wins, and the timeout is surfaced.
        struct SlowProvider;
        #[async_trait]
        impl WebSearchProvider for SlowProvider {
            fn name(&self) -> &str {
                "slow"
            }
            async fn search(
                &self,
                _q: &str,
                _n: usize,
            ) -> crate::error::Result<Vec<WebSearchResult>> {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(vec![])
            }
        }
        struct FastProvider;
        #[async_trait]
        impl WebSearchProvider for FastProvider {
            fn name(&self) -> &str {
                "fast"
            }
            async fn search(
                &self,
                _q: &str,
                _n: usize,
            ) -> crate::error::Result<Vec<WebSearchResult>> {
                Ok(vec![WebSearchResult {
                    title: "hit".to_string(),
                    url: "https://example.com".to_string(),
                    snippet: "snippet".to_string(),
                }])
            }
        }

        let (results, used, err) = run_provider_chain(
            vec![Box::new(SlowProvider), Box::new(FastProvider)],
            "q",
            5,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(used, "fast");
        assert_eq!(results.len(), 1);
        let err_text = err.unwrap_or_default();
        assert!(
            err_text.contains("timed out"),
            "expected a timeout error, got: {err_text}"
        );
    }

    #[test]
    fn test_web_fetch_schema() {
        let schema = WebFetchTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "web_fetch");
    }

    #[tokio::test]
    async fn test_web_search_invalid_args() {
        let tool = WebSearchTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool;
        let result = tool
            .execute(json!({"url": ""}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_web_fetch_blocks_cloud_metadata() {
        // SSRF: 169.254.169.254 (cloud metadata) must be rejected before any fetch.
        let tool = WebFetchTool;
        let result = tool
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
    async fn test_web_fetch_blocks_private_range() {
        // RFC 1918 private ranges must be rejected.
        let tool = WebFetchTool;
        let result = tool
            .execute(
                json!({"url": "http://10.0.0.1/internal"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("SSRF"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_web_fetch_allows_public_ip() {
        // Guard must NOT over-block: a public literal IP passes the pre-flight
        // check (no DNS needed). The subsequent request will fail to connect
        // (8.8.8.8:80 refuses) — the point is it must fail on CONNECT, not on
        // the SSRF guard.
        let tool = WebFetchTool;
        let result = tool
            .execute(json!({"url": "http://8.8.8.8/"}), ToolContext::default())
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(
            !err.contains("SSRF"),
            "public IP should pass the guard, got: {err}"
        );
    }

    #[test]
    fn test_strip_html_tags() {
        let result = strip_html_tags("<b>bold</b> and <i>italic</i>");
        assert_eq!(result, "bold and italic");
    }

    #[test]
    fn test_html_decode() {
        let result = html_decode("&amp; &lt; &gt; &quot; &#39;");
        assert_eq!(result, "& < > \" '");
    }

    #[test]
    fn test_parse_ddg_lite_actual_ddg_format() {
        let html = r#"<html><body><a href="https://example.com/page" class="result-link">Example Site</a><td class="result-snippet">This is an example snippet</td></body></html>"#;
        let results = parse_ddg_lite_results(html, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "Example Site");
        assert_eq!(results[0]["url"], "https://example.com/page");
        assert_eq!(results[0]["snippet"], "This is an example snippet");
    }

    #[test]
    fn test_parse_ddg_lite_results_empty() {
        let results = parse_ddg_lite_results("<html></html>", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_ddg_lite_results_with_links() {
        let html = r#"<html><body><a class="result-link" href="https://example.com">Example</a></body></html>"#;
        let results = parse_ddg_lite_results(html, 10);
        // Should match the fallback heuristic (href="http...")
        assert_eq!(results.len(), 1);
    }
}
