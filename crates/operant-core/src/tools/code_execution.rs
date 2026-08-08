//! Code execution tool
//!
//! Executes user-supplied code (python / javascript / rust / shell) on the
//! **local host** with a hard wall-clock timeout and `kill_on_drop` + whole
//! process-group teardown (no orphaned processes — grandchildren included).
//! This is NOT a sandbox: code runs with the operant process's own
//! permissions and filesystem/network access.
//!
//! Mitigations: the tool call passes through the agent's approval gate
//! (`code_execution` is in the interactive permission list), the run is
//! time-boxed by a single deadline (a child that streams output faster than
//! the timeout per line can no longer run unbounded), and captured output is
//! capped (50 KB stdout / 10 KB stderr with head/tail truncation — hermes
//! `code_execution_tool.py` `MAX_STDOUT_BYTES`/`MAX_STDERR_BYTES` parity).
//! Hermes runs code execution inside a real sandboxed subprocess (AF_UNIX
//! transport, env hardening, docker/modal/vercel backends) — see BUGS.md
//! R12-1; full sandbox parity is future work.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::config::runtime_config;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Hermes parity (`code_execution_tool.py`): capture is capped so a runaway
/// child cannot exhaust memory.
const MAX_STDOUT_BYTES: usize = 50_000;
const MAX_STDERR_BYTES: usize = 10_000;

/// Tool for executing code in various languages
pub struct CodeExecutionTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeExecutionArgs {
    code: String,
    language: String,
    timeout: Option<u64>,
}

#[async_trait]
impl OperantTool for CodeExecutionTool {
    fn name(&self) -> &str {
        "code_execution"
    }

    fn description(&self) -> &str {
        "Execute code in various programming languages (python, javascript, rust, shell). \
        Returns stdout, stderr, and execution time."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CodeExecutionArgs>("code_execution", "Execute code")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: CodeExecutionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("code_execution", format!("Invalid arguments: {}\n", e));
            }
        };
        let settings = runtime_config().tools.code_execution;

        let timeout = Duration::from_secs(
            args.timeout
                .unwrap_or(settings.default_timeout_secs)
                .min(settings.max_timeout_secs),
        );

        let result = match args.language.to_lowercase().as_str() {
            "python" | "py" => execute_python(&args.code, timeout).await,
            "javascript" | "js" | "node" => execute_javascript(&args.code, timeout).await,
            "shell" | "bash" | "sh" => execute_shell(&args.code, timeout).await,
            "rust" | "rs" => execute_rust(&args.code, timeout).await,
            _ => {
                return ToolResult::error(
                    "code_execution",
                    format!("Unsupported language: {}", args.language),
                );
            }
        };

        match result {
            Ok(output) => ToolResult::success("code_execution", output),
            Err(e) => ToolResult::error("code_execution", e),
        }
    }
}

/// Capped output with a truncation flag.
#[derive(Debug, Default)]
struct CappedOutput {
    text: String,
    truncated: bool,
}

/// Largest char boundary at or below `idx` (`floor_char_boundary` is stable
/// only since 1.91, above the workspace MSRV; `is_char_boundary` is stable
/// since 1.0).
fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Bounded output capture. Keeps at most `cap` bytes of streamed output and,
/// when the child exceeded the cap, returns the head 40% + tail 60% with a
/// truncation marker — hermes head/tail truncation parity. The pipe is still
/// drained to EOF so a capped child can never block on a full pipe.
async fn read_capped<R: tokio::io::AsyncRead + Unpin>(mut r: R, cap: usize) -> CappedOutput {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 4096];
    let mut stored: Vec<u8> = Vec::with_capacity(cap.min(8192));
    let mut over_cap = false;
    loop {
        match r.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if stored.len() < cap {
                    let room = cap - stored.len();
                    let take = n.min(room);
                    stored.extend_from_slice(&buf[..take]);
                    if take < n {
                        over_cap = true;
                    }
                } else {
                    over_cap = true;
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&stored).into_owned();
    if !over_cap {
        return CappedOutput {
            text,
            truncated: false,
        };
    }
    let head_bytes = cap * 2 / 5;
    let tail_bytes = cap - head_bytes;
    let head_end = floor_char_boundary(&text, head_bytes);
    let tail_start = floor_char_boundary(&text, text.len().saturating_sub(tail_bytes));
    if tail_start < head_end {
        // Pathological short-multibyte case: head and tail would overlap, so
        // keep the capped text as-is.
        return CappedOutput {
            text,
            truncated: true,
        };
    }
    CappedOutput {
        text: format!(
            "{}\n... [truncated: output exceeded {} bytes] ...\n{}",
            &text[..head_end],
            cap,
            &text[tail_start..]
        ),
        truncated: true,
    }
}

