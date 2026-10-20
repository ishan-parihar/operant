//! MCP (Model Context Protocol) client for Operant-RS
//!
//! Provides integration with MCP servers to extend the agent's capabilities
//! with tools and resources from external sources.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// MCP protocol versions we support, newest first. We send the newest in
/// our `initialize` request, then verify the server's response is in this
/// list. (iter-130 — closes the ponytail-audit gap "no protocol version
/// negotiation; hardcoded 2024-11-05 will silently break against
/// 2025-06-18 servers".)
const MCP_SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2024-11-05"];

/// The default (newest) version we send in `initialize`.
const MCP_DEFAULT_VERSION: &str = "2025-06-18";

/// Check whether a server-returned `protocolVersion` is one we accept.
/// Returns the (possibly normalized) version to use, or None if the
/// server's version is incompatible with all of ours.
fn negotiate_protocol_version(server_version: &str) -> Option<&'static str> {
    // Exact match first.
    for v in MCP_SUPPORTED_VERSIONS {
        if *v == server_version {
            return Some(v);
        }
    }
    // Fallback: prefix match (some servers return "2025-06-18-draft" etc.)
    MCP_SUPPORTED_VERSIONS
        .iter()
        .find(|&v| server_version.starts_with(*v))
        .map(|v| v as _)
}

/// MCP client for connecting to MCP servers
#[derive(Debug, Clone)]
pub struct McpClient {
    /// Server URL
    url: String,
    /// Authentication token
    auth_token: Option<String>,
    /// HTTP client
    client: reqwest::Client,
    /// Connected tools from this server
    tools: Arc<RwLock<Vec<McpTool>>>,
    /// Server capabilities
    capabilities: Arc<RwLock<McpCapabilities>>,
    /// Whether connected
    connected: Arc<RwLock<bool>>,
    /// Monotonic JSON-RPC request id counter. (iter-130 — closes the
    /// ponytail-audit bug "HTTP McClient uses SystemTime nanos for the
    /// JSON-RPC id; non-monotonic, can collide, and the response isn't
    /// verified to match the id".)
    request_id: Arc<AtomicU64>,
}

/// Server capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpCapabilities {
    /// Supports tools
    pub tools: bool,
    /// Supports resources
    pub resources: bool,
    /// Supports prompts
    pub prompts: bool,
}

/// Initialize request
#[derive(Debug, Serialize)]
struct InitializeRequest {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    client_info: ClientInfo,
}

/// Client capabilities
#[derive(Debug, Serialize)]
struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    roots: Option<Roots>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sampling: Option<Sampling>,
}

/// Roots capability
#[derive(Debug, Serialize)]
struct Roots {
    #[serde(rename = "listChanged")]
    list_changed: bool,
}

/// Sampling capability
#[derive(Debug, Serialize)]
struct Sampling {}

/// Client info
#[derive(Debug, Serialize)]
struct ClientInfo {
    name: String,
    version: String,
}

/// Initialize response
#[derive(Debug, Deserialize)]
struct InitializeResponse {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

/// Server capabilities
#[derive(Debug, Deserialize)]
struct ServerCapabilities {
    #[serde(rename = "tools")]
    tools: Option<ToolsCapability>,
    #[serde(rename = "resources")]
    resources: Option<ResourcesCapability>,
    #[serde(rename = "prompts")]
    prompts: Option<PromptsCapability>,
}

/// Tools capability
#[derive(Debug, Deserialize)]
struct ToolsCapability {
    #[expect(
        dead_code,
        reason = "Deserialized from MCP server response; kept for protocol completeness"
    )]
    #[serde(rename = "listChanged")]
    list_changed: Option<bool>,
}

/// Resources capability
#[derive(Debug, Deserialize)]
struct ResourcesCapability {
    #[expect(
        dead_code,
        reason = "Deserialized from MCP server response; kept for protocol completeness"
    )]
    #[serde(rename = "subscribe")]
    subscribe: Option<bool>,
    #[expect(
        dead_code,
        reason = "Deserialized from MCP server response; kept for protocol completeness"
    )]
    #[serde(rename = "listChanged")]
    list_changed: Option<bool>,
}

/// Prompts capability
#[derive(Debug, Deserialize)]
struct PromptsCapability {}

/// Server info
#[derive(Debug, Deserialize)]
struct ServerInfo {
    name: String,
    version: String,
}

/// JSON-RPC request
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
}

/// JSON-RPC response
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[expect(
        dead_code,
        reason = "Deserialized from MCP server; protocol field kept for completeness"
    )]
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

/// JSON-RPC error
#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[expect(
        dead_code,
        reason = "Deserialized from MCP server; may contain debug info"
    )]
    data: Option<Value>,
}

/// Tool listing
#[derive(Debug, Deserialize)]
struct ToolListResult {
    tools: Vec<McpToolDefinition>,
}

/// Tool definition from MCP server
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl McpClient {
    /// Create a new MCP client
    pub fn new(url: impl Into<String>, auth_token: Option<String>) -> Self {
        Self {
            url: url.into(),
            auth_token,
            // iter-130: configure a 60s timeout — previously the HTTP
            // transport had no timeout, so a slow/dead MCP server could
            // hang the agent loop forever. Stdio + SSE paths already had
            // 60s timeouts.
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            tools: Arc::new(RwLock::new(Vec::new())),
            capabilities: Arc::new(RwLock::new(McpCapabilities::default())),
            connected: Arc::new(RwLock::new(false)),
            request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Connect to the MCP server and initialize
    pub async fn connect(&self) -> Result<()> {
        info!(url = %self.url, "Connecting to MCP server");

        let request = InitializeRequest {
            protocol_version: MCP_DEFAULT_VERSION.to_string(),
            capabilities: ClientCapabilities {
                roots: Some(Roots { list_changed: true }),
                sampling: Some(Sampling {}),
            },
            client_info: ClientInfo {
                name: "operant-rs".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let response = self
            .send_request("initialize", Some(serde_json::to_value(request)?))
            .await?;

        let init_response: InitializeResponse = serde_json::from_value(response).map_err(|e| {
            crate::error::Error::ParseResponse(format!(
                "Failed to parse initialize response: {}",
                e
            ))
        })?;

        // iter-130: protocol version negotiation. Verify the server's
        // returned protocolVersion is one we support. If not, log a
        // warning but don't fail — many servers return a newer version
        // they're willing to speak; we just downgrade to our newest.
        match negotiate_protocol_version(&init_response.protocol_version) {
            Some(negotiated) => {
                if negotiated != init_response.protocol_version.as_str() {
                    debug!(
                        server_version = %init_response.protocol_version,
                        negotiated_version = %negotiated,
                        "MCP server returned a non-exact version match — using negotiated version"
                    );
                }
                debug!(
                    server = %init_response.server_info.name,
                    version = %init_response.server_info.version,
                    protocol = %negotiated,
                    "MCP server initialized"
                );
            }
            None => {
                warn!(
                    server_version = %init_response.protocol_version,
                    supported = ?MCP_SUPPORTED_VERSIONS,
                    "MCP server returned an unsupported protocolVersion. Continuing with best-effort compatibility (newest version we support)."
                );
            }
        }

        // Update capabilities
        {
            let mut caps = self.capabilities.write().await;
            caps.tools = init_response.capabilities.tools.is_some();
            caps.resources = init_response.capabilities.resources.is_some();
            caps.prompts = init_response.capabilities.prompts.is_some();
        }

        // Send initialized notification
        self.send_notification("notifications/initialized", Value::Null)
            .await?;

        // List available tools
        self.list_tools().await?;

        *self.connected.write().await = true;
        info!(url = %self.url, "Connected to MCP server");

        Ok(())
    }

    /// Disconnect from the MCP server
    pub async fn disconnect(&self) -> Result<()> {
        *self.connected.write().await = false;
        self.tools.write().await.clear();
        info!(url = %self.url, "Disconnected from MCP server");
        Ok(())
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// List tools from the server
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>> {
        let response = self.send_request("tools/list", None).await?;
        let tool_list: ToolListResult = serde_json::from_value(response).map_err(|e| {
            crate::error::Error::ParseResponse(format!("Failed to parse tool list: {}", e))
        })?;

        let tools: Vec<McpTool> = tool_list
            .tools
            .into_iter()
            .map(|def| McpTool::new(self.clone(), def))
            .collect();

        *self.tools.write().await = tools;

        let count = self.tools.read().await.len();
        debug!(count, "Listed MCP tools");
        Ok(self
            .tools
            .read()
            .await
            .iter()
            .map(|t| (*t.definition).clone())
            .collect())
    }

    /// Call a tool on the MCP server
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", Some(params)).await?;
        Ok(response)
    }

    /// Get all tools
    pub async fn get_tools(&self) -> Vec<McpTool> {
        self.tools.read().await.clone()
    }

    /// Get server capabilities
    pub async fn get_capabilities(&self) -> McpCapabilities {
        self.capabilities.read().await.clone()
    }

    /// Send a JSON-RPC request
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        // iter-130: use the monotonic AtomicU64 counter instead of
        // SystemTime nanos. The previous approach was non-monotonic
        // (clock skew), could collide under load, and the response
        // wasn't matched against the id.
        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(request_id),
        };

        let mut req_builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json");

        if let Some(ref token) = self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        let response = req_builder.json(&request).send().await?;

        // iter-130: handle 401 Unauthorized specifically. Previously a
        // 401 just returned a generic "MCP request failed" error and the
        // user had to manually re-run `operant mcp login`. Now we surface
        // a clear error that tells the caller to refresh the token.
        // (Full auto-refresh + retry is queued for iter-130b — needs the
        // OAuthManager wired into the HTTP path, which is a bigger
        // refactor since McpClient doesn't currently hold an OAuthManager
        // reference.)
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let body = response.text().await.unwrap_or_default();
            warn!(method = %method, body = %body, "MCP server returned 401 — token may be expired. Run `operant mcp login <server>` to refresh.");
            return Err(crate::error::Error::Agent(format!(
                "MCP server returned 401 Unauthorized (token may be expired). Run `operant mcp login` to refresh. Server response: {}",
                body
            )));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            error!(status = %status, body = %body, "MCP request failed");
            return Err(crate::error::Error::Agent(format!(
                "MCP request failed: {} - {}",
                status, body
            )));
        }

        let text = response.text().await?;
        if text.trim().is_empty() {
            return Err(crate::error::Error::Agent(
                "MCP server returned empty response body".to_string(),
            ));
        }

        let rpc_response: JsonRpcResponse = serde_json::from_str(&text).map_err(|e| {
            crate::error::Error::ParseResponse(format!("Failed to parse MCP response: {}", e))
        })?;

        // iter-187: Verify response ID matches request ID. Without this,
        // out-of-order or mismatched responses would silently return
        // the wrong result.
        if rpc_response.id != request_id {
            return Err(crate::error::Error::Agent(format!(
                "MCP response ID mismatch: expected {}, got {} (possible out-of-order response)",
                request_id, rpc_response.id
            )));
        }

        if let Some(error) = rpc_response.error {
            return Err(crate::error::Error::Agent(format!(
                "MCP error {}: {}",
                error.code, error.message
            )));
        }

        rpc_response
            .result
            .ok_or_else(|| crate::error::Error::Agent("No result in MCP response".to_string()))
    }

    /// Send a notification (no response expected)
    async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
            id: None,
        };

        let mut req_builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json");

        if let Some(ref token) = self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        let _ = req_builder.json(&request).send().await;
        Ok(())
    }
}

