// The cutover App-seam: `impl TuiState for App`.
//
// Operant's own code — not a port. The trait in
// `crate::tui::operant_app::tui_state` is the verbatim jcode port; this file is
// the field mapping that makes it live over operant's real `App`. It follows
// the seam discipline of `tui/operant_model/adapter.rs`: every method below is a
// mapping onto operant's own state, and where operant has no field to feed a
// ported widget, the trait's honest default is returned with a
// `// [port-decision]` line naming what would have to feed it. Nothing here
// fabricates plausible-looking data and nothing panics.
//
// Two shapes need calling out because they are not plain field reads:
//
// * `display_messages` / `display_messages_version` — operant's transcript is
//   `Vec<Message>` + `Vec<ToolUseBlock>` + `Vec<SystemAnnotation>`, and the
//   flattening into `DisplayMessage` is owned by
//   `tui/operant_model/adapter.rs` (`operant_model::display_messages`). The trait
//   wants `&[DisplayMessage]`, i.e. a cached row buffer. Operant has no such
//   field (all `App` fields are constructed in `app/init.rs`, which this
//   batch may not touch), so the cache lives in a `thread_local!` keyed on
//   `App::transcript_version` — the exact "one App per thread" precedent
//   `tui/clipboard.rs:371` sets, with the same `ponytail:` note. The adapter
//   allocates an owned `Vec` by design (adapter.rs:61-62), so the buffer is
//   rebuilt exactly once per transcript mutation and borrowed thereafter.
// * `side_panel` — returns `&SidePanelSnapshot`. Operant has no side-panel
//   snapshot field, so this borrows a process-wide `Default`, which is the
//   same `SidePanelSnapshot` the trait's own default path would hand a
//   render fixture.

use super::{App, TurnState};
use crate::tui::adapter_types::types::{ContentBlock, Message, MessageContent, Role};
use crate::tui::operant_app::app::ProcessingStatus;
use crate::tui::operant_app::auth::{AuthState, AuthStatus, ProviderAuth};
use crate::tui::operant_app::config_shim::{
    DiagramDisplayMode, DiagramPanePosition, DiffDisplayMode,
};
use crate::tui::operant_app::info_widget::{AuthMethod, InfoWidgetData};
use crate::tui::operant_app::prompt::ContextInfo;
use crate::tui::operant_app::side_panel::SidePanelSnapshot;
use crate::tui::operant_app::tui_fns::{
    BackgroundTaskRow, CacheTtlInfo, ContextSnapshot, PromptHistorySearchView,
};
use crate::tui::operant_app::tui_state::TuiState;
use crate::tui::operant_model::DisplayMessage;
use crate::tui::operant_model::vendor_types::ToolCall;
use ratatui::text::Line;
use std::time::{Duration, Instant};

/// Flattened transcript rows plus the `transcript_version` they were built
/// from.
///
/// The trait hands out `&[DisplayMessage]`, so the rows must outlive the call.
/// A `RefCell` cannot do that (its borrow cannot outlive the guard), and
/// operant has no `App` field holding them with `app/init.rs` out of scope for
/// this batch — so the buffer is a single leaked box per thread, never per
/// frame. One `App` per thread is an invariant the TUI already relies on
/// (`tui/clipboard.rs:371` keeps its copy-mode state the same way, with the
/// identical caveat). `ponytail:` move this onto `App` as
/// `display_messages: RefCell<Vec<DisplayMessage>>` the next time `init.rs` is
/// editable.
struct DisplayMessageCache {
    /// The `App::transcript_version` the rows below were built from.
    version: u64,
    rows: Vec<DisplayMessage>,
}

fn display_message_cache() -> &'static mut DisplayMessageCache {
    thread_local! {
        static SLOT: std::cell::Cell<*mut DisplayMessageCache> =
            const { std::cell::Cell::new(std::ptr::null_mut()) };
    }
    SLOT.with(|slot| {
        let existing = slot.get();
        if !existing.is_null() {
            // SAFETY: the pointer came from `Box::into_raw` below. The
            // allocation is never freed and is only reachable through this one
            // cell, so there is at most one live reference at any time and it
            // cannot dangle.
            return unsafe { &mut *existing };
        }
        let fresh = Box::into_raw(Box::new(DisplayMessageCache {
            // `u64::MAX` never matches a real transcript version, so the first
            // frame always builds rather than serving an empty cache.
            version: u64::MAX,
            rows: Vec::new(),
        }));
        slot.set(fresh);
        // SAFETY: `fresh` was just created here and is not aliased yet.
        unsafe { &mut *fresh }
    })
}

