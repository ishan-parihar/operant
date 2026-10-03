// Inline-image machinery subset of jcode's `jcode-tui-mermaid` crate, MIT
// License, Copyright (c) 2025 Jeremy Huang, ported verbatim @ 0a9dc7805.
// Truth clause: this module is the terminal-image fitting/materialization
// subset of jcode-tui-mermaid — image caches, picker/protocol detection,
// inline-image id/dims/materialization, fit geometry, and the draw paths
// (widget + stable-fit/Katty-viewport) — ported verbatim. The diagram RENDER
// engine (SVG parse, mermaid-rs-renderer rasterization, layout cache, deferred
// background render worker) is excluded per the W9 engine cut.
//! # Per-symbol upstream citations (upstream `crates/jcode-tui-mermaid/src/`)
//!
//! Caller-facing (exported through `jcode_app::mermaid`'s `pub use` block):
//! - `InlineFitReadiness` — mermaid_viewport.rs:872-883
//! - `TERMINAL_IMAGE_FALLBACK_NOTE` — mermaid_content.rs:74-75
//! - `clear_image_state` — lib.rs:1553-1569
//! - `current_preferred_aspect_ratio_bucket` — lib.rs:100-102
//! - `debug_image_state` / `ImageStateInfo` — debug.rs:95-122 (brief-named anchor)
//! - `get_cached_path` — mermaid_cache_render.rs:724-726
//! - `init_picker` — mermaid_runtime.rs:416-459 ([port-decision] exported though
//!   not in the jcode_ui caller union: it is the sole initializer of the
//!   `PICKER` static every ported definition reads; without a call the whole
//!   port runs dormant (no image protocol). Upstream callers:
//!   jcode-tui tui/session_picker.rs:2359, tui/app/debug_bench.rs:366.)
//! - `inline_fit_geometry` / `inline_fit_geometry_upscaled` /
//!   `inline_fit_geometry_impl` — mermaid_content.rs:177-244
//! - `inline_fit_readiness` — mermaid_viewport.rs:888-932
//! - `inline_image_id` / `inline_image_dims` / `materialize_inline_image` /
//!   `inline_image_is_materialized` / `materialize_inline_image_by_id` /
//!   `rediscover_inline_image` — mermaid_inline.rs:47,77,113,129,140,212
//! - `is_video_export_mode` — mermaid_runtime.rs:561-563
//! - `mermaid_inline_expand_epoch` / `set_mermaid_inline_expand_level` /
//!   `register_inline_level_geometries` — lib.rs:468,453,490
//! - `prewarm_inline_fit_state` — mermaid_viewport.rs:940-977
//! - `rediscover_external_image` — mermaid_runtime.rs:635-661
//! - `render_image_widget` / `render_image_widget_fit` /
//!   `render_image_widget_fit_inner` — mermaid_widget.rs:81-287,291-299,310-503
//! - `render_image_widget_fit_stable` — mermaid_viewport.rs:1183-1300
//! - `text_image_fallback_note_line` — mermaid_content.rs:77-85
//! - `uses_text_image_fallback` — mermaid_runtime.rs:550-552
//!
//! Leaf support pulled in (same file, verbatim): `RenderProfile` +
//! `RENDER_PROFILE_CONTEXT` (lib.rs:65-102 subset); `MermaidCache`,
//! `CachedDiagram`, `cached_width_satisfies`, `parse_cache_filename`,
//! `get_cached_diagram*`, `RENDER_CACHE_MAX`, `CACHE_WIDTH_MATCH_PERCENT`
//! (mermaid_cache_render.rs:3-330,673-726); picker/protocol chain
//! (mermaid_runtime.rs:3-391,416-485,541-563 subset); `get_font_size`
//! (mermaid_runtime.rs:724-729 — NOT re-exported: `jcode_app::mermaid` already
//! defines the W9 fallback's `get_font_size`, a re-export would collide);
//! `ImageState*`/source/fitted caches, Kitty viewport cache, evacuation
//! machinery, `MermaidDebugStats`, `LAST_RENDER`, `evict_old_cache` (lib.rs
//! 539-708,713-1006,1007-1217,1486-1569); widget helpers (mermaid_widget.rs
//! 3-68); viewport helpers incl. `KITTY_DIACRITICS` (mermaid_viewport.rs
//! 4-101,112-226,245-548,550-849,851-1170,1554-1570); log hooks
//! (lib.rs:151-179,194-202).
//!
//! [port-decision] upstream `#[cfg(feature = "renderer")]` items omitted —
//! they belong to the cut diagram engine (layout cache, `cache_path`,
//! `KittyViewportCache::remove`): `RenderProfile::cache_suffix` (lib.rs:85-89),
//! `MermaidCache::cache_path` (mermaid_cache_render.rs:241-252),
//! `KittyViewportCache::remove` (lib.rs:985-994).
//! [port-decision] upstream `#[cfg(test)]` helpers/mods omitted
//! (`mermaid_inline_extension_for_test`, inline tests, `fit_box_tests`,
//! `distinct_level_tests`, `get_cached_diagram_in_memory_for_test`,
//! `picker_init_mode_from_probe_env`).
//! [port-decision] engine-side caller surfaces NOT ported (not in the jcode_ui
//! caller union; deferred render worker / SVG engine per W9):
//! `render_image_widget_scale`, `render_image_widget_viewport`,
//! `render_image_widget_viewport_precise`, `ensure_kitty_viewport_state`,
//! `kitty_scaled_image_for_zoom`, `viewport_crop_should_scale_to_area`,
//! `invalidate_render_state`, `set_video_export_mode` (its static is ported;
//! nothing in operant enables video export yet), `get_cached_png`,
//! `register_inline_image`, `error_lines_for`, `force_test_kitty_picker`,
//! `set_log_hooks`, `set_render_completed_hook`, `set_memory_snapshot_hook`,
//! `take_terminal_image_cleanup_payload`, `render_pending_terminal_image_cleanup`,
//! `cache_stat_syscalls`, `with_preferred_aspect_ratio` and the profile-guard
//! machinery, `mermaid_source_for_hash`/`remember_mermaid_source`,
//! `bump_deferred_render_epoch`, `MermaidCacheEntry`, `ProcessMemorySnapshot`.
//! [port-decision] `remember_external_image_path` is verbatim but currently
//! unwritten in operant: the W9 fallback owns `register_external_image`
//! (a stub), so `rediscover_external_image` always probes an empty registry.
//! [`#[allow(dead_code)]`] added; a real `register_external_image` port can
//! not be exported from here because `jcode_app::mermaid` already defines the
//! fallback's symbol under that name (E0255).
//! [re-rooted imports] `crate::log_info`/`crate::log_warn`/
//! `crate::panic_payload_to_string` -> the verbatim hook-based helpers in this
//! module (lib.rs:169-202); `runtime::new_resize_protocol`,
//! `super::record_cache_stat_syscall`, `super::bounded_bookkeeping_insert`,
//! `super::widget_render::set_cell_if_visible` -> in-module items;
//! `jcode_tui_workspace::color_support::{rgb, clear_buf}` ->
//! `crate::tui::vendor::style::color::{rgb, clear_buf}`;
//! `use super::*` -> flattened local items (this is one module now).

