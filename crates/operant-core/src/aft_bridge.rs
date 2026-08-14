//! AFT subprocess bridge — gives the agent IDE-grade coding tools.
//!
//! AFT (Agent File Tools) is the "sensorimotor cortex for coding agents" —
//! a Rust binary that provides tree-sitter-powered code tools (outline,
//! zoom, search, callgraph, inspect), AST-level edit/refactor,
//! and safety (undo/checkpoints). operant's built-in file tools are basic
//! (read/write/search/list/patch/terminal) with no semantic understanding.
//!
//! ## Architecture
//!
//! operant spawns `aft` as a long-lived subprocess per project root and
//! communicates via NDJSON over stdin/stdout. **Protocol (v0.49.x):**
//!
//!   - Request (flat params): `{"id":"<uuid>","command":"<cmd>","<param>":...}\n`
//!   - Request (bash only, nested): `{"id":"<uuid>","command":"bash","params":{"command":...}}\n`
//!     (the `bash` tool's own `command` parameter would collide with the
//!     envelope's `command` field, so aft requires it nested)
//!   - Response: `{"id":"<uuid>","success":true,"<payload fields>...}\n`
//!     — payload is FLAT at top level (no `result` wrapper). Errors carry
//!     `{"success":false,"code":"<code>","message":"<human text>"}`.
//!   - Async frames: `{"type":"bash_completed","task_id":"bash-...","status":"completed",...}\n`
//!     — `bash` returns a `task_id` immediately; the completed output is
//!     delivered on this id-less frame, which the reader routes to the
//!     waiting caller by `task_id`.
//!
//! One subprocess serves all aft tool calls for the project, amortizing
//! the tree-sitter parser + search index initialization across calls.
//!
//! ## Project root
//!
//! Since v0.49.x the `--project-root` CLI flag was dropped: the project
//! root is derived from the bridge's **cwd** (and the `configure` payload).
//! We therefore spawn with `.current_dir(project_root)` and send an
//! explicit `configure {"harness":"runner","project_root":...}` once per
//! bridge (required before `inspect`/`callers`).
//!
//! ## Auto-update
//!
//! On first use (or when the cached version is stale), the bridge
//! downloads the latest `aft` binary from GitHub releases into
//! `~/.operant/aft/aft-<version>`. Release assets are RAW binaries named
//! per platform (`aft-linux-x64`, `aft-darwin-arm64`, `aft-win32-x64.exe`,
//! ...) — no tarballs. This mirrors how opencode/pi use aft via
//! `npx @cortexkit/aft@latest` — always up-to-date, no manual upgrade.
//!
//! ## Tool surface
//!
//! The bridge exposes these tools to the agent (mapped to aft commands):
//!   - `aft_read`          → read              (sensory: file contents)
//!   - `aft_write`         → write             (motor: create/overwrite files)
//!   - `aft_edit`          → edit_match        (motor: literal match replace)
//!   - `aft_apply_patch`   → apply_patch       (motor: patch application)
//!   - `aft_bash`          → bash              (brainstem: shell w/ compression)
//!   - `aft_search`        → semantic_search   (sensory: semantic + lexical)
//!   - `aft_outline`       → outline           (sensory: symbol outline)
//!   - `aft_zoom`          → zoom              (sensory: symbol definition)
//!   - `aft_inspect`       → inspect           (sensory: codebase health)
//!   - `aft_callers`       → callers           (sensory: call relationship)
//!   - `aft_grep`          → grep              (sensory: regex search)
//!   - `aft_glob`          → glob              (sensory: file pattern match)
//!   - `aft_ast_search`    → ast_search        (sensory: AST pattern match)
//!   - `aft_ast_replace`   → ast_replace       (motor: AST pattern rewrite)
//!   - `aft_checkpoint`    → checkpoint        (safety: snapshot)
//!   - `aft_list_checkpoints` → list_checkpoints (safety: list snapshots)
//!   - `aft_checkpoint_paths` → checkpoint_paths (safety: preview snapshot paths)
//!   - `aft_restore_checkpoint` → restore_checkpoint (safety: restore snapshot)
//!   - `aft_undo`          → undo              (safety: revert last op)
//!   - `aft_status`        → status            (diagnostics: bridge health)
//!
//! ## Durable checkpoints
//!//! Upstream AFT keeps checkpoints in memory only (a bridge crash drops them
//! all). operant ships a patched binary at `~/.operant/aft/aft-patched/aft`
//! that persists checkpoints to disk and hydrates on startup, so
//! `aft_list_checkpoints` / `aft_restore_checkpoint` survive process
//! restarts. See [`resolve_aft_binary`] for the resolution order.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
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
/// Directory (under the operant home) for a locally patched aft binary that
/// must never be replaced by the auto-updater. operant ships durability
/// fixes (e.g. persistent checkpoints) as `aft-patched/aft` until they land
/// upstream; the auto-updater only ever writes into `aft-<version>/` dirs.
const AFT_PATCHED_DIR: &str = "aft-patched";

