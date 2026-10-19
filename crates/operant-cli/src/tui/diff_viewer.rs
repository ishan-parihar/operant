//! Diff viewer TUI component.
//! Mirrors src/components/diff/ and src/components/StructuredDiff.tsx.
//!
//! Shows a two-pane diff dialog: file list (left) + unified diff detail (right).
//! Keyboard: ↑↓ navigate files, Tab switch pane, t toggle diff type, Esc close.

// (iter-209: FileHistory import deleted — stub removed)
use std::collections::HashMap;
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use crate::tui::overlays::{begin_modal_buf, modal_header_line_area, render_modal_title_buf};
use crate::tui::theme_colors;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single hunk of a unified diff.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// Lines in this hunk.
    pub lines: Vec<DiffLine>,
}

/// A single line in a diff hunk.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    /// Original line number (if applicable).
    pub old_line_no: Option<u32>,
    /// New line number (if applicable).
    pub new_line_no: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Unchanged context line.
    Context,
    /// Added line.
    Added,
    /// Removed line.
    Removed,
    /// Hunk header (@@ line).
    Header,
}

/// Stats for a single file in the diff.
#[derive(Debug, Clone)]
pub struct FileDiffStats {
    /// File path (relative to project root).
    pub path: String,
    /// Number of added lines.
    pub added: u32,
    /// Number of removed lines.
    pub removed: u32,
    /// Is this a binary file?
    pub binary: bool,
    /// Is this a newly created file (no previous version)?
    pub is_new_file: bool,
    /// All hunks for this file.
    pub hunks: Vec<DiffHunk>,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Active pane in the diff dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffPane {
    FileList,
    Detail,
}

/// Full state for the diff viewer dialog.
#[derive(Debug, Clone)]
pub struct DiffViewerState {
    /// All files in the diff.
    pub files: Vec<FileDiffStats>,
    /// Currently selected file index.
    pub selected_file: usize,
    /// Active pane.
    pub active_pane: DiffPane,
    /// Scroll offset for the detail pane (in lines).
    pub detail_scroll: u16,
    /// Rendered line cache: (file_index, terminal_width) → lines.
    render_cache: HashMap<(usize, u16), Vec<String>>,
    /// True while `git diff` is read on a worker thread (TUI-11).
    pub loading: bool,
    /// The worker's result; `drain_git_diff` takes it on a tick. Shared with the
    /// paint copy, which only reads `loading` and never drains.
    git_rx: Option<std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Vec<FileDiffStats>>>>>,
    /// Whether the dialog is open.
    pub visible: bool,
    /// Per-file collapsed state (indexed by file position in `files`).
    pub collapsed: Vec<bool>,
}

mod parse;
mod render;
mod state;

#[cfg(test)]
mod tests;

pub use parse::load_git_diff;
pub use render::render_diff_dialog;

#[cfg(test)]
pub(crate) use parse::parse_unified_diff;
#[cfg(test)]
pub(crate) use render::{
    build_diff_lines, build_inline_diff_spans, format_gutter, truncate_spans_to_width,
};
