//! models_panel rendering — the session-hub idiom, applied to slot assignment.
//!
//! Full-frame, one renderer: `paint_models_panel` clears the viewport first
//! (the same load-bearing Clear the session hub documents — `terminal.draw`
//! flushes a CELL DIFF, so a page that only writes its own rows leaves the chat
//! frame's composer and status bar underneath), then this draws INSTEAD of the
//! chat frame. The gate is the OVERLAYS row; this function assumes it.
//!
//! Layout: a scope sidebar on the left, the rows on the right, one hint row at
//! the bottom. The pick step (an armed slot) replaces the rows with the
//! candidate models and retitles the right pane — the sidebar stays, because the
//! operator's place in the panel must not vanish while they choose.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::tui::model_scope::ModelScope;
use crate::tui::models_panel::{ModelsPanel, RoleRow};
use crate::tui::theme_colors;

/// The sidebar's share of the body. Fixed rather than a ratio: the labels are
/// short ("Recent", "Roles", "OpenAI") and a percentage would squeeze them at
/// narrow widths and waste the whole pane at wide ones.
const SIDEBAR: u16 = 22;

/// Render the panel full-frame. Callers gate on `state.visible`.
pub fn render_models_panel(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    if !panel.visible {
        return;
    }

    let rows: Vec<Constraint> = vec![
        Constraint::Length(1), // title strip
        Constraint::Min(3),    // body
        Constraint::Length(1), // hints
    ];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows)
        .split(area);

    render_title(frame, panel, chunks[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR), Constraint::Min(20)])
        .split(chunks[1]);
    render_sidebar(frame, panel, cols[0]);
    if panel.assigning.is_some() {
        render_pick(frame, panel, cols[1]);
    } else {
        render_rows(frame, panel, cols[1]);
    }

    render_hints(frame, panel, chunks[2]);
}

fn render_title(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    // The pick step names the SLOT, not the scope: "assign Roles · vision" is
    // two nouns where the operator needs one. The scope is the sidebar's job and
    // stays there.
    let title = if let Some(slot) = panel.assigning {
        format!(" Models — assign {}", slot.name())
    } else {
        format!(" Models — {}", panel.scope.label())
    };
    let line = Line::from(Span::styled(
        title,
        Style::default()
            .fg(theme_colors::text())
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

/// The sidebar: one row per scope, the active one highlighted.
///
/// Counts are shown where they are known and cheap: Roles is the slot count and
/// All is the catalogue length. Recent's count is the recency list's, which is
/// an intersection with the catalogue rather than the file's length, so it is
/// computed from the rows the scope would show rather than carried separately.
fn render_sidebar(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    let items: Vec<ListItem> = panel
        .scope_rows()
        .iter()
        .enumerate()
        .map(|(index, scope)| {
            let count = panel.scope_count(scope);
            let label = scope.label();
            // The fixed scopes always show a count, including `Recent (0)`:
            // an absent number is how an operator concludes a scope is
            // broken, and a zero is how they learn it is simply empty.
            let text = format!("{label} ({count})");
            let style = if index == panel.scope_cursor {
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme_colors::muted())
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme_colors::muted()));
    frame.render_widget(List::new(items).block(block), area);
}

/// The Roles view: one row per slot.
fn render_rows(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    if matches!(panel.scope, ModelScope::Roles) {
        render_role_rows(frame, panel, area);
    } else {
        render_model_rows(frame, panel, area);
    }
}

fn role_line(row: &RoleRow, selected: bool) -> Line<'static> {
    let marker_style = match row.state {
        crate::tui::models_panel::SlotState::Assigned(_) => theme_colors::success(),
        crate::tui::models_panel::SlotState::Auto => theme_colors::muted(),
        crate::tui::models_panel::SlotState::Empty => theme_colors::error(),
    };
    let base = if selected {
        Style::default()
            .fg(theme_colors::text())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme_colors::text())
    };
    let live = if row.switches_conversation {
        Span::styled(" · live", Style::default().fg(theme_colors::accent()))
    } else {
        Span::raw("")
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", row.marker()),
            Style::default().fg(marker_style),
        ),
        Span::styled(row.slot.name().to_string(), base),
        live,
        Span::styled(
            format!("  {}", row.note()),
            Style::default().fg(theme_colors::muted()),
        ),
    ])
}

fn render_role_rows(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    let items: Vec<ListItem> = panel
        .roles
        .iter()
        .enumerate()
        .map(|(index, row)| ListItem::new(role_line(row, index == panel.selected)))
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::muted()));
    frame.render_widget(List::new(items).block(block), area);
}

fn model_line(model: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(theme_colors::accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme_colors::text())
    };
    Line::from(Span::styled(model.to_string(), style))
}

