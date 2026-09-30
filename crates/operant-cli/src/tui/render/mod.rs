// render/mod.rs — All ratatui rendering logic (decomposed from render.rs).
//
// Sub-modules:
//   utils     — spinner helpers, modal checks, text truncation, shimmer effects
//   cache     — rendered line items, message/completion/streaming caches
//   tools     — tool block rendering, system annotations
//   selection — text selection highlight, row cache, context menu
//   welcome   — startup notices, banner block, welcome box
//   messages  — message pane rendering, turn items, live content
//   footer    — input pane, status row, footer bar, prompt suggestions

pub(crate) mod cache;
pub(crate) mod footer;
pub(crate) mod messages;
pub(crate) mod selection;
pub(crate) mod tools;
pub(crate) mod utils;
pub(crate) mod welcome;

pub(crate) use cache::RenderedLineItem;
pub(crate) use footer::{
    render_footer, render_input, render_prompt_suggestions, render_status_row,
    should_render_status_row,
};
pub(crate) use messages::render_messages;
pub(crate) use selection::{
    apply_selection_highlight, cache_selectable_row_text, render_context_menu,
};
pub(crate) use tools::{build_tool_names, render_system_annotation_lines};
pub(crate) use utils::{
    is_modal_open, render_error_modal, shimmer_spans, spinner_char, spinner_color, truncate_end,
    truncate_middle, truncate_text,
};
// The width seam, re-exported at `pub` so sibling modules can `pub use` it.
pub use utils::{balanced_wrap, display_width, take_width};

// render.rs â€” All ratatui rendering logic.

use crate::tui::agents_view::render_agents_menu;
use crate::tui::app::App;
use crate::tui::context_viz::render_context_viz;
use crate::tui::dialogs::{render_mcp_approval_dialog, render_permission_dialog};
use crate::tui::diff_viewer::render_diff_dialog;
use crate::tui::export_dialog::render_export_dialog;
use crate::tui::model_picker::render_model_picker;
use crate::tui::session_branching::render_session_branching;
use crate::tui::session_browser::render_session_browser;
// (iter-211: feedback_survey render import deleted — no telemetry backend)
use crate::tui::ask_user_dialog::render_ask_user_dialog;
use crate::tui::bypass_permissions_dialog::render_bypass_permissions_dialog;
use crate::tui::custom_provider_dialog::render_custom_provider_dialog;
use crate::tui::device_auth_dialog::render_device_auth_dialog;
use crate::tui::dialog_select::render_dialog_select;
use crate::tui::hooks_config_menu::render_hooks_config_menu;
use crate::tui::import_config_dialog::render_import_config_dialog;
use crate::tui::key_input_dialog::render_key_input_dialog;
use crate::tui::mcp_view::render_mcp_view;
use crate::tui::memory_file_selector::render_memory_file_selector;
use crate::tui::notifications::{NotificationKind, render_notification_banner};
use crate::tui::overlays::{
    render_global_search, render_help_overlay, render_history_search_overlay, render_rewind_flow,
};
use crate::tui::prompt_input::input_height;
use crate::tui::settings_screen::render_settings_screen;
use crate::tui::stats_dialog::render_stats_dialog;
use crate::tui::theme_colors;
use crate::tui::theme_screen::render_theme_screen;
use crate::tui::usage_overlay::{UsageMetrics, render_usage_overlay};
use crate::tui::vendor::style::theme;
use crate::tui::vendor::style::theme_mode;
use crate::tui::voice_mode_notice::render_voice_mode_notice;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block;

