// render/dispatch.rs — The owned paint-order registry for one `render_app` frame.
//
// This table is the ONLY place that decides which surface paints and in what
// order. `render_app` computes the layout, iterates [`DISPATCH`] top to bottom,
// then runs the post-draw passes. Nothing here branches on which surface is
// "the" one to show.
//
// ## It is a paint order, not a lookup
//
// `render_app` is not first-match-wins. It has 41 `if` sites in the dispatch
// region and one `return`, and that return is an *early exit* for the error
// modal, not a dispatch terminator: every visible surface paints, in source
// order, over the top of the one before it. ratatui states the consequence —
// "If multiple widgets cover the same cells, later renders win for those cells"
// — and the ladder leans on it. `permission_dialog` is commented "highest
// priority" at row 8 yet `bypass_permissions_dialog` at row 26 paints over it,
// and `ask_user_dialog` at row 27 paints over that. Collapsing this into
// "first visible wins" would silently delete the overlap and change every
// frame.
//
// ## One table, two sinks, one body
//
// A row paints through `&mut Frame` (35 of them) or through `&mut Buffer` (13,
// reached as `frame.buffer_mut()`). Both are the same buffer: ratatui 0.30's
// `Frame::render_widget` is literally `widget.render(area, self.buffer)`, and
// `frame.buffer_mut()` hands out that very buffer. So a row is declared once,
// against `&mut Frame`, and a `Buffer` row reaches the buffer at the top of its
// body — no row is forked per sink and none may grow one. Same discipline as
// `modal_frame` / `modal_frame_buf` in `overlays/layout.rs`, with the roles of
// primitive and adapter swapped because the surface render functions in this
// module take `&mut Frame`, and that is the sink `render_app` is handed.
//
// ## A row points, it does not draw
//
// Every row body is a delegation to the surface's own module: `stats_dialog` is
// drawn by `stats_dialog.rs`, `usage_overlay` by `usage_overlay.rs`, and so on.
// The only exception is the base fill, which has no module of its own. That is
// the entire point of the split — the parallel rebuild phase gives each surface
// to one agent, and every one of them works inside the module this table already
// points at, so nobody opens this file.
//
// Guards are the real guards, already in their final form, so a rebuild that
// only changes how a surface *looks* cannot reach the table at all.

use ratatui::Frame;
use std::rc::Rc;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::tui::app::App;
use crate::tui::background_tasks::{self, BackgroundTask};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

// The surface render functions, imported from their own modules. The table
// delegates to them and to nothing else; each stays in the file its surface
// already lives in, which is what keeps a rebuild off this module.
use crate::tui::agents_view::render_agents_menu;
use crate::tui::ask_user_dialog::render_ask_user_dialog;
use crate::tui::bypass_permissions_dialog::render_bypass_permissions_dialog;
use crate::tui::context_viz::render_context_viz;
use crate::tui::custom_provider_dialog::render_custom_provider_dialog;
use crate::tui::device_auth_dialog::render_device_auth_dialog;
use crate::tui::dialog_select::render_dialog_select;
use crate::tui::dialogs::{render_mcp_approval_dialog, render_permission_dialog};
use crate::tui::diff_viewer::render_diff_dialog;
use crate::tui::export_dialog::render_export_dialog;
use crate::tui::hooks_config_menu::render_hooks_config_menu;
use crate::tui::import_config_dialog::render_import_config_dialog;
use crate::tui::key_input_dialog::render_key_input_dialog;
use crate::tui::mcp_view::render_mcp_view;
use crate::tui::memory_file_selector::render_memory_file_selector;
use crate::tui::model_picker::render_model_picker;
use crate::tui::notifications::{NotificationKind, render_notification_banner};
use crate::tui::overlays::{
    render_global_search, render_help_overlay, render_history_search_overlay, render_rewind_flow,
};
use crate::tui::session_branching::render_session_branching;
use crate::tui::session_browser::render_session_browser;
use crate::tui::settings_screen::render_settings_screen;
use crate::tui::stats_dialog::render_stats_dialog;
use crate::tui::theme_screen::render_theme_screen;
use crate::tui::usage_overlay::{UsageMetrics, render_usage_overlay};
use crate::tui::voice_mode_notice::render_voice_mode_notice;

// The base-chrome renderers, re-exported by the parent `render` module.
use super::{
    apply_selection_highlight, cache_selectable_row_text, is_modal_open, render_context_menu,
    render_error_modal, render_footer, render_input, render_messages, render_prompt_suggestions,
    render_status_row,
};