use base64::Engine as _;
use image::DynamicImage;
use image::GenericImageView;
use ratatui::prelude::*;
use ratatui::widgets::StatefulWidget;
use ratatui_image::{
    CropOptions, Resize, ResizeEncodeRender, StatefulImage,
    picker::{Picker, ProtocolType, cap_parser::Parser},
    protocol::{ImageSource, StatefulProtocol, StatefulProtocolType, sixel::Sixel},
};
use serde::Serialize;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::hash::{Hash as _, Hasher};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use crate::tui::vendor::style::color::rgb;

// ==== lib.rs:65-102 — render profile context ====

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct RenderProfile {
    preferred_aspect_per_mille: Option<u16>,
}

thread_local! {
    static RENDER_PROFILE_CONTEXT: Cell<RenderProfile> = Cell::new(RenderProfile::default());
}

fn current_render_profile() -> RenderProfile {
    RENDER_PROFILE_CONTEXT.with(|profile| profile.get())
}

pub fn current_preferred_aspect_ratio_bucket() -> Option<u16> {
    current_render_profile().preferred_aspect_per_mille
}

// ==== lib.rs:410-444 — global statics ====

/// When true, mermaid placeholders include image hashes even without a
/// terminal image protocol (used by the video export pipeline so it can
/// embed cached PNGs into the SVG frames).
static VIDEO_EXPORT_MODE: AtomicBool = AtomicBool::new(false);

/// Global picker for terminal capability detection
/// Initialized once on first use
static PICKER: OnceLock<Option<Picker>> = OnceLock::new();

/// Whether the current tmux client can parse and retain sixel images itself.
static TMUX_NATIVE_SIXEL: OnceLock<bool> = OnceLock::new();

/// Track whether cache eviction has run
static CACHE_EVICTED: OnceLock<()> = OnceLock::new();

