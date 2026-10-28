//! session_hub rendering — jcode's picker geometry, operant's theme.
//!
//! Ported from jcode `session_picker.rs::render` (2382-2470): the vertical
//! constraint stack (search row when active, then `Min(8)` for the body) and
//! the horizontal 40/60 split — list left, preview right. jcode's crash
//! banner and onboarding band constraints had no operant sources and were
//! not ported.

use ratatui::widgets::Wrap;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::SessionHubState;
use crate::tui::theme_colors;
use operant_core::session::listing::{RowKind, RowStatus, SessionEntryV2};

/// jcode's split: 40% list, 60% preview.
const SPLIT: (Constraint, Constraint) = (Constraint::Percentage(40), Constraint::Percentage(60));

/// The compact age form the TUI already uses (app/mod.rs's session-list
/// drain) — moved here so the hub owns one formatter.
pub fn relative_time(updated_at: chrono::DateTime<chrono::Utc>) -> String {
    let age = chrono::Utc::now().signed_duration_since(updated_at);
    if age.num_minutes() < 1 {
        "just now".to_string()
    } else if age.num_hours() < 1 {
        format!("{}m ago", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

fn status_glyph(status: RowStatus) -> &'static str {
    match status {
        RowStatus::Live => "⚡",
        RowStatus::Idle => "·",
        RowStatus::Archived => "📦",
    }
}

fn row_title(row: &SessionEntryV2) -> String {
    match &row.kind {
        RowKind::Subagent { goal, .. } => {
            let short: String = goal.chars().take(48).collect();
            format!("🤖 {short}")
        }
        _ => {
            let base = row
                .title
                .clone()
                .unwrap_or_else(|| "(untitled)".to_string());
            if matches!(row.kind, RowKind::Branch) {
                format!("↳ {base}")
            } else {
                base
            }
        }
    }
}

/// Per-char search highlight, jcode's `highlight_spans` semantics: tokens
/// matched independently (OR), overlapping/adjacent merged.
fn highlight_spans(text: &str, tokens: &[String], base: Style, hl: Style) -> Vec<Span<'static>> {
    if tokens.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let lower = text.to_lowercase();
    let mut mask = vec![false; lower.len()];
    for t in tokens {
        let mut from = 0;
        while let Some(at) = lower[from..].find(t.as_str()) {
            for cell in mask.iter_mut().skip(from + at).take(t.len()) {
                *cell = true;
            }
            from += at + t.chars().count();
        }
    }
    let mut spans = Vec::new();
    let mut cur_hl = mask.first().copied().unwrap_or(false);
    let mut buf = String::new();
    for (ch, &m) in text.chars().zip(mask.iter()) {
        if m != cur_hl {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                if cur_hl { hl } else { base },
            ));
            cur_hl = m;
        }
        buf.push(ch);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, if cur_hl { hl } else { base }));
    }
    spans
}

