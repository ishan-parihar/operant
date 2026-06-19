use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::skin;

pub struct ModalLayout {
    pub dialog_area: Rect,
    pub inner_area: Rect,
    pub header_area: Rect,
    pub body_area: Rect,
    pub footer_area: Rect,
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn render_dark_overlay(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let bg = Color::Rgb(0, 0, 0);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                cell.set_bg(bg);
            }
        }
    }
}

pub fn render_dialog_bg(frame: &mut Frame, area: Rect, title: &str) {
    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().fg(skin::get_active().accent()));
    frame.render_widget(block, area);
}

pub fn begin_modal_frame(
    frame: &mut Frame,
    area: Rect,
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    let dialog_area = centered_rect(width, height, area);
    render_dark_overlay(frame, area);

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .split(dialog_area);

    ModalLayout {
        dialog_area,
        inner_area: inner[1],
        header_area: inner[0],
        body_area: inner[1],
        footer_area: inner[2],
    }
}

pub fn modal_title_line(title: &str, right_hint: Option<&str>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(skin::get_active().accent())
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(hint) = right_hint {
        spans.push(Span::styled(
            format!(" {} ", hint),
            Style::default().fg(skin::get_active().muted()),
        ));
    }
    Line::from(spans)
}

pub fn modal_search_line(query: &str, placeholder: &str) -> Line<'static> {
    let display = if query.is_empty() {
        placeholder.to_string()
    } else {
        query.to_string()
    };
    Line::from(vec![
        Span::styled(" 🔍 ", Style::default().fg(skin::get_active().accent())),
        Span::styled(display, Style::default().fg(skin::get_active().text())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_returns_valid_rect() {
        let area = Rect::new(0, 0, 100, 50);
        let centered = centered_rect(50, 50, area);
        assert!(centered.width > 0);
        assert!(centered.height > 0);
        assert!(centered.x > 0);
        assert!(centered.y > 0);
    }

    #[test]
    fn modal_title_line_contains_title() {
        let line = modal_title_line("Test Title", None);
        assert!(line.spans.iter().any(|s| s.content.contains("Test Title")));
    }

    #[test]
    fn modal_search_line_with_placeholder() {
        let line = modal_search_line("", "Search...");
        assert!(line.spans.iter().any(|s| s.content.contains("Search...")));
    }
}
