// Vendored from jcode (crates/jcode-session-types/src/lib.rs + crates/jcode-base/src/session/render.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see jcode_app/mod.rs for scope.
//! Included from jcode-session-types/src/lib.rs: RenderedImageSource (:113),
//! RenderedImageAnchor (:124), RenderedImage (:133). Included from
//! jcode-base/src/session/render.rs: parse_attached_image_label (:252, private),
//! is_attached_image_label_text (:268). [port-excision] the rest of
//! session-types (ResumeTarget, History, titles/transcription) is not ported.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderedImageSource {
    UserInput,
    ToolResult { tool_name: String },
    Other { role: String },
}

/// Where an image belongs in the transcript flow. Used by UIs to render the
/// image inline at the message that produced it instead of appending it at the
/// bottom of the transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderedImageAnchor {
    /// The image came from the tool result for this tool call id.
    ToolCall { id: String },
    /// The image was attached to the nth (0-based) user prompt in the rendered
    /// transcript.
    UserPrompt { ordinal: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderedImage {
    pub media_type: String,
    pub data: String,
    pub label: Option<String>,
    pub source: RenderedImageSource,
    /// Transcript anchor identifying the message this image belongs to, so the
    /// UI can render it inline at that spot. `None` when the producer cannot
    /// anchor it (e.g. older servers); unanchored images fall back to the
    /// bottom of the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<RenderedImageAnchor>,
    /// Insert before this zero-based entry in the accompanying History.messages
    /// array (including hidden/system/tool rows). Its length means append.
    /// Set for restored tool images, whose tool-call row may not be exposed by
    /// a client. Absent on live events and older servers. Preserve vector order
    /// for multiple images at the same boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_message_index: Option<usize>,
}

fn parse_attached_image_label(text: &str) -> Option<String> {
    let prefix = "[Attached image associated with the preceding tool result: ";
    let suffix = "]";
    text.trim()
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// True when `text` is exactly an attached-image label message (the synthetic
/// "[Attached image associated with the preceding tool result: ...]" text that
/// follows tool-result images). UIs use this to keep user-prompt ordinals
/// consistent between live transcripts (which never show these) and rendered
/// history (which does).
pub fn is_attached_image_label_text(text: &str) -> bool {
    parse_attached_image_label(text).is_some()
}

// ============================================================================
// [port-source] batch-4 extension of this leaf — the stored-session model the
// session-picker surface reads. This block supersedes the [port-excision] note
// above for everything it lists; a later wave that needs a new symbol extends
// this same submodule (see jcode_app/mod.rs). Sources, ported verbatim with
// imports re-rooted to jcode_app paths:
// - jcode-session-types/src/lib.rs: ResumeTarget (:13-54), ResponseStats
//   (:56-75), RenderedMessage (:77-93), RenderedCompactedHistoryInfo
//   (:95-109), GitState (:215-221), EnvSnapshot (:223-242), StoredMemoryInjection
//   (:244-265), StoredMessage (:267-280), StoredDisplayRole (:282-287),
//   StoredTokenUsage (:289-301), StoredCompactionState (:303-311),
//   SessionImproveMode (:203-213). SessionStatus is NOT re-ported: it is
//   re-exported from crate::tui::jcode_model::vendor_types (the dedupe rule).
//   [port-excision] StoredMessage::{to_message, content_preview} (:313-:356)
//   not ported — unreferenced by operant's callers.
// - jcode-session-types/src/title.rs:1-93 (prompt_title, strip_blocks).
// - jcode-base/src/session/crash.rs:188-199 (CrashedSessionsInfo struct only).
// - jcode-storage/src/active_pids.rs: active_pids_dir (:20), streaming_pids_dir
//   (:28), internal_pids_dir (:36), process_is_running (:218-:233),
//   SessionPresence (:249-263), session_presence (:270-:320).
//   [port-excision] SessionCounts/session_counts/user_session_presence/
//   user_session_counts unreferenced here.
// - jcode-base/src/session/journal.rs:1-99 (SessionJournalMeta, SessionJournalEntry,
//   PersistVectorMode, SessionPersistState, metadata_requires_snapshot not
//   ported — unreferenced).
// - jcode-base/src/session/model.rs:1-32 (StoredReplayEvent,
//   StoredReplayEventKind; SESSION_CONTEXT_PREFIX excised — unreferenced).
// - jcode-base/src/session/memory_profile.rs: ContentBlockMemoryStats (:7-160),
//   summarize_message_content (:163), summarize_blocks (:176),
//   SessionMemoryProfileCache (:184-198). [port-excision]
//   SessionMemoryProfileSnapshot (:200-214) unreferenced.
// - jcode-base/src/session/storage_paths.rs:1-49 (whole file; the
//   persist_vector_mode_label helper is included verbatim).
// - jcode-base/src/session.rs: is_internal_system_reminder_message (:78),
//   is_visible_conversation_message (:89), is_scheduled_task_message (:98),
//   Session (:105-207), journal_meta (:505), reset_persist_state (:535),
//   reset_provider_messages_cache (:550), mark_memory_profile_dirty (:594),
//   rebuild_memory_profile_cache (:598), mark_messages_append_dirty (:673),
//   append_stored_message (:1301), adopt_prompt_title (:1315),
//   backfill_prompt_title (:1329).
// - jcode-base/src/session/persistence.rs: Session::load (:301),
//   Session::load_from_path (:232) — see the [port-decision] marker inside.
// - jcode-base/src/session/render.rs:1-590 (the render_messages chain; see
//   the [port-excision] note at the render section).
// - jcode-base/src/session/render/response_stats.rs:1-85 (nested verbatim;
//   tests excised).
// ============================================================================
use crate::tui::jcode_app::message::{ContentBlock, Message, Role, ToolCall};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Re-export, not a re-port: jcode's SessionStatus lives in this repo at
/// crate::tui::jcode_model::vendor_types (batch-2 verbatim port incl. its
/// display/icon/detail impls).
pub use crate::tui::jcode_model::vendor_types::SessionStatus;

// --- jcode-session-types/src/lib.rs ---

/// Identifies a session to resume, across the agent backends jcode can import
/// from. This is pure data (only ids/paths) with no UI dependency; it lives in
/// `jcode-session-types` so the foundation/import layer can match on it without
/// depending on any `jcode-tui-*` crate. The session-picker UI re-exports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResumeTarget {
    JcodeSession {
        session_id: String,
    },
    ClaudeCodeSession {
        session_id: String,
        session_path: String,
    },
    CodexSession {
        session_id: String,
        session_path: String,
    },
    PiSession {
        session_path: String,
    },
    OpenCodeSession {
        session_id: String,
        session_path: String,
    },
    CursorSession {
        session_id: String,
        session_path: String,
    },
}

impl ResumeTarget {
    pub fn stable_id(&self) -> &str {
        match self {
            Self::JcodeSession { session_id } => session_id,
            Self::ClaudeCodeSession { session_id, .. } => session_id,
            Self::CodexSession { session_id, .. } => session_id,
            Self::PiSession { session_path } => session_path,
            Self::OpenCodeSession { session_id, .. } => session_id,
            Self::CursorSession { session_id, .. } => session_id,
        }
    }
}

/// Durable usage for one user turn, summed across its assistant/tool rounds.
/// Input is the raw provider-reported count, not normalized across providers.
/// Cache reads may be included in input (OpenAI) or separate (Anthropic).
/// Missing telemetry is unknown, not zero. Counts are absent if any assistant
/// round lacks that metric. This is not a session total or a billing estimate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResponseStats {
    /// Whole-turn wall-clock seconds, including tools. Currently not persisted,
    /// so restored history leaves this absent. Never inferred from tool timings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedMessage {
    /// Present only on the final visible assistant row of a completed stored
    /// user turn. Tool-only intermediate rounds contribute to these totals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_stats: Option<ResponseStats>,
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<String>,
    pub tool_data: Option<ToolCall>,
    /// Index of the stored session message this rendered message came from.
    /// `None` for synthetic UI-only messages (e.g. the compacted-history
    /// notice). Used to map user-facing rewind targets back to
    /// transcript (issue #432).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedCompactedHistoryInfo {
    /// Number of compacted historical messages that can render visibly in the UI.
    /// Hidden internal reminders are excluded from this count.
    pub total_messages: usize,
    /// Number of renderable compacted historical messages included in this payload.
    pub visible_messages: usize,
    /// Number of older renderable compacted historical messages still hidden.
    pub remaining_messages: usize,
    /// Number of user prompts (turns) that are hidden before the first rendered
    /// message. Used to keep prompt numbering correct when older history is
    /// truncated (e.g. the first visible prompt is really the 5th, not the 1st).
    #[serde(default)]
    pub hidden_user_prompts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitState {
    pub root: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvSnapshot {
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub reason: String,
    pub session_id: String,
    pub working_dir: Option<String>,
    pub provider: String,
    pub model: String,
    pub jcode_version: String,
    pub jcode_git_hash: Option<String>,
    pub jcode_git_dirty: Option<bool>,
    pub os: String,
    pub arch: String,
    pub pid: u32,
    pub is_debug: bool,
    pub is_canary: bool,
    pub testing_build: Option<String>,
    pub working_git: Option<GitState>,
}

/// A memory injection event, stored for replay visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMemoryInjection {
    /// Human-readable summary (e.g., "🧠 auto-recalled 3 memories")
    pub summary: String,
    /// The recalled memory content that was injected
    pub content: String,
    /// Number of memories recalled
    pub count: u32,
    /// Stable memory IDs included in this injection, used to avoid re-injecting
    /// the same memories after session resume/reload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_ids: Vec<String>,
    /// Age of memories in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    /// Message index this injection occurred before (for replay timing)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_message: Option<usize>,
    /// Timestamp when injection occurred
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_role: Option<StoredDisplayRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<StoredTokenUsage>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoredDisplayRole {
    System,
    BackgroundTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokenUsage {
    /// Full prompt size resolved per request, before provider identity can change.
    /// Older records lack this and cannot safely reconstruct mixed-provider totals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCompactionState {
    pub summary_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_encrypted_content: Option<String>,
    pub covers_up_to_turn: usize,
    pub original_turn_count: usize,
    pub compacted_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionImproveMode {
    #[serde(rename = "improve_run", alias = "run")]
    ImproveRun,
    #[serde(rename = "improve_plan", alias = "plan")]
    ImprovePlan,
    #[serde(rename = "refactor_run")]
    RefactorRun,
    #[serde(rename = "refactor_plan")]
    RefactorPlan,
}

// --- jcode-session-types/src/title.rs ---

const MAX_TITLE_CHARS: usize = 64;

/// Turn a user prompt into a compact one-line title.
///
/// Internal wrappers (`<system-reminder>` blocks, voice `<transcription>`
/// tags) are removed, whitespace is collapsed, and slash commands produce no
/// title because they describe an action rather than the conversation.
pub fn prompt_title(prompt: &str) -> Option<String> {
    let without_reminders = strip_blocks(prompt, "<system-reminder>", "</system-reminder>", false);
    let visible = strip_blocks(
        &without_reminders,
        "<transcription>",
        "</transcription>",
        true,
    );
    let normalized = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.starts_with("[Scheduled task]")
    {
        return None;
    }
    let mut chars = normalized.chars();
    let title: String = chars.by_ref().take(MAX_TITLE_CHARS).collect();
    Some(if chars.next().is_some() {
        format!("{}…", title.trim_end())
    } else {
        title
    })
}

/// Remove complete `open`..`close` segments. With `keep_inner`, only the tags
/// are dropped. Unbalanced text is kept verbatim.
fn strip_blocks(text: &str, open: &str, close: &str, keep_inner: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        let after_open = &rest[start + open.len()..];
        let Some(end) = after_open.find(close) else {
            break;
        };
        out.push_str(&rest[..start]);
        if keep_inner {
            out.push(' ');
            out.push_str(&after_open[..end]);
        }
        rest = &after_open[end + close.len()..];
    }
    out.push_str(rest);
    out
}

// --- jcode-base/src/session/crash.rs:188-199 ---

/// Info about crashed sessions pending batch restore
#[derive(Debug, Clone)]
pub struct CrashedSessionsInfo {
    /// Session IDs in the guessed relevant restore group.
    pub session_ids: Vec<String>,
    /// Display names of sessions in the guessed relevant restore group.
    pub display_names: Vec<String>,
    /// When the most recent crash occurred
    pub most_recent_crash: DateTime<Utc>,
    /// Crashed sessions excluded because they were outside the relevant group.
    pub omitted_crashed_count: usize,
}

// --- jcode-storage/src/active_pids.rs (presence slice) ---

/// Directory holding one ownership marker per session ID (`~/.jcode/active_pids`).
/// Each file contains the owning process PID, not a client/window PID in server mode.
pub fn active_pids_dir() -> Option<std::path::PathBuf> {
    crate::tui::jcode_app::storage::jcode_dir()
        .ok()
        .map(|d| d.join("active_pids"))
}

/// Directory holding per-session "currently streaming" markers. A marker file
/// exists only while a session is actively generating a model response. The
/// file content is the owning process PID so stale markers (from crashed
/// processes) can be detected and ignored.
pub fn streaming_pids_dir() -> Option<std::path::PathBuf> {
    crate::tui::jcode_app::storage::jcode_dir()
        .ok()
        .map(|d| d.join("streaming_pids"))
}

/// Directory holding markers for internal sessions (debug/test sessions and
/// spawned children such as swarm workers). These sessions keep their normal
/// active-pid lifecycle markers, but presence UIs like the menu bar indicator
/// only want to show top-level sessions.
pub fn internal_pids_dir() -> Option<std::path::PathBuf> {
    crate::tui::jcode_app::storage::jcode_dir()
        .ok()
        .map(|d| d.join("internal_pids"))
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_is_running(pid: u32) -> bool {
    // Best-effort fallback for platforms where this low-level storage crate does
    // not have a process API. The active PID file is still useful, and stale
    // entries are cleaned up by higher-level session lifecycle code.
    pid != 0
}

/// Live presence info for one running session, derived from the active-pid
/// registry and the streaming markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPresence {
    /// Session ID, e.g. `session_fox_1234567890_deadbeef`.
    pub session_id: String,
    /// PID of the process that owns the session.
    pub pid: u32,
    /// Whether the session is actively streaming a model response right now.
    pub streaming: bool,
    /// When the current streaming turn started (streaming marker mtime), if
    /// the session is streaming. Lets presence UIs show "working for 2m".
    pub streaming_since: Option<std::time::SystemTime>,
    /// Whether this is an internal session (debug/test or a spawned child
    /// such as a swarm worker) that user-facing presence UIs should hide.
    pub internal: bool,
}

/// Snapshot per-session presence by scanning the active-pid registry and
/// streaming markers, skipping any entries whose owning process is no longer
/// alive. This is a cheap O(n) scan over a handful of tiny files; used by the
/// menu bar indicator and other presence UI.
/// A live owner does not imply a connected client or an open window.
pub fn session_presence() -> Vec<SessionPresence> {
    let Some(active_dir) = active_pids_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&active_dir) else {
        return Vec::new();
    };

    let streaming_dir = streaming_pids_dir();
    let internal_dir = internal_pids_dir();
    let mut sessions = Vec::new();

    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let session_id = entry.file_name().to_string_lossy().to_string();
        let Some(pid) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
        else {
            continue;
        };
        if !process_is_running(pid) {
            continue;
        }

        let marker_path = streaming_dir.as_ref().map(|dir| dir.join(&session_id));
        let streaming = marker_path.as_ref().is_some_and(|marker| {
            std::fs::read_to_string(marker)
                .ok()
                .and_then(|raw| raw.trim().parse::<u32>().ok())
                .is_some_and(process_is_running)
        });
        let streaming_since = if streaming {
            marker_path
                .as_ref()
                .and_then(|marker| std::fs::metadata(marker).ok())
                .and_then(|meta| meta.modified().ok())
        } else {
            None
        };

        sessions.push(SessionPresence {
            internal: internal_dir
                .as_ref()
                .is_some_and(|dir| dir.join(&session_id).exists()),
            session_id,
            pid,
            streaming,
            streaming_since,
        });
    }

    sessions
}

// --- jcode-base/src/session/journal.rs ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SessionJournalMeta {
    parent_id: Option<String>,
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    updated_at: DateTime<Utc>,
    compaction: Option<StoredCompactionState>,
    provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_usage_turn_id: Option<String>,
    provider_key: Option<String>,
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    subagent_model: Option<String>,
    improve_mode: Option<SessionImproveMode>,
    autoreview_enabled: Option<bool>,
    autojudge_enabled: Option<bool>,
    is_canary: bool,
    testing_build: Option<String>,
    working_dir: Option<String>,
    short_name: Option<String>,
    status: SessionStatus,
    last_pid: Option<u32>,
    last_active_at: Option<DateTime<Utc>>,
    is_debug: bool,
    saved: bool,
    save_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionJournalEntry {
    meta: SessionJournalMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    append_messages: Vec<StoredMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    append_env_snapshots: Vec<EnvSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    append_memory_injections: Vec<StoredMemoryInjection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    append_replay_events: Vec<StoredReplayEvent>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PersistVectorMode {
    #[default]
    Clean,
    Append,
    Full,
}

#[derive(Debug, Clone, Default)]
struct SessionPersistState {
    snapshot_exists: bool,
    messages_len: usize,
    env_snapshots_len: usize,
    memory_injections_len: usize,
    replay_events_len: usize,
    messages_mode: PersistVectorMode,
    env_snapshots_mode: PersistVectorMode,
    memory_injections_mode: PersistVectorMode,
    replay_events_mode: PersistVectorMode,
    last_meta: Option<SessionJournalMeta>,
}

// --- jcode-base/src/session/model.rs:1-32 ---

/// Extra non-conversation UI/state events persisted for replay fidelity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredReplayEvent {
    pub timestamp: DateTime<Utc>,
    #[serde(flatten)]
    pub kind: StoredReplayEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event")]
pub enum StoredReplayEventKind {
    /// A non-provider display message shown in the UI (e.g. swarm/system notice).
    #[serde(rename = "display_message")]
    DisplayMessage {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        content: String,
    },
    /// Historical swarm member status snapshot.
    #[serde(rename = "swarm_status")]
    SwarmStatus {
        members: Vec<crate::tui::jcode_app::protocol::SwarmMemberStatus>,
    },
    /// Historical swarm plan snapshot.
    #[serde(rename = "swarm_plan")]
    SwarmPlan {
        swarm_id: String,
        version: u64,
        items: Vec<crate::tui::jcode_app::plan::PlanItem>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        participants: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

// --- jcode-base/src/session/memory_profile.rs ---

const LARGE_MEMORY_BLOB_THRESHOLD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Default)]
struct ContentBlockMemoryStats {
    block_count: usize,
    text_blocks: usize,
    text_bytes: usize,
    reasoning_blocks: usize,
    reasoning_bytes: usize,
    tool_use_blocks: usize,
    tool_use_input_json_bytes: usize,
    tool_result_blocks: usize,
    tool_result_bytes: usize,
    image_blocks: usize,
    image_data_bytes: usize,
    openai_compaction_blocks: usize,
    openai_compaction_bytes: usize,
    large_block_count: usize,
    large_block_bytes: usize,
    large_tool_result_count: usize,
    large_tool_result_bytes: usize,
    max_block_bytes: usize,
    max_tool_result_bytes: usize,
}

impl ContentBlockMemoryStats {
    fn merge_from(&mut self, other: &Self) {
        self.block_count += other.block_count;
        self.text_blocks += other.text_blocks;
        self.text_bytes += other.text_bytes;
        self.reasoning_blocks += other.reasoning_blocks;
        self.reasoning_bytes += other.reasoning_bytes;
        self.tool_use_blocks += other.tool_use_blocks;
        self.tool_use_input_json_bytes += other.tool_use_input_json_bytes;
        self.tool_result_blocks += other.tool_result_blocks;
        self.tool_result_bytes += other.tool_result_bytes;
        self.image_blocks += other.image_blocks;
        self.image_data_bytes += other.image_data_bytes;
        self.openai_compaction_blocks += other.openai_compaction_blocks;
        self.openai_compaction_bytes += other.openai_compaction_bytes;
        self.large_block_count += other.large_block_count;
        self.large_block_bytes += other.large_block_bytes;
        self.large_tool_result_count += other.large_tool_result_count;
        self.large_tool_result_bytes += other.large_tool_result_bytes;
        self.max_block_bytes = self.max_block_bytes.max(other.max_block_bytes);
        self.max_tool_result_bytes = self.max_tool_result_bytes.max(other.max_tool_result_bytes);
    }

    fn record_bytes(&mut self, bytes: usize) {
        self.max_block_bytes = self.max_block_bytes.max(bytes);
        if bytes >= LARGE_MEMORY_BLOB_THRESHOLD_BYTES {
            self.large_block_count += 1;
            self.large_block_bytes += bytes;
        }
    }

    fn record_block(&mut self, block: &ContentBlock) {
        self.block_count += 1;
        match block {
            ContentBlock::Text { text, .. } => {
                self.text_blocks += 1;
                self.text_bytes += text.len();
                self.record_bytes(text.len());
            }
            ContentBlock::Reasoning { text } | ContentBlock::ReasoningTrace { text } => {
                self.reasoning_blocks += 1;
                self.reasoning_bytes += text.len();
                self.record_bytes(text.len());
            }
            ContentBlock::AnthropicThinking {
                thinking,
                signature,
            } => {
                self.reasoning_blocks += 1;
                let bytes = thinking.len() + signature.len();
                self.reasoning_bytes += bytes;
                self.record_bytes(bytes);
            }
            ContentBlock::OpenAIReasoning {
                id,
                summary,
                encrypted_content,
                status,
            } => {
                self.reasoning_blocks += 1;
                let bytes = id.len()
                    + summary.iter().map(String::len).sum::<usize>()
                    + encrypted_content.as_ref().map(String::len).unwrap_or(0)
                    + status.as_ref().map(String::len).unwrap_or(0);
                self.reasoning_bytes += bytes;
                self.record_bytes(bytes);
            }
            ContentBlock::ToolUse { input, .. } => {
                self.tool_use_blocks += 1;
                let input_bytes = crate::tui::jcode_app::process_memory::estimate_json_bytes(input);
                self.tool_use_input_json_bytes += input_bytes;
                self.record_bytes(input_bytes);
            }
            ContentBlock::ToolResult { content, .. } => {
                self.tool_result_blocks += 1;
                self.tool_result_bytes += content.len();
                self.max_tool_result_bytes = self.max_tool_result_bytes.max(content.len());
                if content.len() >= LARGE_MEMORY_BLOB_THRESHOLD_BYTES {
                    self.large_tool_result_count += 1;
                    self.large_tool_result_bytes += content.len();
                }
                self.record_bytes(content.len());
            }
            ContentBlock::Image { data, .. } => {
                self.image_blocks += 1;
                self.image_data_bytes += data.len();
                self.record_bytes(data.len());
            }
            ContentBlock::OpenAICompaction { encrypted_content } => {
                self.openai_compaction_blocks += 1;
                self.openai_compaction_bytes += encrypted_content.len();
                self.record_bytes(encrypted_content.len());
            }
            ContentBlock::ToolReference { tool_name, .. } => {
                self.record_bytes(tool_name.len());
            }
        }
    }
}

fn summarize_message_content<'a, I>(messages: I) -> ContentBlockMemoryStats
where
    I: IntoIterator<Item = &'a Vec<ContentBlock>>,
{
    let mut stats = ContentBlockMemoryStats::default();
    for blocks in messages {
        for block in blocks {
            stats.record_block(block);
        }
    }
    stats
}

fn summarize_blocks(blocks: &[ContentBlock]) -> ContentBlockMemoryStats {
    let mut stats = ContentBlockMemoryStats::default();
    for block in blocks {
        stats.record_block(block);
    }
    stats
}

#[derive(Debug, Clone, Default)]
struct SessionMemoryProfileCache {
    messages_count: usize,
    messages_json_bytes: usize,
    message_stats: ContentBlockMemoryStats,
    env_snapshots_count: usize,
    env_snapshots_json_bytes: usize,
    memory_injections_count: usize,
    memory_injections_json_bytes: usize,
    replay_events_count: usize,
    replay_events_json_bytes: usize,
    provider_cache_count: usize,
    provider_cache_json_bytes: usize,
    provider_cache_stats: ContentBlockMemoryStats,
}

// --- jcode-base/src/session/storage_paths.rs ---

fn session_path_in_dir(base: &std::path::Path, session_id: &str) -> std::path::PathBuf {
    base.join("sessions").join(format!("{}.json", session_id))
}

fn file_len_or_zero(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn persist_vector_mode_label(mode: PersistVectorMode) -> &'static str {
    match mode {
        PersistVectorMode::Clean => "clean",
        PersistVectorMode::Append => "append",
        PersistVectorMode::Full => "full",
    }
}

pub fn session_path(session_id: &str) -> anyhow::Result<std::path::PathBuf> {
    let base = crate::tui::jcode_app::storage::jcode_dir()?;
    Ok(session_path_in_dir(&base, session_id))
}

pub fn session_journal_path_from_snapshot(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path
        .file_stem()
        .map(|stem| stem.to_os_string())
        .unwrap_or_default();
    name.push(".journal.jsonl");
    path.with_file_name(name)
}

pub fn session_journal_path(session_id: &str) -> anyhow::Result<std::path::PathBuf> {
    Ok(session_journal_path_from_snapshot(&session_path(
        session_id,
    )?))
}

pub fn session_exists(session_id: &str) -> bool {
    session_path(session_id)
        .map(|path| path.exists())
        .unwrap_or(false)
}

// --- jcode-base/src/session.rs (Session struct + the picker-facing impl) ---

fn is_internal_system_reminder_message(message: &StoredMessage) -> bool {
    message
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.trim_start()),
            _ => None,
        })
        .is_some_and(|text| text.starts_with("<system-reminder>"))
}

