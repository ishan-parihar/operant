// Vendored from jcode (crates/operant-base/src/registry.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] the ported session picker needs `ServerInfo` and
// `list_servers` (operant_ui/session_picker/loading.rs:7, :2998-:3005) to group
// sessions under running servers. This leaf is read-only: the registry file is
// a sibling daemon's on-disk format and operant is the picker client, not that
// daemon, so nothing here writes it (see the save() port-decision on
// `cleanup_stale` below).
// [port-decision] three upstream deps are replaced by local ports rather than
// pulled in: `crate::platform::is_process_running`
// (operant-base/src/platform.rs:249, the unix kill(pid, 0) probe) and
// `crate::storage::{operant_dir, runtime_dir}` (operant-base/src/storage.rs →
// operant-storage/src/lib.rs:150/:97). `crate::logging::info` — used only by the
// not-ported `ServerRegistry::save` hardening warnings — is not needed here.
// [port-excision] `ServerRegistry::{save, register, unregister, find_by_name,
// add_session, remove_session}` and `unregister_server` /
// `find_server_by_socket_sync` are not ported: operant is not a jcode server,
// so it never registers or unregisters one; the type itself is inlined because
// `ServerRegistry` is only ever a local parse target for `list_servers`.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::tui::operant_app::storage::{ensure_dir, operant_dir};

/// Information about a running server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Full server ID (e.g., "server_blazing_1705012345678")
    pub id: String,
    /// Short name (e.g., "blazing")
    pub name: String,
    /// Icon for display (e.g., "🔥")
    pub icon: String,
    /// Socket path
    pub socket: PathBuf,
    /// Debug socket path
    pub debug_socket: PathBuf,
    /// Git hash of the binary
    pub git_hash: String,
    /// Version string (e.g., "v0.1.123")
    pub version: String,
    /// Process ID
    pub pid: u32,
    /// When the server started (ISO 8601)
    pub started_at: String,
    /// Session names currently on this server
    #[serde(default)]
    pub sessions: Vec<String>,
}

impl ServerInfo {
    /// Display name with icon (e.g., "🔥 blazing")
    pub fn display_name(&self) -> String {
        format!("{} {}", self.icon, self.name)
    }
}

/// The server registry file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerRegistry {
    /// Map from server name to server info
    #[serde(flatten)]
    pub servers: std::collections::HashMap<String, ServerInfo>,
}

/// Get the path to the registry file
pub fn registry_path() -> anyhow::Result<PathBuf> {
    Ok(operant_dir()?.join("servers.json"))
}

/// Get the socket directory path
pub fn socket_dir() -> anyhow::Result<PathBuf> {
    Ok(runtime_dir().join("operant"))
}

/// Get the socket path for a named server
pub fn server_socket_path(name: &str) -> PathBuf {
    socket_dir()
        .map(|d| d.join(format!("{}.sock", name)))
        .unwrap_or_else(|_| std::env::temp_dir().join(format!("operant-{}.sock", name)))
}

/// Get the debug socket path for a named server
pub fn server_debug_socket_path(name: &str) -> PathBuf {
    socket_dir()
        .map(|d| d.join(format!("{}-debug.sock", name)))
        .unwrap_or_else(|_| std::env::temp_dir().join(format!("operant-{}-debug.sock", name)))
}

/// Check if a process is still running
fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        !matches!(err.raw_os_error(), Some(code) if code == libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// List all running servers
pub async fn list_servers() -> anyhow::Result<Vec<ServerInfo>> {
    let mut registry = ServerRegistry::load().await?;
    registry.cleanup_stale().await?;
    Ok(registry.servers_by_time().into_iter().cloned().collect())
}

impl ServerRegistry {
    /// Load the registry from disk
    async fn load() -> anyhow::Result<Self> {
        let path = registry_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let registry: Self = serde_json::from_str(&content)?;
        Ok(registry)
    }

    /// Get all servers sorted by started_at (newest first)
    fn servers_by_time(&self) -> Vec<&ServerInfo> {
        let mut servers: Vec<_> = self.servers.values().collect();
        servers.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        servers
    }

    /// Clean up stale entries (servers that are no longer running or have been superseded).
    ///
    /// Socket path ownership is managed by the server process itself. Registry
    /// cleanup must not unlink those paths because a new live server can reuse
    /// the same published socket after a reboot or reload while an older
    /// registry entry still references it.
    async fn cleanup_stale(&mut self) -> anyhow::Result<Vec<String>> {
        let mut removed = Vec::new();

        // First pass: remove entries whose process is dead
        let names: Vec<_> = self.servers.keys().cloned().collect();
        for name in &names {
            if let Some(info) = self.servers.get(name) {
                let pid = info.pid;
                if !is_process_running(pid) {
                    removed.push(name.clone());
                    self.servers.remove(name);
                }
            }
        }

        // Second pass: if multiple entries share the same socket path (happens
        // after server exec/reload), keep only the newest one.
        let remaining: Vec<_> = self.servers.keys().cloned().collect();
        let mut socket_to_newest: std::collections::HashMap<PathBuf, (String, String)> =
            std::collections::HashMap::new();
        for name in &remaining {
            if let Some(info) = self.servers.get(name) {
                let entry = socket_to_newest
                    .entry(info.socket.clone())
                    .or_insert_with(|| (name.clone(), info.started_at.clone()));
                if info.started_at > entry.1 {
                    *entry = (name.clone(), info.started_at.clone());
                }
            }
        }
        for name in &remaining {
            if let Some(info) = self.servers.get(name)
                && let Some((newest_name, _)) = socket_to_newest.get(&info.socket)
                && newest_name != name
            {
                removed.push(name.clone());
                self.servers.remove(name);
            }
        }

        // [port-decision] upstream calls `self.save().await?` here (:163) when
        // anything was removed. That write is deliberately NOT ported: operant
        // is the picker client, not the server, and pruning a sibling daemon's
        // registry file on its behalf would race the daemon that owns it.
        // `list_servers` still returns the same filtered list.

        Ok(removed)
    }
}

/// Platform-aware runtime directory for sockets and ephemeral state.
///
/// - Linux: `$XDG_RUNTIME_DIR` (typically `/run/user/<uid>`)
/// - macOS: `$TMPDIR` (per-user, e.g. `/var/folders/xx/.../T/`)
/// - Fallback: `std::env::temp_dir()`
///
/// Can be overridden with `$OPERANT_RUNTIME_DIR`.
/// (operant-storage/src/lib.rs:97; the macos arm is kept, and the
/// `ensure_private_runtime_dir` call at :110 folded into the existing
/// `storage::ensure_dir`.)
fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OPERANT_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(dir) = std::env::var("TMPDIR") {
            return PathBuf::from(dir);
        }
    }
    let dir = fallback_runtime_dir();
    let _ = ensure_dir(&dir);
    dir
}

fn fallback_runtime_dir() -> PathBuf {
    std::env::temp_dir().join(format!("operant-{}", runtime_user_discriminator()))
}

#[cfg(unix)]
fn runtime_user_discriminator() -> String {
    unsafe { libc::geteuid() }.to_string()
}

#[cfg(not(unix))]
fn runtime_user_discriminator() -> String {
    whoami_username().unwrap_or_else(|| "unknown".to_string())
}

#[cfg(not(unix))]
fn whoami_username() -> Option<String> {
    std::env::var("USERNAME").ok()
}
