// Vendored from jcode (crates/jcode-tui/src/tui/ui.rs), MIT License,
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
#![cfg_attr(
    test,
    expect(
        clippy::items_after_test_module,
        clippy::let_and_return,
        clippy::missing_const_for_thread_local,
        clippy::needless_borrow,
        clippy::needless_return,
        clippy::too_many_arguments
    )
)]

use ratatui::{prelude::*, widgets::Paragraph};
#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
#[cfg(not(test))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[path = "ui_animations.rs"]
mod animations;
pub(crate) use animations::{
    idle_animation_debug_json, idle_donut_reserved_height, last_idle_animation_area,
    note_idle_animation_fast_path_blocked, note_idle_animation_full_repaint,
    note_idle_animation_partial_repaint, record_idle_animation_area, render_idle_animation_into,
};
#[path = "ui_box.rs"]
mod box_utils;
#[path = "ui_changelog.rs"]
mod changelog;
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
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "lands before its cutover wiring (App-seam adaptation)")
)]
#[path = "ui_diff.rs"]
mod ui_diff;
use ui_diff::{
    DiffLineKind, ParsedDiffLine, collect_diff_lines, diff_add_color, diff_change_counts_for_tool,
    diff_del_color, generate_diff_lines_from_tool_input, tint_span_with_diff_color,
};
#[path = "ui/selection_highlight.rs"]
pub(crate) mod selection_highlight;

#[cfg(test)]
pub(crate) use box_utils::truncate_line_to_width;
use box_utils::{
    line_plain_text, render_rounded_box, truncate_line_preserving_suffix_to_width,
    truncate_line_with_ellipsis_to_width,
};
use changelog::get_grouped_changelog;
#[cfg(test)]
use changelog::{ChangelogEntry, group_changelog_entries, parse_changelog_from};
pub(crate) use header::capitalize;
use inline_ui::{draw_inline_ui, inline_ui_height};
pub(crate) use memory_estimates::{debug_memory_profile, debug_side_panel_memory_profile};
use memory_estimates::{estimate_prepared_chat_frame_bytes, estimate_prepared_messages_bytes};
#[cfg(test)]
use memory_ui::{
    MemoryTileItem, choose_memory_tile_span, parse_memory_display_entries, plan_memory_tile,
};
use memory_ui::{group_into_tiles, render_memory_tiles, split_by_display_width};
use messages::get_cached_message_lines;
#[cfg_attr(test, allow(unused_imports))]
pub(crate) use messages::{
    SWARM_AGENT_SNAPSHOT_TITLE, compact_swarm_await_summary, encode_swarm_agent_snapshot,
    render_assistant_message, render_background_task_message, render_reasoning_message,
    render_swarm_message, render_system_message, render_tool_message, render_usage_message,
};
#[cfg(test)]
use viewport::compute_visible_margins;
use viewport::draw_messages;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use viewport::{
    copy_badge_reserved_width, expand_badge_reserved_width, pick_copy_badge_line,
    reserve_copy_badge_margins, truncate_line_for_copy_badge,
    truncate_line_in_place_to_width as truncate_copy_badge_line_to_width,
};
use frame_metrics::{
    ChatLayoutMetrics, FLICKER_NOTICE_COPY_KEY, FullPrepPhaseMetrics, ViewportMetrics,
    begin_frame_resource_sample, finalize_frame_metrics, note_body_built, note_body_cache_hit,
    note_body_cache_lookup, note_body_cache_miss, note_body_incremental_reuse, note_body_request,
    note_chat_layout, note_full_prep_built, note_full_prep_cache_hit, note_full_prep_cache_lookup,
    note_full_prep_cache_miss, note_full_prep_phase_metrics, note_full_prep_request,
    note_prep_aspect, note_prep_overflow, note_prep_prepare_at, note_prep_restage,
    note_viewport_metrics, reset_frame_perf_stats, viewport_stability_hash,
};
pub(crate) use frame_metrics::{
    DrawCallAttribution, FrameInputAttribution, frame_input_attribution_snapshot,
    key_to_paint_debug_json, note_frame_painted, note_key_event_read, record_draw_call_attribution,
    set_frame_input_attribution, wall_clock_ms,
};
pub(crate) use frame_metrics::{
    debug_draw_call_history, debug_flicker_frame_history, debug_slow_frame_history,
    recent_flicker_copy_target_for_key, recent_flicker_ui_notice,
};

pub(crate) use crate::tui::jcode_markdown::{CopyTargetKind, RawCopyTarget};
pub(crate) use crate::tui::jcode_model::{
    CopyTarget, EditToolRange, ImageRegion, MessageBoundary, PreparedChatFrame, PreparedMessages,
    PreparedSection, PreparedSectionKind, WrappedLineMap,
};
use crate::tui::jcode_app::color_support::rgb;
// Upstream ui.rs :13-23, :93 glob-consumed by every child's `use super::*`:
// the module imports and the shared trait/type names must be in the parent's
// scope or the children's bare `markdown::`/`TuiState`/`DisplayMessage` refs fail.
use crate::tui::jcode_app::info_widget;
use crate::tui::jcode_app::markdown;
use crate::tui::jcode_app::mermaid;
use crate::tui::jcode_app::core::DisplayMessageRoleExt;
use crate::tui::jcode_app::app::ProcessingStatus;
use crate::tui::jcode_app::tui_fns::TuiState;
use crate::tui::jcode_model::DisplayMessage;

