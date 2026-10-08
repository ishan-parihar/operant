// Vendored from jcode (crates/operant-tui-mermaid/src/{mermaid_inline.rs,
// mermaid_content.rs, mermaid_viewport.rs, mermaid_runtime.rs,
// mermaid_cache_render.rs}), MIT License, Copyright (c) 2025 Jeremy Huang.
//
// [port-decision] batch-3 terminal-image subset: ui_inline_image.rs calls into
// these 11 symbols. Definitions split by their upstream dependency surface:
// - VERBATIM: inline_image_id (mermaid_inline.rs:47 — pure std hashing),
//   InlineFitReadiness (mermaid_viewport.rs:873 — enum with doc comments),
//   inline_fit_geometry / inline_fit_geometry_upscaled +
//   inline_fit_geometry_impl (mermaid_content.rs:177,:185,:196 — pure
//   arithmetic; only dependency is get_font_size, re-rooted to the W2 fallback
//   which supplies a default cell size).
// - DEGRADED (needs ratatui_image + the image registry it drives; the user's
//   dependency call lands at batch-4): is-materialized/materialize/rediscover/
//   cached-path — the engine's own "no picker installed" behavior is the
//   degraded answer (e.g. inline_fit_readiness returns Unsupported, verbatim
//   from the engine's no-picker arm). Nothing reports a materialized image,
//   matching operant's current live behavior.
// The verbatim real definitions live in git history (restore-point commit
// 68efd95c) for batch-4's completion.

use crate::tui::vendor::style::color::rgb;
use ratatui::prelude::*;
use std::hash::Hash as _;

/// Stable content id for an inline image, derived from its media type and
/// base64 payload. No decoding is performed.
pub fn inline_image_id(media_type: &str, data_b64: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    media_type.hash(&mut hasher);
    data_b64.as_bytes().hash(&mut hasher);
    std::hash::Hasher::finish(&hasher)
}

/// Materialize an inline image payload. Degraded (no registry yet): `None`.
#[must_use]
pub fn materialize_inline_image(_media_type: &str, _data_b64: &str) -> Option<(u64, u32, u32)> {
    None
}

/// Cheap presence probe: is this inline image already registered in the
/// in-memory render cache? Used by the per-frame draw path so steady-state
/// scrolling never touches the multi-megabyte payload.
#[must_use]
pub fn inline_image_is_materialized(_id: u64) -> bool {
    false
}

/// [`materialize_inline_image`] for callers that already know the stable
/// content id, skipping the full-payload hash.
#[must_use]
pub fn materialize_inline_image_by_id(
    _id: u64,
    _media_type: &str,
    _data_b64: &str,
) -> Option<(u64, u32, u32)> {
    None
}

/// Re-materialize an inline image whose registry entry was dropped.
#[must_use]
pub fn rediscover_inline_image(_id: u64) -> Option<(u64, u32, u32)> {
    None
}

/// Re-materialize an image known to be on the filesystem but no longer
/// referable by the registry (mermaid_runtime.rs:635).
#[must_use]
pub fn rediscover_external_image(_hash: u64) -> Option<(u64, u32, u32)> {
    None
}

/// The engine's cached-path lookup (mermaid_cache_render.rs:724).
#[must_use]
pub fn get_cached_path(_hash: u64) -> Option<std::path::PathBuf> {
    None
}

// ---- inline-fit geometry (verbatim, mermaid_content.rs:177-243) ----

pub const INLINE_FIT_MIN_ROWS: u16 = 3;

pub fn inline_fit_geometry(width: u32, height: u32, chat_width: u16, cap_rows: u16) -> (u16, u16) {
    inline_fit_geometry_impl(width, height, chat_width, cap_rows, false)
}

/// Expanded inline geometry may grow beyond the source's native pixel size.
/// This matters especially for Mermaid renders, whose cached PNG can be smaller
/// than the terminal pane: changing only the row cap would otherwise update the
/// UI state and toast while leaving the displayed diagram exactly the same size.
pub fn inline_fit_geometry_upscaled(
    width: u32,
    height: u32,
    chat_width: u16,
    cap_rows: u16,
) -> (u16, u16) {
    inline_fit_geometry_impl(width, height, chat_width, cap_rows, true)
}

fn inline_fit_geometry_impl(
    width: u32,
    height: u32,
    chat_width: u16,
    cap_rows: u16,
    allow_upscale: bool,
) -> (u16, u16) {
    if width == 0 || height == 0 {
        return (INLINE_FIT_MIN_ROWS, chat_width.min(2));
    }
    let (cell_w, cell_h) = crate::tui::operant_app::mermaid::get_font_size().unwrap_or((8, 16));
    let cell_w = cell_w.max(1) as u32;
    let cell_h = cell_h.max(1) as u32;

    // Available width in pixels (border bar + padding take 2 cells, matching
    // the renderer's BORDER_WIDTH).
    let avail_cells = chat_width.saturating_sub(2).max(1) as u32;
    let avail_px = avail_cells * cell_w;

    let cap_rows_u32 = (cap_rows as u32).max(INLINE_FIT_MIN_ROWS as u32);
    let cap_px = cap_rows_u32 * cell_h;

    // Scale to fit *both* the width and the row cap, preserving aspect ratio,
    // exactly like the draw-time fit does.
    let scale_num_w = if allow_upscale {
        avail_px
    } else {
        avail_px.min(width)
    };
    let scaled_h_by_w = height.saturating_mul(scale_num_w) / width.max(1);
    let (final_w_px, final_h_px) = if scaled_h_by_w <= cap_px {
        (scale_num_w, scaled_h_by_w)
    } else {
        // Height-bound: shrink further so the height fits the cap.
        let w = width.saturating_mul(cap_px) / height.max(1);
        (w.min(avail_px).max(1), cap_px)
    };

    let rows = final_h_px
        .max(1)
        .div_ceil(cell_h)
        .max(INLINE_FIT_MIN_ROWS as u32) as u16;
    let cols = (final_w_px.max(1).div_ceil(cell_w) as u16)
        .saturating_add(2)
        .min(chat_width);
    (
        rows.min(cap_rows_u32.min(u16::MAX as u32) as u16)
            .max(INLINE_FIT_MIN_ROWS),
        cols,
    )
}

