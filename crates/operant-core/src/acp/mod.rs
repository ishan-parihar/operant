//! ACP (Agent Client Protocol) — JSON-RPC based protocol over stdio.
//!
//! Allows external tools (IDEs, scripts, CI systems) to control the Operant agent
//! via line-delimited JSON-RPC messages on stdin/stdout.
//!
//! # Agent Client Protocol methods
//!
//! These are the methods a real ACP client (Zed, an editor's agent panel) speaks.
//! They are the ones that make this interoperable; the four below are operant's
//! own extensions, kept because existing callers depend on them.
//!
//! | Method           | Kind         | Description                          |
//! |------------------|--------------|--------------------------------------|
//! | `initialize`     | request      | Negotiate protocol version           |
//! | `session/new`    | request      | Create a session, returns a sessionId |
//! | `session/prompt` | request      | Send a prompt; streams `session/update`, returns `stopReason` |
//! | `session/cancel` | notification | Cancel the in-flight prompt for a session |
//! | `session/update` | notification | Emitted BY US while a prompt runs     |
//!
//! # operant extensions
//!
//! | Method    | Description                              |
//! |-----------|------------------------------------------|
//! | `ping`    | Health check — returns `"pong"`          |
//! | `status`  | Returns current agent state              |
//! | `command` | Executes a command in the agent's context|
//! | `stop`    | Gracefully shuts down the ACP server     |
//!
//! # Known limitations, stated rather than implied
//!
//! - **Streaming granularity is one chunk per turn, not per token.** The
//!   [`AcpHandler`] seam exposes a single `execute_command(&str) -> String`, so
//!   there is nowhere for token-level deltas to arrive from. `session/prompt`
//!   emits one `agent_message_chunk` holding the turn's full text. That is
//!   wire-correct and a real client renders it correctly; it is simply not
//!   incremental. Wiring true streaming means adding a callback to the trait.
//! - **Only the `text` content block is implemented.** Image, audio, and
//!   resource blocks are passed through in the prompt but not rendered, and
//!   never emitted outbound.
//! - **`session/load` is not implemented**, so `loadSession` is advertised as
//!   `false` in [`agent_capabilities`]. Advertising a capability that is not
//!   implemented is exactly the failure this protocol makes expensive: a
//!   client will call it and hang.

pub mod server;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

// ─── Agent Client Protocol constants ───────────────────────────────────────

/// The ACP protocol version this server implements.
pub const ACP_PROTOCOL_VERSION: u32 = 1;

/// A content block in a prompt or an agent message.
///
/// Only `text` is constructed by this server. The other variants exist so that
/// an inbound prompt carrying them deserializes rather than failing the whole
/// request, and so the shape matches the protocol — but an inbound non-text
/// block contributes no text to the turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        #[serde(default)]
        data: String,
        #[serde(default)]
        mime_type: String,
    },
    Audio {
        #[serde(default)]
        data: String,
        #[serde(default)]
        mime_type: String,
    },
    Resource {
        #[serde(default)]
        uri: String,
    },
}

impl ContentBlock {
    /// The text this block contributes to a prompt, if any.
    ///
    /// A non-text block contributes nothing rather than erroring: a client that
    /// sends an image alongside text should still get its text answered.
    pub fn as_prompt_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }
}

/// A JSON-RPC 2.0 notification: a request with no `id`, which by spec must
/// never receive a response.
#[derive(Debug, Serialize)]
pub struct RpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcNotification {
    /// Build a `session/update` notification carrying one agent message chunk.
    pub fn agent_message_chunk(session_id: &str, text: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            method: "session/update".to_string(),
            params: Some(serde_json::json!({
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text },
                },
            })),
        }
    }
}

/// What `operant acp` advertises it can do.
///
/// `loadSession` is `false` because `session/load` is not implemented. See the
/// module note on why that must not be advertised as `true`.
pub fn agent_capabilities() -> Value {
    serde_json::json!({
        "loadSession": false,
        "promptCapabilities": {
            "image": false,
            "audio": false,
            "embeddedContext": false,
        },
    })
}

