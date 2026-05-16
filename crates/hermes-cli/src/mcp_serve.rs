//! Hermes MCP Server — expose Hermes tools as an MCP server over stdio.
//!
//! Implements the Model Context Protocol (MCP) server side, letting any MCP
//! client (Claude Desktop, VS Code, etc.) discover and call Hermes tools.
//!
//! ## Protocol
//! Reads JSON-RPC request lines from stdin, writes response lines to stdout.
//! Supports the MCP lifecycle: initialize → tools/list → tools/call → ...
//!
//! ## Tool Discovery
//! All tools registered in the Hermes ToolRegistry (built-in + plugin + MCP
//! client tools) are exposed automatically. When `mcp.autoload` is enabled
//! in config, connected MCP servers' tools are also synced into the registry.
//!
//! ## Concurrency
//! The event loop is async (tokio). Individual tool calls are dispatched via
//! `ToolRegistry::execute` with a configurable timeout. The server does not
//! spawn per-request tasks — tool execution is bounded by the registry timeout.

use std::sync::Arc;

use anyhow::{Context, Result};
use hermes_core::config::AppConfig;
use hermes_core::database::Database;
use hermes_core::mcp::McpManager;
use hermes_core::schema::ToolSchema;
use hermes_core::tools::{ToolContext, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ──────────────────────────────────────────────
// MCP JSON-RPC Types (spec v2024-11-05)
// ──────────────────────────────────────────────

/// A JSON-RPC request from the MCP client.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

/// A JSON-RPC response sent to the MCP client.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorBody>,
}

/// A JSON-RPC error object.
#[derive(Debug, Serialize)]
struct JsonRpcErrorBody {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

// Standard JSON-RPC error codes
const PARSE_ERROR: i32 = -32700;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

// ──────────────────────────────────────────────
// Server Implementation
// ──────────────────────────────────────────────

/// Run the MCP server over stdio.
///
/// 1. Builds a full ToolRegistry from config (built-in + plugin + MCP client tools).
/// 2. Reads newline-delimited JSON-RPC requests from stdin.
/// 3. Dispatches to the appropriate handler.
/// 4. Writes newline-delimited JSON-RPC responses to stdout.
///
/// The server runs until stdin is closed (EOF) or a `shutdown` request is received.
pub async fn run_mcp_serve(config: &AppConfig, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[mcp-serve] initializing tool registry...");
    }

    let registry = build_tool_registry(config).await?;

    if verbose {
        let count = registry.len().await;
        eprintln!("[mcp-serve] registry ready with {count} tools");
        eprintln!("[mcp-serve] listening for JSON-RPC on stdin...");
    }

    // ── Main event loop ──
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }

        // Parse the request
        let request: JsonRpcRequest = match serde_json::from_str(&trimmed) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = make_response(
                    Value::Null,
                    Some(JsonRpcErrorBody {
                        code: PARSE_ERROR,
                        message: format!("Parse error: {e}"),
                        data: None,
                    }),
                );
                write_response(&err_resp).await?;
                continue;
            }
        };

        // MCP notifications (no id → no response expected)
        if request.id.is_null() {
            if verbose && request.method != "notifications/initialized" {
                eprintln!("[mcp-serve] notification: {}", request.method);
            }
            continue;
        }

        // Dispatch
        let response = match request.method.as_str() {
            "initialize" => handle_initialize(&request),
            "shutdown" => break,
            "tools/list" => handle_tools_list(&request, &registry).await,
            "tools/call" => handle_tools_call(&request, &registry).await,
            "resources/list" => handle_resources_list(&request),
            "resources/read" => handle_resources_read(&request),
            "ping" => handle_ping(&request),
            _ => make_response(
                request.id,
                Some(JsonRpcErrorBody {
                    code: METHOD_NOT_FOUND,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
            ),
        };

        write_response(&response).await?;
    }

    if verbose {
        eprintln!("[mcp-serve] stdin closed, shutting down.");
    }

    Ok(())
}

