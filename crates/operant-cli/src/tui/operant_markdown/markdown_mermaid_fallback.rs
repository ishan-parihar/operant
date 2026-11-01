// Vendored from jcode (crates/operant-tui-markdown/src/markdown_mermaid_fallback.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805,
// then re-pointed at operant's native diagram renderer (plan W9, codex
// architecture-deepen row 5): the `native_image_protocol_available()` gate that
// used to be permanently false now opens when the engine is present, and the
// four `render_mermaid_*` entry points route through it.
//
// What changed and why:
//   * `native_image_protocol_available()` returns whether a native RENDERER
//     exists. It is still named for the image protocol because the four call
//     sites speak in image terms (the jcode contract is unchanged), but the
//     answer is now real rather than a constant `false`.
//   * `RenderResult` gains a `Diagram` arm holding the engine's rendered
//     lines plus the source they came from. The `Image` arm stays for the
//     deferred/registration apparatus, which this build never populates.
//   * Every render path that the engine refuses falls back to the ORIGINAL
//     SOURCE with the refusal reason named in it. That is codex's own contract
//     (RenderError's three variants are Unsupported / Limit / TooWide and the
//     caller degrades to source), and it is what the permanently-false gate
//     owed its users and never gave them.
//
// Scope note: the deferred/streaming/inline-image apparatus below is unchanged
// and still inert. This build has no background renderer, no image protocol,
// and no `pinned_images` producer (D3); the rows that would produce one are
// not part of this iteration.

use ratatui::prelude::*;

use crate::tui::operant_mermaid::{self as engine, Role, Span as EngineSpan};

const INLINE_IMAGE_MARKER_PREFIX: &str = "\x00IIMG:";
const INLINE_IMAGE_MARKER_SUFFIX: &str = ":END";

/// The result of rendering a block: an inline image to place, native text to
/// show in its place, or the error text to show instead.
///
/// `Image` is produced by the inline-image path; in this build the fallback
/// path only reaches `Error` or `Diagram`, so the image arm is dead here.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: the inline-image arm is unreachable without a protocol"
    )
)]
#[derive(Debug, Clone)]
pub enum RenderResult {
    Image {
        hash: u64,
        path: std::path::PathBuf,
        width: u32,
        height: u32,
    },
    /// Native terminal text for a diagram the engine accepted, with the source
    /// it rendered and the reason any wider render was refused.
    Diagram {
        lines: Vec<DiagramLine>,
        source: String,
        refused_width: Option<usize>,
    },
    Error(String),
}

/// One rendered diagram row: display text plus the semantic role of each run,
/// so the caller can style nodes, edges and labels with its own theme.
#[derive(Debug, Clone)]
pub struct DiagramLine {
    pub spans: Vec<DiagramSpan>,
}

#[derive(Debug, Clone)]
pub struct DiagramSpan {
    pub text: String,
    pub role: DiagramRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagramRole {
    Node,
    Edge,
    Text,
}

impl From<Role> for DiagramRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Node => Self::Node,
            Role::Edge => Self::Edge,
            Role::Text => Self::Text,
        }
    }
}

pub fn is_mermaid_lang(lang: &str) -> bool {
    lang.eq_ignore_ascii_case("mermaid") || lang.eq_ignore_ascii_case("mmd")
}

pub fn native_image_protocol_available() -> bool {
    // The build default: a native renderer exists. The scoped test override
    // lives in `context` and is read by the GATE, which consults this default
    // when no override is in force. Never call the context from here.
    true
}

#[cfg(test)]
pub fn with_image_protocol_override<T>(enabled: Option<bool>, f: impl FnOnce() -> T) -> T {
    super::context::with_image_protocol_override(enabled, f)
}

/// Monotonic deferred-render epoch. The fallback renderer never defers, so
/// the epoch never advances.
pub fn deferred_render_epoch() -> u64 {
    0
}

/// The single native-render seam. `max_width` is the caller's available width
/// in columns; `None` means "unbounded", which the engine treats as
/// `usize::MAX` and so only refuses on its own `RenderError::Limit` caps.
fn render_native(source: &str, max_width: Option<usize>) -> RenderResult {
    let width = max_width.unwrap_or(usize::MAX);
    match engine::render_spans(source, width) {
        Ok(lines) => {
            let lines = lines
                .into_iter()
                .map(|row| DiagramLine {
                    spans: row
                        .into_iter()
                        .map(|EngineSpan { text, role }| DiagramSpan {
                            text,
                            role: role.into(),
                        })
                        .collect(),
                })
                .collect();
            // Record whether a narrower width was refused, so the caller can
            // say *why* the diagram is exactly this wide.
            let refused_width = (width > 1
                && matches!(
                    engine::render_spans(source, width - 1),
                    Err(engine::RenderError::TooWide)
                ))
            .then_some(width - 1);
            RenderResult::Diagram {
                lines,
                source: source.to_owned(),
                refused_width,
            }
        }
        // Source-with-reason fallback: the diagram's ORIGINAL text plus the
        // reason it could not be drawn. Never a blank or a partial drawing, and
        // never the reason alone: the source is the only thing the operator can
        // edit, so it has to survive the refusal verbatim.
        Err(reason) => RenderResult::Error(format!("{reason}\n{source}")),
    }
}

pub fn render_mermaid_deferred_with_stream_scope(
    content: &str,
    terminal_width: Option<u16>,
    _stream_sequence: u64,
) -> Option<RenderResult> {
    Some(render_native(content, terminal_width.map(usize::from)))
}

pub fn render_mermaid_deferred_with_registration(
    content: &str,
    terminal_width: Option<u16>,
    _register_active: bool,
) -> Option<RenderResult> {
    Some(render_native(content, terminal_width.map(usize::from)))
}

pub fn render_mermaid_untracked(content: &str, terminal_width: Option<u16>) -> RenderResult {
    render_native(content, terminal_width.map(usize::from))
}

pub fn render_mermaid_sized(content: &str, terminal_width: Option<u16>) -> RenderResult {
    render_native(content, terminal_width.map(usize::from))
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
        RenderResult::Diagram { lines, .. } => lines
            .into_iter()
            .map(|line| {
                Line::from(
                    line.spans
                        .into_iter()
                        .map(|span| Span::styled(span.text, style_for_role(span.role)))
                        .collect::<Vec<_>>(),
                )
            })
            .collect(),
        // A refusal carries the reason and the original source, which can span
        // several lines. Each becomes its own row so the transcript shows the
        // diagram's text as the operator wrote it rather than one wrapped blob.
        RenderResult::Error(message) => message
            .lines()
            .map(|row| Line::from(row.to_owned()))
            .collect(),
    }
}

/// The engine separates nodes, edges and labels so a diagram reads as a
/// diagram rather than a wall of undifferentiated text. Roles map to the
/// markdown palette: nodes are the body text colour, edges are dim, and
/// labels inherit the body colour.
fn style_for_role(role: DiagramRole) -> Style {
    match role {
        DiagramRole::Node => Style::default(),
        DiagramRole::Edge => Style::default().dim(),
        DiagramRole::Text => Style::default(),
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
