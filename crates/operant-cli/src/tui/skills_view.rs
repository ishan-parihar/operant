// skills_view.rs — Skills browser overlay.
//
// Mirrors hermes-agent/ui-tui/src/components/skillsHub.tsx (309 LOC, 3-stage
// category→skill→actions). The operant TUI listed `/skills` in the help
// command list but never intercepted it — it fell through to a basic command
// registry handler that just printed a help line. This file gives /skills a
// real overlay: list of installed skills, scrollable, with name / category /
// version / description, and an Inspect action that opens the SKILL.md body.
//
// Data source: operant_core::skills::SkillManager::load_all() — the same
// primitive cmd_skills.rs uses for `operant skills list`.

use operant_core::skills::Skill;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::path::PathBuf;

use crate::tui::overlays::centered_rect;

/// What view stage the overlay is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillsStage {
    /// List of all skills (the default landing view).
    #[default]
    List,
    /// Detail view for a single skill (SKILL.md body + metadata).
    Detail,
}

#[derive(Debug, Clone, Default)]
pub struct SkillsViewState {
    pub visible: bool,
    pub stage: SkillsStage,
    /// All loaded skills, sorted by category then name.
    pub skills: Vec<Skill>,
    /// Cursor index in the list view.
    pub selected: usize,
    /// Vertical scroll offset for the list view (lines from top).
    pub scroll: usize,
    /// Vertical scroll offset for the detail view.
    pub detail_scroll: usize,
    /// Last error from load_all (shown inline if non-empty).
    pub last_error: String,
}

impl SkillsViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, skills_dir: PathBuf) {
        self.visible = true;
        self.stage = SkillsStage::List;
        self.selected = 0;
        self.scroll = 0;
        self.detail_scroll = 0;
        self.last_error.clear();
        self.skills.clear();

        // Load synchronously — the skills dir is a local filesystem read and
        // SkillManager::load_all is fast (one readdir + one read per skill).
        // Errors (e.g. dir doesn't exist yet on a fresh install) are surfaced
        // inline rather than crashing the overlay.
        let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
        match mgr.load_all() {
            Ok(mut loaded) => {
                // Sort by (category, name) so related skills cluster visually.
                loaded.sort_by(|a, b| {
                    a.category
                        .cmp(&b.category)
                        .then_with(|| a.name.cmp(&b.name))
                });
                self.skills = loaded;
            }
            Err(e) => {
                self.last_error = format!("Failed to load skills: {}", e);
            }
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        if self.skills.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.skills.len() - 1
        } else {
            self.selected - 1
        };
        // Snap scroll up if the cursor went above the viewport.
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    pub fn select_next(&mut self) {
        if self.skills.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.skills.len();
        // Snap scroll down if the cursor went below the viewport.
        // The viewport height isn't known here, so we just bump by 1 —
        // render() clamps to the actual visible area.
        if self.selected > self.scroll + 12 {
            self.scroll = self.selected.saturating_sub(12);
        }
    }

    pub fn scroll_down(&mut self, viewport: usize) {
        match self.stage {
            SkillsStage::List => {
                let max = self.skills.len().saturating_sub(viewport);
                if self.scroll < max {
                    self.scroll += 1;
                }
            }
            SkillsStage::Detail => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
            }
        }
    }

    pub fn scroll_up(&mut self) {
        match self.stage {
            SkillsStage::List => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            SkillsStage::Detail => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
        }
    }

    pub fn open_detail(&mut self) {
        if self.skills.is_empty() {
            return;
        }
        self.stage = SkillsStage::Detail;
        self.detail_scroll = 0;
    }

    pub fn back_to_list(&mut self) {
        self.stage = SkillsStage::List;
    }

    pub fn current_skill(&self) -> Option<&Skill> {
        self.skills.get(self.selected)
    }
}

