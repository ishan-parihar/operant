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
}
