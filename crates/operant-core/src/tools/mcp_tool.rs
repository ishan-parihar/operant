//! MCP Management Tool
//!
//! Agent-invocable tool for managing MCP servers at runtime.
//! Wraps McpManager with OperantTool trait for agent access.
//! Supports listing servers and tools, calling tools, adding/removing HTTP servers.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::mcp::McpManager;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub const TOOL_NAME: &str = "mcp_management";

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
enum McpManagementAction {
    ListServers,
    ListTools,
    CallTool,
    AddServer,
    RemoveServer,
}

#[derive(JsonSchema, Deserialize)]
struct McpManagementArgs {
    action: McpManagementAction,
    server_name: Option<String>,
    server_url: Option<String>,
    tool_name: Option<String>,
    arguments: Option<Value>,
    auth_token: Option<String>,
}

/// Validate a model-supplied `server_url` for `add_server`: must parse as a
/// URL with an http/https scheme and a non-empty host. Prevents the agent
/// from registering servers at arbitrary schemes (file://, etc.) or
/// malformed addresses. Loopback hosts stay allowed — local MCP dev servers
/// are a legitimate target.
fn validate_server_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let parsed =
        url::Url::parse(trimmed).map_err(|e| format!("server_url is not a valid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "server_url scheme must be http or https, got '{other}'"
            ));
        }
    }
    if parsed.host_str().is_none_or(|h| h.is_empty()) {
        return Err(format!("server_url must include a host, got: {raw}"));
    }
    Ok(trimmed.to_string())
}

pub struct McpManagementTool {
    mcp_manager: McpManager,
}

impl McpManagementTool {
    pub fn new(manager: McpManager) -> Self {
        Self {
            mcp_manager: manager,
        }
    }
}

