//! ACP (Agent Control Protocol) — JSON-RPC based protocol over stdio.
//!
//! Allows external tools (IDEs, scripts, CI systems) to control the Operant agent
//! via line-delimited JSON-RPC messages on stdin/stdout.
//!
//! # Supported Methods
//!
//! | Method    | Description                              |
//! |-----------|------------------------------------------|
//! | `ping`    | Health check — returns `"pong"`          |
//! | `status`  | Returns current agent state              |
//! | `command` | Executes a command in the agent's context|
//! | `stop`    | Gracefully shuts down the ACP server     |

pub mod server;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    if let Some(id) = &request.id {
        if !(id.is_null() || id.is_string() || id.is_number()) {
            return Err(-32600);
        }
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

/// Dispatch a JSON-RPC request and produce a response.
///
/// Returns `(RpcResponse, bool)` where the bool indicates whether the
/// server should shut down (true for `stop` method).
pub async fn dispatch(request: &RpcRequest, handler: &dyn AcpHandler) -> (RpcResponse, bool) {
    match request.method.as_str() {
        "ping" => (handle_ping(request), false),
        "status" => (handle_status(request, handler).await, false),
        "command" => (handle_command(request, handler).await, false),
        "stop" => (handle_stop(request), true),
        _ => (
            RpcResponse::error(
                request.response_id(),
                -32601,
                format!("Method not found: {}", request.method),
            ),
            false,
        ),
    }
}

fn handle_ping(request: &RpcRequest) -> RpcResponse {
    RpcResponse::success(request.response_id(), serde_json::json!("pong"))
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
        let (resp, shutdown) = dispatch(&req, &TestHandler).await;
        assert!(!shutdown);
        assert!(resp.error.is_none());
        assert_eq!(resp.result, Some(serde_json::json!("pong")));
    }

    #[tokio::test]
    async fn test_status() {
        let req = make_request("status", Value::Null);
        let (resp, shutdown) = dispatch(&req, &TestHandler).await;
        assert!(!shutdown);
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["agent"]["state"], "idle");
        assert_eq!(result["server"]["name"], "operant-acp");
    }

    #[tokio::test]
    async fn test_command() {
        let req = make_request("command", serde_json::json!({ "command": "hello world" }));
        let (resp, shutdown) = dispatch(&req, &TestHandler).await;
        assert!(!shutdown);
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["output"], "executed: hello world");
    }

    #[tokio::test]
    async fn test_command_missing_param() {
        let req = make_request("command", Value::Null);
        let (resp, shutdown) = dispatch(&req, &TestHandler).await;
        assert!(!shutdown);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn test_stop() {
        let req = make_request("stop", Value::Null);
        let (resp, shutdown) = dispatch(&req, &TestHandler).await;
        assert!(shutdown);
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let req = make_request("unknown", Value::Null);
        let (resp, shutdown) = dispatch(&req, &TestHandler).await;
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
