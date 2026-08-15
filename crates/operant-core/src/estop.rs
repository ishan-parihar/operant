//! Global emergency stop (ESTOP) — a resumable pause for NEW work only.
//!
//! `operant pause` writes a sentinel file at `<data_dir>/ESTOP`;
//! `operant resume` removes it. While the sentinel exists:
//!
//! * the cron scheduler skips dispatching due jobs (`cronjobs/scheduler.rs`),
//! * the kanban CLI dispatcher skips claiming tasks (`cmd_kanban.rs`),
//! * new gateway turns get a brief "Operant is paused" reply instead of an
//!   agent run (`gateway/mod.rs::route_message`).
//!
//! In-flight work is NEVER killed — this is pause-new-work, not panic/exit.
//! The check is a single `std::fs::metadata` (via `Path::exists`), so callers
//! may run it every tick with no caching; engaging/disengaging takes effect
//! on the very next check.
//!
//! The sentinel body is optional JSON `{"reason": ..., "engaged_at": ...}`.
//! A corrupt or empty file still counts as engaged (fail safe): the pause
//! must hold even if the file was created by `touch ~/.operant/ESTOP`.
//!
//! Ported from: hermes-agent `agent/estop.py` (gastownhall/gastown estop.go
//! lineage — deliberately resumable, unlike `/panic` kill semantics).

use crate::error::Error;
use crate::platform;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Sentinel file name inside the operant data directory.
pub const SENTINEL_NAME: &str = "ESTOP";

/// Parsed state of the emergency stop sentinel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstopState {
    /// Whether the emergency stop is engaged (fail-safe: any present file).
    pub engaged: bool,
    /// Optional reason recorded when the stop was engaged.
    pub reason: Option<String>,
    /// Unix millis when the stop was engaged (absent for bare `touch` files).
    pub engaged_at: Option<u64>,
}

/// Path of the ESTOP sentinel under the active operant data directory.
pub fn sentinel_path() -> PathBuf {
    sentinel_path_in(&platform::operant_data_dir())
}

/// Path of the ESTOP sentinel under an explicit root (used by tests).
pub fn sentinel_path_in(root: &Path) -> PathBuf {
    root.join(SENTINEL_NAME)
}

/// Cheap check (one stat): is the global emergency stop engaged?
pub fn is_engaged() -> bool {
    is_engaged_in(&platform::operant_data_dir())
}

/// Cheap check (one stat) under an explicit root.
pub fn is_engaged_in(root: &Path) -> bool {
    sentinel_path_in(root).exists()
}

/// Engage the emergency stop. Writes optional JSON `{reason, engaged_at}`.
pub fn engage(reason: Option<&str>) -> Result<(), Error> {
    engage_in(&platform::operant_data_dir(), reason)
}

/// Engage under an explicit root (used by tests).
pub fn engage_in(root: &Path, reason: Option<&str>) -> Result<(), Error> {
    let path = sentinel_path_in(root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let state = EstopState {
        engaged: true,
        reason: reason.map(|r| r.to_string()),
        engaged_at: Some(now_millis()),
    };
    let body = serde_json::to_string_pretty(&state)?;
    std::fs::write(&path, body)?;
    Ok(())
}

/// Disengage (remove the sentinel). No-op when not engaged.
pub fn disengage() -> Result<(), Error> {
    disengage_in(&platform::operant_data_dir())
}

/// Disengage under an explicit root (used by tests).
pub fn disengage_in(root: &Path) -> Result<(), Error> {
    let path = sentinel_path_in(root);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Read the current state: engaged flag plus metadata when parseable.
///
/// A corrupt or empty file still reports `engaged: true` (fail safe).
pub fn state() -> EstopState {
    state_in(&platform::operant_data_dir())
}

/// Read the current state under an explicit root.
pub fn state_in(root: &Path) -> EstopState {
    let path = sentinel_path_in(root);
    if !path.exists() {
        return EstopState::default();
    }
    let mut parsed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<EstopState>(&content).ok())
        .unwrap_or_default();
    parsed.engaged = true;
    parsed
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("operant_estop_test_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn engage_and_disengage_round_trip() {
        let root = temp_root("round_trip");
        assert!(!is_engaged_in(&root));
        engage_in(&root, Some("maintenance window")).unwrap();
        assert!(is_engaged_in(&root));
        let st = state_in(&root);
        assert!(st.engaged);
        assert_eq!(st.reason.as_deref(), Some("maintenance window"));
        assert!(st.engaged_at.is_some());
        disengage_in(&root).unwrap();
        assert!(!is_engaged_in(&root));
        assert!(!state_in(&root).engaged);
    }

    #[test]
    fn bare_touch_file_is_fail_safe_engaged() {
        let root = temp_root("fail_safe");
        fs::write(sentinel_path_in(&root), "").unwrap();
        assert!(is_engaged_in(&root));
        let st = state_in(&root);
        assert!(st.engaged);
        assert!(st.reason.is_none());
        disengage_in(&root).unwrap();
        assert!(!is_engaged_in(&root));
    }

    #[test]
    fn corrupt_sentinel_is_fail_safe_engaged() {
        let root = temp_root("corrupt");
        fs::write(sentinel_path_in(&root), "not json {{{").unwrap();
        assert!(is_engaged_in(&root));
        assert!(state_in(&root).engaged);
    }

    #[test]
    fn engage_without_reason_records_engaged_at() {
        let root = temp_root("no_reason");
        engage_in(&root, None).unwrap();
        let st = state_in(&root);
        assert!(st.engaged);
        assert!(st.reason.is_none());
        assert!(st.engaged_at.is_some());
    }

    #[test]
    fn disengage_when_not_engaged_is_noop() {
        let root = temp_root("noop");
        disengage_in(&root).unwrap();
        assert!(!is_engaged_in(&root));
    }
}
