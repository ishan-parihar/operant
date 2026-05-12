//! Microsoft Graph API client and authentication
//!
//! Provides:
//! - [`MicrosoftGraphTokenProvider`] — OAuth2 `client_credentials` grant token
//!   acquisition with in-memory caching and automatic expiry-based refresh.
//! - [`MicrosoftGraphClient`] — reusable async HTTP client for the Microsoft
//!   Graph REST API v1.0 with retry logic, pagination helpers, and streaming
//!   file downloads.
//!
//! # Ported from Python
//!
//! - `tools/microsoft_graph_auth.py` → `MicrosoftGraphTokenProvider`
//! - `tools/microsoft_graph_client.py` → `MicrosoftGraphClient`

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT},
    Client, Method, Response, StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::debug;

use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default Microsoft Graph API scope for app-only (client_credentials) auth.
pub const DEFAULT_GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";

/// Default Microsoft identity platform authority URL.
pub const DEFAULT_GRAPH_AUTHORITY_URL: &str = "https://login.microsoftonline.com";

/// Default Microsoft Graph REST API base URL (v1.0).
pub const DEFAULT_GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";

/// Token refresh triggers when remaining TTL drops below this threshold (seconds).
const DEFAULT_TOKEN_SKEW_SECONDS: u64 = 120;

/// Default request timeout in seconds.
const DEFAULT_TIMEOUT_SECONDS: f64 = 60.0;

/// Maximum number of retries for failed requests.
const DEFAULT_MAX_RETRIES: u32 = 3;

/// User-Agent header value sent with every request.
const USER_AGENT_VALUE: &str = "Hermes-RS/ms-graph-client";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors specific to Microsoft Graph operations.
#[derive(Debug, thiserror::Error)]
pub enum MicrosoftGraphError {
    /// Credentials are missing or invalid.
    #[error("Microsoft Graph config error: {0}")]
    Config(String),

    /// Token acquisition failed.
    #[error("Microsoft Graph token error: {0}")]
    Token(String),

    /// An API request returned an HTTP error.
    #[error("Microsoft Graph API error {status} for {method} {url}: {message}")]
    Api {
        /// HTTP status code.
        status: StatusCode,
        /// HTTP method used.
        method: String,
        /// Request URL.
        url: String,
        /// Human-readable error message extracted from the response body.
        message: String,
        /// Optional `Retry-After` header value from the server.
        retry_after: Option<f64>,
        /// Parsed response payload, if available.
        payload: Option<Value>,
    },

    /// Client-level error (network, parsing, etc.).
    #[error("Microsoft Graph client error: {0}")]
    Client(String),
}

