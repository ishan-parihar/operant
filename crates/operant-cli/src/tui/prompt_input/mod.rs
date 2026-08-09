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

mod kill_ring;
mod typeahead;
mod vim;
mod vim_command;

pub use kill_ring::KillRing;
pub use typeahead::{
    AcceptForSubmitOutcome, TypeaheadSource, TypeaheadSuggestion, compute_typeahead,
};
pub use vim::{DotRepeatAction, VimFindKind, VimMode, VimPendingState, apply_vim_key};
use vim::{char_idx_to_byte, is_word_char};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

impl PromptInputState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            vim_mode: VimMode::Insert,
            vim_enabled: false,
            mode: InputMode::Default,
            suggestions: Vec::new(),
            suggestion_index: None,
            history: Vec::new(),
            history_pos: None,
            history_draft: String::new(),
            paste_counter: 0,
            paste_contents: std::collections::HashMap::new(),
            yank_buf: String::new(),
            token_estimate: 0,
            vim_pending: VimPendingState::None,
            undo_stack: Vec::new(),
            visual_anchor: None,
            last_find: None,
            vim_registers: std::collections::HashMap::new(),
            vim_macro_recording: None,
            vim_macro_content: std::collections::HashMap::new(),
            vim_marks: std::collections::HashMap::new(),
            vim_dot_action: None,
            vim_insert_text_before: None,
            vim_command_buf: String::new(),
            vim_search_buf: String::new(),
            vim_search_last: None,
            vim_quit_requested: false,
            pending_images: Vec::new(),
            kill_ring: KillRing::new(),
        }
    }

    /// Add a clipboard image attachment to the pending list.
    pub fn add_image(&mut self, img: crate::image_paste::PastedImage) {
        self.pending_images.push(img);
    }

    /// Drain and return all pending image attachments (called at send time).
    pub fn clear_images(&mut self) -> Vec<crate::image_paste::PastedImage> {
        std::mem::take(&mut self.pending_images)
    }

    /// Insert a character at cursor position.
    pub fn insert_char(&mut self, c: char) {
        if self.mode == InputMode::Readonly {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.update_token_estimate();
    }

    /// Insert a newline (Shift+Enter).
    pub fn insert_newline(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        self.insert_char('\n');
    }

    /// Delete the character before cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 || self.mode == InputMode::Readonly {
            return;
        }
        let prev = self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.remove(prev);
        self.cursor = prev;
        self.update_token_estimate();
    }

    /// Delete the character at cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() || self.mode == InputMode::Readonly {
            return;
        }
        self.text.remove(self.cursor);
        self.update_token_estimate();
    }

    /// Move cursor left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor = prev;
        }
    }

    /// Move cursor right.
    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.text.len());
            self.cursor = next;
        }
    }

    /// Navigate history up (older).
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_pos {
            None => {
                self.history_draft = self.text.clone();
                self.history_pos = Some(self.history.len() - 1);
            }
            Some(0) => {}
            Some(n) => {
                self.history_pos = Some(n - 1);
            }
        }
        if let Some(pos) = self.history_pos {
            self.text = self.history[pos].clone();
            self.cursor = self.text.len();
            self.update_token_estimate();
        }
    }

    /// Navigate history down (newer).
    pub fn history_down(&mut self) {
        match self.history_pos {
            None => {}
            Some(n) if n + 1 >= self.history.len() => {
                self.history_pos = None;
                self.text = self.history_draft.clone();
                self.cursor = self.text.len();
                self.update_token_estimate();
            }
            Some(n) => {
                self.history_pos = Some(n + 1);
                self.text = self.history[n + 1].clone();
                self.cursor = self.text.len();
                self.update_token_estimate();
            }
        }
    }

    /// Handle a paste event.
    pub fn paste(&mut self, content: &str) {
        let (text, stored) = handle_paste(content, &mut self.paste_counter);
        if let Some(stored_content) = stored {
            self.paste_contents
                .insert(self.paste_counter, stored_content);
        }
        for c in text.chars() {
            self.text.insert(self.cursor, c);
            self.cursor += c.len_utf8();
        }
        self.update_token_estimate();
        self.kill_ring.mark_non_kill();
    }

    /// Ctrl+U: Cut from line start to cursor and save to kill ring.
    pub fn kill_line_backward(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        let line_start = self.text[..self.cursor]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);

        if self.cursor > line_start {
            let killed = self.text.drain(line_start..self.cursor).collect::<String>();
            self.kill_ring.push(killed);
            self.cursor = line_start;
            self.update_token_estimate();
        }
    }

    /// Ctrl+W: Cut previous word and save to kill ring.
    pub fn kill_word_backward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let mut idx = chars.len();
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 {
            return;
        }
        if is_word_char(chars[idx - 1]) {
            while idx > 0 && is_word_char(chars[idx - 1]) {
                idx -= 1;
            }
        } else {
            while idx > 0 && !is_word_char(chars[idx - 1]) && !chars[idx - 1].is_whitespace() {
                idx -= 1;
            }
        }
        let kill_start = char_idx_to_byte(before, idx);
        if kill_start < self.cursor {
            let killed = self.text.drain(kill_start..self.cursor).collect::<String>();
            self.kill_ring.push(killed);
            self.cursor = kill_start;
            self.update_token_estimate();
        }
    }

    /// Ctrl+Y: Paste from kill ring (most recent).
    pub fn yank(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        if let Some(text) = self.kill_ring.get_current() {
            for c in text.chars() {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+Y: Cycle through kill ring backward.
    pub fn yank_pop(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        self.kill_ring.cycle_backward();
    }

    /// Alt+Backspace: Delete word backward.
    pub fn delete_word_backward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let mut idx = chars.len();
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 {
            return;
        }
        if is_word_char(chars[idx - 1]) {
            while idx > 0 && is_word_char(chars[idx - 1]) {
                idx -= 1;
            }
        } else {
            while idx > 0 && !is_word_char(chars[idx - 1]) && !chars[idx - 1].is_whitespace() {
                idx -= 1;
            }
        }
        let delete_start = char_idx_to_byte(before, idx);
        if delete_start < self.cursor {
            self.text.drain(delete_start..self.cursor);
            self.cursor = delete_start;
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+Delete: Delete word forward.
    pub fn delete_word_forward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = 0;
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() {
            return;
        }
        if is_word_char(chars[idx]) {
            while idx < chars.len() && is_word_char(chars[idx]) {
                idx += 1;
            }
        } else {
            while idx < chars.len() && !is_word_char(chars[idx]) && !chars[idx].is_whitespace() {
                idx += 1;
            }
        }
        let delete_end = self.cursor + char_idx_to_byte(rest, idx);
        if delete_end > self.cursor {
            self.text.drain(self.cursor..delete_end);
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+B: Jump to previous word.
    pub fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let mut idx = chars.len();
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 {
            return;
        }
        if is_word_char(chars[idx - 1]) {
            while idx > 0 && is_word_char(chars[idx - 1]) {
                idx -= 1;
            }
        } else {
            while idx > 0 && !is_word_char(chars[idx - 1]) && !chars[idx - 1].is_whitespace() {
                idx -= 1;
            }
        }
        self.cursor = char_idx_to_byte(before, idx);
    }

    /// Alt+F: Jump to next word.
    pub fn move_word_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = 0;
        if idx < chars.len() {
            if is_word_char(chars[idx]) {
                while idx < chars.len() && is_word_char(chars[idx]) {
                    idx += 1;
                }
            } else if !chars[idx].is_whitespace() {
                while idx < chars.len() && !is_word_char(chars[idx]) && !chars[idx].is_whitespace()
                {
                    idx += 1;
                }
            }
        }
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        self.cursor += char_idx_to_byte(rest, idx);
    }

    /// Alt+D: Delete word after cursor.
    pub fn delete_word_at_cursor(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = 0;
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() {
            return;
        }
        if is_word_char(chars[idx]) {
            while idx < chars.len() && is_word_char(chars[idx]) {
                idx += 1;
            }
        } else {
            while idx < chars.len() && !is_word_char(chars[idx]) && !chars[idx].is_whitespace() {
                idx += 1;
            }
        }
        let delete_end = self.cursor + char_idx_to_byte(rest, idx);
        if delete_end > self.cursor {
            self.text.drain(self.cursor..delete_end);
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Push the current (text, cursor) to the undo stack.
    pub fn push_undo(&mut self) {
        self.undo_stack.push((self.text.clone(), self.cursor));
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    // ---- Named registers ----

    /// Store `text` in the named register `register`.
    pub fn yank_to_register(&mut self, register: char, text: &str) {
        self.vim_registers.insert(register, text.to_string());
    }

    /// Retrieve text from the named register `register`, if any.
    pub fn paste_from_register(&mut self, register: char) -> Option<String> {
        self.vim_registers.get(&register).cloned()
    }

    // ---- Marks ----

    /// Set mark `name` at the current cursor position.
    pub fn set_mark(&mut self, name: char) {
        self.vim_marks
            .insert(name, (self.text.clone(), self.cursor));
    }

    /// Move cursor to the position recorded for mark `name`, if the text still matches.
    pub fn jump_to_mark(&mut self, name: char) {
        if let Some((_saved_text, saved_cursor)) = self.vim_marks.get(&name).cloned() {
            // Clamp to current text length in case text changed.
            let target = saved_cursor.min(self.text.len());
            // Ensure we land on a char boundary.
            let mut pos = target;
            while pos > 0 && !self.text.is_char_boundary(pos) {
                pos -= 1;
            }
            self.cursor = pos;
        }
    }

    // ---- Macro recording ----

    /// Begin recording a macro into register `register`.
    /// If already recording, stops the current recording first.
    pub fn start_macro_recording(&mut self, register: char) {
        self.vim_macro_recording = Some(register);
        self.vim_macro_content.insert(register, Vec::new());
    }

    /// Stop recording the current macro. Returns the register name that was being recorded.
    pub fn stop_macro_recording(&mut self) -> Option<char> {
        self.vim_macro_recording.take()
    }

    /// Return the recorded key sequence for `register`, or an empty vec.
    pub fn replay_macro(&self, register: char) -> Vec<String> {
        self.vim_macro_content
            .get(&register)
            .cloned()
            .unwrap_or_default()
    }

    // ---- Vim command-line execution ----

    /// Execute a `:` command-line command.
    /// Recognised: `q`/`quit`, `wq`, `set` (no-op), `noh` (clear search highlight).
    pub fn execute_vim_cmdline(&mut self, cmd: &str) {
        match cmd {
            "q" | "quit" | "wq" | "x" => {
                // In prompt context we can only signal quit by clearing + a special flag.
                // We set a dedicated field that the app loop can inspect.
                self.vim_quit_requested = true;
            }
            "noh" | "nohlsearch" => {
                self.vim_search_last = None;
            }
            s if s.starts_with("set ") => {
                // `:set vim` → enable, `:set novim` → disable (runtime toggle)
                let arg = s["set ".len()..].trim();
                match arg {
                    "vim" => {
                        self.vim_enabled = true;
                    }
                    "novim" => {
                        self.vim_enabled = false;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // ---- In-prompt search ----

    /// Move cursor to the next occurrence of `pattern` after `cursor + skip`.
    /// `skip = 0` finds from current position; `skip = 1` finds the *next* one.
    pub fn vim_search_forward(&mut self, pattern: &str, skip: usize) {
        if pattern.is_empty() {
            return;
        }
        let start = if skip > 0 {
            // Start after the current character to avoid re-matching same position
            let next = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(b, _)| self.cursor + b)
                .unwrap_or(0);
            next
        } else {
            self.cursor
        };
        // Search from `start` forward, then wrap around
        let text_lc = self.text.to_lowercase();
        let pat_lc = pattern.to_lowercase();
        if let Some(pos) = text_lc[start..].find(&pat_lc) {
            self.cursor = start + pos;
            return;
        }
        // Wrap: search from beginning
        if let Some(pos) = text_lc.find(&pat_lc) {
            self.cursor = pos;
        }
    }

    /// Move cursor to the previous occurrence of `pattern` before current cursor.
    pub fn vim_search_backward(&mut self, pattern: &str) {
        if pattern.is_empty() {
            return;
        }
        let text_lc = self.text.to_lowercase();
        let pat_lc = pattern.to_lowercase();
        // Find all occurrences, pick the last one before cursor
        let before = &text_lc[..self.cursor];
        if let Some(pos) = before.rfind(&pat_lc) {
            self.cursor = pos;
            return;
        }
        // Wrap: find last occurrence in whole text
        if let Some(pos) = text_lc.rfind(&pat_lc) {
            self.cursor = pos;
        }
    }

    /// Clear the input and reset state.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.suggestions.clear();
        self.suggestion_index = None;
        self.history_pos = None;
        self.token_estimate = 0;
        self.vim_pending = VimPendingState::None;
        self.visual_anchor = None;
        self.vim_command_buf.clear();
        self.vim_search_buf.clear();
    }

    /// Take the current text, clearing the input.
    pub fn take(&mut self) -> String {
        let text = self.text.clone();
        self.clear();
        text
    }

    /// Returns true if the text (up to cursor) contains a word-boundary `@` token,
    /// meaning an `@file` reference is actively being typed.
    pub fn has_active_file_ref(&self) -> bool {
        let text = &self.text[..self.cursor];
        text.rfind('@').is_some_and(|at_idx| {
            at_idx == 0
                || text[..at_idx]
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_whitespace())
        })
    }

    /// Update typeahead suggestions for slash commands and file references in the current text.
    pub fn update_suggestions(
        &mut self,
        slash_commands: &[(&str, &str)],
        file_autocomplete_limit: usize,
        file_autocomplete_show_hidden: bool,
    ) {
        // Only look at text up to the cursor — text after the cursor belongs to a
        // different editing position and would confuse rfind('@') / rfind('/').
        let text_before_cursor = &self.text[..self.cursor];
        self.suggestions = compute_typeahead(
            text_before_cursor,
            slash_commands,
            file_autocomplete_limit,
            file_autocomplete_show_hidden,
        );

        if self.suggestions.is_empty() {
            self.suggestion_index = None;
        } else {
            let idx = self
                .suggestion_index
                .unwrap_or(0)
                .min(self.suggestions.len() - 1);
            self.suggestion_index = Some(idx);
        }
    }

    /// Select the next suggestion.
    pub fn suggestion_next(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        self.suggestion_index = Some(
            self.suggestion_index
                .map_or(0, |i| (i + 1) % self.suggestions.len()),
        );
    }

    /// Select the previous suggestion.
    pub fn suggestion_prev(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        self.suggestion_index = Some(self.suggestion_index.map_or(0, |i| {
            if i == 0 {
                self.suggestions.len() - 1
            } else {
                i - 1
            }
        }));
    }

    /// Accept the current suggestion during an Enter (submit) keypress.
    ///
    /// Unlike `accept_suggestion`, this is the state machine for the submit path:
    /// it both fills the text with the chosen suggestion AND signals to the caller
    /// whether the next step is to keep editing (e.g. trailing space after a
    /// file-ref) or to submit the message/slash-command.
    ///
    /// Returns `NoSuggestion` when there is no suggestion to accept — callers
    /// should then fall back to the normal submit path (queue the message).
    pub fn accept_suggestion_for_submit(&mut self) -> AcceptForSubmitOutcome {
        if self.suggestions.is_empty() || self.suggestion_index.is_none() {
            return AcceptForSubmitOutcome::NoSuggestion;
        }
        let source = self
            .suggestions
            .get(self.suggestion_index.unwrap_or(0))
            .map(|s| s.source.clone());
        self.accept_suggestion();
        match source {
            Some(TypeaheadSource::FileRef) => {
                // File refs end with a trailing space so the user can keep typing
                // the instruction that follows the @file reference.
                self.insert_char(' ');
                AcceptForSubmitOutcome::ExtendInput
            }
            Some(TypeaheadSource::SlashCommand) | None => {
                // Slash commands replace the whole input — Enter submits the command.
                AcceptForSubmitOutcome::Submit
            }
        }
    }

    /// Accept the current suggestion.
    pub fn accept_suggestion(&mut self) {
        if let Some(idx) = self.suggestion_index {
            if let Some(s) = self.suggestions.get(idx) {
                let new_cursor = match s.source {
                    TypeaheadSource::SlashCommand => {
                        // Replace entire text; discard anything after cursor too.
                        self.text = s.text.clone();
                        self.text.len()
                    }
                    TypeaheadSource::FileRef => {
                        // Replace from the last word-boundary @ up to the cursor.
                        // Preserve any text that was already after the cursor.
                        let tail = self.text[self.cursor..].to_string();
                        if let Some(at_idx) = self.text[..self.cursor].rfind('@') {
                            let at_word_boundary = at_idx == 0
                                || self.text[..at_idx]
                                    .chars()
                                    .last()
                                    .map(|c| c.is_whitespace())
                                    .unwrap_or(false);
                            if at_word_boundary {
                                let mut new_text = self.text[..at_idx].to_string();
                                new_text.push_str(&s.text);
                                let cursor = new_text.len();
                                new_text.push_str(&tail);
                                self.text = new_text;
                                cursor
                            } else {
                                let mut new_text = s.text.clone();
                                let cursor = new_text.len();
                                new_text.push_str(&tail);
                                self.text = new_text;
                                cursor
                            }
                        } else {
                            let mut new_text = s.text.clone();
                            let cursor = new_text.len();
                            new_text.push_str(&tail);
                            self.text = new_text;
                            cursor
                        }
                    }
                };
                self.cursor = new_cursor;
                self.suggestions.clear();
                self.suggestion_index = None;
                self.update_token_estimate();
            }
        }
    }

    /// Replace the full text buffer and move the cursor to the end.
    pub fn replace_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.history_pos = None;
        self.suggestion_index = None;
        self.update_token_estimate();
    }

    /// Map the current cursor (byte offset) to a (visual_row, visual_col) pair
    /// given the wrap width. `width` is the usable column count for text.
    pub fn cursor_visual_pos(&self, width: usize) -> (usize, usize) {
        if width == 0 {
            return (0, 0);
        }
        let mut byte = 0usize;
        let mut row = 0usize;
        for line in self.text.split('\n') {
            let line_end = byte + line.len();
            if self.cursor <= line_end {
                let intra_byte = self.cursor - byte;
                let intra_byte = intra_byte.min(line.len());
                // walk to char-boundary
                let mut b = intra_byte;
                while b > 0 && !line.is_char_boundary(b) {
                    b -= 1;
                }
                let display_col = UnicodeWidthStr::width(&line[..b]);
                let chunk_idx = if display_col == 0 {
                    0
                } else {
                    display_col / width
                };
                let chunk_col = display_col % width;
                return (row + chunk_idx, chunk_col);
            }
            let chunks = wrap_line(line, width).len().max(1);
            row += chunks;
            byte = line_end + 1; // newline
        }
        (row.saturating_sub(1), 0)
    }

    /// Move the cursor to the same visual column on the row above. Returns
    /// `true` if the cursor actually moved (i.e. there was a row above).
    pub fn move_visual_up(&mut self, width: usize) -> bool {
        if width == 0 {
            return false;
        }
        let (row, col) = self.cursor_visual_pos(width);
        if row == 0 {
            return false;
        }
        self.set_cursor_at_visual(row - 1, col, width);
        true
    }

    /// Move the cursor to the same visual column on the row below. Returns
    /// `true` if the cursor actually moved (i.e. there was a row below).
    pub fn move_visual_down(&mut self, width: usize) -> bool {
        if width == 0 {
            return false;
        }
        let (row, col) = self.cursor_visual_pos(width);
        let total_rows = self.visual_row_count(width);
        if row + 1 >= total_rows {
            return false;
        }
        self.set_cursor_at_visual(row + 1, col, width);
        true
    }

    fn visual_row_count(&self, width: usize) -> usize {
        if self.text.is_empty() || width == 0 {
            return 1;
        }
        let mut total = 0usize;
        for line in self.text.split('\n') {
            total += wrap_line(line, width).len().max(1);
        }
        total.max(1)
    }

    fn set_cursor_at_visual(&mut self, target_row: usize, target_col: usize, width: usize) {
        if width == 0 {
            return;
        }
        let mut byte = 0usize;
        let mut row = 0usize;
        for line in self.text.split('\n').collect::<Vec<_>>() {
            let chunks = wrap_line(line, width).len().max(1);
            if target_row < row + chunks {
                let intra_chunk = target_row - row;
                let chunk_char_start = intra_chunk * width;
                let line_chars: Vec<(usize, char)> = line.char_indices().collect();
                let chunk_chars_len = line_chars.len().saturating_sub(chunk_char_start).min(width);
                let col = target_col.min(chunk_chars_len);
                let target_char_idx = chunk_char_start + col;
                let intra_byte = line_chars
                    .get(target_char_idx)
                    .map(|(b, _)| *b)
                    .unwrap_or(line.len());
                self.cursor = byte + intra_byte;
                self.history_pos = None;
                return;
            }
            row += chunks;
            byte += line.len() + 1; // newline
        }
        self.cursor = self.text.len();
        self.history_pos = None;
    }

    /// Normalize cursor and metadata after external field updates.
    pub fn normalize(&mut self) {
        self.cursor = self.cursor.min(self.text.len());
        while self.cursor > 0 && !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        self.update_token_estimate();
    }

    /// Rough token estimate: ~4 chars per token.
    fn update_token_estimate(&mut self) {
        self.token_estimate = self.text.len().div_ceil(4);
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

impl Default for PromptInputState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Return the number of rows needed to render the input for the given text.
/// `text_width` is the usable column count for wrapped text (i.e. area.width
/// minus the prompt prefix and right margin). When 0 we degrade gracefully and
/// only count logical lines.
///
/// Issue #149 follow-up: previously this only counted `\n`-separated lines, so
/// a single long visually-wrapped line stayed at the minimum height. Now we
/// count the actual visual row count and grow the box up to ~10 text rows
/// (12 total including the underline + breathing room). Larger inputs scroll
/// inside the box (handled in `render_prompt_input`).
pub fn input_height(state: &PromptInputState, text_width: u16) -> u16 {
    let visual_lines = if state.text.is_empty() {
        1usize
    } else if text_width == 0 {
        state.text.lines().count().max(1)
    } else {
        let mut total = 0usize;
        let logical: Vec<&str> = state.text.split('\n').collect();
        for line in &logical {
            let chunks = wrap_line(line, text_width as usize).len().max(1);
            total += chunks;
        }
        total.max(1)
    };
    // top-line + text rows + breathing room + underline, capped so the prompt
    // never eats more than ~half the screen.
    const MAX_TEXT_ROWS: usize = 10;
    let text_rows = visual_lines.min(MAX_TEXT_ROWS) as u16;
    let base = (text_rows + 3).max(4);
    base + if state.pending_images.is_empty() {
        0
    } else {
        1
    }
}

/// Wrap a logical line into visual chunks of `width` terminal cells. Empty
/// input yields a single empty chunk so the caller can still place a cursor.
pub fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    if line.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in line.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if current_width > 0 && current_width + ch_width > width {
            out.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        out.push(current);
    }

    out
}

/// Render the prompt input widget in the same low-chrome style as Operant:
/// multi-line input rows (one per logical line in the text) plus an accent
/// underline. Suggestions are rendered by the footer, not as a boxed dropdown
/// here.
pub fn render_prompt_input(
    state: &PromptInputState,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
    mode: InputMode,
    accent_override: Color,
    cursor_blink_enabled: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // If images are pending, render a pill row above everything else and shrink area.
    let (area, image_row_y) = if !state.pending_images.is_empty() && area.height > 1 {
        let pill_y = area.y;
        let rest = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };
        (rest, Some(pill_y))
    } else {
        (area, None)
    };

    if let Some(pill_y) = image_row_y {
        let mut pills: Vec<Span<'static>> = Vec::new();
        for img in &state.pending_images {
            let label = if let Some((w, h)) = img.dimensions {
                format!(" \u{f03e} {} {}x{} ", img.label, w, h) // nerd-font image icon, fallback to plain text
            } else {
                format!(" \u{f03e} {} ", img.label)
            };
            pills.push(Span::styled(
                label,
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ));
            pills.push(Span::raw(" "));
        }
        if !pills.is_empty() {
            Paragraph::new(Line::from(pills)).render(
                Rect {
                    x: area.x,
                    y: pill_y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }

    let accent = match mode {
        InputMode::Readonly => ACCENT_PRIMARY, // locked while streaming — always pink
        _ => accent_override,                  // use mode-aware accent color
    };
    let prompt_prefix = format!("{PROMPT_POINTER} ");
    let prefix_width = UnicodeWidthStr::width(prompt_prefix.as_str()) as u16;
    // Reserve a 2-cell right margin so wrapped text doesn't kiss the right edge
    // of the box (issue #149: padding too tight).
    let right_pad: u16 = 2;
    let available_width = area
        .width
        .saturating_sub(prefix_width)
        .saturating_sub(right_pad) as usize;
    let cursor_visible = if cursor_blink_enabled {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        (ms / 530).is_multiple_of(2)
    } else {
        true
    };
    // Render cursor as an overlay so its blink state never shifts the
    // underlying text (issue #149: cursor blink shifted the prompt).
    let show_cursor = focused && cursor_visible;

    // Use the raw text — no inline cursor character — so layout is stable.
    let display_text: String = if state.text.is_empty() {
        if focused {
            String::new()
        } else if mode == InputMode::Default {
            "How can I help you?".to_string()
        } else {
            String::new()
        }
    } else {
        state.text.clone()
    };

    // Top separator line (matches bottom underline — visual "box" around the prompt).
    if area.height > 0 {
        Paragraph::new(Line::from(vec![Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(accent),
        )]))
        .render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    // Text rows start 1 row below the top separator.
    let text_start_y = area.y + 1;

    // Split into logical lines; guarantee at least one.
    let logical_lines: Vec<String> = {
        let collected: Vec<String> = display_text.lines().map(|l| l.to_string()).collect();
        if display_text.ends_with('\n') || collected.is_empty() {
            let mut v = collected;
            v.push(String::new());
            v
        } else {
            collected
        }
    };

    let text_style = if state.text.is_empty() && !focused {
        Style::default().fg(Color::DarkGray).bg(Color::Black)
    } else {
        Style::default().fg(Color::White).bg(Color::Black)
    };

    // Wrap each logical line into visual rows that fit `available_width`,
    // and remember the (logical_idx, intra_line_display_col) for each row
    // so we can later compute where the cursor lives.
    let mut visual_rows: Vec<(usize, usize, String)> = Vec::new();
    for (li, line_text) in logical_lines.iter().enumerate() {
        let chunks = wrap_line(line_text, available_width.max(1));
        let mut col_offset = 0usize;
        for chunk in chunks {
            let chunk_len = UnicodeWidthStr::width(chunk.as_str());
            visual_rows.push((li, col_offset, chunk));
            col_offset += chunk_len;
        }
    }

    // Compute cursor's visual (row, col) within `visual_rows`.
    // We map state.cursor (a byte offset into state.text) to
    // (logical_line, display column).
    let cursor_pos: Option<(usize, usize)> = if focused && !state.text.is_empty() {
        let mut byte_idx = 0usize;
        let mut found: Option<(usize, usize)> = None;
        'outer: for (li, line_text) in logical_lines.iter().enumerate() {
            let line_bytes = line_text.len();
            // The +1 accounts for the '\n' between logical lines (last line has no trailing \n).
            let line_end_byte = byte_idx + line_bytes;
            if state.cursor <= line_end_byte {
                let intra_byte = state.cursor - byte_idx;
                let display_col = UnicodeWidthStr::width(&line_text[..intra_byte.min(line_bytes)]);
                found = Some((li, display_col));
                break 'outer;
            }
            byte_idx = line_end_byte + 1; // newline
        }
        // Fallback: cursor at end of text.
        found.or_else(|| {
            let li = logical_lines.len().saturating_sub(1);
            let col = logical_lines
                .get(li)
                .map(|s| UnicodeWidthStr::width(s.as_str()))
                .unwrap_or(0);
            Some((li, col))
        })
    } else if focused && state.text.is_empty() {
        Some((0, 0))
    } else {
        None
    };

    let cursor_visual: Option<(usize, usize)> = cursor_pos.and_then(|(li, col)| {
        // Find the visual row whose logical_idx == li and contains `col`.
        let mut last_match: Option<(usize, usize)> = None;
        for (vi, (row_li, row_col_start, chunk)) in visual_rows.iter().enumerate() {
            if *row_li != li {
                continue;
            }
            let chunk_len = UnicodeWidthStr::width(chunk.as_str());
            let row_col_end = row_col_start + chunk_len;
            if col >= *row_col_start && col <= row_col_end {
                last_match = Some((vi, col - row_col_start));
            }
        }
        last_match
    });

    // Render each visual row (truncated to area height).
    // Ensure at least 1 text row so the input is always visible.
    // (iter-120 — user-reported bug: input text was not showing up,
    // appearing as a "black bar". Root cause: max_text_rows was 0 when
    // area.height <= 2, so no text rows were rendered.)
    let max_text_rows = ((area.height as usize).saturating_sub(2)).max(1);
    // Scroll so the cursor row is visible.
    let scroll_offset = match cursor_visual {
        Some((vi, _)) if visual_rows.len() > max_text_rows && vi >= max_text_rows => {
            vi + 1 - max_text_rows
        }
        _ => 0,
    };

    for (display_idx, (vi, (li, _col_start, chunk))) in visual_rows
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_text_rows)
        .map(|(idx, item)| (idx - scroll_offset, item))
        .enumerate()
        .map(|(d, (idx, item))| (d, (idx + scroll_offset, item)))
    {
        let _ = vi;
        let _ = li;
        let row_y = text_start_y + display_idx as u16;

        // Determine if this is the first visual row of the first logical line —
        // that's the only row that gets the prompt prefix; continuation rows
        // (whether from logical line breaks or wrapping) get whitespace.
        let is_first_row_of_first_logical = display_idx == 0 && scroll_offset == 0;

        let spans: Vec<Span<'static>> = if is_first_row_of_first_logical {
            vec![
                Span::styled(
                    prompt_prefix.clone(),
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(chunk.clone(), text_style),
            ]
        } else {
            vec![
                Span::raw(" ".repeat(prefix_width as usize)),
                Span::styled(chunk.clone(), text_style),
            ]
        };

        Paragraph::new(Line::from(spans)).render(
            Rect {
                x: area.x,
                y: row_y,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    // Overlay the cursor on top of the rendered text. Instead of replacing
    // the character with a solid block (which made the input look like a
    // "black bar" when the user typed), we use reverse video: swap the
    // foreground and background of the character at the cursor position.
    // This shows the character AND the cursor simultaneously.
    // (iter-121 — user-reported bug: input text was invisible because the
    // solid block cursor covered the typed character.)
    if show_cursor {
        if let Some((vi, col_in_row)) = cursor_visual {
            if vi >= scroll_offset {
                let display_idx = vi - scroll_offset;
                if display_idx < max_text_rows {
                    let row_y = text_start_y + display_idx as u16;
                    let x = area.x + prefix_width + col_in_row as u16;
                    if x < area.x + area.width && row_y < area.y + area.height {
                        let cell = &mut buf[(x, row_y)];
                        // Get the current character at this position
                        let current_symbol = cell.symbol().to_string();
                        if current_symbol.is_empty() || current_symbol == " " {
                            // Empty position — show a cursor bar
                            cell.set_symbol("▏");
                            cell.set_style(Style::default().fg(Color::White).bg(Color::Black));
                        } else {
                            // Non-empty position — reverse video
                            cell.set_style(
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            );
                        }
                    }
                }
            }
        }
    }

    // Vim command / search row (shown below text lines, before underline).
    let text_rows_rendered = visual_rows
        .len()
        .saturating_sub(scroll_offset)
        .min(max_text_rows);
    let cmd_line: Option<Line<'static>> = match state.vim_mode {
        VimMode::Command => {
            let buf_text = format!(":{}\u{2588}", state.vim_command_buf);
            Some(Line::from(vec![Span::styled(
                buf_text,
                Style::default().fg(Color::Cyan),
            )]))
        }
        VimMode::Search => {
            let buf_text = format!("/{}\u{2588}", state.vim_search_buf);
            Some(Line::from(vec![Span::styled(
                buf_text,
                Style::default().fg(Color::Yellow),
            )]))
        }
        _ => None,
    };

    let (cmdline_row, underline_row) = if let Some(ref _cl) = cmd_line {
        let cmd_y = text_start_y + text_rows_rendered as u16;
        let ul_y = cmd_y + 1;
        (Some(cmd_y), ul_y)
    } else {
        (None, text_start_y + text_rows_rendered as u16)
    };

    if let (Some(row), Some(cl)) = (cmdline_row, cmd_line) {
        if row < area.y + area.height {
            Paragraph::new(cl).render(
                Rect {
                    x: area.x,
                    y: row,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }

    if underline_row < area.y + area.height {
        Paragraph::new(Line::from(vec![Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(accent),
        )]))
        .render(
            Rect {
                x: area.x,
                y: underline_row,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    // Token estimate overlay on the first text row (top-right corner).
    // Format mirrors TS formatTokens: compact "1.3k" for ≥1000, raw number below that.
    if state.text.len() > 1000 && area.height > 1 {
        let n = state.token_estimate;
        let formatted = if n >= 1000 {
            let k = n as f64 / 1000.0;
            // One decimal place, suppress trailing ".0" (e.g. 2000 → "2k", 1300 → "1.3k")
            if (k * 10.0).round() % 10.0 == 0.0 {
                format!("~{}k", k as u64)
            } else {
                format!("~{:.1}k", k)
            }
        } else {
            format!("~{}", n)
        };
        let count_str = formatted;
        let x = area.x + area.width.saturating_sub(count_str.len() as u16);
        Paragraph::new(Line::from(vec![Span::styled(
            count_str,
            Style::default().fg(Color::DarkGray),
        )]))
        .render(
            Rect {
                x,
                y: text_start_y,
                width: area.width.saturating_sub(x.saturating_sub(area.x)),
                height: 1,
            },
            buf,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::typeahead::{compute_file_suggestions, compute_slash_suggestions};
    use super::vim::{
        VimOperator, apply_operator_range, lowercase_region, motion_B, motion_E, motion_G,
        motion_W, motion_b, motion_e, motion_find_char, motion_first_nonblank, motion_gg, motion_w,
        uppercase_region, vim_count, vim_g, vim_idle, vim_normal, vim_operator, vim_operator_count,
        vim_operator_g,
    };
    use super::*;

    // ---- VimMode --------------------------------------------------------

    #[test]
    fn vim_mode_labels() {
        assert_eq!(VimMode::Insert.label(), "INSERT");
        assert_eq!(VimMode::Normal.label(), "NORMAL");
        assert_eq!(VimMode::Visual.label(), "VISUAL");
    }

    // ---- PromptInputState -----------------------------------------------

    #[test]
    fn insert_char_updates_cursor() {
        let mut s = PromptInputState::new();
        s.insert_char('h');
        s.insert_char('i');
        assert_eq!(s.text, "hi");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn insert_newline_works() {
        let mut s = PromptInputState::new();
        s.insert_char('a');
        s.insert_newline();
        s.insert_char('b');
        assert_eq!(s.text, "a\nb");
    }

    #[test]
    fn backspace_removes_previous_char() {
        let mut s = PromptInputState::new();
        s.text = "hello".to_string();
        s.cursor = 5;
        s.backspace();
        assert_eq!(s.text, "hell");
        assert_eq!(s.cursor, 4);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut s = PromptInputState::new();
        s.text = "hi".to_string();
        s.cursor = 0;
        s.backspace();
        assert_eq!(s.text, "hi");
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut s = PromptInputState::new();
        s.text = "hello".to_string();
        s.cursor = 1;
        s.delete();
        assert_eq!(s.text, "hllo");
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn move_left_right() {
        let mut s = PromptInputState::new();
        s.text = "abc".to_string();
        s.cursor = 1;
        s.move_right();
        assert_eq!(s.cursor, 2);
        s.move_left();
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn cursor_visual_pos_counts_wide_characters() {
        let mut s = PromptInputState::new();
        s.text = "你a".to_string();
        s.cursor = "你".len();

        assert_eq!(s.cursor_visual_pos(10), (0, 2));
    }

    #[test]
    fn render_cursor_after_wide_character() {
        let mut s = PromptInputState::new();
        s.text = "你a".to_string();
        s.cursor = "你".len();

        let area = Rect {
            x: 0,
            y: 0,
            width: 12,
            height: 4,
        };
        let mut buf = Buffer::empty(area);
        render_prompt_input(
            &s,
            area,
            &mut buf,
            true,
            InputMode::Default,
            Color::Blue,
            false,
        );

        // iter-121: cursor now uses reverse video instead of solid block.
        // The character at the cursor position ('a') should still be visible
        // (not replaced by █). We check that the cell contains 'a' and has
        // reverse-video styling (black fg, white bg).
        let cell = &buf[(4, 1)];
        assert_eq!(cell.symbol(), "a");
        assert_eq!(cell.fg, Color::Black);
        assert_eq!(cell.bg, Color::White);
    }

    #[test]
    fn readonly_blocks_insert() {
        let mut s = PromptInputState::new();
        s.mode = InputMode::Readonly;
        s.insert_char('x');
        assert!(s.text.is_empty());
    }

    #[test]
    fn history_navigation_up_down() {
        let mut s = PromptInputState::new();
        s.history = vec!["first".to_string(), "second".to_string()];
        s.history_up();
        assert_eq!(s.text, "second");
        s.history_up();
        assert_eq!(s.text, "first");
        s.history_down();
        assert_eq!(s.text, "second");
        s.history_down();
        assert_eq!(s.text, "");
        assert!(s.history_pos.is_none());
    }

    #[test]
    fn history_draft_restored() {
        let mut s = PromptInputState::new();
        s.text = "draft text".to_string();
        s.cursor = 10;
        s.history = vec!["old entry".to_string()];
        s.history_up();
        assert_eq!(s.text, "old entry");
        s.history_down();
        assert_eq!(s.text, "draft text");
    }

    #[test]
    fn clear_resets_state() {
        let mut s = PromptInputState::new();
        s.text = "something".to_string();
        s.cursor = 5;
        s.token_estimate = 10;
        s.clear();
        assert!(s.text.is_empty());
        assert_eq!(s.cursor, 0);
        assert_eq!(s.token_estimate, 0);
    }

    #[test]
    fn take_returns_and_clears() {
        let mut s = PromptInputState::new();
        s.text = "hello".to_string();
        s.cursor = 5;
        let taken = s.take();
        assert_eq!(taken, "hello");
        assert!(s.text.is_empty());
    }

    #[test]
    fn is_empty_trims_whitespace() {
        let mut s = PromptInputState::new();
        s.text = "   \n  ".to_string();
        assert!(s.is_empty());
        s.text = "  x  ".to_string();
        assert!(!s.is_empty());
    }

    // ---- handle_paste ---------------------------------------------------

    #[test]
    fn paste_small_content_inline() {
        let mut counter = 0u32;
        let (result, stored) = handle_paste("short text", &mut counter);
        assert_eq!(result, "short text");
        assert!(stored.is_none());
        assert_eq!(counter, 0);
    }

    #[test]
    fn paste_large_content_placeholder() {
        let mut counter = 0u32;
        // >150 chars → triggers placeholder
        let big = "x".repeat(200);
        let (result, stored) = handle_paste(&big, &mut counter);
        assert!(
            result.starts_with("[Pasted ~"),
            "expected placeholder, got: {result}"
        );
        assert!(
            result.contains("#1"),
            "expected counter in placeholder, got: {result}"
        );
        assert!(stored.is_some());
        assert_eq!(counter, 1);
    }

    #[test]
    fn paste_large_multiline_placeholder() {
        let mut counter = 0u32;
        // ≥3 lines → triggers placeholder regardless of length
        let big = "line\n".repeat(300);
        let (result, stored) = handle_paste(&big, &mut counter);
        assert!(
            result.starts_with("[Pasted ~"),
            "expected placeholder, got: {result}"
        );
        assert!(
            result.contains("lines"),
            "expected line count in placeholder, got: {result}"
        );
        assert!(stored.is_some());
    }

    #[test]
    fn paste_three_lines_triggers_placeholder() {
        let mut counter = 0u32;
        // Exactly 3 lines (the threshold) should use a placeholder.
        let three_lines = "a\nb\nc";
        let (result, stored) = handle_paste(three_lines, &mut counter);
        assert!(
            result.starts_with("[Pasted ~"),
            "3-line paste should be placeholder, got: {result}"
        );
        assert!(stored.is_some());
    }

    #[test]
    fn paste_two_lines_inline() {
        let mut counter = 0u32;
        // 2 lines, ≤150 chars → inserted verbatim
        let two_lines = "hello\nworld";
        let (result, stored) = handle_paste(two_lines, &mut counter);
        assert_eq!(result, two_lines);
        assert!(stored.is_none());
    }

    #[test]
    fn paste_counter_increments() {
        let mut counter = 0u32;
        let big = "x".repeat(2000);
        handle_paste(&big, &mut counter);
        handle_paste(&big, &mut counter);
        assert_eq!(counter, 2);
    }

    // ---- compute_typeahead ---------------------------------------------

    // Helper constants for tests
    const TEST_FILE_AUTOCOMPLETE_LIMIT: usize = 15;
    const TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN: bool = false;

    #[test]
    fn typeahead_slash_prefix_matches() {
        let cmds = [
            ("help", "Show help"),
            ("history", "Show history"),
            ("compact", "Compact"),
        ];
        let suggestions = compute_slash_suggestions("/h", &cmds);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].text, "/help");
        assert_eq!(suggestions[1].text, "/history");
    }

    #[test]
    fn typeahead_full_match() {
        let cmds = [("compact", "Compact conversation")];
        let suggestions = compute_slash_suggestions("/compact", &cmds);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text, "/compact");
        assert_eq!(suggestions[0].description, "Compact conversation");
    }

    #[test]
    fn typeahead_case_insensitive() {
        let cmds = [("Help", "Show help")];
        let suggestions = compute_slash_suggestions("/H", &cmds);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text, "/Help");
    }

    // ---- suggestion navigation -----------------------------------------

    #[test]
    fn suggestion_next_cycles() {
        let mut s = PromptInputState::new();
        let cmds = [
            ("help", "Help"),
            ("history", "History"),
            ("compact", "Compact"),
        ];
        s.text = "/h".to_string();
        s.cursor = s.text.len();
        s.update_suggestions(&cmds, 15, false);
        assert_eq!(s.suggestions.len(), 2);
        assert_eq!(s.suggestion_index, Some(0));
        s.suggestion_next();
        assert_eq!(s.suggestion_index, Some(1));
        s.suggestion_next();
        assert_eq!(s.suggestion_index, Some(0)); // wraps
    }

    #[test]
    fn accept_suggestion_fills_text() {
        let mut s = PromptInputState::new();
        let cmds = [("help", "Show help")];
        s.text = "/he".to_string();
        s.cursor = s.text.len();
        s.update_suggestions(&cmds, 15, false);
        s.suggestion_next();
        s.accept_suggestion();
        assert_eq!(s.text, "/help");
        assert_eq!(s.cursor, 5);
        assert!(s.suggestions.is_empty());
    }

    // ---- token estimate -------------------------------------------------

    #[test]
    fn token_estimate_rough() {
        let mut s = PromptInputState::new();
        for _ in 0..40 {
            s.insert_char('a');
        }
        // 40 chars / 4 = 10 tokens
        assert_eq!(s.token_estimate, 10);
    }

    // ---- motion_w / motion_b -----------------------------------------------

    #[test]
    fn motion_w_basic() {
        assert_eq!(motion_w("hello world", 0), 6);
        assert_eq!(motion_w("hello world", 6), 11); // at start of 'world', moves to end
        assert_eq!(motion_w("  foo", 0), 2); // skip leading spaces
    }

    #[test]
    fn motion_b_basic() {
        assert_eq!(motion_b("hello world", 6), 0); // 'w' → start of 'hello'
        assert_eq!(motion_b("hello world", 0), 0); // already at start
    }

    #[test]
    fn motion_e_basic() {
        assert_eq!(motion_e("hello world", 0), 4); // cursor on 'h', end at 'o'
        assert_eq!(motion_e("hello world", 4), 10); // at 'o' (end), jump to 'd'
    }

    #[test]
    fn motion_W_B_basic() {
        // "foo.bar baz"  W from 0 → 8 ('b' of 'baz')
        assert_eq!(motion_W("foo.bar baz", 0), 8);
        assert_eq!(motion_B("foo.bar baz", 8), 0);
    }

    #[test]
    fn motion_E_basic() {
        assert_eq!(motion_E("foo.bar baz", 0), 6); // end of 'foo.bar' WORD
    }

    #[test]
    fn motion_first_nonblank_basic() {
        assert_eq!(motion_first_nonblank("  hello", 0), 2);
        assert_eq!(motion_first_nonblank("hello", 0), 0);
    }

    #[test]
    fn motion_G_basic() {
        assert_eq!(motion_G("foo\nbar"), 4);
        assert_eq!(motion_G("single line"), 0);
    }

    #[test]
    fn motion_gg_basic() {
        assert_eq!(motion_gg("foo\nbar\nbaz", 1), 0);
        assert_eq!(motion_gg("foo\nbar\nbaz", 2), 4);
        assert_eq!(motion_gg("foo\nbar\nbaz", 3), 8);
    }

    #[test]
    fn motion_find_char_f() {
        // f: cursor lands on 'o', count=1
        assert_eq!(
            motion_find_char("hello", 0, 'o', VimFindKind::F, 1),
            Some(4)
        );
        // f: not found
        assert_eq!(motion_find_char("hello", 0, 'z', VimFindKind::F, 1), None);
    }

    #[test]
    fn motion_find_char_t() {
        // t: cursor stops before 'o'
        assert_eq!(
            motion_find_char("hello", 0, 'o', VimFindKind::T, 1),
            Some(3)
        );
    }

    #[test]
    fn motion_find_char_bigF() {
        // F: search backward
        assert_eq!(
            motion_find_char("hello", 4, 'h', VimFindKind::BigF, 1),
            Some(0)
        );
    }

    // ---- apply_vim_key new commands ----------------------------------------

    #[test]
    fn vim_key_e_motion() {
        let mut mode = VimMode::Normal;
        let mut text = "hello world".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "e",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 4); // end of 'hello'
    }

    #[test]
    fn vim_key_W_motion() {
        let mut mode = VimMode::Normal;
        let mut text = "foo.bar baz".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "W",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 8); // 'baz'
    }

    #[test]
    fn vim_key_G_last_line() {
        let mut mode = VimMode::Normal;
        let mut text = "first\nsecond\nthird".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "G",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 13); // start of 'third'
    }

    #[test]
    fn vim_key_gg_first_line() {
        let mut mode = VimMode::Normal;
        let mut text = "first\nsecond".to_string();
        let mut cursor = 6usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        // 'g' sets pending G
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "g",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert!(matches!(pending, VimPendingState::G { .. }));
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "g",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 0);
    }

    #[test]
    fn vim_key_count_motion() {
        let mut mode = VimMode::Normal;
        let mut text = "a b c d e".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        // 3w — advance 3 words
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "3",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert!(matches!(pending, VimPendingState::Count { .. }));
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "w",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 6); // 3 words forward: a→b→c→d start = pos 6
    }

    #[test]
    fn vim_key_dw_delete_word() {
        let mut mode = VimMode::Normal;
        let mut text = "hello world".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "d",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert!(matches!(
            pending,
            VimPendingState::Operator {
                op: VimOperator::Delete,
                ..
            }
        ));
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "w",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(text, "world");
        assert_eq!(yank, "hello ");
    }

    #[test]
    fn vim_key_cw_change_word_enters_insert() {
        let mut mode = VimMode::Normal;
        let mut text = "hello world".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "c",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "w",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(mode, VimMode::Insert);
        assert_eq!(text, "world");
    }

    #[test]
    fn vim_key_dd_deletes_line() {
        let mut mode = VimMode::Normal;
        let mut text = "first\nsecond".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "d",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "d",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(text, "second");
        assert_eq!(yank, "first\n");
    }

    #[test]
    fn vim_key_r_replace_char() {
        let mut mode = VimMode::Normal;
        let mut text = "hello".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "r",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert!(matches!(pending, VimPendingState::Replace { .. }));
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "H",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(text, "Hello");
        assert_eq!(mode, VimMode::Normal); // stays in Normal after replace
    }

    #[test]
    fn vim_key_find_f() {
        let mut mode = VimMode::Normal;
        let mut text = "hello world".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "f",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "o",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 4); // first 'o' in 'hello'
        assert_eq!(last_find, Some((VimFindKind::F, 'o')));
    }

    #[test]
    fn vim_key_semicolon_repeat_find() {
        let mut mode = VimMode::Normal;
        let mut text = "a.b.c".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "f",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            ".",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 1);
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            ";",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(cursor, 3); // repeated find → next '.'
    }

    #[test]
    fn vim_key_X_delete_before_cursor() {
        let mut mode = VimMode::Normal;
        let mut text = "hello".to_string();
        let mut cursor = 4usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "X",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(text, "helo");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn vim_key_tilde_toggle_case() {
        let mut mode = VimMode::Normal;
        let mut text = "hello".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "~",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(text, "Hello");
    }

    #[test]
    fn vim_key_o_open_line_below() {
        let mut mode = VimMode::Normal;
        let mut text = "first\nthird".to_string();
        let mut cursor = 0usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "o",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(mode, VimMode::Insert);
        assert!(text.contains('\n'));
        assert_eq!(cursor, 6); // after first newline
    }

    #[test]
    fn vim_key_D_delete_to_eol() {
        let mut mode = VimMode::Normal;
        let mut text = "hello world".to_string();
        let mut cursor = 6usize;
        let mut yank = String::new();
        let mut pending = VimPendingState::None;
        let mut last_find = None;
        apply_vim_key(
            &mut mode,
            &mut text,
            &mut cursor,
            "D",
            &mut yank,
            &mut pending,
            &mut last_find,
        );
        assert_eq!(text, "hello ");
        assert_eq!(yank, "world");
    }

    // ---- PromptInputState undo ---------------------------------------------

    #[test]
    fn prompt_input_undo_restores_text() {
        let mut s = PromptInputState::new();
        s.vim_enabled = true;
        s.vim_mode = VimMode::Normal;
        s.text = "hello".to_string();
        s.cursor = 5;
        s.vim_command("x"); // deletes 'o' (but cursor at 5 = past end)
        // let's set cursor to 4 and delete
        s.cursor = 4;
        s.vim_command("x");
        assert_eq!(s.text, "hell");
        s.vim_command("u");
        assert_eq!(s.text, "hello");
    }

    #[test]
    fn prompt_input_visual_yank() {
        let mut s = PromptInputState::new();
        s.vim_enabled = true;
        s.vim_mode = VimMode::Normal;
        s.text = "hello world".to_string();
        s.cursor = 0;
        s.vim_command("v");
        assert_eq!(s.vim_mode, VimMode::Visual);
        // Move to end of word
        s.vim_command("e");
        s.vim_command("y"); // yank selection
        assert_eq!(s.yank_buf, "hello");
        assert_eq!(s.vim_mode, VimMode::Normal);
    }

    // ---- Named registers ------------------------------------------------

    #[test]
    fn register_yank_and_paste() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello world".to_string();
        s.cursor = 0;
        // `"ay` — yank line to register 'a'
        s.vim_command("\"");
        s.vim_command("a");
        s.vim_command("y");
        assert_eq!(
            s.vim_registers.get(&'a').map(|s| s.as_str()),
            Some("hello world")
        );
        // `"ap` — paste from register 'a' after cursor
        s.cursor = 0;
        s.vim_command("\"");
        s.vim_command("a");
        s.vim_command("p");
        assert!(s.text.contains("hello world"));
    }

    #[test]
    fn register_yank_method() {
        let mut s = PromptInputState::new();
        s.yank_to_register('b', "some text");
        assert_eq!(s.paste_from_register('b'), Some("some text".to_string()));
        assert_eq!(s.paste_from_register('z'), None);
    }

    #[test]
    fn register_delete_to_named() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello\nworld".to_string();
        s.cursor = 0;
        // `"ad` — delete line to register 'a'
        s.vim_command("\"");
        s.vim_command("a");
        s.vim_command("d");
        assert_eq!(
            s.vim_registers.get(&'a').map(|s| s.as_str()),
            Some("hello\n")
        );
        assert_eq!(s.text, "world");
    }

    // ---- Marks ----------------------------------------------------------

    #[test]
    fn mark_set_and_jump() {
        let mut s = PromptInputState::new();
        s.text = "hello world".to_string();
        s.cursor = 6; // at 'w'
        s.set_mark('a');
        s.cursor = 0;
        s.jump_to_mark('a');
        assert_eq!(s.cursor, 6);
    }

    #[test]
    fn mark_jump_nonexistent_is_noop() {
        let mut s = PromptInputState::new();
        s.text = "hello".to_string();
        s.cursor = 3;
        s.jump_to_mark('z'); // no mark 'z' set
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn mark_via_vim_command() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello world".to_string();
        s.cursor = 6;
        // `ma` — set mark 'a'
        s.vim_command("m");
        s.vim_command("a");
        assert!(s.vim_marks.contains_key(&'a'));
        // Move cursor and jump back with `'a`
        s.cursor = 0;
        s.vim_command("'");
        s.vim_command("a");
        assert_eq!(s.cursor, 6);
    }

    #[test]
    fn mark_clamped_when_text_shortened() {
        let mut s = PromptInputState::new();
        s.text = "hello world".to_string();
        s.cursor = 10;
        s.set_mark('x');
        // Shorten the text
        s.text = "hi".to_string();
        s.cursor = 0;
        s.jump_to_mark('x');
        // Should clamp to text length
        assert!(s.cursor <= s.text.len());
        assert!(s.text.is_char_boundary(s.cursor));
    }

    // ---- Macro recording ------------------------------------------------

    #[test]
    fn macro_record_and_replay() {
        let mut s = PromptInputState::new();
        // Start recording into register 'q'
        s.start_macro_recording('q');
        assert_eq!(s.vim_macro_recording, Some('q'));
        // Simulate accumulating keys
        s.vim_macro_content
            .get_mut(&'q')
            .unwrap()
            .push("w".to_string());
        s.vim_macro_content
            .get_mut(&'q')
            .unwrap()
            .push("e".to_string());
        // Stop recording
        let reg = s.stop_macro_recording();
        assert_eq!(reg, Some('q'));
        assert_eq!(s.vim_macro_recording, None);
        // Replay
        let keys = s.replay_macro('q');
        assert_eq!(keys, vec!["w".to_string(), "e".to_string()]);
    }

    #[test]
    fn macro_replay_empty_register() {
        let s = PromptInputState::new();
        let keys = s.replay_macro('z');
        assert!(keys.is_empty());
    }

    #[test]
    fn macro_via_vim_command() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "abc".to_string();
        s.cursor = 0;
        // `qq` — start recording into 'q'
        s.vim_command("q");
        assert!(matches!(s.vim_pending, VimPendingState::MacroRecord));
        s.vim_command("q"); // register name = 'q'
        assert_eq!(s.vim_macro_recording, Some('q'));
        // Record some keys: move right twice
        s.vim_command("l");
        s.vim_command("l");
        // Stop recording with `q`
        s.vim_command("q");
        assert_eq!(s.vim_macro_recording, None);
        // The recorded content should have 'l', 'l'
        let keys = s.replay_macro('q');
        assert_eq!(keys, vec!["l".to_string(), "l".to_string()]);
    }

    #[test]
    fn macro_replay_via_at() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "abcdef".to_string();
        s.cursor = 0;
        // Manually record a macro: move 2 chars right
        s.vim_macro_content
            .insert('q', vec!["l".to_string(), "l".to_string()]);
        // `@q` — replay macro 'q'
        s.vim_command("@");
        assert!(matches!(s.vim_pending, VimPendingState::MacroReplay));
        s.vim_command("q");
        // cursor should have moved right by 2
        assert_eq!(s.cursor, 2);
    }

    // ---- Dot-repeat -----------------------------------------------------

    #[test]
    fn dot_repeat_delete_char() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello".to_string();
        s.cursor = 0;
        // Delete char at cursor with `x`
        s.vim_command("x");
        assert_eq!(s.text, "ello");
        // Dot-repeat should delete again
        s.vim_command(".");
        assert_eq!(s.text, "llo");
    }

    #[test]
    fn dot_repeat_replace_char() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello".to_string();
        s.cursor = 0;
        // Replace 'h' with 'H' using `r`
        s.vim_command("r");
        s.vim_command("H");
        assert_eq!(s.text, "Hello");
        // Move and dot-repeat: should replace 'e' with 'H'
        s.vim_command("l");
        s.vim_command(".");
        assert_eq!(s.text, "HHllo");
    }

    #[test]
    fn dot_repeat_noop_when_no_action() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello".to_string();
        s.cursor = 0;
        // `.` with no prior modifying action should be a no-op
        s.vim_command(".");
        assert_eq!(s.text, "hello");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn dot_repeat_after_visual_delete() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello world".to_string();
        s.cursor = 0;
        // Enter visual, select 'hel', then delete
        s.vim_command("v");
        s.vim_command("l");
        s.vim_command("l");
        s.vim_command("d");
        assert_eq!(s.text, "lo world");
        // Dot-repeat should delete chars again
        s.vim_command(".");
        // The text should be shorter
        assert!(s.text.len() < "lo world".len());
    }

    // ---- Visual line mode (V) -------------------------------------------

    #[test]
    fn visual_line_mode_enter() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "line one\nline two".to_string();
        s.cursor = 0;
        s.vim_command("V");
        assert_eq!(s.vim_mode, VimMode::VisualLine);
        assert!(s.visual_anchor.is_some());
    }

    #[test]
    fn visual_line_yank() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "line one\nline two".to_string();
        s.cursor = 0;
        s.vim_command("V");
        s.vim_command("y");
        assert_eq!(s.vim_mode, VimMode::Normal);
        assert_eq!(s.yank_buf, "line one\n");
    }

    #[test]
    fn visual_line_delete() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "line one\nline two".to_string();
        s.cursor = 0;
        s.vim_command("V");
        s.vim_command("d");
        assert_eq!(s.vim_mode, VimMode::Normal);
        assert_eq!(s.text, "line two");
    }

    #[test]
    fn visual_line_escape_returns_normal() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello".to_string();
        s.vim_command("V");
        assert_eq!(s.vim_mode, VimMode::VisualLine);
        s.vim_command("Escape");
        assert_eq!(s.vim_mode, VimMode::Normal);
    }

    // ---- Command-line mode (:) ------------------------------------------

    #[test]
    fn command_line_mode_enter() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        assert_eq!(s.vim_mode, VimMode::Command);
        assert!(s.vim_command_buf.is_empty());
    }

    #[test]
    fn command_line_accumulates_chars() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        s.vim_command("q");
        assert_eq!(s.vim_command_buf, "q");
        s.vim_command("!");
        assert_eq!(s.vim_command_buf, "q!");
    }

    #[test]
    fn command_line_backspace_pops() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        s.vim_command("q");
        s.vim_command("w");
        s.vim_command("Backspace");
        assert_eq!(s.vim_command_buf, "q");
    }

    #[test]
    fn command_line_empty_backspace_cancels() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        s.vim_command("Backspace");
        assert_eq!(s.vim_mode, VimMode::Normal);
    }

    #[test]
    fn command_q_sets_quit_flag() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        s.vim_command("q");
        s.vim_command("Enter");
        assert!(s.vim_quit_requested);
        assert_eq!(s.vim_mode, VimMode::Normal);
    }

    #[test]
    fn command_noh_clears_search() {
        let mut s = PromptInputState::new();
        s.vim_search_last = Some("foo".to_string());
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        for c in "noh".chars() {
            s.vim_command(&c.to_string());
        }
        s.vim_command("Enter");
        assert!(s.vim_search_last.is_none());
    }

    #[test]
    fn command_escape_cancels() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command(":");
        s.vim_command("q");
        s.vim_command("Escape");
        assert_eq!(s.vim_mode, VimMode::Normal);
    }

    // ---- In-prompt search (/) -------------------------------------------

    #[test]
    fn search_mode_enter() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command("/");
        assert_eq!(s.vim_mode, VimMode::Search);
        assert!(s.vim_search_buf.is_empty());
    }

    #[test]
    fn search_finds_match_and_moves_cursor() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "hello world hello".to_string();
        s.cursor = 0;
        s.vim_command("/");
        for c in "world".chars() {
            s.vim_command(&c.to_string());
        }
        s.vim_command("Enter");
        assert_eq!(s.vim_mode, VimMode::Normal);
        assert_eq!(s.cursor, 6); // "world" starts at byte 6
        assert_eq!(s.vim_search_last.as_deref(), Some("world"));
    }

    #[test]
    fn search_n_finds_next() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "aa bb aa".to_string();
        s.cursor = 0;
        s.vim_command("/");
        s.vim_command("a");
        s.vim_command("a");
        s.vim_command("Enter");
        assert_eq!(s.cursor, 0); // first 'aa'
        s.vim_command("n");
        assert_eq!(s.cursor, 6); // second 'aa'
    }

    #[test]
    fn search_N_finds_prev() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.text = "aa bb aa".to_string();
        s.cursor = 7; // at second 'aa'
        s.vim_search_last = Some("aa".to_string());
        s.vim_command("N");
        assert_eq!(s.cursor, 0); // wraps to first 'aa'
    }

    #[test]
    fn search_escape_cancels() {
        let mut s = PromptInputState::new();
        s.vim_mode = VimMode::Normal;
        s.vim_command("/");
        s.vim_command("f");
        s.vim_command("Escape");
        assert_eq!(s.vim_mode, VimMode::Normal);
    }

    // ---- VimMode labels -------------------------------------------------

    #[test]
    fn vim_mode_new_labels() {
        assert_eq!(VimMode::VisualLine.label(), "VISUAL LINE");
        assert_eq!(VimMode::Command.label(), "COMMAND");
        assert_eq!(VimMode::Search.label(), "SEARCH");
    }

    // ---- File reference (@) autocomplete tests ----

    #[test]
    fn file_autocomplete_slash_commands_still_work() {
        let cmds = vec![("help", "Show help"), ("clear", "Clear messages")];
        let suggestions = compute_slash_suggestions("/he", &cmds);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].text, "/help");
    }

    #[test]
    fn file_autocomplete_at_requires_word_boundary() {
        // @ at word boundary: should suggest files (or be empty if cwd has no files)
        let suggestions_at_boundary = compute_file_suggestions(
            "@",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );
        let suggestions_at_boundary_with_space = compute_file_suggestions(
            "hello @",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );

        // @ not at word boundary: should never suggest files
        let suggestions_no_boundary = compute_file_suggestions(
            "test@",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );
        assert!(
            suggestions_no_boundary.is_empty(),
            "@ without word boundary should never suggest files"
        );

        // At least one of the boundary cases should work if cwd has files
        // but more importantly, the non-boundary case should always be empty
        for suggestion in suggestions_at_boundary
            .iter()
            .chain(suggestions_at_boundary_with_space.iter())
        {
            assert_eq!(suggestion.source, TypeaheadSource::FileRef);
        }
    }

    #[test]
    fn file_autocomplete_returns_fileref_source() {
        let suggestions = compute_file_suggestions(
            "@",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );

        for suggestion in suggestions {
            assert_eq!(suggestion.source, TypeaheadSource::FileRef);
        }
    }

    #[test]
    fn file_autocomplete_format_filenames() {
        let suggestions = compute_file_suggestions(
            "@",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );

        // All suggestions should start with @
        for suggestion in suggestions {
            assert!(suggestion.text.starts_with('@'));
        }
    }

    #[test]
    fn file_autocomplete_with_whitespace_prefix() {
        // @ after whitespace: should suggest files
        let suggestions = compute_file_suggestions(
            "hello @",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );

        // Check they all start with @ and are FileRef source
        for suggestion in suggestions {
            assert!(suggestion.text.starts_with('@'));
            assert_eq!(suggestion.source, TypeaheadSource::FileRef);
        }
    }

    #[test]
    fn file_autocomplete_detects_symlinks() {
        // This test verifies that symlinks/junction links are properly detected.
        // On systems with symlinks/junctions, suggestions will include descriptions
        // like "file link" or "directory link".
        let suggestions = compute_file_suggestions(
            "@",
            TEST_FILE_AUTOCOMPLETE_LIMIT,
            TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
        );

        // All suggestions should have a description (file, directory, file link, or directory link)
        for suggestion in suggestions {
            assert!(!suggestion.description.is_empty());
            assert!(
                suggestion.description.contains("file")
                    || suggestion.description.contains("directory"),
                "Unexpected description: {}",
                suggestion.description
            );
        }
    }

    // ---- has_active_file_ref tests ----------------------------------------

    #[test]
    fn has_active_file_ref_at_start() {
        let mut s = PromptInputState::new();
        s.text = "@src/".to_string();
        s.cursor = s.text.len();
        assert!(s.has_active_file_ref());
    }

    #[test]
    fn has_active_file_ref_after_space() {
        let mut s = PromptInputState::new();
        s.text = "hello @".to_string();
        s.cursor = s.text.len();
        assert!(s.has_active_file_ref());
    }

    #[test]
    fn has_active_file_ref_email_not_boundary() {
        let mut s = PromptInputState::new();
        s.text = "email@host".to_string();
        s.cursor = s.text.len();
        assert!(!s.has_active_file_ref());
    }

    #[test]
    fn has_active_file_ref_no_at() {
        let mut s = PromptInputState::new();
        s.text = "no at sign here".to_string();
        s.cursor = s.text.len();
        assert!(!s.has_active_file_ref());
    }

    // ---- accept_suggestion FileRef tests ------------------------------------

    #[test]
    fn accept_suggestion_file_ref_at_start() {
        let mut s = PromptInputState::new();
        s.text = "@src/ma".to_string();
        s.cursor = s.text.len();
        s.suggestions = vec![TypeaheadSuggestion {
            text: "@src/main.rs".to_string(),
            description: "file".to_string(),
            source: TypeaheadSource::FileRef,
        }];
        s.suggestion_index = Some(0);
        s.accept_suggestion();
        assert_eq!(s.text, "@src/main.rs");
        assert_eq!(s.cursor, "@src/main.rs".len());
        assert!(s.suggestions.is_empty());
    }

    #[test]
    fn accept_suggestion_file_ref_after_text_preserves_prefix() {
        let mut s = PromptInputState::new();
        s.text = "some text @src/ma".to_string();
        s.cursor = s.text.len();
        s.suggestions = vec![TypeaheadSuggestion {
            text: "@src/main.rs".to_string(),
            description: "file".to_string(),
            source: TypeaheadSource::FileRef,
        }];
        s.suggestion_index = Some(0);
        s.accept_suggestion();
        assert_eq!(s.text, "some text @src/main.rs");
        assert_eq!(s.cursor, "some text @src/main.rs".len());
    }

    #[test]
    fn accept_suggestion_file_ref_preserves_tail() {
        let mut s = PromptInputState::new();
        // Cursor is mid-string; tail after cursor is preserved
        let prefix = "@src/ma";
        let tail = " more text";
        s.text = format!("{}{}", prefix, tail);
        s.cursor = prefix.len();
        s.suggestions = vec![TypeaheadSuggestion {
            text: "@src/main.rs".to_string(),
            description: "file".to_string(),
            source: TypeaheadSource::FileRef,
        }];
        s.suggestion_index = Some(0);
        s.accept_suggestion();
        assert_eq!(s.text, "@src/main.rs more text");
        assert_eq!(s.cursor, "@src/main.rs".len());
    }
}