/// Bundled stdin/stdout for a stdio MCP transport
#[derive(Debug)]
struct StdioIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// Internal classification of a stdio request failure so the auto-restart
/// wrapper only retries genuine transport failures (broken pipe / EOF — the
/// child died and the tool provably did not complete), never protocol-level
/// errors (a timed-out or errored call is ambiguous — the server may have
/// executed the tool, so it must not be re-issued).
enum ReqError {
    Transport(crate::error::Error),
    Protocol(crate::error::Error),
}

/// MCP client that communicates over stdin/stdout of a child process
#[derive(Debug, Clone)]
pub struct McpStdioClient {
    /// Command to spawn
    command: String,
    /// Arguments for the command
    args: Vec<String>,
    /// Environment variables for the child process
    env: HashMap<String, String>,
    /// Child process handle
    child: Arc<RwLock<Option<Child>>>,
    /// Stdin/stdout IO pair (locked together for request-response atomicity)
    io: Arc<tokio::sync::Mutex<Option<StdioIo>>>,
    /// Connected tools from this server
    tools: Arc<RwLock<Vec<McpTool>>>,
    /// Server capabilities
    capabilities: Arc<RwLock<McpCapabilities>>,
    /// Whether connected
    connected: Arc<RwLock<bool>>,
    /// Atomic request ID counter
    request_id: Arc<AtomicU64>,
    /// Number of automatic reconnects performed (watchdog + call-time
    /// recovery). Observable via [`McpStdioClient::restart_count`].
    restarts: Arc<AtomicU64>,
    /// Consecutive read timeouts (hung-server detection). Reset on any
    /// successful exchange; when it reaches 2, the next request first
    /// force-reconnects to clear the wedged child (hermes watchdog parity).
    timeouts: Arc<AtomicU64>,
    /// True once a session has ever been established (set at the end of a
    /// successful connect). Lets call-time recovery distinguish a genuine
    /// post-crash call (io is None because the child was killed) from the
    /// never-connected initial handshake, which must never auto-reconnect.
    had_session: Arc<AtomicBool>,
    /// True while a reconnect is in flight. Suppresses call-time auto-retry
    /// inside the reconnect's own initialize handshake so a persistently
    /// dead server can never trigger a reconnect → connect → send_request →
    /// reconnect loop.
    reconnecting: Arc<AtomicBool>,
}