// ---- fit readiness (verbatim enum + engine's no-picker arm) ----

// Verbatim from mermaid_viewport.rs:873-883 (incl. the doc line above the enum).
/// Readiness of the stable-fit path for an inline image at a given placeholder
/// geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineFitReadiness {
    /// Protocol state (Kitty) or a pre-scaled decoded source (other protocols)
    /// already exists for this geometry.
    Ready,
    /// The scaled state is missing. Building it on the UI thread costs tens of
    /// milliseconds for a large screenshot, so callers should prewarm it
    /// off-thread via [`prewarm_inline_fit_state`].
    NeedsPrewarm,
    /// The stable-fit fast path does not apply (no picker or video export).
    Unsupported,
}

/// Check whether stable-fit state for `hash` is ready at the
/// `(target_cols, target_rows)` placeholder geometry. Degraded: with no image
/// picker installed, verbatim from the engine's own no-picker path
/// (`None => return InlineFitReadiness::Unsupported`).
#[must_use]
pub fn inline_fit_readiness(
    _hash: u64,
    _target_cols: u16,
    _target_rows: u16,
    _draw_border: bool,
) -> InlineFitReadiness {
    InlineFitReadiness::Unsupported
}

// ---- degrade: no-picker/no-materialization variants. EXCLUDED at batch-3:
// render_image_widget / render_image_widget_fit / render_image_widget_fit_stable /
// is_video_export_mode — jcode engine draw path. The ported ui_viewport.rs
// call sites gate themselves (#[cfg(any())]) at their enclosing let-if blocks
// rather than borrow fake draw bodies here.

/// Upstream mermaid_content.rs:77-96 ported nearly verbatim (only the upstream
/// `mermaid_style_hint_text()` import had nowhere to go; the [color] styling
/// constants stay flexible — their physical lines remain identical).
pub fn text_image_fallback_note_line() -> Line<'static> {
    let style = Style::default()
        .fg(rgb(140, 170, 200))
        .add_modifier(Modifier::DIM | Modifier::ITALIC);
    Line::from(vec![Span::styled(
        "  ⓘ ",
        Style::default().fg(rgb(100, 100, 100)),
    )])
    .patch_style(style)
}

/// Upstream mermaid_inline.rs:77-82 (signature verbatim). The dims cache exists
/// only after raster decode of a payload; batch-3 has no decoder, so `None`.
pub fn inline_image_dims(_media_type: &str, _data_b64: &str) -> Option<(u64, u32, u32)> {
    None
}

/// Upstream mermaid_runtime.rs:550 (signature verbatim; degraded: no native
/// protocol picker yet, and the text-image fallback flag exists only alongside
/// it — false).
pub fn uses_text_image_fallback() -> bool {
    false
}

/// Upstream mermaid_viewport.rs:940-946 (signature verbatim; degraded: no picker
/// means nothing to prewarm into — false).
pub fn prewarm_inline_fit_state(
    _hash: u64,
    _target_cols: u16,
    _target_rows: u16,
    _draw_border: bool,
) -> bool {
    false
}

/// Upstream mermaid_inline.rs:185. Registering expanded-level geometry only has
/// meaning when a fit cache exists; with no engine, this is a no-op.
pub fn register_inline_level_geometries(_hash: u64, _geometries: [(u16, u16); 3]) {}

/// Whether the video-export pipeline is active. The empty-registry answer is
/// `false` — no video recording lives in this wave.
#[inline]
pub fn is_video_export_mode() -> bool {
    false
}

// ---- degraded helpers the wrap-around renderer does callature ----
/// The epoch number when the last inline@(expand level change) happened.
/// With no inline image state (no registry), it never ticks. Returns the
/// constant `0` — the honest answer today.
#[inline]
#[must_use]
pub fn mermaid_inline_expand_epoch() -> u64 {
    0
}

/// Upstream mermaid_inline.rs:43-`: Set the epoch's countdown inbox for when
/// an inline expand is requested via the terminal image protocol path. No-op
/// here — the engine is off at batch-3.
#[inline]
pub fn set_mermaid_inline_expand_level(_hash: u64, _expand_level: u64) {}

/// Upstream `current_render_profile().preferred_aspect_per_mille` — that field
/// comes from an engine-side render-profile context that does not exist in
/// batch-3. Degraded answer: `None` — no preferred bucket means nothing ever
/// deviates the fit decision.
#[inline]
#[must_use]
pub fn current_preferred_aspect_ratio_bucket() -> Option<u16> {
    None
}

/// Set a global hook that runs at end of render (in the engine, for logging).
/// No-op here.
#[inline]
pub fn set_render_completed_hook(_hook: fn()) {}

/// Set a log hook used for warnings from the render fallback path. No-op.
#[inline]
pub fn set_log_hooks(_open_error: fn(&std::path::Path), _timing: fn(&str)) {}

/// Verbatim from mermaid_content.rs:73-75: the one central note every
/// text-image fallback (raster, Mermaid, LaTeX) shows when the terminal can't
/// render inline images. Under batch-3's degraded surface this is the note the
/// fallback paths CAN legitimately show.
pub const TERMINAL_IMAGE_FALLBACK_NOTE: &str =
    "Your terminal cannot render inline images; showing a text fallback.";
