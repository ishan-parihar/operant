//! Checkpoint Tool - Filesystem Snapshots via Git
//!
//! Provides transparent filesystem snapshots before file-mutating operations.
//! This tool is infrastructure that can be called by the agent to manage checkpoints.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

/// Global checkpoint manager singleton
static CHECKPOINT_MANAGER: OnceLock<CheckpointManager> = OnceLock::new();

/// Get the global checkpoint manager
pub fn get_checkpoint_manager() -> &'static CheckpointManager {
    CHECKPOINT_MANAGER.get_or_init(|| CheckpointManager::new())
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
            .map(|h| h.join(".hermes").join("checkpoints"))
            .unwrap_or_else(|| PathBuf::from("/tmp/hermes-checkpoints"));

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
    config: CheckpointConfig,
    checkpointed_dirs: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl CheckpointManager {
    /// Create a new checkpoint manager
    pub fn new() -> Self {
        Self {
            config: CheckpointConfig::default(),
            checkpointed_dirs: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Configure the checkpoint manager
    pub fn configure(&mut self, config: CheckpointConfig) {
        self.config = config;
    }

    /// Enable or disable checkpoints
    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
    }

    /// Check if checkpoints are enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Reset per-turn deduplication
    pub fn new_turn(&self) {
        if let Ok(mut dirs) = self.checkpointed_dirs.lock() {
            dirs.clear();
        }
    }

    /// Take a checkpoint of the given directory
    pub fn ensure_checkpoint(&self, working_dir: &str) -> bool {
        if !self.config.enabled {
            return false;
        }

        // Check git availability
        if Command::new("git").arg("--version").output().is_err() {
            debug!("Checkpoints disabled: git not found");
            return false;
        }

        // Skip root and home directories
        let abs_dir = PathBuf::from(working_dir);
        if abs_dir == PathBuf::from("/") || abs_dir == dirs::home_dir().unwrap_or_default() {
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
        self.take_checkpoint(working_dir, "auto checkpoint")
    }

    /// Internal: take a checkpoint
    fn take_checkpoint(&self, working_dir: &str, reason: &str) -> bool {
        let store_dir = self.config.base_dir.join("store");
        
        // Create store directory if needed
        if let Err(e) = std::fs::create_dir_all(&store_dir) {
            warn!("Failed to create checkpoint store: {}", e);
            return false;
        }

        // Run git commands to create checkpoint
        // Note: This is a simplified version. Full implementation would use
        // GIT_DIR, GIT_WORK_TREE, and GIT_INDEX_FILE environment variables
        // to isolate the checkpoint repository from the main project.

        let working_path = PathBuf::from(working_dir);
        if !working_path.exists() {
            return false;
        }

        // Check if directory has changes
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&working_path)
            .output();

        match status {
            Ok(output) if output.stdout.is_empty() => {
                debug!("Checkpoint skipped: no changes in {}", working_dir);
                return false;
            }
            Err(e) => {
                warn!("Git status failed: {}", e);
                return false;
            }
            _ => {}
        }

        // Add all files to staging
        let add_result = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&working_path)
            .output();

        if add_result.is_err() || !add_result.unwrap().status.success() {
            debug!("Git add failed, skipping checkpoint");
            return false;
        }

        // Check if there are staged changes
        let diff_result = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(&working_path)
            .output();

        match diff_result {
            Ok(output) if output.status.success() => {
                debug!("Checkpoint skipped: nothing to commit");
                return false;
            }
            Err(_) => {}
            _ => {}
        }

        // Create commit
        let commit_result = Command::new("git")
            .args(["commit", "-m", reason])
            .current_dir(&working_path)
            .output();

        if commit_result.is_err() || !commit_result.unwrap().status.success() {
            debug!("Git commit failed, skipping checkpoint");
            return false;
        }

        info!("Checkpoint taken in {}: {}", working_dir, reason);
        true
    }

    /// List available checkpoints for a directory
    pub fn list_checkpoints(&self, working_dir: &str) -> Vec<Checkpoint> {
        let working_path = PathBuf::from(working_dir);
        
        // Get commit log
        let log_output = Command::new("git")
            .args(["log", "--format=%H|%h|%aI|%s", "-n", "20"])
            .current_dir(&working_path)
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

    /// Restore files to a checkpoint state
    pub fn restore(&self, working_dir: &str, commit_hash: &str, file_path: Option<&str>) -> Result<String> {
        let working_path = PathBuf::from(working_dir);

        let target = file_path.unwrap_or(".");

        let output = Command::new("git")
            .args(["checkout", commit_hash, "--", target])
            .current_dir(&working_path)
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

    /// Show diff between a checkpoint and current state
    pub fn diff(&self, working_dir: &str, commit_hash: &str) -> Result<String> {
        let working_path = PathBuf::from(working_dir);

        let output = Command::new("git")
            .args(["diff", commit_hash, "--", "."])
            .current_dir(&working_path)
            .output()
            .map_err(|e| Error::Agent(format!("Failed to diff: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("Diff failed: {}", stderr)));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Checkpoint tool - provides checkpoint management as a callable tool
pub struct CheckpointTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
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

impl CheckpointTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl HermesTool for CheckpointTool {
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
                
                let success = manager.ensure_checkpoint(working_dir);
                
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
                        "Failed to create checkpoint (may be disabled or no changes)",
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
                
                let file_path = args
                    .get("file_path")
                    .and_then(|v| v.as_str());

                match manager.restore(working_dir, commit_hash, file_path) {
                    Ok(msg) => ToolResult::success(
                        "checkpoint_restore",
                        serde_json::json!({
                            "success": true,
                            "message": msg
                        }),
                    ),
                    Err(e) => ToolResult::error(
                        "checkpoint_restore",
                        e.to_string(),
                    ),
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
                    Err(e) => ToolResult::error(
                        "checkpoint_diff",
                        e.to_string(),
                    ),
                }
            }
            _ => ToolResult::error(
                "checkpoint",
                format!("Unknown action: {}. Use: ensure, list, restore, or diff", action),
            ),
        }
    }
}

/// Register the checkpoint tool
pub fn register_checkpoint_tool() -> impl FnOnce() -> Result<()> {
    || {
        let tool = CheckpointTool::new();
        // Registration would happen here via the registry
        // This is a placeholder for the registration function
        info!("Checkpoint tool loaded");
        Ok(())
    }
}