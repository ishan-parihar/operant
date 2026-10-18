// Vendored from jcode (crates/operant-session-types/src/lib.rs + crates/operant-base/src/session/render.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see operant_app/mod.rs for scope.
//! Included from operant-session-types/src/lib.rs: RenderedImageSource (:113),
//! RenderedImageAnchor (:124), RenderedImage (:133). Included from
//! operant-base/src/session/render.rs: parse_attached_image_label (:252, private),
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
// this same submodule (see operant_app/mod.rs). Sources, ported verbatim with
// imports re-rooted to operant_app paths:
// - operant-session-types/src/lib.rs: ResumeTarget (:13-54), ResponseStats
//   (:56-75), RenderedMessage (:77-93), RenderedCompactedHistoryInfo
//   (:95-109), GitState (:215-221), EnvSnapshot (:223-242), StoredMemoryInjection
//   (:244-265), StoredMessage (:267-280), StoredDisplayRole (:282-287),
//   StoredTokenUsage (:289-301), StoredCompactionState (:303-311),
//   SessionImproveMode (:203-213). SessionStatus is NOT re-ported: it is
//   re-exported from crate::tui::operant_model::vendor_types (the dedupe rule).
//   [port-excision] StoredMessage::{to_message, content_preview} (:313-:356)
//   not ported — unreferenced by operant's callers.
// - operant-session-types/src/title.rs:1-93 (prompt_title, strip_blocks).
// - operant-base/src/session/crash.rs:188-199 (CrashedSessionsInfo struct only).
// - operant-storage/src/active_pids.rs: active_pids_dir (:20), streaming_pids_dir
//   (:28), internal_pids_dir (:36), process_is_running (:218-:233),
//   SessionPresence (:249-263), session_presence (:270-:320).
//   [port-excision] SessionCounts/session_counts/user_session_presence/
//   user_session_counts unreferenced here.
// - operant-base/src/session/journal.rs:1-99 (SessionJournalMeta, SessionJournalEntry,
//   PersistVectorMode, SessionPersistState, metadata_requires_snapshot not
//   ported — unreferenced).
// - operant-base/src/session/model.rs:1-32 (StoredReplayEvent,
//   StoredReplayEventKind; SESSION_CONTEXT_PREFIX excised — unreferenced).
// - operant-base/src/session/memory_profile.rs: ContentBlockMemoryStats (:7-160),
//   summarize_message_content (:163), summarize_blocks (:176),
//   SessionMemoryProfileCache (:184-198). [port-excision]
//   SessionMemoryProfileSnapshot (:200-214) unreferenced.
// - operant-base/src/session/storage_paths.rs:1-49 (whole file; the
//   persist_vector_mode_label helper is included verbatim).
// - operant-base/src/session.rs: is_internal_system_reminder_message (:78),
//   is_visible_conversation_message (:89), is_scheduled_task_message (:98),
//   Session (:105-207), journal_meta (:505), reset_persist_state (:535),
//   reset_provider_messages_cache (:550), mark_memory_profile_dirty (:594),
//   rebuild_memory_profile_cache (:598), mark_messages_append_dirty (:673),
//   append_stored_message (:1301), adopt_prompt_title (:1315),
//   backfill_prompt_title (:1329).
// - operant-base/src/session/persistence.rs: Session::load (:301),
//   Session::load_from_path (:232) — see the [port-decision] marker inside.
// - operant-base/src/session/render.rs:1-590 (the render_messages chain; see
//   the [port-excision] note at the render section).
// - operant-base/src/session/render/response_stats.rs:1-85 (nested verbatim;
//   tests excised).
// ============================================================================

/// Re-export, not a re-port: jcode's SessionStatus lives in this repo at
/// crate::tui::operant_model::vendor_types (batch-2 verbatim port incl. its
/// display/icon/detail impls).

// --- operant-session-types/src/lib.rs ---

// --- operant-session-types/src/title.rs ---

// --- operant-base/src/session/crash.rs:188-199 ---

// --- operant-storage/src/active_pids.rs (presence slice) ---

#[cfg(not(unix))]
fn process_is_running(pid: u32) -> bool {
    // Best-effort fallback for platforms where this low-level storage crate does
    // not have a process API. The active PID file is still useful, and stale
    // entries are cleaned up by higher-level session lifecycle code.
    pid != 0
}

// --- operant-base/src/session/journal.rs ---

// --- operant-base/src/session/model.rs:1-32 ---

// --- operant-base/src/session/memory_profile.rs ---

// --- operant-base/src/session/storage_paths.rs ---

// --- operant-base/src/session.rs (Session struct + the picker-facing impl) ---

// --- operant-base/src/session.rs:269-286 (create_with_id's private helpers) ---

// --- operant-base/src/session/render.rs (the render_messages chain) ---
// [port-excision] render_images (:272), has_rendered_images (:276), and
// summarize_tool_calls (:284) are unreferenced by operant's callers and not
// ported. parse_attached_image_label (:252) and is_attached_image_label_text
// (:268) were already ported in the original wave (top of this file) and are
// reused, not re-ported.

/// Sentinel-wrapped dim+italic markup for a reasoning trace, with no
/// display-mode gate — callers that know the turn context decide visibility.
pub(in crate::tui) fn reasoning_markup(text: &str) -> String {
    let mut out = String::new();
    for line in text.split('\n') {
        out.push_str(&crate::tui::operant_render_core::reasoning_line_markup(
            line,
        ));
    }
    // Blank line terminates the reasoning block.
    out.push('\n');
    out
}

// --- operant-base/src/session/render/response_stats.rs:1-85 ---
// Nested verbatim; `super::` paths re-rooted one level (upstream nests at
// session/render/response_stats.rs, here at session::response_stats), so
// upstream's `super::super::is_scheduled_task_message` becomes
// `super::is_scheduled_task_message` and the operant_session_types imports come
// from this module. Tests excised.
mod response_stats {
    //! Aggregate persisted provider calls, not rendered rows (tool-only calls may
    //! render no assistant row). Work over the full record even for a compacted view.
}
