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
    /// Filter over `themes` by name or label. With 108 rows (8 built-ins + the
    /// bundled palettes, U2) a cursor-only list is unusable, so the picker takes
    /// a query the way the settings screen does.
    pub search_query: String,
    /// The theme that was active when the picker opened, so Esc can put it back
    /// after a preview moved the live palette.
    restore: Option<String>,
}

impl ThemeScreen {
    pub fn new() -> Self {
        Self {
            visible: false,
            themes: builtin_themes(),
            selected_idx: 0,
            search_query: String::new(),
            restore: None,
        }
    }

    pub fn open(&mut self, current_theme: &str) {
        self.visible = true;
        self.search_query.clear();
        self.restore = Some(current_theme.to_string());
        // Select the current theme, if found
        if let Some(idx) = self.themes.iter().position(|t| t.name == current_theme) {
            self.selected_idx = idx;
        } else {
            self.selected_idx = 0;
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.search_query.clear();
        self.restore = None;
    }

    /// The theme to return to when the operator cancels after previewing.
    pub fn restore_target(&self) -> Option<&str> {
        self.restore.as_deref()
    }

    /// The rows the current query leaves visible, in list order.
    pub fn visible_themes(&self) -> Vec<&ThemeOption> {
        let query = self.search_query.to_lowercase();
        self.themes
            .iter()
            .filter(|theme| {
                query.is_empty()
                    || theme.name.to_lowercase().contains(&query)
                    || theme.label.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// The row the cursor is on, within the filtered list.
    pub fn selected(&self) -> Option<&ThemeOption> {
        self.visible_themes().get(self.selected_idx).copied()
    }

    pub fn push_search_char(&mut self, c: char) {
        self.search_query.push(c);
        self.selected_idx = 0;
    }

    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        self.selected_idx = 0;
    }

    pub fn select_prev(&mut self) {
        let len = self.visible_themes().len();
        cycle_prev(&mut self.selected_idx, len);
    }

    pub fn select_next(&mut self) {
        let len = self.visible_themes().len();
        cycle_next(&mut self.selected_idx, len);
    }

    /// Return the name of the currently selected theme.
    pub fn selected_name(&self) -> Option<&str> {
        self.selected().map(|t| t.name.as_str())
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
///
/// The palette's own `NAMES` rather than a second hand-written list: this array
/// was a duplicate of `ColorPalette::for_theme`'s match arms, so a theme added
/// to one and not the other produced a picker row that could only ever render
/// the fallback palette.
const THEME_NAMES: &[&str] = crate::tui::theme_colors::ColorPalette::NAMES;

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
    // Built-ins first, then the operator's own theme files (U2). A built-in is
    // never shadowed by a same-named file — see `theme_files`' module doc — so
    // the two lists are concatenated, not merged by name.
    let operator: Vec<String> = crate::tui::theme_files::available()
        .into_iter()
        .filter(|name| !THEME_NAMES.contains(&name.as_str()))
        .collect();
    let mut rows: Vec<ThemeOption> = THEME_NAMES
        .iter()
        .filter_map(|name| {
            // The swatch is read from the palette itself, so the preview can
            // never drift from what `/theme` actually applies. `THEME_NAMES` is
            // `ColorPalette::NAMES`, so this lookup cannot miss — but a theme
            // that fails to resolve must not render as a row that lies about
            // its colours, so it is omitted rather than swatched from a
            // fallback. `debug_assert` makes the invariant loud in tests.
            let palette = ColorPalette::for_theme(name);
            debug_assert!(palette.is_some(), "{name} must resolve");
            palette.map(|palette| ThemeOption {
                name: (*name).to_string(),
                label: theme_label(name).to_string(),
                description: theme_description(name).to_string(),
                swatch: [
                    palette.panel_bg,
                    palette.accent,
                    palette.success,
                    palette.text,
                ],
            })
        })
        .collect();
    // An operator theme has no hand-written label or description: it is named by
    // its file, and its swatch is read from the file's own pairs so the preview
    // cannot drift from what applying it does. A file that fails to parse shows
    // the default palette's swatch rather than being omitted — an operator who
    // can see the row can see why it looks wrong.
    rows.extend(operator.into_iter().map(|name| {
        let pairs = crate::tui::theme_files::load(&name);
        let mut palette = ColorPalette::for_theme("default").expect("default is built in");
        if let Some(pairs) = &pairs {
            let map: std::collections::BTreeMap<String, String> = pairs.iter().cloned().collect();
            let (overrides, errors) = crate::tui::vendor::style::palette::Palette::from_pairs(
                map.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
            for error in &errors {
                tracing::warn!(error, "ignoring invalid entry in theme file {name}");
            }
            if overrides.has_overrides() {
                use crate::tui::vendor::style::palette::Role;
                for role in crate::tui::vendor::style::palette::ALL_ROLES {
                    if overrides.is_overridden(*role) {
                        let (r, g, b) = overrides.rgb(*role);
                        match role {
                            Role::Accent => palette.accent = Color::Rgb(r, g, b),
                            Role::Success => palette.success = Color::Rgb(r, g, b),
                            Role::Border => palette.border = Color::Rgb(r, g, b),
                            Role::UserBg => palette.panel_bg = Color::Rgb(r, g, b),
                            _ => {}
                        }
                    }
                }
            }
        }
        ThemeOption {
            label: name.clone(),
            description: "Operator theme file".to_string(),
            name,
            swatch: [
                palette.panel_bg,
                palette.accent,
                palette.success,
                palette.text,
            ],
        }
    }));
    rows
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
    let rows = screen.visible_themes().len() as u16 + 4;
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

    let visible = screen.visible_themes();
    let mut lines: Vec<Line> = Vec::new();
    // The query line doubles as the search field's own feedback: with 100+
    // bundled palettes an unfiltered list is unreadable, and an operator who
    // types and sees nothing change needs to know the filter is live.
    let query = if screen.search_query.is_empty() {
        "  / to search".to_string()
    } else {
        format!("  /{}  ({} match)", screen.search_query, visible.len())
    };
    lines.push(Line::from(Span::styled(
        query,
        Style::default().fg(theme_colors::muted()),
    )));
    lines.push(Line::from(""));

    for (i, theme) in visible.iter().enumerate() {
        let theme: &ThemeOption = theme;
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
            " ↑↓ navigate  ·  type to search  ·  enter apply  ·  esc cancel",
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
            // Cancel is a RESTORE, not just a close: moving the cursor previews
            // each theme live, so Esc must put back what was active at open or
            // cancelling would silently keep the last row the cursor touched.
            // Read BEFORE `close()` — close() clears the field, and reading it
            // after is why the first version of this returned None.
            let restore = screen.restore_target().map(String::from);
            screen.close();
            restore
        }
        KeyCode::Enter => {
            let name = screen.selected_name().map(String::from);
            screen.close();
            name
        }
        KeyCode::Up => {
            screen.select_prev();
            screen.selected_name().map(String::from)
        }
        KeyCode::Down => {
            screen.select_next();
            screen.selected_name().map(String::from)
        }
        KeyCode::Backspace => {
            screen.pop_search_char();
            None
        }
        KeyCode::Char(c) => {
            screen.push_search_char(c);
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

        // The list is the eight built-ins THEN the bundled and operator theme
        // files (U2), so the wrap target is the last row, whatever that is — the
        // invariant is the cycle, not a name. Asserting a literal name here is
        // what made this guard fail the moment the palettes shipped.
        let last = screen.themes.len() - 1;
        screen.select_prev();
        assert_eq!(
            screen.selected_name(),
            Some(screen.themes[last].name.as_str())
        );

        screen.select_next();
        assert_eq!(screen.selected_name(), Some("default"));
    }
    /// Search must narrow the rows, and the cursor must stay inside what is left.
    ///
    /// Before U2 the picker held eight rows and a cursor-only list was fine; with
    /// the bundled palettes it holds 100+, so a query is the only way to reach a
    /// named palette. The invariant that matters is the index staying valid across
    /// a filter change — `visible_themes()` is re-derived every move, so a stale
    /// index would render the wrong row's swatch and apply the wrong theme.
    #[test]
    fn typing_searches_the_picker_and_the_cursor_stays_inside_the_matches() {
        let mut screen = ThemeScreen::new();
        screen.open("default");
        let all = screen.themes.len();
        assert!(all > 100, "the bundled palettes are listed: {all}");

        // Every row is visible with no query, and the cursor is on the current one.
        assert_eq!(screen.visible_themes().len(), all);
        assert_eq!(screen.selected_name(), Some("default"));

        // A query that matches nothing leaves no rows and no selection — rather
        // than landing on an index that no longer names the row it looked like.
        for c in "zzzz".chars() {
            screen.push_search_char(c);
        }
        assert_eq!(screen.visible_themes().len(), 0);
        assert_eq!(screen.selected_name(), None);

        // Backspace widens again, and the cursor is valid once more.
        screen.pop_search_char();
        screen.pop_search_char();
        screen.pop_search_char();
        screen.pop_search_char();
        assert_eq!(screen.visible_themes().len(), all);

        // A real query narrows to that palette and the cursor lands on it.
        for c in "gruv".chars() {
            screen.push_search_char(c);
        }
        let visible = screen.visible_themes();
        assert!(
            visible.iter().all(|theme| theme.name.contains("gruv")),
            "only gruvbox-family palettes remain: {:?}",
            visible.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        assert_eq!(screen.selected_name(), Some(visible[0].name.as_str()));
    }

    /// Cancelling restores the theme that was active at open.
    ///
    /// Moving the cursor previews a theme live, so Esc that only closed the picker
    /// would leave the last row the cursor touched in force — a cancel that changes
    /// what the operator sees is not a cancel. This is the guard for the case the
    /// settings-screen bar-style row handles by applying on the cycle: a preview is
    /// a preview until Enter.
    #[test]
    fn escaping_after_a_preview_restores_the_theme_it_started_on() {
        let mut screen = ThemeScreen::new();
        screen.open("nord");

        // Move down, which in the app applies the preview.
        let previewed = {
            screen.select_next();
            screen.selected_name().map(String::from)
        };
        assert_ne!(previewed.as_deref(), Some("nord"), "the cursor moved");

        // Esc reports the theme to go back to, not the one being previewed.
        let restore = super::handle_theme_key(
            &mut screen,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert_eq!(restore.as_deref(), Some("nord"), "Esc restores the opener");
        assert!(!screen.visible, "and closes the picker");
    }
}
