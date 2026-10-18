// Vendored from jcode (crates/operant-tui/src/tui/ui.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-3 map; jcode issue refs stripped.
//
// TRUTH CLAUSE: this is the slim parent skeleton of upstream ui.rs — the
// child module declarations, the upstream re-export subset whose target
// modules are ported, and the App-free parent state helpers, ported
// verbatim. The App-bound frame draw path (render bands, `draw()`) and the
// un-ported children are deferred to the cutover iteration where operant's
// App state gets adapted onto the jcode renderer inputs; every excised item
// is named in the `[port-excision]` deferred block below (nothing dropped
// silently).
// The parent skeleton's re-export surface: upstream ui.rs imports exist FOR
// the child renderers + frame draw path; until the cutover wires them, the
// imports are unused by construction. Pruned/audited at the cutover + W7
// grep-gate pass — not a blanket pass on future edits to living code.
// [port-decision] expect list trimmed at batch-3: four of the six upstream
// lints no longer fire (their trigger code — the App-bound draw path — is
// deferred to the cutover), and an unfulfilled expect is itself a warning.
// Re-add `items_after_test_module`, `let_and_return`, `missing_const_for_
// thread_local`, `needless_borrow` when the draw path ports.
#![cfg_attr(test, expect(clippy::needless_return, clippy::items_after_test_module))]

use ratatui::{prelude::*, widgets::Paragraph};
use serde::Serialize;
#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
#[cfg(not(test))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[path = "ui_animations.rs"]
mod animations;
pub(crate) use animations::{
    disabled_animation_names, idle_animation_debug_json, idle_animation_reserved_height,
    record_idle_animation_area,
};
#[path = "ui_box.rs"]
mod box_utils;
#[path = "ui_changelog.rs"]
mod changelog;
#[path = "ui_diagram_pane.rs"]
mod diagram_pane;
#[path = "ui_frame_metrics.rs"]
mod frame_metrics;
#[path = "ui_header.rs"]
pub(crate) mod header;
#[path = "ui_inline_image.rs"]
pub(crate) mod inline_image_ui;
#[path = "ui_inline_interactive.rs"]
mod inline_interactive_ui;
#[path = "ui_inline.rs"]
mod inline_ui;
#[path = "ui_input.rs"]
pub(crate) mod input_ui;
#[path = "ui_memory_estimates.rs"]
mod memory_estimates;
#[path = "ui_memory.rs"]
mod memory_ui;
#[path = "ui_messages.rs"]
mod messages;
#[path = "ui_overlays.rs"]
mod overlays;
#[path = "ui_panel_image_preview.rs"]
pub(crate) mod panel_image_preview;
#[path = "ui_pinned.rs"]
mod pinned_ui;
#[path = "ui_prepare.rs"]
pub(crate) mod prepare;
#[path = "ui_todo_changes.rs"]
mod todo_changes;
#[path = "ui_tools.rs"]
pub(crate) mod tools_ui;
#[path = "ui_viewport.rs"]
pub(crate) mod viewport;
// Children ported on disk that upstream ui.rs declares outside its leading
// decl block (copy_selection) or the integrator added (frame_metrics is above):
#[path = "copy_selection.rs"]
pub(crate) mod copy_selection;
#[path = "ui_diff.rs"]
mod ui_diff;
use ui_diff::{
    DiffLineKind, ParsedDiffLine, collect_diff_lines, diff_add_color, diff_change_counts_for_tool,
    diff_del_color, generate_diff_lines_from_tool_input, tint_span_with_diff_color,
};
#[path = "ui/display_width.rs"]
pub(crate) mod display_width;
#[path = "ui/selection_highlight.rs"]
pub(crate) mod selection_highlight;
pub(crate) use display_width::{clamp_display_col, display_col_slice, line_display_width};

use box_utils::{
    line_plain_text, render_rounded_box, truncate_line_preserving_suffix_to_width,
    truncate_line_with_ellipsis_to_width,
};
use changelog::get_grouped_changelog;
pub(crate) use frame_metrics::recent_flicker_ui_notice;
use frame_metrics::{
    ChatLayoutMetrics, FLICKER_NOTICE_COPY_KEY, FullPrepPhaseMetrics, ViewportMetrics,
    begin_frame_resource_sample, finalize_frame_metrics, note_body_built, note_body_cache_hit,
    note_body_cache_lookup, note_body_cache_miss, note_body_incremental_reuse, note_body_request,
    note_chat_layout, note_full_prep_built, note_full_prep_cache_hit, note_full_prep_cache_lookup,
    note_full_prep_cache_miss, note_full_prep_phase_metrics, note_full_prep_request,
    note_prep_aspect, note_prep_overflow, note_prep_prepare_at, note_prep_restage,
    note_viewport_metrics, reset_frame_perf_stats, viewport_stability_hash,
};
// [port-source] ui.rs:107-120 — diagram_pane child FLIPPED live at the cutover:
// ui_diagram_pane.rs is on disk (33KB); every symbol below was verified present in it
// before the flip (draw_pinned_diagram :783, estimate_*_pane_height/width :661/:674,
// pinned_diagram_preferred_aspect_ratio :260, the two debug-probe entry points, and
// the cfg(test) helpers :444/:502/:636/:698/:732).
pub(crate) use diagram_pane::reset_pinned_diagram_debug_snapshot;
use diagram_pane::{
    draw_pinned_diagram, estimate_pinned_diagram_pane_height, estimate_pinned_diagram_pane_width,
    pinned_diagram_preferred_aspect_ratio,
};
// [port-source] ui.rs:143 — see the output_style [port-decision] in the excision block:
// the child file lives at operant_app/output_style.rs, so this is a re-rooted re-export.
pub(crate) use crate::tui::operant_app::output_style::adapt_buffer_for_emoji_preference;
// [port-source] ui.rs:144-152 — pinned_ui child FLIPPED live at the cutover:
// ui_pinned.rs is on disk (45KB); SidePanelDebugStats / SidePanelMermaidProbe /
// SidePanelMermaidProbeRect / debug_probe_side_panel_mermaid reach the parent through
// ui_pinned.rs:126-129 (`pub use mermaid_debug_support::{…}`), which was verified before
// the flip; draw_side_panel_markdown is `pub(super)` at ui_pinned.rs:526.
use inline_ui::{draw_inline_ui, inline_ui_height};

use memory_estimates::{estimate_prepared_chat_frame_bytes, estimate_prepared_messages_bytes};
use memory_ui::{group_into_tiles, render_memory_tiles, split_by_display_width};
use messages::get_cached_message_lines;
pub(crate) use messages::{
    render_assistant_message, render_background_task_message, render_reasoning_message,
    render_swarm_message, render_system_message, render_tool_message, render_usage_message,
};
pub(crate) use pinned_ui::clear_side_panel_debug_snapshot;
use pinned_ui::draw_side_panel_markdown;
use viewport::draw_messages;

use crate::tui::operant_app::color_support;
use crate::tui::operant_app::color_support::rgb;
use crate::tui::operant_app::visual_debug;
// Capture types the ported draw path populates (upstream ui.rs:2791 FrameCaptureBuilder,
// :3150 ImageRegionCapture, :3269 MessageCapture, :3424 MarginsCapture, :3600
// RenderTimingCapture). Upstream reaches them through ui.rs's `use super::*` glob;
// operant spells them out since the glob is not re-exported here.
use crate::tui::operant_app::visual_debug::{
    FrameCaptureBuilder, ImageRegionCapture, MarginsCapture, MessageCapture, RenderTimingCapture,
};
pub(crate) use crate::tui::operant_markdown::{CopyTargetKind, RawCopyTarget};
pub(crate) use crate::tui::operant_model::{
    CopyTarget, EditToolRange, ImageRegion, MessageBoundary, PreparedChatFrame, PreparedMessages,
    PreparedSection, PreparedSectionKind, WrappedLineMap,
};
// Upstream ui.rs :13-23, :93 glob-consumed by every child's `use super::*`:
// the module imports and the shared trait/type names must be in the parent's
// scope or the children's bare `markdown::`/`TuiState`/`DisplayMessage` refs fail.
use crate::tui::operant_app::app::ProcessingStatus;
use crate::tui::operant_app::core::DisplayMessageRoleExt;
use crate::tui::operant_app::info_widget;
use crate::tui::operant_app::markdown;
use crate::tui::operant_app::mermaid;
use crate::tui::operant_app::tui_fns::TuiState;
use crate::tui::operant_model::DisplayMessage;

// [port-excision] deferred to cutover (un-ported children). Each entry below was
// re-checked against the upstream child file's real existence at the cutover:
// FLIPPED entries name a child whose file IS on disk and whose `#[path]` decl +
// use blocks now sit above (diagram_pane, panel_image_preview, pinned_ui,
// output_style); still-excised entries name a child whose file is NOT in
// operant_ui/ (verified by `ls` at the cutover) and whose decl would not compile.
//
// [port-excision] debug_capture — `#[path = "ui_debug_capture.rs"] mod debug_capture;` plus its
//   use block `use debug_capture::{build_info_widget_summary, capture_widget_placements,
//   rect_within_bounds, rects_overlap, widget_overlaps_content};` (ui.rs :48-49, :103-106).
//   NOT FLIPPED at the cutover: `crates/operant-cli/src/tui/operant_ui/ui_debug_capture.rs` does
//   not exist. All five symbols live in upstream ui/ui_debug_capture.rs (:5, :18, :77, :88,
//   :105) and none has a ported home (operant's visual_debug.rs is the type-only partial).
//   The draw path's `debug_capture` option is therefore hard-wired to `None` there
//   (see `draw_inner`), so the capture/anomaly recording is inert until W-visual-debug.
// [port-excision] file_diff_ui — `#[path = "ui_file_diff.rs"] mod file_diff_ui;` plus its use
//   blocks (ui.rs :52-53, :121-126).
//   NOT FLIPPED: `ui_file_diff.rs` does not exist in operant_ui/. Upstream's only two call
//   sites in the draw path are `draw_file_diff_view` (:3410) and the layout `has_file_diff_edits`
//   predicate; the draw path's file-diff branch is gated off with a `[port-decision]`.
// [port-excised 2026-10-09] onboarding — `#[path = "ui_onboarding.rs"] mod onboarding;`
//   (ui.rs :72-73): upstream tui/onboarding.rs was never vendored (W8 scope,
//   operant ships its own connect-dialog onboarding), so the module path, the
//   welcome-takeover gate, the test-only draw hook, and the trait methods that
//   drove them are excised. See the 2026-10-09 visual-layer audit.
// [port-excision] smoothness — `#[path = "ui_smoothness.rs"] mod smoothness;` plus the two
//   pub(crate) re-export lines (ui.rs :83-84, :1388-1390).
//   NOT FLIPPED: `ui_smoothness.rs` does not exist in operant_ui/ (upstream tui/ui_smoothness.rs,
//   W7). `draw_inner`'s `smoothness::observe_frame` (:3568) is gated with a `[port-decision]`;
//   it is report-only (anchor-stability smoothing telemetry) and draws nothing.
// [port-excision] transitions — `#[path = "ui_transitions.rs"] mod transitions;` plus cfg(test)
//   uses `extract_line_text` / `inline_ui_gap_height` (ui.rs :89-90, :153-156).
//   NOT FLIPPED: `ui_transitions.rs` does not exist in operant_ui/; both symbols are `#[cfg(test)]`
//   and have no non-test consumer in the ported surface.
// [port-excision] `use crate::tui::mermaid;` (ui.rs :93) is FLIPPED: re-rooted to
//   `use crate::tui::operant_app::mermaid;` (line 183 above), the ported mermaid shim.
// [port-decision] output_style FLIPPED as a re-export, not a `mod` decl: upstream's
//   `mod output_style;` (ui.rs :74) resolves to operant-tui/src/tui/ui/output_style.rs, which
//   operant merged into `operant_app/output_style.rs` (operant-core's `output_style` plus the
//   adapter). The decl is therefore `pub(crate) use crate::tui::operant_app::output_style::…`
//   (ui.rs :143) rather than a local `mod`.
// [port-decision] layout_support / status_support / theme_support were initially
//   deferred as outside the port map, then pulled forward when the children's
//   super:: refs proved them live dependencies (ModRoot report + integrator
//   sizing: 191 LOC total). They land as ported siblings; decls + use blocks
//   below are upstream ui.rs :505-529 verbatim, with the color_support import
//   re-rooted to the operant_app 1-line re-export.
#[path = "ui_layout.rs"]
mod layout_support;
#[path = "ui_status.rs"]
mod status_support;
#[path = "ui_theme.rs"]
mod theme_support;
pub(crate) use layout_support::align_if_unset;
use layout_support::{
    centered_content_block_width, clear_area, draw_right_rail_chrome, left_aligned_content_inset,
    left_pad_lines_to_block_width, right_rail_border_style,
};
use status_support::{
    format_status_for_debug, is_running_stable_release, semver, shorten_model_name,
};
use theme_support::{
    accent_color, activity_indicator, activity_indicator_frame_index, ai_color, ai_text,
    asap_color, blend_color, dim_color, file_link_color, header_icon_color, header_name_color,
    header_session_color, pending_color, prompt_entry_bg_color, prompt_entry_color,
    prompt_entry_shimmer_color, queued_color, rainbow_prompt_color, system_message_color,
    tool_color, user_bg, user_color, user_text,
};
// [port-decision] resolve_tail_follow_scroll + TAIL_CATCHUP_MAX_STEP live in
//   ui_viewport.rs (:18-106, not ui.rs); sibling's viewport port owns them.
// [port-excision] activity_indicator, activity_indicator_frame_index — live in ui_theme.rs
//   (:8-16), not ui.rs and not in the port map.
// [port-excision] copy_badge_alt_label_from_config — lives in ui_viewport.rs :35, sibling's file.
// [port-excision] handterm_native_latex_cell_symbol — lives in ui_viewport.rs :18, sibling's file.
// [port-excision] SWARM_EXPAND_BADGE, SWARM_COLLAPSE_BADGE — consts in ui_messages.rs; reachable
//   via `messages::SWARM_EXPAND_BADGE` as upstream does (ui.rs :2541-2546).
// [port-excision] render_agentgrep_output_body — lives in ui_messages.rs :487, sibling's file.
// [port-excision] tail_catchup_active, request_tail_follow_snap, visible_expand_edit_badge_at,
//   visible_expand_edit_badge, visible_expand_edit_badge_line — App-free but NOT in the batch-3
//   helper list; if a ported child needs them they are tiny state accessors to port from
//   ui.rs :445-505, :608-660 in the same cutover pass.
// [port-excision] record_layout_snapshot, last_layout_snapshot, layout_snapshot, LayoutSnapshot,
//   LAST_LAYOUT, last_layout_state, last_chat_frame, render_state_test_lock,
//   RenderStateTestGuard, PromptViewportState, PROMPT_VIEWPORT_STATE, prompt_viewport_state,
//   clear_prompt_viewport_state, record_copy_viewport_frame_snapshot, clear_copy_viewport_snapshot,
//   copy_snapshot_for_pane, copy_point_from_screen, record_pane_snapshot_from_lines,
//   copy_pane_vertical_edge_point, copy_selection text helpers — parent plumbing feeding the
//   draw/copy paths; port with the cutover iteration.
// [port-excision] ui.rs's own trailing `#[cfg(test)] #[path = "ui_tests/mod.rs"] mod tests;`
//   (ui.rs :3711-3713) — the whole ui_tests tree drives App-bound rendering; deferred whole.
// [port-excision] the App-bound frame draw path (render bands, draw()) — ports at the cutover
//   iteration where operant's App state gets adapted onto the jcode renderer inputs.