/// Which buffer a row reaches its surface through.
///
/// Data, not behaviour: the row body is identical either way (see the module
/// doc). The column exists so the frame's sink split is machine-checkable, and
/// so a rebuild that has to change a surface's sink says so out loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sink {
    /// `Frame::render_widget`, or a render function taking `&mut Frame`.
    Frame,
    /// `frame.buffer_mut()`, or a render function taking `&mut Buffer`.
    Buffer,
}

impl Sink {
    /// The column as the guard test spells it, so the assertion reads like the
    /// table does.
    // Read by the paint-order guard test only.
    #[allow(dead_code)]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Sink::Frame => "frame",
            Sink::Buffer => "buffer",
        }
    }
}

/// Per-frame values the rows need, computed once by `render_app`.
///
/// Deliberately narrow: the six-chunk split, the two heights that decide whether
/// a chrome row has room, and the two channels rows use to talk to their
/// neighbours. The layout *arithmetic* stays in `render_app` — it is
/// hand-synced with `prompt_input/render.rs` and does not belong behind a
/// dispatch row.
pub(crate) struct FrameCtx {
    /// `frame.area()`, the whole viewport. The overlays size against this.
    pub size: Rect,
    /// The six-chunk vertical split: messages, separator, status, input,
    /// suggestions, footer. `Layout::split` hands back an `Rc<[Rect]>`, and the
    /// loop moves it in rather than copying six rects per frame.
    pub chunks: Rc<[Rect]>,
    /// Whether the prompt accepts focus. Read by the input row only.
    pub prompt_focused: bool,
    /// `> 0` exactly when the status row has a row to paint in.
    pub status_height: u16,
    /// `> 0` exactly when the typeahead popup has rows to paint.
    pub suggestions_height: u16,
    /// Filled by the background-task row's guard, read by its body. See that
    /// guard for why the two are split.
    pub bg_rows: Vec<BackgroundTask>,
    /// Set by the error-modal row. The one early exit in the frame: when it is
    /// set, `render_app` stops the ladder, runs the colour-substitution pass
    /// and returns, skipping the banner, the selection passes, the debug
    /// overlay and every post-draw pass.
    pub stop: bool,
}

/// Decides whether a row paints this frame.
///
/// `&mut FrameCtx` rather than `&FrameCtx` for exactly one reason: the
/// background-task row must read the delegation registry *before* it can know
/// whether it has anything to show, and the read has to happen at the point in
/// paint order where it happens today — after the transcript, before the status
/// row. That guard is therefore the one impure guard in the table, and it
/// stashes what it read for the body. Every other guard is a pure read.
type VisibleFn = fn(&App, &mut FrameCtx) -> bool;

/// Paints the surface. One body per row, whichever sink it uses.
type PaintFn = fn(&mut Frame, &App, &mut FrameCtx);

/// One surface, in the order it paints.
// `name` and `sink` are the row's identity and its sink column. The paint path
// reads neither — they are carried for the paint-order guard test and for
// debugging output — so the non-test build sees them as unread.
#[allow(dead_code)]
pub(crate) struct Entry {
    /// The surface's identity. Carried for the guard test and for debugging
    /// output; the paint path never reads it.
    pub name: &'static str,
    /// The real visibility predicate, already in final form.
    pub visible: VisibleFn,
    /// The delegation to the surface's own module.
    pub paint: PaintFn,
    /// Which buffer the delegation reaches.
    pub sink: Sink,
}

/// The guards that are all the same shape: a `visible` flag on a field of
/// `App`. Both halves are named rather than pasted — `macro_rules!` cannot
/// concatenate identifiers on stable — so the invocation reads as a table:
/// `guard => the flag it reads`. Renaming a flag breaks its row instead of
/// silently inverting the guard.
macro_rules! overlay_visible_guards {
    ($($guard:ident => $field:ident),+ $(,)?) => {
        $(
            fn $guard(app: &App, _ctx: &mut FrameCtx) -> bool {
                app.$field.visible
            }
        )+
    };
}