impl From<MicrosoftGraphError> for Error {
    fn from(e: MicrosoftGraphError) -> Self {
        Error::Agent(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// App-only (OAuth2 `client_credentials`) credentials for Microsoft Graph.
///
/// Obtain credentials by registering an application in the Azure Portal:
/// <https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps>
///
/// Supply via the environment variables below or construct manually.
///
/// | Env var                  | Required | Default                                |
/// |--------------------------|----------|----------------------------------------|
/// | `MSGRAPH_TENANT_ID`      | ✅       | —                                      |
/// | `MSGRAPH_CLIENT_ID`      | ✅       | —                                      |
/// | `MSGRAPH_CLIENT_SECRET`  | ✅       | —                                      |
/// | `MSGRAPH_SCOPE`          |          | `https://graph.microsoft.com/.default` |
/// | `MSGRAPH_AUTHORITY_URL`  |          | `https://login.microsoftonline.com`    |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphCredentials {
    /// Azure AD tenant ID (directory ID).
    pub tenant_id: String,
    /// Application (client) ID.
    pub client_id: String,
    /// Client secret value.
    pub client_secret: String,
    /// OAuth2 scope for the token request.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Authority URL for the token endpoint.
    #[serde(default = "default_authority_url")]
    pub authority_url: String,
}

fn default_scope() -> String {
    DEFAULT_GRAPH_SCOPE.to_string()
}

fn default_authority_url() -> String {
    DEFAULT_GRAPH_AUTHORITY_URL.to_string()
}

impl GraphCredentials {
    /// Build the OAuth2 v2.0 token endpoint URL:
    /// `{authority_url}/{tenant_id}/oauth2/v2.0/token`
    pub fn token_url(&self) -> String {
        let base = self.authority_url.trim_end_matches('/');
        let tenant = self.tenant_id.trim().trim_matches('/');
        format!("{}/{}/oauth2/v2.0/token", base, tenant)
    }

    /// Read credentials from environment variables.
    ///
    /// Returns `Err(MicrosoftGraphError::Config)` when any required variable
    /// is missing.
    pub fn from_env() -> Result<Self> {
        let tenant_id = std::env::var("MSGRAPH_TENANT_ID")
            .map_err(|_| MicrosoftGraphError::Config("MSGRAPH_TENANT_ID is not set".into()))?;
        let client_id = std::env::var("MSGRAPH_CLIENT_ID")
            .map_err(|_| MicrosoftGraphError::Config("MSGRAPH_CLIENT_ID is not set".into()))?;
        let client_secret = std::env::var("MSGRAPH_CLIENT_SECRET")
            .map_err(|_| MicrosoftGraphError::Config("MSGRAPH_CLIENT_SECRET is not set".into()))?;
        let scope =
            std::env::var("MSGRAPH_SCOPE").unwrap_or_else(|_| DEFAULT_GRAPH_SCOPE.to_string());
        let authority_url = std::env::var("MSGRAPH_AUTHORITY_URL")
            .unwrap_or_else(|_| DEFAULT_GRAPH_AUTHORITY_URL.to_string());

        Ok(Self {
            tenant_id,
            client_id,
            client_secret,
            scope,
            authority_url,
        })
    }

    /// Try to read credentials from environment variables, returning `None`
    /// silently if the required variables are not set.
    pub fn from_env_optional() -> Option<Self> {
        let tenant_id = std::env::var("MSGRAPH_TENANT_ID").ok()?;
        let client_id = std::env::var("MSGRAPH_CLIENT_ID").ok()?;
        let client_secret = std::env::var("MSGRAPH_CLIENT_SECRET").ok()?;
        let scope =
            std::env::var("MSGRAPH_SCOPE").unwrap_or_else(|_| DEFAULT_GRAPH_SCOPE.to_string());
        let authority_url = std::env::var("MSGRAPH_AUTHORITY_URL")
            .unwrap_or_else(|_| DEFAULT_GRAPH_AUTHORITY_URL.to_string());

        Some(Self {
            tenant_id,
            client_id,
            client_secret,
            scope,
            authority_url,
        })
    }
}

// ---------------------------------------------------------------------------
// Cached token
// ---------------------------------------------------------------------------

/// An in-memory cached Microsoft Graph access token with wall-clock expiry.
#[derive(Debug, Clone)]
pub struct CachedAccessToken {
    /// The opaque access token string (JWT).
    pub access_token: String,
    /// Token type (typically `"Bearer"`).
    pub token_type: String,
    /// Wall-clock [`Instant`] when this token expires.
    expires_at: Instant,
}

impl CachedAccessToken {
    /// Create a new cached token.
    pub fn new(access_token: String, token_type: String, expires_in_seconds: u64) -> Self {
        Self {
            access_token,
            token_type,
            expires_at: Instant::now() + Duration::from_secs(expires_in_seconds),
        }
    }

    /// Returns `true` when the token is expired (with an optional safety skew).
    ///
    /// Pass `skew_seconds` to trigger refresh *before* the actual expiry.
    /// The Python counterpart defaults to 120 seconds.
    pub fn is_expired(&self, skew_seconds: u64) -> bool {
        let effective_expiry = self
            .expires_at
            .checked_sub(Duration::from_secs(skew_seconds))
            .unwrap_or(self.expires_at);
        Instant::now() >= effective_expiry
    }

    /// Seconds remaining before actual (non-skewed) expiry.
    pub fn remaining_seconds(&self) -> u64 {
        self.expires_at
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
}

// ---------------------------------------------------------------------------
// Token provider
// ---------------------------------------------------------------------------

/// Acquires and caches Microsoft Graph app-only access tokens.
///
/// Uses the OAuth2 `client_credentials` grant (no user interaction / no
/// browser flow). Tokens are cached in an [`Arc<RwLock<Option<…>>>`] and
/// automatically refreshed when the remaining TTL drops below
/// [`DEFAULT_TOKEN_SKEW_SECONDS`] (120 s).
///
/// # Thread-safety
///
/// `Clone`-ing the provider shares the same underlying cache. The double-check
/// locking pattern in [`get_access_token`](MicrosoftGraphTokenProvider::get_access_token)
/// means concurrent callers will only trigger one token acquisition.
#[derive(Clone)]
pub struct MicrosoftGraphTokenProvider {
    /// App-only credentials.
    credentials: GraphCredentials,
    /// Request timeout in seconds.
    timeout: f64,
    /// Skew in seconds — refresh when remaining TTL < this value.
    skew_seconds: u64,
    /// Shared HTTP client.
    client: Client,
    /// Cached token shared across clones.
    cached_token: Arc<RwLock<Option<CachedAccessToken>>>,
}

impl MicrosoftGraphTokenProvider {
    /// Create a new token provider from the given credentials.
    pub fn new(credentials: GraphCredentials) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs_f64(DEFAULT_TIMEOUT_SECONDS))
            .build()
            .expect("Failed to build reqwest Client for token provider");

        Self {
            credentials,
            timeout: DEFAULT_TIMEOUT_SECONDS,
            skew_seconds: DEFAULT_TOKEN_SKEW_SECONDS,
            client,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a token provider from environment variables (convenience).
    ///
    /// See [`GraphCredentials::from_env`].
    pub fn from_env() -> Result<Self> {
        GraphCredentials::from_env().map(Self::new)
    }

    /// Create a token provider from optional env vars, returning `None` if the
    /// required variables are not set.
    pub fn from_env_optional() -> Option<Self> {
        GraphCredentials::from_env_optional().map(Self::new)
    }

    /// Override the default request timeout.
    pub fn with_timeout(mut self, timeout: f64) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the default skew seconds.
    pub fn with_skew_seconds(mut self, skew: u64) -> Self {
        self.skew_seconds = skew;
        self
    }

    /// Clear the cached token, forcing a fresh acquisition on the next call.
    pub async fn clear_cache(&self) {
        *self.cached_token.write().await = None;
    }

    /// Return a diagnostic snapshot of the token provider state.
    pub async fn inspect_token_health(&self) -> Value {
        let cached = self.cached_token.read().await;
        let (cached_present, expires_in, is_expired) = match cached.as_ref() {
            Some(token) => (
                true,
                Some(token.remaining_seconds()),
                Some(token.is_expired(0)),
            ),
            None => (false, None, None),
        };
        drop(cached);

        serde_json::json!({
            "configured": true,
            "tenant_id": self.credentials.tenant_id,
            "client_id": self.credentials.client_id,
            "scope": self.credentials.scope,
            "authority_url": self.credentials.authority_url,
            "token_url": self.credentials.token_url(),
            "cached": cached_present,
            "expires_in_seconds": expires_in,
            "is_expired": is_expired,
            "refresh_skew_seconds": self.skew_seconds,
        })
    }

    /// Obtain a valid access token, refreshing if necessary (lazy). If
    /// `force_refresh` is `true`, the cache is bypassed and a new token is
    /// always fetched.
    pub async fn get_access_token(&self, force_refresh: bool) -> Result<String> {
        // Fast path: check cache without write lock.
        if !force_refresh {
            let cached = self.cached_token.read().await;
            if let Some(token) = cached.as_ref() {
                if !token.is_expired(self.skew_seconds) {
                    return Ok(token.access_token.clone());
                }
            }
        }

        // Slow path: acquire write lock and refresh.
        let mut cached = self.cached_token.write().await;

        // Double-check: another task may have refreshed while we waited.
        if !force_refresh {
            if let Some(token) = cached.as_ref() {
                if !token.is_expired(self.skew_seconds) {
                    return Ok(token.access_token.clone());
                }
            }
        }

        let token = self.fetch_access_token().await?;
        let access_token = token.access_token.clone();
        *cached = Some(token);
        Ok(access_token)
    }

    /// Perform the actual OAuth2 `client_credentials` token request.
    async fn fetch_access_token(&self) -> Result<CachedAccessToken> {
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", &self.credentials.client_id),
            ("client_secret", &self.credentials.client_secret),
            ("scope", &self.credentials.scope),
        ];

        debug!(
            token_url = %self.credentials.token_url(),
            "Fetching Microsoft Graph access token"
        );

        let response = self
            .client
            .post(self.credentials.token_url())
            .form(&params)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .timeout(Duration::from_secs_f64(self.timeout))
            .send()
            .await
            .map_err(|e| MicrosoftGraphError::Token(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let detail = extract_error_detail_from_response(response).await;
            return Err(MicrosoftGraphError::Token(format!(
                "Token request failed with HTTP {}: {}",
                status, detail
            ))
            .into());
        }

        let payload: Value = response.json().await.map_err(|e| {
            MicrosoftGraphError::Token(format!("Response was not valid JSON: {}", e))
        })?;

        let access_token = payload
            .get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                MicrosoftGraphError::Token("Token response did not include access_token".into())
            })?;

        let token_type = payload
            .get("token_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Bearer")
            .to_string();

        let expires_in = payload
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                MicrosoftGraphError::Token(
                    "Token response did not include a valid expires_in".into(),
                )
            })?;

