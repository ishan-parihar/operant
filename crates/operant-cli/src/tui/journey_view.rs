// journey_view.rs — Skills + memories timeline overlay.
//
// Mirrors hermes-agent/ui-tui/src/components/journey.tsx. The /journey command
// was previously a "planned overlay" status message (iter-77 backfill); this
// file gives it a real two-pane view:
//   Left pane:  recently-installed/modified skills (name, category, version)
//   Right pane: long-term memories (id, type, importance, content preview)
//
// The "journey" framing is loose — operant doesn't track skill install events
// or memory creation timestamps in a way that maps to a strict timeline, so
// we present the two data sources as parallel columns the user can scroll.
// Memories are sorted by created_at descending (newest first) so the most
// recent activity surfaces at the top.
//
// Data sources:
//   - Skills:  operant_core::skills::SkillManager::load_all()
//   - Memories: operant_core::memory::MemoryStore::read_memories() (sync read
//                of ~/.operant/memory/MEMORY.md)

use operant_core::agent::learning_graph::{self, LearningGraph};
use operant_core::memory::{MemoryBlock, MemoryStore};
use operant_core::skills::Skill;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::path::PathBuf;

use crate::tui::overlays::centered_rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JourneyPane {
    #[default]
    Skills,
    Memories,
}

#[derive(Debug, Clone, Default)]
pub struct JourneyViewState {
    pub visible: bool,
    pub skills: Vec<Skill>,
    pub memories: Vec<MemoryBlock>,
    pub active_pane: JourneyPane,
    pub skills_cursor: usize,
    pub memories_cursor: usize,
    pub skills_scroll: usize,
    pub memories_scroll: usize,
    pub last_error: String,
    /// The learning graph built from skills + memory directories.
    /// Provides node/edge/stats data for graph-aware rendering.
    pub graph: Option<LearningGraph>,
}

impl JourneyViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, skills_dir: PathBuf, memory_dir: PathBuf) {
        self.visible = true;
        self.active_pane = JourneyPane::Skills;
        self.skills_cursor = 0;
        self.memories_cursor = 0;
        self.skills_scroll = 0;
        self.memories_scroll = 0;
        self.last_error.clear();
        self.skills.clear();
        self.memories.clear();

        // Build the learning graph from skills + memory directories.
        // This connects skills and memories as first-class graph nodes
        // with edges derived from lexical overlap, powering the
        // self-learning visualization.
        self.graph = Some(learning_graph::build_learning_graph(
            &skills_dir,
            &memory_dir,
        ));

        // Load skills (same primitive as skills_view.rs).
        let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
        match mgr.load_all() {
            Ok(mut loaded) => {
                loaded.sort_by(|a, b| a.name.cmp(&b.name));
                self.skills = loaded;
            }
            Err(e) => {
                self.last_error = format!("Skills load failed: {}", e);
            }
        }

        // Load memories (sync read of MEMORY.md).
        let store = MemoryStore::new(memory_dir);
        match store.read_memories() {
            Ok(map) => {
                let mut blocks: Vec<MemoryBlock> = map.into_values().collect();
                // Newest first by created_at, fall back to importance.
                blocks.sort_by(|a, b| {
                    b.created_at
                        .cmp(&a.created_at)
                        .then(b.importance.cmp(&a.importance))
                });
                self.memories = blocks;
            }
            Err(e) => {
                // Don't clobber a skills-load error; append.
                let msg = format!("Memories load failed: {}", e);
                if self.last_error.is_empty() {
                    self.last_error = msg;
                } else {
                    self.last_error.push_str(" | ");
                    self.last_error.push_str(&msg);
                }
            }
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn switch_pane(&mut self) {
        self.active_pane = match self.active_pane {
            JourneyPane::Skills => JourneyPane::Memories,
            JourneyPane::Memories => JourneyPane::Skills,
        };
    }

    pub fn cursor_up(&mut self) {
        match self.active_pane {
            JourneyPane::Skills => {
                if !self.skills.is_empty() {
                    self.skills_cursor = if self.skills_cursor == 0 {
                        self.skills.len() - 1
                    } else {
                        self.skills_cursor - 1
                    };
                    if self.skills_cursor < self.skills_scroll {
                        self.skills_scroll = self.skills_cursor;
                    }
                }
            }
            JourneyPane::Memories => {
                if !self.memories.is_empty() {
                    self.memories_cursor = if self.memories_cursor == 0 {
                        self.memories.len() - 1
                    } else {
                        self.memories_cursor - 1
                    };
                    if self.memories_cursor < self.memories_scroll {
                        self.memories_scroll = self.memories_cursor;
                    }
                }
            }
        }
    }

    pub fn cursor_down(&mut self) {
        match self.active_pane {
            JourneyPane::Skills => {
                if !self.skills.is_empty() {
                    self.skills_cursor = (self.skills_cursor + 1) % self.skills.len();
                    if self.skills_cursor > self.skills_scroll + 12 {
                        self.skills_scroll = self.skills_cursor.saturating_sub(12);
                    }
                }
            }
            JourneyPane::Memories => {
                if !self.memories.is_empty() {
                    self.memories_cursor = (self.memories_cursor + 1) % self.memories.len();
                    if self.memories_cursor > self.memories_scroll + 12 {
                        self.memories_scroll = self.memories_cursor.saturating_sub(12);
                    }
                }
            }
        }
    }
}

pub fn render_journey_view(frame: &mut Frame, state: &JourneyViewState, area: Rect) {
    if !state.visible {
        return;
    }

    let w = 90u16.min(area.width.saturating_sub(4));
    let h = 26u16.min(area.height.saturating_sub(4));
    let dlg = centered_rect(w, h, area);

    frame.render_widget(Clear, dlg);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(Span::styled(
            " Journey — skills + memories ",
            Style::default().add_modifier(Modifier::BOLD),
        ));

    let inner = {
        let inner = block.inner(dlg);
        frame.render_widget(block, dlg);
        inner
    };

    if !state.last_error.is_empty() && state.skills.is_empty() && state.memories.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "Could not load journey data:",
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

    // Split inner into two columns with a 1-col gutter.
    let col_w = inner.width / 2;
    let gutter = inner.width - 2 * col_w;
    let left_area = Rect {
        x: inner.x,
        y: inner.y,
        width: col_w,
        height: inner.height,
    };
    let right_area = Rect {
        x: inner.x + col_w + gutter,
        y: inner.y,
        width: col_w,
        height: inner.height,
    };

    render_skills_pane(frame, state, left_area);
    render_memories_pane(frame, state, right_area);

    // Footer: graph stats + key hints (carved out of the bottom of inner).
    let footer = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    let pane_hint = match state.active_pane {
        JourneyPane::Skills => "[Skills]",
        JourneyPane::Memories => "[Memories]",
    };
    let graph_stats = state.graph.as_ref().map_or_else(String::new, |g| {
        format!(
            " │ {} skills · {} memory · {} edges · {:.1}% linked",
            g.stats.skill_nodes,
            g.stats.memory_nodes,
            g.stats.total_edges,
            100.0 - g.stats.isolated_pct,
        )
    });
    let hint = format!(
        "↑/↓ navigate · Tab switch · Esc close · {}{}",
        pane_hint, graph_stats
    );
    frame.render_widget(
        Paragraph::new(vec![Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ))]),
        footer,
    );
}