overlay_visible_guards!(
    is_visible_usage_overlay => usage_overlay,
    is_visible_rewind_flow => rewind_flow,
    is_visible_help_overlay => help_overlay,
    is_visible_history_search_overlay => history_search_overlay,
    is_visible_settings_screen => settings_screen,
    is_visible_theme_screen => theme_screen,
    is_visible_stats_dialog => stats_dialog,
    is_visible_mcp_view => mcp_view,
    is_visible_agents_menu => agents_menu,
    is_visible_diff_viewer => diff_viewer,
    is_visible_global_search => global_search,
    is_visible_memory_file_selector => memory_file_selector,
    is_visible_skills_view => skills_view,
    is_visible_plugins_hub => plugins_hub,
    is_visible_journey_view => journey_view,
    is_visible_hooks_config_menu => hooks_config_menu,
    is_visible_voice_mode_notice => voice_mode_notice,
    is_visible_import_config_dialog => import_config_dialog,
    is_visible_bypass_permissions_dialog => bypass_permissions_dialog,
    is_visible_ask_user_dialog => ask_user_dialog,
    is_visible_effort_picker => effort_picker,
    is_visible_import_config_picker => import_config_picker,
    is_visible_connect_dialog => connect_dialog,
    is_visible_key_input_dialog => key_input_dialog,
    is_visible_custom_provider_dialog => custom_provider_dialog,
    is_visible_free_mode_dialog => free_mode_dialog,
    is_visible_device_auth_dialog => device_auth_dialog,
    is_visible_command_palette => command_palette,
    is_visible_model_picker => model_picker,
    is_visible_session_browser => session_browser,
    is_visible_session_branching => session_branching,
    is_visible_export_dialog => export_dialog,
    is_visible_context_viz => context_viz,
    is_visible_mcp_approval => mcp_approval,
);

