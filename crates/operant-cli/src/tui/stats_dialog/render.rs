// stats_dialog/render.rs — Stats dialog rendering (4 tabs).
//
// Extracted from the stats_dialog.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

pub fn render_stats_dialog(state: &StatsDialogState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    let layout = begin_modal_buf(buf, area, 92, 30, 2, 1);
    render_modal_title_buf(buf, layout.header_area, "Cost & stats", "esc");

    let tab_line = Line::from(vec![
        tab_span("Overview", state.tab == StatsTab::Overview),
        Span::styled("  ·  ", Style::default().fg(OPERANT_MUTED)),
        tab_span("Daily Tokens", state.tab == StatsTab::DailyTokens),
        Span::styled("  ·  ", Style::default().fg(OPERANT_MUTED)),
        tab_span("Cost Heatmap", state.tab == StatsTab::CostHeatmap),
        Span::styled("  ·  ", Style::default().fg(OPERANT_MUTED)),
        tab_span("Models", state.tab == StatsTab::Models),
    ]);
    if let Some(tab_area) = modal_header_line_area(layout.header_area, 1) {
        Paragraph::new(tab_line).render(tab_area, buf);
    }

    let content_area = layout.body_area;

    let Some(data) = &state.data else {
        Paragraph::new("Loading\u{2026}")
            .style(Style::default().fg(OPERANT_MUTED).bg(OPERANT_PANEL_BG))
            .render(content_area, buf);
        return;
    };

    match state.tab {
        StatsTab::Overview => render_overview(data, state, content_area, buf),
        StatsTab::DailyTokens => render_daily_tokens(data, state.range_days, content_area, buf),
        StatsTab::CostHeatmap => render_cost_heatmap(data, content_area, buf),
        StatsTab::Models => render_models(state, content_area, buf),
    }
    Paragraph::new(Line::from(vec![Span::styled(
        " tab/←/→ switch tabs  ·  r cycle range  ·  ↑↓ scroll",
        Style::default()
            .fg(OPERANT_MUTED)
            .add_modifier(Modifier::ITALIC),
    )]))
    .render(layout.footer_area, buf);
}

fn tab_span(label: &str, active: bool) -> Span<'static> {
    if active {
        Span::styled(
            label.to_string(),
            Style::default()
                .fg(OPERANT_ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
    } else {
        Span::styled(label.to_string(), Style::default().fg(OPERANT_MUTED))
    }
}

// ---------------------------------------------------------------------------
// Overview tab
// ---------------------------------------------------------------------------

fn render_overview(data: &AggregatedStats, state: &StatsDialogState, area: Rect, buf: &mut Buffer) {
    let total_tokens = data.total_input_tokens + data.total_output_tokens;
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("Total tokens: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_tokens(total_tokens),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Input:    ", Style::default().fg(Color::DarkGray)),
        Span::raw(format_tokens(data.total_input_tokens)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Output:   ", Style::default().fg(Color::DarkGray)),
        Span::raw(format_tokens(data.total_output_tokens)),
    ]));
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("Total cost: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.2}", data.total_cost_cents / 100.0),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Streak display
    lines.push(Line::default());
    {
        let current = state.current_streak_days;
        let longest = state.longest_streak_days;
        let streak_value = Span::styled(
            format!("● {} day{}", current, if current == 1 { "" } else { "s" }),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        let streak_longest = Span::styled(
            format!(
                "  (longest: {} day{})",
                longest,
                if longest == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::DarkGray),
        );
        lines.push(Line::from(vec![
            Span::styled("Streak: ", Style::default().fg(Color::DarkGray)),
            streak_value,
            streak_longest,
        ]));
    }

    if let Some(peak) = &data.peak_day {
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("Peak day: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} ({} tokens)", peak, format_tokens(data.peak_day_tokens)),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }

    if !data.by_model.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            "By model:",
            Style::default().fg(Color::DarkGray),
        )]));
        let mut models: Vec<_> = data.by_model.iter().collect();
        models.sort_by(|a, b| {
            b.1.cost_cents
                .partial_cmp(&a.1.cost_cents)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (model, stats) in models.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:40} ", model), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!(
                        "{} turns  {}",
                        stats.turns,
                        format_tokens(stats.input_tokens + stats.output_tokens)
                    ),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("  ${:.2}", stats.cost_cents / 100.0),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    Paragraph::new(lines).render(area, buf);
}

