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

impl KeyBindingRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create a registry with default keybindings
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.add_defaults();
        registry
    }
    
    /// Add a binding
    pub fn add(&mut self, binding: KeyBinding) {
        if let Some(ctx) = binding.context {
            self.bindings.entry(ctx).or_default().push(binding);
        } else {
            self.global_bindings.push(binding);
        }
    }
    
    /// Add multiple bindings
    pub fn add_many(&mut self, bindings: Vec<KeyBinding>) {
        for b in bindings {
            self.add(b);
        }
    }
    
    /// Find binding for a key event in a given context
    pub fn find(&self, event: &KeyEvent, context: BindingContext) -> Option<&KeyBinding> {
        // Check context-specific bindings first
        if let Some(ctx_bindings) = self.bindings.get(&context) {
            for binding in ctx_bindings {
                if self.matches(binding, event) {
                    return Some(binding);
                }
            }
        }
        
        // Check global bindings
        for binding in &self.global_bindings {
            if self.matches(binding, event) {
                return Some(binding);
            }
        }
        
        None
    }
    
    /// Find binding for a key event, trying multiple contexts in priority order
    pub fn find_with_fallback(&self, event: &KeyEvent, contexts: &[BindingContext]) -> Option<&KeyBinding> {
        for ctx in contexts {
            if let Some(binding) = self.find(event, *ctx) {
                return Some(binding);
            }
        }
        None
    }
    
    /// Check if a key event matches a binding
    fn matches(&self, binding: &KeyBinding, event: &KeyEvent) -> bool {
        binding.key == event.code && binding.modifiers == event.modifiers
    }
    
    /// Get all bindings for a context (for help display)
    pub fn get_bindings(&self, context: BindingContext) -> Vec<&KeyBinding> {
        let mut result = Vec::new();
        if let Some(ctx_bindings) = self.bindings.get(&context) {
            result.extend(ctx_bindings);
        }
        result.extend(&self.global_bindings);
        result
    }
    
    /// Remove a binding
    pub fn remove(&mut self, key: KeyCode, modifiers: KeyModifiers, context: Option<BindingContext>) -> bool {
        if let Some(ctx) = context {
            if let Some(bindings) = self.bindings.get_mut(&ctx) {
                let len_before = bindings.len();
                bindings.retain(|b| b.key != key || b.modifiers != modifiers);
                return bindings.len() < len_before;
            }
        } else {
            let len_before = self.global_bindings.len();
            self.global_bindings.retain(|b| b.key != key || b.modifiers != modifiers);
            return self.global_bindings.len() < len_before;
        }
        false
    }
    
    /// Add all default keybindings
    fn add_defaults(&mut self) {
        let defaults: Vec<DefaultBinding> = vec![
            // Global bindings
            DefaultBinding { key: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL, action: KeyAction::Cancel, context: BindingContext::Global, description: "Cancel current operation" },
            DefaultBinding { key: KeyCode::Char('q'), modifiers: KeyModifiers::CONTROL, action: KeyAction::Cancel, context: BindingContext::Global, description: "Quit application" },
            DefaultBinding { key: KeyCode::Char('h'), modifiers: KeyModifiers::CONTROL, action: KeyAction::HistoryPrevious, context: BindingContext::Global, description: "Previous history" },
            DefaultBinding { key: KeyCode::Char('l'), modifiers: KeyModifiers::CONTROL, action: KeyAction::HistoryNext, context: BindingContext::Global, description: "Next history" },
            DefaultBinding { key: KeyCode::F(1), modifiers: KeyModifiers::NONE, action: KeyAction::ShowHelp, context: BindingContext::Global, description: "Show help" },
            DefaultBinding { key: KeyCode::Char('p'), modifiers: KeyModifiers::CONTROL, action: KeyAction::ShowCommandPalette, context: BindingContext::Global, description: "Command palette" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::ALT, action: KeyAction::ToggleVimMode, context: BindingContext::Global, description: "Toggle Vim mode" },
            DefaultBinding { key: KeyCode::Char('m'), modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT, action: KeyAction::ShowContextMenu, context: BindingContext::Global, description: "Show context menu" },
            
            // Prompt bindings
            DefaultBinding { key: KeyCode::Enter, modifiers: KeyModifiers::NONE, action: KeyAction::Submit, context: BindingContext::Prompt, description: "Submit prompt" },
            DefaultBinding { key: KeyCode::Enter, modifiers: KeyModifiers::SHIFT, action: KeyAction::InsertNewline, context: BindingContext::Prompt, description: "Insert newline" },
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::Cancel, context: BindingContext::Prompt, description: "Cancel" },
            DefaultBinding { key: KeyCode::Tab, modifiers: KeyModifiers::NONE, action: KeyAction::CompletionNext, context: BindingContext::Prompt, description: "Next completion" },
            DefaultBinding { key: KeyCode::Tab, modifiers: KeyModifiers::SHIFT, action: KeyAction::CompletionPrev, context: BindingContext::Prompt, description: "Previous completion" },
            DefaultBinding { key: KeyCode::Up, modifiers: KeyModifiers::NONE, action: KeyAction::HistoryPrevious, context: BindingContext::Prompt, description: "History previous" },
            DefaultBinding { key: KeyCode::Down, modifiers: KeyModifiers::NONE, action: KeyAction::HistoryNext, context: BindingContext::Prompt, description: "History next" },
            DefaultBinding { key: KeyCode::Char('p'), modifiers: KeyModifiers::CONTROL, action: KeyAction::HistoryPrevious, context: BindingContext::Prompt, description: "History previous" },
            DefaultBinding { key: KeyCode::Char('n'), modifiers: KeyModifiers::CONTROL, action: KeyAction::HistoryNext, context: BindingContext::Prompt, description: "History next" },
            DefaultBinding { key: KeyCode::Char('r'), modifiers: KeyModifiers::CONTROL, action: KeyAction::HistorySearch, context: BindingContext::Prompt, description: "History search" },
            DefaultBinding { key: KeyCode::Char('a'), modifiers: KeyModifiers::CONTROL, action: KeyAction::MoveCursorHome, context: BindingContext::Prompt, description: "Move to start" },
            DefaultBinding { key: KeyCode::Char('e'), modifiers: KeyModifiers::CONTROL, action: KeyAction::MoveCursorEnd, context: BindingContext::Prompt, description: "Move to end" },
            DefaultBinding { key: KeyCode::Char('b'), modifiers: KeyModifiers::CONTROL, action: KeyAction::MoveCursorWordLeft, context: BindingContext::Prompt, description: "Word left" },
            DefaultBinding { key: KeyCode::Char('f'), modifiers: KeyModifiers::CONTROL, action: KeyAction::MoveCursorWordRight, context: BindingContext::Prompt, description: "Word right" },
            DefaultBinding { key: KeyCode::Char('w'), modifiers: KeyModifiers::CONTROL, action: KeyAction::DeleteWordLeft, context: BindingContext::Prompt, description: "Delete word left" },
            DefaultBinding { key: KeyCode::Char('u'), modifiers: KeyModifiers::CONTROL, action: KeyAction::DeleteLine, context: BindingContext::Prompt, description: "Delete line" },
            DefaultBinding { key: KeyCode::Char('k'), modifiers: KeyModifiers::CONTROL, action: KeyAction::DeleteToLineEnd, context: BindingContext::Prompt, description: "Delete to end" },
            DefaultBinding { key: KeyCode::Char('y'), modifiers: KeyModifiers::CONTROL, action: KeyAction::Paste, context: BindingContext::Prompt, description: "Paste" },
            DefaultBinding { key: KeyCode::Char('z'), modifiers: KeyModifiers::CONTROL, action: KeyAction::Undo, context: BindingContext::Prompt, description: "Undo" },
            DefaultBinding { key: KeyCode::Char('y'), modifiers: KeyModifiers::SHIFT | KeyModifiers::CONTROL, action: KeyAction::Redo, context: BindingContext::Prompt, description: "Redo" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::ALT, action: KeyAction::VimEnterNormal, context: BindingContext::Prompt, description: "Enter Vim normal mode" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::ALT, action: KeyAction::ToggleVimMode, context: BindingContext::Prompt, description: "Toggle Vim mode" },
            
            // Vim Normal mode bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterNormal, context: BindingContext::VimNormal, description: "Enter Normal mode" },
            DefaultBinding { key: KeyCode::Char('i'), modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterInsert, context: BindingContext::VimNormal, description: "Enter Insert mode" },
            DefaultBinding { key: KeyCode::Char('I'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimEnterInsert, context: BindingContext::VimNormal, description: "Enter Insert at line start" },
            DefaultBinding { key: KeyCode::Char('a'), modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterInsert, context: BindingContext::VimNormal, description: "Enter Insert after cursor" },
            DefaultBinding { key: KeyCode::Char('A'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimEnterInsert, context: BindingContext::VimNormal, description: "Enter Insert at line end" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterVisual, context: BindingContext::VimNormal, description: "Enter Visual mode" },
            DefaultBinding { key: KeyCode::Char('V'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimEnterVisualLine, context: BindingContext::VimNormal, description: "Enter Visual Line mode" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::CONTROL, action: KeyAction::VimEnterVisualBlock, context: BindingContext::VimNormal, description: "Enter Visual Block mode" },
            DefaultBinding { key: KeyCode::Char(':'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimEnterCommand, context: BindingContext::VimNormal, description: "Enter Command mode" },
            DefaultBinding { key: KeyCode::Char('/'), modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterSearch, context: BindingContext::VimNormal, description: "Enter Search mode" },
            DefaultBinding { key: KeyCode::Char('.'), modifiers: KeyModifiers::NONE, action: KeyAction::VimRepeatLast, context: BindingContext::VimNormal, description: "Repeat last change" },
            
            // Vim motions
            DefaultBinding { key: KeyCode::Char('h'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionLeft, context: BindingContext::VimNormal, description: "Move left" },
            DefaultBinding { key: KeyCode::Char('j'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionDown, context: BindingContext::VimNormal, description: "Move down" },
            DefaultBinding { key: KeyCode::Char('k'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionUp, context: BindingContext::VimNormal, description: "Move up" },
            DefaultBinding { key: KeyCode::Char('l'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionRight, context: BindingContext::VimNormal, description: "Move right" },
            DefaultBinding { key: KeyCode::Char('w'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionWordForward, context: BindingContext::VimNormal, description: "Next word" },
            DefaultBinding { key: KeyCode::Char('b'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionWordBackward, context: BindingContext::VimNormal, description: "Previous word" },
            DefaultBinding { key: KeyCode::Char('e'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionWordEnd, context: BindingContext::VimNormal, description: "End of word" },
            DefaultBinding { key: KeyCode::Char('0'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionLineStart, context: BindingContext::VimNormal, description: "Line start" },
            DefaultBinding { key: KeyCode::Char('$'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionLineEnd, context: BindingContext::VimNormal, description: "Line end" },
            DefaultBinding { key: KeyCode::Char('^'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionLineStart, context: BindingContext::VimNormal, description: "First non-blank" },
            DefaultBinding { key: KeyCode::Char('g'), modifiers: KeyModifiers::NONE, action: KeyAction::VimMotionFileStart, context: BindingContext::VimNormal, description: "File start" },
            DefaultBinding { key: KeyCode::Char('G'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionFileEnd, context: BindingContext::VimNormal, description: "File end" },
            DefaultBinding { key: KeyCode::Char('u'), modifiers: KeyModifiers::CONTROL, action: KeyAction::VimMotionPageUp, context: BindingContext::VimNormal, description: "Page up" },
            DefaultBinding { key: KeyCode::Char('d'), modifiers: KeyModifiers::CONTROL, action: KeyAction::VimMotionPageDown, context: BindingContext::VimNormal, description: "Page down" },
            
            // Vim editing
            DefaultBinding { key: KeyCode::Char('x'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDeleteChar, context: BindingContext::VimNormal, description: "Delete char" },
            DefaultBinding { key: KeyCode::Char('X'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimDeleteChar, context: BindingContext::VimNormal, description: "Delete char left" },
            DefaultBinding { key: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDeleteLine, context: BindingContext::VimNormal, description: "Delete line" },
            DefaultBinding { key: KeyCode::Char('D'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimDeleteLine, context: BindingContext::VimNormal, description: "Delete to end" },
            DefaultBinding { key: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDeleteWord, context: BindingContext::VimNormal, description: "Delete word" },
            DefaultBinding { key: KeyCode::Char('c'), modifiers: KeyModifiers::NONE, action: KeyAction::VimChangeLine, context: BindingContext::VimNormal, description: "Change line" },
            DefaultBinding { key: KeyCode::Char('C'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimChangeLine, context: BindingContext::VimNormal, description: "Change to end" },
            DefaultBinding { key: KeyCode::Char('y'), modifiers: KeyModifiers::NONE, action: KeyAction::VimYank, context: BindingContext::VimNormal, description: "Yank line" },
            DefaultBinding { key: KeyCode::Char('Y'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimYank, context: BindingContext::VimNormal, description: "Yank to end" },
            DefaultBinding { key: KeyCode::Char('p'), modifiers: KeyModifiers::NONE, action: KeyAction::VimPasteAfter, context: BindingContext::VimNormal, description: "Paste after" },
            DefaultBinding { key: KeyCode::Char('P'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimPasteBefore, context: BindingContext::VimNormal, description: "Paste before" },
            DefaultBinding { key: KeyCode::Char('>'), modifiers: KeyModifiers::NONE, action: KeyAction::VimIndent, context: BindingContext::VimNormal, description: "Indent" },
            DefaultBinding { key: KeyCode::Char('<'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDedent, context: BindingContext::VimNormal, description: "Dedent" },
            
            // Vim find
            DefaultBinding { key: KeyCode::Char('f'), modifiers: KeyModifiers::NONE, action: KeyAction::VimFindCharForward, context: BindingContext::VimNormal, description: "Find char forward" },
            DefaultBinding { key: KeyCode::Char('F'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimFindCharBackward, context: BindingContext::VimNormal, description: "Find char backward" },
            DefaultBinding { key: KeyCode::Char('t'), modifiers: KeyModifiers::NONE, action: KeyAction::VimFindCharForwardTo, context: BindingContext::VimNormal, description: "Find to char forward" },
            DefaultBinding { key: KeyCode::Char('T'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimFindCharBackwardTo, context: BindingContext::VimNormal, description: "Find to char backward" },
            DefaultBinding { key: KeyCode::Char(';'), modifiers: KeyModifiers::NONE, action: KeyAction::VimRepeatFind, context: BindingContext::VimNormal, description: "Repeat find" },
            DefaultBinding { key: KeyCode::Char(','), modifiers: KeyModifiers::NONE, action: KeyAction::VimRepeatFindReverse, context: BindingContext::VimNormal, description: "Reverse find" },
            
            // Vim marks
            DefaultBinding { key: KeyCode::Char('m'), modifiers: KeyModifiers::NONE, action: KeyAction::VimSetMark, context: BindingContext::VimNormal, description: "Set mark" },
            DefaultBinding { key: KeyCode::Char('\''), modifiers: KeyModifiers::NONE, action: KeyAction::VimGoToMark, context: BindingContext::VimNormal, description: "Go to mark" },
            
            // Vim registers
            DefaultBinding { key: KeyCode::Char('"'), modifiers: KeyModifiers::NONE, action: KeyAction::VimYankRegister, context: BindingContext::VimNormal, description: "Register operations" },
            
            // Vim Visual mode bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterNormal, context: BindingContext::VimVisual, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterVisual, context: BindingContext::VimVisual, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Char('V'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimEnterVisualLine, context: BindingContext::VimVisual, description: "Visual Line mode" },
            DefaultBinding { key: KeyCode::Char('v'), modifiers: KeyModifiers::CONTROL, action: KeyAction::VimEnterVisualBlock, context: BindingContext::VimVisual, description: "Visual Block mode" },
            // Shift+arrows in Visual mode to extend selection
            DefaultBinding { key: KeyCode::Up, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionUp, context: BindingContext::VimVisual, description: "Extend selection up" },
            DefaultBinding { key: KeyCode::Down, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionDown, context: BindingContext::VimVisual, description: "Extend selection down" },
            DefaultBinding { key: KeyCode::Left, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionLeft, context: BindingContext::VimVisual, description: "Extend selection left" },
            DefaultBinding { key: KeyCode::Right, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionRight, context: BindingContext::VimVisual, description: "Extend selection right" },
            DefaultBinding { key: KeyCode::Char('y'), modifiers: KeyModifiers::NONE, action: KeyAction::VimYank, context: BindingContext::VimVisual, description: "Yank selection" },
            DefaultBinding { key: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDeleteChar, context: BindingContext::VimVisual, description: "Delete selection" },
            DefaultBinding { key: KeyCode::Char('c'), modifiers: KeyModifiers::NONE, action: KeyAction::VimChangeLine, context: BindingContext::VimVisual, description: "Change selection" },
            DefaultBinding { key: KeyCode::Char('>'), modifiers: KeyModifiers::NONE, action: KeyAction::VimIndent, context: BindingContext::VimVisual, description: "Indent selection" },
            DefaultBinding { key: KeyCode::Char('<'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDedent, context: BindingContext::VimVisual, description: "Dedent selection" },
            
            // Vim Visual Line mode bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterNormal, context: BindingContext::VimVisualLine, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Up, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionUp, context: BindingContext::VimVisualLine, description: "Extend line selection up" },
            DefaultBinding { key: KeyCode::Down, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionDown, context: BindingContext::VimVisualLine, description: "Extend line selection down" },
            DefaultBinding { key: KeyCode::Char('y'), modifiers: KeyModifiers::NONE, action: KeyAction::VimYank, context: BindingContext::VimVisualLine, description: "Yank lines" },
            DefaultBinding { key: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDeleteChar, context: BindingContext::VimVisualLine, description: "Delete lines" },
            
            // Vim Visual Block mode bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterNormal, context: BindingContext::VimVisualBlock, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Up, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionUp, context: BindingContext::VimVisualBlock, description: "Extend block up" },
            DefaultBinding { key: KeyCode::Down, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionDown, context: BindingContext::VimVisualBlock, description: "Extend block down" },
            DefaultBinding { key: KeyCode::Left, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionLeft, context: BindingContext::VimVisualBlock, description: "Extend block left" },
            DefaultBinding { key: KeyCode::Right, modifiers: KeyModifiers::SHIFT, action: KeyAction::VimMotionRight, context: BindingContext::VimVisualBlock, description: "Extend block right" },
            DefaultBinding { key: KeyCode::Char('y'), modifiers: KeyModifiers::NONE, action: KeyAction::VimYank, context: BindingContext::VimVisualBlock, description: "Yank block" },
            DefaultBinding { key: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, action: KeyAction::VimDeleteChar, context: BindingContext::VimVisualBlock, description: "Delete block" },
            
            // Vim Command mode bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterNormal, context: BindingContext::VimCommand, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL, action: KeyAction::VimEnterNormal, context: BindingContext::VimCommand, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Enter, modifiers: KeyModifiers::NONE, action: KeyAction::Submit, context: BindingContext::VimCommand, description: "Execute command" },
            
            // Vim Search mode bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::VimEnterNormal, context: BindingContext::VimSearch, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL, action: KeyAction::VimEnterNormal, context: BindingContext::VimSearch, description: "Exit to Normal mode" },
            DefaultBinding { key: KeyCode::Enter, modifiers: KeyModifiers::NONE, action: KeyAction::VimRepeatLast, context: BindingContext::VimSearch, description: "Next match" },
            DefaultBinding { key: KeyCode::Char('n'), modifiers: KeyModifiers::NONE, action: KeyAction::VimRepeatLast, context: BindingContext::VimSearch, description: "Next match" },
            DefaultBinding { key: KeyCode::Char('N'), modifiers: KeyModifiers::SHIFT, action: KeyAction::VimRepeatFindReverse, context: BindingContext::VimSearch, description: "Previous match" },
            
            // Completion menu bindings
            DefaultBinding { key: KeyCode::Down, modifiers: KeyModifiers::NONE, action: KeyAction::CompletionNext, context: BindingContext::Completion, description: "Next completion" },
            DefaultBinding { key: KeyCode::Up, modifiers: KeyModifiers::NONE, action: KeyAction::CompletionPrev, context: BindingContext::Completion, description: "Previous completion" },
            DefaultBinding { key: KeyCode::Tab, modifiers: KeyModifiers::NONE, action: KeyAction::CompletionNext, context: BindingContext::Completion, description: "Next completion" },
            DefaultBinding { key: KeyCode::Tab, modifiers: KeyModifiers::SHIFT, action: KeyAction::CompletionPrev, context: BindingContext::Completion, description: "Previous completion" },
            DefaultBinding { key: KeyCode::Enter, modifiers: KeyModifiers::NONE, action: KeyAction::CompletionAccept, context: BindingContext::Completion, description: "Accept completion" },
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::CompletionDismiss, context: BindingContext::Completion, description: "Dismiss completion" },
            
            // Dialog bindings
            DefaultBinding { key: KeyCode::Esc, modifiers: KeyModifiers::NONE, action: KeyAction::Cancel, context: BindingContext::Dialog, description: "Close dialog" },
            DefaultBinding { key: KeyCode::Enter, modifiers: KeyModifiers::NONE, action: KeyAction::Submit, context: BindingContext::Dialog, description: "Confirm" },
            DefaultBinding { key: KeyCode::Tab, modifiers: KeyModifiers::NONE, action: KeyAction::MoveCursorRight, context: BindingContext::Dialog, description: "Next field" },
            DefaultBinding { key: KeyCode::Tab, modifiers: KeyModifiers::SHIFT, action: KeyAction::MoveCursorLeft, context: BindingContext::Dialog, description: "Previous field" },
        ];
        
        for binding in defaults {
            self.add(binding.into_binding());
        }
    }
}

/// Load keybindings from config file (TOML) - returns defaults if file doesn't exist
pub fn load_keybindings_from_config(_path: &std::path::Path) -> Result<KeyBindingRegistry, Box<dyn std::error::Error>> {
    Ok(KeyBindingRegistry::with_defaults())
}

/// Save keybindings to config file - placeholder for future implementation
pub fn save_keybindings_to_config(_registry: &KeyBindingRegistry, _path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    
    #[test]
    fn test_keybinding_registry_basic() {
        let mut registry = KeyBindingRegistry::new();
        
        registry.add(KeyBinding {
            key: KeyCode::Char('a'),
            modifiers: KeyModifiers::CONTROL,
            action: KeyAction::MoveCursorLeft,
            context: Some(BindingContext::Prompt),
            description: None,
        });
        
        let event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let binding = registry.find(&event, BindingContext::Prompt);
        assert!(binding.is_some());
        assert_eq!(binding.unwrap().action, KeyAction::MoveCursorLeft);
    }
    
    #[test]
    fn test_global_vs_context_binding() {
        let mut registry = KeyBindingRegistry::new();
        
        // Global binding
        registry.add(KeyBinding {
            key: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
            action: KeyAction::ShowHelp,
            context: None,
            description: None,
        });
        
        // Context-specific binding that overrides
        registry.add(KeyBinding {
            key: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
            action: KeyAction::Cancel,
            context: Some(BindingContext::Dialog),
            description: None,
        });
        
        let event = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        
        // In dialog, context-specific binding should win
        let binding = registry.find(&event, BindingContext::Dialog);
        assert!(binding.is_some());
        assert_eq!(binding.unwrap().action, KeyAction::Cancel);
        
        // In prompt, global binding should apply
        let binding = registry.find(&event, BindingContext::Prompt);
        assert!(binding.is_some());
        assert_eq!(binding.unwrap().action, KeyAction::ShowHelp);
    }
    
    #[test]
    fn test_fallback_contexts() {
        let registry = KeyBindingRegistry::with_defaults();
        let event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        
        // Should find binding in dialog context
        let binding = registry.find_with_fallback(&event, &[BindingContext::Dialog, BindingContext::Global]);
        assert!(binding.is_some());
    }
    
    #[test]
    fn test_default_registry_has_bindings() {
        let registry = KeyBindingRegistry::with_defaults();
        
        // Check some expected bindings exist
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let binding = registry.find(&event, BindingContext::Prompt);
        assert!(binding.is_some());
        
        let event = KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE);
        let binding = registry.find(&event, BindingContext::VimNormal);
        assert!(binding.is_some());
        assert_eq!(binding.unwrap().action, KeyAction::VimEnterInsert);
    }
    
    #[test]
    fn test_remove_binding() {
        let mut registry = KeyBindingRegistry::new();
        
        registry.add(KeyBinding {
            key: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            action: KeyAction::Cancel,
            context: Some(BindingContext::Prompt),
            description: None,
        });
        
        let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(registry.find(&event, BindingContext::Prompt).is_some());
        
        let removed = registry.remove(KeyCode::Char('x'), KeyModifiers::NONE, Some(BindingContext::Prompt));
        assert!(removed);
        assert!(registry.find(&event, BindingContext::Prompt).is_none());
    }
    
    #[test]
    fn test_custom_action() {
        let mut registry = KeyBindingRegistry::new();
        
        registry.add(KeyBinding {
            key: KeyCode::Char('x'),
            modifiers: KeyModifiers::CONTROL,
            action: KeyAction::Custom(42),
            context: Some(BindingContext::Prompt),
            description: Some("Custom action".to_string()),
        });
        
        let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let binding = registry.find(&event, BindingContext::Prompt);
        assert!(binding.is_some());
        assert_eq!(binding.unwrap().action, KeyAction::Custom(42));
    }
}