fn is_visible_conversation_message(message: &StoredMessage) -> bool {
    message.display_role.is_none()
        && !is_internal_system_reminder_message(message)
        && !is_scheduled_task_message(message)
}

/// Recognize scheduler prompts persisted before they received an explicit
/// system display role. This keeps old sessions from treating them as user
/// prompts after resume.
pub fn is_scheduled_task_message(message: &StoredMessage) -> bool {
    message.role == Role::User
        && message.content.iter().any(|block| {
            matches!(block, ContentBlock::Text { text, .. } if text.trim_start().starts_with("[Scheduled task]\n"))
        })
}

// --- jcode-base/src/session.rs:269-286 (create_with_id's private helpers) ---

fn current_working_dir_string() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let trimmed = v.trim();
            !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

fn default_is_test_session() -> bool {
    env_flag_enabled("JCODE_TEST_SESSION")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<StoredMessage>,
    /// Full assembled system prompt replacement, including an intentionally empty prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Durable logical input turn identity for per-route usage deduplication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage_turn_id: Option<String>,
    /// Persisted compacted-view state so reload/resume can continue using the
    /// active summary + recent tail instead of re-sending the full transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<StoredCompactionState>,
    /// Provider-specific session ID (e.g., Claude Code CLI session for resume)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    /// Stable provider/profile key for session-source filtering (e.g. "openai",
    /// "opencode", "opencode-go").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_key: Option<String>,
    /// Model identifier for this session (e.g., "gpt-5.2-codex")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// API method/runtime route used to select this model (e.g. "openrouter",
    /// "openai-compatible:nvidia-nim", "openai-api").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_api_method: Option<String>,
    /// Provider reasoning/thinking effort for this session (e.g., OpenAI low|medium|high|xhigh).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Optional fixed model to use for subagents launched from this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
    /// Last requested `/improve` mode for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub improve_mode: Option<SessionImproveMode>,
    /// Whether automatic end-of-turn review is enabled for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoreview_enabled: Option<bool>,
    /// Whether automatic end-of-turn judging is enabled for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autojudge_enabled: Option<bool>,
    /// Whether this session is a canary session (testing new builds)
    #[serde(default)]
    pub is_canary: bool,
    /// Build hash this session is testing (if canary)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testing_build: Option<String>,
    /// Working directory (for self-dev detection)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Memorable short name (e.g., "fox", "oak")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// Session exit status - why it ended (if not active)
    #[serde(default)]
    pub status: SessionStatus,
    /// PID of the process that last owned this session (for crash detection)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pid: Option<u32>,
    /// Last time the session was marked active
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<DateTime<Utc>>,
    /// Whether this is a debug/test session (created via debug socket)
    #[serde(default)]
    pub is_debug: bool,
    /// Whether this session has been saved/bookmarked by the user
    #[serde(default)]
    pub saved: bool,
    /// Optional user-provided label for saved sessions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_label: Option<String>,
    /// Environment snapshots for post-mortem debugging
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_snapshots: Vec<EnvSnapshot>,
    /// Memory injection events (for replay visualization)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_injections: Vec<StoredMemoryInjection>,
    /// Non-conversation UI/state events persisted for higher-fidelity replay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replay_events: Vec<StoredReplayEvent>,
    #[serde(skip)]
    persist_state: SessionPersistState,
    #[serde(skip)]
    provider_messages_cache: Vec<Message>,
    #[serde(skip)]
    provider_message_prefix_hashes_cache: Vec<u64>,
    #[serde(skip)]
    provider_messages_cache_len: usize,
    #[serde(skip)]
    provider_messages_cache_mode: PersistVectorMode,
    #[serde(skip)]
    memory_profile_cache: SessionMemoryProfileCache,
    #[serde(skip)]
    memory_profile_dirty: bool,
}

