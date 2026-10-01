// plugins_hub.rs — Plugins browser + toggle overlay.
//
// Mirrors hermes-agent/ui-tui/src/components/pluginsHub.tsx. The operant TUI
// previously had only a PluginHintBanner (94 LOC) for showing dismissible
// recommendation banners — there was no way to actually browse installed
// plugins or enable/disable them from inside the TUI. The user had to drop
// to `operant plugins list / enable / disable` on the shell.
//
// This file adds a real PluginsHub overlay opened by `/plugins`. It lists
// every directory under `plugins_dir()`, shows enabled/disabled status (the
// `<name>.enabled` marker file pattern from cmd_plugins.rs), and lets the
// user toggle a plugin on/off by pressing Enter or `t`.
//
// Data source: crates/operant-cli/src/cmd_plugins::plugins_dir() — same
// primitive cmd_plugins.rs uses for `operant plugins list`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use std::path::{Path, PathBuf};

use crate::tui::overlays::{HINT_ESC, ModalSpec, cycle_next, cycle_prev, modal_frame};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

/// One row in the plugins list.
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub name: String,
    pub enabled: bool,
    /// Best-effort human-readable size (e.g. "248K"). Computed by walking
    /// the plugin directory tree at load time — matches cmd_plugins::dir_size
    /// but pre-formatted so the render path stays cheap.
    pub size: String,
}

#[derive(Debug, Clone, Default)]
pub struct PluginsHubState {
    pub visible: bool,
    pub plugins: Vec<PluginEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub last_error: String,
    /// Last action confirmation message (shown for 1 frame, then cleared).
    pub flash: Option<String>,
}

impl PluginsHubState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, plugins_dir: PathBuf) {
        self.visible = true;
        self.selected = 0;
        self.scroll = 0;
        self.last_error.clear();
        self.flash = None;
        self.plugins.clear();

        if !plugins_dir.exists() {
            // No plugins installed yet — not an error, just an empty list.
            return;
        }

        let entries = match std::fs::read_dir(&plugins_dir) {
            Ok(e) => e,
            Err(e) => {
                self.last_error = format!("Failed to read plugins dir: {}", e);
                return;
            }
        };

        let mut found: Vec<PluginEntry> = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip the per-plugin enable-marker files (they're files, not dirs,
            // so the is_dir filter already excludes them — but be defensive).
            if name.ends_with(".enabled") {
                continue;
            }
            let marker = plugins_dir.join(format!("{}.enabled", name));
            let enabled = marker.exists();
            let size = format_size(dir_size(&entry.path()));
            found.push(PluginEntry {
                name,
                enabled,
                size,
            });
        }

        found.sort_by(|a, b| a.name.cmp(&b.name));
        self.plugins = found;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected, self.plugins.len());
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected, self.plugins.len());
        if self.selected > self.scroll + 12 {
            self.scroll = self.selected.saturating_sub(12);
        }
    }

    /// Toggle the selected plugin's enabled state by creating or removing
    /// the `<name>.enabled` marker file. Returns a flash message.
    pub fn toggle_selected(&mut self, plugins_dir: &Path) {
        let Some(entry) = self.plugins.get_mut(self.selected) else {
            return;
        };
        let marker = plugins_dir.join(format!("{}.enabled", entry.name));
        if entry.enabled {
            // Disable: remove the marker file.
            match std::fs::remove_file(&marker) {
                Ok(_) => {
                    entry.enabled = false;
                    self.flash = Some(format!("Disabled plugin '{}'", entry.name));
                }
                Err(e) => {
                    self.last_error = format!("Failed to disable '{}': {}", entry.name, e);
                }
            }
        } else {
            // Enable: create the marker file.
            match std::fs::write(&marker, "") {
                Ok(_) => {
                    entry.enabled = true;
                    self.flash = Some(format!("Enabled plugin '{}'", entry.name));
                }
                Err(e) => {
                    self.last_error = format!("Failed to enable '{}': {}", entry.name, e);
                }
            }
        }
    }
}

