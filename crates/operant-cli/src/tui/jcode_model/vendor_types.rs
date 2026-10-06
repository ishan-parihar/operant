// Leaf types pulled from jcode's supporting crates so the ported
// `jcode-tui-messages` model compiles without jcode's crate graph.
// Vendored from jcode, MIT License, Copyright (c) 2025 Jeremy Huang.
//
// Pulled verbatim (fields, derives, serde attributes, doc comments) from
// `parent-projects/jcode` @ `0a9dc7805`:
//   ToolCall                                <- crates/jcode-message-types/src/lib.rs
//   ResponseStats, RenderedMessage           <- crates/jcode-session-types/src/lib.rs
//   DiffDisplayMode, DiagramDisplayMode      <- crates/jcode-config-types/src/lib.rs
//   SessionStatus                           <- crates/jcode-session-types/src/lib.rs
//
// Only the types `jcode-tui-messages` names came over. The upstream impl
// blocks (e.g. `DiffDisplayMode::is_inline`) stayed behind: nothing in this
// module tree calls them, and porting them would add dead code. jcode
// issue-tracker references were stripped from two doc comments.
//
// `CopyTargetKind` was NOT pulled: operant already ports it as
// `crate::tui::copy_targets::CopyTargetKind` (iter-566), and `prepared.rs`
// only stores the kind, never constructs or matches one.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Gemini 3 thought signature attached to this tool call, replayed on
    /// later turns so the Cloud Code backend accepts the function call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
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
    /// notice). Used to map user-facing rewind targets back to the stored
    /// transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_index: Option<usize>,
}

/// How to display file diffs from edit/write tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffDisplayMode {
    /// Don't show diffs at all.
    Off,
    /// Show diffs inline in the chat (default).
    #[default]
    Inline,
    /// Show the full inline diff in the chat without preview truncation.
    #[serde(
        rename = "full-inline",
        alias = "full_inline",
        alias = "fullinline",
        alias = "inline-full",
        alias = "inline_full",
        alias = "inlinefull",
        alias = "full"
    )]
    FullInline,
    /// Show full file with diff highlights in side panel, synced to scroll position.
    File,
}

// [port-decision] DiffDisplayMode impl block: ported verbatim from
// jcode-config-types/src/lib.rs:88 (the enum's only upstream impl). Appended at
// batch-3 because the batch-3 consumers actually call is_inline/is_full_inline/
// is_file/as_str; leaving the impl off was W1's intentional minimal port.
impl DiffDisplayMode {
    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Inline | Self::FullInline)
    }

    pub fn is_full_inline(&self) -> bool {
        matches!(self, Self::FullInline)
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Self::File)
    }

    pub fn has_side_pane(&self) -> bool {
        matches!(self, Self::File)
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Inline,
            Self::Inline => Self::FullInline,
            Self::FullInline => Self::File,
            Self::File => Self::Off,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Inline => "Inline",
            Self::FullInline => "Inline Full",
            Self::File => "File",
        }
    }
}

/// How to display mermaid diagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagramDisplayMode {
    /// Don't show diagrams in dedicated widgets (only inline in messages).
    ///
    /// `inline`/`off` are accepted spellings: diagrams still render inline in
    /// the transcript in this mode, and users reasonably write `"inline"`.
    #[default]
    #[serde(alias = "inline", alias = "off")]
    None,
    /// Show diagrams in info widget margins (opportunistic, if space available).
    Margin,
    /// Show diagrams in a dedicated pinned pane (forces space allocation).
    Pinned,
}

/// Session lifecycle status persisted with each session snapshot/journal
/// entry. Verbatim from crates/jcode-session-types/src/lib.rs:154-:236
/// (struct + display/icon/detail impls — the picker and the session render
/// path call all three).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SessionStatus {
    #[default]
    Active,
    Closed,
    Crashed {
        message: Option<String>,
    },
    Reloaded,
    Compacted,
    RateLimited,
    Error {
        message: String,
    },
}

impl SessionStatus {
    pub fn display(&self) -> &'static str {
        match self {
            SessionStatus::Active => "active",
            SessionStatus::Closed => "closed",
            SessionStatus::Crashed { .. } => "crashed",
            SessionStatus::Reloaded => "reloaded",
            SessionStatus::Compacted => "compacted",
            SessionStatus::RateLimited => "rate limited",
            SessionStatus::Error { .. } => "error",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            SessionStatus::Active => "▶",
            SessionStatus::Closed => "✓",
            SessionStatus::Crashed { .. } => "💥",
            SessionStatus::Reloaded => "🔄",
            SessionStatus::Compacted => "📦",
            SessionStatus::RateLimited => "⏳",
            SessionStatus::Error { .. } => "❌",
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            SessionStatus::Crashed { message } => message.as_deref(),
            SessionStatus::Error { message } => Some(message.as_str()),
            _ => None,
        }
    }
}