#[async_trait]
impl OperantTool for McpManagementTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Manage MCP (Model Context Protocol) servers: list connected servers, list available tools, call tools on servers, add new HTTP servers, or remove servers."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<McpManagementArgs>(self.name(), self.description())
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: McpManagementArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {e}")),
        };

        match parsed.action {
            McpManagementAction::ListServers => {
                let servers = self.mcp_manager.server_names().await;
                let result = serde_json::json!({ "servers": servers });
                ToolResult::success(self.name(), result)
            }
            McpManagementAction::ListTools => {
                let tools = self.mcp_manager.get_all_tools().await;
                let tool_info: Vec<Value> = tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name(),
                            "description": t.description(),
                        })
                    })
                    .collect();
                let result = serde_json::json!({ "tools": tool_info });
                ToolResult::success(self.name(), result)
            }
            McpManagementAction::CallTool => {
                let server_name = match parsed.server_name {
                    Some(ref n) if !n.is_empty() => n.clone(),
                    _ => {
                        return ToolResult::error(
                            self.name(),
                            "server_name is required for call_tool".to_string(),
                        );
                    }
                };
                let tool_name = match parsed.tool_name {
                    Some(ref n) if !n.is_empty() => n.clone(),
                    _ => {
                        return ToolResult::error(
                            self.name(),
                            "tool_name is required for call_tool".to_string(),
                        );
                    }
                };
                let transport = self.mcp_manager.get(&server_name).await;
                match transport {
                    Some(t) => {
                        let args = parsed
                            .arguments
                            .unwrap_or(Value::Object(Default::default()));
                        match t.call_tool(&tool_name, args).await {
                            Ok(result) => {
                                let content = serde_json::to_string(&result)
                                    .expect("MCP tool result always serializes");
                                ToolResult::success(self.name(), content)
                            }
                            Err(e) => {
                                ToolResult::error(self.name(), format!("Tool call failed: {e}"))
                            }
                        }
                    }
                    None => {
                        ToolResult::error(self.name(), format!("Server '{server_name}' not found"))
                    }
                }
            }
            McpManagementAction::AddServer => {
                let server_name = match parsed.server_name {
                    Some(n) => n,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "server_name is required for add_server".to_string(),
                        );
                    }
                };
                let server_url = match parsed.server_url {
                    Some(u) => u,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "server_url is required for add_server".to_string(),
                        );
                    }
                };
                // Model-supplied URL: enforce http/https scheme + host so the
                // agent cannot register servers at arbitrary schemes or
                // malformed addresses (hermes only loads config-declared
                // server URLs).
                let server_url = match validate_server_url(&server_url) {
                    Ok(u) => u,
                    Err(e) => return ToolResult::error(self.name(), e),
                };
                // Reject re-adding an already-connected name instead of
                // silently clobbering the existing server.
                if self.mcp_manager.contains(&server_name).await {
                    return ToolResult::error(
                        self.name(),
                        format!("Server '{server_name}' is already connected"),
                    );
                }
                match self
                    .mcp_manager
                    .add_server(server_name, server_url, parsed.auth_token)
                    .await
                {
                    Ok(_) => ToolResult::success(self.name(), "Server added successfully"),
                    Err(e) => ToolResult::error(self.name(), format!("Failed to add server: {e}")),
                }
            }
            McpManagementAction::RemoveServer => {
                let server_name = match parsed.server_name {
                    Some(ref n) => n.clone(),
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "server_name is required for remove_server".to_string(),
                        );
                    }
                };
                match self.mcp_manager.remove_server(&server_name).await {
                    Ok(_) => {
                        ToolResult::success(self.name(), format!("Server '{server_name}' removed"))
                    }
                    Err(e) => {
                        ToolResult::error(self.name(), format!("Failed to remove server: {e}"))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_tool_name_and_description() {
        let tool = McpManagementTool::new(McpManager::new());
        assert_eq!(tool.name(), "mcp_management");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_mcp_tool_list_servers_empty() {
        let tool = McpManagementTool::new(McpManager::new());
        let args = serde_json::json!({ "action": "list_servers" });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
    }

    #[test]
    fn test_validate_server_url_accepts_http_and_https() {
        assert!(validate_server_url("http://localhost:8080/mcp").is_ok());
        assert!(validate_server_url("https://mcp.example.com").is_ok());
    }

    #[test]
    fn test_validate_server_url_rejects_bad_schemes() {
        let err = validate_server_url("file:///etc/passwd").unwrap_err();
        assert!(err.contains("http or https"), "got: {err}");
        assert!(validate_server_url("ftp://example.com").is_err());
        // `localhost:8080` parses with scheme `localhost` — rejected.
        assert!(validate_server_url("localhost:8080").is_err());
    }

    #[test]
    fn test_validate_server_url_rejects_missing_host() {
        assert!(validate_server_url("http://").is_err());
        assert!(validate_server_url("not a url").is_err());
        // (the URL crate parses `https:///path` as host `path` per the WHATWG
        // spec — a hostname like any other, so it is accepted, not a hole)
        assert!(validate_server_url("https:///path").is_ok());
    }

    #[tokio::test]
    async fn test_mcp_tool_add_server_rejects_bad_url_before_network() {
        // Validation happens before any connect attempt, so a bad URL fails
        // fast without touching the network.
        let tool = McpManagementTool::new(McpManager::new());
        let args = serde_json::json!({
            "action": "add_server",
            "server_name": "evil",
            "server_url": "file:///etc/passwd",
            "auth_token": "secret"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("http or https"), "got: {err}");
    }

    #[tokio::test]
    async fn test_mcp_tool_add_server_connect_failure_reported() {
        // A valid-format URL to a dead port fails at connect and is reported
        // as a failed add — proves the http/https path still reaches connect.
        let tool = McpManagementTool::new(McpManager::new());
        let args = serde_json::json!({
            "action": "add_server",
            "server_name": "dead",
            "server_url": "http://127.0.0.1:1/mcp"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Failed to add server"));
    }

    #[tokio::test]
    async fn test_mcp_tool_invalid_args() {
        let tool = McpManagementTool::new(McpManager::new());
        let args = serde_json::json!({"action": "invalid_action"});
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
    }
}