// Spinner frames matching the TypeScript SpinnerGlyph: platform-specific base
// characters mirrored (forward + reverse) for a smooth pulse effect.
// Windows uses '*' instead of '✳'/'✽' for better font coverage.
#[cfg(target_os = "windows")]
const SPINNER: &[char] = &[
    '\u{00b7}', '\u{2722}', '*', '\u{2736}', '\u{273b}', '\u{273d}', '\u{273d}', '\u{273b}',
    '\u{2736}', '*', '\u{2722}', '\u{00b7}',
];
#[cfg(not(target_os = "windows"))]
const SPINNER: &[char] = &[
    '\u{00b7}', '\u{2722}', '\u{2733}', '\u{2736}', '\u{273b}', '\u{273d}', '\u{273d}', '\u{273b}',
    '\u{2736}', '\u{2733}', '\u{2722}', '\u{00b7}',
];
const WELCOME_BOX_HEIGHT: u16 = 9;
const STATUS_THINKING: &str = "thinking";
const STATUS_THINKING_ELLIPSIS: &str = "thinking\u{2026}";
pub fn render_app(frame: &mut Frame, app: &App) {
    let size = frame.area();
    app.last_selectable_area.set(size);

    // Fill the entire frame with the palette's surface colour so the terminal's
    // default (blue on Windows) doesn't bleed through cells no widget covers.
    frame.render_widget(
        Block::default().style(
            Style::default()
                .bg(theme::user_bg())
                .fg(theme_colors::text()),
        ),
        size,
    );

    let prompt_focused = app.permission_request.is_none() && !app.history_search_overlay.visible;
    // Suggestions popup tracks whether the prompt accepts input, not whether
    // it is the focused widget. Text entry is allowed during streaming so the
    // user can queue the next message, so the typeahead popup must follow
    // that same affordance.
    let suggestions_visible =
        app.permission_request.is_none() && !app.history_search_overlay.visible;
    let status_visible = should_render_status_row(app);
    // One blank separator row above the status/input area when status is active,
    // matching the visual breathing room in the TS layout.
    let separator_height: u16 = if status_visible { 1 } else { 0 };
    let status_height: u16 = if status_visible {
        if app.is_streaming {
            // The spinner row is always a short single line.
            1
        } else if let Some(text) = app.status_message.as_deref() {
            // Measure how many terminal rows the message needs so that long
            // error strings (e.g. "Error: overloaded_error (529): …") wrap
            // instead of overflowing the input area.  Cap at 3 lines.
            let usable_width = size.width.max(1) as usize;
            // Measure display width (not char count) so wide chars (CJK/emoji)
            // don't undercount rows and overflow the status area.
            let text_cols = crate::tui::render::display_width(text);
            text_cols.div_ceil(usable_width).clamp(1, 3) as u16
        } else {
            1
        }
    } else {
        0
    };
    let suggestions_height = if suggestions_visible && !app.prompt_input.suggestions.is_empty() {
        app.prompt_input.suggestions.len().min(5) as u16
    } else {
        0
    };
    // The prompt body width is the terminal width minus the prompt prefix
    // ("> ") and the right-margin padding used inside `render_prompt_input`.
    // Keep this in sync with prefix_width=2 + right_pad=2 there.
    let prompt_text_width = size.width.saturating_sub(4);
    let mut prompt_height = input_height(&app.prompt_input, prompt_text_width) + 1; // +1 for model/mode status line

    // Clamp prompt_height so the prompt can never push itself (or the footer)
    // off-screen. The terminal must accommodate:
    //   - 1 row minimum for the messages area (chunks[0])
    //   - separator_height (chunks[1])
    //   - status_height (chunks[2])
    //   - prompt_height (chunks[3])
    //   - suggestions_height (chunks[4])
    //   - 2 rows for the footer (chunks[5])
    // If the natural prompt_height would overflow, clamp it to whatever
    // remains. Without this clamp, a multi-line paste or a small terminal
    // collapses chunks[3] to 0 rows and the input text vanishes — this is
    // the persistent "input not visible" bug.
    let reserved: u16 = 1u16 // minimum messages area
        .saturating_add(separator_height)
        .saturating_add(status_height)
        .saturating_add(suggestions_height)
        .saturating_add(2); // footer
    let max_prompt_height = size.height.saturating_sub(reserved).max(2);
    if prompt_height > max_prompt_height {
        prompt_height = max_prompt_height;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(separator_height),
            Constraint::Length(status_height),
            Constraint::Length(prompt_height),
            Constraint::Length(suggestions_height),
            Constraint::Length(2),
        ])
        .split(size);

    render_messages(frame, app, chunks[0]);

    // Async-delegation rows, docked to the bottom-left of the messages area.
    // Drawn immediately after the transcript so they sit above it and below the
    // overlays; `refresh` re-reads the process-wide delegation registry with a
    // real clock, so a task that finished or failed drops out on its own.
    app.background_tasks.refresh();
    let bg_rows = app.background_tasks.rows();
    let bg_area = crate::tui::background_tasks::rows_area(chunks[0], bg_rows.len());
    if !bg_rows.is_empty() {
        crate::tui::background_tasks::render_rows(
            frame,
            bg_area,
            &bg_rows,
            crate::tui::background_tasks::now_unix_secs(),
        );
    }
    // chunks[1] is the blank separator — intentionally left empty
    if status_height > 0 {
        render_status_row(frame, app, chunks[2]);
    }
    render_input(frame, app, chunks[3], prompt_focused);
    app.last_input_area.set(chunks[3]);
    if suggestions_height > 0 {
        render_prompt_suggestions(frame, app, chunks[4]);
    }
    render_footer(frame, app, chunks[5]);

    // Persistent usage panel (F8). Drawn above the base chrome but below the
    // overlay block below, so any modal correctly occludes it.
    if app.usage_overlay.visible {
        render_usage_overlay(
            frame,
            &app.usage_overlay,
            size,
            &UsageMetrics::from_app(app),
        );
    }

    // Overlays (rendered on top in Z-order)

    // Permission dialog (highest priority)
    if let Some(ref pr) = app.permission_request {
        render_permission_dialog(frame, pr, size);
    }

    // Rewind flow (takes over screen)
    if app.rewind_flow.visible {
        render_rewind_flow(frame, &app.rewind_flow, size);
    }

    // New help overlay
    if app.help_overlay.visible {
        render_help_overlay(frame, &app.help_overlay, size);
    }

    // History search overlay
    if app.history_search_overlay.visible {
        render_history_search_overlay(
            frame,
            &app.history_search_overlay,
            &app.prompt_input.history,
            size,
        );
    }
    // (iter-156: legacy history_search render deleted — field is always None)

    // Settings screen (highest-priority full-screen overlay)
    if app.settings_screen.visible {
        render_settings_screen(frame, &app.settings_screen, size);
    }

    // Theme picker overlay
    if app.theme_screen.visible {
        render_theme_screen(frame, &app.theme_screen, size);
    }

    if app.stats_dialog.visible {
        render_stats_dialog(&app.stats_dialog, size, frame.buffer_mut());
    }

    if app.mcp_view.visible {
        render_mcp_view(&app.mcp_view, size, frame.buffer_mut());
    }

    if app.agents_menu.visible {
        render_agents_menu(&app.agents_menu, size, frame.buffer_mut());
    }

    if app.diff_viewer.visible {
        let mut state = app.diff_viewer.clone();
        render_diff_dialog(&mut state, size, frame.buffer_mut());
    }

    if app.global_search.visible {
        render_global_search(&app.global_search, size, frame.buffer_mut());
    }

    // (iter-211: feedback_survey render block deleted — no telemetry backend)

    if app.memory_file_selector.visible {
        render_memory_file_selector(&app.memory_file_selector, size, frame.buffer_mut());
    }

    if app.skills_view.visible {
        crate::tui::skills_view::render_skills_view(frame, &app.skills_view, size);
    }

    if app.plugins_hub.visible {
        crate::tui::plugins_hub::render_plugins_hub(frame, &app.plugins_hub, size);
    }

    if app.journey_view.visible {
        crate::tui::journey_view::render_journey_view(frame, &app.journey_view, size);
    }

    if app.hooks_config_menu.visible {
        render_hooks_config_menu(&app.hooks_config_menu, size, frame.buffer_mut());
    }

    // Voice mode availability notice — rendered ABOVE the input box (near
    // the bottom of the screen), not at the top. Was at y: size.y (top).
    // (iter-118 — user-reported bug: notification was at top of TUI.)
    if app.voice_mode_notice.visible {
        let notice_h = app.voice_mode_notice.height();
        if size.height > notice_h + 4 {
            // Place it 2 lines above the bottom (above the footer + input).
            let notice_y = size.y + size.height.saturating_sub(notice_h + 2);
            let notice_area = Rect {
                x: size.x,
                y: notice_y,
                width: size.width,
                height: notice_h,
            };
            render_voice_mode_notice(&app.voice_mode_notice, notice_area, frame.buffer_mut());
        }
    }

    // Import-config preview dialog
    if app.import_config_dialog.visible {
        render_import_config_dialog(frame, &app.import_config_dialog, size);
    }

    // Bypass-permissions confirmation dialog (topmost — rendered last so it sits above all)
    if app.bypass_permissions_dialog.visible {
        render_bypass_permissions_dialog(frame, &app.bypass_permissions_dialog, size);
    }

    // AskUserQuestion dialog — renders above bypass-permissions so the model's
    // question is never obscured by the startup confirmation prompt.
    if app.ask_user_dialog.visible {
        render_ask_user_dialog(&app.ask_user_dialog, size, frame.buffer_mut());
    }

    // /effort picker
    if app.effort_picker.visible {
        crate::effort_picker::render_effort_picker(frame, &app.effort_picker, size);
    }

    // Import-config source picker
    if app.import_config_picker.visible {
        render_dialog_select(frame, &app.import_config_picker, size);
    }

    // Connect-a-provider dialog (/connect command)
    if app.connect_dialog.visible {
        render_dialog_select(frame, &app.connect_dialog, size);
    }

    // API key input dialog (opened from /connect for key-based providers)
    if app.key_input_dialog.visible {
        render_key_input_dialog(frame, &app.key_input_dialog, size);
    }

    // Custom provider URL + API key dialog.
    if app.custom_provider_dialog.visible {
        render_custom_provider_dialog(frame, &app.custom_provider_dialog, size);
    }

    // "Free" composite-provider setup dialog (Zen + OpenRouter).
    if app.free_mode_dialog.visible {
        crate::free_mode_dialog::render_free_mode_dialog(frame, &app.free_mode_dialog, size);
    }

    // Device code / browser auth dialog (GitHub Copilot, Anthropic OAuth)
    if app.device_auth_dialog.visible {
        render_device_auth_dialog(frame, &app.device_auth_dialog, size);
    }

    // Ctrl+K command palette
    if app.command_palette.visible {
        render_dialog_select(frame, &app.command_palette, size);
    }

    // Model picker overlay
    if app.model_picker.visible {
        render_model_picker(&app.model_picker, size, frame.buffer_mut());
    }

    // Session browser overlay
    if app.session_browser.visible {
        render_session_browser(&app.session_browser, size, frame.buffer_mut());
    }

    // Session branching overlay
    if app.session_branching.visible {
        render_session_branching(&app.session_branching, size, frame.buffer_mut());
    }

    // Export format picker dialog
    if app.export_dialog.visible {
        render_export_dialog(frame, &app.export_dialog, size);
    }

    // Context visualization overlay
    if app.context_viz.visible {
        render_context_viz(
            frame,
            &app.context_viz,
            size,
            app.context_used_tokens,
            app.context_window_size,
            app.rate_limit_5h_pct,
            app.rate_limit_7day_pct,
            app.cost_usd,
        );
    }

    // MCP approval dialog
    if app.mcp_approval.visible {
        render_mcp_approval_dialog(&app.mcp_approval, size, frame.buffer_mut());
    }

    // Always show error modals on top of everything (highest priority)
    if let Some(notif) = app.notifications.current()
        && notif.kind == NotificationKind::Error
    {
        let is_welcome_screen = app.messages.is_empty()
            && app.streaming_text.is_empty()
            && app.streaming_thinking.is_empty()
            && app.tool_use_blocks.is_empty();
        render_error_modal(
            frame,
            size,
            notif,
            app.error_modal_scroll_offset,
            app.footer_right_column_area.get(),
            is_welcome_screen,
        );
        // Don't render other overlays/notifications when error modal is showing.
        // The substitution pass still has to run — see the tail of this fn.
        theme_mode::adapt_buffer_for_display(frame.buffer_mut());
        return;
    }
    let modal_active = is_modal_open(app);

    // Render non-error notifications as toast banners (unless another modal is open)
    if !modal_active && app.notifications.current().is_some() {
        render_notification_banner(frame, &app.notifications, size);
    }

    // ---- Text selection highlight (topmost post-pass) ---------------------
    apply_selection_highlight(frame, app);
    cache_selectable_row_text(frame, app);
    render_context_menu(frame, app);

    // ---- Debug overlay (F12) — topmost, always last ---------------------
    crate::tui::debug::overlay::render_debug_overlay(frame, &app.debug_hub, size);

    // ---- OSC 8 hyperlink overlay (post-paint pass) ---------------------
    // Scan the rendered buffer for URLs and emit OSC 8 escape sequences so
    // terminals that support the protocol (Windows Terminal, iTerm2, WezTerm,
    // Kitty, etc.) make them Ctrl/Cmd-clickable. This runs after all other
    // rendering so it sees the final buffer state.
    let hits = crate::tui::osc8::scan_buffer_for_urls(frame.buffer_mut());
    if let Err(e) = crate::tui::osc8::emit_hits(&hits) {
        tracing::debug!("OSC8 hyperlink emission failed: {e}");
    }

    // ---- Pinned graphics (per-frame pass) ---------------------------------
    // Terminal graphics protocols (Kitty/Sixel/iTerm2) paint outside ratatui's
    // cell grid, so nothing in the buffer represents a painted image. The old
    // pass drained each producer — `pending_inline_images` was emptied and a
    // mermaid raster consumed — wrote the sequence once, and had nothing left to
    // restore from, so any redraw touching those cells destroyed the image
    // permanently. Both producers now go into a persistent registry that
    // re-emits every graphic EVERY frame at a fixed rect; the queue is still
    // drained once, because the attachment is consumed, but the graphic it
    // produced is not. See `tui::pinned_images`.
    crate::tui::pinned_images::pin_pasted(&app.pinned_images, &app.pending_inline_images);

    // A ```mermaid block rasterises on a worker thread, so the picture lands
    // here some frames after the transcript first showed the diagram. When a
    // raster lands the memoized transcript lines are stale — a "rendering…"
    // placeholder just became the real thing — so drop them and let the next
    // frame rebuild.
    let mut landed = crate::tui::mermaid::drain_ready_rasters();

    // A ```latex / ```math / ```tex block is the same shape of problem: the
    // formula rasterises on a worker thread, so its placeholder goes stale on
    // exactly the same schedule. Both ladders report "a PNG just landed, and the
    // transcript lines are stale", so there is still exactly ONE registry call —
    // `pin_rasters` consumes a list of PNG paths, not a producer.
    let formulas = crate::tui::latex::drain_ready_rasters();
    if landed.resolved || formulas.resolved {
        crate::tui::render::cache::MESSAGE_LINES_CACHE.with(|cache| cache.borrow_mut().take());
    }
    landed.pngs.extend(formulas.pngs);
    crate::tui::pinned_images::pin_rasters(&app.pinned_images, &landed);

    // Re-emit every pinned graphic, and blank the cells it owns so the flush
    // ratatui runs after this closure has nothing to rewrite underneath it.
    crate::tui::pinned_images::prepare_strip(frame.buffer_mut(), &app.pinned_images, chunks[0]);

    // ---- Per-frame colour substitution (the single theming choke point) ----
    //
    // Every migrated call site emits a `style::theme::*` role DEFAULT; this pass
    // rewrites each such cell onto the configured role colour, so one `/theme`
    // action repaints the whole frame without any widget knowing about it.
    // It must stay the LAST colour transform in the function — the error-modal
    // branch above has its own copy of this call because it returns early.
    theme_mode::adapt_buffer_for_display(frame.buffer_mut());
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    use crate::tui::vendor::style::palette::Role;

    fn make_app() -> App {
        let cost_tracker = std::sync::Arc::new(crate::tui::adapter_types::cost::CostTracker::new());
        App::new(
            operant_core::config::AppConfig::default(),
            crate::tui::adapter_types::Settings::default(),
            cost_tracker,
            crate::commands::CommandRegistry::new(),
        )
    }

    /// Hold the palette lock for the whole body.
    ///
    /// Holding it per-theme is not enough here, because `App::new` itself calls
    /// `set_active_theme_enum`. Building the `App` outside the lock lets a
    /// concurrent test's `App::new` reset the process-global palette between
    /// this test's `set_active_theme` and its `paint`, and the frame then comes
    /// back in the default theme — a real flake, not a theoretical one.
    fn with_palette_lock<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::tui::vendor::style::color::pin_truecolor_for_tests();
        f()
    }

    /// Paint one frame with `name` active. Caller must hold the palette lock.
    fn frame_under(name: &str, app: &App) -> Buffer {
        theme_colors::set_active_theme(name);
        painted_frame(app)
    }

    /// One full `render_app` into a 120x40 test frame, post-substitution.
    fn painted_frame(app: &App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test backend");
        terminal.draw(|f| render_app(f, app)).expect("draw");
        terminal.backend().buffer().clone()
    }

    /// The distinct foreground colours present in a frame, as `Color`.
    fn foregrounds(buf: &Buffer) -> Vec<Color> {
        let mut seen: Vec<Color> = buf.content.iter().map(|c| c.fg).collect();
        seen.sort_by_key(|c| format!("{c:?}"));
        seen.dedup();
        seen
    }

    /// The end-to-end proof that `/theme` repaints the frame: two themes must
    /// produce different foregrounds in the finished buffer.
    ///
    /// This is the only test that exercises the whole chain — role call site →
    /// buffer → `adapt_buffer_for_display` — and it is the regression net for
    /// the choke point. If the pass is removed, or a call site reverts to
    /// `theme_colors`, the two frames converge and this fails.
    #[test]
    fn switching_theme_repaints_the_frame() {
        with_palette_lock(|| {
            let app = make_app();
            let dark = frame_under("dark", &app);
            let light = frame_under("light", &app);
            theme_colors::set_active_theme("default");
            assert_ne!(
                foregrounds(&dark),
                foregrounds(&light),
                "the frame must not be theme-invariant"
            );
        });
    }

    /// The base fill is the largest surface in the frame and was a hardcoded
    /// `Color::Black` until the role migration. It must follow the theme.
    #[test]
    fn base_fill_follows_the_theme() {
        with_palette_lock(|| {
            let app = make_app();
            let dark = frame_under("dark", &app);
            let light = frame_under("light", &app);
            theme_colors::set_active_theme("default");
            // Row 0 col 0 is a cell no widget claims, so it still holds the fill.
            assert_ne!(
                dark[(0, 0)].bg,
                light[(0, 0)].bg,
                "the base fill must repaint"
            );
            assert_ne!(
                dark[(0, 0)].bg,
                Color::Black,
                "the fill is no longer frozen black"
            );
        });
    }

    /// The accent bar / prompt rule is the surface `ACCENT_BUILD` used to freeze
    /// to the default theme's amber, which is why `/theme` could not repaint it
    /// on seven of eight themes. Assert the resolved accent really lands in the
    /// finished frame, not merely that the two frames differ somehow.
    #[test]
    fn accent_role_reaches_the_frame() {
        let accent_in_frame = |name: &str, app: &App| {
            let buf = frame_under(name, app);
            // The resolved role colour, not `theme_colors::accent()`: the
            // `default` and `light` themes hold a *named* accent (Cyan / Blue)
            // and a role slot can only carry RGB, so the frame gets the xterm
            // RGB of that name.
            let accent = crate::tui::vendor::style::palette::palette().color(Role::Accent);
            assert!(
                buf.content.iter().any(|c| c.fg == accent),
                "{name}: no cell resolved to the theme accent {accent:?}"
            );
            accent
        };
        with_palette_lock(|| {
            let app = make_app();
            let dark = accent_in_frame("dark", &app);
            let light = accent_in_frame("light", &app);
            theme_colors::set_active_theme("default");
            assert_ne!(dark, light);
        });
    }
}
