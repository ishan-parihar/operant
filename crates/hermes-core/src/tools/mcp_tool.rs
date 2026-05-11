//! MCP Management Tool
//!
//! Agent-invocable tool for managing MCP servers at runtime.
//! Wraps McpManager with HermesTool trait for agent access.
//! Supports listing servers and tools, calling tools, adding/removing HTTP servers.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::mcp::McpManager;
use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

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
impl HermesTool for McpManagementTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Manage MCP (Model Context Protocol) servers: list connected servers, list available tools, call tools on servers, add new HTTP servers, or remove servers."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<McpManagementArgs>(self.name(), self.description())
    }

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
                        )
                    }
                };
                let tool_name = match parsed.tool_name {
                    Some(ref n) if !n.is_empty() => n.clone(),
                    _ => {
                        return ToolResult::error(
                            self.name(),
                            "tool_name is required for call_tool".to_string(),
                        )
                    }
                };
                let transport = self.mcp_manager.get(&server_name).await;
                match transport {
                    Some(t) => {
                        let args = parsed.arguments.unwrap_or(Value::Object(Default::default()));
                        match t.call_tool(&tool_name, args).await {
                            Ok(result) => {
                                let content = serde_json::to_string(&result)
                                    .unwrap_or_else(|_| "{}".to_string());
                                ToolResult::success(self.name(), content)
                            }
                            Err(e) => ToolResult::error(
                                self.name(),
                                format!("Tool call failed: {e}"),
                            ),
                        }
                    }
                    None => ToolResult::error(
                        self.name(),
                        format!("Server '{server_name}' not found"),
                    ),
                }
            }
            McpManagementAction::AddServer => {
                let server_name = match parsed.server_name {
                    Some(n) => n,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "server_name is required for add_server".to_string(),
                        )
                    }
                };
                let server_url = match parsed.server_url {
                    Some(u) => u,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "server_url is required for add_server".to_string(),
                        )
                    }
                };
                match self
                    .mcp_manager
                    .add_server(server_name, server_url, parsed.auth_token)
                    .await
                {
                    Ok(_) => ToolResult::success(self.name(), "Server added successfully"),
                    Err(e) => {
                        ToolResult::error(self.name(), format!("Failed to add server: {e}"))
                    }
                }
            }
            McpManagementAction::RemoveServer => {
                let server_name = match parsed.server_name {
                    Some(ref n) => n.clone(),
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "server_name is required for remove_server".to_string(),
                        )
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

    #[tokio::test]
    async fn test_mcp_tool_invalid_args() {
        let tool = McpManagementTool::new(McpManager::new());
        let args = serde_json::json!({"action": "invalid_action"});
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
    }
}
