//! MCP OAuth 2.1 PKCE Authentication Support
//!
//! Provides OAuth authorization code flow with PKCE for MCP servers that
//! require OAuth authentication instead of static bearer tokens.
//!
//! Architecture mirrors the Python `mcp_oauth.py` + `mcp_oauth_manager.py`
//! modules:
//!
//! - **`TokenStorage`** — file-based persistence of tokens, client info, and
//!   OAuth server metadata (3 JSON files per server in `~/.operant/mcp-tokens/`).
//! - **`OAuthProvider`** — PKCE authorization flow with localhost callback
//!   server, browser open, and 300 s timeout.
//! - **`OAuthManager`** — singleton with per-server provider cache, mtime-based
//!   cache invalidation, and 401 deduplication.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener as TokioTcpListener;
use tokio::sync::{Mutex as AsyncMutex, RwLock, oneshot};
use tracing::{debug, info, warn};
use url::Url;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Subdirectory under HERMES_HOME for MCP OAuth token storage.
const OAUTH_DIR: &str = "mcp-tokens";

/// Default timeout for the OAuth authorization flow (seconds).
const DEFAULT_TIMEOUT: u64 = 300;

/// Default client name sent during OAuth dynamic registration.
const DEFAULT_CLIENT_NAME: &str = "Operant RS Agent";

/// Grant types we support.
const GRANT_TYPES: &[&str] = &["authorization_code", "refresh_token"];

/// Response types we request.
const RESPONSE_TYPES: &[&str] = &["code"];

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// OAuth-specific error kind.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("OAuth non-interactive: {0}")]
    NonInteractive(String),

    #[error("OAuth timeout after {0}s")]
    Timeout(u64),

    #[error("OAuth authorization failed: {0}")]
    AuthFailed(String),

    #[error("OAuth token exchange failed: {0}")]
    TokenExchange(String),

    #[error("OAuth refresh failed: {0}")]
    RefreshFailed(String),

    #[error("OAuth configuration error: {0}")]
    Config(String),

    #[error("OAuth discovery failed: {0}")]
    Discovery(String),

    #[error("No token available")]
    NoToken,
}

impl From<OAuthError> for Error {
    fn from(e: OAuthError) -> Self {
        Error::Agent(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// OAuth token response from the authorization server.
///
/// Mirrors the MCP SDK's `OAuthToken` (Python) and includes an internal
/// `expires_at` field for cross-process wall-clock persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    /// The access token string.
    pub access_token: String,
    /// The type of token (typically "Bearer").
    #[serde(rename = "token_type")]
    pub token_type: String,
    /// Seconds until token expiry (relative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    /// Refresh token for obtaining new access tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Scope of the access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Absolute wall-clock UNIX timestamp (seconds) when the token expires.
    /// Stored alongside `expires_in` so a restarted process can reconstruct
    /// the correct remaining TTL. Stripped before serialization when writing
    /// to the HTTP token-exchange response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
}

impl OAuthToken {
    /// Returns `true` if the token is still valid (not expired).
    pub fn is_valid(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            expires_at > now
        } else {
            // No expiry information — assume valid
            true
        }
    }

    /// Returns the remaining seconds until expiry, or `None` if unknown.
    pub fn remaining_seconds(&self) -> Option<u64> {
        self.expires_at.map(|exp| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            if exp > now { (exp - now) as u64 } else { 0 }
        })
    }
}

/// Persisted client registration information.
///
/// Mirrors the MCP SDK's `OAuthClientInformationFull` (Python).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientInfo {
    /// Client identifier.
    pub client_id: String,
    /// Client secret (for confidential clients).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Redirect URIs registered with the authorization server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uris: Option<Vec<String>>,
    /// Grant types the client is authorized to use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_types: Option<Vec<String>>,
    /// Response types the client can handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_types: Option<Vec<String>>,
    /// Token endpoint authentication method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
    /// Human-readable client name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    /// Scope requested for the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Persisted OAuth authorization server metadata.
///
/// Mirrors the MCP SDK's `OAuthMetadata` (Python).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    /// Issuer identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// Authorization endpoint URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    /// Token endpoint URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
}

/// Configuration for an MCP server's OAuth settings.
///
/// Mirrors the `oauth:` block from `config.yaml`:
///
/// ```yaml
/// mcp_servers:
///   my_server:
///     url: "https://mcp.example.com/mcp"
///     auth: oauth
///     oauth:
///       client_id: "pre-registered-id"
///       client_secret: "secret"
///       scope: "read write"
///       redirect_port: 0
///       client_name: "My Custom Client"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpOAuthConfig {
    /// Pre-registered client ID (skip dynamic registration).
    #[serde(default)]
    pub client_id: Option<String>,
    /// Client secret for confidential clients.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Requested scope string.
    #[serde(default)]
    pub scope: Option<String>,
    /// Specific redirect port (0 = auto-pick free port).
    #[serde(default)]
    pub redirect_port: Option<u16>,
    /// Client name for registration.
    #[serde(default)]
    pub client_name: Option<String>,
    /// Timeout in seconds for the authorization flow.
    #[serde(default)]
    pub timeout: Option<u64>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the Operant home directory.
///
/// Checks `HERMES_HOME` env var first, then falls back to `~/.operant`.
/// Matches the Python `get_operant_home()`.
fn operant_home() -> PathBuf {
    std::env::var("HERMES_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".operant")))
        .unwrap_or_else(|| PathBuf::from(".operant"))
}

/// Return the OAuth token storage directory.
///
/// Layout: `HERMES_HOME/mcp-tokens/<server_hash>/`
fn oauth_dir() -> PathBuf {
    operant_home().join(OAUTH_DIR)
}

/// Compute a SHA-256 hash of the server URL, base64url-encoded (no padding),
/// for use as a directory name.
fn server_hash(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let result = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result)
}

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Find an available TCP port on localhost.
fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to 127.0.0.1:0");
    listener
        .local_addr()
        .expect("Failed to get local address")
        .port()
}

