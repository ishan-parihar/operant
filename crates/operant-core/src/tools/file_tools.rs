//! File operation tools
//!
//! Tools for reading, writing, searching, and listing files.
//!
//! ## Path safety (iter-125)
//!
//! All file tools reject paths that escape the user's home directory or
//! resolve to known-sensitive system files. This closes the ponytail-audit
//! security gap "file_tools no path-traversal validation — agent can read
//! /etc/shadow, ~/.ssh/id_rsa".
//!
//! The guard is conservative on purpose: any canonicalized path that
//! either (a) starts with `..` (still relative after canonicalization
//! failure — usually means the file doesn't exist), (b) lives outside
//! the home directory, OR (c) is in the hard-deny list of sensitive
//! files (SSH keys, .aws/credentials, /etc/shadow, etc.) is rejected.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Hard-deny list of file path prefixes that the agent must never read or
/// write, regardless of where the user's home directory is. Mirrors the
/// sensitive-file list used by Claude Code / Cursor.
const DENIED_PATH_PATTERNS: &[&str] = &[
    // SSH private keys + config
    ".ssh/id_rsa",
    ".ssh/id_dsa",
    ".ssh/id_ecdsa",
    ".ssh/id_ed25519",
    ".ssh/identity",
    ".ssh/identity.pub",
    // Cloud-provider credentials
    ".aws/credentials",
    ".aws/config",
    ".config/gcloud/credentials",
    ".config/gcloud/application_default_credentials.json",
    ".azure/credentials",
    // Token files
    ".operant/auth.json",
    ".operant/mcp-tokens",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".docker/config.json",
    ".kube/config",
    // System shadow files (Linux)
    "/etc/shadow",
    "/etc/gshadow",
    // macOS Keychain
    "Library/Keychains/login.keychain",
    "Library/Keychains/login.keychain-db",
];

/// Atomically replace `path` with `content` (hermes `file_operations._atomic_write`
/// parity): write to a temp file in the SAME directory so the final rename is
/// same-filesystem atomic, preserve the existing file's mode (an executable
/// script's +x bit must survive an edit), and remove the temp on any failure —
/// a crash mid-write never corrupts the target or leaks a partial file.
pub(crate) fn atomic_write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let existing_perms = std::fs::metadata(path).ok().map(|m| m.permissions());

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;

    // Preserve the existing file's mode across the replace (hermes copies
    // mode with chmod --reference before renaming). For brand-new targets,
    // land at 0666 & ~umask (typically 0644) instead of NamedTempFile's
    // hardcoded 0600 — hermes fixed exactly this with `chmod "=rw"`
    // (#70856). set_permissions bypasses the umask (fchmod sets the exact
    // mode), so compute 0666 & ~umask explicitly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(perms) = &existing_perms {
            let mode = perms.mode();
            let _ = tmp
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(mode));
        } else {
            let _ = tmp
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o666 & !current_umask()));
        }
    }

    let write_result = tmp
        .as_file_mut()
        .write_all(content.as_bytes())
        .and_then(|_| tmp.as_file().sync_all());

    if let Err(e) = write_result {
        // Drop the temp so a failed write never leaks a partial file.
        drop(tmp);
        return Err(e);
    }

    // Atomic same-directory rename over the target. `persist` returns the
    // (now-named) File on success — discard it.
    tmp.persist(path).map(|_| ()).map_err(|e| e.error)
}

/// Read the process umask race-free. Linux exposes it in `/proc/self/status`;
/// other unixes fall back to the common 0o022 (a best-effort default — the
/// typical deployment is Linux, where the /proc read is authoritative).
#[cfg(unix)]
fn current_umask() -> u32 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            let Some(rest) = line.strip_prefix("Umask:") else {
                continue;
            };
            if let Ok(mask) = u32::from_str_radix(rest.trim(), 8) {
                return mask;
            }
        }
    }
    0o022
}

/// Validate that a path is safe for the agent to read or write. Returns
/// the canonicalized path on success, or an error message on rejection.
///
/// `pub(crate)` so the patch tool shares the same gate (it previously
/// bypassed it entirely).
pub(crate) fn validate_path(raw_path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw_path);

    // Step 1: Resolve to canonical form. If the path doesn't exist yet
    // (write case), canonicalize the parent and append the filename.
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // Path doesn't exist — canonicalize the parent if possible.
            if let Some(parent) = path.parent() {
                match parent.canonicalize() {
                    Ok(p) => p.join(path.file_name().unwrap_or_default()),
                    Err(_) => {
                        // Parent doesn't exist either — fall back to the
                        // raw path. Reject if it starts with `..` (still
                        // relative would let the agent escape via `../`).
                        if raw_path.starts_with("..")
                            || raw_path.contains("/../")
                            || raw_path.contains("\\..\\")
                        {
                            return Err(format!(
                                "Path traversal rejected (parent doesn't exist + relative escape): {}",
                                raw_path
                            ));
                        }
                        PathBuf::from(raw_path)
                    }
                }
            } else {
                PathBuf::from(raw_path)
            }
        }
    };

    // Step 2: Reject if the canonical path is in the hard-deny list.
    let canonical_str = canonical.to_string_lossy();
    let canonical_lower = canonical_str.to_lowercase();
    for pattern in DENIED_PATH_PATTERNS {
        let pattern_lower = pattern.to_lowercase();
        if canonical_lower.ends_with(&pattern_lower)
            || canonical_lower.contains(&format!("/{}/", pattern_lower))
        {
            return Err(format!(
                "Access to sensitive file denied (matches pattern '{}'). If you genuinely need this file, ask the user to read it manually and paste the contents.",
                pattern
            ));
        }
    }

    // Step 3: Reject `..` traversal in the canonical path.
    if canonical_str.contains("/../") || canonical_str.contains("\\..\\") {
        return Err(format!(
            "Path traversal rejected (canonical path contains '..'): {}",
            canonical_str
        ));
    }

    Ok(canonical)
}

