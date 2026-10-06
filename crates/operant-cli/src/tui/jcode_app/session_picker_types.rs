// Vendored from jcode (crates/jcode-tui-session-picker/src/lib.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] the picker UI crate is not a linked dependency here (the
// session-picker port lives under jcode_ui::session_picker), so its whole
// pure-data + pure-predicate surface is ported verbatim into this leaf and the
// ported call sites are sed-mapped from `jcode_tui_session_picker::` to
// `crate::tui::jcode_app::session_picker_types::`.
// [port-decision] two upstream deps are replaced by their existing operant
// ports rather than duplicated: `jcode_message_types::ToolCall` ->
// crate::tui::jcode_model::vendor_types::ToolCall (the W1 verbatim port of the
// same struct) and `jcode_session_types::{ResumeTarget, SessionStatus}` ->
// crate::tui::jcode_app::session::ResumeTarget and
// crate::tui::jcode_model::vendor_types::SessionStatus. Upstream re-exports
// ResumeTarget from jcode-session-types at :36 for exactly this reason.
// [port-decision] upstream guards these derives behind a `serde` feature
// (:8, :38, :113, :157, :166, :173). `operant-cli` declares no such feature, so
// an honest `cfg_attr` would leave them inert and break the picker's on-disk
// `GroupedSessionListDiskCache` (jcode_ui/session_picker/loading.rs:159), which
// serializes `ServerGroup`/`SessionInfo`. The derives are therefore
// unconditional here — a superset of what upstream's enabled build produces.
use chrono::{DateTime, Utc};

pub use crate::tui::jcode_app::session::ResumeTarget;
use crate::tui::jcode_model::vendor_types::{SessionStatus, ToolCall};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SessionSource {
    Jcode,
    ClaudeCode,
    Codex,
    Pi,
    OpenCode,
    Cursor,
}

impl SessionSource {
    pub fn badge(self) -> Option<&'static str> {
        match self {
            Self::Jcode => None,
            Self::ClaudeCode => Some("🧵 Claude Code"),
            Self::Codex => Some("🧠 Codex"),
            Self::Pi => Some("π Pi"),
            Self::OpenCode => Some("◌ OpenCode"),
            Self::Cursor => Some("▮ Cursor"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SessionFilterMode {
    All,
    /// Sessions whose working directory matches the directory `/resume` was
    /// opened from.
    CurrentDir,
    CatchUp,
    Saved,
    /// Sessions with a live process right now (from the active-pid registry),
    /// annotated with whether each is still streaming a response or is ready
    /// for input. Backs the opt-in "active sessions manager" view.
    Active,
    ClaudeCode,
    Codex,
    Pi,
    OpenCode,
    Cursor,
    /// External CLI transcripts (Codex and/or Claude Code) shown together.
    /// Used by the first-run onboarding "continue where you left off" picker so
    /// it surfaces every external CLI the user is logged into, not just one.
    ExternalClis,
}

impl SessionFilterMode {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::CurrentDir,
            Self::CurrentDir => Self::CatchUp,
            Self::CatchUp => Self::Saved,
            Self::Saved => Self::Active,
            Self::Active => Self::ClaudeCode,
            Self::ClaudeCode => Self::Codex,
            Self::Codex => Self::Pi,
            Self::Pi => Self::OpenCode,
            Self::OpenCode => Self::Cursor,
            Self::Cursor => Self::All,
            // ExternalClis is an onboarding-only composite filter, not part of
            // the user-facing cycle; treat it as a no-op anchor.
            Self::ExternalClis => Self::All,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::All => Self::Cursor,
            Self::CurrentDir => Self::All,
            Self::CatchUp => Self::CurrentDir,
            Self::Saved => Self::CatchUp,
            Self::Active => Self::Saved,
            Self::ClaudeCode => Self::Active,
            Self::Codex => Self::ClaudeCode,
            Self::Pi => Self::Codex,
            Self::OpenCode => Self::Pi,
            Self::Cursor => Self::OpenCode,
            Self::ExternalClis => Self::All,
        }
    }

    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::CurrentDir => Some("📁 current dir"),
            Self::CatchUp => Some("⏭ catch up"),
            Self::Saved => Some("📌 saved"),
            Self::Active => Some("⚡ active"),
            Self::ClaudeCode => Some("🧵 Claude Code"),
            Self::Codex => Some("🧠 Codex"),
            Self::Pi => Some("π Pi"),
            Self::OpenCode => Some("◌ OpenCode"),
            Self::Cursor => Some("▮ Cursor"),
            Self::ExternalClis => Some("🧠 Codex + 🧵 Claude Code + π Pi + ◌ OpenCode + ▮ Cursor"),
        }
    }
}

