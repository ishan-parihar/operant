// app/mod.rs — App state struct and main event loop.

mod agent_events;
mod commands;
mod dialog_routing;
mod enums;
mod helpers;
mod import_config;
mod init;
mod key_handling;
mod messaging;
mod mouse;
mod prompt;
mod providers;
mod turn_state;

#[cfg(test)]
mod tests;
pub use enums::*;
use helpers::*;

use crate::tui::adapter_types::config::{Settings, Theme};
use crate::tui::adapter_types::cost::CostTracker;
use crate::tui::context_viz::ContextVizState;
use crate::tui::dialog_select::{DialogSelectState, SelectItem};
use crate::tui::dialogs::McpApprovalDialogState;
use crate::tui::dialogs::PermissionRequest;
use crate::tui::diff_viewer::DiffViewerState;
use crate::tui::export_dialog::{ExportDialogState, ExportFormat};
use crate::tui::import_config_dialog::ImportConfigDialogState;
use crate::tui::mcp_view::{McpServerView, McpToolView, McpViewState, McpViewStatus};
use crate::tui::model_picker::{EffortLevel, ModelPickerState};
use crate::tui::notifications::{NotificationKind, NotificationQueue};
use crate::tui::overlays::{
    GlobalSearchState, HelpOverlay, HistorySearchOverlay, RewindFlowOverlay, SelectorMessage,
};
use crate::tui::prompt_input::{InputMode, PromptInputState, VimMode};
use crate::tui::render;
use crate::tui::session_browser::SessionBrowserState;
use crate::tui::settings_screen::SettingsScreen;
use crate::tui::stats_dialog::StatsDialogState;
use crate::tui::tasks_overlay::TasksOverlay;
use crate::tui::theme_screen::ThemeScreen;
use crate::tui::{
    agents_view::{AgentInfo, AgentStatus, AgentsMenuState, AgentsRoute},
    diff_viewer::DiffPane,
};
use operant_core::config::AppConfig;
// (iter-209: FileHistory import deleted — stub removed, turn-diff feature cut)
use crate::tui::adapter_types::types::{ContentBlock, Message, Role};
use crate::tui::adapter_types::{sample_completion_verb, sample_spinner_verb};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use operant_core::agent::AgentEvent;
use ratatui::Terminal;
use ratatui::style::Color;
use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};
use tracing::debug;

// ---------------------------------------------------------------------------
// App struct
// ---------------------------------------------------------------------------

/// The top-level TUI application.
pub struct App {
    // Core state
    pub config: AppConfig,
    pub settings: Settings,
    pub project_dir: Option<std::path::PathBuf>,
    pub is_simulating: bool,
    pub simulated_keys: Vec<crossterm::event::KeyEvent>,
    /// Headless-simulation frame cap. When simulating, the run loop exits
    /// once `frame_count` reaches this, so a scenario that never stops
    /// streaming can't hang the test suite. `None` = no cap (interactive).
    pub simulation_max_frames: Option<u64>,
    pub cost_tracker: Arc<CostTracker>,
    /// TUI debugging hub — event bus, frame counter, F12 overlay.
    /// Enabled by OPERANT_TUI_DEBUG=1. Zero overhead when disabled.
    pub debug_hub: crate::tui::debug::TuiDebugHub,
    /// Command registry for dispatching slash commands to backend handlers.
    pub command_registry: crate::commands::CommandRegistry,
    pub messages: Vec<Message>,
    /// Synthetic system annotations interleaved between real messages at render time.
    pub system_annotations: Vec<SystemAnnotation>,
    pub input: String,
    pub prompt_input: PromptInputState,
    pub scroll_offset: usize,
    pub is_streaming: bool,
    pub streaming_text: String,
    pub streaming_thinking: String,
    /// Whether reasoning/thinking blocks are expanded by default in the
    /// transcript. Toggled by /reasoning. (Bug #18 from iter-82 audit —
    /// /reasoning previously just printed a status message without toggling.)
    pub show_reasoning: bool,
    pub status_message: Option<String>,
    /// Randomly chosen thinking verb shown next to the spinner while streaming.
    pub spinner_verb: Option<String>,
    pub should_exit: bool,
    pub show_help: bool,

    /// Pending shell command to run after the current `app.run()` returns.
    /// Set by slash commands like `/setup` that need to suspend the TUI and
    /// spawn an interactive subprocess. The run loop in `TuiApp::run` polls
    /// this after each frame and, if set, leaves alt screen + raw mode,
    /// spawns the command with inherited stdio, waits for it, then re-enters
    /// alt screen + raw mode and clears this field.
    pub pending_shell_command: Option<Vec<String>>,