impl McpStdioClient {
    /// Create a new stdio MCP client
    pub fn new(
        command: impl Into<String>,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            env,
            child: Arc::new(RwLock::new(None)),
            io: Arc::new(tokio::sync::Mutex::new(None)),
            tools: Arc::new(RwLock::new(Vec::new())),
            capabilities: Arc::new(RwLock::new(McpCapabilities::default())),
            connected: Arc::new(RwLock::new(false)),
            request_id: Arc::new(AtomicU64::new(1)),
            restarts: Arc::new(AtomicU64::new(0)),
            timeouts: Arc::new(AtomicU64::new(0)),
            had_session: Arc::new(AtomicBool::new(false)),
            reconnecting: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Number of automatic reconnects performed since this client was built.
    pub async fn restart_count(&self) -> u64 {
        self.restarts.load(Ordering::SeqCst)
    }

    /// Non-blocking liveness check on the underlying child process.
    ///
    /// Returns `false` when the child has exited (crash/EOF) or was never
    /// spawned — the watchdog uses this to decide whether to reconnect.
    pub async fn process_alive(&self) -> bool {
        let mut guard = self.child.write().await;
        match guard.as_mut() {
            Some(child) => match child.try_wait() {
                // Ok(Some(_)) → the child has exited.
                Ok(Some(_)) => false,
                // Ok(None) → still running.
                Ok(None) => true,
                // Err → OS-level failure; treat as alive and let the
                // reconnect path surface the real error.
                Err(_) => true,
            },
            None => false,
        }
    }

    /// Drop the IO handles and kill + reap the current child (if any).
    /// Idempotent — safe to call on an already-dead or absent child.
    async fn kill_child(&self) {
        // Drop stdin/stdout first so any pending request unblocks with an
        // EOF/broken-pipe error instead of hanging on the 60s timeout.
        *self.io.lock().await = None;
        *self.connected.write().await = false;
        if let Some(mut child) = self.child.write().await.take() {
            let _ = child.kill().await;
            // Reap the zombie so the PID doesn't linger.
            let _ = child.wait().await;
        }
    }

    /// Reconnect: kill the current child (if any), then spawn a fresh one
    /// and run the full initialize handshake. Used by the stdio watchdog
    /// (crashed child) and by call-time recovery (broken pipe on a call).
    pub async fn reconnect(&self) -> Result<()> {
        // Mark the reconnect in flight BEFORE any handshake work so the
        // wrapper's call-time auto-retry is suppressed inside it (a dead
        // server must fail the reconnect cleanly, never loop).
        self.reconnecting.store(true, Ordering::SeqCst);
        let result = async {
            self.kill_child().await;
            self.restarts.fetch_add(1, Ordering::SeqCst);
            self.timeouts.store(0, Ordering::SeqCst);
            // Box the connect future to break the (syntactic) async-recursion
            // cycle reconnect → connect → send_request → reconnect. At runtime
            // the loop never fires — see `reconnecting` above.
            Box::pin(self.connect()).await
        }
        .await;
        self.reconnecting.store(false, Ordering::SeqCst);
        result
    }

    /// Connect to the MCP server by spawning the child process and initializing
    pub async fn connect(&self) -> Result<()> {
        // Pass `&str` (not `%`) so the tracing field is Send-safe across the
        // awaits below — `%` would capture a non-'static `Arguments` and make
        // this future !Send (blocks tokio::spawn of stdio MCP reconnect).
        info!(command = self.command.as_str(), "Spawning MCP stdio server");

        let mut cmd = tokio::process::Command::new(&self.command);
        cmd.args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            crate::error::Error::Agent(format!(
                "Failed to spawn MCP server '{}': {}",
                self.command, e
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            crate::error::Error::Agent("Failed to capture child stdin".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            crate::error::Error::Agent("Failed to capture child stdout".to_string())
        })?;
        // (plan 07.1) stderr was piped and never read — a chatty MCP
        // server (sourcehound logs to stderr) fills the 64KB OS buffer
        // and the child blocks on its next write, freezing the server.
        // Drain to tracing; the pipe can never fill.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::debug!(target: "mcp_child", line = %line, "mcp child stderr");
                    }
                }
            });
        }

        *self.child.write().await = Some(child);
        *self.io.lock().await = Some(StdioIo {
            stdin,
            stdout: BufReader::new(stdout),
        });

        // Send initialize request
        let request = InitializeRequest {
            protocol_version: MCP_DEFAULT_VERSION.to_string(),
            capabilities: ClientCapabilities {
                roots: Some(Roots { list_changed: true }),
                sampling: Some(Sampling {}),
            },
            client_info: ClientInfo {
                name: "operant-rs".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let response = self
            .send_request("initialize", Some(serde_json::to_value(request)?))
            .await?;

        let init_response: InitializeResponse = serde_json::from_value(response).map_err(|e| {
            crate::error::Error::ParseResponse(format!(
                "Failed to parse initialize response: {}",
                e
            ))
        })?;

        // iter-191: protocol version negotiation for stdio transport.
        // The HTTP path already did this; the stdio path was missing it.
        match negotiate_protocol_version(&init_response.protocol_version) {
            Some(negotiated) => {
                if negotiated != init_response.protocol_version.as_str() {
                    debug!(
                        server_version = init_response.protocol_version.as_str(),
                        negotiated_version = negotiated,
                        "MCP stdio server returned a non-exact version match — using negotiated version"
                    );
                }
                debug!(
                    server = init_response.server_info.name.as_str(),
                    version = init_response.server_info.version.as_str(),
                    protocol = negotiated,
                    "MCP stdio server initialized"
                );
            }
            None => {
                warn!(
                    server_version = init_response.protocol_version.as_str(),
                    supported = format!("{MCP_SUPPORTED_VERSIONS:?}").as_str(),
                    "MCP stdio server returned an unsupported protocolVersion. Continuing with best-effort compatibility."
                );
            }
        }

        // Update capabilities
        {
            let mut caps = self.capabilities.write().await;
            caps.tools = init_response.capabilities.tools.is_some();
            caps.resources = init_response.capabilities.resources.is_some();
            caps.prompts = init_response.capabilities.prompts.is_some();
        }

        // Send initialized notification
        self.send_notification("notifications/initialized", Value::Null)
            .await?;

        // List available tools
        self.list_tools().await?;

        *self.connected.write().await = true;
        self.had_session.store(true, Ordering::SeqCst);
        info!(
            command = self.command.as_str(),
            "Connected to MCP stdio server"
        );

        Ok(())
    }

    /// Disconnect from the MCP server by killing the child process
    pub async fn disconnect(&self) -> Result<()> {
        *self.connected.write().await = false;
        self.tools.write().await.clear();

        // Drop IO handles to close stdin (signals EOF to child)
        *self.io.lock().await = None;

        // Kill child process if still running
        if let Some(mut child) = self.child.write().await.take() {
            if let Err(e) = child.kill().await {
                let err = e.to_string();
                warn!(
                    error = err.as_str(),
                    "Failed to kill MCP stdio server process"
                );
            } else {
                debug!("MCP stdio server process killed");
            }
        }

        info!(
            command = self.command.as_str(),
            "Disconnected from MCP stdio server"
        );
        Ok(())
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// List tools from the server
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>> {
        let response = self.send_request("tools/list", None).await?;
        let tool_list: ToolListResult = serde_json::from_value(response).map_err(|e| {
            crate::error::Error::ParseResponse(format!("Failed to parse tool list: {}", e))
        })?;

        let tools: Vec<McpTool> = tool_list
            .tools
            .into_iter()
            .map(|def| McpTool::new_stdio(self.clone(), def))
            .collect();

        *self.tools.write().await = tools;

        // Hoist the count out of the tracing macro — an `.await` inside a
        // macro field captures a non-Send `dyn tracing::Value` across the
        // await and makes this future !Send (blocks stdio MCP reconnect
        // in a tokio::spawn).
        let count = self.tools.read().await.len();
        debug!(count, "Listed MCP stdio tools");
        Ok(self
            .tools
            .read()
            .await
            .iter()
            .map(|t| (*t.definition).clone())
            .collect())
    }

    /// Call a tool on the MCP server
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", Some(params)).await?;
        Ok(response)
    }

    /// Get all tools
    pub async fn get_tools(&self) -> Vec<McpTool> {
        self.tools.read().await.clone()
    }

    /// Get server capabilities
    pub async fn get_capabilities(&self) -> McpCapabilities {
        self.capabilities.read().await.clone()
    }

    /// Send a JSON-RPC request over stdin and read response from stdout,
    /// with automatic recovery for crashed stdio children (hermes
    /// `mcp_stdio_watchdog.py` parity):
    ///
    /// 1. A transport-level failure (broken pipe / EOF — the child died)
    ///    triggers ONE reconnect + retry of the same call (safe: a dead
    ///    child could not have executed the tool).
    /// 2. Two consecutive read timeouts (hung server) force a reconnect
    ///    BEFORE the next call so the wedged child is cleared without ever
    ///    re-issuing an ambiguous call.
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        // Hung-server detection: if the previous call timed out (>=2
        // consecutive), proactively kill + respawn the wedged child before
        // sending. The timed-out call itself is never re-issued; the fresh
        // child serves the NEXT call.
        if self.timeouts.load(Ordering::SeqCst) >= 2
            && self.had_session.load(Ordering::SeqCst)
            && !self.reconnecting.load(Ordering::SeqCst)
        {
            warn!(
                command = self.command.as_str(),
                "MCP stdio server appears hung — force-reconnecting before next call"
            );
            if let Err(e) = self.reconnect().await {
                let msg = e.to_string();
                return Err(crate::error::Error::Agent(format!(
                    "MCP stdio server hung; auto-reconnect failed: {msg}"
                )));
            }
        }

        match self.send_request_impl(method, params.clone()).await {
            Ok(value) => {
                // Any successful exchange clears the hang counter.
                self.timeouts.store(0, Ordering::SeqCst);
                Ok(value)
            }
            Err(ReqError::Transport(e)) => {
                // Child died / pipe broke mid-call — the tool provably did
                // not complete, so one auto-restart + retry is safe. Guarded
                // to post-connect calls so the initial handshake (connected
                // == false) can never loop.
                if self.had_session.load(Ordering::SeqCst)
                    && !self.reconnecting.load(Ordering::SeqCst)
                {
                    warn!(
                        command = self.command.as_str(),
                        error = %e,
                        "MCP stdio transport failure — auto-reconnecting and retrying once"
                    );
                    if let Err(re) = self.reconnect().await {
                        let rmsg = re.to_string();
                        let emsg = e.to_string();
                        return Err(crate::error::Error::Agent(format!(
                            "MCP stdio call failed ({emsg}); auto-reconnect also failed: {rmsg}"
                        )));
                    }
                    match self.send_request_impl(method, params).await {
                        Ok(value) => Ok(value),
                        Err(ReqError::Transport(e2)) => {
                            let msg = e2.to_string();
                            Err(crate::error::Error::Agent(format!(
                                "MCP stdio call failed after reconnect: {msg}"
                            )))
                        }
                        Err(ReqError::Protocol(e2)) => Err(e2),
                    }
                } else {
                    Err(e)
                }
            }
            Err(ReqError::Protocol(e)) => Err(e),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "error_response is a json! object, which always serializes"
    )]
    /// Send a JSON-RPC request over stdin and read response from stdout
    /// (single attempt — the wrapper above handles recovery).
    async fn send_request_impl(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> std::result::Result<Value, ReqError> {
        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(request_id),
        };

        let mut request_line = serde_json::to_string(&request).map_err(|e| {
            ReqError::Protocol(crate::error::Error::Agent(format!(
                "Failed to serialize request: {}",
                e
            )))
        })?;
        request_line.push('\n');

        let mut io_guard = self.io.lock().await;
        let io = io_guard.as_mut().ok_or_else(|| {
            // io None mid-session means the child was killed (crash or
            // watchdog) — a Transport failure so the wrapper reconnects.
            // The wrapper's had_session guard keeps a never-connected client
            // from auto-reconnecting.
            ReqError::Transport(crate::error::Error::Agent(
                "MCP stdio transport not connected".to_string(),
            ))
        })?;

        // Write request to stdin. A broken pipe here means the child died —
        // classified as a Transport failure so the wrapper can restart.
        io.stdin
            .write_all(request_line.as_bytes())
            .await
            .map_err(|e| {
                ReqError::Transport(crate::error::Error::Agent(format!(
                    "Failed to write to MCP stdin: {}",
                    e
                )))
            })?;
        io.stdin.flush().await.map_err(|e| {
            ReqError::Transport(crate::error::Error::Agent(format!(
                "Failed to flush MCP stdin: {}",
                e
            )))
        })?;

        // Read response from stdout — with a 60s timeout. Without this, a
        // hung MCP server would block the agent loop forever.
        let mut response_line = String::new();
        let read_result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            io.stdout.read_line(&mut response_line),
        )
        .await;
        match read_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                // EOF / read error — the child died mid-call.
                return Err(ReqError::Transport(crate::error::Error::Agent(format!(
                    "Failed to read from MCP stdout: {}",
                    e
                ))));
            }
            Err(_) => {
                // Read timeout — the server may be hung. Never re-issue this
                // call (it may have executed the tool); count it so the next
                // request force-reconnects once consecutive timeouts reach 2.
                let n = self.timeouts.fetch_add(1, Ordering::SeqCst) + 1;
                return Err(ReqError::Protocol(crate::error::Error::Agent(format!(
                    "MCP stdio server did not respond within 60s (possible hang; consecutive timeouts: {n})"
                ))));
            }
        }

        let trimmed = response_line.trim();
        if trimmed.is_empty() {
            return Err(ReqError::Protocol(crate::error::Error::Agent(
                "MCP server returned empty or whitespace-only response".to_string(),
            )));
        }

        // Check if this is a server-initiated request (has "method" field)
        // rather than a response (has "result" or "error"). If so, handle
        // it and read the next line for our actual response.
        let parsed: Value = serde_json::from_str(trimmed).map_err(|e| {
            ReqError::Protocol(crate::error::Error::ParseResponse(format!(
                "Failed to parse MCP stdio response: {}",
                e
            )))
        })?;

        if parsed.get("method").is_some() {
            // This is a server-initiated request (e.g. sampling/createMessage,
            // elicitation/create, notifications/*).
            let method = parsed["method"].as_str().unwrap_or("");
            let id = parsed.get("id").and_then(|v| v.as_u64());

            match method {
                "sampling/createMessage" => {
                    // Handle sampling request — the server wants us to
                    // sample an LLM. For now, return an error indicating
                    // sampling is not supported (the agent can be wired
                    // in later via a callback).
                    if let Some(req_id) = id {
                        let error_response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "error": {
                                "code": -32601,
                                "message": "sampling not supported by this client"
                            }
                        });
                        let mut err_line = serde_json::to_string(&error_response)
                            .expect("error_response is serializable");
                        err_line.push('\n');
                        let _ = io.stdin.write_all(err_line.as_bytes()).await;
                        let _ = io.stdin.flush().await;
                    }
                }
                "elicitation/create" => {
                    // Handle elicitation request — the server wants user
                    // input. Return an error for now.
                    if let Some(req_id) = id {
                        let error_response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "error": {
                                "code": -32601,
                                "message": "elicitation not supported by this client"
                            }
                        });
                        let mut err_line = serde_json::to_string(&error_response)
                            .expect("error_response is serializable");
                        err_line.push('\n');
                        let _ = io.stdin.write_all(err_line.as_bytes()).await;
                        let _ = io.stdin.flush().await;
                    }
                }
                "notifications/progress" | "notifications/cancelled" => {
                    // Server-side notifications — log and ignore.
                    debug!(method = method, "MCP server notification received");
                }
                _ => {
                    debug!(method = method, "Unknown MCP server request — ignoring");
                }
            }

            // Read the next line for our actual response
            response_line.clear();
            let read_result2 = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                io.stdout.read_line(&mut response_line),
            )
            .await;
            match read_result2 {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    return Err(ReqError::Transport(crate::error::Error::Agent(format!(
                        "Failed to read MCP stdout after server request: {}",
                        e
                    ))));
                }
                Err(_) => {
                    let n = self.timeouts.fetch_add(1, Ordering::SeqCst) + 1;
                    return Err(ReqError::Protocol(crate::error::Error::Agent(format!(
                        "MCP stdio server did not respond within 60s after server request (consecutive timeouts: {n})"
                    ))));
                }
            }
        }

        let trimmed = response_line.trim();
        if trimmed.is_empty() {
            return Err(ReqError::Protocol(crate::error::Error::Agent(
                "MCP server returned empty response after server request".to_string(),
            )));
        }

        let rpc_response: JsonRpcResponse = serde_json::from_str(trimmed).map_err(|e| {
            ReqError::Protocol(crate::error::Error::ParseResponse(format!(
                "Failed to parse MCP stdio response: {}",
                e
            )))
        })?;

        // Verify response ID matches request ID. Without this, out-of-order
        // or mismatched responses would silently return the wrong result.
        if rpc_response.id != request_id {
            return Err(ReqError::Protocol(crate::error::Error::Agent(format!(
                "MCP response ID mismatch: expected {}, got {} (possible out-of-order response)",
                request_id, rpc_response.id
            ))));
        }

        if let Some(error) = rpc_response.error {
            return Err(ReqError::Protocol(crate::error::Error::Agent(format!(
                "MCP error {}: {}",
                error.code, error.message
            ))));
        }

        rpc_response.result.ok_or_else(|| {
            ReqError::Protocol(crate::error::Error::Agent(
                "No result in MCP response".to_string(),
            ))
        })
    }

    /// Send a notification (no response expected)
    async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
            id: None,
        };

        let mut request_line = serde_json::to_string(&request).map_err(|e| {
            crate::error::Error::Agent(format!("Failed to serialize notification: {}", e))
        })?;
        request_line.push('\n');

        let mut io_guard = self.io.lock().await;
        if let Some(io) = io_guard.as_mut() {
            let _ = io.stdin.write_all(request_line.as_bytes()).await;
            let _ = io.stdin.flush().await;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SSE Transport (Server-Sent Events)
// ---------------------------------------------------------------------------

/// MCP client using SSE (Server-Sent Events) transport.
///
/// SSE transport works differently from HTTP:
/// 1. Client connects to the SSE endpoint via GET (long-lived stream)
/// 2. Server sends an `endpoint` event with a URI for POST requests
/// 3. Client sends JSON-RPC requests via HTTP POST to that endpoint
/// 4. Server sends responses back via the SSE stream
///
/// This is the transport used by older MCP servers (pre-Streamable HTTP).
#[derive(Debug, Clone)]
pub struct McpSseClient {
    /// SSE endpoint URL (e.g. http://localhost:8000/sse)
    sse_url: String,
    /// POST endpoint URL (received from the server's `endpoint` event)
    post_url: Arc<RwLock<Option<String>>>,
    /// Authentication token
    auth_token: Option<String>,
    /// HTTP client for POST requests
    http_client: reqwest::Client,
    /// Connected tools from this server
    tools: Arc<RwLock<Vec<McpTool>>>,
    /// Whether connected
    connected: Arc<RwLock<bool>>,
    /// Request ID counter
    request_id: Arc<AtomicU64>,
    /// Pending responses keyed by request ID
    pending: Arc<tokio::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    /// Background SSE reader task handle
    _reader_task: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl McpSseClient {
    /// Create a new SSE MCP client
    pub fn new(sse_url: impl Into<String>, auth_token: Option<String>) -> Self {
        Self {
            sse_url: sse_url.into(),
            post_url: Arc::new(RwLock::new(None)),
            auth_token,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            tools: Arc::new(RwLock::new(Vec::new())),
            connected: Arc::new(RwLock::new(false)),
            request_id: Arc::new(AtomicU64::new(0)),
            pending: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            _reader_task: Arc::new(RwLock::new(None)),
        }
    }

    /// Connect to the MCP SSE server
    pub async fn connect(&self) -> Result<()> {
        info!(url = %self.sse_url, "Connecting to MCP SSE server");

        // Start SSE stream
        let mut req_builder = self.http_client.get(&self.sse_url);
        if let Some(ref token) = self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }
        req_builder = req_builder.header("Accept", "text/event-stream");

        let response = req_builder.send().await?;

        if !response.status().is_success() {
            return Err(crate::error::Error::Agent(format!(
                "MCP SSE connection failed: {}",
                response.status()
            )));
        }

        // Clone state for the background reader task
        let post_url_clone = self.post_url.clone();
        let pending_clone = self.pending.clone();
        let sse_url = self.sse_url.clone();

        // Spawn background SSE reader task
        let task = tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete SSE events (separated by \n\n)
                        while let Some(pos) = buffer.find("\n\n") {
                            let event_block = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();

                            // Parse the SSE event
                            let mut event_type = String::new();
                            let mut data = String::new();

                            for line in event_block.lines() {
                                if let Some(rest) = line.strip_prefix("event: ") {
                                    event_type = rest.trim().to_string();
                                } else if let Some(rest) = line.strip_prefix("data: ") {
                                    data = rest.to_string();
                                }
                            }

                            if event_type == "endpoint" {
                                // Server sent us the POST endpoint URL
                                let endpoint_url = if data.starts_with("http") {
                                    data.clone()
                                } else {
                                    // Relative URL — resolve against the SSE URL
                                    let base = sse_url.trim_end_matches("/sse");
                                    format!("{}{}", base, data)
                                };
                                debug!(endpoint = %endpoint_url, "MCP SSE: received POST endpoint");
                                *post_url_clone.write().await = Some(endpoint_url);
                            } else if event_type == "message" || data.starts_with("{") {
                                // JSON-RPC response
                                if let Ok(parsed) = serde_json::from_str::<Value>(&data) {
                                    // Check if it's a response (has "id")
                                    if let Some(id) = parsed.get("id").and_then(|v| v.as_u64()) {
                                        let mut pending = pending_clone.lock().await;
                                        if let Some(tx) = pending.remove(&id) {
                                            let result = parsed
                                                .get("result")
                                                .cloned()
                                                .or_else(|| parsed.get("error").cloned())
                                                .unwrap_or(Value::Null);
                                            let _ = tx.send(result);
                                        }
                                    }
                                    // Server-initiated requests (sampling, elicitation, etc.)
                                    // are handled by the HTTP client's response processing
                                    // — for SSE, we just log them.
                                    if parsed.get("method").is_some() {
                                        debug!(method = ?parsed["method"], "MCP SSE: server-initiated request (not handled)");
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "MCP SSE stream error");
                        break;
                    }
                }
            }
            debug!("MCP SSE reader task ended");
        });

        *self._reader_task.write().await = Some(task);

        // Wait for the server to send the endpoint event (up to 10s)
        let timeout = tokio::time::Duration::from_secs(10);
        let start = std::time::Instant::now();
        loop {
            if self.post_url.read().await.is_some() {
                break;
            }
            if start.elapsed() > timeout {
                return Err(crate::error::Error::Agent(
                    "MCP SSE: timed out waiting for endpoint event".to_string(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Send initialize request
        let init_request = InitializeRequest {
            protocol_version: MCP_DEFAULT_VERSION.to_string(),
            capabilities: ClientCapabilities {
                roots: Some(Roots { list_changed: true }),
                sampling: Some(Sampling {}),
            },
            client_info: ClientInfo {
                name: "operant-rs".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let init_result = self
            .send_request("initialize", Some(serde_json::to_value(init_request)?))
            .await?;

        debug!(result = ?init_result, "MCP SSE: initialized");

        // Send initialized notification
        self.send_notification("notifications/initialized", Value::Null)
            .await?;

        // List tools
        self.list_tools().await?;

        *self.connected.write().await = true;
        info!(url = %self.sse_url, "Connected to MCP SSE server");

        Ok(())
    }

    /// Disconnect from the MCP SSE server
    pub async fn disconnect(&self) -> Result<()> {
        *self.connected.write().await = false;
        self.tools.write().await.clear();
        // Abort the reader task
        if let Some(handle) = self._reader_task.write().await.take() {
            handle.abort();
        }
        info!(url = %self.sse_url, "Disconnected from MCP SSE server");
        Ok(())
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// List tools from the server
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>> {
        let response = self.send_request("tools/list", None).await?;
        let tool_list: ToolListResult = serde_json::from_value(response).map_err(|e| {
            crate::error::Error::ParseResponse(format!("Failed to parse SSE tool list: {}", e))
        })?;

        let tools: Vec<McpTool> = tool_list
            .tools
            .into_iter()
            .map(|def| McpTool::new_sse(self.clone(), def))
            .collect();

        let defs: Vec<McpToolDefinition> = tools.iter().map(|t| t.definition().clone()).collect();
        *self.tools.write().await = tools;

        debug!(count = defs.len(), "Listed MCP SSE tools");
        Ok(defs)
    }

    /// Get tools
    pub async fn get_tools(&self) -> Vec<McpTool> {
        self.tools.read().await.clone()
    }

    /// Call a tool on the server
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });
        let result = self.send_request("tools/call", Some(params)).await?;
        Ok(result)
    }

    /// Send a JSON-RPC request via POST and wait for the response via SSE
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(request_id),
        };

        let post_url = self.post_url.read().await.clone().ok_or_else(|| {
            crate::error::Error::Agent("MCP SSE: POST endpoint not set".to_string())
        })?;

        let mut req_builder = self.http_client.post(&post_url);
        if let Some(ref token) = self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        // Register pending response channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id, tx);
        }

        // Send the request
        req_builder
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| crate::error::Error::Agent(format!("MCP SSE POST failed: {}", e)))?;

        // Wait for the response via the SSE stream (with 60s timeout)
        match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err(crate::error::Error::Agent(
                "MCP SSE: response channel closed".to_string(),
            )),
            Err(_) => {
                // Remove from pending on timeout
                let mut pending = self.pending.lock().await;
                pending.remove(&request_id);
                Err(crate::error::Error::Agent(
                    "MCP SSE: timed out waiting for response (60s)".to_string(),
                ))
            }
        }
    }

    /// Send a notification (no response expected)
    async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let post_url = self.post_url.read().await.clone().ok_or_else(|| {
            crate::error::Error::Agent("MCP SSE: POST endpoint not set".to_string())
        })?;

        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let mut req_builder = self.http_client.post(&post_url);
        if let Some(ref token) = self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        req_builder
            .header("Content-Type", "application/json")
            .json(&notification)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| {
                crate::error::Error::Agent(format!("MCP SSE notification failed: {}", e))
            })?;

        Ok(())
    }
}

