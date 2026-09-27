// keybindings.rs — Custom keybinding system for the TUI.
//
// Provides a flexible keybinding system that goes beyond simple vim_enabled bool,
// allowing users to define custom key mappings for any action.
//
// This registry is the source of truth for *what the bindings are*: `/keys`
// and `/hotkeys` read their catalogue from here, and every accepted key
// event is matched against it to count usage. It is deliberately NOT yet the
// dispatcher — `app/key_handling.rs` still runs the if-chain. Migrating
// dispatch is a separate, riskier change; see `keybindings/report.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Action that can be bound to a key.
///
/// This is a *catalogue*: 65 of these 76 actions have a default binding, the
/// other 11 are declared-but-unbound (the table has no chord for them yet).
/// An enum-level allow is the right granularity for that — 11 scattered
/// per-variant attributes would be more noise than signal, and blanket-
/// allowing the module (as this file did before) hid genuinely dead code
/// elsewhere in the registry.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyAction {
    // Navigation
    MoveCursorLeft,
    MoveCursorRight,
    MoveCursorHome,
    MoveCursorEnd,
    MoveCursorWordLeft,
    MoveCursorWordRight,
    MoveCursorLineStart,
    MoveCursorLineEnd,

    // Editing
    DeleteCharLeft,
    DeleteCharRight,
    DeleteWordLeft,
    DeleteWordRight,
    DeleteLine,
    DeleteToLineStart,
    DeleteToLineEnd,
    InsertNewline,
    Undo,
    Redo,
    Paste,

    // History
    HistoryPrevious,
    HistoryNext,
    HistorySearch,

    // Vim mode
    VimEnterNormal,
    VimEnterInsert,
    VimEnterVisual,
    VimEnterVisualLine,
    VimEnterVisualBlock,
    VimEnterCommand,
    VimEnterSearch,
    VimRepeatLast,

    // Vim motions
    VimMotionUp,
    VimMotionDown,
    VimMotionLeft,
    VimMotionRight,
    VimMotionWordForward,
    VimMotionWordBackward,
    VimMotionWordEnd,
    VimMotionLineStart,
    VimMotionLineEnd,
    VimMotionPageUp,
    VimMotionPageDown,
    VimMotionFileStart,
    VimMotionFileEnd,

    // Vim editing
    VimDeleteChar,
    VimDeleteLine,
    VimDeleteWord,
    VimChangeWord,
    VimChangeLine,
    VimYank,
    VimPasteAfter,
    VimPasteBefore,
    VimIndent,
    VimDedent,

    // Vim find
    VimFindCharForward,
    VimFindCharBackward,
    VimFindCharForwardTo,
    VimFindCharBackwardTo,
    VimRepeatFind,
    VimRepeatFindReverse,

    // Vim marks/registers
    VimSetMark,
    VimGoToMark,
    VimYankRegister,
    VimPaste,

    // Completion
    CompletionNext,
    CompletionPrev,
    CompletionAccept,
    CompletionDismiss,

    // App-level
    Submit,
    SubmitAlt,
    Cancel,
    ToggleVimMode,
    TogglePlanMode,
    ShowHelp,
    ShowCommandPalette,
    ShowContextMenu,

    // Custom user actions (extensible)
    Custom(u32),
}

/// Context where a binding is active
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingContext {
    /// Global - always active
    Global,
    /// In the prompt input area
    Prompt,
    /// In the transcript/message pane
    Transcript,
    /// In a dialog/overlay
    Dialog,
    /// In vim normal mode
    VimNormal,
    /// In vim insert mode
    VimInsert,
    /// In vim visual mode
    VimVisual,
    /// In vim visual line mode
    VimVisualLine,
    /// In vim visual block mode
    VimVisualBlock,
    /// In vim command mode
    VimCommand,
    /// In vim search mode
    VimSearch,
    /// In completion menu
    Completion,
}

impl BindingContext {
    /// Every context, in the order `/keys` lists them.
    pub const ALL: [BindingContext; 12] = [
        BindingContext::Global,
        BindingContext::Prompt,
        BindingContext::Transcript,
        BindingContext::Dialog,
        BindingContext::Completion,
        BindingContext::VimNormal,
        BindingContext::VimInsert,
        BindingContext::VimVisual,
        BindingContext::VimVisualLine,
        BindingContext::VimVisualBlock,
        BindingContext::VimCommand,
        BindingContext::VimSearch,
    ];
}

/// A key binding: key combination -> action
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    /// The key code
    pub key: KeyCode,
    /// Modifier keys (Ctrl, Alt, Shift, etc.)
    pub modifiers: KeyModifiers,
    /// The action to perform
    pub action: KeyAction,
    /// Optional context where this binding applies
    pub context: Option<BindingContext>,
    /// Description for help display
    pub description: Option<String>,
}

/// Default binding entry for a given context
/// This is an internal struct used only for building defaults, not for serialization
struct DefaultBinding {
    key: KeyCode,
    modifiers: KeyModifiers,
    action: KeyAction,
    context: BindingContext,
    description: &'static str,
}

impl DefaultBinding {
    fn into_binding(self) -> KeyBinding {
        KeyBinding {
            key: self.key,
            modifiers: self.modifiers,
            action: self.action,
            context: Some(self.context),
            description: Some(self.description.to_string()),
        }
    }
}

/// Key binding registry - manages all bindings
#[derive(Debug, Clone, Default)]
pub struct KeyBindingRegistry {
    /// All bindings, indexed by context
    bindings: HashMap<BindingContext, Vec<KeyBinding>>,
    /// Global bindings (apply everywhere)
    global_bindings: Vec<KeyBinding>,
}

mod defaults;
mod registry;
// `pub` rather than plain `mod`: `KeyUsageReport`'s public fields are
// `Vec<BindingView>`, so the row type has to stay nameable even though no
// call site spells it out (rows are reached through field access).
pub mod report;

#[cfg(test)]
mod tests;

// The names other modules reference by `keybindings::` path. `BindingView` is
// deliberately absent — see the note on `pub mod report`.
pub use report::{KeyUsageReport, os_conflict_reason, usage_key};

/// The built-in binding table, built once and shared.
///
/// `/keys` and `/hotkeys` read it and `App::record_keybinding_usage` matches
/// every accepted key event against it. Rebuilding ~120 entries per keystroke
/// would be wasteful, so it is constructed once behind a `LazyLock`.
pub static DEFAULT_KEYBINDINGS: LazyLock<KeyBindingRegistry> =
    LazyLock::new(KeyBindingRegistry::with_defaults);

/// Load keybindings from config file (TOML) - returns defaults if file doesn't exist
#[allow(dead_code)] // Still a stub: config loading lands with user-defined bindings
pub fn load_keybindings_from_config(
    _path: &std::path::Path,
) -> Result<KeyBindingRegistry, Box<dyn std::error::Error>> {
    Ok(KeyBindingRegistry::with_defaults())
}

/// Save keybindings to config file - placeholder for future implementation
#[allow(dead_code)] // Paired stub with load_keybindings_from_config
pub fn save_keybindings_to_config(
    _registry: &KeyBindingRegistry,
    _path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
