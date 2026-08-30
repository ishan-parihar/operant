//! Plan 008 — `working_diff` tool (hermes `tools/working_diff.py` parity).
//!
//! Lets the agent see the current uncommitted diff of a git repository. Two
//! modes:
//! - `working` (default): `git diff` of unstaged changes + untracked list
//! - `staged`: `git diff --cached` of staged changes
//!
//! Sanity caps mirror hermes's `_MAX_UNTRACKED_FILES=50`; diff output is
//! truncated to ~100KB to keep the tool result bounded.

use crate::tools::{OperantTool, ToolContext, ToolResult};
use anyhow::Context as _;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

const MAX_UNTRACKED_FILES: usize = 50;
const MAX_DIFF_BYTES: usize = 100 * 1024;

pub struct WorkingDiffTool;

impl Default for WorkingDiffTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl OperantTool for WorkingDiffTool {
    fn name(&self) -> &str {
        "working_diff"
    }

    fn description(&self) -> &str {
        "Show the current uncommitted git diff of a repository. Returns either the \
         working tree diff (`mode=\"working\"`, default) or the staged diff \
         (`mode=\"staged\"`). Includes a capped untracked-files list."
    }

    fn schema(&self) -> crate::schema::ToolSchema {
        crate::schema::ToolSchema::new(
            self.name(),
            self.description(),
            json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["working", "staged"],
                        "description": "Which diff to show. Default: \"working\"."
                    },
                    "path": {
                        "type": "string",
                        "description": "Repository root (defaults to the agent's cwd)."
                    }
                }
            }),
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("working");
        let path = args.get("path").and_then(|v| v.as_str());

        let cwd = match path {
            Some(p) => Path::new(p).to_path_buf(),
            None => match std::env::current_dir() {
                Ok(c) => c,
                Err(e) => {
                    return ToolResult::error(
                        String::new(),
                        format!("Cannot resolve cwd: {e}"),
                    )
                }
            },
        };

        // Confirm cwd is a git repository.
        if !is_git_repo(&cwd) {
            return ToolResult::error(
                String::new(),
                format!(
                    "Not a git repository: {}. Pass `path` to a git working tree.",
                    cwd.display()
                ),
            );
        }

        let mut body = String::new();

        // 1. The diff itself.
        let diff_args: &[&str] = match mode {
            "staged" => &["diff", "--cached", "--no-color"],
            _ => &["diff", "--no-color"],
        };
        let diff_output = run_git(&cwd, diff_args);
        match diff_output {
            Ok(text) if text.is_empty() => body.push_str("(no changes)\n"),
            Ok(text) => {
                body.push_str(&truncate(&text, MAX_DIFF_BYTES, "[diff truncated]\n"));
            }
            Err(e) => {
                return ToolResult::error(String::new(), format!("git diff failed: {e}"))
            }
        }

        // 2. Untracked files (only for working mode — staged mode doesn't include them).
        if mode != "staged" {
            match untracked_list(&cwd) {
                Ok(text) => body.push_str(&text),
                Err(e) => {
                    body.push_str(&format!("\n[untracked list error: {e}]\n"));
                }
            }
        }

        ToolResult::success(String::new(), body)
    }
}

fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_git(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("spawning git {args:?}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} exited {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn untracked_list(cwd: &Path) -> anyhow::Result<String> {
    let text = run_git(cwd, &["status", "--porcelain"])?;
    let mut untracked: Vec<String> = Vec::new();
    for line in text.lines() {
        // porcelain v1: "??" prefix for untracked entries.
        if line.starts_with("??") {
            let path = line.get(3..).unwrap_or("").to_string();
            if !path.is_empty() {
                untracked.push(path);
            }
        }
    }
    if untracked.is_empty() {
        return Ok(String::new());
    }
    let mut body = String::from("\nUntracked files (not in diff):\n");
    let shown = untracked.len().min(MAX_UNTRACKED_FILES);
    for p in &untracked[..shown] {
        body.push_str("  ");
        body.push_str(p);
        body.push('\n');
    }
    if untracked.len() > MAX_UNTRACKED_FILES {
        body.push_str(&format!(
            "  ... ({} more untracked files not shown)\n",
            untracked.len() - MAX_UNTRACKED_FILES
        ));
    }
    Ok(body)
}

fn truncate(text: &str, max_bytes: usize, marker: &str) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut out = String::with_capacity(max_bytes + marker.len());
    out.push_str(&text[..max_bytes]);
    out.push_str(marker);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fresh temp git repo with one initial commit. Returns (TempDir, repo_root).
    fn init_repo() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .expect("git")
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "hello\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        (dir, root)
    }

    fn run_tool(args: Value) -> ToolResult {
        let tool = WorkingDiffTool;
        // Tests block on a runtime; execute() is async but the body is sync-friendly.
        futures::executor::block_on(tool.execute(args, ToolContext::default()))
    }

    fn body_of(r: &ToolResult) -> String {
        r.content.clone()
    }

    fn error_of(r: &ToolResult) -> String {
        r.error.clone().unwrap_or_default()
    }

    #[test]
    fn working_diff_shows_modified_lines() {
        let (_tmp, root) = init_repo();
        std::fs::write(root.join("a.txt"), "hello\nworld\n").unwrap();
        let r = run_tool(json!({ "path": root.to_str().unwrap() }));
        assert!(r.success);
        let body = body_of(&r);
        assert!(body.contains("world"), "diff must contain new line: {body}");
    }

    #[test]
    fn working_diff_staged_vs_working() {
        let (_tmp, root) = init_repo();
        std::fs::write(root.join("a.txt"), "staged\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&root)
            .output()
            .unwrap();
        let staged = run_tool(json!({ "mode": "staged", "path": root.to_str().unwrap() }));
        assert!(staged.success);
        assert!(body_of(&staged).contains("staged"));

        // Now unstage and change again — staged should be empty.
        Command::new("git")
            .args(["reset", "-q"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::fs::write(root.join("a.txt"), "working\n").unwrap();
        let working = run_tool(json!({ "mode": "staged", "path": root.to_str().unwrap() }));
        assert!(working.success);
        assert!(body_of(&working).contains("no changes"));
    }

    #[test]
    fn untracked_cap_50() {
        let (_tmp, root) = init_repo();
        for i in 0..60 {
            std::fs::write(root.join(format!("f{i:03}.txt")), "x").unwrap();
        }
        let r = run_tool(json!({ "path": root.to_str().unwrap() }));
        assert!(r.success);
        let body = body_of(&r);
        assert!(body.contains("10 more untracked files not shown"));
    }

    #[test]
    fn not_a_git_repo_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let r = run_tool(json!({ "path": tmp.path().to_str().unwrap() }));
        assert!(!r.success);
        assert!(error_of(&r).contains("Not a git repository"));
    }

    #[test]
    fn diff_output_truncated() {
        let (_tmp, root) = init_repo();
        // Modify the tracked file with a much larger body to force the
        // 100KB diff cap to fire.
        let big = "x".repeat(200 * 1024);
        std::fs::write(root.join("a.txt"), &big).unwrap();
        let r = run_tool(json!({ "path": root.to_str().unwrap() }));
        assert!(r.success);
        let body = body_of(&r);
        assert!(body.contains("[diff truncated]"), "body must be capped: len={}", body.len());
    }
}