/// MCP transport type — either HTTP, stdio, SSE, or Streamable-HTTP
#[derive(Debug, Clone)]
pub enum McpTransport {
    /// HTTP-based MCP client (plain POST + JSON-RPC)
    Http(McpClient),
    /// Stdio-based MCP client (child process)
    Stdio(McpStdioClient),
    /// SSE-based MCP client (Server-Sent Events, legacy)
    Sse(McpSseClient),
}

impl McpTransport {
    /// Check if the transport is connected
    pub async fn is_connected(&self) -> bool {
        match self {
            McpTransport::Http(c) => c.is_connected().await,
            McpTransport::Stdio(c) => c.is_connected().await,
            McpTransport::Sse(c) => c.is_connected().await,
        }
    }

    /// Get all tools from this transport
    pub async fn get_tools(&self) -> Vec<McpTool> {
        match self {
            McpTransport::Http(c) => c.get_tools().await,
            McpTransport::Stdio(c) => c.get_tools().await,
            McpTransport::Sse(c) => c.get_tools().await,
        }
    }

    /// Disconnect the transport
    pub async fn disconnect(&self) -> Result<()> {
        match self {
            McpTransport::Http(c) => c.disconnect().await,
            McpTransport::Stdio(c) => c.disconnect().await,
            McpTransport::Sse(c) => c.disconnect().await,
        }
    }