/// Render the hub full-frame. Callers gate on `state.visible` (the overlay
/// guard table), so this renders INSTEAD of the chat frame — never both
/// (the one-renderer rule).
pub fn render_session_hub(frame: &mut Frame, state: &SessionHubState, area: ratatui::layout::Rect) {
    let has_search = state.search_active || !state.search_query.is_empty();
    let mut v = Vec::new();
    if has_search {
        v.push(Constraint::Length(1));
    }
    v.push(Constraint::Min(8));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(v)
        .split(area);
    let mut idx = 0;
    if has_search {
        let search_area = chunks[idx];
        idx += 1;
        let cursor = if state.search_active { "▎" } else { "" };
        let line = Line::from(vec![
            Span::styled(" 🔍 ", Style::default().fg(theme_colors::accent())),
            Span::styled(
                state.search_query.clone(),
                Style::default()
                    .fg(theme_colors::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                cursor.to_string(),
                Style::default().fg(theme_colors::accent()),
            ),
            Span::styled(
                if state.search_active {
                    "  Esc to clear"
                } else {
                    "  / to edit"
                },
                Style::default().fg(theme_colors::muted()),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), search_area);
    }
    let body = chunks[idx];
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([SPLIT.0, SPLIT.1])
        .split(body);

    render_list(frame, state, cols[0]);
    render_preview(frame, state, cols[1]);
}

fn render_list(frame: &mut Frame, state: &SessionHubState, area: ratatui::layout::Rect) {
    let items: Vec<ListItem> = if state.loading {
        vec![
            ListItem::new(Line::from(vec![
                Span::styled("  ⏳ ", Style::default().fg(theme_colors::accent())),
                Span::styled(
                    "Loading sessions…",
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            ListItem::new(Line::from(Span::styled(
                "     scanning local sessions and live subagents…",
                Style::default().fg(theme_colors::muted()),
            ))),
        ]
    } else if state.visible_rows().is_empty() {
        vec![
            ListItem::new(Line::from(Span::styled(
                "  No sessions match",
                Style::default()
                    .fg(theme_colors::text())
                    .add_modifier(Modifier::BOLD),
            ))),
            ListItem::new(Line::from(Span::styled(
                "     s cycles filters · Esc closes",
                Style::default().fg(theme_colors::muted()),
            ))),
        ]
    } else {
        let tokens = if state.search_query.is_empty() {
            Vec::new()
        } else {
            SessionHubState::search_tokens()(&state.search_query)
        };
        state
            .visible_rows()
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let selected = i == state.selected;
                let title_style = if selected {
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme_colors::text())
                };
                let mut spans = vec![Span::styled(
                    format!(" {} ", status_glyph(row.status)),
                    Style::default().fg(if matches!(row.status, RowStatus::Live) {
                        theme_colors::success()
                    } else {
                        theme_colors::muted()
                    }),
                )];
                spans.extend(highlight_spans(
                    &row_title(row),
                    &tokens,
                    title_style,
                    Style::default()
                        .fg(theme_colors::warning())
                        .add_modifier(Modifier::BOLD),
                ));
                // meta: employee tag + age + cost (jcode's compact row tail).
                let emp = row
                    .employee_id
                    .as_deref()
                    .map(|e| format!(" 🪪{e}"))
                    .unwrap_or_default();
                spans.push(Span::styled(
                    format!(
                        "{emp}  {} · {} · ${:.4}",
                        relative_time(row.updated_at),
                        row.message_count,
                        row.cost_usd
                    ),
                    Style::default().fg(theme_colors::muted()),
                ));
                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::border()))
        .title(Span::styled(
            format!(" Sessions [{}] ", state.filter_mode.label()),
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        ));
    let mut list_state = ListState::default();
    if !state.loading && !state.visible_rows().is_empty() {
        list_state.select(Some(state.selected));
    }
    frame.render_stateful_widget(
        List::new(items).block(block).highlight_style(
            Style::default()
                .bg(theme_colors::selection_bg())
                .fg(theme_colors::text()),
        ),
        area,
        &mut list_state,
    );
}

fn render_preview(frame: &mut Frame, state: &SessionHubState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::border()))
        .title(Span::styled(
            " Preview ",
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(row) = state.selected_row() else {
        return;
    };

    let lines: Vec<Line> = match (&row.kind, state.focused_subagent.as_deref()) {
        // iter-C focus view: the subagent row's own detail replaces the
        // message preview (omp's chat view — goal/status/result inline).
        (
            RowKind::Subagent {
                goal,
                model,
                dispatch_time,
                status,
            },
            Some(focused),
        ) if focused == row.id => {
            let dispatched = chrono::DateTime::from_timestamp(*dispatch_time as i64, 0)
                .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());
            vec![
                Line::from(Span::styled(
                    format!("🤖 subagent {} [{status}]", row.id),
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "goal: ",
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    goal.clone(),
                    Style::default().fg(theme_colors::text()),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("model: {model}"),
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    format!("dispatched: {dispatched}"),
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    "←← / Esc returns to the list",
                    Style::default().fg(theme_colors::muted()),
                )),
            ]
        }
        _ => vec![
            Line::from(Span::styled(
                row_title(row),
                Style::default()
                    .fg(theme_colors::text())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("id: {}", row.id),
                Style::default().fg(theme_colors::muted()),
            )),
            if let Some(e) = &row.employee_id {
                Line::from(Span::styled(
                    format!("employee: {e}"),
                    Style::default().fg(theme_colors::muted()),
                ))
            } else {
                Line::from(Span::styled(
                    "employee: — (personal)",
                    Style::default().fg(theme_colors::muted()),
                ))
            },
            if let Some(c) = &row.cwd {
                Line::from(Span::styled(
                    format!("cwd: {c}"),
                    Style::default().fg(theme_colors::muted()),
                ))
            } else {
                Line::from("")
            },
            Line::from(Span::styled(
                format!(
                    "tokens: ~{} · cost: ${:.4} · messages: {}",
                    row.tokens, row.cost_usd, row.message_count
                ),
                Style::default().fg(theme_colors::muted()),
            )),
            Line::from(Span::styled(
                "Enter switches to this session",
                Style::default().fg(theme_colors::accent()),
            )),
        ],
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((state.preview_scroll, 0));
    frame.render_widget(paragraph, inner);
}