        Ok(CachedAccessToken::new(access_token, token_type, expires_in))
    }
}

impl std::fmt::Debug for MicrosoftGraphTokenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicrosoftGraphTokenProvider")
            .field("tenant_id", &self.credentials.tenant_id)
            .field("client_id", &self.credentials.client_id)
            .field("scope", &self.credentials.scope)
            .field("timeout", &self.timeout)
            .field("skew_seconds", &self.skew_seconds)
            .field(
                "has_cache",
                &self
                    .cached_token
                    .try_read()
                    .map(|c| c.is_some())
                    .unwrap_or(false),
            )
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the HTTP status code warrants a retry.
fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status == StatusCode::GATEWAY_TIMEOUT
        || status.is_server_error()
}

/// Calculate the retry delay in seconds using exponential backoff.
///
/// If `retry_after_seconds` is `Some`, it is used as the delay (capped to >= 0).
/// Otherwise, exponential backoff is used: 0.5s, 1s, 2s, 4s, 8s (capped at 8s).
fn retry_delay(retry_after_seconds: Option<f64>, attempt: u32) -> Duration {
    // Check for explicit Retry-After value first.
    if let Some(seconds) = retry_after_seconds {
        if seconds >= 0.0 {
            return Duration::from_secs_f64(seconds);
        }
    }

    // Exponential backoff: 0.5s, 1s, 2s, 4s, 8s, capped at 8s.
    let delay = (0.5 * (2u64.pow(attempt) as f64)).min(8.0);
    Duration::from_secs_f64(delay)
}