/// The frame, as data: every surface, in paint order.
///
/// The order IS the behaviour. Reordering two rows changes which of two
/// overlapping surfaces wins the cells they share; deleting a row deletes
/// pixels. Both are pinned by
/// `dispatch_rows_are_the_registered_surfaces_in_paint_order`.
pub(crate) const DISPATCH: &[Entry] = &[
    // --- base chrome --------------------------------------------------------
    Entry {
        name: "base_fill",
        visible: always_visible,
        paint: paint_base_fill,
        sink: Sink::Frame,
    },
    Entry {
        name: "messages",
        visible: always_visible,
        paint: paint_messages,
        sink: Sink::Frame,
    },
    Entry {
        name: "background_task_rows",
        visible: visible_background_task_rows,
        paint: paint_background_task_rows,
        sink: Sink::Frame,
    },
    Entry {
        name: "status_row",
        visible: visible_status_row,
        paint: paint_status_row,
        sink: Sink::Frame,
    },
    Entry {
        name: "input",
        visible: always_visible,
        paint: paint_input,
        sink: Sink::Frame,
    },
    Entry {
        name: "prompt_suggestions",
        visible: visible_prompt_suggestions,
        paint: paint_prompt_suggestions,
        sink: Sink::Frame,
    },
    Entry {
        name: "footer",
        visible: always_visible,
        paint: paint_footer,
        sink: Sink::Frame,
    },
    // --- the overlay block, in the order the ladder painted it --------------
    // `usage_overlay` sits below the modals so that any modal occludes it.
    Entry {
        name: "usage_overlay",
        visible: is_visible_usage_overlay,
        paint: paint_usage_overlay,
        sink: Sink::Frame,
    },
    Entry {
        name: "permission_dialog",
        visible: visible_permission_dialog,
        paint: paint_permission_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "rewind_flow",
        visible: is_visible_rewind_flow,
        paint: paint_rewind_flow,
        sink: Sink::Frame,
    },
    Entry {
        name: "help_overlay",
        visible: is_visible_help_overlay,
        paint: paint_help_overlay,
        sink: Sink::Frame,
    },
    Entry {
        name: "history_search_overlay",
        visible: is_visible_history_search_overlay,
        paint: paint_history_search_overlay,
        sink: Sink::Frame,
    },
    Entry {
        name: "settings_screen",
        visible: is_visible_settings_screen,
        paint: paint_settings_screen,
        sink: Sink::Frame,
    },
    Entry {
        name: "theme_screen",
        visible: is_visible_theme_screen,
        paint: paint_theme_screen,
        sink: Sink::Frame,
    },
    Entry {
        name: "stats_dialog",
        visible: is_visible_stats_dialog,
        paint: paint_stats_dialog,
        sink: Sink::Buffer,
    },
    Entry {
        name: "mcp_view",
        visible: is_visible_mcp_view,
        paint: paint_mcp_view,
        sink: Sink::Buffer,
    },
    Entry {
        name: "agents_menu",
        visible: is_visible_agents_menu,
        paint: paint_agents_menu,
        sink: Sink::Buffer,
    },
    Entry {
        name: "diff_viewer",
        visible: is_visible_diff_viewer,
        paint: paint_diff_viewer,
        sink: Sink::Buffer,
    },
    Entry {
        name: "global_search",
        visible: is_visible_global_search,
        paint: paint_global_search,
        sink: Sink::Buffer,
    },
    Entry {
        name: "memory_file_selector",
        visible: is_visible_memory_file_selector,
        paint: paint_memory_file_selector,
        sink: Sink::Buffer,
    },
    Entry {
        name: "skills_view",
        visible: is_visible_skills_view,
        paint: paint_skills_view,
        sink: Sink::Frame,
    },
    Entry {
        name: "plugins_hub",
        visible: is_visible_plugins_hub,
        paint: paint_plugins_hub,
        sink: Sink::Frame,
    },
    Entry {
        name: "journey_view",
        visible: is_visible_journey_view,
        paint: paint_journey_view,
        sink: Sink::Frame,
    },
    Entry {
        name: "hooks_config_menu",
        visible: is_visible_hooks_config_menu,
        paint: paint_hooks_config_menu,
        sink: Sink::Buffer,
    },
    Entry {
        name: "voice_mode_notice",
        visible: is_visible_voice_mode_notice,
        paint: paint_voice_mode_notice,
        sink: Sink::Buffer,
    },
    Entry {
        name: "import_config_dialog",
        visible: is_visible_import_config_dialog,
        paint: paint_import_config_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "bypass_permissions_dialog",
        visible: is_visible_bypass_permissions_dialog,
        paint: paint_bypass_permissions_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "ask_user_dialog",
        visible: is_visible_ask_user_dialog,
        paint: paint_ask_user_dialog,
        sink: Sink::Buffer,
    },
    Entry {
        name: "effort_picker",
        visible: is_visible_effort_picker,
        paint: paint_effort_picker,
        sink: Sink::Frame,
    },
    Entry {
        name: "import_config_picker",
        visible: is_visible_import_config_picker,
        paint: paint_import_config_picker,
        sink: Sink::Frame,
    },
    Entry {
        name: "connect_dialog",
        visible: is_visible_connect_dialog,
        paint: paint_connect_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "key_input_dialog",
        visible: is_visible_key_input_dialog,
        paint: paint_key_input_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "custom_provider_dialog",
        visible: is_visible_custom_provider_dialog,
        paint: paint_custom_provider_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "free_mode_dialog",
        visible: is_visible_free_mode_dialog,
        paint: paint_free_mode_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "device_auth_dialog",
        visible: is_visible_device_auth_dialog,
        paint: paint_device_auth_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "command_palette",
        visible: is_visible_command_palette,
        paint: paint_command_palette,
        sink: Sink::Frame,
    },
    Entry {
        name: "model_picker",
        visible: is_visible_model_picker,
        paint: paint_model_picker,
        sink: Sink::Buffer,
    },
    Entry {
        name: "session_browser",
        visible: is_visible_session_browser,
        paint: paint_session_browser,
        sink: Sink::Buffer,
    },
    Entry {
        name: "session_branching",
        visible: is_visible_session_branching,
        paint: paint_session_branching,
        sink: Sink::Buffer,
    },
    Entry {
        name: "export_dialog",
        visible: is_visible_export_dialog,
        paint: paint_export_dialog,
        sink: Sink::Frame,
    },
    Entry {
        name: "context_viz",
        visible: is_visible_context_viz,
        paint: paint_context_viz,
        sink: Sink::Frame,
    },
    Entry {
        name: "mcp_approval",
        visible: is_visible_mcp_approval,
        paint: paint_mcp_approval,
        sink: Sink::Buffer,
    },
    // --- the error modal is the frame's one early exit ----------------------
    Entry {
        name: "error_modal",
        visible: visible_error_modal,
        paint: paint_error_modal,
        sink: Sink::Frame,
    },
    // --- post-pass, and the topmost surfaces --------------------------------
    Entry {
        name: "notification_banner",
        visible: visible_notification_banner,
        paint: paint_notification_banner,
        sink: Sink::Frame,
    },
    Entry {
        name: "selection_highlight",
        visible: always_visible,
        paint: paint_selection_highlight,
        sink: Sink::Frame,
    },
    Entry {
        name: "selection_row_cache",
        visible: always_visible,
        paint: paint_selection_row_cache,
        sink: Sink::Frame,
    },
    Entry {
        name: "context_menu",
        visible: always_visible,
        paint: paint_context_menu,
        sink: Sink::Frame,
    },
    Entry {
        name: "debug_overlay",
        visible: always_visible,
        paint: paint_debug_overlay,
        sink: Sink::Frame,
    },
];

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// The eight chrome and post-pass rows that always paint.
fn always_visible(_app: &App, _ctx: &mut FrameCtx) -> bool {
    true
}

/// `permission_request` is the one overlay whose state is an `Option` rather
/// than a `.visible` flag.
fn visible_permission_dialog(app: &App, _ctx: &mut FrameCtx) -> bool {
    app.permission_request.is_some()
}

