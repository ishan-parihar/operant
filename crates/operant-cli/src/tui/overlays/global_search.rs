// overlays/global_search.rs — Global ripgrep search dialog (T2-7).
//
// Extracted from the overlays.rs monolith.

// ---------------------------------------------------------------------------
// Global Search Dialog (T2-7)
// ---------------------------------------------------------------------------

/// State for the global ripgrep search dialog.
#[derive(Debug, Clone, Default)]
pub struct GlobalSearchState {
    pub visible: bool,
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected: usize,
    pub total_matches: usize,
    pub searching: bool,
}

/// A single search result from ripgrep.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file: String,
    pub line: u32,
    pub text: String,
}

impl GlobalSearchState {
    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
        self.results.clear();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        let count = self.results.len();
        if count == 0 {
            return;
        }
        if self.selected == 0 {
            self.selected = count - 1;
        } else {
            self.selected -= 1;
        }
    }

    pub fn select_next(&mut self) {
        let count = self.results.len();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + 1) % count;
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    /// Run ripgrep synchronously (should be called from tokio::task::spawn_blocking).
    pub fn run_search(&mut self, project_root: &std::path::Path) {
        if self.query.is_empty() {
            self.results.clear();
            return;
        }
        self.searching = true;
        let output = std::process::Command::new("rg")
            .args([
                "--json",
                "--max-count",
                "10",
                "--max-filesize",
                "1M",
                &self.query,
                ".",
            ])
            .current_dir(project_root)
            .output();

        self.searching = false;
        self.results.clear();
        self.total_matches = 0;

        if let Ok(out) = output {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some("match") = val["type"].as_str() {
                        let data = &val["data"];
                        let file = data["path"]["text"].as_str().unwrap_or("").to_string();
                        let line_no = data["line_number"].as_u64().unwrap_or(0) as u32;
                        let text = data["lines"]["text"]
                            .as_str()
                            .unwrap_or("")
                            .trim_end_matches('\n')
                            .to_string();
                        self.results.push(SearchResult {
                            file,
                            line: line_no,
                            text,
                        });
                        self.total_matches += 1;
                        if self.results.len() >= 500 {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Return the selected result as a `file:line` string for prompt injection.
    pub fn selected_ref(&self) -> Option<String> {
        self.results
            .get(self.selected)
            .map(|r| format!("{}:{}", r.file, r.line))
    }
}

/// Render the global search dialog overlay.
pub fn render_global_search(
    state: &GlobalSearchState,
    area: ratatui::layout::Rect,
    buf: &mut ratatui::buffer::Buffer,
) {
    use ratatui::{
        layout::Rect,
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph, Widget},
    };
    use std::path::Path;

    if !state.visible {
        return;
    }

    let w = (area.width * 4 / 5).max(40).min(area.width);
    let h = (area.height * 3 / 4).max(10).min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 4;
    let dialog = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    Clear.render(dialog, buf);
    Block::default()
        .title(" Search [Esc: close, Enter: insert, \u{2191}\u{2193}: navigate] ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan))
        .render(dialog, buf);

    let inner = Rect {
        x: dialog.x + 1,
        y: dialog.y + 1,
        width: dialog.width.saturating_sub(2),
        height: dialog.height.saturating_sub(2),
    };

    // Query input bar (first row)
    let query_line = Line::from(vec![
        Span::styled("/ ", Style::default().fg(Color::Cyan)),
        Span::styled(state.query.clone(), Style::default().fg(Color::White)),
        Span::styled("\u{2588}", Style::default().fg(Color::Cyan)),
    ]);
    Paragraph::new(query_line).render(
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
        buf,
    );

    // Separator
    let sep = Line::from(Span::styled(
        "\u{2500}".repeat(inner.width as usize),
        Style::default().fg(Color::DarkGray),
    ));
    Paragraph::new(sep).render(
        Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 1,
        },
        buf,
    );

    let results_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(3),
    };

    // Build grouped display rows: (is_header, result_idx_or_none, file_label, match_count, result_ref)
    // Group results by file
    #[derive(Clone)]
    enum DisplayRow {
        Header { label: String, count: usize },
        Result { result_idx: usize },
    }

    let mut rows: Vec<DisplayRow> = Vec::new();
    if !state.results.is_empty() {
        let mut current_file = "";
        let mut group_count = 0usize;
        let mut group_start = 0usize;

        for (idx, result) in state.results.iter().enumerate() {
            if result.file.as_str() != current_file {
                if !current_file.is_empty() {
                    // Patch the header we already pushed with the real count
                    if let Some(DisplayRow::Header { count, .. }) = rows.get_mut(group_start) {
                        *count = group_count;
                    }
                }
                current_file = result.file.as_str();
                group_count = 0;
                group_start = rows.len();
                let label = Path::new(&result.file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&result.file)
                    .to_string();
                rows.push(DisplayRow::Header { label, count: 0 });
            }
            group_count += 1;
            rows.push(DisplayRow::Result { result_idx: idx });
        }
        // Patch last group
        if let Some(DisplayRow::Header { count, .. }) = rows.get_mut(group_start) {
            *count = group_count;
        }
    }

    let max_visible = results_area.height as usize;
    // Scroll so the selected result is visible — find which display row it's in
    let selected_display_row = rows
        .iter()
        .position(|r| {
            if let DisplayRow::Result { result_idx } = r {
                *result_idx == state.selected
            } else {
                false
            }
        })
        .unwrap_or(0);
    let start = selected_display_row.saturating_sub(max_visible / 2);

    for (i, row) in rows[start..].iter().enumerate() {
        if i >= max_visible {
            break;
        }
        let row_y = results_area.y + i as u16;

        match row {
            DisplayRow::Header { label, count } => {
                // File group header: ─── filename (N) ──────────
                let count_str = format!(" ({}) ", count);
                let label_part = format!(" {} ", label);
                let dashes_right = (results_area.width as usize)
                    .saturating_sub(4 + label_part.len() + count_str.len());
                let header_line = Line::from(vec![Span::styled(
                    format!(
                        "\u{2500}\u{2500}\u{2500}{}{}{}",
                        label_part,
                        count_str,
                        "\u{2500}".repeat(dashes_right)
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]);
                Paragraph::new(header_line).render(
                    Rect {
                        x: results_area.x,
                        y: row_y,
                        width: results_area.width,
                        height: 1,
                    },
                    buf,
                );
            }
            DisplayRow::Result { result_idx } => {
                let result = &state.results[*result_idx];
                let selected = *result_idx == state.selected;
                let prefix = if selected { "> " } else { "  " };
                let style = if selected {
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::White)
                } else {
                    Style::default().fg(Color::Gray)
                };

                // Highlight query match in text
                let text_trimmed = result.text.trim();
                let query_lc = state.query.to_lowercase();
                let text_spans: Vec<Span<'static>> = if !query_lc.is_empty() {
                    let text_lc = text_trimmed.to_lowercase();
                    if let Some(pos) = text_lc.find(query_lc.as_str()) {
                        let before: String = text_trimmed
                            .chars()
                            .take(text_trimmed[..pos].chars().count())
                            .collect();
                        let matched: String = text_trimmed[pos..pos + query_lc.len()].to_string();
                        let after: String = text_trimmed[pos + query_lc.len()..]
                            .chars()
                            .take(30)
                            .collect();
                        vec![
                            Span::styled(before, style),
                            Span::styled(
                                matched,
                                style.bg(Color::Rgb(60, 50, 0)).fg(Color::Yellow),
                            ),
                            Span::styled(after, style),
                        ]
                    } else {
                        let t: String = text_trimmed.chars().take(50).collect();
                        vec![Span::styled(t, style)]
                    }
                } else {
                    let t: String = text_trimmed.chars().take(50).collect();
                    vec![Span::styled(t, style)]
                };

                let mut spans = vec![
                    Span::styled(prefix.to_string(), style),
                    Span::styled(format!("{:>4}  ", result.line), style.fg(Color::DarkGray)),
                ];
                spans.extend(text_spans);

                Paragraph::new(Line::from(spans)).render(
                    Rect {
                        x: results_area.x,
                        y: row_y,
                        width: results_area.width,
                        height: 1,
                    },
                    buf,
                );
            }
        }
    }

    // Status bar
    let status = if state.searching {
        "Searching\u{2026}".to_string()
    } else if state.results.is_empty() && !state.query.is_empty() {
        "No matches".to_string()
    } else if state.total_matches > 0 {
        format!(
            "{} matches in {} files",
            state.total_matches,
            state
                .results
                .iter()
                .map(|r| &r.file)
                .collect::<std::collections::HashSet<_>>()
                .len()
        )
    } else {
        "Type to search".to_string()
    };
    let status_y = inner.y + inner.height.saturating_sub(1);
    Paragraph::new(Line::from(vec![Span::styled(
        status,
        Style::default().fg(Color::DarkGray),
    )]))
    .render(
        Rect {
            x: inner.x,
            y: status_y,
            width: inner.width,
            height: 1,
        },
        buf,
    );
}