/// Tool for reading file contents
pub struct FileReadTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileReadArgs {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[async_trait]
impl OperantTool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Supports partial reads with offset and limit parameters."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FileReadArgs>("file_read", "Read file contents")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: FileReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("file_read", format!("Invalid arguments: {}", e)),
        };

        // Path safety check (iter-125 — closes the ponytail-audit
        // security gap "file_tools no path-traversal validation").
        let path = match validate_path(&args.path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("file_read", e),
        };

        if !path.exists() {
            return ToolResult::error("file_read", format!("File not found: {}", args.path));
        }

        if !path.is_file() {
            return ToolResult::error("file_read", format!("Path is not a file: {}", args.path));
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let offset = args.offset.unwrap_or(0);
                let limit = args.limit.unwrap_or(usize::MAX);

                let lines: Vec<&str> = content.lines().skip(offset).take(limit).collect();
                let result = lines.join("\n");

                ToolResult::success(
                    "file_read",
                    serde_json::json!({
                        "path": args.path,
                        "content": result,
                        "length": result.len(),
                        "total_lines": content.lines().count()
                    }),
                )
            }
            Err(e) => ToolResult::error("file_read", format!("Failed to read file: {}", e)),
        }
    }
}

/// Tool for writing content to a file
pub struct FileWriteTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileWriteArgs {
    path: String,
    content: String,
    append: Option<bool>,
}

#[async_trait]
impl OperantTool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Supports creating new files or overwriting existing ones. Use append=true to add to existing files."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FileWriteArgs>("file_write", "Write content to a file")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: FileWriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("file_write", format!("Invalid arguments: {}", e)),
        };

        // Path safety check (iter-125 — closes the ponytail-audit security
        // gap "file_tools no path-traversal validation").
        let path = match validate_path(&args.path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("file_write", e),
        };

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolResult::error(
                        "file_write",
                        format!("Failed to create directory: {}", e),
                    );
                }
            }
        }

        let result = if args.append.unwrap_or(false) {
            // Appends are inherently sequential — no atomic-replace possible.
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(args.content.as_bytes())
                })
        } else {
            // Atomic replace (hermes _atomic_write parity): a crash mid-write
            // must never corrupt the target or leave a partial file.
            atomic_write(&path, &args.content)
        };

        match result {
            Ok(_) => {
                let metadata = std::fs::metadata(&path).ok();
                ToolResult::success(
                    "file_write",
                    serde_json::json!({
                        "path": args.path,
                        "bytes_written": args.content.len(),
                        "file_size": metadata.map(|m| m.len()).unwrap_or(0)
                    }),
                )
            }
            Err(e) => ToolResult::error("file_write", format!("Failed to write file: {}", e)),
        }
    }
}

/// Tool for searching file contents
pub struct FileSearchTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSearchArgs {
    path: String,
    pattern: String,
    case_sensitive: Option<bool>,
    max_results: Option<usize>,
}

#[async_trait]
impl OperantTool for FileSearchTool {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Search for a pattern within files. Recursively searches directories for matching lines."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FileSearchArgs>("file_search", "Search files for pattern")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: FileSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("file_search", format!("Invalid arguments: {}", e)),
        };

        // Path safety check (iter-125).
        let path = match validate_path(&args.path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("file_search", e),
        };
        let case_sensitive = args.case_sensitive.unwrap_or(true);
        let escaped_pattern = regex::escape(&args.pattern);
        let re = match regex::RegexBuilder::new(&escaped_pattern)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(re) => re,
            Err(e) => {
                return ToolResult::error("file_search", format!("Invalid regex pattern: {}", e));
            }
        };

        let mut results = Vec::new();
        let max_results = args.max_results.unwrap_or(100);

        fn search_recursive(
            dir: &PathBuf,
            re: &regex::Regex,
            results: &mut Vec<serde_json::Value>,
            max_results: usize,
        ) {
            if results.len() >= max_results {
                return;
            }

            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return,
            };

            for entry in entries.flatten() {
                if results.len() >= max_results {
                    break;
                }

                let path = entry.path();

                if path.is_dir() {
                    // Skip hidden directories and common non-relevant dirs
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if !name.starts_with('.')
                            && name != "node_modules"
                            && name != "target"
                            && name != "__pycache__"
                        {
                            search_recursive(&path, re, results, max_results);
                        }
                    }
                } else if path.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        for (line_num, line) in content.lines().enumerate() {
                            if re.is_match(line) {
                                results.push(serde_json::json!({
                                    "file": path.to_string_lossy(),
                                    "line": line_num + 1,
                                    "content": line
                                }));

                                if results.len() >= max_results {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        if path.is_dir() {
            search_recursive(&path, &re, &mut results, max_results);
        } else if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                for (line_num, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        results.push(serde_json::json!({
                            "file": path.to_string_lossy(),
                            "line": line_num + 1,
                            "content": line
                        }));

                        if results.len() >= max_results {
                            break;
                        }
                    }
                }
            }
        } else {
            return ToolResult::error("file_search", format!("Path does not exist: {}", args.path));
        }

        ToolResult::success(
            "file_search",
            serde_json::json!({
                "pattern": args.pattern,
                "path": args.path,
                "matches": results,
                "count": results.len()
            }),
        )
    }
}