impl App {
    /// The transcript rows, rebuilt when the transcript has moved since the
    /// last frame.
    ///
    /// # Invariant
    /// The rows live in a leaked allocation that outlives the thread, so the
    /// returned slice stays valid for as long as the caller holds it. A later
    /// call that sees a moved `transcript_version` overwrites the buffer in
    /// place, so a caller must not hold the slice across a mutation — the
    /// renderers do not: a frame is painted from one borrow and mutations only
    /// arrive on the event path between frames.
    fn display_messages_slice(&self) -> &[DisplayMessage] {
        let cache = display_message_cache();
        let version = self.transcript_version.get();
        if cache.version != version {
            cache.rows.clear();
            cache
                .rows
                .extend(crate::tui::operant_model::display_messages(self));
            cache.version = version;
        }
        &cache.rows
    }

    /// Elapsed wall time since `anchor`, as the trait's `Duration`.
    fn since(anchor: Instant) -> Option<Duration> {
        Some(anchor.elapsed())
    }
}

/// The process-wide empty side-panel snapshot: operant has no `side_panel`
/// tool state, and `SidePanelSnapshot: Default` is exactly what a session with
/// no side panel means.
static EMPTY_SIDE_PANEL: SidePanelSnapshot = SidePanelSnapshot {
    focus_revision: 0,
    focused_page_id: None,
    pages: Vec::new(),
};

impl TuiState for App {
    // ---- Transcript ----

    fn display_messages(&self) -> &[DisplayMessage] {
        self.display_messages_slice()
    }