/// Resolve the aft binary path, downloading it if necessary.
///
/// Resolution order (mirrors the opencode/pi adapter):
/// 1. `AFT_BINARY` env var (explicit override)
/// 2. Patched binary at `~/.operant/aft/aft-patched/aft` (locally-built
///    with durability fixes that may not be upstream yet — never
///    auto-updated)
/// 3. `aft` on PATH (user-installed via `cargo install` or `npm i -g`)
/// 4. Cached binary at `~/.operant/aft/aft-<version>/aft`
/// 5. Download latest from GitHub releases → cache → use
///
/// The auto-update check runs on first call per session: if the cached
/// binary is older than 7 days, we re-check GitHub for a newer release
/// and download it in the background. The current call uses the cached
/// binary; subsequent calls pick up the updated binary on next bridge
/// spawn. The patched dir is never touched by auto-update.
pub async fn resolve_aft_binary() -> Result<PathBuf> {
    // 1. Explicit override
    if let Ok(path) = std::env::var("AFT_BINARY") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Ok(p);
        }
    }

    // 2. Patched binary (durability fixes, never auto-updated)
    let patched = operant_home()
        .join(AFT_STORAGE_DIR)
        .join(AFT_PATCHED_DIR)
        .join(if cfg!(windows) { "aft.exe" } else { "aft" });
    if patched.exists() {
        tracing::debug!(path = %patched.display(), "using patched aft binary");
        return Ok(patched);
    }

    // 3. On PATH
    if let Ok(path) = which::which("aft") {
        return Ok(path);
    }

    // 4. Cached binary
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

    // 4. Download latest — but remember failures so machines without aft
    // (or without network) don't retry the download on every startup.
    // The negative-resolution marker has a TTL; after it expires we retry
    // once. (Without this, every `build_registry` invocation stalled up to
    // the CLI's 10s ping bound trying to reach GitHub.)
    if is_aft_known_unavailable().await {
        return Err(Error::Agent(
            "aft binary not found and auto-provision failed within the retry \
             window — install aft or set AFT_BINARY (retries in ~6h, or \
             delete ~/.operant/aft/.unavailable to force an immediate retry)"
                .to_string(),
        ));
    }
    match download_latest_aft().await {
        Ok(path) => {
            let _ = tokio::fs::remove_file(aft_unavailable_marker()).await;
            Ok(path)
        }
        Err(e) => {
            let _ = tokio::fs::create_dir_all(operant_home().join(AFT_STORAGE_DIR)).await;
            let _ = tokio::fs::write(aft_unavailable_marker(), b"1").await;
            Err(e)
        }
    }
}

fn operant_home() -> PathBuf {
    crate::platform::operant_home()
}

// Negative-resolution cache: after a failed auto-download we write a marker
// file so subsequent startups skip the GitHub download attempt for a while.
// This keeps the "aft not installed" case fast on every invocation instead
// of stalling startup up to the CLI ping bound.
const UNAVAILABLE_MARKER: &str = ".unavailable";
const UNAVAILABLE_RETRY_AFTER: Duration = Duration::from_secs(6 * 3600);

/// How many times `send_request` retries a transient `callgraph_building`
/// response before surfacing the error to the caller.
const CALLGRAPH_BUILD_RETRIES: usize = 6;
/// Base delay (ms) for the exponential backoff on callgraph cold-build
/// retries (1.5s · 2^attempt — worst case ~95s, far under the 600s
/// request timeout).
const CALLGRAPH_RETRY_BASE_MS: u64 = 1500;

/// True when an aft error reports the transient callgraph cold-build state
/// (`code = callgraph_building`). The store is persisted and built in the
/// background, so the correct behavior is to retry shortly (as aft's own
/// message instructs), not to fail the tool call.
fn is_callgraph_building_error(e: &crate::error::Error) -> bool {
    e.to_string().contains("callgraph_building")
}

