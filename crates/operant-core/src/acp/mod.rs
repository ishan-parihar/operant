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
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent is running and processing
    Running,
    /// Agent is paused (user-requested halt)
    Paused,
    /// Agent is idle (running but not actively processing)
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
                request.id.clone(),
                -32601,
                format!("Method not found: {}", request.method),
            ),
            false,
        ),
    }
}

fn handle_ping(request: &RpcRequest) -> RpcResponse {
    RpcResponse::success(request.id.clone(), serde_json::json!("pong"))
}

async fn handle_status(request: &RpcRequest, handler: &dyn AcpHandler) -> RpcResponse {
    let state = handler.agent_state().await;
    let info = handler.server_info();

    RpcResponse::success(
        request.id.clone(),
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
            request.id.clone(),
            -32602,
            "Missing required parameter: 'command'",
        );
    }

    match handler.execute_command(command).await {
        Ok(output) => RpcResponse::success(
            request.id.clone(),
            serde_json::json!({
                "success": true,
                "output": output,
            }),
        ),
        Err(err) => RpcResponse::success(
            request.id.clone(),
            serde_json::json!({
                "success": false,
                "output": err,
            }),
        ),
    }
}

fn handle_stop(request: &RpcRequest) -> RpcResponse {
    RpcResponse::success(
        request.id.clone(),
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
            id: serde_json::json!(1),
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
    fn test_parse_request_valid() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
        let req = parse_request(line).unwrap();
        assert_eq!(req.method, "ping");
        assert_eq!(req.id, serde_json::json!(1));
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
