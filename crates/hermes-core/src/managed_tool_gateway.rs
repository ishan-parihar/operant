//! Managed-tool gateway helpers for Nous-hosted vendor passthroughs.
//!
//! Provides URL construction, token resolution, and HTTP client logic for
//! communicating with managed MCP tool gateways (e.g. Nous Research's
//! vendor-gateway infrastructure).
//!
//! # Example
//!
//! ```ignore
//! use hermes_core::managed_tool_gateway::{GatewayConfig, ManagedToolGateway};
//!
//! let config = GatewayConfig::new("https://acme-gateway.nousresearch.com");
//! let gateway = ManagedToolGateway::new(config);
//!
//! let tools = gateway.list_available_tools().await?;
//! let info = gateway.get_tool_info("web_search").await?;
//! ```

use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;

// -----------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------

/// Default domain for the Nous Research tool gateway.
const DEFAULT_GATEWAY_DOMAIN: &str = "nousresearch.com";

/// Default URL scheme for the tool gateway.
const DEFAULT_GATEWAY_SCHEME: &str = "https";

/// Default HTTP request timeout in seconds.
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

// -----------------------------------------------------------------------
// UrlPattern
// -----------------------------------------------------------------------

/// A URL pattern that can substitute `{base}`, `{tool}`, and `{action}`
/// placeholders.
///
/// Parsed from a template string like `"{base}/api/{tool}/{action}"`.
///
/// # Examples
///
/// ```
/// use hermes_core::managed_tool_gateway::UrlPattern;
///
/// let pattern: UrlPattern = "{base}/api/{tool}/{action}".parse().unwrap();
/// let url = pattern.render("https://gw.example.com", "search", "query");
/// assert_eq!(url, "https://gw.example.com/api/search/query");
/// ```
#[derive(Debug, Clone)]
pub struct UrlPattern {
    /// Segments of the template — either literal text or a placeholder name
    /// (`base`, `tool`, or `action`).
    segments: Vec<TemplateSegment>,
}

#[derive(Debug, Clone)]
enum TemplateSegment {
    Literal(String),
    Placeholder(Placeholder),
}

#[derive(Debug, Clone)]
enum Placeholder {
    Base,
    Tool,
    Action,
}

impl std::str::FromStr for UrlPattern {
    type Err = String;

    fn from_str(template: &str) -> std::result::Result<Self, Self::Err> {
        let mut segments = Vec::new();
        let mut remaining = template;

        while let Some(start) = remaining.find('{') {
            // Push literal text before the placeholder
            if start > 0 {
                segments.push(TemplateSegment::Literal(remaining[..start].to_string()));
            }

            let end = remaining[start..]
                .find('}')
                .ok_or_else(|| format!("Unclosed placeholder in pattern: {template}"))?;
            let placeholder = &remaining[start + 1..start + end];

            let ph = match placeholder {
                "base" => Placeholder::Base,
                "tool" => Placeholder::Tool,
                "action" => Placeholder::Action,
                other => {
                    return Err(format!(
                        "Unknown placeholder '{{{other}}}' in pattern: {template}"
                    ))
                }
            };
            segments.push(TemplateSegment::Placeholder(ph));

            remaining = &remaining[start + end + 1..];
        }

        // Push trailing literal text
        if !remaining.is_empty() {
            segments.push(TemplateSegment::Literal(remaining.to_string()));
        }

        Ok(Self { segments })
    }
}

impl UrlPattern {
    /// Render the URL pattern by substituting the given values.
    ///
    /// * `base`   — Value for `{base}` placeholder (typically the gateway origin).
    /// * `tool`   — Value for `{tool}` placeholder (tool name).
    /// * `action` — Value for `{action}` placeholder (endpoint action).
    pub fn render(&self, base: &str, tool: &str, action: &str) -> String {
        let mut result = String::new();
        for segment in &self.segments {
            match segment {
                TemplateSegment::Literal(lit) => result.push_str(lit),
                TemplateSegment::Placeholder(Placeholder::Base) => result.push_str(base),
                TemplateSegment::Placeholder(Placeholder::Tool) => result.push_str(tool),
                TemplateSegment::Placeholder(Placeholder::Action) => result.push_str(action),
            }
        }
        result
    }
}

