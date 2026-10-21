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

// The ported jcode chrome (operant_ui::draw) replaced the dispatch registry, the
// transcript/footer/welcome renderers at iter-648; what stays are the helpers
// operant-only surfaces still call (selection's copy-badge pool, tool group
// lines, text measurement) plus the async-raster hash cache.

pub(crate) mod operant_overlays;
pub(crate) mod selection;
pub(crate) mod tools;
pub(crate) mod utils;

pub(crate) use selection::{
    apply_selection_highlight, cache_selectable_row_text, render_context_menu,
};
// The width seam, re-exported at `pub` so sibling modules can `pub use` it.
pub use utils::{balanced_wrap, display_width, take_width};

// render.rs â€” All ratatui rendering logic.

// `render_app` itself needs almost nothing: the layout arithmetic, the dispatch
// table, and the post-draw passes. The surface render functions moved into
// `dispatch.rs`, which imports each one from the module its surface lives in.
use crate::tui::app::App;
use ratatui::Frame;

// Spinner frames matching the TypeScript SpinnerGlyph: platform-specific base
// characters mirrored (forward + reverse) for a smooth pulse effect.
// Windows uses '*' instead of '✳'/'✽' for better font coverage.
#[cfg(target_os = "windows")]
const SPINNER: &[char] = &[
    '\u{00b7}', '\u{2722}', '*', '\u{2736}', '\u{273b}', '\u{273d}', '\u{273d}', '\u{273b}',
    '\u{2736}', '*', '\u{2722}', '\u{00b7}',
];
pub fn render_app(frame: &mut Frame, app: &App) {
    let size = frame.area();

    // The ported jcode chrome (operant_ui::draw, upstream ui.rs:2659-3624) owns
    // the whole frame: the full-frame clear, every band, and the per-frame
    // colour substitution at its tail. operant's dispatch rows are gone; what
    // stays below are the three passes with no jcode counterpart.
    crate::tui::operant_ui::draw(frame, app);

    // ---- Publish the ported layout to operant's hit-test/scroll state ----
    // Three App cells were written by the deleted dispatch table. Each still
    // has production readers, and a zeroed cell is not a neutral value: the
    // mouse gates input clicks on a non-empty input rect, and the prepend
    // correction reconciles against the painted scroll row.
    //
    // last_selectable_area MUST publish after operant_ui::draw, not before:
    // message_area() reads the thread-local the draw just updated, so a
    // pre-draw copy is one frame stale — zero-rect on the first frame, which
    // silently drops the first mouse click after startup (a simulated
    // scenario's opening Down lands exactly there).
    app.last_selectable_area
        .set(crate::tui::operant_ui::message_area());
    app.last_input_area
        .set(crate::tui::operant_ui::input_area());
    app.last_render_scroll_offset
        .set(crate::tui::operant_ui::last_resolved_chat_scroll().min(u16::MAX as usize) as u16);
    // The reader's content address this frame resolved to — the one source
    // `note_resize` captures from. Published here (not read lazily) so the
    // App-side one-shot and the chrome anchor stay in lockstep with the frame
    // that actually painted (2026-10-09 P4-1: the seam was half-wired — the
    // render read a per-frame global as the pending anchor and nothing ever
    // fed the App's capture, so user scroll was overridden to the tail every
    // frame and resize-while-parked captured nothing).
    app.last_render_content_pos
        .set(crate::tui::operant_ui::resolved_reader_anchor());
    // Transcript rect for the legacy message-context consumers (right-click
    // context-menu bounds, cursor-open menu, menu clamping). Same publish
    // discipline as the cells above: the iter-648 cutover deleted the
    // dispatch-row writer, leaving a zero rect that silently killed every
    // consumer — dead right-click menus, invisible keyboard menus (2026-10-09
    // visual audit). `message_area()` is the chat band; clamping to it matches
    // the legacy pane-rect semantics.
    app.last_msg_area
        .set(crate::tui::operant_ui::message_area());

    // ---- Background task rows (operant-only) ---------------------------
    // Docked to the bottom-left of the messages band, so the rows sit above
    // the transcript and below the overlays. The pin test in
    // background_tasks.rs exists because this wiring is invisible when it
    // breaks: the module still compiles, still passes its tests, and quietly
    // shows the user nothing.
    app.background_tasks.refresh();
    let bg_rows = app.background_tasks.rows();
    if !bg_rows.is_empty() {
        let bg_area = crate::tui::background_tasks::rows_area(
            crate::tui::operant_ui::message_area(),
            bg_rows.len(),
        );
        crate::tui::background_tasks::render_rows(
            frame,
            bg_area,
            &bg_rows,
            crate::tui::background_tasks::now_unix_secs(),
        );
    }

    // ---- operant-only overlays ------------------------------------------
    // Every dialog, menu and screen the ported jcode chrome does not own.
    // They lost their invocation at iter-649 (the dispatch table was deleted
    // with the chrome rows) and the corpus proved it: 60 surfaces stopped
    // rendering. Painted in the same order as before, on top of the chrome.
    if operant_overlays::draw_operant_overlays(frame, app) {
        // The error modal is the frame's only early exit: every buffer-level
        // post-pass below (rasters, pinned graphics, OSC 8) paints over it
        // otherwise, which is what the old dispatch ladder's early return
        // suppressed. The colour substitution already ran inside the draw.
        return;
    }

    // Pinned graphics re-emit every registered graphic EVERY frame at a fixed
    // rect (see tui::pinned_images); the queue is still drained once because
    // the attachment is consumed. [port-decision] the claim area is the whole
    // frame: the messages-area constraint (chunks[0]) belonged to the deleted
    // dispatch layout, and the ported chrome does not expose a transcript rect.
    crate::tui::pinned_images::pin_pasted(&app.pinned_images, &app.pending_inline_images);
    crate::tui::pinned_images::prepare_strip(frame.buffer_mut(), &app.pinned_images, size);

    // ---- OSC 8 hyperlink overlay (post-paint pass) ---------------------
    // Runs after every other render so it sees the final buffer state.
    let hits = crate::tui::osc8::scan_buffer_for_urls(frame.buffer_mut());
    if let Err(e) = crate::tui::osc8::emit_hits(&hits) {
        tracing::debug!("OSC8 hyperlink emission failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    /// The ported chrome owns the frame layout, but operant's hit-test and
    /// scroll state still lives in App cells that production code reads. They
    /// are republished here from the ported layout. Behavioural, not a source
    /// pin: a text assertion would stay green on an empty band, and empty is
    /// exactly the failure (mouse routes nothing, the prepend correction
    /// reconciles against 0). Drawn at two sizes so the packed and unpacked
    /// layout paths are both covered.
    #[test]
    fn the_ported_layout_republishes_the_hit_test_cells() {
        // `make_app` calls `App::new`, which writes the process-global
        // palette. The two theme-asserting tests below hold the palette lock
        // across their whole body (and so across their `make_app`); this one
        // did not, so its `App::new` reset the palette CONCURRENTLY with
        // theirs — between their `set_active_theme` and the read of the role
        // palette their assertion compares against. That is the whole of the
        // `accent_role_reaches_the_frame` flake: the frame was painted under
        // one theme and the assertion compared it against another.
        // (iter-1136, RT-149)
        with_palette_lock(|| {
            for (width, height) in [(120u16, 40u16), (80, 24)] {
                let app = make_app();
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("test backend");
                terminal.draw(|f| render_app(f, &app)).expect("draw");

                let input = app.last_input_area.get();
                assert!(
                    input.width > 0 && input.height > 0,
                    "{width}x{height}: the input band was published empty — every click \
                     would route as 'not the input' and cursor up/down would compute a \
                     zero-width cell"
                );
                assert!(
                    input.y + input.height <= height,
                    "{width}x{height}: the input band runs past the frame ({input:?})"
                );

                let painted = crate::tui::operant_ui::last_resolved_chat_scroll()
                    .min(u16::MAX as usize) as u16;
                assert_eq!(
                    app.last_render_scroll_offset.get(),
                    painted,
                    "{width}x{height}: the published scroll row does not match the \
                     ported chrome's resolved scroll — the prepend correction would \
                     drift the reader"
                );
            }
        });
    }

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