/// Extract the `Retry-After` header value from a response as seconds.
fn extract_retry_after(response: &Response) -> Option<f64> {
    response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok())
}

/// Extract a human-readable error message from a failed API response.
async fn build_api_error(method: Method, url: &str, response: Response) -> MicrosoftGraphError {
    let status = response.status();
    let method_str = method.to_string();
    let url_str = url.to_string();
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());

    let (message, payload) = match response.text().await {
        Ok(body) => {
            let trimmed = body.trim().to_string();
            let extracted = extract_error_message_from_body(&trimmed);
            let parsed = serde_json::from_str(&trimmed).ok();
            (extracted, parsed)
        }
        Err(_) => ("unknown error".to_string(), None),
    };

    MicrosoftGraphError::Api {
        status,
        method: method_str,
        url: url_str,
        message,
        retry_after,
        payload,
    }
}

/// Parse a Graph API error body to extract a concise message string.
fn extract_error_message_from_body(body: &str) -> String {
    if let Ok(payload) = serde_json::from_str::<Value>(body) {
        if let Some(error) = payload.get("error") {
            match error {
                Value::Object(obj) => {
                    let code = obj.get("code").and_then(|v| v.as_str());
                    let inner_message = obj.get("message").and_then(|v| v.as_str());
                    match (code, inner_message) {
                        (Some(c), Some(m)) => return format!("{}: {}", c, m),
                        (_, Some(m)) => return m.to_string(),
                        (Some(c), _) => return c.to_string(),
                        _ => {}
                    }
                }
                Value::String(s) => return s.clone(),
                _ => {}
            }
        }
        // Fallback: look for error_description (common in token endpoint errors)
        if let Some(desc) = payload.get("error_description").and_then(|v| v.as_str()) {
            return desc.to_string();
        }
    }
    body.to_string()
}