    // Extended state
    pub tool_use_blocks: Vec<ToolUseBlock>,
    pub permission_request: Option<PermissionRequest>,
    pub frame_count: u64,
    /// Performance tier for adaptive redraw cadence (Minimal/Normal/High).
    pub perf_tier: crate::tui::redraw::PerformanceTier,
    /// Timestamp of last user activity (for idle detection in redraw cadence).
    pub last_activity: std::time::Instant,
    /// Whether the terminal window currently has focus. When the app is
    /// backgrounded, the redraw cadence drops to the slowest tier so a
    /// backgrounded tab doesn't burn CPU/battery. Updated from crossterm
    /// `Event::FocusGained`/`Event::FocusLost` (Phase 2.3).
    pub client_focused: bool,
    pub token_count: u32,
    pub cost_usd: f64,
    pub model_name: String,
    /// Active provider for the current session (e.g. "anthropic", "openai").
    /// This is a runtime-only field — it is NOT persisted to settings.json.
    /// Source of truth is `config.agent.model`; provider is inferred from it.
    pub active_provider: Option<String>,
    /// Whether the app has valid API credentials configured.
    /// False = show the in-TUI provider setup dialog on startup.
    pub has_credentials: bool,
    /// Current effort level (controls extended-thinking budget_tokens).
    pub effort_level: EffortLevel,
    /// Whether fast mode is currently active (model locked to FAST_MODE_MODEL).
    pub fast_mode: bool,
    /// Active speech mode: None = normal, Some("caveman") / Some("rocky").
    /// Speech mode intensity: "lite", "full", "ultra".
    /// Current agent mode name: "build", "plan", "explore", etc.
    pub agent_mode: Option<String>,
    /// Accent color for TUI chrome (currently fixed at the build/default accent).
    pub accent_color: Color,
    /// Set when the agent mode changes (e.g. via `/personality`) so the main
    /// loop can update the query config and tool list to match.
    pub agent_mode_changed: bool,
    pub agent_status: Vec<(String, String)>,

    // Cursor position within input (byte offset)
    pub cursor_pos: usize,

    // ---- Scrollback / auto-scroll -----------------------------------------
    /// When `true`, the message pane follows the latest messages automatically.
    pub auto_scroll: bool,
    /// Count of messages that arrived while the user was scrolled up.
    pub new_messages_while_scrolled: usize,

    // ---- Token warning tracking -------------------------------------------
    /// Which threshold (0 = none, 80, 95, 100) was last notified so we only
    /// show each banner once.
    pub token_warning_threshold_shown: u8,

    // ---- Session timing ---------------------------------------------------
    /// Instant the session started (used for elapsed-time in the status bar).
    pub session_start: std::time::Instant,
    /// Current Rustle pose for rendering (updated each frame).
    /// Temporary Rustle pose override (e.g. look-down on Tab). Reverts to
    /// default after this instant passes.
    /// Frame counter at which the next random eye-shift should fire.
    /// Instant the current turn's streaming began (reset each time streaming starts).
    pub turn_start: Option<std::time::Instant>,
    /// Elapsed time string for the last completed turn, e.g. "2m 5s".
    pub last_turn_elapsed: Option<String>,
    /// Past-tense verb shown after turn completes, e.g. "Worked" / "Baked".
    pub last_turn_verb: Option<&'static str>,
    /// Per-user turn snapshots used by the transcript renderer.
    pub turn_metadata: Vec<TurnMetadata>,
    /// Incremented whenever transcript-visible state changes so rendering can
    /// reuse cached layout between keystrokes.
    pub transcript_version: Cell<u64>,

    // ---- New overlay / notification fields --------------------------------
    /// Full-screen help overlay (? / F1).
    pub help_overlay: HelpOverlay,
    /// Ctrl+R history search overlay.
    pub history_search_overlay: HistorySearchOverlay,
    /// Global ripgrep search / quick-open overlay.
    pub global_search: GlobalSearchState,
    /// Message selector used by /rewind.
    /// Multi-step rewind flow overlay.
    pub rewind_flow: RewindFlowOverlay,
    /// Bridge connection state.
    /// Active notification queue.
    pub notifications: NotificationQueue,
    /// Scroll offset for error modal text (in lines).
    pub error_modal_scroll_offset: usize,
    /// Plugin hint banners.
    /// Optional session title shown in the status bar.
    pub session_title: Option<String>,
    /// Remote session URL (set when bridge connects; readable by commands).
    #[allow(dead_code)] // Prepared for bridge connection UI
    pub remote_session_url: Option<String>,
    /// Bridge/gateway connection state for status bar badge.
    #[allow(dead_code)] // Prepared for bridge status badge
    pub bridge_state: crate::tui::bridge_state::BridgeConnectionState,
    /// Live MCP manager snapshot source when available.
    /// (iter-208: stub mcp_manager field deleted — load_mcp_servers now reads
    /// from core_mcp_manager directly.)
    /// Real operant-core McpManager handle for reconnect operations.
    /// Set by TuiApp::run after create_runtime_agent. When /mcp 'r' is
    /// pressed, the run loop calls remove_server + add_server on this.
    /// (iter-93 — closes the /mcp reconnect parity gap.)
    /// (iter-208 — also used by load_mcp_servers for live tool/status data.)
    pub core_mcp_manager: Option<Arc<operant_core::mcp::McpManager>>,
    /// Agent steer queue handle. Set by TuiApp::run after create_runtime_agent.
    /// When the user types while a turn is streaming, the input is pushed here
    /// so the agent sees it as a steer directive at the next iteration boundary.
    /// (iter-93 — closes the /steer parity gap.)
    pub steer_queue_handle: Option<Arc<tokio::sync::Mutex<Vec<String>>>>,
    /// Queued request for a real MCP reconnect from the interactive loop.
    pub pending_mcp_reconnect: bool,
    /// Pending MCP panel-auth request for the interactive loop.
    pub pending_mcp_panel_auth: Option<String>,
    // (iter-209: file_history + current_turn fields deleted — stub
    // FileHistory removed, turn-diff feature cut as YAGNI.)
    /// Slash-command usage stats (recency + frequency) for smart ordering
    /// of `/` suggestions. Loaded from `~/.operant/slash-usage.json` on
    /// startup; saved back on every command invocation.
    /// (iter-125 — smart slash-command ordering.)
    pub slash_usage: crate::tui::slash_usage::UsageStore,
    /// Standing session goal set via /goal. Injected as a system preamble
    /// message whenever it is set and displayed in the status bar.
    /// (iter-270 — wires /goal to real state.)
    pub session_goal: Option<String>,
    /// If set, the run loop will re-submit this query on the next iteration
    /// as if the user typed it. Used by /retry to resubmit the last user msg.
    /// (iter-270 — wires /retry to real state.)
    pub pending_retry_query: Option<String>,

