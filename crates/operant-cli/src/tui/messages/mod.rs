//! Message type renderers for the TUI.
//! Mirrors src/components/messages/ and src/components/Messages.tsx.
//!
//! Each message type has a dedicated render function.

use std::collections::HashMap;

use ratatui::style::Color;

pub(crate) mod cache;
mod helpers;
mod markdown;
mod markdown_enhanced;
mod tools;

// [port-decision] iter-648: commands/transcript (the operant transcript render
// chain) are deleted — the ported jcode chrome renders the transcript now.
// These re-exports survive for the operant-only surfaces still on this module:
// the async raster producers (mermaid/latex hash + preview render), the MCP
// view's fold labels, and render/tools' summary extractor.
pub(crate) use helpers::*;
pub use markdown::render_markdown;
pub use tools::*;

/// Context passed to all renderers.
pub struct RenderContext {
    /// Current terminal width (for word-wrap decisions).
    pub width: u16,
    /// Whether to show thinking blocks.
    pub show_thinking: bool,
    /// Maps `tool_use_id` → `tool_name` so ToolResult blocks can dispatch to
    /// the correct specialized renderer (e.g. Bash output vs. generic result).
    pub tool_names: HashMap<String, String>,
    /// Set of thinking block content hashes that are expanded per-block.
    pub expanded_thinking: std::collections::HashSet<u64>,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            width: 80,
            show_thinking: false,
            tool_names: HashMap::new(),
            expanded_thinking: std::collections::HashSet::new(),
        }
    }
}

const MAX_USER_PROMPT_DISPLAY_CHARS: usize = 10_000;
const TRUNCATE_USER_PROMPT_HEAD_CHARS: usize = 2_500;
const TRUNCATE_USER_PROMPT_TAIL_CHARS: usize = 2_500;

// The accent is read through `theme_colors::accent()` at every use site rather
// than frozen into a `const` here: the accessors are runtime `fn`s (the active
// palette lives behind a lock), so a `const` would pin every theme to the
// default theme's amber.
//
// `TRANSCRIPT_USER_BG` / `TRANSCRIPT_CHIP_BG` used to sit in this list. Both were
// read only by `helpers.rs`, so their use sites now name the role directly:
// `UserBg` for the message panel and `SelectionBg` for the chip raised above it.
// The remaining five are still read *as values* by `transcript.rs`,
// `commands.rs` and `tools.rs`; turning a `const` into a role accessor turns it
// into an `fn` and breaks every one of those call sites, so they migrate with
// those files rather than ahead of them.
const TRANSCRIPT_TEXT: Color = Color::Rgb(236, 236, 241);
const TRANSCRIPT_MUTED: Color = Color::Rgb(139, 139, 153);
const TRANSCRIPT_SUBTLE: Color = Color::Rgb(112, 112, 126);

const TOOL_RESULT_MAX_LINES: usize = 30;

/// Accent color for goal-event blocks (warm amber/gold).
const GOAL_ACCENT: Color = Color::Rgb(255, 170, 50);
/// Body text color for goal-event objective display.
const GOAL_BODY: Color = Color::Rgb(215, 180, 110);

#[cfg(test)]
mod tests;