// ─── Session registry ───────────────────────────────────────────────────────

/// Live ACP sessions, and the ones cancelled mid-prompt.
///
/// Session ids are minted from a process-local counter rather than a UUID so
/// this adds no dependency. They are opaque to the client either way, and
/// uniqueness only has to hold within one server process.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    next_id: AtomicU64,
    live: std::sync::Mutex<HashSet<String>>,
}

impl SessionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            live: std::sync::Mutex::new(HashSet::new()),
        }
    }

    /// Mint and register a new session id.
    pub fn create(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("operant-session-{n}");
        // A poisoned lock here would mean a previous holder panicked while
        // holding it. The set is not load-bearing state — losing it costs a
        // client its sessions, not the process its integrity — so recover
        // rather than propagating a panic across an async boundary.
        match self.live.lock() {
            Ok(mut set) => {
                set.insert(id.clone());
            }
            Err(poisoned) => {
                poisoned.into_inner().insert(id.clone());
            }
        }
        id
    }

    /// Whether a session id was minted by this registry and not yet ended.
    pub fn is_live(&self, session_id: &str) -> bool {
        match self.live.lock() {
            Ok(set) => set.contains(session_id),
            Err(poisoned) => poisoned.into_inner().contains(session_id),
        }
    }

    /// End a session, returning whether it was live.
    pub fn end(&self, session_id: &str) -> bool {
        match self.live.lock() {
            Ok(mut set) => set.remove(session_id),
            Err(poisoned) => poisoned.into_inner().remove(session_id),
        }
    }
}

// ─── JSON-RPC Protocol Types ───────────────────────────────────────────────

/// A valid JSON-RPC 2.0 request.
///
/// `jsonrpc` and `id` are `#[serde(default)]` so that malformed requests are
/// rejected with the correct error code (-32600 Invalid Request) instead of a
/// parse error (-32700): a missing `jsonrpc` member or a request without an
/// `id` (a JSON-RPC notification) must still deserialize. (R18)
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[serde(default)]
    pub jsonrpc: String,
    /// `None` = id omitted → notification (no response). `Some(Value::Null)`
    /// is an *explicit* null id, which per JSON-RPC 2.0 is a valid id that
    /// still receives a response with `"id": null`. (R18: the Option
    /// distinguishes these two cases, which a defaulted `Value` cannot. The
    /// `deserialize_with` is required because plain `Option<T>` collapses an
    /// explicit JSON `null` into `None`.)
    #[serde(default, deserialize_with = "deserialize_id")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Presence-aware deserializer for the `id` member: a missing `id` yields
/// `None` (a notification), while an explicit `null` yields `Some(Value::Null)`
/// — a valid request id that still receives a response. (R18)
fn deserialize_id<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

impl RpcRequest {
    /// True when this is a JSON-RPC notification (no `id`), which per the
    /// spec must NOT receive a response.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// The id to echo in a response (null when absent / a notification).
    pub fn response_id(&self) -> Value {
        self.id.clone().unwrap_or(Value::Null)
    }
}

/// Validate a parsed request against the JSON-RPC 2.0 framing rules.
///
/// Returns `Err(code)` with the spec error code on violation:
/// - `-32600` (Invalid Request) when `jsonrpc != "2.0"` or the `id` is not
///   a string, number, or null.
pub fn validate_request(request: &RpcRequest) -> Result<(), i32> {
    if request.jsonrpc != "2.0" {
        return Err(-32600);
    }
    if let Some(id) = &request.id
        && !(id.is_null() || id.is_string() || id.is_number())
    {
        return Err(-32600);
    }
    Ok(())
}

/// A valid JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    /// Create a successful response with the given result.
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    pub fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ─── Agent State ──────────────────────────────────────────────────────────

