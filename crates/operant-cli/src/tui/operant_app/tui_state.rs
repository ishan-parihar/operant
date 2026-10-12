// Vendored from jcode (crates/operant-tui/src/tui/mod.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
//! This is the cutover App-seam contract: the read-only TuiState presentation
//! surface operant's App implements and the shared renderers take as
//! `&dyn TuiState`. Included from tui/mod.rs: hash_rendered_image_anchor (:353)
//! and hash_rendered_image_signature_fields (:375), shared by the trait's
//! default `side_pane_images_signature` and the App override, and the TuiState
//! trait itself (:387 doc, :395-916) — every method, default impl and doc
//! comment verbatim (incl. the docs/TUISTATE_TRAIT_DECOMPOSITION.md reference),
//! imports re-rooted to the operant_app/operant_ui homes verified on disk.
//! OnboardingWelcomeKind with its LoginImportPrompt, ImportSummaryPill,
//! TelemetryChoice and LoginImportRow closure — excised 2026-10-09 with the
//! welcome-takeover gate (upstream onboarding module was never vendored;
//! operant ships its own connect-dialog onboarding). is_ssh_remote (:1791) is
//! ported here beside the
//! trait, mirroring the upstream layout: they live in tui/mod.rs next to
//! TuiState upstream, and their natural operant home file (tui_fns.rs, the
//! mod.rs port) is not modifiable in this batch.
//! [port-decision] added at batch-3: TuiState trait dependency.
//! [port-excision] from the TuiState trait: workspace_map_rows (:703-706),
//! session_picker_overlay, login_picker_overlay, account_picker_overlay and
//! usage_overlay (:809-816) — the identical excision the landed tui_fns.rs
//! port documents (tui_fns.rs:18-21): their payload modules (workspace_map,
//! the four overlay pickers) are unported waves (session_picker and
//! usage_overlay are W7; login/account pickers sit on W8, the plan's
//! recommended auth cut line — docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md).
//! No ported renderer calls them, and restoring them now would force the
//! cutover App to carry state for widgets that cannot render yet.
//! [port-decision] the `openai_reset_hint` default body references
//! crate::tui::operant_app::usage::get_openai_usage_sync and
//! crate::tui::operant_app::auth::codex::active_account_label. Neither has an
//! operant home: their verbatim closures cross into the usage network-fetch
//! subsystem (operant-base/src/usage/accessors.rs:85-165, which spawns
//! provider_fetch.rs fetches with OAuth token refresh) and the auth-file I/O
//! subsystem (operant-base/src/auth/codex.rs:160-204, which pulls account_store.rs
//! and the operant-storage secret machinery). Those are wave-scale, not
//! leaf-scale. The call paths are re-rooted to their eventual operant_app homes;
//! the symbols land with their waves (reported as this batch's unresolved
//! items).
use crate::tui::operant_app::app::ProcessingStatus;
use crate::tui::operant_app::info_widget;
use crate::tui::operant_app::tui_fns::{
    BackgroundTaskRow, CacheTtlInfo, ContextSnapshot, InlineInteractiveState, InlineUiStateRef,
    InlineViewState, PromptHistorySearchView, scheduled_notification_text, ssh_remote_host,
};
use crate::tui::operant_model::DisplayMessage;
use crate::tui::operant_model::vendor_types::ToolCall;
use crate::tui::operant_ui::copy_selection::{CopySelectionRange, CopySelectionStatus};
use crate::tui::operant_ui::inline_image_ui::ImageExpandLevel;
use ratatui::text::Line;
use std::time::Duration;

/// Hash a rendered image's transcript anchor into `hasher`. Shared by the
/// default and `App` implementations of `side_pane_images_signature` so both
/// stay in lockstep.
pub(crate) fn hash_rendered_image_anchor(
    anchor: Option<&crate::tui::operant_app::session::RenderedImageAnchor>,
    hasher: &mut impl std::hash::Hasher,
) {
    use std::hash::Hash;
    match anchor {
        None => 0u8.hash(hasher),
        Some(crate::tui::operant_app::session::RenderedImageAnchor::ToolCall { id }) => {
            1u8.hash(hasher);
            id.hash(hasher);
        }
        Some(crate::tui::operant_app::session::RenderedImageAnchor::UserPrompt { ordinal }) => {
            2u8.hash(hasher);
            ordinal.hash(hasher);
        }
    }
}