// -----------------------------------------------------------------------
// GatewayConfig
// -----------------------------------------------------------------------

/// Configuration for a managed tool gateway connection.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Base URL of the gateway server (e.g. `"https://vendor-gateway.nousresearch.com"`).
    pub base_url: String,
    /// Optional API key for authenticated requests.
    pub api_key: Option<String>,
    /// Optional default model identifier for tool calls that require one.
    pub default_model: Option<String>,
    /// HTTP request timeout in seconds (default: 30).
    pub timeout_seconds: u64,
}

impl GatewayConfig {
    /// Create a new `GatewayConfig` with the given base URL.
    ///
    /// Defaults: no API key, no default model, 30-second timeout.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            default_model: None,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }

    /// Set the API key.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the default model.
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = Some(model.into());
        self
    }

    /// Set the request timeout in seconds.
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }
}

// -----------------------------------------------------------------------
// ManagedToolGateway
// -----------------------------------------------------------------------

/// HTTP client for interacting with a managed tool gateway.
///
/// Provides methods for listing tools, getting tool info, calling tools,
/// checking health, and resolving tokens.
#[derive(Debug, Clone)]
pub struct ManagedToolGateway {
    config: GatewayConfig,
    client: reqwest::Client,
    /// Default URL pattern used by `resolve_url` / `call_tool`.
    url_pattern: UrlPattern,
}

impl ManagedToolGateway {
    /// Create a new gateway client.
    pub fn new(config: GatewayConfig) -> Self {
        let timeout = Duration::from_secs(config.timeout_seconds);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create reqwest::Client");

        // Default pattern: {base}/api/{tool}/{action}
        let url_pattern = "{base}/api/{tool}/{action}"
            .parse()
            .expect("Default URL pattern is valid");

        Self {
            config,
            client,
            url_pattern,
        }
    }

    /// Create a new gateway client with a custom URL pattern.
    ///
    /// The pattern should use `{base}`, `{tool}`, and `{action}` placeholders.
    /// If called with `None`, the default `{base}/api/{tool}/{action}` is used.
    pub fn with_url_pattern(config: GatewayConfig, pattern: Option<UrlPattern>) -> Self {
        let timeout = Duration::from_secs(config.timeout_seconds);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create reqwest::Client");

        let url_pattern = pattern.unwrap_or_else(|| {
            "{base}/api/{tool}/{action}"
                .parse()
                .expect("Default URL pattern is valid")
        });

        Self {
            config,
            client,
            url_pattern,
        }
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Build a fully-qualified gateway URL for the given tool and endpoint.
    ///
    /// Uses the configured URL pattern to substitute `{base}`, `{tool}`,
    /// and `{action}`.
    pub fn resolve_url(&self, tool_name: &str, endpoint: &str) -> String {
        self.url_pattern
            .render(&self.config.base_url, tool_name, endpoint)
    }

    /// Make a generic HTTP call to the gateway.
    ///
    /// * `tool_name` — The tool to target.
    /// * `action`    — The endpoint action (e.g. `"query"`, `"execute"`).
    /// * `params`    — JSON body to send.
    pub async fn call_tool(&self, tool_name: &str, action: &str, params: Value) -> Result<Value> {
        let url = self.resolve_url(tool_name, action);
        let mut req = self.client.post(&url).json(&params);

        if let Some((name, value)) = self.build_auth_header() {
            req = req.header(&name, &value);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to call gateway tool '{tool_name}/{action}'"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Gateway request failed ({} {tool_name}/{action}): {status} — {body}",
                status.as_u16(),
            );
        }

        let text = resp
            .text()
            .await
            .with_context(|| format!("Failed to read gateway response body for '{tool_name}/{action}'"))?;

        if text.trim().is_empty() {
            anyhow::bail!("Gateway tool '{tool_name}/{action}' returned an empty response body");
        }

        serde_json::from_str(&text)
            .with_context(|| format!("Failed to parse gateway response for '{tool_name}/{action}': {text}"))
    }

