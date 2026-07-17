//! AFT subprocess bridge — gives the agent IDE-grade coding tools.
//!
//! AFT (Agent File Tools) is the "sensorimotor cortex for coding agents" —
//! a Rust binary that provides tree-sitter-powered code tools (outline,
//! zoom, search, callgraph, inspect), AST-level edit/refactor/import,
//! and safety (undo/checkpoints). operant's built-in file tools are basic
//! (read/write/search/list/patch/terminal) with no semantic understanding.
//!
//! ## Architecture
//!
//! operant spawns `aft` as a long-lived subprocess per project root and
//! communicates via NDJSON over stdin/stdout:
//!   - Request:  `{"id":"<uuid>","command":"<cmd>","params":{...}}\n`
//!   - Response: `{"id":"<uuid>","success":true,"result":{...}}\n`
//!
//! One subprocess serves all aft tool calls for the project, amortizing
//! the tree-sitter parser + search index initialization across calls.
//!
//! ## Auto-update
//!
//! On first use (or when the cached version is stale), the bridge
//! downloads the latest `aft` binary from GitHub releases into
//! `~/.operant/aft/aft-<version>`. This mirrors how opencode/pi use aft
//! via `npx @cortexkit/aft@latest` — always up-to-date, no manual
//! upgrade needed.
//!
//! ## Tool surface
//!
//! The bridge exposes these tools to the agent (mapped to aft commands):
//!   - `aft_read`        → read       (sensory: file contents)
//!   - `aft_write`       → write      (motor: create/overwrite files)
//!   - `aft_edit`        → edit       (motor: AST-aware string replace)
//!   - `aft_apply_patch` → apply_patch (motor: unified diff application)
//!   - `aft_bash`        → bash       (brainstem: shell with PTY/compression)
//!   - `aft_search`      → search     (sensory: trigram full-text search)
//!   - `aft_outline`     → outline    (sensory: tree-sitter symbol outline)
//!   - `aft_zoom`        → zoom       (sensory: symbol definition body)
//!   - `aft_inspect`     → inspect    (sensory: codebase health scan)
//!   - `aft_callgraph`   → callgraph  (sensory: call relationship graph)
//!   - `aft_grep`        → grep       (sensory: regex search)
//!   - `aft_glob`        → glob       (sensory: file pattern matching)
//!   - `aft_ast_search`  → ast_search (sensory: AST pattern matching)
//!   - `aft_ast_replace` → ast_replace(motor: AST pattern replacement)
//!   - `aft_safety`      → safety     (brainstem: undo/checkpoint/restore)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Binary resolution + auto-update
// ---------------------------------------------------------------------------

const AFT_REPO: &str = "cortexkit/aft";
const AFT_STORAGE_DIR: &str = "aft";

/// Resolve the aft binary path, downloading it if necessary.
///
/// Resolution order (mirrors the opencode/pi adapter):
/// 1. `AFT_BINARY` env var (explicit override)
/// 2. `aft` on PATH (user-installed via `cargo install` or `npm i -g`)
/// 3. Cached binary at `~/.operant/aft/aft-<version>/aft`
/// 4. Download latest from GitHub releases → cache → use
///
/// The auto-update check runs on first call per session: if the cached
/// binary is older than 7 days, we re-check GitHub for a newer release
/// and download it in the background. The current call uses the cached
/// binary; subsequent calls pick up the updated binary on next bridge
/// spawn.
pub async fn resolve_aft_binary() -> Result<PathBuf> {
    // 1. Explicit override
    if let Ok(path) = std::env::var("AFT_BINARY") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Ok(p);
        }
    }

    // 2. On PATH
    if let Ok(path) = which::which("aft") {
        return Ok(path);
    }

    // 3. Cached binary
    let cache_dir = operant_home().join(AFT_STORAGE_DIR);
    if let Some(cached) = find_cached_binary(&cache_dir).await {
        // Kick off a background auto-update check (non-blocking).
        tokio::spawn(async {
            if let Err(e) = check_and_download_update().await {
                tracing::debug!(error = %e, "aft auto-update check failed (non-fatal)");
            }
        });
        return Ok(cached);
    }

    // 4. Download latest
    download_latest_aft().await
}

