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

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::path::{Path, PathBuf};

use crate::tui::overlays::{centered_rect, cycle_next, cycle_prev};

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

    let w = 72u16.min(area.width.saturating_sub(4));
    let h = 20u16.min(area.height.saturating_sub(4));
    let dlg = centered_rect(w, h, area);

    frame.render_widget(Clear, dlg);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(Span::styled(
            " Plugins ",
            Style::default().add_modifier(Modifier::BOLD),
        ));

    let inner = {
        let inner = block.inner(dlg);
        frame.render_widget(block, dlg);
        inner
    };

    if !state.last_error.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "Plugin operation failed:",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                state.last_error.clone(),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close.",
                Style::default().fg(Color::DarkGray),
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
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Plugins are git repositories cloned into your operant"),
            Line::from("plugins directory. Install one with:"),
            Line::from(""),
            Line::from(Span::styled(
                "  operant plugins install <git-url>",
                Style::default().fg(Color::Cyan),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close.",
                Style::default().fg(Color::DarkGray),
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
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(Span::styled(
        " ".repeat(inner.width as usize),
        Style::default().fg(Color::DarkGray),
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
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let status_str = if entry.enabled { "enabled" } else { "disabled" };
        let status_style = if entry.enabled {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
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
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        " ↑/↓ navigate · Enter/t toggle · Esc close ",
        Style::default().fg(Color::DarkGray),
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

/// Truncate `s` to `max` chars, appending `…` if cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