pub fn render_plugins_hub(frame: &mut Frame, state: &PluginsHubState, area: Rect) {
    if !state.visible {
        return;
    }

    // Natural desired size is 72×20; `modal_frame` clamps it to the terminal.
    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: "Plugins",
            hint: HINT_ESC,
            width: 72,
            height: 20,
            header_height: 1,
            footer_height: 0,
            // The border carries meaning here (it was `success()` before), which
            // is what `ModalSpec::border_fg` exists for.
            border_fg: theme_colors::success(),
        },
    );
    let inner = layout.body_area;

    if !state.last_error.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "Plugin operation failed:",
                Style::default()
                    .fg(theme_colors::error())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                state.last_error.clone(),
                Style::default().fg(theme_colors::warning()),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close.",
                Style::default().fg(theme::dim_color()),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    if state.plugins.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "No plugins installed.",
                Style::default()
                    .fg(theme_colors::warning())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Plugins are git repositories cloned into your operant"),
            Line::from("plugins directory. Install one with:"),
            Line::from(""),
            Line::from(Span::styled(
                "  operant plugins install <git-url>",
                Style::default().fg(theme_colors::accent()),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close.",
                Style::default().fg(theme::dim_color()),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Header row.
    lines.push(Line::from(vec![Span::styled(
        format!(
            " {:<3}  {:<8}  {:<24}  {:>8}",
            "#", "Status", "Name", "Size"
        ),
        Style::default()
            .fg(theme::dim_color())
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(Span::styled(
        " ".repeat(inner.width as usize),
        Style::default().fg(theme::dim_color()),
    )));

    let viewport = inner.height.saturating_sub(6) as usize;
    let start = state
        .scroll
        .min(state.plugins.len().saturating_sub(viewport));
    let end = (start + viewport).min(state.plugins.len());

    for i in start..end {
        let entry = &state.plugins[i];
        let is_selected = i == state.selected;
        let prefix = if is_selected { "›" } else { " " };
        let row_style = if is_selected {
            Style::default()
                .fg(theme_colors::on_selection())
                .bg(theme_colors::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme_colors::text())
        };
        let status_str = if entry.enabled { "enabled" } else { "disabled" };
        let status_style = if entry.enabled {
            Style::default()
                .fg(theme_colors::success())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::dim_color())
        };
        let name = truncate(&entry.name, 24);
        let size = truncate(&entry.size, 8);
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", prefix), row_style),
            Span::styled(format!("{:<3}  ", i + 1), row_style),
            Span::styled(format!("{:<8}  ", status_str), status_style),
            Span::styled(format!("{:<24}  ", name), row_style),
            Span::styled(format!("{:>8}", size), row_style),
        ]));
    }

    // Pad to viewport.
    let pad = viewport.saturating_sub(state.plugins.len() - start);
    for _ in 0..pad {
        lines.push(Line::from(""));
    }

    // Flash / footer.
    if let Some(ref msg) = state.flash {
        lines.push(Line::from(Span::styled(
            msg.clone(),
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        " ↑/↓ navigate · Enter/t toggle · Esc close ",
        Style::default().fg(theme::dim_color()),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Walk a directory tree and return its total size in bytes.
/// Mirrors cmd_plugins::dir_size but inlined here so the overlay doesn't need
/// to depend on cmd_plugins' private function.
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&p) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_file() {
                total += meta.len();
            } else if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    total
}

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    if bytes == 0 {
        return "0B".to_string();
    }
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{}B", bytes)
    } else {
        format!("{:.0}{}", size, UNITS[unit_idx])
    }
}

/// Truncate `s` to `max` display cells, appending `…` if cut.
fn truncate(s: &str, max: usize) -> String {
    use crate::tui::render::{display_width, take_width};
    if display_width(s) <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out = take_width(s, max - 1);
        out.push('…');
        out
    }
}