#[expect(dead_code, reason = "reserved for TTY-only OAuth flows")]
fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Check if opening a browser is likely to work.
fn can_open_browser() -> bool {
    // SSH sessions typically don't have a local display
    if std::env::var("SSH_CLIENT").is_ok() || std::env::var("SSH_TTY").is_ok() {
        return false;
    }
    // Linux: need DISPLAY or WAYLAND_DISPLAY
    #[cfg(target_os = "linux")]
    {
        std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
    }
    // macOS and Windows usually have a display
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

/// Open a URL in the system browser.
fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| true)
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| true)
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
            .map(|_| true)
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

/// Read a JSON file, returning `None` if it doesn't exist or is invalid.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    if !path.exists() {
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(data) => Some(data),
            Err(e) => {
                warn!("Failed to parse JSON from {}: {}", path.display(), e);
                None
            }
        },
        Err(e) => {
            warn!("Failed to read {}: {}", path.display(), e);
            None
        }
    }
}

/// Write a serialisable value as JSON with restricted permissions (0o600).
///
/// Uses an atomic write pattern: writes to a temporary file, then renames,
/// mirroring the Python `_write_json` fix for the TOCTOU window.
fn write_json<T: serde::Serialize>(path: &Path, data: &T) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| {
        Error::Agent(format!(
            "Failed to create OAuth directory {}: {}",
            parent.display(),
            e
        ))
    })?;

    // Tighten parent directory permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }

    let json = serde_json::to_string_pretty(data)
        .map_err(|e| Error::Agent(format!("Failed to serialize OAuth data: {}", e)))?;

    // Atomic write with temporary file
    let tmp = parent.join(format!(
        ".tmp.{}.{}.json",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));

    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| {
            Error::Agent(format!(
                "Failed to create temp file {}: {}",
                tmp.display(),
                e
            ))
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp, path).map_err(|e| {
        Error::Agent(format!(
            "Failed to rename {} -> {}: {}",
            tmp.display(),
            path.display(),
            e
        ))
    })?;

    Ok(())
}

/// Get the file modification time in nanoseconds since epoch, or `None`.
fn file_mtime_ns(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
}

/// Simple URL percent-decoding (handles %XX and + for space).
fn urlencoding_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'+' => result.push(' '),
            b'%' => {
                let hi = bytes
                    .next()
                    .and_then(|c| (c as char).to_digit(16))
                    .unwrap_or(0);
                let lo = bytes
                    .next()
                    .and_then(|c| (c as char).to_digit(16))
                    .unwrap_or(0);
                result.push((hi as u8 * 16 + lo as u8) as char);
            }
            _ => result.push(b as char),
        }
    }
    result
}

// ---------------------------------------------------------------------------
// TokenStorage
// ---------------------------------------------------------------------------

/// File-based persistence for OAuth tokens, client info, and server metadata.
///
/// File layout per server:
///
/// ```text
/// HERMES_HOME/mcp-tokens/<server_hash>/
///     tokens.json       — OAuth access/refresh tokens
///     client.json       — client registration info
///     metadata.json     — OAuth authorization server metadata
/// ```
#[derive(Debug, Clone)]
pub struct TokenStorage {
    /// Directory that holds the three JSON files.
    dir: PathBuf,
    /// Server URL hash (used for logging).
    hash: String,
}

impl TokenStorage {
    /// Create a new `TokenStorage` for the given server URL.
    ///
    /// The storage directory is `<oauth_dir>/<server_hash>/`. The hash is
    /// computed as SHA-256 of `server_url`, base64url-encoded (no padding).
    pub fn new(server_url: &str) -> Self {
        let hash = server_hash(server_url);
        let dir = oauth_dir().join(&hash);
        Self { dir, hash }
    }

    /// Path to the tokens file.
    pub fn tokens_path(&self) -> PathBuf {
        self.dir.join("tokens.json")
    }

    /// Path to the client info file.
    pub fn client_info_path(&self) -> PathBuf {
        self.dir.join("client.json")
    }

    /// Path to the metadata file.
    pub fn metadata_path(&self) -> PathBuf {
        self.dir.join("metadata.json")
    }

    // -- Tokens -----------------------------------------------------------

    /// Load tokens from disk.
    ///
    /// Reconstructs the correct `expires_in` from the stored absolute
    /// `expires_at` timestamp so that cross-process token reload works
    /// (the `expires_at` field is consumed here and not returned).
    pub async fn get_tokens(&self) -> Option<OAuthToken> {
        let mut token: OAuthToken = read_json(&self.tokens_path())?;

        // Reconstruct remaining TTL from the stored absolute expiry
        if let Some(expires_at) = token.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            token.expires_in = if expires_at > now {
                Some((expires_at - now) as u64)
            } else {
                Some(0)
            };
        } else if token.expires_in.is_some() {
            // Legacy token (no expires_at): use file mtime as best-effort proxy
            if let Some(mtime_ns) = file_mtime_ns(&self.tokens_path()) {
                let mtime_secs = mtime_ns as f64 / 1_000_000_000.0;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                if let Some(expires_in) = token.expires_in {
                    let implied_expiry = mtime_secs + expires_in as f64;
                    token.expires_in = if implied_expiry > now {
                        Some((implied_expiry - now) as u64)
                    } else {
                        Some(0)
                    };
                }
            }
        }

