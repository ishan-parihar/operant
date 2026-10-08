// Vendored from jcode (crates/operant-tui/src/tui/ui_pinned_table.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-3 map. See operant_ui/mod.rs for scope.

use ratatui::text::Line;

pub(crate) fn is_rendered_table_line(line: &Line<'_>) -> bool {
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    text.contains(" │ ") || text.contains("─┼─")
}
