// overlays/help.rs — Full-screen help overlay (? / F1 / /help).
//
// Extracted from the overlays.rs monolith. Renders the two-column
// keyboard-shortcut + slash-command reference panel and owns the
// HelpOverlay / HelpEntry state types.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

// ============================================================================
// HelpOverlay
// ============================================================================

/// State for the full-screen help overlay (? / F1 / /help).
#[derive(Debug, Default)]
pub struct HelpOverlay {
    pub visible: bool,
    pub scroll_offset: u16,
    /// Live search filter — only commands matching this substring are shown.
    pub filter: String,
    /// Dynamically populated entries from the command registry.
    pub commands: Vec<HelpEntry>,
}

/// A single command entry shown in the help overlay.
#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub name: String,
    /// Comma-separated aliases, e.g. "h, ?"
    pub aliases: String,
    pub description: String,
    pub category: String,
}

impl HelpOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Populate (or replace) the command entries from the command registry.
    /// Entries are sorted by category then name.
    pub fn populate_from_commands(&mut self, entries: Vec<HelpEntry>) {
        self.commands = entries;
        // Sort stable by category, then name for consistent display.
        self.commands
            .sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if !self.visible {
            // Reset state when closing
            self.scroll_offset = 0;
            self.filter.clear();
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.scroll_offset = 0;
        self.filter.clear();
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll_offset + 1 < max {
            self.scroll_offset += 1;
        }
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.scroll_offset = 0;
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.scroll_offset = 0;
    }
}

/// Render the help overlay into the frame.
pub fn render_help_overlay(frame: &mut Frame, overlay: &HelpOverlay, area: Rect) {
    use crate::tui::adapter_types::constants::APP_VERSION;
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::Wrap;
    if !overlay.visible {
        return;
    }
    let layout = begin_modal_frame(frame, area, 100, 36, 3, 1);
    render_modal_title_frame(frame, layout.header_area, "Shortcuts & commands", "esc");
    let search_line = modal_search_line(
        &overlay.filter,
        "Search shortcuts or commands",
        OPERANT_MUTED,
        OPERANT_TEXT,
    );
    if let Some(search_area) = modal_header_line_area(layout.header_area, 2) {
        frame.render_widget(Paragraph::new(search_line), search_area);
    }
    let content_area = layout.body_area;
    if content_area.height == 0 {
        return;
    }
    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(42),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(content_area);
    // ─── Left column: keyboard shortcuts by category ───────────────────────
    let mut left_lines: Vec<Line<'static>> = Vec::new();

    left_lines.push(Line::from(Span::styled(
        " Keyboard Shortcuts",
        Style::default()
            .fg(OPERANT_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));
    left_lines.push(Line::from(""));

    // Navigation category
    left_lines.push(Line::from(Span::styled(
        " Navigation",
        Style::default()
            .fg(OPERANT_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in &[
        ("PageUp / PgDn", "Scroll messages"),
        ("j / k", "Scroll one line"),
        ("Home / End", "Top / bottom"),
    ] {
        left_lines.push(kb_line(key, desc));
    }
    left_lines.push(Line::from(""));

    // Input category
    left_lines.push(Line::from(Span::styled(
        " Input",
        Style::default()
            .fg(OPERANT_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in &[
        ("Enter", "Submit message"),
        ("Up / Down", "Input history"),
        ("Ctrl+R", "Search history"),
        ("Esc", "Cancel / close"),
    ] {
        left_lines.push(kb_line(key, desc));
    }
    left_lines.push(Line::from(""));

    // App category
    left_lines.push(Line::from(Span::styled(
        " App",
        Style::default()
            .fg(OPERANT_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in &[
        ("F1 / ?", "Toggle help"),
        ("Ctrl+Shift+A", "Model picker (Ctrl+A)"),
        ("Ctrl+K", "Command palette"),
        ("Ctrl+C", "Cancel / quit"),
        ("Ctrl+D", "Quit (empty input)"),
        ("Ctrl+L", "Clear input line"),
        ("t", "(unbound — use mouse-click to toggle thinking)"),
    ] {
        left_lines.push(kb_line(key, desc));
    }

    frame.render_widget(
        Paragraph::new(left_lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(OPERANT_PANEL_BG)),
        col_chunks[0],
    );

    // ─── Center divider ────────────────────────────────────────────────────
    let divider_lines: Vec<Line<'static>> = (0..content_area.height)
        .map(|_| Line::from(Span::styled("\u{2502}", Style::default().fg(OPERANT_MUTED))))
        .collect();
    frame.render_widget(Paragraph::new(divider_lines), col_chunks[1]);

    // ─── Right column: slash commands by category ──────────────────────────
    let filter_lc = overlay.filter.to_lowercase();
    let filtered: Vec<&HelpEntry> = overlay
        .commands
        .iter()
        .filter(|e| {
            filter_lc.is_empty()
                || e.name.to_lowercase().contains(filter_lc.as_str())
                || e.aliases.to_lowercase().contains(filter_lc.as_str())
                || e.description.to_lowercase().contains(filter_lc.as_str())
        })
        .collect();

    let mut right_lines: Vec<Line<'static>> = Vec::new();

    right_lines.push(Line::from(Span::styled(
        " Slash Commands",
        Style::default()
            .fg(OPERANT_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));
    right_lines.push(Line::from(""));

    let mut current_cat = "";
    for entry in &filtered {
        if entry.category.as_str() != current_cat {
            current_cat = entry.category.as_str();
            if right_lines.len() > 2 {
                right_lines.push(Line::from(""));
            }
            right_lines.push(Line::from(Span::styled(
                format!(" {}", entry.category),
                Style::default()
                    .fg(OPERANT_ACCENT)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let aliases_text = if entry.aliases.is_empty() {
            String::new()
        } else {
            format!(" ({})", entry.aliases)
        };
        right_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("/{:<14}", entry.name),
                Style::default()
                    .fg(OPERANT_TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(aliases_text, Style::default().fg(OPERANT_MUTED)),
            Span::raw("  "),
            Span::styled(
                entry.description.clone(),
                Style::default().fg(OPERANT_MUTED),
            ),
        ]));
    }

    if filtered.is_empty() {
        right_lines.push(Line::from(Span::styled(
            " No matching commands",
            Style::default().fg(OPERANT_MUTED),
        )));
    }

    let right_total = right_lines.len() as u16;
    let right_visible = col_chunks[2].height;
    let max_scroll = right_total.saturating_sub(right_visible);
    let scroll = overlay.scroll_offset.min(max_scroll);

    frame.render_widget(
        Paragraph::new(right_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .style(Style::default().bg(OPERANT_PANEL_BG)),
        col_chunks[2],
    );

    let version_line = Line::from(vec![Span::styled(
        format!(
            " v{}  ·  type to filter  ·  ↑↓ scroll commands  ·  esc close",
            APP_VERSION
        ),
        Style::default()
            .fg(OPERANT_MUTED)
            .add_modifier(Modifier::ITALIC),
    )]);
    frame.render_widget(Paragraph::new(version_line), layout.footer_area);
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

fn kb_line<'a>(key: &str, desc: &str) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<20}", key),
            Style::default()
                .fg(OPERANT_TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(OPERANT_MUTED)),
    ])
}