/// Current state of the agent, returned by the `status` method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent is running and processing
    Running,
    /// Agent is paused (user-requested halt)
    Paused,
    /// Agent is idle (running but not actively processing)
    #[default]
    Idle,
    /// Agent encountered an error
    Error(String),
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Running => write!(f, "running"),
            AgentState::Paused => write!(f, "paused"),
            AgentState::Idle => write!(f, "idle"),
            AgentState::Error(msg) => write!(f, "error: {}", msg),
        }
    }
}

/// Shared agent-state tracker so the `status` method reports real activity
/// while a command is executing. (R18: the CLI handler previously returned
/// `Idle` unconditionally, so `status` lied during a running command.)
///
/// `Clone` shares the same underlying state (via `Arc`), which lets a
/// long-running `spawn_blocking` command task update the tracker from the
/// handler side without races.
#[derive(Clone, Default)]
pub struct AgentStateTracker(std::sync::Arc<std::sync::Mutex<AgentState>>);

impl AgentStateTracker {
    /// Create a tracker initialized to `Idle`.
    pub fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(AgentState::Idle)))
    }

    /// Set the current agent state.
    pub fn set(&self, state: AgentState) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = state;
        }
    }

    /// Read the current agent state.
    pub fn get(&self) -> AgentState {
        self.0
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| AgentState::Error("state tracker poisoned".to_string()))
    }
}

// ─── ACP Handler Trait ────────────────────────────────────────────────────

/// Trait that must be implemented to connect the ACP server to an actual agent.
///
/// The implementor provides access to the agent's state and command execution.
#[async_trait::async_trait]
pub trait AcpHandler: Send + Sync {
    /// Return the current agent state.
    async fn agent_state(&self) -> AgentState;

    /// Execute a command string in the agent's context and return the result.
    async fn execute_command(&self, command: &str) -> Result<String, String>;

    /// Optional: return server metadata (name, version, uptime).
    fn server_info(&self) -> Value {
        serde_json::json!({
            "name": "operant-acp",
            "version": env!("CARGO_PKG_VERSION"),
        })
    }
}

// ─── Dispatch Logic ───────────────────────────────────────────────────────

/// The result of dispatching one request.
#[derive(Debug, Default)]
pub struct DispatchOutcome {
    /// Notifications to write to stdout BEFORE the response. Per JSON-RPC these
    /// carry no `id` and must never be answered.
    pub notifications: Vec<RpcNotification>,
    /// The response, or `None` when the request was a notification.
    pub response: Option<RpcResponse>,
    /// Whether the server should shut down (true for `stop`).
    pub should_shutdown: bool,
}

/// Dispatch a JSON-RPC request.
///
/// Emits the real Agent Client Protocol methods plus operant's four extensions.
/// `session/prompt` produces `session/update` notifications, which is why the
/// outcome carries notifications separately from the response: a JSON-RPC
/// notification has no `id` and must never be answered, so it cannot travel in
/// the same slot as a response.
pub async fn dispatch(
    request: &RpcRequest,
    handler: &dyn AcpHandler,
    sessions: &SessionRegistry,
) -> DispatchOutcome {
    let mut outcome = DispatchOutcome::default();

    match request.method.as_str() {
        // --- Agent Client Protocol ---
        "initialize" => {
            outcome.response = Some(handle_initialize(request));
        }
        "session/new" => {
            outcome.response = Some(handle_session_new(request, sessions));
        }
        "session/prompt" => {
            let (response, notifications) = handle_session_prompt(request, handler, sessions).await;
            outcome.response = Some(response);
            outcome.notifications = notifications;
        }
        "session/cancel" => {
            // A notification by spec: acknowledge with nothing at all.
            sessions.end(&session_id_param(request).unwrap_or_default());
            outcome.response = None;
        }
        // --- operant extensions ---
        "ping" => outcome.response = Some(handle_ping(request)),
        "status" => outcome.response = Some(handle_status(request, handler).await),
        "command" => outcome.response = Some(handle_command(request, handler).await),
        "stop" => {
            outcome.response = Some(handle_stop(request));
            outcome.should_shutdown = true;
        }
        _ => {
            outcome.response = Some(RpcResponse::error(
                request.response_id(),
                -32601,
                format!("Method not found: {}", request.method),
            ));
        }
    }

    outcome
}

