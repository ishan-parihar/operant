// theme_screen.rs — Theme picker overlay opened by /theme.
//
// Shows a list of available themes with colour swatches. Arrow keys navigate,
// Enter selects, Esc cancels.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::overlays::{
    begin_modal_frame, cycle_next, cycle_prev, modal_header_line_area, render_modal_title_frame,
};
use crate::tui::theme_colors;
use crate::tui::theme_colors::ColorPalette;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single theme option shown in the picker.
#[derive(Debug, Clone)]
pub struct ThemeOption {
    pub name: String,
    pub label: String,
    pub description: String,
    /// A few representative colours used for the swatch preview.
    pub swatch: [Color; 4],
}

pub struct ThemeScreen {
    pub visible: bool,
    pub themes: Vec<ThemeOption>,
    pub selected_idx: usize,
}

impl ThemeScreen {
    pub fn new() -> Self {
        Self {
            visible: false,
            themes: builtin_themes(),
            selected_idx: 0,
        }
    }

    pub fn open(&mut self, current_theme: &str) {
        self.visible = true;
        // Select the current theme, if found
        if let Some(idx) = self.themes.iter().position(|t| t.name == current_theme) {
            self.selected_idx = idx;
        } else {
            self.selected_idx = 0;
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected_idx, self.themes.len());
    }

    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected_idx, self.themes.len());
    }

    /// Return the name of the currently selected theme.
    pub fn selected_name(&self) -> Option<&str> {
        self.themes.get(self.selected_idx).map(|t| t.name.as_str())
    }
}

impl Default for ThemeScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in themes
// ---------------------------------------------------------------------------

/// The themes offered by the picker, in display order.
const THEME_NAMES: &[&str] = &[
    "default",
    "dark",
    "light",
    "solarized",
    "nord",
    "dracula",
    "monokai",
    "deuteranopia",
];

fn theme_label(name: &str) -> &'static str {
    match name {
        "default" => "Default",
        "dark" => "Dark",
        "light" => "Light",
        "solarized" => "Solarized",
        "nord" => "Nord",
        "dracula" => "Dracula",
        "monokai" => "Monokai",
        "deuteranopia" => "Deuteranopia",
        _ => "Custom",
    }
}

fn theme_description(name: &str) -> &'static str {
    match name {
        "default" => "Operant default — dark background, cyan accents",
        "dark" => "High-contrast dark theme",
        "light" => "Light background with dark text",
        "solarized" => "Solarized Dark — warm tones with blue accents",
        "nord" => "Nord — cool blue-grey palette",
        "dracula" => "Dracula — purple/pink dark theme",
        "monokai" => "Monokai — vibrant colours on dark background",
        "deuteranopia" => "Red-green color blind friendly — blue/yellow/gray palette",
        _ => "Unknown palette — falls back to the Operant default",
    }
}

fn builtin_themes() -> Vec<ThemeOption> {
    THEME_NAMES
        .iter()
        .map(|name| {
            // The swatch is read from the palette itself, so the preview can
            // never drift from what `/theme` actually applies.
            let palette = ColorPalette::for_theme(name);
            ThemeOption {
                name: (*name).to_string(),
                label: theme_label(name).to_string(),
                description: theme_description(name).to_string(),
                swatch: [
                    palette.panel_bg,
                    palette.accent,
                    palette.success,
                    palette.text,
                ],
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the theme picker overlay into `frame`.
pub fn render_theme_screen(frame: &mut Frame, screen: &ThemeScreen, area: Rect) {
    if !screen.visible {
        return;
    }

    // `begin_modal_frame` routes through `modal_layout`, which already clamps to
    // `area.height - space::M` and floors at `MIN_MODAL_H`. This used to restate
    // a caller-side `.min(area.height.saturating_sub(6))` on the way in; the
    // two clamps bound the same result, so dropping the outer one is a no-op.
    let rows = screen.themes.len() as u16 + 2;
    let layout = begin_modal_frame(frame, area, 70, rows + 6, 2, 1);
    render_modal_title_frame(frame, layout.header_area, "Choose a theme", "esc");
    if let Some(subtitle_area) = modal_header_line_area(layout.header_area, 1) {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Applies immediately — the whole TUI repaints in this palette.",
                Style::default().fg(theme_colors::muted()),
            )])),
            subtitle_area,
        );
    }

    let mut lines: Vec<Line> = Vec::new();

    for (i, theme) in screen.themes.iter().enumerate() {
        let is_selected = i == screen.selected_idx;
        let bg = if is_selected {
            theme_colors::accent()
        } else {
            theme_colors::panel_bg()
        };
        let fg = if is_selected {
            theme_colors::on_selection()
        } else {
            theme_colors::text()
        };
        let desc_fg = if is_selected {
            theme_colors::on_selection()
        } else {
            theme_colors::muted()
        };

        // Build the swatch using block characters with background colour
        let swatch_spans: Vec<Span> = theme
            .swatch
            .iter()
            .map(|&c| Span::styled("  ", Style::default().bg(c)))
            .collect();

        let mut row_spans: Vec<Span> = Vec::new();
        row_spans.push(Span::styled(" ", Style::default().bg(bg)));
        row_spans.extend(swatch_spans);
        row_spans.push(Span::styled("  ", Style::default().bg(bg)));
        row_spans.push(Span::styled(
            format!("{:<12}", theme.label),
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        ));
        row_spans.push(Span::styled(
            theme.description.clone(),
            Style::default().fg(desc_fg).bg(bg),
        ));
        let used: usize = row_spans.iter().map(|span| span.content.len()).sum();
        let pad = layout.body_area.width.saturating_sub(used as u16) as usize;
        if pad > 0 {
            row_spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
        }

        lines.push(Line::from(row_spans));
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme_colors::panel_bg())),
        layout.body_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " ↑↓ navigate  ·  enter apply  ·  esc cancel",
            Style::default()
                .fg(theme_colors::muted())
                .add_modifier(Modifier::ITALIC),
        )])),
        layout.footer_area,
    );
}

// ---------------------------------------------------------------------------
// Key handling helpers (called from app.rs)
// ---------------------------------------------------------------------------

/// Returns the selected theme name when the user confirms, `None` otherwise.
/// Call this from the app's key handler when `theme_screen.visible`.
pub fn handle_theme_key(
    screen: &mut ThemeScreen,
    key: crossterm::event::KeyEvent,
) -> Option<String> {
    use crossterm::event::KeyCode;

    if !screen.visible {
        return None;
    }

    match key.code {
        KeyCode::Esc => {
            screen.close();
            None
        }
        KeyCode::Enter => {
            let name = screen.selected_name().map(String::from);
            screen.close();
            name
        }
        KeyCode::Up => {
            screen.select_prev();
            None
        }
        KeyCode::Down => {
            screen.select_next();
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn theme_screen_renders_current_theme() {
        let mut screen = ThemeScreen::new();
        screen.open("dark");

        let backend = TestBackend::new(90, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_theme_screen(frame, &screen, frame.area()))
            .unwrap();

        let rendered = terminal.backend().buffer();
        let content = rendered
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(content.contains("Choose a theme"));
        assert!(content.contains("Dark"));
    }

    #[test]
    fn theme_navigation_wraps() {
        let mut screen = ThemeScreen::new();
        screen.open("default");

        screen.select_prev();
        assert_eq!(screen.selected_name(), Some("deuteranopia"));

        screen.select_next();
        assert_eq!(screen.selected_name(), Some("default"));
    }
}
