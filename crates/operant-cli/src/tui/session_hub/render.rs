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

use super::{PaneFocus, SessionHubState};
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

/// Compact token count for the row's meta line: a 14,103-message session's
/// token total is seven digits, which at a 40%-width list would push the
/// session id off the row entirely. `k`/`M` keeps the identifier visible.
fn tokens_suffix(tokens: u64) -> String {
    const K: f64 = 1_000.0;
    const M: f64 = 1_000_000.0;
    let t = tokens as f64;
    if tokens == 0 {
        String::new()
    } else if t >= M {
        format!(" · {:.1}M tok", t / M)
    } else if t >= K {
        format!(" · {:.1}k tok", t / K)
    } else {
        format!(" · {tokens} tok")
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

/// The preview renders one line per message, so a multi-paragraph message
/// needs its first non-empty line (the operator is scanning shape, not reading
/// the body — that is what Enter-switch is for). Blank bodies collapse to a
/// single placeholder so a tool-only message still produces a row.
fn first_line(content: &str) -> String {
    let line = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(no text)");
    line.chars().take(200).collect()
}

/// Render the hub full-frame. Callers gate on `state.visible` (the overlay
/// guard table), so this renders INSTEAD of the chat frame — never both
/// (the one-renderer rule).
pub fn render_session_hub(
    frame: &mut Frame,
    state: &SessionHubState,
    area: ratatui::layout::Rect,
    ctx: &mut crate::tui::render::FrameCtx,
) {
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
    render_preview(frame, state, cols[1], ctx);
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
                // (2026-10-29) The operator could not see the session's ID,
                // employee, message count or tokens: the meta was ONE appended
                // span, and the list is 40% of the frame, so a 100+ column
                // overflow truncated it to "age · count · $". jcode's picker
                // renders a title row plus a dedicated subtitle row
                // (`session_picker.rs`'s ListItem of two lines); the second
                // line is what carries the identifiers.
                //
                // ID and employee are DIFFERENT things and both are shown:
                // the id is the handle for /resume and `sessions show`, the
                // employee is who is executing (and it is the branch/anchor
                // key when the operator filters by employee).
                // An employee session's id IS the employee id
                // (`emp-cron_fa1e44c`), so print the pair once: showing both
                // doubled a row and pushed the metrics off a 40%-width pane.
                let emp = row
                    .employee_id
                    .as_deref()
                    .filter(|e| *e != row.id)
                    .map(|e| format!("  🪪 {e}"))
                    .unwrap_or_default();
                // 12 chars fits `#emp-cron_fa1e4` and `#sess_87b7abb9-`
                // inside a 40%-width pane alongside the metrics; the operator
                // resolves the full id from /resume's completion or
                // `sessions list --json` when they need the whole string.
                let short_id: String = row.id.chars().take(12).collect();
                let token_span = tokens_suffix(row.tokens);
                let meta_line = Line::from(vec![
                    // ID first: it is the row's handle, and the one thing an
                    // operator is never otherwise shown on screen.
                    Span::styled(
                        format!("#{short_id}"),
                        Style::default().fg(theme_colors::muted()),
                    ),
                    Span::styled(emp, Style::default().fg(theme_colors::muted())),
                    Span::styled(
                        format!("  {} msg{token_span}", row.message_count),
                        Style::default().fg(theme_colors::muted()),
                    ),
                    Span::styled(
                        format!("  {}", relative_time(row.updated_at)),
                        Style::default().fg(theme_colors::muted()),
                    ),
                    Span::styled(
                        format!("  ${:.4}", row.cost_usd),
                        Style::default().fg(theme_colors::muted()),
                    ),
                ]);
                // ONE ListItem holding BOTH lines, not two items: the
                // selection index must stay 1:1 with `visible_idx`, and a
                // second item per session would drift Up/Down onto the meta
                // row. ratatui's List supports a multi-line item body.
                ListItem::new(vec![Line::from(spans), meta_line])
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

fn render_preview(
    frame: &mut Frame,
    state: &SessionHubState,
    area: ratatui::layout::Rect,
    ctx: &mut crate::tui::render::FrameCtx,
) {
    let focused = state.preview_focused();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            theme_colors::accent()
        } else {
            theme_colors::border()
        }))
        .title(Span::styled(
            // jcode marks the focused pane in its title
            // (session_picker.rs:1430) so a keyboard-only operator can see
            // where Tab put them.
            if focused {
                format!(
                    " Preview [focused — Tab to {}] ",
                    PaneFocus::Sessions.label()
                )
            } else {
                " Preview ".to_string()
            },
            Style::default()
                .fg(if focused {
                    theme_colors::warning()
                } else {
                    theme_colors::accent()
                })
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    let inner_height = inner.height;
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
        _ => {
            let mut lines: Vec<Line> = vec![
                Line::from(Span::styled(
                    row_title(row),
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "id: {} · {} · ${:.4} · {} msg",
                        row.id,
                        relative_time(row.updated_at),
                        row.cost_usd,
                        row.message_count
                    ),
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    match &row.employee_id {
                        Some(e) => format!("employee: {e}"),
                        None => "employee: — (personal)".to_string(),
                    },
                    Style::default().fg(theme_colors::muted()),
                )),
            ];
            // The transcript itself (jcode's preview body). The load is
            // asynchronous, so the pane shows an explicit state rather than
            // silently looking empty — the two states that read as "the
            // preview is broken" are "still loading" and "no content", and
            // naming both is what makes a slow load legible.
            match &state.preview {
                Some((id, msgs, hidden)) if id == &row.id && !msgs.is_empty() => {
                    if *hidden > 0 {
                        lines.push(Line::from(Span::styled(
                            format!("— {hidden} earlier prompt(s) not shown (tail window) —"),
                            Style::default().fg(theme_colors::muted()),
                        )));
                    }
                    lines.push(Line::from(""));
                    for (i, msg) in msgs.iter().enumerate() {
                        let (label, color) = match msg.role.as_str() {
                            "user" => ("you", theme_colors::accent()),
                            "assistant" => ("agent", theme_colors::success()),
                            "system" => ("system", theme_colors::muted()),
                            other => (other, theme_colors::muted()),
                        };
                        let mut spans = vec![Span::styled(
                            format!(
                                "{}. {label}: ",
                                // Global prompt numbering: the hidden prompts
                                // above offset the window's own index, so the
                                // operator can match a preview row to the
                                // prompt number they typed (jcode's
                                // `hidden_prompts` reasoning).
                                if msg.role == "user" {
                                    *hidden
                                        + msgs[..i].iter().filter(|m| m.role == "user").count()
                                        + 1
                                } else {
                                    i + 1
                                }
                            ),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        )];
                        for call in &msg.tool_calls {
                            spans.push(Span::styled(
                                format!("[{call}] "),
                                Style::default().fg(theme_colors::warning()),
                            ));
                        }
                        spans.push(Span::raw(first_line(&msg.content)));
                        lines.push(Line::from(spans));
                    }
                }
                Some((id, _, _)) if id == &row.id => {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "No stored messages for this session.",
                        Style::default().fg(theme_colors::muted()),
                    )));
                }
                _ => {
                    // jcode's third state: no preview cached yet and none in
                    // flight. That is a MISS, not a load — writing "loading"
                    // here is exactly the operator's stuck "Loading session…"
                    // card. Name what the operator can do about it.
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        if state.preview_loading {
                            "⏳ Loading transcript…"
                        } else if state.has_preview_content() {
                            "Preview available."
                        } else {
                            "No transcript cached — press r to load."
                        },
                        Style::default().fg(theme_colors::muted()),
                    )));
                }
            }
            lines.push(Line::from(Span::styled(
                "Enter switches to this session",
                Style::default().fg(theme_colors::accent()),
            )));
            // The scroll bound must count WRAPPED rows, not source lines: the
            // paragraph wraps below, so a 200-char message occupies three
            // visual rows while `lines.len()` still sees one. Measuring the
            // unwrapped count leaves the operator unable to reach the last
            // message and makes the pane look like it scrolls "past" content
            // (the right-pane glitch the operator reported).
            //
            // Only the FIRST span of each line decides the wrap (a row is one
            // grammatical unit here: `N. label: text`), so measuring the
            // joined spans is exact for every row this pane builds.
            let width = inner.width.max(1) as usize;
            let visual_rows: usize = lines
                .iter()
                .map(|line| {
                    let text: String = line
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<Vec<_>>()
                        .join("");
                    if text.is_empty() {
                        1
                    } else {
                        crate::tui::render::utils::balanced_wrap(&text, width)
                            .len()
                            .max(1)
                    }
                })
                .sum();
            ctx.preview_max_scroll = Some((visual_rows as u16).saturating_sub(inner_height));
            lines
        }
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((state.preview_scroll, 0));
    frame.render_widget(paragraph, inner);
}
