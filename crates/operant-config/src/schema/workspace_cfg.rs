//! `workspace_cfg` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::{Context, Result};
use directories::UserDirs;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "workspace"]
/// Multi-workspace profile configuration: each named engagement gets its own
/// memory, secrets, and audit directories.
pub struct WorkspaceConfig {
    /// Turn on multi-workspace profiles — each named engagement gets its own memory, secrets, and audit directories so work for one client/project never bleeds into another. Leave off for single-workspace mode where everything lives under `~/.operant/workspace`.
    #[serde(default)]
    pub enabled: bool,
    /// Which workspace profile is currently active — picks the `<workspaces_dir>/<name>/` directory Operant reads from and writes to. Required when multi-workspace is enabled; ignored otherwise.
    #[serde(default)]
    pub active_workspace: Option<String>,
    /// Parent directory holding all workspace profiles, one subdirectory per profile. Override to keep profiles on a separate disk or inside an encrypted volume.
    #[serde(default = "default_workspaces_dir")]
    pub workspaces_dir: String,
    /// Give each profile its own `brain.db` so conversation history, notes, and memories from one engagement don't leak into another. Turn off only if you want all profiles sharing a single memory store.
    #[serde(default = "default_true")]
    pub isolate_memory: bool,
    /// Scope provider API keys, channel tokens, and other secrets to the active profile — so a key added while on `client-a` isn't visible from `client-b`. Turn off only if you want all profiles sharing one secret namespace.
    #[serde(default = "default_true")]
    pub isolate_secrets: bool,
    /// Give each profile its own tool-call and channel-message audit trail, so you can hand off logs for a single engagement without exposing other work.
    #[serde(default = "default_true")]
    pub isolate_audit: bool,
    /// Let memory search span all workspaces instead of only the active one. Off by default — turning it on defeats the point of isolation and is only useful for global admin queries.
    #[serde(default)]
    pub cross_workspace_search: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            active_workspace: None,
            workspaces_dir: default_workspaces_dir(),
            isolate_memory: true,
            isolate_secrets: true,
            isolate_audit: true,
            cross_workspace_search: false,
        }
    }
}

impl Default for GoogleWorkspaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_services: Vec::new(),
            allowed_operations: Vec::new(),
            credentials_path: None,
            default_account: None,
            rate_limit_per_minute: default_gws_rate_limit(),
            timeout_secs: default_gws_timeout_secs(),
            audit_log: false,
        }
    }
}

pub(crate) const ACTIVE_WORKSPACE_STATE_FILE: &str = "active_workspace.toml";

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub(crate) struct ActiveWorkspaceState {
    pub(crate) config_dir: String,
}

pub(crate) fn active_workspace_state_path(default_dir: &Path) -> PathBuf {
    default_dir.join(ACTIVE_WORKSPACE_STATE_FILE)
}

/// Returns `true` if `path` lives under the OS temp directory.
pub(crate) fn is_temp_directory(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    // Canonicalize when possible to handle symlinks (macOS /var → /private/var)
    let canon_temp = temp.canonicalize().unwrap_or_else(|_| temp.clone());
    let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canon_path.starts_with(&canon_temp)
}

pub(crate) async fn load_persisted_workspace_dirs(
    default_config_dir: &Path,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let state_path = active_workspace_state_path(default_config_dir);
    if !state_path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&state_path).await {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                "Failed to read active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let state: ActiveWorkspaceState = match toml::from_str(&contents) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                "Failed to parse active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let raw_config_dir = state.config_dir.trim();
    if raw_config_dir.is_empty() {
        tracing::warn!(
            "Ignoring active workspace marker {} because config_dir is empty",
            state_path.display()
        );
        return Ok(None);
    }

    let parsed_dir = expand_tilde_path(raw_config_dir);
    let config_dir = if parsed_dir.is_absolute() {
        parsed_dir
    } else {
        default_config_dir.join(parsed_dir)
    };
    Ok(Some((config_dir.clone(), config_dir.join("workspace"))))
}

/// Persist the active workspace's config-dir marker file to disk.
pub async fn persist_active_workspace_config_dir(config_dir: &Path) -> Result<()> {
    persist_active_workspace_config_dir_in(config_dir, &default_config_dir()?).await
}

