//! The Surface IR's widget renderer (B0, iter-2652).
//!
//! Renders a `SurfaceOp`'s widget tree as `Vec<Line<'static>>` — the same
//! shape every other transcript row body uses — so a settled surface row
//! draws as a bordered card with aligned columns and bars instead of a
//! paragraph of plain text.
//!
//! Three rules carried from the IR, and they are load-bearing:
//!
//! 1. `SemanticTone` is the ONLY colour surface. No node names a colour; this
//!    module maps the six tones onto the palette roles the rest of the TUI
//!    uses, so a surface cannot introduce a colour nothing else renders.
//! 2. The renderer TRUSTS the gate. The caller emits the post-scrub op, so no
//!    string here is re-scrubbed; re-scrubbing would mean the gate's verdict
//!    depends on who drew.
//! 3. Nothing interactive is interactive here. `Button`/`ChoicePicker`/… draw
//!    their LABELS; the dialog (`ask_user_dialog`) owns the actual
//!    interaction. A transcript row must not claim a tap target.
//!
//! `plain::render_plain` stays the text fallback — the tool result the model
//! sees, the gateway caption, the PNG rasteriser. This module is the TUI's
//! settled-row body.

use unicode_width::UnicodeWidthStr;

use operant_core::ui::{BarDatum, ChartKind, ColumnAlign, DiffKind, SemanticTone};
use operant_core::ui::{TableColumn, TableRow, WidgetNode};

use ratatui::prelude::{Line, Span, Style};
use ratatui::style::Color;

use crate::tui::operant_app::color_support::rgb;
use crate::tui::operant_render::render_rounded_box;

/// The label style per tone. One arm per `SemanticTone` variant — the
/// compiler enforces this, which is the point: a new tone cannot be silently
/// unrendered.
fn tone_style(tone: SemanticTone) -> Style {
    match tone {
        SemanticTone::Default => Style::default().fg(Color::Gray),
        SemanticTone::Muted => Style::default().fg(rgb(120, 124, 138)),
        SemanticTone::Accent => Style::default().fg(rgb(130, 140, 255)),
        SemanticTone::Success => Style::default().fg(rgb(110, 220, 150)),
        SemanticTone::Warning => Style::default().fg(rgb(255, 200, 90)),
        SemanticTone::Danger => Style::default().fg(rgb(255, 110, 110)),
    }
}

fn tone_badge(label: &str, tone: SemanticTone) -> Span<'static> {
    Span::styled(format!(" {} ", label), tone_style(tone).reversed())
}

/// Render one node as lines at `width` display columns.
///
/// This is the settled-row entry point: the transcript assembles the body
/// into its own column, so no outer gutter is added here.
pub fn render_surface(node: &WidgetNode, width: usize) -> Vec<Line<'static>> {
    render_surface_padded(node, width, 0)
}

/// Render one node as lines at `width` columns, dropping `pad` columns of
/// left gutter of each line so nested bodies align under their border.
fn render_surface_padded(node: &WidgetNode, width: usize, pad: usize) -> Vec<Line<'static>> {
    let span = width.max(1).min(120);
    let mut lines = Vec::new();
    render_node(node, span, &mut lines);
    if pad > 0 {
        for line in &mut lines {
            line.spans.insert(0, Span::raw(" ".repeat(pad)));
        }
    }
    lines
}

fn push_wrapped(lines: &mut Vec<Line<'static>>, text: &str, style: Style, width: usize) {
    if text.is_empty() {
        lines.push(Line::from(Span::styled("".to_string(), style)));
        return;
    }
    for chunk in wrap_display_text(text, width) {
        lines.push(Line::from(Span::styled(chunk, style)));
    }
}

