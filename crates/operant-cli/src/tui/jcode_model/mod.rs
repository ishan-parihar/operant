// Message data model vendored from jcode (MIT, Copyright (c) 2025 Jeremy Huang).
//
// Upstream: `parent-projects/jcode` @ commit `0a9dc7805` —
// `crates/jcode-tui-messages/src/` (all files except `swarm_collapse.rs`,
// which is swarm-only and has no operant consumer) plus
// `crates/jcode-tui-tool-display/src/` ported as `tool_display.rs`.
// Structure and logic are preserved verbatim per W1 of
// docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md; the only lines touched are
// import paths (upstream sibling crates re-rooted to this module and to
// `vendor_types`), jcode issue-tracker references in doc comments, and the
// single adaptation point below. Every later renderer is a pure
// `(&DisplayMessage, width) -> Vec<Line>`, so nothing renders until this
// seam exists.
//
// `adapter.rs` is the one deliberate adaptation: operant transcript state to
// [`DisplayMessage`]. Everything else is a verbatim port.
//
// Lint gates (mirrors `vendor/style/mod.rs`): upstream tests are ported as-is
// and cover only a subset of the API, so in test builds many verbatim items
// (e.g. most `DisplayMessage` constructors) read as dead; covering them with
// synthetic call-tests would be tautological. The re-export `use`s read as
// unused until W3 imports them.

// In test builds only, silence dead_code inside this tree; the non-test build
// is governed by the `expect` on the `pub mod jcode_model;` declaration in
// `tui/mod.rs`, which must keep firing loudly until W3 wires the renderers in.
#![cfg_attr(test, allow(dead_code))]

mod adapter;
mod anchor;
mod cache;
mod message;
mod prepared;
mod tool_display;
mod vendor_types;
mod wrapped_line_map;

#[allow(unused_imports)]
pub use adapter::display_messages;
#[allow(unused_imports)]
pub use anchor::{
    Anchor, ContentPos, anchor_at_row, content_pos_at_row, message_row_ranges, resolve,
    resolve_content_pos,
};
#[allow(unused_imports)]
pub use cache::{
    MessageCacheContext, centered_wrap_width, get_cached_message_lines,
    left_pad_lines_for_centered_mode,
};
#[allow(unused_imports)]
pub use message::{
    DisplayMessage, TranscriptPreviewLabels, display_messages_from_rendered_messages,
    latest_user_transcript_preview, normalize_transcript_preview_text, transcript_preview_line,
    transcript_preview_lines, truncate_transcript_preview,
};
#[allow(unused_imports)]
pub use prepared::{
    CopyTarget, EditToolRange, ImageRegion, ImageRegionRender, MessageBoundary, PreparedChatFrame,
    PreparedMessages, PreparedSection, PreparedSectionKind,
};
#[allow(unused_imports)]
pub use tool_display::{
    canonical_tool_name, concise_tool_error_summary, edit_render_name, is_edit_tool_name,
    resolve_display_tool_name, tool_output_looks_failed, truncate_middle_display,
};
#[allow(unused_imports)]
pub use vendor_types::{
    DiagramDisplayMode, DiffDisplayMode, RenderedMessage, ResponseStats, ToolCall,
};
#[allow(unused_imports)]
pub use wrapped_line_map::WrappedLineMap;
