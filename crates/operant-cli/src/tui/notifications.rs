use std::collections::VecDeque;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::tui::skin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: String,
    pub kind: NotificationKind,
    pub message: String,
    pub pushed_at: Instant,
    pub expires_at: Option<Instant>,
    pub dismissible: bool,
}

#[derive(Debug, Default)]
pub struct NotificationQueue {
    pub notifications: VecDeque<Notification>,
    next_id: u64,
}

impl Clone for NotificationQueue {
    fn clone(&self) -> Self {
        Self {
            notifications: self.notifications.clone(),
            next_id: self.next_id,
        }
    }
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self {
            notifications: VecDeque::new(),
            next_id: 0,
        }
    }

    pub fn push(&mut self, kind: NotificationKind, msg: String, duration_secs: Option<u64>) {
        let pushed_at = Instant::now();
        let expires_at = duration_secs.map(|secs| pushed_at + std::time::Duration::from_secs(secs));
        self.notifications
            .retain(|n| !(n.kind == kind && n.message == msg));
        let id = format!("notif-{}", self.next_id);
        self.next_id += 1;
        self.notifications.push_back(Notification {
            id,
            kind,
            message: msg,
            pushed_at,
            expires_at,
            dismissible: true,
        });
    }

    pub fn dismiss(&mut self, id: &str) {
        self.notifications.retain(|n| n.id != id);
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        self.notifications.retain(|n| {
            n.expires_at.map_or(true, |exp| exp > now)
        });
    }

    pub fn current(&self) -> Option<&Notification> {
        self.notifications.back()
    }

    pub fn dismiss_current(&mut self) {
        if let Some(n) = self.notifications.back().cloned() {
            if n.dismissible {
                self.notifications.pop_back();
            }
        }
    }

    pub fn current_is_error(&self) -> bool {
        self.current().map_or(false, |n| n.kind == NotificationKind::Error)
    }

    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty()
    }
}

impl NotificationKind {
    pub fn color(&self) -> Color {
        match self {
            NotificationKind::Info => skin::get_active().accent(),
            NotificationKind::Warning => Color::Yellow,
            NotificationKind::Error => Color::Red,
            NotificationKind::Success => Color::Rgb(80, 200, 120),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            NotificationKind::Info => "ℹ",
            NotificationKind::Warning => "⚠",
            NotificationKind::Error => "✗",
            NotificationKind::Success => "✓",
        }
    }
}

