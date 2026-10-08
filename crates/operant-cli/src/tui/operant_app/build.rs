// Vendored from jcode (crates/operant-build-support/src/storage_helpers.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — only read_build_progress and the two statics it reads:
//! build_progress_path (:172), BUILD_PROGRESS_CACHE (:187), BUILD_PROGRESS_TTL
//! (:190), read_build_progress (:208), read_build_progress_uncached (:222).
//! [port-excision] the rest of storage_helpers (activation/manifest/...) is
//! not ported. `Result` is re-rooted: upstream returns
//! `anyhow::Result<PathBuf>` via `use anyhow::Result` at the top of the impl
//! block it lives in; the re-rooted import is added below.
use crate::tui::operant_app::storage;
use anyhow::Result;
use std::path::PathBuf;

/// Get path to build progress file (for TUI to watch)
pub fn build_progress_path() -> Result<PathBuf> {
    Ok(storage::operant_dir()?.join("build-progress"))
}

/// Process-local cache for `read_build_progress`. Stores the last-read value
/// alongside the time it was read so per-frame TUI calls can be served without
/// a disk hit.
static BUILD_PROGRESS_CACHE: std::sync::Mutex<Option<(std::time::Instant, Option<String>)>> =
    std::sync::Mutex::new(None);

const BUILD_PROGRESS_TTL: std::time::Duration = std::time::Duration::from_millis(100);

fn invalidate_build_progress_cache() {
    if let Ok(mut guard) = BUILD_PROGRESS_CACHE.lock() {
        *guard = None;
    }
}

// [port-decision] dedup: upstream storage_helpers.rs defined these twice across
// concatenated sources; kept the first (identical) copy.

/// Read current build progress.
///
/// The TUI calls this from its per-frame redraw scheduler (several times per
/// frame, across every connected client), so a naive implementation performs a
/// synchronous disk read on every render tick even when no build is running.
/// Build progress is a purely cosmetic status string, so we cache the result
/// for a short window. The cache is invalidated immediately on
/// `write_build_progress`/`clear_build_progress` so progress still updates
/// promptly when a build is driven from the same process; cross-process updates
/// become visible within the TTL.
pub fn read_build_progress() -> Option<String> {
    if let Ok(guard) = BUILD_PROGRESS_CACHE.lock()
        && let Some((at, ref value)) = *guard
        && at.elapsed() < BUILD_PROGRESS_TTL
    {
        return value.clone();
    }

    let value = read_build_progress_uncached();

    if let Ok(mut guard) = BUILD_PROGRESS_CACHE.lock() {
        *guard = Some((std::time::Instant::now(), value.clone()));
    }

    value
}

fn read_build_progress_uncached() -> Option<String> {
    build_progress_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// [port-decision] leaf port: stable_binary_path's chain, verbatim from
// operant-build-support — builds_dir + resolve_builds_dir (storage_helpers.rs:7-36),
// stable_binary_path (storage_helpers.rs:52), binary_stem/binary_name
// (paths.rs:65-74). `storage` is operant_app::storage (re-rooted).
/// Get path to builds directory
pub fn builds_dir() -> Result<PathBuf> {
    let dir = resolve_builds_dir(
        std::env::var_os("OPERANT_HOME").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        crate::tui::operant_app::storage::operant_dir()?,
        cfg!(windows),
    );
    crate::tui::operant_app::storage::ensure_dir(&dir)?;
    Ok(dir)
}

fn resolve_builds_dir(
    operant_home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    default_operant_dir: PathBuf,
    is_windows: bool,
) -> PathBuf {
    if let Some(operant_home) = operant_home {
        return operant_home.join("builds");
    }

    if is_windows && let Some(local_app_data) = local_app_data {
        // Keep runtime channel discovery aligned with scripts/install.ps1 and
        // the supported Windows layout under %LOCALAPPDATA%\operant\builds.
        // Durable user state and logs still live under ~/.operant.
        return local_app_data.join("operant").join("builds");
    }

    default_operant_dir.join("builds")
}

/// Get path to stable symlink
pub fn stable_binary_path() -> Result<PathBuf> {
    Ok(builds_dir()?.join("stable").join(binary_name()))
}

pub fn binary_stem() -> &'static str {
    "operant"
}

pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "operant.exe"
    } else {
        binary_stem()
    }
}