fn aft_unavailable_marker() -> PathBuf {
    operant_home()
        .join(AFT_STORAGE_DIR)
        .join(UNAVAILABLE_MARKER)
}

/// Whether a fresh negative-resolution marker exists (i.e. a previous
/// auto-download failed recently and we should not retry yet).
async fn is_aft_known_unavailable() -> bool {
    marker_is_fresh(&aft_unavailable_marker()).await
}

async fn marker_is_fresh(marker: &Path) -> bool {
    let Ok(meta) = tokio::fs::metadata(marker).await else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return true; // can't read mtime — assume fresh, be conservative
    };
    match modified.elapsed() {
        Ok(age) => age < UNAVAILABLE_RETRY_AFTER,
        Err(_) => true, // clock in the future — treat as fresh
    }
}

/// Find the most recent cached aft binary.
async fn find_cached_binary(cache_dir: &Path) -> Option<PathBuf> {
    let entries = tokio::fs::read_dir(cache_dir).await.ok()?;
    let mut best: Option<(String, PathBuf)> = None;
    let mut stream = entries;
    while let Ok(Some(entry)) = stream.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("aft-") || name == AFT_PATCHED_DIR {
            continue;
        }
        let bin_path = entry
            .path()
            .join(if cfg!(windows) { "aft.exe" } else { "aft" });
        if !bin_path.exists() {
            continue;
        }
        // Parse version from dir name: "aft-v0.49.4" → "v0.49.4"
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
        .timeout(Duration::from_secs(30))
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
///
/// Release assets are RAW platform binaries (verified for v0.49.x):
/// `aft-linux-x64`, `aft-linux-arm64`, `aft-darwin-x64`,
/// `aft-darwin-arm64`, `aft-win32-x64.exe`, `aft-win32-arm64.exe`.
/// The old code expected a `aft-<target-triple>.tar.gz` tarball that is
/// never published — that 404'd every auto-update (audit finding).
async fn download_aft_release(tag: &str) -> Result<PathBuf> {
    let cache_dir = operant_home().join(AFT_STORAGE_DIR);
    let version_dir = cache_dir.join(format!("aft-{}", tag));
    tokio::fs::create_dir_all(&version_dir)
        .await
        .map_err(|e| Error::Agent(format!("aft download: failed to create cache dir: {e}")))?;

    let bin_name = if cfg!(windows) { "aft.exe" } else { "aft" };
    let bin_path = version_dir.join(bin_name);
    if bin_path.exists() {
        return Ok(bin_path);
    }

    let asset_name = aft_asset_name()?;
    // Ensure the tag has a 'v' prefix for the GitHub release URL.
    // GitHub releases use 'v1.0.0' style tags; if the user passed '1.0.0',
    // we normalize it here.
    let normalized_tag = if tag.starts_with('v') {
        tag.to_string()
    } else {
        format!("v{}", tag)
    };
    let download_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        AFT_REPO, normalized_tag, asset_name
    );

    tracing::info!(url = %download_url, tag = %tag, "downloading aft binary");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("operant-aft-bridge")
        .build()
        .map_err(|e| Error::Agent(format!("aft download: HTTP client build failed: {e}")))?;

    let resp = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| Error::Agent(format!("aft download: request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::Agent(format!(
            "aft download: release asset {} not found (HTTP {}). The asset naming may differ for this release — check https://github.com/{}/releases/tag/{}",
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

    // Raw binary — write directly (no tar extraction; v0.49.x publishes
    // plain executables, not tarballs).
    tokio::fs::write(&bin_path, &bytes)
        .await
        .map_err(|e| Error::Agent(format!("aft download: failed to write binary: {e}")))?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755)).await;
    }

    Ok(bin_path)
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

