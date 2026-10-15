// rewind_selector.rs — fullscreen /rewind selector (2026-10-14 batch, plan 05).
//
// Replaces the old RewindFlowOverlay modal list. The selector shows the REAL
// transcript (the body renderer's own wrapped lines), outlines the selected
// user turn, dims everything after it (the preview of what a rewind drops),
// and carries a filter over user prompts plus a keeps/tokens/drops footer.
// The marks come from the agent (`OperantAgent::rewind_marks`); Enter asks
// the agent to fork the session at the selected turn (run_pending_agent_actions).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::vendor::style::theme;

/// One rewindable user turn, mirrored from the agent's `UserTurnMark`.
#[derive(Debug, Clone)]
pub struct RewindEntry {
    pub turn_id: u64,
    /// The prompt text (relocation key; also the filter subject).
    pub text: String,
    /// True when the mark's prompt text no longer matches any live display
    /// row — the turn was folded away by compaction and cannot be rewound.
    pub compacted: bool,
}

#[derive(Debug, Default)]
pub struct RewindSelectorState {
    pub visible: bool,
    /// Mark list, synced from the agent each pass while open.
    pub entries: Vec<RewindEntry>,
    /// Cursor into `entries` (the selected turn, in mark order).
    pub selected: usize,
    /// Case-insensitive substring filter over prompt texts.
    pub filter: String,
    pub filter_open: bool,
    /// Inline notice (e.g. "That turn was folded by compaction…"), shown in
    /// the footer; cleared on the next cursor move.
    pub notice: Option<String>,
    /// Preview for the selection: (kept messages, estimated tokens).
    pub preview: Option<(usize, u64)>,
    /// Vertical scroll of the transcript body, in wrapped lines.
    pub scroll: usize,
}

impl RewindSelectorState {
    pub fn open(&mut self) {
        self.visible = true;
        self.selected = self.entries.len().saturating_sub(1);
        self.filter.clear();
        self.filter_open = false;
        self.notice = None;
        self.preview = None;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.filter_open = false;
        self.notice = None;
    }