fn visible_status_row(_app: &App, ctx: &mut FrameCtx) -> bool {
    ctx.status_height > 0
}

fn visible_prompt_suggestions(_app: &App, ctx: &mut FrameCtx) -> bool {
    ctx.suggestions_height > 0
}

/// The error modal keys off the *current* notification's kind, not off an
/// overlay flag.
fn visible_error_modal(app: &App, _ctx: &mut FrameCtx) -> bool {
    matches!(app.notifications.current(), Some(n) if n.kind == NotificationKind::Error)
}

/// Non-error notifications render as toast banners, but not while a modal owns
/// the screen. `is_modal_open` is read here, at the point in paint order the
/// pre-refactor code read it — after the error-modal branch.
fn visible_notification_banner(app: &App, _ctx: &mut FrameCtx) -> bool {
    !is_modal_open(app) && app.notifications.current().is_some()
}

/// The one impure guard, and the reason `FrameCtx` reaches guards by `&mut`.
///
/// `refresh()` re-reads the process-wide delegation registry against a real
/// clock, so it has to run every frame whether or not anything is showing. It
/// is called from the guard because the guard is this row's only position in the
/// iteration, so the registry read lands at exactly the point in paint order it
/// lands today — after the transcript row, before the status row — and the body
/// paints whatever the guard stashed. The alternative (hoisting `refresh()` above
/// the loop) would move the clock read one row earlier; this keeps the order
/// byte-for-byte instead of arguing about it.
fn visible_background_task_rows(app: &App, ctx: &mut FrameCtx) -> bool {
    app.background_tasks.refresh();
    ctx.bg_rows = app.background_tasks.rows();
    !ctx.bg_rows.is_empty()
}

// ---------------------------------------------------------------------------
// Row bodies — every one a delegation to the surface's own module
// ---------------------------------------------------------------------------

/// The only row that builds a widget. The base fill has no module of its own, so
/// there is nowhere else for it to live; everything with a module delegates.
/// Pinned by `dispatch_owns_no_widget_construction`.
fn paint_base_fill(frame: &mut Frame, _app: &App, ctx: &mut FrameCtx) {
    frame.render_widget(
        Block::default().style(
            Style::default()
                .bg(theme::user_bg())
                .fg(theme_colors::text()),
        ),
        ctx.size,
    );
}

fn paint_messages(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_messages(frame, app, ctx.chunks[0]);
}

/// Docked to the bottom-left of the messages area, so the rows sit above the
/// transcript and below the overlays.
fn paint_background_task_rows(frame: &mut Frame, _app: &App, ctx: &mut FrameCtx) {
    let bg_area = background_tasks::rows_area(ctx.chunks[0], ctx.bg_rows.len());
    background_tasks::render_rows(
        frame,
        bg_area,
        &ctx.bg_rows,
        background_tasks::now_unix_secs(),
    );
}

fn paint_status_row(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_status_row(frame, app, ctx.chunks[2]);
}

/// Also records where the input landed. `render_prompt_suggestions` and
/// `render_footer` below read it, so the write has to stay inside this row
/// rather than moving after the loop.
fn paint_input(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_input(frame, app, ctx.chunks[3], ctx.prompt_focused);
    app.last_input_area.set(ctx.chunks[3]);
}

fn paint_prompt_suggestions(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_prompt_suggestions(frame, app, ctx.chunks[4]);
}

fn paint_footer(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_footer(frame, app, ctx.chunks[5]);
}

fn paint_usage_overlay(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_usage_overlay(
        frame,
        &app.usage_overlay,
        ctx.size,
        &UsageMetrics::from_app(app),
    );
}

/// The guard already established that the request exists; the `if let` is the
/// panic-free way to reach the value without an `unwrap` in production code.
fn paint_permission_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    if let Some(pr) = app.permission_request.as_ref() {
        render_permission_dialog(frame, pr, ctx.size);
    }
}

fn paint_rewind_flow(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_rewind_flow(frame, &app.rewind_flow, ctx.size);
}

fn paint_help_overlay(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_help_overlay(frame, &app.help_overlay, ctx.size);
}

fn paint_history_search_overlay(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_history_search_overlay(
        frame,
        &app.history_search_overlay,
        &app.prompt_input.history,
        ctx.size,
    );
}

fn paint_settings_screen(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_settings_screen(frame, &app.settings_screen, ctx.size);
}

fn paint_theme_screen(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_theme_screen(frame, &app.theme_screen, ctx.size);
}

fn paint_stats_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_stats_dialog(&app.stats_dialog, ctx.size, frame.buffer_mut());
}

fn paint_mcp_view(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_mcp_view(&app.mcp_view, ctx.size, frame.buffer_mut());
}

