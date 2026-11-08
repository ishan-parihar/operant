use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::sync::{LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

#[derive(Debug, Clone)]
struct FileSnapshot {
    path: String,
    size: u64,
    modified: u64,
    hash: String,
    permissions: String,
}

static FILE_STATES: LazyLock<Mutex<HashMap<String, FileSnapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct FileStateTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileStateArgs {
    operation: String,
    path: String,
}

/// The content hash for a path.
///
/// A FILE is its bytes. A DIRECTORY is the sorted sequence of its files'
/// relative paths and bytes (RT-346): the metadata path already records
/// directories — `get_file_metadata` appends the "/d" permission suffix and
/// `fs::metadata` accepts them — so `fs::read`'s "Is a directory" failure at
/// the last step made every directory check/watch/diff fail after the code
/// had gone out of its way to describe the directory.
fn compute_hash(path: &str) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let metadata = fs::metadata(path).map_err(|e| format!("Failed to stat file: {}", e))?;
    if metadata.is_dir() {
        // An empty directory still gets a well-defined, stable hash (and one
        // that differs from an empty file's).
        hasher.update(b"dir:");
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        collect_dir_files(std::path::Path::new(path), &mut files);
        // Sorted: the walk order is the filesystem's, not ours, and the same
        // tree must hash the same on every call.
        files.sort();
        for file in &files {
            // The RELATIVE path is the digest input (the absolute prefix is
            // this process's tempdir, not the tree); the file itself is read
            // through the absolute path — feeding the relative path to
            // `fs::read` would silently skip every file and hash names only
            // (RT-346).
            let rel = file.strip_prefix(path).unwrap_or(file);
            hasher.update(rel.to_string_lossy().as_bytes());
            hasher.update(b"\0");
            if let Ok(bytes) = fs::read(file) {
                hasher.update(&bytes);
            }
            hasher.update(b"\0");
        }
    } else {
        let content = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
        hasher.update(&content);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Depth-first collect of the FILES under `dir` (absolute paths).
/// Unreadable entries are skipped (a permissions failure is not a hash
/// failure — the metadata is still worth reporting).
fn collect_dir_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn get_file_metadata(path: &str) -> Result<FileSnapshot, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("Failed to stat file: {}", e))?;

    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let size = metadata.len();

    let mut perms = String::new();
    let mode = metadata.permissions();
    perms.push(if mode.readonly() { 'r' } else { 'w' });
    perms.push_str(if metadata.is_dir() { "/d" } else { "/f" });

    let hash = compute_hash(path)?;

    Ok(FileSnapshot {
        path: path.to_string(),
        size,
        modified,
        hash,
        permissions: perms,
    })
}

#[async_trait]
impl OperantTool for FileStateTool {
    fn name(&self) -> &str {
        "file_state"
    }

    fn description(&self) -> &str {
        "Track and detect changes in file system state. Operations: \
         'check' returns current file metadata (size, modified time, permissions), \
         'watch' stores the current file state for later comparison, \
         'diff' compares the current file state against the last watched state."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FileStateArgs>("file_state", "Track file system state")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: FileStateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("file_state", format!("Invalid arguments: {}", e)),
        };

        match args.operation.as_str() {
            "check" => handle_check(&args.path),
            "watch" => handle_watch(&args.path),
            "diff" => handle_diff(&args.path),
            other => ToolResult::error(
                "file_state",
                format!(
                    "Unknown operation '{}'. Supported: check, watch, diff",
                    other
                ),
            ),
        }
    }
}

fn handle_check(path: &str) -> ToolResult {
    let snapshot = match get_file_metadata(path) {
        Ok(s) => s,
        Err(e) => return ToolResult::error("file_state", format!("Check failed: {}", e)),
    };

    ToolResult::success(
        "file_state",
        serde_json::json!({
            "operation": "check",
            "path": snapshot.path,
            "size": snapshot.size,
            "modified": snapshot.modified,
            "permissions": snapshot.permissions,
            "hash": snapshot.hash,
            "exists": true
        }),
    )
}

#[expect(
    clippy::expect_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