fn render_node(node: &WidgetNode, width: usize, lines: &mut Vec<Line<'static>>) {
    match node {
        WidgetNode::Row { children } => {
            let mut row: Vec<Span<'static>> = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    row.push(Span::raw("  "));
                }
                let cell = render_surface(child, width / children.len().max(1));
                for (j, line) in cell.into_iter().enumerate() {
                    if j == 0 {
                        row.extend(line.spans);
                    } else {
                        // Wrapped content from a row child goes on its own
                        // line — a row cannot share a cell across lines.
                        lines.push(Line::from(std::mem::take(&mut row)));
                        lines.push(line);
                    }
                }
            }
            if !row.is_empty() {
                lines.push(Line::from(row));
            }
        }
        WidgetNode::Column { children } => {
            for child in children {
                render_node(child, width, lines);
            }
        }
        WidgetNode::Card { title, child, tone } => {
            let body = render_surface(child, width.saturating_sub(4).max(1));
            let border_style = tone_style(*tone);
            let boxed = render_rounded_box(title, body, width.max(6), border_style);
            for line in boxed {
                lines.push(line);
            }
        }
        WidgetNode::Tabs {
            tabs,
            active,
            child,
        } => {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (i, tab) in tabs.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" │ ", Style::default().fg(rgb(80, 84, 96))));
                }
                let style = if i == *active {
                    Style::default().fg(rgb(160, 175, 255)).bold().underlined()
                } else {
                    Style::default().fg(rgb(120, 124, 138))
                };
                spans.push(Span::styled(tab.clone(), style));
            }
            lines.push(Line::from(spans));
            render_node(child, width, lines);
        }
        WidgetNode::List { items, ordered } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };
                let mut spans = vec![Span::styled(
                    format!("{marker} "),
                    Style::default().fg(rgb(150, 160, 200)),
                )];
                spans.extend(tone_spans(&item.text, item.tone));
                if let Some(detail) = item.detail.as_deref() {
                    spans.push(Span::styled(
                        format!("  {detail}"),
                        Style::default().fg(rgb(110, 114, 128)),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }
        WidgetNode::Divider => {
            lines.push(Line::from(Span::styled(
                "─".repeat(width.max(1).min(120)),
                Style::default().fg(rgb(70, 74, 86)),
            )));
        }
        WidgetNode::Text { text, tone, mono } => {
            let style = if *mono {
                tone_style(*tone).fg(Color::White)
            } else {
                tone_style(*tone)
            };
            push_wrapped(lines, text, style, width);
        }
        WidgetNode::Badge { label, tone } => {
            lines.push(Line::from(vec![tone_badge(label, *tone)]));
        }
        WidgetNode::KeyValueGrid { pairs, columns } => {
            let columns = (*columns).max(1);
            for chunk in pairs.chunks(columns) {
                let mut spans: Vec<Span<'static>> = Vec::new();
                for (i, pair) in chunk.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::raw("   "));
                    }
                    spans.push(Span::styled(
                        pair.key.clone() + ":",
                        Style::default().fg(rgb(130, 134, 148)),
                    ));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(pair.value.clone(), tone_style(pair.tone)));
                }
                lines.push(Line::from(spans));
            }
        }
        WidgetNode::Table { columns, rows } => {
            render_table(columns, rows, width, lines);
        }
        WidgetNode::BarChart { title, kind, data } => {
            render_bar_chart(title, *kind, data, width, lines);
        }
        WidgetNode::Progress {
            label,
            value,
            detail,
        } => {
            let pct = value.clamp(0.0, 100.0);
            let bar_width = 28usize.min(width.saturating_sub(label.len() + 10).max(8));
            let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
            let mut spans = vec![
                Span::styled(label.clone(), Style::default().fg(rgb(170, 175, 195))),
                Span::raw(" "),
                Span::styled("█".repeat(filled), Style::default().fg(rgb(110, 220, 150))),
                Span::styled(
                    "░".repeat(bar_width.saturating_sub(filled)),
                    Style::default().fg(rgb(70, 74, 86)),
                ),
                Span::styled(
                    format!(" {pct:.0}%"),
                    Style::default().fg(rgb(160, 165, 185)),
                ),
            ];
            if let Some(detail) = detail.as_deref() {
                spans.push(Span::styled(
                    format!("  {detail}"),
                    Style::default().fg(rgb(110, 114, 128)),
                ));
            }
            lines.push(Line::from(spans));
        }
        WidgetNode::CodeDiff {
            path,
            lines: diff_lines,
            show_markers,
        } => {
            if let Some(path) = path.as_deref() {
                lines.push(Line::from(Span::styled(
                    path.to_string(),
                    Style::default().fg(rgb(150, 160, 200)).bold(),
                )));
            }
            for diff in diff_lines {
                let (marker, style) = match diff.kind {
                    DiffKind::Context => (" ", Style::default().fg(rgb(140, 144, 158))),
                    DiffKind::Added => ("+", Style::default().fg(rgb(110, 220, 150))),
                    DiffKind::Removed => ("-", Style::default().fg(rgb(255, 110, 110))),
                    DiffKind::Hunk => ("@", Style::default().fg(rgb(130, 140, 255))),
                };
                let mut spans = Vec::new();
                if *show_markers {
                    spans.push(Span::styled(format!("{marker} "), style));
                }
                spans.push(Span::styled(diff.text.clone(), style));
                lines.push(Line::from(spans));
            }
        }
        WidgetNode::StatusTimeline { steps } => {
            for step in steps.iter() {
                let marker = match step.tone {
                    SemanticTone::Success => "✓",
                    SemanticTone::Warning => "!",
                    SemanticTone::Danger => "✗",
                    SemanticTone::Accent => "◐",
                    _ => "○",
                };
                let mut spans = vec![Span::styled(format!("{marker} "), tone_style(step.tone))];
                spans.extend(tone_spans(&step.label, SemanticTone::Default));
                if let Some(detail) = step.detail.as_deref() {
                    spans.push(Span::styled(
                        format!("  {detail}"),
                        Style::default().fg(rgb(110, 114, 128)),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }
        WidgetNode::Button { buttons } => {
            // A transcript row cannot be a tap target; it draws the offered
            // actions as labels so the reader knows what the card wanted.
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                "actions: ",
                Style::default().fg(rgb(110, 114, 128)),
            )];
            for (i, button) in buttons.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(tone_badge(&button.label, button.tone));
            }
            lines.push(Line::from(spans));
        }
        WidgetNode::ChoicePicker { question, choices } => {
            push_wrapped(
                lines,
                question,
                Style::default().bold().fg(Color::White),
                width,
            );
            for (i, choice) in choices.iter().enumerate() {
                let mut spans = vec![Span::styled(
                    format!("  {}. ", i + 1),
                    Style::default().fg(rgb(130, 140, 255)),
                )];
                spans.push(Span::styled(
                    choice.label.clone(),
                    Style::default().fg(Color::Gray),
                ));
                if let Some(desc) = choice.description.as_deref() {
                    spans.push(Span::styled(
                        format!("  {desc}"),
                        Style::default().fg(rgb(110, 114, 128)),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }
        WidgetNode::TextField {
            label,
            value,
            placeholder,
            masked,
        } => {
            let shown = if !value.is_empty() {
                if *masked {
                    "•".repeat(value.chars().count())
                } else {
                    value.clone()
                }
            } else {
                placeholder.clone().unwrap_or_default()
            };
            let mut spans = vec![Span::styled(
                format!("{label}: "),
                Style::default().fg(rgb(130, 134, 148)),
            )];
            spans.push(Span::styled(
                format!("[{shown}]"),
                if value.is_empty() {
                    Style::default().fg(rgb(110, 114, 128)).italic()
                } else {
                    Style::default().fg(Color::White)
                },
            ));
            lines.push(Line::from(spans));
        }
        WidgetNode::Confirm { question, default } => {
            let suffix = if *default { " [y/N]" } else { " [Y/n]" };
            push_wrapped(
                lines,
                &format!("{question}{suffix}"),
                Style::default().fg(Color::White),
                width,
            );
        }
        WidgetNode::Modal { title, child } => {
            lines.push(Line::from(Span::styled(
                format!("╔═ {title} ═╗"),
                Style::default().fg(rgb(130, 140, 255)).bold(),
            )));
            render_node(child, width.saturating_sub(2).max(1), lines);
            lines.push(Line::from(Span::styled(
                "╚══════════╝".to_string(),
                Style::default().fg(rgb(130, 140, 255)).bold(),
            )));
        }
    }
}

fn tone_spans(text: &str, tone: SemanticTone) -> Vec<Span<'static>> {
    vec![Span::styled(text.to_string(), tone_style(tone))]
}

/// Split text into display-width chunks without breaking a wide glyph.
fn wrap_display_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split(' ') {
        let word_width = UnicodeWidthStr::width(word);
        let needed = if current.is_empty() {
            word_width
        } else {
            word_width + 1
        };
        if current_width + needed > width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn render_table(
    columns: &[TableColumn],
    rows: &[TableRow],
    width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let count = columns.len();
    if count == 0 {
        return;
    }
    let mut widths: Vec<usize> = columns
        .iter()
        .map(|c| UnicodeWidthStr::width(c.header.as_str()))
        .collect();
    for row in rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i < count {
                widths[i] = widths[i].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }
    let total: usize = widths.iter().sum::<usize>() + (count - 1) * 2;
    if total > width {
        let shrink = total - width;
        // Squeeze the widest columns first.
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(widths[i]));
        let mut remaining = shrink;
        for &i in &order {
            if remaining == 0 {
                break;
            }
            let cut = widths[i].saturating_sub(1);
            let take = remaining.min(cut);
            widths[i] -= take;
            remaining -= take;
        }
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let cell = truncate_to_width(&column.header, widths[i]);
        spans.push(Span::styled(
            pad_to_width(&cell, widths[i], column.align),
            Style::default().fg(rgb(150, 160, 200)).bold(),
        ));
    }
    lines.push(Line::from(spans));
    let mut rule: Vec<Span<'static>> = Vec::new();
    for (i, _) in columns.iter().enumerate() {
        if i > 0 {
            rule.push(Span::raw("  "));
        }
        rule.push(Span::styled(
            "─".repeat(widths[i]),
            Style::default().fg(rgb(70, 74, 86)),
        ));
    }
    lines.push(Line::from(rule));

    for row in rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for i in 0..count {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            let cell = row.cells.get(i).cloned().unwrap_or_default();
            let cell = truncate_to_width(&cell, widths[i]);
            spans.push(Span::styled(
                pad_to_width(&cell, widths[i], columns[i].align),
                Style::default().fg(Color::Gray),
            ));
        }
        lines.push(Line::from(spans));
    }
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthStr::width(ch.to_string().as_str());
        if used + w > width.saturating_sub(1) && width > 0 {
            out.push('…');
            return out;
        }
        out.push(ch);
        used += w;
    }
    out
}

