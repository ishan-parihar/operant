// messages/helpers.rs — Shared rendering primitives for message renderers.
//
// Extracted from messages/mod.rs. Low-level helpers used by the
// transcript, tool, and command renderers.

use super::*;
use crate::tui::app::TurnMetadata;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

pub(crate) fn render_user_text_with_ctx(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let truncated = truncate_user_prompt_text(text);
    render_markdown(&truncated, ctx.width.saturating_sub(3))
}

pub(crate) fn indent_line(
    mut line: Line<'static>,
    prefix: &str,
    prefix_style: Style,
    default_fg: Color,
) -> Line<'static> {
    for span in &mut line.spans {
        if span.style.fg.is_none() {
            span.style = span.style.fg(default_fg);
        }
    }

    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::styled(prefix.to_string(), prefix_style));
    spans.extend(line.spans);
    Line::from(spans)
}

pub(crate) fn indent_lines(
    lines: Vec<Line<'static>>,
    prefix: &str,
    prefix_style: Style,
    default_fg: Color,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| indent_line(line, prefix, prefix_style, default_fg))
        .collect()
}

pub(crate) fn apply_block_style(mut line: Line<'static>, width: u16) -> Line<'static> {
    let bg = TRANSCRIPT_USER_BG;
    for span in &mut line.spans {
        if span.style.fg.is_none() {
            span.style = span.style.fg(TRANSCRIPT_TEXT);
        }
        span.style = span.style.bg(bg);
    }

    let mut spans = vec![
        Span::styled("▏", Style::default().fg(ACCENT_PRIMARY).bg(bg)),
        Span::styled(" ", Style::default().bg(bg)),
    ];
    spans.extend(line.spans);

    let used = spans.iter().map(|span| span.content.width()).sum::<usize>();
    if used < width as usize {
        spans.push(Span::styled(
            " ".repeat(width as usize - used),
            Style::default().bg(bg),
        ));
    }

    Line::from(spans)
}

pub(crate) fn empty_block_line(width: u16) -> Line<'static> {
    apply_block_style(Line::from(""), width)
}
pub(crate) fn render_attachment_chip(kind: &str, label: String) -> Line<'static> {
    render_attachment_chip_colored(kind, label, ACCENT_PRIMARY, Color::Black)
}

pub(crate) fn render_file_chip(label: String) -> Line<'static> {
    // Use a steel-blue badge with white text for file injections — distinct from
    // the orange img/doc chips and readable on dark terminal backgrounds.
    render_attachment_chip_colored("file", label, Color::Rgb(51, 102, 170), Color::White)
}

fn render_attachment_chip_colored(
    kind: &str,
    label: String,
    badge_bg: Color,
    badge_fg: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {} ", kind),
            Style::default()
                .fg(badge_fg)
                .bg(badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", label),
            Style::default().fg(TRANSCRIPT_MUTED).bg(TRANSCRIPT_CHIP_BG),
        ),
    ])
}

pub(crate) fn user_metadata_line(_meta: Option<&TurnMetadata>) -> Option<Line<'static>> {
    // User prompt line has no metadata — mode/model/duration are shown on the
    // assistant footer instead (matching OpenCode's layout).
    None
}

fn truncate_user_prompt_text(text: &str) -> String {
    if text.len() <= MAX_USER_PROMPT_DISPLAY_CHARS {
        return text.to_string();
    }

    let head = &text[..TRUNCATE_USER_PROMPT_HEAD_CHARS.min(text.len())];
    let tail_start = text.len().saturating_sub(TRUNCATE_USER_PROMPT_TAIL_CHARS);
    let tail = &text[tail_start..];
    let hidden_lines = text
        .chars()
        .take(TRUNCATE_USER_PROMPT_HEAD_CHARS)
        .filter(|c| *c == '\n')
        .count()
        .saturating_sub(tail.chars().filter(|c| *c == '\n').count());

    format!("{head}\n… +{hidden_lines} lines …\n{tail}")
}
