//! Checkpoint Tool - Filesystem Snapshots via an isolated shadow git store
//!
//! Provides transparent filesystem snapshots before file-mutating operations
//! and a `checkpoint` tool for explicit ensure/list/restore/diff.
//!
//! Hermes parity (`hermes-agent/tools/checkpoint_manager.py`): every snapshot
//! is committed to a private store under `~/.operant/checkpoints/store/<hash>`
//! using `GIT_DIR` + `GIT_WORK_TREE` + `GIT_INDEX_FILE`, so **no git state
//! ever leaks into the user's project repository**. Any directory can be
//! checkpointed — a project git repo is not required — and the user's git
//! identity is never consulted (snapshots use an internal identity).
//!
//! Checkpoints are opt-in via `[checkpoints] enabled = true` in config; when
//! disabled the manager is inert (the tool returns a clear error).

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Global checkpoint manager singleton
static CHECKPOINT_MANAGER: OnceLock<CheckpointManager> = OnceLock::new();

/// Get the global checkpoint manager
pub fn get_checkpoint_manager() -> &'static CheckpointManager {
    CHECKPOINT_MANAGER.get_or_init(CheckpointManager::new)
}

/// Checkpoint configuration
#[derive(Debug, Clone)]
pub struct CheckpointConfig {
    /// Base directory for checkpoint storage
    pub base_dir: PathBuf,
    /// Maximum snapshots per project
    pub max_snapshots: usize,
    /// Maximum total size in MB
    pub max_total_size_mb: usize,
    /// Maximum file size in MB to include in snapshots
    pub max_file_size_mb: usize,
    /// Whether checkpoints are enabled
    pub enabled: bool,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        let base_dir = dirs::home_dir()
            .map(|h| h.join(".operant").join("checkpoints"))
            .unwrap_or_else(|| PathBuf::from("/tmp/operant-checkpoints"));

        Self {
            base_dir,
            max_snapshots: 20,
            max_total_size_mb: 500,
            max_file_size_mb: 10,
            enabled: false, // Disabled by default, enable via config
        }
    }
}

/// Checkpoint metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Full commit hash
    pub hash: String,
    /// Short commit hash
    pub short_hash: String,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
    /// Commit message/reason
    pub reason: String,
    /// Number of files changed
    pub files_changed: usize,
    /// Number of insertions
    pub insertions: usize,
    /// Number of deletions
    pub deletions: usize,
}