/// Last known max scroll value from the renderer. Updated each frame.
/// Scroll handlers use this to clamp scroll_offset and prevent overshoot.
#[cfg(not(test))]
static LAST_MAX_SCROLL: AtomicUsize = AtomicUsize::new(0);
/// Whether the chat viewport used a native scrollbar in the most recent frame.
///
/// Initialized to `1` (assume visible) so the very first frame of a freshly
/// resumed/loaded session prepares the narrow (scrollbar-reserved) width FIRST.
/// Because narrow wraps at least as much as wide, an overflowing transcript is
/// detected on that single narrow build and kept, avoiding a wasted wide build
/// (~seconds on a long transcript) that would otherwise be discarded. Short
/// transcripts that fit still fall through to a (cheap) second wide build, and
/// the real decision is written back every frame, so steady state is unaffected.
#[cfg(not(test))]
static LAST_CHAT_SCROLLBAR_VISIBLE: AtomicUsize = AtomicUsize::new(1);
/// Total line count in the pinned diff/content pane (set during render).
#[cfg(not(test))]
static PINNED_PANE_TOTAL_LINES: AtomicUsize = AtomicUsize::new(0);
/// Effective scroll position of the side pane after render-time clamping.
#[cfg(not(test))]
static LAST_DIFF_PANE_EFFECTIVE_SCROLL: AtomicUsize = AtomicUsize::new(0);
/// Maximum scroll offset of the side pane on the most recent render frame.
#[cfg(not(test))]
static LAST_DIFF_PANE_MAX_SCROLL: AtomicUsize = AtomicUsize::new(0);
/// Total wrapped line count of the chat transcript on the most recent frame.
/// Used together with `LAST_RESOLVED_CHAT_SCROLL` to anchor the viewport when
/// older compacted history is loaded in (so the content under the reader stays
/// put instead of teleporting to the new absolute top).
#[cfg(not(test))]
static LAST_TOTAL_WRAPPED_LINES: AtomicUsize = AtomicUsize::new(0);
/// Height (rows) of the chat messages viewport on the most recent frame.
/// Terminal-style clear (Ctrl+L) sizes its blank spacer block from this so the
/// visible screen ends up exactly empty.
#[cfg(not(test))]
static LAST_CHAT_VIEWPORT_HEIGHT: AtomicUsize = AtomicUsize::new(0);
/// The chat scroll offset the renderer actually used on the most recent frame
/// (after clamping and after resolving any pending history anchor). Scroll
/// handlers adopt this so manual scrolling resumes from the on-screen position.
#[cfg(not(test))]
static LAST_RESOLVED_CHAT_SCROLL: AtomicUsize = AtomicUsize::new(0);
/// Whether the tail-follow viewport is mid catch-up slide (a large content
/// append is being scrolled into view over several frames instead of jumping).
/// Drives the redraw loop so the slide completes promptly.
#[cfg(not(test))]
static TAIL_CATCHUP_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Set by explicit user actions that resume bottom-follow (typing, End,
/// submitting a prompt). The next renderer pass consumes this request and snaps
/// directly to the tail instead of mistaking the large offset change for a
/// newly-appended content block that should use catch-up animation.
#[cfg(not(test))]
static TAIL_FOLLOW_SNAP_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    static TEST_LAST_MAX_SCROLL: Cell<usize> = const { Cell::new(0) };
    static TEST_LAST_CHAT_SCROLLBAR_VISIBLE: Cell<bool> = const { Cell::new(false) };
    static TEST_PINNED_PANE_TOTAL_LINES: Cell<usize> = const { Cell::new(0) };
    static TEST_LAST_DIFF_PANE_EFFECTIVE_SCROLL: Cell<usize> = const { Cell::new(0) };
    static TEST_LAST_DIFF_PANE_MAX_SCROLL: Cell<usize> = const { Cell::new(0) };
    static TEST_LAST_TOTAL_WRAPPED_LINES: Cell<usize> = const { Cell::new(0) };
    static TEST_LAST_CHAT_VIEWPORT_HEIGHT: Cell<usize> = const { Cell::new(0) };
    static TEST_LAST_RESOLVED_CHAT_SCROLL: Cell<usize> = const { Cell::new(0) };
    static TEST_TAIL_CATCHUP_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static TEST_TAIL_FOLLOW_SNAP_PENDING: Cell<bool> = const { Cell::new(false) };
    static TEST_LAST_LAYOUT: RefCell<Option<LayoutSnapshot>> = const { RefCell::new(None) };
    static TEST_LAST_CHAT_FRAME: RefCell<Option<Arc<PreparedChatFrame>>> = const { RefCell::new(None) };
    static TEST_LAST_STATUS_AREA: RefCell<Option<Rect>> = const { RefCell::new(None) };
    static TEST_VISIBLE_EXPAND_EDIT_BADGE: Cell<bool> = const { Cell::new(false) };
    static TEST_VISIBLE_EXPAND_EDIT_BADGE_LINE: Cell<Option<usize>> = const { Cell::new(None) };
    static TEST_VISIBLE_EXPAND_EDIT_BADGE_RECT: Cell<Option<Rect>> = const { Cell::new(None) };
    static TEST_VISIBLE_COPY_TARGETS: RefCell<Vec<VisibleCopyTarget>> = const { RefCell::new(Vec::new()) };
    static TEST_PROMPT_VIEWPORT_STATE: RefCell<PromptViewportState> =
        const { RefCell::new(PromptViewportState {
            initialized: false,
            last_visible_start: 0,
            last_visible_end: 0,
            active: None,
        }) };
    static TEST_COPY_VIEWPORT: RefCell<CopyViewportSnapshots> = RefCell::new(CopyViewportSnapshots::default());
}

/// Get the last known max scroll value (from the most recent render frame).
/// Returns 0 if no frame has been rendered yet.
pub fn last_max_scroll() -> usize {
    #[cfg(test)]
    {
        return TEST_LAST_MAX_SCROLL.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        LAST_MAX_SCROLL.load(Ordering::Relaxed)
    }
}

fn set_last_chat_scrollbar_visible(visible: bool) {
    #[cfg(test)]
    {
        TEST_LAST_CHAT_SCROLLBAR_VISIBLE.with(|state| state.set(visible));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_CHAT_SCROLLBAR_VISIBLE.store(usize::from(visible), Ordering::Relaxed);
    }
}

/// Whether the chat native scrollbar was visible on the most recent render frame.
/// Used as hysteresis so steady-state frames only prepare a single chat width
/// instead of both the wide and narrow variants (which thrashes the prep caches
/// on long transcripts during streaming).
fn last_chat_scrollbar_visible() -> bool {
    #[cfg(test)]
    {
        return TEST_LAST_CHAT_SCROLLBAR_VISIBLE.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        LAST_CHAT_SCROLLBAR_VISIBLE.load(Ordering::Relaxed) != 0
    }
}

pub(crate) fn set_last_max_scroll(value: usize) {
    #[cfg(test)]
    {
        TEST_LAST_MAX_SCROLL.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_MAX_SCROLL.store(value, Ordering::Relaxed);
    }
}

pub(crate) fn set_last_total_wrapped_lines(value: usize) {
    #[cfg(test)]
    {
        TEST_LAST_TOTAL_WRAPPED_LINES.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_TOTAL_WRAPPED_LINES.store(value, Ordering::Relaxed);
    }
}

pub(crate) fn set_last_chat_viewport_height(value: usize) {
    #[cfg(test)]
    {
        TEST_LAST_CHAT_VIEWPORT_HEIGHT.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_CHAT_VIEWPORT_HEIGHT.store(value, Ordering::Relaxed);
    }
}

/// The chat scroll offset the renderer actually used on the most recent frame
/// (after clamping and after resolving any pending history anchor).
pub fn last_resolved_chat_scroll() -> usize {
    #[cfg(test)]
    {
        return TEST_LAST_RESOLVED_CHAT_SCROLL.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        LAST_RESOLVED_CHAT_SCROLL.load(Ordering::Relaxed)
    }
}

pub(crate) fn set_last_resolved_chat_scroll(value: usize) {
    #[cfg(test)]
    {
        TEST_LAST_RESOLVED_CHAT_SCROLL.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_RESOLVED_CHAT_SCROLL.store(value, Ordering::Relaxed);
    }
}

pub(crate) fn set_tail_catchup_active(active: bool) {
    #[cfg(test)]
    {
        TEST_TAIL_CATCHUP_ACTIVE.with(|cell| cell.set(active));
        return;
    }
    #[cfg(not(test))]
    {
        TAIL_CATCHUP_ACTIVE.store(active, Ordering::Relaxed);
    }
}

pub(crate) fn take_tail_follow_snap_request() -> bool {
    #[cfg(test)]
    {
        return TEST_TAIL_FOLLOW_SNAP_PENDING.with(|cell| cell.replace(false));
    }
    #[cfg(not(test))]
    {
        TAIL_FOLLOW_SNAP_PENDING.swap(false, Ordering::Relaxed)
    }
}

pub(super) fn hash_text_for_cache(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    std::hash::Hasher::finish(&hasher)
}

// [port-decision] cache family below ported verbatim from operant-tui/src/tui/ui.rs
// :927-1331 at batch-3, after the sweep's `#[port-decision]` gate list proved the
// cache is on the transcript-content path (ui_prepare.rs six sites, ui_memory_estimates
// four). Only `crate::config::DiffDisplayMode` and `crate::config::DiagramDisplayMode`
// re-rooted to config_shim. The rest verbatim.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BodyCacheKey {
    width: u16,
    diff_mode: crate::tui::operant_app::config_shim::DiffDisplayMode,
    messages_version: u64,
    diagram_mode: crate::tui::operant_app::config_shim::DiagramDisplayMode,
    centered: bool,
    /// Mermaid render geometry depends on the scoped transcript/pane aspect
    /// profile as well as width. A vertical terminal resize can change this
    /// bucket without changing `width`, so it must invalidate the prepared body.
    mermaid_aspect_bucket: Option<u16>,
    /// Whether inline images render at all (Alt+M hides them).
    pin_images: bool,
    /// Whether inline images render expanded or as collapsed label stubs
    /// (Alt+Shift+I toggles; persisted).
    inline_images_visible: bool,
    /// Signature of the inline image set; anchored images render inside the
    /// body, so the body must rebuild when images arrive or change.
    images_signature: (usize, u64),
    /// Monotonic per-image expand-level version. Anchored images embed their
    /// expand-level geometry into the body, so a level change must rebuild the
    /// body exactly like an image-set change does.
    expanded_images_version: u64,
    /// Live swarm-member data renders beneath the tool call that spawned each
    /// member, so status/todo/tool-intent updates must invalidate the body.
    swarm_members_signature: u64,
}

#[derive(Clone)]
struct BodyCacheEntry {
    key: BodyCacheKey,
    prepared: Arc<PreparedMessages>,
    prepared_bytes: usize,
    msg_count: usize,
    /// `compacted_hidden_user_prompts` at build time. Prepend (suffix) reuse
    /// renders absolute prompt numbers into the reused lines, so a rebuild that
    /// stitches older history above a cached suffix must verify numbering
    /// continuity against the offset the base was built with.
    prompt_offset: usize,
}

const BODY_CACHE_MAX_ENTRIES: usize = 8;
// Keep enough room for a single large transcript snapshot so long sessions do not
// fall off a hard per-entry cache cliff and get rebuilt every frame.
const BODY_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const BODY_OVERSIZED_CACHE_MAX_ENTRIES: usize = 2;

#[derive(Default)]
struct BodyCacheState {
    entries: VecDeque<BodyCacheEntry>,
    oversized_entries: VecDeque<BodyCacheEntry>,
}

impl BodyCacheState {
    fn total_bytes(&self) -> usize {
        self.entries.iter().map(|entry| entry.prepared_bytes).sum()
    }

    fn get_exact_with_kind(
        &mut self,
        key: &BodyCacheKey,
    ) -> Option<(Arc<PreparedMessages>, CacheEntryKind)> {
        if let Some(pos) = self.entries.iter().position(|entry| &entry.key == key) {
            let entry = self.entries.remove(pos)?;
            let prepared = entry.prepared.clone();
            self.entries.push_front(entry);
            Some((prepared, CacheEntryKind::Regular))
        } else {
            let pos = self
                .oversized_entries
                .iter()
                .position(|entry| &entry.key == key)?;
            let entry = self.oversized_entries.remove(pos)?;
            let prepared = entry.prepared.clone();
            self.oversized_entries.push_front(entry);
            Some((prepared, CacheEntryKind::Oversized))
        }
    }

    fn take_best_incremental_base(
        &mut self,
        key: &BodyCacheKey,
    ) -> Option<(Arc<PreparedMessages>, usize, usize)> {
        let regular = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.msg_count > 0
                    && entry.key.width == key.width
                    && entry.key.diff_mode == key.diff_mode
                    && entry.key.diagram_mode == key.diagram_mode
                    && entry.key.centered == key.centered
                    && entry.key.mermaid_aspect_bucket == key.mermaid_aspect_bucket
                    // Anchored inline images render inside the body, and a
                    // late-arriving image may target an already-prepared
                    // message; only reuse bases built with the same image set.
                    && entry.key.pin_images == key.pin_images
                    && entry.key.inline_images_visible == key.inline_images_visible
                    && entry.key.images_signature == key.images_signature
                    && entry.key.expanded_images_version == key.expanded_images_version
                    && entry.key.swarm_members_signature == key.swarm_members_signature
            })
            .max_by_key(|(_, entry)| entry.msg_count)
            .map(|(idx, entry)| (false, idx, entry.msg_count));
        let oversized = self
            .oversized_entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.msg_count > 0
                    && entry.key.width == key.width
                    && entry.key.diff_mode == key.diff_mode
                    && entry.key.diagram_mode == key.diagram_mode
                    && entry.key.centered == key.centered
                    && entry.key.mermaid_aspect_bucket == key.mermaid_aspect_bucket
                    // Anchored inline images render inside the body, and a
                    // late-arriving image may target an already-prepared
                    // message; only reuse bases built with the same image set.
                    && entry.key.pin_images == key.pin_images
                    && entry.key.inline_images_visible == key.inline_images_visible
                    && entry.key.images_signature == key.images_signature
                    && entry.key.expanded_images_version == key.expanded_images_version
                    && entry.key.swarm_members_signature == key.swarm_members_signature
            })
            .max_by_key(|(_, entry)| entry.msg_count)
            .map(|(idx, entry)| (true, idx, entry.msg_count));

        let chosen = match (regular, oversized) {
            (Some(left), Some(right)) => {
                if left.2 >= right.2 {
                    left
                } else {
                    right
                }
            }
            (Some(entry), None) | (None, Some(entry)) => entry,
            (None, None) => return None,
        };

        let (is_oversized, idx, msg_count) = chosen;
        let entry = if is_oversized {
            self.oversized_entries.remove(idx)?
        } else {
            self.entries.remove(idx)?
        };
        Some((entry.prepared, msg_count, entry.prompt_offset))
    }

    fn insert(
        &mut self,
        key: BodyCacheKey,
        prepared: Arc<PreparedMessages>,
        msg_count: usize,
        prompt_offset: usize,
    ) {
        let prepared_bytes = estimate_prepared_messages_bytes(&prepared);
        if prepared_bytes > BODY_CACHE_MAX_BYTES {
            if let Some(pos) = self
                .oversized_entries
                .iter()
                .position(|entry| entry.key == key)
            {
                self.oversized_entries.remove(pos);
            }
            self.oversized_entries.push_front(BodyCacheEntry {
                key,
                prepared,
                prepared_bytes,
                msg_count,
                prompt_offset,
            });
            while self.oversized_entries.len() > BODY_OVERSIZED_CACHE_MAX_ENTRIES {
                self.oversized_entries.pop_back();
            }
            return;
        }
        if let Some(pos) = self
            .oversized_entries
            .iter()
            .position(|entry| entry.key == key)
        {
            self.oversized_entries.remove(pos);
        }
        if let Some(pos) = self.entries.iter().position(|entry| entry.key == key) {
            self.entries.remove(pos);
        }
        self.entries.push_front(BodyCacheEntry {
            key,
            prepared,
            prepared_bytes,
            msg_count,
            prompt_offset,
        });
        while self.entries.len() > BODY_CACHE_MAX_ENTRIES
            || self.total_bytes() > BODY_CACHE_MAX_BYTES
        {
            self.entries.pop_back();
        }
    }
}

