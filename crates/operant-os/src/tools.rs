//! The native Holon tools, generated from ONE definition (audit O-04/O-05):
//! `vendor/holon-tools.json` — the exact output of Holon's own
//! `holon.mcp.tools()` registry walk. The vendor file is the contract;
//! `native_tool_table()` parses it, `register_all` registers every entry,
//! and `NATIVE_TOOL_NAMES` is the single list the observe posture and the
//! availability gates read. The two sides cannot drift because the table
//! IS the list.
//!
//! Argv rules (checked against Holon's argparse):
//! - every property becomes `--<name> <value>`, booleans become flags;
//! - free text (content, body, result, intent, title, message…) goes on
//!   STDIN with `-` as the argument (O-09: untrusted text never in argv,
//!   and it can never name a file the kernel reads);
//! - the kernel is resolved through `$HOLON` (a shell command string,
//!   split with shlex rules), else `[holon] bin`, else `holon` on PATH.

use std::sync::LazyLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolResult};

/// One entry of the vendored tool list.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// the kernel's generator writes camelCase
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

/// The vendored kernel tool list — the output of `holon.mcp.tools()`.
#[expect(
    clippy::expect_used,
    reason = "compile-time vendored JSON; a parse failure is a build-time invariant violation"
)]
pub static NATIVE_TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| {
    static VENDOR: &str = include_str!("../vendor/holon-tools.json");
    serde_json::from_str(VENDOR)
        .expect("vendor/holon-tools.json must be the exact holon.mcp.tools() output")
});

/// The single list of native tool names (O-05): the observe allowlist and
/// the availability gates read this, so a name that is not registered
/// cannot appear anywhere.
pub fn native_tool_names() -> Vec<&'static str> {
    // SAFETY-free: names are static strings in the vendor; leak the
    // allocation once — the process lives as long as the registry.
    NATIVE_TOOLS.iter().map(|t| t.name.as_str()).collect()
}

/// OS-8: a JSON key becomes `--key` on the kernel's argv, so it must be a plain
/// flag name: a letter, then letters, digits, `-` or `_`. Anything else (`--`,
/// `-x`, `a=b`) could end option parsing or smuggle a second flag.
fn is_flag_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Arguments whose values are free (possibly untrusted) text: they ride
/// STDIN with `-`, never argv (O-09).
fn is_stdin_arg(name: &str) -> bool {
    matches!(
        name,
        "content"
            | "body"
            | "result"
            | "intent"
            | "title"
            | "message"
            | "text"
            | "findings"
            | "notes"
    )
}

/// How operant reaches the kernel for a body's tools: the contract exports
/// `$HOLON` as a shell command string (e.g. `/usr/bin/python3 -m holon
/// --home …`); `[holon] bin` or the PATH `holon` is the fallback.
fn kernel_command(bin: Option<&str>) -> Vec<String> {
    if let Ok(holon) = std::env::var("HOLON")
        && !holon.is_empty()
    {
        // shlex-style split: quotes group; no escapes beyond that (the
        // contract's shape is `program args…` with no shell metacharacters
        // of its own — we do not spawn a shell).
        return split_word_list(&holon);
    }
    let bin = bin
        .map(str::to_string)
        .or_else(|| std::env::var("HOLON_BIN").ok())
        .unwrap_or_else(|| "holon".to_string());
    vec![bin]
}

/// Minimal shell-word split (POSIX single/double quotes, no escapes).
fn split_word_list(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in s.chars() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// One generic tool, driven by its vendored definition.
pub struct NativeCliTool {
    def: ToolDef,
    /// The subcommand path: `holon_org` -> ["org"], `holon_task_show` ->
    /// ["task", "show"]; whoami/submit are special-cased.
    argv_path: Vec<String>,
}

impl NativeCliTool {
    pub fn from_def(def: &ToolDef) -> Self {
        let argv_path = def
            .name
            .strip_prefix("holon_")
            .unwrap_or(def.name.as_str())
            .split('_')
            .map(str::to_string)
            .collect();
        Self {
            def: def.clone(),
            argv_path,
        }
    }
}

#[async_trait]
impl OperantTool for NativeCliTool {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn description(&self) -> &str {
        &self.def.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            self.def.input_schema.clone(),
        )
    }

    /// A tool is available exactly when this process is a Holon body: the
    /// run env (the kernel's prepared variables) decides, not config.
    fn is_available(&self) -> bool {
        operant_core::platform::is_holon_run()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        if !self.is_available() {
            return ToolResult::error(self.name(), "only available inside a Holon run".to_string());
        }
        if self.def.name == "holon_whoami" {
            return self.run_kernel(&[], None).await;
        }
        if self.def.name == "holon_submit" {
            return self.submit(args).await;
        }
        self.cli_call(args).await
    }
}

