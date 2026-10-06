// Vendored from jcode (crates/jcode-base/src/claude_live.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
//! Discovery and explicit handoff support for live Claude Code sessions.
//!
//! Claude Code 2.1.x publishes one small registry record per interactive
//! process at `~/.claude/sessions/<pid>.json`. The record's `procStart` value
//! matches Linux `/proc/<pid>/stat` field 22, which lets us guard against PID
//! reuse before presenting or signaling a process.
//!
//! [port-decision] only the discovery arm is ported — `live_claude_sessions`
//! (:78) with `find_live_claude_session` (:83), `live_claude_sessions_in` (:89),
//! `registry_record_is_takeover_candidate` (:128) and the two
//! `process_identity_matches` arms (:176 / :183) plus `linux_process_identity`
//! (:151). The picker calls only `live_claude_sessions`
//! (jcode_ui/session_picker.rs:523). The handoff arm upstream — the
//! `StopLiveClaudeOutcome` enum (:44), `open_pidfd` (:189),
//! `stop_live_claude_session` and its timeout/sign-ramp loop — is not ported:
//! nothing in operant asks an external CLI process to exit.
#[cfg(not(target_os = "linux"))]
use anyhow::anyhow;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ClaudeSessionRegistryRecord {
    pid: u32,
    session_id: String,
    cwd: String,
    proc_start: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    started_at: Option<i64>,
    #[serde(default)]
    version: Option<String>,
}

/// A Claude Code process whose registry record and OS process identity agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveClaudeSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub proc_start: String,
    pub name: Option<String>,
    pub started_at: Option<i64>,
    pub version: Option<String>,
    registry_path: PathBuf,
}

impl LiveClaudeSession {
    fn from_record(record: ClaudeSessionRegistryRecord, registry_path: PathBuf) -> Self {
        Self {
            pid: record.pid,
            session_id: record.session_id,
            cwd: record.cwd,
            proc_start: record.proc_start,
            name: record.name,
            started_at: record.started_at,
            version: record.version,
            registry_path,
        }
    }

    /// Where this session's registry record lives, used when reporting what was
    /// inspected during a handoff.
    pub fn registry_path(&self) -> &Path {
        &self.registry_path
    }
}

/// Return live, identity-verified interactive Claude Code CLI sessions.
///
/// On platforms where Claude's process-start token cannot yet be verified, the
/// safe behavior is to return no takeover candidates rather than trust a PID.
pub fn live_claude_sessions() -> Result<Vec<LiveClaudeSession>> {
    let root = crate::tui::jcode_app::storage::user_home_path(".claude/sessions")?;
    live_claude_sessions_in(&root)
}

pub fn find_live_claude_session(session_id: &str) -> Result<Option<LiveClaudeSession>> {
    Ok(live_claude_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id))
}

fn live_claude_sessions_in(root: &Path) -> Result<Vec<LiveClaudeSession>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| {
        format!(
            "failed to read Claude live-session registry {}",
            root.display()
        )
    })? {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<ClaudeSessionRegistryRecord>(&bytes) else {
            continue;
        };
        if !registry_record_is_takeover_candidate(&path, &record) {
            continue;
        }
        if process_identity_matches(record.pid, &record.proc_start) {
            sessions.push(LiveClaudeSession::from_record(record, path));
        }
    }
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(sessions)
}

fn registry_record_is_takeover_candidate(
    path: &Path,
    record: &ClaudeSessionRegistryRecord,
) -> bool {
    if record
        .kind
        .as_deref()
        .is_some_and(|kind| kind != "interactive")
        || record
            .entrypoint
            .as_deref()
            .is_some_and(|entrypoint| entrypoint != "cli")
    {
        return false;
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.parse::<u32>().ok())
        == Some(record.pid)
}

#[cfg(target_os = "linux")]
fn linux_process_identity(pid: u32) -> std::io::Result<(String, char)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let close_paren = stat
        .rfind(')')
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid proc stat"))?;
    let fields = stat
        .get(close_paren + 2..)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid proc stat"))?
        .split_whitespace()
        .collect::<Vec<_>>();
    let state = fields
        .first()
        .and_then(|field| field.chars().next())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing state"))?;
    // `/proc/<pid>/stat` field 22 is the process start time in clock ticks.
    // `fields[0]` here is overall field 3 (`state`), hence index 19.
    let start = fields
        .get(19)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing start"))?;
    Ok(((*start).to_string(), state))
}

#[cfg(target_os = "linux")]
fn process_identity_matches(pid: u32, expected_start: &str) -> bool {
    linux_process_identity(pid)
        .map(|(actual_start, state)| state != 'Z' && actual_start == expected_start)
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn process_identity_matches(_pid: u32, _expected_start: &str) -> bool {
    false
}
