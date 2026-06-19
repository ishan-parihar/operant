//! Terminal/shell command execution tool
//!
//! Provides secure shell command execution capabilities with pluggable backends
//! (local, Docker, SSH).

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::config::runtime_config;
use crate::schema::ToolSchema;
use crate::tools::terminal_backend::{self, CommandOutput};
use crate::tools::{HermesTool, ToolContext, ToolResult};

pub struct TerminalTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalArgs {
    command: String,
    working_dir: Option<String>,
    env_vars: Option<HashMap<String, String>>,
    timeout: Option<u64>,
    max_output: Option<usize>,
    use_shell: Option<bool>,
}

#[async_trait]
impl HermesTool for TerminalTool {
    fn name(&self) -> &str {
        "terminal"
    }

    fn description(&self) -> &str {
        "Execute a command and return its output. Supports custom working directory and environment variables. Uses direct execution by default (preventing injection), but can use a shell if `useShell` is set to true."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TerminalArgs>("terminal", "Execute shell command")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TerminalArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("terminal", format!("Invalid arguments: {}", e)),
        };
        let settings = runtime_config().tools.terminal;

        let timeout = std::time::Duration::from_secs(
            args.timeout
                .unwrap_or(settings.max_timeout_secs)
                .min(settings.max_timeout_secs),
        );
        let max_output = args.max_output.unwrap_or(settings.max_output_bytes);

        let cwd = args.working_dir.as_deref().map(std::path::Path::new);
        let env_vars = args.env_vars.unwrap_or_default();
        let use_shell = args.use_shell.unwrap_or(false);

        let backend = terminal_backend::create_backend(&runtime_config());

        let output: CommandOutput = match backend
            .execute_command(&args.command, cwd, &env_vars, use_shell, timeout, max_output)
            .await
        {
            Ok(o) => o,
            Err(e) => return ToolResult::error("terminal", format!("{}", e)),
        };

        if output.success {
            ToolResult::success(
                "terminal",
                serde_json::json!({
                    "success": true,
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "runtime": backend.name(),
                }),
            )
        } else {
            ToolResult::success(
                "terminal",
                serde_json::json!({
                    "success": false,
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "runtime": backend.name(),
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_terminal_tool_direct_execution() {
        let tool = TerminalTool;
        let args = json!({
            "command": "echo 'hello world'"
        });

        let result = tool.execute(args, ToolContext::default()).await;

        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(!stdout.is_empty(), "stdout should not be empty");
    }

    #[tokio::test]
    async fn test_terminal_tool_empty_command() {
        let tool = TerminalTool;
        let args = json!({ "command": "" });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("Empty command"));
    }

    #[tokio::test]
    async fn test_terminal_tool_command_fails() {
        let tool = TerminalTool;
        let args = json!({ "command": "false" });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success); // Non-zero exit is still a "successful" execution
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["success"], false);
        assert_eq!(v["exit_code"], 1);
    }

    #[tokio::test]
    async fn test_terminal_tool_shell_mode() {
        let tool = TerminalTool;
        let args = json!({
            "command": "echo hello",
            "use_shell": true
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["exit_code"], 0);
        let stdout = v["stdout"].as_str().unwrap();
        assert!(stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_terminal_tool_env_vars() {
        let tool = TerminalTool;
        let args = json!({
            "command": "echo $HERMES_TEST_VAR",
            "envVars": { "HERMES_TEST_VAR": "custom_value" },
            "useShell": true
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(stdout.contains("custom_value"));
    }

    #[tokio::test]
    async fn test_terminal_tool_working_dir() {
        let tool = TerminalTool;
        let args = json!({
            "command": "pwd",
            "workingDir": "/tmp"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(stdout.contains("/tmp"));
    }

    #[tokio::test]
    async fn test_backend_factory_local() {
        let config = crate::config::AppConfig::default();
        let backend = terminal_backend::create_backend(&config);
        assert_eq!(backend.name(), "local");
        assert!(backend.is_available());
    }
}