static BODY_CACHE: OnceLock<Mutex<BodyCacheState>> = OnceLock::new();

fn body_cache() -> &'static Mutex<BodyCacheState> {
    BODY_CACHE.get_or_init(|| Mutex::new(BodyCacheState::default()))
}

// The reader's content position from the last drawn frame, in content
// coordinates (which message / raw line was under the reader's top row).
//
// The publisher used to be operant's deleted `render_messages`
// (`App::last_render_content_pos`); the ported chrome resolves the anchor
// from the prepared frame instead, so it records here and `TuiState::
// pending_resize_anchor` reads it back. `None` before the first draw, or when
// the top row carries no content identity.
thread_local! {
    static RESOLVED_READER_ANCHOR: std::cell::Cell<Option<crate::tui::operant_model::ContentPos>> =
        const { std::cell::Cell::new(None) };
}

/// Record where the reader sat after this frame's scroll resolve.
pub(crate) fn record_reader_anchor(pos: Option<crate::tui::operant_model::ContentPos>) {
    RESOLVED_READER_ANCHOR.with(|slot| slot.set(pos));
}

/// The reader's last recorded content position.
pub(crate) fn resolved_reader_anchor() -> Option<crate::tui::operant_model::ContentPos> {
    RESOLVED_READER_ANCHOR.with(|slot| slot.get())
}

thread_local! {
    /// The messages band's rect from the last `draw`, so operant-only
    /// surfaces that dock into the transcript (the background-task rows) can
    /// find it without duplicating the 10-band chrome's layout math.
    static LAST_MESSAGE_AREA: std::cell::Cell<Rect> =
        const { std::cell::Cell::new(Rect::new(0, 0, 0, 0)) };
}

thread_local! {
    /// The input band's rect from the last `draw`. `App::last_input_area`
    /// (the mouse click-routing gate) was written by the operant dispatch
    /// table the cutover deleted; without this the gate saw a zero-sized rect
    /// and classified every click as "not the input".
    static LAST_INPUT_AREA: std::cell::Cell<Rect> =
        const { std::cell::Cell::new(Rect::new(0, 0, 0, 0)) };
}

/// The input band from the last drawn frame (zero-sized before the first draw).
pub(crate) fn input_area() -> Rect {
    LAST_INPUT_AREA.with(|area| area.get())
}

/// The messages band from the last drawn frame (zero-sized before the first
/// draw). Consumers must tolerate a zero height.
pub(crate) fn message_area() -> Rect {
    LAST_MESSAGE_AREA.with(|area| area.get())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FullPrepCacheKey {
    width: u16,
    height: u16,
    diff_mode: crate::tui::operant_app::config_shim::DiffDisplayMode,
    messages_version: u64,
    diagram_mode: crate::tui::operant_app::config_shim::DiagramDisplayMode,
    centered: bool,
    /// The scoped Mermaid profile can also change when pane geometry changes
    /// while the transcript rectangle stays the same.
    mermaid_aspect_bucket: Option<u16>,
    is_processing: bool,
    streaming_text_len: usize,
    streaming_text_hash: u64,
    batch_progress_hash: u64,
    inline_images_signature: (usize, u64),
    /// Whether inline images render expanded or as collapsed label stubs.
    inline_images_visible: bool,
    /// Per-image expand-level version; anchored image geometry is embedded in
    /// the prepared frame, so a level change must invalidate it.
    expanded_images_version: u64,
    /// Signature of live swarm member cards embedded beneath spawn tool calls.
    swarm_members_signature: u64,
}

#[derive(Clone)]
struct FullPrepCacheEntry {
    key: FullPrepCacheKey,
    prepared: Arc<PreparedChatFrame>,
    prepared_bytes: usize,
}

const FULL_PREP_CACHE_MAX_ENTRIES: usize = 4;
// Full prepared frames duplicate some body data, so give them enough headroom to
// retain the active large transcript instead of forcing full recomposition.
const FULL_PREP_CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const FULL_PREP_OVERSIZED_CACHE_MAX_ENTRIES: usize = 2;

#[derive(Default)]
struct FullPrepCacheState {
    entries: VecDeque<FullPrepCacheEntry>,
    oversized_entries: VecDeque<FullPrepCacheEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum CacheEntryKind {
    Regular,
    Oversized,
}

impl FullPrepCacheState {
    fn total_bytes(&self) -> usize {
        self.entries.iter().map(|entry| entry.prepared_bytes).sum()
    }

    fn get_exact_with_kind(
        &mut self,
        key: &FullPrepCacheKey,
    ) -> Option<(Arc<PreparedChatFrame>, CacheEntryKind)> {
        if let Some(pos) = self.entries.iter().position(|entry| &entry.key == key) {
            let entry = self.entries.remove(pos)?;
            let prepared = entry.prepared.clone();
            self.entries.push_front(entry);
            Some((prepared, CacheEntryKind::Regular))
        } else {
            let pos = self
                .oversized_entries
                .iter()
                .position(|entry| &entry.key == key)?;
            let entry = self.oversized_entries.remove(pos)?;
            let prepared = entry.prepared.clone();
            self.oversized_entries.push_front(entry);
            Some((prepared, CacheEntryKind::Oversized))
        }
    }

    fn insert(&mut self, key: FullPrepCacheKey, prepared: Arc<PreparedChatFrame>) {
        let prepared_bytes = estimate_prepared_chat_frame_bytes(&prepared);
        if prepared_bytes > FULL_PREP_CACHE_MAX_BYTES {
            if let Some(pos) = self
                .oversized_entries
                .iter()
                .position(|entry| entry.key == key)
            {
                self.oversized_entries.remove(pos);
            }
            self.oversized_entries.push_front(FullPrepCacheEntry {
                key,
                prepared,
                prepared_bytes,
            });
            while self.oversized_entries.len() > FULL_PREP_OVERSIZED_CACHE_MAX_ENTRIES {
                self.oversized_entries.pop_back();
            }
            return;
        }
        if let Some(pos) = self
            .oversized_entries
            .iter()
            .position(|entry| entry.key == key)
        {
            self.oversized_entries.remove(pos);
        }
        if let Some(pos) = self.entries.iter().position(|entry| entry.key == key) {
            self.entries.remove(pos);
        }
        self.entries.push_front(FullPrepCacheEntry {
            key,
            prepared,
            prepared_bytes,
        });
        while self.entries.len() > FULL_PREP_CACHE_MAX_ENTRIES
            || self.total_bytes() > FULL_PREP_CACHE_MAX_BYTES
        {
            self.entries.pop_back();
        }
    }
}

static FULL_PREP_CACHE: OnceLock<Mutex<FullPrepCacheState>> = OnceLock::new();

fn full_prep_cache() -> &'static Mutex<FullPrepCacheState> {
    FULL_PREP_CACHE.get_or_init(|| Mutex::new(FullPrepCacheState::default()))
}

// Copy badges intentionally avoid h/j/k/l so they never shadow vi-style
// movement keys while the user is scanning visible actions.
pub(crate) const COPY_BADGE_KEYS: [char; 12] =
    ['s', 'd', 'f', 'g', 'w', 'e', 'r', 't', 'x', 'c', 'v', 'b'];

#[derive(Clone, Debug)]
pub(crate) struct VisibleCopyTarget {
    pub key: char,
    pub kind_label: String,
    pub copied_notice: String,
    pub content: String,
    /// Screen cells occupied by the rendered shortcut badge in the latest frame.
    pub badge_rect: Option<Rect>,
}

#[cfg(not(test))]
static VISIBLE_COPY_TARGETS: OnceLock<Mutex<Vec<VisibleCopyTarget>>> = OnceLock::new();

#[cfg(not(test))]
pub(crate) fn visible_copy_targets_state() -> &'static Mutex<Vec<VisibleCopyTarget>> {
    VISIBLE_COPY_TARGETS.get_or_init(|| Mutex::new(Vec::new()))
}

// [port-decision] prompt-entry animation block, verbatim port of upstream
// ui.rs:741-929 (struct + const + statics + four fns). No re-roots needed —
// state walks the SAME test/OnceLock pattern as the parent helpers above.

#[derive(Clone, Copy)]
struct PromptViewportAnimation {
    line_idx: usize,
    start_ms: u64,
}

#[derive(Clone, Copy, Default)]
struct PromptViewportState {
    initialized: bool,
    last_visible_start: usize,
    last_visible_end: usize,
    active: Option<PromptViewportAnimation>,
}

const PROMPT_ENTRY_ANIMATION_MS: u64 = 450;

#[cfg(not(test))]
static PROMPT_VIEWPORT_STATE: OnceLock<Mutex<PromptViewportState>> = OnceLock::new();

#[cfg(not(test))]
fn prompt_viewport_state() -> &'static Mutex<PromptViewportState> {
    PROMPT_VIEWPORT_STATE.get_or_init(|| Mutex::new(PromptViewportState::default()))
}

fn active_prompt_entry_animation(now_ms: u64) -> Option<PromptViewportAnimation> {
    #[cfg(test)]
    {
        TEST_PROMPT_VIEWPORT_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if let Some(anim) = state.active {
                if now_ms.saturating_sub(anim.start_ms) <= PROMPT_ENTRY_ANIMATION_MS {
                    return Some(anim);
                }
                state.active = None;
            }
            None
        })
    }
    #[cfg(not(test))]
    {
        let mut state = match prompt_viewport_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if let Some(anim) = state.active {
            if now_ms.saturating_sub(anim.start_ms) <= PROMPT_ENTRY_ANIMATION_MS {
                return Some(anim);
            }
            state.active = None;
        }
        None
    }
}

fn record_prompt_viewport(visible_start: usize, visible_end: usize) {
    #[cfg(test)]
    {
        TEST_PROMPT_VIEWPORT_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.initialized = true;
            state.last_visible_start = visible_start;
            state.last_visible_end = visible_end;
            state.active = None;
        });
        return;
    }
    #[cfg(not(test))]
    {
        let mut state = match prompt_viewport_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.initialized = true;
        state.last_visible_start = visible_start;
        state.last_visible_end = visible_end;
        state.active = None;
    }
}

fn update_prompt_entry_animation(
    user_prompt_lines: &[usize],
    visible_start: usize,
    visible_end: usize,
    now_ms: u64,
) {
    #[cfg(test)]
    {
        TEST_PROMPT_VIEWPORT_STATE.with(|state| {
            let mut state = state.borrow_mut();

            if !state.initialized {
                state.initialized = true;
                state.last_visible_start = visible_start;
                state.last_visible_end = visible_end;
                return;
            }

            let prev_visible_start = state.last_visible_start;
            let prev_visible_end = state.last_visible_end;
            let viewport_changed =
                prev_visible_start != visible_start || prev_visible_end != visible_end;

            if let Some(anim) = state.active {
                let still_fresh = now_ms.saturating_sub(anim.start_ms) <= PROMPT_ENTRY_ANIMATION_MS;
                let still_visible = anim.line_idx >= visible_start && anim.line_idx < visible_end;
                if still_fresh && still_visible {
                    state.last_visible_start = visible_start;
                    state.last_visible_end = visible_end;
                    return;
                }
                if !still_fresh || !still_visible {
                    state.active = None;
                }
            }

            if viewport_changed && state.active.is_none() {
                let newly_visible = user_prompt_lines.iter().copied().find(|line| {
                    *line >= visible_start
                        && *line < visible_end
                        && (*line < prev_visible_start || *line >= prev_visible_end)
                });
                if let Some(line_idx) = newly_visible {
                    state.active = Some(PromptViewportAnimation {
                        line_idx,
                        start_ms: now_ms,
                    });
                }
            }

            state.last_visible_start = visible_start;
            state.last_visible_end = visible_end;
        });
        return;
    }
    #[cfg(not(test))]
    {
        let mut state = match prompt_viewport_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if !state.initialized {
            state.initialized = true;
            state.last_visible_start = visible_start;
            state.last_visible_end = visible_end;
            return;
        }

        let prev_visible_start = state.last_visible_start;
        let prev_visible_end = state.last_visible_end;
        let viewport_changed =
            prev_visible_start != visible_start || prev_visible_end != visible_end;

        if let Some(anim) = state.active {
            let still_fresh = now_ms.saturating_sub(anim.start_ms) <= PROMPT_ENTRY_ANIMATION_MS;
            let still_visible = anim.line_idx >= visible_start && anim.line_idx < visible_end;
            if still_fresh && still_visible {
                state.last_visible_start = visible_start;
                state.last_visible_end = visible_end;
                return;
            }
            if !still_fresh || !still_visible {
                state.active = None;
            }
        }

        if viewport_changed && state.active.is_none() {
            let newly_visible = user_prompt_lines.iter().copied().find(|line| {
                *line >= visible_start
                    && *line < visible_end
                    && (*line < prev_visible_start || *line >= prev_visible_end)
            });
            if let Some(line_idx) = newly_visible {
                state.active = Some(PromptViewportAnimation {
                    line_idx,
                    start_ms: now_ms,
                });
            }
        }

        state.last_visible_start = visible_start;
        state.last_visible_end = visible_end;
    }
}

// [port-decision] batch-3 mod-tail splice. The redacted copy-viewport/state
// tail of upstream ui.rs lands here verbatim (scroll getters, expand-edit
// badge family, status/layout/chat-frame snapshots, render-state test family,
// copy-viewport snapshot family, native scrollbar helpers, ActiveFileDiffContext
// plumbing, profile module), with only the batch-3 re-roots applied:
//   crate::tui::CopySelection*  -> crate::tui::operant_ui::copy_selection::*
//   crate::tui::TuiState        -> crate::tui::operant_app::tui_fns::TuiState
//   operant_tui_messages::ImageRegionRender -> crate::tui::operant_model::ImageRegionRender
//   crate::logging::info        -> crate::tui::operant_app::logging::info
// Upstream siblings re-homed into this module (single-file splice) drop their
// `pub(super)` (they sit in the parent now): active_file_diff_context (from
// ui_file_diff.rs), copy_point_from_snapshot (from ui/copy_selection.rs),
// line_display_width + clamp_display_col (from ui/display_width.rs).
// Per-block source lines cited in [port-source] comments.