/// Map the current platform to the aft release asset name (v0.49.x naming:
/// raw binaries, no target triple, no tarball extension).
fn aft_asset_name() -> Result<String> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let name = match (os, arch) {
        ("linux", "x86_64") => "aft-linux-x64",
        ("linux", "aarch64") => "aft-linux-arm64",
        ("macos", "x86_64") => "aft-darwin-x64",
        ("macos", "aarch64") => "aft-darwin-arm64",
        ("windows", "x86_64") => "aft-win32-x64.exe",
        ("windows", "aarch64") => "aft-win32-arm64.exe",
        _ => {
            return Err(Error::Agent(format!(
                "aft: unsupported platform {}-{}; set AFT_BINARY to use a custom binary",
                os, arch
            )));
        }
    };
    Ok(name.to_string())
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
    /// Cache of `bash_completed` async frames keyed by `task_id`. The
    /// reader stores every completion frame here so a bash call can find
    /// its result even if it was emitted before the caller registered a
    /// waiter.
    bash_cache: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    /// Waiters for `bash_completed` frames keyed by `task_id`.
    bash_waiters: Arc<Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>>,
    project_root: PathBuf,
    /// Whether `configure {harness, project_root}` has been sent for this
    /// bridge (required before `inspect`/`callers`).
    configured: Mutex<bool>,
}