/// Hash every field that affects inline image rendering. The production App
/// memoizes this signature until its image set changes, so exact payload hashing
/// happens on image updates rather than during scrolling. Correctness matters
/// here: sampling can miss a same-length change and reuse a stale prepared frame.
pub(crate) fn hash_rendered_image_signature_fields(
    image: &crate::tui::operant_app::session::RenderedImage,
    hasher: &mut impl std::hash::Hasher,
) {
    use std::hash::Hash;

    image.media_type.hash(hasher);
    image.data.hash(hasher);
    image.label.hash(hasher);
    hash_rendered_image_anchor(image.anchor.as_ref(), hasher);
}

/// Trait for TUI state consumed by the shared renderer.
///
/// This is a wide (114-method) presentation interface: the read-only surface the
/// renderer needs from `App`. The methods are grouped into the domain sections
/// below (transcript, input, scroll, stream/status, provider, session/server,
/// workspace, diagram pane, diff pane, side panel, inline, overlay, copy
/// selection, onboarding, misc). See `docs/TUISTATE_TRAIT_DECOMPOSITION.md` for
/// the incremental plan to split these into composable sub-traits.
pub trait TuiState {
    // ---- Transcript ----
    fn display_messages(&self) -> &[DisplayMessage];
    fn display_user_message_count(&self) -> usize;
    /// Number of user prompts hidden before the first visible message because of
    /// compacted-history truncation. Used to keep prompt numbers absolute.
    fn compacted_hidden_user_prompts(&self) -> usize {
        0
    }
    fn has_display_edit_tool_messages(&self) -> bool;
    fn side_pane_images(&self) -> Vec<crate::tui::operant_app::session::RenderedImage>;
    /// Cheap signature of the current inline-image set: `(count, content_hash)`.
    /// Used by the prepared-frame cache so the inline image section invalidates
    /// when images are added/removed without cloning the payloads every frame.
    /// The default implementation derives it from `side_pane_images`; overrides
    /// can provide a cheaper path.
    fn side_pane_images_signature(&self) -> (usize, u64) {
        use std::hash::Hasher;
        let images = self.side_pane_images();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for image in &images {
            hash_rendered_image_signature_fields(image, &mut hasher);
        }
        (images.len(), hasher.finish())
    }
    /// Version counter for display_messages (monotonic, increments on mutation)
    fn display_messages_version(&self) -> u64;
    fn streaming_text(&self) -> &str;
    /// JSON payload for the pinned todo band rendered at the top of the chat
    /// viewport when `display.pin_todos` is enabled. `None` when the feature
    /// is off or the session has no todos.
    fn pinned_todos_payload(&self) -> Option<&str> {
        None
    }
    /// Whether the pinned todo band is temporarily expanded to show every row.
    fn pinned_todos_expanded(&self) -> bool {
        false
    }
    /// Running and recently completed background tasks rendered beneath pinned todos.
    fn background_task_rows(&self) -> &[BackgroundTaskRow] {
        &[]
    }

    // ---- Input ----
    fn input(&self) -> &str;
    fn cursor_pos(&self) -> usize;
    /// [operant extension] Active composer shift-selection as normalized
    /// byte offsets, if any (jcode textarea selection). Default: none.
    fn input_selection(&self) -> Option<(usize, usize)> {
        None
    }
    fn is_processing(&self) -> bool;
    fn queued_messages(&self) -> &[String];
    fn interleave_message(&self) -> Option<&str>;
    /// Messages sent as soft interrupt but not yet injected (shown in queue preview)
    fn pending_soft_interrupts(&self) -> &[String];

