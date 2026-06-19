use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::skin;
use crate::tui::state::Tone;

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayType {
    None,
    Approval(ApprovalData),
    Clarify(ClarifyData),
    Confirm(ConfirmData),
    ModelPicker,
    SessionBrowser,
    McpBrowser,
    Help,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalData {
    pub tool_name: String,
    pub arguments: String,
    pub risk_level: String,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClarifyData {
    pub question: String,
    pub options: Vec<String>,
    pub selected: usize,
    pub free_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmData {
    pub message: String,
    pub selected: bool,
}

pub fn render_approval_overlay(
    frame: &mut Frame,
    area: Rect,
    data: &ApprovalData,
    accent: Color,
    muted: Color,
) {
    let area = centered_rect(60, 40, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Approval Required ")
        .borders(Borders::ALL)
        .style(Style::default().fg(accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Tool: {}", data.tool_name),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(Span::styled(
        format!("Risk: {}", data.risk_level),
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(""));

    let args = if data.arguments.len() > 100 {
        format!("{}...", &data.arguments[..97])
    } else {
        data.arguments.clone()
    };
    lines.push(Line::from(Span::styled(
        format!("Arguments: {}", args),
        Style::default().fg(muted),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "1. Allow Once",
        Style::default().fg(if data.selected == 0 { accent } else { Color::White }),
    )));
    lines.push(Line::from(Span::styled(
        "2. Allow Always",
        Style::default().fg(if data.selected == 1 { accent } else { Color::White }),
    )));
    lines.push(Line::from(Span::styled(
        "3. Deny",
        Style::default().fg(if data.selected == 2 { accent } else { Color::White }),
    )));

    let paragraph = Paragraph::new(Text::from(lines));
    frame.render_widget(paragraph, inner);
}

pub fn render_clarify_overlay(
    frame: &mut Frame,
    area: Rect,
    data: &ClarifyData,
    accent: Color,
    muted: Color,
) {
    let area = centered_rect(70, 50, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Clarification Needed ")
        .borders(Borders::ALL)
        .style(Style::default().fg(accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        &data.question,
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));

    for (i, option) in data.options.iter().enumerate() {
        let style = if i == data.selected {
            Style::default().fg(accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("{}. {}", i + 1, option),
            style,
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Input: {}", data.free_text),
        Style::default().fg(muted),
    )));

    let paragraph = Paragraph::new(Text::from(lines));
    frame.render_widget(paragraph, inner);
}

pub fn render_confirm_overlay(
    frame: &mut Frame,
    area: Rect,
    data: &ConfirmData,
    accent: Color,
    muted: Color,
) {
    let area = centered_rect(50, 30, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .style(Style::default().fg(accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        &data.message,
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        if data.selected { "[Yes]  No" } else { "Yes  [No]" },
        Style::default().fg(accent),
    )));

    let paragraph = Paragraph::new(Text::from(lines));
    frame.render_widget(paragraph, inner);
}

pub fn render_help_overlay(frame: &mut Frame, area: Rect, accent: Color, muted: Color) {
    let area = centered_rect(70, 70, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .style(Style::default().fg(accent));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("Ctrl+C", Style::default().fg(accent))),
        Line::from(Span::raw("  Interrupt / Copy")),
        Line::from(Span::styled("Ctrl+D", Style::default().fg(accent))),
        Line::from(Span::raw("  Exit")),
        Line::from(Span::styled("Ctrl+L", Style::default().fg(accent))),
        Line::from(Span::raw("  Clear / Redraw")),
        Line::from(Span::styled("Ctrl+R", Style::default().fg(accent))),
        Line::from(Span::raw("  History search")),
        Line::from(Span::styled("Tab", Style::default().fg(accent))),
        Line::from(Span::raw("  Cycle panels")),
        Line::from(Span::styled("Enter", Style::default().fg(accent))),
        Line::from(Span::raw("  Send message")),
        Line::from(Span::styled("Escape", Style::default().fg(accent))),
        Line::from(Span::raw("  Cancel / Back")),
        Line::from(Span::styled("Shift+Enter", Style::default().fg(accent))),
        Line::from(Span::raw("  New line")),
        Line::from(""),
        Line::from(Span::styled(
            "Press Escape to close",
            Style::default().fg(muted),
        )),
    ];

    let paragraph = Paragraph::new(Text::from(lines));
    frame.render_widget(paragraph, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

pub fn handle_overlay_key(
    overlay: &mut OverlayType,
    key: KeyEvent,
) -> Option<OverlayAction> {
    match overlay {
        OverlayType::None => None,
        OverlayType::Approval(data) => match key.code {
            KeyCode::Char('1') => Some(OverlayAction::ApprovalAllowOnce),
            KeyCode::Char('2') => Some(OverlayAction::ApprovalAllowAlways),
            KeyCode::Char('3') => Some(OverlayAction::ApprovalDeny),
            KeyCode::Up => {
                if data.selected > 0 {
                    data.selected -= 1;
                }
                None
            }
            KeyCode::Down => {
                if data.selected < 2 {
                    data.selected += 1;
                }
                None
            }
            KeyCode::Enter => Some(match data.selected {
                0 => OverlayAction::ApprovalAllowOnce,
                1 => OverlayAction::ApprovalAllowAlways,
                _ => OverlayAction::ApprovalDeny,
            }),
            KeyCode::Esc => Some(OverlayAction::Close),
            _ => None,
        },
        OverlayType::Clarify(data) => match key.code {
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let idx = c as usize - 1;
                if idx < data.options.len() {
                    Some(OverlayAction::ClarifyOption(idx))
                } else {
                    None
                }
            }
            KeyCode::Up => {
                if data.selected > 0 {
                    data.selected -= 1;
                }
                None
            }
            KeyCode::Down => {
                if data.selected < data.options.len() {
                    data.selected += 1;
                }
                None
            }
            KeyCode::Enter => {
                if data.selected < data.options.len() {
                    Some(OverlayAction::ClarifyOption(data.selected))
                } else {
                    Some(OverlayAction::ClarifyFreeText(data.free_text.clone()))
                }
            }
            KeyCode::Char(c) => {
                data.free_text.push(c);
                None
            }
            KeyCode::Backspace => {
                data.free_text.pop();
                None
            }
            KeyCode::Esc => Some(OverlayAction::Close),
            _ => None,
        },
        OverlayType::Confirm(data) => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(OverlayAction::ConfirmYes),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(OverlayAction::ConfirmNo),
            KeyCode::Left | KeyCode::Right => {
                data.selected = !data.selected;
                None
            }
            KeyCode::Enter => Some(if data.selected {
                OverlayAction::ConfirmYes
            } else {
                OverlayAction::ConfirmNo
            }),
            KeyCode::Esc => Some(OverlayAction::Close),
            _ => None,
        },
        OverlayType::Help => match key.code {
            KeyCode::Esc => Some(OverlayAction::Close),
            _ => None,
        },
        _ => match key.code {
            KeyCode::Esc => Some(OverlayAction::Close),
            _ => None,
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayAction {
    Close,
    ApprovalAllowOnce,
    ApprovalAllowAlways,
    ApprovalDeny,
    ClarifyOption(usize),
    ClarifyFreeText(String),
    ConfirmYes,
    ConfirmNo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_type_default() {
        let overlay = OverlayType::None;
        assert_eq!(overlay, OverlayType::None);
    }

    #[test]
    fn test_approval_data() {
        let data = ApprovalData {
            tool_name: "read_file".to_string(),
            arguments: "{}".to_string(),
            risk_level: "low".to_string(),
            selected: 0,
        };
        assert_eq!(data.tool_name, "read_file");
    }

    #[test]
    fn test_clarify_data() {
        let data = ClarifyData {
            question: "What do you mean?".to_string(),
            options: vec!["Option A".to_string(), "Option B".to_string()],
            selected: 0,
            free_text: String::new(),
        };
        assert_eq!(data.options.len(), 2);
    }
}
