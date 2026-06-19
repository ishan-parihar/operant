use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub alignments: Vec<Alignment>,
}

#[derive(Debug, Clone, Copy)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

pub fn detect_table<'a>(lines: &[&'a str], start: usize) -> Option<(Table, usize)> {
    if start >= lines.len() {
        return None;
    }

    let header_line = lines[start].trim();
    if !header_line.starts_with('|') || !header_line.ends_with('|') {
        return None;
    }

    let separator_idx = start + 1;
    if separator_idx >= lines.len() {
        return None;
    }

    let sep_line = lines[separator_idx].trim();
    if !sep_line.starts_with('|') || !sep_line.ends_with('|') {
        return None;
    }

    let headers: Vec<String> = header_line[1..header_line.len() - 1]
        .split('|')
        .map(|s| s.trim().to_string())
        .collect();

    let alignments: Vec<Alignment> = sep_line[1..sep_line.len() - 1]
        .split('|')
        .map(|s| {
            let s = s.trim();
            if s.starts_with(':') && s.ends_with(':') {
                Alignment::Center
            } else if s.ends_with(':') {
                Alignment::Right
            } else {
                Alignment::Left
            }
        })
        .collect();

    let mut rows = Vec::new();
    let mut idx = start + 2;
    while idx < lines.len() {
        let row_line = lines[idx].trim();
        if !row_line.starts_with('|') || !row_line.ends_with('|') {
            break;
        }
        let row: Vec<String> = row_line[1..row_line.len() - 1]
            .split('|')
            .map(|s| s.trim().to_string())
            .collect();
        rows.push(row);
        idx += 1;
    }

    if rows.is_empty() {
        return None;
    }

    Some((Table { headers, rows, alignments }, idx))
}

pub fn render_table(table: &Table) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let col_widths: Vec<usize> = table
        .headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let max_data = table
                .rows
                .iter()
                .filter_map(|r| r.get(i))
                .map(|c| UnicodeWidthStr::width(c.as_str()))
                .max()
                .unwrap_or(0);
            max_data.max(UnicodeWidthStr::width(h.as_str()))
        })
        .collect();

    let header_spans: Vec<Span> = table
        .headers
        .iter()
        .enumerate()
        .flat_map(|(i, h)| {
            let w = col_widths[i];
            let padded = pad_str(h, w, table.alignments.get(i).copied().unwrap_or(Alignment::Left));
            vec![
                Span::styled(" ", Style::default()),
                Span::styled(padded, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(" ", Style::default()),
            ]
        })
        .collect();
    lines.push(Line::from(header_spans));

    let sep_spans: Vec<Span> = col_widths
        .iter()
        .map(|w| Span::styled(
            format!("{} ", "-".repeat(*w)),
            Style::default().fg(Color::DarkGray),
        ))
        .collect();
    lines.push(Line::from(sep_spans));

    for row in &table.rows {
        let row_spans: Vec<Span> = row
            .iter()
            .enumerate()
            .flat_map(|(i, c)| {
                let w = col_widths[i];
                let padded = pad_str(c, w, table.alignments.get(i).copied().unwrap_or(Alignment::Left));
                vec![
                    Span::styled(" ", Style::default()),
                    Span::styled(padded, Style::default()),
                    Span::styled(" ", Style::default()),
                ]
            })
            .collect();
        lines.push(Line::from(row_spans));
    }

    lines
}

fn pad_str(s: &str, width: usize, align: Alignment) -> String {
    let s_width = UnicodeWidthStr::width(s);
    if s_width >= width {
        return s.to_string();
    }
    let padding = width - s_width;
    match align {
        Alignment::Left => format!("{}{}", s, " ".repeat(padding)),
        Alignment::Right => format!("{}{}", " ".repeat(padding), s),
        Alignment::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_simple_table() {
        let lines = vec![
            "| Name | Age |",
            "|------|-----|",
            "| Alice | 30 |",
            "| Bob | 25 |",
        ];
        let result = detect_table(&lines, 0);
        assert!(result.is_some());
        let (table, end) = result.unwrap();
        assert_eq!(table.headers, vec!["Name", "Age"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(end, 4);
    }

    #[test]
    fn render_table_produces_lines() {
        let table = Table {
            headers: vec!["A".into(), "B".into()],
            rows: vec![vec!["1".into(), "2".into()]],
            alignments: vec![Alignment::Left, Alignment::Right],
        };
        let lines = render_table(&table);
        assert!(lines.len() >= 3);
    }
}