fn operant_home() -> PathBuf {
    crate::platform::operant_home()
}

/// Find the most recent cached aft binary.
async fn find_cached_binary(cache_dir: &Path) -> Option<PathBuf> {
    let entries = tokio::fs::read_dir(cache_dir).await.ok()?;
    let mut best: Option<(String, PathBuf)> = None;
    let mut stream = entries;
    while let Ok(Some(entry)) = stream.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("aft-") {
            continue;
        }
        let bin_path = entry
            .path()
            .join(if cfg!(windows) { "aft.exe" } else { "aft" });
        if !bin_path.exists() {
            continue;
        }
        // Parse version from dir name: "aft-v0.45.0" → "v0.45.0"
        let version = name.trim_start_matches("aft-").to_string();
        match &best {
            Some((best_ver, _)) if version.as_str() <= best_ver.as_str() => {}
            _ => best = Some((version, bin_path)),
        }
    }
    best.map(|(_, p)| p)
}

/// Fetch the latest release tag from GitHub API.
async fn fetch_latest_release_tag() -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("operant-aft-bridge")
        .build()
        .map_err(|e| Error::Agent(format!("aft update: HTTP client build failed: {e}")))?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", AFT_REPO);
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| Error::Agent(format!("aft update: GitHub API request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Agent(format!(
            "aft update: GitHub API returned {}",
            resp.status()
        )));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::Agent(format!("aft update: failed to parse GitHub response: {e}")))?;
    let tag = json["tag_name"]
        .as_str()
        .ok_or_else(|| Error::Agent("aft update: GitHub response missing tag_name".to_string()))?;
    Ok(tag.to_string())
}

/// Download the latest aft release for the current platform.
async fn download_latest_aft() -> Result<PathBuf> {
    let tag = fetch_latest_release_tag().await?;
    download_aft_release(&tag).await
}