pub fn render_notification_banner(frame: &mut Frame, queue: &NotificationQueue, area: Rect) {
    let notif = match queue.current() {
        Some(n) => n,
        None => return,
    };

    let toast_width = 52u16.min(area.width.saturating_sub(4));
    if toast_width < 20 {
        return;
    }
    let toast_height = 3u16.min(area.height.saturating_sub(1).max(1));
    let toast_area = Rect {
        x: area.x + area.width.saturating_sub(toast_width + 2),
        y: if area.height >= 4 { area.y + 1 } else { area.y + area.height.saturating_sub(toast_height) },
        width: toast_width,
        height: toast_height,
    };

    let color = notif.kind.color();
    let bg = Color::Rgb(18, 18, 22);

    frame.render_widget(Clear, toast_area);

    let inner_w = toast_width.saturating_sub(4) as usize;
    let esc_hint = "  esc";
    let icon_with_spaces = format!(" {} ", notif.kind.icon());
    let icon_width = icon_with_spaces.width();
    let esc_width = if notif.dismissible { esc_hint.width() } else { 0 };

    let msg_width_budget = inner_w.saturating_sub(icon_width + esc_width);

    let message = {
        let msg_width = notif.message.width();
        if msg_width > msg_width_budget {
            let mut truncated = String::new();
            for ch in notif.message.chars() {
                let test = format!("{}{}", truncated, ch);
                if test.width() + 1 > msg_width_budget {
                    break;
                }
                truncated.push(ch);
            }
            format!("{}…", truncated)
        } else {
            notif.message.clone()
        }
    };

    let mut row0_spans = vec![
        Span::styled(icon_with_spaces.clone(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(message, Style::default().fg(skin::get_active().text())),
    ];
    if notif.dismissible {
        row0_spans.push(Span::styled(esc_hint.to_string(), Style::default().fg(skin::get_active().muted())));
    }

    let progress_line = if let Some(exp) = notif.expires_at {
        let now = Instant::now();
        let remaining = if exp > now { (exp - now).as_millis() } else { 0 };
        let total_ms = (exp - notif.pushed_at).as_millis().max(1);
        let frac = (remaining as f64 / total_ms as f64).min(1.0);
        let bar_w = (inner_w as f64 * frac) as usize;
        let bar_w = bar_w.min(inner_w);
        let filled: String = "─".repeat(bar_w);
        let empty: String = " ".repeat(inner_w.saturating_sub(bar_w));
        Line::from(vec![
            Span::styled(format!(" {}", filled), Style::default().fg(color)),
            Span::styled(empty, Style::default().fg(skin::get_active().muted())),
            Span::raw(" "),
        ])
    } else {
        Line::from(Span::styled(
            format!(" {}", "─".repeat(inner_w)),
            Style::default().fg(skin::get_active().muted()),
        ))
    };

    {
        let buf = frame.buffer_mut();

        let paint_row = |buf: &mut ratatui::buffer::Buffer, row: u16| {
            if toast_area.y + row >= buf.area().bottom() {
                return;
            }
            for col in 0..toast_width {
                let x = toast_area.x + col;
                if x >= buf.area().right() {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, toast_area.y + row)) {
                    cell.set_bg(bg);
                }
            }
        };
        for row in 0..toast_height {
            paint_row(buf, row);
        }

        if toast_area.x < buf.area().right() {
            for row in 0..toast_height {
                if toast_area.y + row < buf.area().bottom() {
                    if let Some(cell) = buf.cell_mut((toast_area.x, toast_area.y + row)) {
                        cell.set_bg(bg);
                        cell.set_fg(color);
                        cell.set_char('▌');
                    }
                }
            }
        }
        let right_x = toast_area.x + toast_width.saturating_sub(1);
        if right_x < buf.area().right() && toast_area.x < buf.area().right() {
            for row in 0..toast_height {
                if toast_area.y + row < buf.area().bottom() {
                    if let Some(cell) = buf.cell_mut((right_x, toast_area.y + row)) {
                        cell.set_bg(bg);
                        cell.set_fg(skin::get_active().muted());
                        cell.set_char('▐');
                    }
                }
            }
        }
    }

    if toast_area.y < frame.area().height {
        let msg_rect = Rect {
            x: toast_area.x + 1,
            y: toast_area.y,
            width: toast_width.saturating_sub(2),
            height: 1,
        };
        let para0 = Paragraph::new(Line::from(row0_spans)).style(Style::default().bg(bg));
        frame.render_widget(para0, msg_rect);
    }

    if toast_height > 1 && toast_area.y + 1 < frame.area().height {
        let prog_rect = Rect {
            x: toast_area.x + 1,
            y: toast_area.y + 1,
            width: toast_width.saturating_sub(2),
            height: 1,
        };
        let para1 = Paragraph::new(progress_line).style(Style::default().bg(bg));
        frame.render_widget(para1, prog_rect);
    }

    if toast_height > 2 && toast_area.y + 2 < frame.area().height {
        let pad_rect = Rect {
            x: toast_area.x + 1,
            y: toast_area.y + 2,
            width: toast_width.saturating_sub(2),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(bg)),
            pad_rect,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_current() {
        let mut q = NotificationQueue::new();
        assert!(q.current().is_none());
        q.push(NotificationKind::Info, "hello".to_string(), None);
        assert_eq!(q.current().unwrap().message, "hello");
    }

    #[test]
    fn dismiss_by_id() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "warn".to_string(), None);
        let id = q.current().unwrap().id.clone();
        q.dismiss(&id);
        assert!(q.is_empty());
    }

    #[test]
    fn duplicate_notification_is_refreshed() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Info, "same".to_string(), Some(3));
        q.push(NotificationKind::Info, "same".to_string(), Some(5));
        assert_eq!(q.notifications.len(), 1);
    }

    #[test]
    fn tick_removes_expired() {
        let mut q = NotificationQueue::new();
        q.notifications.push_back(Notification {
            id: "x".to_string(),
            kind: NotificationKind::Info,
            message: "gone".to_string(),
            pushed_at: Instant::now(),
            expires_at: Some(Instant::now() - std::time::Duration::from_secs(1)),
            dismissible: true,
        });
        assert!(!q.is_empty());
        q.tick();
        assert!(q.is_empty());
    }
}