/// Inner implementation that accepts the default config directory explicitly,
/// so callers (including tests) control where the marker is written without
/// manipulating process-wide environment variables.
pub(crate) async fn persist_active_workspace_config_dir_in(
    config_dir: &Path,
    default_config_dir: &Path,
) -> Result<()> {
    let state_path = active_workspace_state_path(default_config_dir);

    // Guard: refuse to write a temp-directory config_dir into a non-temp
    // default location. This prevents transient test runs or one-off
    // invocations from hijacking the real user's daemon config resolution.
    // When both paths are temp (e.g. in tests), the write is harmless.
    if is_temp_directory(config_dir) && !is_temp_directory(default_config_dir) {
        tracing::warn!(
            path = %config_dir.display(),
            "Refusing to persist temp directory as active workspace marker"
        );
        return Ok(());
    }

    if config_dir == default_config_dir {
        if state_path.exists() {
            fs::remove_file(&state_path).await.with_context(|| {
                format!(
                    "Failed to clear active workspace marker: {}",
                    state_path.display()
                )
            })?;
        }
        return Ok(());
    }

    fs::create_dir_all(&default_config_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create default config directory: {}",
                default_config_dir.display()
            )
        })?;

    let state = ActiveWorkspaceState {
        config_dir: config_dir.to_string_lossy().into_owned(),
    };
    let serialized =
        toml::to_string_pretty(&state).context("Failed to serialize active workspace marker")?;

    let temp_path = default_config_dir.join(format!(
        ".{ACTIVE_WORKSPACE_STATE_FILE}.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, serialized).await.with_context(|| {
        format!(
            "Failed to write temporary active workspace marker: {}",
            temp_path.display()
        )
    })?;

    if let Err(error) = fs::rename(&temp_path, &state_path).await {
        let _ = fs::remove_file(&temp_path).await;
        anyhow::bail!(
            "Failed to atomically persist active workspace marker {}: {error}",
            state_path.display()
        );
    }

    sync_directory(default_config_dir).await?;
    Ok(())
}

/// Resolve the `(config_dir, workspace_dir)` pair for a workspace directory,
/// preferring a local `config.toml` next to the workspace when present.
pub fn resolve_config_dir_for_workspace(workspace_dir: &Path) -> (PathBuf, PathBuf) {
    let workspace_config_dir = workspace_dir.to_path_buf();
    if workspace_config_dir.join("config.toml").exists() {
        return (
            workspace_config_dir.clone(),
            workspace_config_dir.join("workspace"),
        );
    }

    let legacy_config_dir = workspace_dir.parent().map(|parent| parent.join(".operant"));
    if let Some(legacy_dir) = legacy_config_dir {
        if legacy_dir.join("config.toml").exists() {
            return (legacy_dir, workspace_config_dir);
        }

        if workspace_dir
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("workspace"))
        {
            return (legacy_dir, workspace_config_dir);
        }
    }

    (
        workspace_config_dir.clone(),
        workspace_config_dir.join("workspace"),
    )
}

/// Expand tilde in paths, falling back to `UserDirs` when HOME is unset.
///
/// In non-TTY environments (e.g. cron), HOME may not be set, causing
/// `shellexpand::tilde` to return the literal `~` unexpanded. This helper
/// detects that case and uses `directories::UserDirs` as a fallback.
pub(crate) fn expand_tilde_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    let expanded_str = expanded.as_ref();

    // If the path still starts with '~', tilde expansion failed (HOME unset)
    if expanded_str.starts_with('~') {
        if let Some(user_dirs) = UserDirs::new() {
            let home = user_dirs.home_dir();
            // Replace leading ~ with home directory
            if let Some(rest) = expanded_str.strip_prefix('~') {
                return home.join(rest.trim_start_matches(['/', '\\']));
            }
        }
        // If UserDirs also fails, log a warning and use the literal path
        tracing::warn!(
            path = path,
            "Failed to expand tilde: HOME environment variable is not set and UserDirs failed. \
             In cron/non-TTY environments, use absolute paths or set HOME explicitly."
        );
    }

    PathBuf::from(expanded_str)
}

pub(crate) async fn resolve_runtime_config_dirs(
    default_operant_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Ok(custom_config_dir) = std::env::var("OPERANT_CONFIG_DIR") {
        let custom_config_dir = custom_config_dir.trim();
        if !custom_config_dir.is_empty() {
            let operant_dir = expand_tilde_path(custom_config_dir);
            return Ok((
                operant_dir.clone(),
                operant_dir.join("workspace"),
                ConfigResolutionSource::EnvConfigDir,
            ));
        }
    }

    if let Ok(custom_workspace) = std::env::var("OPERANT_WORKSPACE")
        && !custom_workspace.is_empty()
    {
        let expanded = expand_tilde_path(&custom_workspace);
        let (operant_dir, workspace_dir) = resolve_config_dir_for_workspace(&expanded);
        return Ok((
            operant_dir,
            workspace_dir,
            ConfigResolutionSource::EnvWorkspace,
        ));
    }

    if let Some((operant_dir, workspace_dir)) =
        load_persisted_workspace_dirs(default_operant_dir).await?
    {
        return Ok((
            operant_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
        ));
    }

    Ok((
        default_operant_dir.to_path_buf(),
        default_workspace_dir.to_path_buf(),
        ConfigResolutionSource::DefaultConfigDir,
    ))
}

pub(crate) fn config_dir_creation_error(path: &Path) -> String {
    format!(
        "Failed to create config directory: {}. If running as an OpenRC service, \
         ensure this path is writable by user 'operant'.",
        path.display()
    )
}