fn pad_to_width(text: &str, width: usize, align: ColumnAlign) -> String {
    let used = UnicodeWidthStr::width(text);
    if used >= width {
        return text.to_string();
    }
    let pad = width - used;
    match align {
        ColumnAlign::Left => format!("{text}{}", " ".repeat(pad)),
        ColumnAlign::Right => format!("{}{text}", " ".repeat(pad)),
        ColumnAlign::Center => {
            let left = pad / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(pad - left))
        }
    }
}

fn render_bar_chart(
    title: &str,
    kind: ChartKind,
    data: &[BarDatum],
    width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(rgb(150, 160, 200)).bold(),
    )));
    if data.is_empty() {
        return;
    }
    match kind {
        ChartKind::Sparkline => {
            let mut spark = String::new();
            let max = data.iter().map(|d| d.value.abs()).fold(0.0f64, f64::max);
            for datum in data {
                let frac = if max > 0.0 {
                    datum.value.abs() / max
                } else {
                    0.0
                };
                // Partial blocks: the nine levels, lowest first.
                let glyph = match (frac * 8.0) as usize {
                    0 => '▁',
                    1 => '▂',
                    2 => '▃',
                    3 => '▄',
                    4 => '▅',
                    5 => '▆',
                    6 => '▇',
                    _ => '█',
                };
                spark.push(glyph);
            }
            lines.push(Line::from(Span::styled(
                spark,
                Style::default().fg(rgb(130, 140, 255)),
            )));
        }
        ChartKind::Bar => {
            let max = data.iter().map(|d| d.value.abs()).fold(0.0f64, f64::max);
            let label_width = data
                .iter()
                .map(|d| UnicodeWidthStr::width(d.label.as_str()))
                .max()
                .unwrap_or(0)
                .min(20);
            let bar_space = width.saturating_sub(label_width + 2).max(4);
            for datum in data {
                let frac = if max > 0.0 {
                    datum.value.abs() / max
                } else {
                    0.0
                };
                let bar = ((frac * bar_space as f64).round() as usize).max(1);
                let label = truncate_to_width(&datum.label, label_width);
                let color = if datum.value < 0.0 {
                    rgb(255, 110, 110)
                } else {
                    rgb(110, 220, 150)
                };
                let spans = vec![
                    Span::styled(
                        pad_to_width(&label, label_width, ColumnAlign::Left),
                        Style::default().fg(rgb(170, 175, 195)),
                    ),
                    Span::raw("  "),
                    Span::styled("█".repeat(bar), Style::default().fg(color)),
                ];
                lines.push(Line::from(spans));
            }
        }
    }
}