impl NativeCliTool {
    /// Build argv from the tool's properties: `--name value`, booleans as
    /// flags, stdin-text properties as `--name -` with the value piped.
    async fn cli_call(&self, args: Value) -> ToolResult {
        let obj = match args {
            Value::Object(m) => m,
            _ => {
                return ToolResult::error(
                    self.def.name.clone(),
                    "arguments must be an object".to_string(),
                );
            }
        };
        let mut argv: Vec<String> = Vec::new();
        let mut stdin_text: Option<String> = None;
        for (key, val) in &obj {
            if !is_flag_name(key) {
                return ToolResult::error(
                    self.def.name.clone(),
                    format!("argument `{key}` is not a flag name"),
                );
            }
            if val.is_null() {
                continue;
            }
            let flag = format!("--{key}");
            if val.as_bool() == Some(true) {
                argv.push(flag);
                continue;
            }
            if val.as_bool() == Some(false) {
                continue; // a false flag is the default; never pass it
            }
            let text = match val {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if is_stdin_arg(key) {
                if stdin_text.is_some() {
                    return ToolResult::error(
                        self.def.name.clone(),
                        "one stdin-text argument per call".to_string(),
                    );
                }
                stdin_text = Some(text);
                argv.push(flag);
                argv.push("-".to_string());
            } else {
                argv.push(flag);
                argv.push(text);
            }
        }
        self.run_kernel(&argv, stdin_text).await
    }

    /// `holon_submit`: the envelope is a JSON object — the kernel's
    /// `submit <run_id> --file -` reads it from stdin.
    async fn submit(&self, args: Value) -> ToolResult {
        let Some(run_id) = std::env::var("HOLON_RUN_ID").ok() else {
            return ToolResult::error(
                self.def.name.clone(),
                "no HOLON_RUN_ID — this body has nothing to submit".to_string(),
            );
        };
        let Some(result) = args.get("result") else {
            return ToolResult::error(
                self.def.name.clone(),
                "the `result` envelope is required".to_string(),
            );
        };
        let body = serde_json::to_string(result).unwrap_or_default();
        self.run_kernel(
            &["submit".into(), run_id, "--file".into(), "-".into()],
            Some(body),
        )
        .await
    }

    /// One kernel invocation: `sh`-free, env passed explicitly (the body's
    /// own env — the token is the body's), stdout/stderr captured.
    async fn run_kernel(&self, extra: &[String], stdin_text: Option<String>) -> ToolResult {
        use tokio::io::AsyncWriteExt;
        let mut argv = kernel_command(None);
        argv.extend(self.argv_path.iter().cloned());
        argv.extend(extra.iter().cloned());
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(
                    self.def.name.clone(),
                    format!(
                        "kernel not reachable ({e}): $HOLON or [holon] bin must name the launcher"
                    ),
                );
            }
        };
        if let Some(text) = stdin_text {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes()).await;
                let _ = stdin.shutdown().await;
            }
        } else if let Some(stdin) = child.stdin.take() {
            drop(stdin); // no stdin: close it so the kernel does not wait
        }
        // wait_with_output consumes the child; dropping the timed-out future
        // drops it, and kill_on_drop SIGKILLs it (OS-8), so a retry cannot
        // race a still-running submit.
        let wait = child.wait_with_output();
        match tokio::time::timeout(std::time::Duration::from_secs(120), wait).await {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if out.status.success() {
                    ToolResult::success(self.def.name.clone(), stdout.trim().to_string())
                } else {
                    ToolResult::error(
                        self.def.name.clone(),
                        if stderr.trim().is_empty() {
                            format!(
                                "kernel exit {}: {}",
                                out.status.code().unwrap_or(-1),
                                stdout.trim()
                            )
                        } else {
                            stderr.trim().to_string()
                        },
                    )
                }
            }
            Ok(Err(e)) => ToolResult::error(self.def.name.clone(), format!("kernel wait: {e}")),
            Err(_) => ToolResult::error(
                self.def.name.clone(),
                "kernel call timed out (120s)".to_string(),
            ),
        }
    }
}

/// Register every tool the vendored kernel list names. The registry stays
/// in lockstep with `vendor/holon-tools.json` — the vendor-is-stale test
/// (tests/native_tools.rs) regenerates and compares when HOLON_REPO is set.
pub async fn register_all(registry: &operant_core::tools::ToolRegistry) {
    for def in NATIVE_TOOLS.iter() {
        let name = def.name.clone();
        if let Err(e) = registry
            .register_dyn(std::sync::Arc::new(NativeCliTool::from_def(def)))
            .await
        {
            tracing::warn!(tool = %name, error = %e, "holon tool registration failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_flag_name;

    #[test]
    fn only_plain_names_become_flags() {
        for ok in ["file", "run_id", "run-id", "a1"] {
            assert!(is_flag_name(ok), "{ok} should be a flag name");
        }
        for bad in ["", "-", "--", "-x", "a=b", "1x", "a b", "x/../y"] {
            assert!(!is_flag_name(bad), "{bad:?} must not become a flag");
        }
    }
}