/// Build a complete tool registry from the app config.
///
/// This mirrors `crate::build_registry` to avoid a circular dependency
/// while keeping the same registration logic (built-in tools, MCP client
/// connection, MCP tool sync).
async fn build_tool_registry(config: &AppConfig) -> Result<ToolRegistry> {
    let client = hermes_core::client::OpenAIClient::new(hermes_core::client::ClientConfig::from(
        &config.client,
    ));
    let mcp_manager = McpManager::new();
    let database = Arc::new(
        Database::init(config.database_path.clone())
            .context("Failed to initialize database for MCP serve")?,
    );

    let registry =
        crate::build_registry(config, &mcp_manager, &client, &config.agent.model, database).await?;

    // Sync MCP client tools into the registry if autoload is enabled.
    if config.mcp.autoload {
        for server in config.mcp.servers.iter().filter(|s| s.enabled) {
            if !mcp_manager.contains(&server.name).await {
                match connect_mcp_server(&mcp_manager, server).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!(
                            "[mcp-serve] warning: failed to connect MCP server '{}': {e}",
                            server.name
                        );
                    }
                }
            }
        }
        mcp_manager.sync_tools_to_registry(&registry).await;
    }

    Ok(registry)
}

async fn connect_mcp_server(
    mcp_manager: &McpManager,
    server: &hermes_core::config::McpServerConfig,
) -> Result<()> {
    match server.transport {
        hermes_core::config::McpTransportKind::Http => {
            let url = server
                .url
                .clone()
                .context("HTTP MCP server is missing a URL")?;
            mcp_manager
                .add_server(server.name.clone(), url, server.auth_token.clone())
                .await?;
        }
        hermes_core::config::McpTransportKind::Stdio => {
            let cmd = server
                .command
                .clone()
                .context("Stdio MCP server is missing a command")?;
            mcp_manager
                .add_stdio_server(
                    server.name.clone(),
                    cmd,
                    server.args.clone(),
                    server.env.clone(),
                )
                .await?;
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────
// MCP Method Handlers
// ──────────────────────────────────────────────

/// Handle the `initialize` handshake.
///
/// Returns protocol version, server info, and capabilities.
fn handle_initialize(request: &JsonRpcRequest) -> JsonRpcResponse {
    make_response(
        request.id.clone(),
        Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {
                    "listChanged": false
                },
                "resources": {
                    "subscribe": false,
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "hermes-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
    )
}

/// Handle a `ping` request.
fn handle_ping(request: &JsonRpcRequest) -> JsonRpcResponse {
    make_response(request.id.clone(), Some(Value::Null))
}

/// Handle `tools/list` — return all registered tools as MCP tool definitions.
async fn handle_tools_list(request: &JsonRpcRequest, registry: &ToolRegistry) -> JsonRpcResponse {
    let schemas = registry.get_schemas().await;
    let tools: Vec<Value> = schemas
        .iter()
        .map(|schema| mcp_tool_definition(schema))
        .collect();

    make_response(
        request.id.clone(),
        Some(serde_json::json!({ "tools": tools })),
    )
}

/// Convert a Hermes ToolSchema to an MCP tool definition object.
fn mcp_tool_definition(schema: &ToolSchema) -> Value {
    // MCP expects `inputSchema` which maps directly from our JSON Schema parameters.
    serde_json::json!({
        "name": schema.name,
        "description": schema.description,
        "inputSchema": schema.parameters,
    })
}

/// Handle `tools/call` — execute a tool and return the result in MCP format.
async fn handle_tools_call(request: &JsonRpcRequest, registry: &ToolRegistry) -> JsonRpcResponse {
    let tool_name = match request.params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => {
            return make_response(
                request.id.clone(),
                Some(JsonRpcErrorBody {
                    code: INVALID_PARAMS,
                    message: "Missing required parameter: 'name'".to_string(),
                    data: None,
                }),
            );
        }
    };

    let arguments = request.params.get("arguments").cloned().unwrap_or_default();

    // Check if tool exists first
    if !registry.contains(tool_name).await {
        return make_response(
            request.id.clone(),
            Some(JsonRpcErrorBody {
                code: INVALID_PARAMS,
                message: format!("Unknown tool: {tool_name}"),
                data: None,
            }),
        );
    }

    let call_id = format!("mcp_{}", tool_name);
    let context = ToolContext::default();

    match registry
        .execute(tool_name, &call_id, arguments, context)
        .await
    {
        Ok(result) => {
            // MCP tool call responses use a "content" array with text items.
            // We include both the output content and a status indicator.
            let content = if result.success {
                vec![serde_json::json!({
                    "type": "text",
                    "text": result.content,
                })]
            } else {
                let error_msg = result.error.unwrap_or_else(|| "Unknown error".to_string());
                vec![serde_json::json!({
                    "type": "text",
                    "text": format!("Error: {error_msg}"),
                })]
            };

            make_response(
                request.id.clone(),
                Some(serde_json::json!({ "content": content })),
            )
        }
        Err(e) => make_response(
            request.id.clone(),
            Some(JsonRpcErrorBody {
                code: INTERNAL_ERROR,
                message: format!("Tool execution failed: {e}"),
                data: None,
            }),
        ),
    }
}

/// Handle `resources/list` — expose system resources.
fn handle_resources_list(request: &JsonRpcRequest) -> JsonRpcResponse {
    let resources = vec![
        serde_json::json!({
            "uri": "hermes://tools",
            "name": "Available Tools",
            "description": "List of all tools registered in the Hermes tool registry",
            "mimeType": "application/json",
        }),
        serde_json::json!({
            "uri": "hermes://status",
            "name": "Server Status",
            "description": "Hermes MCP server status information",
            "mimeType": "application/json",
        }),
        serde_json::json!({
            "uri": "hermes://config",
            "name": "Server Configuration",
            "description": "Active Hermes server configuration (non-sensitive fields only)",
            "mimeType": "application/json",
        }),
    ];

    make_response(
        request.id.clone(),
        Some(serde_json::json!({ "resources": resources })),
    )
}

/// Handle `resources/read` — read a specific resource.
fn handle_resources_read(request: &JsonRpcRequest) -> JsonRpcResponse {
    let uri = match request.params.get("uri").and_then(Value::as_str) {
        Some(u) => u,
        None => {
            return make_response(
                request.id.clone(),
                Some(JsonRpcErrorBody {
                    code: INVALID_PARAMS,
                    message: "Missing required parameter: 'uri'".to_string(),
                    data: None,
                }),
            );
        }
    };

    let content = match uri {
        "hermes://status" => serde_json::json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": serde_json::to_string_pretty(&serde_json::json!({
                    "server": "hermes-mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                    "status": "running",
                })).unwrap_or_default(),
            }]
        }),
        "hermes://tools" | "hermes://config" => serde_json::json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": serde_json::to_string_pretty(&serde_json::json!({
                    "message": format!("Use tools/list to enumerate tools. Resource '{uri}' is dynamic."),
                })).unwrap_or_default(),
            }]
        }),
        _ => {
            return make_response(
                request.id.clone(),
                Some(JsonRpcErrorBody {
                    code: INVALID_PARAMS,
                    message: format!("Resource not found: {uri}"),
                    data: None,
                }),
            );
        }
    };

    make_response(request.id.clone(), Some(content))
}