fn handle_ping(request: &RpcRequest) -> RpcResponse {
    RpcResponse::success(request.response_id(), serde_json::json!("pong"))
}

// ─── Agent Client Protocol handlers ─────────────────────────────────────────

/// Read the `sessionId` member from a request's params.
fn session_id_param(request: &RpcRequest) -> Option<String> {
    request
        .params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// `initialize` — negotiate the protocol version and advertise capabilities.
///
/// Echoes the version the server implements rather than the client's, per ACP:
/// the client proposes, the server states what it will speak.
fn handle_initialize(request: &RpcRequest) -> RpcResponse {
    let requested = request
        .params
        .get("protocolVersion")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    RpcResponse::success(
        request.response_id(),
        serde_json::json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "agentCapabilities": agent_capabilities(),
            "authMethods": [],
            // Not part of ACP. A client that proposed a version we do not speak
            // needs to be able to tell the difference between "we negotiated down
            // to something older" and "we ignored you", and the spec result has
            // nowhere to say so. Silently answering 1 to a client's 99 would
            // produce a confusing downstream failure instead of a clear one.
            "requestedProtocolVersion": requested,
        }),
    )
}

/// `session/new` — mint a session and return its id.
fn handle_session_new(request: &RpcRequest, sessions: &SessionRegistry) -> RpcResponse {
    let session_id = sessions.create();
    RpcResponse::success(
        request.response_id(),
        serde_json::json!({ "sessionId": session_id }),
    )
}

/// `session/prompt` — run a prompt, streaming the answer as it arrives.
///
/// Returns the response AND the `session/update` notifications to write first.
///
/// The prompt is routed through [`AcpHandler::execute_command`], which is the
/// single seam this trait offers for agent execution. The consequence is
/// recorded in the module docs: the agent's whole turn arrives as one chunk,
/// because there is no per-token channel to receive it on.
async fn handle_session_prompt(
    request: &RpcRequest,
    handler: &dyn AcpHandler,
    sessions: &SessionRegistry,
) -> (RpcResponse, Vec<RpcNotification>) {
    let Some(session_id) = session_id_param(request) else {
        return (
            RpcResponse::error(
                request.response_id(),
                -32602,
                "Missing required parameter: 'sessionId'",
            ),
            Vec::new(),
        );
    };

    if !sessions.is_live(&session_id) {
        return (
            RpcResponse::error(
                request.response_id(),
                -32002,
                format!("Unknown session: {session_id}"),
            ),
            Vec::new(),
        );
    }

    let prompt_text = request
        .params
        .get("prompt")
        .and_then(|v| v.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| serde_json::from_value::<ContentBlock>(b.clone()).ok())
                .filter_map(|b| b.as_prompt_text().map(str::to_string))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    if prompt_text.trim().is_empty() {
        return (
            RpcResponse::error(
                request.response_id(),
                -32602,
                "Prompt contains no text content blocks",
            ),
            Vec::new(),
        );
    }

    match handler.execute_command(&prompt_text).await {
        Ok(text) => {
            let notification = RpcNotification::agent_message_chunk(&session_id, &text);
            (
                RpcResponse::success(
                    request.response_id(),
                    serde_json::json!({ "stopReason": "end_turn" }),
                ),
                vec![notification],
            )
        }
        Err(err) => {
            // The turn failed. Report it as a JSON-RPC error rather than a
            // successful empty turn, so a client can tell the difference between
            // "the agent declined" and "the agent broke".
            (
                RpcResponse::error(
                    request.response_id(),
                    -32000,
                    format!("Agent turn failed: {err}"),
                ),
                Vec::new(),
            )
        }
    }
}

async fn handle_status(request: &RpcRequest, handler: &dyn AcpHandler) -> RpcResponse {
    let state = handler.agent_state().await;
    let info = handler.server_info();

    RpcResponse::success(
        request.response_id(),
        serde_json::json!({
            "agent": {
                "state": state,
            },
            "server": info,
        }),
    )
}