/// Render the skills overlay. Called from render.rs after all other overlays
/// so it sits on top of the transcript.
pub fn render_skills_view(frame: &mut Frame, state: &SkillsViewState, area: Rect) {
    if !state.visible {
        return;
    }

    // Centered 80×24 modal (or smaller if the terminal is narrow).
    let w = 80u16.min(area.width.saturating_sub(4));
    let h = 24u16.min(area.height.saturating_sub(4));
    let dlg = centered_rect(w, h, area);

    frame.render_widget(Clear, dlg);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Skills ",
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
                "Could not load skills:",
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

    if state.skills.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "No skills installed.",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Skills are markdown files (SKILL.md) living in subdirectories"),
            Line::from("of your operant skills directory. Install one with:"),
            Line::from(""),
            Line::from(Span::styled(
                "  operant skills install <path-or-url>",
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

    match state.stage {
        SkillsStage::List => render_list_stage(frame, state, inner),
        SkillsStage::Detail => render_detail_stage(frame, state, inner),
    }
}

fn render_list_stage(frame: &mut Frame, state: &SkillsViewState, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header row.
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                " {:<3}  {:<24} {:<14} {:<8} ",
                "#", "Name", "Category", "Version"
            ),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        " ".repeat(area.width as usize),
        Style::default().fg(Color::DarkGray),
    )));

    let viewport = area.height.saturating_sub(6) as usize; // header + footer
    let start = state.scroll.min(state.skills.len().saturating_sub(viewport));
    let end = (start + viewport).min(state.skills.len());

    for display_idx in start..end {
        let skill = &state.skills[display_idx];
        let is_selected = display_idx == state.selected;
        let prefix = if is_selected { "›" } else { " " };
        let row_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let name = truncate(skill.name.as_str(), 24);
        let cat = truncate(skill.category.as_str(), 14);
        let ver = truncate(skill.version.as_str(), 8);
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", prefix), row_style),
            Span::styled(format!("{:<3}  ", display_idx + 1), row_style),
            Span::styled(format!("{:<24} ", name), row_style),
            Span::styled(format!("{:<14} ", cat), row_style),
            Span::styled(format!("{:<8}", ver), row_style),
        ]));
    }

    // Footer with the description of the highlighted skill + keybindings.
    let pad_lines = viewport.saturating_sub(state.skills.len() - start);
    for _ in 0..pad_lines {
        lines.push(Line::from(""));
    }

    if let Some(skill) = state.current_skill() {
        lines.push(Line::from(Span::styled(
            truncate_to_width(&skill.description, area.width as usize - 2),
            Style::default().fg(Color::Yellow),
        )));
    } else {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        " ↑/↓ navigate · Enter inspect · Esc close ",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_detail_stage(frame: &mut Frame, state: &SkillsViewState, area: Rect) {
    let Some(skill) = state.current_skill() else {
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Name:        ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            skill.name.clone(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Category:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(skill.category.clone(), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Version:     ", Style::default().fg(Color::DarkGray)),
        Span::styled(skill.version.clone(), Style::default().fg(Color::Yellow)),
    ]));
    if !skill.tags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Tags:        ", Style::default().fg(Color::DarkGray)),
            Span::styled(skill.tags.join(", "), Style::default().fg(Color::White)),
        ]));
    }
    if !skill.platforms.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Platforms:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(skill.platforms.join(", "), Style::default().fg(Color::White)),
        ]));
    }
    if !skill.prerequisites_env.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Env vars:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                skill.prerequisites_env.join(", "),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }
    if !skill.prerequisites_commands.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Commands:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                skill.prerequisites_commands.join(", "),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }
    lines.push(Line::from(""));

    // Body — render the SKILL.md content. We don't run the full markdown
    // renderer here because the detail view already has its own padding /
    // scroll discipline; a plain monospace dump with a thin separator reads
    // better in a fixed-height modal.
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let body_lines: Vec<&str> = skill.content.lines().collect();
    let viewport = area.height.saturating_sub(lines.len() as u16 + 2) as usize;
    let start = state.detail_scroll.min(body_lines.len().saturating_sub(viewport));
    let end = (start + viewport).min(body_lines.len());
    for line in &body_lines[start..end] {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ↑/↓ scroll · Backspace back to list · Esc close ",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        area,
    );
}

/// Truncate `s` to `max` characters, appending `…` if truncated.
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

/// Word-wrap-aware truncation for the description footer line. Hard-truncates
/// at `max` display columns (no wrapping), appending `…` if cut.
fn truncate_to_width(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if width + cw > max {
            break;
        }
        out.push(ch);
        width += cw;
    }
    if out.chars().count() < s.chars().count() {
        out.push('…');
    }
    out
}
