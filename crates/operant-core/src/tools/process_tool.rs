use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::process_registry::{ProcessRegistry, ProcessSession};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ProcessToolArgs {
    /// Action: list, spawn, poll, wait, kill, get_output
    action: String,
    /// Process command (for spawn action)
    command: Option<String>,
    /// Working directory (for spawn action)
    cwd: Option<String>,
    /// Session ID (for poll/wait/kill/get_output actions)
    session_id: Option<String>,
    /// Timeout in seconds (for wait action)
    timeout_secs: Option<u64>,
    /// Whether to notify on completion (for spawn action)
    notify_on_complete: Option<bool>,
}

#[derive(Serialize)]
struct ProcessListResult {
    processes: Vec<ProcessSession>,
    running_count: usize,
}

pub struct ProcessTool {
    registry: ProcessRegistry,
}

impl ProcessTool {
    pub fn new(registry: ProcessRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl OperantTool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage background processes: list, spawn, poll, wait, kill, and get output"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ProcessToolArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: ProcessToolArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {e}")),
        };

        match parsed.action.as_str() {
            "list" => {
                let sessions = self.registry.list().await;
                let running_count = self.registry.running_count().await;
                ToolResult::success(
                    self.name(),
                    ProcessListResult {
                        processes: sessions,
                        running_count,
                    },
                )
            }

            "spawn" => {
                let command = match parsed.command {
                    Some(c) => c,
                    None => {
                        return ToolResult::error(self.name(), "Missing 'command' for spawn action")
                    }
                };
                match self.registry.spawn(command, parsed.cwd).await {
                    Ok(session) => ToolResult::success(self.name(), session),
                    Err(e) => ToolResult::error(self.name(), format!("Failed to spawn: {e}")),
                }
            }

            "poll" => {
                let sid = match parsed.session_id {
                    Some(s) => s,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "Missing 'sessionId' for poll action",
                        )
                    }
                };
                match self.registry.poll(&sid).await {
                    Some(session) => ToolResult::success(self.name(), session),
                    None => ToolResult::error(self.name(), format!("Process '{sid}' not found")),
                }
            }

            "wait" => {
                let sid = match parsed.session_id {
                    Some(s) => s,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "Missing 'sessionId' for wait action",
                        )
                    }
                };
                match self.registry.wait(&sid, parsed.timeout_secs).await {
                    Some(session) => ToolResult::success(self.name(), session),
                    None => ToolResult::error(self.name(), format!("Process '{sid}' not found")),
                }
            }

            "kill" => {
                let sid = match parsed.session_id {
                    Some(s) => s,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "Missing 'sessionId' for kill action",
                        )
                    }
                };
                match self.registry.kill(&sid).await {
                    Ok(()) => ToolResult::success(self.name(), serde_json::json!({"killed": true})),
                    Err(e) => ToolResult::error(self.name(), e),
                }
            }

            "get_output" => {
                let sid = match parsed.session_id {
                    Some(s) => s,
                    None => {
                        return ToolResult::error(
                            self.name(),
                            "Missing 'sessionId' for get_output action",
                        )
                    }
                };
                match self.registry.poll(&sid).await {
                    Some(session) => ToolResult::success(
                        self.name(),
                        serde_json::json!({
                            "sessionId": session.id,
                            "output": session.output_buffer,
                            "status": session.status,
                        }),
                    ),
                    None => ToolResult::error(self.name(), format!("Process '{sid}' not found")),
                }
            }

            other => ToolResult::error(
                self.name(),
                format!(
                    "Unknown action: '{other}'. Use: list, spawn, poll, wait, kill, get_output"
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_schema() {
        let schema = ToolSchema::from_type::<ProcessToolArgs>("process", "test");
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
    }
}