fn handle_watch(path: &str) -> ToolResult {
    let snapshot = match get_file_metadata(path) {
        Ok(s) => s,
        Err(e) => return ToolResult::error("file_state", format!("Watch failed: {}", e)),
    };

    let mut states = FILE_STATES
        .lock()
        .expect("FILE_STATES mutex poisoned — programmer error");
    states.insert(path.to_string(), snapshot);

    ToolResult::success(
        "file_state",
        serde_json::json!({
            "operation": "watch",
            "path": path,
            "message": "File state recorded for change tracking"
        }),
    )
}

#[expect(
    clippy::expect_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
fn handle_diff(path: &str) -> ToolResult {
    let states = FILE_STATES
        .lock()
        .expect("FILE_STATES mutex poisoned — programmer error");
    let stored = match states.get(path) {
        Some(s) => s.clone(),
        None => {
            return ToolResult::success(
                "file_state",
                serde_json::json!({
                    "operation": "diff",
                    "path": path,
                    "watched": false,
                    "message": "No stored state found for this file. Use 'watch' first."
                }),
            );
        }
    };
    drop(states);

    let current = match get_file_metadata(path) {
        Ok(s) => s,
        Err(e) => return ToolResult::error("file_state", format!("Diff failed: {}", e)),
    };

    let mut changes: Vec<String> = Vec::new();

    if stored.size != current.size {
        changes.push(format!("size changed: {} -> {}", stored.size, current.size));
    }
    if stored.modified != current.modified {
        changes.push("modified time changed".to_string());
    }
    if stored.hash != current.hash {
        changes.push("content hash changed".to_string());
    }
    if stored.permissions != current.permissions {
        changes.push(format!(
            "permissions changed: {} -> {}",
            stored.permissions, current.permissions
        ));
    }

    let changed = !changes.is_empty();

    ToolResult::success(
        "file_state",
        serde_json::json!({
            "operation": "diff",
            "path": path,
            "changed": changed,
            "changes": changes,
            "current": {
                "size": current.size,
                "modified": current.modified,
                "hash": current.hash,
                "permissions": current.permissions
            },
            "stored": {
                "size": stored.size,
                "modified": stored.modified,
                "hash": stored.hash,
                "permissions": stored.permissions
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// RT-346: the metadata path records directories (the "/d" permission
    /// suffix) but the hash step used `fs::read`, which fails with "Is a
    /// directory" — so every directory check/watch/diff failed at the LAST
    /// step, after the code described the directory. The directory hash is
    /// the sorted sequence of its files' relative paths and bytes.
    #[tokio::test]
    async fn a_directory_hashes_deterministically_and_reports_content_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_str().expect("utf8 path");
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "beta").unwrap();

        async fn check(path: &str) -> ToolResult {
            FileStateTool
                .execute(
                    serde_json::json!({"operation": "check", "path": path}),
                    ToolContext::default(),
                )
                .await
        }

        let first = check(root).await;
        assert!(first.success, "a directory must not fail the check");
        let payload: Value = serde_json::from_str(&first.content).expect("json");
        let hash = payload["hash"].as_str().expect("hash").to_string();
        assert!(
            payload["permissions"].as_str().unwrap_or("").contains("/d"),
            "the snapshot still records the directory: {payload}"
        );

        // Stable across calls.
        let again = check(root).await;
        let payload2: Value = serde_json::from_str(&again.content).expect("json");
        assert_eq!(hash, payload2["hash"], "the same tree hashes the same");

        // A change inside the directory moves the hash.
        std::fs::write(dir.path().join("a.txt"), "ALPHA").unwrap();
        let changed = check(root).await;
        let payload3: Value = serde_json::from_str(&changed.content).expect("json");
        assert_ne!(
            hash, payload3["hash"],
            "a content change inside must move the hash"
        );
    }

    #[tokio::test]
    async fn test_file_state_invalid_args() {
        let tool = FileStateTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_state_check_nonexistent() {
        let tool = FileStateTool;
        let args = serde_json::json!({
            "operation": "check",
            "path": "/tmp/nonexistent_file_xyz_123.test"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_state_watch_and_diff() {
        let tmp_path = "/tmp/operant_test_file_state.txt";
        let mut f = fs::File::create(tmp_path).unwrap();
        f.write_all(b"initial content").unwrap();
        f.sync_all().unwrap();

        let tool = FileStateTool;

        let watch_args = serde_json::json!({
            "operation": "watch",
            "path": tmp_path
        });
        let watch_result = tool.execute(watch_args, ToolContext::default()).await;
        assert!(watch_result.success);

        let diff_args = serde_json::json!({
            "operation": "diff",
            "path": tmp_path
        });
        let diff_result = tool
            .execute(diff_args.clone(), ToolContext::default())
            .await;
        assert!(diff_result.success);
        let v: Value = serde_json::from_str(&diff_result.content).unwrap();
        assert_eq!(v["changed"], false);

        let mut f = fs::OpenOptions::new().append(true).open(tmp_path).unwrap();
        f.write_all(b" modified content").unwrap();
        f.sync_all().unwrap();

        let diff_result2 = tool.execute(diff_args, ToolContext::default()).await;
        assert!(diff_result2.success);
        let v2: Value = serde_json::from_str(&diff_result2.content).unwrap();
        assert_eq!(v2["changed"], true);

        let _ = fs::remove_file(tmp_path);
    }

    // ---- compute_hash tests --------------------------------------------

    #[test]
    fn test_compute_hash_known_content() {
        let dir = std::env::temp_dir();
        let path = dir.join("operant_test_hash_known.txt");
        std::fs::write(&path, b"hello").unwrap();

        let hash = compute_hash(path.to_str().unwrap()).unwrap();
        // SHA-256 of "hello"
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_compute_hash_empty_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("operant_test_hash_empty.txt");
        std::fs::write(&path, b"").unwrap();

        let hash = compute_hash(path.to_str().unwrap()).unwrap();
        // SHA-256 of empty string
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_compute_hash_different_content_different_hash() {
        let dir = std::env::temp_dir();
        let path_a = dir.join("operant_test_hash_a.txt");
        let path_b = dir.join("operant_test_hash_b.txt");
        std::fs::write(&path_a, b"content a").unwrap();
        std::fs::write(&path_b, b"content b").unwrap();

        let hash_a = compute_hash(path_a.to_str().unwrap()).unwrap();
        let hash_b = compute_hash(path_b.to_str().unwrap()).unwrap();
        assert_ne!(hash_a, hash_b);

        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn test_compute_hash_nonexistent_file() {
        let result = compute_hash("/tmp/nonexistent_file_xyz_hash_test_123");
        assert!(result.is_err());
    }

    // ---- get_file_metadata tests ---------------------------------------

    #[test]
    fn test_get_file_metadata_nonexistent() {
        let result = get_file_metadata("/tmp/nonexistent_file_xyz_meta_test_123");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_file_metadata_basic() {
        let dir = std::env::temp_dir();
        let path = dir.join("operant_test_meta_basic.txt");
        std::fs::write(&path, b"test data").unwrap();

        let meta = get_file_metadata(path.to_str().unwrap()).unwrap();
        assert_eq!(meta.size, 9);
        assert_eq!(meta.path, path.to_str().unwrap());
        assert!(meta.modified > 0);
        assert!(!meta.hash.is_empty());
        assert!(!meta.permissions.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    // ---- tool-level edge cases -----------------------------------------

    #[tokio::test]
    async fn test_file_state_unknown_operation() {
        let tool = FileStateTool;
        let args = serde_json::json!({
            "operation": "unknown_op",
            "path": "/tmp/somefile.txt"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_file_state_diff_without_watch() {
        let dir = std::env::temp_dir();
        let path = dir.join("operant_test_diff_nowatch.txt");
        std::fs::write(&path, b"content").unwrap();

        let tool = FileStateTool;
        let args = serde_json::json!({
            "operation": "diff",
            "path": path.to_str().unwrap()
        });
        let result = tool.execute(args, ToolContext::default()).await;
        // Diff without prior watch should succeed but report not watched
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["watched"], false);

        let _ = std::fs::remove_file(&path);
    }
}
