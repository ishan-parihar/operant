// render/selection.rs — Text selection highlight, row cache, context menu.

use crate::tui::app::{App, ContextMenuKind};
use crate::tui::operant_app::tui_state::TuiState as _;
use crate::tui::theme_colors;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

pub(crate) fn cache_selectable_row_text(frame: &mut Frame, app: &App) {
    let selectable_area = app.last_selectable_area.get();
    if selectable_area.width == 0 || selectable_area.height == 0 {
        app.last_row_text.borrow_mut().clear();
        return;
    }
    let buf = frame.buffer_mut();
    let max_row = selectable_area
        .y
        .saturating_add(selectable_area.height)
        .saturating_sub(1);
    let max_col = selectable_area
        .x
        .saturating_add(selectable_area.width)
        .saturating_sub(1);
    let mut cache = app.last_row_text.borrow_mut();
    cache.clear();
    for row in selectable_area.y..=max_row {
        let mut s = String::new();
        for col in selectable_area.x..=max_col {
            if let Some(cell) = buf.cell_mut((col, row)) {
                let sym = cell.symbol();
                if sym.is_empty() || sym == "\0" {
                    s.push(' ');
                } else {
                    s.push_str(sym);
                }
            }
        }
        cache.insert(row, s);
    }
}

/// Post-render pass: blend an accent highlight onto selected cells and
/// extract the selection text into `app.selection_text`.
/// Post-render pass: refresh `app.selection_text` from the frame just drawn.
/// Painting is NOT done here — jcode's line-level highlight
/// (`operant_ui::ui_viewport`, `highlight_line_selection`) paints only the
/// characters a copy carries; this pass used to repaint every selected row
/// edge to edge on top of it, highlighting gutters, padding and tool rows.
pub(crate) fn apply_selection_highlight(_frame: &mut Frame, app: &App) {
    if app.selection_anchor.is_some() && app.selection_anchor != app.selection_focus {
        *app.selection_text.borrow_mut() = selection_copy_text(app);
    }
}

/// Display messages whose rows are interface chrome, not conversation text:
/// a selection that also covers conversation text never copies them.
///
/// `tool` is deliberately ABSENT: a tool call is conversation content the user
/// may need (the command, the path, the intent line), and jcode copies it —
/// its `ui_prepare` pushes tool rows through the same `push_auto` path as
/// prose, so every rendered line carries a raw plain line. Dropping them here
/// (iter-801) was the parity gap: a drag across prose and a tool call copied
/// the prose and silently lost the call.
const UI_ROLES: &[&str] = &[
    "meta",
    "usage",
    "swarm",
    "background_task",
    "system",
    "error",
];

/// Whether wrapped line `line` is interface chrome: a row with no line map
/// (header, padding) or one owned by a UI-role message (status rows,
/// system notices, error cards).
fn is_ui_row(
    frame: &crate::tui::operant_model::PreparedChatFrame,
    messages: &[crate::tui::operant_model::DisplayMessage],
    line: usize,
) -> bool {
    frame.wrapped_line_map(line).is_none()
        || frame
            .message_index_at_line(line)
            .and_then(|i| messages.get(i))
            .is_some_and(|m| UI_ROLES.contains(&m.role.as_str()))
}

/// Whether the current selection paints / copies wrapped line `line`: UI rows
/// are skipped unless the selection lies entirely inside UI rows (then the
/// user explicitly selected them). One predicate for highlight and copy, so
/// what is highlighted is exactly what is copied.
pub(crate) fn selection_line_selectable(app: &App, line: usize) -> bool {
    let (Some(anchor), Some(focus)) = (app.selection_anchor, app.selection_focus) else {
        return true;
    };
    let Some(frame) = crate::tui::operant_ui::last_chat_frame() else {
        return true;
    };
    let messages = app.display_messages();
    if !is_ui_row(&frame, messages, line) {
        return true;
    }
    let last = frame.total_wrapped_lines.saturating_sub(1);
    let (lo, hi) = (
        anchor.1.min(focus.1).min(last),
        anchor.1.max(focus.1).min(last),
    );
    (lo..=hi).all(|l| is_ui_row(&frame, messages, l))
}