fn paint_agents_menu(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_agents_menu(&app.agents_menu, ctx.size, frame.buffer_mut());
}

/// The viewer works on a copy, so scrolling or folding cannot mutate app state
/// mid-paint.
fn paint_diff_viewer(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    let mut state = app.diff_viewer.clone();
    render_diff_dialog(&mut state, ctx.size, frame.buffer_mut());
}

fn paint_global_search(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_global_search(&app.global_search, ctx.size, frame.buffer_mut());
}

fn paint_memory_file_selector(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_memory_file_selector(&app.memory_file_selector, ctx.size, frame.buffer_mut());
}

fn paint_skills_view(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    crate::tui::skills_view::render_skills_view(frame, &app.skills_view, ctx.size);
}

fn paint_plugins_hub(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    crate::tui::plugins_hub::render_plugins_hub(frame, &app.plugins_hub, ctx.size);
}

fn paint_journey_view(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    crate::tui::journey_view::render_journey_view(frame, &app.journey_view, ctx.size);
}

fn paint_hooks_config_menu(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_hooks_config_menu(&app.hooks_config_menu, ctx.size, frame.buffer_mut());
}

/// Painted above the input box, two lines up from the bottom — NOT at the top of
/// the screen, which is where it used to be (iter-118, user-reported).
fn paint_voice_mode_notice(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    let notice_h = app.voice_mode_notice.height();
    if ctx.size.height > notice_h + 4 {
        let notice_y = ctx.size.y + ctx.size.height.saturating_sub(notice_h + 2);
        let notice_area = Rect {
            x: ctx.size.x,
            y: notice_y,
            width: ctx.size.width,
            height: notice_h,
        };
        render_voice_mode_notice(&app.voice_mode_notice, notice_area, frame.buffer_mut());
    }
}

fn paint_import_config_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_import_config_dialog(frame, &app.import_config_dialog, ctx.size);
}

fn paint_bypass_permissions_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_bypass_permissions_dialog(frame, &app.bypass_permissions_dialog, ctx.size);
}

/// Paints over the bypass-permissions dialog below it, so the model's question
/// is never hidden behind the startup confirmation.
fn paint_ask_user_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_ask_user_dialog(&app.ask_user_dialog, ctx.size, frame.buffer_mut());
}

fn paint_effort_picker(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    crate::effort_picker::render_effort_picker(frame, &app.effort_picker, ctx.size);
}

fn paint_import_config_picker(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_dialog_select(frame, &app.import_config_picker, ctx.size);
}

fn paint_connect_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_dialog_select(frame, &app.connect_dialog, ctx.size);
}

fn paint_key_input_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_key_input_dialog(frame, &app.key_input_dialog, ctx.size);
}

fn paint_custom_provider_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_custom_provider_dialog(frame, &app.custom_provider_dialog, ctx.size);
}

fn paint_free_mode_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    crate::free_mode_dialog::render_free_mode_dialog(frame, &app.free_mode_dialog, ctx.size);
}

fn paint_device_auth_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_device_auth_dialog(frame, &app.device_auth_dialog, ctx.size);
}

fn paint_command_palette(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_dialog_select(frame, &app.command_palette, ctx.size);
}

fn paint_model_picker(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_model_picker(&app.model_picker, ctx.size, frame.buffer_mut());
}

fn paint_session_browser(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_session_browser(&app.session_browser, ctx.size, frame.buffer_mut());
}

fn paint_session_branching(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_session_branching(&app.session_branching, ctx.size, frame.buffer_mut());
}

fn paint_export_dialog(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_export_dialog(frame, &app.export_dialog, ctx.size);
}

fn paint_context_viz(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_context_viz(
        frame,
        &app.context_viz,
        ctx.size,
        app.context_used_tokens,
        app.context_window_size,
        app.rate_limit_5h_pct,
        app.rate_limit_7day_pct,
        app.cost_usd,
    );
}

fn paint_mcp_approval(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    render_mcp_approval_dialog(&app.mcp_approval, ctx.size, frame.buffer_mut());
}

/// The one row that ends the frame. It paints on top of every row above it,
/// then `render_app` stops the ladder, runs the substitution pass and returns —
/// so the banner, the three selection passes, the debug overlay and every
/// post-draw pass are skipped exactly as the pre-refactor early return skipped
/// them.
fn paint_error_modal(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    if let Some(notif) = app.notifications.current()
        && notif.kind == NotificationKind::Error
    {
        let is_welcome_screen = app.messages.is_empty()
            && app.streaming_text.is_empty()
            && app.streaming_thinking.is_empty()
            && app.tool_use_blocks.is_empty();
        render_error_modal(
            frame,
            ctx.size,
            notif,
            app.error_modal_scroll_offset,
            app.footer_right_column_area.get(),
            is_welcome_screen,
        );
        ctx.stop = true;
    }
}