    // ---- Visual mode indicators -------------------------------------------
    /// Plan mode — input border turns blue, [PLAN] shown in status bar.
    pub plan_mode: bool,
    /// "While you were away" summary text shown on the welcome screen.
    /// When streaming stalled (used to turn the spinner red after 3 s).
    pub stall_start: Option<std::time::Instant>,

    // ---- Settings / theme / privacy screens --------------------------------
    /// Full-screen tabbed settings screen (/config, /settings).
    pub settings_screen: SettingsScreen,
    /// Theme picker overlay (/theme).
    pub theme_screen: ThemeScreen,
    /// Token/cost analytics dialog.
    pub stats_dialog: StatsDialogState,
    /// MCP server browser and tool detail view.
    pub mcp_view: McpViewState,
    /// Agent definitions and active agent status overlay.
    pub agents_menu: AgentsMenuState,
    /// Diff viewer overlay.
    pub diff_viewer: DiffViewerState,
    // (iter-211: feedback_survey field deleted — no telemetry backend, YAGNI)
    /// Memory file selector overlay (AGENTS.md browser).
    pub memory_file_selector: crate::tui::memory_file_selector::MemoryFileSelectorState,
    pub skills_view: crate::tui::skills_view::SkillsViewState,
    pub plugins_hub: crate::tui::plugins_hub::PluginsHubState,
    pub journey_view: crate::tui::journey_view::JourneyViewState,
    /// Read-only hooks configuration browser.
    pub hooks_config_menu: crate::tui::hooks_config_menu::HooksConfigMenuState,
    /// Overage credit upsell banner.
    /// Voice mode availability notice.
    pub voice_mode_notice: crate::tui::voice_mode_notice::VoiceModeNoticeState,
    /// Desktop app upsell startup dialog.
    /// Startup error dialog for malformed settings.json or AGENTS.md.
    /// Memory update notification banner.
    /// MCP elicitation dialog (form requested by an MCP server).
    /// Model picker overlay (/model command).
    pub model_picker: ModelPickerState,
    /// Session browser overlay (/session, /resume, /rename, /export).
    pub session_browser: SessionBrowserState,
    /// Session branching overlay (Ctrl+B) — create and switch branches.
    pub session_branching: crate::tui::session_branching::SessionBranchingState,
    /// Task progress overlay (Ctrl+T) — shows task status with toggle capability.
    pub tasks_overlay: TasksOverlay,
    /// Export format picker dialog (/export).
    pub export_dialog: ExportDialogState,
    /// Context window / rate limit visualization overlay (/context).
    pub context_viz: ContextVizState,
    /// MCP server approval dialog.
    pub mcp_approval: McpApprovalDialogState,
    /// Go to Line dialog (Ctrl+G in message pane).

    /// Bypass-permissions startup confirmation dialog.
    /// Shown at startup when --dangerously-skip-permissions was passed.
    /// User must explicitly accept or the session exits.
    pub bypass_permissions_dialog:
        crate::tui::bypass_permissions_dialog::BypassPermissionsDialogState,
    /// First-launch onboarding welcome dialog.
    /// Effort-level picker (/effort with no args).
    pub effort_picker: crate::tui::effort_picker::EffortPickerState,
    /// API key input dialog (opened from /connect for key-based providers).
    pub key_input_dialog: crate::tui::key_input_dialog::KeyInputDialogState,
    /// Custom provider dialog for URL + API key input.
    pub custom_provider_dialog: crate::tui::custom_provider_dialog::CustomProviderDialogState,
    /// "Free" composite-provider setup dialog (warning + 2 API keys).
    pub free_mode_dialog: crate::tui::free_mode_dialog::FreeModeDialogState,
    /// Device code / browser auth dialog (GitHub Copilot device flow, Anthropic OAuth).
    pub device_auth_dialog: crate::tui::device_auth_dialog::DeviceAuthDialogState,
    /// When set, the main loop should spawn the async auth task for this provider.
    pub device_auth_pending: Option<String>,
    /// Shared provider registry for dynamic model fetching.
    /// Model registry populated from models.dev — single source of truth for
    /// all provider models shown in the `/model` picker.
    pub model_registry: crate::tui::adapter_types::ModelRegistry,
    /// When `true`, the main event loop should spawn an async task to fetch
    /// the model list from the current provider's `list_models()` API.
    pub model_picker_fetch_pending: bool,
    /// The provider ID that the model picker was opened for (used when the
    /// fetch is triggered from /connect before the provider is activated).
    pub model_picker_provider_id: Option<String>,
    /// When `true`, the main event loop should spawn an async task to load
    /// the session list from disk and populate the session browser.
    pub session_list_pending: bool,
    /// Receiver for background session-list results.
    pub session_list_rx:
        Option<tokio::sync::mpsc::Receiver<Vec<crate::tui::session_browser::SessionEntry>>>,
    /// Session ID to load on the next run-loop iteration (set by /resume
    /// Enter). The run loop spawns a background load_session task and stores
    /// the receiver in session_load_rx.
    pub session_load_pending: Option<String>,
    /// Receiver for background session-message-load results. Each item is a
    /// Vec of (role, content) pairs that will replace app.messages.
    pub session_load_rx: Option<tokio::sync::mpsc::Receiver<Vec<(String, String)>>>,
    /// Credential store for provider API keys and OAuth tokens.
    pub auth_store: crate::tui::adapter_types::AuthStore,
    /// Connect-a-provider dialog (/connect command).
    pub connect_dialog: DialogSelectState,
    /// Import-config source picker (/import-config command).
    pub import_config_picker: DialogSelectState,
    /// Import-config preview and confirmation dialog.
    pub import_config_dialog: ImportConfigDialogState,
    /// Ctrl+K command palette overlay.
    pub command_palette: DialogSelectState,
    /// Output style: "auto" | "stream" | "verbose".
    pub output_style: String,
    /// PR number for the current branch (None if not in a PR context).
    /// PR URL for the current branch.
    /// PR review state: "approved", "changes_requested", "review_required", etc.
    /// Current working directory path.
    pub current_dir: Option<String>,
    /// Current git branch name.
    pub git_branch: Option<String>,
    /// Count of in-progress background tasks (drives the footer pill).
    /// Background task status text shown in footer pill.
    /// External status line command output (from CLAUDE_STATUS_COMMAND).
    /// Whether auto-compact is enabled (from settings).
    pub auto_compact_enabled: bool,
    /// Guard to prevent re-triggering auto-compact while one is in flight.