    /// Call a tool on this transport
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        match self {
            McpTransport::Http(c) => c.call_tool(name, arguments).await,
            McpTransport::Stdio(c) => c.call_tool(name, arguments).await,
            McpTransport::Sse(c) => c.call_tool(name, arguments).await,
        }
    }
}

/// A tool from an MCP server
#[derive(Debug, Clone)]
pub struct McpTool {
    transport: McpTransport,
    definition: Arc<McpToolDefinition>,
}

impl McpTool {
    /// Create a new MCP tool wrapper (HTTP transport)
    pub fn new(client: McpClient, definition: McpToolDefinition) -> Self {
        Self {
            transport: McpTransport::Http(client),
            definition: Arc::new(definition),
        }
    }

    /// Create a new MCP tool wrapper (stdio transport)
    pub fn new_stdio(client: McpStdioClient, definition: McpToolDefinition) -> Self {
        Self {
            transport: McpTransport::Stdio(client),
            definition: Arc::new(definition),
        }
    }

    /// Create a new MCP tool wrapper (SSE transport)
    pub fn new_sse(client: McpSseClient, definition: McpToolDefinition) -> Self {
        Self {
            transport: McpTransport::Sse(client),
            definition: Arc::new(definition),
        }
    }

    /// Get the tool name
    pub fn name(&self) -> &str {
        &self.definition.name
    }

    /// Get the tool definition
    pub fn definition(&self) -> &McpToolDefinition {
        &self.definition
    }
}

#[async_trait]
impl OperantTool for McpTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn schema(&self) -> ToolSchema {
        let params = serde_json::to_value(&self.definition.input_schema)
            .unwrap_or_else(|_| serde_json::json!({"type": "object"}));

        ToolSchema::new(&self.definition.name, &self.definition.description, params)
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let name = self.definition.name.clone();

        match self.transport.call_tool(&name, args).await {
            Ok(result) => ToolResult::success(name, result),
            Err(e) => ToolResult::error(name, e.to_string()),
        }
    }
}

/// A namespaced wrapper around McpTool for registration in ToolRegistry.
///
/// Prefixes the tool name with `mcp_{server_name}/` to avoid collisions
/// with built-in tools and tools from other MCP servers.
#[derive(Debug, Clone)]
pub struct McpNamespacedTool {
    qualified_name: String,
    tool: McpTool,
}

impl McpNamespacedTool {
    fn new(server_name: &str, tool: McpTool) -> Self {
        let qualified_name = qualified_tool_name(server_name, tool.name());
        Self {
            qualified_name,
            tool,
        }
    }
}

/// The registry name for an MCP tool.
///
/// It must be INJECTIVE: two different (server, tool) pairs must never
/// produce the same name, because the registry's `register` REPLACES an
/// existing tool with the same name (`tools.rs`: "Tool already
/// registered, replacing") and the sync's teardown removes BY NAME — so a
/// collision silently deletes another server's live tool.
///
/// The tool name comes from the MCP SERVER, which is untrusted, so the
/// encoding has to survive a hostile name.
///
/// The first version concatenated with `_` as the separator and sanitized
/// `/` and `-` to `_`. That was broken three ways:
///
/// 1. **Not injective.** Server `a` with tool `b_c` and server `a_b` with
///    tool `c` both produced `mcp_a_b_c`. The second registration replaced
///    the first, and the teardown list held the name twice — so tearing
///    one server down removed the other's still-live tool.
/// 2. **Lossy.** A server named `my.srv` produced `mcp_my.srv_tool`,
///    which contains a `.` — not in OpenAI's strict tool-name alphabet
///    (`^[a-zA-Z0-9_-]{1,64}$`). The whole request was rejected.
/// 3. **The provider saw a different name than the registry.** The
///    registry keys on `OperantTool::name()`, but the provider sees
///    `ToolSchema::name`, and `ToolSchema::new` runs `sanitize_tool_name`
///    over it — so any character it rewrote produced a name the model
///    could call but the dispatcher could not resolve.
///
/// The fix is a LOSSLESS escape and a separator that cannot appear in an
/// escaped component:
///
/// - Inside a component, `-` becomes `--` and `/` becomes `//`.
/// - The two components are joined with a SINGLE `-`.
///
/// An escaped run of `-` is always EVEN (a run of N becomes 2N), so a
/// single `-` in the qualified name is unambiguously the join. That makes
/// the encoding injective: the pair can be recovered by splitting on the
/// first single `-` and un-escaping each half.
///
/// The `_` in a tool name is left ALONE, so the common case stays
/// readable: `github` + `create_issue` is `mcp_github-create_issue`.
fn qualified_tool_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp_{}_{}",
        escape_component(server_name),
        escape_component(tool_name)
    )
}

