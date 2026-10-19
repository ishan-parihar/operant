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
use std::path::{Component, Path, PathBuf};
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
    /// Why the last `ensure_checkpoint` refused, when it refused for a
    /// reason the caller can act on (a size-bound skip). `None` after a
    /// successful checkpoint.
    last_skip_reason: std::sync::Mutex<Option<String>>,
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
            last_skip_reason: std::sync::Mutex::new(None),
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

    /// Record (or clear) the reason the last checkpoint was skipped.
    fn set_skip_reason(&self, reason: Option<String>) {
        if let Ok(mut slot) = self.last_skip_reason.lock() {
            *slot = reason;
        }
    }

    /// Delete the shadow store for a working directory and report how many
    /// bytes it held.
    ///
    /// The store is a bare git repo on disk; the DATABASE rows are a
    /// separate thing. `operant checkpoints prune` and `clear` remove the
    /// rows, and the bytes on disk stay — a store that had grown to
    /// 1.5 GB of packed objects reported "pruned 3 checkpoints" and
    /// reclaimed nothing. This is the path that reclaims them.
    ///
    /// The store holds SNAPSHOTS of the working directory, so deleting it
    /// removes a rollback history and nothing else. The caller decides.
    pub fn clear_store(&self, working_dir: &str) -> crate::Result<u64> {
        let store = self.store_path(working_dir);
        if !store.exists() {
            return Ok(0);
        }
        let bytes = dir_size(&store);
        std::fs::remove_dir_all(&store).map_err(|e| {
            crate::error::Error::Agent(format!("Failed to remove the checkpoint store: {e}"))
        })?;
        Ok(bytes)
    }

    /// Why the last `ensure_checkpoint` call refused, if it refused for a
    /// reason the caller can act on.
    ///
    /// The tool's error message used to name only two causes
    /// ("checkpoints may be disabled" and "there were no changes"), and
    /// neither was the size-bound skip that iter-992 added. An operator
    /// whose tree was skipped for SIZE was told to enable checkpoints that
    /// were already enabled. The reason is now reported.
    pub fn last_skip_reason(&self) -> Option<String> {
        self.last_skip_reason.lock().ok().and_then(|r| r.clone())
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

        // hermes `_MAX_FILES` parity: skip pathological directories instead of
        // hanging `git add -A` on the shadow store.
        if dir_file_count(&workdir, MAX_CHECKPOINT_FILES) > MAX_CHECKPOINT_FILES {
            debug!(
                "Checkpoint skipped: {} exceeds {} files",
                working_dir, MAX_CHECKPOINT_FILES
            );
            return false;
        }

        // The SIZE bounds. `max_file_size_mb` and `max_total_size_mb` were
        // declared on the config and never read, so nothing bounded the
        // bytes a snapshot could take: the count bound above is a COUNT,
        // and 50,000 files of any size passes it. Measured on this box:
        // the shadow store for one directory had reached 1.42 GiB of
        // packed objects — 3x over the 500 MB the config documented.
        //
        // A checkpoint is a safety net for a mutation, not a backup. A
        // tree this size is not something a rollback can restore anyway,
        // so the honest behaviour is to skip the checkpoint and say so.
        let (max_total_bytes, max_file_bytes) = self
            .config
            .lock()
            .map(|g| {
                (
                    (g.max_total_size_mb as u64).saturating_mul(1024 * 1024),
                    (g.max_file_size_mb as u64).saturating_mul(1024 * 1024),
                )
            })
            .unwrap_or((500 * 1024 * 1024, 10 * 1024 * 1024));
        if max_total_bytes > 0 || max_file_bytes > 0 {
            let size = tree_size(&workdir);
            if max_file_bytes > 0 && size.largest_file_bytes > max_file_bytes {
                let msg = format!(
                    "the tree has a {} MiB file, over the {} MiB per-file checkpoint limit \
                     ([checkpoints] max_file_size_mb)",
                    size.largest_file_bytes / (1024 * 1024),
                    max_file_bytes / (1024 * 1024)
                );
                debug!("Checkpoint skipped: {msg}");
                self.set_skip_reason(Some(msg));
                return false;
            }
            if max_total_bytes > 0 && size.total_bytes > max_total_bytes {
                let msg = format!(
                    "the tree is {} MiB, over the {} MiB checkpoint limit \
                     ([checkpoints] max_total_size_mb)",
                    size.total_bytes / (1024 * 1024),
                    max_total_bytes / (1024 * 1024)
                );
                debug!("Checkpoint skipped: {msg}");
                self.set_skip_reason(Some(msg));
                return false;
            }
        }
        // The checkpoint is going ahead: any previous skip reason is stale.
        self.set_skip_reason(None);

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
    pub fn list_checkpoints(&self, working_dir: &str, limit: Option<usize>) -> Vec<Checkpoint> {
        let workdir = PathBuf::from(working_dir);
        let store = self.store_path(working_dir);
        if !store.join("HEAD").exists() {
            return Vec::new();
        }
        let env = git_shadow_env(&store, &workdir, &store.join("index"));

        // Honor the explicit limit, defaulting to the configured snapshot cap
        // (the old implementation hardcoded `-n 20` and ignored the tool's
        // `limit` argument).
        let max = self.config.lock().map(|g| g.max_snapshots).unwrap_or(20);
        let n = limit.unwrap_or(max).clamp(1, 100);

        // Get commit log from the shadow store
        let log_output = Command::new("git")
            .args(["log", "--format=%H|%h|%aI|%s", "-n", &n.to_string()])
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
        // hermes `_validate_file_path` parity: the target never reaches git
        // unchecked — absolute paths and `..` traversal are rejected up front.
        if let Some(target) = file_path {
            validate_restore_path(target, working_dir).map_err(Error::Agent)?;
        }
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

/// Maximum files a checkpoint directory may contain — hermes
/// `checkpoint_manager.py` `_MAX_FILES` parity: beyond this, `git add -A` on
/// the shadow store would hang on pathological directories.
const MAX_CHECKPOINT_FILES: usize = 50_000;

/// Directory names the shadow store's `info/exclude` tells git to skip — the
/// file counter mirrors the same list so it never falsely refuses a
/// checkpoint for a repo with a large object store / node_modules.
///
/// `checkpoints` is in this list for a reason that is not a "large
/// directory" concern: the shadow store lives at `<operant home>/checkpoints`
/// and the snapshot's working directory is the operant home itself, so
/// WITHOUT this exclusion the store SNAPSHOTS ITSELF. `git add -A` then
/// commits the store's own packed objects (measured on this box: a
/// 406 MB and a 428 MB pack file inside the store's own tree), and every
/// snapshot embeds the previous one's pack — geometric growth. That is
/// where the 1.5 GB store came from: 11 commits whose reachable blobs
/// total 2.6 GiB, the largest of them being the store's own packs.
const CHECKPOINT_EXCLUDE_DIRS: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "checkpoints",
];

/// Count files under `dir` recursively, stopping early once `max` is exceeded
/// (hermes `_dir_file_count` parity: `os.walk` with `followlinks=False` —
/// symlinks are never followed, so a symlink cycle cannot loop forever).
fn dir_file_count(dir: &Path, max: usize) -> usize {
    let mut count = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            // entry.file_type() does NOT follow symlinks — `path.is_dir()`
            // would follow a `loop -> .` cycle forever.
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if CHECKPOINT_EXCLUDE_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                stack.push(entry.path());
            } else if ft.is_file() {
                count += 1;
                if count > max {
                    return count;
                }
            }
        }
    }
    count
}

