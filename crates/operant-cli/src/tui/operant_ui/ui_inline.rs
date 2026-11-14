// Vendored from jcode (crates/operant-tui/src/tui/ui_inline.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-3 map. See operant_ui/mod.rs for scope.

use super::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

fn inline_view_display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(super) fn inline_ui_height(app: &dyn TuiState) -> u16 {
    match app.inline_ui_state() {
        Some(crate::tui::operant_app::tui_fns::InlineUiStateRef::Interactive(picker)) => {
            // Model choices replace the command suggestions, without reflowing chat.
            // (2026-11-15, model plan M1) The effort picker does the same: it is
            // the same surface shape with different rows, so it reserves no rows
            // either. Left on the counted path it claimed a block of up to twenty
            // rows above the composer for four levels.
            if matches!(
                picker.kind,
                crate::tui::operant_app::tui_fns::PickerKind::Model
                    | crate::tui::operant_app::tui_fns::PickerKind::Effort
            ) {
                return 0;
            }
            let visible_rows = picker.filtered.len() as u16;
            let rows_needed = visible_rows + 1 + 2; // header + rounded border
            rows_needed.min(20)
        }
        Some(crate::tui::operant_app::tui_fns::InlineUiStateRef::View(view)) => {
            let visible_rows = view.lines.len().max(1) as u16;
            let rows_needed = visible_rows + 1 + 2; // header + rounded border
            rows_needed.min(10)
        }
        None => 0,
    }
}

pub(super) fn draw_inline_ui(frame: &mut Frame, app: &dyn TuiState, area: Rect) {
    match app.inline_ui_state() {
        // Both pickers draw through the command-palette surface rather than the
        // bordered one; the kind now says which, instead of the effort picker
        // having to impersonate the model picker to be drawn at all.
        Some(crate::tui::operant_app::tui_fns::InlineUiStateRef::Interactive(picker))
            if !matches!(
                picker.kind,
                crate::tui::operant_app::tui_fns::PickerKind::Model
                    | crate::tui::operant_app::tui_fns::PickerKind::Effort
            ) =>
        {
            super::inline_interactive_ui::draw_inline_interactive(frame, app, area)
        }
        Some(crate::tui::operant_app::tui_fns::InlineUiStateRef::View(view)) => {
            draw_inline_view(frame, app, view, area)
        }
        _ => {}
    }
}

fn draw_inline_view(
    frame: &mut Frame,
    app: &dyn TuiState,
    view: &crate::tui::operant_app::tui_fns::InlineViewState,
    area: Rect,
) {
    let height = area.height as usize;
    let width = area.width as usize;
    if height <= 2 || width <= 2 {
        return;
    }

    let mut content_width = inline_view_display_width(view.title.as_str());
    if let Some(status) = view.status.as_ref() {
        content_width = content_width.max(inline_view_display_width(status.as_str()) + 2);
    }
    for line in &view.lines {
        content_width = content_width.max(inline_view_display_width(line.as_str()));
    }
    let content_width = content_width.min(width.saturating_sub(2)).max(1);
    let outer_width = content_width.saturating_add(2).min(width);
    let horizontal_offset = if app.centered_mode() {
        area.width.saturating_sub(outer_width as u16) / 2
    } else {
        0
    };
    let render_area = Rect {
        x: area.x + horizontal_offset,
        y: area.y,
        width: outer_width as u16,
        height: area.height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(rgb(85, 85, 110)))
        .style(Style::default().bg(rgb(18, 18, 26)));
    frame.render_widget(block.clone(), render_area);

    let inner = block.inner(render_area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut header_spans = vec![Span::styled(
        view.title.clone(),
        Style::default().fg(Color::White).bold(),
    )];
    if let Some(status) = view.status.as_ref() {
        header_spans.push(Span::styled(
            format!("  {}", status),
            Style::default().fg(dim_color()).italic(),
        ));
    }
    lines.push(Line::from(header_spans));

    for line in &view.lines {
        lines.push(Line::from(Span::styled(
            line.clone(),
            Style::default().fg(rgb(200, 200, 220)),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
