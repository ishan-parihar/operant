//! Web search and fetch tools
//!
//! Tools for searching the web and fetching web content.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::config::runtime_config;
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
        // Explicit user config wins: only prefer IGS when it's the configured
        // provider (or config is unset/"auto"). Previously `|| igs_available`
        // silently overrode an explicit tavily/exa/searxng choice whenever the
        // igs binary happened to be installed — hermes resolves the explicit
        // web.search_backend config first and only auto-selects when unset.
        let want_igs = should_prefer_igs(&settings.preferred_provider, igs_available);

        let provider: Box<dyn WebSearchProvider> = if want_igs && igs_available {
            Box::new(IgsSearchProvider)
        } else {
            match settings.preferred_provider.as_str() {
                "tavily" => {
                    let key = settings.tavily_api_key.clone().unwrap_or_default();
                    Box::new(TavilyProvider::new(key))
                }
                "exa" => {
                    let key = settings.exa_api_key.clone().unwrap_or_default();
                    Box::new(ExaProvider::new(key))
                }
                "searxng" => {
                    let base = settings.searxng_base_url.clone().unwrap_or_default();
                    Box::new(SearXNGProvider::new(base))
                }
                _ => Box::new(DDGProvider::new(
                    settings.search_url.clone(),
                    settings.user_agent.clone(),
                )),
            }
        };

        let mut used_provider = provider.name().to_string();
        let results = match provider.search(&args.query, num_results).await {
            Ok(results) if !results.is_empty() => results,
            // igs returned zero results (upstream key missing) — fall back
            // to DuckDuckGo so search still works out of the box.
            _ if used_provider == "igs" => {
                tracing::warn!("igs search returned no results — falling back to DuckDuckGo");
                let ddg =
                    DDGProvider::new(settings.search_url.clone(), settings.user_agent.clone());
                match ddg.search(&args.query, num_results).await {
                    Ok(results) => {
                        used_provider = ddg.name().to_string();
                        results
                    }
                    Err(e) => {
                        return ToolResult::error("web_search", format!("Search failed: {e}"));
                    }
                }
            }
            Ok(results) => results,
            Err(e) => return ToolResult::error("web_search", format!("Search failed: {e}")),
        };

        let serialized: Vec<WebSearchResult> = results;
        ToolResult::success(
            "web_search",
            serde_json::json!({
                "query": args.query,
                "num_results": serialized.len(),
                "results": serialized,
                "provider": used_provider,
            }),
        )
    }
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

    #[test]
    fn test_web_search_schema() {
        let schema = WebSearchTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "web_search");
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
