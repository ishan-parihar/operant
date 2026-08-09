// overlays/message_selector.rs — Message selector used by /rewind step 1.
//
// Extracted from the overlays.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

// ============================================================================
// MessageSelectorOverlay
// ============================================================================

/// A single entry shown in the message selector list.
#[derive(Debug, Clone)]
pub struct SelectorMessage {
    /// Original index in the conversation.
    pub idx: usize,
    pub role: String,
    /// First ~80 chars of content.
    pub preview: String,
    pub has_tool_use: bool,
}

/// State for the message selector overlay used by /rewind step 1.
#[derive(Debug, Default)]
pub struct MessageSelectorOverlay {
    pub visible: bool,
    pub messages: Vec<SelectorMessage>,
    pub selected_idx: usize,
    pub scroll_offset: usize,
}

impl MessageSelectorOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(messages: Vec<SelectorMessage>) -> Self {
        // Start with selection at the end (most recent)
        let selected = messages.len().saturating_sub(1);
        Self {
            visible: true,
            messages,
            selected_idx: selected,
            scroll_offset: selected.saturating_sub(5),
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        const VISIBLE_ROWS: usize = 12;
        let count = self.messages.len();
        if count == 0 {
            return;
        }
        if self.selected_idx == 0 {
            self.selected_idx = count - 1;
            self.scroll_offset = count.saturating_sub(VISIBLE_ROWS);
        } else {
            self.selected_idx -= 1;
            if self.selected_idx < self.scroll_offset {
                self.scroll_offset = self.selected_idx;
            }
        }
    }

    pub fn select_next(&mut self) {
        const VISIBLE_ROWS: usize = 12;
        let count = self.messages.len();
        if count == 0 {
            return;
        }
        if self.selected_idx + 1 >= count {
            self.selected_idx = 0;
            self.scroll_offset = 0;
        } else {
            self.selected_idx += 1;
            if self.selected_idx >= self.scroll_offset + VISIBLE_ROWS {
                self.scroll_offset = self.selected_idx - VISIBLE_ROWS + 1;
            }
        }
    }

    pub fn current_message(&self) -> Option<&SelectorMessage> {
        self.messages.get(self.selected_idx)
    }
}

/// Render the message selector overlay.
pub fn render_message_selector(frame: &mut Frame, overlay: &MessageSelectorOverlay, area: Rect) {
    if !overlay.visible {
        return;
    }

    const VISIBLE_ROWS: usize = 12;
    let dialog_width = 70u16.min(area.width.saturating_sub(4));
    let rows = VISIBLE_ROWS.min(overlay.messages.len().max(1)) as u16;
    let dialog_height = (rows + 4).min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    frame.render_widget(Clear, dialog_area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![Span::styled(
        "  Select a message to rewind to:",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    if overlay.messages.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  (no messages)",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        let start = overlay.scroll_offset;
        let end = (start + VISIBLE_ROWS).min(overlay.messages.len());

        for (display_i, msg) in overlay.messages[start..end].iter().enumerate() {
            let real_i = start + display_i;
            let is_selected = real_i == overlay.selected_idx;

            let role_color = if msg.role == "user" {
                Color::Cyan
            } else {
                Color::Green
            };

            let tool_tag = if msg.has_tool_use { " [tool]" } else { "" };

            let preview_max = dialog_width as usize - 20;
            let preview = if UnicodeWidthStr::width(msg.preview.as_str()) > preview_max {
                format!("{}…", &msg.preview[..preview_max.saturating_sub(1)])
            } else {
                msg.preview.clone()
            };

            let prefix = if is_selected { "  \u{25BA} " } else { "    " };
            let idx_style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("{:>3}. ", msg.idx), idx_style),
                Span::styled(
                    format!("{:<10}", msg.role),
                    Style::default()
                        .fg(role_color)
                        .add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    preview,
                    if is_selected {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(tool_tag.to_string(), Style::default().fg(Color::Yellow)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  ↑↓ navigate  ·  Enter to select  ·  Esc to cancel",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )]));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Rewind — Select Message ")
        .border_style(Style::default().fg(Color::Yellow));

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, dialog_area);
}