/// The settled-row entry point for an op serialised as JSON (the form that
/// crosses the `SystemAnnotation` boundary): parse, walk, render.
///
/// Returns `Err` with a one-line reason on a corrupt or unknown payload; the
/// caller renders that line instead of a blank row, so a row that cannot be
/// understood is still legible.
pub fn render_surface_op_json(op_json: &str, width: usize) -> Result<Vec<Line<'static>>, String> {
    let op: operant_core::ui::SurfaceOp =
        serde_json::from_str(op_json).map_err(|e| format!("unreadable surface op: {e}"))?;
    Ok(render_surface(&op.widgets_column(), width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::ui::{
        BarDatum, ChartKind, ChoiceOption, DiffLine, KeyValue, ListItem, SemanticTone, TableColumn,
        TableRow, TimelineStep, WidgetNode,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn op_json(node: WidgetNode) -> String {
        let op = operant_core::ui::SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "test".into(),
            provenance: operant_core::ui::Provenance::ToolVerified {
                tool_name: "ui_render_surface".into(),
                source_uri: None,
            },
            body: node,
        };
        serde_json::to_string(&op).unwrap()
    }

    /// The 120x40 G0 snapshot: a card with a table and a chart renders
    /// every row inside the terminal, and the interactive labels survive.
    #[test]
    fn card_with_table_and_chart_fits_120x40() {
        let node = WidgetNode::Column {
            children: vec![
                WidgetNode::Card {
                    title: "Deploy plan".into(),
                    tone: SemanticTone::Accent,
                    child: Box::new(WidgetNode::Table {
                        columns: vec![
                            TableColumn {
                                header: "step".into(),
                                align: ColumnAlign::Left,
                            },
                            TableColumn {
                                header: "owner".into(),
                                align: ColumnAlign::Right,
                            },
                        ],
                        rows: vec![
                            TableRow {
                                cells: vec!["build".into(), "ci".into()],
                            },
                            TableRow {
                                cells: vec!["migrate".into(), "dba".into()],
                            },
                        ],
                    }),
                },
                WidgetNode::BarChart {
                    title: "coverage".into(),
                    kind: ChartKind::Bar,
                    data: vec![
                        BarDatum {
                            label: "core".into(),
                            value: 82.0,
                        },
                        BarDatum {
                            label: "cli".into(),
                            value: 64.0,
                        },
                    ],
                },
                WidgetNode::ChoicePicker {
                    question: "Ship it?".into(),
                    choices: vec![
                        ChoiceOption {
                            id: "yes".into(),
                            label: "ship".into(),
                            description: Some("now".into()),
                        },
                        ChoiceOption {
                            id: "no".into(),
                            label: "hold".into(),
                            description: None,
                        },
                    ],
                },
            ],
        };
        let lines = render_surface_op_json(&op_json(node), 116).unwrap();
        assert!(
            lines.len() >= 10,
            "the card must produce real rows, got {}",
            lines.len()
        );
        let drawn: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "Deploy plan",
            "step",
            "owner",
            "build",
            "migrate",
            "coverage",
            "core",
            "Ship it?",
            "ship",
            "hold",
        ] {
            assert!(drawn.contains(needle), "missing {needle:?} in:\n{drawn}");
        }
        assert!(
            lines.iter().all(|l| l.width() <= 116),
            "every row must fit 116 cols"
        );
    }

    /// The 55x24 G0 snapshot: a narrow terminal truncates cell text but the
    /// card still renders every row inside the width.
    #[test]
    fn narrow_terminal_never_exceeds_width() {
        let node = WidgetNode::Card {
            title: "A quite long title for a narrow card".into(),
            tone: SemanticTone::Warning,
            child: Box::new(WidgetNode::Column {
                children: vec![
                    WidgetNode::Table {
                        columns: vec![
                            TableColumn {
                                header: "name".into(),
                                align: ColumnAlign::Left,
                            },
                            TableColumn {
                                header: "value".into(),
                                align: ColumnAlign::Right,
                            },
                        ],
                        rows: vec![TableRow {
                            cells: vec![
                                "a very long cell value that will not fit".into(),
                                "1".into(),
                            ],
                        }],
                    },
                    WidgetNode::Progress {
                        label: "disk".into(),
                        value: 47.0,
                        detail: Some("half".into()),
                    },
                ],
            }),
        };
        let lines = render_surface_op_json(&op_json(node), 51).unwrap();
        assert!(
            lines.iter().all(|l| l.width() <= 51),
            "rows over 51 cols: {lines:?}"
        );
        let drawn: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            drawn.contains("disk"),
            "the progress row must survive: {drawn}"
        );
    }

    /// Zero-argument truncation at 55 cols: the interactive card's args are
    /// short (ids, labels), so the narrow render must carry them whole.
    #[test]
    fn interactive_card_args_survive_at_55_cols() {
        let node = WidgetNode::Column {
            children: vec![
                WidgetNode::Text {
                    text: "pick".into(),
                    tone: SemanticTone::Default,
                    mono: false,
                },
                WidgetNode::ChoicePicker {
                    question: "q".into(),
                    choices: vec![
                        ChoiceOption {
                            id: "a".into(),
                            label: "alpha".into(),
                            description: Some("first".into()),
                        },
                        ChoiceOption {
                            id: "b".into(),
                            label: "beta".into(),
                            description: None,
                        },
                    ],
                },
                WidgetNode::Confirm {
                    question: "go?".into(),
                    default: true,
                },
                WidgetNode::TextField {
                    label: "name".into(),
                    value: String::new(),
                    placeholder: Some("your name".into()),
                    masked: false,
                },
            ],
        };
        let lines = render_surface_op_json(&op_json(node), 51).unwrap();
        let drawn: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "alpha",
            "first",
            "beta",
            "go?",
            "[y/N]",
            "name",
            "your name",
        ] {
            assert!(drawn.contains(needle), "missing {needle:?} in:\n{drawn}");
        }
    }

    /// A corrupt payload is legible, not blank.
    #[test]
    fn corrupt_op_json_renders_a_reason() {
        let lines = render_surface_op_json("not json", 40).unwrap_err();
        assert!(lines.contains("unreadable surface op"), "{lines}");
    }

    /// Every interactive label draws; the transcript row never claims a tap.
    #[test]
    fn button_labels_draw_without_tap_targets() {
        let node = WidgetNode::Button {
            buttons: vec![
                operant_core::ui::ButtonSpec {
                    id: "go".into(),
                    label: "go".into(),
                    tone: SemanticTone::Success,
                },
                operant_core::ui::ButtonSpec {
                    id: "stop".into(),
                    label: "stop".into(),
                    tone: SemanticTone::Danger,
                },
            ],
        };
        let lines = render_surface_op_json(&op_json(node), 60).unwrap();
        let drawn: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("actions:"), "{drawn}");
        assert!(drawn.contains("go") && drawn.contains("stop"), "{drawn}");
    }

    /// The [v] raw view draws inside a TestBackend at 80x24 and shows the
    /// POST-SCRUB op, never a secret.
    #[test]
    fn raw_view_shows_scrubbed_op_never_the_secret() {
        let node = WidgetNode::Text {
            text: "[REDACTED]".into(),
            tone: SemanticTone::Default,
            mono: false,
        };
        let lines = render_surface_op_json(&op_json(node), 76).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        // Paint the lines through a minimal Paragraph so the TestBackend sees
        // exactly the bytes the transcript would draw.
        terminal
            .draw(|f| {
                let area = f.area();
                let para = ratatui::widgets::Paragraph::new(lines);
                f.render_widget(para, area);
            })
            .unwrap();
        let drawn: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            drawn.contains("[REDACTED]"),
            "the raw view must show the scrubbed op: {drawn:?}"
        );
        assert!(!drawn.contains("sk-"), "no secret may appear: {drawn:?}");
    }
}
