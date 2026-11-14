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
mod ghost;
mod history;
mod kill_ring;
mod paste;
mod prediction;
mod state;
mod suggestions;
mod typeahead;
mod vim;
mod vim_command;
mod vim_ops;
mod visual;
mod word_nav;

#[cfg(test)]
mod tests;

pub use ghost::GhostText;
pub use kill_ring::KillRing;
pub use paste::{detect_pasted_path, expand_paste_placeholders};
pub use prediction::{EngineHandle, EngineReply, spawn as spawn_word_predictor};
pub use typeahead::{
    AcceptForSubmitOutcome, FileSuggestJob, FileSuggestResult, PendingFileQuery, TypeaheadSource,
    TypeaheadSuggestion, active_file_query, active_file_token, compute_typeahead,
    fuzzy_file_suggestions, parse_file_ref_token, register_invocable_skills,
    register_invocable_skills_in,
};
pub use vim::{DotRepeatAction, VimFindKind, VimMode, VimPendingState, apply_vim_key};
pub use visual::{VisualRow, wrap_input_rows};

// Only `tests.rs` needs this in scope (it resolves `Color` through
// `use super::*`); production code reads its colours from `theme_colors`.

// `Color` stays imported: `tests.rs` picks it up through `use super::*`.
// The accent is read through `theme_colors::accent()` at its use site in
// `render.rs` — the accessors are runtime `fn`s, so a `const` would pin every
// theme to the default theme's amber.

// The large-paste collapse/expand store lives in `paste.rs`: one owner for the
// placeholder label, the payload and the drop-on-delete rule (RT-559 / RT-562).

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
    /// (2026-10-25 history-nav fix) One-shot: the text change that just came
    /// from a history recall must not open the typeahead popup. OMP's editor
    /// auto-triggers autocomplete only on a TYPED trigger character in its
    /// character-insert path (`char === "/"` / `"@"`); programmatic
    /// `#setTextInternal` recalls never pass through it. Operant's single
    /// `refresh_prompt_input` choke point cannot tell them apart, so the
    /// recall path arms this flag and `update_suggestions` consumes it: the
    /// popup stays shut while browsing, and the first genuinely typed
    /// character after a recall re-arms normal behaviour.
    pub suppress_suggestions_once: bool,
    /// Stored pastes: `(placeholder, content)` in the order they were held.
    /// The label is derived from THIS set (not from a counter) so a deleted
    /// placeholder's number is reclaimed and its payload goes with it
    /// (RT-559 / RT-562).
    pub pending_pastes: Vec<(String, String)>,
    /// One-shot: the text change that just came from a paste-EXPAND must not
    /// touch the store, so expanding a paste cannot delete what it just
    /// inserted.
    pub suppress_paste_expand: bool,
    /// Set by the expand hand and drained by the app, which owns the one
    /// status line. Codex's `FooterFlash` rides the composer itself; here the
    /// notice is the app's, so the composer reports "I expanded something" and
    /// the app decides how it reads (RT-562: the collapse used to be silent).
    pub paste_expanded: Option<String>,
    /// The text of the history entry the operator last recalled AND has not
    /// edited since. `None` means "not browsing, or edited" — the Up/Down
    /// arms navigate the ladder only while the text still equals this
    /// (codex's `last_history_text` rule, RT-560).
    pub history_recall: Option<String>,
    /// The display column Up/Down remember while walking rows of unequal
    /// length (codex's `preferred_col`, RT-561). `None` means "use the caret's
    /// column"; the first vertical move saves it, a non-vertical edit drops it.
    pub preferred_col: Option<usize>,
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
    /// Shift-selection anchor for the composer text area (jcode textarea
    /// selection, 2026-10-09 visual audit): `Some` while a Shift+arrow
    /// selection is being extended; the cursor is the moving end. Byte
    /// offset into `text`. Cleared by any non-extending edit key.
    pub sel_anchor: Option<usize>,
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
    /// Contents parked by `stash_input` (Ctrl+S). One slot, not a stack: a
    /// second stash while one is held replaces it. `None` = nothing held.
    pub stash: Option<String>,
    /// Coalesced composer snapshots for the burst undo (Ctrl+Z). Separate
    /// from `undo_stack` above, which is vim's per-operation `:undo`.
    pub burst_undo: Vec<(String, usize)>,
    /// Composer state the open burst started from — the value `burst_undo`
    /// receives when a burst closes.
    pub burst_anchor: (String, usize),
    /// Edits recorded since the last snapshot, via `record_edit`.
    pub burst_edits: usize,
    /// Fuzzy file-suggestion request for the App to spawn (plan
    /// 2026-10-24 M3). Set by `update_suggestions` while an `@` token is
    /// live; taken by the App's `refresh_prompt_input`.
    pub pending_file_query: Option<typeahead::PendingFileQuery>,
    /// Ghost-text word completion shown at the caret (plan 2026-10-24 M6).
    pub ghost: Option<GhostText>,
}

/// Snapshots retained by the burst undo. 64 bursts is ~500 keystrokes of
/// history at [`UNDO_COALESCE`] edits each, at one `String` clone per burst.
///
/// ponytail: fixed ceiling — 64 bursts (≈500 edits) is what a real prompt
/// needs. If someone ever wants deeper history, make this a config field
/// rather than growing the constant; the stack is already bounded in one
/// place.
pub const UNDO_STACK_MAX: usize = 64;

/// Edits that coalesce into one undo step. A snapshot per keystroke would
/// make Ctrl+Z useless (one keypress per step), so the counter closes a
/// burst instead. Count-based rather than time-based: no clock, and a test
/// can drive an exact number of edits without sleeping.
pub const UNDO_COALESCE: usize = 8;

impl Default for PromptInputState {
    fn default() -> Self {
        Self::new()
    }
}