/// Outcome of a capped run.
struct ExecOutcome {
    stdout: CappedOutput,
    stderr: CappedOutput,
    exit_code: Option<i32>,
    timed_out: bool,
}

/// Kill the child's whole process group (created at spawn via
/// `process_group(0)`), escalating TERM → KILL after a short grace period.
/// A lone TERM to the direct child would orphan `&`-spawned descendants, so a
/// negative pid targets the group (hermes `_kill_process_group` parity).
#[cfg(unix)]
async fn kill_process_group(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{pid}"))
        .output();
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .output();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let _ = std::process::Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .output();
    let _ = std::process::Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .output();
}

#[cfg(windows)]
async fn kill_process_group(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .arg("/PID")
        .arg(pid.to_string())
        .arg("/T")
        .arg("/F")
        .output();
}

/// Run a pre-built command with a single hard wall-clock deadline, concurrent
/// capped stdout/stderr reads (no stderr-pipe deadlock), and whole-group
/// teardown on timeout.
async fn run_capped(mut cmd: tokio::process::Command, timeout: Duration) -> ExecOutcome {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ExecOutcome {
                stdout: CappedOutput::default(),
                stderr: CappedOutput {
                    text: format!("Failed to spawn: {e}"),
                    truncated: false,
                },
                exit_code: None,
                timed_out: false,
            };
        }
    };
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let out_task = tokio::spawn(async move {
        match stdout {
            Some(r) => read_capped(r, MAX_STDOUT_BYTES).await,
            None => CappedOutput::default(),
        }
    });
    let err_task = tokio::spawn(async move {
        match stderr {
            Some(r) => read_capped(r, MAX_STDERR_BYTES).await,
            None => CappedOutput::default(),
        }
    });

    // The deadline bounds total wall time, not just each line-read — a child
    // emitting output faster than `timeout` per line can no longer run
    // unbounded (the old implementation wrapped only the per-line reads and
    // the final wait, so a chatty child ran forever). `child.wait()` is
    // cancel-safe, so dropping the branch on timeout is sound.
    let status = {
        let wait_fut = child.wait();
        tokio::pin!(wait_fut);
        tokio::select! {
            status = &mut wait_fut => Some(status),
            _ = tokio::time::sleep(timeout.max(Duration::from_millis(1))) => None,
        }
    };

    match status {
        Some(status) => {
            let (so, se) = tokio::join!(out_task, err_task);
            ExecOutcome {
                stdout: so.unwrap_or_default(),
                stderr: se.unwrap_or_default(),
                exit_code: status.ok().and_then(|s| s.code()),
                timed_out: false,
            }
        }
        None => {
            if let Some(pid) = pid {
                kill_process_group(pid).await;
            }
            // Pipes close once the group is dead; collect partial output.
            let (so, se) = tokio::join!(out_task, err_task);
            let _ = child.wait().await; // reap so no zombie lingers
            ExecOutcome {
                stdout: so.unwrap_or_default(),
                stderr: se.unwrap_or_default(),
                exit_code: None,
                timed_out: true,
            }
        }
    }
}

/// Build the result JSON shared by all language executors.
fn result_json(
    language: &str,
    outcome: &ExecOutcome,
    runtime_ms: u64,
    stage: Option<&str>,
) -> Value {
    let mut v = json!({
        "language": language,
        "exit_code": outcome.exit_code,
        "stdout": outcome.stdout.text,
        "stderr": outcome.stderr.text,
        "runtime_ms": runtime_ms,
        "success": !outcome.timed_out && outcome.exit_code == Some(0),
        "timed_out": outcome.timed_out,
        "stdout_truncated": outcome.stdout.truncated,
        "stderr_truncated": outcome.stderr.truncated,
    });
    if let Some(stage) = stage {
        v["stage"] = Value::String(stage.to_string());
    }
    v
}

