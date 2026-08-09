#![allow(dead_code)] // Foundation modules for future multi-crate extraction — wired in Phase 2I
// keybindings.rs — Custom keybinding system for the TUI.
//
// Provides a flexible keybinding system that goes beyond simple vim_enabled bool,
// allowing users to define custom key mappings for any action.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

/// Action that can be bound to a key
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

#[cfg(test)]
mod tests;

/// Load keybindings from config file (TOML) - returns defaults if file doesn't exist
pub fn load_keybindings_from_config(
    _path: &std::path::Path,
) -> Result<KeyBindingRegistry, Box<dyn std::error::Error>> {
    Ok(KeyBindingRegistry::with_defaults())
}

/// Save keybindings to config file - placeholder for future implementation
pub fn save_keybindings_to_config(
    _registry: &KeyBindingRegistry,
    _path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
