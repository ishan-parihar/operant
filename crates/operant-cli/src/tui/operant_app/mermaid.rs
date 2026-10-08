// Vendored from jcode (crates/operant-tui/src/tui/mermaid.rs + the W9 fallback at
// crates/operant-tui-markdown/src/markdown_mermaid_fallback.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] per the W9 engine cut, operant's only mermaid path is the
// upstream cfg-fallback; operant-tui/src/tui/mermaid.rs's giant re-export of the
// engine is therefore replaced by the verbatim fallback definitions for the
// symbols the ported renderers call (is_mermaid_lang, image_protocol_available,
// get_font_size, deferred_render_epoch, notify_deferred_render_completed,
// render_mermaid_* stubs, RenderResult, result_to_lines, parse_image_placeholder,
// inline_image_placeholder_lines, parse_inline_image_placeholder,
// register_external_image — operant_markdown's ported fallback at :24-:127, which
// is private there, so the same verbatim definitions are carried here).
// [port-unresolved] engine-only symbols the renderers still call:
// current_preferred_aspect_ratio_bucket, set_mermaid_inline_expand_level,
// mermaid_inline_expand_epoch, is_video_export_mode, render_image_widget,
// render_image_widget_fit, render_image_widget_fit_stable, set_log_hooks,
// set_render_completed_hook — they exist only in the cut operant-tui-mermaid
// engine; the integrator either stubs them at the W9 seam or excises callers.
use ratatui::prelude::*;
use ratatui::style::Color;

use super::color_support::rgb;

// [port-decision] the W4.5 inline-image subset: 9/11 of the callers' symbols
// come from this module directly (verbatim or degraded where ratatui_image is
// unavailable); InlineFitReadiness itself is a verbatim port of
// mermaid_viewport.rs:868-883. See mermaid_inline.rs for the spine.
// [port-decision] the engine-arm widget/runtime surface the pinned + diagram
// panes reach for. Every signature below is the upstream one verbatim
// (operant-tui-mermaid @ 0a9dc7805); only the bodies are degraded, because the
// engine (the renderer, the picker, the PNG cache) is not linked here — the W9
// cut keeps only the cfg-fallback. Without a protocol the draw arms report the
// honest `area.height`/`false`/`None` answer the upstream no-protocol path
// produces, so the panes take their existing text-fallback branch. Sites:
// lib.rs:104 preferred_aspect_ratio_bucket, :127 with_preferred_aspect_ratio,
// :1553 clear_image_state; mermaid_runtime.rs:423 init_picker, :473
// protocol_type, :556 set_video_export_mode, :567 get_cached_png;
// mermaid_widget.rs:301 render_image_widget_scale; mermaid_viewport.rs:1303
// render_image_widget_viewport, :1328 render_image_widget_viewport_precise;
// mermaid_content.rs:301-302 the marker consts, :332
// image_widget_placeholder_markdown, :409 write_video_export_marker, :433
// image_placeholder_lines, :451 diagram_placeholder_lines;
// mermaid_cache_render.rs:601 evict_render_cache_for_content.
// [port-decision] ProtocolType is NOT ported: it is `ratatui_image::picker::ProtocolType`
// and ratatui-image is not an operant dependency. `protocol_type()` therefore
// returns the honest `None` ("no picker, no protocol"), which is the exact
// value upstream's VIDEO_EXPORT_MODE/inference-failure arm returns — and the
// call sites only ever ask `.is_some()`.
#[allow(unused_imports)] // re-export: engine-side hooks consumed at W6
pub use crate::tui::operant_app::mermaid_inline::{
    InlineFitReadiness, TERMINAL_IMAGE_FALLBACK_NOTE, current_preferred_aspect_ratio_bucket,
    get_cached_path, inline_fit_geometry, inline_fit_geometry_upscaled, inline_fit_readiness,
    inline_image_dims, inline_image_id, inline_image_is_materialized, is_video_export_mode,
    materialize_inline_image, materialize_inline_image_by_id, mermaid_inline_expand_epoch,
    prewarm_inline_fit_state, rediscover_external_image, rediscover_inline_image,
    register_inline_level_geometries, set_log_hooks, set_mermaid_inline_expand_level,
    set_render_completed_hook, text_image_fallback_note_line, uses_text_image_fallback,
};

const INLINE_IMAGE_MARKER_PREFIX: &str = "\x00IIMG:";
const INLINE_IMAGE_MARKER_SUFFIX: &str = ":END";

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum RenderResult {
    Image {
        hash: u64,
        path: std::path::PathBuf,
        width: u32,
        height: u32,
    },
    Error(String),
}

pub fn is_mermaid_lang(lang: &str) -> bool {
    lang.eq_ignore_ascii_case("mermaid") || lang.eq_ignore_ascii_case("mmd")
}

pub fn image_protocol_available() -> bool {
    false
}

pub fn native_image_protocol_available() -> bool {
    false
}

#[cfg(test)]
pub fn with_image_protocol_override<T>(_enabled: Option<bool>, f: impl FnOnce() -> T) -> T {
    f()
}

