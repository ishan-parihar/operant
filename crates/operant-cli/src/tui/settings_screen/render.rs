// settings_screen/render.rs — Settings screen rendering.
//
// Extracted from the settings_screen.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

pub fn render_settings_screen(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    if !screen.visible {
        return;
    }

    render_dark_overlay(frame, area);

    // 80% width, 90% height, centred
    let w = (area.width * 4 / 5)
        .max(60)
        .min(area.width.saturating_sub(2));
    let h = (area.height * 9 / 10)
        .max(20)
        .min(area.height.saturating_sub(2));
    let popup = centered_rect(w, h, area);
    render_dialog_bg(frame, popup);

    // Inset inner area
    let inner = Rect {
        x: popup.x + 2,
        y: popup.y + 1,
        width: popup.width.saturating_sub(4),
        height: popup.height.saturating_sub(2),
    };

    if inner.height < 6 {
        return;
    }

    // Split into header + search + spacer + content + description + footer
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Percentage(50),
            Constraint::Length(1),
        ])
        .split(inner);

    let header_area = layout[0];
    let search_area = layout[1];
    let content_area = layout[3];
    let description_area = layout[4];
    let footer_area = layout[5];

    // Header
    let title = Line::from(vec![
        Span::styled(
            " Settings",
            Style::default()
                .fg(OPERANT_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" — Operant", Style::default().fg(OPERANT_MUTED)),
        Span::styled(
            format!(
                "{:>width$}",
                "Esc close",
                width = inner.width.saturating_sub(19) as usize
            ),
            Style::default().fg(OPERANT_MUTED),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title).style(Style::default().bg(OPERANT_PANEL_BG)),
        header_area,
    );

    // Search
    let search_line = modal_search_line(
        &screen.search_query,
        "Type to search settings...",
        Color::DarkGray,
        OPERANT_ACCENT,
    );
    frame.render_widget(
        Paragraph::new(search_line).style(Style::default().bg(OPERANT_PANEL_BG)),
        search_area,
    );

    // Content
    render_settings_list(frame, screen, content_area);

    // Description of selected entry
    let all = all_entries(screen);
    let filtered: Vec<_> = all
        .iter()
        .filter(|e| {
            e.label
                .to_lowercase()
                .contains(&screen.search_query.to_lowercase())
        })
        .collect();

    let desc_text = if let Some(entry) = filtered.get(screen.selected_idx) {
        // For Output Style, show current selection and all available options with descriptions
        if entry.key == "output_style" {
            let mut lines = vec![entry.description.to_string(), String::new()];

            let all_styles = builtin_styles();
            let current_style_name = if screen.output_style.is_empty() {
                "default"
            } else {
                &screen.output_style
            };
            if let Some(current_style) = find_style(&all_styles, current_style_name) {
                lines.push(format!(
                    "Current: {} — {}",
                    current_style.label, current_style.description
                ));
                lines.push(String::new());
            }

            lines.push("Available:".to_string());
            for style in builtin_styles() {
                lines.push(format!("  {} — {}", style.name, style.description));
            }
            lines.join("\n")
        } else {
            entry.description.to_string()
        }
    } else {
        String::new()
    };
    let desc_para = Paragraph::new(desc_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Left)
        .block(Block::default().padding(ratatui::widgets::Padding::new(1, 0, 1, 0)));
    frame.render_widget(desc_para, description_area);

    // Footer
    let footer = if screen.edit_field.is_some() {
        Line::from(vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(OPERANT_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("save  "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("cancel"),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " ↑↓ ",
                Style::default()
                    .fg(OPERANT_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("navigate  "),
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(OPERANT_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("toggle/edit  "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("close"),
        ])
    };
    let footer_para = Paragraph::new(vec![footer])
        .style(Style::default().fg(OPERANT_MUTED).bg(OPERANT_PANEL_BG))
        .alignment(Alignment::Center);
    frame.render_widget(footer_para, footer_area);
}

fn render_settings_list(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    let all = all_entries(screen);

    // Filter entries by search query
    let filtered: Vec<_> = all
        .iter()
        .filter(|e| {
            e.label
                .to_lowercase()
                .contains(&screen.search_query.to_lowercase())
        })
        .collect();

    if filtered.is_empty() {
        let para = Paragraph::new("No settings match your search.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(para, area);
        return;
    }

    // Build lines
    let mut lines: Vec<Line> = Vec::new();
    let visible_rows = area.height as usize;

    for (i, entry) in filtered.iter().enumerate() {
        let is_selected = i == screen.selected_idx;
        let marker = if is_selected { "►" } else { " " };

        let label_len = 40usize;

        // Show edit value if currently editing this field, otherwise show the entry value
        let value_str = if screen.edit_field.as_deref() == Some(entry.key) && is_selected {
            format!("{}_ ", screen.edit_value) // Add cursor indicator
        } else {
            entry.value.clone()
        };

        let row_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(OPERANT_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let line = Line::from(vec![
            Span::styled(
                format!("   {} {:<label_len$}", marker, entry.label),
                row_style,
            ),
            Span::styled(value_str, row_style),
        ]);
        lines.push(line);
    }

    // Scroll tracking is handled in update_scroll_offset_for_selection()

    // Apply manual scrolling
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(screen.scroll_offset)
        .take(visible_rows.max(1))
        .collect();

    let para = Paragraph::new(visible_lines);
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------