/// What a tree's SIZE looks like, for the checkpoint's size bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreeSize {
    /// The total bytes of the regular files under the tree.
    total_bytes: u64,
    /// The largest single file's size in bytes.
    largest_file_bytes: u64,
}

/// The total size of the files under a directory, in bytes.
///
/// Symlinks are not followed, for the same reason the file counter does
/// not follow them.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file()
                && let Ok(meta) = entry.metadata()
            {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Measure a tree's total and largest file size.
///
/// The excluded directory names are skipped, matching the file counter and
/// the shadow store's `info/exclude`, so the measurement covers what would
/// actually be snapshotted.
///
/// Symlinks are never followed, for the same reason the file counter does
/// not follow them: a `loop -> .` cycle would otherwise loop forever.
fn tree_size(dir: &Path) -> TreeSize {
    let mut total_bytes = 0u64;
    let mut largest_file_bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if CHECKPOINT_EXCLUDE_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                stack.push(entry.path());
            } else if ft.is_file() {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let len = meta.len();
                total_bytes = total_bytes.saturating_add(len);
                largest_file_bytes = largest_file_bytes.max(len);
            }
        }
    }
    TreeSize {
        total_bytes,
        largest_file_bytes,
    }
}

/// Lexically normalize a path (`.`/`..`) without requiring it to exist —
/// hermes `Path.resolve()` non-strict parity for the restore-path gate.
fn resolve_non_strict(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Validate a restore `file_path` before it reaches git: reject absolute
/// paths and `..` traversal that escapes the working directory — hermes
/// `_validate_file_path` parity (`checkpoint_manager.py`).
fn validate_restore_path(file_path: &str, working_dir: &str) -> std::result::Result<(), String> {
    if file_path.trim().is_empty() {
        return Err("Empty file path".to_string());
    }
    let p = Path::new(file_path);
    if p.is_absolute() {
        return Err(format!(
            "File path must be relative, got absolute path: {file_path:?}"
        ));
    }
    let workdir_abs = if Path::new(working_dir).is_absolute() {
        PathBuf::from(working_dir)
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(working_dir)
    };
    // Resolve symlinks in the working dir itself for a consistent base.
    let base = std::fs::canonicalize(&workdir_abs).unwrap_or_else(|_| workdir_abs.clone());
    let resolved = resolve_non_strict(&base.join(file_path));
    if !resolved.starts_with(&base) {
        return Err(format!(
            "File path escapes the working directory via traversal: {file_path:?}"
        ));
    }
    // Defense-in-depth (hermes `Path.resolve()` parity): the lexical check
    // cannot see a symlink INSIDE the workdir pointing outside (e.g.
    // `link -> /etc`). Resolve the longest existing prefix canonically and
    // re-check containment; the lexical tail covers restoring deleted files.
    let mut probe = resolved.clone();
    let mut suffix: Vec<PathBuf> = Vec::new();
    loop {
        match std::fs::canonicalize(&probe) {
            Ok(canon) => {
                let mut final_path = canon;
                for part in suffix.iter().rev() {
                    final_path.push(part);
                }
                if !final_path.starts_with(&base) {
                    return Err(format!(
                        "File path escapes the working directory via traversal: {file_path:?}"
                    ));
                }
                break;
            }
            Err(_) => match probe.file_name() {
                Some(name) => {
                    suffix.push(PathBuf::from(name));
                    probe.pop();
                }
                None => break,
            },
        }
    }
    Ok(())
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
    if !store.join("HEAD").exists() {
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
    }

    // Default excludes for the shadow store: never snapshot the user's own
    // `.git` (would otherwise be added as a gitlink) or common heavy dirs.
    //
    // The file is REWRITTEN every time, not only when the store is new: a
    // store created before the list grew kept the old one forever, and the
    // self-snapshot loop below is exactly what that let happen.
    //
    // The store's own path is excluded BY PATH as well as by name. The
    // name (`checkpoints`) only matches the default layout; `base_dir` is
    // configurable, and a store at any other path would still snapshot
    // itself.
    let exclude = store.join("info").join("exclude");
    let _ = std::fs::create_dir_all(store.join("info"));
    let store_path = store.to_string_lossy();
    let _ = std::fs::write(
        &exclude,
        format!(
            ".git\nnode_modules/\ntarget/\n__pycache__/\n.venv/\nvenv/\ncheckpoints/\n{}\n",
            store_path
        ),
    );
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
                    // The reason, when there is one the operator can act
                    // on. Before this, the message named only two causes
                    // and NEITHER was the size-bound skip, so an operator
                    // whose tree was too big was told to enable
                    // checkpoints that were already enabled.
                    let message = match manager.last_skip_reason() {
                        Some(why) => format!("Failed to create checkpoint: {why}."),
                        None => "Failed to create checkpoint (checkpoints may be disabled in config — set [checkpoints] enabled = true — or there were no changes to snapshot)".to_string(),
                    };
                    ToolResult::error("checkpoint_ensure", message)
                }
            }
            "list" => {
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                let checkpoints = manager.list_checkpoints(working_dir, limit);
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

    /// The size bounds must be ENFORCED, not just declared.
    ///
    /// `max_file_size_mb` and `max_total_size_mb` were on the config and
    /// never read, so nothing bounded the bytes a snapshot could take: the
    /// count bound (`MAX_CHECKPOINT_FILES` = 50,000) is a COUNT, and 50,000
    /// files of any size passes it. Measured on this box: one shadow store
    /// had reached 1.42 GiB of packed objects — 3x over the 500 MB the
    /// config documented.
    #[test]
    fn a_tree_over_the_size_bounds_is_not_snapshotted() {
        use std::process::Command as StdCommand;
        if StdCommand::new("git").arg("--version").output().is_err() {
            return;
        }

        let tmp = std::env::temp_dir().join(format!("operant_ckpt_size_{}", uuid::Uuid::new_v4()));
        let base =
            std::env::temp_dir().join(format!("operant_ckpt_size_store_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        // A file over the per-file limit.
        std::fs::write(tmp.join("big.bin"), vec![0u8; 3 * 1024 * 1024]).unwrap();

        let mgr = CheckpointManager::new();
        mgr.configure(CheckpointConfig {
            base_dir: base.clone(),
            enabled: true,
            // 1 MiB per file, 3 MiB total: the 3 MiB file breaks both.
            max_file_size_mb: 1,
            max_total_size_mb: 1,
            ..Default::default()
        });

        assert!(
            !mgr.ensure_checkpoint(tmp.to_str().unwrap(), "too big"),
            "a tree over the size bounds must not be snapshotted"
        );

        // The reason must be REPORTED, not just logged at debug level:
        // the tool's error message used to name only "checkpoints may be
        // disabled" and "there were no changes", and NEITHER was the
        // size-bound skip. An operator whose tree was too big was told to
        // enable checkpoints that were already enabled.
        let reason = mgr
            .last_skip_reason()
            .expect("a size-bound skip must record why it skipped");
        assert!(
            reason.contains("MiB") && reason.contains("limit"),
            "the skip reason must name the size and the limit, got: {reason}"
        );
        assert!(
            reason.contains("max_file_size_mb") || reason.contains("max_total_size_mb"),
            "the skip reason must name the config key to change, got: {reason}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Clearing the checkpoints must reclaim the BYTES, not just the rows.
    ///
    /// The database rows and the shadow store on disk are two different
    /// things, and `operant checkpoints prune` and `clear` only removed the
    /// rows: a store that had grown to 1.5 GB of packed objects reported
    /// "cleared N checkpoints" and reclaimed nothing.
    #[test]
    fn clearing_the_checkpoints_removes_the_store_and_reports_the_bytes() {
        use std::process::Command as StdCommand;
        if StdCommand::new("git").arg("--version").output().is_err() {
            return;
        }

        let home =
            std::env::temp_dir().join(format!("operant_ckpt_clear_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("a.txt"), "hello").unwrap();

        let mgr = CheckpointManager::new();
        mgr.configure(CheckpointConfig {
            base_dir: home.join("checkpoints"),
            enabled: true,
            ..Default::default()
        });

        assert!(
            mgr.ensure_checkpoint(home.to_str().unwrap(), "one"),
            "the checkpoint must be taken"
        );
        let store = mgr.store_path(home.to_str().unwrap());
        assert!(store.exists(), "the store must exist after a checkpoint");

        let bytes = mgr
            .clear_store(home.to_str().unwrap())
            .expect("clear the store");
        assert!(bytes > 0, "the reclaim must report the bytes it freed");
        assert!(!store.exists(), "the store must be gone after the clear");
        // A second clear is a no-op, not an error.
        assert_eq!(
            mgr.clear_store(home.to_str().unwrap())
                .expect("clear again"),
            0,
            "clearing a store that is not there is a no-op"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The shadow store must NOT snapshot itself.
    ///
    /// The store lives at `<operant home>/checkpoints/store/<hash>` and the
    /// snapshot's working directory is the operant home itself. Without an
    /// exclusion, `git add -A` commits the store's own packed objects into
    /// the next snapshot, and every snapshot embeds the previous one's pack
    /// — geometric growth. Measured on this box: 11 commits whose
    /// reachable blobs total 2.6 GiB, the largest being the store's own
    /// 406 MB and 428 MB pack files.
    ///
    /// The test puts the store INSIDE the snapshotted tree — which is the
    /// production layout — and asserts the store's own pack is not among
    /// the committed blobs.
    #[test]
    fn the_shadow_store_does_not_snapshot_itself() {
        use std::process::Command as StdCommand;
        if StdCommand::new("git").arg("--version").output().is_err() {
            return;
        }

        // The production layout: the store is inside the working dir.
        let home = std::env::temp_dir().join(format!("operant_ckpt_self_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("a.txt"), "hello").unwrap();

        let mgr = CheckpointManager::new();
        mgr.configure(CheckpointConfig {
            base_dir: home.join("checkpoints"),
            enabled: true,
            ..Default::default()
        });

        assert!(
            mgr.ensure_checkpoint(home.to_str().unwrap(), "one"),
            "the checkpoint must be taken"
        );

        // The store's own objects must not be in the commit.
        let store = mgr.store_path(home.to_str().unwrap());
        let listing = StdCommand::new("git")
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .current_dir(&store)
            .output()
            .expect("list the commit");
        let names = String::from_utf8_lossy(&listing.stdout);
        assert!(
            !names.contains("checkpoints/") && !names.contains("objects/"),
            "the store must not commit its own objects, the commit holds:\n{names}"
        );
        assert!(
            names.contains("a.txt"),
            "the working tree's own file must be committed, got:\n{names}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// A tree WITHIN the bounds is still snapshotted. The size check must
    /// not refuse everything.
    #[test]
    fn a_tree_within_the_size_bounds_is_snapshotted() {
        use std::process::Command as StdCommand;
        if StdCommand::new("git").arg("--version").output().is_err() {
            return;
        }

        let tmp = std::env::temp_dir().join(format!("operant_ckpt_ok_{}", uuid::Uuid::new_v4()));
        let base =
            std::env::temp_dir().join(format!("operant_ckpt_ok_store_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("small.txt"), b"hello").unwrap();

        let mgr = CheckpointManager::new();
        mgr.configure(CheckpointConfig {
            base_dir: base.clone(),
            enabled: true,
            max_file_size_mb: 1,
            max_total_size_mb: 1,
            ..Default::default()
        });

        assert!(
            mgr.ensure_checkpoint(tmp.to_str().unwrap(), "small enough"),
            "a tree within the size bounds must be snapshotted"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The size measurement covers what would actually be snapshotted: the
    /// EXCLUDED directory names are skipped, matching the shadow store's
    /// `info/exclude` and the file counter.
    #[test]
    fn the_size_measurement_skips_the_excluded_directories() {
        let dir = std::env::temp_dir().join(format!("operant_ckpt_excl_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("small.txt"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("target").join("big.bin"), vec![0u8; 10_000]).unwrap();

        let size = tree_size(&dir);
        assert_eq!(
            size.total_bytes, 100,
            "the excluded directory must not count toward the total"
        );
        assert_eq!(
            size.largest_file_bytes, 100,
            "the excluded directory's file must not count as the largest"
        );

        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(mgr.list_checkpoints(dir, None).len(), 1);

        // Mutate, second snapshot, then restore the first
        std::fs::write(tmp.join("a.txt"), "changed").unwrap();
        mgr.new_turn(); // clear per-turn dedup
        assert!(mgr.ensure_checkpoint(dir, "two"));
        let cps = mgr.list_checkpoints(dir, None);
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
            mgr.list_checkpoints(dir, None).len() <= 2,
            "expected <=2, got {}",
            mgr.list_checkpoints(dir, None).len()
        );
        // The newest checkpoint is still restorable
        let cps = mgr.list_checkpoints(dir, None);
        if let Some(latest) = cps.first() {
            mgr.restore(dir, &latest.hash, Some("a.txt")).unwrap();
        }

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_validate_restore_path_rejects_absolute_and_traversal() {
        let tmp = std::env::temp_dir().join(format!("operant_ckpt_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir = tmp.to_str().unwrap();

        // Absolute paths are rejected outright (hermes `_validate_file_path`).
        let err = validate_restore_path("/etc/passwd", dir).unwrap_err();
        assert!(err.contains("absolute"), "got: {err}");

        // `..` traversal escaping the working directory is rejected.
        let err = validate_restore_path("../../etc/passwd", dir).unwrap_err();
        assert!(err.contains("escapes"), "got: {err}");

        // Safe relative paths (including in-dir normalization) are accepted.
        std::fs::write(tmp.join("a.txt"), "x").unwrap();
        assert!(validate_restore_path("a.txt", dir).is_ok());
        assert!(validate_restore_path("sub/../a.txt", dir).is_ok());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_checkpoint_restore_rejects_absolute_path() {
        let tool = CheckpointTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "restore",
                    "working_dir": ".",
                    "commit_hash": "abc123",
                    "file_path": "/etc/passwd"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("absolute"), "got: {err}");
    }

    #[tokio::test]
    async fn test_checkpoint_restore_rejects_traversal() {
        let tool = CheckpointTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "restore",
                    "working_dir": ".",
                    "commit_hash": "abc123",
                    "file_path": "../../etc/passwd"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("escapes"), "got: {err}");
    }

    #[test]
    fn test_dir_file_count_stops_early() {
        let tmp = std::env::temp_dir().join(format!("operant_ckpt_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("f1"), "a").unwrap();
        std::fs::write(tmp.join("f2"), "b").unwrap();
        std::fs::write(tmp.join("sub/f3"), "c").unwrap();

        assert_eq!(dir_file_count(&tmp, 100), 3);
        assert!(dir_file_count(&tmp, 2) > 2); // stops early once over-cap

        // Excluded dirs (mirroring the shadow store's info/exclude list) are
        // not counted — a repo with a large .git object store must not be
        // falsely refused a checkpoint.
        std::fs::create_dir_all(tmp.join(".git/objects")).unwrap();
        for i in 0..10 {
            std::fs::write(tmp.join(format!(".git/objects/o{i}")), "x").unwrap();
        }
        assert_eq!(dir_file_count(&tmp, 100), 3);

        // Symlinks are never followed (os.walk followlinks=False parity) — a
        // self-cycle must terminate instead of looping forever.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&tmp, tmp.join("loop")).unwrap();
            assert_eq!(dir_file_count(&tmp, 100), 3);
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_shadow_list_checkpoints_honors_limit() {
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

        assert_eq!(mgr.list_checkpoints(dir, Some(2)).len(), 2);
        assert_eq!(mgr.list_checkpoints(dir, Some(10)).len(), 3);
        assert_eq!(mgr.list_checkpoints(dir, None).len(), 3);

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&base);
    }
}