pub fn get_font_size() -> Option<(u16, u16)> {
    None
}

/// Monotonic deferred-render epoch. The fallback renderer never defers, so
/// the epoch never advances.
pub fn deferred_render_epoch() -> u64 {
    0
}

/// No-op: without the mermaid renderer there is no shared deferred epoch.
pub fn notify_deferred_render_completed() {}

pub fn render_mermaid_deferred_with_stream_scope(
    _content: &str,
    _terminal_width: Option<u16>,
    _stream_sequence: u64,
) -> Option<RenderResult> {
    Some(RenderResult::Error(
        "Mermaid rendering is disabled".to_string(),
    ))
}

pub fn render_mermaid_deferred_with_registration(
    _content: &str,
    _terminal_width: Option<u16>,
    _register_active: bool,
) -> Option<RenderResult> {
    Some(RenderResult::Error(
        "Mermaid rendering is disabled".to_string(),
    ))
}

pub fn render_mermaid_untracked(_content: &str, _terminal_width: Option<u16>) -> RenderResult {
    RenderResult::Error("Mermaid rendering is disabled".to_string())
}

pub fn render_mermaid_sized(_content: &str, _terminal_width: Option<u16>) -> RenderResult {
    RenderResult::Error("Mermaid rendering is disabled".to_string())
}

pub fn set_streaming_preview_diagram(
    _hash: u64,
    _width: u32,
    _height: u32,
    _label: Option<String>,
) {
}

pub fn result_to_lines(result: RenderResult, _max_width: Option<usize>) -> Vec<Line<'static>> {
    match result {
        RenderResult::Image { .. } => Vec::new(),
        RenderResult::Error(message) => vec![Line::from(message)],
    }
}

fn first_content_span<'a>(line: &'a Line<'_>) -> Option<&'a str> {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .find(|content| !content.trim().is_empty())
}

/// [port-decision] this marker parser is pure text (upstream
/// mermaid_content.rs:350) and does NOT depend on ratatui_image — ported
/// verbatim so the side-panel placement tests exercise the real mechanism in
/// the degraded build.
pub fn parse_image_placeholder(line: &Line<'_>) -> Option<u64> {
    let content = first_content_span(line)?;
    if content.starts_with(MERMAID_MARKER_PREFIX) && content.ends_with(MERMAID_MARKER_SUFFIX) {
        // Extract hex between prefix and suffix
        let start = MERMAID_MARKER_PREFIX.len();
        let end = content.len() - MERMAID_MARKER_SUFFIX.len();
        if end > start {
            let hex = &content[start..end];
            return u64::from_str_radix(hex, 16).ok();
        }
    }
    None
}

pub fn inline_image_placeholder_lines(hash: u64, rows: u16, cols: u16) -> Vec<Line<'static>> {
    let rows = rows.max(1);
    let mut lines = Vec::with_capacity(rows as usize);
    lines.push(Line::from(format!(
        "{INLINE_IMAGE_MARKER_PREFIX}{hash:016x}:{rows:04x}:{cols:04x}{INLINE_IMAGE_MARKER_SUFFIX}"
    )));
    lines.extend((1..rows).map(|_| Line::default()));
    lines
}

pub fn parse_inline_image_placeholder(line: &Line<'_>) -> Option<(u64, u16, u16)> {
    let content = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .find(|content| !content.trim().is_empty())?;
    let rest = content.strip_prefix(INLINE_IMAGE_MARKER_PREFIX)?;
    let rest = rest.strip_suffix(INLINE_IMAGE_MARKER_SUFFIX)?;
    let mut parts = rest.split(':');
    let hash = u64::from_str_radix(parts.next()?, 16).ok()?;
    let rows = u16::from_str_radix(parts.next()?, 16).ok()?;
    let cols = u16::from_str_radix(parts.next()?, 16).ok()?;
    (parts.next().is_none()).then_some((hash, rows, cols))
}

pub fn register_external_image(_path: &std::path::Path, _width: u32, _height: u32) -> u64 {
    0
}

pub fn install_operant_mermaid_hooks() {
    // [port-excision] upstream (tui/mermaid.rs:45-:56) wired logging, the
    // render-completed bus hook and the process-memory snapshot into the
    // mermaid engine; the fallback has no engine hooks to install, and the
    // MermaidRenderCompleted publish already happens at the ui_inline_image
    // call sites.
}
// ---- engine-arm widget/runtime surface (signatures verbatim @ 0a9dc7805) ----

/// Upstream lib.rs:71 `RenderProfile::from_preferred_aspect_ratio` buckets a
/// preferred aspect ratio to per-mille. With no render engine there is no
/// profile to bucket into, so the answer is always "no preference".
pub fn preferred_aspect_ratio_bucket(_ratio: Option<f32>) -> Option<u16> {
    None
}

/// Upstream lib.rs:127 — pushes the render profile for the duration of `f`
/// and restores it after. No profile exists, so `f` simply runs.
pub fn with_preferred_aspect_ratio<R>(_ratio: Option<f32>, f: impl FnOnce() -> R) -> R {
    f()
}