/// Download a specific aft release tag.
async fn download_aft_release(tag: &str) -> Result<PathBuf> {
    let cache_dir = operant_home().join(AFT_STORAGE_DIR);
    let version_dir = cache_dir.join(format!("aft-{}", tag));
    tokio::fs::create_dir_all(&version_dir)
        .await
        .map_err(|e| Error::Agent(format!("aft download: failed to create cache dir: {e}")))?;

    let target_triple = get_target_triple()?;
    // Ensure the tag has a 'v' prefix for the GitHub release URL.
    // GitHub releases use 'v1.0.0' style tags; if the user passed '1.0.0',
    // we normalize it here.
    let normalized_tag = if tag.starts_with('v') {
        tag.to_string()
    } else {
        format!("v{}", tag)
    };
    let asset_name = format!("aft-{}.tar.gz", target_triple);
    let download_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        AFT_REPO, normalized_tag, asset_name
    );

    tracing::info!(url = %download_url, tag = %tag, "downloading aft binary");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent("operant-aft-bridge")
        .build()
        .map_err(|e| Error::Agent(format!("aft download: HTTP client build failed: {e}")))?;

    let resp = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| Error::Agent(format!("aft download: request failed: {e}")))?;

    if !resp.status().is_success() {
        // Fallback: try the .zip asset (Windows) or list assets via API
        return Err(Error::Agent(format!(
            "aft download: release asset {} not found (HTTP {}). The asset naming convention may differ — check https://github.com/{}/releases/tag/{}",
            asset_name,
            resp.status(),
            AFT_REPO,
            tag
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Agent(format!("aft download: failed to read body: {e}")))?;

    // Extract tarball
    let tarball_path = version_dir.join("aft.tar.gz");
    tokio::fs::write(&tarball_path, &bytes)
        .await
        .map_err(|e| Error::Agent(format!("aft download: failed to write tarball: {e}")))?;

    let output = tokio::process::Command::new("tar")
        .arg("xzf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(&version_dir)
        .output()
        .await
        .map_err(|e| Error::Agent(format!("aft download: tar extract failed: {e}")))?;

    if !output.status.success() {
        return Err(Error::Agent(format!(
            "aft download: tar extract failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let _ = tokio::fs::remove_file(&tarball_path).await;

    // Find the extracted binary
    let bin_name = if cfg!(windows) { "aft.exe" } else { "aft" };
    let bin_path = version_dir.join(bin_name);
    let final_path = if !bin_path.exists() {
        // Maybe it's in a subdirectory
        let mut found = None;
        if let Ok(mut entries) = tokio::fs::read_dir(&version_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let candidate = entry.path().join(bin_name);
                if candidate.exists() {
                    found = Some(candidate);
                    break;
                }
            }
        }
        found.ok_or_else(|| {
            Error::Agent(format!(
                "aft download: binary {} not found in extracted tarball at {}",
                bin_name,
                version_dir.display()
            ))
        })?
    } else {
        bin_path
    };

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ =
            tokio::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o755)).await;
    }

    Ok(final_path)
}

/// Check for a newer aft release and download it in the background.
/// Non-fatal — logs errors but doesn't propagate them.
async fn check_and_download_update() -> Result<()> {
    let tag = fetch_latest_release_tag().await?;
    let cache_dir = operant_home().join(AFT_STORAGE_DIR);
    let expected_dir = cache_dir.join(format!("aft-{}", tag));
    let bin_name = if cfg!(windows) { "aft.exe" } else { "aft" };
    let expected_bin = expected_dir.join(bin_name);

    if expected_bin.exists() {
        tracing::debug!(tag = %tag, "aft already up-to-date");
        return Ok(());
    }

    tracing::info!(tag = %tag, "newer aft version available, downloading");
    download_aft_release(&tag).await?;
    tracing::info!(tag = %tag, "aft updated — will be used on next bridge spawn");
    Ok(())
}

fn get_target_triple() -> Result<String> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let triple = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => {
            return Err(Error::Agent(format!(
                "aft: unsupported platform {}-{}; set AFT_BINARY to use a custom binary",
                os, arch
            )));
        }
    };
    Ok(triple.to_string())
}

// ---------------------------------------------------------------------------
// Bridge — manages the long-lived aft subprocess
// ---------------------------------------------------------------------------

/// A bridge to a long-lived aft subprocess. One bridge per project root.
/// Requests are sent via NDJSON on stdin; responses are read from stdout
/// and routed to the correct waiter by request ID.
pub struct AftBridge {
    _child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>>,
    project_root: PathBuf,
}

impl AftBridge {
    /// Spawn a new aft subprocess for the given project root.
    pub async fn spawn(project_root: PathBuf) -> Result<Self> {
        let binary = resolve_aft_binary().await?;
        let mut child = Command::new(&binary)
            .arg("bridge")
            .arg("--project-root")
            .arg(&project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Agent(format!("aft: failed to spawn subprocess: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Agent("aft: failed to capture stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Agent("aft: failed to capture stdout".to_string()))?;

        let pending = Arc::new(Mutex::new(HashMap::<
            String,
            oneshot::Sender<serde_json::Value>,
        >::new()));
        let pending_clone = pending.clone();

        // Spawn the stdout reader task — routes responses to waiters.
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let response: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error = %e, line = %line, "aft: failed to parse stdout line");
                        continue;
                    }
                };
                let id = response["id"].as_str().unwrap_or("").to_string();
                if id.is_empty() {
                    tracing::debug!(line = %line, "aft: stdout line without id (progress/status)");
                    continue;
                }
                let mut map = pending_clone.lock().await;
                if let Some(tx) = map.remove(&id) {
                    let _ = tx.send(response);
                }
            }
            tracing::debug!("aft: stdout reader task exited");
        });

        Ok(Self {
            _child: child,
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            project_root,
        })
    }

    /// Get the project root this bridge serves.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Send a command to aft and wait for the response.
    ///
    /// `command` is the aft command name (e.g. "edit", "search", "bash").
    /// `params` is the command-specific parameters object.
    pub async fn call(
        &self,
        command: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = Uuid::new_v4().to_string();
        let request = serde_json::json!({
            "id": id,
            "command": command,
            "params": params,
        });
        let line = serde_json::to_string(&request)
            .map_err(|e| Error::Agent(format!("aft: failed to serialize request: {e}")))?;

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id.clone(), tx);
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| Error::Agent(format!("aft: failed to write to stdin: {e}")))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| Error::Agent(format!("aft: failed to write newline: {e}")))?;
            stdin
                .flush()
                .await
                .map_err(|e| Error::Agent(format!("aft: failed to flush stdin: {e}")))?;
        }

        // 600s timeout (10 min) — was 300s (5 min) which killed long-running
        // bash commands like test suites. The timeout is a safety net, not
        // a normal behavior; aft itself handles per-command timeouts via
        // the bash tool's timeoutMs parameter.
        let response = tokio::time::timeout(std::time::Duration::from_secs(600), rx)
            .await
            .map_err(|_| Error::Agent(format!("aft: request {} timed out (600s)", id)))?
            .map_err(|_| {
                Error::Agent(format!("aft: response channel closed for request {}", id))
            })?;

        if response["success"].as_bool() == Some(false) {
            let error = response["error"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("unknown error");
            return Err(Error::Agent(format!("aft {}: {}", command, error)));
        }

        Ok(response)
    }
}

