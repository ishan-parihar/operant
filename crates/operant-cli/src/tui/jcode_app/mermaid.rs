// Vendored from jcode (crates/jcode-tui/src/tui/mermaid.rs + the W9 fallback at
// crates/jcode-tui-markdown/src/markdown_mermaid_fallback.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] per the W9 engine cut, operant's only mermaid path is the
// upstream cfg-fallback; jcode-tui/src/tui/mermaid.rs's giant re-export of the
// engine is therefore replaced by the verbatim fallback definitions for the
// symbols the ported renderers call (is_mermaid_lang, image_protocol_available,
// get_font_size, deferred_render_epoch, notify_deferred_render_completed,
// render_mermaid_* stubs, RenderResult, result_to_lines, parse_image_placeholder,
// inline_image_placeholder_lines, parse_inline_image_placeholder,
// register_external_image — jcode_markdown's ported fallback at :24-:127, which
// is private there, so the same verbatim definitions are carried here).
// [port-unresolved] engine-only symbols the renderers still call:
// current_preferred_aspect_ratio_bucket, set_mermaid_inline_expand_level,
// mermaid_inline_expand_epoch, is_video_export_mode, render_image_widget,
// render_image_widget_fit, render_image_widget_fit_stable, set_log_hooks,
// set_render_completed_hook — they exist only in the cut jcode-tui-mermaid
// engine; the integrator either stubs them at the W9 seam or excises callers.
use ratatui::prelude::*;

// [port-decision] the W4.5 inline-image subset: 9/11 of the callers' symbols
// come from this module directly (verbatim or degraded where ratatui_image is
// unavailable); InlineFitReadiness itself is a verbatim port of
// mermaid_viewport.rs:868-883. See mermaid_inline.rs for the spine.
#[allow(unused_imports)] // re-export: engine-side hooks consumed at W6
pub use crate::tui::jcode_app::mermaid_inline::{
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

pub fn parse_image_placeholder(_line: &Line<'_>) -> Option<u64> {
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

pub fn install_jcode_mermaid_hooks() {
    // [port-excision] upstream (tui/mermaid.rs:45-:56) wired logging, the
    // render-completed bus hook and the process-memory snapshot into the
    // mermaid engine; the fallback has no engine hooks to install, and the
    // MermaidRenderCompleted publish already happens at the ui_inline_image
    // call sites.
}
