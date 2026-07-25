//! Supporting types for the TUI application.
//!
//! Contains all enum and struct definitions used throughout the app module:
//! `SystemMessageStyle`, `ContextMenuKind`, `ContextMenuState`, `ContextMenuItem`,
//! `KeyContext`, `DialogPriority`, `ToolStatus`, `ToolUseBlock`, `TurnMetadata`,
//! `FocusTarget`, `SystemAnnotation`, and `ACCENT_BUILD`.

/// Visual style for inline system messages in the conversation pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemMessageStyle {
    Info,
    /// Compact / auto-compact boundary marker.
    Compact,
}

/// A synthetic system annotation inserted between conversation messages.
/// `after_index` is the index in `App::messages` after which this annotation
/// should appear (0 = before all messages, 1 = after message 0, etc.).
#[derive(Debug, Clone)]
pub struct SystemAnnotation {
    pub after_index: usize,
    pub text: String,
    pub style: SystemMessageStyle,
}

/// Context menu state: position and currently selected item index.
#[derive(Debug, Clone, Copy)]
pub struct ContextMenuState {
    /// X coordinate of the menu (column).
    pub x: u16,
    /// Y coordinate of the menu (row).
    pub y: u16,
    /// Currently selected menu item index (0-based).
    pub selected_index: usize,
    /// What the context menu is acting on.
    pub kind: ContextMenuKind,
}

/// What content the context menu is currently targeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuKind {
    /// A specific transcript message.
    Message { message_index: usize },
    /// The current text selection anywhere in the frame.
    Selection,
}

/// Available context menu items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    Copy,
    Fork,
}

/// Key context for determining which key bindings apply.
/// Mirrors claurst's KeyContext for cleaner key routing.
#[allow(dead_code)] // Prepared for key routing system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// Normal prompt input mode
    Prompt,
    /// Vim normal mode in prompt
    VimNormal,
    /// Vim visual mode in prompt
    VimVisual,
    /// Vim visual line mode
    VimVisualLine,
    /// Vim visual block mode
    VimVisualBlock,
    /// Vim command mode
    VimCommand,
    /// Global context (always active)
    Global,
    /// Transcript/message pane
    Transcript,
    /// Diff viewer
    DiffViewer,
    /// Dialog overlay (any modal dialog)
    Dialog,
    /// Context menu open
    ContextMenu,
    /// Help overlay
    Help,
    /// Settings screen
    Settings,
    /// Model picker
    ModelPicker,
    /// Session browser
    SessionBrowser,
    /// Command palette
    CommandPalette,
    /// Global search
    GlobalSearch,
    /// History search overlay
    HistorySearch,
    /// MCP view
    MCPView,
    /// Agents menu
    AgentsMenu,
    /// Stats dialog
    Stats,
    /// Export dialog
    Export,
    /// Context visualization
    ContextViz,
    /// Session branching
    SessionBranching,
    /// Tasks overlay
    Tasks,
    /// Menu context (dialog pickers)
    Menu,
    /// Plugins hub
    PluginsHub,
    /// Skills view
    SkillsView,
    /// Journey view
    JourneyView,
    /// Hooks config menu
    HooksConfig,
    /// Voice mode notice
    VoiceModeNotice,
}

/// Dialog priority for key routing.
/// Higher values = higher priority (handled first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DialogPriority {
    /// No dialog active
    #[allow(dead_code)] // Prepared for dialog priority routing
    None = 0,
    /// Context menu
    ContextMenu = 10,
    /// Bypass permissions - must accept or session exits
    BypassPermissions = 20,
    /// MCP approval
    McpApproval = 30,
    /// Device auth (OAuth)
    DeviceAuth = 40,
    /// Ask user dialog
    AskUser = 50,
    /// Key input dialog
    KeyInput = 60,
    /// Custom provider dialog
    CustomProvider = 70,
    /// Free mode dialog
    FreeMode = 80,
    /// Import config dialog
    ImportConfig = 90,
    /// Effort picker
    EffortPicker = 100,
    /// Connect dialog
    Connect = 110,
    /// Import config picker
    ImportConfigPicker = 120,
    /// Command palette
    CommandPalette = 130,
    /// Model picker
    ModelPicker = 140,
    /// Settings screen
    Settings = 150,
    /// Export dialog
    Export = 160,
    /// Stats dialog
    Stats = 170,
    /// Context viz
    ContextViz = 180,
    /// Session browser
    SessionBrowser = 190,
    /// Session branching
    SessionBranching = 200,
    /// Tasks overlay
    Tasks = 210,
    /// Global search
    GlobalSearch = 220,
    /// History search overlay
    HistorySearch = 230,
    /// Help overlay
    Help = 240,
    /// MCP view
    MCPView = 250,
    /// Agents menu
    AgentsMenu = 260,
    /// Diff viewer
    DiffViewer = 270,
    /// Plugins hub
    PluginsHub = 280,
    /// Skills view
    SkillsView = 290,
    /// Journey view
    JourneyView = 300,
    /// Hooks config menu
    HooksConfig = 310,
    /// Voice mode notice
    VoiceModeNotice = 320,
}

/// Status of an active or completed tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Done,
    Error,
}

/// Represents an active or completed tool invocation visible in the UI.
#[derive(Debug, Clone)]
pub struct ToolUseBlock {
    pub id: String,
    pub name: String,
    pub turn_index: Option<usize>,
    pub status: ToolStatus,
    pub output_preview: Option<String>,
    /// JSON-serialised input for the tool call (populated from the API stream).
    pub input_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct TurnMetadata {
    pub model_name: Option<String>,
    pub agent_mode: Option<String>,
    pub duration: Option<String>,
    pub interrupted: bool,
}

/// Which area of the TUI currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// Keyboard input goes to the prompt editor.
    Input,
    /// Keyboard input goes to the transcript/message pane (scroll, etc.).
    Transcript,
}

/// Accent color for build mode (default pink).
pub const ACCENT_BUILD: ratatui::style::Color = ratatui::style::Color::Rgb(255, 191, 0);