impl Session {
    // [port-source] jcode-base/src/session.rs:738-791 — verbatim.
    pub fn create_with_id(
        session_id: String,
        parent_id: Option<String>,
        title: Option<String>,
    ) -> Self {
        let now = Utc::now();
        let is_debug = default_is_test_session();
        // Try to extract short name from ID if it's a memorable ID
        let short_name =
            crate::tui::jcode_app::id::extract_session_name(&session_id).map(|s| s.to_string());
        let mut session = Self {
            id: session_id,
            parent_id,
            title,
            custom_title: None,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            system_prompt: None,
            model_usage_turn_id: None,
            compaction: None,
            provider_session_id: None,
            provider_key: None,
            model: None,
            route_api_method: None,
            reasoning_effort: None,
            subagent_model: None,
            improve_mode: None,
            autoreview_enabled: None,
            autojudge_enabled: None,
            is_canary: false,
            testing_build: None,
            working_dir: current_working_dir_string(),
            short_name,
            status: SessionStatus::Active,
            last_pid: Some(std::process::id()),
            last_active_at: Some(now),
            is_debug,
            saved: false,
            save_label: None,
            env_snapshots: Vec::new(),
            memory_injections: Vec::new(),
            replay_events: Vec::new(),
            persist_state: SessionPersistState::default(),
            provider_messages_cache: Vec::new(),
            provider_message_prefix_hashes_cache: Vec::new(),
            provider_messages_cache_len: 0,
            provider_messages_cache_mode: PersistVectorMode::Full,
            memory_profile_cache: SessionMemoryProfileCache::default(),
            memory_profile_dirty: false,
        };
        session.reset_persist_state(false);
        session
    }