/// Checkpoint manager for creating and managing filesystem snapshots
pub struct CheckpointManager {
    /// Interior mutability so the process-global manager can be configured
    /// from `&self` (it lives in a `OnceLock`).
    config: std::sync::Mutex<CheckpointConfig>,
    checkpointed_dirs: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl Default for CheckpointManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CheckpointManager {
    /// Create a new checkpoint manager
    pub fn new() -> Self {
        Self {
            config: std::sync::Mutex::new(CheckpointConfig::default()),
            checkpointed_dirs: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Configure the checkpoint manager
    pub fn configure(&self, config: CheckpointConfig) {
        if let Ok(mut guard) = self.config.lock() {
            *guard = config;
        }
    }

    /// Enable or disable checkpoints
    pub fn set_enabled(&self, enabled: bool) {
        if let Ok(mut guard) = self.config.lock() {
            guard.enabled = enabled;
        }
    }

    /// Check if checkpoints are enabled
    pub fn is_enabled(&self) -> bool {
        self.config.lock().map(|g| g.enabled).unwrap_or(false)
    }

    /// Reset per-turn deduplication
    pub fn new_turn(&self) {
        if let Ok(mut dirs) = self.checkpointed_dirs.lock() {
            dirs.clear();
        }
    }

    /// Take a checkpoint of the given directory
    pub fn ensure_checkpoint(&self, working_dir: &str, reason: &str) -> bool {
        let config_guard = match self.config.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if !config_guard.enabled {
            return false;
        }
        drop(config_guard);

        // Check git availability
        if Command::new("git").arg("--version").output().is_err() {
            debug!("Checkpoints disabled: git not found");
            return false;
        }

        // Skip root and home directories
        let abs_dir = PathBuf::from(working_dir);
        if abs_dir == Path::new("/") || abs_dir == dirs::home_dir().unwrap_or_default() {
            debug!("Checkpoint skipped: directory too broad");
            return false;
        }

        // Check if already checkpointed this turn
        let is_new = {
            let mut dirs = match self.checkpointed_dirs.lock() {
                Ok(d) => d,
                Err(_) => return false,
            };
            if dirs.contains(working_dir) {
                return false;
            }
            dirs.insert(working_dir.to_string());
            true
        };

        if !is_new {
            return false;
        }

        // Take the checkpoint
        self.take_checkpoint(working_dir, reason)
    }

    /// Internal: take a checkpoint into the shadow store.
    ///
    /// Hermes parity (`checkpoint_manager.py`): snapshots are committed to an
    /// isolated git store via `GIT_DIR` + `GIT_WORK_TREE` + `GIT_INDEX_FILE`,
    /// so no git state leaks into the user's project directory (the old
    /// implementation ran `git add`/`git commit` inside the user's repo — see
    /// BUGS.md R12-2). Works in any directory — a project git repo is not
    /// required, and the user's git identity is never consulted.
    fn take_checkpoint(&self, working_dir: &str, reason: &str) -> bool {
        let workdir = PathBuf::from(working_dir);
        if !workdir.is_dir() {
            return false;
        }

        let base_dir = self
            .config
            .lock()
            .map(|g| g.base_dir.clone())
            .unwrap_or_default();
        let store = base_dir.join("store").join(store_name(working_dir));
        if let Err(e) = ensure_store(&store) {
            warn!("{e}");
            return false;
        }

        let env = git_shadow_env(&store, &workdir, &store.join("index"));
        let env_iter = env.iter().map(|(k, v)| (k.as_str(), v.as_str()));

        // Stage all files into the shadow index (user repo untouched)
        let add_result = Command::new("git")
            .args(["add", "-A"])
            .envs(env_iter)
            .output();
        if !matches!(add_result, Ok(ref output) if output.status.success()) {
            debug!("Checkpoint git add failed for {}", working_dir);
            return false;
        }

        // Nothing staged → no changes
        let diff_result = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output();
        if matches!(diff_result, Ok(ref output) if output.status.success()) {
            debug!("Checkpoint skipped: no changes in {}", working_dir);
            return false;
        }

        // Commit with an internal identity — the shadow store is private, so
        // the user's git identity is neither required nor consulted.
        let commit_result = Command::new("git")
            .args(["commit", "-q", "-m", reason])
            .envs(git_identity_env())
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output();

        match commit_result {
            Ok(ref output) if output.status.success() => {
                // Enforce max_snapshots — keep the newest N commits by moving
                // the store's branch ref (never touches the index or the user's
                // worktree; hermes prune parity).
                self.prune_old_checkpoints(&store, &env);
                info!("Checkpoint taken in {}: {}", working_dir, reason);
                true
            }
            Ok(ref output) => {
                debug!(
                    "Git commit failed for {}: {}",
                    working_dir,
                    String::from_utf8_lossy(&output.stderr)
                );
                false
            }
            Err(e) => {
                debug!("Git commit failed for {}: {}", working_dir, e);
                false
            }
        }
    }

    /// List available checkpoints for a directory
    pub fn list_checkpoints(&self, working_dir: &str) -> Vec<Checkpoint> {
        let workdir = PathBuf::from(working_dir);
        let store = self.store_path(working_dir);
        if !store.join("HEAD").exists() {
            return Vec::new();
        }
        let env = git_shadow_env(&store, &workdir, &store.join("index"));

        // Get commit log from the shadow store
        let log_output = Command::new("git")
            .args(["log", "--format=%H|%h|%aI|%s", "-n", "20"])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output();

        match log_output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split('|').collect();
                        if parts.len() >= 4 {
                            Some(Checkpoint {
                                hash: parts[0].to_string(),
                                short_hash: parts[1].to_string(),
                                timestamp: parts[2].to_string(),
                                reason: parts[3].to_string(),
                                files_changed: 0,
                                insertions: 0,
                                deletions: 0,
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Restore files to a checkpoint state (writes into the working directory)
    pub fn restore(
        &self,
        working_dir: &str,
        commit_hash: &str,
        file_path: Option<&str>,
    ) -> Result<String> {
        let workdir = PathBuf::from(working_dir);
        let store = self.store_path(working_dir);
        if !store.join("HEAD").exists() {
            return Err(Error::Agent(format!(
                "No checkpoint store for {}",
                working_dir
            )));
        }
        let env = git_shadow_env(&store, &workdir, &store.join("index"));

        let target = file_path.unwrap_or(".");

        let output = Command::new("git")
            .args(["checkout", commit_hash, "--", target])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output()
            .map_err(|e| Error::Agent(format!("Failed to restore: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("Restore failed: {}", stderr)));
        }

        Ok(format!(
            "Restored to checkpoint {} in {}",
            &commit_hash[..8.min(commit_hash.len())],
            working_dir
        ))
    }

    /// Show diff between a checkpoint and the current working tree
    pub fn diff(&self, working_dir: &str, commit_hash: &str) -> Result<String> {
        let workdir = PathBuf::from(working_dir);
        let store = self.store_path(working_dir);
        if !store.join("HEAD").exists() {
            return Err(Error::Agent(format!(
                "No checkpoint store for {}",
                working_dir
            )));
        }
        let env = git_shadow_env(&store, &workdir, &store.join("index"));

        let output = Command::new("git")
            .args(["diff", commit_hash, "--", "."])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output()
            .map_err(|e| Error::Agent(format!("Failed to diff: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("Diff failed: {}", stderr)));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl CheckpointManager {
    /// Stable shadow-store path for a working directory.
    fn store_path(&self, working_dir: &str) -> PathBuf {
        self.config
            .lock()
            .map(|g| g.base_dir.join("store").join(store_name(working_dir)))
            .unwrap_or_default()
    }

    /// Drop the oldest shadow-store commits so at most `max_snapshots` remain.
    /// Uses `git update-ref`, which is safe on a bare store and never touches
    /// the user's index or worktree.
    fn prune_old_checkpoints(&self, store: &Path, env: &[(String, String)]) {
        let max = self.config.lock().map(|g| g.max_snapshots).unwrap_or(20);
        if max == 0 {
            return;
        }
        let env_iter = env.iter().map(|(k, v)| (k.as_str(), v.as_str()));
        let count = match Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .envs(env_iter)
            .output()
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => return,
        };
        let Ok(n) = count.parse::<usize>() else {
            return;
        };
        if n <= max {
            return;
        }
        let excess = n - max;
        let keep = match Command::new("git")
            .args(["rev-list", "-n", "1", &format!("HEAD~{}", excess)])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output()
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => return,
        };
        if keep.is_empty() {
            return;
        }
        let Some(branch) = current_branch(store) else {
            return;
        };
        // NOTE: `current_branch` must return the full `refs/heads/<name>` form;
        // a bare `master` would create a stray top-level loose ref instead of
        // moving the branch (covered by test_max_snapshots_caps_commits).
        let pruned = Command::new("git")
            .args(["update-ref", &branch, &keep])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output();
        match pruned {
            Ok(o) if o.status.success() => {
                debug!(
                    "Pruned old checkpoints for {} (kept newest {})",
                    store.display(),
                    max
                );
            }
            other => {
                warn!(
                    "Failed to prune old checkpoints for {} (excess {}): {:?}",
                    store.display(),
                    excess,
                    other.as_ref().map(|o| &o.stderr)
                );
                return;
            }
        }
        // update-ref only hides the dropped commits; reclaim their objects so
        // the store doesn't grow unbounded even with max_snapshots capped.
        // gc must run with the same GIT_DIR env so it operates on the store,
        // never the user's repo.
        let gc = Command::new("git")
            .args(["gc", "--prune=now", "--quiet"])
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output();
        if let Ok(o) = gc
            && !o.status.success()
        {
            warn!(
                "git gc on checkpoint store {} failed (non-fatal): {}",
                store.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
    }
}

/// Read the store's current branch from its `HEAD` symref (`ref: refs/heads/..`).
fn current_branch(store: &Path) -> Option<String> {
    let head = std::fs::read_to_string(store.join("HEAD")).ok()?;
    head.strip_prefix("ref: ")
        .map(str::trim)
        .map(str::to_string)
}

/// Stable 16-hex store name for a working directory.
fn store_name(working_dir: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(working_dir.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

/// Shadow-store environment (hermes parity): git operations run against the
/// private store with the user's directory as work tree, so no refs, index,
/// or objects ever touch the user's repository.
fn git_shadow_env(store: &Path, workdir: &Path, index_file: &Path) -> Vec<(String, String)> {
    vec![
        ("GIT_DIR".to_string(), store.to_string_lossy().to_string()),
        (
            "GIT_WORK_TREE".to_string(),
            workdir.to_string_lossy().to_string(),
        ),
        (
            "GIT_INDEX_FILE".to_string(),
            index_file.to_string_lossy().to_string(),
        ),
        ("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string()),
        ("GIT_CONFIG_SYSTEM".to_string(), "/dev/null".to_string()),
        ("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string()),
    ]
}

/// Internal identity for shadow-store commits (user git config not required).
fn git_identity_env() -> Vec<(String, String)> {
    vec![
        ("GIT_AUTHOR_NAME".to_string(), "operant".to_string()),
        ("GIT_AUTHOR_EMAIL".to_string(), "operant@local".to_string()),
        ("GIT_COMMITTER_NAME".to_string(), "operant".to_string()),
        (
            "GIT_COMMITTER_EMAIL".to_string(),
            "operant@local".to_string(),
        ),
    ]
}

/// Ensure the shadow store (bare repo) exists for a working directory.
fn ensure_store(store: &Path) -> std::result::Result<(), String> {
    if store.join("HEAD").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(store)
        .map_err(|e| format!("Failed to create checkpoint store: {}", e))?;
    let out = Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(store)
        .output()
        .map_err(|e| format!("git init failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git init failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    // Default excludes for the shadow store: never snapshot the user's own
    // `.git` (would otherwise be added as a gitlink) or common heavy dirs.
    let exclude = store.join("info").join("exclude");
    if !exclude.exists() {
        let _ = std::fs::create_dir_all(store.join("info"));
        let _ = std::fs::write(
            &exclude,
            ".git\nnode_modules/\ntarget/\n__pycache__/\n.venv/\nvenv/\n",
        );
    }
    Ok(())
}

/// Checkpoint tool - provides checkpoint management as a callable tool
pub struct CheckpointTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct CheckpointArgs {
    /// Action: 'ensure' (create checkpoint), 'list' (show checkpoints), 'restore' (revert), 'diff' (show changes)
    action: String,
    /// Working directory to checkpoint
    working_dir: String,
    /// Commit hash for restore/diff (short or full)
    commit_hash: Option<String>,
    /// Optional specific file to restore (default: entire directory)
    file_path: Option<String>,
    /// Reason for checkpoint (for 'ensure' action)
    reason: Option<String>,
    /// Limit for list action
    limit: Option<usize>,
}

impl Default for CheckpointTool {
    fn default() -> Self {
        Self::new()
    }
}

impl CheckpointTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OperantTool for CheckpointTool {
    fn name(&self) -> &str {
        "checkpoint"
    }

    fn description(&self) -> &str {
        "Create, list, and restore filesystem checkpoints using git. \
         Use before file-mutating operations to enable rollback. \
         Call 'ensure' before making changes, 'list' to see available checkpoints, \
         'restore' to revert to a previous state."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CheckpointArgs>(
            "checkpoint",
            "Filesystem checkpoint management - create snapshots before changes, list available checkpoints, restore to previous states",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        let working_dir = args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let manager = get_checkpoint_manager();

        match action {
            "ensure" => {
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("manual checkpoint");

                let success = manager.ensure_checkpoint(working_dir, reason);

                if success {
                    ToolResult::success(
                        "checkpoint_ensure",
                        serde_json::json!({
                            "success": true,
                            "message": format!("Checkpoint created in {}", working_dir)
                        }),
                    )
                } else {
                    ToolResult::error(
                        "checkpoint_ensure",
                        "Failed to create checkpoint (checkpoints may be disabled in config — set [checkpoints] enabled = true — or there were no changes to snapshot)",
                    )
                }
            }
            "list" => {
                let checkpoints = manager.list_checkpoints(working_dir);
                ToolResult::success(
                    "checkpoint_list",
                    serde_json::json!({
                        "success": true,
                        "working_dir": working_dir,
                        "checkpoints": checkpoints,
                        "count": checkpoints.len()
                    }),
                )
            }
            "restore" => {
                let commit_hash = args
                    .get("commit_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let file_path = args.get("file_path").and_then(|v| v.as_str());

                match manager.restore(working_dir, commit_hash, file_path) {
                    Ok(msg) => ToolResult::success(
                        "checkpoint_restore",
                        serde_json::json!({
                            "success": true,
                            "message": msg
                        }),
                    ),
                    Err(e) => ToolResult::error("checkpoint_restore", e.to_string()),
                }
            }
            "diff" => {
                let commit_hash = args
                    .get("commit_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                match manager.diff(working_dir, commit_hash) {
                    Ok(diff) => ToolResult::success(
                        "checkpoint_diff",
                        serde_json::json!({
                            "success": true,
                            "working_dir": working_dir,
                            "commit_hash": commit_hash,
                            "diff": diff
                        }),
                    ),
                    Err(e) => ToolResult::error("checkpoint_diff", e.to_string()),
                }
            }
            _ => ToolResult::error(
                "checkpoint",
                format!(
                    "Unknown action: {}. Use: ensure, list, restore, or diff",
                    action
                ),
            ),
        }
    }
}

/// Register the checkpoint tool
pub fn register_checkpoint_tool() -> impl FnOnce() -> Result<()> {
    || {
        let _tool = CheckpointTool::new();
        // Registration would happen here via the registry
        // This is a placeholder for the registration function
        info!("Checkpoint tool loaded");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_name_and_description() {
        let tool = CheckpointTool::new();
        assert_eq!(tool.name(), "checkpoint");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_checkpoint_schema_has_action_and_working_dir() {
        let schema = CheckpointTool::new().schema();
        assert_eq!(schema.name, "checkpoint");
        let schema_json = serde_json::to_value(&schema).unwrap();
        if let Some(props) = schema_json["inputSchema"]["properties"].as_object() {
            assert!(
                props.contains_key("action"),
                "Schema should have 'action' property"
            );
            assert!(
                props.contains_key("workingDir"),
                "Schema should have 'workingDir' property"
            );
            assert!(
                props.contains_key("commitHash"),
                "Schema should have 'commitHash' property"
            );
        }
    }

    #[tokio::test]
    async fn test_checkpoint_execute_unknown_action() {
        let tool = CheckpointTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "action": "invalid", "working_dir": "." }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("Unknown action") || err.contains("invalid"));
    }

    #[tokio::test]
    async fn test_checkpoint_list_with_defaults() {
        let tool = CheckpointTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "action": "list" }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let content: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(content["success"], true);
    }

    #[tokio::test]
    async fn test_checkpoint_default_action_is_list() {
        let tool = CheckpointTool::new();
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_checkpoint_restore_missing_hash() {
        let tool = CheckpointTool::new();
        let result = tool
            .execute(
                serde_json::json!({ "action": "restore", "working_dir": "." }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[test]
    fn test_store_name_is_stable_and_distinct() {
        let a = store_name("/tmp/proj");
        let b = store_name("/tmp/proj");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert_ne!(store_name("/tmp/proj"), store_name("/tmp/proj2"));
    }

    #[test]
    fn test_checkpoints_disabled_by_default() {
        let mgr = CheckpointManager::new();
        assert!(!mgr.is_enabled());
        assert!(!mgr.ensure_checkpoint(".", "test"));
    }

    #[test]
    fn test_shadow_checkpoint_roundtrip() {
        use std::process::Command as StdCommand;
        // git must be available; otherwise skip
        if StdCommand::new("git").arg("--version").output().is_err() {
            return;
        }

        let tmp = std::env::temp_dir().join(format!("operant_ckpt_{}", uuid::Uuid::new_v4()));
        let base =
            std::env::temp_dir().join(format!("operant_ckpt_store_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "hello").unwrap();

        let mgr = CheckpointManager::new();
        mgr.configure(CheckpointConfig {
            base_dir: base.clone(),
            enabled: true,
            ..Default::default()
        });
        let dir = tmp.to_str().unwrap();

        // First snapshot captures a.txt
        assert!(mgr.ensure_checkpoint(dir, "one"));
        // No git state leaks into the working directory
        assert!(!tmp.join(".git").exists());
        assert_eq!(mgr.list_checkpoints(dir).len(), 1);

        // Mutate, second snapshot, then restore the first
        std::fs::write(tmp.join("a.txt"), "changed").unwrap();
        mgr.new_turn(); // clear per-turn dedup
        assert!(mgr.ensure_checkpoint(dir, "two"));
        let cps = mgr.list_checkpoints(dir);
        assert_eq!(cps.len(), 2);

        mgr.restore(dir, &cps[1].hash, Some("a.txt")).unwrap();
        assert_eq!(std::fs::read_to_string(tmp.join("a.txt")).unwrap(), "hello");

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_shadow_checkpoint_respects_max_snapshots() {
        use std::process::Command as StdCommand;
        if StdCommand::new("git").arg("--version").output().is_err() {
            return;
        }

        let tmp = std::env::temp_dir().join(format!("operant_ckpt_{}", uuid::Uuid::new_v4()));
        let base =
            std::env::temp_dir().join(format!("operant_ckpt_store_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "v0").unwrap();

        let mgr = CheckpointManager::new();
        mgr.configure(CheckpointConfig {
            base_dir: base.clone(),
            enabled: true,
            max_snapshots: 2,
            ..Default::default()
        });
        let dir = tmp.to_str().unwrap();

        assert!(mgr.ensure_checkpoint(dir, "one"));
        std::fs::write(tmp.join("a.txt"), "v1").unwrap();
        mgr.new_turn();
        assert!(mgr.ensure_checkpoint(dir, "two"));
        std::fs::write(tmp.join("a.txt"), "v2").unwrap();
        mgr.new_turn();
        assert!(mgr.ensure_checkpoint(dir, "three"));

        // Only the newest `max_snapshots` remain
        assert!(
            mgr.list_checkpoints(dir).len() <= 2,
            "expected <=2, got {}",
            mgr.list_checkpoints(dir).len()
        );
        // The newest checkpoint is still restorable
        let cps = mgr.list_checkpoints(dir);
        if let Some(latest) = cps.first() {
            mgr.restore(dir, &latest.hash, Some("a.txt")).unwrap();
        }

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&base);
    }
}
