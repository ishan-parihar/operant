//! Markdown table rendering.
//!
//! Block structure (lists, headings, block quotes, code fences, tables and
//! footnotes) is parsed by `pulldown_cmark` in `markdown.rs`, which detects a
//! table and hands the extracted cells to [`render_table`] here. This module
//! owns only the box-drawing presentation: column sizing, per-column
//! alignment and truncation to the wrap width.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Narrowest a table column may be squeezed before it is left over-wide.
const MIN_COL_WIDTH: usize = 3;

/// Alignment for one table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlignment {
    Left,
    Center,
    Right,
}

/// A parsed markdown table: header cells, body rows and per-column alignment.
#[derive(Debug, Clone)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub alignments: Vec<TableAlignment>,
}

/// Columns consumed by borders, padding and separators, excluding cell content.
fn table_overhead(columns: usize) -> usize {
    // leading "  " + the two border corners, plus per column two padding
    // dashes and the three columns the " │ " row framing costs.
    4 + 3 * columns
}

/// Shrink the widest columns, one column at a time, until the rendered row
/// fits `width`. Columns never go below [`MIN_COL_WIDTH`], so a table with
/// more columns than the terminal can hold stays over-wide rather than
/// collapsing to unreadable slivers.
fn fit_columns(col_widths: &mut [usize], width: usize) {
    let budget = width.saturating_sub(table_overhead(col_widths.len()));
    let mut total: usize = col_widths.iter().sum();
    while total > budget {
        let Some(widest) = col_widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > MIN_COL_WIDTH)
            .max_by_key(|(_, w)| **w)
            .map(|(i, _)| i)
        else {
            break;
        };
        col_widths[widest] -= 1;
        total -= 1;
    }
}

/// Truncate to `max` display columns, marking the cut with an ellipsis.
/// Grapheme clusters are never split.
fn truncate_to_width(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for grapheme in text.graphemes(true) {
        if used + grapheme.width() > max.saturating_sub(1) {
            break;
        }
        out.push_str(grapheme);
        used += grapheme.width();
    }
    out.push('\u{2026}');
    out
}

/// Render a markdown table as styled lines with box-drawing characters.
///
/// Column widths are the natural content widths, shrunk widest-first until the
/// rendered row fits `width`; over-wide cells are truncated with an ellipsis.
pub fn render_table(table: &Table, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Calculate column widths
    let mut col_widths: Vec<usize> = table
        .headers
        .iter()
        .map(|h| UnicodeWidthStr::width(h.as_str()).max(3))
        .collect();

    for row in &table.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_widths.len() {
                col_widths[i] = col_widths[i].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    fit_columns(&mut col_widths, width as usize);

    // Top border: ┌─┬─┐
    let mut top_border = String::from("  ┌");
    for (i, width) in col_widths.iter().enumerate() {
        top_border.push_str(&"─".repeat(width + 2));
        if i < col_widths.len() - 1 {
            top_border.push('┬');
        }
    }
    top_border.push('┐');
    lines.push(Line::from(vec![Span::styled(
        top_border,
        Style::default().fg(Color::DarkGray),
    )]));

    // Header row with bold styling
    let mut header_spans = vec![Span::styled(
        "  │ ".to_string(),
        Style::default().fg(Color::DarkGray),
    )];
    for (i, header) in table.headers.iter().enumerate() {
        let col_width = col_widths[i];
        let padded = match table
            .alignments
            .get(i)
            .copied()
            .unwrap_or(TableAlignment::Left)
        {
            TableAlignment::Left => format!("{:<width$}", header, width = col_width),
            TableAlignment::Right => format!("{:>width$}", header, width = col_width),
            TableAlignment::Center => {
                let hdr_width = UnicodeWidthStr::width(header.as_str());
                let total_pad = col_width.saturating_sub(hdr_width);
                let left_pad = total_pad / 2;
                format!(
                    "{:>width$}",
                    format!("{}{}", " ".repeat(left_pad), header),
                    width = col_width + left_pad
                )
            }
        };
        header_spans.push(Span::styled(
            truncate_to_width(&padded, col_width),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        header_spans.push(Span::styled(
            " │ ".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(header_spans));

    // Separator: ├─┼─┤
    let mut sep = String::from("  ├");
    for (i, width) in col_widths.iter().enumerate() {
        sep.push_str(&"─".repeat(width + 2));
        if i < col_widths.len() - 1 {
            sep.push('┼');
        }
    }
    sep.push('┤');
    lines.push(Line::from(vec![Span::styled(
        sep,
        Style::default().fg(Color::DarkGray),
    )]));

    // Data rows
    for row in &table.rows {
        let mut row_spans = vec![Span::styled(
            "  │ ".to_string(),
            Style::default().fg(Color::DarkGray),
        )];
        for (i, cell) in row.iter().enumerate() {
            if i < col_widths.len() {
                let col_width = col_widths[i];
                let padded = match table
                    .alignments
                    .get(i)
                    .copied()
                    .unwrap_or(TableAlignment::Left)
                {
                    TableAlignment::Left => format!("{:<width$}", cell, width = col_width),
                    TableAlignment::Right => format!("{:>width$}", cell, width = col_width),
                    TableAlignment::Center => {
                        let cell_width = UnicodeWidthStr::width(cell.as_str());
                        let total_pad = col_width.saturating_sub(cell_width);
                        let left_pad = total_pad / 2;
                        format!(
                            "{:>width$}",
                            format!("{}{}", " ".repeat(left_pad), cell),
                            width = col_width + left_pad
                        )
                    }
                };
                row_spans.push(Span::raw(truncate_to_width(&padded, col_width)));
            }
            row_spans.push(Span::styled(
                " │ ".to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(row_spans));
    }

    // Bottom border: └─┴─┘
    let mut bottom_border = String::from("  └");
    for (i, width) in col_widths.iter().enumerate() {
        bottom_border.push_str(&"─".repeat(width + 2));
        if i < col_widths.len() - 1 {
            bottom_border.push('┴');
        }
    }
    bottom_border.push('┘');
    lines.push(Line::from(vec![Span::styled(
        bottom_border,
        Style::default().fg(Color::DarkGray),
    )]));

    lines
}