/// Escape one name component into the provider's alphabet, INJECTIVELY.
///
/// Every byte outside `[A-Za-z0-9]` is encoded as `-` + two hex digits
/// (`_` is `-5f`, `-` is `-2d`, `.` is `-2e`, `/` is `-2f`, `:` is `-3a`).
/// The encoding is over BYTES, so a non-ASCII character — whose UTF-8
/// bytes are all outside the alphabet — is encoded losslessly and decodes
/// back to the same string.
///
/// Two properties make the qualified name unambiguous:
///
/// 1. The encoded component contains NO underscore (a literal `_` is
///    encoded as `-5f`), and it contains a `-` ONLY as the start of an
///    escape. So the encoding is UNIQUELY DECODABLE: scan, and on a `-`
///    read the next two characters as a hex byte.
/// 2. The two components are joined with a single `_`, which — by (1) —
///    can only be the join. So the qualified name's body carries EXACTLY
///    ONE underscore, and splitting on it recovers the two encoded
///    components.
///
/// Together: the pair is recoverable, so two different (server, tool)
/// pairs can never produce the same name.
///
/// WHY NOT "leave `_` alone and join with `_`": the first version of
/// this did, and server `a` with tool `b_c` and server `a_b` with tool `c`
/// both produced `mcp_a_b_c` — the registry REPLACES a tool with the same
/// name, and the sync's teardown removes BY NAME, so the second
/// registration silently replaced the first and tearing one server down
/// removed the other's still-live tool. And a version that left `_` alone
/// but encoded the illegal characters as `_<hex>_` was NOT injective
/// either: `-` and the literal string `_2d_` both encoded to `_2d_`.
///
/// The common case stays readable: `github` + `create_issue` is
/// `mcp_github_create-5fissue`.
fn escape_component(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.as_bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push(*byte as char);
            continue;
        }
        out.push('-');
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Undo [`escape_component`].
///
/// The inverse of the byte-wise encoding: on a `-`, read the next two
/// characters as a hex byte; anything else is a literal byte.
///
/// Test-only: the production path never decodes a qualified name — it
/// only builds one. The decoder exists so the round-trip test can prove
/// the encoding is injective BY CONSTRUCTION rather than by assertion.
#[cfg(test)]
fn decode_component(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'-' && i + 2 < bytes.len() {
            let hex = &encoded[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // The bytes came from a valid UTF-8 string, so this round-trips.
    String::from_utf8(out).unwrap_or_else(|_| encoded.to_string())
}

#[async_trait]
impl OperantTool for McpNamespacedTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        self.tool.description()
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            &self.qualified_name,
            self.tool.description(),
            self.tool.definition().input_schema.clone(),
        )
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        self.tool.execute(args, context).await
    }
}

/// MCP server connection manager
#[derive(Debug, Clone)]
pub struct McpManager {
    /// Connected servers (HTTP and stdio)
    servers: Arc<RwLock<HashMap<String, McpTransport>>>,
    /// Tracks tool names registered in ToolRegistry for incremental sync
    registered_tool_names: Arc<RwLock<Vec<String>>>,
    /// The last connect failure per server name.
    ///
    /// A server that fails to connect is NOT inserted into `servers`, so
    /// before this map existed the failure had nowhere to live: the caller
    /// got an `Err` and either aborted the whole command on it or dropped
    /// it, and the MCP view had no way to tell "not configured" from
    /// "configured and broken". The message is kept so the TUI's
    /// `McpViewStatus::Error` and its error detail row can name the cause.
    ///
    /// A successful connect clears the entry, so a server that was broken
    /// and then fixed does not keep showing its old error.
    connect_failures: Arc<RwLock<HashMap<String, String>>>,
}