// ---------------------------------------------------------------------------
// Daily Tokens tab
// ---------------------------------------------------------------------------

fn render_daily_tokens(data: &AggregatedStats, range_days: u32, area: Rect, buf: &mut Buffer) {
    // Filter to range
    let filtered: Vec<_> = if range_days == 0 {
        data.daily_tokens.iter().collect()
    } else {
        data.daily_tokens
            .iter()
            .rev()
            .take(range_days as usize)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    };

    if filtered.is_empty() {
        Paragraph::new("No data yet.")
            .style(Style::default().fg(Color::DarkGray))
            .render(area, buf);
        return;
    }

    let range_label = match range_days {
        7 => "7 days",
        30 => "30 days",
        _ => "all time",
    };
    let label_line = Line::from(vec![Span::styled(
        format!("Range: {} [r: cycle]", range_label),
        Style::default().fg(Color::DarkGray),
    )]);
    Paragraph::new(label_line).render(
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
        buf,
    );

    let chart_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(2),
    };

    // Build bar chart data
    let max_val = filtered.iter().map(|d| d.1).max().unwrap_or(1).max(1);
    let bar_data: Vec<(&str, u64)> = filtered
        .iter()
        .map(|d| {
            let label: &str = if d.0.len() >= 5 {
                &d.0[5..]
            } else {
                d.0.as_str()
            };
            (label, d.1 * (chart_area.height as u64 - 1) / max_val)
        })
        .collect();

    // Render ASCII bar chart manually (ratatui BarChart needs 'static strs)
    for (i, (label, height)) in bar_data.iter().enumerate() {
        let x = chart_area.x + i as u16 * 6;
        if x + 5 >= chart_area.x + chart_area.width {
            break;
        }
        let bar_height = (*height as u16).min(chart_area.height.saturating_sub(1));
        for row in 0..bar_height {
            let y = chart_area.y + chart_area.height - 1 - row;
            let cell = buf.cell_mut((x + 1, y));
            if let Some(c) = cell {
                c.set_symbol("\u{2588}");
                c.set_style(Style::default().fg(Color::Cyan));
            }
            let cell2 = buf.cell_mut((x + 2, y));
            if let Some(c) = cell2 {
                c.set_symbol("\u{2588}");
                c.set_style(Style::default().fg(Color::Cyan));
            }
        }
        // Label
        let y = chart_area.y + chart_area.height - 1;
        let label_short: String = label.chars().take(4).collect();
        for (j, ch) in label_short.chars().enumerate() {
            let cell = buf.cell_mut((x + j as u16, y));
            if let Some(c) = cell {
                c.set_symbol(&ch.to_string());
                c.set_style(Style::default().fg(Color::DarkGray));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cost Heatmap tab (GitHub-style)
// ---------------------------------------------------------------------------

fn render_cost_heatmap(data: &AggregatedStats, area: Rect, buf: &mut Buffer) {
    if data.daily_costs.is_empty() {
        Paragraph::new("No cost data yet.")
            .style(Style::default().fg(Color::DarkGray))
            .render(area, buf);
        return;
    }

    let max_cost = data
        .daily_costs
        .values()
        .cloned()
        .fold(0.0_f64, f64::max)
        .max(0.01);

    // Header legend
    Paragraph::new(Line::from(vec![
        Span::styled(
            "Cost Heatmap (last 12 weeks)   no activity ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("\u{25a0}", Style::default().fg(Color::Rgb(30, 30, 30))),
        Span::styled(" low ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{25a0}", Style::default().fg(Color::Rgb(0, 100, 0))),
        Span::styled(" med ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{25a0}", Style::default().fg(Color::Rgb(0, 200, 0))),
        Span::styled(" high ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{25a0}", Style::default().fg(Color::Rgb(0, 255, 0))),
    ]))
    .render(
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
        buf,
    );

    // Weekday labels column (Mon..Sun order)
    let weekday_labels = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let heatmap_area = Rect {
        x: area.x + 4, // leave 4 cols for "Mon" etc.
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(3),
    };

    for (i, label) in weekday_labels.iter().enumerate() {
        let y = heatmap_area.y + i as u16;
        if y >= heatmap_area.y + heatmap_area.height {
            break;
        }
        Paragraph::new(Line::from(vec![Span::styled(
            label.to_string(),
            Style::default().fg(Color::DarkGray),
        )]))
        .render(
            Rect {
                x: area.x,
                y,
                width: 3,
                height: 1,
            },
            buf,
        );
    }

    // 12 weeks x 7 days grid — sorted ascending, display newest on right
    let sorted_dates: Vec<_> = {
        let mut v: Vec<_> = data.daily_costs.iter().collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v
    };

    // We group into chunks of 7 calendar days (by index, as in the original)
    // and place week columns right-to-left from the most-recent week.
    let chunks: Vec<_> = sorted_dates.chunks(7).collect();
    let total_chunks = chunks.len();
    let start_chunk = total_chunks.saturating_sub(12);

    for (display_col, chunk) in chunks[start_chunk..].iter().enumerate() {
        let x = heatmap_area.x + display_col as u16 * 2;
        if x >= heatmap_area.x + heatmap_area.width {
            break;
        }
        for (day_idx, (_, cost)) in chunk.iter().enumerate() {
            let y = heatmap_area.y + day_idx as u16;
            if y >= heatmap_area.y + heatmap_area.height {
                break;
            }
            let intensity = (*cost / max_cost).min(1.0);
            let color = heatmap_color(intensity);
            let cell = buf.cell_mut((x, y));
            if let Some(c) = cell {
                c.set_symbol("\u{25a0}");
                c.set_style(Style::default().fg(color));
            }
        }
    }
}

/// Map a 0..=1 intensity to a green-shade color matching the GitHub heatmap spec.
pub(crate) fn heatmap_color(intensity: f64) -> Color {
    if intensity < 0.01 {
        Color::Rgb(30, 30, 30)
    } else if intensity < 0.25 {
        Color::Rgb(0, 100, 0)
    } else if intensity < 0.50 {
        Color::Rgb(0, 150, 0)
    } else if intensity < 0.75 {
        Color::Rgb(0, 200, 0)
    } else {
        Color::Rgb(0, 255, 0)
    }
}

// ---------------------------------------------------------------------------
// Models tab
// ---------------------------------------------------------------------------

fn render_models(state: &StatsDialogState, area: Rect, buf: &mut Buffer) {
    if state.model_breakdown.is_empty() {
        Paragraph::new("No model usage data yet.")
            .style(Style::default().fg(Color::DarkGray))
            .render(area, buf);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Table header
    lines.push(Line::from(vec![Span::styled(
        format!(
            "{:<42} {:>12} {:>13} {:>10}",
            "Model", "Input", "Output", "Cost"
        ),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));
    // Separator
    lines.push(Line::from(vec![Span::styled(
        "\u{2500}".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(Color::DarkGray),
    )]));

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cost: f64 = 0.0;

    for entry in &state.model_breakdown {
        total_input += entry.input_tokens;
        total_output += entry.output_tokens;
        total_cost += entry.cost_usd;

        // Truncate long model IDs
        let model_display = if entry.model_id.len() > 42 {
            format!("{}...", &entry.model_id[..39])
        } else {
            entry.model_id.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<42} ", model_display),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("{:>12} ", format_tokens(entry.input_tokens)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>13} ", format_tokens(entry.output_tokens)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>9}", format!("${:.4}", entry.cost_usd)),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }

    // Grand total separator + row
    lines.push(Line::from(vec![Span::styled(
        "\u{2500}".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:<42} ", "TOTAL"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>12} ", format_tokens(total_input)),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>13} ", format_tokens(total_output)),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>9}", format!("${:.4}", total_cost)),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    Paragraph::new(lines).render(area, buf);
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