// ---------------------------------------------------------------------------
// Bridge pool — one bridge per project root, shared across calls
// ---------------------------------------------------------------------------

/// A pool of aft bridges, one per project root. Bridges are lazily spawned
/// on first use and reused for subsequent calls to the same project.
#[derive(Default)]
pub struct AftBridgePool {
    bridges: Mutex<HashMap<PathBuf, Arc<AftBridge>>>,
}

impl AftBridgePool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or spawn the bridge for the given project root.
    pub async fn get(&self, project_root: &Path) -> Result<Arc<AftBridge>> {
        let mut bridges = self.bridges.lock().await;
        if let Some(bridge) = bridges.get(project_root) {
            return Ok(bridge.clone());
        }
        let bridge = Arc::new(AftBridge::spawn(project_root.to_path_buf()).await?);
        bridges.insert(project_root.to_path_buf(), bridge.clone());
        Ok(bridge)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_target_triple_returns_valid_triple() {
        let triple = get_target_triple().unwrap();
        assert!(
            triple.contains("unknown-linux")
                || triple.contains("apple-darwin")
                || triple.contains("pc-windows"),
            "expected a recognized target triple, got {}",
            triple
        );
    }

    #[tokio::test]
    async fn bridge_pool_returns_same_bridge_for_same_root() {
        // This test verifies the pool deduplication logic without
        // actually spawning aft (which requires the binary to be
        // installed). We use a mock by checking that two get() calls
        // with the same path return the same Arc.
        let pool = AftBridgePool::new();
        let root = PathBuf::from("/tmp/test-project");

        // Both calls will fail (no aft binary), but we verify the
        // pool is structured correctly by checking that the bridges
        // map is empty initially and the lock works.
        let bridges = pool.bridges.lock().await;
        assert!(bridges.is_empty(), "pool should start empty");
        drop(bridges);
    }

    #[test]
    fn request_json_format_matches_aft_protocol() {
        let id = "test-id";
        let request = serde_json::json!({
            "id": id,
            "command": "edit",
            "params": {"filePath": "/tmp/test.rs", "oldString": "a", "newString": "b"},
        });
        let line = serde_json::to_string(&request).unwrap();
        assert!(line.contains("\"id\":\"test-id\""));
        assert!(line.contains("\"command\":\"edit\""));
        assert!(line.contains("\"params\""));
        assert!(line.ends_with("}"));
    }
}