    // ---- Voice hold-to-talk ------------------------------------------------

    /// The global voice recorder, Some when voice is enabled in config.
    pub voice_recorder: Option<Arc<Mutex<crate::tui::adapter_types::voice::VoiceRecorder>>>,
    /// True while recording is active (Alt+V toggled on).
    pub voice_recording: bool,
    /// Receiver for VoiceEvent messages produced by the recorder task.
    pub voice_event_rx:
        Option<tokio::sync::mpsc::Receiver<crate::tui::adapter_types::voice::VoiceEvent>>,
    /// Receiver for AgentEvent messages from the agent. (iter-114 — was
    /// QueryEvent via the bridge; now receives AgentEvent directly, eliminating
    /// the bridge layer and its translation bugs.)
    pub agent_event_rx: Option<tokio::sync::mpsc::Receiver<AgentEvent>>,
    /// Receiver for tool permission requests from the agent. Each request is
    /// surfaced as a `PermissionRequest` dialog; the user's choice is routed
    /// back to the agent via `pending_permission_response_tx`.
    pub permission_rx:
        Option<tokio::sync::mpsc::Receiver<operant_core::agent::ToolPermissionRequest>>,
    /// The response channel for the currently-shown permission dialog. Set
    /// when a `ToolPermissionRequest` is popped from `permission_rx` and
    /// consumed when the user picks an option (or Esc/deny).
    pub pending_permission_response_tx:
        Option<tokio::sync::oneshot::Sender<operant_core::agent::ToolPermissionResponse>>,
    pub run_complete_rx: Option<tokio::sync::oneshot::Receiver<operant_core::error::Result<()>>>,
    /// Handle to the background agent task. Aborted on Escape so the task
    /// actually stops instead of continuing to run in the background.
    pub agent_task_handle: Option<tokio::task::JoinHandle<()>>,
    /// A single key event that was drained from the queue during paste-burst
    /// detection but wasn't part of the burst (e.g. a modifier key that stopped
    /// the burst). Replayed at the top of the next loop iteration.
    pending_key: Option<crossterm::event::KeyEvent>,
    /// Receiver for model-list results fetched in the background when the
    /// /model picker opens.  Drained each frame so models appear as soon as
    /// the fetch completes.
    pub model_fetch_rx: Option<
        tokio::sync::mpsc::Receiver<Result<Vec<crate::tui::model_picker::ModelEntry>, String>>,
    >,
    /// Receiver for `UserQuestionRequest`s produced by the clarify tool.
    /// When a question arrives, `ask_user_dialog` is populated and shown.
    /// The reply_tx in the request is stored in `pending_permission_response_tx`
    /// (reused) or `ask_user_dialog.reply_tx` — when the user confirms,
    /// the reply flows back through the oneshot to the clarify tool.
    pub user_question_rx: Option<
        tokio::sync::mpsc::UnboundedReceiver<operant_core::user_question::UserQuestionRequest>,
    >,
    /// State for the model-initiated ask-user question dialog.
    pub ask_user_dialog: crate::tui::ask_user_dialog::AskUserDialogState,

    // ---- Bridge/gateway connection state -----------------------------------
    /// Receiver for bridge connection state updates from gateway handler.
    pub bridge_state_rx: Option<
        tokio::sync::mpsc::UnboundedReceiver<crate::tui::bridge_state::BridgeConnectionState>,
    >,
    /// Sender for bridge connection state updates (given to gateway handler).
    #[allow(dead_code)] // Prepared for bridge connection handler
    pub bridge_state_tx:
        Option<tokio::sync::mpsc::UnboundedSender<crate::tui::bridge_state::BridgeConnectionState>>,

