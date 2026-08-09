//! Message type renderers for the TUI.
//! Mirrors src/components/messages/ and src/components/Messages.tsx.
//!
//! Each message type has a dedicated render function.

use std::collections::HashMap;

use ratatui::style::Color;

mod cache;
mod commands;
mod helpers;
mod markdown;
mod markdown_enhanced;
mod tools;
mod transcript;

pub use commands::*;
pub(crate) use helpers::*;
pub use markdown::render_markdown;
pub use tools::*;
pub use transcript::*;

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

/// Claude orange: Rgb(215, 119, 87)
const ACCENT_PRIMARY: Color = Color::Rgb(255, 191, 0);
const TRANSCRIPT_USER_BG: Color = Color::Rgb(23, 23, 31);
const TRANSCRIPT_CHIP_BG: Color = Color::Rgb(31, 31, 41);
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
