//! Integration test for the ACP JSON-RPC request-response cycle.
//!
//! Tests the full parse → dispatch → serialize cycle through the public API,
//! simulating what happens over stdio without needing actual pipe I/O.

use operant_core::acp::{AcpHandler, AgentState};
use std::sync::Arc;

/// A test handler that always returns a fixed state and echoes commands.
struct TestAcpHandler;

#[async_trait::async_trait]
impl AcpHandler for TestAcpHandler {
    async fn agent_state(&self) -> AgentState {
        AgentState::Running
    }

    async fn execute_command(&self, command: &str) -> Result<String, String> {
        Ok(format!("handled: {}", command))
    }
}

#[tokio::test]
async fn test_ping_pong() {
    let handler = Arc::new(TestAcpHandler);
    let raw = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, should_shutdown) = operant_core::acp::dispatch(&request, &*handler).await;

    assert!(!should_shutdown, "ping should not trigger shutdown");
    assert!(response.error.is_none(), "ping should not error");
    assert_eq!(
        response.result,
        Some(serde_json::json!("pong")),
        "ping should return pong"
    );
}

#[tokio::test]
async fn test_status() {
    let handler = Arc::new(TestAcpHandler);
    let raw = r#"{"jsonrpc":"2.0","id":2,"method":"status","params":{}}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, should_shutdown) = operant_core::acp::dispatch(&request, &*handler).await;

    assert!(!should_shutdown);
    assert!(response.error.is_none());

    let result = response.result.unwrap();
    assert_eq!(result["agent"]["state"], "running");
    assert_eq!(result["server"]["name"], "operant-acp");
}

#[tokio::test]
async fn test_command_valid() {
    let handler = Arc::new(TestAcpHandler);
    let raw = r#"{"jsonrpc":"2.0","id":3,"method":"command","params":{"command":"hello world"}}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, should_shutdown) = operant_core::acp::dispatch(&request, &*handler).await;

    assert!(!should_shutdown);
    assert!(response.error.is_none());

    let result = response.result.unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["output"], "handled: hello world");
}

#[tokio::test]
async fn test_command_missing_param() {
    let handler = Arc::new(TestAcpHandler);
    let raw = r#"{"jsonrpc":"2.0","id":4,"method":"command","params":{}}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, should_shutdown) = operant_core::acp::dispatch(&request, &*handler).await;

    assert!(!should_shutdown);
    assert!(response.error.is_some());
    assert_eq!(response.error.unwrap().code, -32602);
}

#[tokio::test]
async fn test_stop_triggers_shutdown() {
    let handler = Arc::new(TestAcpHandler);
    let raw = r#"{"jsonrpc":"2.0","id":5,"method":"stop","params":{}}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, should_shutdown) = operant_core::acp::dispatch(&request, &*handler).await;

    assert!(should_shutdown, "stop should trigger shutdown");
    assert!(response.error.is_none());
}

#[tokio::test]
async fn test_unknown_method() {
    let handler = Arc::new(TestAcpHandler);
    let raw = r#"{"jsonrpc":"2.0","id":6,"method":"unknown","params":{}}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, should_shutdown) = operant_core::acp::dispatch(&request, &*handler).await;

    assert!(!should_shutdown);
    assert!(response.error.is_some());
    assert_eq!(response.error.unwrap().code, -32601);
}

#[tokio::test]
async fn test_invalid_json_parse_error() {
    let result = operant_core::acp::parse_request("not valid json");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_round_trip_serialization() {
    let handler = Arc::new(TestAcpHandler);

    let raw = r#"{"jsonrpc":"2.0","id":42,"method":"ping","params":null}"#;
    let request = operant_core::acp::parse_request(raw).unwrap();
    let (response, _) = operant_core::acp::dispatch(&request, &*handler).await;
    let serialized = operant_core::acp::serialize_response(&response).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(parsed["jsonrpc"], "2.0");
    assert_eq!(parsed["id"], 42);
    assert_eq!(parsed["result"], "pong");
    assert!(parsed.get("error").is_none());
}
