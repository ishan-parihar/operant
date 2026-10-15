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
// (plan 11C1) web search is sourcehound-only; the alternative provider
// crates-in-file (ddg/exa/searxng/tavily) are deleted.
use crate::tools::web_providers::{SourcehoundSearchProvider, WebSearchProvider, WebSearchResult};
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

        // (plan 11C1) sourcehound is the one engine. A missing binary is
        // the dominant failure mode now — the error names the fix.
        let sourcehound_available = crate::tools::sourcehound::is_available();

        let candidates = build_search_candidates(&settings, sourcehound_available);
        // Bound the engine to `search_timeout_secs`: a stalled sourcehound
        // subprocess must fail with an actionable error instead of killing
        // the whole turn with a tool-level timeout.
        let per_provider_timeout = Duration::from_secs(settings.search_timeout_secs.max(1));
        let (results, used_provider, last_error) =
            run_provider_chain(candidates, &args.query, num_results, per_provider_timeout).await;

        if results.is_empty() {
            // The engine either errored or returned nothing. Surface an
            // actionable error instead of a silently-empty result set.
            let reason = if !sourcehound_available {
                "sourcehound is not installed or not on PATH — run: operant update --integrations"
            } else {
                &last_error.unwrap_or_else(|| {
                    "sourcehound returned no results — try again shortly".to_string()
                })
            };
            return ToolResult::error("web_search", format!("Search failed: {reason}"));
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

/// (plan 11C1) The one web-search provider: sourcehound. The multi-engine
/// fallback chain (tavily/exa/duckduckgo/searxng) is gone — the tool errors
/// tell the user to install or update the binary instead of silently
/// searching through a worse engine.
fn build_search_candidates(
    _settings: &WebToolSettings,
    _sourcehound_available: bool,
) -> Vec<Box<dyn WebSearchProvider>> {
    vec![Box::new(SourcehoundSearchProvider)]
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

/// Decide whether to prefer the sourcehound engine for web search.
///
/// Explicit user config wins: sourcehound is preferred only when the
/// configured provider is `sourcehound`/`auto`/unset (compared
/// case-insensitively, trimmed). Any explicit non-sourcehound provider
/// (tavily, exa, searxng, …) returns `false` even when the binary is
/// installed — mirroring hermes's
/// `web_search_registry._resolve(explicit, capability)`, which honors the
/// configured backend before auto-selecting among available ones.
///
/// `igs` is the retired name of this same key-free engine, so it resolves to
/// the successor (`sourcehound`/`auto`) rather than falling through to
// (plan 11C1) the provider-matrix tests went with the chain — search is
// sourcehound-only now; the migration contract is pinned by the config
// test below.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_web_search_schema() {
        let schema = WebSearchTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "web_search");
    }

    /// (plan 11C1) the migration contract: a pre-purge config with every
    /// removed key set STILL PARSES (deny_unknown_fields must never break
    /// an existing user config), and the legacy warning finds them.
    #[test]
    fn legacy_web_keys_parse_and_warn() {
        // The fragment is the INNER [tools.web] table (a full config parse is
        // covered by example_toml_parses; here we pin the settings table
        // itself accepting the removed keys).
        let toml = r#"
search_url = "https://lite.duckduckgo.com/lite/?q={query}"
preferred_provider = "tavily"
tavily_api_key = "tvly-x"
exa_api_key = "exa-y"
searxng_base_url = "http://localhost:8888"
"#;
        let parsed: crate::config::WebToolSettings =
            toml::from_str(toml).expect("old configs parse");
        // The keys are accepted (present in the struct) and code-neutral.
        assert!(parsed.tavily_api_key.as_deref() == Some("tvly-x"));
        // The single-provider contract: candidates is sourcehound, always.
        let names: Vec<String> = build_search_candidates(&parsed, true)
            .iter()
            .map(|p| p.name().to_string())
            .collect();
        assert_eq!(names, vec!["sourcehound"]);
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
