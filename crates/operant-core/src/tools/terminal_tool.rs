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
use crate::terminal_hints::annotate_failure;
use crate::tools::terminal_backend::{self, CommandOutput};
use crate::tools::{OperantTool, ToolContext, ToolResult};

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

/// What the rtk path decided about one command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RtkOutcome {
    /// rtk has a compressed equivalent; this is the command to run.
    Rewrite(String),
    /// rtk has no equivalent, or the bridge could not ask it.
    NoRewrite,
    /// rtk's own policy denies this command; it must not run.
    Denied(String),
}

/// Ask rtk how it would run this command.
///
/// The answer is a MAPPING, never a permission: `rtk rewrite` answers "how
/// would I run this", and operant's own fence decides whether the answer may
/// run at all.
///
/// Two guards keep the substitution honest:
///
/// * A rewrite that introduces shell syntax is only used when the command
///   was already going through a shell. Without this, `ls | grep foo`
///   rewritten to `ls | rtk grep foo` would be handed to a direct exec, which
///   would run `ls` with the rest as arguments — a silent change of meaning.
/// * A rewrite is never used when rtk is not installed or not answering: the
///   original command runs unchanged.
fn rtk_rewritten_command(command: &str, use_shell: bool) -> RtkOutcome {
    use crate::rtk_bridge::{RewriteVerdict, find_rtk, rewrite_command};

    let Some(rtk) = find_rtk() else {
        return RtkOutcome::NoRewrite;
    };
    match rewrite_command(&rtk, command) {
        RewriteVerdict::Rewritten { command: rewritten } => {
            if !use_shell && rewritten.contains(['|', '>', '<', '&', ';', '`', '$']) {
                // The rewrite needs a shell and the command was going to run
                // directly. Running it as-is would change what it does.
                return RtkOutcome::NoRewrite;
            }
            RtkOutcome::Rewrite(rewritten)
        }
        RewriteVerdict::NoRule => RtkOutcome::NoRewrite,
        RewriteVerdict::Refused { reason } => RtkOutcome::Denied(reason),
        // rtk could not be asked. That is not a refusal: the original runs.
        RewriteVerdict::Failed { .. } => RtkOutcome::NoRewrite,
    }
}

#[async_trait]
impl OperantTool for TerminalTool {
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

        // O-18: the policy-commands fence — a Holon run whose policy.json
        // lists allowed commands refuses everything else up front, with
        // the refusal recorded for the result envelope. Personal runs and
        // ungoverned policies pass straight through.
        // RT-454: the fence judges every command in the line when the line
        // runs through a shell (the backend hands the whole line to it), so
        // `git status; rm -rf /` cannot smuggle the second command past the
        // policy's allowlist. Without a shell the line is one command with
        // arguments, and the first token is the binary.
        let use_shell = args.use_shell.unwrap_or(false);
        if let Err(why) =
            crate::tools::file_tools::holon_command_fence_with_shell(&args.command, use_shell)
        {
            return ToolResult::error("terminal", why);
        }

        let timeout = std::time::Duration::from_secs(
            args.timeout
                .unwrap_or(settings.max_timeout_secs)
                .min(settings.max_timeout_secs),
        );
        let max_output = args.max_output.unwrap_or(settings.max_output_bytes);

        let cwd = args.working_dir.as_deref().map(std::path::Path::new);
        let env_vars = args.env_vars.unwrap_or_default();

        let backend = match terminal_backend::create_backend(&runtime_config()) {
            Ok(b) => b,
            Err(e) => return ToolResult::error("terminal", format!("{}", e)),
        };

        // The rtk path: a BACKEND SUBSTITUTION, not a second filter.
        //
        // `[compression] shell = "rtk"` makes the `rtk` binary the executor
        // for terminal commands — the one defensible place to run RTK, because
        // operant then does NOT compress the result a second time (the
        // evidence layer skips shell results in this mode; see
        // `agent/evidence/mod.rs`).
        //
        // Two rules, both load-bearing:
        //
        // 1. THE REWRITE IS A MAPPING, NOT A GATE. `rtk rewrite` answers how
        //    rtk would run the command; its exit code is a permission verdict
        //    (exit 3 = "here is the rewrite, you decide", exit 0 = a host
        //    granted it). It must never BYPASS operant's own fence: the fence
        //    below runs on the command that will ACTUALLY execute.
        // 2. A DENY IS THE OPERATOR'S OWN POLICY. rtk's deny rules live in the
        //    operator's `~/.config/rtk/filters.toml`, so a command rtk denies
        //    is one the operator blocked. Running it anyway would make that
        //    deny rule a lie, so the command does not run.
        //
        // Outside rtk mode this is a single config read and no process at all.
        let rtk_mode = runtime_config().compression.shell == crate::agent::evidence::ShellMode::Rtk;
        let (effective_command, rtk_note): (String, Option<String>) = if rtk_mode {
            match rtk_rewritten_command(&args.command, use_shell) {
                RtkOutcome::Rewrite(command) => {
                    (command, Some("rtk rewrote this command".to_string()))
                }
                RtkOutcome::Denied(reason) => {
                    return ToolResult::error(
                        "terminal",
                        format!("rtk denied this command and it was not run: {reason}"),
                    );
                }
                RtkOutcome::NoRewrite => (args.command.clone(), None),
            }
        } else {
            (args.command.clone(), None)
        };

        // The fence sees the command that WILL run — the rewrite, not the
        // original. Without this, a rewrite could smuggle a command past the
        // Holon policy allowlist under a name the fence never judged.
        //
        // And when the REWRITE is what the fence refuses, the ORIGINAL runs
        // instead: the rewrite is an optimisation, and an optimisation that is
        // not allowed must not become a refusal of the command the model
        // actually asked for (the original already passed the fence above).
        let effective_command = if effective_command == args.command
            || crate::tools::file_tools::holon_command_fence_with_shell(
                &effective_command,
                use_shell,
            )
            .is_ok()
        {
            effective_command
        } else {
            args.command.clone()
        };

        let output: CommandOutput = match backend
            .execute_command(
                &effective_command,
                cwd,
                &env_vars,
                use_shell,
                timeout,
                max_output,
            )
            .await
        {
            Ok(o) => o,
            Err(e) => return ToolResult::error("terminal", format!("{}", e)),
        };

        let runtime_label = match rtk_note {
            Some(note) => format!("{} ({note})", backend.name()),
            None => backend.name().to_string(),
        };

        if output.success {
            ToolResult::success(
                "terminal",
                serde_json::json!({
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "runtime": runtime_label,
                }),
            )
        } else {
            // Plan 010: append an actionable hint when the failure matches
            // one of the terminal_hints heuristics. The hint becomes part
            // of the model-visible error content so it can self-correct.
            let hint = annotate_failure(&effective_command, output.exit_code, &output.stderr);
            ToolResult::error(
                "terminal",
                serde_json::json!({
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "runtime": runtime_label,
                    "hint": hint,
                })
                .to_string(),
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
        // Failed command (non-zero exit) should return error result
        assert!(!result.success);
        assert!(result.error.is_some());
        let v: Value = serde_json::from_str(result.error.as_ref().unwrap()).unwrap();
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
        let backend = terminal_backend::create_backend(&config)
            .expect("default config selects the local backend");
        assert_eq!(backend.name(), "local");
        assert!(backend.is_available());
    }
}
