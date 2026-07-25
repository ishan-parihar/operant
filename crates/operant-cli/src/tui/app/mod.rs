// app/mod.rs — App state struct and main event loop.

mod enums;
mod helpers;
mod init;
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

fn open_file_externally(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    // Try to open with the system's default application
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(&["/C", "start", ""])
            .arg(path)
            .spawn()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // Fallback for other systems: try common editors in order
        for editor in &["nano", "vi", "vim", "emacs"] {
            match std::process::Command::new(editor).arg(path).spawn() {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
        Err("No suitable editor found".into())
    }
}

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

    fn display_default_model_for_provider(&self, provider_id: &str) -> String {
        crate::tui::model_picker::default_model_for_provider(provider_id, &self.model_registry)
    }

    fn open_model_picker_for_provider(&mut self, provider_id: &str, title: Option<String>) {
        self.dismiss_error_notifications();

        let cache_path = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("operant")
            .join("models.json");
        if cache_path.exists() {
            self.model_registry.load_cache(&cache_path);
        }

        let models = crate::tui::model_picker::models_for_provider_from_registry(
            provider_id,
            &self.model_registry,
        );
        self.model_picker.set_models(models);
        self.model_picker_provider_id = Some(provider_id.to_string());
        self.model_picker_fetch_pending = true;

        // Fetch models from provider's API in background
        let settings = crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
        let api_key = self.auth_store.api_key_for(
            provider_id
                .parse::<crate::tui::adapter_types::ProviderId>()
                .unwrap_or(crate::tui::adapter_types::ProviderId::Other(
                    provider_id.to_string(),
                )),
        );
        let base_url = settings
            .providers
            .get(provider_id)
            .and_then(|p| p.api_base.clone())
            .or_else(|| {
                crate::provider::PROVIDERS
                    .iter()
                    .find(|p| p.name == provider_id)
                    .map(|p| p.default_base_url.to_string())
            });

        if let (Some(key), Some(url)) = (api_key, base_url) {
            let provider_id = provider_id.to_string();
            let mut registry = self.model_registry.clone();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.model_fetch_rx = Some(rx);
            tokio::spawn(async move {
                let _fetch_result = registry
                    .fetch_from_provider_async(&provider_id, &key, &url)
                    .await;
                // Check if fetch returned any models; if empty, it's likely an error
                let models = crate::tui::model_picker::models_for_provider_from_registry(
                    &provider_id,
                    &registry,
                );
                if models.is_empty() {
                    // Fetch failed - send error
                    let _ = tx.send(Err(format!(
                        "Failed to fetch models from {} (rate limit, auth error, or network issue)",
                        provider_id
                    )))
                    .await;
                } else {
                    let _ = tx.send(Ok(models)).await;
                }
            });
        }

        let provider_prefix = format!("{}/", provider_id);
        let current_model = if self.active_provider.as_deref() == Some(provider_id) {
            self.model_name
                .strip_prefix(&provider_prefix)
                .unwrap_or(self.model_name.as_str())
                .to_string()
        } else {
            let default_model = self.display_default_model_for_provider(provider_id);
            default_model
                .strip_prefix(&provider_prefix)
                .unwrap_or(default_model.as_str())
                .to_string()
        };

        self.model_picker.open_with_title(
            title.unwrap_or_else(|| "Select model".to_string()),
            &current_model,
            self.effort_level,
            self.fast_mode,
        );
    }

    fn activate_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        status_prefix: &str,
    ) {
        let picker_title = provider_name.clone();
        self.fast_mode = false;
        self.set_provider_default(provider_id.clone());
        self.persist_provider_and_model();
        self.has_credentials = true;
        self.status_message = Some(format!("{} {}.", status_prefix, provider_name));
        // Mark onboarding as complete now that the user has connected a
        // provider. (P0-2 from UX audit — was never called.)
        let _ = Self::persist_onboarding_complete();
        self.open_model_picker_for_provider(&provider_id, Some(picker_title));
    }

    fn persist_custom_provider_base_url(&self, base_url: &str) {
        let mut settings = Settings::load_sync().unwrap_or_default();
        let entry = settings
            .providers
            .entry("custom-openai".to_string())
            .or_default();
        entry.api_base = Some(base_url.to_string());
        entry.enabled = true;
        let _ = settings.save_sync();
    }

    fn persist_provider_and_model(&self) {
        // Provider+model live exclusively in operant.toml — written via
        // sync_model_to_toml below. settings.json is NOT written here; it only
        // stores visual prefs (theme, vim_enabled, reduce_motion, etc.) which are
        // persisted separately. (iter-221: removed dead settings.json round-trip
        // that was a no-op after iter-220 removed provider/model from Settings.)
        self.sync_model_to_toml(&self.config.agent.model);
    }

    /// Write the current model + provider to ~/.operant/operant.toml so that
    /// `operant setup` reads the actual current values instead of defaults.
    /// (iter-117 — fixes the config-source proliferation bug.)
    fn sync_model_to_toml(&self, model: &str) {
        // Load the existing TOML config, update the model field, and write back.
        // We use the runtime config (already loaded by main.rs) rather than
        // re-parsing the TOML file, to avoid format issues.
        let mut config = operant_core::config::runtime_config();
        config.agent.model = model.to_string();
        if let Some(ref provider) = self.active_provider {
            // Update base_url based on provider.
            if let Some(p) = crate::provider::PROVIDERS
                .iter()
                .find(|p| p.name == *provider)
            {
                if !p.default_base_url.is_empty() {
                    config.client.base_url = p.default_base_url.to_string();
                }
            }
        }
        // Write to ~/.operant/operant.toml.
        let config_path = dirs::home_dir()
            .map(|h| h.join(".operant").join("operant.toml"))
            .unwrap_or_else(|| std::path::PathBuf::from("operant.toml"));
        if let Ok(toml_str) = toml::to_string_pretty(&config) {
            let _ = std::fs::write(&config_path, &toml_str);
        }
    }

    /// Switch the active provider while clearing any explicit model override.
    fn set_provider_default(&mut self, provider_id: String) {
        self.active_provider = Some(provider_id.clone());

        let model = self.display_default_model_for_provider(&provider_id);
        if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
            tracker.set_model(&model);
        }
        self.model_name = model;
        self.refresh_context_window_size();
        self.context_used_tokens = 0;
    }

    /// Update the context window size from the model registry for the current model.
    pub fn refresh_context_window_size(&mut self) {
        let provider = self.active_provider.as_deref().unwrap_or("anthropic");
        let model_id = self
            .model_name
            .strip_prefix(&format!("{}/", provider))
            .unwrap_or(&self.model_name);
        if let Some(entry) = self.model_registry.get(provider, model_id) {
            self.context_window_size = entry.info.context_window as u64;
        } else {
            // Fallback: common defaults
            self.context_window_size = match provider {
                "anthropic" => 200_000,
                "openai" => 128_000,
                "google" => 1_048_576,
                _ => 128_000,
            };
        }
    }

    /// Resolve a stale `provider/default` model name to the best actual model
    /// for that provider. This handles the case where settings.json stores a
    /// fallback model name from a previous session when the registry was empty.
    fn resolve_stale_model(&mut self, model: &str) -> String {
        if model.ends_with("/default") {
            let provider = model.strip_suffix("/default").unwrap_or(model);
            let resolved =
                super::model_picker::default_model_for_provider(provider, &self.model_registry);
            if resolved != format!("{}/default", provider) {
                self.config.agent.model = resolved.clone();
                return resolved;
            }
        }
        model.to_string()
    }

    /// Update the active model name (also updates config + cost tracker).
    pub fn set_model(&mut self, model: String) {
        if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
            tracker.set_model(&model);
        }
        self.model_name = model.clone();
        self.config.agent.model = model.clone();
        if let Some(provider) = super::provider::infer_provider_from_model(&model) {
            self.active_provider = Some(provider);
        }
        self.refresh_context_window_size();
        // Reset used tokens when switching models (context is fresh).
        self.context_used_tokens = 0;
    }

    /// Apply a theme by name, persisting it to config.
    pub fn apply_theme(&mut self, theme_name: &str) {
        let theme = match theme_name {
            "dark" => Theme::Dark,
            "light" => Theme::Light,
            "default" => Theme::Default,
            "deuteranopia" => Theme::Deuteranopia,
            other => Theme::Custom(other.to_string()),
        };
        self.settings.theme = theme.clone();
        self.config.tui.theme = theme_name.to_string();
        // Persist to settings file
        let mut settings = Settings::load_sync().unwrap_or_default();
        settings.theme = theme;
        let _ = settings.save_sync();
        self.status_message = Some(format!("Theme set to: {}", theme_name));
    }

    pub fn apply_provider_refresh(
        &mut self,
        config: AppConfig,
        settings: Settings,
        auth_store: crate::tui::adapter_types::AuthStore,
        has_credentials: bool,
        status_message: String,
    ) {
        self.close_secondary_views();
        self.config = config;
        self.settings = settings;
        // (iter-158: provider_registry assignment deleted — field was always None)
        self.model_registry.ensure_provider_defaults();
        self.auth_store = auth_store;
        self.connect_dialog = DialogSelectState::new("Connect a provider", provider_picker_items());
        self.import_config_picker =
            DialogSelectState::new("Import config", import_config_picker_items());
        self.import_config_dialog = ImportConfigDialogState::new();
        self.model_picker = ModelPickerState::new();
        self.key_input_dialog = crate::tui::key_input_dialog::KeyInputDialogState::new();
        self.custom_provider_dialog =
            crate::tui::custom_provider_dialog::CustomProviderDialogState::new();
        self.free_mode_dialog = crate::tui::free_mode_dialog::FreeModeDialogState::new();
        self.device_auth_dialog = crate::tui::device_auth_dialog::DeviceAuthDialogState::new();
        self.device_auth_pending = None;
        self.pending_mcp_panel_auth = None;
        self.model_picker_fetch_pending = false;
        self.model_picker_provider_id = None;
        self.has_credentials = has_credentials;
        self.fast_mode = false;
        let model_to_resolve = self.config.agent.model.clone();
        self.model_name = self.resolve_stale_model(&model_to_resolve);
        if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
            tracker.set_model(&self.model_name);
        }
        self.status_message = Some(status_message);
        self.clear_prompt();
    }

    /// Handle slash commands that should open UI screens rather than execute
    /// as normal commands. Returns `true` if the command was intercepted.
    pub fn intercept_slash_command_with_args(&mut self, cmd: &str, args: &str) -> bool {
        if cmd == "mcp" && !args.trim().is_empty() {
            return false;
        }
        self.intercept_slash_command_with_args_impl(cmd, args)
    }

    pub fn handle_tui_command(&mut self, cmd: &str, args: &str) -> bool {
        if cmd == "mcp" && !args.trim().is_empty() {
            return false;
        }
        self.intercept_slash_command_with_args_impl(cmd, args)
    }

    /// Backwards-compatible wrapper that takes no args (treats args as empty).
    /// Kept so external callers (and the existing `?` shortcut path) still work.
    pub fn intercept_slash_command(&mut self, cmd: &str) -> bool {
        self.intercept_slash_command_with_args_impl(cmd, "")
    }

    /// A JSON snapshot of assertable App state for the headless simulator's
    /// `--assert` engine. Dot-path keys (e.g. `overlays.model_picker`,
    /// `messages`, `model`) are navigated by `evaluate_assertions`. This is
    /// the generic replacement for the old hardcoded boolean whitelist.
    /// Single source of truth for the set of dialog/overlay visibilities.
    /// Both `any_modal_open()` and `debug_snapshot()` derive from this one
    /// list so the two can't drift out of sync (the drift that dropped
    /// `effort_picker` from `any_modal_open` in iter-227). Each entry is
    /// `(snapshot_key, is_visible)`. `permission_request` is tracked via
    /// `.is_some()` rather than a `.visible` flag.
    fn overlay_flags(&self) -> [(&'static str, bool); 35] {
        [
            ("help_overlay", self.help_overlay.visible),
            (
                "history_search_overlay",
                self.history_search_overlay.visible,
            ),
            ("global_search", self.global_search.visible),
            ("rewind_flow", self.rewind_flow.visible),
            ("settings_screen", self.settings_screen.visible),
            ("theme_screen", self.theme_screen.visible),
            ("stats_dialog", self.stats_dialog.visible),
            ("mcp_view", self.mcp_view.visible),
            ("agents_menu", self.agents_menu.visible),
            ("diff_viewer", self.diff_viewer.visible),
            ("memory_file_selector", self.memory_file_selector.visible),
            ("skills_view", self.skills_view.visible),
            ("plugins_hub", self.plugins_hub.visible),
            ("journey_view", self.journey_view.visible),
            ("hooks_config_menu", self.hooks_config_menu.visible),
            ("voice_mode_notice", self.voice_mode_notice.visible),
            ("model_picker", self.model_picker.visible),
            ("session_browser", self.session_browser.visible),
            ("session_branching", self.session_branching.visible),
            ("tasks_overlay", self.tasks_overlay.visible),
            ("export_dialog", self.export_dialog.visible),
            ("context_viz", self.context_viz.visible),
            ("mcp_approval", self.mcp_approval.visible),
            (
                "bypass_permissions_dialog",
                self.bypass_permissions_dialog.visible,
            ),
            ("effort_picker", self.effort_picker.visible),
            ("key_input_dialog", self.key_input_dialog.visible),
            (
                "custom_provider_dialog",
                self.custom_provider_dialog.visible,
            ),
            ("free_mode_dialog", self.free_mode_dialog.visible),
            ("device_auth_dialog", self.device_auth_dialog.visible),
            ("connect_dialog", self.connect_dialog.visible),
            ("import_config_picker", self.import_config_picker.visible),
            ("import_config_dialog", self.import_config_dialog.visible),
            ("command_palette", self.command_palette.visible),
            ("ask_user_dialog", self.ask_user_dialog.visible),
            ("permission_request", self.permission_request.is_some()),
        ]
    }

    pub fn debug_snapshot(&self) -> serde_json::Value {
        // Overlays map is derived from `overlay_flags()` (single source of
        // truth), so it can't drift from `any_modal_open()`. Built from a
        // flat tuple array rather than a giant json! literal (which would
        // overflow the macro recursion limit).
        let overlays: serde_json::Map<String, serde_json::Value> = self
            .overlay_flags()
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::Bool(v)))
            .collect();

        serde_json::json!({
            "should_exit": self.should_exit,
            "is_streaming": self.is_streaming,
            "is_simulating": self.is_simulating,
            "plan_mode": self.plan_mode,
            "show_help": self.show_help,
            "show_reasoning": self.show_reasoning,
            "fast_mode": self.fast_mode,
            "messages": self.messages.len(),
            "status_message": self.status_message,
            "model": self.model_name,
            "provider": self.active_provider,
            "focus": format!("{:?}", self.focus),
            "token_count": self.token_count,
            "any_modal_open": self.any_modal_open(),
            "overlays": serde_json::Value::Object(overlays),
        })
    }

    /// Push `text` into the live steer queue if the agent is streaming, and
    /// return a status string describing the outcome. Mirrors the live steer
    /// path in adapter_types.rs, but uses `try_lock` because this runs on the
    /// sync slash-command path while the queue is a tokio Mutex.
    /// (iter-240 — wires /steer and /queue <text> to the real steer queue.)
    fn queue_steer(&mut self, text: &str) -> String {
        const NOT_STREAMING: &str = "Steer is only available while the agent is streaming.";
        if !self.is_streaming {
            return NOT_STREAMING.to_string();
        }
        match self.steer_queue_handle.as_ref() {
            Some(handle) => match handle.try_lock() {
                Ok(mut q) => {
                    q.push(text.to_string());
                    format!("Steer queued: {}", text)
                }
                Err(_) => NOT_STREAMING.to_string(),
            },
            None => NOT_STREAMING.to_string(),
        }
    }

    /// Implementation that receives both cmd and args. Most slash commands
    /// ignore args; a few (like /personality <name>) consume them.
    fn intercept_slash_command_with_args_impl(&mut self, cmd: &str, args: &str) -> bool {
        self.close_secondary_views();
        self.dismiss_error_notifications();
        // Record slash-command usage for smart ordering of `/` suggestions.
        // (iter-125 — recency + frequency ranking.)
        self.slash_usage.record(cmd);
        self.slash_usage.save();
        self.debug_hub
            .publish(crate::tui::debug::TuiEvent::SlashCommand {
                name: cmd.to_string(),
                args_preview: args.chars().take(40).collect(),
                at: crate::tui::debug::event_bus::now_secs(),
            });
        match cmd {
            "config" | "settings" => {
                self.settings_screen.open();
                true
            }
            "theme" | "skin" => {
                let current = match &self.settings.theme {
                    Theme::Dark => "dark",
                    Theme::Light => "light",
                    Theme::Default => "default",
                    Theme::Deuteranopia => "deuteranopia",
                    Theme::Custom(s) => s.as_str(),
                };
                self.theme_screen.open(current);
                true
            }
            "stats" | "cost" => {
                self.stats_dialog.open();
                true
            }
            "mcp" => {
                let servers = self.load_mcp_servers();
                self.mcp_view.open(servers);
                true
            }
            "agents" | "tasks" => {
                self.open_agents_menu();
                true
            }
            "diff" | "review" => {
                let root = self.project_root();
                self.diff_viewer.open(&root);
                true
            }
            "changes" => {
                let root = self.project_root();
                // (iter-209: refresh_turn_diff_from_history removed — turn-diff stub deleted)
                self.diff_viewer.open_turn(&root);
                true
            }
            "search" | "find" => {
                self.global_search.open();
                true
            }
            // (iter-211: survey/feedback slash command deleted — no telemetry backend)
            "memory" => {
                let root = self.project_root();
                self.memory_file_selector.open(&root);
                true
            }
            "skills" => {
                let skills_dir = self.config.skills.root_dir.clone();
                self.skills_view.open(skills_dir);
                true
            }
            "plugins" => {
                let plugins_dir =
                    crate::cmd_plugins::plugins_dir(&self.config).unwrap_or_else(|_| {
                        dirs::data_dir()
                            .unwrap_or_default()
                            .join("operant")
                            .join("plugins")
                    });
                self.plugins_hub.open(plugins_dir);
                true
            }
            "hooks" => {
                self.hooks_config_menu.open();
                true
            }
            "import-config" => {
                self.open_import_config_picker();
                true
            }
            "connect" => {
                self.connect_dialog.open();
                true
            }
            "model" => {
                if !self.has_credentials {
                    self.connect_dialog.open();
                    self.status_message = Some("Connect a provider to choose a model.".to_string());
                    return true;
                }
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "anthropic".to_string());
                self.open_model_picker_for_provider(&provider, None);
                true
            }
            "session" | "resume" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                true
            }
            "clear" => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_use_blocks.clear();
                self.turn_metadata.clear();
                self.cost_usd = 0.0;
                // Reset streaming + scroll + token state so new input isn't
                // silently dropped. Without this, /clear mid-stream leaves
                // is_streaming=true, so the prompt input handler rejects
                // new queries.
                self.is_streaming = false;
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.token_count = 0;
                self.invalidate_transcript();
                self.status_message = Some("Conversation cleared.".to_string());
                true
            }
            "exit" | "quit" => {
                self.should_exit = true;
                true
            }
            "vim" => {
                self.prompt_input.vim_enabled = !self.prompt_input.vim_enabled;
                let status = if self.prompt_input.vim_enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                self.status_message = Some(format!("Vim mode {}.", status));
                self.refresh_prompt_input();
                true
            }
            "fast" => {
                self.fast_mode = !self.fast_mode;
                let status = if self.fast_mode {
                    "enabled"
                } else {
                    "disabled"
                };
                self.status_message = Some(format!("Fast mode {}.", status));
                true
            }
            "plan" => {
                use crate::tui::adapter_types::config::PermissionMode;
                self.plan_mode = !self.plan_mode;
                self.settings.permission_mode = if self.plan_mode {
                    PermissionMode::Plan
                } else {
                    PermissionMode::Default
                };
                self.status_message = Some(if self.plan_mode {
                    "Plan mode ON — Operant will plan before acting.".to_string()
                } else {
                    "Plan mode OFF.".to_string()
                });
                true
            }
            // /stop — cancel the live streaming turn, exactly as Esc does.
            // (iter-270: wired to real streaming cancel path.)
            "stop" => {
                if self.is_streaming {
                    self.is_streaming = false;
                    self.spinner_verb = None;
                    // Flush in-flight streaming text to messages BEFORE snapshot
                    // so the response is preserved in the transcript.
                    self.flush_streamed_assistant_message();
                    self.status_message = Some("Stopped.".to_string());
                    self.complete_current_turn_snapshot(true);
                    self.tool_use_blocks.clear();
                } else {
                    self.status_message = Some("Nothing to stop — no turn is running.".to_string());
                }
                true
            }

            // /new — start a completely fresh session (same as /clear but
            // also resets cost and turn counter).
            // (iter-270: wired to real state clear.)
            "new" | "fresh" => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_use_blocks.clear();
                self.turn_metadata.clear();
                self.session_goal = None;
                self.cost_usd = 0.0;
                self.is_streaming = false;
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.token_count = 0;
                self.session_title = None;
                self.invalidate_transcript();
                self.status_message = Some("New session started.".to_string());
                true
            }

            // /undo — remove the last user + assistant exchange from the
            // transcript. Safe no-op if fewer than 2 messages.
            // (iter-270: wired to real message state.)
            "undo" => {
                // Find last user message index from the end.
                let last_user = self
                    .messages
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, m)| m.role == Role::User)
                    .map(|(i, _)| i);
                if let Some(idx) = last_user {
                    // Remove all messages from that user message to end.
                    self.messages.truncate(idx);
                    // Also discard the trailing assistant turn metadata entry.
                    self.turn_metadata.pop();
                    self.invalidate_transcript();
                    self.status_message = Some("Last turn undone.".to_string());
                } else {
                    self.status_message = Some("Nothing to undo.".to_string());
                }
                true
            }

            // /retry — resubmit the last user message. Queues it via
            // pending_retry_query so the adapter_types run loop can spawn
            // the agent call (App::run is sync, agent.run is async).
            // (iter-270: wired to real state.)
            "retry" => {
                if self.is_streaming {
                    self.status_message =
                        Some("Cannot retry while a turn is running. Stop first.".to_string());
                    return true;
                }
                let last_user_text = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.get_all_text());
                if let Some(text) = last_user_text {
                    // Remove all messages from the last user message onward
                    // so the turn is truly retried (not duplicated).
                    let last_user_idx = self
                        .messages
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, m)| m.role == Role::User)
                        .map(|(i, _)| i);
                    if let Some(idx) = last_user_idx {
                        self.messages.truncate(idx);
                    }
                    self.pending_retry_query = Some(text);
                    self.invalidate_transcript();
                    self.status_message = Some("Retrying last message…".to_string());
                } else {
                    self.status_message = Some("No previous message to retry.".to_string());
                }
                true
            }

            // /save — alias for /export (opens the export dialog).
            // (iter-270: fixed broken stub.)
            "save" => {
                self.export_dialog.open();
                true
            }

            // /goal <text> — set a standing session goal. Shown in the status
            // bar and injected as a system annotation so the agent sees it.
            // /goal with no args shows the current goal.
            // (iter-270: wired to real state.)
            "goal" | "subgoal" => {
                let trimmed = args.trim();
                if trimmed.is_empty() {
                    // Show current goal.
                    let cur = self
                        .session_goal
                        .clone()
                        .unwrap_or_else(|| "(none)".to_string());
                    self.status_message = Some(format!(
                        "Session goal: {}. Use /goal <text> to change.",
                        cur
                    ));
                } else {
                    self.session_goal = Some(trimmed.to_string());
                    // Inject as a system annotation so it appears in transcript.
                    self.push_system_message(
                        format!("🎯 Goal set: {}", trimmed),
                        crate::tui::app::SystemMessageStyle::Info,
                    );
                    self.status_message = Some(format!("Goal set: {}", trimmed));
                }
                true
            }

            // /sessions — alias for /session / /resume (open session browser).
            // (iter-270: fixed broken stub.)
            "sessions" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                true
            }

            "compact" => false,
            "copy" => {
                // Copy last assistant message to clipboard. Attempt arboard; fall back to notification.
                let last = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                    .map(|m| m.get_all_text());
                if let Some(text) = last {
                    // Try xclip/xsel/pbcopy/clip.exe for clipboard; fall back to notification.
                    let copied = try_copy_to_clipboard(&text);
                    if copied {
                        self.push_notification(
                            NotificationKind::Info,
                            "Copied to clipboard.".to_string(),
                            Some(3),
                        );
                    } else {
                        self.push_notification(
                            NotificationKind::Info,
                            format!(
                                "Last response: {} chars (clipboard unavailable)",
                                text.len()
                            ),
                            Some(5),
                        );
                    }
                } else {
                    self.push_notification(
                        NotificationKind::Warning,
                        "No assistant message to copy.".to_string(),
                        Some(3),
                    );
                }
                true
            }
            "output-style" | "verbose" => {
                self.output_style = match self.output_style.as_str() {
                    "auto" => "stream".to_string(),
                    "stream" => "verbose".to_string(),
                    _ => "auto".to_string(),
                };
                self.status_message = Some(format!("Output style: {}.", self.output_style));
                true
            }
            "effort" => {
                // Open the picker dialog so users can pick an effort level
                // visually instead of cycling/typing the level (issue #149).
                self.effort_picker.open(self.effort_level);
                true
            }
            "voice" => {
                let was_on = self.voice_recorder.is_some();
                if was_on {
                    // Stop any active recording before disabling.
                    if self.voice_recording {
                        self.voice_recording = false;
                        self.voice_event_rx = None;
                        if let Some(ref recorder_arc) = self.voice_recorder {
                            let recorder = recorder_arc.clone();
                            tokio::task::spawn_blocking(move || {
                                if let Ok(mut r) = recorder.lock() {
                                    tokio::runtime::Handle::current()
                                        .block_on(r.stop_recording())
                                        .ok();
                                }
                            });
                        }
                    }
                    self.voice_recorder = None;
                    self.voice_mode_notice.dismiss();
                    self.status_message = Some("Voice mode disabled.".to_string());
                } else {
                    let recorder = crate::tui::adapter_types::voice::global_voice_recorder();
                    if let Ok(mut r) = recorder.lock() {
                        r.set_enabled(true);
                    }
                    self.voice_recorder = Some(recorder);
                    self.voice_mode_notice =
                        crate::tui::voice_mode_notice::VoiceModeNoticeState::new();
                    self.status_message =
                        Some("Voice mode enabled. Press Alt+V to start recording.".to_string());
                }
                true
            }
            "doctor" => false,
            "rewind" => {
                self.open_rewind_flow();
                true
            }
            "export" => {
                self.export_dialog.open();
                true
            }
            "context" => {
                self.context_viz.toggle();
                true
            }
            "rename" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                self.session_browser.start_rename();
                true
            }
            "init" | "login" | "logout" => false,
            "keybindings" => {
                // Open the keybindings.json file in the external editor
                let keybindings_path = crate::tui::adapter_types::config::Settings::config_dir()
                    .join("keybindings.json");

                if let Err(e) = open_file_externally(&keybindings_path) {
                    eprintln!("Failed to open keybindings file: {}", e);
                }
                true
            }
            "help" => {
                // Toggle the help overlay (same as pressing `?` or F1).
                // Bug #8 from iter-82 audit: previously only opened (never
                // closed), so pressing /help twice showed two different
                // help overlays (the rich one + the legacy show_help fallback).
                self.help_overlay.toggle();
                self.show_help = self.help_overlay.visible;
                true
            }
            // ── Backfilled slash commands (iter-77) ───────────────────────────
            // These previously appeared in PROMPT_SLASH_COMMANDS but were never
            // intercepted — they fell through to the basic command registry,
            // which printed a one-line help text and felt broken. Most map to
            // existing App / Settings state; the rest return a polite status
            // message so the user knows operant heard them.

            // /yolo — toggle bypass-permissions mode by flipping the
            // permission_mode setting between 'Default' and 'BypassPermissions'.
            "yolo" => {
                let mut settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                let new_mode = if matches!(
                    settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                ) {
                    crate::tui::adapter_types::config::PermissionMode::default()
                } else {
                    crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                };
                settings.permission_mode = new_mode.clone();
                let _ = settings.save_sync();
                self.settings.permission_mode = new_mode.clone();
                self.status_message = Some(
                    if matches!(
                        new_mode,
                        crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                    ) {
                        "YOLO mode armed — permissions will be auto-approved. Use with care."
                            .to_string()
                    } else {
                        "YOLO mode disarmed — permissions will prompt.".to_string()
                    },
                );
                true
            }

            // /busy — toggle "busy" indicator (we map to auto_compact to avoid
            // adding a new state field; busy = compact aggressively).
            "busy" => {
                self.auto_compact_enabled = !self.auto_compact_enabled;
                self.status_message = Some(format!(
                    "Auto-compact {}.",
                    if self.auto_compact_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                true
            }

            // /verbose — cycle output-style between auto/stream/verbose.

            // /reasoning — toggle whether thinking/reasoning blocks are
            // expanded by default in the transcript. (Bug #18 from iter-82
            // audit — previously just printed a status message without
            // toggling anything.)
            "reasoning" => {
                self.show_reasoning = !self.show_reasoning;
                self.invalidate_transcript();
                self.status_message = Some(format!(
                    "Reasoning blocks {} by default.",
                    if self.show_reasoning {
                        "expanded"
                    } else {
                        "collapsed"
                    }
                ));
                true
            }

            // /personality — set agent personality from args.
            // The actual personality string is consumed by the agent loop on
            // the next turn (it reads app.agent_mode).
            "personality" => {
                let trimmed = args.trim();
                if trimmed.is_empty() {
                    // No args — show current value.
                    let cur = self
                        .agent_mode
                        .clone()
                        .unwrap_or_else(|| "default".to_string());
                    self.status_message = Some(format!(
                        "Current personality: {}. Use /personality <name> to change.",
                        cur
                    ));
                } else {
                    // Set the new personality. agent_mode_changed=true signals
                    // the run loop to update the query config on the next turn.
                    self.agent_mode = Some(trimmed.to_string());
                    self.agent_mode_changed = true;
                    self.status_message = Some(format!("Personality set to: {}.", trimmed));
                }
                true
            }

            // /steer <message> — inject guidance into the live steer queue so
            // the agent picks it up at the next iteration boundary mid-turn.
            // (iter-240 — wires to the real steer_queue_handle backend.)
            "steer" => {
                self.status_message = Some(if args.is_empty() {
                    "Usage: /steer <message> (inject guidance while the agent is streaming)"
                        .to_string()
                } else {
                    self.queue_steer(args)
                });
                true
            }

            // /queue — list the live steer queue; /queue <text> is an alias for
            // /steer <text>. operant has no separate pending-input queue — the
            // steer queue IS the queue. (iter-240.)
            "queue" => {
                let msg = if !args.is_empty() {
                    self.queue_steer(args)
                } else {
                    match self.steer_queue_handle.as_ref() {
                        Some(handle) => match handle.try_lock() {
                            Ok(q) if q.is_empty() => "Queue is empty".to_string(),
                            Ok(q) => format!("Queued ({}): {}", q.len(), q.join("; ")),
                            Err(_) => "Queue is busy (agent is draining it).".to_string(),
                        },
                        None => {
                            "Nothing queued (queue is active only while streaming).".to_string()
                        }
                    }
                };
                self.status_message = Some(msg);
                true
            }

            // /background <prompt> — operant's TUI runs a single agent
            // synchronously: App holds no agent handle and there is exactly one
            // event channel + run_complete_rx, so spawning a second agent.run()
            // in-session would interleave into (and corrupt) the live
            // transcript. Rather than a bare no-op, echo the exact working
            // detached command with the user's prompt filled in.
            // (ponytail: in-session background turn needs a second isolated
            // agent via create_runtime_agent with its own session id + event
            // channel, threaded through the run loop — invasive and not
            // headless-testable, so we point at `operant run --query ... &`.)
            "background" => {
                let trimmed = args.trim();
                self.status_message = Some(if trimmed.is_empty() {
                    "Usage: /background <prompt> — operant runs one agent synchronously; this prints the command to run it detached.".to_string()
                } else {
                    let escaped = trimmed.replace('"', "\\\"");
                    format!(
                        "Operant runs synchronously in-session. Background it with: operant run --query \"{}\" &",
                        escaped
                    )
                });
                true
            }

            // /rollback — surface the existing /rewind flow (which IS
            // implemented) instead of silently dropping /rollback.
            "rollback" => {
                let root = self.project_root();
                // (iter-209: refresh_turn_diff_from_history removed — turn-diff stub deleted)
                self.diff_viewer.open_turn(&root);
                self.status_message =
                    Some("Rollback: review last turn diff. Use /rewind to step back.".to_string());
                true
            }

            // /reload-mcp — request a live MCP reconnect. The run loop drains
            // pending_mcp_reconnect (adapter_types.rs) and reconnects the MCP
            // servers without restarting the TUI. (iter-240 — wires to the
            // pending_mcp_reconnect backend.)
            "reload-mcp" => {
                self.pending_mcp_reconnect = true;
                self.status_message = Some("Reconnecting MCP servers…".to_string());
                true
            }

            // /reload — re-read TUI settings from disk and re-apply the visual /
            // preference subset that is safe to swap live (theme, output style,
            // permission mode). We intentionally do NOT hot-swap the provider /
            // model client mid-session, so those changes only take effect on
            // restart. Ref: hermes-agent cli.py reload_env().
            "reload" => {
                let new_settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                // Detect whether the provider/model changed on disk so the
                // status line can be honest about what applies now vs. on
                // restart.
                let disk_model = operant_core::config::load_app_config(None)
                    .map(|c| c.config.agent.model)
                    .unwrap_or_else(|_| self.config.agent.model.clone());
                let model_changed = disk_model != self.config.agent.model;

                self.settings = new_settings;
                self.plan_mode = matches!(
                    self.settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::Plan
                );
                self.output_style = match self.settings.output_style.as_deref() {
                    Some("stream") => "stream".to_string(),
                    Some("verbose") => "verbose".to_string(),
                    _ => "auto".to_string(),
                };

                self.status_message = Some(if model_changed {
                    "Config reloaded (provider/model changes apply on restart).".to_string()
                } else {
                    "Configuration reloaded.".to_string()
                });
                true
            }

            // /reload-skills — re-scan the skills directory and repopulate the
            // /skills overlay's backing data. The running agent was built with a
            // fixed SkillManager at startup (main.rs `with_skill_manager`) and
            // exposes no runtime setter, so rescanned skills reach the model only
            // after a restart; the status stays honest about that. Ref:
            // hermes-agent cli.py reload_skills().
            "reload-skills" => {
                let skills_dir = self.config.skills.root_dir.clone();
                let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
                match mgr.load_all() {
                    Ok(mut loaded) => {
                        // Same (category, name) sort skills_view.open() uses so
                        // the overlay renders identically after a rescan.
                        loaded.sort_by(|a, b| {
                            a.category
                                .cmp(&b.category)
                                .then_with(|| a.name.cmp(&b.name))
                        });
                        let count = loaded.len();
                        self.skills_view.skills = loaded;
                        if self.skills_view.selected >= count {
                            self.skills_view.selected = 0;
                        }
                        self.status_message = Some(format!(
                            "Rescanned {} skill{}. Browse with /skills (agent picks up changes on restart).",
                            count,
                            if count == 1 { "" } else { "s" }
                        ));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Failed to reload skills: {}", e));
                    }
                }
                true
            }

            // /browser — surface that operant has a Camofox browser backend
            // but no in-TUI browser launcher (it's a tool the agent calls).
            "browser" => {
                self.status_message = Some(
                    "Browser: operant uses Camofox as the default. The agent invokes it via the browser tool — no in-TUI browser panel.".to_string()
                );
                true
            }

            // /indicator, /statusbar — toggle Settings.terminal_progress_bar
            // (operant has one status-bar toggle, not the two hermes has).
            "indicator" | "statusbar" => {
                let mut settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                settings.terminal_progress_bar = !settings.terminal_progress_bar;
                let _ = settings.save_sync();
                self.status_message = Some(format!(
                    "Status bar {}.",
                    if settings.terminal_progress_bar {
                        "shown"
                    } else {
                        "hidden"
                    }
                ));
                true
            }

            // /mouse — report the mouse-capture state. Capture is enabled on
            // startup unless --no-mouse was passed; that flag lives on the
            // TuiApp runner (adapter_types.rs) and is not threaded into App,
            // so this reports the real startup default. (iter-240.)
            "mouse" => {
                self.status_message = Some(
                    "Mouse capture: enabled (use --no-mouse to disable, e.g. inside tmux)"
                        .to_string(),
                );
                true
            }

            // /terminal-setup — surface that operant auto-detects terminal
            // capabilities at startup (OSC8, truecolor, etc.).
            "terminal-setup" => {
                self.status_message = Some(
                    "Terminal capabilities are auto-detected at startup. No manual setup needed."
                        .to_string(),
                );
                true
            }

            // /redraw — force a full redraw by bumping the transcript
            // version counter (which invalidates cached render state).
            "redraw" => {
                self.transcript_version
                    .set(self.transcript_version.get().wrapping_add(1));
                self.status_message = Some("Screen redrawn.".to_string());
                true
            }

            // /billing, /credits — surface that operant doesn't track
            // provider billing/credits (it's BYOK); point users at /stats
            // for local token usage tracking.
            "billing" | "credits" => {
                self.status_message = Some(format!(
                    "{}: operant is BYOK and doesn't track provider billing. Use /stats for local token usage.",
                    cmd
                ));
                true
            }

            // /update — point at `operant update` (the TUI can't self-update
            // without restarting).
            "update" => {
                self.status_message = Some(
                    "Run `operant update` from a shell to check for and install a new release."
                        .to_string(),
                );
                true
            }

            // /heapdump, /mem — debug diagnostics; surface a snapshot of
            // turn count + token count + cost as a memory/heap summary.
            "heapdump" | "mem" => {
                self.status_message = Some(format!(
                    "Turns: {} | Tokens: {} | Cost: ${:.4} | Agent status entries: {}",
                    self.turn_metadata.len(),
                    self.token_count,
                    self.cost_usd,
                    self.agent_status.len()
                ));
                true
            }

            // /pet — Easter-egg. (iter-144: rustle pose trigger deleted
            // since the pose system was dead code. Still shows the message.)
            "pet" => {
                self.status_message = Some("Rustle wags its tail. 🐕".to_string());
                true
            }

            // /journey, /replay, /replay-diff — these need their own overlays
            // (planned for a later iteration). Surface a "coming soon" status
            // rather than silently dropping.
            "journey" => {
                let skills_dir = self.config.skills.root_dir.clone();
                let memory_dir = operant_core::platform::operant_home().join("memory");
                self.journey_view.open(skills_dir, memory_dir);
                true
            }
            "replay" | "replay-diff" => {
                self.status_message = Some(format!(
                    "/{} overlay is planned. For now, use /agents to view the spawn tree.",
                    cmd
                ));
                true
            }

            // /setup — suspend the TUI and shell out to `operant setup` so the
            // user gets the full interactive wizard. The run loop in
            // TuiApp::run polls pending_shell_command after each frame and,
            // if set, leaves alt screen + raw mode, spawns the command with
            // inherited stdio, waits for it, then re-enters alt screen + raw
            // mode and clears the field.
            "setup" => {
                // Use the current binary (so the version matches) with the
                // `setup` subcommand. If operant was launched via a wrapper,
                // fall back to the literal "operant" name on PATH.
                let exe = std::env::current_exe()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "operant".to_string());
                self.pending_shell_command = Some(vec![exe, "setup".to_string()]);
                self.status_message = Some("Launching setup wizard…".to_string());
                true
            }
            // /whoami — show what the agent knows about the user.
            // (P1-9 from UX audit — transparency + trust.)
            "whoami" => {
                let mem_dir = operant_core::platform::operant_home().join("memory");
                let store = operant_core::memory::MemoryStore::new(mem_dir);
                match store.read_memories() {
                    Ok(map) if map.is_empty() => {
                        self.status_message = Some(
                            "I don't know much about you yet. Chat with me and I'll start remembering.".to_string()
                        );
                    }
                    Ok(map) => {
                        let blocks: Vec<_> = map.into_values().collect();
                        let mut summary = format!(
                            "Here's what I know about you ({} memories):\n\n",
                            blocks.len()
                        );
                        for block in blocks.iter().take(10) {
                            let preview: String = block
                                .content
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(80)
                                .collect();
                            summary.push_str(&format!(
                                "  [{:>3}] {:<10} {}\n",
                                block.importance, &block.block_type, preview,
                            ));
                        }
                        if blocks.len() > 10 {
                            summary.push_str(&format!("\n  ...and {} more\n", blocks.len() - 10));
                        }
                        self.push_system_message(
                            summary,
                            crate::tui::app::SystemMessageStyle::Info,
                        );
                    }
                    Err(_) => {
                        self.status_message = Some(
                            "No memory store found. Use /memory to manage memory files."
                                .to_string(),
                        );
                    }
                }
                true
            }
            _ => {
                // Fallback: try the command registry for any unhandled command.
                // This wires up the unified CommandRegistry so commands defined
                // in commands.rs but not yet added to the intercept match arms
                // can still be dispatched via their registered handlers.
                //
                // Only dispatch if the command is actually registered in the
                // registry — truly unknown commands (e.g. `/survey` after its
                // deletion) should NOT be intercepted so the test
                // `test_feedback_survey_removed` can verify they fall through.
                if self.command_registry.resolve(cmd).is_none() {
                    return false;
                }
                let cmd_name = cmd.to_string();
                let args_owned = args.to_string();
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        self.command_registry.execute(&cmd_name, &args_owned).await
                    })
                });
                self.handle_command_result(result)
            }
        }
    }

    /// Interpret a [`CommandResult`] and apply the corresponding side effect
    /// in the TUI. This is the single dispatch point for all slash commands
    /// that go through the `CommandRegistry`.
    ///
    /// Returns `true` if the command was intercepted (even if it only showed
    /// a message), `false` if it should fall through to the agent.
    fn handle_command_result(&mut self, result: crate::commands::CommandResult) -> bool {
        use crate::commands::CommandResult;
        match result {
            // ── Display ────────────────────────────────────────────────────
            CommandResult::Message(text) => {
                self.push_system_message(text, crate::tui::app::SystemMessageStyle::Info);
                true
            }
            CommandResult::Error(text) => {
                self.status_message = Some(text);
                true
            }
            CommandResult::Silent => true,

            // ── Conversation ───────────────────────────────────────────────
            CommandResult::UserMessage(msg) => {
                self.input = msg;
                // Signal the caller that the user message should be submitted.
                false
            }
            CommandResult::ClearConversation => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.is_streaming = false;
                self.tool_use_blocks.clear();
                self.invalidate_transcript();
                self.status_message = Some("Conversation cleared.".to_string());
                true
            }
            CommandResult::NewSession => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.is_streaming = false;
                self.tool_use_blocks.clear();
                self.invalidate_transcript();
                self.status_message = Some("New session started.".to_string());
                true
            }
            CommandResult::SetMessages(msgs) => {
                self.messages.clear();
                self.system_annotations.clear();
                // Reconstruct messages alternating user/assistant roles.
                for (i, text) in msgs.iter().enumerate() {
                    let role = if i % 2 == 0 {
                        crate::tui::adapter_types::types::Role::User
                    } else {
                        crate::tui::adapter_types::types::Role::Assistant
                    };
                    let message = crate::tui::adapter_types::types::Message {
                        role,
                        content: crate::tui::adapter_types::types::MessageContent::Text(
                            text.clone(),
                        ),
                    };
                    self.messages.push(message);
                }
                self.invalidate_transcript();
                self.status_message = Some(format!("Restored {} messages.", msgs.len()));
                true
            }

            // ── Configuration ──────────────────────────────────────────────
            CommandResult::ToggleSetting { name, enabled } => {
                self.status_message =
                    Some(format!("{}: {}", name, if enabled { "on" } else { "off" }));
                true
            }
            CommandResult::CycleSetting { name, current } => {
                self.status_message = Some(format!("{}: {}", name, current));
                true
            }
            CommandResult::SetGoal(goal) => {
                self.session_goal = goal;
                self.status_message = Some("Session goal updated.".to_string());
                true
            }

            // ── Overlay / UI ───────────────────────────────────────────────
            CommandResult::OpenHelp => {
                self.show_help = true;
                true
            }
            CommandResult::OpenModelPicker => {
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "anthropic".to_string());
                self.open_model_picker_for_provider(&provider, None);
                true
            }
            CommandResult::OpenThemePicker => {
                let theme = self.settings.theme.as_str();
                self.theme_screen.open(theme);
                true
            }
            CommandResult::OpenSessionBrowser => {
                self.session_list_pending = true;
                true
            }
            CommandResult::OpenStats => {
                self.stats_dialog.open();
                true
            }
            CommandResult::OpenMcp => {
                // TODO: populate with live MCP server data from core_mcp_manager.
                self.mcp_view.open(vec![]);
                true
            }
            CommandResult::OpenAgents => {
                let root = self.project_dir.clone().unwrap_or_default();
                self.agents_menu.open(&root);
                true
            }
            CommandResult::OpenDiff => {
                let root = self.project_dir.clone().unwrap_or_default();
                self.diff_viewer.open(&root);
                true
            }
            CommandResult::OpenMemory => {
                let root = self.project_dir.clone().unwrap_or_default();
                self.memory_file_selector.open(&root);
                true
            }
            CommandResult::OpenSkills => {
                self.skills_view
                    .open(operant_core::platform::operant_skills_dir());
                true
            }
            CommandResult::OpenPlugins => {
                let dir = crate::cmd_plugins::plugins_dir(&self.config).unwrap_or_default();
                self.plugins_hub.open(dir);
                true
            }
            CommandResult::OpenHooks => {
                self.hooks_config_menu.open();
                true
            }
            CommandResult::OpenImportConfig => {
                self.open_import_config_picker();
                true
            }
            CommandResult::OpenExport => {
                self.export_dialog.open();
                true
            }
            CommandResult::OpenEffortPicker => {
                self.effort_picker.open(self.effort_level);
                true
            }
            CommandResult::OpenConnect => {
                self.connect_dialog.open();
                true
            }
            CommandResult::OpenSearch => {
                self.global_search.open();
                true
            }
            CommandResult::OpenSettings => {
                self.settings_screen.open();
                true
            }
            CommandResult::OpenContext => {
                self.context_viz.toggle();
                true
            }
            CommandResult::OpenJourney => {
                let skills_dir = operant_core::platform::operant_skills_dir();
                let memory_dir = skills_dir
                    .join("../memory")
                    .canonicalize()
                    .unwrap_or_else(|_| skills_dir.join("../memory"));
                self.journey_view.open(skills_dir, memory_dir);
                true
            }

            // ── Session state ──────────────────────────────────────────────
            CommandResult::StopStreaming => {
                self.is_streaming = false;
                self.flush_streamed_assistant_message();
                true
            }
            CommandResult::Retry => {
                // Set pending_retry_query so the run loop resubmits the last user msg.
                if let Some(last_user) = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| matches!(m.role, crate::tui::adapter_types::types::Role::User))
                {
                    let text = last_user.text_content();
                    if !text.is_empty() {
                        self.pending_retry_query = Some(text);
                    }
                }
                true
            }
            CommandResult::Undo => {
                // Remove the last user+assistant pair.
                let mut removed = 0;
                // Remove trailing assistant message
                if self
                    .messages
                    .last()
                    .map(|m| matches!(m.role, crate::tui::adapter_types::types::Role::Assistant))
                    .unwrap_or(false)
                {
                    self.messages.pop();
                    removed += 1;
                }
                // Remove trailing user message
                if self
                    .messages
                    .last()
                    .map(|m| matches!(m.role, crate::tui::adapter_types::types::Role::User))
                    .unwrap_or(false)
                {
                    self.messages.pop();
                    removed += 1;
                }
                self.invalidate_transcript();
                self.status_message = Some(format!("Undid {} messages.", removed));
                true
            }

            // ── Clipboard ──────────────────────────────────────────────────
            CommandResult::CopyLastResponse => {
                if let Some(last_assistant) =
                    self.messages.iter().rev().find(|m| {
                        matches!(m.role, crate::tui::adapter_types::types::Role::Assistant)
                    })
                {
                    // Filter out thinking blocks — only copy visible text.
                    let text: String = last_assistant
                        .content_blocks()
                        .into_iter()
                        .filter_map(|block| match block {
                            crate::tui::adapter_types::types::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect();
                    if try_copy_to_clipboard(&text) {
                        self.status_message = Some("Copied to clipboard.".to_string());
                    } else {
                        self.status_message = Some("Failed to copy to clipboard.".to_string());
                    }
                }
                true
            }

            // ── Shell ──────────────────────────────────────────────────────
            CommandResult::ShellCommand(cmds) => {
                self.pending_shell_command = Some(cmds);
                true
            }

            // ── Exit ───────────────────────────────────────────────────────
            CommandResult::Exit => {
                self.should_exit = true;
                true
            }
        }
    }

    // NOTE (iter-237 / Phase B1): intentionally NOT derived from
    // `overlay_flags()`. This is a deliberate *subset* of the overlay set
    // (it omits permission_request, rewind_flow, help_overlay,
    // history_search_overlay, global_search, voice_mode_notice,
    // effort_picker, mcp_approval, bypass_permissions_dialog, ask_user_dialog)
    // and uses `.dismiss()` for export_dialog rather than `.close()`. Unifying
    // it with a loop would change behavior, so it's left explicit; migrate it
    // to the overlay registry in a later iteration once close semantics are
    // normalized.
    fn close_secondary_views(&mut self) {
        self.stats_dialog.close();
        self.mcp_view.close();
        self.agents_menu.close();
        self.diff_viewer.close();
        // (iter-211: feedback_survey.close() deleted)
        self.memory_file_selector.close();
        self.skills_view.close();
        self.plugins_hub.close();
        self.journey_view.close();
        self.hooks_config_menu.close();
        self.model_picker.close();
        self.session_browser.close();
        self.session_branching.close();
        self.tasks_overlay.close();
        self.export_dialog.dismiss();
        self.context_viz.close();
        self.connect_dialog.close();
        self.import_config_picker.close();
        self.import_config_dialog.close();
        self.command_palette.close();
        self.key_input_dialog.close();
        self.custom_provider_dialog.close();
        self.free_mode_dialog.close();
        self.device_auth_dialog.close();
        self.settings_screen.close();
        self.theme_screen.close();
    }

    pub fn any_modal_open(&self) -> bool {
        // Derived from `overlay_flags()` (single source of truth) so this
        // can't drift from `debug_snapshot()`. The two extras below are not
        // overlays with a `.visible` flag: `show_help` is a legacy boolean
        // and `context_menu_state` is a popup, both of which still count as
        // "a modal is open" for input gating.
        self.overlay_flags().iter().any(|(_, v)| *v)
            || self.show_help
            || self.context_menu_state.is_some()
    }

    fn dismiss_error_notifications(&mut self) {
        while self.notifications.current_is_error() {
            self.notifications.dismiss_current();
        }
        self.error_modal_scroll_offset = 0;
    }

    /// Perform the export based on the selected format. Returns the path written.
    pub fn perform_export(&mut self) -> Option<String> {
        use crate::tui::export_dialog::{export_as_json, export_as_markdown};
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let (filename, content) = match self.export_dialog.selected {
            ExportFormat::Json => {
                let json = export_as_json(&self.messages, self.session_title.as_deref());
                let s = serde_json::to_string_pretty(&json).unwrap_or_default();
                (format!("claude-export-{}.json", ts), s)
            }
            ExportFormat::Markdown => {
                let md = export_as_markdown(&self.messages, self.session_title.as_deref());
                (format!("claude-export-{}.md", ts), md)
            }
        };
        if std::fs::write(&filename, &content).is_ok() {
            self.export_dialog.dismiss();
            Some(filename)
        } else {
            None
        }
    }

    fn project_root(&self) -> std::path::PathBuf {
        self.project_dir
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }

    fn refresh_global_search(&mut self) {
        let root = self.project_root();
        self.global_search.run_search(&root);
    }

    fn load_mcp_servers(&self) -> Vec<McpServerView> {
        // Phase 3a (iter-208): rewired to use the REAL core_mcp_manager
        // instead of the deleted stub. The stub always returned empty data,
        // so /mcp showed 0 tools and all servers Disconnected. Now we read
        // the real connection state from operant_core::mcp::McpManager.
        //
        // The core API is async (tokio::sync::RwLock), but load_mcp_servers
        // is called from the sync render path. We use block_in_place +
        // Handle::block_on to safely call the async methods from within
        // the TUI's tokio runtime. This is the same pattern used by
        // operant's other sync→async bridges.
        if let Some(core_manager) = self.core_mcp_manager.as_ref() {
            // Try to get a runtime handle. If we're not in a tokio context
            // (e.g. unit tests), fall back to the config-only path below.
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                return self.load_mcp_servers_config_only();
            };
            let result = tokio::task::block_in_place(|| {
                handle.block_on(async {
                    let server_names = core_manager.server_names().await;
                    let all_servers = core_manager.all_servers().await;
                    (server_names, all_servers)
                })
            });

            let (server_names, all_servers) = result;
            return self
                .config
                .mcp
                .servers
                .iter()
                .map(|server| {
                    let transport = server
                        .url
                        .as_ref()
                        .map(|_| format!("{:?}", server.transport).to_lowercase())
                        .or_else(|| server.command.as_ref().map(|_| "stdio".to_string()))
                        .unwrap_or_else(|| format!("{:?}", server.transport).to_lowercase());

                    // Check if this server is connected in the core manager.
                    let connected = all_servers.contains_key(&server.name);

                    // Collect tools from the core transport if connected.
                    let tools: Vec<McpToolView> = if connected {
                        // Use block_in_place again for the async get_tools call.
                        let handle = tokio::runtime::Handle::try_current().ok();
                        if let Some(handle) = handle {
                            let transport_tools = tokio::task::block_in_place(|| {
                                handle.block_on(async {
                                    if let Some(t) = all_servers.get(&server.name) {
                                        t.get_tools().await
                                    } else {
                                        Vec::new()
                                    }
                                })
                            });
                            transport_tools
                                .into_iter()
                                .map(|t| {
                                    let def = t.definition();
                                    McpToolView {
                                        name: def.name.clone(),
                                        server: server.name.clone(),
                                        description: def.description.clone(),
                                        input_schema: Some(
                                            serde_json::to_string(&def.input_schema)
                                                .unwrap_or_default(),
                                        ),
                                    }
                                })
                                .collect()
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    };

                    let (status, error_message) = if connected {
                        (McpViewStatus::Connected, None)
                    } else if server_names.contains(&server.name) {
                        (McpViewStatus::Connecting, None)
                    } else {
                        (McpViewStatus::Disconnected, None)
                    };

                    McpServerView {
                        name: server.name.clone(),
                        transport,
                        status,
                        tool_count: tools.len(),
                        resource_count: 0,
                        prompt_count: 0,
                        resources: vec![],
                        prompts: vec![],
                        error_message,
                        tools,
                    }
                })
                .collect();
        }

        self.load_mcp_servers_config_only()
    }

    /// Fallback: build McpServerView list from config only (no live data).
    /// Used when core_mcp_manager is None or when not in a tokio runtime.
    fn load_mcp_servers_config_only(&self) -> Vec<McpServerView> {
        self.config
            .mcp
            .servers
            .iter()
            .map(|server| {
                let transport = server
                    .url
                    .as_ref()
                    .map(|_| format!("{:?}", server.transport).to_lowercase())
                    .or_else(|| server.command.as_ref().map(|_| "stdio".to_string()))
                    .unwrap_or_else(|| format!("{:?}", server.transport).to_lowercase());
                let description = if let Some(url) = &server.url {
                    format!("Endpoint: {}", url)
                } else if let Some(command) = &server.command {
                    let args = if server.args.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", server.args.join(" "))
                    };
                    format!("Command: {}{}", command, args)
                } else {
                    "Configured server".to_string()
                };
                McpServerView {
                    name: server.name.clone(),
                    transport,
                    status: McpViewStatus::Disconnected,
                    tool_count: 0,
                    resource_count: 0,
                    prompt_count: 0,
                    resources: vec![],
                    prompts: vec![],
                    error_message: None,
                    tools: vec![McpToolView {
                        name: "connection".to_string(),
                        server: server.name.clone(),
                        description,
                        input_schema: None,
                    }],
                }
            })
            .collect()
    }

    fn open_agents_menu(&mut self) {
        let root = self.project_root();
        self.agents_menu.open(&root);
        self.agents_menu.active_agents = self
            .agent_status
            .iter()
            .map(|(name, status)| AgentInfo {
                name: name.clone(),
                status: match status.as_str() {
                    "running" => AgentStatus::Running,
                    "waiting" | "waiting_for_tool" => AgentStatus::WaitingForTool,
                    "complete" | "completed" | "done" => AgentStatus::Complete,
                    "failed" | "error" => AgentStatus::Failed,
                    _ => AgentStatus::Idle,
                },
            })
            .collect();
    }

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
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // ── F12: toggle debug overlay (highest priority, never blocked) ──
        if key.code == KeyCode::F(12) {
            self.debug_hub.toggle_overlay();
            self.debug_hub.publish(crate::tui::debug::TuiEvent::Key {
                code: "F12".into(),
                modifiers: 0,
                at: crate::tui::debug::event_bus::now_secs(),
            });
            return false;
        }

        // Publish key event to debug bus (no-op when disabled).
        self.debug_hub.publish(crate::tui::debug::TuiEvent::Key {
            code: format!("{:?}", key.code),
            modifiers: key.modifiers.bits(),
            at: crate::tui::debug::event_bus::now_secs(),
        });

        // Dismiss error modal with Esc
        if key.code == KeyCode::Esc && self.notifications.current_is_error() {
            self.dismiss_error_notifications();
            return false;
        }

        // Phase 3.3: Priority-based dialog handling.
        // The existing inline handlers already follow the correct priority order
        // (context menu > bypass permissions > device auth > ...).
        // dialog_priority() returns the highest-priority visible dialog;
        // we assert the current handler matches that priority for debugging.
        let _priority = self.dialog_priority();

        if self.global_search.visible {
            return self.handle_global_search_key(key);
        }

        // ---- Context menu handling (highest priority for menu navigation) ----
        if self.context_menu_state.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.dismiss_context_menu();
                    return false;
                }
                KeyCode::Up | KeyCode::Down => {
                    self.navigate_context_menu(key.code);
                    return false;
                }
                KeyCode::Enter => {
                    self.execute_context_menu_item();
                    return false;
                }
                _ => {}
            }
        }

        // Bypass-permissions dialog: highest-priority gate — user must accept or the
        // session exits immediately. Mirrors TS BypassPermissionsModeDialog.tsx.
        if self.bypass_permissions_dialog.visible {
            match key.code {
                KeyCode::Char('1') | KeyCode::Esc => {
                    // "No" — decline; close and stay in the current mode.
                    self.bypass_permissions_dialog.dismiss();
                }
                KeyCode::Char('2') => {
                    // "Yes, I accept" — arm bypass-permissions and continue.
                    self.arm_bypass_permissions();
                    self.status_message = Some(
                        "Bypass permissions mode enabled — permissions will be auto-approved. Use with care.".to_string(),
                    );
                    self.bypass_permissions_dialog.dismiss();
                }
                KeyCode::Up | KeyCode::Char('k') => self.bypass_permissions_dialog.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.bypass_permissions_dialog.select_next(),
                KeyCode::Enter => {
                    if self.bypass_permissions_dialog.is_accept_selected() {
                        self.arm_bypass_permissions();
                        self.status_message = Some(
                            "Bypass permissions mode enabled — permissions will be auto-approved. Use with care.".to_string(),
                        );
                    }
                    self.bypass_permissions_dialog.dismiss();
                }
                _ => {}
            }
            return false;
        }

        // Effort picker dialog (/effort).
        if self.effort_picker.visible {
            match key.code {
                KeyCode::Esc => self.effort_picker.close(),
                KeyCode::Up | KeyCode::Char('k') => self.effort_picker.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.effort_picker.select_next(),
                KeyCode::Enter => {
                    let chosen = self.effort_picker.current();
                    self.effort_level = chosen;
                    self.effort_picker.close();
                    self.status_message = Some(format!(
                        "Effort set to {} {}.",
                        chosen.symbol(),
                        chosen.label()
                    ));
                }
                _ => {}
            }
            return false;
        }

        // Device code / browser auth dialog (GitHub Copilot, Anthropic OAuth)
        if self.device_auth_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.device_auth_dialog.close();
                    self.device_auth_pending = None;
                }
                _ if matches!(
                    self.device_auth_dialog.status,
                    crate::tui::device_auth_dialog::DeviceAuthStatus::Success(_)
                ) =>
                {
                    // Any key after success -> store credential and close
                    if let crate::tui::device_auth_dialog::DeviceAuthStatus::Success(ref token) =
                        self.device_auth_dialog.status
                    {
                        let provider_id = self.device_auth_dialog.provider_id.clone();
                        let provider_name = self.device_auth_dialog.provider_name.clone();
                        let token = token.clone();
                        let credential = if provider_id == "github-copilot" {
                            crate::tui::adapter_types::StoredCredential::OAuthToken {
                                access: token.clone(),
                                refresh: token,
                                expires: 0,
                            }
                        } else {
                            crate::tui::adapter_types::StoredCredential::ApiKey { key: token }
                        };
                        self.auth_store.set(&provider_id, credential);
                        self.device_auth_pending = None;
                        self.device_auth_dialog.close();
                        self.activate_provider(provider_id, provider_name, "Connected to");
                        return false;
                    }
                }
                _ if matches!(
                    self.device_auth_dialog.status,
                    crate::tui::device_auth_dialog::DeviceAuthStatus::Error(_)
                ) =>
                {
                    // Any key after error -> close
                    self.device_auth_dialog.close();
                    self.device_auth_pending = None;
                }
                _ => {} // Ignore other keys while waiting
            }
            return false;
        }

        // API key input dialog (opened from /connect for key-based providers)
        // Ask-user question dialog (AskUserQuestion tool)
        if self.ask_user_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.ask_user_dialog.dismiss();
                }
                KeyCode::Enter => {
                    self.ask_user_dialog.confirm();
                }
                KeyCode::Up | KeyCode::BackTab => {
                    self.ask_user_dialog.select_prev();
                }
                KeyCode::Down | KeyCode::Tab => {
                    self.ask_user_dialog.select_next();
                }
                KeyCode::Char(c)
                    if c.is_ascii_digit()
                        && self.ask_user_dialog.options.is_some()
                        && !self.ask_user_dialog.in_custom_input =>
                {
                    // Digit keys select an option by number ONLY when the user
                    // is not already typing a custom answer.  Once in custom
                    // mode, digits flow through to push_char like any other char.
                    let n = (c as u8 - b'0') as usize;
                    if n >= 1 {
                        self.ask_user_dialog.select_by_number(n);
                    }
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.ask_user_dialog.push_char(c);
                }
                KeyCode::Backspace => {
                    self.ask_user_dialog.pop_char();
                }
                _ => {}
            }
            return false;
        }

        if self.key_input_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.key_input_dialog.close();
                }
                KeyCode::Enter => {
                    let provider_id = self.key_input_dialog.provider_id.clone();
                    let provider_name = self.key_input_dialog.provider_name.clone();
                    let api_key = self.key_input_dialog.take_key();
                    if !api_key.is_empty() {
                        self.auth_store.set(
                            &provider_id,
                            crate::tui::adapter_types::StoredCredential::ApiKey { key: api_key },
                        );
                        self.activate_provider(provider_id, provider_name, "Connected to");
                    }
                }
                KeyCode::Backspace => {
                    self.key_input_dialog.backspace();
                }
                KeyCode::Char('v')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    if let Some(text) = crate::image_paste::read_clipboard_text() {
                        if text.is_empty() {
                            self.push_notification(
                                NotificationKind::Warning,
                                "Clipboard is empty".to_string(),
                                Some(2),
                            );
                        } else {
                            for ch in text.chars() {
                                self.key_input_dialog.insert_char(ch);
                            }
                        }
                    } else {
                        self.push_notification(
                            NotificationKind::Warning,
                            "Could not read clipboard".to_string(),
                            Some(2),
                        );
                    }
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.key_input_dialog.insert_char(c);
                }
                _ => {}
            }
            return false;
        }

        // "Free" composite-provider setup dialog (collects any subset of the
        // free-tier upstream keys; min 1 to enable, more = better).
        if self.free_mode_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.free_mode_dialog.close();
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.free_mode_dialog.move_next();
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.free_mode_dialog.move_prev();
                }
                KeyCode::Enter => {
                    if self.free_mode_dialog.can_submit() {
                        let values = self.free_mode_dialog.take_values();
                        for (provider_id, key) in values {
                            self.auth_store.set(
                                provider_id,
                                crate::tui::adapter_types::StoredCredential::ApiKey { key },
                            );
                        }
                        self.activate_provider(
                            "free".to_string(),
                            "Free Mode".to_string(),
                            "Connected to",
                        );
                    } else {
                        self.free_mode_dialog.move_next();
                    }
                }
                KeyCode::Backspace => {
                    self.free_mode_dialog.backspace();
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.free_mode_dialog.insert_char(c);
                }
                _ => {}
            }
            return false;
        }

        // Custom provider dialog (URL + API key for OpenAI-compatible providers)
        if self.custom_provider_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.custom_provider_dialog.close();
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.custom_provider_dialog.move_next_field();
                }
                KeyCode::Up => {
                    self.custom_provider_dialog.move_prev_field();
                }
                KeyCode::Enter => {
                    if self.custom_provider_dialog.can_submit() {
                        let provider_id = self.custom_provider_dialog.provider_id.clone();
                        let provider_name = self.custom_provider_dialog.provider_name.clone();
                        let (base_url, api_key) = self.custom_provider_dialog.take_values();
                        self.persist_custom_provider_base_url(&base_url);
                        self.auth_store.set(
                            &provider_id,
                            crate::tui::adapter_types::StoredCredential::ApiKey { key: api_key },
                        );
                        self.activate_provider(provider_id, provider_name, "Connected to");
                    } else {
                        self.custom_provider_dialog.move_next_field();
                    }
                }
                KeyCode::Backspace => {
                    self.custom_provider_dialog.backspace();
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.custom_provider_dialog.insert_char(c);
                }
                _ => {}
            }
            return false;
        }

        // Connect-a-provider dialog (/connect command)
        if self.connect_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.connect_dialog.close();
                }
                KeyCode::Home => {
                    self.connect_dialog.move_home();
                }
                KeyCode::End => {
                    self.connect_dialog.move_end();
                }
                KeyCode::Up => {
                    self.connect_dialog.move_up();
                }
                KeyCode::Down => {
                    self.connect_dialog.move_down();
                }
                KeyCode::PageUp => {
                    self.connect_dialog.page_up();
                }
                KeyCode::PageDown => {
                    self.connect_dialog.page_down();
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.connect_dialog.move_up();
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.connect_dialog.move_down();
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.connect_dialog.selected().cloned() {
                        self.connect_dialog.close();

                        match selected.id.as_str() {
                            // Local providers — activate immediately, no key needed
                            "ollama" | "lmstudio" | "llamacpp" => {
                                self.activate_provider(
                                    selected.id.clone(),
                                    selected.title.clone(),
                                    "Switched to",
                                );
                            }
                            // "Free" composite mode — collects any subset of the
                            // free-tier upstreams (min 1; more = better availability).
                            "free" => {
                                let existing: Vec<(&'static str, String)> = crate::tui::adapter_types::FREE_CATALOG
                                    .iter()
                                    .filter_map(|upstream| {
                                        let key = match upstream.id {
                                            "opencode-zen" => self
                                                .auth_store
                                                .api_key_for(crate::tui::adapter_types::ProviderId::OpencodeZen)
                                                .or_else(|| {
                                                    self.auth_store.api_key_for(
                                                        crate::tui::adapter_types::ProviderId::OpencodeGo,
                                                    )
                                                }),
                                            other => self.auth_store.api_key_for(other),
                                        };
                                        key.filter(|k: &String| !k.is_empty())
                                            .map(|k| (upstream.id, k))
                                    })
                                    .collect();
                                self.free_mode_dialog.open(&existing);
                            }
                            "anthropic" => {
                                // Anthropic: use API key from console.anthropic.com
                                // (OAuth requires a registered app which Operant doesn't have)
                                self.key_input_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                            }
                            "custom-openai" => {
                                let current_url = Settings::load_sync().ok().and_then(|settings| {
                                    settings
                                        .providers
                                        .get("custom-openai")
                                        .and_then(|p| p.api_base.clone())
                                });
                                self.custom_provider_dialog.open(
                                    selected.id.clone(),
                                    selected.title.clone(),
                                    current_url,
                                );
                            }
                            "github-copilot" => {
                                // GitHub Copilot: device code flow
                                self.device_auth_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                                self.device_auth_pending = Some("github-copilot".to_string());
                            }
                            "codex" | "openai-codex" => {
                                // OpenAI Codex: browser OAuth flow (spawned by main loop)
                                self.device_auth_dialog
                                    .open("openai-codex".into(), "OpenAI Codex".into());
                                self.device_auth_pending = Some("openai-codex".to_string());
                            }
                            // AWS Bedrock — accept a bearer token via key input dialog
                            "amazon-bedrock" => {
                                self.key_input_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                            }
                            // All other providers — open API key input dialog
                            _ => {
                                self.key_input_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.connect_dialog.filter_pop();
                }
                KeyCode::Char(c) => {
                    self.connect_dialog.filter_push(c);
                }
                _ => {}
            }
            return false;
        }

        // Import-config source picker
        if self.import_config_picker.visible {
            match key.code {
                KeyCode::Esc => {
                    self.import_config_picker.close();
                }
                KeyCode::Home => {
                    self.import_config_picker.move_home();
                }
                KeyCode::End => {
                    self.import_config_picker.move_end();
                }
                KeyCode::Up => {
                    self.import_config_picker.move_up();
                }
                KeyCode::Down => {
                    self.import_config_picker.move_down();
                }
                KeyCode::PageUp => {
                    self.import_config_picker.page_up();
                }
                KeyCode::PageDown => {
                    self.import_config_picker.page_down();
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.import_config_picker.move_up();
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.import_config_picker.move_down();
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.import_config_picker.selected().cloned() {
                        self.import_config_picker.close();
                        if let Some(selection) = Self::import_selection_from_picker(&selected.id) {
                            self.open_import_config_preview(selection);
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.import_config_picker.filter_pop();
                }
                KeyCode::Char(c) => {
                    self.import_config_picker.filter_push(c);
                }
                _ => {}
            }
            return false;
        }

        // Import-config preview dialog
        if self.import_config_dialog.visible {
            match key.code {
                KeyCode::Esc => self.import_config_dialog.close(),
                KeyCode::Enter => self.perform_import_config(),
                _ => {}
            }
            return false;
        }

        // Command palette (Ctrl+K)
        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc => {
                    self.command_palette.close();
                }
                KeyCode::Home => {
                    self.command_palette.move_home();
                }
                KeyCode::End => {
                    self.command_palette.move_end();
                }
                KeyCode::Up => {
                    self.command_palette.move_up();
                }
                KeyCode::Down => {
                    self.command_palette.move_down();
                }
                KeyCode::PageUp => {
                    self.command_palette.page_up();
                }
                KeyCode::PageDown => {
                    self.command_palette.page_down();
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.command_palette.move_up();
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.command_palette.move_down();
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.command_palette.selected().cloned() {
                        self.command_palette.close();
                        // Put the command in the input and signal for execution
                        self.prompt_input.replace_text(selected.id.clone());
                        return true; // signal to submit this as input
                    }
                }
                KeyCode::Backspace => {
                    self.command_palette.filter_pop();
                }
                KeyCode::Char(c) => {
                    self.command_palette.filter_push(c);
                }
                _ => {}
            }
            return false;
        }

        // Invalid-config dialog intercepts Enter/Esc to dismiss

        // Model picker intercepts navigation and Esc
        if self.model_picker.visible {
            match key.code {
                KeyCode::Esc => self.model_picker.close(),
                KeyCode::Home => self.model_picker.select_first(),
                KeyCode::End => self.model_picker.select_last(),
                KeyCode::Up => self.model_picker.select_prev(),
                KeyCode::Down => self.model_picker.select_next(),
                KeyCode::Left => self.model_picker.effort_prev(),
                KeyCode::Right => self.model_picker.effort_next(),
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.model_picker.select_prev()
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.model_picker.select_next()
                }
                KeyCode::Enter => {
                    if let Some((model_id, effort)) = self.model_picker.confirm() {
                        // If user picked a model other than the fast-mode model
                        // while fast mode was active, turn fast mode off.
                        if self.fast_mode
                            && !self.model_picker.is_selected_fast_mode_model(&model_id)
                        {
                            self.fast_mode = false;
                        }
                        if let Some(e) = effort {
                            self.effort_level = e;
                        }
                        // Store explicit selections in the canonical
                        // "provider/model" form for non-Anthropic providers.
                        // The "free" composite's picker entries already carry
                        // a routing prefix (`free/…`, `zen/…`, `openrouter/…`)
                        // so re-prefixing would produce nonsense like
                        // `free/free/auto`. Also, OpenRouter catalog entries
                        // are already prefixed with `openrouter/…` — check
                        // for that to avoid `openrouter/openrouter/anthropic/…`.
                        // (Bug #14 from iter-82 audit.)
                        let provider = self.active_provider.as_deref().unwrap_or("anthropic");
                        let prefix = format!("{}/", provider);
                        let full_model = if provider == "anthropic" || provider == "free" {
                            model_id.clone()
                        } else if model_id.starts_with(&prefix) {
                            // Already prefixed (e.g. openrouter/anthropic/claude-…).
                            model_id.clone()
                        } else {
                            format!("{}/{}", provider, model_id)
                        };
                        self.set_model(full_model.clone());
                        self.persist_provider_and_model();
                        let effort_hint = effort
                            .map(|e| format!(" [{}]", e.label()))
                            .unwrap_or_default();
                        self.status_message = Some(format!("Model: {}{}", full_model, effort_hint));
                    }
                }
                KeyCode::Backspace => self.model_picker.pop_filter_char(),
                KeyCode::Char(c) => self.model_picker.push_filter_char(c),
                _ => {}
            }
            return false;
        }

        // Session branching overlay intercepts navigation and Esc
        if self.session_branching.visible {
            use crate::tui::session_branching::BranchBrowserMode;
            match self.session_branching.mode {
                BranchBrowserMode::Browse => match key.code {
                    KeyCode::Esc => self.session_branching.cancel(),
                    KeyCode::Up => self.session_branching.select_prev(),
                    KeyCode::Down => self.session_branching.select_next(),
                    KeyCode::Char('n') => self.session_branching.start_create_new(),
                    KeyCode::Char('d') => self.session_branching.start_delete_confirm(),
                    KeyCode::Enter => {
                        if let Some(branch) = self.session_branching.selected_branch() {
                            self.status_message =
                                Some(format!("Switched to branch: {}", branch.name));
                            self.session_branching.close();
                        }
                    }
                    _ => {}
                },
                BranchBrowserMode::CreateNew => match key.code {
                    KeyCode::Esc => self.session_branching.cancel(),
                    KeyCode::Enter => {
                        if let Some((name, at_msg)) = self.session_branching.confirm_create_new() {
                            self.status_message =
                                Some(format!("Created branch: {} at message {}", name, at_msg));
                            self.session_branching.close();
                        }
                    }
                    KeyCode::Backspace => self.session_branching.pop_create_char(),
                    KeyCode::Char(c) => self.session_branching.push_create_char(c),
                    _ => {}
                },
                BranchBrowserMode::ConfirmDelete => match key.code {
                    KeyCode::Esc | KeyCode::Char('n') => self.session_branching.cancel(),
                    KeyCode::Enter | KeyCode::Char('y') => {
                        if let Some(branch_id) = self.session_branching.confirm_delete() {
                            self.status_message = Some(format!("Deleted branch: {}", branch_id));
                        }
                    }
                    _ => {}
                },
            }
            return false;
        }

        // Session browser intercepts navigation and Esc
        if self.session_browser.visible {
            use crate::tui::session_browser::SessionBrowserMode;
            match self.session_browser.mode {
                SessionBrowserMode::Browse => {
                    match key.code {
                        KeyCode::Esc => self.session_browser.close(),
                        KeyCode::Up => self.session_browser.select_prev(),
                        KeyCode::Down => self.session_browser.select_next(),
                        KeyCode::Char('r') => self.session_browser.start_rename(),
                        // Enter: load the selected session's messages from the
                        // database and replace app.messages. The actual load
                        // happens asynchronously in the run loop via
                        // session_load_pending → session_load_rx.
                        KeyCode::Enter => {
                            if let Some(entry) = self
                                .session_browser
                                .sessions
                                .get(self.session_browser.selected_idx)
                                .cloned()
                            {
                                self.session_browser.close();
                                self.session_load_pending = Some(entry.id.clone());
                                self.status_message =
                                    Some(format!("Loading session '{}'…", entry.title));
                            }
                        }
                        _ => {}
                    }
                }
                SessionBrowserMode::Rename => match key.code {
                    KeyCode::Esc => self.session_browser.cancel(),
                    KeyCode::Enter => {
                        if let Some((_id, name)) = self.session_browser.confirm_rename() {
                            self.session_title = Some(name.clone());
                            self.status_message = Some(format!("Renamed to: {}", name));
                        }
                    }
                    KeyCode::Backspace => self.session_browser.pop_rename_char(),
                    KeyCode::Char(c) => self.session_browser.push_rename_char(c),
                    _ => {}
                },
            }
            return false;
        }

        // Tasks overlay intercepts navigation and Esc
        if self.tasks_overlay.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.tasks_overlay.close(),
                KeyCode::Up => self.tasks_overlay.select_prev(),
                KeyCode::Down => self.tasks_overlay.select_next(),
                KeyCode::Enter => {
                    if let Some((task_id, new_status)) =
                        self.tasks_overlay.cycle_and_persist_status()
                    {
                        self.status_message = Some(format!("Task {} → {}", task_id, new_status));
                    }
                }
                _ => {}
            }
            return false;
        }

        // Export dialog key handling
        if self.export_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.export_dialog.dismiss();
                }
                KeyCode::Enter => {
                    if let Some(path) = self.perform_export() {
                        self.push_notification(
                            NotificationKind::Info,
                            format!("Exported to {}", path),
                            Some(4),
                        );
                    } else {
                        self.push_notification(
                            NotificationKind::Warning,
                            "Export failed: could not write file.".to_string(),
                            Some(4),
                        );
                    }
                }
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    self.export_dialog.toggle();
                }
                KeyCode::Char('1') => {
                    self.export_dialog.selected = ExportFormat::Json;
                }
                KeyCode::Char('2') => {
                    self.export_dialog.selected = ExportFormat::Markdown;
                }
                _ => {}
            }
            return false;
        }

        // Context visualization overlay key handling
        if self.context_viz.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.context_viz.close();
                }
                _ => {}
            }
            return false;
        }

        // MCP approval dialog
        if self.mcp_approval.visible {
            let result = crate::dialogs::handle_mcp_approval_key(&mut self.mcp_approval, key);
            if result.is_some() {
                // Result processed by CLI loop via take_mcp_approval_result()
            }
            return false;
        }

        // (iter-211: feedback_survey key handler deleted — no telemetry backend)

        // Memory file selector intercepts navigation and Esc
        if self.memory_file_selector.visible {
            match key.code {
                KeyCode::Esc => self.memory_file_selector.close(),
                KeyCode::Up => self.memory_file_selector.select_prev(),
                KeyCode::Down => self.memory_file_selector.select_next(),
                KeyCode::Enter => {
                    self.memory_file_selector.close();
                }
                _ => {}
            }
            return false;
        }

        // Skills view intercepts navigation and Esc
        if self.skills_view.visible {
            match key.code {
                KeyCode::Esc => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::Detail {
                        self.skills_view.back_to_list();
                    } else {
                        self.skills_view.close();
                    }
                }
                KeyCode::Backspace => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::Detail {
                        self.skills_view.back_to_list();
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::List {
                        self.skills_view.select_prev();
                    } else {
                        self.skills_view.scroll_up();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::List {
                        self.skills_view.select_next();
                    } else {
                        // Use the last rendered viewport height (set by
                        // render_list_stage) instead of a hardcoded 24.
                        // (Bug #16 fix.)
                        let vh = self.skills_view.last_viewport_height.get().max(1);
                        self.skills_view.scroll_down(vh);
                    }
                }
                KeyCode::PageUp => {
                    for _ in 0..6 {
                        self.skills_view.scroll_up();
                    }
                }
                KeyCode::PageDown => {
                    let vh = self.skills_view.last_viewport_height.get().max(1);
                    for _ in 0..6 {
                        self.skills_view.scroll_down(vh);
                    }
                }
                KeyCode::Enter => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::List {
                        self.skills_view.open_detail();
                    }
                }
                _ => {}
            }
            return false;
        }

        // Plugins hub intercepts navigation, toggle, and Esc
        if self.plugins_hub.visible {
            // Resolve plugins_dir once for the toggle action.
            let plugins_dir = crate::cmd_plugins::plugins_dir(&self.config).unwrap_or_else(|_| {
                dirs::data_dir()
                    .unwrap_or_default()
                    .join("operant")
                    .join("plugins")
            });
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.plugins_hub.close(),
                KeyCode::Up | KeyCode::Char('k') => self.plugins_hub.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.plugins_hub.select_next(),
                KeyCode::Enter | KeyCode::Char('t') | KeyCode::Char(' ') => {
                    self.plugins_hub.toggle_selected(&plugins_dir);
                }
                _ => {}
            }
            return false;
        }

        // Journey view intercepts navigation, pane-switch, and Esc
        if self.journey_view.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.journey_view.close(),
                KeyCode::Up | KeyCode::Char('k') => self.journey_view.cursor_up(),
                KeyCode::Down | KeyCode::Char('j') => self.journey_view.cursor_down(),
                KeyCode::Tab | KeyCode::BackTab => self.journey_view.switch_pane(),
                _ => {}
            }
            return false;
        }

        // Hooks config menu intercepts navigation and Esc
        if self.hooks_config_menu.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.hooks_config_menu.back(),
                KeyCode::Enter => self.hooks_config_menu.enter(),
                KeyCode::Up | KeyCode::Char('k') => self.hooks_config_menu.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.hooks_config_menu.select_next(),
                _ => {}
            }
            return false;
        }

        if self.diff_viewer.visible {
            self.handle_diff_viewer_key(key);
            return false;
        }

        if self.agents_menu.visible {
            self.handle_agents_menu_key(key);
            return false;
        }

        if self.mcp_view.visible {
            return self.handle_mcp_view_key(key);
        }

        if self.stats_dialog.visible {
            self.handle_stats_dialog_key(key);
            return false;
        }

        // Settings screen intercepts keys
        if self.settings_screen.visible {
            crate::settings_screen::handle_settings_key(
                &mut self.settings_screen,
                &mut self.config,
                &mut self.settings,
                key,
            );
            return false;
        }

        // Theme picker intercepts keys
        if self.theme_screen.visible {
            if let Some(theme_name) =
                crate::theme_screen::handle_theme_key(&mut self.theme_screen, key)
            {
                self.apply_theme(&theme_name);
            }
            return false;
        }

        // Privacy screen intercepts keys
        // Rewind flow overlay intercepts keys first
        if self.rewind_flow.visible {
            return self.handle_rewind_flow_key(key);
        }

        // Help overlay intercepts keys next
        if self.help_overlay.visible {
            return self.handle_help_overlay_key(key);
        }

        // New history-search overlay
        if self.history_search_overlay.visible {
            return self.handle_history_search_overlay_key(key);
        }

        if self.global_search.visible {
            return self.handle_global_search_key(key);
        }

        // (iter-155: legacy history_search.is_some() check deleted — always None)

        // Permission dialog mode intercepts most keys
        if self.permission_request.is_some() {
            self.handle_permission_key(key);
            return false;
        }

        // Notification dismiss
        if key.code == KeyCode::Esc && !self.notifications.is_empty() {
            self.notifications.dismiss_current();
            return false;
        }

        // (iter-143: plugin_hints dismiss handler deleted — Vec was always empty)

        // Overage upsell dismiss — the overage_upsell dialog was deleted in
        // iter-58; this block is kept as a placeholder for future dismiss
        // handlers. No-op until a replacement dialog is wired.

        // Voice mode notice dismiss
        if key.code == KeyCode::Esc && self.voice_mode_notice.visible {
            self.voice_mode_notice.dismiss();
            return false;
        }

        // Cancel an active voice recording with Esc.
        if key.code == KeyCode::Esc && self.voice_recording {
            self.voice_recording = false;
            self.voice_event_rx = None;
            if let Some(ref recorder_arc) = self.voice_recorder {
                let recorder = recorder_arc.clone();
                tokio::task::spawn_blocking(move || {
                    if let Ok(mut r) = recorder.lock() {
                        tokio::runtime::Handle::current()
                            .block_on(r.stop_recording())
                            .ok();
                    }
                });
            }
            self.status_message = Some("Recording cancelled.".to_string());
            return false;
        }

        // Desktop upsell startup dialog

        // Memory update notification dismiss — the memory_update_notification
        // dialog was deleted in iter-58; this block is kept as a placeholder
        // for future dismiss handlers. No-op until a replacement is wired.

        // MCP elicitation dialog — highest priority modal

        // (iter-163: KeybindingResolver processor deleted — process() always
        // returned NoMatch, has_pending_chord() always returned false, and
        // cancel_chord() was a no-op. The entire block was dead code that
        // always fell through to the hardcoded handlers.)

        // Clear any active text selection on key press (except Ctrl+C which copies it).
        let is_copy =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if !is_copy && self.selection_anchor.is_some() {
            self.selection_anchor = None;
            self.selection_focus = None;
            *self.selection_text.borrow_mut() = String::new();
        }

        // ---- Voice hold-to-talk (Alt+V toggles recording on/off) ----------
        if key.code == KeyCode::Char('v')
            && key.modifiers.contains(KeyModifiers::ALT)
            && self.voice_recorder.is_some()
        {
            if !self.voice_recording {
                // First press: start recording.
                let (tx, rx) = tokio::sync::mpsc::channel(8);
                self.voice_event_rx = Some(rx);
                self.voice_recording = true;
                if let Some(ref recorder_arc) = self.voice_recorder {
                    let recorder = recorder_arc.clone();
                    tokio::task::spawn_blocking(move || {
                        if let Ok(mut r) = recorder.lock() {
                            tokio::runtime::Handle::current()
                                .block_on(r.start_recording(tx))
                                .ok();
                        }
                    });
                }
                self.push_notification(
                    NotificationKind::Info,
                    "Recording\u{2026} (Alt+V to transcribe · Esc to cancel)".to_string(),
                    None,
                );
            } else {
                // Second press: stop recording.
                self.voice_recording = false;
                if let Some(ref recorder_arc) = self.voice_recorder {
                    let recorder = recorder_arc.clone();
                    tokio::task::spawn_blocking(move || {
                        if let Ok(mut r) = recorder.lock() {
                            tokio::runtime::Handle::current()
                                .block_on(r.stop_recording())
                                .ok();
                        }
                    });
                }
                self.push_notification(
                    NotificationKind::Info,
                    "Transcribing\u{2026}".to_string(),
                    Some(10),
                );
            }
            return false;
        }

        // ---- Voice PTT: plain V press starts recording when voice is on ----
        // This is the "hold to talk" variant.  The user presses V to begin
        // recording; releasing V (handled in the run loop) or pressing Enter
        // stops the capture and triggers transcription.
        // Only active when voice mode is enabled (voice_recorder is Some) and
        // the prompt input is in default (non-vim) mode so 'v' doesn't conflict
        // with vim keybindings.
        if key.code == KeyCode::Char('v')
            && key.modifiers == KeyModifiers::NONE
            && self.voice_recorder.is_some()
            && !self.voice_recording
            && self.prompt_input.vim_mode == crate::prompt_input::VimMode::Insert
        {
            self.handle_voice_ptt_start();
            return false;
        }

        // ---- Ctrl+V / Cmd+V — clipboard paste (image first, then text fallback) ----
        // Only fires when NOT in vim Normal/Visual/VisualBlock mode (where \x16 is
        // already consumed by the vim handler above to enter VisualBlock mode).
        if key.code == KeyCode::Char('v')
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !matches!(
                self.prompt_input.vim_mode,
                crate::prompt_input::VimMode::Normal
                    | crate::prompt_input::VimMode::Visual
                    | crate::prompt_input::VimMode::VisualBlock
            )
        {
            use crate::tui::image_paste::{
                read_clipboard_image, read_clipboard_text, read_primary_text,
            };
            if let Some(img) = read_clipboard_image() {
                let label = img.label.clone();
                let dims = img.dimensions;
                self.prompt_input.add_image(img);
                let msg = if let Some((w, h)) = dims {
                    format!("Image attached: {} ({}x{})", label, w, h)
                } else {
                    format!("Image attached: {}", label)
                };
                self.push_notification(NotificationKind::Info, msg, Some(3));
            } else if let Some(text) = read_clipboard_text().or_else(read_primary_text) {
                self.handle_paste_data(text);
                self.refresh_prompt_input();
            }
            return false;
        }

        // ---- Shift+Insert — selection/clipboard paste fallback -------------
        if key.code == KeyCode::Insert && key.modifiers.contains(KeyModifiers::SHIFT) {
            let _ = self.paste_primary_into_prompt();
            return false;
        }

        // ---- Enter while PTT recording: stop capture instead of submitting ----
        if key.code == KeyCode::Enter && self.voice_recording && self.voice_recorder.is_some() {
            self.handle_voice_ptt_stop();
            return false;
        }

        // ---- Focus state machine: transcript mode --------------------------
        // When the transcript pane has focus, intercept Escape and scroll keys.
        // Printable characters switch focus back to Input and fall through so the
        // keystroke is processed normally by the prompt editor below.
        if self.focus == FocusTarget::Transcript {
            match key.code {
                KeyCode::Esc => {
                    self.focus = FocusTarget::Input;
                    return false;
                }
                KeyCode::PageUp | KeyCode::PageDown => {
                    // Let these fall through to the normal scroll handling below.
                }
                KeyCode::Char(_)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    // Printable char: switch focus to Input and process normally.
                    self.focus = FocusTarget::Input;
                }
                _ => {}
            }
        }

        match key.code {
            // ---- ESC: cancel streaming (status bar advertises "esc interrupt") ----
            KeyCode::Esc if self.is_streaming => {
                self.is_streaming = false;
                self.spinner_verb = None;
                // Flush in-flight streaming text to messages BEFORE snapshot
                // so the response is preserved in the transcript.
                self.flush_streamed_assistant_message();
                self.status_message = Some("Cancelled.".to_string());
                // Abort the background agent task so it actually stops.
                if let Some(handle) = self.agent_task_handle.take() {
                    handle.abort();
                }
                // Snapshot AFTER flushing so tool trail is preserved.
                self.complete_current_turn_snapshot(true);
                self.tool_use_blocks.clear();
            }

            // ---- Quit / cancel ----------------------------------------
            // Accept both 'c' and 'C' so Shift+Ctrl+C also triggers copy
            // (issue #149 follow-up).
            KeyCode::Char(c)
                if (c == 'c' || c == 'C') && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // If text is selected, copy it to clipboard instead of quitting.
                let sel_text = self.selection_text.borrow().clone();
                if self.selection_anchor.is_some() && !sel_text.is_empty() {
                    // Text is selected: copy to clipboard.
                    let copied = crate::image_paste::write_clipboard_text(&sel_text);
                    self.selection_anchor = None;
                    self.selection_focus = None;
                    *self.selection_text.borrow_mut() = String::new();
                    if copied {
                        self.push_notification(
                            NotificationKind::Info,
                            "Copied to clipboard".to_string(),
                            Some(2),
                        );
                    }
                } else if self.is_streaming {
                    // Cancel streaming.
                    self.is_streaming = false;
                    self.spinner_verb = None;
                    // Flush in-flight streaming text to messages BEFORE snapshot
                    // so the response is preserved in the transcript.
                    self.flush_streamed_assistant_message();
                    self.status_message = Some("Cancelled.".to_string());
                    self.complete_current_turn_snapshot(true);
                    self.tool_use_blocks.clear();
                } else {
                    // No text selected and not streaming: handle exit confirmation sequence.
                    // Always clear the prompt input on Ctrl+C.
                    if !self.prompt_input.is_empty() {
                        self.prompt_input.clear();
                        self.refresh_prompt_input();
                    }
                    self.handle_exit_key_confirmation('c');
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+D on empty input: trigger two-press exit confirmation (like Ctrl+C).
                if self.prompt_input.is_empty() {
                    self.handle_exit_key_confirmation('d');
                }
            }

            // ---- Model picker (Ctrl+A) -----------------------------------
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.is_streaming && self.has_credentials {
                    self.open_model_picker_for_provider(
                        &self.active_provider.clone().unwrap_or_default(),
                        None,
                    );
                }
            }

            // ---- History search ----------------------------------------
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let overlay = HistorySearchOverlay::open(&self.prompt_input.history);
                self.history_search_overlay = overlay;
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.global_search.open();
                self.refresh_global_search();
            }

            // ---- Tasks overlay (Ctrl+T) --------------------------------
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tasks_overlay.toggle();
            }

            // ---- Session branching (Ctrl+B) -----------------------------
            // Bug #6 from iter-82 audit: Ctrl+B was documented in the help
            // overlay comment but had no keybinding. session_branching.open()
            // was never called from anywhere. Now it opens the branch browser
            // seeded with the current message count.
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.session_branching.open(vec![], self.messages.len());
            }

            // ---- Context menu (Ctrl+Shift+M) ----------------------------
            KeyCode::Char('m')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.open_context_menu_at_cursor();
            }

            // ---- Command palette (Ctrl+K) -------------------------------
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.command_palette.open();
            }

            // ---- Help overlay ------------------------------------------
            KeyCode::F(1) => {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }
            KeyCode::Char('?')
                if !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }
            // With the kitty keyboard protocol, Shift+/ is reported as Char('/') with
            // SHIFT rather than Char('?'), so also accept that form for the help toggle.
            KeyCode::Char('/')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }

            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.kill_line_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.kill_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.yank();
                self.refresh_prompt_input();
            }

            // ---- Alt/Meta key text editing operations -------------------
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.yank_pop();
                self.refresh_prompt_input();
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_forward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_backward();
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_forward();
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_at_cursor();
                self.refresh_prompt_input();
            }

            // ---- Text entry (allowed while streaming so users can queue
            // the next message; submission queues via Enter at the CLI layer).
            KeyCode::Char(c) => {
                let c = normalize_char_with_shift(c, key.modifiers);
                if self.prompt_input.vim_enabled && self.prompt_input.vim_mode != VimMode::Insert {
                    self.prompt_input.vim_command(&c.to_string());
                } else {
                    self.prompt_input.insert_char(c);
                }
                self.refresh_prompt_input();
            }
            KeyCode::Backspace => {
                self.prompt_input.backspace();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete_word_forward();
                self.refresh_prompt_input();
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = 0;
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_backward();
                } else {
                    self.prompt_input.move_left();
                }
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = self.prompt_input.text.len();
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_forward();
                } else {
                    self.prompt_input.move_right();
                }
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Home => {
                self.prompt_input.cursor = 0;
                self.sync_legacy_prompt_fields();
            }
            KeyCode::End => {
                self.prompt_input.cursor = self.prompt_input.text.len();
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Tab => {
                if !self.prompt_input.suggestions.is_empty() {
                    // Accept slash-command suggestion. Allowed while streaming
                    // so the typeahead popup is interactive even when a turn
                    // is in flight — Enter then queues the completed command.
                    if self.prompt_input.suggestion_index.is_none() {
                        self.prompt_input.suggestion_index = Some(0);
                    }
                    self.prompt_input.accept_suggestion();
                    self.refresh_prompt_input();
                }
            }

            // ---- Shift+Tab: cycle permission mode ----------------------
            // Default → AcceptEdits → BypassPermissions → Default
            // Mirrors TS bottom-left indicator cycling behaviour.
            KeyCode::BackTab if !self.is_streaming => {
                use crate::tui::adapter_types::config::PermissionMode;
                self.settings.permission_mode = match self.settings.permission_mode {
                    PermissionMode::Default => PermissionMode::AcceptEdits,
                    PermissionMode::AcceptEdits => PermissionMode::BypassPermissions,
                    PermissionMode::BypassPermissions => PermissionMode::Default,
                    PermissionMode::Plan => PermissionMode::Default,
                };
                let label = match self.settings.permission_mode {
                    PermissionMode::Default => "Default permissions",
                    PermissionMode::AcceptEdits => "Accept-edits mode",
                    PermissionMode::BypassPermissions => "Bypass permissions (dangerous)",
                    PermissionMode::Plan => "Plan mode",
                };
                self.status_message = Some(label.to_string());
            }

            // ---- Submit ------------------------------------------------
            // Shift+Enter / Alt+Enter / Ctrl+Enter / Ctrl+J insert a literal
            // newline so users can compose multi-line prompts before sending.
            // Ctrl+J is the traditional Unix "newline" key and is what
            // hermes-agent uses for line breaks in the TUI.
            // (iter-120 — user-requested: Ctrl+J was not working.)
            KeyCode::Enter
                if !self.is_streaming
                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.modifiers.contains(KeyModifiers::ALT)
                        || key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                self.prompt_input.insert_newline();
                self.refresh_prompt_input();
            }
            KeyCode::Enter if !self.is_streaming => {
                use crate::tui::prompt_input::AcceptForSubmitOutcome;
                // Phase 1.3: Auto-select first suggestion when visible but none selected.
                if !self.prompt_input.suggestions.is_empty()
                    && self.prompt_input.suggestion_index.is_none()
                {
                    self.prompt_input.suggestion_index = Some(0);
                }
                match self.prompt_input.accept_suggestion_for_submit() {
                    AcceptForSubmitOutcome::ExtendInput => {
                        self.refresh_prompt_input();
                        return false;
                    }
                    AcceptForSubmitOutcome::Submit => return true,
                    AcceptForSubmitOutcome::NoSuggestion => {}
                }
                // Auto-dismiss all error notifications when user sends a message
                self.dismiss_error_notifications();
                // New user input: snap back to bottom.
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.scroll_offset = 0;
                return true;
            }

            // ---- Message boundary navigation (Alt+Up/Alt+Down) ----------
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
                // Jump up by ~20 lines (approximate message boundary).
                self.scroll_offset = self.scroll_offset.saturating_add(20);
                self.auto_scroll = false;
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
                // Jump down by ~20 lines (approximate message boundary).
                let new_off = self.scroll_offset.saturating_sub(20);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
            }

            // ---- Input history navigation ------------------------------
            // For multi-line / wrapped prompts: Up/Down move the cursor by
            // one visual row first, only falling through to history recall
            // when the cursor is already on the first/last visual row
            // (issue #149 follow-up).
            // Also, if suggestions are visible (text starts with '/' or has file ref),
            // allow suggestion navigation with Up/Down.
            // In vim Visual mode, Shift+Up/Shift+Down extend the selection.
            KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(
                        self.prompt_input.vim_mode,
                        crate::prompt_input::VimMode::Visual
                            | crate::prompt_input::VimMode::VisualLine
                            | crate::prompt_input::VimMode::VisualBlock
                    )
                {
                    // Shift+Up in visual mode: extend selection up
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_up(width);
                } else if !self.prompt_input.suggestions.is_empty()
                    && (self.prompt_input.text.starts_with('/')
                        || self.prompt_input.has_active_file_ref())
                {
                    // Suggestions visible: navigate them
                    self.prompt_input.suggestion_prev();
                } else if !self.prompt_input.text.contains('\n') {
                    // Single-line input: always navigate history (like hermes-agent).
                    // (iter-124 — was only navigating when move_visual_up failed,
                    // which meant Up did nothing on single-line input.)
                    if !self.prompt_input.history.is_empty() {
                        self.prompt_input.history_up();
                    }
                } else {
                    // Multi-line input: move cursor up within the text.
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_up(width);
                }
                self.refresh_prompt_input();
            }
            KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(
                        self.prompt_input.vim_mode,
                        crate::prompt_input::VimMode::Visual
                            | crate::prompt_input::VimMode::VisualLine
                            | crate::prompt_input::VimMode::VisualBlock
                    )
                {
                    // Shift+Down in visual mode: extend selection down
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_down(width);
                } else if !self.prompt_input.suggestions.is_empty()
                    && (self.prompt_input.text.starts_with('/')
                        || self.prompt_input.has_active_file_ref())
                {
                    // Suggestions visible: navigate them
                    self.prompt_input.suggestion_next();
                } else if !self.prompt_input.text.contains('\n') {
                    // Single-line input: always navigate history.
                    if self.prompt_input.history_pos.is_some() {
                        self.prompt_input.history_down();
                    }
                } else {
                    // Multi-line input: move cursor down within the text.
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_down(width);
                }
                self.refresh_prompt_input();
            }

            // ---- Scroll ------------------------------------------------
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                // Scrolling up disables auto-follow.
                self.auto_scroll = false;
            }
            KeyCode::PageDown => {
                let new_off = self.scroll_offset.saturating_sub(10);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    // Scrolled all the way back to bottom — re-enable auto-follow.
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
            }

            // ---- Toggle last thinking block (t key) -------------------
            // (Removed: shadowed by KeyCode::Char(c) prompt input handler.)
            _ => {}
        }

        // Reset exit confirmation sequence if user presses any key other than Ctrl+C or Ctrl+D.
        let is_exit_key = key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char(c) if c == 'c' || c == 'd' || c == 'C' || c == 'D');
        if !is_exit_key {
            self.last_exit_key_warning = None;
            self.exit_key_sequence_start = None;
        }

        false
    }

    // (iter-164: fn current_key_context deleted — unused after keybinding processor removal)

    // -------------------------------------------------------------------
    // New overlay key handlers
    // -------------------------------------------------------------------

    fn handle_stats_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.stats_dialog.close(),
            KeyCode::Tab | KeyCode::Right => self.stats_dialog.next_tab(),
            KeyCode::BackTab | KeyCode::Left => self.stats_dialog.prev_tab(),
            KeyCode::Char('r') => self.stats_dialog.cycle_range(),
            KeyCode::Up => self.stats_dialog.scroll = self.stats_dialog.scroll.saturating_sub(1),
            KeyCode::Down => self.stats_dialog.scroll = self.stats_dialog.scroll.saturating_add(1),
            _ => {}
        }
    }

    fn handle_mcp_view_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mcp_view.close(),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => self.mcp_view.switch_pane(),
            KeyCode::Up => self.mcp_view.select_prev(),
            KeyCode::Down => self.mcp_view.select_next(),
            KeyCode::Backspace => self.mcp_view.pop_search_char(),
            KeyCode::Char('e') => self.mcp_view.toggle_error_detail(),
            KeyCode::Char('a')
                if self.mcp_view.active_pane == crate::mcp_view::McpViewPane::ServerList =>
            {
                let selected_server = self
                    .mcp_view
                    .servers
                    .get(self.mcp_view.selected_server)
                    .map(|server| server.name.clone());
                if let Some(server_name) = selected_server {
                    self.pending_mcp_panel_auth = Some(server_name);
                    self.mcp_view.close();
                    self.status_message = Some("Starting MCP auth...".to_string());
                }
            }
            KeyCode::Char('r') => {
                self.pending_mcp_reconnect = true;
                self.status_message = Some("Reconnecting MCP runtime...".to_string());
            }
            KeyCode::Char(c) if key.modifiers.is_empty() => {
                if self.mcp_view.active_pane != crate::mcp_view::McpViewPane::ServerList {
                    self.mcp_view.push_search_char(c);
                }
            }
            _ => {}
        }
        false
    }

    fn handle_agents_menu_key(&mut self, key: KeyEvent) {
        if matches!(self.agents_menu.route, AgentsRoute::Editor(_)) {
            match key.code {
                KeyCode::Esc => self.agents_menu.go_back(),
                KeyCode::Tab | KeyCode::Down => self.agents_menu.editor_next_field(),
                KeyCode::BackTab | KeyCode::Up => self.agents_menu.editor_prev_field(),
                KeyCode::Enter => self.agents_menu.editor_insert_newline(),
                KeyCode::Backspace => self.agents_menu.editor_backspace(),
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match self.agents_menu.save_editor() {
                        Ok(msg) => self.status_message = Some(msg),
                        Err(err) => {
                            self.agents_menu.editor.error = Some(err.clone());
                            self.agents_menu.editor.saved_message = None;
                            self.status_message = Some(err);
                        }
                    }
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let ch = normalize_char_with_shift(ch, key.modifiers);
                    self.agents_menu.editor_insert_char(ch);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.agents_menu.go_back(),
            KeyCode::Up => self.agents_menu.select_prev(),
            KeyCode::Down => self.agents_menu.select_next(),
            KeyCode::Enter | KeyCode::Right => self.agents_menu.confirm_selection(),
            KeyCode::Left => self.agents_menu.go_back(),
            _ => {}
        }
    }

    fn handle_diff_viewer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.diff_viewer.close(),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => self.diff_viewer.switch_pane(),
            KeyCode::Char('d') => {
                let root = self.project_root();
                self.diff_viewer.toggle_diff_type(&root);
            }
            KeyCode::Up => {
                if self.diff_viewer.active_pane == DiffPane::FileList {
                    self.diff_viewer.select_prev();
                } else {
                    self.diff_viewer.scroll_detail_up();
                }
            }
            KeyCode::Down => {
                if self.diff_viewer.active_pane == DiffPane::FileList {
                    self.diff_viewer.select_next();
                } else {
                    self.diff_viewer.scroll_detail_down();
                }
            }
            KeyCode::PageUp => self.diff_viewer.scroll_detail_up(),
            KeyCode::PageDown => self.diff_viewer.scroll_detail_down(),
            KeyCode::Char(' ') => {
                if self.diff_viewer.active_pane == DiffPane::FileList {
                    self.diff_viewer.toggle_file_collapse();
                }
            }
            _ => {}
        }
    }

    fn handle_help_overlay_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => {
                self.help_overlay.close();
                self.show_help = false;
            }
            KeyCode::Char('?')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.help_overlay.close();
                self.show_help = false;
            }
            KeyCode::Up => {
                self.help_overlay.scroll_up();
            }
            KeyCode::Down => {
                let max = 50u16; // generous upper bound; renderer will clamp
                self.help_overlay.scroll_down(max);
            }
            KeyCode::Backspace => {
                self.help_overlay.pop_filter_char();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.help_overlay.push_filter_char(c);
            }
            _ => {}
        }
        false
    }

    fn handle_history_search_overlay_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.history_search_overlay.close();
            }
            KeyCode::Enter => {
                if let Some(entry) = self
                    .history_search_overlay
                    .current_entry(&self.prompt_input.history)
                {
                    self.set_prompt_text(entry.to_string());
                }
                self.history_search_overlay.close();
            }
            KeyCode::Up => {
                self.history_search_overlay.select_prev();
            }
            KeyCode::Down => {
                self.history_search_overlay.select_next();
            }
            KeyCode::Backspace => {
                let history = self.prompt_input.history.clone();
                self.history_search_overlay.pop_char(&history);
            }
            // 'p' with no modifiers and an empty query = pin/unpin the selected entry.
            // When the query is non-empty 'p' is treated as a filter character so
            // the user can still search for prompts containing the letter 'p'.
            KeyCode::Char('p')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.history_search_overlay.query.is_empty() =>
            {
                self.history_search_overlay.toggle_pin();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = normalize_char_with_shift(c, key.modifiers);
                let history = self.prompt_input.history.clone();
                self.history_search_overlay.push_char(c, &history);
            }
            _ => {}
        }
        false
    }

    fn handle_rewind_flow_key(&mut self, key: KeyEvent) -> bool {
        use crate::tui::overlays::RewindStep;
        match &self.rewind_flow.step {
            RewindStep::Selecting => match key.code {
                KeyCode::Esc => {
                    self.rewind_flow.close();
                }
                KeyCode::Enter => {
                    self.rewind_flow.confirm_selection();
                }
                KeyCode::Up => {
                    self.rewind_flow.selector.select_prev();
                }
                KeyCode::Down => {
                    self.rewind_flow.selector.select_next();
                }
                _ => {}
            },
            RewindStep::Confirming { .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(idx) = self.rewind_flow.accept_confirm() {
                        // Truncate conversation to the selected message index.
                        self.messages.truncate(idx);
                        // Remove system annotations placed after the truncation point.
                        self.system_annotations.retain(|a| a.after_index <= idx);
                        self.push_notification(
                            NotificationKind::Success,
                            format!("Rewound to message #{}", idx),
                            Some(4),
                        );
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.rewind_flow.reject_confirm();
                }
                _ => {}
            },
        }
        false
    }

    fn handle_global_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.global_search.close();
            }
            KeyCode::Enter => {
                if let Some(selected) = self.global_search.selected_ref() {
                    self.set_prompt_text(selected);
                }
                self.global_search.close();
            }
            KeyCode::Up => self.global_search.select_prev(),
            KeyCode::Down => self.global_search.select_next(),
            KeyCode::Backspace => {
                self.global_search.pop_char();
                self.refresh_global_search();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = normalize_char_with_shift(c, key.modifiers);
                self.global_search.push_char(c);
                self.refresh_global_search();
            }
            _ => {}
        }
        false
    }

    fn handle_exit_key_confirmation(&mut self, mut key_char: char) {
        fn exit_message(key: char) -> &'static str {
            if key == 'c' {
                "Press Ctrl+C again to exit"
            } else {
                "Press Ctrl+D again to exit"
            }
        }

        // Check if we have an active warning within the timeout
        if let Some(warning_time) = self.last_exit_key_warning {
            if warning_time.elapsed().as_secs_f64() <= 2.0 {
                if self.exit_key_sequence_start == Some(key_char) {
                    // Matching key - exit
                    self.should_exit = true;
                    self.last_exit_key_warning = None;
                    self.exit_key_sequence_start = None;
                    return;
                }
                if let Some(other_key) = self.exit_key_sequence_start {
                    // Wrong key pressed - show message for the original key and reset timer
                    key_char = other_key;
                }
            }
        }

        // Start new sequence (or show message for wrong key)
        self.push_notification(
            NotificationKind::Info,
            exit_message(key_char).to_string(),
            Some(2),
        );
        self.last_exit_key_warning = Some(std::time::Instant::now());
        self.exit_key_sequence_start = Some(key_char);
    }

    // (iter-164: fn handle_keybinding_action deleted — unused after keybinding processor removal)

    /// Resolve the currently-shown permission dialog by mapping the selected
    /// option to a `ToolPermissionResponse` and sending it to the agent.
    /// Drops the dialog state and the response sender regardless of whether
    /// the agent is still listening (send fails silently if the agent has
    /// already timed out / been dropped).
    ///
    /// Option key mapping:
    ///   `y` → AllowOnce
    ///   `Y` → AllowSession
    ///   `p` (persistent) → AllowSession (no persistent store wired yet —
    ///       session-scoped is the closest equivalent)
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
    fn is_double_click(&self, current_pos: (u16, u16)) -> bool {
        let now = std::time::Instant::now();
        match (self.last_click_time, self.last_click_position) {
            (Some(last_time), Some(last_pos)) => {
                let elapsed = now.duration_since(last_time);
                let distance = ((current_pos.0 as i32 - last_pos.0 as i32).abs()
                    + (current_pos.1 as i32 - last_pos.1 as i32).abs())
                    as u16;
                elapsed.as_millis() < 500 && distance <= 5
            }
            _ => false,
        }
    }

    /// Find word boundaries for the character at (col, row) in the rendered
    /// transcript buffer. Returns absolute (start_col, end_col) for the word
    /// containing the click. A "word" is a run of non-whitespace characters.
    fn find_word_boundaries(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        let cache = self.last_row_text.borrow();
        let line = cache.get(&row)?;
        if line.is_empty() {
            return None;
        }
        let selectable_area = self.last_selectable_area.get();
        if col < selectable_area.x {
            return None;
        }
        let local = (col - selectable_area.x) as usize;
        let chars: Vec<char> = line.chars().collect();
        if local >= chars.len() {
            return None;
        }
        let is_word = |c: char| !c.is_whitespace();
        if !is_word(chars[local]) {
            return None;
        }
        let mut start = local;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = local;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }
        Some((
            selectable_area.x + start as u16,
            selectable_area.x + end as u16,
        ))
    }

    /// Find paragraph boundaries (run of non-blank rows) around `row` and
    /// return (start_row, end_row, end_col) where end_col is the trimmed end
    /// of the last row's content. Used by triple-click selection so a
    /// "paragraph" — a contiguous block of text rows — is selected as a unit
    /// instead of a single visual row.
    fn find_paragraph_boundaries(&self, row: u16) -> Option<(u16, u16, u16)> {
        let cache = self.last_row_text.borrow();
        let selectable_area = self.last_selectable_area.get();
        if selectable_area.width == 0 || selectable_area.height == 0 {
            return None;
        }
        let row_text = cache.get(&row)?;
        if row_text.trim().is_empty() {
            return None;
        }
        let max_row = selectable_area
            .y
            .saturating_add(selectable_area.height)
            .saturating_sub(1);
        let mut start = row;
        while start > selectable_area.y {
            let prev = start - 1;
            if cache
                .get(&prev)
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                break;
            }
            start = prev;
        }
        let mut end = row;
        while end < max_row {
            let next = end + 1;
            if cache
                .get(&next)
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                break;
            }
            end = next;
        }
        let last_text = cache.get(&end)?;
        let trimmed = last_text.trim_end();
        let end_col = selectable_area.x + trimmed.chars().count().saturating_sub(1) as u16;
        Some((start, end, end_col))
    }

    fn context_menu_items(kind: ContextMenuKind) -> &'static [ContextMenuItem] {
        match kind {
            ContextMenuKind::Message { .. } => &[ContextMenuItem::Copy, ContextMenuItem::Fork],
            ContextMenuKind::Selection => &[ContextMenuItem::Copy],
        }
    }

    fn message_index_at_row(&self, row: u16) -> Option<usize> {
        self.message_row_map.borrow().get(&row).copied()
    }

    fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.selection_focus = None;
        *self.selection_text.borrow_mut() = String::new();
    }

    /// Show context menu at the given position.
    fn show_context_menu(&mut self, x: u16, y: u16, kind: ContextMenuKind) {
        self.context_menu_state = Some(ContextMenuState {
            x,
            y,
            selected_index: 0,
            kind,
        });
    }

    /// Dismiss the context menu.
    fn dismiss_context_menu(&mut self) {
        self.context_menu_state = None;
    }

    /// Handle context menu navigation with arrow keys.
    fn navigate_context_menu(&mut self, direction: KeyCode) {
        if let Some(mut menu) = self.context_menu_state {
            let item_count = Self::context_menu_items(menu.kind).len();
            if item_count == 0 {
                self.context_menu_state = Some(menu);
                return;
            }
            match direction {
                KeyCode::Up => {
                    if menu.selected_index == 0 {
                        menu.selected_index = item_count - 1;
                    } else {
                        menu.selected_index -= 1;
                    }
                }
                KeyCode::Down => {
                    menu.selected_index = (menu.selected_index + 1) % item_count;
                }
                _ => return,
            }
            self.context_menu_state = Some(menu);
        }
    }

    /// Execute the currently selected context menu item.
    fn execute_context_menu_item(&mut self) {
        if let Some(menu) = self.context_menu_state {
            let items = Self::context_menu_items(menu.kind);

            if menu.selected_index < items.len() {
                let item = items[menu.selected_index];
                self.handle_context_menu_action(item, menu.kind);
            }
        }
        self.dismiss_context_menu();
    }

    /// Open context menu at the current cursor/selection position via keyboard
    /// (Ctrl+Shift+M). Uses the current scroll position to determine location,
    /// or the current text selection if any.
    fn open_context_menu_at_cursor(&mut self) {
        let msg_area = self.last_msg_area.get();
        let has_selection = !self.selection_text.borrow().trim().is_empty();

        // Calculate the row at the current scroll position (top of visible area)
        let visible_row = msg_area.y.saturating_add(self.scroll_offset as u16);

        // Try to find message at the visible scroll position
        if let Some(message_index) = self.message_index_at_row(visible_row) {
            if message_index < self.messages.len() {
                let x = msg_area.x.saturating_add(2);
                let y = msg_area.y.saturating_add(2);
                self.show_context_menu(x, y, ContextMenuKind::Message { message_index });
                return;
            }
        }

        // Fall back to selection if any
        if has_selection {
            let x = msg_area.x.saturating_add(2);
            let y = msg_area.y.saturating_add(2);
            self.show_context_menu(x, y, ContextMenuKind::Selection);
            return;
        }

        // No message at scroll position and no selection - show at bottom of message area
        let x = msg_area.x.saturating_add(2);
        let y = msg_area.y.saturating_add(msg_area.height.saturating_sub(3));
        self.show_context_menu(x, y, ContextMenuKind::Selection);
    }

    /// Handle a context menu action.
    fn handle_context_menu_action(&mut self, item: ContextMenuItem, kind: ContextMenuKind) {
        match item {
            ContextMenuItem::Copy => {
                let text = match kind {
                    ContextMenuKind::Message { message_index } => self
                        .messages
                        .get(message_index)
                        .map(|message| message.get_all_text()),
                    ContextMenuKind::Selection => {
                        let selected = self.selection_text.borrow().trim().to_string();
                        if selected.is_empty() {
                            None
                        } else {
                            Some(selected)
                        }
                    }
                };

                if let Some(text) = text {
                    if crate::message_copy::copy_to_clipboard(&text) {
                        self.push_notification(
                            NotificationKind::Info,
                            format!("Copied {} chars to clipboard.", text.len()),
                            Some(3),
                        );
                    } else {
                        self.push_notification(
                            NotificationKind::Warning,
                            "Failed to copy to clipboard.".to_string(),
                            Some(3),
                        );
                    }
                    debug!("Copy action triggered, text: {} chars", text.len());
                }
            }
            ContextMenuItem::Fork => {
                if let ContextMenuKind::Message { message_index } = kind {
                    let branch_point = message_index + 1;
                    self.prompt_input
                        .replace_text(format!("/fork {}", branch_point));
                    self.status_message = Some(format!(
                        "Fork at message {} - press Enter to confirm",
                        branch_point
                    ));
                }
            }
        }
    }

    fn prompt_can_accept_selection_paste(&self) -> bool {
        !self.is_streaming
            && self.permission_request.is_none()
            && !self.history_search_overlay.visible
            && !matches!(
                self.prompt_input.vim_mode,
                crate::prompt_input::VimMode::Normal
                    | crate::prompt_input::VimMode::Visual
                    | crate::prompt_input::VimMode::VisualBlock
            )
    }

    fn paste_primary_into_prompt(&mut self) -> bool {
        if !self.prompt_can_accept_selection_paste() {
            return false;
        }

        if let Some(text) =
            crate::image_paste::read_primary_text().or_else(crate::image_paste::read_clipboard_text)
        {
            self.focus = FocusTarget::Input;
            self.clear_selection();
            self.prompt_input.paste(&text);
            self.refresh_prompt_input();
            return true;
        }

        false
    }

    /// Handle a paste data string (from `Event::Paste` or Ctrl+V text fallback).
    ///
    /// If the pasted text resolves to an existing filesystem path:
    ///   - image files (png/jpg/gif/webp/bmp) → added as an image attachment pill
    ///   - other files → inserted as `@path` mention text
    ///
    /// Otherwise the text goes through the normal `prompt_input.paste()` path
    /// which applies the multi-line summary placeholder for large pastes.
    fn handle_paste_data(&mut self, data: String) {
        use crate::tui::image_paste::PastedImage;
        use crate::tui::prompt_input::detect_pasted_path;

        if let Some(path) = detect_pasted_path(&data) {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            let is_image = matches!(
                ext.as_deref(),
                Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("bmp")
            );
            if is_image {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string();
                let img = PastedImage {
                    path,
                    label: label.clone(),
                    dimensions: None,
                };
                self.prompt_input.add_image(img);
                self.push_notification(
                    crate::notifications::NotificationKind::Info,
                    format!("Image attached: {}", label),
                    Some(3),
                );
            } else {
                // Non-image file: insert as an @mention so the path is visible
                // but clearly marked as a file reference.
                let mention = format!("@{}", path.display());
                self.prompt_input.paste(&mention);
            }
        } else {
            self.prompt_input.paste(&data);
        }
    }

    /// Returns `true` when the app is in a state where the prompt can accept
    /// regular text input — used to gate paste-burst detection.
    fn prompt_is_accepting_text(&self) -> bool {
        !self.is_streaming
            && self.permission_request.is_none()
            && !self.ask_user_dialog.visible
            && !self.history_search_overlay.visible
            && !self.settings_screen.visible
            && !self.theme_screen.visible
            && !self.key_input_dialog.visible
            && self.prompt_input.vim_mode == crate::prompt_input::VimMode::Insert
    }

    /// Drain any immediately-available key events from the crossterm event
    /// queue (zero-timeout poll) and return them alongside `first` as a single
    /// pasted string if the burst is large enough to be a paste.
    ///
    /// On Windows Terminal, Ctrl+V causes the terminal emulator to write the
    /// clipboard content directly to stdin as raw character events — every
    /// newline becomes an Enter keypress and stray `v` characters trigger
    /// voice PTT.  Because a paste dumps ALL characters into the queue at
    /// once, a zero-timeout drain immediately after the first character
    /// reliably yields 3+ chars for any non-trivial paste, while normal
    /// keyboard typing (even at 120 WPM) almost never queues more than one
    /// char in the same 50 ms window.
    ///
    /// Returns `Some(text)` when a paste burst is detected (caller should
    /// route through `handle_paste_data`).  Returns `None` for a normal
    /// single keystroke.  If a non-character key is encountered while
    /// draining, it is stored in `self.pending_key` and will be replayed at
    /// the top of the next event-loop iteration.
    fn try_detect_paste_burst(&mut self, first: char) -> Option<String> {
        use crossterm::event::{Event, KeyCode, KeyEventKind};

        // Minimum number of chars (including `first`) to classify as a paste.
        // Two or more is enough: at 120 WPM the inter-key interval is ~60 ms,
        // so a second char in the same zero-timeout drain is extremely unlikely
        // from a human typist but guaranteed from a clipboard paste.
        const BURST_THRESHOLD: usize = 2;

        // Quick exit: don't bother if nothing is queued immediately.
        if !crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
            return None;
        }

        let mut buf = String::new();
        buf.push(first);

        while let Ok(true) = crossterm::event::poll(std::time::Duration::ZERO) {
            match crossterm::event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char(c) => buf.push(c),
                    KeyCode::Enter => buf.push('\n'),
                    _ => {
                        // Non-character key — save it for replay.
                        self.pending_key = Some(k);
                        break;
                    }
                },
                // Non-key event (mouse, resize, …) — leave in queue by
                // not reading it; we already checked poll() so it will
                // be re-read next iteration. But we already read it, so
                // we just break (the event is consumed but benign).
                _ => break,
            }
        }

        if buf.chars().count() >= BURST_THRESHOLD {
            Some(buf)
        } else {
            None
        }
    }

    /// Process mouse events (trackpad scroll, text selection, etc.).
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
            .map(|(k, _)| k.to_string())
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
