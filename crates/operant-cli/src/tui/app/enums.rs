//! Supporting types for the TUI application.
//!
//! Contains all enum and struct definitions used throughout the app module:
//! `SystemMessageStyle`, `ContextMenuKind`, `ContextMenuState`, `ContextMenuItem`,
//! `KeyContext`, `DialogPriority`, `ToolStatus`, `TurnState`, `ToolUseBlock`,
//! `TurnMetadata`, `FocusTarget`, `SystemAnnotation`, and `ComposerRecovery`.

/// What the composer should hold after a submission failed.
///
/// The decision itself lives in
/// [`App::resolve_composer_after_failure`](super::App::resolve_composer_after_failure);
/// this is its output, split out so the rule can be unit-tested without an
/// agent, a provider, or a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComposerRecovery {
    /// The submission never produced anything. Nothing to give back.
    Nothing,
    /// Put this text back verbatim.
    Restore(String),
    /// The user has already started typing a new prompt. Leave the composer
    /// exactly as it is.
    KeepExisting,
}

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
    /// Theme picker. Gated inline in key_handling.rs but absent from
    /// dialog_priority() until now; see docs/ROADMAP-TUI-FLEET.md 6.1.
    ThemeScreen = 330,
    /// Rewind flow (message select + confirm). Same omission as ThemeScreen.
    RewindFlow = 340,
    /// Memory file selector. Same omission as ThemeScreen.
    MemoryFileSelector = 350,
}

/// Status of an active or completed tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool call parsed and waiting for a worker permit from the
    /// concurrent-execution pool. Set on the first `ToolStart` for a
    /// tool call id; flipped to [`ToolStatus::Running`] by the second
    /// `ToolStart`, which the agent emits once the permit is acquired.
    Queued,
    Running,
    Done,
    Error,
}

impl ToolStatus {
    /// True while the tool has not settled — queued for a permit OR
    /// actively running. Every "is this turn still working?" check goes
    /// through here so a queued tool is never mistaken for a finished
    /// one.
    pub fn is_pending(self) -> bool {
        matches!(self, ToolStatus::Queued | ToolStatus::Running)
    }
}

/// Where the current agent turn is in its lifecycle.
///
/// `App::is_streaming` collapses every in-flight state below into one
/// boolean, which is why "awaiting approval", "retrying", "compacting" and
/// "errored" used to be indistinguishable from "thinking". `TurnState` is
/// what the status line reads so the specific phase survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnState {
    /// No turn in flight.
    #[default]
    Idle,
    /// The request was handed to the agent; nothing has come back yet.
    Sending,
    /// The provider transport is opening (connect / first byte).
    ///
    /// No `AgentEvent` signals the pre-first-byte window yet — the agent
    /// emits nothing between `run()` being called and the first Content
    /// event — so nothing assigns this today. Kept because the lifecycle is
    /// modelled in full and a provider-connect event will land in it.
    #[expect(dead_code, reason = "no pre-first-byte event exists to drive it yet")]
    Connecting,
    /// The model is reasoning; no assistant text yet.
    Thinking,
    /// Assistant text is arriving.
    Streaming,
    /// A tool call is executing (or queued on the worker pool).
    RunningTool,
    /// A permission dialog is blocking the turn.
    WaitingForApproval,
    /// A retry is scheduled or rate limited — nothing is moving on the wire.
    WaitingForNetwork,
    /// Context compaction is rewriting the transcript.
    Compacting,
}

impl TurnState {
    /// Short human label for the transient status line. The renderer appends
    /// the ellipsis, so no variant carries a trailing `…`.
    pub fn label(self) -> &'static str {
        match self {
            TurnState::Idle => "Idle",
            TurnState::Sending => "Sending",
            TurnState::Connecting => "Connecting",
            TurnState::Thinking => "Thinking",
            TurnState::Streaming => "Streaming",
            TurnState::RunningTool => "Running tool",
            TurnState::WaitingForApproval => "Awaiting approval",
            TurnState::WaitingForNetwork => "Retrying",
            TurnState::Compacting => "Compacting context",
        }
    }

    /// True when the state names something the generic shimmer can't
    /// express, so the status line must show it even when `status_message`
    /// is empty.
    pub fn is_noteworthy(self) -> bool {
        matches!(
            self,
            TurnState::RunningTool
                | TurnState::WaitingForApproval
                | TurnState::WaitingForNetwork
                | TurnState::Compacting
        )
    }
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