/// Session info for display in the interactive session picker.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub short_name: String,
    pub icon: String,
    pub title: String,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub created_at: DateTime<Utc>,
    pub last_message_time: DateTime<Utc>,
    pub last_active_at: Option<DateTime<Utc>>,
    pub working_dir: Option<String>,
    pub model: Option<String>,
    pub provider_key: Option<String>,
    pub is_canary: bool,
    pub is_debug: bool,
    pub saved: bool,
    pub save_label: Option<String>,
    pub status: SessionStatus,
    pub needs_catchup: bool,
    pub estimated_tokens: usize,
    /// First visible user prompt in the session, shown in compact list rows.
    pub first_user_prompt: Option<String>,
    pub messages_preview: Vec<PreviewMessage>,
    /// Lowercased searchable text used by picker filtering.
    pub search_index: String,
    /// Server name this session belongs to (if running).
    pub server_name: Option<String>,
    /// Server icon.
    pub server_icon: Option<String>,
    /// Human/session source classification shown in the UI.
    pub source: SessionSource,
    /// How this entry should be resumed when selected.
    pub resume_target: ResumeTarget,
    /// Backing external transcript/storage path when available.
    pub external_path: Option<String>,
}

/// A group of sessions under a server.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerGroup {
    pub name: String,
    pub icon: String,
    pub version: String,
    pub git_hash: String,
    pub is_running: bool,
    pub sessions: Vec<SessionInfo>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PreviewMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<String>,
    pub tool_data: Option<ToolCall>,
    pub timestamp: Option<DateTime<Utc>>,
}

/// An item in the picker list, either a server/header row or a session row.
#[derive(Clone)]
pub enum PickerItem {
    ServerHeader {
        name: String,
        icon: String,
        version: String,
        session_count: usize,
    },
    Session,
    OrphanHeader {
        session_count: usize,
    },
    SavedHeader {
        session_count: usize,
    },
}

pub fn session_is_claude_code(source: SessionSource, id: &str) -> bool {
    source == SessionSource::ClaudeCode || id.starts_with("imported_cc_")
}

pub fn session_is_codex(source: SessionSource, model: Option<&str>) -> bool {
    if source == SessionSource::Codex {
        return true;
    }
    model
        .map(|model| model.to_ascii_lowercase().contains("codex"))
        .unwrap_or(false)
}

pub fn session_is_pi(
    source: SessionSource,
    provider_key: Option<&str>,
    model: Option<&str>,
) -> bool {
    if source == SessionSource::Pi {
        return true;
    }
    let provider_matches = provider_key
        .map(|key| {
            let key = key.to_ascii_lowercase();
            key == "pi" || key.starts_with("pi-")
        })
        .unwrap_or(false);
    let model_matches = model
        .map(|model| {
            let model = model.to_ascii_lowercase();
            model == "pi"
                || model.starts_with("pi-")
                || model.starts_with("pi/")
                || model.contains("/pi-")
        })
        .unwrap_or(false);
    provider_matches || model_matches
}

pub fn session_is_open_code(source: SessionSource, provider_key: Option<&str>) -> bool {
    if source == SessionSource::OpenCode {
        return true;
    }
    provider_key
        .map(|key| {
            let key = key.to_ascii_lowercase();
            key == "opencode" || key == "opencode-go" || key.contains("opencode")
        })
        .unwrap_or(false)
}

pub fn session_is_cursor(source: SessionSource, provider_key: Option<&str>) -> bool {
    if source == SessionSource::Cursor {
        return true;
    }
    provider_key
        .map(|key| {
            let key = key.to_ascii_lowercase();
            key == "cursor" || key == "cursor-agent"
        })
        .unwrap_or(false)
}