// [port-source] ui.rs:334-344
pub(crate) fn set_pinned_pane_total_lines(value: usize) {
    #[cfg(test)]
    {
        TEST_PINNED_PANE_TOTAL_LINES.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        PINNED_PANE_TOTAL_LINES.store(value, Ordering::Relaxed);
    }
}

// [port-source] ui.rs:346-356
pub(crate) fn set_last_diff_pane_effective_scroll(value: usize) {
    #[cfg(test)]
    {
        TEST_LAST_DIFF_PANE_EFFECTIVE_SCROLL.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_DIFF_PANE_EFFECTIVE_SCROLL.store(value, Ordering::Relaxed);
    }
}

// [port-source] ui.rs:358-368
pub(crate) fn set_last_diff_pane_max_scroll(value: usize) {
    #[cfg(test)]
    {
        TEST_LAST_DIFF_PANE_MAX_SCROLL.with(|cell| cell.set(value));
        return;
    }
    #[cfg(not(test))]
    {
        LAST_DIFF_PANE_MAX_SCROLL.store(value, Ordering::Relaxed);
    }
}

// [port-source] ui.rs:370-381
/// Total wrapped line count of the chat transcript on the most recent frame.
/// Returns 0 if no frame has been rendered yet.
pub fn last_total_wrapped_lines() -> usize {
    #[cfg(test)]
    {
        return TEST_LAST_TOTAL_WRAPPED_LINES.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        LAST_TOTAL_WRAPPED_LINES.load(Ordering::Relaxed)
    }
}

// [port-source] ui.rs:445-456
/// Whether the tail-follow viewport is still sliding toward the bottom after a
/// large append. The redraw loop keeps animation cadence while this is set.
pub(crate) fn tail_catchup_active() -> bool {
    #[cfg(test)]
    {
        return TEST_TAIL_CATCHUP_ACTIVE.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        TAIL_CATCHUP_ACTIVE.load(Ordering::Relaxed)
    }
}

// [port-source] ui.rs:470-487
/// Request that the next tail-follow render land at the exact bottom.
///
/// Set by explicit navigation and composer actions, and by a terminal resize,
/// which rewraps the transcript and would otherwise look like a large append
/// that the catch-up animation slides through. Automatic transcript growth does
/// not set it, so large committed blocks still use the bounded catch-up
/// animation.
pub(crate) fn request_tail_follow_snap() {
    #[cfg(test)]
    {
        TEST_TAIL_FOLLOW_SNAP_PENDING.with(|cell| cell.set(true));
        return;
    }
    #[cfg(not(test))]
    {
        TAIL_FOLLOW_SNAP_PENDING.store(true, Ordering::Relaxed);
    }
}

// [port-source] ui.rs:537-545
#[derive(Clone, Debug)]
struct ActiveFileDiffContext {
    edit_index: usize,
    msg_index: usize,
    file_path: String,
    start_line: usize,
    end_line: usize,
    expandable: bool,
}

// [port-source] ui_file_diff.rs:356-411
fn find_visible_edit_tool(
    edit_ranges: &[EditToolRange],
    scroll: usize,
    visible_height: usize,
) -> Option<&EditToolRange> {
    if edit_ranges.is_empty() {
        return None;
    }

    let visible_start = scroll;
    let visible_end = scroll + visible_height;
    let visible_mid = scroll + visible_height / 2;
    let candidate_start = edit_ranges.partition_point(|range| range.end_line <= visible_start);
    let candidate_end = edit_ranges.partition_point(|range| range.start_line < visible_end);

    let mut best: Option<&EditToolRange> = None;
    let mut best_overlap = 0usize;
    let mut best_distance = usize::MAX;

    for range in &edit_ranges[candidate_start..candidate_end] {
        let overlap_start = range.start_line.max(visible_start);
        let overlap_end = range.end_line.min(visible_end);
        let overlap = overlap_end.saturating_sub(overlap_start);

        let range_mid = (range.start_line + range.end_line) / 2;
        let distance = range_mid.abs_diff(visible_mid);

        if overlap > best_overlap || (overlap == best_overlap && distance < best_distance) {
            best = Some(range);
            best_overlap = overlap;
            best_distance = distance;
        }
    }

    if best.is_some() {
        return best;
    }

    // No overlapping edit range. Check the nearest neighbors around the insertion window
    // instead of rescanning the entire history.
    for idx in [candidate_start.checked_sub(1), Some(candidate_start)]
        .into_iter()
        .flatten()
    {
        if let Some(range) = edit_ranges.get(idx) {
            let range_mid = (range.start_line + range.end_line) / 2;
            let distance = range_mid.abs_diff(visible_mid);
            if best.is_none() || distance < best_distance {
                best = Some(range);
                best_distance = distance;
            }
        }
    }

    best
}

// [port-source] ui_file_diff.rs:413-427
fn active_file_diff_context(
    prepared: &PreparedChatFrame,
    scroll: usize,
    visible_height: usize,
) -> Option<ActiveFileDiffContext> {
    let range = find_visible_edit_tool(&prepared.edit_tool_ranges, scroll, visible_height)?;
    Some(ActiveFileDiffContext {
        edit_index: range.edit_index + 1,
        msg_index: range.msg_index,
        file_path: range.file_path.clone(),
        start_line: range.start_line,
        end_line: range.end_line,
        expandable: range.expandable,
    })
}

// [port-source] ui.rs:564-571
#[cfg(not(test))]
static VISIBLE_EXPAND_EDIT_BADGE: OnceLock<Mutex<bool>> = OnceLock::new();

#[cfg(not(test))]
static VISIBLE_EXPAND_EDIT_BADGE_LINE: OnceLock<Mutex<Option<usize>>> = OnceLock::new();

#[cfg(not(test))]
static VISIBLE_EXPAND_EDIT_BADGE_RECT: OnceLock<Mutex<Option<Rect>>> = OnceLock::new();

// [port-source] ui.rs:578-591
#[cfg(not(test))]
fn visible_expand_edit_badge_state() -> &'static Mutex<bool> {
    VISIBLE_EXPAND_EDIT_BADGE.get_or_init(|| Mutex::new(false))
}

#[cfg(not(test))]
fn visible_expand_edit_badge_line_state() -> &'static Mutex<Option<usize>> {
    VISIBLE_EXPAND_EDIT_BADGE_LINE.get_or_init(|| Mutex::new(None))
}

#[cfg(not(test))]
fn visible_expand_edit_badge_rect_state() -> &'static Mutex<Option<Rect>> {
    VISIBLE_EXPAND_EDIT_BADGE_RECT.get_or_init(|| Mutex::new(None))
}

// [port-source] ui.rs:593-606
pub(crate) fn set_visible_expand_edit_badge_rect(rect: Option<Rect>) {
    #[cfg(test)]
    {
        TEST_VISIBLE_EXPAND_EDIT_BADGE_RECT.with(|state| state.set(rect));
        return;
    }
    #[cfg(not(test))]
    {
        let mut state = visible_expand_edit_badge_rect_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = rect;
    }
}

// [port-source] ui.rs:623-644
pub(crate) fn set_visible_expand_edit_badge(visible: bool, line: Option<usize>) {
    #[cfg(test)]
    {
        TEST_VISIBLE_EXPAND_EDIT_BADGE.with(|state| state.set(visible));
        TEST_VISIBLE_EXPAND_EDIT_BADGE_LINE.with(|state| state.set(line));
        return;
    }
    #[cfg(not(test))]
    {
        let mut visible_state = match visible_expand_edit_badge_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *visible_state = visible;

        let mut line_state = match visible_expand_edit_badge_line_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *line_state = line;
    }
}

// [port-source] ui.rs:676-692
fn set_visible_copy_targets(targets: Vec<VisibleCopyTarget>) {
    #[cfg(test)]
    {
        TEST_VISIBLE_COPY_TARGETS.with(|state| {
            *state.borrow_mut() = targets;
        });
        return;
    }
    #[cfg(not(test))]
    {
        let mut state = match visible_copy_targets_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *state = targets;
    }
}

// [port-source] ui.rs:1332-1368
#[cfg(not(test))]
static LAST_STATUS_AREA: OnceLock<Mutex<Option<Rect>>> = OnceLock::new();

#[cfg(not(test))]
fn last_status_area_state() -> &'static Mutex<Option<Rect>> {
    LAST_STATUS_AREA.get_or_init(|| Mutex::new(None))
}

pub(crate) fn record_status_area(area: Rect) {
    #[cfg(test)]
    {
        TEST_LAST_STATUS_AREA.with(|snapshot| {
            *snapshot.borrow_mut() = Some(area);
        });
        return;
    }
    #[cfg(not(test))]
    {
        if let Ok(mut snapshot) = last_status_area_state().lock() {
            *snapshot = Some(area);
        }
    }
}

// [port-source] ui.rs:1398-1500
#[derive(Clone, Copy, Debug)]
pub struct LayoutSnapshot {
    pub messages_area: Rect,
    pub diagram_area: Option<Rect>,
    pub diff_pane_area: Option<Rect>,
    pub input_area: Option<Rect>,
}

#[cfg(not(test))]
static LAST_LAYOUT: OnceLock<Mutex<Option<LayoutSnapshot>>> = OnceLock::new();

/// The prepared transcript frame the renderer last drew. The retained frame
/// *is* the published geometry: it carries per-item row ranges and totals, so
/// handlers outside `draw` can resolve a viewport anchor against it.
#[cfg(not(test))]
static LAST_CHAT_FRAME: OnceLock<Mutex<Option<Arc<PreparedChatFrame>>>> = OnceLock::new();

#[cfg(not(test))]
fn last_chat_frame_state() -> &'static Mutex<Option<Arc<PreparedChatFrame>>> {
    LAST_CHAT_FRAME.get_or_init(|| Mutex::new(None))
}

#[cfg(not(test))]
fn last_layout_state() -> &'static Mutex<Option<LayoutSnapshot>> {
    LAST_LAYOUT.get_or_init(|| Mutex::new(None))
}

pub fn record_layout_snapshot(
    messages_area: Rect,
    diagram_area: Option<Rect>,
    diff_pane_area: Option<Rect>,
    input_area: Option<Rect>,
) {
    #[cfg(test)]
    {
        TEST_LAST_LAYOUT.with(|snapshot| {
            *snapshot.borrow_mut() = Some(LayoutSnapshot {
                messages_area,
                diagram_area,
                diff_pane_area,
                input_area,
            });
        });
        return;
    }
    #[cfg(not(test))]
    {
        if let Ok(mut snapshot) = last_layout_state().lock() {
            *snapshot = Some(LayoutSnapshot {
                messages_area,
                diagram_area,
                diff_pane_area,
                input_area,
            });
        }
    }
}

/// Record the prepared transcript frame the renderer just drew.
pub(crate) fn set_last_chat_frame(frame: Arc<PreparedChatFrame>) {
    #[cfg(test)]
    {
        TEST_LAST_CHAT_FRAME.with(|slot| *slot.borrow_mut() = Some(frame));
        return;
    }
    #[cfg(not(test))]
    {
        if let Ok(mut slot) = last_chat_frame_state().lock() {
            *slot = Some(frame);
        }
    }
}

/// The prepared transcript frame the renderer last drew, if any.
// First production consumer lands in epic #1411 phase 4/5a.
pub(crate) fn last_chat_frame() -> Option<Arc<PreparedChatFrame>> {
    #[cfg(test)]
    {
        return TEST_LAST_CHAT_FRAME.with(|slot| slot.borrow().clone());
    }
    #[cfg(not(test))]
    {
        last_chat_frame_state()
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
    }
}

/// Guard for [`render_state_test_lock`] that also records ownership on this
/// thread, so a nested `clear_test_render_state_for_tests` can tell it is
/// already inside the lock instead of deadlocking on it.
#[cfg(test)]
pub(crate) struct RenderStateTestGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for RenderStateTestGuard {
    fn drop(&mut self) {
        RENDER_STATE_LOCK_HELD.with(|held| held.set(false));
    }
}

#[cfg(test)]
thread_local! {
    /// Whether this thread currently holds the render-state lock. Set by
    /// [`render_state_test_lock`]'s guard so nested clears can detect it.
    static RENDER_STATE_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// [port-source] ui.rs:1605-1616 — the upstream test-only onboarding draw hook
// (draw_onboarding_welcome_for_tests) was excised with the onboarding module
// path (see the [port-excised] note at the file head): it called a module that
// was never vendored and compiled under `#[cfg(any())]`.
// [port-source] ui.rs:1618-1727
#[derive(Clone)]
enum CopyViewportData {
    Dense {
        wrapped_plain_lines: Arc<Vec<String>>,
        wrapped_copy_offsets: Arc<Vec<usize>>,
        raw_plain_lines: Arc<Vec<String>>,
        wrapped_line_map: Arc<Vec<WrappedLineMap>>,
    },
    ChatFrame {
        prepared: Arc<PreparedChatFrame>,
    },
}

#[derive(Clone)]
struct CopyViewportSnapshot {
    pane: crate::tui::operant_ui::copy_selection::CopySelectionPane,
    data: CopyViewportData,
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
    left_margins: Vec<u16>,
}

impl CopyViewportSnapshot {
    fn wrapped_plain_line(&self, abs_line: usize) -> Option<&str> {
        match &self.data {
            CopyViewportData::Dense {
                wrapped_plain_lines,
                ..
            } => wrapped_plain_lines.get(abs_line).map(String::as_str),
            CopyViewportData::ChatFrame { prepared } => prepared.wrapped_plain_line(abs_line),
        }
    }

    fn wrapped_copy_offset(&self, abs_line: usize) -> Option<usize> {
        match &self.data {
            CopyViewportData::Dense {
                wrapped_copy_offsets,
                ..
            } => wrapped_copy_offsets.get(abs_line).copied(),
            CopyViewportData::ChatFrame { prepared } => prepared.wrapped_copy_offset(abs_line),
        }
    }
}

#[derive(Clone, Default)]
struct CopyViewportSnapshots {
    chat: Option<CopyViewportSnapshot>,
    side: Option<CopyViewportSnapshot>,
    input: Option<CopyViewportSnapshot>,
}

#[cfg(not(test))]
static LAST_COPY_VIEWPORT: OnceLock<Mutex<CopyViewportSnapshots>> = OnceLock::new();

// [port-source] ui.rs:1734-1735 + ui/profile.rs:1-54 (inlined: single-file splice)

mod profile {
    use std::sync::{Mutex, OnceLock};
    // Child modules do not inherit the parent's `use` bindings, so the timing
    // types the ported draw path records are imported here explicitly.
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct RenderProfile {
        frames: u64,
        total: Duration,
        prepare: Duration,
        draw: Duration,
        last_log: Option<Instant>,
    }

    static PROFILE_STATE: OnceLock<Mutex<RenderProfile>> = OnceLock::new();

    fn profile_state() -> &'static Mutex<RenderProfile> {
        PROFILE_STATE.get_or_init(|| Mutex::new(RenderProfile::default()))
    }

    pub(super) fn profile_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("OPERANT_TUI_PROFILE").is_ok())
    }

    pub(super) fn record_profile(prepare: Duration, draw: Duration, total: Duration) {
        let mut state = match profile_state().lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.frames += 1;
        state.prepare += prepare;
        state.draw += draw;
        state.total += total;

        let now = Instant::now();
        let should_log = match state.last_log {
            Some(last) => now.duration_since(last) >= Duration::from_secs(1),
            None => true,
        };
        if should_log && state.frames > 0 {
            let frames = state.frames as f64;
            let avg_prepare = state.prepare.as_secs_f64() * 1000.0 / frames;
            let avg_draw = state.draw.as_secs_f64() * 1000.0 / frames;
            let avg_total = state.total.as_secs_f64() * 1000.0 / frames;
            crate::tui::operant_app::logging::info(&format!(
                "TUI perf: {:.1} fps | prepare {:.2}ms | draw {:.2}ms | total {:.2}ms",
                frames, avg_prepare, avg_draw, avg_total
            ));
            state.frames = 0;
            state.prepare = Duration::from_secs(0);
            state.draw = Duration::from_secs(0);
            state.total = Duration::from_secs(0);
            state.last_log = Some(now);
        }
    }
}