// [port-excision] deferred to cutover (un-ported children):
// [port-excision] debug_capture — `#[path = "ui_debug_capture.rs"] mod debug_capture;` plus its
//   use block `use debug_capture::{build_info_widget_summary, capture_widget_placements,
//   rect_within_bounds, rects_overlap, widget_overlaps_content};` (ui.rs :48-49, :103-106).
// [port-excision] diagram_pane — `#[path = "ui_diagram_pane.rs"] mod diagram_pane;` plus its
//   pub/cfg(test)/pub(crate) use blocks (ui.rs :50-51, :107-120). Also `use crate::tui::mermaid;`
//   (ui.rs :93) — mermaid path module not in the batch-3 port map.
// [port-excision] file_diff_ui — `#[path = "ui_file_diff.rs"] mod file_diff_ui;` plus its use
//   blocks (ui.rs :52-53, :121-126).
// [port-excision] onboarding — `#[path = "ui_onboarding.rs"] mod onboarding;` (ui.rs :72-73).
// [port-excision] output_style — `mod output_style;` + `pub(crate) use
//   output_style::adapt_buffer_for_emoji_preference;` (ui.rs :74, :143).
// [port-excision] panel_image_preview — `#[path = "ui_panel_image_preview.rs"] pub(crate) mod
//   panel_image_preview;` (ui.rs :77-78).
// [port-excision] pinned_ui — `#[path = "ui_pinned.rs"] mod pinned_ui;` plus its use/pub use
//   blocks (ui.rs :79-80, :144-152).
// [port-excision] smoothness — `#[path = "ui_smoothness.rs"] mod smoothness;` plus the two
//   pub(crate) re-export lines (ui.rs :83-84, :1388-1390).
// [port-excision] transitions — `#[path = "ui_transitions.rs"] mod transitions;` plus cfg(test)
//   uses `extract_line_text` / `inline_ui_gap_height` (ui.rs :89-90, :153-156).
// [port-decision] layout_support / status_support / theme_support were initially
//   deferred as outside the port map, then pulled forward when the children's
//   super:: refs proved them live dependencies (ModRoot report + integrator
//   sizing: 191 LOC total). They land as ported siblings; decls + use blocks
//   below are upstream ui.rs :505-529 verbatim, with the color_support import
//   re-rooted to the jcode_app 1-line re-export.
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
#[cfg(test)]
pub(crate) use status_support::calculate_input_lines;
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
    static TEST_LAST_CHAT_FRAME: RefCell<Option<Arc<PreparedChatFrame>>> = const { RefCell::new(None) };
    static TEST_VISIBLE_EXPAND_EDIT_BADGE: Cell<bool> = const { Cell::new(false) };
    static TEST_VISIBLE_EXPAND_EDIT_BADGE_LINE: Cell<Option<usize>> = const { Cell::new(None) };
    static TEST_VISIBLE_EXPAND_EDIT_BADGE_RECT: Cell<Option<Rect>> = const { Cell::new(None) };
    static TEST_VISIBLE_COPY_TARGETS: RefCell<Vec<VisibleCopyTarget>> = const { RefCell::new(Vec::new()) };
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

/// Get the total line count from the pinned diff/content pane (set during render).
pub fn pinned_pane_total_lines() -> usize {
    #[cfg(test)]
    {
        return TEST_PINNED_PANE_TOTAL_LINES.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        PINNED_PANE_TOTAL_LINES.load(Ordering::Relaxed)
    }
}