/// Upstream lib.rs:1553 — clears the image/fitted-source/active-diagram state.
/// All three are engine-owned; the fallback keeps no image state to clear.
pub fn clear_image_state() {}

/// Upstream mermaid_runtime.rs:423 — probes the terminal for an image protocol
/// and starts the picker. There is no picker and `image_protocol_available()`
/// is `false`, so there is nothing to probe.
pub fn init_picker() {}

/// Upstream mermaid_runtime.rs:473. Returns `None` because no picker exists
/// (see the `ProtocolType` port-decision above).
pub fn protocol_type() -> Option<()> {
    None
}

/// Upstream mermaid_runtime.rs:556 — flipped the renderer's video-export
/// latch. The fallback renderer has no such latch, so the flag is not kept and
/// `is_video_export_mode()` stays `false`.
pub fn set_video_export_mode(_enabled: bool) {}

/// Upstream mermaid_runtime.rs:567 — looks up a cached PNG render by content
/// hash. The fallback never renders a PNG, so nothing is ever cached and the
/// honest answer is `None`.
pub fn get_cached_png(_hash: u64) -> Option<(std::path::PathBuf, u32, u32)> {
    None
}

/// Upstream mermaid_cache_render.rs:601 — drops every cache entry derived from
/// `content`. No render cache exists.
pub fn evict_render_cache_for_content(_content: &str) {}

/// Upstream mermaid_widget.rs:301 — draws the cached image scaled to fit.
/// No cached image exists, so nothing is drawn and the caller's layout budget
/// is unchanged (`area.height`), matching upstream's no-protocol early return.
pub fn render_image_widget_scale(
    _hash: u64,
    area: Rect,
    _buf: &mut Buffer,
    _draw_border: bool,
) -> u16 {
    area.height
}

/// Upstream mermaid_viewport.rs:1303 — draws the cached image into a
/// scrollable viewport. Same degraded answer as `render_image_widget_scale`.
pub fn render_image_widget_viewport(
    _hash: u64,
    area: Rect,
    _buf: &mut Buffer,
    _scroll_x: i32,
    _scroll_y: i32,
    _zoom_percent: u8,
    _draw_border: bool,
) -> u16 {
    area.height
}

/// Upstream mermaid_viewport.rs:1328 — as above, with pixel-precise scaling
/// (upstream takes the already-converted u16 zoom; the non-precise wrapper
/// above converts the u8 it receives).
pub fn render_image_widget_viewport_precise(
    _hash: u64,
    area: Rect,
    _buf: &mut Buffer,
    _scroll_x: i32,
    _scroll_y: i32,
    _zoom_percent: u16,
    _draw_border: bool,
) -> u16 {
    area.height
}

/// Marker prefix/suffix for mermaid image placeholders
/// (mermaid_content.rs:301-:302), verbatim.
const MERMAID_MARKER_PREFIX: &str = "\x00MERMAID_IMAGE:";
const MERMAID_MARKER_SUFFIX: &str = "\x00";

/// Upstream mermaid_content.rs:332 — the markdown marker line the side-panel
/// renderer recognizes as an inline-image placeholder for a registered hash.
/// Verbatim: this is pure formatting and needs no engine.
pub fn image_widget_placeholder_markdown(hash: u64) -> String {
    format!(
        "{}{:016x}{}\n",
        MERMAID_MARKER_PREFIX, hash, MERMAID_MARKER_SUFFIX
    )
}

/// Upstream mermaid_content.rs:409 — writes the `JMERMAID:<hash>:END` marker
/// into the video-export buffer. Verbatim: the exporter matches on the marker
/// text, which is why it must survive even with no renderer.
pub fn write_video_export_marker(hash: u64, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let invisible = Style::default().fg(Color::Black).bg(Color::Black);
    // Use printable marker characters that won't break SVG XML
    let marker = format!("JMERMAID:{:016x}:END", hash);
    // Write marker on the first row
    let y = area.y;
    for (i, ch) in marker.chars().enumerate() {
        let x = area.x + i as u16;
        if x >= area.x + area.width {
            break;
        }
        buf[(x, y)].set_char(ch).set_style(invisible);
    }
}

/// Upstream mermaid_content.rs:433 `image_placeholder_lines` — the text box
/// shown when an image cannot be drawn. Verbatim; this is the degraded arm the
/// panes already render, so the strings are the ones users see.
fn image_placeholder_lines(width: u32, height: u32) -> Vec<Line<'static>> {
    let dim = Style::default().fg(rgb(100, 100, 100));
    let info = Style::default().fg(rgb(140, 170, 200));

    vec![
        Line::from(Span::styled("┌─ mermaid diagram ", dim)),
        Line::from(vec![
            Span::styled("│ ", dim),
            Span::styled(
                format!("{}×{} px (image protocols not available)", width, height),
                info,
            ),
        ]),
        Line::from(Span::styled("└─", dim)),
    ]
}

/// Upstream mermaid_content.rs:451 — public helper for the pinned diagram pane.
pub fn diagram_placeholder_lines(width: u32, height: u32) -> Vec<Line<'static>> {
    image_placeholder_lines(width, height)
}