fn paint_notification_banner(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    if !is_modal_open(app) && app.notifications.current().is_some() {
        render_notification_banner(frame, &app.notifications, ctx.size);
    }
}

/// Post-render pass: inverts the selected cells and writes `app.selection_text`.
fn paint_selection_highlight(frame: &mut Frame, app: &App, _ctx: &mut FrameCtx) {
    apply_selection_highlight(frame, app);
}

/// Snapshots the rendered rows so the context menu can copy from them. Runs
/// after the highlight, as it does today.
fn paint_selection_row_cache(frame: &mut Frame, app: &App, _ctx: &mut FrameCtx) {
    cache_selectable_row_text(frame, app);
}

/// Reads `app.selection_text`, so it has to paint after both rows above.
fn paint_context_menu(frame: &mut Frame, app: &App, _ctx: &mut FrameCtx) {
    render_context_menu(frame, app);
}

/// Topmost. Always last.
fn paint_debug_overlay(frame: &mut Frame, app: &App, ctx: &mut FrameCtx) {
    crate::tui::debug::overlay::render_debug_overlay(frame, &app.debug_hub, ctx.size);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The expected table, spelled out independently of the declaration: the
    /// surface name and the sink, at a fixed position each. A dropped row, a
    /// duplicated row, a rename or a reordering all fail here, and every one of
    /// them is a frame change.
    const EXPECTED: &[(&str, Sink)] = &[
        ("base_fill", Sink::Frame),
        ("messages", Sink::Frame),
        ("background_task_rows", Sink::Frame),
        ("status_row", Sink::Frame),
        ("input", Sink::Frame),
        ("prompt_suggestions", Sink::Frame),
        ("footer", Sink::Frame),
        ("usage_overlay", Sink::Frame),
        ("permission_dialog", Sink::Frame),
        ("rewind_flow", Sink::Frame),
        ("help_overlay", Sink::Frame),
        ("history_search_overlay", Sink::Frame),
        ("settings_screen", Sink::Frame),
        ("theme_screen", Sink::Frame),
        ("stats_dialog", Sink::Buffer),
        ("mcp_view", Sink::Buffer),
        ("agents_menu", Sink::Buffer),
        ("diff_viewer", Sink::Buffer),
        ("global_search", Sink::Buffer),
        ("memory_file_selector", Sink::Buffer),
        ("skills_view", Sink::Frame),
        ("plugins_hub", Sink::Frame),
        ("journey_view", Sink::Frame),
        ("hooks_config_menu", Sink::Buffer),
        ("voice_mode_notice", Sink::Buffer),
        ("import_config_dialog", Sink::Frame),
        ("bypass_permissions_dialog", Sink::Frame),
        ("ask_user_dialog", Sink::Buffer),
        ("effort_picker", Sink::Frame),
        ("import_config_picker", Sink::Frame),
        ("connect_dialog", Sink::Frame),
        ("key_input_dialog", Sink::Frame),
        ("custom_provider_dialog", Sink::Frame),
        ("free_mode_dialog", Sink::Frame),
        ("device_auth_dialog", Sink::Frame),
        ("command_palette", Sink::Frame),
        ("model_picker", Sink::Buffer),
        ("session_browser", Sink::Buffer),
        ("session_branching", Sink::Buffer),
        ("export_dialog", Sink::Frame),
        ("context_viz", Sink::Frame),
        ("mcp_approval", Sink::Buffer),
        ("error_modal", Sink::Frame),
        ("notification_banner", Sink::Frame),
        ("selection_highlight", Sink::Frame),
        ("selection_row_cache", Sink::Frame),
        ("context_menu", Sink::Frame),
        ("debug_overlay", Sink::Frame),
    ];

    /// The structural guard for the table: every registered surface, once, in
    /// paint order, on the sink it paints through.
    ///
    /// The order is the contract. This ladder paints in z-order — later rows win
    /// the cells they share with earlier ones — so a reordering is not a
    /// refactor, it is a different frame. Pinning the whole column means a
    /// surface added, dropped, renamed or moved fails loudly here instead of
    /// quietly changing pixels that 120 committed goldens are watching.
    #[test]
    fn dispatch_rows_are_the_registered_surfaces_in_paint_order() {
        assert_eq!(
            DISPATCH.len(),
            EXPECTED.len(),
            "the dispatch table has {} rows but {} surfaces are registered — a \
             surface was added or dropped without updating the expectation",
            DISPATCH.len(),
            EXPECTED.len(),
        );
        for (i, (want_name, want_sink)) in EXPECTED.iter().enumerate() {
            let got = &DISPATCH[i];
            assert_eq!(
                got.name, *want_name,
                "row {i} is `{}`, expected `{want_name}` — paint order is the \
                 behaviour, so a row moved or renamed here changes the frame",
                got.name,
            );
            assert_eq!(
                got.sink,
                *want_sink,
                "row {i} (`{want_name}`) paints through the {} sink, expected {}",
                got.sink.as_str(),
                want_sink.as_str(),
            );
        }
    }

    /// No row may quietly become a second name for the same surface. The order
    /// check above cannot catch a duplicate on its own — editing the expectation
    /// to match would make it pass — so the two are separate.
    #[test]
    fn dispatch_declares_each_surface_exactly_once() {
        let names: Vec<&str> = DISPATCH.iter().map(|e| e.name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            names.len(),
            "a surface is registered twice: {names:?}"
        );
    }

    /// Both sinks are reachable from one row body, and the buffer is the
    /// primitive. In ratatui 0.30 `Frame::render_widget` is exactly
    /// `widget.render(area, self.buffer)`, so `frame.buffer_mut()` hands out the
    /// same buffer — which is why a row declares one body against `&mut Frame`.
    /// The `_buf` fork this rules out is the duplication class
    /// `begin_modal_frame` / `begin_modal_buf` guards against in
    /// `overlays/layout.rs`.
    #[test]
    fn no_row_declares_a_sink_fork() {
        let src = include_str!("dispatch.rs");
        // Assembled at runtime so this test's own source does not contain them.
        for needle in [
            ["_b", "uf("].concat(),
            ["render_rows", "_buf"].concat(),
            ["_render", "_buf"].concat(),
        ] {
            assert!(
                !src.contains(needle.as_str()),
                "`{needle}` in dispatch.rs: a row has forked its body per sink. One \
                 body reaches the buffer through `frame.buffer_mut()`."
            );
        }
    }

    /// The table owns no pixels. Every widget except the base fill is built in
    /// the surface's own module, which is what lets one agent rebuild one
    /// surface without opening this file. A widget appearing here is the sign
    /// that drawing logic has leaked back into the registry.
    #[test]
    fn dispatch_owns_no_widget_construction() {
        let src = include_str!("dispatch.rs");
        // Assembled at runtime so this test's own source does not contain them.
        let block = ["Block", "::default()"].concat();
        // The base fill is the one sanctioned widget, so its own constructor is
        // allowed exactly once.
        assert_eq!(
            src.matches(block.as_str()).count(),
            1,
            "only the base-fill row may build a widget here; a second one means a \
             surface is drawing in the registry"
        );
        for needle in [
            ["Para", "graph::"].concat(),
            ["Bor", "ders::"].concat(),
            ["Clear", ".render"].concat(),
        ] {
            assert!(
                !src.contains(needle.as_str()),
                "`{needle}` in dispatch.rs: a row is drawing instead of \
                 delegating. Move the drawing into the surface's own module."
            );
        }
    }

    /// The error modal is the frame's only early exit, and it is the last
    /// guarded row before the post-pass surfaces. If it moved earlier, the
    /// banner and the selection passes would start painting over it; if it
    /// moved later, the debug overlay would cover it.
    #[test]
    fn the_error_modal_stops_the_ladder_where_the_ladder_used_to_end() {
        let stop_row = DISPATCH
            .iter()
            .position(|e| e.name == "error_modal")
            .expect("the error modal has a row");
        assert_eq!(
            DISPATCH[stop_row + 1].name,
            "notification_banner",
            "the row after the error modal must be the banner — that pair is the \
             early exit's boundary"
        );
        assert_eq!(
            DISPATCH.last().map(|e| e.name),
            Some("debug_overlay"),
            "the debug overlay is topmost and must stay last"
        );
        assert_eq!(
            DISPATCH[0].name, "base_fill",
            "the base fill is the backdrop and paints first"
        );
    }

    /// Both sinks are actually in use. If a future change routed every row
    /// through one of them, the sink column would be decoration and this would
    /// say so.
    #[test]
    fn both_sinks_are_in_use() {
        let frames = DISPATCH.iter().filter(|e| e.sink == Sink::Frame).count();
        let buffers = DISPATCH.iter().filter(|e| e.sink == Sink::Buffer).count();
        assert_eq!(frames + buffers, DISPATCH.len());
        assert!(frames > 0, "no row paints through the Frame sink");
        assert!(buffers > 0, "no row paints through the Buffer sink");
    }
}