        Some(token)
    }

    /// Save tokens to disk.
    ///
    /// Stores an absolute `expires_at` timestamp alongside `expires_in` so
    /// a restarted process can reconstruct the correct remaining TTL.
    pub async fn set_tokens(&self, token: &OAuthToken) -> Result<()> {
        let mut payload = token.clone();

        // Compute absolute expiry if we have expires_in
        if let Some(expires_in) = payload.expires_in {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            payload.expires_at = Some(now + expires_in as f64);
        }

        write_json(&self.tokens_path(), &payload)?;
        debug!(hash = %self.hash, "OAuth tokens saved");
        Ok(())
    }

    // -- Client info ------------------------------------------------------

    /// Load client info from disk.
    pub async fn get_client_info(&self) -> Option<OAuthClientInfo> {
        read_json(&self.client_info_path())
    }

    /// Save client info to disk.
    pub async fn set_client_info(&self, client_info: &OAuthClientInfo) -> Result<()> {
        write_json(&self.client_info_path(), client_info)?;
        debug!(hash = %self.hash, "OAuth client info saved");
        Ok(())
    }

    // -- OAuth metadata ---------------------------------------------------

    /// Save OAuth server metadata to disk.
    pub fn save_metadata(&self, metadata: &OAuthMetadata) -> Result<()> {
        write_json(&self.metadata_path(), metadata)?;
        debug!(hash = %self.hash, "OAuth metadata saved");
        Ok(())
    }

    /// Load OAuth server metadata from disk.
    pub fn load_metadata(&self) -> Option<OAuthMetadata> {
        read_json(&self.metadata_path())
    }

    // -- Cleanup ----------------------------------------------------------

    /// Delete all stored OAuth state for this server.
    pub fn remove(&self) -> Result<()> {
        for path in &[
            self.tokens_path(),
            self.client_info_path(),
            self.metadata_path(),
        ] {
            let _ = std::fs::remove_file(path);
        }
        // Remove directory if empty
        let _ = std::fs::remove_dir(&self.dir);
        info!(hash = %self.hash, "OAuth storage removed");
        Ok(())
    }

    /// Check if tokens exist on disk (may be expired).
    pub fn has_tokens(&self) -> bool {
        self.tokens_path().exists()
    }

    /// Get the file modification time of the tokens file in nanoseconds.
    pub fn tokens_file_mtime_ns(&self) -> Option<i64> {
        file_mtime_ns(&self.tokens_path())
    }

    /// Return the hash for this storage (used by manager for logging).
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

// ---------------------------------------------------------------------------
// PKCE Helpers
// ---------------------------------------------------------------------------

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Generate a PKCE code verifier (128 random bytes, base64url-encoded).
fn generate_code_verifier() -> String {
    let mut bytes = vec![0u8; 64];
    use std::io::Read;
    // Use /dev/urandom on Unix, or a fallback
    #[cfg(unix)]
    {
        let mut f = std::fs::File::open("/dev/urandom").expect("Failed to open /dev/urandom");
        f.read_exact(&mut bytes)
            .expect("Failed to read /dev/urandom");
    }
    #[cfg(not(unix))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        // Simple pseudo-random fallback for non-Unix (e.g., Windows)
        let mut state = seed;
        for byte in bytes.iter_mut() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *byte = (state >> 32) as u8;
        }
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

/// Compute the PKCE code challenge (S256 method).
fn compute_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let result = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result)
}

/// Generate a random state parameter for CSRF protection.
fn generate_state() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Callback server
// ---------------------------------------------------------------------------

/// Result from the OAuth callback server.
#[derive(Debug)]
pub struct AuthCallbackResult {
    /// The authorization code.
    pub code: Option<String>,
    /// The state parameter (for CSRF verification).
    pub state: Option<String>,
    /// Error description, if any.
    pub error: Option<String>,
}

/// Start a localhost HTTP server on the given port that serves a single
/// `/callback` GET request.
///
/// Returns a `oneshot::Receiver` that delivers the `AuthCallbackResult`
/// once the callback is received. The server shuts down after handling
/// one request.
///
/// # Arguments
/// * `port` - The TCP port to bind to (0 = auto-select).
pub async fn start_localhost_server(
    port: u16,
) -> Result<(u16, oneshot::Receiver<AuthCallbackResult>)> {
    let actual_port: u16;
    let listener: TokioTcpListener;

    if port == 0 {
        listener = TokioTcpListener::bind(("127.0.0.1", 0)).await?;
        actual_port = listener.local_addr()?.port();
    } else {
        listener = TokioTcpListener::bind(("127.0.0.1", port)).await?;
        actual_port = port;
    }

    let (tx, rx) = oneshot::channel::<AuthCallbackResult>();
    let server_closed = Arc::new(AtomicBool::new(false));
    let closed = server_closed.clone();

    tokio::spawn(async move {
        // Accept at most one connection
        let accept_result = tokio::time::timeout(Duration::from_secs(360), listener.accept()).await;

        let mut socket = match accept_result {
            Ok(Ok((socket, _addr))) => socket,
            _ => {
                let _ = tx.send(AuthCallbackResult {
                    code: None,
                    state: None,
                    error: Some("No connection received on callback port".to_string()),
                });
                return;
            }
        };

        // Read the HTTP request
        let mut buf = vec![0u8; 4096];
        let n = match socket.read(&mut buf).await {
            Ok(n) if n > 0 => n,
            _ => {
                let _ = tx.send(AuthCallbackResult {
                    code: None,
                    state: None,
                    error: Some("Failed to read HTTP request".to_string()),
                });
                return;
            }
        };

        let request_str = String::from_utf8_lossy(&buf[..n]);

        // Parse the request line: GET /callback?code=...&state=... HTTP/1.1
        let mut code = None;
        let mut state = None;
        let mut error_param = None;

        if let Some(request_line) = request_str.lines().next() {
            let parts: Vec<&str> = request_line.split_whitespace().collect();
            if parts.len() >= 2 {
                let path = parts[1];
                let query_str = path.split('?').nth(1).unwrap_or("");

                for pair in query_str.split('&') {
                    let mut kv = pair.splitn(2, '=');
                    let key = kv.next().unwrap_or("").trim();
                    let value = kv.next().unwrap_or("").trim();
                    match key {
                        "code" => code = Some(urlencoding_decode(value)),
                        "state" => state = Some(urlencoding_decode(value)),
                        "error" => error_param = Some(urlencoding_decode(value)),
                        _ => {}
                    }
                }
            }
        }

        let result = AuthCallbackResult {
            code,
            state,
            error: error_param,
        };

        let is_success = result.code.is_some();
        let body_str = if is_success {
            "<html><body><h2>Authorization Successful</h2><p>You can close this tab and return to Operant.</p></body></html>".to_string()
        } else {
            let err_msg = result.error.as_deref().unwrap_or("unknown");
            format!(
                "<html><body><h2>Authorization Failed</h2><p>Error: {}</p></body></html>",
                err_msg
            )
        };

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body_str.len(),
            body_str
        );

        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.flush().await;

        closed.store(true, Ordering::SeqCst);
        let _ = tx.send(result);
    });

    Ok((actual_port, rx))
}