async fn handle_command(request: &RpcRequest, handler: &dyn AcpHandler) -> RpcResponse {
    let command = request
        .params
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if command.is_empty() {
        return RpcResponse::error(
            request.response_id(),
            -32602,
            "Missing required parameter: 'command'",
        );
    }

    match handler.execute_command(command).await {
        Ok(output) => RpcResponse::success(
            request.response_id(),
            serde_json::json!({
                "success": true,
                "output": output,
            }),
        ),
        Err(err) => RpcResponse::success(
            request.response_id(),
            serde_json::json!({
                "success": false,
                "output": err,
            }),
        ),
    }
}

fn handle_stop(request: &RpcRequest) -> RpcResponse {
    RpcResponse::success(
        request.response_id(),
        serde_json::json!({
            "message": "ACP server shutting down gracefully",
        }),
    )
}

// ─── Convenience ──────────────────────────────────────────────────────────

/// Parse a JSON-RPC request from a raw JSON string.
pub fn parse_request(line: &str) -> Result<RpcRequest, String> {
    serde_json::from_str::<RpcRequest>(line).map_err(|e| format!("Parse error: {}", e))
}

/// Serialize a JSON-RPC response to a JSON string (without newline).
pub fn serialize_response(response: &RpcResponse) -> Result<String, String> {
    serde_json::to_string(response).map_err(|e| format!("Serialize error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHandler;

    #[async_trait::async_trait]
    impl AcpHandler for TestHandler {
        async fn agent_state(&self) -> AgentState {
            AgentState::Idle
        }

        async fn execute_command(&self, command: &str) -> Result<String, String> {
            Ok(format!("executed: {}", command))
        }
    }

    fn make_request(method: &str, params: Value) -> RpcRequest {
        RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: method.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn test_ping() {
        let req = make_request("ping", Value::Null);
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome
            .response
            .expect("a request, not a notification, must produce a response");
        let shutdown = outcome.should_shutdown;
        assert!(!shutdown);
        assert!(resp.error.is_none());
        assert_eq!(resp.result, Some(serde_json::json!("pong")));
    }

    // ---- Agent Client Protocol ----

    #[tokio::test]
    async fn initialize_advertises_our_version_and_capabilities() {
        let req = make_request("initialize", serde_json::json!({ "protocolVersion": 1 }));
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome.response.expect("initialize must respond");
        assert!(resp.error.is_none());
        let result = resp.result.expect("initialize must return a result");
        assert_eq!(result["protocolVersion"], ACP_PROTOCOL_VERSION);
        // `loadSession` must be false because `session/load` is not
        // implemented. Advertising a capability that is not there is the failure
        // mode this protocol makes expensive: a client calls it and hangs.
        assert_eq!(result["agentCapabilities"]["loadSession"], false);
        assert_eq!(
            result["agentCapabilities"]["promptCapabilities"]["image"],
            false
        );
    }

    /// A client proposing a version we do not speak must be able to tell that
    /// apart from us having ignored it, so the proposal is echoed back.
    #[tokio::test]
    async fn initialize_echoes_the_version_the_client_proposed() {
        let req = make_request("initialize", serde_json::json!({ "protocolVersion": 99 }));
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome.response.expect("initialize must respond");
        let result = resp.result.expect("initialize must return a result");
        assert_eq!(result["requestedProtocolVersion"], 99);
        assert_eq!(result["protocolVersion"], ACP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn session_new_mints_a_live_session() {
        let req = make_request("session/new", serde_json::json!({ "cwd": "/tmp" }));
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome.response.expect("session/new must respond");
        assert!(resp.error.is_none());
        let session_id = resp.result.expect("result")["sessionId"]
            .as_str()
            .expect("sessionId must be a string")
            .to_string();
        assert!(
            sessions.is_live(&session_id),
            "a freshly minted session must be live, or every later prompt fails"
        );
    }

    /// The core streaming behaviour: a prompt on a live session emits exactly
    /// one `session/update` carrying the turn, and returns `stopReason`.
    #[tokio::test]
    async fn session_prompt_streams_an_update_and_returns_a_stop_reason() {
        let new_req = make_request("session/new", serde_json::json!({ "cwd": "/tmp" }));
        let sessions = SessionRegistry::new();
        let session_id = dispatch(&new_req, &TestHandler, &sessions)
            .await
            .response
            .expect("session/new must respond")
            .result
            .expect("result")["sessionId"]
            .as_str()
            .expect("string")
            .to_string();

        let req = make_request(
            "session/prompt",
            serde_json::json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "hello" }],
            }),
        );
        let outcome = dispatch(&req, &TestHandler, &sessions).await;

        let resp = outcome.response.expect("session/prompt must respond");
        assert!(
            resp.error.is_none(),
            "prompt must not error: {:?}",
            resp.error
        );
        assert_eq!(
            resp.result.expect("result")["stopReason"],
            "end_turn",
            "a completed turn must report stopReason"
        );

        assert_eq!(
            outcome.notifications.len(),
            1,
            "a turn must emit exactly one agent_message_chunk"
        );
        let note = &outcome.notifications[0];
        assert_eq!(note.method, "session/update");
        let params = note.params.as_ref().expect("update must carry params");
        assert_eq!(params["sessionId"], session_id.as_str());
        assert_eq!(params["update"]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(params["update"]["content"]["text"], "executed: hello");
    }

    /// A notification must never be answered. Getting this wrong makes a real
    /// client wait forever for a response that the spec says must not exist.
    #[tokio::test]
    async fn session_update_serializes_without_an_id_member() {
        let note = RpcNotification::agent_message_chunk("s1", "hi");
        let json = serde_json::to_value(&note).expect("notification must serialize");
        assert!(
            json.get("id").is_none(),
            "a notification carrying `id` is an unanswered request: {json}"
        );
        assert_eq!(json["method"], "session/update");
    }

    /// `session/cancel` is itself a notification: it must produce no response
    /// at all, only ending the session.
    #[tokio::test]
    async fn session_cancel_produces_no_response_and_ends_the_session() {
        let sessions = SessionRegistry::new();
        let id = sessions.create();

        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "session/cancel".to_string(),
            params: serde_json::json!({ "sessionId": id }),
        };
        let outcome = dispatch(&req, &TestHandler, &sessions).await;

        assert!(
            outcome.response.is_none(),
            "session/cancel is a notification and must not be answered"
        );
        assert!(!sessions.is_live(&id), "cancel must end the session");
    }

    /// A prompt against a session that was never created, or has been ended,
    /// must be rejected rather than silently opening a turn.
    #[tokio::test]
    async fn session_prompt_rejects_an_unknown_session() {
        let req = make_request(
            "session/prompt",
            serde_json::json!({
                "sessionId": "never-existed",
                "prompt": [{ "type": "text", "text": "hi" }],
            }),
        );
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome.response.expect("must respond with an error");
        assert_eq!(resp.error.expect("error").code, -32002);
        assert!(
            outcome.notifications.is_empty(),
            "a rejected prompt must not stream anything"
        );
    }

    #[tokio::test]
    async fn session_prompt_rejects_a_prompt_with_no_text() {
        let sessions = SessionRegistry::new();
        let id = sessions.create();
        let req = make_request(
            "session/prompt",
            serde_json::json!({ "sessionId": id, "prompt": [] }),
        );
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome.response.expect("must respond with an error");
        assert_eq!(resp.error.expect("error").code, -32602);
    }

    /// A non-text block contributes no text but must not fail the request: a
    /// client sending an image alongside text should still get its text
    /// answered.
    #[tokio::test]
    async fn a_non_text_block_is_skipped_rather_than_failing_the_prompt() {
        let sessions = SessionRegistry::new();
        let id = sessions.create();
        let req = make_request(
            "session/prompt",
            serde_json::json!({
                "sessionId": id,
                "prompt": [
                    { "type": "image", "data": "...", "mimeType": "image/png" },
                    { "type": "text", "text": "describe" },
                ],
            }),
        );
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome.response.expect("must respond");
        assert!(
            resp.error.is_none(),
            "an image block must not fail the turn"
        );
        let text =
            outcome.notifications[0].params.as_ref().expect("params")["update"]["content"]["text"]
                .as_str()
                .expect("string")
                .to_string();
        assert_eq!(text, "executed: describe");
    }

    /// The full sequence a real ACP client performs, in order, through the same
    /// registry a live server uses. This is the regression that matters: each
    /// method passing in isolation while the sequence breaks.
    #[tokio::test]
    async fn a_full_client_sequence_succeeds_end_to_end() {
        let sessions = SessionRegistry::new();

        let init = dispatch(
            &make_request("initialize", serde_json::json!({ "protocolVersion": 1 })),
            &TestHandler,
            &sessions,
        )
        .await;
        assert!(init.response.expect("initialize").error.is_none());

        let new = dispatch(
            &make_request("session/new", serde_json::json!({ "cwd": "/tmp" })),
            &TestHandler,
            &sessions,
        )
        .await;
        let session_id = new.response.expect("session/new").result.expect("result")["sessionId"]
            .as_str()
            .expect("string")
            .to_string();

        let prompt = dispatch(
            &make_request(
                "session/prompt",
                serde_json::json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": "ping" }],
                }),
            ),
            &TestHandler,
            &sessions,
        )
        .await;
        assert!(prompt.response.expect("prompt").error.is_none());
        assert_eq!(prompt.notifications.len(), 1);
        assert!(
            sessions.is_live(&session_id),
            "a turn must not end its session"
        );
    }

    /// The stdio loop must actually WRITE the notifications `dispatch` produces.
    ///
    /// Every ACP unit test above exercises `dispatch` directly, so all of them
    /// pass whether or not `run_stdio_server` ever emits a notification. If the
    /// loop dropped them, `session/prompt` would return a correct `stopReason`,
    /// every test in this file would stay green, and a real client would receive
    /// no agent output at all — the protocol working perfectly on paper and
    /// silently doing nothing on the wire.
    ///
    /// The loop cannot be driven without mocking stdin, so this pins the call
    /// site instead. It matches a COMMENT-STRIPPED scan: a plain `contains` gate
    /// is satisfied by its own commented-out call, which is how the equivalent
    /// gate in `tui/pinned_images` was found to be satisfied by its own
    /// commented-out line.
    #[test]
    fn the_stdio_loop_writes_the_notifications_dispatch_produces() {
        let source = strip_rust_comments(include_str!("server.rs"));

        assert!(
            source.contains("for notification in &outcome.notifications"),
            "server.rs no longer iterates `outcome.notifications` — `session/prompt` would \
             return a stopReason while the agent's text never reaches the client"
        );
        assert!(
            source.contains("write_notification(notification)"),
            "server.rs no longer calls `write_notification` — notifications are produced \
             and then dropped"
        );

        // And the ordering: a streaming client expects the text before the turn
        // is reported finished, not after.
        let notify_at = source
            .find("write_notification(notification)")
            .expect("call site pinned above");
        let respond_at = source
            .find("write_response(response)")
            .expect("the response call site is what this ordering is relative to");
        assert!(
            notify_at < respond_at,
            "notifications are written AFTER the response, so a client sees the turn \
             reported finished before the text that finished it arrives"
        );
    }

    /// Remove `//` and `/* */` comments so a gate cannot be satisfied by prose.
    fn strip_rust_comments(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
                out.push(' ');
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    #[tokio::test]
    async fn test_status() {
        let req = make_request("status", Value::Null);
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome
            .response
            .expect("a request, not a notification, must produce a response");
        let shutdown = outcome.should_shutdown;
        assert!(!shutdown);
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["agent"]["state"], "idle");
        assert_eq!(result["server"]["name"], "operant-acp");
    }

    #[tokio::test]
    async fn test_command() {
        let req = make_request("command", serde_json::json!({ "command": "hello world" }));
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome
            .response
            .expect("a request, not a notification, must produce a response");
        let shutdown = outcome.should_shutdown;
        assert!(!shutdown);
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["output"], "executed: hello world");
    }

    #[tokio::test]
    async fn test_command_missing_param() {
        let req = make_request("command", Value::Null);
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome
            .response
            .expect("a request, not a notification, must produce a response");
        let shutdown = outcome.should_shutdown;
        assert!(!shutdown);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn test_stop() {
        let req = make_request("stop", Value::Null);
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome
            .response
            .expect("a request, not a notification, must produce a response");
        let shutdown = outcome.should_shutdown;
        assert!(shutdown);
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let req = make_request("unknown", Value::Null);
        let sessions = SessionRegistry::new();
        let outcome = dispatch(&req, &TestHandler, &sessions).await;
        let resp = outcome
            .response
            .expect("a request, not a notification, must produce a response");
        let shutdown = outcome.should_shutdown;
        assert!(!shutdown);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[test]
    fn request_without_id_is_a_notification() {
        let req = parse_request(r#"{"jsonrpc":"2.0","method":"ping"}"#).unwrap();
        assert!(req.is_notification());
        let req = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert!(!req.is_notification());
        // Explicit null id is NOT a notification — it must get a response.
        let explicit_null =
            parse_request(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).unwrap();
        assert_eq!(explicit_null.id, Some(Value::Null));
        assert!(!explicit_null.is_notification());
        assert_eq!(explicit_null.response_id(), Value::Null);
    }

    #[test]
    fn validate_request_accepts_wellformed_and_rejects_wrong_version_or_id_type() {
        let ok = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert_eq!(validate_request(&ok), Ok(()));
        // Missing jsonrpc member deserializes to "" → Invalid Request.
        let no_version = parse_request(r#"{"id":1,"method":"ping"}"#).unwrap();
        assert_eq!(validate_request(&no_version), Err(-32600));
        let bad_version = parse_request(r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#).unwrap();
        assert_eq!(validate_request(&bad_version), Err(-32600));
        // A notification (omitted id) is still a valid request.
        let notify = parse_request(r#"{"jsonrpc":"2.0","method":"ping"}"#).unwrap();
        assert_eq!(validate_request(&notify), Ok(()));
        // An explicit null id is a valid request per spec.
        let null_id = parse_request(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).unwrap();
        assert_eq!(validate_request(&null_id), Ok(()));
        // An object id is invalid per spec.
        let obj_id = parse_request(r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#).unwrap();
        assert_eq!(validate_request(&obj_id), Err(-32600));
    }

    #[test]
    fn agent_state_tracker_reports_transitions_and_shares_across_clones() {
        let tracker = AgentStateTracker::new();
        assert_eq!(tracker.get(), AgentState::Idle);
        tracker.set(AgentState::Running);
        assert_eq!(tracker.get(), AgentState::Running);
        let clone = tracker.clone();
        clone.set(AgentState::Error("boom".to_string()));
        assert_eq!(tracker.get(), AgentState::Error("boom".to_string()));
    }

    #[test]
    fn test_parse_request_valid() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
        let req = parse_request(line).unwrap();
        assert_eq!(req.method, "ping");
        assert_eq!(req.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_parse_request_invalid() {
        let line = "not-json";
        assert!(parse_request(line).is_err());
    }

    #[test]
    fn test_serialize_response() {
        let resp = RpcResponse::success(serde_json::json!(1), serde_json::json!("pong"));
        let json = serialize_response(&resp).unwrap();
        assert!(json.contains("\"result\":\"pong\""));
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
    }

    #[test]
    fn test_agent_state_display() {
        assert_eq!(AgentState::Running.to_string(), "running");
        assert_eq!(AgentState::Paused.to_string(), "paused");
        assert_eq!(AgentState::Idle.to_string(), "idle");
        assert_eq!(
            AgentState::Error("oops".to_string()).to_string(),
            "error: oops"
        );
    }
}
