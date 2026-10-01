// model_picker/render.rs — Model picker dialog rendering.
//
// Extracted from the model_picker.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::tui::overlays::{HINT_ESC, ModalSpec, modal_frame_buf};

pub fn render_model_picker(state: &ModelPickerState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    use ratatui::prelude::Stylize;

    let dim = theme_colors::DIALOG_DIM;
    let dialog_bg = theme_colors::panel_bg();
    let highlight_bg = theme_colors::selection_bg();
    let highlight_fg = theme_colors::text();

    // ── Dialog size ──
    // The window is sized from the filtered-model count, and from that count
    // alone, *before* any row is built — so on a short terminal the dialog is
    // still bounded to what fits and the selected model stays visible. The
    // filter is resolved once here and reused by the rows below.
    let filtered = state.filtered_models();
    let desired_height = ((filtered.len() as u16).saturating_add(6))
        .min((area.height as f32 * 0.75) as u16)
        .max(8);

    // ── Frame (overlay, background, rounded border, title, hint) ──
    // +6 above is this dialog's own chrome: two border rows, a three-row header
    // (title, blank, search) and a one-row footer.
    let layout = modal_frame_buf(
        buf,
        area,
        &ModalSpec {
            title: &state.title,
            hint: HINT_ESC,
            width: 65,
            height: desired_height,
            header_height: 3,
            footer_height: 1,
            ..Default::default()
        },
    );

    // ── Fixed header: the blank row + the search field ──
    // The title itself is painted by `modal_frame_buf` on header row 0.
    let search_area = Rect {
        height: 2,
        ..layout.header_area
    };
    let header_para = Paragraph::new(vec![
        Line::from(""),
        modal_search_line(&state.filter, "Search", dim, theme_colors::text()),
    ])
    .bg(dialog_bg);
    header_para.render(search_area, buf);

    let body_area = layout.body_area;

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
            Style::default().fg(theme_colors::warning()),
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
                Style::default().fg(theme_colors::DIALOG_TEXT_BRIGHT),
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
                (theme_colors::text(), dialog_bg)
            };

            let mut spans: Vec<Span<'static>> = Vec::new();

            // Current model indicator
            if model.is_current {
                spans.push(Span::styled(
                    " \u{25cf} ",
                    Style::default().fg(theme_colors::success()).bg(bg),
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
                    // The effort chip sits on the selected row's background, so its
                    // foreground is the role for text on a selection background.
                    // A pale literal here measures ~1.9:1 on `selection_bg`.
                    Style::default().fg(theme_colors::on_selection()).bg(bg),
                ));
            }

            // Description
            if !model.description.is_empty() {
                let desc_fg = if is_selected {
                    theme_colors::DIALOG_TEXT_BRIGHT
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
                let pad = body_area.width.saturating_sub(text_len as u16) as usize;
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

    // The dismissal hint now lives on the bottom border as `HINT_ESC`, so this
    // row keeps the action prose only.
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
    Paragraph::new(Line::from(footer_spans))
        .bg(dialog_bg)
        .render(layout.footer_area, buf);
}