    fn display_user_message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| matches!(message.role, Role::User))
            .count()
    }

    fn has_display_edit_tool_messages(&self) -> bool {
        self.tool_use_blocks
            .iter()
            .any(|block| crate::tui::operant_model::tool_display::is_edit_tool_name(&block.name))
    }

    fn side_pane_images(&self) -> Vec<crate::tui::operant_app::session::RenderedImage> {
        // [port-decision] side_pane_images: operant's `PinnedImageRegistry`
        // stores terminal escape sequences, not the media payloads
        // `RenderedImage` carries, so there is nothing to map; returns empty —
        // wire when a `session::RenderedImage` producer exists on the client.
        Vec::new()
    }

    fn display_messages_version(&self) -> u64 {
        self.transcript_version.get()
    }

    fn streaming_text(&self) -> &str {
        &self.streaming_text
    }

    fn pinned_todos_payload(&self) -> Option<&str> {
        // [port-decision] pinned_todos_payload: operant has no todo list on the
        // session state; returns None — wire when a todo store lands on App
        // (the info-widget `todos` field is the same missing source).
        None
    }

    fn pinned_todos_expanded(&self) -> bool {
        // [port-decision] pinned_todos_expanded: no operant todo band exists to
        // expand; returns false — wire with pinned_todos_payload.
        false
    }

    fn background_task_rows(&self) -> &[BackgroundTaskRow] {
        // [port-decision] background_task_rows: `App::background_tasks` is a
        // `BackgroundTaskRegistry` whose rows are
        // `crate::tui::background_tasks` types, not the ported
        // `BackgroundTaskRow`; returns empty — wire when the registry grows a
        // projection into the ported row shape.
        &[]
    }

    // ---- Input ----

    fn input(&self) -> &str {
        &self.prompt_input.text
    }

    fn cursor_pos(&self) -> usize {
        self.prompt_input.cursor
    }

    fn is_processing(&self) -> bool {
        self.is_streaming
    }

    fn queued_messages(&self) -> &[String] {
        // [port-decision] queued_messages: operant's steer queue lives behind a
        // tokio `Arc<Mutex<Vec<String>>>` handle (`steer_queue_handle`) that
        // the drain owns; returns empty — wire when a snapshot of the drained
        // queue is mirrored onto App.
        &[]
    }

    fn interleave_message(&self) -> Option<&str> {
        // [port-decision] interleave_message: no operant state renders a
        // between-turns interleave notice; returns None — wire when the
        // queue-preview band lands.
        None
    }

    fn pending_soft_interrupts(&self) -> &[String] {
        // [port-decision] pending_soft_interrupts: same missing source as
        // queued_messages (the tokio steer queue handle); returns empty.
        &[]
    }

    // ---- Scroll ----

    fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    fn auto_scroll_paused(&self) -> bool {
        !self.auto_scroll
    }

    fn terminal_clear_collapsed(&self) -> bool {
        // Derived: see `App::terminal_clear_state_live` (messaging.rs). The
        // layout side (zero-height messages chunk, packed layout) is already
        // ported — operant_ui reads this predicate directly.
        self.terminal_clear_state_live()
    }

    fn pending_resize_anchor(&self) -> Option<crate::tui::operant_model::ContentPos> {
        // The ported chrome records the reader's content position every frame
        // (operant_ui::record_reader_anchor, published from the prepared frame's
        // scroll resolve). The ported ContentPos IS the target shape; operant's
        // own scroll_anchor::ContentPos was the pre-cutover type and its
        // publisher died with the dispatch table.
        crate::tui::operant_ui::resolved_reader_anchor()
    }

    fn pending_history_anchor_lines_from_bottom(&self) -> Option<usize> {
        // [port-decision] pending_history_anchor_lines_from_bottom: operant
        // prepends no compacted history at render time; returns None — wire
        // with the compacted-history loader.
        None
    }

    fn copy_selection_edge_autoscroll_active(&self) -> bool {
        // [port-decision] copy_selection_edge_autoscroll_active: operant's
        // mouse wheel smoothing is a scroll accumulator
        // (`scroll_accel`/`scroll_last_time`), not a drag-at-edge flag;
        // returns false — wire when copy-selection edge autoscroll lands.
        false
    }

    // ---- Provider ----

    fn provider_name(&self) -> String {
        self.active_provider.clone().unwrap_or_default()
    }

    fn provider_model(&self) -> String {
        self.model_name.clone()
    }

    fn upstream_provider(&self) -> Option<String> {
        // [port-decision] upstream_provider: operant's provider is set
        // directly, never as a router's upstream; returns None — wire when a
        // routed-provider field is added.
        None
    }

    fn connection_type(&self) -> Option<String> {
        // [port-decision] connection_type: operant has no websocket/https
        // transport label on App; returns None — wire from the transport layer.
        None
    }

    fn status_detail(&self) -> Option<String> {
        self.status_message.clone()
    }

    fn mcp_servers(&self) -> Vec<(String, usize)> {
        self.mcp_view
            .servers
            .iter()
            .map(|server| (server.name.clone(), server.tool_count))
            .collect()
    }

    fn available_skills(&self) -> Vec<String> {
        self.skills_view
            .skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect()
    }

    // ---- Stream / status ----

    fn streaming_tokens(&self) -> (u64, u64) {
        (self.turn_input_tokens, self.turn_output_tokens)
    }

    fn streaming_cache_tokens(&self) -> (Option<u64>, Option<u64>) {
        (
            Some(self.turn_cache_read_tokens),
            Some(self.turn_cache_write_tokens),
        )
    }

    fn output_tps(&self) -> Option<f32> {
        let elapsed = self.turn_started_at?.elapsed().as_secs_f32();
        if elapsed <= 0.0 || self.turn_output_tokens == 0 {
            return None;
        }
        Some(self.turn_output_tokens as f32 / elapsed)
    }

    fn streaming_tool_calls(&self) -> Vec<ToolCall> {
        self.tool_use_blocks
            .iter()
            .filter(|block| block.status.is_pending())
            .map(|block| ToolCall {
                id: block.id.clone(),
                name: block.name.clone(),
                input: serde_json::from_str(&block.input_json).unwrap_or(serde_json::Value::Null),
                intent: None,
                thought_signature: None,
            })
            .collect()
    }

    fn elapsed(&self) -> Option<Duration> {
        self.turn_started_at.map(|started| started.elapsed())
    }

    fn connection_phase_elapsed(&self) -> Option<Duration> {
        // [port-decision] connection_phase_elapsed: operant emits no
        // connection-phase update during a turn; returns None — wire from the
        // transport layer's Connecting phase.
        None
    }

    fn status(&self) -> ProcessingStatus {
        match self.display_turn_state() {
            TurnState::Idle => ProcessingStatus::Idle,
            TurnState::Sending => ProcessingStatus::Sending,
            TurnState::Connecting => ProcessingStatus::Sending,
            TurnState::Thinking => {
                ProcessingStatus::Thinking(self.turn_started_at.unwrap_or_else(Instant::now))
            }
            TurnState::Streaming => ProcessingStatus::Streaming,
            TurnState::RunningTool => {
                let name = self
                    .tool_use_blocks
                    .iter()
                    .rev()
                    .find(|block| block.status.is_pending())
                    .map(|block| block.name.clone())
                    .unwrap_or_default();
                ProcessingStatus::RunningTool(name)
            }
            TurnState::WaitingForApproval => ProcessingStatus::RunningTool("permission".into()),
            TurnState::WaitingForNetwork => ProcessingStatus::WaitingForNetwork {
                listener: self.spinner_verb.clone().unwrap_or_default(),
            },
            TurnState::Compacting => ProcessingStatus::RunningTool("compacting".into()),
        }
    }

    fn command_suggestions(&self) -> Vec<(String, String)> {
        // Descriptions are owned (`TypeaheadSuggestion.description`, built per
        // keystroke in prompt_input/typeahead.rs), so the tuple carries String
        // rather than &'static str — widening the seam instead of dropping the
        // descriptions the overlay renders.
        self.prompt_input
            .suggestions
            .iter()
            .map(|suggestion| (suggestion.text.clone(), suggestion.description.clone()))
            .collect()
    }

    fn command_suggestion_selected(&self) -> usize {
        self.prompt_input.suggestion_index.unwrap_or(0)
    }

    fn prompt_history_search(&self) -> Option<PromptHistorySearchView> {
        if !self.history_search_overlay.visible {
            return None;
        }
        Some(PromptHistorySearchView {
            query: self.history_search_overlay.query.clone(),
            matches: self
                .history_search_overlay
                .matches
                .iter()
                .filter_map(|matched| {
                    self.history_search_overlay
                        .snapshot
                        .get(matched.snapshot_idx)
                        .map(|entry| entry.text.clone())
                })
                .collect(),
            selected: self.history_search_overlay.selected_idx,
        })
    }

    fn active_skill(&self) -> Option<String> {
        // [port-decision] active_skill: operant resolves a skill at submit time
        // (slash-command expansion) and keeps no "currently active" field;
        // returns None — wire when a turn-scoped skill is retained.
        None
    }

    fn subagent_status(&self) -> Option<String> {
        self.agent_status
            .first()
            .map(|(name, status)| format!("{name}: {status}"))
    }

    fn batch_progress(&self) -> Option<crate::tui::operant_app::bus::BatchProgress> {
        // [port-decision] batch_progress: operant executes tools on its own
        // worker pool and never emits a batch sub-call event; returns None —
        // wire when a batch-tool event exists.
        None
    }

    fn time_since_activity(&self) -> Option<Duration> {
        App::since(self.last_activity)
    }

    fn client_focused(&self) -> bool {
        if self.is_simulating {
            return false; // simulated runs never claim focus; suppresses decorative animations
        }
        self.client_focused
    }

    fn total_session_tokens(&self) -> Option<(u64, u64)> {
        Some((self.context_used_tokens, self.turn_output_tokens))
    }

    fn session_compaction_count(&self) -> usize {
        // [port-decision] session_compaction_count: operant compacts in place
        // and keeps no running count; returns 0 — wire when the compaction
        // event increments a counter on App.
        0
    }

    // ---- Session / server ----

    fn is_remote_mode(&self) -> bool {
        matches!(
            self.bridge_state,
            crate::tui::bridge_state::BridgeConnectionState::Connected { .. }
        )
    }

    fn is_canary(&self) -> bool {
        // [port-decision] is_canary: operant has no canary/self-dev channel
        // flag; returns false — wire from the build channel.
        false
    }

    fn is_replay(&self) -> bool {
        self.is_simulating
    }

    fn diff_mode(&self) -> DiffDisplayMode {
        DiffDisplayMode::Off
    }

    fn current_session_id(&self) -> Option<String> {
        // [port-decision] current_session_id: operant's session identity is the
        // `session_title` display string plus the loaded-session path, with no
        // stable id on App; returns None — wire when a session id is retained.
        None
    }

    fn session_display_name(&self) -> Option<String> {
        self.session_title.clone()
    }

    fn server_display_name(&self) -> Option<String> {
        match &self.bridge_state {
            crate::tui::bridge_state::BridgeConnectionState::Connected { .. } => {
                Some("bridge".to_string())
            }
            _ => None,
        }
    }

    fn server_display_icon(&self) -> Option<String> {
        self.is_remote_mode().then(|| "🌫️".to_string())
    }

    fn server_sessions(&self) -> Vec<String> {
        // [port-decision] server_sessions: operant's bridge exposes a session
        // URL, not a server-side session list; returns empty — wire when the
        // bridge reports its session index.
        Vec::new()
    }

    fn connected_clients(&self) -> Option<usize> {
        match &self.bridge_state {
            crate::tui::bridge_state::BridgeConnectionState::Connected { peer_count, .. } => {
                Some(*peer_count as usize)
            }
            _ => None,
        }
    }

    fn status_notice(&self) -> Option<String> {
        self.status_message.clone()
    }

    fn time_since_user_interaction(&self) -> Option<Duration> {
        App::since(self.last_activity)
    }

    fn remote_startup_phase_active(&self) -> bool {
        matches!(
            self.bridge_state,
            crate::tui::bridge_state::BridgeConnectionState::Connecting
                | crate::tui::bridge_state::BridgeConnectionState::Reconnecting { .. }
        )
    }

    fn has_pending_mouse_scroll_animation(&self) -> bool {
        // [port-decision] has_pending_mouse_scroll_animation: operant smooths
        // wheel deltas in the same tick (`scroll_accel` is private and has no
        // "lines still queued" flag); returns false — wire when the smoother
        // exposes its queue depth.
        false
    }

    fn dictation_key_label(&self) -> Option<String> {
        // [port-decision] dictation_key_label: no built-in dictation surface
        // exists after the voice-mode purge (iter-668); no label to show.
        None
    }

    fn animation_elapsed(&self) -> f32 {
        self.session_start.elapsed().as_secs_f32()
    }

    fn rate_limit_remaining(&self) -> Option<Duration> {
        // [port-decision] rate_limit_remaining: operant records rate-limit
        // percentages (`rate_limit_5h_pct`/`rate_limit_7day_pct`) and a retry
        // notice, but keeps no reset instant; returns None — wire when the
        // RateLimitNotice payload is stored as a deadline.
        None
    }

    fn queue_mode(&self) -> bool {
        self.plan_mode
    }

    fn has_stashed_input(&self) -> bool {
        self.prompt_input.stash.is_some()
    }

    fn context_info(&self) -> ContextInfo {
        let mut info = ContextInfo::default();
        // A message's char count must cover the same content the adapter
        // flattens, so this walks `MessageContent` exactly as
        // `adapter::expand_message` does: the `Text` arm verbatim, the
        // `Blocks` arm summing the `Text` blocks (non-text blocks already land
        // in their own rows).
        let chars = |message: &Message| -> usize {
            match &message.content {
                MessageContent::Text(text) => text.len(),
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.len()),
                        _ => None,
                    })
                    .sum(),
            }
        };
        let messages_chars: usize = self.messages.iter().map(chars).sum();
        info.user_messages_count = self
            .messages
            .iter()
            .filter(|message| matches!(message.role, Role::User))
            .count();
        info.assistant_messages_count = self
            .messages
            .iter()
            .filter(|message| matches!(message.role, Role::Assistant))
            .count();
        info.user_messages_chars = self
            .messages
            .iter()
            .filter(|message| matches!(message.role, Role::User))
            .map(chars)
            .sum();
        info.assistant_messages_chars = self
            .messages
            .iter()
            .filter(|message| matches!(message.role, Role::Assistant))
            .map(chars)
            .sum();
        info.tool_calls_count = self.tool_use_blocks.len();
        info.tool_calls_chars = self
            .tool_use_blocks
            .iter()
            .map(|block| {
                block.input_json.len() + block.output_preview.as_deref().map_or(0, str::len)
            })
            .sum();
        info.tool_results_chars = self
            .tool_use_blocks
            .iter()
            .filter_map(|block| block.output_preview.as_deref().map(str::len))
            .sum();
        info.tool_results_count = info.tool_calls_count;
        // [port-decision] tool_defs_count/tool_defs_chars: the registry behind
        // `core_tool_registry` is an async-locked `Arc<RwLock<HashMap<..>>>`
        // with no cheap len()/char-count accessor, and a render method must
        // not block on it; left at 0 — wire when the registry exposes a
        // snapshot of its tool schema sizes.
        info.tool_defs_count = 0;
        info.tool_defs_chars = 0;
        info.total_chars = messages_chars + info.tool_calls_chars;
        info
    }

    fn context_limit(&self) -> Option<usize> {
        (self.context_window_size > 0).then_some(self.context_window_size as usize)
    }

    fn context_snapshot(&self) -> ContextSnapshot {
        let info = self.context_info();
        ContextSnapshot {
            info: (info.total_chars > 0).then_some(info),
            revision: self.transcript_version.get(),
            fresh: !self.is_streaming,
        }
    }

    fn client_update_available(&self) -> bool {
        // [port-decision] client_update_available: operant's update check
        // publishes a notification rather than a sticky flag on App; returns
        // false — wire the check's "newer version found" result onto App.
        false
    }

    fn server_update_available(&self) -> Option<bool> {
        // [port-decision] server_update_available: no bridge-side version
        // check exists; returns None — wire with the bridge's version probe.
        None
    }

    fn info_widget_data(&self) -> InfoWidgetData {
        InfoWidgetData {
            context_info: Some(self.context_info()),
            context_limit: self.context_limit(),
            queue_mode: Some(self.plan_mode),
            model: Some(self.model_name.clone()),
            provider_name: self.active_provider.clone(),
            working_dir: self.current_dir.clone(),
            tokens_per_second: self.output_tps(),
            auth_method: if self.has_credentials {
                AuthMethod::ApiKey
            } else {
                AuthMethod::Unknown
            },
            git_info: None,
            todos: Vec::new(),
            agent_edited: std::sync::Arc::new(std::collections::HashSet::new()),
            ..Default::default()
        }
    }

    // ---- Workspace ----

    fn workspace_mode_enabled(&self) -> bool {
        // [port-decision] workspace_mode_enabled: operant has no workspace map
        // surface; returns false — wire when workspace mode is added.
        false
    }

    fn render_streaming_markdown(&self, width: usize) -> Vec<Line<'static>> {
        crate::tui::operant_app::markdown::render_markdown_with_width(
            &self.streaming_text,
            Some(width),
        )
    }

    fn centered_mode(&self) -> bool {
        // [port-decision] centered_mode: operant renders left-aligned; returns
        // false — wire when a centered transcript layout is added.
        false
    }

    fn auth_status(&self) -> AuthStatus {
        let mut status = AuthStatus::default();
        // Operant keeps one boolean ("are there usable credentials for the
        // session's provider"), not a per-provider matrix. Map it onto the
        // active provider's slot and leave the rest at their default.
        let state = if self.has_credentials {
            AuthState::Available
        } else {
            AuthState::NotConfigured
        };
        match self.active_provider.as_deref() {
            Some("anthropic") => {
                status.anthropic = ProviderAuth {
                    state,
                    has_oauth: false,
                    oauth_state: state,
                    has_api_key: self.has_credentials,
                };
            }
            Some("openai") => {
                status.openai = state;
                status.openai_has_api_key = self.has_credentials;
            }
            Some("openrouter") => status.openrouter = state,
            Some("azure") => status.azure = state,
            Some("copilot") => {
                status.copilot = state;
                status.copilot_has_api_token = self.has_credentials;
            }
            Some("gemini") => status.gemini = state,
            _ => {}
        }
        status
    }

    fn update_cost(&mut self) {
        if let Some(tracker) = std::sync::Arc::get_mut(&mut self.cost_tracker) {
            tracker.record_usage(
                self.turn_input_tokens as u32,
                self.turn_output_tokens as u32,
            );
            self.cost_usd = tracker.total_cost;
        }
    }

    // ---- Diagram pane ----

    fn diagram_mode(&self) -> DiagramDisplayMode {
        DiagramDisplayMode::None
    }

    fn diagram_focus(&self) -> bool {
        false
    }

    fn diagram_index(&self) -> usize {
        0
    }

    fn diagram_scroll(&self) -> (i32, i32) {
        (0, 0)
    }

    fn diagram_pane_ratio(&self) -> u8 {
        0
    }

    fn diagram_pane_ratio_user_adjusted(&self) -> bool {
        false
    }

    fn diagram_pane_animating(&self) -> bool {
        false
    }

    fn diagram_pane_enabled(&self) -> bool {
        false
    }

    fn diagram_pane_position(&self) -> DiagramPanePosition {
        DiagramPanePosition::Side
    }

    fn diagram_zoom(&self) -> u8 {
        100
    }

    // ---- Diff pane ----

    fn diff_pane_scroll(&self) -> usize {
        self.diff_viewer.detail_scroll as usize
    }

    fn diff_pane_scroll_x(&self) -> i32 {
        0
    }

    fn side_panel_image_zoom_percent(&self) -> u8 {
        100
    }

    fn diff_pane_focus(&self) -> bool {
        false
    }

    // ---- Side panel ----

    fn side_panel(&self) -> &SidePanelSnapshot {
        &EMPTY_SIDE_PANEL
    }

    fn pin_images(&self) -> bool {
        !self.pinned_images.is_empty()
    }

    // ---- Native scrollbars ----

    fn chat_native_scrollbar(&self) -> bool {
        false
    }

    fn side_panel_native_scrollbar(&self) -> bool {
        false
    }

    // ---- Inline ----

    fn inline_interactive_state(
        &self,
    ) -> Option<&crate::tui::operant_app::tui_fns::InlineInteractiveState> {
        // [port-decision] inline_interactive_state: operant's pickers are
        // separate overlay structs (`model_picker`, `agents_menu`,
        // `skills_view`), not the ported unified picker; returns None — wire
        // when the ported picker state lands.
        None
    }

    // ---- Overlay ----

    fn changelog_scroll(&self) -> Option<usize> {
        // [port-decision] changelog_scroll: operant has no changelog overlay;
        // returns None — wire when one lands.
        None
    }

    fn help_scroll(&self) -> Option<usize> {
        self.help_overlay
            .visible
            .then_some(self.help_overlay.scroll_offset as usize)
    }

    fn working_dir(&self) -> Option<String> {
        self.current_dir.clone()
    }

    fn git_branch(&self) -> Option<String> {
        self.git_branch.clone()
    }

    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as u64)
            .unwrap_or(0)
    }

    // ---- Copy selection ----

    fn copy_badge_ui(&self) -> crate::tui::operant_app::app::CopyBadgeUiState {
        // [port-decision] copy_badge_ui: operant's copy feedback is a plain
        // status message, with no badge pulse timers; returns default — wire
        // when the copy badge's Alt/Shift/key pulses land.
        Default::default()
    }

    fn copy_selection_mode(&self) -> bool {
        self.selection_anchor.is_some() && self.selection_focus.is_some()
    }

    fn copy_selection_range(
        &self,
    ) -> Option<crate::tui::operant_ui::copy_selection::CopySelectionRange> {
        use crate::tui::operant_ui::copy_selection::{
            CopySelectionPane, CopySelectionPoint, CopySelectionRange,
        };
        // Selection points are `(col, content_line)` in scroll-stable content
        // space (iter-672) — exactly jcode's `(abs_line, column)` model.
        let (anchor_col, anchor_line) = self.selection_anchor?;
        let (focus_col, focus_line) = self.selection_focus?;
        let point = |line: usize, col: u16| CopySelectionPoint {
            pane: CopySelectionPane::Chat,
            abs_line: line,
            column: col as usize,
        };
        Some(CopySelectionRange {
            start: point(anchor_line, anchor_col),
            end: point(focus_line, focus_col),
        })
    }

    fn copy_selection_status(
        &self,
    ) -> Option<crate::tui::operant_ui::copy_selection::CopySelectionStatus> {
        use crate::tui::operant_ui::copy_selection::{CopySelectionPane, CopySelectionStatus};
        self.copy_selection_mode().then(|| CopySelectionStatus {
            pane: CopySelectionPane::Chat,
            has_action: !self.selection_text.borrow().is_empty(),
            selected_chars: self.selection_text.borrow().chars().count(),
            selected_lines: match (self.selection_anchor, self.selection_focus) {
                (Some((_, start_line)), Some((_, end_line))) => {
                    end_line.max(start_line).saturating_sub(end_line.min(start_line)) + 1
                }
                _ => 0,
            },
            dragging: self.last_click_position.is_some(),
        })
    }

    // ---- Onboarding ----

    fn suggestion_prompts(&self) -> Vec<(String, String)> {
        // [port-decision] suggestion_prompts: operant's onboarding has no
        // starter prompt cards; returns empty — wire when they are authored.
        Vec::new()
    }

    fn cache_ttl_status(&self) -> Option<CacheTtlInfo> {
        // [port-decision] cache_ttl_status: operant receives cache-read token
        // counts but never a provider retention window; returns None — wire
        // from the provider's cache TTL in the usage payload.
        None
    }

    fn has_notification(&self) -> bool {
        !self.notifications.notifications.is_empty()
    }
}