use self::profile::{profile_enabled, record_profile};

// [port-decision] line_display_width/clamp_display_col moved to the
// ui/display_width.rs module (upstream's home — the earlier inlined splice is
// superseded by the whole-file module port; tests reach them via the
// mod.rs re-export).

// [port-source] ui.rs:1747-1784
#[cfg(not(test))]
fn copy_viewport_state() -> &'static Mutex<CopyViewportSnapshots> {
    LAST_COPY_VIEWPORT.get_or_init(|| Mutex::new(CopyViewportSnapshots::default()))
}

fn copy_snapshot_slot_mut(
    snapshots: &mut CopyViewportSnapshots,
    pane: crate::tui::operant_ui::copy_selection::CopySelectionPane,
) -> &mut Option<CopyViewportSnapshot> {
    match pane {
        crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat => &mut snapshots.chat,
        crate::tui::operant_ui::copy_selection::CopySelectionPane::SidePane => &mut snapshots.side,
        crate::tui::operant_ui::copy_selection::CopySelectionPane::Input => &mut snapshots.input,
    }
}

fn copy_snapshot_for_pane(
    pane: crate::tui::operant_ui::copy_selection::CopySelectionPane,
) -> Option<CopyViewportSnapshot> {
    #[cfg(test)]
    {
        TEST_COPY_VIEWPORT.with(|snapshots| {
            let snapshots = snapshots.borrow().clone();
            match pane {
                crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat => snapshots.chat,
                crate::tui::operant_ui::copy_selection::CopySelectionPane::SidePane => {
                    snapshots.side
                }
                crate::tui::operant_ui::copy_selection::CopySelectionPane::Input => snapshots.input,
            }
        })
    }
    #[cfg(not(test))]
    {
        let snapshots = copy_viewport_state().lock().ok()?.clone();
        match pane {
            crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat => snapshots.chat,
            crate::tui::operant_ui::copy_selection::CopySelectionPane::SidePane => snapshots.side,
            crate::tui::operant_ui::copy_selection::CopySelectionPane::Input => snapshots.input,
        }
    }
}

// [port-source] ui.rs:1786-1798
pub(crate) fn clear_copy_viewport_snapshot() {
    #[cfg(test)]
    {
        TEST_COPY_VIEWPORT.with(|state| {
            *state.borrow_mut() = CopyViewportSnapshots::default();
        });
        return;
    }
    #[cfg(not(test))]
    if let Ok(mut state) = copy_viewport_state().lock() {
        *state = CopyViewportSnapshots::default();
    }
}

// [port-source] ui.rs:1800-1850
#[expect(
    clippy::too_many_arguments,
    reason = "Viewport snapshot helpers carry explicit render state to avoid hidden globals in call sites"
)]
fn record_copy_pane_snapshot(
    pane: crate::tui::operant_ui::copy_selection::CopySelectionPane,
    wrapped_plain_lines: Arc<Vec<String>>,
    wrapped_copy_offsets: Arc<Vec<usize>>,
    raw_plain_lines: Arc<Vec<String>>,
    wrapped_line_map: Arc<Vec<WrappedLineMap>>,
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
    left_margins: &[u16],
) {
    #[cfg(test)]
    {
        TEST_COPY_VIEWPORT.with(|state| {
            *copy_snapshot_slot_mut(&mut state.borrow_mut(), pane) = Some(CopyViewportSnapshot {
                pane,
                data: CopyViewportData::Dense {
                    wrapped_plain_lines,
                    wrapped_copy_offsets,
                    raw_plain_lines,
                    wrapped_line_map,
                },
                scroll,
                visible_end,
                content_area,
                left_margins: left_margins.to_vec(),
            });
        });
        return;
    }
    #[cfg(not(test))]
    if let Ok(mut state) = copy_viewport_state().lock() {
        *copy_snapshot_slot_mut(&mut state, pane) = Some(CopyViewportSnapshot {
            pane,
            data: CopyViewportData::Dense {
                wrapped_plain_lines,
                wrapped_copy_offsets,
                raw_plain_lines,
                wrapped_line_map,
            },
            scroll,
            visible_end,
            content_area,
            left_margins: left_margins.to_vec(),
        });
    }
}

// [port-source] ui.rs:1852-1886
fn record_copy_viewport_frame_snapshot(
    prepared: Arc<PreparedChatFrame>,
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
    left_margins: &[u16],
) {
    #[cfg(test)]
    {
        TEST_COPY_VIEWPORT.with(|state| {
            *copy_snapshot_slot_mut(
                &mut state.borrow_mut(),
                crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat,
            ) = Some(CopyViewportSnapshot {
                pane: crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat,
                data: CopyViewportData::ChatFrame { prepared },
                scroll,
                visible_end,
                content_area,
                left_margins: left_margins.to_vec(),
            });
        });
        return;
    }
    #[cfg(not(test))]
    if let Ok(mut state) = copy_viewport_state().lock() {
        *copy_snapshot_slot_mut(
            &mut state,
            crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat,
        ) = Some(CopyViewportSnapshot {
            pane: crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat,
            data: CopyViewportData::ChatFrame { prepared },
            scroll,
            visible_end,
            content_area,
            left_margins: left_margins.to_vec(),
        });
    }
}

// [port-source] ui.rs:1888-1913
#[expect(
    clippy::too_many_arguments,
    reason = "Viewport snapshot helpers carry explicit render state to avoid hidden globals in call sites"
)]
pub(crate) fn record_side_pane_snapshot_precomputed(
    wrapped_plain_lines: Arc<Vec<String>>,
    wrapped_copy_offsets: Arc<Vec<usize>>,
    raw_plain_lines: Arc<Vec<String>>,
    wrapped_line_map: Arc<Vec<WrappedLineMap>>,
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
    left_margins: &[u16],
) {
    record_copy_pane_snapshot(
        crate::tui::operant_ui::copy_selection::CopySelectionPane::SidePane,
        wrapped_plain_lines,
        wrapped_copy_offsets,
        raw_plain_lines,
        wrapped_line_map,
        scroll,
        visible_end,
        content_area,
        left_margins,
    );
}