    // ---- Scroll ----
    fn scroll_offset(&self) -> usize;
    /// Whether auto-scroll to bottom is paused (user scrolled up during streaming)
    fn auto_scroll_paused(&self) -> bool;
    /// Whether the screen is currently in the terminal-style cleared state
    /// produced by Ctrl+L / Cmd+L: the transcript ends in a blank spacer, the
    /// view is pinned to the bottom, and nothing is streaming. In that state
    /// the renderer collapses the (entirely blank) messages viewport so the
    /// status line and numbered prompt sit at the *top* of the screen, exactly
    /// like a terminal after `clear`, instead of floating at the bottom under
    /// a screenful of blanks.
    fn terminal_clear_collapsed(&self) -> bool {
        false
    }
    /// Content-coordinate reading position captured before a resize rewrapped
    /// the transcript. The renderer resolves it against the frame it is drawing
    /// so the anchored message stays under the reader.
    fn pending_resize_anchor(&self) -> Option<crate::tui::operant_model::ContentPos> {
        None
    }
    /// When older compacted history is being loaded in, this is the reader's
    /// captured distance (in wrapped lines) from the bottom of the transcript.
    /// The renderer uses it to keep the viewport anchored to the same content as
    /// older messages are prepended above, instead of snapping to the new top.
    fn pending_history_anchor_lines_from_bottom(&self) -> Option<usize> {
        None
    }
    /// Whether a mouse drag-selection is currently held at the top/bottom edge of
    /// a pane and should keep auto-scrolling on every tick (browser-style). When
    /// true the redraw loop must stay responsive even if the transcript is
    /// otherwise idle, since the terminal sends no further events while the mouse
    /// is held still.
    fn copy_selection_edge_autoscroll_active(&self) -> bool {
        false
    }

    // ---- Provider ----
    fn provider_name(&self) -> String;
    fn provider_model(&self) -> String;
    /// Upstream provider (e.g., which provider OpenRouter routed to)
    fn upstream_provider(&self) -> Option<String>;
    /// Active transport/connection type (websocket/https/etc.)
    fn connection_type(&self) -> Option<String>;
    /// Provider-supplied human-readable status detail for the current stream.
    fn status_detail(&self) -> Option<String>;
    fn mcp_servers(&self) -> Vec<(String, usize)>;
    fn available_skills(&self) -> Vec<String>;
    /// Authoritative active credential (OAuth vs API key) for a dual-auth
    /// provider, as resolved from the live provider / remote server rather than
    /// from the `OPERANT_RUNTIME_PROVIDER` env var. The header must prefer this
    /// over its own env-based heuristic: the TUI client process often does not
    /// inherit `OPERANT_RUNTIME_PROVIDER` (it is set inside the agent/server
    /// process), so the env heuristic silently falls back to "auto prefers
    /// OAuth" and the header claimed OAuth while the info widget correctly
    /// reported an API key. Returns `None` when the credential cannot be
    /// determined, in which case callers fall back to the cached `AuthStatus`.
    fn active_dual_credential(
        &self,
        _provider: crate::tui::operant_app::provider::ActiveProvider,
    ) -> Option<crate::tui::operant_app::auth::ActiveCredential> {
        None
    }

    // ---- Stream / status ----
    fn streaming_tokens(&self) -> (u64, u64);
    fn streaming_cache_tokens(&self) -> (Option<u64>, Option<u64>);
    /// Output tokens per second during streaming (for status bar)
    fn output_tps(&self) -> Option<f32>;
    fn streaming_tool_calls(&self) -> Vec<ToolCall>;
    fn elapsed(&self) -> Option<Duration>;
    /// Time since the current connection phase (authenticating/connecting/
    /// waiting for response/retrying) began. Used to decide when a connection
    /// attempt has been suspiciously long and should render yellow, measured
    /// per-attempt rather than inheriting the whole-turn elapsed time. Defaults
    /// to `elapsed()` for impls that do not track per-phase timing.
    fn connection_phase_elapsed(&self) -> Option<Duration> {
        self.elapsed()
    }
    fn status(&self) -> ProcessingStatus;
    fn command_suggestions(&self) -> Vec<(String, String)>;
    /// Invalidate any per-frame memo backing [`Self::command_suggestions`].
    ///
    /// Called once at the top of each rendered frame. The suggestion list is
    /// read many times while composing a single frame; implementations may
    /// cache within a frame but must not serve that cache across frames, since
    /// the list also depends on mutable session state. Defaults to a no-op for
    /// impls that do not cache.
    fn advance_command_suggestions_epoch(&self) {}
    fn command_suggestion_selected(&self) -> usize {
        0
    }
    /// Snapshot of the Ctrl+R reverse prompt-history search overlay, or None
    /// when the overlay is closed.
    fn prompt_history_search(&self) -> Option<PromptHistorySearchView> {
        None
    }
    fn active_skill(&self) -> Option<String>;
    fn subagent_status(&self) -> Option<String>;
    /// Progress of a currently-running batch tool call.
    fn batch_progress(&self) -> Option<crate::tui::operant_app::bus::BatchProgress>;
    fn time_since_activity(&self) -> Option<Duration>;
    /// Whether the client terminal currently has focus. Decorative animations and
    /// periodic idle redraws pause while unfocused so backgrounded windows/tabs do
    /// not burn CPU. Defaults to true for state impls that do not track focus.
    fn client_focused(&self) -> bool {
        true
    }
    /// Whether the provider/server has ended the visible assistant message while turn cleanup
    /// still finishes in the background.
    fn stream_message_ended(&self) -> bool {
        false
    }
    /// Total session token usage (input, output) - used for high usage warnings
    fn total_session_tokens(&self) -> Option<(u64, u64)>;
    /// Number of jcode compactions already applied to this session, when known.
    fn session_compaction_count(&self) -> usize {
        0
    }