    /// List all available tools from the gateway.
    ///
    /// GET `{base}/api/tools` — returns a list of tool names.
    pub async fn list_available_tools(&self) -> Result<Vec<String>> {
        let url = format!("{}/tools", self.config.base_url.trim_end_matches('/'));
        let mut req = self.client.get(&url);

        if let Some((name, value)) = self.build_auth_header() {
            req = req.header(&name, &value);
        }

        let resp = req
            .send()
            .await
            .context("Failed to fetch tool list from gateway")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to list tools: {status} — {body}");
        }

        let value: Value = resp.json().await.context("Failed to parse tool list")?;

        // Try common response shapes: { "tools": [...] } or just [...]
        let names: Vec<String> = if let Some(tools) = value.get("tools").and_then(|v| v.as_array())
        {
            tools
                .iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        } else if let Some(arr) = value.as_array() {
            arr.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        } else {
            anyhow::bail!(
                "Unexpected tool list format: expected array under 'tools' key or top-level array"
            );
        };

        if names.is_empty() {
            anyhow::bail!("Empty tool list in gateway response");
        }

        Ok(names)
    }

    /// Get detailed information about a specific tool.
    ///
    /// GET `{tools_url}/{tool_name}`
    pub async fn get_tool_info(&self, tool_name: &str) -> Result<Value> {
        let base = self.config.base_url.trim_end_matches('/');
        let url = format!("{base}/tools/{tool_name}");
        let mut req = self.client.get(&url);

        if let Some((name, value)) = self.build_auth_header() {
            req = req.header(&name, &value);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to fetch info for tool '{tool_name}'"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get tool info for '{tool_name}': {status} — {body}");
        }

        resp.json()
            .await
            .with_context(|| format!("Failed to parse tool info for '{tool_name}'"))
    }

    /// Check the health of the gateway server.
    ///
    /// GET `{base}/health` — returns `true` if the server responds with
    /// a 2xx status.
    pub async fn check_health(&self) -> Result<bool> {
        let url = format!("{}/health", self.config.base_url.trim_end_matches('/'));
        let mut req = self.client.get(&url);

        if let Some((name, value)) = self.build_auth_header() {
            req = req.header(&name, &value);
        }

        let resp = req.send().await.context("Health check request failed")?;
        Ok(resp.status().is_success())
    }

    /// Resolve an authentication token by name.
    ///
    /// Checks environment variables first (`{NAME}_TOKEN`, `{NAME}_API_KEY`),
    /// then falls back to the configured `api_key` if the token name is
    /// `"default"` or `"api"`.
    pub fn resolve_token(&self, token_name: &str) -> Result<String> {
        // Check env var: {UPPER_NAME}_TOKEN
        let env_key = format!("{}_TOKEN", token_name.to_uppercase().replace('-', "_"));
        if let Ok(val) = std::env::var(&env_key) {
            if !val.trim().is_empty() {
                return Ok(val.trim().to_string());
            }
        }

        // Check env var: {UPPER_NAME}_API_KEY
        let env_key = format!("{}_API_KEY", token_name.to_uppercase().replace('-', "_"));
        if let Ok(val) = std::env::var(&env_key) {
            if !val.trim().is_empty() {
                return Ok(val.trim().to_string());
            }
        }

        // Fall back to configured api_key for well-known names
        if token_name.eq_ignore_ascii_case("default") || token_name.eq_ignore_ascii_case("api") {
            if let Some(ref key) = self.config.api_key {
                return Ok(key.clone());
            }
        }

        anyhow::bail!(
            "Token '{}' not found in env vars ({env_key}_TOKEN, {env_key}_API_KEY) or config",
            token_name,
        )
    }

    /// Build the authentication header for gateway requests.
    ///
    /// Returns `Some(("header_name", "header_value"))` if an API key
    /// is configured in the gateway config, otherwise `None`.
    ///
    /// The default header name is `"Authorization"` with `"Bearer {key}"`.
    pub fn build_auth_header(&self) -> Option<(String, String)> {
        self.config
            .api_key
            .as_ref()
            .map(|key| ("Authorization".to_string(), format!("Bearer {key}")))
    }
}

