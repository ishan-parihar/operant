// memory_file_selector.rs — Memory file selector overlay mirroring TS MemoryFileSelector.tsx

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::overlays::{
    HINT_ESC, begin_modal_buf, cycle_next, cycle_prev, render_modal_title_buf,
};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryFileType {
    User,
    Project,
    Local,
}

pub struct MemoryFile {
    pub display_path: String,
    pub file_type: MemoryFileType,
    pub exists: bool,
}

pub struct MemoryFileSelectorState {
    pub visible: bool,
    pub files: Vec<MemoryFile>,
    pub selected: usize,
    pub project_root: std::path::PathBuf,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl MemoryFileSelectorState {
    pub fn new() -> Self {
        Self {
            visible: false,
            files: Vec::new(),
            selected: 0,
            project_root: std::path::PathBuf::new(),
        }
    }

    /// Open the selector for the given project root.
    ///
    /// Populates the file list with:
    /// - User:    `~/.operant/AGENTS.md`
    /// - Project: `{project_root}/AGENTS.md`
    /// - Local:   `{project_root}/.operant/AGENTS.md`
    ///
    /// Each entry is marked `exists = true/false` based on the filesystem.
    pub fn open(&mut self, project_root: &std::path::Path) {
        self.project_root = project_root.to_path_buf();
        self.selected = 0;
        self.files.clear();

        // User-level: ~/.operant/AGENTS.md
        let user_path = crate::tui::adapter_types::config::Settings::config_dir().join("AGENTS.md");
        let user_display = {
            let home = dirs::home_dir().unwrap_or_default();
            let rel = user_path.strip_prefix(&home).unwrap_or(&user_path);
            format!("~/{}", rel.display())
        };
        self.files.push(MemoryFile {
            exists: user_path.exists(),
            display_path: user_display,
            file_type: MemoryFileType::User,
        });

        // Project-level: {project_root}/AGENTS.md
        let project_path = project_root.join("AGENTS.md");
        let project_display = project_path.display().to_string();
        self.files.push(MemoryFile {
            exists: project_path.exists(),
            display_path: project_display,
            file_type: MemoryFileType::Project,
        });

        // Local-level: {project_root}/.operant/AGENTS.md
        let local_path = project_root.join(".operant").join("AGENTS.md");
        let local_display = local_path.display().to_string();
        self.files.push(MemoryFile {
            exists: local_path.exists(),
            display_path: local_display,
            file_type: MemoryFileType::Local,
        });

        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected, self.files.len());
    }

    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected, self.files.len());
    }
}

impl Default for MemoryFileSelectorState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the memory file selector as a centered floating dialog.
pub fn render_memory_file_selector(state: &MemoryFileSelectorState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    // Height is derived from content (2 border + 1 title + 1 blank + N files
    // + 1 blank + 1 footer) and handed to the modal primitive unclamped — it
    // owns the clamp and the readable floors, which this panel previously had
    // none of (its height could exceed the terminal).
    let dialog_height = state.files.len() as u16 + 6;
    let layout = begin_modal_buf(buf, area, 70, dialog_height, 1, 1);
    let inner = layout.body_area;

    render_modal_title_buf(buf, layout.header_area, "Memory — choose a file", HINT_ESC);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (i, file) in state.files.iter().enumerate() {
        let type_label = match file.file_type {
            MemoryFileType::User => "User    ",
            MemoryFileType::Project => "Project ",
            MemoryFileType::Local => "Local   ",
        };

        let new_tag = if !file.exists {
            Span::styled(" (new)", Style::default().fg(theme_colors::muted()))
        } else {
            Span::raw("")
        };

        if i == state.selected {
            lines.push(Line::from(vec![Span::styled(
                pad_line(
                    &format!("  \u{203a} {type_label} {}", file.display_path),
                    inner.width,
                ),
                Style::default()
                    .fg(theme::user_bg())
                    .bg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            )]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("    {type_label} {}", file.display_path),
                    Style::default().fg(theme_colors::text()),
                ),
                new_tag,
            ]));
        }
    }

    let para = Paragraph::new(lines)
        .style(
            Style::default()
                .bg(theme_colors::panel_bg())
                .fg(theme_colors::text()),
        )
        .alignment(Alignment::Left);

    use ratatui::widgets::Widget;
    para.render(inner, buf);

    Paragraph::new(Line::from(vec![Span::styled(
        "  \u{2191}\u{2193} navigate  Enter select  Esc close",
        Style::default().fg(theme_colors::muted()),
    )]))
    .style(Style::default().bg(theme_colors::panel_bg()))
    .alignment(Alignment::Left)
    .render(layout.footer_area, buf);
}

fn pad_line(text: &str, width: u16) -> String {
    use crate::tui::render::{display_width, take_width};
    let max_width = width as usize;
    let mut clipped = take_width(text, max_width);
    // Pad by the *display* shortfall, so a two-cell glyph counts as two.
    let visible = display_width(&clipped);
    if visible < max_width {
        clipped.push_str(&" ".repeat(max_width - visible));
    }
    clipped
}