/// Cache for rendered mermaid diagrams
static RENDER_CACHE: LazyLock<Mutex<MermaidCache>> =
    LazyLock::new(|| Mutex::new(MermaidCache::new()));

/// Placeholder `(rows, cols)` for each inline expand level.
type InlineLevelGeometry = [(u16, u16); 3];

static MERMAID_INLINE_EXPAND_LEVEL: LazyLock<Mutex<HashMap<u64, u8>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static MERMAID_INLINE_EXPAND_EPOCH: AtomicU64 = AtomicU64::new(0);
static MERMAID_INLINE_LEVEL_GEOMETRY: LazyLock<Mutex<HashMap<u64, InlineLevelGeometry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ==== lib.rs:438-494 — inline expand levels ====

pub fn set_mermaid_inline_expand_level(hash: u64, level: u8) {
    if let Ok(mut levels) = MERMAID_INLINE_EXPAND_LEVEL.lock() {
        let previous = levels.get(&hash).copied().unwrap_or(0);
        let level = level.min(2);
        if level == 0 {
            levels.remove(&hash);
        } else {
            levels.insert(hash, level);
        }
        if previous != level {
            MERMAID_INLINE_EXPAND_EPOCH.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn mermaid_inline_expand_epoch() -> u64 {
    MERMAID_INLINE_EXPAND_EPOCH.load(Ordering::Relaxed)
}

pub fn register_inline_level_geometries(hash: u64, geometries: [(u16, u16); 3]) {
    if let Ok(mut all) = MERMAID_INLINE_LEVEL_GEOMETRY.lock() {
        all.insert(hash, geometries);
    }
}

// ==== lib.rs:151-202 — log hooks (no-op until the W9 seam wires them) ====

static LOG_INFO_HOOK: OnceLock<fn(&str)> = OnceLock::new();
static LOG_WARN_HOOK: OnceLock<fn(&str)> = OnceLock::new();

pub(crate) fn log_info(message: &str) {
    if let Some(hook) = LOG_INFO_HOOK.get() {
        hook(message);
    }
}

pub(crate) fn log_warn(message: &str) {
    if let Some(hook) = LOG_WARN_HOOK.get() {
        hook(message);
    }
}

pub(crate) fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

// ==== lib.rs:1219-1284 — debug stats ====

/// Debug stats for mermaid rendering
#[derive(Debug, Clone, Default, Serialize)]
pub struct MermaidDebugStats {
    pub total_requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    /// Layout-tier cache hits: the PNG cache missed but the computed layout
    /// was reused, so only SVG+PNG rasterization ran (no parse/compute_layout).
    pub layout_cache_hits: u64,
    /// Layout-tier cache misses: full parse + compute_layout executed.
    pub layout_cache_misses: u64,
    pub deferred_enqueued: u64,
    pub deferred_deduped: u64,
    pub deferred_superseded: u64,
    pub deferred_worker_renders: u64,
    pub deferred_worker_skips: u64,
    pub deferred_epoch_bumps: u64,
    pub render_success: u64,
    pub render_errors: u64,
    pub last_render_ms: Option<f32>,
    pub last_parse_ms: Option<f32>,
    pub last_layout_ms: Option<f32>,
    pub last_svg_ms: Option<f32>,
    pub last_png_ms: Option<f32>,
    pub last_error: Option<String>,
    pub last_hash: Option<String>,
    pub last_nodes: Option<usize>,
    pub last_edges: Option<usize>,
    pub last_content_len: Option<usize>,
    pub image_state_hits: u64,
    pub image_state_misses: u64,
    pub skipped_renders: u64,
    pub fit_state_reuse_hits: u64,
    pub fit_protocol_rebuilds: u64,
    pub viewport_state_reuse_hits: u64,
    pub viewport_protocol_rebuilds: u64,
    pub clear_operations: u64,
    pub last_image_render_ms: Option<f32>,
    pub cache_entries: usize,
    pub cache_dir: Option<String>,
    pub protocol: Option<String>,
    pub render_size_backend: &'static str,
    pub last_png_width: Option<u32>,
    pub last_png_height: Option<u32>,
    pub last_measured_width: Option<u32>,
    pub last_measured_height: Option<u32>,
    pub last_viewbox_width: Option<u32>,
    pub last_viewbox_height: Option<u32>,
    pub last_target_width: Option<u32>,
    pub last_target_height: Option<u32>,
    pub deferred_pending: usize,
    pub deferred_epoch: u64,
    /// Layout-tier cache resident entries (see `layout_cache_hits`).
    pub layout_cache_entries: usize,
    pub layout_cache_limit: usize,
    /// Approximate resident bytes held by cached layouts.
    pub layout_cache_approx_bytes: u64,
}

#[derive(Debug, Clone, Default)]
struct MermaidDebugState {
    stats: MermaidDebugStats,
}

static MERMAID_DEBUG: LazyLock<Mutex<MermaidDebugState>> =
    LazyLock::new(|| Mutex::new(MermaidDebugState::default()));

// ==== lib.rs:539-548 — cache stat syscall counter ====

/// Count of `path.exists()`/`read_dir` filesystem stat syscalls performed by
/// the render-cache lookup paths. The inline-image scroll hot path used to pay
/// one of these per visible (and prefetched) image *per frame*, so this counter
/// makes that cost observable to the image-scroll benchmark and regression tests.
static CACHE_STAT_SYSCALLS: AtomicU64 = AtomicU64::new(0);

#[inline]
pub(crate) fn record_cache_stat_syscall() {
    CACHE_STAT_SYSCALLS.fetch_add(1, Ordering::Relaxed);
}

// ==== lib.rs:583-601, 626-688 — image cache constants, statics, kitty queue ====

/// Maximum number of StatefulProtocol entries to keep in IMAGE_STATE.
/// Each entry holds the full decoded+encoded image data and can consume
/// several MB of RAM (e.g. a 1440×1080 RGBA image ≈ 6 MB, plus protocol
/// encoding overhead).  Keeping this bounded prevents unbounded memory
/// growth over long sessions with many diagrams.
///
/// Sized to comfortably cover the viewport plus the look-ahead prefetch band
/// (see `ui_inline_image::prefetch`) so scrolling back through a transcript of
/// inline screenshots reuses warm protocol state instead of re-encoding.
const IMAGE_STATE_MAX: usize = 24;
/// Approximate source-pixel budget for `IMAGE_STATE`.
///
/// `ratatui-image::StatefulProtocol` retains the original decoded image in
/// addition to protocol-specific encoded data. A count-only cap therefore lets
/// a handful of 4K screenshots pin hundreds of MiB. The source-pixel budget is
/// intentionally conservative; encoded buffers are extra, so keeping decoded
/// sources below this line keeps the real cache working set bounded too.
const IMAGE_STATE_MAX_SOURCE_BYTES: usize = 48 * 1024 * 1024;

/// Maximum number of Kitty virtual-placement state entries to keep.
///
/// Unlike `IMAGE_STATE` (which holds full decoded+encoded `StatefulProtocol`
/// data), a steady-state `KittyViewportState` entry is tiny: once its one-shot
/// `pending_transmit` payload has been drawn it is just metadata (a path, a u32
/// id, and a few dimensions, ~100 bytes). The terminal itself retains the
/// transmitted pixels, so keeping the id->geometry mapping warm lets a scroll
/// back over a long transcript of screenshots re-address the existing image with
/// unicode placeholders instead of paying a synchronous decode + scale + base64
/// re-transmit. We therefore size this far larger than `IMAGE_STATE_MAX` so the
/// scroll working set for a screenshot-heavy session stays warm; the memory cost
/// of the extra metadata entries is negligible.
const KITTY_VIEWPORT_STATE_MAX: usize = 256;
/// Maximum encoded Kitty transmissions retained before their first draw.
///
/// A prewarmed state temporarily owns a base64 PNG escape payload. Count-only
/// eviction is not sufficient because a handful of high-resolution images can
/// otherwise retain hundreds of MiB while they are still off screen. Once a
/// state is drawn this drops to zero and only its tiny terminal id metadata
/// remains. As with the other byte-bounded caches, one oversized newest entry is
/// retained so a single large image can still make forward progress.
const KITTY_VIEWPORT_PENDING_MAX_BYTES: usize = 32 * 1024 * 1024;

/// Image state cache - holds StatefulProtocol for each rendered image
/// Keyed by content hash; source_path guards prevent stale reuse when
/// a higher-resolution PNG for the same hash replaces the old one.
static IMAGE_STATE: LazyLock<Mutex<ImageStateCache>> =
    LazyLock::new(|| Mutex::new(ImageStateCache::new()));

/// Cache decoded source images to avoid reloading from disk on every pan
static SOURCE_CACHE: LazyLock<Mutex<SourceImageCache>> =
    LazyLock::new(|| Mutex::new(SourceImageCache::new()));

/// Cache images pre-scaled to their inline placeholder geometry. Non-Kitty
/// protocols cannot re-address a terminal-retained image like Kitty can, but
/// keeping this bounded decoded source lets scroll-only updates crop the visible
/// rows without re-decoding or re-scaling the complete screenshot.
static FITTED_SOURCE_CACHE: LazyLock<Mutex<FittedSourceCache>> =
    LazyLock::new(|| Mutex::new(FittedSourceCache::new()));

/// Cache Kitty-specific viewport state so scroll-only updates can reuse the
/// same transmitted image data and adjust placeholders instead of rebuilding a
/// fresh cropped protocol payload on every tick.
static KITTY_VIEWPORT_STATE: LazyLock<Mutex<KittyViewportCache>> =
    LazyLock::new(|| Mutex::new(KittyViewportCache::new()));

/// Terminal image ids whose Kitty allocations should be deleted on the next
/// image draw. Eviction can happen while only cache locks are available, so the
/// actual escape sequence is deferred until a render buffer is being built.
static KITTY_PENDING_DELETE_IDS: LazyLock<Mutex<VecDeque<u32>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

/// Monotonic process-local Kitty image id allocator. Folding a 64-bit content
/// hash into 32 bits allowed unrelated images to alias and overwrite each other.
static NEXT_KITTY_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

fn queue_kitty_delete(unique_id: u32) {
    if unique_id == 0 {
        return;
    }
    if let Ok(mut pending) = KITTY_PENDING_DELETE_IDS.lock()
        && !pending.contains(&unique_id)
    {
        pending.push_back(unique_id);
    }
}

fn queue_kitty_delete_if_transmitted(state: &KittyViewportState) {
    // A pending transmit has never reached the terminal, so there is no terminal
    // allocation to reclaim. Skipping it also keeps the deferred-delete queue
    // bounded naturally during large offscreen prewarm bursts.
    if state.pending_transmit.is_none() {
        queue_kitty_delete(state.unique_id);
    }
}

fn take_kitty_delete_ids() -> Vec<u32> {
    KITTY_PENDING_DELETE_IDS
        .lock()
        .map(|mut pending| pending.drain(..).collect())
        .unwrap_or_default()
}

/// Last render state for skip-redundant-render optimization
static LAST_RENDER: LazyLock<Mutex<HashMap<u64, LastRenderState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Cap for the LAST_RENDER / RENDER_ERRORS bookkeeping maps. Entries are tiny,
/// but both maps are keyed by content hash and previously grew without bound
/// over a long session. On overflow the map is simply cleared: LAST_RENDER only
/// powers a skipped-render stat, and a cleared RENDER_ERRORS entry just means
/// one failed diagram re-renders (and re-fail) once more.
pub(crate) const RENDER_BOOKKEEPING_MAX: usize = 1024;

/// Insert into a bounded bookkeeping map, clearing it if it would exceed
/// [`RENDER_BOOKKEEPING_MAX`] distinct keys.
pub(crate) fn bounded_bookkeeping_insert<V>(map: &mut HashMap<u64, V>, hash: u64, value: V) {
    if map.len() >= RENDER_BOOKKEEPING_MAX && !map.contains_key(&hash) {
        map.clear();
    }
    map.insert(hash, value);
}

// ==== lib.rs:713-828 — ImageState, ViewportState, ResizeMode ====

/// State for a rendered image
struct ImageState {
    protocol: StatefulProtocol,
    source_path: PathBuf,
    /// Exact bytes retained by the protocol's decoded source image before any
    /// protocol-specific encoding overhead.
    source_bytes: usize,
    /// The area this was last rendered to (for change detection)
    last_area: Option<Rect>,
    /// Resize mode locked at creation time (prevents flickering on scroll)
    resize_mode: ResizeMode,
    /// Whether the last render clipped from the top (to show bottom portion)
    last_crop_top: bool,
    /// Last viewport parameters (for pan/scroll)
    last_viewport: Option<ViewportState>,
}

/// LRU-bounded cache for ImageState entries.
struct ImageStateCache {
    entries: HashMap<u64, ImageState>,
    order: VecDeque<u64>,
    total_source_bytes: usize,
}

impl ImageStateCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_source_bytes: 0,
        }
    }

    fn touch(&mut self, hash: u64) {
        if let Some(pos) = self.order.iter().position(|h| *h == hash) {
            self.order.remove(pos);
        }
        self.order.push_back(hash);
    }

    fn get_mut(&mut self, hash: u64) -> Option<&mut ImageState> {
        if self.entries.contains_key(&hash) {
            self.touch(hash);
            self.entries.get_mut(&hash)
        } else {
            None
        }
    }

    fn get(&self, hash: &u64) -> Option<&ImageState> {
        self.entries.get(hash)
    }

    fn insert(&mut self, hash: u64, state: ImageState) {
        if let Some(old) = self.entries.remove(&hash) {
            self.total_source_bytes = self.total_source_bytes.saturating_sub(old.source_bytes);
            if let Some(pos) = self.order.iter().position(|h| *h == hash) {
                self.order.remove(pos);
            }
        }
        self.total_source_bytes = self.total_source_bytes.saturating_add(state.source_bytes);
        self.entries.insert(hash, state);
        self.order.push_back(hash);
        // Keep one oversized image rather than immediately evicting the state
        // that the caller is about to draw and entering a decode/rebuild loop.
        while (self.order.len() > IMAGE_STATE_MAX
            || self.total_source_bytes > IMAGE_STATE_MAX_SOURCE_BYTES)
            && self.order.len() > 1
        {
            if let Some(old) = self.order.pop_front()
                && let Some(old_state) = self.entries.remove(&old)
            {
                self.total_source_bytes = self
                    .total_source_bytes
                    .saturating_sub(old_state.source_bytes);
            }
        }
    }

    fn remove(&mut self, hash: &u64) {
        if let Some(old) = self.entries.remove(hash) {
            self.total_source_bytes = self.total_source_bytes.saturating_sub(old.source_bytes);
        }
        if let Some(pos) = self.order.iter().position(|h| h == hash) {
            self.order.remove(pos);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.total_source_bytes = 0;
    }

    fn iter(&self) -> impl Iterator<Item = (&u64, &ImageState)> {
        self.entries.iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ViewportState {
    scroll_x_px: u32,
    scroll_y_px: u32,
    view_w_px: u32,
    view_h_px: u32,
}

/// Resize mode for images - locked at creation time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeMode {
    Fit,
    Scale,
    Crop,
    Viewport,
    FitViewport,
}

/// Track what was rendered last frame for skip-redundant optimization
#[derive(Debug, Clone, PartialEq, Eq)]
struct LastRenderState {
    area: Rect,
    crop_top: bool,
    resize_mode: ResizeMode,
}

// ==== lib.rs:835-1006 — source/fitted cache types + kitty viewport cache ====

/// Cache decoded source images for fast viewport cropping.
///
/// Sized to cover the viewport plus the inline-image look-ahead prefetch band
/// so scrolling back over recently seen screenshots reuses the decoded pixels
/// instead of re-opening and re-decoding the cached PNG from disk.
const SOURCE_CACHE_MAX: usize = 16;
/// Exact decoded-byte budget for source images used by viewport and fit paths.
/// Count-only bounding is unsafe for heterogeneous images: sixteen 4K RGBA
/// screenshots are already roughly 500 MiB before allocator overhead.
const SOURCE_CACHE_MAX_BYTES: usize = 48 * 1024 * 1024;

/// Pre-scaled sources are normally much smaller than their originals because
/// they are bounded by the inline placeholder. Keep a modest working set for
/// back-scrolling while preventing terminal resizes from accumulating variants.
const FITTED_SOURCE_CACHE_MAX: usize = 16;
const FITTED_SOURCE_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

struct SourceImageEntry {
    path: PathBuf,
    image: Arc<DynamicImage>,
    decoded_bytes: usize,
}

struct SourceImageCache {
    order: VecDeque<u64>,
    entries: HashMap<u64, SourceImageEntry>,
    total_decoded_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FittedSourceKey {
    hash: u64,
    target_cols: u16,
    target_rows: u16,
    font_size: (u16, u16),
}

struct FittedSourceEntry {
    source_path: PathBuf,
    image: Arc<DynamicImage>,
    decoded_bytes: usize,
}

struct FittedSourceCache {
    order: VecDeque<FittedSourceKey>,
    entries: HashMap<FittedSourceKey, FittedSourceEntry>,
    total_decoded_bytes: usize,
}

struct KittyViewportState {
    source_path: PathBuf,
    zoom_percent: u8,
    font_size: (u16, u16),
    unique_id: u32,
    full_cols: u16,
    full_rows: u16,
    pending_transmit: Option<String>,
    /// Exact bytes currently held by `pending_transmit`. Stored explicitly so
    /// cache accounting remains cheap and tests can exercise budget eviction
    /// without allocating multi-megabyte strings.
    pending_transmit_bytes: usize,
    /// `Some((cols, rows))` when this entry was built by the inline fit path
    /// (image pre-scaled to fit a placeholder region); `None` for the zoomable
    /// diagram viewport path. Keeps the two users of this cache from
    /// mistaking each other's transmitted pixels.
    fit_target: Option<(u16, u16)>,
}

struct KittyViewportCache {
    entries: HashMap<u64, KittyViewportState>,
    /// Monotonic recency stamps make the per-frame hit path O(1). Finding the
    /// oldest entry is O(n) only during insertion/eviction, where PNG scaling
    /// and encoding dominate and the cache is capped at 256 entries.
    recency: HashMap<u64, u64>,
    clock: u64,
    total_pending_transmit_bytes: usize,
}

impl KittyViewportCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            recency: HashMap::new(),
            clock: 0,
            total_pending_transmit_bytes: 0,
        }
    }

    fn touch(&mut self, hash: u64) {
        self.clock = self.clock.saturating_add(1);
        self.recency.insert(hash, self.clock);
    }

    fn oldest_hash(&self) -> Option<u64> {
        self.recency
            .iter()
            .min_by_key(|(_, stamp)| *stamp)
            .map(|(hash, _)| *hash)
    }

    fn get_mut(&mut self, hash: u64) -> Option<&mut KittyViewportState> {
        if self.entries.contains_key(&hash) {
            self.touch(hash);
            self.entries.get_mut(&hash)
        } else {
            None
        }
    }

    fn insert(&mut self, hash: u64, state: KittyViewportState) {
        let pending_bytes = state.pending_transmit_bytes;
        if let std::collections::hash_map::Entry::Occupied(mut entry) = self.entries.entry(hash) {
            let old = entry.insert(state);
            self.total_pending_transmit_bytes = self
                .total_pending_transmit_bytes
                .saturating_sub(old.pending_transmit_bytes)
                .saturating_add(pending_bytes);
            if entry.get().unique_id != old.unique_id {
                queue_kitty_delete_if_transmitted(&old);
            }
            self.touch(hash);
        } else {
            self.entries.insert(hash, state);
            self.total_pending_transmit_bytes = self
                .total_pending_transmit_bytes
                .saturating_add(pending_bytes);
            self.touch(hash);
        }
        while (self.entries.len() > KITTY_VIEWPORT_STATE_MAX
            || self.total_pending_transmit_bytes > KITTY_VIEWPORT_PENDING_MAX_BYTES)
            && self.entries.len() > 1
        {
            if let Some(old) = self.oldest_hash()
                && let Some(old_state) = self.entries.remove(&old)
            {
                self.recency.remove(&old);
                self.total_pending_transmit_bytes = self
                    .total_pending_transmit_bytes
                    .saturating_sub(old_state.pending_transmit_bytes);
                queue_kitty_delete_if_transmitted(&old_state);
            }
        }
    }

    fn take_pending_transmit(&mut self, hash: u64) -> Option<(u32, Option<String>)> {
        let state = self.get_mut(hash)?;
        let unique_id = state.unique_id;
        let pending = state.pending_transmit.take();
        let pending_bytes = std::mem::take(&mut state.pending_transmit_bytes);
        self.total_pending_transmit_bytes = self
            .total_pending_transmit_bytes
            .saturating_sub(pending_bytes);
        Some((unique_id, pending))
    }

    // [port-decision] upstream `#[cfg(feature = "renderer")] KittyViewportCache::remove`
    // (lib.rs:985-994) omitted: diagram-engine-only per the W9 cut.

    fn clear(&mut self) {
        for state in self.entries.values() {
            queue_kitty_delete_if_transmitted(state);
        }
        self.entries.clear();
        self.recency.clear();
        self.clock = 0;
        self.total_pending_transmit_bytes = 0;
    }
}