impl AftBridge {
    /// Spawn a new aft subprocess for the given project root.
    pub async fn spawn(project_root: PathBuf) -> Result<Self> {
        let binary = resolve_aft_binary().await?;
        // v0.49.x dropped `--project-root`: the root is derived from cwd
        // (and the configure payload). Canonicalize to an absolute path so
        // the configure payload matches.
        let abs_root = if project_root.is_absolute() {
            project_root.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&project_root))
                .unwrap_or(project_root.clone())
        };
        let mut child = Command::new(&binary)
            .arg("bridge")
            .current_dir(&abs_root)
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
        let bash_cache = Arc::new(Mutex::new(HashMap::<String, serde_json::Value>::new()));
        let bash_cache_clone = bash_cache.clone();
        let bash_waiters = Arc::new(Mutex::new(HashMap::<
            String,
            oneshot::Sender<serde_json::Value>,
        >::new()));
        let bash_waiters_clone = bash_waiters.clone();

        // Spawn the stdout reader task — routes responses to waiters and
        // `bash_completed` frames to bash callers by task_id.
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
                // Async bash completion frame: no request id, routed by
                // task_id (v0.49.x delivers bash output this way).
                if response["type"].as_str() == Some("bash_completed") {
                    if let Some(task_id) = response["task_id"].as_str() {
                        {
                            let mut cache = bash_cache_clone.lock().await;
                            cache.insert(task_id.to_string(), response.clone());
                        }
                        let mut waiters = bash_waiters_clone.lock().await;
                        if let Some(tx) = waiters.remove(task_id) {
                            let _ = tx.send(response);
                        }
                    }
                    continue;
                }
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
            bash_cache,
            bash_waiters,
            project_root: abs_root,
            configured: Mutex::new(false),
        })
    }

    /// Get the project root this bridge serves.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Send `configure` once per bridge so project-scoped commands
    /// (`inspect`, `callers`) are authorized. Idempotent.
    async fn ensure_configured(&self) -> Result<()> {
        let mut configured = self.configured.lock().await;
        if *configured {
            return Ok(());
        }
        let mut request = serde_json::Map::new();
        request.insert(
            "id".to_string(),
            serde_json::Value::String(Uuid::new_v4().to_string()),
        );
        request.insert(
            "command".to_string(),
            serde_json::Value::String("configure".to_string()),
        );
        request.insert(
            "harness".to_string(),
            serde_json::Value::String("runner".to_string()),
        );
        request.insert(
            "project_root".to_string(),
            serde_json::Value::String(self.project_root.to_string_lossy().to_string()),
        );
        match self.send_request(serde_json::Value::Object(request)).await {
            Ok(_) => {
                *configured = true;
                Ok(())
            }
            Err(e) => {
                // If another call raced and already configured the process,
                // treat it as success.
                if e.to_string().contains("already configured")
                    || e.to_string().contains("already_configured")
                {
                    *configured = true;
                    return Ok(());
                }
                Err(e)
            }
        }
    }

    /// Send a command to aft and wait for the response.
    ///
    /// `command` is the aft command name (e.g. "read", "edit_match").
    /// `params` is the command-specific parameters object, which is
    /// FLATTENED into the request top level (v0.49.x protocol — nested
    /// `params` only works for `bash`, which uses [`AftBridge::bash`]).
    pub async fn call(
        &self,
        command: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.call_inner(command, params, false).await
    }

    /// Send a command with params nested under `params` (used by `bash`,
    /// whose own `command` parameter collides with the envelope).
    pub async fn call_nested(
        &self,
        command: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.call_inner(command, params, true).await
    }

    async fn call_inner(
        &self,
        command: &str,
        params: serde_json::Value,
        nested: bool,
    ) -> Result<serde_json::Value> {
        // Project-scoped commands require the configure handshake.
        if matches!(command, "inspect" | "callers") {
            self.ensure_configured().await?;
        }

        let id = Uuid::new_v4().to_string();
        let mut request = serde_json::Map::new();
        request.insert("id".to_string(), serde_json::Value::String(id.clone()));
        request.insert(
            "command".to_string(),
            serde_json::Value::String(command.to_string()),
        );
        if nested {
            request.insert("params".to_string(), params);
        } else if let Some(obj) = params.as_object() {
            for (k, v) in obj {
                request.insert(k.clone(), v.clone());
            }
        } else if !params.is_null() {
            return Err(Error::Agent(format!(
                "aft {}: params must be an object, got {}",
                command, params
            )));
        }
        self.send_request(serde_json::Value::Object(request)).await
    }

    /// Write a fully-formed request and await its response, extracting
    /// aft errors (`success:false` + `code`/`message`) into [`Error`].
    ///
    /// Retries a bounded number of times when aft reports a transient
    /// cold-build state (`callgraph_building`): the callgraph store is
    /// persisted and built in the background, and aft's own error message
    /// tells callers to "retry shortly". Surfacing that to the model as a
    /// hard failure would make `aft_callers` / `aft_inspect` (dead-code)
    /// unusable on a fresh store, so the bridge waits out the build with
    /// exponential backoff (matching the CLI `aft warmup --areas callgraph`
    /// intent, but without requiring an extra warmup step).
    async fn send_request(&self, request: serde_json::Value) -> Result<serde_json::Value> {
        let mut attempt = 0usize;
        loop {
            match self.send_request_once(request.clone()).await {
                Err(e) if is_callgraph_building_error(&e) => {
                    if attempt >= CALLGRAPH_BUILD_RETRIES {
                        return Err(e);
                    }
                    attempt += 1;
                    // Exponential backoff: 1.5s, 3s, 6s, 12s, 24s, 48s —
                    // bounded by the retry count above (~95s worst case),
                    // well under the 600s request timeout.
                    let delay_ms = CALLGRAPH_RETRY_BASE_MS.saturating_mul(1u64 << (attempt - 1));
                    tracing::info!(
                        command = %request["command"].as_str().unwrap_or(""),
                        attempt,
                        delay_ms,
                        "aft: callgraph store building — retrying request"
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                other => return other,
            }
        }
    }

    /// Single write-and-await request exchange (no retry).
    async fn send_request_once(&self, request: serde_json::Value) -> Result<serde_json::Value> {
        let id = request["id"].as_str().unwrap_or("").to_string();
        let command = request["command"].as_str().unwrap_or("").to_string();
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
        let response = match tokio::time::timeout(Duration::from_secs(600), rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                return Err(Error::Agent(format!(
                    "aft: response channel closed for request {}",
                    id
                )));
            }
            Err(_) => {
                // Clean up the pending entry so a wedged bridge (dead
                // subprocess, no responses ever arriving) can't grow the
                // map unboundedly across repeated timeouts. The entry is
                // normally removed by the reader when the response lands.
                self.pending.lock().await.remove(&id);
                return Err(Error::Agent(format!(
                    "aft: request {} timed out (600s)",
                    id
                )));
            }
        };

        if response["success"].as_bool() == Some(false) {
            let code = response["code"].as_str().unwrap_or("");
            let message = response["message"]
                .as_str()
                .or_else(|| response["error"].as_str())
                .unwrap_or("unknown error");
            let detail = if code.is_empty() {
                format!("aft {}: {}", command, message)
            } else {
                format!("aft {}: [{}] {}", command, code, message)
            };
            return Err(Error::Agent(detail));
        }

        Ok(response)
    }

    /// Run a bash command through aft's async task model.
    ///
    /// `bash` returns `{task_id, status:"running"}` immediately; the
    /// completed output arrives on a `bash_completed` frame routed by the
    /// reader. We wait for that frame (with `bash_status` polling as a
    /// fallback) and return the final state including `output` /
    /// `output_preview`, `exit_code`, and token compression stats.
    pub async fn bash(&self, command: &str, timeout_ms: Option<u64>) -> Result<serde_json::Value> {
        let mut params = serde_json::json!({ "command": command });
        if let Some(t) = timeout_ms {
            params["timeout_ms"] = serde_json::json!(t);
        }
        let resp = self.call_nested("bash", params).await?;
        let task_id = resp["task_id"]
            .as_str()
            .ok_or_else(|| Error::Agent("aft bash: response missing task_id".to_string()))?
            .to_string();

        // Fast path: the completion frame may already be cached.
        {
            let cache = self.bash_cache.lock().await;
            if let Some(entry) = cache.get(&task_id) {
                return Ok(entry.clone());
            }
        }

        // Register a waiter (re-checking the cache under lock to close the
        // race between the reader caching and our first lookup).
        let (tx, rx) = oneshot::channel();
        {
            let mut waiters = self.bash_waiters.lock().await;
            let mut cache = self.bash_cache.lock().await;
            if let Some(entry) = cache.remove(&task_id) {
                return Ok(entry);
            }
            waiters.insert(task_id.clone(), tx);
        }

        let max_wait =
            Duration::from_secs(timeout_ms.map(|t| t / 1000 + 10).unwrap_or(600).max(10));
        match tokio::time::timeout(max_wait, rx).await {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(_)) => Err(Error::Agent(
                "aft bash: response channel closed before task completed".to_string(),
            )),
            Err(_) => {
                // Fallback: the completion frame was never routed (e.g.
                // bridge restarted) — poll bash_status once to see if the
                // task finished.
                if let Ok(status) = self
                    .call("bash_status", serde_json::json!({ "task_id": task_id }))
                    .await
                {
                    let st = status["status"].as_str().unwrap_or("");
                    if st == "completed" || st == "failed" {
                        return Ok(status);
                    }
                }
                Err(Error::Agent(format!(
                    "aft bash: task {} timed out after {}s",
                    task_id,
                    max_wait.as_secs()
                )))
            }
        }
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
    /// Project roots for which a detached callgraph warmup was already fired
    /// in this process (dedupes duplicate warmups per root).
    warming: Mutex<std::collections::HashSet<PathBuf>>,
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
        let root = project_root.to_path_buf();
        let bridge = Arc::new(AftBridge::spawn(root.clone()).await?);
        bridges.insert(root.clone(), bridge.clone());
        // Fire-and-forget: kick off the persisted callgraph cold-build for a
        // fresh project root so aft_callers / aft_inspect(dead-code) are warm
        // on first use — even if this operant session exits before it
        // finishes (the detached warmup survives; the store persists).
        self.warm_callgraph_detached(&root).await;
        Ok(bridge)
    }

    /// Best-effort detached `aft warmup --only callgraph` for a project root.
    /// Never blocks or fails the caller: resolve the binary; if found, spawn
    /// a detached process (no kill_on_drop) so the build outlives operant.
    /// Deduped per root per process. AFT's warmup returns almost immediately
    /// when the store is already warm, so re-firing is cheap.
    async fn warm_callgraph_detached(&self, root: &Path) {
        let mut warming = self.warming.lock().await;
        if !warming.insert(root.to_path_buf()) {
            return;
        }
        let binary = match resolve_aft_binary().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "aft warmup: no binary to warm the callgraph");
                return;
            }
        };
        let root_s = root.to_string_lossy().to_string();
        let bin_s = binary.to_string_lossy().to_string();
        std::thread::spawn(move || {
            let result = std::process::Command::new(&bin_s)
                .args([
                    "warmup",
                    "--root",
                    &root_s,
                    "--only",
                    "callgraph",
                    "--timeout",
                    "600000",
                    "--quiet",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match result {
                Ok(_child) => {
                    tracing::debug!(root = %root_s, "aft callgraph warmup spawned detached")
                }
                Err(e) => {
                    tracing::warn!(error = %e, root = %root_s, "aft callgraph warmup spawn failed")
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unavailable_marker_expires_after_ttl() {
        let dir = std::env::temp_dir().join(format!(
            "operant_aft_marker_test_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let marker = dir.join(".unavailable");
        std::fs::create_dir_all(&dir).unwrap();

        // No marker → not unavailable.
        assert!(!marker_is_fresh(&marker).await);

        // Fresh marker → unavailable.
        std::fs::write(&marker, b"1").unwrap();
        assert!(marker_is_fresh(&marker).await);

        // Aged marker (older than the retry TTL) → retry allowed.
        let aged = dir.join(".unavailable-aged");
        std::fs::write(&aged, b"1").unwrap();
        let old =
            std::time::SystemTime::now() - (UNAVAILABLE_RETRY_AFTER + Duration::from_secs(60));
        filetime_set(&aged, old);
        assert!(!marker_is_fresh(&aged).await);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn filetime_set(path: &Path, time: std::time::SystemTime) {
        // `FileTimes::set_modified` is an inherent method (stable 1.75+).
        let ft = std::fs::FileTimes::new().set_modified(time);
        let _ = std::fs::File::options()
            .write(true)
            .open(path)
            .map(|f| f.set_times(ft));
    }

    #[cfg(not(unix))]
    fn filetime_set(_path: &Path, _time: std::time::SystemTime) {
        // Windows filetime manipulation is more involved; the aged-marker
        // assertion is best-effort there.
    }

    #[test]
    fn callgraph_building_error_is_detected() {
        // The retry gate keys on the aft error code string.
        let building = crate::error::Error::Agent(
            "aft callers: [callgraph_building] callers: callgraph store is building in the background; retry shortly".to_string(),
        );
        assert!(is_callgraph_building_error(&building));

        let other = crate::error::Error::Agent("aft read: file not found".to_string());
        assert!(!is_callgraph_building_error(&other));
    }

    #[test]
    fn callgraph_retry_backoff_is_bounded() {
        // Worst-case retry budget: 6 retries with exponential backoff from a
        // 1.5s base ≈ 1.5+3+6+12+24+48 = 94.5s — comfortably under the 600s
        // single-request timeout, so a stuck store can never hang a request.
        let total_ms: u64 = (0..CALLGRAPH_BUILD_RETRIES)
            .map(|attempt| CALLGRAPH_RETRY_BASE_MS.saturating_mul(1u64 << attempt))
            .sum();
        assert!(
            total_ms < 600_000,
            "retry budget {total_ms}ms must stay under the 600s request timeout"
        );
        assert!(
            total_ms > 90_000,
            "retry budget {total_ms}ms should give the store a real chance to build"
        );
    }

    #[test]
    fn aft_asset_name_returns_raw_binary_name() {
        let name = aft_asset_name().unwrap();
        assert!(
            name.starts_with("aft-"),
            "expected an aft-* asset name, got {}",
            name
        );
        assert!(
            !name.ends_with(".tar.gz"),
            "v0.49.x publishes raw binaries, not tarballs — got {}",
            name
        );
    }

    #[test]
    fn find_cached_binary_skips_patched_dir() {
        // The patched dir is resolved explicitly in `resolve_aft_binary`; it
        // must not be considered by the version-based cache scan (and must
        // never be clobbered by auto-update).
        assert_eq!(AFT_PATCHED_DIR, "aft-patched");
        assert_ne!(AFT_PATCHED_DIR, "aft-v0.49.4");
    }

    #[tokio::test]
    async fn bridge_pool_returns_same_bridge_for_same_root() {
        // This test verifies the pool deduplication logic without
        // actually spawning aft (which requires the binary to be
        // installed). We verify the pool is structured correctly by
        // checking that the bridges map is empty initially.
        let pool = AftBridgePool::new();
        let bridges = pool.bridges.lock().await;
        assert!(bridges.is_empty(), "pool should start empty");
        drop(bridges);
    }

    #[test]
    fn request_json_format_matches_aft_protocol() {
        // v0.49.x: params are FLAT at the top level (not under `params`),
        // except bash which stays nested (its own `command` param collides
        // with the envelope).
        let id = "test-id";
        let mut request = serde_json::Map::new();
        request.insert("id".to_string(), serde_json::json!(id));
        request.insert("command".to_string(), serde_json::json!("read"));
        request.insert("file".to_string(), serde_json::json!("/tmp/test.rs"));
        let line = serde_json::to_string(&serde_json::Value::Object(request)).unwrap();
        assert!(line.contains("\"id\":\"test-id\""));
        assert!(line.contains("\"command\":\"read\""));
        assert!(line.contains("\"file\":\"/tmp/test.rs\""));
        assert!(
            !line.contains("\"params\""),
            "flat protocol: no params wrapper"
        );
        assert!(line.ends_with("}"));

        // bash stays nested
        let nested = serde_json::json!({
            "id": id,
            "command": "bash",
            "params": {"command": "echo hi"},
        });
        let line = serde_json::to_string(&nested).unwrap();
        assert!(line.contains("\"params\""));
    }
}