impl Default for McpManager {
    fn default() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            registered_tool_names: Arc::new(RwLock::new(Vec::new())),
            connect_failures: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl McpManager {
    /// Create a new MCP manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Add and connect to an HTTP MCP server.
    ///
    /// If `auth_token` is `None`, falls back to any OAuth token persisted by
    /// `operant mcp login` (via `mcp_oauth::OAuthManager::get_token`).
    pub async fn add_server(
        &self,
        name: impl Into<String>,
        url: String,
        auth_token: Option<String>,
    ) -> Result<()> {
        let name = name.into();
        // Reject a name that is already connected instead of silently
        // clobbering the existing server. Checked both before the connect
        // (fail fast) and again under the write lock after the connect
        // (atomic — closes the TOCTOU between a concurrent caller's check
        // and insert). All CLI callers either use fresh names or pre-check
        // `contains`, so this only fires on genuine duplicate adds.
        if self.servers.read().await.contains_key(&name) {
            return Err(crate::error::Error::Agent(format!(
                "MCP server '{name}' is already connected"
            )));
        }
        let effective_token = match auth_token {
            Some(t) => Some(t),
            None => {
                // Try OAuth: if the user previously ran `operant mcp login`
                // for this server, a token will be persisted on disk.
                match crate::mcp_oauth::get_manager().get_token(&url).await {
                    Some(token) => {
                        tracing::debug!(
                            "MCP server {}: using OAuth access token from {}",
                            name,
                            url
                        );
                        Some(token.access_token)
                    }
                    None => None,
                }
            }
        };
        let client = McpClient::new(url, effective_token);
        if let Err(e) = client.connect().await {
            self.record_connect_failure(&name, &e.to_string()).await;
            return Err(e);
        }
        let mut servers = self.servers.write().await;
        if servers.contains_key(&name) {
            return Err(crate::error::Error::Agent(format!(
                "MCP server '{name}' is already connected"
            )));
        }
        servers.insert(name.clone(), McpTransport::Http(client));
        self.clear_connect_failure(&name).await;
        Ok(())
    }

    /// Record a connect failure so the failure is visible instead of lost.
    ///
    /// The message is the connect error's own text, so the TUI names the
    /// cause (a refused connection, a missing binary, a bad URL) rather
    /// than showing a bare "error".
    async fn record_connect_failure(&self, name: &str, message: &str) {
        tracing::warn!(
            server = name,
            error = message,
            "MCP server failed to connect"
        );
        self.connect_failures
            .write()
            .await
            .insert(name.to_string(), message.to_string());
    }

    /// Clear a recorded failure after a successful connect.
    async fn clear_connect_failure(&self, name: &str) {
        self.connect_failures.write().await.remove(name);
    }

    /// The last connect failure for a server, if it failed to connect.
    ///
    /// `None` means the server either connected or was never tried.
    pub async fn connect_failure(&self, name: &str) -> Option<String> {
        self.connect_failures.read().await.get(name).cloned()
    }

    /// Every recorded connect failure, keyed by server name.
    pub async fn all_connect_failures(&self) -> HashMap<String, String> {
        self.connect_failures.read().await.clone()
    }

    // iter-190: add_streamable_http_server was deleted — it was a duplicate
    // of add_server with a different McpTransport variant. Streamable HTTP
    // is now handled by the same McpClient::connect() path.

    /// Add and connect to a stdio MCP server (child process)
    pub async fn add_stdio_server(
        &self,
        name: impl Into<String>,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<()> {
        let name = name.into();
        let client = McpStdioClient::new(command, args, env);
        if let Err(e) = client.connect().await {
            self.record_connect_failure(&name, &e.to_string()).await;
            return Err(e);
        }
        self.servers
            .write()
            .await
            .insert(name.clone(), McpTransport::Stdio(client));
        self.clear_connect_failure(&name).await;
        Ok(())
    }

    /// Add and connect to an SSE MCP server
    pub async fn add_sse_server(
        &self,
        name: impl Into<String>,
        sse_url: String,
        auth_token: Option<String>,
    ) -> Result<()> {
        let name = name.into();
        let client = McpSseClient::new(sse_url, auth_token);
        if let Err(e) = client.connect().await {
            self.record_connect_failure(&name, &e.to_string()).await;
            return Err(e);
        }
        self.servers
            .write()
            .await
            .insert(name.clone(), McpTransport::Sse(client));
        self.clear_connect_failure(&name).await;
        Ok(())
    }

    /// Remove and disconnect a server
    pub async fn remove_server(&self, name: &str) -> Result<()> {
        if let Some(transport) = self.servers.write().await.remove(name) {
            transport.disconnect().await?;
        }
        // The server is gone, so its last failure is no longer about
        // anything the user can act on.
        self.connect_failures.write().await.remove(name);
        Ok(())
    }

    /// Get a clone of a server transport by name
    pub async fn get(&self, name: &str) -> Option<McpTransport> {
        self.servers.read().await.get(name).cloned()
    }

    /// Check if a server exists (async)
    pub async fn contains(&self, name: &str) -> bool {
        self.servers.read().await.contains_key(name)
    }

    /// Get all server names
    pub async fn server_names(&self) -> Vec<String> {
        self.servers.read().await.keys().cloned().collect()
    }

    /// Get a snapshot of all servers (HashMap clone)
    pub async fn all_servers(&self) -> HashMap<String, McpTransport> {
        self.servers.read().await.clone()
    }

    /// Get all tools from all servers
    pub async fn get_all_tools(&self) -> Vec<McpTool> {
        let mut tools = Vec::new();
        let servers = self.servers.read().await;
        for transport in servers.values() {
            if transport.is_connected().await {
                tools.extend(transport.get_tools().await);
            }
        }
        tools
    }

    /// Synchronize all MCP tools into a ToolRegistry with namespaced names.
    ///
    /// Each tool is registered as `mcp_{server_name}/{tool_name}` to avoid
    /// collisions with built-in tools. Previously registered MCP tools are
    /// unregistered before re-syncing. Returns the number of namespaced
    /// tools registered, so callers (e.g. the TUI /mcp reconnect task) can
    /// surface a precise "N tools synced" completion message.
    pub async fn sync_tools_to_registry(&self, registry: &ToolRegistry) -> usize {
        // Unregister all previously synced tools
        let mut prev_names = self.registered_tool_names.write().await;
        for name in prev_names.iter() {
            registry.unregister(name).await;
        }
        prev_names.clear();

        // Register current tools with namespaced names
        let servers = self.servers.read().await;
        let mut registered = 0usize;
        for (server_name, transport) in servers.iter() {
            if !transport.is_connected().await {
                continue;
            }
            for tool in transport.get_tools().await {
                let namespaced = McpNamespacedTool::new(server_name, tool);
                let name = namespaced.name().to_string();
                if registry
                    .register::<McpNamespacedTool>(namespaced)
                    .await
                    .is_ok()
                {
                    prev_names.push(name);
                    registered += 1;
                }
            }
        }
        registered
    }

    /// Spawn a background stdio watchdog task (hermes
    /// `mcp_stdio_watchdog.py` parity): every `interval`, sweep all
    /// connected stdio servers and reconnect any whose child process has
    /// exited (crash), then re-sync tools into `registry` when at least one
    /// server was restarted.
    ///
    /// The task is detached — it dies with the process — and is safe to
    /// spawn from short-lived CLI commands (an empty sweep is a cheap
    /// no-op). Returns the task handle so callers can abort it if needed.
    pub fn spawn_watchdog(
        &self,
        registry: ToolRegistry,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let restarted = manager.sweep_stdio_servers().await;
                if restarted > 0 {
                    let synced = manager.sync_tools_to_registry(&registry).await;
                    info!(
                        restarted,
                        synced, "MCP watchdog: stdio servers restarted, tools re-synced"
                    );
                }
            }
        })
    }

    /// One watchdog sweep: for every connected stdio server whose child has
    /// exited, reconnect it. Returns the number of servers restarted.
    /// Non-stdio transports are skipped (HTTP/SSE have no child to watch).
    async fn sweep_stdio_servers(&self) -> usize {
        let mut restarted = 0usize;
        let names: Vec<String> = self.servers.read().await.keys().cloned().collect();
        for name in names {
            let transport = {
                let servers = self.servers.read().await;
                servers.get(&name).cloned()
            };
            let Some(McpTransport::Stdio(client)) = transport else {
                continue;
            };
            if client.process_alive().await {
                continue;
            }
            warn!(
                server = name.as_str(),
                "MCP watchdog: stdio server child exited — reconnecting"
            );
            match client.reconnect().await {
                Ok(()) => {
                    info!(
                        server = name.as_str(),
                        "MCP watchdog: stdio server reconnected"
                    );
                    restarted += 1;
                }
                Err(e) => {
                    let msg = e.to_string();
                    error!(
                        server = name.as_str(),
                        error = msg.as_str(),
                        "MCP watchdog: reconnect failed (will retry next sweep)"
                    );
                }
            }
        }
        restarted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The qualified name must be INJECTIVE.
    ///
    /// The name is built from the server name and the tool name, and the
    /// tool name comes from the MCP SERVER — which is untrusted. The
    /// registry's `register` REPLACES an existing tool with the same
    /// name, and the sync's teardown removes BY NAME, so a collision
    /// silently deletes another server's live tool.
    ///
    /// The first version concatenated with `_` and sanitized `/` and `-`
    /// to `_`. Server `a` with tool `b_c` and server `a_b` with tool `c`
    /// both produced `mcp_a_b_c`.
    #[test]
    fn the_mcp_qualified_name_is_injective() {
        // The collisions that used to exist.
        assert_ne!(
            qualified_tool_name("a", "b_c"),
            qualified_tool_name("a_b", "c"),
            "two different (server, tool) pairs must not share a name"
        );
        assert_ne!(
            qualified_tool_name("a", "b__c"),
            qualified_tool_name("a_b", "_c"),
            "a double underscore must not reintroduce a collision"
        );
        assert_ne!(
            qualified_tool_name("a", "b"),
            qualified_tool_name("a", "b/c"),
            "a sanitised separator must not reintroduce a collision"
        );
        assert_ne!(
            qualified_tool_name("a", "b-c"),
            qualified_tool_name("a-b", "c"),
            "a hyphen must not reintroduce a collision"
        );
        assert_ne!(
            qualified_tool_name("a", "b.c"),
            qualified_tool_name("a.b", "c"),
            "a dot must not reintroduce a collision"
        );
        // The collision a NEAR-MISS encoding scheme produced: a `-` and
        // the literal string `_2d_` both encoded to `_2d_`. The encoding
        // now escapes `_` too, so the two differ.
        assert_ne!(
            qualified_tool_name("-", "x"),
            qualified_tool_name("_2d_", "x"),
            "an escaped character must not collide with its own literal spelling"
        );
        // The common case stays readable: the tool name's structure is
        // visible, with only the underscore escaped.
        assert_eq!(
            qualified_tool_name("github", "create_issue"),
            "mcp_github_create-5fissue",
            "the common name must stay readable"
        );
    }

    /// The qualified name must be inside the PROVIDER's tool-name
    /// alphabet.
    ///
    /// OpenAI strict requires `^[a-zA-Z0-9_-]{1,64}$`. A server named
    /// `my.srv` used to produce `mcp_my.srv_tool`, which carries a `.` —
    /// and the whole REQUEST was rejected, not just that tool.
    #[test]
    fn the_mcp_qualified_name_is_inside_the_provider_alphabet() {
        for (server, tool) in [
            ("my.srv", "do.thing"),
            ("srv:one", "a-tool"),
            ("a/b", "c"),
            ("weird name", "with spaces"),
            ("ünïcode", "tööl"),
            ("", ""),
        ] {
            let name = qualified_tool_name(server, tool);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "the qualified name {name:?} (server {server:?}, tool {tool:?}) \
                 must be inside the provider's alphabet"
            );
        }
    }

    /// The schema the PROVIDER sees must be the SAME name the registry
    /// keys on.
    ///
    /// `ToolSchema::new` runs `sanitize_tool_name` over its input, so a
    /// qualified name carrying a character it rewrote produced a
    /// mismatch: the model called one name, the dispatcher looked up
    /// another, and the call silently failed.
    #[test]
    fn the_schema_name_equals_the_registry_name() {
        for (server, tool) in [
            ("github", "create_issue"),
            ("my.srv", "do.thing"),
            ("a/b", "c"),
            ("srv:one", "a-tool"),
        ] {
            let qualified = qualified_tool_name(server, tool);
            let schema = ToolSchema::new(
                &qualified,
                "a tool",
                serde_json::json!({"type": "object", "properties": {}}),
            );
            assert_eq!(
                schema.name, qualified,
                "the provider must see the same name the registry keys on \
                 (server {server:?}, tool {tool:?})"
            );
        }
    }

    /// The encoding is INJECTIVE by construction: the pair is recoverable
    /// from the qualified name.
    ///
    /// The components are encoded with a byte-wise escape that never emits
    /// an underscore, and joined with a single `_`. So splitting the
    /// qualified name's body on that one underscore recovers the two
    /// encoded components, and decoding each one recovers the original.
    /// Two different pairs therefore cannot share a name.
    #[test]
    fn the_mcp_qualified_name_round_trips() {
        for (server, tool) in [
            ("github", "create_issue"),
            ("a", "b_c"),
            ("a_b", "c"),
            ("my.srv", "do.thing"),
            ("srv:one", "a-tool"),
            ("a/b", "c"),
            ("", ""),
            ("ünïcode", "tööl"),
            ("-", "_2d_"),
        ] {
            let name = qualified_tool_name(server, tool);
            let body = name
                .strip_prefix("mcp_")
                .unwrap_or_else(|| panic!("the name {name:?} must start with mcp_"));
            // Exactly ONE underscore: the join. The encoded components
            // never contain one.
            let parts: Vec<&str> = body.split('_').collect();
            assert_eq!(
                parts.len(),
                2,
                "the encoded body {body:?} must hold exactly one join"
            );
            assert_eq!(
                decode_component(parts[0]),
                server,
                "the server part must decode back (name {name:?})"
            );
            assert_eq!(
                decode_component(parts[1]),
                tool,
                "the tool part must decode back (name {name:?})"
            );
        }
    }

    #[test]
    fn test_tool_definition() {
        let def = McpToolDefinition {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        };

        assert_eq!(def.name, "test_tool");
    }

    #[tokio::test]
    async fn test_mcp_manager_empty() {
        let manager = McpManager::new();
        assert!(manager.server_names().await.is_empty());
    }

    /// RT-32: a server that fails to connect must leave a record behind.
    ///
    /// Before this, a failed connect had nowhere to live: the caller got an
    /// `Err` and either aborted the whole command on it or dropped it, and
    /// the MCP view could not tell "not configured" from "configured and
    /// broken". The message is the connect error's own text, so the view
    /// names the cause.
    #[tokio::test]
    async fn a_failed_connect_records_the_failure_for_the_view() {
        let manager = McpManager::new();
        let err = manager
            .add_stdio_server(
                "broken",
                "operant-mcp-test-binary-that-does-not-exist".to_string(),
                vec![],
                HashMap::new(),
            )
            .await
            .expect_err("a missing binary must fail the connect");
        assert!(!err.to_string().is_empty(), "the error must say something");

        let failure = manager
            .connect_failure("broken")
            .await
            .expect("the failure must be recorded");
        assert!(
            !failure.is_empty(),
            "the recorded message must not be empty"
        );
        assert!(
            manager.all_connect_failures().await.contains_key("broken"),
            "the failure must appear in the snapshot the TUI reads"
        );
        // The server did not connect, so it is not in the connected map.
        assert!(!manager.contains("broken").await);
    }

    /// A server that was never tried has no failure, so the view can tell
    /// it apart from one that was tried and broke.
    #[tokio::test]
    async fn a_server_that_was_never_tried_has_no_failure() {
        let manager = McpManager::new();
        assert!(manager.connect_failure("never-tried").await.is_none());
    }

    /// A successful connect clears an earlier failure, so a server that was
    /// broken and then fixed does not keep showing its old error.
    #[tokio::test]
    async fn a_successful_connect_clears_an_earlier_failure() {
        let manager = McpManager::new();
        let (client, _dir) = fake_stdio_client();
        client.connect().await.expect("fake server connect");
        // Record a failure for the same name, then connect for real.
        manager
            .add_stdio_server(
                "flaky",
                "operant-mcp-test-binary-that-does-not-exist".to_string(),
                vec![],
                HashMap::new(),
            )
            .await
            .expect_err("the first connect fails");
        assert!(manager.connect_failure("flaky").await.is_some());

        // Re-adding the same name with a working server must clear it.
        // `add_stdio_server` refuses a duplicate only for HTTP; stdio
        // replaces, so the success path runs and clears the failure.
        let (client2, _dir2) = fake_stdio_client();
        let _ = client2;
        manager
            .add_stdio_server(
                "flaky",
                "python3".to_string(),
                vec![_dir.join("fake_server.py").to_string_lossy().to_string()],
                HashMap::new(),
            )
            .await
            .expect("the second connect succeeds");
        assert!(
            manager.connect_failure("flaky").await.is_none(),
            "a successful connect must clear the earlier failure"
        );
    }

    #[tokio::test]
    async fn test_sync_tools_to_registry_returns_registered_count() {
        // Fabricate a connected HTTP client with two tools. The fields are
        // Arc-shared and reachable from this child test module.
        let client = McpClient::new("http://localhost:9999", None);
        *client.connected.write().await = true;
        let def = |name: &str| McpToolDefinition {
            name: name.to_string(),
            description: "test tool".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
        };
        client
            .tools
            .write()
            .await
            .push(McpTool::new(client.clone(), def("search")));
        client
            .tools
            .write()
            .await
            .push(McpTool::new(client.clone(), def("save")));

        let manager = McpManager::new();
        manager
            .servers
            .write()
            .await
            .insert("test-server".to_string(), McpTransport::Http(client));

        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));
        let count = manager.sync_tools_to_registry(&registry).await;
        assert_eq!(count, 2, "both tools must be registered");

        // Tools are namespaced mcp_{server}_{tool} in the live registry.
        let names: Vec<String> = registry
            .get_schemas()
            .await
            .iter()
            .map(|s| s.name.clone())
            .collect();
        // The qualified name joins the two components with a `_`, and
        // every byte outside `[A-Za-z0-9]` is encoded as `-` + two hex
        // digits — so the join can never be forged from a component, and
        // the pair is recoverable. The server here is `test-server`, so
        // it encodes to `test-2dserver`.
        assert!(names.iter().any(|n| n == "mcp_test-2dserver_search"));
        assert!(names.iter().any(|n| n == "mcp_test-2dserver_save"));

        // Re-syncing is idempotent and reports the same count.
        let count2 = manager.sync_tools_to_registry(&registry).await;
        assert_eq!(count2, 2);
    }

    /// Minimal MCP stdio server (single-quoted python, no backslashes) used
    /// by the watchdog tests. Responds to initialize / tools/list / any other
    /// request with an empty result; stays silent on notifications (no id).
    const FAKE_MCP_SERVER: &str = r#"import sys, json
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
    mid = req.get('method')
    if mid == 'initialize':
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'fake', 'version': '1.0'}}
    elif mid == 'tools/list':
        result = {'tools': []}
    else:
        result = {}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