    // [port-source] jcode-base/src/session/persistence.rs:232-260 — verbatim
    // with one [port-decision] deviation, see the marker inside.
    pub fn load_from_path(path: &std::path::Path) -> anyhow::Result<Self> {
        let mut session: Session = crate::tui::jcode_app::storage::read_json(path)?;
        // [port-decision] journal-replay subsystem
        // (jcode-base/src/session/persistence.rs: replay_journal_lines +
        // apply_journal_entry + corrupt-journal checkpointing) not ported: this
        // load returns the snapshot state without merging journal tail entries
        // appended after the last snapshot. Re-activate at cutover when the
        // session-persist layer lands.
        session.backfill_prompt_title();
        session.reset_persist_state(path.exists());
        session.reset_provider_messages_cache();
        session.mark_memory_profile_dirty();
        Ok(session)
    }

    // [port-source] jcode-base/src/session/persistence.rs:376-378 (save) —
    // signature verbatim; body is the honest degraded arm of the same
    // [port-decision] journal-persist subsystem gated in load_from_path above:
    // upstream's save_inner appends journal entries and only checkpoints a full
    // snapshot when needed. The journal-append arm is not ported, so this save
    // always writes the full snapshot — the same full-checkpoint path upstream
    // takes after a corrupt journal. Re-activate the incremental arm at cutover
    // when the session-persist layer lands.
    pub fn save(&mut self) -> anyhow::Result<()> {
        self.updated_at = Utc::now();
        let path = session_path(&self.id)?;
        crate::tui::jcode_app::storage::write_json_fast(&path, self)?;
        self.reset_persist_state(true);
        Ok(())
    }