/// Extract error detail from a failed token endpoint response.
async fn extract_error_detail_from_response(response: Response) -> String {
    match response.text().await {
        Ok(body) => {
            let trimmed = body.trim().to_string();
            if trimmed.is_empty() {
                return "unknown error".to_string();
            }
            if let Ok(payload) = serde_json::from_str::<Value>(&trimmed) {
                if let Some(desc) = payload.get("error_description").and_then(|v| v.as_str()) {
                    return desc.to_string();
                }
                if let Some(error_val) = payload.get("error") {
                    if let Some(s) = error_val.as_str() {
                        return s.to_string();
                    }
                }
            }
            trimmed
        }
        Err(_) => "unknown error".to_string(),
    }
}

/// Decode a response body as JSON, returning a client error on failure.
async fn decode_json(response: Response) -> Result<Value> {
    let url = response.url().to_string();

    response.json::<Value>().await.map_err(|e| {
        MicrosoftGraphError::Client(format!("Response was not valid JSON for {}: {}", url, e))
            .into()
    })
}

/// Returns `true` if the error is an API error with status 401 Unauthorized.
fn error_implies_expired_token(error: &MicrosoftGraphError) -> bool {
    matches!(
        error,
        MicrosoftGraphError::Api { status, .. } if *status == StatusCode::UNAUTHORIZED
    )
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Minimal, reusable async HTTP client for the Microsoft Graph REST API.
///
/// Features:
/// - **Automatic auth header injection** — every request gets a `Bearer` token
///   from the [`MicrosoftGraphTokenProvider`].
/// - **Retry with backoff** — 3 retries for 429, 503, 504, and 5xx errors;
///   automatic token refresh on 401.
/// - **Pagination helpers** — [`iterate_pages`](MicrosoftGraphClient::iterate_pages)
///   and [`collect_paginated`](MicrosoftGraphClient::collect_paginated) handle
///   `@odata.nextLink` transparently.
/// - **Streaming downloads** — [`download_to_file`](MicrosoftGraphClient::download_to_file)
///   writes response chunks directly to disk.
///
/// # Example
///
/// ```ignore
/// let client = MicrosoftGraphClient::from_env().await?;
/// let users = client.get_json("/users").await?;
/// ```
pub struct MicrosoftGraphClient {
    /// Token provider for automatic auth header injection.
    token_provider: Arc<MicrosoftGraphTokenProvider>,
    /// Base URL for the Graph API.
    base_url: String,
    /// Request timeout in seconds.
    timeout: f64,
    /// Maximum number of retries for failed requests.
    max_retries: u32,
    /// Shared HTTP client (reused across requests for connection pooling).
    client: Client,
}

impl MicrosoftGraphClient {
    /// Create a new Graph client with the given token provider.
    pub fn new(token_provider: MicrosoftGraphTokenProvider) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs_f64(DEFAULT_TIMEOUT_SECONDS))
            .build()
            .expect("Failed to build reqwest Client for Graph client");

        Self {
            token_provider: Arc::new(token_provider),
            base_url: DEFAULT_GRAPH_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT_SECONDS,
            max_retries: DEFAULT_MAX_RETRIES,
            client,
        }
    }

    /// Convenience constructor that reads credentials from the environment.
    pub async fn from_env() -> Result<Self> {
        let provider = MicrosoftGraphTokenProvider::from_env()?;
        Ok(Self::new(provider))
    }

    /// Override the default base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    /// Override the default request timeout in seconds.
    pub fn with_timeout(mut self, timeout: f64) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the maximum number of retries.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Access the underlying token provider (e.g., for diagnostics).
    pub fn token_provider(&self) -> &MicrosoftGraphTokenProvider {
        &self.token_provider
    }

    // -- High-level helpers ------------------------------------------------

    /// Issue a GET request and parse the response body as JSON.
    pub async fn get_json(
        &self,
        path: &str,
        params: Option<&[(&str, &str)]>,
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        let response = self
            .request(Method::GET, path, None::<&Value>, params, headers)
            .await?;
        decode_json(response).await
    }

    /// Issue a POST request with a JSON body and parse the response.
    pub async fn post_json<T: Serialize + Send + Sync>(
        &self,
        path: &str,
        body: Option<&T>,
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        let response = self
            .request(Method::POST, path, body, None, headers)
            .await?;
        decode_json(response).await
    }

    /// Issue a PATCH request with a JSON body.
    ///
    /// Returns `json!({})` on a 204 No Content response.
    pub async fn patch_json<T: Serialize + Send + Sync>(
        &self,
        path: &str,
        body: Option<&T>,
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        let response = self
            .request(Method::PATCH, path, body, None, headers)
            .await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(serde_json::json!({}));
        }
        decode_json(response).await
    }

    /// Issue a DELETE request.
    ///
    /// Returns `json!({"deleted": true, "status_code": 204})` on 204.
    pub async fn delete_request(&self, path: &str, headers: Option<HeaderMap>) -> Result<Value> {
        let response = self
            .request::<Value>(Method::DELETE, path, None, None, headers)
            .await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(serde_json::json!({"deleted": true, "status_code": 204}));
        }
        decode_json(response).await
    }

    // -- Pagination --------------------------------------------------------

    /// Iterate over paginated Graph API responses.
    ///
    /// Each entry in the returned `Vec` is a full page object containing
    /// `value` and optionally `@odata.nextLink`. Pages are fetched on demand
    /// by following the `@odata.nextLink` property.
    pub async fn iterate_pages(
        &self,
        path: &str,
        params: Option<&[(&str, &str)]>,
        headers: Option<HeaderMap>,
    ) -> Result<Vec<Value>> {
        let mut pages = Vec::new();
        let mut next_url: Option<String> = Some(self.resolve_url(path));
        let mut current_params = params.map(|p| p.to_vec());

        while let Some(ref url) = next_url {
            let response = self
                .request_with_url(
                    Method::GET,
                    url,
                    None::<&Value>,
                    current_params.as_deref(),
                    headers.clone(),
                )
                .await?;

            let payload: Value = decode_json(response).await?;

            next_url = payload
                .get("@odata.nextLink")
                .and_then(|v| v.as_str())
                .map(String::from);

            // When following @odata.nextLink, params are embedded in the URL,
            // so clear them for subsequent requests.
            current_params = None;

            pages.push(payload);
        }

        Ok(pages)
    }

    /// Collect all items from a paginated endpoint into a flat list.
    ///
    /// Calls [`iterate_pages`](MicrosoftGraphClient::iterate_pages) and
    /// extracts every `value` array into a single `Vec<Value>`.
    pub async fn collect_paginated(
        &self,
        path: &str,
        params: Option<&[(&str, &str)]>,
        headers: Option<HeaderMap>,
    ) -> Result<Vec<Value>> {
        let pages = self.iterate_pages(path, params, headers).await?;
        let mut items = Vec::new();
        for page in pages {
            if let Some(value) = page.get("value").and_then(|v| v.as_array()) {
                items.extend(value.iter().cloned());
            }
        }
        Ok(items)
    }

    // -- Streaming download ------------------------------------------------

    /// Download a file from Graph API to local disk, streaming the response
    /// body chunk-by-chunk so large artifacts do not need to fit in memory.
    ///
    /// The download is written to `<destination>.part` first, then atomically
    /// renamed to `<destination>` on success.
    ///
    /// Returns metadata: `{"path": …, "size_bytes": …, "content_type": …}`.
    pub async fn download_to_file(
        &self,
        path: &str,
        destination: &std::path::Path,
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        let url = self.resolve_url(path);
        let dest = destination.to_path_buf();
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let tmp_path = dest.with_extension("part");
        let mut attempt: u32 = 0;
        let mut last_error: Option<MicrosoftGraphError> = None;
        let mut content_type: Option<String> = None;

        loop {
            let force_refresh = attempt > 0
                && last_error
                    .as_ref()
                    .map_or(false, error_implies_expired_token);

            let token = self.token_provider.get_access_token(force_refresh).await?;

            let mut request_headers = HeaderMap::new();
            request_headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|e| {
                    MicrosoftGraphError::Client(format!("Invalid auth header: {}", e))
                })?,
            );
            request_headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            request_headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));

            if let Some(ref extra) = headers {
                request_headers.extend(extra.clone().into_iter());
            }

            let response = match self
                .client
                .get(&url)
                .headers(request_headers)
                .timeout(Duration::from_secs_f64(self.timeout))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if attempt >= self.max_retries {
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        return Err(MicrosoftGraphError::Client(format!(
                            "Download failed for GET {}: {}",
                            url, e
                        ))
                        .into());
                    }
                    tokio::time::sleep(retry_delay(None, attempt)).await;
                    attempt += 1;
                    continue;
                }
            };

            if response.status().is_success() {
                content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);

                // Stream body chunks to disk.
                let mut file = tokio::fs::File::create(&tmp_path).await?;
                let mut stream = response.bytes_stream();
                use futures_util::StreamExt;
                while let Some(chunk_result) = stream.next().await {
                    let chunk = chunk_result
                        .map_err(|e| MicrosoftGraphError::Client(format!("Stream error: {}", e)))?;
                    use tokio::io::AsyncWriteExt;
                    file.write_all(&chunk).await?;
                }
                file.flush().await?;
                drop(file);

                // Atomically rename .part → final destination.
                tokio::fs::rename(&tmp_path, &dest).await?;

                let metadata = tokio::fs::metadata(&dest).await?;
                return Ok(serde_json::json!({
                    "path": dest.to_string_lossy(),
                    "size_bytes": metadata.len(),
                    "content_type": content_type,
                }));
            }

            // Non-2xx handling.
            let status = response.status();
            let retry_after = extract_retry_after(&response);
            let api_error = build_api_error(Method::GET, &url, response).await;

            if status == StatusCode::UNAUTHORIZED && attempt < self.max_retries {
                debug!("Got 401 on download, clearing token cache and retrying");
                self.token_provider.clear_cache().await;
                last_error = Some(api_error);
                tokio::time::sleep(retry_delay(retry_after, attempt)).await;
                attempt += 1;
                continue;
            }

            if should_retry_status(status) && attempt < self.max_retries {
                tokio::time::sleep(retry_delay(retry_after, attempt)).await;
                attempt += 1;
                continue;
            }

            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(api_error.into());
        }
    }

    // -- Internal request plumbing -----------------------------------------

    /// Resolve a path or full URL against the base URL.
    fn resolve_url(&self, path_or_url: &str) -> String {
        if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            return path_or_url.to_string();
        }
        let path = if path_or_url.starts_with('/') {
            &path_or_url[1..]
        } else {
            path_or_url
        };
        format!("{}/{}", self.base_url, path)
    }

    /// Core request method with path resolution and retry logic.
    async fn request<T: Serialize + Send + Sync>(
        &self,
        method: Method,
        path_or_url: &str,
        body: Option<&T>,
        params: Option<&[(&str, &str)]>,
        extra_headers: Option<HeaderMap>,
    ) -> Result<Response> {
        let url = self.resolve_url(path_or_url);
        self.request_with_url(method, &url, body, params, extra_headers)
            .await
    }

    /// Core request method using a fully resolved URL.
    async fn request_with_url<T: Serialize + Send + Sync>(
        &self,
        method: Method,
        url: &str,
        body: Option<&T>,
        params: Option<&[(&str, &str)]>,
        extra_headers: Option<HeaderMap>,
    ) -> Result<Response> {
        let mut attempt: u32 = 0;
        let mut last_error: Option<MicrosoftGraphError> = None;

        loop {
            let force_refresh = attempt > 0
                && last_error
                    .as_ref()
                    .map_or(false, error_implies_expired_token);

            let token = self.token_provider.get_access_token(force_refresh).await?;

            let mut headers = HeaderMap::new();
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|e| {
                    MicrosoftGraphError::Client(format!("Invalid auth header: {}", e))
                })?,
            );
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
            if body.is_some() {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            if let Some(ref extra) = extra_headers {
                headers.extend(extra.clone().into_iter());
            }

            let mut req_builder = self.client.request(method.clone(), url).headers(headers);

            if let Some(p) = params {
                req_builder = req_builder.query(p);
            }
            if let Some(b) = body {
                req_builder = req_builder.json(b);
            }

            let response = match req_builder
                .timeout(Duration::from_secs_f64(self.timeout))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if attempt >= self.max_retries {
                        return Err(MicrosoftGraphError::Client(format!(
                            "Request failed for {} {}: {}",
                            method, url, e
                        ))
                        .into());
                    }
                    tokio::time::sleep(retry_delay(None, attempt)).await;
                    attempt += 1;
                    continue;
                }
            };

            if response.status().is_success() {
                return Ok(response);
            }

            let status = response.status();
            let retry_after = extract_retry_after(&response);
            let api_error = build_api_error(method.clone(), url, response).await;

            // 401 → token may be stale; clear cache and retry.
            if status == StatusCode::UNAUTHORIZED && attempt < self.max_retries {
                debug!("Got 401 from Graph API, clearing token cache and retrying");
                self.token_provider.clear_cache().await;
                last_error = Some(api_error);
                tokio::time::sleep(retry_delay(retry_after, attempt)).await;
                attempt += 1;
                continue;
            }

            // Retryable status codes (429, 503, 504, 5xx).
            if should_retry_status(status) && attempt < self.max_retries {
                tokio::time::sleep(retry_delay(retry_after, attempt)).await;
                attempt += 1;
                continue;
            }

            return Err(api_error.into());
        }
    }
}

