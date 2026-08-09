//! Complete PromptInput — multi-line text editor for the TUI.
//!
//! Mirrors src/components/PromptInput/ (21 files) and src/vim/ (5 files).
//!
//! Features:
//! - Multi-line editing (Shift+Enter for newlines)
//! - Vim Normal/Insert/Visual modes
//! - History navigation (↕ through history.jsonl)
//! - Slash command typeahead
//! - Paste handling (large pastes → placeholder)
//! - Character count + token estimate

mod editing;
mod history;
mod kill_ring;
mod render;
mod state;
mod suggestions;
mod typeahead;
mod vim;
mod vim_command;
mod vim_ops;
mod visual;

#[cfg(test)]
mod tests;

pub use kill_ring::KillRing;
pub use render::{input_height, render_prompt_input, wrap_line};
pub use typeahead::{
    AcceptForSubmitOutcome, TypeaheadSource, TypeaheadSuggestion, compute_typeahead,
    register_typeahead_names,
};
pub use vim::{DotRepeatAction, VimFindKind, VimMode, VimPendingState, apply_vim_key};

use ratatui::style::Color;

const ACCENT_PRIMARY: Color = Color::Rgb(255, 191, 0);
const PROMPT_POINTER: &str = "❯";

pub fn handle_paste(content: &str, paste_counter: &mut u32) -> (String, Option<String>) {
    let line_count = content.lines().count();
    let is_large = line_count >= 3 || content.len() > 150;
    if !is_large {
        return (content.to_string(), None);
    }
    *paste_counter += 1;
    let placeholder = format!("[Pasted ~{} lines #{}]", line_count, paste_counter);
    (placeholder, Some(content.to_string()))
}

/// Normalize a pasted string into a filesystem path if it looks like one.
///
/// Handles:
/// - `file:///path/to/file` — URL-encoded paths
/// - `"C:\path"` / `'/path'` — quoted paths (strips quotes)
/// - Bare absolute paths (`/home/...`, `C:\...`)
///
/// Returns `None` if the text is multiline, not path-shaped, or the resolved
/// path does not exist on the filesystem.  Callers can use the returned
/// `PathBuf` to decide whether to treat the paste as a file attachment.
pub fn detect_pasted_path(text: &str) -> Option<std::path::PathBuf> {
    let trimmed = text.trim();
    // Multiline content is never a bare path.
    if trimmed.contains('\n') {
        return None;
    }
    // Strip outer matching quotes.
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);

    // file:// URL — strip the scheme (skip the leading //).
    let candidate = if let Some(rest) = unquoted.strip_prefix("file://") {
        rest
    } else {
        unquoted
    };

    let path = std::path::Path::new(candidate);
    if path.is_absolute() && path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Kill ring (Emacs-style kill/yank system)
// ---------------------------------------------------------------------------

/// Kill ring stores accumulated kills (deleted text) for cycling through with Alt+Y.
/// Maintains a FIFO list of kills with a current index for cycling backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Default,
    Plan,
    Readonly,
}

/// Full state for the prompt input widget.
#[derive(Debug, Clone)]
pub struct PromptInputState {
    /// Current text content.
    pub text: String,
    /// Cursor position (byte offset into `text`).
    pub cursor: usize,
    /// Current vim mode.
    pub vim_mode: VimMode,
    /// Whether vim mode is enabled.
    pub vim_enabled: bool,
    /// Input mode (default / plan / readonly).
    pub mode: InputMode,
    /// Typeahead suggestions.
    pub suggestions: Vec<TypeaheadSuggestion>,
    /// Currently selected suggestion index.
    pub suggestion_index: Option<usize>,
    /// History entries for ↑↓ navigation.
    pub history: Vec<String>,
    /// Current history position (-1 = not browsing history).
    pub history_pos: Option<usize>,
    /// Saved draft while browsing history.
    pub history_draft: String,
    /// Paste counter for placeholder numbering.
    pub paste_counter: u32,
    /// Stored paste contents: counter → content.
    pub paste_contents: std::collections::HashMap<u32, String>,
    /// Yank buffer for vim operations.
    pub yank_buf: String,
    /// Estimated token count for current text.
    pub token_estimate: usize,
    /// Pending multi-key vim command state (persists across keystrokes).
    pub vim_pending: VimPendingState,
    /// Undo stack: Vec of (text, cursor) snapshots before modifications.
    pub undo_stack: Vec<(String, usize)>,
    /// Visual mode selection anchor (byte offset).
    pub visual_anchor: Option<usize>,
    /// Last f/F/t/T find for `;`/`,` repeat.
    pub last_find: Option<(VimFindKind, char)>,
    /// Named registers: key is the register name char (a-z, 0-9, etc.), value is text.
    pub vim_registers: std::collections::HashMap<char, String>,
    /// Macro recording state: Some(register_name) when recording.
    pub vim_macro_recording: Option<char>,
    /// Recorded macro content (accumulates key descriptions while recording).
    pub vim_macro_content: std::collections::HashMap<char, Vec<String>>,
    /// Named marks: maps mark char to (text, cursor) snapshots.
    pub vim_marks: std::collections::HashMap<char, (String, usize)>,
    /// The last modifying command for dot-repeat.
    pub vim_dot_action: Option<DotRepeatAction>,
    /// Pending insert-mode text (accumulates between entering and leaving insert mode).
    vim_insert_text_before: Option<String>,
    /// Command-line buffer for `:` command mode.
    pub vim_command_buf: String,
    /// In-prompt search buffer for `/` search mode.
    pub vim_search_buf: String,
    /// Last executed search pattern for `n`/`N` navigation.
    pub vim_search_last: Option<String>,
    /// Set by `:q`/`:wq` — the app loop should check and honour this.
    pub vim_quit_requested: bool,
    /// Pending image attachments (from clipboard paste) to be sent with next message.
    pub pending_images: Vec<crate::image_paste::PastedImage>,
    /// Emacs-style kill ring for Ctrl+K, Ctrl+U, Ctrl+W operations.
    pub kill_ring: KillRing,
}

impl Default for PromptInputState {
    fn default() -> Self {
        Self::new()
    }
}