    // [port-source] jcode-base/src/session.rs:1068-1071 (mark_crashed) + :1109-1134
    // (detect_crash) — verbatim; is_pid_running re-rooted to super::platform.
    pub fn mark_crashed(&mut self, message: Option<String>) {
        self.status = SessionStatus::Crashed { message };
        crate::tui::jcode_app::storage::unregister_active_pid(&self.id);
    }

    pub fn detect_crash(&mut self) -> bool {
        if self.status != SessionStatus::Active {
            return false;
        }

        if let Some(pid) = self.last_pid {
            if !crate::tui::jcode_app::platform::is_process_running(pid) {
                self.mark_crashed(Some(format!(
                    "Process {} exited unexpectedly (no shutdown signal captured)",
                    pid
                )));
                return true;
            }
        } else {
            // No PID info (older sessions): fall back to age heuristic
            let age = chrono::Utc::now().signed_duration_since(self.updated_at);
            if age.num_seconds() > 120 {
                self.mark_crashed(Some(
                    "Stale active session (possible abrupt termination)".to_string(),
                ));
                return true;
            }
        }

        false
    }

    // [port-source] jcode-base/src/session.rs:858-874 (mark_saved) — verbatim.
    /// Save/bookmark this session with an optional label.
    ///
    /// A label is the name the user chose for the session, so it also becomes
    /// the session's display title everywhere sessions are listed.
    pub fn mark_saved(&mut self, label: Option<String>) {
        self.saved = true;
        let label = label.and_then(|label| {
            let label = label.trim();
            (!label.is_empty()).then(|| label.to_string())
        });
        if let Some(label) = label {
            self.custom_title = Some(label.clone());
            self.save_label = Some(label);
            self.updated_at = Utc::now();
        }
    }

    // [port-source] jcode-base/src/session.rs:885-891 (rename_title) — verbatim.
    /// Set or clear the user-provided display title.
    ///
    /// This intentionally does not change the immutable session id, memorable
    /// short name, generated title, provider session id, or saved/bookmark label.
    pub fn rename_title(&mut self, title: Option<String>) {
        self.custom_title = title.and_then(|title| {
            let title = title.trim();
            (!title.is_empty()).then(|| title.to_string())
        });
        self.updated_at = Utc::now();
    }

    // [port-source] jcode-base/src/session/persistence.rs:301-304 — verbatim.
    pub fn load(session_id: &str) -> anyhow::Result<Self> {
        let path = session_path(session_id)?;
        Self::load_from_path(&path)
    }

    // [port-source] jcode-base/src/session.rs:505-533 — verbatim.
    fn journal_meta(&self) -> SessionJournalMeta {
        SessionJournalMeta {
            parent_id: self.parent_id.clone(),
            title: self.title.clone(),
            custom_title: self.custom_title.clone(),
            system_prompt: self.system_prompt.clone(),
            updated_at: self.updated_at,
            compaction: self.compaction.clone(),
            provider_session_id: self.provider_session_id.clone(),
            model_usage_turn_id: self.model_usage_turn_id.clone(),
            provider_key: self.provider_key.clone(),
            model: self.model.clone(),
            reasoning_effort: self.reasoning_effort.clone(),
            subagent_model: self.subagent_model.clone(),
            improve_mode: self.improve_mode,
            autoreview_enabled: self.autoreview_enabled,
            autojudge_enabled: self.autojudge_enabled,
            is_canary: self.is_canary,
            testing_build: self.testing_build.clone(),
            working_dir: self.working_dir.clone(),
            short_name: self.short_name.clone(),
            status: self.status.clone(),
            last_pid: self.last_pid,
            last_active_at: self.last_active_at,
            is_debug: self.is_debug,
            saved: self.saved,
            save_label: self.save_label.clone(),
        }
    }

    // [port-source] jcode-base/src/session.rs:535-548 — verbatim.
    fn reset_persist_state(&mut self, snapshot_exists: bool) {
        self.persist_state = SessionPersistState {
            snapshot_exists,
            messages_len: self.messages.len(),
            env_snapshots_len: self.env_snapshots.len(),
            memory_injections_len: self.memory_injections.len(),
            replay_events_len: self.replay_events.len(),
            messages_mode: PersistVectorMode::Clean,
            env_snapshots_mode: PersistVectorMode::Clean,
            memory_injections_mode: PersistVectorMode::Clean,
            replay_events_mode: PersistVectorMode::Clean,
            last_meta: Some(self.journal_meta()),
        };
    }

    // [port-source] jcode-base/src/session.rs:550-559 — verbatim.
    fn reset_provider_messages_cache(&mut self) {
        self.provider_messages_cache.clear();
        self.provider_message_prefix_hashes_cache.clear();
        self.provider_messages_cache_len = 0;
        self.provider_messages_cache_mode = PersistVectorMode::Full;
        self.memory_profile_cache.provider_cache_count = 0;
        self.memory_profile_cache.provider_cache_json_bytes = 0;
        self.memory_profile_cache.provider_cache_stats = ContentBlockMemoryStats::default();
    }

    // [port-source] jcode-base/src/session.rs:594-596 — verbatim.
    fn mark_memory_profile_dirty(&mut self) {
        self.memory_profile_dirty = true;
    }

    // [port-source] jcode-base/src/session.rs:598-630 — verbatim.
    fn rebuild_memory_profile_cache(&mut self) {
        let message_stats =
            summarize_message_content(self.messages.iter().map(|message| &message.content));
        let provider_cache_stats = summarize_message_content(
            self.provider_messages_cache
                .iter()
                .map(|message| &message.content),
        );

        self.memory_profile_cache = SessionMemoryProfileCache {
            messages_count: self.messages.len(),
            messages_json_bytes: self
                .messages
                .iter()
                .map(crate::tui::jcode_app::process_memory::estimate_json_bytes)
                .sum(),
            message_stats,
            env_snapshots_count: self.env_snapshots.len(),
            env_snapshots_json_bytes: self
                .env_snapshots
                .iter()
                .map(crate::tui::jcode_app::process_memory::estimate_json_bytes)
                .sum(),
            memory_injections_count: self.memory_injections.len(),
            memory_injections_json_bytes: self
                .memory_injections
                .iter()
                .map(crate::tui::jcode_app::process_memory::estimate_json_bytes)
                .sum(),
            replay_events_count: self.replay_events.len(),
            replay_events_json_bytes: self
                .replay_events
                .iter()
                .map(crate::tui::jcode_app::process_memory::estimate_json_bytes)
                .sum(),
            provider_cache_count: self.provider_messages_cache.len(),
            provider_cache_json_bytes: self
                .provider_messages_cache
                .iter()
                .map(crate::tui::jcode_app::process_memory::estimate_json_bytes)
                .sum(),
            provider_cache_stats,
        };
        self.memory_profile_dirty = false;
    }