// -----------------------------------------------------------------------
// Free-standing helpers
// -----------------------------------------------------------------------

/// Build a vendor-specific gateway URL.
///
/// # Logic
///
/// 1. If the env var `{VENDOR}_GATEWAY_URL` is set, use it directly.
/// 2. Otherwise construct `{scheme}://{vendor}-gateway.{domain}`.
///
/// The scheme defaults to `https`; the domain defaults to `nousresearch.com`.
/// Both can be overridden via `TOOL_GATEWAY_SCHEME` and `TOOL_GATEWAY_DOMAIN`
/// environment variables.
pub fn build_vendor_gateway_url(vendor: &str) -> String {
    let vendor_upper = vendor.to_uppercase().replace('-', "_");
    let explicit_key = format!("{vendor_upper}_GATEWAY_URL");

    if let Ok(url) = std::env::var(&explicit_key) {
        let trimmed = url.trim().trim_end_matches('/').to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    let scheme = std::env::var("TOOL_GATEWAY_SCHEME")
        .ok()
        .and_then(|s| {
            let s = s.trim().to_lowercase();
            if s == "http" || s == "https" {
                Some(s)
            } else {
                None
            }
        })
        .unwrap_or_else(|| DEFAULT_GATEWAY_SCHEME.to_string());

    let domain = std::env::var("TOOL_GATEWAY_DOMAIN")
        .ok()
        .map(|d| d.trim().trim_matches('/').to_string())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| DEFAULT_GATEWAY_DOMAIN.to_string());

    format!("{scheme}://{vendor}-gateway.{domain}")
}

// -----------------------------------------------------------------------
// Nous token resolution
// -----------------------------------------------------------------------

/// Environment variable that can override the Nous access token.
const TOOL_GATEWAY_USER_TOKEN_ENV: &str = "TOOL_GATEWAY_USER_TOKEN";

/// Skew (in seconds) before token expiry to consider it expiring.
const NOUS_ACCESS_TOKEN_REFRESH_SKEW_SECONDS: i64 = 120;

/// Read the Nous Subscriber OAuth access token from the environment or
/// the configured API key on the gateway.
///
/// Checks `TOOL_GATEWAY_USER_TOKEN` env var first, then falls back to
/// the config's `api_key`.
pub fn read_nous_access_token(gateway: &ManagedToolGateway) -> Result<String> {
    // 1. Check env var
    if let Ok(token) = std::env::var(TOOL_GATEWAY_USER_TOKEN_ENV) {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    // 2. Fall back to gateway's configured API key
    gateway.config.api_key.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "No Nous access token available: {} not set and no api_key configured",
            TOOL_GATEWAY_USER_TOKEN_ENV
        )
    })
}

/// Resolve a full `ManagedToolGateway` configuration for a given vendor.
///
/// Returns `None` if the gateway URL cannot be built or the token is
/// unavailable.
pub fn resolve_managed_tool_gateway(
    vendor: &str,
    api_key: Option<&str>,
    gateway_builder: Option<fn(&str) -> String>,
) -> Option<ManagedToolGateway> {
    let builder = gateway_builder.unwrap_or(build_vendor_gateway_url);
    let gateway_url = builder(vendor);

    if gateway_url.is_empty() {
        return None;
    }

    let mut config = GatewayConfig::new(&gateway_url);
    if let Some(key) = api_key {
        config = config.with_api_key(key);
    }

    let gateway = ManagedToolGateway::new(config);

    // Verify token availability
    if read_nous_access_token(&gateway).is_err() {
        // If no api_key was explicitly passed, check env one more time
        if api_key.is_none() && std::env::var(TOOL_GATEWAY_USER_TOKEN_ENV).is_err() {
            return None;
        }
    }

    Some(gateway)
}