async fn execute_python(code: &str, timeout: Duration) -> Result<Value, String> {
    // NamedTempFile: 0600 + O_EXCL (never follows a pre-existing symlink) and
    // auto-removed on drop, so nothing leaks even on early error paths.
    let mut script = tempfile::Builder::new()
        .prefix("operant_code_")
        .suffix(".py")
        .tempfile()
        .map_err(|e| format!("Failed to create temp script: {e}"))?;
    script
        .write_all(code.as_bytes())
        .map_err(|e| format!("Failed to write temp script: {e}"))?;
    let script_path = script.into_temp_path(); // handle closed; removed on drop

    let python_cmd = crate::platform::find_python().unwrap_or_else(|| PathBuf::from("python3"));
    let mut cmd = tokio::process::Command::new(&python_cmd);
    cmd.kill_on_drop(true); // belt-and-braces on top of the group kill
    #[cfg(unix)]
    cmd.process_group(0); // whole-tree teardown on timeout (hermes parity)
    cmd.arg(script_path.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let outcome = run_capped(cmd, timeout).await;
    let runtime_ms = start.elapsed().as_millis() as u64;

    Ok(result_json("python", &outcome, runtime_ms, None))
}

async fn execute_javascript(code: &str, timeout: Duration) -> Result<Value, String> {
    let mut script = tempfile::Builder::new()
        .prefix("operant_code_")
        .suffix(".js")
        .tempfile()
        .map_err(|e| format!("Failed to create temp script: {e}"))?;
    script
        .write_all(code.as_bytes())
        .map_err(|e| format!("Failed to write temp script: {e}"))?;
    let script_path = script.into_temp_path();

    let mut cmd = tokio::process::Command::new("node");
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.arg(script_path.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let outcome = run_capped(cmd, timeout).await;
    let runtime_ms = start.elapsed().as_millis() as u64;

    Ok(result_json("javascript", &outcome, runtime_ms, None))
}

async fn execute_shell(code: &str, timeout: Duration) -> Result<Value, String> {
    let shell = crate::platform::detect_shell();
    let mut cmd = tokio::process::Command::new(&shell.path);
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    for arg in &shell.args_pattern {
        cmd.arg(arg);
    }
    cmd.arg(code);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let start = Instant::now();
    let outcome = run_capped(cmd, timeout).await;
    let runtime_ms = start.elapsed().as_millis() as u64;

    Ok(result_json("shell", &outcome, runtime_ms, None))
}

async fn execute_rust(code: &str, timeout: Duration) -> Result<Value, String> {
    // TempDir auto-removes the whole project on drop — the old implementation
    // leaked the project on compile failures and timeouts.
    let project = tempfile::Builder::new()
        .prefix("operant_rust_")
        .tempdir()
        .map_err(|e| format!("Failed to create project dir: {e}"))?;
    std::fs::create_dir_all(project.path().join("src"))
        .map_err(|e| format!("Failed to create src dir: {e}"))?;
    std::fs::write(project.path().join("src/main.rs"), code)
        .map_err(|e| format!("Failed to write main.rs: {e}"))?;

    let src = project.path().join("src/main.rs");
    let bin = project.path().join("main");

    let mut compile = tokio::process::Command::new("rustc");
    compile.kill_on_drop(true);
    #[cfg(unix)]
    compile.process_group(0);
    compile
        .arg(&src)
        .arg("--edition")
        .arg("2021")
        .arg("-o")
        .arg(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let outcome = run_capped(compile, timeout).await;
    if outcome.timed_out || outcome.exit_code != Some(0) {
        return Ok(result_json("rust", &outcome, 0, Some("compilation")));
    }

    let mut run_cmd = tokio::process::Command::new(&bin);
    run_cmd.kill_on_drop(true);
    #[cfg(unix)]
    run_cmd.process_group(0);
    run_cmd.stdout(Stdio::piped());
    run_cmd.stderr(Stdio::piped());

    let start = Instant::now();
    let outcome = run_capped(run_cmd, timeout).await;
    let runtime_ms = start.elapsed().as_millis() as u64;

    Ok(result_json("rust", &outcome, runtime_ms, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_code_execution_schema() {
        let schema = CodeExecutionTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "code_execution");
    }

    #[tokio::test]
    async fn test_code_execution_invalid_args() {
        let tool = CodeExecutionTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_code_execution_unsupported_language() {
        let tool = CodeExecutionTool;
        let result = tool
            .execute(
                json!({"code": "print('hi')", "language": "brainfuck"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_code_execution_python_happy_path() {
        let tool = CodeExecutionTool;
        let result = tool
            .execute(
                json!({"code": "print('hi')", "language": "python"}),
                ToolContext::default(),
            )
            .await;
        assert!(
            result.success,
            "error: {}",
            result.error.unwrap_or_default()
        );
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["stdout"], "hi\n");
        assert_eq!(v["success"], true);
        assert_eq!(v["timed_out"], false);
    }

    #[tokio::test]
    async fn test_code_execution_output_capped() {
        let tool = CodeExecutionTool;
        let result = tool
            .execute(
                json!({
                    "code": "import sys\nsys.stdout.write('x' * 200000)\nsys.stdout.flush()",
                    "language": "python",
                    "timeout": 10
                }),
                ToolContext::default(),
            )
            .await;
        assert!(
            result.success,
            "error: {}",
            result.error.unwrap_or_default()
        );
        let v: Value = serde_json::from_str(&result.content).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(
            stdout.len() <= MAX_STDOUT_BYTES + 256,
            "stdout {} bytes",
            stdout.len()
        );
        assert_eq!(v["stdout_truncated"], true);
    }

    #[tokio::test]
    async fn test_code_execution_runaway_output_times_out() {
        let tool = CodeExecutionTool;
        // Regression: the old implementation only wrapped each line-read, so a
        // child streaming output faster than the timeout per line ran forever
        // (this test would hang the suite). The new single deadline bounds it.
        let result = tool
            .execute(
                json!({
                    "code": "import time\nwhile True:\n    print('x' * 100)\n    time.sleep(0.01)",
                    "language": "python",
                    "timeout": 1
                }),
                ToolContext::default(),
            )
            .await;
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["timed_out"], true, "content: {}", result.content);
        assert_eq!(v["success"], false);
    }

    #[tokio::test]
    async fn test_code_execution_shell_timeout_kills_process_group() {
        let tool = CodeExecutionTool;
        // `sleep 30 & wait`: the direct child is sh, the grandchild is sleep.
        // kill_on_drop alone would orphan the sleep; the group kill must reap
        // both. If the group kill regressed, a `sleep 30` would keep the
        // pipes open and this test would hang until the outer test timeout.
        let result = tool
            .execute(
                json!({"code": "sleep 30 & wait", "language": "shell", "timeout": 1}),
                ToolContext::default(),
            )
            .await;
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["timed_out"], true, "content: {}", result.content);
        assert_eq!(v["success"], false);
    }

    #[tokio::test]
    async fn test_code_execution_rust_compile_error() {
        let tool = CodeExecutionTool;
        let result = tool
            .execute(
                json!({"code": "fn main( { }", "language": "rust", "timeout": 60}),
                ToolContext::default(),
            )
            .await;
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["stage"], "compilation");
        assert_eq!(v["success"], false);
    }

    /// Minimal in-memory `AsyncRead` for unit-testing `read_capped`.
    struct SliceReader<'a>(&'a [u8]);

    impl tokio::io::AsyncRead for SliceReader<'_> {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let n = self.0.len().min(buf.remaining());
            buf.put_slice(&self.0[..n]);
            self.get_mut().0 = &self.0[n..];
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_read_capped_truncates_head_tail() {
        let long = "abcdefghij".repeat(6_000); // 60 KB > 50 KB cap
        let capped = read_capped(SliceReader(long.as_bytes()), MAX_STDOUT_BYTES).await;
        assert!(capped.truncated);
        assert!(capped.text.contains("truncated"));
        assert!(
            capped.text.len() <= MAX_STDOUT_BYTES + 256,
            "capped {} bytes",
            capped.text.len()
        );
        assert!(capped.text.starts_with("abcdefghij"), "head lost");
        assert!(capped.text.ends_with("abcdefghij"), "tail lost");
    }

    #[tokio::test]
    async fn test_read_capped_under_cap_untouched() {
        let short = "hello\n".repeat(10);
        let capped = read_capped(SliceReader(short.as_bytes()), MAX_STDOUT_BYTES).await;
        assert!(!capped.truncated);
        assert_eq!(capped.text, short);
    }
}
