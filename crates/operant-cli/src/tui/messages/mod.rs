//! Message type renderers for the TUI.
//! Mirrors src/components/messages/ and src/components/Messages.tsx.
//!
//! Each message type has a dedicated render function.

use std::collections::HashMap;

use ratatui::style::Color;

pub(crate) mod cache;
mod helpers;
mod markdown_enhanced;
mod tools;

// [port-decision] iter-648: commands/transcript (the operant transcript render
// chain) are deleted — the ported jcode chrome renders the transcript now.
// These re-exports survive for the operant-only surfaces still on this module:
// the async raster producers (mermaid/latex hash + preview render), the MCP
// view's fold labels, and render/tools' summary extractor.
pub(crate) use helpers::*;
// [dead-strata sweep 2026-10-09] the pre-port `markdown` renderer module is
// deleted: the live transcript renders through the vendored renderer
// (operant_app/markdown → operant_markdown). This module keeps helpers,
// cache, tools, and the enhanced markdown helper that still serve live
// consumers.
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

// Transcript color notes: there is deliberately no `TRANSCRIPT_*` const list
// here anymore. The last hardcoded shade read as a value (`TRANSCRIPT_MUTED`,
// the tool-row summary gray) migrated to the `Dim` palette role in `tools.rs`
// (jcode paints the same spans with `dim_color()`); everything else in this
// list was dead after `helpers.rs`'s unused primitives were removed.

#[cfg(test)]
mod tests;