/// Check whether the managed tool gateway is ready for a given vendor.
pub fn is_managed_tool_gateway_ready(
    vendor: &str,
    api_key: Option<&str>,
    gateway_builder: Option<fn(&str) -> String>,
) -> bool {
    resolve_managed_tool_gateway(vendor, api_key, gateway_builder).is_some()
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    // ---- UrlPattern tests ----

    #[test]
    fn test_url_pattern_default() {
        let pattern: UrlPattern = "{base}/api/{tool}/{action}".parse().unwrap();
        let url = pattern.render("https://gw.example.com", "search", "query");
        assert_eq!(url, "https://gw.example.com/api/search/query");
    }

    #[test]
    fn test_url_pattern_no_placeholders() {
        let pattern: UrlPattern = "https://fixed.example.com/endpoint".parse().unwrap();
        let url = pattern.render("ignored", "ignored", "ignored");
        assert_eq!(url, "https://fixed.example.com/endpoint");
    }

    #[test]
    fn test_url_pattern_unknown_placeholder() {
        let result: std::result::Result<UrlPattern, String> = "{base}/api/{unknown}".parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown"));
    }

    // ---- GatewayConfig tests ----

    #[test]
    fn test_gateway_config_defaults() {
        let config = GatewayConfig::new("https://test.example.com");
        assert_eq!(config.base_url, "https://test.example.com");
        assert!(config.api_key.is_none());
        assert_eq!(config.timeout_seconds, 30);
    }

    #[test]
    fn test_gateway_config_builder() {
        let config = GatewayConfig::new("https://test.example.com")
            .with_api_key("key-123")
            .with_default_model("gpt-4")
            .with_timeout(60);

        assert_eq!(config.api_key.unwrap(), "key-123");
        assert_eq!(config.default_model.unwrap(), "gpt-4");
        assert_eq!(config.timeout_seconds, 60);
    }

    // ---- ManagedToolGateway tests ----

    fn mock_gateway(server: &Server) -> ManagedToolGateway {
        let config = GatewayConfig::new(server.url()).with_api_key("test-token");
        ManagedToolGateway::new(config)
    }

    #[tokio::test]
    async fn test_check_health_ok() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/health")
            .with_status(200)
            .with_body("OK")
            .create();

        let gateway = mock_gateway(&server);
        let healthy = gateway.check_health().await.unwrap();
        assert!(healthy);
        mock.assert();
    }

    #[tokio::test]
    async fn test_check_health_failure() {
        let mut server = Server::new_async().await;
        let mock = server.mock("GET", "/health").with_status(503).create();

        let gateway = mock_gateway(&server);
        let healthy = gateway.check_health().await.unwrap();
        assert!(!healthy);
        mock.assert();
    }

    #[tokio::test]
    async fn test_list_available_tools() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/tools")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tools":["web_search","web_fetch","crawl"]}"#)
            .create();

        let gateway = mock_gateway(&server);
        let tools = gateway.list_available_tools().await.unwrap();
        assert_eq!(tools.len(), 3);
        assert!(tools.contains(&"web_search".to_string()));
        mock.assert();
    }

    #[tokio::test]
    async fn test_call_tool() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/web_search/query")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":"success"}"#)
            .create();

        let gateway = mock_gateway(&server);
        let result = gateway
            .call_tool("web_search", "query", serde_json::json!({"q": "hello"}))
            .await
            .unwrap();

        assert_eq!(result["result"], "success");
        mock.assert();
    }

    #[tokio::test]
    async fn test_call_tool_auth_header() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/test/run")
            .match_header("Authorization", "Bearer test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .create();

        let gateway = mock_gateway(&server);
        let result = gateway
            .call_tool("test", "run", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(result["ok"], true);
        mock.assert();
    }

    // ---- Helper function tests ----

    #[test]
    fn test_resolve_token_fallback_to_config() {
        let config = GatewayConfig::new("https://example.com").with_api_key("cfg-key");
        let gateway = ManagedToolGateway::new(config);
        let token = gateway.resolve_token("default").unwrap();
        assert_eq!(token, "cfg-key");
    }

    #[test]
    fn test_resolve_token_not_found() {
        let config = GatewayConfig::new("https://example.com");
        let gateway = ManagedToolGateway::new(config);
        let err = gateway.resolve_token("nonexistent").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