    // ---- Session / server ----
    /// Whether running in remote (client-server) mode
    fn is_remote_mode(&self) -> bool;
    /// Whether running in canary/self-dev mode
    fn is_canary(&self) -> bool;
    /// Whether running in replay mode
    fn is_replay(&self) -> bool;
    /// Diff display mode (off/inline/full-inline/file)
    fn diff_mode(&self) -> crate::tui::operant_app::config_shim::DiffDisplayMode;
    /// Current session ID (if available)
    fn current_session_id(&self) -> Option<String>;
    /// Session display name (memorable short name like "fox" or "oak")
    fn session_display_name(&self) -> Option<String>;
    /// Server display name (modifier like "running" or "blazing") - only set in remote mode
    fn server_display_name(&self) -> Option<String>;
    /// Server icon (e.g., "🔥", "🌫️") - only set in remote mode
    fn server_display_icon(&self) -> Option<String>;
    /// Server binary version (e.g., "v0.25.19-dev (abc1234)") - remote mode only
    fn server_display_version(&self) -> Option<String> {
        None
    }
    /// List of all session IDs on the server (remote mode only)
    fn server_sessions(&self) -> Vec<String>;
    /// Number of connected clients (remote mode only)
    fn connected_clients(&self) -> Option<usize>;
    /// Short-lived notice shown in the status line (e.g., model switch, toggle diff)
    fn status_notice(&self) -> Option<String>;
    /// Whether thinking/reasoning traces render (jcode display.show_thinking,
    /// toggled by /thinking-display; default on).
    fn show_thinking(&self) -> bool {
        true
    }
    /// Whether terminal-scrollback mode (P5-1 inline viewport) is active.
    /// Gates P5-2 emission and the viewport watermark floor.
    fn terminal_scroll_mode(&self) -> bool {
        false
    }
    /// Last-emitted-row watermark (P5-2): rows below this live in native
    /// scrollback already; the live viewport floors its window here.
    fn scroll_emitted_rows(&self) -> usize {
        0
    }
    /// How long since the user last pressed a key, scrolled, or pasted, or
    /// `None` when they have not interacted yet.
    ///
    /// Distinct from [`time_since_activity`], which tracks provider output:
    /// typing into an idle session produces no stream events, so only this can
    /// tell "actively composing" from "sitting untouched".
    fn time_since_user_interaction(&self) -> Option<Duration> {
        None
    }
    /// Distinct learned-keybinding nudge shown in its own pop-out color, e.g.
    /// "you usually do X the slow way, press <key>". Separate from
    /// [`status_notice`] so the UI can style it differently.
    fn learn_hint(&self) -> Option<String> {
        None
    }
    /// Inline hotkey feedback: "you just pressed X → does Y" for rarely-used
    /// known chords, or "X isn't bound · nearest ..." for unknown chords.
    fn hotkey_feedback(&self) -> Option<String> {
        None
    }
    /// First-use experimental feature warning for the currently active operation.
    fn active_experimental_feature_notice(&self) -> Option<String> {
        None
    }
    /// Whether a transient remote startup phase is active and should keep redraws responsive.
    fn remote_startup_phase_active(&self) -> bool;
    /// Whether mouse-wheel smoothing has queued lines to animate.
    fn has_pending_mouse_scroll_animation(&self) -> bool {
        false
    }
    /// Optional configured keybinding label for external dictation.
    fn dictation_key_label(&self) -> Option<String>;
    /// Time since app started (for startup animations)
    fn animation_elapsed(&self) -> f32;
    /// Time remaining until rate limit resets (if rate limited)
    fn rate_limit_remaining(&self) -> Option<Duration>;
    /// Whether queue mode is enabled (true = wait, false = immediate)
    fn queue_mode(&self) -> bool;
    /// Whether the next normal prompt will be routed into a new headed session.
    fn next_prompt_new_session_armed(&self) -> bool {
        false
    }
    /// Whether there is a stashed input (saved via Ctrl+S)
    fn has_stashed_input(&self) -> bool;
    /// Context info (what's loaded in context window - static + dynamic)
    fn context_info(&self) -> crate::tui::operant_app::prompt::ContextInfo;
    /// Authoritative, freshness-tagged context snapshot used by widgets.
    fn context_snapshot(&self) -> ContextSnapshot {
        let info = self.context_info();
        ContextSnapshot {
            info: (info.total_chars > 0).then_some(info),
            revision: 0,
            fresh: true,
        }
    }
    /// Context window limit in tokens (if known)
    fn context_limit(&self) -> Option<usize>;
    /// Whether floating information widgets should be drawn for this state.
    /// Implementations normally use the default; deterministic render fixtures
    /// can suppress overlays without mutating process-global widget settings.
    fn info_widget_overlays_enabled(&self) -> bool {
        true
    }
    /// Whether a newer client binary is available
    fn client_update_available(&self) -> bool;
    /// Whether a newer server binary is available (remote mode)
    fn server_update_available(&self) -> Option<bool>;
    /// Get info widget data (todos, client count, etc.)
    fn info_widget_data(&self) -> info_widget::InfoWidgetData;