// ==== lib.rs:1007-1209 — source/fitted cache impls ====

impl SourceImageCache {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            entries: HashMap::new(),
            total_decoded_bytes: 0,
        }
    }

    fn touch(&mut self, hash: u64) {
        if let Some(pos) = self.order.iter().position(|h| *h == hash) {
            self.order.remove(pos);
        }
        self.order.push_back(hash);
    }

    fn get(&mut self, hash: u64, expected_path: &Path) -> Option<Arc<DynamicImage>> {
        let img = match self.entries.get(&hash) {
            Some(entry) if entry.path == expected_path => Some(entry.image.clone()),
            Some(_) => {
                self.remove(hash);
                None
            }
            None => None,
        };
        if img.is_some() {
            self.touch(hash);
        }
        img
    }

    fn insert(&mut self, hash: u64, path: PathBuf, image: DynamicImage) -> Arc<DynamicImage> {
        let decoded_bytes = image.as_bytes().len();
        self.insert_with_decoded_bytes(hash, path, image, decoded_bytes)
    }

    fn insert_with_decoded_bytes(
        &mut self,
        hash: u64,
        path: PathBuf,
        image: DynamicImage,
        decoded_bytes: usize,
    ) -> Arc<DynamicImage> {
        self.remove(hash);
        let arc = Arc::new(image);
        self.total_decoded_bytes = self.total_decoded_bytes.saturating_add(decoded_bytes);
        self.entries.insert(
            hash,
            SourceImageEntry {
                path,
                image: arc.clone(),
                decoded_bytes,
            },
        );
        self.touch(hash);
        // Preserve one oversized source so a single large image can still draw
        // without thrashing between decode and immediate eviction.
        while (self.order.len() > SOURCE_CACHE_MAX
            || self.total_decoded_bytes > SOURCE_CACHE_MAX_BYTES)
            && self.order.len() > 1
        {
            if let Some(old) = self.order.pop_front()
                && let Some(old_entry) = self.entries.remove(&old)
            {
                self.total_decoded_bytes = self
                    .total_decoded_bytes
                    .saturating_sub(old_entry.decoded_bytes);
            }
        }
        arc
    }

    fn remove(&mut self, hash: u64) {
        if let Some(old) = self.entries.remove(&hash) {
            self.total_decoded_bytes = self.total_decoded_bytes.saturating_sub(old.decoded_bytes);
        }
        if let Some(pos) = self.order.iter().position(|h| h == hash) {
            self.order.remove(pos);
        }
    }

    fn remove_if_path(&mut self, hash: u64, expected_path: &Path) {
        if self
            .entries
            .get(&hash)
            .is_some_and(|entry| entry.path == expected_path)
        {
            self.remove(hash);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.total_decoded_bytes = 0;
    }
}

