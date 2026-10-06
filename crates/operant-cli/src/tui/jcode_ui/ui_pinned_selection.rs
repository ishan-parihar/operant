// Vendored from jcode (crates/jcode-tui/src/tui/ui_pinned_selection.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-3 map. See jcode_ui/mod.rs for scope.

use super::*;

use super::selection_highlight::highlight_line_selection;

pub(super) fn apply_side_selection_highlight(
    app: &dyn TuiState,
    visible_lines: &mut [Line<'static>],
    scroll: usize,
) {
    let Some(range) = app.copy_selection_range().filter(|range| {
        range.start.pane == crate::tui::jcode_ui::copy_selection::CopySelectionPane::SidePane
            && range.end.pane == crate::tui::jcode_ui::copy_selection::CopySelectionPane::SidePane
    }) else {
        return;
    };

    let (start, end) =
        if (range.start.abs_line, range.start.column) <= (range.end.abs_line, range.end.column) {
            (range.start, range.end)
        } else {
            (range.end, range.start)
        };

    let visible_end = scroll.saturating_add(visible_lines.len());
    for abs_idx in start.abs_line.max(scroll)..=end.abs_line.min(visible_end.saturating_sub(1)) {
        let rel_idx = abs_idx.saturating_sub(scroll);
        if let Some(line) = visible_lines.get_mut(rel_idx) {
            let start_col = if abs_idx == start.abs_line {
                start.column
            } else {
                0
            };
            let end_col = if abs_idx == end.abs_line {
                end.column
            } else {
                line.width()
            };
            *line = highlight_line_selection(line, start_col, end_col);
        }
    }
}