pub fn last_diff_pane_effective_scroll() -> usize {
    #[cfg(test)]
    {
        return TEST_LAST_DIFF_PANE_EFFECTIVE_SCROLL.with(Cell::get);
    }
    #[cfg(not(test))]
    {
        LAST_DIFF_PANE_EFFECTIVE_SCROLL.load(Ordering::Relaxed)
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

// [port-decision] cache family below ported verbatim from jcode-tui/src/tui/ui.rs
// :927-1331 at batch-3, after the sweep's `#[port-decision]` gate list proved the
// cache is on the transcript-content path (ui_prepare.rs six sites, ui_memory_estimates
// four). Only `crate::config::DiffDisplayMode` and `crate::config::DiagramDisplayMode`
// re-rooted to config_shim. The rest verbatim.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BodyCacheKey {
    width: u16,
    diff_mode: crate::tui::jcode_app::config_shim::DiffDisplayMode,
    messages_version: u64,
    diagram_mode: crate::tui::jcode_app::config_shim::DiagramDisplayMode,
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

    #[cfg(test)]
    fn get_exact(&mut self, key: &BodyCacheKey) -> Option<Arc<PreparedMessages>> {
        self.get_exact_with_kind(key).map(|(prepared, _)| prepared)
    }

    #[cfg(test)]
    fn best_incremental_base(
        &self,
        key: &BodyCacheKey,
        _msg_count: usize,
    ) -> Option<(Arc<PreparedMessages>, usize)> {
        let regular = self
            .entries
            .iter()
            .filter(|entry| {
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
            .max_by_key(|entry| entry.msg_count)
            .map(|entry| (entry.prepared.clone(), entry.msg_count));
        let oversized = self
            .oversized_entries
            .iter()
            .filter(|entry| {
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
            .max_by_key(|entry| entry.msg_count)
            .map(|entry| (entry.prepared.clone(), entry.msg_count));

        match (regular, oversized) {
            (Some(left), Some(right)) => {
                if left.1 >= right.1 {
                    Some(left)
                } else {
                    Some(right)
                }
            }
            (Some(entry), None) | (None, Some(entry)) => Some(entry),
            (None, None) => None,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FullPrepCacheKey {
    width: u16,
    height: u16,
    diff_mode: crate::tui::jcode_app::config_shim::DiffDisplayMode,
    messages_version: u64,
    diagram_mode: crate::tui::jcode_app::config_shim::DiagramDisplayMode,
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

    #[cfg(test)]
    fn get_exact(&mut self, key: &FullPrepCacheKey) -> Option<Arc<PreparedChatFrame>> {
        self.get_exact_with_kind(key).map(|(prepared, _)| prepared)
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
pub(crate) const COPY_BADGE_KEYS: [char; 12] = ['s', 'd', 'f', 'g', 'w', 'e', 'r', 't', 'x', 'c', 'v', 'b'];

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

#[cfg(not(test))]
static VISIBLE_EXPAND_EDIT_BADGE: OnceLock<Mutex<bool>> = OnceLock::new();

#[cfg(not(test))]
static VISIBLE_EXPAND_EDIT_BADGE_LINE: OnceLock<Mutex<Option<usize>>> = OnceLock::new();

#[cfg(not(test))]
static VISIBLE_EXPAND_EDIT_BADGE_RECT: OnceLock<Mutex<Option<Rect>>> = OnceLock::new();

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

/// The prepared transcript frame the renderer last drew. The retained frame
/// *is* the published geometry: it carries per-item row ranges and totals, so
/// handlers outside `draw` can resolve a viewport anchor against it.
#[cfg(not(test))]
static LAST_CHAT_FRAME: OnceLock<Mutex<Option<Arc<PreparedChatFrame>>>> = OnceLock::new();

#[cfg(not(test))]
fn last_chat_frame_state() -> &'static Mutex<Option<Arc<PreparedChatFrame>>> {
    LAST_CHAT_FRAME.get_or_init(|| Mutex::new(None))
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
    pane: copy_selection::CopySelectionPane,
    data: CopyViewportData,
    scroll: usize,
    visible_end: usize,
    content_area: Rect,
    left_margins: Vec<u16>,
}

#[derive(Clone, Default)]
struct CopyViewportSnapshots {
    chat: Option<CopyViewportSnapshot>,
    side: Option<CopyViewportSnapshot>,
    input: Option<CopyViewportSnapshot>,
}

#[cfg(not(test))]
static LAST_COPY_VIEWPORT: OnceLock<Mutex<CopyViewportSnapshots>> = OnceLock::new();

#[cfg(not(test))]
fn copy_viewport_state() -> &'static Mutex<CopyViewportSnapshots> {
    LAST_COPY_VIEWPORT.get_or_init(|| Mutex::new(CopyViewportSnapshots::default()))
}

fn copy_snapshot_slot_mut(
    snapshots: &mut CopyViewportSnapshots,
    pane: copy_selection::CopySelectionPane,
) -> &mut Option<CopyViewportSnapshot> {
    match pane {
        copy_selection::CopySelectionPane::Chat => &mut snapshots.chat,
        copy_selection::CopySelectionPane::SidePane => &mut snapshots.side,
        copy_selection::CopySelectionPane::Input => &mut snapshots.input,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Viewport snapshot helpers carry explicit render state to avoid hidden globals in call sites"
)]
fn record_copy_pane_snapshot(
    pane: copy_selection::CopySelectionPane,
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

/// Record a copy-selection snapshot for the prompt composer (input box).
/// Called from `draw_input` each frame with the composer's wrapped rows so a
/// mouse drag over the text being typed selects and copies it, exactly like
/// the chat transcript. Raw lines are the logical `\n`-separated
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
        copy_selection::CopySelectionPane::Input,
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

/// How close (in rows) the mouse must be to the top/bottom edge of a scrollable
/// pane before a drag autoscrolls it. Using a zone rather than a single row
/// avoids the "stuck at the edge" feel where a fast drag overshoots the pane
/// boundary and the autoscroll stops firing; the extra rows give the handler
/// pulling in more transcript, instead of requiring the cursor to land exactly
/// on the boundary row. Scales gently with pane height and is capped so small
/// panes keep a usable middle region.
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
    let max_scroll = total_lines.saturating_sub(visible_height);
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