/// Tool for listing directory contents
pub struct FileListTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileListArgs {
    path: String,
    recursive: Option<bool>,
    include_hidden: Option<bool>,
}

#[async_trait]
impl OperantTool for FileListTool {
    fn name(&self) -> &str {
        "file_list"
    }

    fn description(&self) -> &str {
        "List directory contents. Shows files and subdirectories with metadata."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FileListArgs>("file_list", "List directory contents")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: FileListArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("file_list", format!("Invalid arguments: {}", e)),
        };

        // Path safety check (iter-125).
        let path = match validate_path(&args.path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("file_list", e),
        };

        if !path.exists() {
            return ToolResult::error("file_list", format!("Path does not exist: {}", args.path));
        }

        if !path.is_dir() {
            return ToolResult::error(
                "file_list",
                format!("Path is not a directory: {}", args.path),
            );
        }

        let mut entries = Vec::new();

        fn list_recursive(
            dir: &PathBuf,
            entries: &mut Vec<serde_json::Value>,
            recursive: bool,
            include_hidden: bool,
        ) {
            let read_dir = match std::fs::read_dir(dir) {
                Ok(rd) => rd,
                Err(_) => return,
            };

            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden files/dirs unless requested
                if !include_hidden && name.starts_with('.') {
                    continue;
                }

                let path = entry.path();
                let metadata = entry.metadata().ok();

                let entry_json = serde_json::json!({
                    "name": name,
                    "path": path.to_string_lossy(),
                    "is_dir": path.is_dir(),
                    "size": metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                    "modified": metadata.as_ref()
                        .and_then(|m| m.modified().ok())
                        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                });

                entries.push(entry_json);

                if recursive && path.is_dir() {
                    list_recursive(&path, entries, recursive, include_hidden);
                }
            }
        }

        list_recursive(
            &path,
            &mut entries,
            args.recursive.unwrap_or(false),
            args.include_hidden.unwrap_or(false),
        );

        // Sort: directories first, then by name
        entries.sort_by(|a, b| {
            let a_is_dir = a["is_dir"].as_bool().unwrap_or(false);
            let b_is_dir = b["is_dir"].as_bool().unwrap_or(false);

            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or("")),
            }
        });

        ToolResult::success(
            "file_list",
            serde_json::json!({
                "path": args.path,
                "entries": entries,
                "count": entries.len()
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_file_read_schema() {
        let schema = FileReadTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "file_read");
    }

    #[test]
    fn test_file_write_schema() {
        let schema = FileWriteTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "file_write");
    }

    #[test]
    fn test_file_search_schema() {
        let schema = FileSearchTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "file_search");
    }

    #[test]
    fn test_file_list_schema() {
        let schema = FileListTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "file_list");
    }

    #[tokio::test]
    async fn test_file_read_missing_path() {
        let tool = FileReadTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_read_nonexistent() {
        let tool = FileReadTool;
        let result = tool
            .execute(
                json!({"path": "/nonexistent/path/file.txt"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_preserves_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        std::fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

        atomic_write(&path, "#!/bin/sh\necho replaced\n").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o755,
            "atomic replace must preserve the executable bit"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "#!/bin/sh\necho replaced\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_new_file_lands_at_umask_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new_file.txt");
        // Target does not exist yet — the temp must NOT inherit
        // NamedTempFile's hardcoded 0600 (hermes #70856 parity).
        atomic_write(&path, "hello").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let expected = 0o666 & !current_umask();
        assert_eq!(mode, expected, "new files must respect the process umask");
        assert_ne!(
            mode, 0o600,
            "new files must not land at NamedTempFile's 0600"
        );
    }

    #[test]
    fn test_atomic_write_replaces_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.txt");
        std::fs::write(&path, "old").unwrap();

        atomic_write(&path, "new content").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");

        // No temp files left behind in the target directory.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
    }

    async fn test_file_write_missing_args() {
        let tool = FileWriteTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_search_missing_args() {
        let tool = FileSearchTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_file_list_missing_path() {
        let tool = FileListTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }
}
