// diff_viewer/render.rs — Diff dialog rendering (file list + detail panes).
//
// Extracted from the diff_viewer.rs monolith. render_diff_dialog, pane
// renderers, and the inline word-level diff / syntax-highlight helpers.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

pub fn render_diff_dialog(state: &mut DiffViewerState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    let layout = begin_modal_buf(buf, area, 98, 32, 2, 1);
    let title = match state.diff_type {
        DiffType::GitDiff => "Review changes",
        DiffType::TurnDiff => "Changes from this turn",
    };
    render_modal_title_buf(buf, layout.header_area, title, "esc");
    let total_added: u32 = state.files.iter().map(|file| file.added).sum();
    let total_removed: u32 = state.files.iter().map(|file| file.removed).sum();
    if let Some(subtitle_area) = modal_header_line_area(layout.header_area, 1) {
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                " {} files  ·  +{} -{}  ·  {} mode",
                state.files.len(),
                total_added,
                total_removed,
                match state.diff_type {
                    DiffType::GitDiff => "git diff",
                    DiffType::TurnDiff => "turn diff",
                }
            ),
            Style::default().fg(OPERANT_MUTED),
        )]))
        .render(subtitle_area, buf);
    }

    if state.files.is_empty() {
        let empty = match state.diff_type {
            DiffType::GitDiff => " No git changes available.",
            DiffType::TurnDiff => " No changes were captured for this turn.",
        };
        Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                empty,
                Style::default()
                    .fg(OPERANT_TEXT)
                    .add_modifier(Modifier::ITALIC),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                " Use /review for the current git diff, or make an edit and reopen /changes.",
                Style::default().fg(OPERANT_MUTED),
            )]),
        ])
        .render(layout.body_area, buf);
        return;
    }

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(31),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(layout.body_area);

    let divider: Vec<Line<'static>> = (0..layout.body_area.height)
        .map(|_| Line::from(Span::styled("│", Style::default().fg(OPERANT_MUTED))))
        .collect();
    Paragraph::new(divider).render(panes[1], buf);

    render_file_list(state, panes[0], buf);
    render_diff_detail(state, panes[2], buf);
    Paragraph::new(Line::from(vec![Span::styled(
        " tab switch pane  ·  ↑↓ navigate  ·  space collapse  ·  d toggle scope",
        Style::default()
            .fg(OPERANT_MUTED)
            .add_modifier(Modifier::ITALIC),
    )]))
    .render(layout.footer_area, buf);
}

fn render_file_list(state: &DiffViewerState, area: Rect, buf: &mut Buffer) {
    let focused = state.active_pane == DiffPane::FileList;
    if area.height == 0 {
        return;
    }
    let header = Line::from(vec![
        Span::styled(
            " Files",
            Style::default()
                .fg(if focused {
                    OPERANT_ACCENT
                } else {
                    OPERANT_TEXT
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", state.files.len()),
            Style::default().fg(OPERANT_MUTED),
        ),
    ]);
    Paragraph::new(header)
        .style(Style::default().bg(OPERANT_PANEL_BG))
        .render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );

    let inner = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let max_visible = inner.height as usize;
    let start = state.selected_file.saturating_sub(max_visible / 2);
    let end = (start + max_visible).min(state.files.len());

    for (i, file) in state.files[start..end].iter().enumerate() {
        let abs_idx = start + i;
        let selected = abs_idx == state.selected_file;

        // Truncate path to fit
        let avail = inner.width.saturating_sub(10) as usize;
        let path = if file.path.len() > avail {
            format!("…{}", &file.path[file.path.len() - avail..])
        } else {
            file.path.clone()
        };

        let is_collapsed = *state.collapsed.get(abs_idx).unwrap_or(&false);
        let collapse_char = if is_collapsed { "\u{25b8}" } else { "\u{25be}" }; // ▸ / ▾
        let (stats, stats_color) = if file.binary {
            ("binary".to_string(), OPERANT_MUTED)
        } else if file.is_new_file {
            (format!("new  +{}", file.added), Color::Yellow)
        } else {
            (format!("+{} -{}", file.added, file.removed), OPERANT_MUTED)
        };

        let bg = if selected {
            OPERANT_ACCENT
        } else {
            OPERANT_PANEL_BG
        };
        let base_style = if selected {
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::White)
                .bg(bg)
        } else {
            Style::default().fg(OPERANT_TEXT).bg(bg)
        };

        let y = inner.y + i as u16;
        if y >= area.y + area.height {
            break;
        }

        let stats_text = format!(" {}", stats);
        let prefix = format!(" {} {}", collapse_char, path);
        let used = prefix.len() + stats_text.len();
        let pad = inner.width.saturating_sub(used as u16) as usize;
        let line = Line::from(vec![
            Span::styled(prefix, base_style),
            Span::styled(" ".repeat(pad), Style::default().bg(bg)),
            Span::styled(
                stats_text,
                Style::default()
                    .fg(if selected {
                        Color::Rgb(248, 220, 236)
                    } else {
                        stats_color
                    })
                    .bg(bg),
            ),
        ]);
        let row_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        Paragraph::new(line).render(row_area, buf);
    }
}