fn render_skills_pane(frame: &mut Frame, state: &JourneyViewState, area: Rect) {
    let is_active = state.active_pane == JourneyPane::Skills;
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let title_style = if is_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" Skills ({}) ", state.skills.len()),
            title_style,
        ));
    let inner = {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    };

    if state.skills.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "No skills installed.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  operant skills install <path>",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        format!(" {:<24} {:<14} {:<6}", "Name", "Category", "Ver"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    let viewport = inner.height.saturating_sub(4) as usize;
    let start = state
        .skills_scroll
        .min(state.skills.len().saturating_sub(viewport));
    let end = (start + viewport).min(state.skills.len());

    for i in start..end {
        let skill = &state.skills[i];
        let is_sel = i == state.skills_cursor && is_active;
        let prefix = if is_sel { "›" } else { " " };
        let row_style = if is_sel {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let name = truncate(&skill.name, 24);
        let cat = truncate(&skill.category, 14);
        let ver = truncate(&skill.version, 6);
        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), row_style),
            Span::styled(format!("{:<24} ", name), row_style),
            Span::styled(format!("{:<14} ", cat), row_style),
            Span::styled(format!("{:<6}", ver), row_style),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_memories_pane(frame: &mut Frame, state: &JourneyViewState, area: Rect) {
    let is_active = state.active_pane == JourneyPane::Memories;
    let border_color = if is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let title_style = if is_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" Memories ({}) ", state.memories.len()),
            title_style,
        ));
    let inner = {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    };

    if state.memories.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "No memories stored yet.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Memories accumulate as you use operant.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "Use /memory to manage MEMORY.md directly.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        format!(" {:<10} {:<3} {:<14} {}", "Type", "Imp", "ID", "Content"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    let viewport = inner.height.saturating_sub(4) as usize;
    let start = state
        .memories_scroll
        .min(state.memories.len().saturating_sub(viewport));
    let end = (start + viewport).min(state.memories.len());

    let content_w = inner.width.saturating_sub(32) as usize;

    for i in start..end {
        let mem = &state.memories[i];
        let is_sel = i == state.memories_cursor && is_active;
        let prefix = if is_sel { "›" } else { " " };
        let row_style = if is_sel {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let imp_style = if mem.importance >= 70 {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else if mem.importance >= 40 {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mtype = truncate(&mem.block_type, 10);
        let id = truncate(&mem.id, 14);
        let content = truncate_first_line(&mem.content, content_w);
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", prefix), row_style),
            Span::styled(format!("{:<10} ", mtype), row_style),
            Span::styled(format!("{:<3} ", mem.importance), imp_style),
            Span::styled(format!("{:<14} ", id), row_style),
            Span::styled(content, row_style),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
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

/// Truncate the first line of `s` to `max` chars, appending `…` if cut.
/// Multi-line content collapses to its first line so the row stays one line tall.
fn truncate_first_line(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    truncate(first, max)
}