/// The text a drag selection copies — clean document text, the jcode model
/// (`ui/copy_selection.rs::copy_selection_text_from_raw_lines_base`): both
/// endpoints map through the frame's `wrapped_line_map` to (logical line,
/// column), and whole LOGICAL lines are copied, so soft wraps never become
/// line breaks and gutter prefixes (`1›`, hanging indents) never copy.
///
/// Rows that are interface chrome — status rows and the header, which
/// carries no line map — are dropped when the selection also covers
/// conversation text; a selection entirely inside them copies them as shown.
/// Tool rows are NOT chrome: they copy like prose (see `UI_ROLES`).
/// Extracted synchronously from the last drawn frame, so release, Ctrl+C and
/// copy mode always see the current selection, including off-screen parts.
pub(crate) fn selection_copy_text(app: &App) -> String {
    let (Some(anchor), Some(focus)) = (app.selection_anchor, app.selection_focus) else {
        return String::new();
    };
    if anchor == focus {
        return String::new();
    }
    let (start, end) = if (anchor.1, anchor.0) <= (focus.1, focus.0) {
        (anchor, focus)
    } else {
        (focus, anchor)
    };
    let Some(frame) = crate::tui::operant_ui::last_chat_frame() else {
        return String::new();
    };
    let count = frame.total_wrapped_lines;
    if count == 0 {
        return String::new();
    }
    let end_line = end.1.min(count - 1);
    let start_line = start.1.min(end_line);

    let messages = app.display_messages();
    let ui_row = |line: usize| is_ui_row(&frame, messages, line);
    let all_ui = (start_line..=end_line).all(ui_row);
    let keep = |line: usize| all_ui || !ui_row(line);
    let Some(first) = (start_line..=end_line).find(|&l| keep(l)) else {
        return String::new();
    };
    let last = (start_line..=end_line)
        .rev()
        .find(|&l| keep(l))
        .unwrap_or(first);
    // An endpoint dropped as chrome moves inward to a line edge.
    let from_col = (first == start_line).then_some(start.0);
    let to_col = (last == end_line).then_some(end.0);

    match raw_selection_text(&frame, first, last, from_col, to_col, &keep) {
        Some(text) => text,
        // No line map at an endpoint (all-chrome selection): copy the rows
        // as displayed, minus gutter prefixes.
        None => {
            use crate::tui::operant_ui::chat_line_copy_slice as slice;
            (first..=last)
                .map(|line| {
                    let from = if line == first { from_col } else { None };
                    let to = if line == last { to_col } else { None };
                    slice(line, from, to).unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_string()
        }
    }
}

/// Copy logical (raw) lines between two selection endpoints. `from_col` /
/// `to_col` are inclusive SCREEN columns on the first / last wrapped line
/// (`None` = that line's edge). Raw lines owned by rows `keep` rejects are
/// skipped. `None` when an endpoint has no line map.
fn raw_selection_text(
    frame: &crate::tui::operant_model::PreparedChatFrame,
    first: usize,
    last: usize,
    from_col: Option<u16>,
    to_col: Option<u16>,
    keep: &dyn Fn(usize) -> bool,
) -> Option<String> {
    use crate::tui::operant_ui::{chat_text_col, display_col_slice, line_display_width};
    // jcode `raw_selection_point`: wrapped display column → raw column.
    let raw_point = |line: usize, screen_col: Option<u16>, at_end: bool| {
        let map = frame.wrapped_line_map(line)?;
        let copy_start = frame.wrapped_copy_offset(line).unwrap_or(0);
        let segment = map.end_col.saturating_sub(map.start_col);
        let local = match screen_col {
            Some(col) => chat_text_col(line, col)?
                .saturating_sub(copy_start)
                .min(segment),
            None if at_end => segment,
            None => 0,
        };
        Some((map.raw_line, map.start_col + local))
    };
    let (start_raw, start_col) = raw_point(first, from_col, false)?;
    // Inclusive screen column → exclusive end.
    let (end_raw, end_col) = raw_point(last, to_col.map(|c| c.saturating_add(1)), true)?;
    if end_raw < start_raw {
        return None;
    }
    let dropped: std::collections::HashSet<usize> = (first..=last)
        .filter(|&line| !keep(line))
        .filter_map(|line| frame.wrapped_line_map(line).map(|m| m.raw_line))
        .collect();
    let subs = chrome_free_substitutions(frame, first, last, start_raw, end_raw);

    let mut lines: Vec<String> = Vec::new();
    for raw in start_raw..=end_raw {
        if dropped.contains(&raw) {
            continue;
        }
        if let Some(sub) = subs.get(&raw) {
            if let Some(text) = sub {
                lines.push(text.clone());
            }
            continue;
        }
        let text = frame.raw_plain_line(raw).unwrap_or("");
        let width = line_display_width(text);
        let from = if raw == start_raw {
            start_col.min(width)
        } else {
            0
        };
        let to = if raw == end_raw {
            end_col.min(width)
        } else {
            width
        };
        let piece = if to > from {
            display_col_slice(text, from, to)
        } else {
            ""
        };
        let piece = piece.trim_end();
        // Dropping a chrome row must not leave a double blank line.
        if piece.is_empty() && lines.last().is_some_and(|l| l.is_empty()) {
            continue;
        }
        lines.push(piece.to_string());
    }
    Some(lines.join("\n").trim().to_string())
}

/// Raw lines whose rendered form is interface chrome around document text,
/// mapped to the text a copy should carry (`None` = drop the line):
/// - code block: frame rows (`┌─ lang`, `└─`) drop, body rows lose the `│ `
///   gutter;
/// - blockquote: every `│ ` gutter level is stripped;
/// - table: the `──┼──` rule drops, rows become tab-separated cells (pastes
///   as real cells in Docs, Word and Sheets).
fn chrome_free_substitutions(
    frame: &crate::tui::operant_model::PreparedChatFrame,
    first: usize,
    last: usize,
    start_raw: usize,
    end_raw: usize,
) -> std::collections::HashMap<usize, Option<String>> {
    use crate::tui::operant_markdown::CopyTargetKind;
    let mut subs = std::collections::HashMap::new();
    for target in frame.copy_targets.iter() {
        let is_code = matches!(target.kind, CopyTargetKind::CodeBlock { .. });
        if !(is_code || matches!(target.kind, CopyTargetKind::Blockquote)) {
            continue;
        }
        let rows = target.start_line.max(first)..target.end_line.min(last + 1);
        for row in rows {
            let Some(map) = frame.wrapped_line_map(row) else {
                continue;
            };
            let text = frame.raw_plain_line(map.raw_line).unwrap_or("");
            let sub = if is_code && (text.starts_with('┌') || text.starts_with('└')) {
                None
            } else if is_code {
                Some(
                    text.strip_prefix("│ ")
                        .or(text.strip_prefix('│'))
                        .unwrap_or(text)
                        .to_string(),
                )
            } else {
                let mut body = text;
                while let Some(rest) = body.strip_prefix("│ ").or(body.strip_prefix('│')) {
                    body = rest;
                }
                Some(body.to_string())
            };
            subs.insert(map.raw_line, sub);
        }
    }
    // Tables: a rule line made only of `─`/`┼` with the header above it and
    // body rows below, each carrying one fewer `│` than the rule has columns.
    let is_rule = |t: &str| t.contains('┼') && t.chars().all(|c| matches!(c, '─' | '┼' | ' '));
    for rule in start_raw..=end_raw {
        let Some(rule_text) = frame.raw_plain_line(rule) else {
            continue;
        };
        if !is_rule(rule_text) {
            continue;
        }
        let bars = rule_text.matches('┼').count();
        let cells_of = |raw: usize| {
            frame
                .raw_plain_line(raw)
                .filter(|t| t.matches('│').count() == bars)
                .map(|t| {
                    t.split('│')
                        .map(|c| c.trim().to_string())
                        .collect::<Vec<_>>()
                })
        };
        subs.insert(rule, None);
        // Header above the rule (it may wrap upward), body rows below.
        let mut header_start = rule;
        while let Some(cells) = header_start.checked_sub(1).and_then(cells_of) {
            header_start -= 1;
            if cells.first().is_some_and(|c| !c.is_empty()) {
                break;
            }
        }
        let header: Vec<usize> = (header_start..rule).collect();
        let body: Vec<usize> = (rule + 1..)
            .take_while(|&raw| cells_of(raw).is_some())
            .collect();
        for group in [header, body] {
            for (raw, text) in table_rows_as_tsv(&group, &cells_of) {
                subs.insert(raw, text);
            }
        }
    }
    subs
}

/// Render a right-click context menu at the specified position.
pub(crate) fn render_context_menu(frame: &mut Frame, app: &App) {
    if let Some(menu) = app.context_menu_state {
        let selection_present = !app.selection_text.borrow().trim().is_empty();
        let items: Vec<(&str, bool)> = match menu.kind {
            ContextMenuKind::Message { message_index } => vec![
                ("Copy", app.messages.get(message_index).is_some()),
                ("Fork new chat", app.messages.get(message_index).is_some()),
            ],
            ContextMenuKind::Selection => vec![("Copy", selection_present)],
        };

        let menu_height = (items.len() as u16).saturating_add(2);
        let menu_width = items
            .iter()
            .map(|(label, _)| label.len())
            .max()
            .unwrap_or(4)
            .saturating_add(4) as u16;

        // Clamp menu position to screen bounds
        let screen = frame.area();
        let menu_x = menu.x.min(screen.width.saturating_sub(menu_width + 1));
        let menu_y = menu.y.min(screen.height.saturating_sub(menu_height + 1));

        let menu_area = Rect {
            x: menu_x,
            y: menu_y,
            width: menu_width,
            height: menu_height,
        };

        // Draw menu background with border
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(
                Style::default()
                    .fg(theme_colors::text())
                    .bg(theme_colors::panel_bg()),
            )
            .border_style(Style::default().fg(theme_colors::accent()));
        menu_block.render(menu_area, frame.buffer_mut());

        // Render menu items
        let inner = Rect {
            x: menu_area.x + 1,
            y: menu_area.y + 1,
            width: menu_area.width.saturating_sub(2),
            height: menu_area.height.saturating_sub(2),
        };

        for (idx, (label, enabled)) in items.iter().enumerate() {
            if idx >= inner.height as usize {
                break;
            }

            let y = inner.y + idx as u16;
            let is_selected = idx == menu.selected_index;

            let fg_color = if *enabled {
                if is_selected {
                    theme_colors::on_selection()
                } else {
                    theme_colors::text()
                }
            } else {
                theme_colors::disabled()
            };

            let bg_color = if is_selected {
                if *enabled {
                    theme_colors::selection_bg()
                } else {
                    theme_colors::panel_bg()
                }
            } else {
                theme_colors::panel_bg()
            };

            let style = Style::default().fg(fg_color).bg(bg_color);
            let padded_label = format!(
                " {:<width$} ",
                label,
                width = menu_width.saturating_sub(2) as usize
            );

            if let Some(cell) = frame.buffer_mut().cell_mut((inner.x, y)) {
                cell.set_symbol(&padded_label[0..1.min(padded_label.len())]);
                cell.set_style(style);
            }

            for (col_offset, ch) in padded_label.chars().enumerate() {
                if col_offset >= inner.width as usize {
                    break;
                }
                if let Some(cell) = frame
                    .buffer_mut()
                    .cell_mut((inner.x + col_offset as u16, y))
                {
                    cell.set_symbol(&ch.to_string());
                    cell.set_style(style);
                }
            }
        }
    }
}

// -----------------------------------------------------------------------
// Messages pane
// -----------------------------------------------------------------------

/// Tab-separated text for a run of table rows. A row whose first cell is
/// empty is a wrapped continuation of the row above (the renderer wraps long
/// cells onto extra rows): its text joins the cells above and the row itself
/// drops (`None`).
fn table_rows_as_tsv(
    raws: &[usize],
    cells_of: &dyn Fn(usize) -> Option<Vec<String>>,
) -> Vec<(usize, Option<String>)> {
    let mut rows: Vec<(usize, Vec<String>)> = Vec::new();
    let mut dropped = Vec::new();
    for &raw in raws {
        let Some(cells) = cells_of(raw) else { continue };
        match rows.last_mut() {
            Some((_, prev)) if cells.first().is_some_and(String::is_empty) => {
                for (into, more) in prev.iter_mut().zip(cells) {
                    if !more.is_empty() {
                        if !into.is_empty() {
                            into.push(' ');
                        }
                        into.push_str(&more);
                    }
                }
                dropped.push((raw, None));
            }
            _ => rows.push((raw, cells)),
        }
    }
    rows.into_iter()
        .map(|(raw, cells)| (raw, Some(cells.join("\t"))))
        .chain(dropped)
        .collect()
}