    // [port-source] jcode-base/src/session.rs:673-679 — verbatim.
    fn mark_messages_append_dirty(&mut self) {
        if self.persist_state.messages_mode != PersistVectorMode::Full {
            self.persist_state.messages_mode = PersistVectorMode::Append;
        }
        if self.provider_messages_cache_mode != PersistVectorMode::Full {
            self.provider_messages_cache_mode = PersistVectorMode::Append;
        }
    }

    // [port-source] jcode-base/src/session.rs:1301-1313 — verbatim.
    pub fn append_stored_message(&mut self, message: StoredMessage) {
        self.memory_profile_cache.messages_count += 1;
        self.memory_profile_cache.messages_json_bytes +=
            crate::tui::jcode_app::process_memory::estimate_json_bytes(&message);
        self.memory_profile_cache
            .message_stats
            .merge_from(&summarize_blocks(&message.content));
        self.adopt_prompt_title(&message);
        self.messages.push(message);
        self.mark_messages_append_dirty();
    }

    // [port-source] jcode-base/src/session.rs:1315-1327 — verbatim.
    /// Name an untitled session after its first real user prompt so lists show
    /// something recognizable instead of a generic placeholder. Renames,
    /// bookmark labels, and todo goals still take precedence at display time.
    fn adopt_prompt_title(&mut self, message: &StoredMessage) {
        if self.title.is_some()
            || message.role != Role::User
            || !is_visible_conversation_message(message)
        {
            return;
        }
        self.title = message.content.iter().find_map(|block| match block {
            ContentBlock::Text { text, .. } => prompt_title(text),
            _ => None,
        });
    }

    // [port-source] jcode-base/src/session.rs:1329-1341 — verbatim.
    /// Give sessions recorded before prompt titles existed the same fallback.
    pub(crate) fn backfill_prompt_title(&mut self) {
        if self.title.is_some() {
            return;
        }
        let first_prompt = self
            .messages
            .iter()
            .find(|message| message.role == Role::User && is_visible_conversation_message(message))
            .cloned();
        if let Some(message) = first_prompt {
            self.adopt_prompt_title(&message);
        }
    }
}

// --- jcode-base/src/session/render.rs (the render_messages chain) ---
// [port-excision] render_images (:272), has_rendered_images (:276), and
// summarize_tool_calls (:284) are unreferenced by operant's callers and not
// ported. parse_attached_image_label (:252) and is_attached_image_label_text
// (:268) were already ported in the original wave (top of this file) and are
// reused, not re-ported.

/// Number of compacted historical messages shown by default in the UI.
///
/// Compaction still keeps older history out of the active model context, but
/// the transcript should retain recent continuity instead of replacing the
/// entire compacted prefix with a marker.
pub const DEFAULT_VISIBLE_COMPACTED_HISTORY_MESSAGES: usize = 64;

/// Format persisted reasoning/thinking text into the dim+italic markdown used
/// by the live streaming path. Each line is wrapped via the shared `reasoning_line_markup` so resumed
/// sessions render reasoning identically to how it streamed, terminated by a
/// blank line so following answer text renders as a normal paragraph.
///
/// Honors the active `reasoning_display` mode so re-rendered history (reload,
/// resume, remote sync, compaction-window expand) matches the live behavior:
/// - `Off`: persisted reasoning is hidden entirely.
/// - `Current`: only the *live* reasoning block is ever shown, so historical
///   reasoning is hidden on re-render (the live block already streamed and was
///   discarded once the model answered), matching the ephemeral live behavior.
/// - `Full`: every reasoning line is shown (classic behavior).
fn format_reasoning_markup(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let mode = crate::tui::jcode_app::config_shim::config()
        .display
        .reasoning_display();
    match mode {
        // In both `Off` and `Current` modes persisted reasoning is not re-rendered:
        // `Current` only ever shows the live block, which is discarded once the
        // model answers, so reloaded history shows no past reasoning.
        crate::tui::jcode_app::config_shim::ReasoningDisplayMode::Off
        | crate::tui::jcode_app::config_shim::ReasoningDisplayMode::Current => {
            return String::new();
        }
        crate::tui::jcode_app::config_shim::ReasoningDisplayMode::Full => {}
    }
    let mut out = String::new();
    for line in text.split('\n') {
        out.push_str(&crate::tui::jcode_render_core::reasoning_line_markup(line));
    }
    // Blank line terminates the reasoning block.
    out.push('\n');
    out
}

fn is_internal_system_reminder(msg: &StoredMessage) -> bool {
    msg.content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.trim_start()),
            _ => None,
        })
        .is_some_and(|text| text.starts_with("<system-reminder>"))
}

/// True when a stored user message is a synthetic auto-poke continuation
/// (incomplete-todos poke or todo confidence summary). These are persisted as
/// `Role::User` so the model treats them as a normal continuation turn, but
/// the live UI never shows them as user prompts (it shows an "Auto-poking..."
/// notice instead). Re-rendered history must not resurrect them as the user's
/// "last prompt" after a reload/resume/remote attach, so they render with the
/// system role.
fn is_auto_poke_user_message(msg: &StoredMessage) -> bool {
    matches!(msg.role, Role::User)
        && msg.display_role.is_none()
        && msg
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .is_some_and(crate::tui::jcode_app::todo::is_auto_poke_message)
}

/// Short user-facing stand-in for a synthetic gate continuation, when one
/// exists. `None` means render the stored text as-is.
fn auto_poke_user_message_display_summary(msg: &StoredMessage) -> Option<&'static str> {
    if !is_auto_poke_user_message(msg) {
        return None;
    }
    msg.content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .and_then(crate::tui::jcode_app::todo::auto_poke_display_summary)
}

fn stored_message_renders_visible_message(msg: &StoredMessage) -> bool {
    if is_internal_system_reminder(msg) {
        return false;
    }

    msg.content.iter().any(|block| match block {
        ContentBlock::Text { text, .. } => !text.is_empty(),
        ContentBlock::ToolResult { .. } => true,
        _ => false,
    })
}

/// A compacted prefix is only truncated when it is genuinely large. Below this
/// many renderable messages we always show the whole prefix, even if the
/// `requested_visible` window is smaller, so short histories stay intact.
const COMPACTED_HISTORY_MIN_RENDERABLE_TO_TRUNCATE: usize = 80;

/// We never truncate a compacted prefix that contains this many user turns or
/// fewer. This guarantees a single very long turn (1 turn, possibly hundreds
/// of tool messages) is never cut off, and short multi-turn histories stay whole.
const COMPACTED_HISTORY_MIN_TURNS_TO_TRUNCATE: usize = 5;

fn stored_message_is_user_turn(msg: &StoredMessage) -> bool {
    matches!(msg.role, Role::User)
        && msg.display_role.is_none()
        && !is_auto_poke_user_message(msg)
        && stored_message_renders_visible_message(msg)
}

