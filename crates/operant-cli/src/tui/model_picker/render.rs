// model_picker/render.rs — Model picker dialog rendering.
//
// Extracted from the model_picker.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

pub fn render_model_picker(state: &ModelPickerState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    use ratatui::prelude::Stylize;

    let _pink = Color::Rgb(255, 191, 0);
    let dim = Color::Rgb(90, 90, 90);
    let dialog_bg = OPERANT_PANEL_BG;
    let highlight_bg = Color::Rgb(255, 191, 0);
    let highlight_fg = Color::White;

    // ── Dark overlay ──
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_bg(Color::Rgb(10, 10, 14));
                cell.set_fg(Color::Rgb(40, 40, 45));
            }
        }
    }

    // ── Dialog size ──
    let width = 65u16.min(area.width.saturating_sub(6));
    let max_height = (area.height as f32 * 0.75) as u16;
    let filtered = state.filtered_models();
    let content_h = (filtered.len() as u16 + 6).min(max_height).max(8);
    let dialog_area = centered_rect(width, content_h, area);

    // ── Fill dialog bg (no border) ──
    for y in dialog_area.y..dialog_area.y + dialog_area.height {
        for x in dialog_area.x..dialog_area.x + dialog_area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_bg(dialog_bg);
                cell.set_fg(Color::White);
            }
        }
    }

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    let footer_height = 1u16.min(inner.height);
    let header_height = 3u16.min(inner.height.saturating_sub(footer_height));
    let header_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: header_height,
    };
    let body_area = Rect {
        x: inner.x,
        y: inner.y.saturating_add(header_height),
        width: inner.width,
        height: inner.height.saturating_sub(header_height + footer_height),
    };
    let footer_area = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(footer_height),
        width: inner.width,
        height: footer_height,
    };

    // ── Fixed header ──
    let mut header_lines: Vec<Line> = Vec::new();

    // Title row: "Select model" left, "esc" right
    let title_pad = inner.width.saturating_sub(state.title.len() as u16 + 5) as usize;
    header_lines.push(Line::from(vec![
        Span::styled(
            format!(" {}", state.title),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>w$}", "esc ", w = title_pad),
            Style::default().fg(dim),
        ),
    ]));

    // Search field
    header_lines.push(Line::from(""));
    header_lines.push(modal_search_line(
        &state.filter,
        "Search",
        dim,
        Color::White,
    ));

    let header_para = Paragraph::new(header_lines).bg(dialog_bg);
    header_para.render(header_area, buf);

    if body_area.height == 0 {
        return;
    }

    // ── Model items ──
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line_idx: u16 = 0;

    if state.fast_mode {
        lines.push(Line::from(vec![Span::styled(
            format!(
                " \u{26a1} Fast mode ON ({})",
                state.fast_mode_model.as_deref().unwrap_or("current model")
            ),
            Style::default().fg(Color::Yellow),
        )]));
    }

    if state.loading_models {
        lines.push(Line::from(vec![Span::styled(
            " Loading models\u{2026}",
            Style::default().fg(dim),
        )]));
    }

    if !lines.is_empty() {
        lines.push(Line::from(""));
    }

    if filtered.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " No results found",
            Style::default().fg(dim),
        )]));
        if !state.filter.trim().is_empty() {
            lines.push(Line::from(vec![Span::styled(
                " Press Enter to use custom model",
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )]));
        }
    } else {
        for (i, model) in filtered.iter().enumerate() {
            let is_selected = i == state.selected_idx;
            let supports_effort = model_supports_effort(&model.id);

            if is_selected {
                selected_line_idx = lines.len() as u16;
            }

            let (fg, bg) = if is_selected {
                (highlight_fg, highlight_bg)
            } else {
                (Color::White, dialog_bg)
            };

            let mut spans: Vec<Span<'static>> = Vec::new();

            // Current model indicator
            if model.is_current {
                spans.push(Span::styled(
                    " \u{25cf} ",
                    Style::default().fg(Color::Green).bg(bg),
                ));
            } else {
                spans.push(Span::styled("   ", Style::default().bg(bg)));
            }

            spans.push(Span::styled(
                model.display_name.clone(),
                Style::default().fg(fg).bg(bg),
            ));

            // Effort indicator
            if supports_effort && is_selected {
                spans.push(Span::styled(
                    format!(
                        "  {} {}",
                        state.effort_level.symbol(),
                        state.effort_level.label()
                    ),
                    Style::default().fg(Color::Rgb(200, 255, 200)).bg(bg),
                ));
            }

            // Description
            if !model.description.is_empty() {
                let desc_fg = if is_selected {
                    Color::Rgb(200, 200, 200)
                } else {
                    dim
                };
                spans.push(Span::styled(
                    format!("  {}", model.description),
                    Style::default().fg(desc_fg).bg(bg),
                ));
            }

            // Pad for full-width highlight
            if is_selected {
                let text_len: usize = spans.iter().map(|s| s.content.len()).sum();
                let pad = inner.width.saturating_sub(text_len as u16) as usize;
                if pad > 0 {
                    spans.push(Span::styled(
                        " ".repeat(pad),
                        Style::default().bg(highlight_bg),
                    ));
                }
            }

            lines.push(Line::from(spans));
        }
    }

    // ── Scroll ──
    let total_lines = lines.len() as u16;
    let visible = body_area.height;
    let scroll_y = if total_lines <= visible {
        0u16
    } else if selected_line_idx + 3 >= visible {
        (selected_line_idx + 3).saturating_sub(visible)
    } else {
        0
    };

    let para = Paragraph::new(lines).bg(dialog_bg).scroll((scroll_y, 0));

    para.render(body_area, buf);

    let mut footer_spans = vec![
        Span::styled(" enter", Style::default().fg(dim)),
        Span::styled(" select", Style::default().fg(dim)),
    ];
    if let Some(model) = filtered.get(state.selected_idx)
        && model_supports_effort(&model.id)
    {
        footer_spans.push(Span::raw("  "));
        footer_spans.push(Span::styled("\u{2190}/\u{2192}", Style::default().fg(dim)));
        footer_spans.push(Span::styled(" effort", Style::default().fg(dim)));
    }
    footer_spans.push(Span::raw("  "));
    footer_spans.push(Span::styled("Esc", Style::default().fg(dim)));
    footer_spans.push(Span::styled(" close", Style::default().fg(dim)));
    Paragraph::new(Line::from(footer_spans))
        .bg(dialog_bg)
        .render(footer_area, buf);
}