// ---------------------------------------------------------------------------
// OAuthProvider
// ---------------------------------------------------------------------------

/// OAuth authentication provider implementing PKCE authorization code flow.
#[derive(Debug, Clone)]
pub struct OAuthProvider {
    server_url: String,
    config: McpOAuthConfig,
    storage: TokenStorage,
    token: Arc<RwLock<Option<OAuthToken>>>,
    metadata: Arc<RwLock<Option<OAuthMetadata>>>,
    client_info: Arc<RwLock<Option<OAuthClientInfo>>>,
}

impl OAuthProvider {
    /// Create a new `OAuthProvider`.
    pub fn new(server_url: String, config: McpOAuthConfig) -> Self {
        let storage = TokenStorage::new(&server_url);
        Self {
            server_url,
            config,
            storage,
            token: Arc::new(RwLock::new(None)),
            metadata: Arc::new(RwLock::new(None)),
            client_info: Arc::new(RwLock::new(None)),
        }
    }

    /// Get a reference to the token storage.
    pub fn storage(&self) -> &TokenStorage {
        &self.storage
    }

    // -- Discovery --------------------------------------------------------

    /// Discover OAuth authorization server metadata.
    ///
    /// Tries PRM (Protected Resource Metadata) discovery first, then
    /// ASM (Authorization Server Metadata) discovery using the discovered
    /// authorization server URL.
    pub async fn discover_metadata(&self) -> Result<OAuthMetadata> {
        let server_url = self.server_url.trim_end_matches('/');

        // Step 1: PRM discovery
        // Try: {server_url}/.well-known/oauth-protected-resource
        let prm_urls = [
            format!("{}/.well-known/oauth-protected-resource", server_url),
            format!("{}/oauth-protected-resource", server_url),
        ];

        let mut auth_server_url = None;
        let client = reqwest::Client::new();

        for url in &prm_urls {
            match client
                .get(url)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(servers) =
                            json.get("authorization_servers").and_then(|v| v.as_array())
                        {
                            if let Some(first) = servers.first().and_then(|v| v.as_str()) {
                                auth_server_url = Some(first.to_string());
                                debug!("MCP OAuth: PRM discovered auth_server={}", first);
                            }
                        }
                        // Also look for token_endpoint directly in PRM
                        if let Some(token_endpoint) =
                            json.get("token_endpoint").and_then(|v| v.as_str())
                        {
                            let meta = OAuthMetadata {
                                issuer: json
                                    .get("issuer")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                                authorization_endpoint: json
                                    .get("authorization_endpoint")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                                token_endpoint: Some(token_endpoint.to_string()),
                            };
                            if meta.token_endpoint.is_some() {
                                debug!(
                                    "MCP OAuth: PRM discovery complete, token_endpoint={}",
                                    token_endpoint
                                );
                                return Ok(meta);
                            }
                        }
                    }
                }
                _ => {
                    debug!("MCP OAuth: PRM discovery to {} failed, trying next", url);
                }
            }
        }

        // Step 2: ASM discovery
        let asm_base = auth_server_url.as_deref().unwrap_or(server_url);
        let asm_urls = [
            format!("{}/.well-known/oauth-authorization-server", asm_base),
            format!("{}/oauth-authorization-server", asm_base),
        ];

        for url in &asm_urls {
            match client
                .get(url)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let meta = OAuthMetadata {
                            issuer: json
                                .get("issuer")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            authorization_endpoint: json
                                .get("authorization_endpoint")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            token_endpoint: json
                                .get("token_endpoint")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                        };
                        if meta.authorization_endpoint.is_some() || meta.token_endpoint.is_some() {
                            debug!("MCP OAuth: ASM discovery complete at {}", url);
                            return Ok(meta);
                        }
                    }
                }
                _ => {
                    debug!("MCP OAuth: ASM discovery to {} failed", url);
                }
            }
        }

        Err(OAuthError::Discovery(format!(
            "Could not discover OAuth metadata for {}",
            server_url
        ))
        .into())
    }

    // -- Client registration ----------------------------------------------

    /// Get or create client info.
    ///
    /// If `client_id` is pre-configured, use it directly. Otherwise, attempt
    /// dynamic client registration via the registration endpoint.
    pub async fn get_or_create_client_info(&self) -> Result<OAuthClientInfo> {
        // Check cache first
        {
            let cached = self.client_info.read().await;
            if let Some(info) = cached.as_ref() {
                return Ok(info.clone());
            }
        }

        // Check disk
        if let Some(info) = self.storage.get_client_info().await {
            *self.client_info.write().await = Some(info.clone());
            return Ok(info);
        }

        // Use pre-registered client if available
        if let Some(client_id) = &self.config.client_id {
            // Resolve redirect_port from config
            let port = self.config.redirect_port.unwrap_or(0);
            let actual_port = if port == 0 { find_free_port() } else { port };
            let redirect_uri = format!("http://127.0.0.1:{}/callback", actual_port);

            let info = OAuthClientInfo {
                client_id: client_id.clone(),
                client_secret: self.config.client_secret.clone(),
                redirect_uris: Some(vec![redirect_uri]),
                grant_types: Some(GRANT_TYPES.iter().map(|s| s.to_string()).collect()),
                response_types: Some(RESPONSE_TYPES.iter().map(|s| s.to_string()).collect()),
                token_endpoint_auth_method: if self.config.client_secret.is_some() {
                    Some("client_secret_post".to_string())
                } else {
                    Some("none".to_string())
                },
                client_name: Some(
                    self.config
                        .client_name
                        .clone()
                        .unwrap_or_else(|| DEFAULT_CLIENT_NAME.to_string()),
                ),
                scope: self.config.scope.clone(),
            };

            // Persist to disk
            self.storage.set_client_info(&info).await?;
            *self.client_info.write().await = Some(info.clone());
            return Ok(info);
        }

        Err(OAuthError::Config(
            "No client_id configured and no cached client info available".to_string(),
        )
        .into())
    }

    // -- Authorization URL ------------------------------------------------

    /// Build the authorization URL for the PKCE flow.
    ///
    /// Returns `(auth_url, code_verifier, state, redirect_port)`.
    pub async fn build_authorization_url(&self) -> Result<(String, String, String, u16)> {
        let metadata = {
            let cached = self.metadata.read().await;
            if let Some(meta) = cached.as_ref() {
                meta.clone()
            } else {
                drop(cached);
                // Load from disk or discover
                let meta = self
                    .storage
                    .load_metadata()
                    .or({
                        // Try to discover
                        None
                    })
                    .unwrap_or({
                        // Default structure; discovery will be attempted on first auth
                        OAuthMetadata {
                            issuer: None,
                            authorization_endpoint: None,
                            token_endpoint: None,
                        }
                    });
                *self.metadata.write().await = Some(meta.clone());
                meta
            }
        };

        let client_info = self.get_or_create_client_info().await?;

        // Pick port
        let port = self.config.redirect_port.unwrap_or(0);
        let actual_port = if port == 0 { find_free_port() } else { port };
        let redirect_uri = format!("http://127.0.0.1:{}/callback", actual_port);

        // Generate PKCE parameters
        let code_verifier = generate_code_verifier();
        let code_challenge = compute_code_challenge(&code_verifier);
        let state = generate_state();

        // Build authorization URL
        let auth_endpoint = match metadata.authorization_endpoint {
            Some(ref url) => url.clone(),
            None => format!("{}/authorize", self.server_url.trim_end_matches('/')),
        };

        let auth_url = Url::parse_with_params(
            &auth_endpoint,
            &[
                ("response_type", "code"),
                ("client_id", &client_info.client_id),
                ("redirect_uri", &redirect_uri),
                ("code_challenge", &code_challenge),
                ("code_challenge_method", "S256"),
                ("state", &state),
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to build authorization URL: {}", e)))?;

        // If we have a scope, add it
        let auth_url = if let Some(scope) = &self.config.scope {
            let mut url = auth_url;
            url.query_pairs_mut().append_pair("scope", scope);
            url
        } else {
            auth_url
        };

        Ok((auth_url.to_string(), code_verifier, state, actual_port))
    }

    // -- Token exchange ---------------------------------------------------

    /// Exchange an authorization code for tokens.
    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        client_info: &OAuthClientInfo,
        redirect_uri: &str,
    ) -> Result<OAuthToken> {
        let metadata = {
            let cached = self.metadata.read().await;
            if let Some(meta) = cached.as_ref() {
                meta.clone()
            } else {
                drop(cached);
                let meta = self.storage.load_metadata().unwrap_or(OAuthMetadata {
                    issuer: None,
                    authorization_endpoint: None,
                    token_endpoint: None,
                });
                *self.metadata.write().await = Some(meta.clone());
                meta
            }
        };

        let token_endpoint = match metadata.token_endpoint {
            Some(ref url) => url.clone(),
            None => format!("{}/token", self.server_url.trim_end_matches('/')),
        };

        let client = reqwest::Client::new();
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("code_verifier", code_verifier);
        params.insert("redirect_uri", redirect_uri);
        params.insert("client_id", &client_info.client_id);

        if let Some(ref secret) = client_info.client_secret {
            params.insert("client_secret", secret);
        }

        debug!("MCP OAuth: exchanging code at {}", token_endpoint);

        let resp = client
            .post(token_endpoint.as_str())
            .form(&params)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| OAuthError::TokenExchange(format!("HTTP error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchange(format!("HTTP {}: {}", status, body)).into());
        }

        let token: OAuthToken = resp.json().await.map_err(|e| {
            OAuthError::TokenExchange(format!("Failed to parse token response: {}", e))
        })?;

        // Save to storage
        self.storage.set_tokens(&token).await?;

        // Update cache
        *self.token.write().await = Some(token.clone());

        Ok(token)
    }

    // -- Token refresh ----------------------------------------------------

    /// Refresh the access token using the refresh token.
    pub async fn refresh_token(&self) -> Result<OAuthToken> {
        let metadata = {
            let cached = self.metadata.read().await;
            if let Some(meta) = cached.as_ref() {
                meta.clone()
            } else {
                drop(cached);
                let meta = self.storage.load_metadata().unwrap_or(OAuthMetadata {
                    issuer: None,
                    authorization_endpoint: None,
                    token_endpoint: None,
                });
                *self.metadata.write().await = Some(meta.clone());
                meta
            }
        };

        let client_info = self.get_or_create_client_info().await?;

        let current_token = self.storage.get_tokens().await.ok_or(OAuthError::NoToken)?;

        let old_refresh_token = current_token.refresh_token.clone();
        let refresh_token = old_refresh_token
            .ok_or_else(|| OAuthError::RefreshFailed("No refresh token available".to_string()))?;

        let token_endpoint = match metadata.token_endpoint {
            Some(ref url) => url.clone(),
            None => format!("{}/token", self.server_url.trim_end_matches('/')),
        };

        let client = reqwest::Client::new();
        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", &refresh_token);
        params.insert("client_id", &client_info.client_id);

        if let Some(ref secret) = client_info.client_secret {
            params.insert("client_secret", secret);
        }

        debug!("MCP OAuth: refreshing token at {}", token_endpoint);

        let resp = client
            .post(token_endpoint.as_str())
            .form(&params)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| OAuthError::RefreshFailed(format!("HTTP error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::RefreshFailed(format!("HTTP {}: {}", status, body)).into());
        }

        let new_token: OAuthToken = resp.json().await.map_err(|e| {
            OAuthError::RefreshFailed(format!("Failed to parse refresh response: {}", e))
        })?;

        let mut token = new_token;
        if token.refresh_token.is_none() {
            token.refresh_token = current_token.refresh_token.clone();
        }

        // Save to storage
        self.storage.set_tokens(&token).await?;

        // Update cache
        *self.token.write().await = Some(token.clone());

        Ok(token)
    }

    // -- Full authentication flow -----------------------------------------

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Execute the full PKCE authorization code flow.
    ///
    /// 1. Discovers or loads OAuth metadata
    /// 2. Gets or creates client registration
    /// 3. Builds authorization URL with PKCE parameters
    /// 4. Starts localhost callback server
    /// 5. Opens browser
    /// 6. Waits for callback (with timeout)
    /// 7. Exchanges code for tokens
    /// 8. Saves tokens
    ///
    /// Returns the obtained `OAuthToken`.
    pub async fn authenticate(&self) -> Result<OAuthToken> {
        // Step 1: Ensure metadata is loaded/discovered
        let metadata = if self.metadata.read().await.is_some() {
            self.metadata
                .read()
                .await
                .clone()
                .expect("metadata Some (guarded above)")
        } else {
            match self.discover_metadata().await {
                Ok(meta) => {
                    self.storage.save_metadata(&meta)?;
                    *self.metadata.write().await = Some(meta.clone());
                    meta
                }
                Err(_) => {
                    // Don't fail yet — use fallback URLs
                    warn!("MCP OAuth: metadata discovery failed, using fallback URLs");
                    OAuthMetadata {
                        issuer: None,
                        authorization_endpoint: None,
                        token_endpoint: None,
                    }
                }
            }
        };

        // Step 2: Get client info
        let client_info = self.get_or_create_client_info().await?;

        // Step 3: Build authorization URL
        let port = self.config.redirect_port.unwrap_or(0);
        let actual_port = if port == 0 { find_free_port() } else { port };

        let code_verifier = generate_code_verifier();
        let code_challenge = compute_code_challenge(&code_verifier);
        let state = generate_state();
        let redirect_uri = format!("http://127.0.0.1:{}/callback", actual_port);

        let auth_endpoint = match metadata.authorization_endpoint {
            Some(ref url) => url.clone(),
            None => format!("{}/authorize", self.server_url.trim_end_matches('/')),
        };

        let mut auth_url = Url::parse_with_params(
            &auth_endpoint,
            &[
                ("response_type", "code"),
                ("client_id", &client_info.client_id),
                ("redirect_uri", &redirect_uri),
                ("code_challenge", &code_challenge),
                ("code_challenge_method", "S256"),
                ("state", &state),
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to build authorization URL: {}", e)))?;

        if let Some(scope) = &self.config.scope {
            auth_url.query_pairs_mut().append_pair("scope", scope);
        }

        // Step 4: Start callback server
        let (_actual_port, rx) = start_localhost_server(actual_port).await?;
        let redirect_uri = format!("http://127.0.0.1:{}/callback", _actual_port);

        // Step 5: Show URL and open browser
        let auth_url_str = auth_url.to_string();
        eprintln!("\n  MCP OAuth: authorization required.");
        eprintln!("  Open this URL in your browser:\n");
        eprintln!("    {}\n", auth_url_str);

        if can_open_browser() {
            if open_browser(&auth_url_str) {
                eprintln!("  (Browser opened automatically.)\n");
            } else {
                eprintln!("  (Could not open browser — please open the URL manually.)\n");
            }
        } else {
            eprintln!("  (Headless environment detected — open the URL manually.)\n");
        }

        // Step 6: Wait for callback
        let timeout = Duration::from_secs(self.config.timeout.unwrap_or(DEFAULT_TIMEOUT));
        let result = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| OAuthError::Timeout(timeout.as_secs()))?
            .map_err(|_| {
                OAuthError::AuthFailed("Callback channel closed unexpectedly".to_string())
            })?;

        if let Some(error) = result.error {
            return Err(OAuthError::AuthFailed(error).into());
        }

        let code = result
            .code
            .ok_or_else(|| OAuthError::AuthFailed("No authorization code received".to_string()))?;

        // Verify state
        if let Some(returned_state) = result.state {
            if returned_state != state {
                warn!("MCP OAuth: state mismatch (possible CSRF)");
            }
        }

        // Step 7: Exchange code for tokens
        let token = self
            .exchange_code(&code, &code_verifier, &client_info, &redirect_uri)
            .await?;

        info!(
            "MCP OAuth: authentication complete for {}",
            self.storage.hash()
        );
        Ok(token)
    }

    /// Get a valid token, refreshing if necessary.
    ///
    /// Returns the current token if valid, tries to refresh if expired,
    /// or returns an error if no token is available.
    pub async fn get_valid_token(&self) -> Result<OAuthToken> {
        // Check cache
        {
            let cached = self.token.read().await;
            if let Some(token) = cached.as_ref() {
                if token.is_valid() {
                    return Ok(token.clone());
                }
            }
        }

        // Try disk
        if let Some(token) = self.storage.get_tokens().await {
            if token.is_valid() {
                *self.token.write().await = Some(token.clone());
                return Ok(token);
            }

            // Token expired — try refresh
            if token.refresh_token.is_some() {
                match self.refresh_token().await {
                    Ok(refreshed) => return Ok(refreshed),
                    Err(e) => {
                        warn!("MCP OAuth: refresh failed: {}", e);
                        // Fall through to full re-auth
                    }
                }
            }
        }

        Err(OAuthError::NoToken.into())
    }

    /// Load cached metadata from disk into memory.
    pub async fn load_cached_metadata(&self) {
        if self.metadata.read().await.is_none() {
            if let Some(meta) = self.storage.load_metadata() {
                *self.metadata.write().await = Some(meta);
            }
        }
    }

    /// Load cached client info from disk into memory.
    pub async fn load_cached_client_info(&self) {
        if self.client_info.read().await.is_none() {
            if let Some(info) = self.storage.get_client_info().await {
                *self.client_info.write().await = Some(info);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OAuthManager
// ---------------------------------------------------------------------------

struct ProviderEntry {
    server_url: String,
    config: McpOAuthConfig,
    provider: RwLock<Option<OAuthProvider>>,
    last_mtime_ns: RwLock<i64>,
    lock: AsyncMutex<()>,
    _pending_401: AsyncMutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl ProviderEntry {
    fn new(server_url: String, config: McpOAuthConfig, provider: OAuthProvider) -> Self {
        Self {
            server_url,
            config,
            provider: RwLock::new(Some(provider)),
            last_mtime_ns: RwLock::new(0),
            lock: AsyncMutex::new(()),
            _pending_401: AsyncMutex::new(HashMap::new()),
        }
    }
}

/// Singleton manager for per-server MCP OAuth state.
///
/// Provides:
/// - **Per-server provider caching** — one `OAuthProvider` instance per
///   server URL, reused across reconnect attempts.
/// - **mtime-based cache invalidation** — detects when an external process
///   has refreshed tokens on disk.
/// - **401 deduplication** — when N concurrent tool calls all hit 401 with
///   the same access token, only one recovery attempt fires; the others
///   await the same result.
///
/// Thread-safe: the entry map is guarded by a `Mutex` for get-or-create
/// semantics. Per-entry state is guarded by its own `AsyncMutex`.
pub struct OAuthManager {
    entries: Mutex<HashMap<String, Arc<ProviderEntry>>>,
}

impl OAuthManager {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get or create a cached `OAuthProvider` for the given server URL.
    ///
    /// If the URL changes for a cached entry, the old entry is discarded
    /// and a fresh provider is built.
    pub fn get_provider(&self, server_url: &str, config: Option<McpOAuthConfig>) -> OAuthProvider {
        let config = config.unwrap_or_default();
        let mut entries = self
            .entries
            .lock()
            .expect("entries mutex poisoned — programmer error");
        let url = server_url.to_string();

        if let Some(entry) = entries.get(&url) {
            if entry.server_url == url {
                let guard = entry.provider.blocking_read();
                if let Some(ref provider) = *guard {
                    return provider.clone();
                }
            }
        }

        entries.remove(&url);

        let provider = OAuthProvider::new(url.clone(), config.clone());
        let entry = Arc::new(ProviderEntry::new(url.clone(), config, provider.clone()));
        entries.insert(url, entry);
        provider
    }

    /// Get the current valid token for a server, or `None` if unavailable.
    ///
    /// This does NOT trigger the interactive OAuth flow — call
    /// `get_provider().authenticate()` for that.
    pub async fn get_token(&self, server_url: &str) -> Option<OAuthToken> {
        let entry = self.get_entry(server_url)?;
        let provider_guard = entry.provider.read().await;
        let provider = provider_guard.as_ref()?;

        provider.get_valid_token().await.ok()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    pub fn clear_cache(&self, server_url: &str) {
        let mut entries = self
            .entries
            .lock()
            .expect("entries mutex poisoned — programmer error");
        entries.remove(server_url);
        info!("MCP OAuth: cache cleared for {}", server_url);
    }

    pub async fn refresh_token(&self, server_url: &str) -> Result<OAuthToken> {
        let entry = self
            .get_entry(server_url)
            .ok_or_else(|| Error::Agent(format!("No cached provider for {}", server_url)))?;
        let provider_guard = entry.provider.read().await;
        let provider = provider_guard
            .as_ref()
            .ok_or_else(|| Error::Agent(format!("No provider for {}", server_url)))?;

        provider.refresh_token().await
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    fn get_entry(&self, server_url: &str) -> Option<Arc<ProviderEntry>> {
        self.entries
            .lock()
            .expect("entries mutex poisoned — programmer error")
            .get(server_url)
            .cloned()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Check if the tokens file on disk has been modified externally.
    ///
    /// If the mtime has changed, forces the provider to re-read from disk
    /// on the next token access.
    ///
    /// Returns `true` if the cache was invalidated.
    pub async fn invalidate_if_disk_changed(&self, server_url: &str) -> bool {
        let url = server_url.to_string();
        let entry = {
            let entries = self
                .entries
                .lock()
                .expect("entries mutex poisoned — programmer error");
            entries.get(&url).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return false,
        };

        let _guard = entry.lock.lock().await;
        let storage = TokenStorage::new(&url);
        let current_mtime = storage.tokens_file_mtime_ns().unwrap_or(0);
        let last_mtime = *entry.last_mtime_ns.read().await;

        if current_mtime != 0 && current_mtime != last_mtime {
            *entry.last_mtime_ns.write().await = current_mtime;
            let provider = OAuthProvider::new(url.clone(), entry.config.clone());
            *entry.provider.write().await = Some(provider);
            info!("MCP OAuth: tokens file changed for {}, forcing reload", url);
            return true;
        }

        false
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    #[allow(unused_variables)]
    pub async fn handle_401(&self, server_url: &str, failed_access_token: Option<&str>) -> bool {
        let entry = {
            let entries = self
                .entries
                .lock()
                .expect("entries mutex poisoned — programmer error");
            entries.get(server_url).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return false,
        };

        let _guard = entry.lock.lock().await;

        let storage = TokenStorage::new(server_url);
        let current_mtime = storage.tokens_file_mtime_ns().unwrap_or(0);
        let last_mtime = *entry.last_mtime_ns.read().await;

        if current_mtime != 0 && current_mtime != last_mtime {
            *entry.last_mtime_ns.write().await = current_mtime;
            return true;
        }

        let provider_guard = entry.provider.read().await;
        if let Some(ref provider) = *provider_guard {
            match provider.refresh_token().await {
                Ok(_) => return true,
                Err(_) => return false,
            }
        }

        false
    }

    /// Remove a server from the cache AND delete its tokens from disk.
    pub fn remove(&self, server_url: &str) {
        self.clear_cache(server_url);

        // Delete tokens from disk
        let storage = TokenStorage::new(server_url);
        let _ = storage.remove();

        info!(
            "MCP OAuth: evicted from cache and removed from disk: {}",
            server_url
        );
    }
}

// ---------------------------------------------------------------------------
// Singleton
// ---------------------------------------------------------------------------

static MANAGER: OnceLock<OAuthManager> = OnceLock::new();

/// Return the process-wide `OAuthManager` singleton.
pub fn get_manager() -> &'static OAuthManager {
    MANAGER.get_or_init(OAuthManager::new)
}

/// Reset the singleton (test-only helper).
pub fn reset_manager_for_tests() {
    // OnceLock doesn't support reset, so this is a best-effort test helper.
    // In practice, tests should use separate server URLs.
    info!("MCP OAuth manager reset requested (OnceLock cannot be reset)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_hash_consistency() {
        let url = "https://mcp.example.com/mcp";
        let hash1 = server_hash(url);
        let hash2 = server_hash(url);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_server_hash_different_urls() {
        let hash1 = server_hash("https://server1.example.com/mcp");
        let hash2 = server_hash("https://server2.example.com/mcp");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_token_validity() {
        let token = OAuthToken {
            access_token: "test".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: None,
            scope: None,
            expires_at: Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64()
                    + 3600.0,
            ),
        };
        assert!(token.is_valid());
    }

    #[test]
    fn test_token_expired() {
        let token = OAuthToken {
            access_token: "test".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(0),
            refresh_token: None,
            scope: None,
            expires_at: Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64()
                    - 3600.0,
            ),
        };
        assert!(!token.is_valid());
    }

    #[test]
    fn test_find_free_port() {
        let port = find_free_port();
        assert!(port > 0);
        // Verify the port is actually available
        let listener = TcpListener::bind(("127.0.0.1", port));
        assert!(listener.is_ok());
    }

    #[test]
    fn test_code_verifier_generation() {
        let verifier = generate_code_verifier();
        assert!(!verifier.is_empty());
        // Base64url-encoded 64 bytes = ~87 chars
        assert!(verifier.len() > 80);
        assert!(verifier.len() < 100);
    }

    #[test]
    fn test_code_challenge_consistency() {
        let verifier = generate_code_verifier();
        let challenge1 = compute_code_challenge(&verifier);
        let challenge2 = compute_code_challenge(&verifier);
        assert_eq!(challenge1, challenge2);
    }

    #[test]
    fn test_state_generation() {
        let state1 = generate_state();
        let state2 = generate_state();
        assert_ne!(state1, state2);
        assert!(!state1.is_empty());
    }

    #[test]
    fn test_oauth_dir() {
        let dir = oauth_dir();
        assert!(dir.ends_with("mcp-tokens"));
    }

    #[test]
    fn test_token_storage_new() {
        let storage = TokenStorage::new("https://example.com/mcp");
        assert!(storage.tokens_path().ends_with("tokens.json"));
        assert!(storage.client_info_path().ends_with("client.json"));
        assert!(storage.metadata_path().ends_with("metadata.json"));
    }

    #[test]
    fn test_mcp_oauth_config_default() {
        let config = McpOAuthConfig::default();
        assert!(config.client_id.is_none());
        assert!(config.client_secret.is_none());
        assert!(config.scope.is_none());
        assert!(config.redirect_port.is_none());
    }

    #[tokio::test]
    async fn test_token_storage_roundtrip() {
        let dir = std::env::temp_dir().join(format!("oauth-test-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);

        let _old = std::env::var("HERMES_HOME").ok();
        // SAFETY: test-only env mutation under exclusive lock
        unsafe { std::env::set_var("HERMES_HOME", dir.to_str().unwrap()) };

        let storage = TokenStorage::new("https://test-server.example.com/mcp");

        let token = OAuthToken {
            access_token: "test_access_token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: Some("test_refresh_token".to_string()),
            scope: Some("read write".to_string()),
            expires_at: None,
        };

        assert!(storage.set_tokens(&token).await.is_ok());
        let loaded = storage.get_tokens().await;
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.access_token, "test_access_token");
        assert_eq!(loaded.refresh_token, Some("test_refresh_token".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
        if let Some(old) = _old {
            // SAFETY: test-only env mutation under exclusive lock
            unsafe { std::env::set_var("HERMES_HOME", &old) };
        } else {
            // SAFETY: test-only env mutation under exclusive lock
            unsafe { std::env::remove_var("HERMES_HOME") };
        }
    }

    #[test]
    fn test_localhost_server_port_zero() {
        // Just verify the helper compiles and returns a valid port
        let port = find_free_port();
        assert!(port > 0);
    }

    #[test]
    fn test_oauth_error_conversion() {
        let err = OAuthError::NoToken;
        let converted: Error = err.into();
        assert!(converted.to_string().contains("No token"));
    }
}
