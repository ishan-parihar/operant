use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

use crate::schema::ToolSchema;
use crate::security::ssrf_verdict;
use crate::tools::{OperantTool, ToolContext, ToolResult};

const DEFAULT_XAI_BASE_URL: &str = "https://api.x.ai";

pub struct XaiHttpTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct XaiHttpArgs {
    url: String,
    method: Option<String>,
    headers: Option<HashMap<String, String>>,
    body: Option<Value>,
}

#[async_trait]
impl OperantTool for XaiHttpTool {
    fn name(&self) -> &str {
        "xai_http_request"
    }

    fn description(&self) -> &str {
        "Make an HTTP request to the X.AI API (default: https://api.x.ai). \
         Supports overriding the base URL via the XAI_BASE_URL environment variable. \
         Use this for X.AI-specific API endpoints like model inference, embeddings, etc."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<XaiHttpArgs>("xai_http_request", "Make HTTP request to X.AI API")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: XaiHttpArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("xai_http_request", format!("Invalid arguments: {}", e));
            }
        };

        // Determine base URL from env var or default
        let base_url =
            std::env::var("XAI_BASE_URL").unwrap_or_else(|_| DEFAULT_XAI_BASE_URL.to_string());

        // Build the full URL
        let full_url = if args.url.starts_with("http://") || args.url.starts_with("https://") {
            args.url.clone()
        } else {
            let base = base_url.trim_end_matches('/');
            let path = args.url.trim_start_matches('/');
            format!("{}/{}", base, path)
        };

        // SSRF protection: block private/internal addresses (cloud metadata,
        // localhost, RFC 1918, CGNAT, metadata hostnames). Fail-closed on DNS
        // errors — same guard as http_request / web_fetch / web_scrape.
        let (safe, block_msg) = ssrf_verdict(&full_url).await;
        if !safe {
            return ToolResult::error("xai_http_request", block_msg);
        }

        let method = args.method.as_deref().unwrap_or("GET").to_uppercase();

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    "xai_http_request",
                    format!("Failed to create HTTP client: {}", e),
                );
            }
        };

        let mut request = match method.as_str() {
            "GET" => client.get(&full_url),
            "POST" => client.post(&full_url),
            "PUT" => client.put(&full_url),
            "DELETE" => client.delete(&full_url),
            "PATCH" => client.patch(&full_url),
            _ => {
                return ToolResult::error(
                    "xai_http_request",
                    format!("Unsupported HTTP method: {}", method),
                );
            }
        };

        // Add custom headers
        if let Some(ref headers) = args.headers {
            for (key, value) in headers {
                request = request.header(key.as_str(), value.as_str());
            }
        }

        // Add JSON body if present
        if let Some(ref body) = args.body {
            request = request.json(body);
        }

        let start = std::time::Instant::now();

        match request.send().await {
            Ok(response) => {
                let elapsed = start.elapsed();
                let status = response.status().as_u16();

                let response_headers: HashMap<String, String> = response
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();

                let body_text = response.text().await.unwrap_or_default();

                ToolResult::success(
                    "xai_http_request",
                    serde_json::json!({
                        "url": full_url,
                        "method": method,
                        "status_code": status,
                        "headers": response_headers,
                        "body": body_text,
                        "response_time_ms": elapsed.as_millis() as u64
                    }),
                )
            }
            Err(e) => ToolResult::error("xai_http_request", format!("Request failed: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_xai_http_invalid_args() {
        let tool = XaiHttpTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn test_xai_http_url_building() {
        // Test that relative URLs get the base prepended
        // We can't actually make network calls, but we can verify args parsing
        // Set base URL to an invalid address so connection always fails
        // SAFETY: test-only env mutation in #[cfg(test)]
        unsafe { std::env::set_var("XAI_BASE_URL", "http://0.0.0.0:1") };
        let tool = XaiHttpTool;
        let args = serde_json::json!({
            "url": "v1/chat/completions",
            "method": "POST"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        // Args parse fine and the URL is built from the base. The request
        // must never reach the network: 0.0.0.0 is an unspecified address
        // which the SSRF guard blocks pre-flight (fail-closed).
        assert!(!result.success);
        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("URL blocked") || err.contains("URL safety check failed"),
            "expected SSRF pre-flight block, got: {err}"
        );
    }
}