    // ---- Context window & rate limit info ----------------------------------
    /// Total context window size for the current model (tokens).
    pub context_window_size: u64,
    /// How many tokens are currently used in the context window.
    pub context_used_tokens: u64,
    /// Rate limit info — 5-hour window usage percentage (0–100).
    pub rate_limit_5h_pct: Option<f32>,
    /// Rate limit info — 7-day window usage percentage (0–100).
    pub rate_limit_7day_pct: Option<f32>,
    /// Active worktree name (if in a worktree). Rendered in the footer.
    /// Active worktree branch (if in a worktree). Rendered in the footer.
    /// Agent type badge: "agent" | "coordinator" | "subagent".
    /// Goal badge string shown in the footer, e.g. "active · 5m · 3 turns".
    /// None when no goal is active. Updated by the REPL after each turn.

    // ---- Thinking block expansion state ----------------------------------
    /// Set of thinking block content hashes that are expanded.
    pub thinking_expanded: std::collections::HashSet<u64>,
    /// The message pane area from the last render frame (used for mouse hit testing).
    pub last_msg_area: Cell<ratatui::layout::Rect>,
    /// The frame region that supports text selection.
    pub last_selectable_area: Cell<ratatui::layout::Rect>,
    /// The prompt input area from the last render frame (used for focus routing).
    pub last_input_area: Cell<ratatui::layout::Rect>,
    /// The footer's right column area (where tips are shown) from the last render.
    pub footer_right_column_area: Cell<ratatui::layout::Rect>,
    /// Which area of the TUI currently has keyboard focus.
    pub focus: FocusTarget,
    /// Maps virtual_row_index → thinking_block_hash for click detection.
    pub thinking_row_map: RefCell<std::collections::HashMap<u16, u64>>,
    /// Maps screen row → transcript message index for right-click hit testing.
    pub message_row_map: RefCell<std::collections::HashMap<u16, usize>>,
    /// Scroll offset from the last render frame (used for selection validation).
    pub last_render_scroll_offset: Cell<u16>,

    // ---- Text selection state --------------------------------------------
    /// Selection drag anchor (col, row) — set on mouse-down.
    pub selection_anchor: Option<(u16, u16)>,
    /// Selection drag focus (col, row) — updated on mouse-drag / mouse-up.
    pub selection_focus: Option<(u16, u16)>,
    /// Text extracted from the current selection (updated each render frame).
    pub selection_text: RefCell<String>,
    /// Cache of row -> rendered text within the selectable area, refreshed
    /// each frame. Used by double/triple-click word and paragraph detection
    /// (issue #149 follow-up: prior word-boundary detection was a placeholder).
    pub last_row_text: RefCell<std::collections::HashMap<u16, String>>,

    // ---- Advanced mouse interaction state --------------------------------
    /// Timestamp of the last left mouse click (for double/triple-click detection).
    pub last_click_time: Option<std::time::Instant>,
    /// Position of the last left mouse click (for double/triple-click detection).
    pub last_click_position: Option<(u16, u16)>,
    /// Count of consecutive clicks: 1 = single, 2 = double, 3+ = triple.
    pub click_count: u32,
    /// Context menu state: position and selected index.
    pub context_menu_state: Option<ContextMenuState>,

    // ---- Scroll acceleration state (trackpad feel) -----------------------
    /// Current acceleration multiplier for scroll events.
    scroll_accel: f32,
    /// Timestamp of the last scroll event (for burst detection).
    scroll_last_time: Option<std::time::Instant>,

    // ---- Bash prefix allowlist -------------------------------------------
    /// Command prefixes that have been permanently allowed this session via
    /// the "Allow commands starting with X" option in the bash permission dialog.
    /// Before showing the dialog for a bash command, the first whitespace-delimited
    /// word is checked against this set; a match silently auto-approves the request.
    pub bash_prefix_allowlist: std::collections::HashSet<String>,

    // ---- Auto-update notification ----------------------------------------
    /// If a newer version was found during background update check, this holds
    /// the latest version string (e.g. "0.1.0"). Shown in the footer status bar.
    /// Whether managed agent mode is currently active.
    /// Timestamp of the first exit key press that showed confirmation (valid for ~2 seconds).
    pub last_exit_key_warning: Option<std::time::Instant>,
    /// Which exit key ('c' or 'd') started the current confirmation sequence.
    pub exit_key_sequence_start: Option<char>,
}

// Spinner verbs are now imported from crate::tui::adapter_types::spinner

// (iter-143b already deleted the speech_mode system; caveman_prompt/rocky_prompt
// leaf functions were left behind as orphans and are now removed too.)

impl App {
    // -------------------------------------------------------------------
    // Main run loop
    // -------------------------------------------------------------------

