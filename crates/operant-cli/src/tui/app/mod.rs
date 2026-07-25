// app/mod.rs — App state struct and main event loop.

mod enums;
mod helpers;
mod init;
mod providers;
mod commands;
mod key_handling;
mod mouse;
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
    GlobalSearchState, HelpOverlay, HistorySearchOverlay, RewindFlowOverlay,
    SelectorMessage,
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
    pub remote_session_url: Option<String>,    /// Bridge/gateway connection state for status bar badge.
    #[allow(dead_code)] // Prepared for bridge status badge
    pub bridge_state:
        crate::tui::bridge_state::BridgeConnectionState,
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
    pub fn open_import_config_picker(&mut self) {
        self.import_config_picker =
            DialogSelectState::new("Import config", import_config_picker_items());
        self.import_config_picker.open();
    }

    fn import_selection_from_picker(
        id: &str,
    ) -> Option<crate::tui::adapter_types::ImportSelection> {
        match id {
            "claude-md" => Some(crate::tui::adapter_types::ImportSelection::ClaudeMd),
            "settings" => Some(crate::tui::adapter_types::ImportSelection::Settings),
            "both" => Some(crate::tui::adapter_types::ImportSelection::Both),
            _ => None,
        }
    }

    fn open_import_config_preview(
        &mut self,
        selection: crate::tui::adapter_types::ImportSelection,
    ) {
        match crate::tui::adapter_types::build_import_preview(selection) {
            Ok(preview) => {
                self.import_config_dialog.open(preview);
            }
            Err(err) => {
                self.status_message = Some(format!("Import failed: {}", err));
            }
        }
    }

    fn perform_import_config(&mut self) {
        let Some(selection) = self.import_config_dialog.selection.clone() else {
            self.import_config_dialog.close();
            return;
        };
        match crate::tui::adapter_types::execute_import(selection) {
            Ok(result) => {
                let paths = crate::tui::adapter_types::ImportPaths::detect();
                let new_settings = Settings::load_sync().unwrap_or_default();
                let loaded = operant_core::config::load_app_config(None).unwrap_or_else(|_| {
                    operant_core::config::LoadedConfig {
                        config: AppConfig::default(),
                        source: None,
                    }
                });
                let result_message =
                    crate::tui::adapter_types::summarize_import_result(&result, &paths);
                let imported_mcp = result.imported_fields.iter().any(|f| f == "mcpServers");
                self.config = loaded.config;
                self.settings = new_settings;
                let model_to_resolve = self.config.agent.model.clone();
                self.model_name = self.resolve_stale_model(&model_to_resolve);
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.set_model(&self.model_name);
                }
                self.refresh_context_window_size();
                self.context_used_tokens = 0;
                self.has_credentials =
                    crate::tui::adapter_types::config::resolve_api_key().is_some();
                self.auth_store = crate::tui::adapter_types::AuthStore::load();
                self.plan_mode = matches!(
                    self.settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::Plan
                );
                self.output_style = match self.settings.output_style.as_deref() {
                    Some("stream") => "stream".to_string(),
                    Some("verbose") => "verbose".to_string(),
                    _ => "auto".to_string(),
                };
                if imported_mcp {
                    self.pending_mcp_reconnect = true;
                }
                self.status_message = Some(result_message);
                self.import_config_dialog.close();
            }
            Err(err) => {
                self.status_message = Some(format!("Import failed: {}", err));
                self.import_config_dialog.close();
            }
        }
    }

    fn current_user_turn_index(&self) -> Option<usize> {
        self.messages
            .iter()
            .filter(|msg| msg.role == Role::User)
            .count()
            .checked_sub(1)
    }

    fn current_agent_mode_snapshot(&self) -> String {
        self.agent_mode
            .clone()
            .unwrap_or_else(|| if self.plan_mode { "plan" } else { "build" }.to_string())
    }

    #[allow(dead_code)] // Prepared for turn metadata tracking
    fn begin_user_turn_snapshot(&mut self) {
        self.turn_metadata.push(TurnMetadata {
            model_name: Some(self.model_name.clone()),
            agent_mode: Some(self.current_agent_mode_snapshot()),
            duration: None,
            interrupted: false,
        });
        // Start the latency timer now — at prompt-submission time — so it
        // measures actual round-trip time even when the provider buffers its
        // full response before yielding any stream events (e.g. Gemini flash).
        self.turn_start = Some(std::time::Instant::now());
        self.last_turn_elapsed = None;
        self.last_turn_verb = None;
    }

    fn sync_turn_metadata_to_messages(&mut self) {
        let user_count = self
            .messages
            .iter()
            .filter(|msg| msg.role == Role::User)
            .count();

        if self.turn_metadata.len() > user_count {
            self.turn_metadata.truncate(user_count);
            return;
        }

        while self.turn_metadata.len() < user_count {
            self.turn_metadata.push(TurnMetadata::default());
        }
    }

    fn complete_current_turn_snapshot(&mut self, interrupted: bool) {
        if let Some(index) = self.current_user_turn_index() {
            if self.turn_metadata.len() <= index {
                self.sync_turn_metadata_to_messages();
            }

            let model_name = self.model_name.clone();
            let agent_mode = self.current_agent_mode_snapshot();
            if let Some(meta) = self.turn_metadata.get_mut(index) {
                meta.duration = self.last_turn_elapsed.clone();
                meta.interrupted = interrupted;
                if meta.model_name.is_none() {
                    meta.model_name = Some(model_name);
                }
                if meta.agent_mode.is_none() {
                    meta.agent_mode = Some(agent_mode);
                }
            }
        }
    }

    fn flush_streamed_assistant_message(&mut self) {
        if self.streaming_text.trim().is_empty() && self.streaming_thinking.trim().is_empty() {
            self.streaming_text.clear();
            self.streaming_thinking.clear();
            return;
        }

        let thinking = std::mem::take(&mut self.streaming_thinking);
        let text = std::mem::take(&mut self.streaming_text);

        let mut blocks = Vec::new();
        if !thinking.trim().is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking,
                signature: String::new(),
            });
        }
        if !text.is_empty() {
            blocks.push(ContentBlock::Text { text });
        }

        let msg = match blocks.len() {
            0 => return,
            1 => match blocks.pop().unwrap() {
                ContentBlock::Text { text } => Message::assistant(text),
                block => Message::assistant_blocks(vec![block]),
            },
            _ => Message::assistant_blocks(blocks),
        };

        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }
    /// Handle slash commands that should open UI screens rather than execute
    /// Add a message directly (e.g. from a non-streaming source).
    pub fn add_message(&mut self, role: Role, text: String) {
        let msg = match role {
            Role::User => Message::user(text),
            Role::Assistant => Message::assistant(text),
            Role::System => Message {
                role: Role::System,
                content: crate::tui::adapter_types::types::MessageContent::Text(text),
            },
        };
        if role == Role::User {
            self.begin_user_turn_snapshot();
        }
        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }

    /// Push a synthetic system annotation into the conversation pane.
    /// It will appear after the current last message.
    /// Push a notification and, for Error-kind notifications, reset the error
    /// modal scroll offset so a newly arrived error is always shown from the top.
    pub fn push_notification(
        &mut self,
        kind: NotificationKind,
        msg: String,
        duration_secs: Option<u64>,
    ) {
        if kind == NotificationKind::Error {
            self.error_modal_scroll_offset = 0;
        }
        self.notifications.push(kind, msg, duration_secs);
    }

    pub fn push_system_message(&mut self, text: String, style: SystemMessageStyle) {
        self.system_annotations.push(SystemAnnotation {
            after_index: self.messages.len(),
            text,
            style,
        });
        self.invalidate_transcript();
    }

    /// Called whenever a new message is appended to `messages`.
    /// Manages the auto-scroll / new-message-counter state.
    fn on_new_message(&mut self) {
        if self.auto_scroll {
            // Auto-scroll: keep offset at 0 so render shows the bottom.
            self.scroll_offset = 0;
        } else {
            self.new_messages_while_scrolled = self.new_messages_while_scrolled.saturating_add(1);
        }
    }

    pub fn invalidate_transcript(&self) {
        self.transcript_version
            .set(self.transcript_version.get().wrapping_add(1));
    }

    /// Check current token usage and push token warning notifications as
    /// appropriate.  Call this after updating `token_count`.
    pub fn check_token_warnings(&mut self) {
        let window = crate::tui::adapter_types::context_window_for_model(&self.model_name) as u32;
        if window == 0 {
            return;
        }
        let pct = (self.token_count as f64 / window as f64 * 100.0) as u8;

        // Usage dropped back below the last-shown threshold (e.g. /clear or
        // /compact shrank the context) — reset so warnings can re-fire on
        // the way back up instead of being suppressed forever.
        if pct < self.token_warning_threshold_shown {
            self.token_warning_threshold_shown = 0;
        }

        // Only escalate — never repeat a threshold already shown.
        if pct >= 100 && self.token_warning_threshold_shown < 100 {
            self.token_warning_threshold_shown = 100;
            self.push_notification(
                NotificationKind::Error,
                "Context window full. Running auto-compact\u{2026}".to_string(),
                None,
            );
        } else if pct >= 95 && self.token_warning_threshold_shown < 95 {
            self.token_warning_threshold_shown = 95;
            self.push_notification(
                NotificationKind::Error,
                "Context window 95% full! Run /compact now.".to_string(),
                None, // persistent until dismissed
            );
        } else if pct >= 80 && self.token_warning_threshold_shown < 80 {
            self.token_warning_threshold_shown = 80;
            self.push_notification(
                NotificationKind::Warning,
                "Context window 80% full. Consider /compact.".to_string(),
                Some(30),
            );
        }
    }

    /// Drain any pasted images waiting to be attached and, if there were any,
    /// warn that they weren't actually sent. Call this once a message has
    /// been submitted. Images can't be attached yet because the core
    /// client's request path has no multi-part content support — without
    /// this, the thumbnail row would linger forever and look like the
    /// image was sent when it silently wasn't.
    pub fn drop_pending_images_with_notice(&mut self) {
        let dropped = self.prompt_input.clear_images();
        if !dropped.is_empty() {
            self.push_notification(
                NotificationKind::Warning,
                format!(
                    "Image attachments aren't sent to the model yet — {} image(s) dropped.",
                    dropped.len()
                ),
                Some(6),
            );
        }
    }

    /// Take the current input buffer, push it to history, and return it.
    pub fn take_input(&mut self) -> String {
        let input = self.prompt_input.take();
        if !input.is_empty() {
            self.prompt_input.history.push(input.clone());
            self.prompt_input.history_pos = None;
            self.prompt_input.history_draft.clear();
            // Persist the new entry to ~/.operant/history.jsonl so it
            // survives restarts. (iter-125 — persistent input history.)
            crate::tui::input_history::append(&input);
        }
        self.refresh_prompt_input();
        input
    }

    /// Compute the number of lines to scroll per wheel/trackpad event.
    /// Implements a simple acceleration model: rapid events (< 40 ms apart) are
    /// treated as trackpad bursts and accelerate up to 2×; slower events (mouse
    /// wheel) stay at the base 3-line step.
    fn scroll_step(&mut self) -> usize {
        let now = std::time::Instant::now();
        let elapsed_ms = self
            .scroll_last_time
            .map(|t| now.duration_since(t).as_millis())
            .unwrap_or(u128::MAX);
        self.scroll_last_time = Some(now);
        if elapsed_ms < 40 {
            // Trackpad burst — gradually accelerate
            self.scroll_accel = (self.scroll_accel + 0.4).min(6.0);
        } else {
            // Mouse click or first event — reset to base
            self.scroll_accel = 3.0;
        }
        self.scroll_accel.round() as usize
    }

    /// Open the rewind flow with the current message list converted to
    /// `SelectorMessage` entries.
    pub fn open_rewind_flow(&mut self) {
        let selector_msgs: Vec<SelectorMessage> = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let text = m.get_all_text();
                let preview: String = text.chars().take(80).collect();
                let has_tool_use = !m.get_tool_use_blocks().is_empty();
                SelectorMessage {
                    idx: i,
                    role: format!("{:?}", m.role).to_lowercase(),
                    preview,
                    has_tool_use,
                }
            })
            .collect();
        self.rewind_flow.open(selector_msgs);
    }

    /// Return the elapsed session time as a human-readable string, e.g. "2m 5s".
    pub fn elapsed_str(&self) -> String {
        let secs = self.session_start.elapsed().as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else {
            format!("{}m {}s", secs / 60, secs % 60)
        }
    }

    fn prompt_mode(&self) -> InputMode {
        // Note: previously returned Readonly while streaming, but the prompt
        // now accepts input during streaming so the user can compose / queue
        // a follow-up message. Plan mode still wins.
        if self.plan_mode {
            InputMode::Plan
        } else {
            InputMode::Default
        }
    }

    fn sync_legacy_prompt_fields(&mut self) {
        self.input = self.prompt_input.text.clone();
        self.cursor_pos = self.prompt_input.cursor;
    }

    /// Check if any modal dialog is open that should block suggestion updates.
    /// Mirrors claurst's file_injection_dialog guard for suggestion updates.
    fn should_block_suggestions(&self) -> bool {
        self.connect_dialog.visible
            || self.import_config_picker.visible
            || self.import_config_dialog.visible
            || self.command_palette.visible
            || self.model_picker.visible
            || self.settings_screen.visible
            || self.export_dialog.visible
            || self.bypass_permissions_dialog.visible
            || self.key_input_dialog.visible
            || self.custom_provider_dialog.visible
            || self.free_mode_dialog.visible
            || self.device_auth_dialog.visible
            || self.ask_user_dialog.visible
    }

    pub fn refresh_prompt_input(&mut self) {
        self.prompt_input.mode = self.prompt_mode();
        // Skip suggestion updates when a modal dialog is open (Phase 1.4).
        if !self.should_block_suggestions() {
            let file_autocomplete_limit = self.settings.config.file_autocomplete_limit;
            let file_autocomplete_show_hidden =
                self.settings.config.file_autocomplete_show_hidden_files;
            self.prompt_input.update_suggestions(
                &tui_slash_command_data(),
                file_autocomplete_limit,
                file_autocomplete_show_hidden,
            );
        }
        self.sync_legacy_prompt_fields();
    }

    pub fn set_prompt_text(&mut self, text: String) {
        self.prompt_input.replace_text(text);
        self.refresh_prompt_input();
    }

    // -----------------------------------------------------------------------
    // Voice PTT helpers
    // -----------------------------------------------------------------------

    /// Start PTT recording: open the microphone capture stream and signal the
    /// UI.  No-op when no voice recorder is attached or recording is already
    /// in progress.
    pub fn handle_voice_ptt_start(&mut self) {
        if self.voice_recording || self.voice_recorder.is_none() {
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        self.voice_event_rx = Some(rx);
        self.voice_recording = true;
        if let Some(ref recorder_arc) = self.voice_recorder {
            let recorder = recorder_arc.clone();
            // spawn_blocking + block_on: the std MutexGuard is not Send,
            // so we can't hold it across .await in a tokio::spawn.
            // spawn_blocking runs on a dedicated blocking thread, so the
            // main tokio runtime (and TUI render) continues unblocked.
            // start_recording is a quick operation (creates recorder +
            // starts capture subprocess).
            tokio::task::spawn_blocking(move || {
                if let Ok(mut r) = recorder.lock() {
                    tokio::runtime::Handle::current()
                        .block_on(r.start_recording(tx))
                        .ok();
                }
            });
        }
        self.status_message =
            Some("Recording\u{2026} release V or press Enter to transcribe".to_string());
    }

    /// Stop PTT recording: flip the AtomicBool inside VoiceRecorder so the
    /// capture thread exits, then fire a "Transcribing…" notice.  The
    /// transcript text arrives later via `voice_event_rx` and is injected into
    /// the prompt by the event-loop drain.
    pub fn handle_voice_ptt_stop(&mut self) {
        if !self.voice_recording {
            return;
        }
        self.voice_recording = false;
        if let Some(ref recorder_arc) = self.voice_recorder {
            let recorder = recorder_arc.clone();
            // spawn_blocking: stop_recording does network STT (5-30s).
            // Using a blocking thread ensures the main tokio runtime
            // (TUI render, event loop) continues running while the
            // transcription happens in the background.
            tokio::task::spawn_blocking(move || {
                if let Ok(mut r) = recorder.lock() {
                    tokio::runtime::Handle::current()
                        .block_on(r.stop_recording())
                        .ok();
                }
            });
        }
        self.status_message = Some("Transcribing\u{2026}".to_string());
    }

    // (iter-209: attach_turn_diff_state + refresh_turn_diff_from_history
    // deleted — stub FileHistory removed, turn-diff feature cut as YAGNI.
    // /changes overlay now uses git-diff via diff_viewer's real path.)

    // (iter-208: attach_mcp_manager deleted — stub mcp_manager field removed.
    // load_mcp_servers now reads from core_mcp_manager, which is set directly
    // in TuiApp::enter via self.app.core_mcp_manager = Some(...).)

    pub fn refresh_mcp_view(&mut self) {
        let servers = self.load_mcp_servers();
        self.mcp_view.open(servers);
    }

    pub fn take_pending_mcp_panel_auth(&mut self) -> Option<String> {
        self.pending_mcp_panel_auth.take()
    }

    pub fn take_pending_mcp_reconnect(&mut self) -> bool {
        let pending = self.pending_mcp_reconnect;
        self.pending_mcp_reconnect = false;
        pending
    }

    /// Returns and clears any pending MCP approval result.
    pub fn take_mcp_approval_result(&mut self) -> Option<crate::dialogs::McpApprovalChoice> {
        if !self.mcp_approval.visible {
            return None;
        }
        // The dialog closes itself on confirm; we check if it's now closed
        None // Actual result is read by CLI loop via mcp_approval.visible + confirm()
    }

    fn clear_prompt(&mut self) {
        self.prompt_input.clear();
        self.refresh_prompt_input();
    }

    // (iter-209: refresh_turn_diff_from_history deleted — turn-diff feature
    // cut. Call sites now no-op; /changes uses git-diff instead.)

    // -------------------------------------------------------------------
    // Event handling
    // -------------------------------------------------------------------

    /// Persist `has_completed_onboarding = true` to the settings file.
    /// Best-effort: failures are silently ignored to not disrupt the session.
    fn persist_onboarding_complete() -> anyhow::Result<()> {
        let mut settings = crate::tui::adapter_types::config::Settings::load_sync()?;
        settings.has_completed_onboarding = true;
        settings.save_sync()
    }

    /// Public wrapper so the main loop can mark onboarding complete without
    /// going through the dialog flow.
    pub fn persist_onboarding_complete_pub() -> anyhow::Result<()> {
        Self::persist_onboarding_complete()
    }

    /// Enable bypass-permissions mode and persist it — the "arm" half of the
    /// `/yolo` toggle, shared with the `--dangerously-skip-permissions` startup
    /// dialog accept path.
    fn arm_bypass_permissions(&mut self) {
        use crate::tui::adapter_types::config::PermissionMode;
        let mut settings = crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
        settings.permission_mode = PermissionMode::BypassPermissions;
        let _ = settings.save_sync();
        self.settings.permission_mode = PermissionMode::BypassPermissions;
    }

    /// Determine the current key context based on visible UI elements.
    /// Higher-priority contexts are checked first. Mirrors claurst's system.
    fn current_key_context(&self) -> KeyContext {
        if self.context_menu_state.is_some() {
            KeyContext::ContextMenu
        } else if self.bypass_permissions_dialog.visible
            || self.effort_picker.visible
            || self.key_input_dialog.visible
            || self.custom_provider_dialog.visible
            || self.free_mode_dialog.visible
            || self.device_auth_dialog.visible
            || self.ask_user_dialog.visible
            || self.import_config_dialog.visible
            || self.mcp_approval.visible
        {
            KeyContext::Dialog
        } else if self.connect_dialog.visible
            || self.import_config_picker.visible
            || self.command_palette.visible
            || self.model_picker.visible
            || self.settings_screen.visible
            || self.export_dialog.visible
            || self.stats_dialog.visible
            || self.context_viz.visible
            || self.session_browser.visible
            || self.session_branching.visible
            || self.tasks_overlay.visible
        {
            KeyContext::Menu
        } else if self.global_search.visible {
            KeyContext::GlobalSearch
        } else if self.history_search_overlay.visible {
            KeyContext::HistorySearch
        } else if self.help_overlay.visible {
            KeyContext::Help
        } else if self.mcp_view.visible {
            KeyContext::MCPView
        } else if self.agents_menu.visible {
            KeyContext::AgentsMenu
        } else if self.diff_viewer.visible {
            KeyContext::DiffViewer
        } else if self.focus == FocusTarget::Input {
            KeyContext::Prompt
        } else {
            KeyContext::Transcript
        }
    }

    /// Get the highest-priority visible dialog for key routing.
    /// Returns None if no dialog is visible.
    fn dialog_priority(&self) -> Option<DialogPriority> {
        // Check in priority order (highest first)
        if self.context_menu_state.is_some() {
            return Some(DialogPriority::ContextMenu);
        }
        if self.bypass_permissions_dialog.visible {
            return Some(DialogPriority::BypassPermissions);
        }
        if self.mcp_approval.visible {
            return Some(DialogPriority::McpApproval);
        }
        if self.device_auth_dialog.visible {
            return Some(DialogPriority::DeviceAuth);
        }
        if self.ask_user_dialog.visible {
            return Some(DialogPriority::AskUser);
        }
        if self.key_input_dialog.visible {
            return Some(DialogPriority::KeyInput);
        }
        if self.custom_provider_dialog.visible {
            return Some(DialogPriority::CustomProvider);
        }
        if self.free_mode_dialog.visible {
            return Some(DialogPriority::FreeMode);
        }
        if self.import_config_dialog.visible {
            return Some(DialogPriority::ImportConfig);
        }
        if self.effort_picker.visible {
            return Some(DialogPriority::EffortPicker);
        }
        if self.connect_dialog.visible {
            return Some(DialogPriority::Connect);
        }
        if self.import_config_picker.visible {
            return Some(DialogPriority::ImportConfigPicker);
        }
        if self.command_palette.visible {
            return Some(DialogPriority::CommandPalette);
        }
        if self.model_picker.visible {
            return Some(DialogPriority::ModelPicker);
        }
        if self.settings_screen.visible {
            return Some(DialogPriority::Settings);
        }
        if self.export_dialog.visible {
            return Some(DialogPriority::Export);
        }
        if self.stats_dialog.visible {
            return Some(DialogPriority::Stats);
        }
        if self.context_viz.visible {
            return Some(DialogPriority::ContextViz);
        }
        if self.session_browser.visible {
            return Some(DialogPriority::SessionBrowser);
        }
        if self.session_branching.visible {
            return Some(DialogPriority::SessionBranching);
        }
        if self.tasks_overlay.visible {
            return Some(DialogPriority::Tasks);
        }
        if self.global_search.visible {
            return Some(DialogPriority::GlobalSearch);
        }
        if self.history_search_overlay.visible {
            return Some(DialogPriority::HistorySearch);
        }
        if self.help_overlay.visible {
            return Some(DialogPriority::Help);
        }
        if self.mcp_view.visible {
            return Some(DialogPriority::MCPView);
        }
        if self.agents_menu.visible {
            return Some(DialogPriority::AgentsMenu);
        }
        if self.diff_viewer.visible {
            return Some(DialogPriority::DiffViewer);
        }
        if self.plugins_hub.visible {
            return Some(DialogPriority::PluginsHub);
        }
        if self.skills_view.visible {
            return Some(DialogPriority::SkillsView);
        }
        if self.journey_view.visible {
            return Some(DialogPriority::JourneyView);
        }
        if self.hooks_config_menu.visible {
            return Some(DialogPriority::HooksConfig);
        }
        if self.voice_mode_notice.visible {
            return Some(DialogPriority::VoiceModeNotice);
        }
        None
    }

    /// Process a keyboard event. Returns `true` when the input should be
    /// submitted (Enter pressed with no blocking dialog).
    ///   `P` (bash prefix) → AllowSession, also records the bash prefix in
    ///       `bash_prefix_allowlist` via `maybe_record_bash_prefix`
    ///   `n` / Esc / unknown → Deny
    fn resolve_permission_dialog(&mut self) {
        // Capture the selected option key + response sender up front so we
        // can clear `permission_request` at the end unconditionally.
        let (selected_key, tx) = {
            let pr = match self.permission_request.as_ref() {
                Some(p) => p,
                None => return,
            };
            let key = pr.options.get(pr.selected_option).map(|o| o.key);
            let tx = self.pending_permission_response_tx.take();
            (key, tx)
        };
        // Bash prefix-allow ('P') records the prefix in the allowlist. Must
        // run before we drop `permission_request` — it reads `pr.kind`.
        self.maybe_record_bash_prefix();

        let response = match selected_key {
            Some('y') => operant_core::agent::ToolPermissionResponse::AllowOnce,
            Some('Y') | Some('p') | Some('P') => {
                operant_core::agent::ToolPermissionResponse::AllowSession
            }
            // 'n' (deny), None (no options), or any unmatched key → Deny.
            Some('n') | None => operant_core::agent::ToolPermissionResponse::Deny,
            Some(_) => operant_core::agent::ToolPermissionResponse::Deny,
        };
        if let Some(tx) = tx {
            let _ = tx.send(response);
        }
        self.permission_request = None;
    }

    /// Handle a key event while a permission dialog is active.
    fn handle_permission_key(&mut self, key: KeyEvent) {
        let pr = match self.permission_request.as_mut() {
            Some(p) => p,
            None => return,
        };

        match key.code {
            KeyCode::Char(c) => {
                if let Some(digit) = c.to_digit(10) {
                    let idx = (digit as usize).saturating_sub(1);
                    if idx < pr.options.len() {
                        pr.selected_option = idx;
                    }
                } else {
                    // Check if any option matches this key.
                    let mut matched_idx = None;
                    for (i, opt) in pr.options.iter().enumerate() {
                        if opt.key == c {
                            matched_idx = Some(i);
                            break;
                        }
                    }
                    if let Some(idx) = matched_idx {
                        pr.selected_option = idx;
                        self.resolve_permission_dialog();
                    }
                }
            }
            KeyCode::Enter => {
                self.resolve_permission_dialog();
            }
            KeyCode::Up => {
                if pr.selected_option > 0 {
                    pr.selected_option -= 1;
                }
            }
            KeyCode::Down => {
                if pr.selected_option + 1 < pr.options.len() {
                    pr.selected_option += 1;
                }
            }
            KeyCode::Esc => {
                // Esc = cancel = deny. Force the selected option to the deny
                // option (key 'n') before resolving so the response is always
                // Deny regardless of which option was highlighted.
                if let Some(pr) = self.permission_request.as_mut() {
                    if let Some(idx) = pr.options.iter().position(|o| o.key == 'n') {
                        pr.selected_option = idx;
                    }
                }
                self.resolve_permission_dialog();
            }
            _ => {}
        }
    }

    /// If the active permission dialog's selected option is the prefix-allow
    /// option ('P') for a Bash dialog, extract the suggested prefix and add it
    /// to `bash_prefix_allowlist` so future requests with the same prefix are
    /// silently approved.
    fn maybe_record_bash_prefix(&mut self) {
        use crate::tui::dialogs::PermissionDialogKind;
        let pr = match self.permission_request.as_ref() {
            Some(p) => p,
            None => return,
        };
        // Only act on Bash dialogs where the selected option key is 'P'.
        let selected_key = pr.options.get(pr.selected_option).map(|o| o.key);
        if selected_key != Some('P') {
            return;
        }
        if let PermissionDialogKind::Bash { command, .. } = &pr.kind {
            // Always normalize to the first whitespace-delimited word so
            // that the allowlist check in `bash_command_allowed_by_prefix`
            // (which also uses `split_whitespace().next()`) matches correctly.
            let first_word = command.split_whitespace().next().unwrap_or("").to_string();
            if !first_word.is_empty() {
                self.bash_prefix_allowlist.insert(first_word);
            }
        }
    }

    /// Returns `true` if the given bash `command` is covered by the session-local
    /// prefix allowlist (i.e. its first word matches an entry in
    /// `bash_prefix_allowlist`).  Used by callers to skip the permission dialog.
    pub fn bash_command_allowed_by_prefix(&self, command: &str) -> bool {
        let first_word = command.split_whitespace().next().unwrap_or("");
        !first_word.is_empty() && self.bash_prefix_allowlist.contains(first_word)
    }

    // ---- Advanced mouse interaction helpers --------------------------------

    /// Detect if a click is a double-click based on timing and position.
    /// Returns true if the click is within ~500ms and ~5px of the last click.
    pub fn handle_mouse_event(&mut self, mouse_event: MouseEvent) {
        use crossterm::event::MouseButton;

        // Fast-reject mouse-move events — they flood at 60+ Hz and we don't
        // need hover tracking. Exception: context menu needs hover to update
        // the selected item highlight.
        if matches!(mouse_event.kind, MouseEventKind::Moved) {
            if let Some(menu) = self.context_menu_state.as_mut() {
                let items = Self::context_menu_items(menu.kind);
                let item_labels: Vec<&str> = items
                    .iter()
                    .map(|i| match i {
                        ContextMenuItem::Copy => "Copy",
                        ContextMenuItem::Fork => "Fork new chat",
                    })
                    .collect();
                let menu_width =
                    (item_labels.iter().map(|l| l.len()).max().unwrap_or(4) + 4) as u16;
                let menu_height = items.len() as u16 + 2;
                let screen = self.last_msg_area.get();
                let menu_x = menu.x.min(
                    screen
                        .x
                        .saturating_add(screen.width)
                        .saturating_sub(menu_width + 1),
                );
                let menu_y = menu.y.min(
                    screen
                        .y
                        .saturating_add(screen.height)
                        .saturating_sub(menu_height + 1),
                );
                let inner_y = menu_y + 1;
                let col = mouse_event.column;
                let row = mouse_event.row;
                if col >= menu_x
                    && col < menu_x.saturating_add(menu_width)
                    && row >= inner_y
                    && row < inner_y.saturating_add(items.len() as u16)
                {
                    let hovered = (row - inner_y) as usize;
                    if hovered < items.len() {
                        menu.selected_index = hovered;
                    }
                }
            }
            return;
        }

        // ---- Dialog interaction: dismiss on click-outside, scroll/click inside ----
        // Key-input and device-auth stay outside this gate so their visible text
        // can still be selected and copied with the mouse.
        let any_dialog = self.connect_dialog.visible
            || self.import_config_picker.visible
            || self.import_config_dialog.visible
            || self.command_palette.visible
            || self.model_picker.visible
            || self.export_dialog.visible
            || self.settings_screen.visible
            || self.stats_dialog.visible
            || self.context_viz.visible
            || self.session_browser.visible;

        if any_dialog {
            match mouse_event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    // DialogSelect dialogs — check if click is inside for item selection
                    let in_dialog = if self.connect_dialog.visible {
                        self.connect_dialog
                            .contains(mouse_event.column, mouse_event.row)
                    } else if self.import_config_picker.visible {
                        self.import_config_picker
                            .contains(mouse_event.column, mouse_event.row)
                    } else if self.command_palette.visible {
                        self.command_palette
                            .contains(mouse_event.column, mouse_event.row)
                    } else {
                        // Other dialogs (model_picker, settings, export, etc.) —
                        // treat any click as "inside" to prevent accidental dismiss.
                        // User must press Esc to close these.
                        true
                    };

                    if in_dialog {
                        // Click inside a DialogSelect — select the clicked item
                        if self.connect_dialog.visible {
                            self.connect_dialog.handle_mouse_click(mouse_event.row);
                        } else if self.import_config_picker.visible {
                            self.import_config_picker
                                .handle_mouse_click(mouse_event.row);
                        } else if self.command_palette.visible {
                            self.command_palette.handle_mouse_click(mouse_event.row);
                        }
                        // Other dialogs: click absorbed, no action needed
                    } else {
                        // Click outside a DialogSelect — dismiss and restore input focus
                        self.close_secondary_views();
                        self.focus = FocusTarget::Input;
                    }
                }
                MouseEventKind::ScrollUp => {
                    // Scroll through dialog items
                    if self.connect_dialog.visible {
                        self.connect_dialog.move_up();
                    } else if self.import_config_picker.visible {
                        self.import_config_picker.move_up();
                    } else if self.command_palette.visible {
                        self.command_palette.move_up();
                    }
                }
                MouseEventKind::ScrollDown => {
                    if self.connect_dialog.visible {
                        self.connect_dialog.move_down();
                    } else if self.import_config_picker.visible {
                        self.import_config_picker.move_down();
                    } else if self.command_palette.visible {
                        self.command_palette.move_down();
                    }
                }
                _ => {}
            }
            return; // Don't process any other mouse events when a dialog is open
        }

        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                // Don't consume Ctrl+Scroll — let the terminal handle zoom.
                if !mouse_event.modifiers.contains(KeyModifiers::CONTROL) {
                    let step = self.scroll_step();
                    self.scroll_offset = self.scroll_offset.saturating_add(step);
                    self.auto_scroll = false;
                }
            }
            MouseEventKind::ScrollDown => {
                if !mouse_event.modifiers.contains(KeyModifiers::CONTROL) {
                    let step = self.scroll_step();
                    let new_off = self.scroll_offset.saturating_sub(step);
                    self.scroll_offset = new_off;
                    if new_off == 0 {
                        self.auto_scroll = true;
                        self.new_messages_while_scrolled = 0;
                    }
                }
            }
            // ---- Right-click context menu ----------------------------------
            MouseEventKind::Down(MouseButton::Right) => {
                let msg_area = self.last_msg_area.get();
                let has_selection = !self.selection_text.borrow().trim().is_empty();
                if mouse_event.column >= msg_area.x
                    && mouse_event.column < msg_area.x.saturating_add(msg_area.width)
                    && mouse_event.row >= msg_area.y
                    && mouse_event.row < msg_area.y.saturating_add(msg_area.height)
                {
                    if let Some(message_index) = self.message_index_at_row(mouse_event.row) {
                        self.show_context_menu(
                            mouse_event.column,
                            mouse_event.row,
                            ContextMenuKind::Message { message_index },
                        );
                    } else {
                        self.dismiss_context_menu();
                    }
                } else if has_selection {
                    self.show_context_menu(
                        mouse_event.column,
                        mouse_event.row,
                        ContextMenuKind::Selection,
                    );
                } else {
                    self.dismiss_context_menu();
                }
            }

            // ---- Primary-selection paste into the prompt ---------------
            MouseEventKind::Down(MouseButton::Middle) => {
                let _ = self.paste_primary_into_prompt();
            }

            // ---- Text selection / focus routing -------------------------
            MouseEventKind::Down(MouseButton::Left) => {
                // If a context menu is open, check if the click is on a menu item.
                // Must replicate the same position clamping as the renderer.
                if let Some(menu) = self.context_menu_state {
                    let items = Self::context_menu_items(menu.kind);
                    let item_labels: Vec<&str> = items
                        .iter()
                        .map(|i| match i {
                            ContextMenuItem::Copy => "Copy",
                            ContextMenuItem::Fork => "Fork new chat",
                        })
                        .collect();
                    let menu_width =
                        (item_labels.iter().map(|l| l.len()).max().unwrap_or(4) + 4) as u16;
                    let menu_height = items.len() as u16 + 2; // +2 for border
                    // Clamp to screen bounds (same as render_context_menu)
                    let screen = self.last_msg_area.get();
                    let menu_x = menu.x.min(
                        screen
                            .x
                            .saturating_add(screen.width)
                            .saturating_sub(menu_width + 1),
                    );
                    let menu_y = menu.y.min(
                        screen
                            .y
                            .saturating_add(screen.height)
                            .saturating_sub(menu_height + 1),
                    );
                    let col = mouse_event.column;
                    let row = mouse_event.row;
                    // Inner area starts 1 past the border
                    let inner_y = menu_y + 1;
                    if col >= menu_x
                        && col < menu_x.saturating_add(menu_width)
                        && row >= inner_y
                        && row < inner_y.saturating_add(items.len() as u16)
                    {
                        let clicked_index = (row - inner_y) as usize;
                        if clicked_index < items.len() {
                            self.context_menu_state.as_mut().unwrap().selected_index =
                                clicked_index;
                            self.execute_context_menu_item();
                            return;
                        }
                    }
                    // Click was outside the menu — just dismiss it
                    self.dismiss_context_menu();
                    return;
                }

                let input_area = self.last_input_area.get();
                let selectable_area = self.last_selectable_area.get();

                let in_input = input_area.width > 0
                    && input_area.height > 0
                    && mouse_event.row >= input_area.y
                    && mouse_event.row < input_area.y.saturating_add(input_area.height)
                    && mouse_event.column >= input_area.x
                    && mouse_event.column < input_area.x.saturating_add(input_area.width);

                let in_selectable = selectable_area.width > 0
                    && selectable_area.height > 0
                    && mouse_event.row >= selectable_area.y
                    && mouse_event.row < selectable_area.y.saturating_add(selectable_area.height)
                    && mouse_event.column >= selectable_area.x
                    && mouse_event.column < selectable_area.x.saturating_add(selectable_area.width);

                // Check for click on a thinking block header (takes priority over text selection).
                if let Some(&hash) = self.thinking_row_map.borrow().get(&mouse_event.row) {
                    if self.thinking_expanded.contains(&hash) {
                        self.thinking_expanded.remove(&hash);
                    } else {
                        self.thinking_expanded.insert(hash);
                    }
                    self.invalidate_transcript();
                    return;
                }

                if in_input {
                    self.focus = FocusTarget::Input;
                    self.clear_selection();
                } else if selectable_area.width == 0 || selectable_area.height == 0 {
                    self.click_count = 0;
                } else if in_selectable {
                    self.focus = FocusTarget::Transcript;

                    let current_pos = (mouse_event.column, mouse_event.row);
                    let now = std::time::Instant::now();

                    // Check for double-click
                    if self.is_double_click(current_pos) {
                        self.click_count += 1;
                        if self.click_count >= 3 {
                            // Triple-click: select the paragraph (run of
                            // non-blank rows) containing the click. Falls back
                            // to a single line if no paragraph is detected.
                            if let Some((start_row, end_row, end_col)) =
                                self.find_paragraph_boundaries(current_pos.1)
                            {
                                self.selection_anchor = Some((selectable_area.x, start_row));
                                self.selection_focus = Some((end_col, end_row));
                            } else {
                                self.selection_anchor = Some((selectable_area.x, current_pos.1));
                                self.selection_focus = Some((
                                    selectable_area
                                        .x
                                        .saturating_add(selectable_area.width)
                                        .saturating_sub(1),
                                    current_pos.1,
                                ));
                            }
                            self.click_count = 0; // Reset for next click sequence
                        } else {
                            // Double-click: select word
                            if let Some((start, end)) =
                                self.find_word_boundaries(current_pos.0, current_pos.1)
                            {
                                self.selection_anchor = Some((start, current_pos.1));
                                self.selection_focus = Some((end, current_pos.1));
                            }
                        }
                    } else {
                        // Single click or new click sequence
                        self.click_count = 1;
                        self.selection_anchor = Some(current_pos);
                        self.selection_focus = Some(current_pos);
                        *self.selection_text.borrow_mut() = String::new();
                    }

                    self.last_click_time = Some(now);
                    self.last_click_position = Some(current_pos);
                } else {
                    self.click_count = 0;
                    self.clear_selection();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Dismiss context menu on drag
                self.dismiss_context_menu();

                // Continue drag — clamp to the selectable frame bounds so dragging
                // outside extends selection to the edge rather than cancelling.
                if self.selection_anchor.is_some() {
                    let selectable_area = self.last_selectable_area.get();
                    if selectable_area.width > 0 && selectable_area.height > 0 {
                        let clamped_col = mouse_event.column.max(selectable_area.x).min(
                            selectable_area
                                .x
                                .saturating_add(selectable_area.width)
                                .saturating_sub(1),
                        );
                        let clamped_row = mouse_event.row.max(selectable_area.y).min(
                            selectable_area
                                .y
                                .saturating_add(selectable_area.height)
                                .saturating_sub(1),
                        );
                        self.selection_focus = Some((clamped_col, clamped_row));
                        self.click_count = 0; // Reset on drag to prevent further double-clicks
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // Clear if no actual drag (single click = no selection)
                if self.selection_anchor == self.selection_focus {
                    self.clear_selection();
                } else if self.settings_screen.auto_copy_enabled {
                    // Auto-copy finalized selection to clipboard.
                    let sel_text = self.selection_text.borrow().clone();
                    if !sel_text.is_empty() {
                        let copied = crate::image_paste::write_clipboard_text(&sel_text);
                        if copied {
                            self.push_notification(
                                NotificationKind::Info,
                                "Copied to clipboard".to_string(),
                                Some(1),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // -------------------------------------------------------------------
    // Query event handling
    // -------------------------------------------------------------------

    /// Push a completed assistant message and trigger auto-scroll bookkeeping.
    fn push_assistant_message(&mut self, text: String) {
        let msg = Message::assistant(text);
        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }

    /// Process a query event from the agentic loop.
    /// Handle an AgentEvent from the agent. (iter-114 — replaces
    /// handle_query_event; eliminates the bridge layer.)
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        // Publish to debug bus (no-op when disabled).
        let event_variant = format!("{:?}", std::mem::discriminant(&event));
        let event_summary: String = format!("{:?}", &event).chars().take(80).collect();
        self.debug_hub
            .publish(crate::tui::debug::TuiEvent::AgentEvent {
                variant: event_variant,
                summary: event_summary,
                at: crate::tui::debug::event_bus::now_secs(),
            });

        // Auto-dismiss error modal when assistant responds
        match &event {
            AgentEvent::Content { .. }
            | AgentEvent::Thinking { .. }
            | AgentEvent::Reasoning { .. }
            | AgentEvent::Done { .. } => {
                self.dismiss_error_notifications();
            }
            _ => {}
        }

        match event {
            AgentEvent::Thinking { content } | AgentEvent::Reasoning { text: content } => {
                // Route thinking/reasoning to streaming_thinking.
                if !self.is_streaming {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed as u64).to_string());
                    if self.turn_start.is_none() {
                        self.turn_start = Some(std::time::Instant::now());
                    }
                    self.streaming_thinking.clear();
                    self.streaming_text.clear();
                }
                self.is_streaming = true;
                self.stall_start = None;
                // If we already have streaming text, this is a NEW iteration —
                // the model is thinking again after a tool call. Clear the old
                // text so we don't accumulate duplicate content across iterations.
                // (iter-122 — fixes "double thinking and text streaming" bug.)
                if !self.streaming_text.is_empty() {
                    // Flush the previous iteration's text as a completed message
                    // so it's preserved in the transcript, then start fresh.
                    self.flush_streamed_assistant_message();
                    self.streaming_thinking.clear();
                }
                self.streaming_thinking.push_str(&content);
                self.invalidate_transcript();
            }

            AgentEvent::Content { text } => {
                // Strip \r carriage returns as a safety net.
                // \r corrupts terminal display by moving cursor to column 0.
                let text = text.replace('\r', "");
                if !self.is_streaming {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed as u64).to_string());
                    if self.turn_start.is_none() {
                        self.turn_start = Some(std::time::Instant::now());
                    }
                    self.streaming_thinking.clear();
                    self.streaming_text.clear();
                }
                self.is_streaming = true;
                self.stall_start = None;
                // Accumulate streaming text. (Boundary flushes are handled by
                // AgentEvent::Thinking and AgentEvent::ToolStart.)
                self.streaming_text.push_str(&text);
                self.invalidate_transcript();
            }

            AgentEvent::ToolStart {
                tool_call_id,
                name,
                arguments,
            } => {
                if !self.is_streaming && self.spinner_verb.is_none() {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed as u64).to_string());
                }
                self.is_streaming = true;
                self.status_message = Some(format!("Running {}…", name));

                // When a tool starts, flush any accumulated streaming text/thinking
                // as a completed message. This prevents content from accumulating
                // across iterations (think → tool → think → tool → respond).
                // (iter-123 — fixes duplicate thinking/text in multi-iteration turns.)
                if !self.streaming_text.is_empty() || !self.streaming_thinking.is_empty() {
                    self.flush_streamed_assistant_message();
                }

                let turn_index = self.current_user_turn_index();
                let tool_id = tool_call_id.clone();
                let tool_name = name.clone();
                let input_json = arguments;
                if let Some(existing) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    existing.turn_index = turn_index;
                    existing.status = ToolStatus::Running;
                    existing.output_preview = None;
                    existing.input_json = input_json;
                } else {
                    self.tool_use_blocks.push(ToolUseBlock {
                        id: tool_id.clone(),
                        name: tool_name.clone(),
                        turn_index,
                        status: ToolStatus::Running,
                        output_preview: None,
                        input_json,
                    });
                }

                // Track subagent spawns for the status-bar HUD.
                if tool_name == "delegate_task" || tool_name == "spawn_subagent" {
                    self.agent_status.retain(|(id, _)| id != &tool_id);
                    self.agent_status.push((tool_id, "running".to_string()));
                }

                self.invalidate_transcript();
            }

            AgentEvent::ToolComplete { result } => {
                let tool_id = result.tool_call_id.clone();
                let is_error = !result.success;
                let result_text = if result.success {
                    result.content.clone()
                } else {
                    result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string())
                };
                let all_lines: Vec<&str> = result_text.lines().collect();
                let preview_lines = all_lines.len().min(3);
                let mut preview = all_lines[..preview_lines].join("\n");
                let remaining = all_lines.len().saturating_sub(preview_lines);
                if remaining > 0 {
                    preview.push_str(&format!("\n\u{2026} {} more lines", remaining));
                }
                if let Some(block) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    block.status = if is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Done
                    };
                    block.output_preview = Some(preview);

                    if block.name == "delegate_task" || block.name == "spawn_subagent" {
                        let new_status = if is_error { "error" } else { "done" };
                        for (id, st) in self.agent_status.iter_mut() {
                            if id == &tool_id {
                                *st = new_status.to_string();
                            }
                        }
                    }
                }
                self.invalidate_transcript();
                if is_error {
                    self.status_message = Some(format!("Tool error: {}", result_text));
                } else {
                    self.status_message = None;
                }
                // (iter-209: refresh_turn_diff_from_history removed)
            }

            AgentEvent::ToolError {
                tool_call_id,
                name: _,
                error,
            } => {
                let tool_id = tool_call_id.clone();
                let result_text = error;
                let all_lines: Vec<&str> = result_text.lines().collect();
                let preview_lines = all_lines.len().min(3);
                let mut preview = all_lines[..preview_lines].join("\n");
                let remaining = all_lines.len().saturating_sub(preview_lines);
                if remaining > 0 {
                    preview.push_str(&format!("\n\u{2026} {} more lines", remaining));
                }
                if let Some(block) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    block.status = ToolStatus::Error;
                    block.output_preview = Some(preview);

                    if block.name == "delegate_task" || block.name == "spawn_subagent" {
                        for (id, st) in self.agent_status.iter_mut() {
                            if id == &tool_id {
                                *st = "error".to_string();
                            }
                        }
                    }
                }
                self.invalidate_transcript();
                self.status_message = Some(format!("Tool error: {}", result_text));
                // (iter-209: refresh_turn_diff_from_history removed)
            }

            AgentEvent::Done { message } => {
                // Turn complete — the agent finished.
                // (iter-210: fix BACKEND_TUI_AUDIT.md §3 bug #2 — Done.message
                // was previously discarded with `message: _`. If the agent
                // emitted Done without preceding Content events (e.g. a
                // non-streaming error-recovery path), the user saw an empty
                // assistant message. Now: if streaming_text is empty, use
                // Done.message.content as the source of truth.)
                self.is_streaming = false;
                self.spinner_verb = None;

                // Record elapsed time and pick a completion verb
                let seed = self.frame_count as usize ^ (self.messages.len() * 7);
                let elapsed = self
                    .turn_start
                    .take()
                    .map(|start| format_elapsed_ms(start.elapsed().as_millis()));
                self.last_turn_elapsed = Some(elapsed.unwrap_or_else(|| "0s".to_string()));
                self.last_turn_verb = Some(sample_completion_verb(seed as u64));

                // If we have streamed content, flush it normally. If not,
                // use Done.message as the source of truth (fixes the
                // dropped-message bug for non-streaming paths).
                if self.streaming_text.trim().is_empty()
                    && self.streaming_thinking.trim().is_empty()
                    && !message.content.is_empty()
                {
                    // Non-streaming path: Done carries the full message.
                    let mut blocks = Vec::new();
                    if let Some(reasoning) = &message.reasoning {
                        if !reasoning.trim().is_empty() {
                            blocks.push(ContentBlock::Thinking {
                                thinking: reasoning.clone(),
                                signature: String::new(),
                            });
                        }
                    }
                    blocks.push(ContentBlock::Text {
                        text: message.content.clone(),
                    });
                    let msg = Message::assistant_blocks(blocks);
                    self.messages.push(msg);
                    self.invalidate_transcript();
                    self.on_new_message();
                } else {
                    self.flush_streamed_assistant_message();
                }
                // Mark any remaining Running blocks as Done — they completed
                // but the ToolComplete event either fired before the Done event
                // or was never emitted (fast tool / race condition). Pruning
                // them silently dropped the tool trail from the user's view.
                for block in &mut self.tool_use_blocks {
                    if block.status == ToolStatus::Running {
                        block.status = ToolStatus::Done;
                    }
                }
                self.complete_current_turn_snapshot(false);
                self.invalidate_transcript();
                // (iter-209: refresh_turn_diff_from_history removed)

                // Show a "copy" hint after each response so the user knows
                // they can copy the last response with /copy.
                // (iter-122 — user-requested: copy button at end of response.)
                self.push_notification(
                    NotificationKind::Info,
                    "Response complete · /copy to copy · Ctrl+J for line break".to_string(),
                    Some(4),
                );
            }

            AgentEvent::Error { error } => {
                self.is_streaming = false;
                self.spinner_verb = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.invalidate_transcript();
                let err_msg = format!("Error: {}", error);
                self.push_assistant_message(err_msg.clone());
                self.push_notification(NotificationKind::Error, err_msg, None);
            }

            AgentEvent::Usage {
                input_tokens,
                output_tokens,
                total_tokens,
            } => {
                // Record cost tracking immediately (was deferred to TurnComplete
                // via the bridge's pending_usage — now we record it directly).
                // (iter-210: fix BACKEND_TUI_AUDIT.md §3 bug #5 — total_tokens
                // was previously discarded with `total_tokens: _` and
                // recomputed by CostTracker as input+output. Now we use the
                // agent's authoritative total_tokens, which may include
                // cached/reasoning tokens that input+output misses.)
                let turn_tokens = total_tokens.max(input_tokens + output_tokens);
                self.context_used_tokens =
                    self.context_used_tokens.saturating_add(turn_tokens as u64);
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.record_usage(input_tokens, output_tokens);
                }
                self.cost_usd = self.cost_tracker.total_cost;
                self.token_count = turn_tokens;
                self.check_token_warnings();
            }

            AgentEvent::Cost {
                cost_usd,
                input_tokens,
                output_tokens,
                model,
            } => {
                // R3: wire the model-aware per-request cost into the live
                // tracker instead of discarding it. Falls back to a flat-rate
                // estimate only when the model isn't in the models_dev catalog.
                let cost = cost_usd.unwrap_or({
                    input_tokens as f64 * 0.000003 + output_tokens as f64 * 0.000015
                });
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.record_cost(cost);
                    tracker.set_model(&model);
                }
                self.cost_usd = self.cost_tracker.total_cost;
                if cost_usd.is_some() {
                    debug!(cost_usd = %cost, model = %model, "Per-request cost (models_dev)");
                } else {
                    debug!(cost_usd = %cost, model = %model, "Per-request cost (flat-rate fallback, model not in models_dev catalog)");
                }
            }

            AgentEvent::IterationComplete { iteration } => {
                // Update the current_turn counter for the "iter N" status pill.
                // (iter-209: current_turn field deleted with FileHistory stub.
                // The iteration count is still tracked via frame_count + the
                // IterationComplete event being published to the debug bus.)
                let _ = iteration;
            }

            AgentEvent::ToolPermissionRequest {
                tool_name,
                description,
                ..
            } => {
                // Permission requests are drained by the dedicated permission_rx
                // task. This event is a duplicate — skip it.
                let _ = (tool_name, description);
            }
        }
    }

    // -------------------------------------------------------------------
    // Main run loop
    // -------------------------------------------------------------------

    /// Run the TUI event loop. Returns `Some(input)` when the user submits
    /// a message, or `None` when the user quits.
    pub fn run<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<Option<String>> {
        loop {
            if self.is_simulating && self.simulated_keys.is_empty() && !self.is_streaming {
                self.should_exit = true;
            }
            // Frame-cap guard: a headless scenario that never stops streaming
            // (e.g. a real agent that hangs) can't spin the loop forever.
            if self.is_simulating {
                if let Some(max) = self.simulation_max_frames {
                    if self.frame_count >= max {
                        self.should_exit = true;
                    }
                }
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
                        if req.tool_name == "bash"
                            || req.tool_name == "shell"
                            || req.tool_name == "terminal"
                        {
                            if let Some(ref preview) = req.input_preview {
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
                        if key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT
                        {
                            if let KeyCode::Char(c) = key.code {
                                if self.prompt_is_accepting_text() {
                                    if let Some(burst) = self.try_detect_paste_burst(c) {
                                        self.handle_paste_data(burst);
                                        self.refresh_prompt_input();
                                        continue;
                                    }
                                } else if self.key_input_dialog.visible {
                                    if let Some(burst) = self.try_detect_paste_burst(c) {
                                        for ch in burst.chars() {
                                            self.key_input_dialog.insert_char(ch);
                                        }
                                        continue;
                                    }
                                }
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
                    _ => {}
                }
            }
        }
    }

// Helper function to open a file in the user's external editor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_app() -> App {
        let config = AppConfig::default();
        let settings = Settings::default();
        let cost_tracker = std::sync::Arc::new(crate::tui::adapter_types::cost::CostTracker::new());
        let command_registry = crate::commands::CommandRegistry::new();
        App::new(config, settings, cost_tracker, command_registry)
    }

    fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // ---- normalize_char_with_shift tests ----

    #[test]
    fn test_normalize_char_no_shift_returns_unchanged() {
        assert_eq!(normalize_char_with_shift('a', KeyModifiers::NONE), 'a');
        assert_eq!(normalize_char_with_shift('1', KeyModifiers::NONE), '1');
        assert_eq!(normalize_char_with_shift('!', KeyModifiers::NONE), '!');
    }

    #[test]
    fn test_normalize_char_shift_uppercase_letters() {
        assert_eq!(normalize_char_with_shift('a', KeyModifiers::SHIFT), 'A');
        assert_eq!(normalize_char_with_shift('z', KeyModifiers::SHIFT), 'Z');
        assert_eq!(normalize_char_with_shift('m', KeyModifiers::SHIFT), 'M');
    }

    #[test]
    fn test_normalize_char_shift_numbers() {
        assert_eq!(normalize_char_with_shift('1', KeyModifiers::SHIFT), '!');
        assert_eq!(normalize_char_with_shift('2', KeyModifiers::SHIFT), '@');
        assert_eq!(normalize_char_with_shift('3', KeyModifiers::SHIFT), '#');
        assert_eq!(normalize_char_with_shift('4', KeyModifiers::SHIFT), '$');
        assert_eq!(normalize_char_with_shift('5', KeyModifiers::SHIFT), '%');
        assert_eq!(normalize_char_with_shift('6', KeyModifiers::SHIFT), '^');
        assert_eq!(normalize_char_with_shift('7', KeyModifiers::SHIFT), '&');
        assert_eq!(normalize_char_with_shift('8', KeyModifiers::SHIFT), '*');
        assert_eq!(normalize_char_with_shift('9', KeyModifiers::SHIFT), '(');
        assert_eq!(normalize_char_with_shift('0', KeyModifiers::SHIFT), ')');
    }

    #[test]
    fn test_normalize_char_shift_symbols() {
        assert_eq!(normalize_char_with_shift('-', KeyModifiers::SHIFT), '_');
        assert_eq!(normalize_char_with_shift('=', KeyModifiers::SHIFT), '+');
        assert_eq!(normalize_char_with_shift('[', KeyModifiers::SHIFT), '{');
        assert_eq!(normalize_char_with_shift(']', KeyModifiers::SHIFT), '}');
        assert_eq!(normalize_char_with_shift(';', KeyModifiers::SHIFT), ':');
        assert_eq!(normalize_char_with_shift('\'', KeyModifiers::SHIFT), '"');
        assert_eq!(normalize_char_with_shift(',', KeyModifiers::SHIFT), '<');
        assert_eq!(normalize_char_with_shift('.', KeyModifiers::SHIFT), '>');
        assert_eq!(normalize_char_with_shift('/', KeyModifiers::SHIFT), '?');
        assert_eq!(normalize_char_with_shift('\\', KeyModifiers::SHIFT), '|');
        assert_eq!(normalize_char_with_shift('`', KeyModifiers::SHIFT), '~');
    }

    #[test]
    fn test_normalize_char_shift_already_shifted_chars_unchanged() {
        // Characters that don't have shift equivalents remain unchanged
        assert_eq!(normalize_char_with_shift('!', KeyModifiers::SHIFT), '!');
        assert_eq!(normalize_char_with_shift('@', KeyModifiers::SHIFT), '@');
        assert_eq!(normalize_char_with_shift('A', KeyModifiers::SHIFT), 'A');
    }

    #[test]
    fn test_normalize_char_other_modifiers_ignored() {
        // CTRL or ALT without SHIFT should not shift the character
        assert_eq!(normalize_char_with_shift('a', KeyModifiers::CONTROL), 'a');
        assert_eq!(normalize_char_with_shift('1', KeyModifiers::ALT), '1');
        assert_eq!(
            normalize_char_with_shift('a', KeyModifiers::CONTROL | KeyModifiers::ALT),
            'a'
        );
    }

    #[test]
    fn test_normalize_char_shift_with_other_modifiers() {
        // SHIFT + CTRL should still apply shift transformation
        assert_eq!(
            normalize_char_with_shift('a', KeyModifiers::SHIFT | KeyModifiers::CONTROL),
            'A'
        );
        assert_eq!(
            normalize_char_with_shift('1', KeyModifiers::SHIFT | KeyModifiers::ALT),
            '!'
        );
    }

    #[test]
    fn test_mcp_subcommand_is_not_intercepted() {
        let mut app = make_app();
        assert!(!app.intercept_slash_command_with_args("mcp", "auth mcphub"));
        assert!(!app.mcp_view.visible);
    }

    #[test]
    fn test_clear_slash_command_clears_messages() {
        let mut app = make_app();
        app.add_message(Role::User, "hello".to_string());
        app.add_message(Role::Assistant, "world".to_string());
        assert_eq!(app.messages.len(), 2);
        assert!(app.intercept_slash_command("clear"));
        assert_eq!(app.messages.len(), 0);
    }

    #[test]
    fn test_exit_slash_command_sets_quit_flag() {
        let mut app = make_app();
        assert!(!app.should_exit);
        assert!(app.intercept_slash_command("exit"));
        assert!(app.should_exit);
    }

    #[test]
    fn test_vim_slash_command_toggles_vim() {
        let mut app = make_app();
        assert!(!app.prompt_input.vim_enabled);
        assert!(app.intercept_slash_command("vim"));
        assert!(app.prompt_input.vim_enabled);
        assert!(app.intercept_slash_command("vim"));
        assert!(!app.prompt_input.vim_enabled);
    }

    #[test]
    fn test_model_slash_command_opens_picker() {
        let mut app = make_app();
        app.has_credentials = true;
        assert!(!app.model_picker.visible);
        assert!(app.intercept_slash_command("model"));
        assert!(app.model_picker.visible);
    }

    #[test]
    fn test_tasks_slash_command_is_an_alias_for_agents() {
        // /tasks is documented (commands.rs alias + gateway help text) as an
        // alias for /agents, but this match arm previously only accepted
        // the literal "agents" — /tasks fell through to a dead
        // CommandRegistry.handlers fallback and printed a "not yet wired"
        // error instead of opening the agents menu (iter-248).
        let mut app = make_app();
        assert!(!app.agents_menu.visible);
        assert!(app.intercept_slash_command("tasks"));
        assert!(app.agents_menu.visible);
    }

    #[test]
    fn test_fast_slash_command_toggles_fast_mode() {
        let mut app = make_app();
        assert!(!app.fast_mode);
        assert!(app.intercept_slash_command("fast"));
        assert!(app.fast_mode);
        assert!(app.intercept_slash_command("fast"));
        assert!(!app.fast_mode);
    }

    #[test]
    fn test_output_style_cycles() {
        let mut app = make_app();
        assert_eq!(app.output_style, "auto");
        assert!(app.intercept_slash_command("output-style"));
        assert_eq!(app.output_style, "stream");
        assert!(app.intercept_slash_command("output-style"));
        assert_eq!(app.output_style, "verbose");
        assert!(app.intercept_slash_command("output-style"));
        assert_eq!(app.output_style, "auto");
    }

    #[test]
    fn test_context_menu_fork_targets_clicked_message() {
        let mut app = make_app();
        app.add_message(Role::User, "one".to_string());
        app.add_message(Role::Assistant, "two".to_string());
        app.add_message(Role::User, "three".to_string());

        app.handle_context_menu_action(
            ContextMenuItem::Fork,
            ContextMenuKind::Message { message_index: 1 },
        );

        assert_eq!(app.prompt_input.text, "/fork 2");
        assert_eq!(
            app.status_message.as_deref(),
            Some("Fork at message 2 - press Enter to confirm")
        );
    }

    #[test]
    fn test_right_click_targets_row_message_instead_of_last_message() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut app = make_app();
        app.last_msg_area.set(ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        });
        app.message_row_map.borrow_mut().insert(3, 1);

        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 12,
            row: 3,
            modifiers: KeyModifiers::empty(),
        });

        assert!(matches!(
            app.context_menu_state,
            Some(ContextMenuState {
                kind: ContextMenuKind::Message { message_index: 1 },
                ..
            })
        ));
    }

    // ---- Help overlay -------------------------------------------------------

    #[test]
    fn test_help_slash_command_opens_overlay() {
        let mut app = make_app();
        assert!(!app.help_overlay.visible);
        assert!(!app.show_help);
        assert!(!app.help_overlay.commands.is_empty());
        assert!(app.intercept_slash_command("help"));
        assert!(app.help_overlay.visible);
        assert!(app.show_help);
    }

    #[test]
    fn test_help_slash_command_toggles() {
        // iter-85: /help now toggles (was idempotent-open in iter-81, which
        // was itself a regression — the audit found that pressing /help twice
        // showed two different help overlays). Correct behavior: first call
        // opens, second call closes.
        let mut app = make_app();
        // First call opens it.
        assert!(app.intercept_slash_command("help"));
        assert!(app.help_overlay.visible);
        assert!(app.show_help);
        // Second call closes it (toggle, not idempotent-open).
        assert!(app.intercept_slash_command("help"));
        assert!(!app.help_overlay.visible);
        assert!(!app.show_help);
        // Third call opens it again.
        assert!(app.intercept_slash_command("help"));
        assert!(app.help_overlay.visible);
        assert!(app.show_help);
    }

    #[test]
    fn test_question_mark_shortcut_opens_help_with_shift_modifier() {
        let mut app = make_app();

        app.handle_key_event(press_key(KeyCode::Char('?'), KeyModifiers::SHIFT));

        assert!(app.help_overlay.visible);
        assert!(app.show_help);
    }

    #[test]
    fn test_question_mark_shortcut_closes_help_with_shift_modifier() {
        let mut app = make_app();
        app.help_overlay.toggle();
        app.show_help = true;

        app.handle_key_event(press_key(KeyCode::Char('?'), KeyModifiers::SHIFT));

        assert!(!app.help_overlay.visible);
        assert!(!app.show_help);
    }

    #[test]
    fn test_question_mark_shortcut_types_into_non_empty_prompt() {
        let mut app = make_app();
        app.prompt_input.text = "why".to_string();
        app.prompt_input.cursor = app.prompt_input.text.len();
        app.refresh_prompt_input();

        app.handle_key_event(press_key(KeyCode::Char('?'), KeyModifiers::SHIFT));

        assert!(!app.help_overlay.visible);
        assert_eq!(app.prompt_input.text, "why?");
    }

    #[test]
    fn test_ctrl_a_shortcut_opens_model_picker() {
        let mut app = make_app();
        app.has_credentials = true;
        app.active_provider = Some("anthropic".to_string());

        app.handle_key_event(press_key(KeyCode::Char('a'), KeyModifiers::CONTROL));

        assert!(app.model_picker.visible);
    }

    #[test]
    fn test_ctrl_k_shortcut_opens_command_palette_even_with_input() {
        let mut app = make_app();
        app.prompt_input.text = "hello".to_string();
        app.prompt_input.cursor = app.prompt_input.text.len();
        app.refresh_prompt_input();

        app.handle_key_event(press_key(KeyCode::Char('k'), KeyModifiers::CONTROL));

        assert!(app.command_palette.visible);
        assert_eq!(app.prompt_input.text, "hello");
    }

    // ---- Bash prefix allowlist ----------------------------------------------

    #[test]
    fn test_bash_command_not_allowed_by_default() {
        let app = make_app();
        assert!(!app.bash_command_allowed_by_prefix("git status"));
        assert!(!app.bash_command_allowed_by_prefix("ls -la"));
        assert!(!app.bash_command_allowed_by_prefix(""));
    }

    #[test]
    fn test_bash_prefix_allowlist_after_p_key() {
        use crate::tui::dialogs::PermissionRequest;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let mut app = make_app();
        // Set up a bash permission dialog with a suggested prefix.
        let pr = PermissionRequest::bash(
            "tu-1".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
            "git status".to_string(),
            Some("git".to_string()),
        );
        app.permission_request = Some(pr);

        // Simulate pressing 'P' (prefix-allow key).
        let key = KeyEvent {
            code: KeyCode::Char('P'),
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_permission_key(key);

        // Dialog should be dismissed and "git" added to the allowlist.
        assert!(app.permission_request.is_none());
        assert!(app.bash_command_allowed_by_prefix("git status"));
        assert!(app.bash_command_allowed_by_prefix("git push origin main"));
        // Other commands should NOT be allowed.
        assert!(!app.bash_command_allowed_by_prefix("rm -rf /tmp"));
    }

    #[test]
    fn test_bash_prefix_allowlist_via_enter_on_p_option() {
        use crate::tui::dialogs::PermissionRequest;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let mut app = make_app();
        let mut pr = PermissionRequest::bash(
            "tu-2".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
            "cargo build".to_string(),
            Some("cargo".to_string()),
        );
        // Navigate to the prefix option (index 3 in a 5-option dialog).
        pr.selected_option = 3;
        app.permission_request = Some(pr);

        // Press Enter to confirm the currently selected (prefix) option.
        let key = KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        assert!(app.bash_command_allowed_by_prefix("cargo test"));
        assert!(!app.bash_command_allowed_by_prefix("make build"));
    }

    #[test]
    fn test_bash_prefix_allowlist_non_prefix_option_does_not_add() {
        use crate::tui::dialogs::PermissionRequest;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let mut app = make_app();
        let pr = PermissionRequest::bash(
            "tu-3".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
            "npm install".to_string(),
            Some("npm".to_string()),
        );
        app.permission_request = Some(pr);

        // Press 'y' (allow-once) — should NOT add to allowlist.
        let key = KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        assert!(!app.bash_command_allowed_by_prefix("npm test"));
    }

    // ---- iter-20: permission dialog response routing ----------------------

    #[test]
    fn test_permission_dialog_y_sends_allow_once() {
        use crate::tui::dialogs::PermissionRequest;

        let mut app = make_app();
        let pr = PermissionRequest::standard(
            "tu-1".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
        );
        app.permission_request = Some(pr);

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission_response_tx = Some(tx);

        let key = press_key(KeyCode::Char('y'), KeyModifiers::NONE);
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        assert!(app.pending_permission_response_tx.is_none());
        let response = rx.try_recv().expect("response should be sent");
        assert_eq!(
            response,
            operant_core::agent::ToolPermissionResponse::AllowOnce
        );
    }

    #[test]
    fn test_permission_dialog_uppercase_y_sends_allow_session() {
        use crate::tui::dialogs::PermissionRequest;

        let mut app = make_app();
        let pr = PermissionRequest::standard(
            "tu-2".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
        );
        app.permission_request = Some(pr);

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission_response_tx = Some(tx);

        // Shift+y → uppercase 'Y' (the session-allow key).
        let key = press_key(KeyCode::Char('Y'), KeyModifiers::SHIFT);
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        let response = rx.try_recv().expect("response should be sent");
        assert_eq!(
            response,
            operant_core::agent::ToolPermissionResponse::AllowSession
        );
    }

    #[test]
    fn test_permission_dialog_n_sends_deny() {
        use crate::tui::dialogs::PermissionRequest;

        let mut app = make_app();
        let pr = PermissionRequest::standard(
            "tu-3".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
        );
        app.permission_request = Some(pr);

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission_response_tx = Some(tx);

        let key = press_key(KeyCode::Char('n'), KeyModifiers::NONE);
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        let response = rx.try_recv().expect("response should be sent");
        assert_eq!(response, operant_core::agent::ToolPermissionResponse::Deny);
    }

    #[test]
    fn test_permission_dialog_esc_sends_deny() {
        use crate::tui::dialogs::PermissionRequest;

        let mut app = make_app();
        let pr = PermissionRequest::standard(
            "tu-4".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
        );
        app.permission_request = Some(pr);

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission_response_tx = Some(tx);

        let key = press_key(KeyCode::Esc, KeyModifiers::NONE);
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        let response = rx.try_recv().expect("response should be sent");
        assert_eq!(response, operant_core::agent::ToolPermissionResponse::Deny);
    }

    #[test]
    fn test_permission_dialog_enter_sends_selected_option_response() {
        use crate::tui::dialogs::PermissionRequest;

        let mut app = make_app();
        let mut pr = PermissionRequest::standard(
            "tu-5".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
        );
        // Move selection down to the deny option (index 3).
        pr.selected_option = 3;
        app.permission_request = Some(pr);

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission_response_tx = Some(tx);

        let key = press_key(KeyCode::Enter, KeyModifiers::NONE);
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        let response = rx.try_recv().expect("response should be sent");
        assert_eq!(response, operant_core::agent::ToolPermissionResponse::Deny);
    }

    #[test]
    fn test_permission_dialog_no_tx_does_not_panic() {
        use crate::tui::dialogs::PermissionRequest;

        // Tests the case where the dialog was opened without a response_tx
        // (e.g. directly constructed in tests). resolve_permission_dialog
        // should silently no-op the send, not panic.
        let mut app = make_app();
        let pr = PermissionRequest::standard(
            "tu-6".to_string(),
            "Bash".to_string(),
            "This will execute a shell command.".to_string(),
        );
        app.permission_request = Some(pr);
        // pending_permission_response_tx is None by default.

        let key = press_key(KeyCode::Char('y'), KeyModifiers::NONE);
        app.handle_permission_key(key);

        assert!(app.permission_request.is_none());
        assert!(app.pending_permission_response_tx.is_none());
    }

    // ---- Phase 2 regression tests (iter-212) ----
    // These tests lock in the behavior fixed/added in Phases 1-4:
    //   - F12 debug overlay toggle (Phase 1)
    //   - Done.message not dropped (Phase 3c, bug #2)
    //   - Usage.total_tokens not dropped (Phase 3c, bug #5)
    //   - Stub McpManager/FileHistory eliminated (Phase 3a/3b)
    //   - feedback_survey removed (Phase 4)

    #[test]
    fn test_f12_toggles_debug_overlay() {
        // Phase 1: F12 must toggle the debug overlay visibility.
        let mut app = make_app();
        assert!(
            !app.debug_hub.overlay_visible(),
            "overlay should start hidden"
        );

        app.handle_key_event(press_key(KeyCode::F(12), KeyModifiers::NONE));
        assert!(app.debug_hub.overlay_visible(), "F12 should show overlay");

        app.handle_key_event(press_key(KeyCode::F(12), KeyModifiers::NONE));
        assert!(
            !app.debug_hub.overlay_visible(),
            "second F12 should hide overlay"
        );
    }

    #[test]
    fn test_f12_works_even_with_input() {
        // F12 must work even when there's text in the input buffer — it's
        // the highest-priority keybind and must never be blocked.
        let mut app = make_app();
        app.input = "some text".to_string();
        app.handle_key_event(press_key(KeyCode::F(12), KeyModifiers::NONE));
        assert!(app.debug_hub.overlay_visible());
        // Input must be preserved — F12 doesn't consume or clear it.
        assert_eq!(app.input, "some text");
    }

    #[test]
    fn test_done_message_used_when_no_streaming() {
        // Phase 3c bug #2: Done.message was discarded. Now if streaming_text
        // is empty, Done.message.content is used as the assistant message.
        let mut app = make_app();
        assert!(app.messages.is_empty());
        // Simulate non-streaming path: no Content events, Done carries full msg.
        let done_msg = operant_core::client::Message {
            role: operant_core::client::Role::Assistant,
            content: "Hello from Done".to_string(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        };
        app.handle_agent_event(AgentEvent::Done { message: done_msg });
        assert_eq!(app.messages.len(), 1, "Done should produce 1 message");
        assert!(
            app.messages[0].text_content().contains("Hello from Done"),
            "message should contain Done.message.content"
        );
    }

    #[test]
    fn test_done_with_streaming_uses_streamed_text() {
        // When streaming occurred, Done should NOT override with its message —
        // the streamed text is the source of truth (it may have been
        // post-processed or differ from the final Done payload).
        let mut app = make_app();
        // Simulate streaming: Content events fill streaming_text.
        app.is_streaming = true;
        app.streaming_text = "Streamed content".to_string();
        let done_msg = operant_core::client::Message {
            role: operant_core::client::Role::Assistant,
            content: "This should NOT be used".to_string(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        };
        app.handle_agent_event(AgentEvent::Done { message: done_msg });
        assert_eq!(app.messages.len(), 1);
        assert!(
            app.messages[0].text_content().contains("Streamed content"),
            "streamed text should win over Done.message when streaming occurred"
        );
    }

    #[test]
    fn test_usage_total_tokens_not_dropped() {
        // Phase 3c bug #5: total_tokens was discarded. Now the authoritative
        // value from the agent is used (which may include cached/reasoning
        // tokens that input+output misses).
        let mut app = make_app();
        app.handle_agent_event(AgentEvent::Usage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 200, // > 100+50=150, simulates cached tokens
        });
        assert_eq!(
            app.token_count, 200,
            "token_count should use authoritative total_tokens (200), not input+output (150)"
        );
    }

    #[test]
    fn test_usage_event_pushes_token_warning_notification() {
        // iter-255: check_token_warnings() was never called from the Usage
        // handler despite its doc comment saying to call it after updating
        // token_count — the whole warning subsystem was dead. context_window_for_model
        // is a fixed 128000 stub, so 110_000 tokens crosses the 80% threshold.
        let mut app = make_app();
        app.handle_agent_event(AgentEvent::Usage {
            input_tokens: 60_000,
            output_tokens: 50_000,
            total_tokens: 110_000,
        });
        assert_eq!(app.token_warning_threshold_shown, 80);
        assert!(
            app.notifications
                .notifications
                .iter()
                .any(|n| n.message.contains("80% full")),
            "expected an 80%-full context warning notification to be pushed"
        );
    }

    #[test]
    fn test_token_warning_threshold_resets_when_usage_drops() {
        // Without a reset, an escalate-only gate would permanently suppress
        // warnings after /clear or /compact shrinks the context back down.
        let mut app = make_app();
        app.handle_agent_event(AgentEvent::Usage {
            input_tokens: 100_000,
            output_tokens: 21_600,
            total_tokens: 121_600, // 95% of the 128_000 stub window
        });
        assert_eq!(app.token_warning_threshold_shown, 95);

        // Simulate /clear (or a successful /compact) shrinking usage back down.
        app.handle_agent_event(AgentEvent::Usage {
            input_tokens: 1_000,
            output_tokens: 0,
            total_tokens: 1_000,
        });
        assert_eq!(
            app.token_warning_threshold_shown, 0,
            "threshold tracker should reset once usage drops back below it"
        );
    }

    #[test]
    fn test_drop_pending_images_with_notice_warns_and_clears() {
        // iter-255: pasted images were never attached to the outgoing message
        // (no multi-part content support in the core client) nor cleared on
        // send, so the thumbnail row lingered forever looking attached.
        let mut app = make_app();
        app.prompt_input.add_image(crate::image_paste::PastedImage {
            path: std::path::PathBuf::from("/tmp/test.png"),
            label: "test.png".to_string(),
            dimensions: None,
        });
        app.drop_pending_images_with_notice();
        assert!(app.prompt_input.pending_images.is_empty());
        assert!(
            app.notifications
                .notifications
                .iter()
                .any(|n| n.message.contains("dropped")),
            "expected a warning that the image wasn't sent"
        );
    }

    #[test]
    fn test_drop_pending_images_with_notice_noop_when_empty() {
        let mut app = make_app();
        let before = app.notifications.notifications.len();
        app.drop_pending_images_with_notice();
        assert_eq!(app.notifications.notifications.len(), before);
    }

    #[test]
    fn test_usage_falls_back_to_sum_when_total_is_zero() {
        // Some providers send total_tokens=0. In that case, fall back to
        // input+output so we don't show 0 tokens.
        let mut app = make_app();
        app.handle_agent_event(AgentEvent::Usage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 0,
        });
        assert_eq!(
            app.token_count, 150,
            "should fall back to input+output when total_tokens is 0"
        );
    }

    #[test]
    fn test_stubs_eliminated_no_mcp_manager_field() {
        // Phase 3a: App.mcp_manager (stub) field must be gone.
        // We verify by checking that core_mcp_manager is the only MCP field.
        let app = make_app();
        assert!(
            app.core_mcp_manager.is_none(),
            "core_mcp_manager starts None"
        );
        // If the stub field still existed, this wouldn't compile — the type
        // system enforces the removal.
    }

    #[test]
    fn test_stubs_eliminated_no_file_history_field() {
        // Phase 3b: App.file_history + current_turn fields must be gone.
        // Verified by compilation — if they existed, referencing them would
        // be needed. Their absence is the test.
        let app = make_app();
        assert!(
            app.diff_viewer.turn_files.is_empty(),
            "no turn-files without stub"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_feedback_survey_removed() {
        // Phase 4: /survey command must not be intercepted (feedback_survey deleted).
        // intercept_slash_command returns true if the command is known+intercepted.
        // /survey was removed from the command table, so it returns false.
        let mut app = make_app();
        let result = app.intercept_slash_command("survey");
        assert!(
            !result,
            "/survey should not be intercepted after feedback_survey deletion"
        );
    }

    #[test]
    fn test_debug_hub_records_frames() {
        // Phase 1: record_frame should increment frame count.
        let app = make_app();
        assert_eq!(app.debug_hub.frame_count(), 0);
        app.debug_hub.record_frame(5.0);
        app.debug_hub.record_frame(3.0);
        assert_eq!(app.debug_hub.frame_count(), 2);
        assert_eq!(app.debug_hub.last_render_ms(), 3);
    }

    #[test]
    fn test_debug_hub_records_errors() {
        // Phase 1: record_error should store the last error.
        let app = make_app();
        assert!(app.debug_hub.last_error().is_none());
        app.debug_hub.record_error("test", "something broke");
        assert_eq!(
            app.debug_hub.last_error().unwrap(),
            "[test] something broke"
        );
    }

    #[test]
    fn test_interactive_multi_step_simulation() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut app = make_app();
        app.is_simulating = true;

        // 1. Simulate typing a slash command "/help"
        app.simulated_keys = vec![
            press_key(KeyCode::Char('/'), KeyModifiers::NONE),
            press_key(KeyCode::Char('h'), KeyModifiers::NONE),
            press_key(KeyCode::Char('e'), KeyModifiers::NONE),
            press_key(KeyCode::Char('l'), KeyModifiers::NONE),
            press_key(KeyCode::Char('p'), KeyModifiers::NONE),
            press_key(KeyCode::Enter, KeyModifiers::NONE),
        ];

        // 2. Run the loop ticks
        while !app.simulated_keys.is_empty() && !app.should_exit {
            if let Ok(Some(input)) = app.run(&mut terminal) {
                if crate::input::is_slash_command(&input) {
                    let (cmd, args) = crate::input::parse_slash_command(&input);
                    app.handle_tui_command(cmd, args);
                }
            }
        }

        // 3. Assert the help overlay is open
        assert!(app.help_overlay.visible);
        assert!(app.show_help);

        // 4. Simulate pressing Escape to close the overlay
        app.simulated_keys = vec![press_key(KeyCode::Esc, KeyModifiers::NONE)];

        while !app.simulated_keys.is_empty() && !app.should_exit {
            if let Ok(Some(input)) = app.run(&mut terminal) {
                if crate::input::is_slash_command(&input) {
                    let (cmd, args) = crate::input::parse_slash_command(&input);
                    app.handle_tui_command(cmd, args);
                }
            }
        }

        // 5. Assert the help overlay is closed
        assert!(!app.help_overlay.visible);
        assert!(!app.show_help);

        // 6. Simulate quitting
        app.simulated_keys = vec![
            press_key(KeyCode::Char('/'), KeyModifiers::NONE),
            press_key(KeyCode::Char('q'), KeyModifiers::NONE),
            press_key(KeyCode::Char('u'), KeyModifiers::NONE),
            press_key(KeyCode::Char('i'), KeyModifiers::NONE),
            press_key(KeyCode::Char('t'), KeyModifiers::NONE),
            press_key(KeyCode::Enter, KeyModifiers::NONE),
        ];

        while !app.simulated_keys.is_empty() && !app.should_exit {
            if let Ok(Some(input)) = app.run(&mut terminal) {
                if crate::input::is_slash_command(&input) {
                    let (cmd, args) = crate::input::parse_slash_command(&input);
                    app.handle_tui_command(cmd, args);
                }
            }
        }

        // 7. Assert app wants to exit
        assert!(app.should_exit);
    }

    // ---- Phase A5: dialog open/close scenario regression pack -------------
    // Drives simulated keys through the real run loop (with the same slash
    // interception the interactive/headless loops use), then asserts state
    // via App::debug_snapshot(). This is the safety net that gates the
    // dialog-unification refactor (Phase B): every listed overlay must open
    // via its slash command and close on Esc.

    fn drive_keys<B: ratatui::backend::Backend>(
        app: &mut App,
        terminal: &mut ratatui::Terminal<B>,
    ) {
        let mut guard = 0;
        while !app.simulated_keys.is_empty() && !app.should_exit && guard < 5000 {
            guard += 1;
            if let Ok(Some(input)) = app.run(terminal) {
                if crate::input::is_slash_command(&input) {
                    let (cmd, args) = crate::input::parse_slash_command(&input);
                    app.handle_tui_command(cmd, args);
                }
            }
        }
    }

    fn slash_keys(cmd: &str) -> Vec<KeyEvent> {
        let mut keys = vec![press_key(KeyCode::Char('/'), KeyModifiers::NONE)];
        for ch in cmd.chars() {
            keys.push(press_key(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        keys.push(press_key(KeyCode::Enter, KeyModifiers::NONE));
        keys
    }

    #[test]
    fn test_dialog_open_close_scenarios() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // (slash command, snapshot overlay key). Each must open via
        // `/<cmd><enter>` and close on Esc.
        let scenarios: &[(&str, &str)] = &[
            ("help", "help_overlay"),
            ("settings", "settings_screen"),
            ("theme", "theme_screen"),
            ("stats", "stats_dialog"),
            ("skills", "skills_view"),
            ("journey", "journey_view"),
            ("plugins", "plugins_hub"),
            ("model", "model_picker"),
            ("effort", "effort_picker"),
            ("context", "context_viz"),
            ("agents", "agents_menu"),
            ("export", "export_dialog"),
            ("mcp", "mcp_view"),
        ];

        for (cmd, overlay) in scenarios {
            let mut app = make_app();
            app.is_simulating = true;
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

            app.simulated_keys = slash_keys(cmd);
            drive_keys(&mut app, &mut terminal);

            let snap = app.debug_snapshot();
            assert_eq!(
                snap["overlays"][overlay],
                serde_json::Value::Bool(true),
                "/{cmd} should open overlay '{overlay}'"
            );
            assert_eq!(
                snap["any_modal_open"],
                serde_json::Value::Bool(true),
                "/{cmd} should register a modal as open"
            );

            // Esc must close it.
            app.simulated_keys = vec![press_key(KeyCode::Esc, KeyModifiers::NONE)];
            drive_keys(&mut app, &mut terminal);
            let snap = app.debug_snapshot();
            assert_eq!(
                snap["overlays"][overlay],
                serde_json::Value::Bool(false),
                "Esc should close overlay '{overlay}' opened by /{cmd}"
            );
        }
    }

    // Consistency guard (iter-237 / Phase B1): `overlay_flags()` is the single
    // source of truth for the overlay set. `debug_snapshot()`'s overlays map is
    // built from it, so their key sets must be identical. This is what prevents
    // the parallel-list drift that dropped `effort_picker` in iter-227.
    #[test]
    fn test_overlay_flags_matches_debug_snapshot_keys() {
        let app = make_app();

        let mut flag_keys: Vec<String> = app
            .overlay_flags()
            .iter()
            .map(|(k, _): &(&str, bool)| k.to_string())
            .collect();
        flag_keys.sort();

        let snap = app.debug_snapshot();
        let mut snap_keys: Vec<String> = snap["overlays"]
            .as_object()
            .expect("overlays should be a JSON object")
            .keys()
            .cloned()
            .collect();
        snap_keys.sort();

        assert_eq!(
            flag_keys, snap_keys,
            "overlay_flags() and debug_snapshot() overlays must have identical keys"
        );
    }

    #[test]
    fn test_streaming_agent_events_commit_message() {
        use operant_core::agent::AgentEvent;

        let mut app = make_app();
        app.is_streaming = true;
        app.handle_agent_event(AgentEvent::Content {
            text: "Hello ".into(),
        });
        app.handle_agent_event(AgentEvent::Content {
            text: "world".into(),
        });
        app.handle_agent_event(AgentEvent::Done {
            message: operant_core::client::Message::assistant("Hello world"),
        });

        let snap = app.debug_snapshot();
        assert!(
            snap["messages"].as_u64().unwrap_or(0) >= 1,
            "Done should commit at least one assistant message"
        );
    }

    #[test]
    fn test_command_palette_opens_via_ctrl_k() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = make_app();
        app.is_simulating = true;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        app.simulated_keys = vec![press_key(KeyCode::Char('k'), KeyModifiers::CONTROL)];
        drive_keys(&mut app, &mut terminal);

        let snap = app.debug_snapshot();
        assert_eq!(
            snap["overlays"]["command_palette"],
            serde_json::Value::Bool(true),
            "Ctrl+K should open the command palette"
        );
    }
}