fn render_model_rows(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    if panel.models.is_empty() {
        let note = if matches!(panel.scope, ModelScope::Recent) {
            "No recently used models yet — a switch records one here."
        } else {
            "No models in this scope. Connect a provider or refresh the catalogue."
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme_colors::muted()));
        frame.render_widget(
            Paragraph::new(Span::styled(
                note,
                Style::default().fg(theme_colors::muted()),
            ))
            .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = panel
        .models
        .iter()
        .enumerate()
        .map(|(index, model)| ListItem::new(model_line(model, index == panel.selected)))
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::muted()));
    frame.render_widget(List::new(items).block(block), area);
}

/// The pick step: the candidates for the armed slot.
fn render_pick(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    if panel.pick.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme_colors::muted()));
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No model in the catalogue is available for this slot.",
                Style::default().fg(theme_colors::muted()),
            ))
            .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = panel
        .pick
        .iter()
        .enumerate()
        .map(|(index, model)| ListItem::new(model_line(model, index == panel.pick_cursor)))
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::muted()));
    frame.render_widget(List::new(items).block(block), area);
}

fn render_hints(frame: &mut Frame, panel: &ModelsPanel, area: Rect) {
    let hint = if panel.assigning.is_some() {
        " ↑↓ model · Enter assign · Esc cancel"
    } else if matches!(panel.scope, ModelScope::Roles) {
        " ↑↓ row · ←→ scope · Enter assign · x clear · Esc close"
    } else {
        " ↑↓ row · ←→ scope · Enter use · Esc close"
    };
    let mut spans = vec![Span::styled(
        hint,
        Style::default().fg(theme_colors::muted()),
    )];
    if let Some(notice) = &panel.notice {
        spans.push(Span::styled(
            format!("   {notice}"),
            Style::default().fg(theme_colors::accent()),
        ));
    }
    if let Some(refusal) = &panel.refusal {
        spans.push(Span::styled(
            format!("   {refusal}"),
            Style::default().fg(theme_colors::error()),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The whole viewport as a `Rect`. `Terminal::size()` returns a `Result`
    /// in this ratatui, and a test that unwraps it three times reads worse than
    /// one helper that states the conversion once.
    fn full_rect(terminal: &Terminal<TestBackend>) -> Rect {
        let size = terminal.size().expect("the test terminal has a size");
        Rect::new(0, 0, size.width, size.height)
    }

    fn panel_at(scope: ModelScope) -> ModelsPanel {
        let cfg = operant_core::config::AppConfig::default();
        let mut panel = ModelsPanel {
            visible: true,
            ..Default::default()
        };
        panel.open(
            scope,
            &cfg,
            &[],
            &[
                "gpt-4".into(),
                "openai/gpt-4o".into(),
                "anthropic/claude".into(),
            ],
        );
        panel
    }

    fn text_of(buffer: &ratatui::buffer::Buffer, area: Rect) -> String {
        let mut out = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn the_roles_view_paints_every_slot_and_the_live_marker() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let panel = panel_at(ModelScope::Roles);
        terminal
            .draw(|f| render_models_panel(f, &panel, f.area()))
            .expect("draw");
        let text = text_of(terminal.backend().buffer(), full_rect(&terminal));
        for slot in operant_core::model_roles::ALL_SLOTS {
            assert!(
                text.contains(slot.name()),
                "{} is missing from the painted panel:\n{text}",
                slot.name()
            );
        }
        assert!(text.contains("live"), "the conversation slot is marked");
        assert!(text.contains("Roles"), "the scope is named");
        assert!(
            text.contains("Enter assign"),
            "the footer advertises the action: {text}"
        );
    }

    #[test]
    fn the_pick_step_retitles_the_pane_and_lists_the_candidates() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let cfg = operant_core::config::AppConfig::default();
        let mut panel = panel_at(ModelScope::Roles);
        let vision = panel
            .roles
            .iter()
            .position(|row| row.slot == operant_core::model_roles::ModelSlot::Vision)
            .expect("the vision row");
        panel.selected = vision;
        panel.begin_assign(&cfg, &["gpt-4".into(), "claude".into()]);
        assert!(
            !panel.pick.is_empty(),
            "the pick step must be populated before it can be drawn"
        );
        terminal
            .draw(|f| render_models_panel(f, &panel, f.area()))
            .expect("draw");
        let text = text_of(terminal.backend().buffer(), full_rect(&terminal));
        assert!(
            text.contains("assign vision"),
            "the title names the SLOT, not the scope: {text}"
        );
        for model in &panel.pick {
            assert!(
                text.contains(model.as_str()),
                "the candidate {model} is missing from the pick pane: {text}"
            );
        }
        assert!(
            text.contains("Esc cancel"),
            "the footer offers the way back"
        );
    }

    #[test]
    fn an_empty_scope_says_why_rather_than_drawing_a_blank_pane() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut panel = panel_at(ModelScope::Recent);
        panel.models.clear();
        panel.selected = 0;
        terminal
            .draw(|f| render_models_panel(f, &panel, f.area()))
            .expect("draw");
        let text = text_of(terminal.backend().buffer(), full_rect(&terminal));
        assert!(
            text.contains("recently used"),
            "an empty Recent explains itself: {text}"
        );
    }
}