impl FittedSourceCache {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            entries: HashMap::new(),
            total_decoded_bytes: 0,
        }
    }

    fn touch(&mut self, key: FittedSourceKey) {
        if let Some(pos) = self.order.iter().position(|entry| *entry == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    fn get(&mut self, key: FittedSourceKey, source_path: &Path) -> Option<Arc<DynamicImage>> {
        let image = match self.entries.get(&key) {
            Some(entry) if entry.source_path == source_path => Some(entry.image.clone()),
            Some(_) => {
                self.remove_key(key);
                None
            }
            None => None,
        };
        if image.is_some() {
            self.touch(key);
        }
        image
    }

    fn insert(
        &mut self,
        key: FittedSourceKey,
        source_path: PathBuf,
        image: DynamicImage,
    ) -> Arc<DynamicImage> {
        let decoded_bytes = image.as_bytes().len();
        self.insert_with_decoded_bytes(key, source_path, image, decoded_bytes)
    }

    fn insert_with_decoded_bytes(
        &mut self,
        key: FittedSourceKey,
        source_path: PathBuf,
        image: DynamicImage,
        decoded_bytes: usize,
    ) -> Arc<DynamicImage> {
        // Only one placeholder geometry per image is useful after a resize. Drop
        // older variants immediately rather than waiting for global LRU pressure.
        self.remove_hash(key.hash);
        let image = Arc::new(image);
        self.total_decoded_bytes = self.total_decoded_bytes.saturating_add(decoded_bytes);
        self.entries.insert(
            key,
            FittedSourceEntry {
                source_path,
                image: image.clone(),
                decoded_bytes,
            },
        );
        self.order.push_back(key);
        // Preserve one oversized fitted source so the image remains drawable
        // instead of cycling through scale -> immediate eviction on every frame.
        while (self.order.len() > FITTED_SOURCE_CACHE_MAX
            || self.total_decoded_bytes > FITTED_SOURCE_CACHE_MAX_BYTES)
            && self.order.len() > 1
        {
            if let Some(old) = self.order.pop_front()
                && let Some(entry) = self.entries.remove(&old)
            {
                self.total_decoded_bytes =
                    self.total_decoded_bytes.saturating_sub(entry.decoded_bytes);
            }
        }
        image
    }

    fn remove_key(&mut self, key: FittedSourceKey) {
        if let Some(entry) = self.entries.remove(&key) {
            self.total_decoded_bytes = self.total_decoded_bytes.saturating_sub(entry.decoded_bytes);
        }
        if let Some(pos) = self.order.iter().position(|entry| *entry == key) {
            self.order.remove(pos);
        }
    }

    fn remove_hash(&mut self, hash: u64) {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|key| key.hash == hash)
            .copied()
            .collect();
        for key in keys {
            self.remove_key(key);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.total_decoded_bytes = 0;
    }
}