fn render_diff_detail(state: &DiffViewerState, area: Rect, buf: &mut Buffer) {
    let focused = state.active_pane == DiffPane::Detail;

    let file = match state.files.get(state.selected_file) {
        Some(f) => f,
        None => return,
    };

    let header = Line::from(vec![
        Span::styled(
            format!(" {}", file.path),
            Style::default()
                .fg(if focused {
                    OPERANT_ACCENT
                } else {
                    OPERANT_TEXT
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  +{} -{}", file.added, file.removed),
            Style::default().fg(OPERANT_MUTED),
        ),
    ]);
    Paragraph::new(header)
        .style(Style::default().bg(OPERANT_PANEL_BG))
        .render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );

    let inner = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    if *state.collapsed.get(state.selected_file).unwrap_or(&false) {
        Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                " [collapsed]  press Space to expand",
                Style::default()
                    .fg(OPERANT_MUTED)
                    .add_modifier(Modifier::ITALIC),
            )]),
        ])
        .render(inner, buf);
        return;
    }

    if file.binary {
        Paragraph::new("Binary file — no diff available")
            .style(Style::default().fg(OPERANT_MUTED))
            .render(inner, buf);
        return;
    }

    // Build lines for rendering
    let lines = build_diff_lines(file, inner.width);
    let total_lines = lines.len();
    let scroll =
        (state.detail_scroll as usize).min(total_lines.saturating_sub(inner.height as usize));
    let visible = &lines[scroll..];

    // Shrink inner width by 1 to leave room for scrollbar
    let text_width = if total_lines > inner.height as usize {
        inner.width.saturating_sub(1)
    } else {
        inner.width
    };

    for (i, line) in visible.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let y = inner.y + i as u16;
        let row_area = Rect {
            x: inner.x,
            y,
            width: text_width,
            height: 1,
        };
        Paragraph::new(line.clone()).render(row_area, buf);
    }

    // Simple scrollbar on the rightmost column of inner
    if total_lines > inner.height as usize && inner.width > 1 {
        let bar_x = inner.x + inner.width - 1;
        let bar_h = inner.height as usize;
        // Thumb size proportional to visible fraction, minimum 1
        let thumb_size = ((bar_h * bar_h) / total_lines).max(1).min(bar_h);
        // Thumb position
        let scroll_range = total_lines.saturating_sub(bar_h);
        let thumb_top = if scroll_range > 0 {
            (scroll * (bar_h.saturating_sub(thumb_size))) / scroll_range
        } else {
            0
        };

        for row in 0..bar_h {
            let y = inner.y + row as u16;
            let ch = if row == 0 {
                '\u{25b2}' // ▲
            } else if row == bar_h - 1 {
                '\u{25bc}' // ▼
            } else if row > thumb_top && row < thumb_top + thumb_size + 1 {
                '\u{2588}' // █ (thumb)
            } else {
                '\u{2502}' // │ (track)
            };
            let cell_area = Rect {
                x: bar_x,
                y,
                width: 1,
                height: 1,
            };
            Paragraph::new(Line::from(Span::styled(
                ch.to_string(),
                Style::default().fg(OPERANT_MUTED),
            )))
            .render(cell_area, buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Inline word-level diff helpers
// ---------------------------------------------------------------------------

/// Format a 10-char line-number gutter.
pub(crate) fn format_gutter(old_no: Option<u32>, new_no: Option<u32>) -> String {
    match (old_no, new_no) {
        (Some(o), Some(n)) => format!("{:>4} {:>4} ", o, n),
        (Some(o), None) => format!("{:>4}      ", o),
        (None, Some(n)) => format!("     {:>4} ", n),
        (None, None) => "          ".to_string(),
    }
}

/// Truncate a list of owned spans so the total character count ≤ `max_chars`.
pub(crate) fn truncate_spans_to_width(
    spans: Vec<Span<'static>>,
    max_chars: usize,
) -> Vec<Span<'static>> {
    let mut remaining = max_chars;
    let mut result = Vec::new();
    for span in spans {
        if remaining == 0 {
            break;
        }
        let char_count: usize = span.content.chars().count();
        if char_count <= remaining {
            remaining -= char_count;
            result.push(span);
        } else {
            let truncated: String = span.content.chars().take(remaining).collect();
            remaining = 0;
            result.push(Span::styled(truncated, span.style));
        }
    }
    result
}

/// Compute word-level inline diff spans for an adjacent (removed, added) line pair.
/// Returns `(old_spans, new_spans)` where changed words have a highlighted background.
pub(crate) fn build_inline_diff_spans(
    old: &str,
    new: &str,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_words(old, new);
    let mut old_spans: Vec<Span<'static>> = Vec::new();
    let mut new_spans: Vec<Span<'static>> = Vec::new();

    for change in diff.iter_all_changes() {
        let s: String = change.to_string();
        match change.tag() {
            ChangeTag::Equal => {
                old_spans.push(Span::styled(s.clone(), Style::default().fg(OPERANT_TEXT)));
                new_spans.push(Span::styled(s, Style::default().fg(OPERANT_TEXT)));
            }
            ChangeTag::Delete => {
                old_spans.push(Span::styled(
                    s,
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Rgb(150, 30, 30)),
                ));
            }
            ChangeTag::Insert => {
                new_spans.push(Span::styled(
                    s,
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Rgb(30, 130, 30)),
                ));
            }
        }
    }

    (old_spans, new_spans)
}