// [port-source] ui.rs:1958-1971
pub(crate) fn line_left_margins_for_area(lines: &[Line<'static>], area_width: u16) -> Vec<u16> {
    lines
        .iter()
        .map(|line| {
            let used = line.width().min(area_width as usize) as u16;
            let total_margin = area_width.saturating_sub(used);
            match line.alignment.unwrap_or(Alignment::Left) {
                Alignment::Left => 0,
                Alignment::Center => total_margin / 2,
                Alignment::Right => total_margin,
            }
        })
        .collect()
}

// [port-source] ui.rs:1988-2006
/// Record a copy-selection snapshot for the chat pane from already-wrapped
/// display lines. Used by full-screen overlays (e.g. `/changelog`) that replace
/// the chat viewport but still want drag-to-select-and-copy support. Each
/// display line is treated as a single raw line, so the copied text matches the
/// rendered text verbatim.
pub(crate) fn record_chat_overlay_copy_snapshot(
    wrapped_lines: &[Line<'static>],
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
) {
    record_pane_snapshot_from_lines(
        crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat,
        wrapped_lines,
        scroll,
        visible_end,
        content_area,
    );
}

// [port-source] ui.rs:2008-2034
/// Record a copy-selection snapshot for the prompt composer (input box).
/// Called from `draw_input` each frame with the composer's wrapped rows so a
/// mouse drag over the text being typed selects and copies it, exactly like
/// the chat transcript (issue #430). Raw lines are the logical `\n`-separated
/// input lines, so selections spanning soft wraps copy the original text.
pub(crate) fn record_input_copy_snapshot(
    wrapped_plain_lines: Vec<String>,
    raw_plain_lines: Vec<String>,
    wrapped_line_map: Vec<WrappedLineMap>,
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
    left_margins: &[u16],
) {
    let wrapped_copy_offsets = vec![0usize; wrapped_plain_lines.len()];
    record_copy_pane_snapshot(
        crate::tui::operant_ui::copy_selection::CopySelectionPane::Input,
        Arc::new(wrapped_plain_lines),
        Arc::new(wrapped_copy_offsets),
        Arc::new(raw_plain_lines),
        Arc::new(wrapped_line_map),
        scroll,
        visible_end,
        content_area,
        left_margins,
    );
}

// [port-source] ui.rs:2036-2068
fn record_pane_snapshot_from_lines(
    pane: crate::tui::operant_ui::copy_selection::CopySelectionPane,
    wrapped_lines: &[Line<'static>],
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
) {
    let left_margins = line_left_margins_for_area(wrapped_lines, content_area.width);
    let raw_plain_lines: Vec<String> = wrapped_lines.iter().map(line_plain_text).collect();
    let wrapped_line_map: Vec<WrappedLineMap> = raw_plain_lines
        .iter()
        .enumerate()
        .map(|(raw_line, text)| WrappedLineMap {
            raw_line,
            start_col: 0,
            end_col: line_display_width(text),
        })
        .collect();
    let visible_left_margins = left_margins
        .get(scroll..visible_end.min(left_margins.len()))
        .unwrap_or(&[]);
    record_copy_pane_snapshot(
        pane,
        Arc::new(raw_plain_lines.clone()),
        Arc::new(vec![0; wrapped_lines.len()]),
        Arc::new(raw_plain_lines),
        Arc::new(wrapped_line_map),
        scroll,
        visible_end,
        content_area,
        visible_left_margins,
    );
}

// [port-source] ui.rs:2123-2148
fn edge_autoscroll_zone_rows(height: u16) -> u16 {
    (height / 4).clamp(1, 3)
}

#[cfg(test)]
mod edge_autoscroll_zone_tests {
    use super::edge_autoscroll_zone_rows;

    #[test]
    fn zone_is_at_least_one_row_for_tiny_panes() {
        // Even a 1-2 row pane should keep a usable hot zone so the edge still triggers.
        assert_eq!(edge_autoscroll_zone_rows(0), 1);
        assert_eq!(edge_autoscroll_zone_rows(1), 1);
        assert_eq!(edge_autoscroll_zone_rows(3), 1);
        assert_eq!(edge_autoscroll_zone_rows(4), 1);
    }

    #[test]
    fn zone_scales_with_height_but_is_capped() {
        assert_eq!(edge_autoscroll_zone_rows(8), 2);
        assert_eq!(edge_autoscroll_zone_rows(12), 3);
        // Capped at 3 so tall panes keep a large neutral middle region.
        assert_eq!(edge_autoscroll_zone_rows(40), 3);
        assert_eq!(edge_autoscroll_zone_rows(200), 3);
    }
}

// [port-source] ui.rs:2308-2316
fn copy_pane_line_text(
    pane: crate::tui::operant_ui::copy_selection::CopySelectionPane,
    abs_line: usize,
) -> Option<String> {
    copy_snapshot_for_pane(pane)?
        .wrapped_plain_line(abs_line)
        .map(str::to_owned)
}

pub(crate) fn copy_viewport_line_text(abs_line: usize) -> Option<String> {
    copy_pane_line_text(
        crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat,
        abs_line,
    )
}

/// Copy text of one chat content line between two SCREEN columns
/// (inclusive; `None` = line edge), read synchronously from the last drawn
/// frame's copy snapshot — never from the painted cell buffer, which lags a
/// frame and only covers the visible window. The gutter prefix
/// (`wrapped_copy_offset`) is excluded and trailing spaces trimmed.
pub(crate) fn chat_line_copy_slice(
    abs_line: usize,
    from_col: Option<u16>,
    to_col: Option<u16>,
) -> Option<String> {
    let snapshot =
        copy_snapshot_for_pane(crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat)?;
    let text = snapshot.wrapped_plain_line(abs_line)?;
    let copy_start = snapshot.wrapped_copy_offset(abs_line).unwrap_or(0);
    let start = from_col
        .and_then(|col| chat_text_col(abs_line, col))
        .unwrap_or(0)
        .max(copy_start);
    let end = to_col
        .and_then(|col| chat_text_col(abs_line, col.saturating_add(1)))
        .unwrap_or_else(|| line_display_width(text));
    if end <= start {
        return Some(String::new());
    }
    Some(display_col_slice(text, start, end).trim_end().to_string())
}

/// SCREEN column → display column within chat content line `abs_line`,
/// resolved against the last drawn frame (content-area origin + that row's
/// left margin), clamped to the line. The gutter prefix is NOT excluded.
pub(crate) fn chat_text_col(abs_line: usize, screen_col: u16) -> Option<usize> {
    let snapshot =
        copy_snapshot_for_pane(crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat)?;
    let text = snapshot.wrapped_plain_line(abs_line)?;
    // Left margins are recorded per visible row; off-screen rows carry none.
    let margin = abs_line
        .checked_sub(snapshot.scroll)
        .and_then(|row| snapshot.left_margins.get(row).copied())
        .unwrap_or(0);
    let origin = snapshot.content_area.x.saturating_add(margin);
    Some(clamp_display_col(
        text,
        screen_col.saturating_sub(origin) as usize,
    ))
}

// [port-source] ui.rs:3626-3709
pub(crate) fn split_native_scrollbar_area(area: Rect, enabled: bool) -> (Rect, Option<Rect>) {
    if !enabled || area.width <= 1 {
        return (area, None);
    }

    let content = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    let scrollbar = Rect {
        x: area.x.saturating_add(area.width.saturating_sub(1)),
        y: area.y,
        width: 1,
        height: area.height,
    };
    (content, Some(scrollbar))
}

pub(crate) fn native_scrollbar_visible(
    enabled: bool,
    total_lines: usize,
    visible_height: usize,
) -> bool {
    enabled && visible_height > 0 && total_lines > visible_height
}

pub(crate) fn render_native_scrollbar(
    frame: &mut Frame,
    area: Rect,
    scroll: usize,
    total_lines: usize,
    visible_height: usize,
    focused: bool,
) {
    if area.width == 0
        || area.height == 0
        || !native_scrollbar_visible(true, total_lines, visible_height)
    {
        return;
    }

    let track_height = area.height as usize;
    let thumb_height = if visible_height == 0 || total_lines == 0 {
        1
    } else if total_lines <= visible_height {
        track_height
    } else {
        ((visible_height * track_height).div_ceil(total_lines)).clamp(1, track_height)
    };
    let max_thumb_offset = track_height.saturating_sub(thumb_height);
    #[allow(clippy::manual_checked_ops)] // vendored-verbatim upstream scrollbar math
    let max_scroll = total_lines.saturating_sub(visible_height);
    #[allow(clippy::manual_checked_ops)] // vendored-verbatim upstream scrollbar math
    let thumb_offset = if max_scroll == 0 {
        0
    } else {
        scroll.min(max_scroll) * max_thumb_offset / max_scroll
    };

    let thumb_color = if focused {
        rgb(188, 208, 240)
    } else {
        rgb(136, 148, 172)
    };

    let mut lines = Vec::with_capacity(track_height);
    for row in 0..track_height {
        let (glyph, color) = if row >= thumb_offset && row < thumb_offset + thumb_height {
            let glyph = if thumb_height == 1 {
                "•"
            } else if row == thumb_offset {
                "╷"
            } else if row + 1 == thumb_offset + thumb_height {
                "╵"
            } else {
                "│"
            };
            (glyph, thumb_color)
        } else {
            (" ", Color::Reset)
        };
        lines.push(Line::from(Span::styled(glyph, Style::default().fg(color))));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

// constant, inlined because operant's operant_render/swarm_gallery.rs kept the
// base const but excised this one (see that file's [port-excision] note: the
// strip renderers are W7). Same formula, same base const.
const STRIP_SPINNER_FPS: f32 =
    1000.0 / crate::tui::operant_render::swarm_gallery::STRIP_SPINNER_FRAME_MS as f32;

// [port-source] ui.rs:2659-3624 — the App-bound frame draw path, ported at the
// cutover onto the `TuiState` seam. Upstream already takes `app: &dyn TuiState`
// (post-W1 upstream refactor), so the bodies are verbatim modulo the re-root map
// and the four gated children named in the excision block above:
//
//   re-root table (upstream → operant)
//     crate::tui::markdown           → crate::tui::operant_app::markdown
//     crate::tui::mermaid            → crate::tui::operant_app::mermaid
//     crate::tui::keybind            → crate::tui::operant_app::keybind
//     crate::tui::OnboardingWelcomeKind → crate::tui::operant_app::tui_state::OnboardingWelcomeKind
//     crate::tui::side_panel_debug_json      → side_panel_debug_json (pinned_ui re-export)
//     crate::tui::reset_pinned_diagram_debug_snapshot → reset_pinned_diagram_debug_snapshot
//     crate::tui::clear_side_panel_debug_snapshot     → clear_side_panel_debug_snapshot
//     crate::config::DiagramDisplayMode    → crate::tui::operant_app::config_shim::DiagramDisplayMode
//     crate::config::DiagramPanePosition   → crate::tui::operant_app::config_shim::DiagramPanePosition
//     operant_tui_render::swarm_gallery      → crate::tui::operant_render::swarm_gallery
//     operant_tui_style                      → crate::tui::vendor::style::theme_mode
//     operant_tui_messages::ImageRegionRender → crate::tui::operant_model::ImageRegionRender
//     crate::logging::warn                 → crate::tui::operant_app::logging::warn
//     super::info_widget::swarm_strip_stands_down_for_dock → info_widget::swarm_strip_stands_down_for_dock
//     super::info_widget::swarm_gallery::  → info_widget::swarm_gallery::
//     super::idle_animation_active              → idle_animation_active (ported below from
//                                            upstream tui/redraw_schedule.rs:209)
//
// [port-decision] debug_capture is hard-wired to `None`: upstream's
// `visual_debug::is_enabled()` and the whole ui_debug_capture.rs helper set
// (capture_widget_placements / build_info_widget_summary / rect_within_bounds /
// rects_overlap / widget_overlaps_content) have no ported home, so the capture
// recording below is compiled but inert until W-visual-debug lands the sink.
// Wiring it back is a one-line change at the `debug_capture` binding: the
// `capture.*` field writes are all still there.
pub fn draw(frame: &mut Frame, app: &dyn TuiState) {
    // Reset the published band rects before anything can skip the layout: a
    // narrow frame, an early-exit branch or a fullscreen overlay can skip the
    // band assignment, and a STALE rect would route mouse clicks and cursor
    // motion against the previous frame's geometry.
    LAST_MESSAGE_AREA.with(|area| area.set(Rect::new(0, 0, 0, 0)));
    LAST_INPUT_AREA.with(|area| area.set(Rect::new(0, 0, 0, 0)));
    record_idle_animation_area(None);
    // Suggestions are read many times while composing one frame. Bump the
    // epoch here so the memo is scoped to exactly this frame.
    app.advance_command_suggestions_epoch();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::tui::operant_app::markdown::with_deferred_mermaid_render_context(|| {
            draw_inner(frame, app)
        })
    })) {
        Ok(()) => {}
        // [port-decision] render_recovered_panic_frame: no operant equivalent —
        // upstream tui/ui/draw_recovery.rs is not ported (it needs
        // operant_core::panic_util::panic_payload_to_string, which operant has no
        // home for), so a recovered frame is left as the partially-drawn buffer
        // instead of the "rendering error recovered" placeholder — wire at
        // operant_ui/ui/draw_recovery.rs.
        // The payload IS logged: once this draw drives the live render loop a
        // recovered panic would otherwise be invisible (stderr is swallowed by
        // raw mode + the alternate screen, and catch_unwind keeps the TUI
        // running on a partial frame), leaving only golden-baseline drift as
        // the symptom.
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            crate::tui::operant_app::logging::error(&format!(
                "TUI draw recovered from panic: {message}"
            ));
        }
    }
    // Adapt the finished frame at buffer level so every widget and overlay
    // follows the same policy. User-configured colors remain exact.
    // Attribute explicit palette overrides before light-theme contrast repair
    // can make distinct muted source grays converge to the same rendered ink.
    crate::tui::vendor::style::theme_mode::adapt_buffer_for_display(frame.buffer_mut());
    adapt_buffer_for_emoji_preference(frame.buffer_mut());
    // Cache eviction/clearing can outlive the last visible image. Carry Kitty
    // deletion commands on any completed frame so terminal-side pixel storage
    // is reclaimed even when no image widget renders again.
    // [port-decision] mermaid::render_pending_terminal_image_cleanup: no operant
    // equivalent — the ported operant_app/mermaid.rs is the no-engine shim (no
    // ratatui_image, no Kitty pixel store), so there is nothing to reclaim —
    // wire at operant_app::mermaid (upstream operant-tui-mermaid lib.rs:1581).
}

fn draw_inner(frame: &mut Frame, app: &dyn TuiState) {
    panel_image_preview::clear_regions();
    let area = frame.area().intersection(*frame.buffer_mut().area());
    if area.width == 0 || area.height == 0 {
        return;
    }

    let total_start = Instant::now();
    reset_frame_perf_stats();
    begin_frame_resource_sample();

    clear_copy_viewport_snapshot();

    // Clear full frame to prevent stale cells from prior layouts.
    // This is critical on macOS terminals where ratatui's diff-based updates
    // can leave outdated content when layout dimensions change between frames
    // (e.g., diagram pane toggling, streaming text clearing, tool calls finishing).
    // Uses Color::Reset (terminal default bg) so text selection highlighting works
    // natively in all terminal emulators.
    clear_area(frame, area);

    if let Some(hash) = app.panel_image_preview() {
        panel_image_preview::draw_preview(frame, area, hash);
        finalize_frame_metrics(
            app,
            total_start,
            Duration::ZERO,
            total_start.elapsed(),
            None,
        );
        return;
    }

    if let Some(scroll) = app.changelog_scroll() {
        overlays::draw_changelog_overlay(frame, area, scroll, app);
        finalize_frame_metrics(
            app,
            total_start,
            Duration::ZERO,
            total_start.elapsed(),
            None,
        );
        return;
    }

    if let Some(scroll) = app.help_scroll() {
        // 2026-10-14 batch, plan 10: /keys and /hotkeys reuse the help-overlay
        // machinery with binding-catalogue content (opened at the top),
        // rendered by their own painter with chord filtering.
        if app.help_overlay_content() == crate::tui::overlays::help::HelpOverlayContent::Commands {
            overlays::draw_help_overlay(frame, area, scroll, app);
        } else {
            overlays::draw_keys_overlay(frame, area, scroll, app);
        }
        finalize_frame_metrics(
            app,
            total_start,
            Duration::ZERO,
            total_start.elapsed(),
            None,
        );
        return;
    }

    // (2026-10-14 batch, plan 02) the model-status overlay dispatch is gone
    // with its orphaned renderer (see ui_overlays.rs note).
    // [port-decision] session_picker_overlay / login_picker_overlay /
    //   account_picker_overlay: no operant equivalent — the three overlay
    //   methods are excised from the ported TuiState trait (see
    //   operant_app/tui_fns.rs:18-23: their payload picker modules are unported
    //   waves), so the three early-return full-screen picker frames cannot be
    //   reached through the seam — wire at tui_state.rs (:809-816) plus the
    //   session/login/account picker modules. Anything a picker would have
    //   covered falls through to the normal frame below.
    // [port-decision] hard-wired to `None` — see the header note: upstream's
    // `visual_debug::is_enabled()` and the ui_debug_capture.rs helper set have no
    // ported home, so every `capture.*` write below is compiled but inert.
    let mut debug_capture: Option<FrameCaptureBuilder> = None;

    // Initialize visual debug capture if enabled
    let swarm_page_active = app.swarm_panel_full_page();

    // Check diagram display mode and get active diagrams early so we can
    // determine the horizontal split before computing input width etc.
    let diagram_mode = app.diagram_mode();
    // [port-decision] get_active_diagrams: no operant equivalent — the active
    //   diagram registry (upstream operant-tui-mermaid/src/mermaid_active.rs:86)
    //   is engine-side and the ported operant_app/mermaid.rs is the no-engine
    //   shim, so the pinned diagram pane has no candidates this frame —
    //   wire at operant_app::mermaid::get_active_diagrams.
    let diagrams: Vec<info_widget::DiagramInfo> = Vec::new();
    let diagram_count = diagrams.len();
    let selected_index = if diagram_count > 0 {
        app.diagram_index().min(diagram_count - 1)
    } else {
        0
    };
    let pane_enabled = app.diagram_pane_enabled();
    let pane_position = app.diagram_pane_position();
    let has_side_panel_content = !swarm_page_active && app.side_panel().focused_page().is_some();
    let diff_mode = app.diff_mode();
    let has_file_diff_edits =
        !swarm_page_active && diff_mode.is_file() && app.has_display_edit_tool_messages();
    let has_right_side_pane_content = has_side_panel_content || has_file_diff_edits;
    // Fullscreen side panel replaces the transcript area; status line and input stay.
    let side_panel_fullscreen = has_side_panel_content && app.side_panel_fullscreen();
    // Regular side-panel pages and full-file diffs share the right-hand surface.
    // Suppress a separate diagram pane to avoid a triple-split layout.
    let suppress_side_diagram = has_right_side_pane_content;
    let pinned_diagram = if !swarm_page_active
        && diagram_mode == crate::tui::operant_app::config_shim::DiagramDisplayMode::Pinned
        && pane_enabled
        && !suppress_side_diagram
    {
        diagrams.get(selected_index).cloned()
    } else {
        None
    };
    let diagram_focus = app.diagram_focus();
    let (diagram_scroll_x, diagram_scroll_y) = app.diagram_scroll();

    // Compute layout depending on pane position (Side = right column, Top = above chat).
    let (chat_area, diagram_area) = if let Some(diagram) = pinned_diagram.as_ref() {
        match pane_position {
            crate::tui::operant_app::config_shim::DiagramPanePosition::Side => {
                const MIN_DIAGRAM_WIDTH: u16 = 24;
                const MIN_CHAT_WIDTH: u16 = 20;
                let max_diagram = area.width.saturating_sub(MIN_CHAT_WIDTH);
                if max_diagram >= MIN_DIAGRAM_WIDTH {
                    let ratio = app.diagram_pane_ratio().clamp(25, 100) as u32;
                    let ratio_target = ((area.width as u32 * ratio) / 100) as u16;
                    let needed =
                        estimate_pinned_diagram_pane_width(diagram, area.height, MIN_DIAGRAM_WIDTH);
                    // The configured ratio is the upper bound for the pane so the
                    // transcript (which still renders the diagram inline) is never
                    // crushed. Shrink below the ratio when a diagram is narrow
                    // enough to need less, but do not grow past it: a large/tall
                    // diagram just scales down to fit the pane instead of eating
                    // the chat column.
                    let diagram_width = ratio_target
                        .min(needed.max(MIN_DIAGRAM_WIDTH))
                        .max(MIN_DIAGRAM_WIDTH)
                        .min(max_diagram);
                    let chat_width = area.width.saturating_sub(diagram_width);
                    if diagram_width > 0 && chat_width > 0 {
                        let chat = Rect {
                            x: area.x,
                            y: area.y,
                            width: chat_width,
                            height: area.height,
                        };
                        let diag = Rect {
                            x: area.x + chat_width,
                            y: area.y,
                            width: diagram_width,
                            height: area.height,
                        };
                        (chat, Some(diag))
                    } else {
                        (area, None)
                    }
                } else {
                    (area, None)
                }
            }
            crate::tui::operant_app::config_shim::DiagramPanePosition::Top => {
                const MIN_DIAGRAM_HEIGHT: u16 = 6;
                const MIN_CHAT_HEIGHT: u16 = 8;
                let max_diagram = area.height.saturating_sub(MIN_CHAT_HEIGHT);
                if max_diagram >= MIN_DIAGRAM_HEIGHT {
                    let ratio = app.diagram_pane_ratio().clamp(20, 100) as u32;
                    let ratio_target = ((area.height as u32 * ratio) / 100) as u16;
                    let needed = estimate_pinned_diagram_pane_height(
                        diagram,
                        area.width,
                        MIN_DIAGRAM_HEIGHT,
                    );
                    // Cap the pane at the configured ratio so the transcript keeps
                    // its rows; shrink below it when the diagram is short. A tall
                    // diagram scales down to fit rather than swallowing the chat.
                    let diagram_height = ratio_target
                        .min(needed.max(MIN_DIAGRAM_HEIGHT))
                        .max(MIN_DIAGRAM_HEIGHT)
                        .min(max_diagram);
                    let chat_height = area.height.saturating_sub(diagram_height);
                    if diagram_height > 0 && chat_height > 0 {
                        let diag = Rect {
                            x: area.x,
                            y: area.y,
                            width: area.width,
                            height: diagram_height,
                        };
                        let chat = Rect {
                            x: area.x,
                            y: area.y + diagram_height,
                            width: area.width,
                            height: chat_height,
                        };
                        (chat, Some(diag))
                    } else {
                        (area, None)
                    }
                } else {
                    (area, None)
                }
            }
        }
    } else {
        (area, None)
    };

    let needs_side_pane = has_right_side_pane_content && !side_panel_fullscreen;

    let (chat_area, diff_pane_area) = if needs_side_pane {
        const MIN_DIFF_WIDTH: u16 = 30;
        const MIN_CHAT_WIDTH: u16 = 20;
        let effective_ratio = app.diagram_pane_ratio().clamp(25, 100) as u32;
        let max_diff = chat_area.width.saturating_sub(MIN_CHAT_WIDTH);
        if max_diff >= MIN_DIFF_WIDTH {
            let diff_width = (((chat_area.width as u32 * effective_ratio) / 100) as u16)
                .max(MIN_DIFF_WIDTH)
                .min(max_diff);
            let new_chat_width = chat_area.width.saturating_sub(diff_width);
            let chat = Rect {
                x: chat_area.x,
                y: chat_area.y,
                width: new_chat_width,
                height: chat_area.height,
            };
            let diff = Rect {
                x: chat_area.x + new_chat_width,
                y: chat_area.y,
                width: diff_width,
                height: chat_area.height,
            };
            (chat, Some(diff))
        } else {
            (chat_area, None)
        }
    } else {
        (chat_area, None)
    };

    // Inline swarm strip: when `swarm_spawn_mode = inline` and this session
    // manages agents, render a compact strip (vertical agent list by default,
    // see `agents.swarm_strip_layout`) directly above the status line instead
    // of a big gallery band. When the panel is focused (alt+n), the selected
    // agent's row expands in place with its live transcript tail and todos;
    // alt+↑/↓ select, alt+o pops out, alt+shift+p opens the swarm prompt,
    // esc exits, and plain typing keeps flowing to the chat input. The strip stands
    // down while the SwarmStatus dock widget (margin HUD) is showing the same
    // agents, unless the panel is focused (keyboard interaction lives here).
    // The stand-down is sticky (anchored blinks count as engaged, plus a short
    // linger after disengagement) because each strip appearance adds a row to
    // the bottom chrome and shoves the transcript up: reacting to raw
    // frame-by-frame dock visibility made the strip pop in and out and the
    // whole screen bounce (flicker).
    let swarm_strip_lines: Vec<Line<'static>> = if !swarm_page_active
        && app.inline_swarm_gallery_active()
        && (app.swarm_panel_focused() || !info_widget::swarm_strip_stands_down_for_dock())
    {
        let members = app.inline_swarm_members();
        if chat_area.width >= 24 {
            // [port-decision] swarm_panel_focus_key_label: no operant equivalent —
            //   operant's operant_app/keybind.rs exposes only side_panel_toggle_key_label
            //   and diagram_pane_visibility_key_label; the swarm-panel binding is not
            //   ported, so the strip's focus hint renders with an empty label —
            //   wire at operant_app/keybind.rs.
            let focus_key = String::new();
            // Use the same smooth cadence as the primary status spinner.
            let spinner_frame = (app.animation_elapsed() * STRIP_SPINNER_FPS) as usize;
            // Focused budget: chips + hints + a ~14-line detail viewport, but
            // never more than a third of the chat column so the transcript
            // stays usable on short terminals.
            let focused_budget = ((chat_area.height as usize) / 3).clamp(3, 16);
            info_widget::swarm_gallery::render_swarm_strip_lines(
                &members,
                app.swarm_panel_selected(),
                app.swarm_panel_focused(),
                &focus_key,
                spinner_frame,
                chat_area.width as usize,
                focused_budget,
            )
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let swarm_strip_height = swarm_strip_lines.len() as u16;

    // Calculate pending messages (queued + interleave) for numbering and layout
    let pending_count = input_ui::pending_prompt_count(app);
    let queued_height = pending_count.min(3) as u16;

    // Count user messages to show next prompt number
    let user_count = app.display_user_message_count();
    let next_prompt = user_count + 1;

    // Calculate input height based on the same wrapping logic used for rendering
    // (max 10 lines visible, scrolls if more).
    let base_input_height =
        input_ui::wrapped_input_line_count(app, chat_area.width, next_prompt).min(10) as u16;
    // Add 1 line for command suggestions, shell mode hints, or the Ctrl+Enter hint.
    let hint_line_height = input_ui::input_hint_line_height(app);
    let inline_block_height: u16 = inline_ui_height(app);
    let inline_ui_gap_height: u16 = if inline_block_height > 0 { 1 } else { 0 };
    let input_height = base_input_height + hint_line_height;

    if let Some(capture) = &mut debug_capture {
        capture.render_order.push("prepare_messages".to_string());
    }
    let prep_start = Instant::now();
    let chat_left_inset = left_aligned_content_inset(chat_area.width, app.centered_mode());
    let wide_prepare_width = chat_area.width.saturating_sub(chat_left_inset);
    let narrow_prepare_width = wide_prepare_width.saturating_sub(1);
    let pinned_mermaid_aspect_ratio =
        diagram_area.and_then(|area| pinned_diagram_preferred_aspect_ratio(area, pane_position));
    let aspect_start = Instant::now();
    // Aspect-ratio goal for transcript mermaid renders (deferred and
    // synchronous): the pinned pane's aspect wins when the pane is open so
    // inline and pane share one cached PNG; otherwise a terminal-friendly
    // inline goal keeps diagrams within a readable-height budget. Best-effort:
    // falls back to None (today's 4:3 sizing) when font geometry is
    // unavailable.
    // [port-decision] mermaid::transcript_preferred_aspect_ratio: no operant
    //   equivalent — the font-metric aspect goal
    //   (upstream operant-tui-mermaid/src/mermaid_content.rs:159) is engine-side
    //   and the ported operant_app/mermaid.rs is the no-engine shim, so the
    //   transcript mermaid goal is the pinned pane's ratio (or None) instead of a
    //   terminal-friendly inline goal — wire at operant_app::mermaid.
    let transcript_mermaid_aspect_ratio = pinned_mermaid_aspect_ratio;
    note_prep_aspect(aspect_start.elapsed());
    let prepare_at = |width: u16| {
        let started = Instant::now();
        let prepared =
            mermaid::with_preferred_aspect_ratio(transcript_mermaid_aspect_ratio, || {
                prepare::prepare_messages(app, width, chat_area.height)
            });
        note_prep_prepare_at(started.elapsed());
        prepared
    };

    // [port-excised 2026-10-09] onboarding welcome takeover: upstream gates a
    // first-run welcome screen on `onboarding_welcome_active()`, but the
    // upstream onboarding module (tui/onboarding.rs) was never vendored and
    // no implementor ever opted in — the gate was constant-false dead logic
    // guarding a phantom. Removed with its trait methods and the
    // OnboardingWelcomeKind/LoginImport* type stratum (tui_state.rs).
    // Operant's onboarding is its own connect-dialog flow.

    let show_animation = idle_animation_active(app);
    let animation_height: u16 = idle_animation_reserved_height(show_animation, input_height);
    let notification_height =
        input_ui::notification_height(app, chat_area.width).min(chat_area.height.saturating_sub(4));
    // Session status line (dir, branch, context, provider, model), always
    // pinned directly below the input line.
    let overscroll_height: u16 = 1;
    let fixed_height = 1
        + queued_height
        + swarm_strip_height
        + notification_height
        + inline_block_height
        + inline_ui_gap_height
        + input_height
        + overscroll_height
        + animation_height; // status + queued + swarm strip + notification + inline UI + gap + input + overscroll + idle animation
    let available_height = chat_area.height;
    let stable_fixed_height = fixed_height;
    let overflows = |prepared: &PreparedChatFrame| {
        let started = Instant::now();
        let result =
            (prepared.total_wrapped_lines().max(1) as u16) + stable_fixed_height > available_height;
        note_prep_overflow(started.elapsed());
        result
    };

    // Resolving native-scrollbar overflow can require wrapping the transcript at
    // two different widths (the wide layout, and one column narrower to reserve a
    // scrollbar column). Preparing both every frame doubles the most expensive work
    // and thrashes the prep caches on long transcripts during streaming. Use the
    // previous frame's scrollbar decision as hysteresis so the steady state only
    // prepares a single width; the second width is only built at a visibility
    // transition. This is safe because narrow wraps at least as much as wide, so
    // "narrow fits" implies "wide fits".
    let scrollbar_enabled = app.chat_native_scrollbar() && chat_area.width > 1;
    let initial_content_height;
    let (prepared, chat_scrollbar_visible) = if !scrollbar_enabled {
        let prepared_wide = prepare_at(wide_prepare_width);
        initial_content_height = prepared_wide.total_wrapped_lines().max(1) as u16;
        (prepared_wide, false)
    } else if last_chat_scrollbar_visible() {
        // Scrollbar was visible last frame: prepare the narrow (reserved-column)
        // layout first. If it still overflows we keep it without touching wide.
        let prepared_narrow = prepare_at(narrow_prepare_width);
        initial_content_height = prepared_narrow.total_wrapped_lines().max(1) as u16;
        if overflows(&prepared_narrow) {
            (prepared_narrow, true)
        } else {
            // Content shrank enough to fit even at the narrower width, so the wide
            // layout (which wraps no more) also fits: drop the scrollbar.
            (prepare_at(wide_prepare_width), false)
        }
    } else {
        // No scrollbar last frame: prepare the wide layout first. Only when it
        // overflows do we evaluate the narrow layout to decide on the scrollbar.
        let prepared_wide = prepare_at(wide_prepare_width);
        initial_content_height = prepared_wide.total_wrapped_lines().max(1) as u16;
        if !overflows(&prepared_wide) {
            (prepared_wide, false)
        } else {
            let prepared_narrow = prepare_at(narrow_prepare_width);
            if overflows(&prepared_narrow) {
                (prepared_narrow, true)
            } else {
                // Reserving a scrollbar column changed the wrapped content enough to
                // make it fit. Prefer the wide layout without the native scrollbar so
                // the transcript does not oscillate between layouts across
                // consecutive frames.
                (prepared_wide, false)
            }
        }
    };
    set_last_chat_scrollbar_visible(chat_scrollbar_visible);
    if let Some(capture) = &mut debug_capture {
        capture.image_regions = prepared
            .image_regions
            .iter()
            .map(|region| ImageRegionCapture {
                hash: format!("{:016x}", region.hash),
                abs_line_idx: region.abs_line_idx,
                height: region.height,
            })
            .collect();
    }
    let prep_elapsed = prep_start.elapsed();
    let content_height = prepared.total_wrapped_lines().max(1) as u16;

    // Terminal-style clear (Ctrl+L): the trailing spacer makes every visible
    // transcript row blank, so a normal bottom-anchored layout would park the
    // status line and numbered prompt at the *bottom* of a screenful of
    // blanks. Collapse the messages area instead, exactly like a terminal
    // after `clear`: the prompt sits at the top and the history is one scroll
    // away. Scrolling up, new output, or streaming all end the state (see
    // `terminal_clear_collapsed`) and restore the normal layout.
    let terminal_clear_collapsed = !swarm_page_active && app.terminal_clear_collapsed();
    let content_height = if terminal_clear_collapsed {
        0
    } else {
        content_height
    };

    // Use packed layout when content fits, scrolling layout otherwise
    let use_packed = terminal_clear_collapsed
        || (!swarm_page_active
            && !side_panel_fullscreen
            && content_height + fixed_height <= available_height);

    // Layout: messages (includes header), queued, status, notification, inline UI, gap, input, idle animation
    // All vertical chunks are within the chat_area (left column).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if use_packed {
            vec![
                Constraint::Length(if terminal_clear_collapsed {
                    0
                } else {
                    content_height.max(1)
                }), // 0 Messages (exact height; 0 when terminal-cleared)
                Constraint::Length(queued_height), // 1 Queued messages (above status)
                Constraint::Length(swarm_strip_height), // 2 Swarm strip (above status)
                Constraint::Length(1),             // 3 Status line
                Constraint::Length(notification_height), // 4 Notification line
                Constraint::Length(inline_block_height), // 5 Inline UI
                Constraint::Length(inline_ui_gap_height), // 6 Inline UI/input spacing
                Constraint::Length(input_height),  // 7 Input
                Constraint::Length(overscroll_height), // 8 Overscroll status line
                Constraint::Length(animation_height), // idle animation
            ]
        } else {
            vec![
                Constraint::Min(3),                       // 0 Messages (scrollable)
                Constraint::Length(queued_height),        // 1 Queued messages (above status)
                Constraint::Length(swarm_strip_height),   // 2 Swarm strip (above status)
                Constraint::Length(1),                    // 3 Status line
                Constraint::Length(notification_height),  // 4 Notification line
                Constraint::Length(inline_block_height),  // 5 Inline UI
                Constraint::Length(inline_ui_gap_height), // 6 Inline UI/input spacing
                Constraint::Length(input_height),         // 7 Input
                Constraint::Length(overscroll_height),    // 8 Overscroll status line
                Constraint::Length(animation_height),     // idle animation
            ]
        })
        .split(chat_area);
    record_status_area(chunks[3]);

    // Draw the inline swarm strip directly above the status line if present.
    if swarm_strip_height > 0 {
        clear_area(frame, chunks[2]);
        frame.render_widget(Paragraph::new(swarm_strip_lines.clone()), chunks[2]);
    }

    LAST_MESSAGE_AREA.with(|area| area.set(chunks[0]));
    LAST_INPUT_AREA.with(|area| area.set(chunks[7]));

    // Capture layout info for visual debug
    if let Some(capture) = &mut debug_capture {
        capture.layout.use_packed = use_packed;
        capture.layout.estimated_content_height = content_height as usize;
        capture.layout.messages_area = Some(chunks[0].into());
        if queued_height > 0 {
            capture.layout.queued_area = Some(chunks[1].into());
        }
        capture.layout.status_area = Some(chunks[3].into());
        capture.layout.input_area = Some(chunks[7].into());
        capture.layout.input_lines_raw = app.input().lines().count().max(1);
        capture.layout.input_lines_wrapped = base_input_height as usize;

        // Capture state snapshot
        capture.state.is_processing = app.is_processing();
        capture.state.input_len = app.input().len();
        capture.state.input_preview = app.input().chars().take(100).collect();
        capture.state.cursor_pos = app.cursor_pos();
        capture.state.scroll_offset = app.scroll_offset();
        capture.state.queued_count = pending_count;
        capture.state.message_count = app.display_messages().len();
        capture.state.streaming_text_len = app.streaming_text().len();
        capture.state.has_suggestions = !app.command_suggestions().is_empty();
        capture.state.status = format!("{:?}", app.status());
        capture.state.diagram_mode = Some(format!("{:?}", diagram_mode));
        capture.state.diagram_focus = diagram_focus;
        capture.state.diagram_index = selected_index;
        capture.state.diagram_count = diagram_count;
        capture.state.diagram_scroll_x = diagram_scroll_x;
        capture.state.diagram_scroll_y = diagram_scroll_y;
        capture.state.diagram_pane_ratio = app.diagram_pane_ratio();
        capture.state.diagram_pane_enabled = app.diagram_pane_enabled();
        capture.state.diagram_pane_position = Some(format!("{:?}", app.diagram_pane_position()));
        capture.state.diagram_zoom = app.diagram_zoom();

        // Capture rendered content
        // Queued messages
        capture.rendered_text.queued_messages = input_ui::pending_queue_preview(app);

        // Recent display messages (last 5 for context)
        capture.rendered_text.recent_messages = app
            .display_messages()
            .iter()
            .rev()
            .take(5)
            .map(|m| MessageCapture {
                role: m.role.clone(),
                content_preview: m.content.chars().take(200).collect(),
                content_len: m.content.len(),
            })
            .collect();

        // Streaming text preview
        let streaming = app.streaming_text();
        if !streaming.is_empty() {
            capture.rendered_text.streaming_text_preview = streaming.chars().take(500).collect();
        }

        // Status line content
        capture.rendered_text.status_line = format_status_for_debug(app);
    }

    if let Some(capture) = &mut debug_capture {
        capture.render_order.push("draw_messages".to_string());
    }
    let draw_start = Instant::now();

    // Messages area is chunks[0] within the chat column (already excludes diagram).
    let messages_area = chunks[0];
    let diff_pane_area = if side_panel_fullscreen {
        Some(messages_area)
    } else {
        diff_pane_area
    };
    let _ = swarm_strip_height;
    note_chat_layout(ChatLayoutMetrics {
        chat_area,
        messages_area,
        initial_content_height: initial_content_height as usize,
        content_height: content_height as usize,
        chat_scrollbar_visible,
        use_packed_layout: use_packed,
        has_side_panel_content,
        has_file_diff_edits,
    });

    if let Some(capture) = &mut debug_capture {
        capture.layout.messages_area = Some(messages_area.into());
        capture.layout.diagram_area = diagram_area.map(|r| r.into());
    }
    record_layout_snapshot(messages_area, diagram_area, diff_pane_area, Some(chunks[7]));

    let margins = if swarm_page_active {
        let members = app.inline_swarm_members();
        let spinner_frame = (app.animation_elapsed() * STRIP_SPINNER_FPS) as usize;
        let lines = info_widget::swarm_gallery::render_swarm_page_lines(
            &members,
            app.swarm_panel_selected(),
            spinner_frame,
            messages_area.width as usize,
            messages_area.height as usize,
        );
        clear_area(frame, messages_area);
        frame.render_widget(Paragraph::new(lines), messages_area);
        info_widget::Margins {
            right_widths: Vec::new(),
            left_widths: Vec::new(),
            centered: false,
            ..Default::default()
        }
    } else if terminal_clear_collapsed || side_panel_fullscreen {
        if side_panel_fullscreen {
            clear_area(frame, messages_area);
        }
        // Collapsed terminal-style clear: the messages chunk is zero-height, so
        // there is nothing to draw. Deliberately skip `draw_messages` so it does
        // not publish a zero-height viewport/max-scroll geometry that the scroll
        // handlers would then resolve against; the last real geometry stays
        // authoritative until the first scroll-up restores the full layout.
        info_widget::Margins {
            right_widths: Vec::new(),
            left_widths: Vec::new(),
            centered: app.centered_mode(),
            ..Default::default()
        }
    } else {
        draw_messages(
            frame,
            app,
            messages_area,
            prepared.clone(),
            chat_scrollbar_visible,
        )
    };

    reset_pinned_diagram_debug_snapshot();
    // Render pinned diagram if we have one
    if let (Some(diagram_info), Some(area)) = (&pinned_diagram, diagram_area) {
        if let Some(capture) = &mut debug_capture {
            capture.render_order.push("draw_pinned_diagram".to_string());
        }
        draw_pinned_diagram(
            frame,
            diagram_info,
            area,
            selected_index,
            diagram_count,
            diagram_focus,
            diagram_scroll_x,
            diagram_scroll_y,
            app.diagram_zoom(),
            pane_position,
            app.diagram_pane_animating(),
        );
    }

    clear_side_panel_debug_snapshot();
    if let Some(diff_area) = diff_pane_area {
        if has_side_panel_content {
            if let Some(capture) = &mut debug_capture {
                capture
                    .render_order
                    .push("draw_side_panel_markdown".to_string());
            }
            draw_side_panel_markdown(
                frame,
                diff_area,
                app,
                app.side_panel(),
                app.diff_pane_scroll(),
                app.diff_pane_focus(),
                app.centered_mode(),
            );
        } else if has_file_diff_edits {
            // [port-decision] draw_file_diff_view: no operant equivalent — upstream
            //   tui/ui_file_diff.rs is not ported (its cache types
            //   FileDiffCacheKey / FileDiffViewCacheEntry / file_diff_cache have no
            //   home), so a file-diff session shows the reserved right pane empty
            //   instead of the whole-file diff — wire at operant_ui/ui_file_diff.rs.
            if let Some(capture) = &mut debug_capture {
                capture.render_order.push("draw_file_diff_view".to_string());
            }
        }
    }

    let messages_draw = draw_start.elapsed();

    if let Some(capture) = &mut debug_capture {
        capture.layout.margins = Some(MarginsCapture {
            left_widths: margins.left_widths.clone(),
            right_widths: margins.right_widths.clone(),
            centered: margins.centered,
        });
    }
    let chrome_start = Instant::now();
    if queued_height > 0 {
        if let Some(capture) = &mut debug_capture {
            capture.render_order.push("draw_queued".to_string());
        }
        input_ui::draw_queued(frame, app, chunks[1], user_count + 1);
    }
    if let Some(capture) = &mut debug_capture {
        capture.render_order.push("draw_status".to_string());
    }
    input_ui::draw_status(frame, app, chunks[3], pending_count);
    if notification_height > 0 {
        input_ui::draw_notification(frame, app, chunks[4]);
    }
    if let Some(capture) = &mut debug_capture {
        capture.render_order.push("draw_input".to_string());
    }
    // Draw inline UI if active
    if inline_block_height > 0 {
        draw_inline_ui(frame, app, chunks[5]);
    }

    let _input_cursor = input_ui::draw_input(
        frame,
        app,
        chunks[7],
        user_count + pending_count + 1,
        &mut debug_capture,
    );

    if overscroll_height > 0 {
        input_ui::draw_overscroll_status(frame, app, chunks[8]);
    }

    if animation_height > 0 {
        animations::draw_idle_animation(frame, app, chunks[9]);
    }
    let chrome_elapsed = chrome_start.elapsed();

    // Draw info widget overlays (skip during idle animation - they look out of place)
    let widget_data_start = Instant::now();
    let widget_data = app.info_widget_data();
    let widget_data_elapsed = widget_data_start.elapsed();
    let mut widget_render_ms: Option<f32> = None;
    let mut placements: Vec<info_widget::WidgetPlacement> = Vec::new();
    let widget_bounds = messages_area;
    if app.info_widget_overlays_enabled()
        && !widget_data.is_empty()
        && !show_animation
        && !swarm_page_active
        && !side_panel_fullscreen
    {
        if let Some(capture) = &mut debug_capture {
            capture.render_order.push("render_info_widgets".to_string());
        }
        placements = info_widget::calculate_placements(widget_bounds, &margins, &widget_data);

        // [port-decision] debug_capture widget-placement capture + the
        //   overlap/out-of-bounds anomaly checks (upstream ui.rs:3487-3531) are
        //   inert: capture_widget_placements / build_info_widget_summary /
        //   rect_within_bounds / rects_overlap / widget_overlaps_content all live
        //   in the un-ported ui_debug_capture.rs. The render pass below is
        //   unaffected — wire at operant_ui/ui_debug_capture.rs.
        let widget_start = Instant::now();
        info_widget::render_all(frame, &placements, &widget_data);
        widget_render_ms = Some(widget_start.elapsed().as_secs_f32() * 1000.0);

        // Optional visual overlay for placements
    } else {
        // The widget pass did not run (idle-animation takeover or no widget data),
        // so nothing from the previous frame is on screen anymore. Clear the
        // remembered placements/anchors so consumers of last-frame state (the
        // swarm strip stand-down, idle fallback facts) do not keep reacting to
        // widgets that are no longer drawn.
        info_widget::note_widget_pass_skipped();
        if let Some(capture) = &mut debug_capture {
            capture.info_widgets = None;
        }
    }

    if let Some(capture) = &mut debug_capture
        && capture.info_widgets.is_none()
    {
        capture.info_widgets = None;
    }

    if visual_debug_overlay_enabled() {
        overlays::draw_debug_overlay(frame, &placements, &chunks);
    }

    // Command-suggestion popover: a late overlay pass so the palette floats
    // over existing rows (blank space, pinned footer, or the transcript tail)
    // instead of reserving layout height and shoving everything around.
    input_ui::draw_command_suggestions_overlay(frame, app, chunks[7]);

    // Ctrl+R reverse prompt-history search overlay (drawn after the command
    // palette so it wins when both could be visible).
    input_ui::draw_prompt_history_search_overlay(frame, app, chunks[7]);

    // Observe the rendered messages area for the anchor-stability (smoothness)
    // report. Runs on the final buffer so it sees exactly what the user sees.
    // [port-decision] smoothness::observe_frame: no operant equivalent —
    //   upstream tui/ui_smoothness.rs is not ported (W7); the call is
    //   report-only telemetry and draws nothing, so the frame is unaffected —
    //   wire at operant_ui/ui_smoothness.rs.

    let frame_elapsed = total_start.elapsed();
    if frame_elapsed >= Duration::from_millis(250) {
        crate::tui::operant_app::logging::warn(&format!(
            "TUI_RENDER_PHASES prepare={}ms messages={}ms chrome={}ms widget_data={}ms widget_render={}ms final={}ms total={}ms",
            prep_elapsed.as_millis(),
            messages_draw.as_millis(),
            chrome_elapsed.as_millis(),
            widget_data_elapsed.as_millis(),
            widget_render_ms.unwrap_or_default(),
            frame_elapsed
                .saturating_sub(prep_elapsed)
                .saturating_sub(messages_draw)
                .saturating_sub(chrome_elapsed)
                .saturating_sub(widget_data_elapsed)
                .saturating_sub(Duration::from_secs_f32(
                    widget_render_ms.unwrap_or_default() / 1000.0,
                ))
                .as_millis(),
            frame_elapsed.as_millis(),
        ));
    }

    // Record the frame capture if enabled
    if let Some(capture) = debug_capture {
        let total_draw = draw_start.elapsed();
        let render_timing = RenderTimingCapture {
            prepare_ms: prep_elapsed.as_secs_f32() * 1000.0,
            draw_ms: total_draw.as_secs_f32() * 1000.0,
            total_ms: total_start.elapsed().as_secs_f32() * 1000.0,
            messages_ms: Some(messages_draw.as_secs_f32() * 1000.0),
            widgets_ms: widget_render_ms,
        };

        let mut capture = capture;
        capture.render_timing = Some(render_timing);
        // [port-decision] the four debug-stat snapshots (mermaid / side_panel /
        //   markdown / theme) are part of the inert capture sink: mermaid's
        //   debug_stats_json is engine-side (un-ported shim) and the rest are
        //   reached through the debug_capture modules; they are captured by
        //   `visual_debug::record_frame` once W-visual-debug lands.
        capture.mermaid = None;
        capture.side_panel = None;
        capture.markdown = None;
        capture.theme = overlays::debug_palette_json();
        // [port-decision] visual_debug::record_frame: no operant equivalent — the
        //   capture sink family (buffer capture, sinks, serialization writers —
        //   upstream operant-tui-visual-debug lib.rs:308) is the plan's
        //   visual-debug skip, so the finished capture is dropped here rather
        //   than written anywhere — wire at operant_app::visual_debug.
        let _ = capture.build();
    }

    finalize_frame_metrics(
        app,
        total_start,
        prep_elapsed,
        draw_start.elapsed(),
        Some(messages_draw.as_secs_f64() * 1000.0),
    );
}

// [port-decision] visual_debug_overlay_enabled: no operant equivalent — operant's
//   operant_app/visual_debug.rs is the type-only partial and does not carry
//   upstream's `overlay_enabled()` gate (operant-tui-visual-debug lib.rs:298), so
//   the placement debug overlay is off — wire at operant_app::visual_debug.
fn visual_debug_overlay_enabled() -> bool {
    false
}

// [port-source] tui/redraw_schedule.rs:22, :40-52, :209-212 — ported here because
// `draw_inner` consumes it and upstream reaches it through `super::`. The
// upstream tui/redraw_schedule.rs module is not part of this port, so its
// REDRAW_DEEP_IDLE_AFTER const and deep_idle_dormant helper are inlined; the
// policy/config gates re-root to the ported perf + config_shim modules.
// [port-decision] display.idle_animation: no operant equivalent — operant's
//   config_shim::DisplayConfig has no `idle_animation` field (upstream
//   operant-config-types/src/display.rs:58), and its default is `false`
//   (display.rs:141), so the decorative-idle gate resolves to the same value
//   upstream ships by default — wire at config_shim::DisplayConfig.
const REDRAW_DEEP_IDLE_AFTER: Duration = Duration::from_secs(30);

fn deep_idle_dormant(state: &dyn TuiState) -> bool {
    let stream_dormant = state
        .time_since_activity()
        .map(|d| d >= REDRAW_DEEP_IDLE_AFTER)
        .unwrap_or(false);
    let user_dormant = state
        .time_since_user_interaction()
        .map(|d| d >= REDRAW_DEEP_IDLE_AFTER)
        // No interaction recorded yet: a fresh client that has never been
        // touched. Fall back to the stream/app clock alone, as before.
        .unwrap_or(true);
    stream_dormant && user_dormant
}

/// Whether the transcript contains any real conversation yet (a user prompt or
/// an assistant/tool/reasoning reply). A fresh screen that only holds
/// non-conversational notices (e.g. the "run /login when you're ready" system
/// message left after onboarding is declined) is still "idle", so the decorative
/// animation should keep playing until the user actually starts chatting.
fn has_started_conversation(state: &dyn TuiState) -> bool {
    state
        .display_messages()
        .iter()
        .any(|m| matches!(m.role.as_str(), "user" | "assistant" | "tool" | "reasoning"))
}

fn idle_animation_active_with_policy(
    state: &dyn TuiState,
    policy: &crate::tui::operant_app::perf::TuiPerfPolicy,
) -> bool {
    if state.remote_startup_phase_active() {
        return false;
    }

    // Decorative animations are purely visual; never spin them while the terminal
    // window/tab is backgrounded. A swarm of unfocused sessions would otherwise
    // each render a full-screen 3D scene at animation FPS, saturating every core.
    if !state.client_focused() {
        return false;
    }

    // The idle animation is decorative.  Leaving many dormant tabs/sessions open
    // should not keep every TUI repainting forever, especially when those tabs
    // are hidden behind a terminal multiplexer or kitty single-instance window.
    if deep_idle_dormant(state) {
        return false;
    }

    policy.enable_decorative_animations
        && policy.tier.idle_animation_enabled()
        && !has_started_conversation(state)
        && !state.is_processing()
        && state.streaming_text().is_empty()
        && state.queued_messages().is_empty()
}

pub(crate) fn idle_animation_active(state: &dyn TuiState) -> bool {
    let policy = crate::tui::operant_app::perf::tui_policy();
    idle_animation_active_with_policy(state, &policy)
}
