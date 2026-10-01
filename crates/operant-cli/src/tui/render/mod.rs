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
//   dispatch  — the paint-order registry: which surface paints, and in what order

pub(crate) mod cache;
pub(crate) mod dispatch;
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
    clear_area, is_modal_open, render_error_modal, shimmer_spans, spinner_char, spinner_color,
    truncate_end, truncate_middle, truncate_text,
};
// The width seam, re-exported at `pub` so sibling modules can `pub use` it.
pub use utils::{balanced_wrap, display_width, take_width};

// render.rs â€” All ratatui rendering logic.

// `render_app` itself needs almost nothing: the layout arithmetic, the dispatch
// table, and the post-draw passes. The surface render functions moved into
// `dispatch.rs`, which imports each one from the module its surface lives in.
use crate::tui::app::App;
use crate::tui::prompt_input::input_height;
use crate::tui::vendor::style::theme_mode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

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

    // Clear the frame before painting. A cell can only survive from the previous
    // frame if the terminal's real state has diverged from the buffer ratatui
    // diffs against; a per-cell reset removes buffer divergence as a candidate.
    // See `utils::clear_area` for why this is defence-in-depth today rather than
    // a fix for an observed defect. The out-of-grid writers (pinned graphics, the
    // OSC 8 overlay) are the remaining divergence source and are handled by
    // their own passes, not by this one.
    clear_area(frame, size);

    // The whole-frame fill is dispatch row 0 (`base_fill`): it must paint before
    // anything else so the terminal's default (blue on Windows) does not bleed
    // through cells no widget covers.

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

    // ---- The dispatch table -------------------------------------------------
    //
    // Every visible surface paints, in this order, over the top of the one
    // before it. `dispatch::DISPATCH` is that order as data: a row's `visible`
    // guard decides whether it paints this frame and its `paint` body delegates
    // to the surface's own module. There is no first-match-wins here and there
    // must not be one — `permission_dialog` at row 8 is commented "highest
    // priority" yet `bypass_permissions_dialog` at row 26 paints over it, and
    // `ask_user_dialog` at row 27 paints over that.
    //
    // chunks[1] is the blank separator between the transcript and the status
    // row — it is layout, not a surface, and no row paints into it.
    let mut ctx = dispatch::FrameCtx {
        size,
        // Cloned so the post-draw pass can still read `chunks[0]`; `Rc::clone`
        // is a refcount bump, and `Layout::split` already handed us a shared
        // slice.
        chunks: chunks.clone(),
        prompt_focused,
        status_height,
        suggestions_height,
        bg_rows: Vec::new(),
        stop: false,
    };
    for entry in dispatch::DISPATCH {
        if !(entry.visible)(app, &mut ctx) {
            continue;
        }
        (entry.paint)(frame, app, &mut ctx);
        // The error-modal row set this: it owns the rest of the frame.
        if ctx.stop {
            break;
        }
    }
    if ctx.stop {
        // The error modal is the frame's one early exit. The substitution pass
        // still has to run — see the tail of this fn — and nothing else does.
        theme_mode::adapt_buffer_for_display(frame.buffer_mut());
        return;
    }

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

    use crate::tui::theme_colors;
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