/// Highlight a line of source code using syntect, returning ratatui Spans.
/// Falls back to plain styling if the language is not recognised.
fn highlight_code_line(line: &str, path: &str, base_style: Style) -> Vec<Span<'static>> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let ss = &*SYNTAX_SET;
    let ts = &*THEME_SET;

    let syntax = if let Some(s) = ss.find_syntax_by_extension(ext) {
        s
    } else {
        return vec![Span::styled(line.to_string(), base_style)];
    };

    let theme = ts
        .themes
        .get("base16-ocean.dark")
        .or_else(|| ts.themes.values().next());

    let theme = match theme {
        Some(t) => t,
        None => return vec![Span::styled(line.to_string(), base_style)],
    };

    let mut h = HighlightLines::new(syntax, theme);
    match h.highlight_line(line, ss) {
        Ok(ranges) => {
            let mut result = Vec::new();
            for (style, text) in ranges {
                if text.is_empty() {
                    continue;
                }
                // Blend syntect foreground with the diff color (added=green, removed=red)
                let fg = style.foreground;
                // Only apply syntect color when it's not a "default" near-white color
                let is_default = fg.r > 200 && fg.g > 200 && fg.b > 200;
                let color = if is_default {
                    // Use the diff marker color (passed in base_style)
                    base_style.fg.unwrap_or(Color::White)
                } else {
                    Color::Rgb(fg.r, fg.g, fg.b)
                };
                result.push(Span::styled(text.to_string(), Style::default().fg(color)));
            }
            if result.is_empty() {
                vec![Span::styled(line.to_string(), base_style)]
            } else {
                result
            }
        }
        Err(_) => vec![Span::styled(line.to_string(), base_style)],
    }
}

pub(crate) fn build_diff_lines(file: &FileDiffStats, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Gutter = 10 chars ("dddd dddd "), prefix marker = 3 chars ("+  " etc.)
    let gutter_width: usize = 10;
    let prefix_width: usize = 3;
    let avail = (width as usize).saturating_sub(gutter_width + prefix_width);

    for hunk in &file.hunks {
        let hunk_lines = &hunk.lines;
        let mut i = 0;
        while i < hunk_lines.len() {
            let diff_line = &hunk_lines[i];

            // Detect adjacent Removed → Added pair for inline word-level diff
            if diff_line.kind == DiffLineKind::Removed {
                if let Some(next_line) = hunk_lines.get(i + 1) {
                    if next_line.kind == DiffLineKind::Added {
                        let (old_spans, new_spans) =
                            build_inline_diff_spans(&diff_line.content, &next_line.content);

                        let mut removed_row = vec![
                            Span::styled(
                                format_gutter(diff_line.old_line_no, None),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled("-  ", Style::default().fg(Color::Red)),
                        ];
                        removed_row.extend(truncate_spans_to_width(old_spans, avail));
                        lines.push(Line::from(removed_row));

                        let mut added_row = vec![
                            Span::styled(
                                format_gutter(None, next_line.new_line_no),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled("+  ", Style::default().fg(Color::Green)),
                        ];
                        added_row.extend(truncate_spans_to_width(new_spans, avail));
                        lines.push(Line::from(added_row));

                        i += 2;
                        continue;
                    }
                }
            }

            // Standard single-line rendering
            let (marker, content_style) = match diff_line.kind {
                DiffLineKind::Header => (
                    Span::styled("@@ ", Style::default().fg(Color::Cyan)),
                    Style::default().fg(Color::Cyan),
                ),
                DiffLineKind::Added => (
                    Span::styled("+  ", Style::default().fg(Color::Green)),
                    Style::default().fg(Color::Green),
                ),
                DiffLineKind::Removed => (
                    Span::styled("-  ", Style::default().fg(Color::Red)),
                    Style::default().fg(Color::Red),
                ),
                DiffLineKind::Context => (
                    Span::styled("   ", Style::default().fg(Color::DarkGray)),
                    Style::default().fg(Color::White),
                ),
            };

            let ln_str = format_gutter(diff_line.old_line_no, diff_line.new_line_no);
            let content: String = diff_line.content.chars().take(avail).collect();

            let mut row = vec![
                Span::styled(ln_str, Style::default().fg(Color::DarkGray)),
                marker,
            ];

            // Apply syntax highlighting for code lines (not headers)
            if diff_line.kind == DiffLineKind::Header {
                row.push(Span::styled(content, content_style));
            } else {
                let highlighted = highlight_code_line(&content, &file.path, content_style);
                row.extend(highlighted);
            }

            lines.push(Line::from(row));

            i += 1;
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