    /// Mirror the agent's marks into entries; keep the cursor on the same
    /// turn id when possible, else the last turn. Called from
    /// run_pending_agent_actions while the selector is open (marks change as
    /// turns land).
    pub fn sync_marks(&mut self, marks: Vec<(u64, String)>, display_texts: &[String]) {
        let keep_id = self.entries.get(self.selected).map(|e| e.turn_id);
        self.entries = marks
            .into_iter()
            .map(|(turn_id, text)| RewindEntry {
                turn_id,
                compacted: !text.is_empty()
                    && !display_texts.iter().any(|t| prompt_texts_match(t, &text)),
                text,
            })
            .collect();
        if let Some(id) = keep_id
            && let Some(idx) = self.entries.iter().position(|e| e.turn_id == id)
        {
            self.selected = idx;
        } else {
            self.selected = self.entries.len().saturating_sub(1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.notice = None;
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
        self.notice = None;
    }

    /// The entries that pass the current filter, with their positions.
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        (0..self.entries.len())
            .filter(|&i| self.entries[i].text.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected_turn_id(&self) -> Option<u64> {
        self.entries.get(self.selected).map(|e| e.turn_id)
    }
}

/// Prompt-text equivalence for the compacted check: display prompt texts are
/// the wrapped first lines; the mark carries the full prompt. A prefix match
/// on the first 64 chars is the honest comparison available without storing
/// display-side turn ids.
fn prompt_texts_match(display: &str, mark: &str) -> bool {
    let d: String = display.chars().take(64).collect();
    let m: String = mark.chars().take(64).collect();
    !d.is_empty() && d == m
}

/// Paint the selector: the real transcript body (via the body renderer's own
/// prepared lines), the selected turn outlined, everything after it dimmed,
/// header with filter, footer with preview stats and hints.
pub fn draw_rewind_selector(frame: &mut Frame, area: Rect, app: &crate::tui::app::App) {
    use crate::tui::operant_ui::prepare::prepare_body_for_overlay;
    use ratatui::layout::{Constraint, Layout};

    let state = &app.rewind_selector;
    let width = area.width;

    // Header: title (+ filter line when open) — PINNED.
    let title_style = Style::default()
        .fg(theme::accent_color())
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme::dim_color());
    let mut header: Vec<Line> = vec![Line::from(vec![
        Span::styled("  Rewind to a previous prompt ", title_style),
        Span::styled("· Enter to rewind · f filter · Esc cancel", dim),
    ])];
    if state.filter_open {
        header.push(Line::from(vec![
            Span::styled("  filter: ", dim),
            Span::raw(state.filter.clone()),
            Span::styled("█", title_style),
        ]));
    }

    // The real transcript, from the body renderer — the scrollable middle.
    let prepared = prepare_body_for_overlay(app, width);
    let body = &prepared.wrapped_lines;

    let sel_start = user_prompt_range(&prepared, app).or_else(|| {
        let sel_text = state.entries.get(state.selected)?.text.clone();
        let idx = prepared.user_prompt_texts.iter().position(|t| {
            let a: String = t.chars().take(64).collect();
            let b: String = sel_text.chars().take(64).collect();
            a == b
        })?;
        prepared.wrapped_user_prompt_starts.get(idx).copied()
    });
    let dim_from = sel_start.map(|start| {
        let idx = prepared
            .wrapped_user_prompt_starts
            .iter()
            .position(|&s| s == start)
            .map(|i| i + 1);
        idx.and_then(|next| prepared.wrapped_user_prompt_ends.get(next - 1).copied())
            .unwrap_or(start)
    });

    let mut mid: Vec<Line> = Vec::new();
    for (i, line) in body.iter().enumerate() {
        let is_dimmed = dim_from.is_some_and(|from| i >= from);
        let outlined = sel_start.is_some_and(|s| i >= s && dim_from.is_some_and(|d| i < d));
        let mut styled = line.clone();
        if is_dimmed {
            styled = restyle_dim(styled);
        } else if outlined {
            styled = Line::from(
                styled
                    .spans
                    .iter()
                    .map(|s| {
                        Span::styled(
                            s.content.to_string(),
                            s.style.add_modifier(Modifier::REVERSED),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        mid.push(styled);
    }
    if state.entries.is_empty() {
        mid.push(Line::from(""));
        mid.push(Line::styled(
            "  No rewindable turns yet — send a prompt first.",
            dim,
        ));
    }

    // Footer: preview stats + inline notice — PINNED.
    let mut footer: Vec<Line> = vec![Line::from("")];
    let stats = match state.preview {
        Some((msgs, tokens)) => format!(
            "  Keeps {msgs} messages · ~{tokens} tokens · drops {} turns",
            state.entries.len().saturating_sub(state.selected + 1)
        ),
        None => "  …".to_string(),
    };
    footer.push(Line::from(Span::styled(stats, dim)));
    if let Some(notice) = &state.notice {
        footer.push(Line::from(Span::styled(
            format!("  {notice}"),
            Style::default().fg(theme::asap_color()),
        )));
    }

    // Header/body/footer split: only the middle scrolls.
    let header_h = header.len() as u16;
    let footer_h = footer.len() as u16;
    let chunks = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(0),
        Constraint::Length(footer_h),
    ])
    .split(area);

    frame.render_widget(Paragraph::new(header), chunks[0]);

    let body_h = chunks[1].height as usize;
    let scroll = state.scroll.min(mid.len().saturating_sub(body_h));
    frame.render_widget(Paragraph::new(mid).scroll((scroll as u16, 0)), chunks[1]);

    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

/// The wrapped-line range of the selector's CURRENT cursor turn, resolved
/// against the display prompts in order (marks and display prompts are both
/// in conversation order, so position aligns while nothing is compacted).
fn user_prompt_range(
    prepared: &crate::tui::operant_model::PreparedMessages,
    app: &crate::tui::app::App,
) -> Option<usize> {
    let sel = app.rewind_selector.selected;
    prepared.wrapped_user_prompt_starts.get(sel).copied()
}

fn restyle_dim(line: Line<'static>) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|s| {
                Span::styled(
                    s.content.to_string(),
                    s.style.fg(theme::dim_color()).add_modifier(Modifier::DIM),
                )
            })
            .collect::<Vec<_>>(),
    )
}