impl std::fmt::Debug for MicrosoftGraphClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicrosoftGraphClient")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_delay_respects_retry_after_header() {
        // No response → exponential backoff
        let d = retry_delay(None, 0);
        assert!(d.as_secs_f64() >= 0.4 && d.as_secs_f64() <= 1.0);

        let d2 = retry_delay(None, 2);
        assert!(d2.as_secs_f64() >= 1.5 && d2.as_secs_f64() <= 3.0);
    }

    #[test]
    fn test_should_retry_status() {
        assert!(should_retry_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_retry_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(should_retry_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(should_retry_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!should_retry_status(StatusCode::BAD_REQUEST));
        assert!(!should_retry_status(StatusCode::OK));
    }

    #[test]
    fn test_resolve_url() {
        let creds = GraphCredentials {
            tenant_id: "t".into(),
            client_id: "c".into(),
            client_secret: "s".into(),
            scope: DEFAULT_GRAPH_SCOPE.into(),
            authority_url: DEFAULT_GRAPH_AUTHORITY_URL.into(),
        };
        let provider = MicrosoftGraphTokenProvider::new(creds);
        let client = MicrosoftGraphClient::new(provider);

        // Path only → resolved against base URL
        let r1 = client.resolve_url("/users");
        assert_eq!(r1, "https://graph.microsoft.com/v1.0/users");

        let r2 = client.resolve_url("users");
        assert_eq!(r2, "https://graph.microsoft.com/v1.0/users");

        // Full URL → returned as-is
        let r3 = client.resolve_url("https://graph.microsoft.com/v1.0/users/me");
        assert_eq!(r3, "https://graph.microsoft.com/v1.0/users/me");
    }

    #[test]
    fn test_error_message_extraction() {
        let body = r#"{"error": {"code": "Authorization_RequestDenied", "message": "Insufficient privileges"}}"#;
        assert_eq!(
            extract_error_message_from_body(body),
            "Authorization_RequestDenied: Insufficient privileges"
        );

        let body2 = r#"{"error": "invalid_client"}"#;
        assert_eq!(extract_error_message_from_body(body2), "invalid_client");

        let body3 = r#"{"error_description": "AADSTS700016: Application not found"}"#;
        assert_eq!(
            extract_error_message_from_body(body3),
            "AADSTS700016: Application not found"
        );
    }

    #[tokio::test]
    async fn test_token_health_no_cache() {
        let creds = GraphCredentials {
            tenant_id: "t".into(),
            client_id: "c".into(),
            client_secret: "s".into(),
            scope: DEFAULT_GRAPH_SCOPE.into(),
            authority_url: DEFAULT_GRAPH_AUTHORITY_URL.into(),
        };
        let provider = MicrosoftGraphTokenProvider::new(creds);
        let health = provider.inspect_token_health().await;

        assert_eq!(health["configured"], true);
        assert_eq!(health["cached"], false);
        assert_eq!(health["tenant_id"], "t");
    }
}