"#;

    /// Build a stdio client pointed at a freshly-written copy of the fake
    /// MCP server script. The temp dir is returned so tests can keep it
    /// alive (or it is cleaned up by the OS on exit).
    fn fake_stdio_client() -> (McpStdioClient, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_mcp_fake_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("fake_server.py");
        std::fs::write(&script, FAKE_MCP_SERVER).expect("write fake server");
        let client = McpStdioClient::new(
            "python3",
            vec![script.to_string_lossy().to_string()],
            HashMap::new(),
        );
        (client, dir)
    }

    #[tokio::test]
    async fn test_process_alive_detects_child_exit_and_reconnect_recovers() {
        let (client, _dir) = fake_stdio_client();
        client.connect().await.expect("fake server connect");
        assert!(client.is_connected().await);
        assert!(
            client.process_alive().await,
            "child should be alive after connect"
        );

        // Kill the child in place (keep the handle) so process_alive
        // exercises the try_wait Ok(Some(_)) path, then poll until the OS
        // reaps it (SIGKILL is fast but reaping is async).
        {
            let mut guard = client.child.write().await;
            if let Some(child) = guard.as_mut() {
                child.kill().await.expect("kill child");
            }
        }
        let mut dead = false;
        for _ in 0..20 {
            if !client.process_alive().await {
                dead = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(dead, "child should be detected as dead within 2s");

        // Reconnect spawns a fresh child and increments the restart counter.
        client.reconnect().await.expect("reconnect after crash");
        assert!(client.is_connected().await);
        assert_eq!(client.restart_count().await, 1);
        assert!(client.process_alive().await, "fresh child should be alive");

        // The transport still routes calls through the shared client.
        let value = client
            .call_tool("echo", serde_json::json!({}))
            .await
            .expect("call after reconnect");
        assert_eq!(value, serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_call_time_auto_restart_recovers_from_crashed_child() {
        let (client, _dir) = fake_stdio_client();
        client.connect().await.expect("fake server connect");

        // Crash the child mid-session; the NEXT call must transparently
        // reconnect + retry (hermes watchdog call-time parity).
        client.kill_child().await;
        assert!(!client.process_alive().await);

        let value = client
            .call_tool("echo", serde_json::json!({}))
            .await
            .expect("auto-recovered call");
        assert_eq!(value, serde_json::json!({}));
        assert_eq!(
            client.restart_count().await,
            1,
            "call-time recovery must count one restart"
        );
        assert!(client.is_connected().await);
    }

    #[tokio::test]
    async fn test_manager_sweep_restarts_dead_stdio_server() {
        let (client, _dir) = fake_stdio_client();
        client.connect().await.expect("fake server connect");

        let manager = McpManager::new();
        manager
            .servers
            .write()
            .await
            .insert("fake".to_string(), McpTransport::Stdio(client.clone()));

        // Crash the child; the sweep must detect and reconnect it.
        client.kill_child().await;
        assert!(!client.process_alive().await);

        let restarted = manager.sweep_stdio_servers().await;
        assert_eq!(restarted, 1, "sweep must restart the dead server");
        assert_eq!(client.restart_count().await, 1);
        assert!(client.is_connected().await);
        assert!(client.process_alive().await);
    }

    #[tokio::test]
    async fn test_spawn_watchdog_restarts_dead_server() {
        let (client, _dir) = fake_stdio_client();
        client.connect().await.expect("fake server connect");

        let manager = McpManager::new();
        manager
            .servers
            .write()
            .await
            .insert("fake".to_string(), McpTransport::Stdio(client.clone()));
        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));

        // Crash the child, then let the background watchdog recover it.
        client.kill_child().await;
        assert!(!client.process_alive().await);

        let handle = manager.spawn_watchdog(registry, std::time::Duration::from_millis(50));

        // Poll instead of sleeping a fixed window: under parallel test load
        // the reconnect handshake (spawn python3 + initialize) can outlive a
        // 600ms sleep even though the watchdog fired. Wait until the child is
        // both restarted and reconnected, with a generous deadline.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if client.process_alive().await && client.is_connected().await {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watchdog must reconnect the dead server (restarts={}, alive={}, connected={})",
                client.restart_count().await,
                client.process_alive().await,
                client.is_connected().await
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        assert_eq!(
            client.restart_count().await,
            1,
            "watchdog must reconnect the dead server exactly once"
        );
        handle.abort();
    }
}