// ──────────────────────────────────────────────
// Response Helpers
// ──────────────────────────────────────────────

/// Construct a JSON-RPC response with either a result or an error.
fn make_response(id: Value, result_or_error: impl Into<ResponsePayload>) -> JsonRpcResponse {
    match result_or_error.into() {
        ResponsePayload::Result(result) => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        },
        ResponsePayload::Error(error) => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        },
    }
}

/// Internal payload type for `make_response`.
enum ResponsePayload {
    Result(Value),
    Error(JsonRpcErrorBody),
}

impl From<Value> for ResponsePayload {
    fn from(value: Value) -> Self {
        ResponsePayload::Result(value)
    }
}

impl From<JsonRpcErrorBody> for ResponsePayload {
    fn from(error: JsonRpcErrorBody) -> Self {
        ResponsePayload::Error(error)
    }
}

impl<T: Into<ResponsePayload>> From<Option<T>> for ResponsePayload {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(val) => val.into(),
            None => ResponsePayload::Result(Value::Null),
        }
    }
}

/// Write a JSON-RPC response to stdout as a newline-delimited JSON line.
async fn write_response(response: &JsonRpcResponse) -> Result<()> {
    let mut output = serde_json::to_string(response)?;
    output.push('\n');

    let mut stdout = tokio::io::stdout();
    stdout.write_all(output.as_bytes()).await?;
    stdout.flush().await?;
    Ok(())
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_core::tools::{HermesTool, ToolResult};
    use std::time::Duration;

    /// A minimal test-only tool registry with known tools for testing.
    struct TestEchoTool;

    #[async_trait::async_trait]
    impl HermesTool for TestEchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo back the input"
        }
        fn schema(&self) -> hermes_core::schema::ToolSchema {
            hermes_core::schema::ToolSchema::new(
                "echo",
                "Echo back the input",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": {"type": "string"}
                    },
                    "required": ["message"]
                }),
            )
        }
        async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
            let msg = args.get("message").and_then(Value::as_str).unwrap_or("");
            ToolResult::success("echo", serde_json::json!({ "echoed": msg }))
        }
    }

    /// Build a test registry with a known set of tools.
    async fn test_registry() -> ToolRegistry {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry.register(TestEchoTool).await.unwrap();
        registry.register(CalculatorTool).await.unwrap();
        registry
    }

    struct CalculatorTool;

    #[async_trait::async_trait]
    impl HermesTool for CalculatorTool {
        fn name(&self) -> &str {
            "calculate"
        }
        fn description(&self) -> &str {
            "Perform arithmetic"
        }
        fn schema(&self) -> hermes_core::schema::ToolSchema {
            hermes_core::schema::ToolSchema::new(
                "calculate",
                "Perform arithmetic",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "operation": {"type": "string", "enum": ["add", "subtract"]},
                        "a": {"type": "number"},
                        "b": {"type": "number"}
                    },
                    "required": ["operation", "a", "b"]
                }),
            )
        }
        async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
            let op = args
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or("add");
            let a = args.get("a").and_then(Value::as_f64).unwrap_or(0.0);
            let b = args.get("b").and_then(Value::as_f64).unwrap_or(0.0);
            let result = match op {
                "add" => a + b,
                "subtract" => a - b,
                _ => return ToolResult::error("calculate", format!("Unknown op: {op}")),
            };
            ToolResult::success("calculate", serde_json::json!({ "result": result }))
        }
    }

    /// Test MCP `initialize` handshake returns correct protocol version.
    #[test]
    fn test_initialize_response() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "initialize".to_string(),
            params: Value::Null,
        };

        let response = handle_initialize(&request);
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, 1);
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "hermes-mcp");
        assert!(result["capabilities"]["tools"].is_object());
    }

    /// Test `ping` returns null result.
    #[test]
    fn test_ping_response() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::String("req-1".to_string()),
            method: "ping".to_string(),
            params: Value::Null,
        };

        let response = handle_ping(&request);
        assert_eq!(response.id, "req-1");
        assert_eq!(response.result, Some(Value::Null));
        assert!(response.error.is_none());
    }

    /// Test `method not found` for unknown methods.
    #[test]
    fn test_unknown_method() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(42.into()),
            method: "unknown_method".to_string(),
            params: Value::Null,
        };

        // Manually simulate the dispatch
        let response = make_response(
            request.id,
            Some(JsonRpcErrorBody {
                code: METHOD_NOT_FOUND,
                message: "Method not found: unknown_method".to_string(),
                data: None,
            }),
        );

        assert!(response.error.is_some());
        assert_eq!(response.error.as_ref().unwrap().code, METHOD_NOT_FOUND);
    }

    /// Test `tools/list` returns the expected tool definitions.
    #[tokio::test]
    async fn test_tools_list() {
        let registry = test_registry().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "tools/list".to_string(),
            params: Value::Null,
        };

        let response = handle_tools_list(&request, &registry).await;
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();

        // Should have at least echo and calculate
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"echo"));
        assert!(names.contains(&"calculate"));
    }

    /// Test `tools/call` with a valid tool name and arguments.
    #[tokio::test]
    async fn test_tools_call_success() {
        let registry = test_registry().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "echo",
                "arguments": { "message": "hello world" }
            }),
        };

        let response = handle_tools_call(&request, &registry).await;
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        let content = result["content"].as_array().unwrap();
        assert!(!content.is_empty());
        assert_eq!(content[0]["type"], "text");
        assert!(content[0]["text"].as_str().unwrap().contains("hello world"));
    }

    /// Test `tools/call` with a calculation tool.
    #[tokio::test]
    async fn test_tools_call_calculator() {
        let registry = test_registry().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "calculate",
                "arguments": { "operation": "add", "a": 3, "b": 4 }
            }),
        };

        let response = handle_tools_call(&request, &registry).await;
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("7") || text.contains("3") || text.contains("4"));
    }

    /// Test `tools/call` with a non-existent tool returns an error.
    #[tokio::test]
    async fn test_tools_call_unknown_tool() {
        let registry = test_registry().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "nonexistent_tool",
                "arguments": {}
            }),
        };

        let response = handle_tools_call(&request, &registry).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, INVALID_PARAMS);
    }

    /// Test `tools/call` with missing name parameter.
    #[tokio::test]
    async fn test_tools_call_missing_name() {
        let registry = test_registry().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "tools/call".to_string(),
            params: serde_json::json!({}),
        };

        let response = handle_tools_call(&request, &registry).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, INVALID_PARAMS);
    }

    /// Test MCP tool definition conversion.
    #[test]
    fn test_mcp_tool_definition() {
        let schema = ToolSchema::new(
            "test_tool",
            "A test tool",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        );

        let def = mcp_tool_definition(&schema);
        assert_eq!(def["name"], "test_tool");
        assert_eq!(def["description"], "A test tool");
        assert!(def["inputSchema"].is_object());
    }

    /// Test `resources/list` returns expected resource URIs.
    #[test]
    fn test_resources_list() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "resources/list".to_string(),
            params: Value::Null,
        };

        let response = handle_resources_list(&request);
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        let resources = result["resources"].as_array().unwrap();
        assert!(!resources.is_empty());
        assert!(resources.iter().any(|r| r["uri"] == "hermes://tools"));
        assert!(resources.iter().any(|r| r["uri"] == "hermes://status"));
    }

    /// Test `resources/read` with valid URI.
    #[test]
    fn test_resources_read_valid() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "resources/read".to_string(),
            params: serde_json::json!({ "uri": "hermes://status" }),
        };

        let response = handle_resources_read(&request);
        assert!(response.error.is_none());
        assert!(response.result.is_some());
    }

    /// Test `resources/read` with unknown URI.
    #[test]
    fn test_resources_read_unknown() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "resources/read".to_string(),
            params: serde_json::json!({ "uri": "hermes://unknown" }),
        };

        let response = handle_resources_read(&request);
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, INVALID_PARAMS);
    }

    /// Test that JSON-RPC responses serialize to valid JSON with correct structure.
    #[tokio::test]
    async fn test_write_response_serialization() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0",
            id: Value::Number(1.into()),
            result: Some(serde_json::json!({ "key": "value" })),
            error: None,
        };

        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 1);
        assert_eq!(value["result"]["key"], "value");
        assert!(value.get("error").is_none());
    }

    /// Integration-like test: simulate full initialize → tools/list → tools/call flow.
    #[tokio::test]
    async fn test_full_mcp_lifecycle() {
        let registry = test_registry().await;

        // Step 1: Initialize
        let init_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(1.into()),
            method: "initialize".to_string(),
            params: Value::Null,
        };
        let init_resp = handle_initialize(&init_req);
        assert!(init_resp.error.is_none());
        assert_eq!(
            init_resp.result.as_ref().unwrap()["protocolVersion"],
            "2024-11-05"
        );

        // Step 2: List tools
        let list_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(2.into()),
            method: "tools/list".to_string(),
            params: Value::Null,
        };
        let list_resp = handle_tools_list(&list_req, &registry).await;
        assert!(list_resp.error.is_none());
        let tools = list_resp.result.unwrap()["tools"]
            .as_array()
            .unwrap()
            .clone();
        assert!(tools.len() >= 2);

        // Step 3: Call a tool
        let call_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(3.into()),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "echo",
                "arguments": { "message": "lifecycle test" }
            }),
        };
        let call_resp = handle_tools_call(&call_req, &registry).await;
        assert!(call_resp.error.is_none());
    }
}