    /// Whether the inline swarm gallery band should be shown above the chat.
    /// Active when `agents.swarm_spawn_mode = inline` and the swarm has members.
    fn inline_swarm_gallery_active(&self) -> bool {
        false
    }
    /// Members to render in the inline swarm gallery band.
    fn inline_swarm_members(&self) -> Vec<crate::tui::operant_app::protocol::SwarmMemberStatus> {
        Vec::new()
    }
    /// Members available for cards embedded beneath swarm spawn tool calls.
    ///
    /// This may be broader than `inline_swarm_members`: the gallery is scoped by
    /// the current ownership tree, while a transcript card can be matched safely
    /// using the exact spawned session ID recorded in the tool result.
    fn swarm_members_for_transcript(
        &self,
    ) -> Vec<crate::tui::operant_app::protocol::SwarmMemberStatus> {
        self.inline_swarm_members()
    }
    /// Selected agent index in the inline swarm panel (display order).
    fn swarm_panel_selected(&self) -> usize {
        0
    }
    /// Whether the inline swarm panel currently has keyboard focus.
    fn swarm_panel_focused(&self) -> bool {
        false
    }
    /// Whether the live swarm page currently replaces the transcript viewport.
    fn swarm_panel_full_page(&self) -> bool {
        false
    }

    // ---- Workspace ----
    /// Whether workspace mode is enabled for this client.
    fn workspace_mode_enabled(&self) -> bool {
        false
    }
    /// Animation tick used for lightweight workspace map animation.
    fn workspace_animation_tick(&self) -> u64 {
        0
    }
    /// Render streaming text using incremental markdown renderer
    /// This is more efficient than re-rendering on every frame
    fn render_streaming_markdown(&self, width: usize) -> Vec<Line<'static>>;
    /// Whether centered mode is enabled
    fn centered_mode(&self) -> bool;
    /// Authentication status for all supported providers
    fn auth_status(&self) -> crate::tui::operant_app::auth::AuthStatus;
    /// Update cost calculation based on token usage (for API-key providers)
    fn update_cost(&mut self);
    /// Diagram display mode (none/margin/pinned)
    // ---- Diagram pane ----
    fn diagram_mode(&self) -> crate::tui::operant_app::config_shim::DiagramDisplayMode;
    /// Whether the diagram pane is focused (pinned mode)
    fn diagram_focus(&self) -> bool;
    /// Selected diagram index (pinned mode, most-recent = 0)
    fn diagram_index(&self) -> usize;
    /// Diagram scroll offsets in cells (x, y) when focused
    fn diagram_scroll(&self) -> (i32, i32);
    /// Diagram pane width ratio percentage
    fn diagram_pane_ratio(&self) -> u8;
    /// Whether the user has manually resized the diagram/side pane width.
    fn diagram_pane_ratio_user_adjusted(&self) -> bool;
    /// Whether the diagram pane ratio is currently animating
    fn diagram_pane_animating(&self) -> bool;
    /// Whether the pinned diagram pane is visible
    fn diagram_pane_enabled(&self) -> bool;
    /// Position of pinned diagram pane (side or top)
    fn diagram_pane_position(&self) -> crate::tui::operant_app::config_shim::DiagramPanePosition;
    /// Diagram zoom percentage (100 = normal)
    fn diagram_zoom(&self) -> u8;
    /// Scroll offset for pinned diff pane (line index)
    // ---- Diff pane ----
    fn diff_pane_scroll(&self) -> usize;
    /// Horizontal pan offset for the shared right pane (side-panel diagrams)
    fn diff_pane_scroll_x(&self) -> i32;
    /// Zoom percentage for image widgets rendered inside the side panel.
    fn side_panel_image_zoom_percent(&self) -> u8;
    /// Image shown in the dismissible full-screen panel preview.
    fn panel_image_preview(&self) -> Option<u64> {
        None
    }
    /// Whether the pinned diff pane is focused
    fn diff_pane_focus(&self) -> bool;
    /// Session-scoped side panel state managed by the side_panel tool
    // ---- Side panel ----
    fn side_panel(&self) -> &crate::tui::operant_app::side_panel::SidePanelSnapshot;
    /// Whether the side panel replaces the transcript (fullscreen mode).
    fn side_panel_fullscreen(&self) -> bool {
        false
    }
    /// Whether to pin read images to a side pane
    fn pin_images(&self) -> bool;
    /// Whether inline transcript images render expanded. When false, each
    /// image collapses to a one-line label stub with a `show image` badge.
    /// Persisted across restarts/resume via UI preferences.
    fn inline_images_visible(&self) -> bool {
        true
    }
    /// Per-image inline expand level for `image_id` (Fit when never expanded).
    /// Cycled by clicking the per-image `expand` badge.
    fn image_expand_level(&self, _image_id: u64) -> ImageExpandLevel {
        ImageExpandLevel::Fit
    }
    /// Monotonic counter bumped whenever any image's expand level changes, so
    /// prepared-frame caches that embed anchored image geometry invalidate.
    fn expanded_images_version(&self) -> u64 {
        0
    }
    /// Remaining seconds before the pinned image side pane auto-hides.
    fn pinned_images_auto_hide_remaining_secs(&self) -> Option<u64> {
        None
    }
    /// Whether to show a native terminal scrollbar for the chat viewport
    fn chat_native_scrollbar(&self) -> bool;
    /// Whether to show a native terminal scrollbar for the side panel
    fn side_panel_native_scrollbar(&self) -> bool;
    /// Interactive inline UI state (picker-like flows shown above input)
    // ---- Inline ----
    fn inline_interactive_state(&self) -> Option<&InlineInteractiveState>;
    /// Passive inline UI state (informational views shown above input)
    fn inline_view_state(&self) -> Option<&InlineViewState> {
        None
    }
    /// General inline UI state shown above input.
    fn inline_ui_state(&self) -> Option<InlineUiStateRef<'_>> {
        self.inline_interactive_state()
            .map(InlineUiStateRef::Interactive)
            .or_else(|| self.inline_view_state().map(InlineUiStateRef::View))
    }
    /// Changelog overlay scroll offset (None = not showing)
    // ---- Overlay ----
    fn changelog_scroll(&self) -> Option<usize>;
    /// Help overlay scroll offset (None = not showing)
    fn help_scroll(&self) -> Option<usize>;
    /// Model status overlay scroll offset and markdown content (None = not showing)
    fn model_status_overlay(&self) -> Option<(usize, &str)> {
        None
    }
    /// Working directory for this session
    // ---- Misc ----
    fn working_dir(&self) -> Option<String>;
    /// Current git branch of the working directory, if in a repo.
    fn git_branch(&self) -> Option<String> {
        None
    }
    /// Monotonic clock for viewport animations
    fn now_millis(&self) -> u64;
    /// UI state for live copy badge highlighting / feedback
    // ---- Copy selection ----
    fn copy_badge_ui(&self) -> crate::tui::operant_app::app::CopyBadgeUiState;
    /// Whether modal in-app copy selection mode is active.
    fn copy_selection_mode(&self) -> bool;
    /// Current in-app copy selection range, if any.
    fn copy_selection_range(&self) -> Option<CopySelectionRange>;
    /// Persistent status for in-app copy selection mode.
    fn copy_selection_status(&self) -> Option<CopySelectionStatus>;
    /// Whether the first-run onboarding empty state is being previewed in this session.
    // ---- Onboarding ----
    fn onboarding_preview_mode(&self) -> bool {
        false
    }
    // [port-excised 2026-10-09] onboarding_welcome_active + onboarding_welcome_kind:
    //   fed only the never-vendored welcome-takeover gate (operant_ui draw);
    //   removed with the OnboardingWelcomeKind/LoginImport* stratum below.
    /// Suggestion prompts for new users (shown in initial empty state).
    /// Returns (label, prompt_text) pairs. Empty if user is experienced or not authenticated.
    fn suggestion_prompts(&self) -> Vec<(String, String)>;
    /// Cache TTL status - shows whether the prompt cache is warm/cold based on idle time
    fn cache_ttl_status(&self) -> Option<CacheTtlInfo>;
    /// Read-only reset guidance for the active OpenAI OAuth account.
    fn openai_reset_hint(&self) -> Option<String> {
        // SSH sessions may use a different login on the remote host. Local
        // cached credits cannot establish reset availability for that account.
        if self.is_processing() || is_ssh_remote() {
            return None;
        }
        let auth_method = self.info_widget_data().auth_method;
        if auth_method != info_widget::AuthMethod::OpenAIOAuth {
            return None;
        }
        // [port-decision] gate: usage::get_openai_usage_sync and
        // auth::codex::active_account_label drag the un-ported async
        // usage-fetch / codex-account subsystems (operant-base/src/usage/accessors.rs:148 +
        // operant-auth's codex module); re-activate at cutover. The hint fn itself
        // is ported (operant_ui/ui_input.rs:1863); its path was re-rooted
        // (operant_ui::input_ui -> operant_ui::ui_input).
        #[cfg(any())]
        {
            let usage = crate::tui::operant_app::usage::get_openai_usage_sync();
            let account_label = crate::tui::operant_app::auth::codex::active_account_label();
            return crate::tui::operant_ui::ui_input::openai_reset_status_hint(
                auth_method,
                &usage,
                account_label.as_deref(),
            );
        }
        #[allow(unreachable_code)]
        None
    }
    /// Whether the notification line has content to show
    fn has_notification(&self) -> bool {
        if self.openai_reset_hint().is_some() {
            return true;
        }
        if self.copy_selection_status().is_some() {
            return true;
        }
        if crate::tui::operant_ui::recent_flicker_ui_notice().is_some() {
            return true;
        }
        if self.status_notice().is_some() {
            return true;
        }
        if self.learn_hint().is_some() {
            return true;
        }
        if self.hotkey_feedback().is_some() {
            return true;
        }
        if self.has_stashed_input() {
            return true;
        }
        if !self.is_processing() {
            let info = self.info_widget_data();
            if scheduled_notification_text(info.ambient_info.as_ref()).is_some() {
                return true;
            }
            if let Some(cache_info) = self.cache_ttl_status()
                && cache_info.expiry_notification_active()
            {
                return true;
            }
        }
        false
    }
    // [port-excised 2026-10-09] the OnboardingWelcomeKind enum, LoginImportPrompt,
    // ImportSummaryPill, TelemetryChoice, and LoginImportRow stratum: they fed only
    // the never-vendored welcome-takeover gate in the ported draw (see the excision
    // note above the trait). Nothing else in the workspace referenced them.
}

pub(crate) fn is_ssh_remote() -> bool {
    ssh_remote_host().is_some()
}
