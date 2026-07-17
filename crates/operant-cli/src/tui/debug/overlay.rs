//! TuiDebugOverlay — F12 in-TUI panel showing live debug state.
//!
//! Renders a semi-transparent panel at the bottom of the screen showing:
//! - Uptime + frame count + last render time
//! - Last error
//! - Last 15 events from the event bus
//!
//! Toggle with F12. Non-blocking — renders on top of the existing layout.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use super::debug_hub::TuiDebugHub;

/// Render the debug overlay. No-op if the overlay is not visible.
/// Call this from `render::render_app` after the main render, passing the
/// full frame area.
pub fn render_debug_overlay(f: &mut Frame, hub: &TuiDebugHub, full_area: Rect) {
    if !hub.overlay_visible() {
        return;
    }

    // Overlay occupies the bottom 40% of the screen, with a 1-cell margin.
    let height = (full_area.height as f32 * 0.40) as u16;
    let height = height.max(10).min(full_area.height.saturating_sub(2));
    let overlay_area = Rect {
        x: full_area.x + 1,
        y: full_area.y + full_area.height - height - 1,
        width: full_area.width.saturating_sub(2),
        height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Debug (F12 to close) ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(overlay_area);
    f.render_widget(block, overlay_area);

    // Split into stats (top) + event log (bottom).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(1)])
        .split(inner);

    // ── Stats panel ──────────────────────────────────────────────────
    let last_err = hub.last_error();
    let last_err_str = last_err.as_deref().unwrap_or("(none)");
    let stats = vec![
        Line::from(vec![
            Span::styled("Uptime:    ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{:.1}s", hub.uptime_secs())),
        ]),
        Line::from(vec![
            Span::styled("Frames:    ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}", hub.frame_count())),
        ]),
        Line::from(vec![
            Span::styled("Render:    ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}ms last", hub.last_render_ms())),
        ]),
        Line::from(vec![
            Span::styled("Last err:  ", Style::default().fg(Color::Cyan)),
            Span::styled(
                last_err_str,
                Style::default().fg(if last_err.is_some() {
                    Color::Red
                } else {
                    Color::DarkGray
                }),
            ),
        ]),
    ];
    let stats_para = Paragraph::new(stats).alignment(Alignment::Left);
    f.render_widget(stats_para, chunks[0]);

    // ── Event log ────────────────────────────────────────────────────
    let events = hub.event_bus().recent(50);
    let items: Vec<ListItem> = events
        .iter()
        .rev()
        .take(chunks[1].height as usize)
        .map(|e| {
            let color = match e {
                super::TuiEvent::Error { .. } => Color::Red,
                super::TuiEvent::FrameRendered { .. } => Color::DarkGray,
                super::TuiEvent::Key { .. } | super::TuiEvent::Mouse { .. } => Color::Green,
                super::TuiEvent::SlashCommand { .. } => Color::Yellow,
                super::TuiEvent::AgentEvent { .. } => Color::Blue,
                _ => Color::White,
            };
            ListItem::new(Line::from(vec![Span::styled(
                e.summary(),
                Style::default().fg(color),
            )]))
        })
        .collect();

    let event_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .title(Span::styled(
                    " Recent events (newest first) ",
                    Style::default().fg(Color::DarkGray),
                ))
                .style(Style::default().bg(Color::Black)),
        )
        .style(Style::default().fg(Color::White));
    f.render_widget(event_list, chunks[1]);
}