fn compacted_history_render_window(
    messages: &[StoredMessage],
    compacted_count: usize,
    requested_visible: usize,
) -> (usize, RenderedCompactedHistoryInfo) {
    let compacted_count = compacted_count.min(messages.len());
    let compacted_prefix = &messages[..compacted_count];
    let total_renderable = compacted_prefix
        .iter()
        .filter(|msg| stored_message_renders_visible_message(msg))
        .count();
    let total_turns = compacted_prefix
        .iter()
        .filter(|msg| stored_message_is_user_turn(msg))
        .count();

    // Guardrails: only truncate when the prefix is BOTH very long AND has more
    // than a handful of turns. Otherwise show everything. This avoids cutting a
    // single long turn or a short multi-turn history.
    let must_show_all = total_renderable < COMPACTED_HISTORY_MIN_RENDERABLE_TO_TRUNCATE
        || total_turns <= COMPACTED_HISTORY_MIN_TURNS_TO_TRUNCATE;

    let visible_renderable = if must_show_all {
        total_renderable
    } else {
        requested_visible.min(total_renderable)
    };
    let remaining_renderable = total_renderable.saturating_sub(visible_renderable);

    let mut render_start_idx = if visible_renderable == 0 {
        compacted_count
    } else if remaining_renderable == 0 {
        0
    } else {
        let mut seen = 0usize;
        let mut start_idx = compacted_count;
        for (idx, msg) in compacted_prefix.iter().enumerate().rev() {
            if stored_message_renders_visible_message(msg) {
                seen += 1;
                if seen >= visible_renderable {
                    start_idx = idx;
                    break;
                }
            }
        }
        start_idx
    };

    // Snap the start back to a user-turn boundary so the visible window begins
    // at the start of a prompt. This keeps prompt numbering and turn grouping
    // coherent (we never render a half turn at the top).
    if render_start_idx > 0 && render_start_idx < compacted_count {
        let mut boundary = render_start_idx;
        while boundary > 0 && !stored_message_is_user_turn(&compacted_prefix[boundary]) {
            boundary -= 1;
        }
        if stored_message_is_user_turn(&compacted_prefix[boundary]) {
            render_start_idx = boundary;
        }
    }

    // Recompute visible/remaining/hidden after snapping so the reported counts
    // match what is actually rendered.
    let visible_renderable = compacted_prefix[render_start_idx..]
        .iter()
        .filter(|msg| stored_message_renders_visible_message(msg))
        .count();
    let remaining_renderable = total_renderable.saturating_sub(visible_renderable);
    let hidden_user_prompts = compacted_prefix[..render_start_idx]
        .iter()
        .filter(|msg| stored_message_is_user_turn(msg))
        .count();

    (
        render_start_idx,
        RenderedCompactedHistoryInfo {
            total_messages: total_renderable,
            visible_messages: visible_renderable,
            remaining_messages: remaining_renderable,
            hidden_user_prompts,
        },
    )
}

fn image_source_for_message(role: Role, tool: Option<&ToolCall>) -> RenderedImageSource {
    if let Some(tool) = tool {
        return RenderedImageSource::ToolResult {
            tool_name: tool.name.clone(),
        };
    }

    match role {
        Role::User => RenderedImageSource::UserInput,
        Role::Assistant => RenderedImageSource::Other {
            role: "assistant".to_string(),
        },
    }
}

fn image_anchor_for_message(
    rendered_role: &str,
    tool: Option<&ToolCall>,
    user_prompt_ordinal: usize,
) -> Option<RenderedImageAnchor> {
    if let Some(tool) = tool {
        return Some(RenderedImageAnchor::ToolCall {
            id: tool.id.clone(),
        });
    }
    if rendered_role == "user" {
        return Some(RenderedImageAnchor::UserPrompt {
            ordinal: user_prompt_ordinal,
        });
    }
    None
}