    /// Run the TUI event loop. Returns `Some(input)` when the user submits
    /// a message, or `None` when the user quits.
    pub fn run<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<Option<String>>
    where
        B::Error: Send + Sync + 'static,
    {
        loop {
            if self.is_simulating && self.simulated_keys.is_empty() && !self.is_streaming {
                self.should_exit = true;
            }
            // Frame-cap guard: a headless scenario that never stops streaming
            // (e.g. a real agent that hangs) can't spin the loop forever.
            if self.is_simulating
                && let Some(max) = self.simulation_max_frames
                && self.frame_count >= max
            {
                self.should_exit = true;
            }
            if self.should_exit {
                self.debug_hub.dump_on_exit();
                return Ok(None);
            }
            self.frame_count = self.frame_count.wrapping_add(1);

            // Tick notifications so expired ones are removed. Without this,
            // notifications with a TTL stay visible forever (Bug #5 from
            // iter-82 audit).
            self.notifications.tick();

            // Drain background session-list results.
            if let Some(ref mut rx) = self.session_list_rx {
                match rx.try_recv() {
                    Ok(entries) => {
                        self.debug_hub
                            .publish(crate::tui::debug::TuiEvent::SessionList {
                                count: entries.len(),
                                at: crate::tui::debug::event_bus::now_secs(),
                            });
                        self.session_browser.sessions = entries;
                        self.session_browser.selected_idx = 0;
                        self.session_list_rx = None;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        self.session_list_rx = None;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
            }

            // Drain background model-fetch results.
            if let Some(ref mut rx) = self.model_fetch_rx {
                match rx.try_recv() {
                    Ok(Ok(models)) => {
                        self.debug_hub
                            .publish(crate::tui::debug::TuiEvent::ModelFetch {
                                ok: true,
                                count: models.len(),
                                at: crate::tui::debug::event_bus::now_secs(),
                            });
                        self.model_picker.set_models(models);
                        self.model_fetch_rx = None;
                        self.model_picker_fetch_pending = false;
                    }
                    Ok(Err(_)) => {
                        self.model_fetch_rx = None;
                        self.model_picker_fetch_pending = false;
                        self.status_message = Some(
                            "Failed to fetch models from provider (rate limit or auth error). Using cached models.".to_string()
                        );
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        self.model_fetch_rx = None;
                        self.model_picker_fetch_pending = false;
                        self.status_message =
                            Some("Model fetch task disconnected unexpectedly.".to_string());
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
            }

            // Spawn async session-list load when requested.
            if self.session_list_pending {
                self.session_list_pending = false;
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                self.session_list_rx = Some(rx);
                tokio::spawn(async move {
                    let sessions = crate::tui::adapter_types::history::list_sessions().await;
                    let entries: Vec<crate::tui::session_browser::SessionEntry> = sessions
                        .into_iter()
                        .map(|s| {
                            let age = chrono::Utc::now().signed_duration_since(s.updated_at);
                            let last_updated = if age.num_minutes() < 1 {
                                "just now".to_string()
                            } else if age.num_hours() < 1 {
                                format!("{}m ago", age.num_minutes())
                            } else if age.num_hours() < 24 {
                                format!("{}h ago", age.num_hours())
                            } else {
                                format!("{}d ago", age.num_days())
                            };
                            crate::tui::session_browser::SessionEntry {
                                id: s.id,
                                title: s.title.unwrap_or_else(|| "(untitled)".to_string()),
                                last_updated,
                                message_count: s.messages.len(),
                                cost_usd: s.total_cost,
                            }
                        })
                        .collect();
                    let _ = tx.send(entries).await;
                });
            }

            // Spawn async session-message load when /resume picks a session.
            // The background task calls history::load_session(id) and sends
            // the Vec<(role, content)> back via session_load_rx.
            if let Some(session_id) = self.session_load_pending.take() {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                self.session_load_rx = Some(rx);
                tokio::spawn(async move {
                    let msgs = crate::tui::adapter_types::history::load_session(session_id).await;
                    let _ = tx.send(msgs).await;
                });
            }

            // Drain background session-load results. Replace app.messages
            // with the loaded (role, content) pairs.
            if let Some(ref mut rx) = self.session_load_rx {
                match rx.try_recv() {
                    Ok(msgs) => {
                        self.messages.clear();
                        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                        for (role, content) in msgs {
                            let r = match role.as_str() {
                                "user" => Role::User,
                                "assistant" => Role::Assistant,
                                _ => Role::System,
                            };
                            self.messages.push(Message {
                                role: r,
                                content: MessageContent::Text(content),
                            });
                        }
                        self.invalidate_transcript();
                        self.debug_hub
                            .publish(crate::tui::debug::TuiEvent::SessionLoad {
                                session_id: self.session_title.clone().unwrap_or_default(),
                                msg_count: self.messages.len(),
                                at: crate::tui::debug::event_bus::now_secs(),
                            });
                        self.session_load_rx = None;
                        self.status_message = Some("Session loaded.".to_string());
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        self.session_load_rx = None;
                        self.status_message = Some("Session load failed.".to_string());
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
            }

            // Drain user-question requests from the clarify tool. The
            // clarify tool pushes a UserQuestionRequest (question + choices
            // + reply_tx) and blocks awaiting the reply. We open the
            // ask_user_dialog with the reply_tx — when the user confirms,
            // the dialog sends the answer via reply_tx, and the clarify
            // tool receives it and returns it as the tool result.
            // (iter-97 — closes Bug #2 from iter-82 audit. The sender side
            // is wired in TuiApp::run via set_user_question_sender.)
            if let Some(ref mut rx) = self.user_question_rx {
                // Only one dialog at a time — use if-let (not while-let) so
                // we process at most one event per frame and don't starve
                // the render loop.
                if let Ok(req) = rx.try_recv() {
                    self.debug_hub
                        .publish(crate::tui::debug::TuiEvent::UserQuestion {
                            question_preview: req.question.chars().take(40).collect(),
                            at: crate::tui::debug::event_bus::now_secs(),
                        });
                    // Open the ask_user_dialog with the real reply_tx.
                    // The dialog stores it and sends the user's answer when
                    // confirm() is called. If the user presses Esc, the
                    // dialog is dismissed and reply_tx is dropped — the
                    // clarify tool receives a RecvError and returns
                    // "[user dismissed the question]".
                    self.ask_user_dialog
                        .open(req.question, req.choices, req.reply_tx);
                }
            }

            // Drain voice transcription events (non-blocking).
            // When the background recording/transcription task emits a
            // TranscriptReady event we insert the text directly into the
            // prompt so the user can review and submit it.
            {
                use crate::tui::adapter_types::voice::VoiceEvent;
                let mut events = Vec::new();
                if let Some(ref mut rx) = self.voice_event_rx {
                    while let Ok(ev) = rx.try_recv() {
                        events.push(ev);
                    }
                }
                for ev in events {
                    self.debug_hub
                        .publish(crate::tui::debug::TuiEvent::VoiceEvent {
                            variant: format!("{ev:?}"),
                            at: crate::tui::debug::event_bus::now_secs(),
                        });
                    match ev {
                        VoiceEvent::RecordingStarted => {
                            self.voice_recording = true;
                            self.status_message =
                                Some("Recording\u{2026} (Alt+V or Esc to stop)".to_string());
                        }
                        VoiceEvent::RecordingStopped => {
                            self.voice_recording = false;
                            self.status_message = Some("Transcribing\u{2026}".to_string());
                        }
                        VoiceEvent::Transcription(text) => {
                            if !text.is_empty() {
                                if !self.prompt_input.text.is_empty()
                                    && !self.prompt_input.text.ends_with(' ')
                                {
                                    self.prompt_input.paste(" ");
                                }
                                self.prompt_input.paste(&text);
                                self.refresh_prompt_input();
                                self.status_message =
                                    Some(format!("Transcribed: {}", &text[..text.len().min(60)]));
                            }
                            self.voice_event_rx = None;
                        }
                        VoiceEvent::TranscriptReady(text) => {
                            if !text.is_empty() {
                                // Append to existing prompt text with a space separator
                                // so the user can combine voice + typed input.
                                if !self.prompt_input.text.is_empty()
                                    && !self.prompt_input.text.ends_with(' ')
                                {
                                    self.prompt_input.paste(" ");
                                }
                                self.prompt_input.paste(&text);
                                self.refresh_prompt_input();
                                self.status_message =
                                    Some(format!("Transcribed: {}", &text[..text.len().min(60)]));
                            }
                            // Clear the channel once we have the result.
                            self.voice_event_rx = None;
                        }
                        VoiceEvent::Error(msg) => {
                            self.voice_recording = false;
                            self.voice_event_rx = None;
                            self.push_notification(
                                NotificationKind::Warning,
                                format!("Voice: {}", msg),
                                Some(8),
                            );
                        }
                    }
                }
            }

            // Drain query events from the agent bridge task.
            {
                let mut events = Vec::new();
                if let Some(ref mut rx) = self.agent_event_rx {
                    while let Ok(ev) = rx.try_recv() {
                        events.push(ev);
                    }
                }
                for ev in events {
                    self.handle_agent_event(ev);
                }
            }

            // Drain pending tool permission requests from the agent. Each
            // request is converted into a `PermissionRequest` dialog and shown
            // to the user; the per-request `response_tx` is stashed in
            // `pending_permission_response_tx` so the user's choice can be
            // routed back when the dialog is dismissed. If a dialog is already
            // active (rare — the agent blocks on each request), the new
            // request is denied to avoid deadlock.
            {
                if let Some(ref mut rx) = self.permission_rx {
                    while let Ok(req) = rx.try_recv() {
                        self.debug_hub
                            .publish(crate::tui::debug::TuiEvent::PermissionRequest {
                                tool_name: req.tool_name.clone(),
                                at: crate::tui::debug::event_bus::now_secs(),
                            });
                        if self.permission_request.is_some() {
                            // A dialog is already shown — deny the new request
                            // so the agent doesn't block forever.
                            let _ = req
                                .response_tx
                                .send(operant_core::agent::ToolPermissionResponse::Deny);
                            continue;
                        }

                        // bash_prefix_allowlist: if the tool is bash/shell and
                        // the first word of the command is in the allowlist,
                        // auto-approve without showing a dialog. (Bug #21 from
                        // iter-82 audit — the allowlist was written but never
                        // consulted.) "Always allow" in the bypass-permissions
                        // dialog populates the allowlist via
                        // maybe_record_bash_prefix; this is the read side.
                        if (req.tool_name == "bash"
                            || req.tool_name == "shell"
                            || req.tool_name == "terminal")
                            && let Some(ref preview) = req.input_preview
                        {
                            let first_word = preview
                                .split_whitespace()
                                .next()
                                .map(|w| w.trim_start_matches("./").to_string())
                                .unwrap_or_default();
                            if !first_word.is_empty()
                                && self.bash_prefix_allowlist.contains(&first_word)
                            {
                                let _ = req.response_tx.send(
                                    operant_core::agent::ToolPermissionResponse::AllowSession,
                                );
                                continue;
                            }
                        }

                        let reason = if req.danger_explanation.is_empty() {
                            req.description.clone()
                        } else {
                            format!("{}\n{}", req.description, req.danger_explanation)
                        };
                        let dialog = crate::tui::dialogs::PermissionRequest::from_reason(
                            req.tool_id,
                            req.tool_name,
                            reason,
                            req.input_preview,
                        );
                        self.permission_request = Some(dialog);
                        self.pending_permission_response_tx = Some(req.response_tx);
                    }
                }
            }

            // Check if background agent.run() completed.
            if let Some(ref mut rx) = self.run_complete_rx {
                match rx.try_recv() {
                    Ok(result) => {
                        self.is_streaming = false;
                        if let Err(e) = result {
                            self.handle_agent_event(AgentEvent::Error {
                                error: e.user_message(),
                            });
                        }
                        self.run_complete_rx = None;
                        self.agent_task_handle.take(); // Clear completed handle.
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.is_streaming = false;
                        self.run_complete_rx = None;
                        self.agent_task_handle.take(); // Clear completed handle.
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Draw the frame, and immediately scan the *just-rendered*
            // buffer for URL runs. ratatui swaps its two buffers at the
            // end of draw(), so by the time draw() returns,
            // `terminal.current_buffer_mut()` points at the empty next-frame
            // slot. `CompletedFrame.buffer` is the one we actually want.
            let osc8_hits = {
                let draw_start = std::time::Instant::now();
                let completed = terminal.draw(|f| render::render_app(f, self))?;
                let render_ms = draw_start.elapsed().as_secs_f64() * 1000.0;
                self.debug_hub.record_frame(render_ms);
                crate::osc8::scan_buffer_for_urls(completed.buffer)
            };

            // Post-paint OSC 8 overlay: re-emit URL cells wrapped in
            // hyperlink escapes so terminals that support OSC 8 (Windows
            // Terminal, iTerm2, WezTerm, Kitty, Konsole, VS Code, …) make
            // them Ctrl/Cmd-clickable. Failure is non-fatal — we never want
            // an overlay glitch to kill the TUI.
            if let Err(err) = crate::osc8::emit_hits(&osc8_hits) {
                tracing::debug!(target: "osc8", "hyperlink overlay write failed: {err}");
            }

            // Replay a key that was saved by try_detect_paste_burst in a
            // previous iteration (e.g. a modifier key that terminated a burst).
            let pending = self.pending_key.take();
            let has_simulated = !self.simulated_keys.is_empty();

            // Poll for events with an adaptive timeout based on performance tier and
            // activity state. This reduces CPU usage by 5-10x on idle terminals.
            let got_event = pending.is_some() || has_simulated || {
                if self.is_simulating {
                    false
                } else {
                    let poll_timeout = crate::tui::redraw::redraw_interval(
                        self.perf_tier,
                        self.is_streaming,
                        Some(self.last_activity.elapsed()),
                        None, // use default 5s idle threshold
                        self.client_focused,
                    );
                    event::poll(poll_timeout)?
                }
            };

            if got_event {
                // Update activity timestamp for adaptive redraw cadence.
                self.last_activity = std::time::Instant::now();
                let event = if let Some(k) = pending {
                    Event::Key(k)
                } else if has_simulated {
                    Event::Key(self.simulated_keys.remove(0))
                } else {
                    event::read()?
                };
                match event {
                    Event::Key(key) => {
                        // On Windows crossterm fires both Press and Release events.
                        // We normally skip non-press events, but when voice PTT mode
                        // is active we need the Release event for the `V` key so we
                        // can stop recording as soon as the user lifts the key.
                        if key.kind != crossterm::event::KeyEventKind::Press {
                            // Handle V-key release to stop PTT recording.
                            if key.kind == crossterm::event::KeyEventKind::Release
                                && key.code == KeyCode::Char('v')
                                && key.modifiers == KeyModifiers::NONE
                                && self.voice_recording
                                && self.voice_recorder.is_some()
                            {
                                self.handle_voice_ptt_stop();
                            }
                            continue;
                        }

                        // ---- Paste-burst detection -----------------------------------------
                        // On Windows Terminal, Ctrl+V causes the terminal to write clipboard
                        // content as raw character events (not as Event::Paste).  Every `\n`
                        // fires as Enter (submitting the prompt) and stray `v` chars trigger
                        // voice PTT.  We detect this by draining the event queue with a
                        // zero-timeout immediately after the first character arrives — a paste
                        // dumps every character at once while normal typing rarely queues more
                        // than one char in the same 50 ms window.
                        if (key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT)
                            && let KeyCode::Char(c) = key.code
                        {
                            if self.prompt_is_accepting_text() {
                                if let Some(burst) = self.try_detect_paste_burst(c) {
                                    self.handle_paste_data(burst);
                                    self.refresh_prompt_input();
                                    continue;
                                }
                            } else if self.key_input_dialog.visible
                                && let Some(burst) = self.try_detect_paste_burst(c)
                            {
                                for ch in burst.chars() {
                                    self.key_input_dialog.insert_char(ch);
                                }
                                continue;
                            }
                        }
                        // -------------------------------------------------------------------

                        let should_submit = self.handle_key_event(key);
                        // Honour `:q`/`:wq` from vim command-line mode
                        if self.prompt_input.vim_quit_requested {
                            self.prompt_input.vim_quit_requested = false;
                            self.should_exit = true;
                        }
                        if self.should_exit {
                            self.debug_hub.dump_on_exit();
                            return Ok(None);
                        }
                        if should_submit {
                            self.dismiss_error_notifications();
                            let input = self.take_input();
                            if !input.is_empty() {
                                self.drop_pending_images_with_notice();
                                return Ok(Some(input));
                            }
                        }
                    }
                    Event::Paste(data)
                        if !self.is_streaming
                            && self.permission_request.is_none()
                            && !self.history_search_overlay.visible =>
                    {
                        if self.key_input_dialog.visible {
                            for ch in data.chars() {
                                self.key_input_dialog.insert_char(ch);
                            }
                        } else {
                            self.handle_paste_data(data);
                            self.refresh_prompt_input();
                        }
                    }
                    Event::Mouse(mouse_event) => {
                        self.handle_mouse_event(mouse_event);
                    }
                    Event::FocusGained => self.handle_focus_event(true),
                    Event::FocusLost => self.handle_focus_event(false),
                    _ => {}
                }
            }
        }
    }
}
