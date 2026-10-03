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