fn fallback_image_label_for_tool(tool: &ToolCall) -> Option<String> {
    tool.input
        .get("file_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Convert stored session messages into renderable messages (including tool output).
pub fn render_messages(session: &Session) -> Vec<RenderedMessage> {
    render_messages_and_images(session).0
}

pub fn render_messages_and_images(session: &Session) -> (Vec<RenderedMessage>, Vec<RenderedImage>) {
    let (messages, images, _) = render_messages_and_images_with_compacted_history(
        session,
        DEFAULT_VISIBLE_COMPACTED_HISTORY_MESSAGES,
    );
    (messages, images)
}

pub fn render_messages_and_images_with_compacted_history(
    session: &Session,
    compacted_history_visible: usize,
) -> (
    Vec<RenderedMessage>,
    Vec<RenderedImage>,
    Option<RenderedCompactedHistoryInfo>,
) {
    let mut rendered: Vec<RenderedMessage> = Vec::new();
    let mut images: Vec<RenderedImage> = Vec::new();
    let mut tool_map: HashMap<String, ToolCall> = HashMap::new();
    // 0-based ordinal of the next rendered user prompt, used to anchor pasted
    // user images to their prompt in the transcript.
    let mut user_prompt_count = 0usize;
    let compacted_count = session
        .compaction
        .as_ref()
        .map(|state| state.compacted_count.min(session.messages.len()))
        .unwrap_or(0);
    let (render_start_idx, compacted_info) = compacted_history_render_window(
        &session.messages,
        compacted_count,
        compacted_history_visible,
    );
    let compacted_info = (compacted_count > 0).then_some(compacted_info);

    if compacted_count > 0 {
        let visible_compacted = compacted_info
            .as_ref()
            .map(|info| info.visible_messages)
            .unwrap_or(0);
        let remaining_compacted = compacted_info
            .as_ref()
            .map(|info| info.remaining_messages)
            .unwrap_or(0);
        let total_compacted = compacted_info
            .as_ref()
            .map(|info| info.total_messages)
            .unwrap_or(0);
        let content = if remaining_compacted == 0 {
            format!(
                "Earlier conversation compacted - showing all {} compacted historical messages. Redraw may be slower while this view is open.",
                total_compacted
            )
        } else if visible_compacted == 0 {
            format!(
                "Earlier conversation compacted - {} historical messages hidden from the UI. Scroll to the top to load older history.",
                remaining_compacted
            )
        } else {
            format!(
                "Earlier conversation compacted - {} older historical messages hidden. Showing {} of {} compacted messages. Scroll to the top to load more.",
                remaining_compacted, visible_compacted, total_compacted
            )
        };
        rendered.push(RenderedMessage {
            response_stats: None,
            role: "system".to_string(),
            content,
            tool_calls: Vec::new(),
            tool_data: None,
            stored_index: None,
        });
    }

    for (stored_index, msg) in session.messages.iter().enumerate().skip(render_start_idx) {
        if is_internal_system_reminder(msg) {
            continue;
        }

        let role = match msg.display_role {
            Some(StoredDisplayRole::System) => "system",
            Some(StoredDisplayRole::BackgroundTask) => "background_task",
            None if is_auto_poke_user_message(msg) => "system",
            None if is_scheduled_task_message(msg) => "system",
            None => match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
        };
        // Gate continuations are model-facing instructions naming specific
        // todos and fields. In the transcript the user only wants to know a
        // check happened, so replace the body with a one-liner.
        if let Some(summary) = auto_poke_user_message_display_summary(msg) {
            rendered.push(RenderedMessage {
                response_stats: None,
                role: "system".to_string(),
                content: summary.to_string(),
                tool_calls: Vec::new(),
                tool_data: None,
                stored_index: Some(stored_index),
            });
            continue;
        }
        let message_role = msg.role.clone();
        let mut text = String::new();
        // Reasoning is accumulated separately so it can be rendered *before* the
        // answer text, matching the live streaming order. Providers persist the
        // assistant turn as `[Text, ReasoningTrace, ToolUse]`, so appending
        // reasoning into `text` in block order would otherwise show the thinking
        // *after* the answer on resume/re-render.
        let mut reasoning = String::new();
        let mut tool_calls: Vec<String> = Vec::new();
        let mut current_tool: Option<ToolCall> = None;
        let mut last_image_idx: Option<usize> = None;
        // Images from blocks with no owning tool result (e.g. a pasted user
        // screenshot). Their user-prompt ordinal is only known once we know the
        // message actually renders a user prompt, so patch them afterwards.
        let mut pending_prompt_image_indices: Vec<usize> = Vec::new();

        for block in &msg.content {
            match block {
                ContentBlock::Text { text: t, .. } => {
                    // The `[Attached image associated with the preceding tool
                    // result: ...]` block is synthetic metadata jcode injects so
                    // the model can associate a label with the image. It lives in
                    // the same (user) turn as the tool result, so if we rendered
                    // it as message text it would surface as a bogus user prompt
                    // (showing up as the "last prompt" instead of the user's real
                    // message). Consume it into the image label and never display
                    // it.
                    if let Some(label) = parse_attached_image_label(t) {
                        if let Some(last_idx) = last_image_idx
                            && let Some(image) = images.get_mut(last_idx)
                        {
                            image.label = Some(label);
                        }
                        continue;
                    }
                    text.push_str(t);
                }
                ContentBlock::ToolUse {
                    id,
                    name,
                    input,
                    thought_signature,
                } => {
                    let tool_call = ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                        intent: ToolCall::intent_from_input(input),
                        thought_signature: thought_signature.clone(),
                    };
                    tool_map.insert(id.clone(), tool_call);
                    tool_calls.push(name.clone());
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    let combined = format!("{}{}", reasoning, text);
                    if !combined.is_empty() {
                        if role == "user" && !is_attached_image_label_text(&text) {
                            user_prompt_count += 1;
                        }
                        text.clear();
                        reasoning.clear();
                        rendered.push(RenderedMessage {
                            response_stats: None,
                            role: role.to_string(),
                            content: combined,
                            tool_calls: tool_calls.clone(),
                            tool_data: None,
                            stored_index: Some(stored_index),
                        });
                    }

                    let tool_data = tool_map.get(tool_use_id).cloned().or_else(|| {
                        Some(ToolCall {
                            id: tool_use_id.clone(),
                            name: "tool".to_string(),
                            input: serde_json::Value::Null,
                            intent: None,
                            thought_signature: None,
                        })
                    });
                    current_tool = tool_data.clone();

                    rendered.push(RenderedMessage {
                        response_stats: None,
                        role: "tool".to_string(),
                        content: content.clone(),
                        tool_calls: Vec::new(),
                        tool_data,
                        stored_index: Some(stored_index),
                    });
                }
                ContentBlock::Reasoning { text: t } | ContentBlock::ReasoningTrace { text: t } => {
                    reasoning.push_str(&format_reasoning_markup(t));
                }
                ContentBlock::AnthropicThinking { .. } | ContentBlock::OpenAIReasoning { .. } => {}
                ContentBlock::Image { media_type, data } => {
                    let anchor =
                        image_anchor_for_message(role, current_tool.as_ref(), user_prompt_count);
                    let is_pending_prompt_anchor = current_tool.is_none() && role == "user";
                    images.push(RenderedImage {
                        history_message_index: current_tool.as_ref().map(|_| rendered.len()),
                        media_type: media_type.clone(),
                        data: data.clone(),
                        label: current_tool
                            .as_ref()
                            .and_then(fallback_image_label_for_tool),
                        source: image_source_for_message(
                            message_role.clone(),
                            current_tool.as_ref(),
                        ),
                        anchor,
                    });
                    last_image_idx = Some(images.len().saturating_sub(1));
                    if is_pending_prompt_anchor {
                        pending_prompt_image_indices.push(images.len() - 1);
                    }
                }
                ContentBlock::OpenAICompaction { .. } | ContentBlock::ToolReference { .. } => {}
            }
        }

        let combined = format!("{}{}", reasoning, text);
        if !combined.is_empty() {
            if role == "user" && !is_attached_image_label_text(&text) {
                user_prompt_count += 1;
            }
            rendered.push(RenderedMessage {
                response_stats: None,
                role: role.to_string(),
                content: combined,
                tool_calls,
                tool_data: None,
                stored_index: Some(stored_index),
            });
        } else if !pending_prompt_image_indices.is_empty() {
            // The message carried images but produced no rendered user prompt;
            // drop the anchor so these images fall back to the transcript tail
            // instead of pointing at the wrong prompt.
            for idx in pending_prompt_image_indices {
                if let Some(image) = images.get_mut(idx) {
                    image.anchor = None;
                }
            }
        }
    }

    response_stats::attach(&session.messages, &mut rendered);
    (rendered, images, compacted_info)
}

// --- jcode-base/src/session/render/response_stats.rs:1-85 ---
// Nested verbatim; `super::` paths re-rooted one level (upstream nests at
// session/render/response_stats.rs, here at session::response_stats), so
// upstream's `super::super::is_scheduled_task_message` becomes
// `super::is_scheduled_task_message` and the jcode_session_types imports come
// from this module. Tests excised.
mod response_stats {
    //! Aggregate persisted provider calls, not rendered rows (tool-only calls may
    //! render no assistant row). Work over the full record even for a compacted view.
    use super::{
        ContentBlock, RenderedMessage, ResponseStats, Role, StoredMessage, StoredTokenUsage,
        is_auto_poke_user_message, is_internal_system_reminder, is_scheduled_task_message,
        parse_attached_image_label,
    };

    fn is_user_prompt(message: &StoredMessage) -> bool {
        matches!(message.role, Role::User)
            && message.display_role.is_none()
            && !is_internal_system_reminder(message)
            && !is_auto_poke_user_message(message)
            && !is_scheduled_task_message(message)
            && message.content.iter().any(|block| match block {
                ContentBlock::Text { text, .. } => {
                    !text.trim().is_empty() && parse_attached_image_label(text).is_none()
                }
                ContentBlock::Image { .. } => true,
                _ => false,
            })
            // Tool-result rows can include synthetic text and images. They are a
            // continuation of the same turn, not another human prompt.
            && !message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
    }

    pub(super) fn attach(messages: &[StoredMessage], rendered: &mut [RenderedMessage]) {
        let row_by_stored_index: std::collections::HashMap<_, _> = rendered
            .iter()
            .enumerate()
            .filter(|(_, row)| row.role == "assistant")
            .filter_map(|(i, row)| row.stored_index.map(|stored| (stored, i)))
            .collect();
        let mut start = 0;
        for end in 0..=messages.len() {
            if end == messages.len() || is_user_prompt(&messages[end]) {
                attach_turn(messages, start, end, rendered, &row_by_stored_index);
                start = end;
            }
        }
    }

    fn attach_turn(
        messages: &[StoredMessage],
        start: usize,
        end: usize,
        rendered: &mut [RenderedMessage],
        row_by_stored_index: &std::collections::HashMap<usize, usize>,
    ) {
        let assistants: Vec<_> = (start..end)
            .filter(|&i| matches!(messages[i].role, Role::Assistant))
            .collect();
        let Some(&last) = assistants.last() else {
            return;
        };
        // A pending tool call is not a completed response. Do not attach its usage
        // to an earlier visible assistant row, even when the pending call is hidden.
        if messages[last]
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
        {
            return;
        }
        let Some(&row_index) = row_by_stored_index.get(&last) else {
            return;
        };
        let row = &mut rendered[row_index];
        let sum = |field: fn(&StoredTokenUsage) -> Option<u64>| {
            assistants.iter().try_fold(0u64, |total, &i| {
                total.checked_add(field(messages[i].token_usage.as_ref()?)?)
            })
        };
        let stats = ResponseStats {
            duration_secs: None,
            input_tokens: sum(|usage| Some(usage.input_tokens)),
            output_tokens: sum(|usage| Some(usage.output_tokens)),
            cache_read_tokens: sum(|usage| usage.cache_read_input_tokens),
            cache_creation_tokens: sum(|usage| usage.cache_creation_input_tokens),
        };
        if stats.input_tokens.is_some()
            || stats.output_tokens.is_some()
            || stats.cache_read_tokens.is_some()
            || stats.cache_creation_tokens.is_some()
        {
            row.response_stats = Some(stats);
        }
    }
}
