// render/selection.rs — Text selection highlight, row cache, context menu.

use crate::tui::app::{App, ContextMenuKind};
use crate::tui::operant_app::tui_state::TuiState as _;
use crate::tui::operant_model::{DisplayMessage, PreparedChatFrame};
use crate::tui::theme_colors;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Borders, Widget};
use unicode_width::UnicodeWidthStr;

/// Clean-copy text for a run of fully-covered transcript lines, rebuilt from
/// the pre-wrap source so wrapped fragments rejoin into their logical line
/// with the ORIGINAL spacing (2026-10-09 live-audit P1-6: cell-based copy
/// reflowed nothing — every hard-wrapped row copied as its own line with the
/// wrap padding baked in, and tool-status rows copied as text).
///
/// Lines owned by a `tool` display message are skipped: the copied text is
/// the conversation's TEXT, not the ✓-status chrome. The filter needs the
/// per-message `message_boundaries` to line up with `messages`; when they
/// don't (synthetic/partial bodies) the filter degrades to include-everything
/// rather than misclassify.
///
/// Returns `None` when any covered line lacks map data (e.g. a synthetic
/// body) — the caller falls back to cell extraction for the whole run.
pub(crate) fn reflowed_selection_text(
    frame: &PreparedChatFrame,
    messages: &[DisplayMessage],
    from_line: usize,
    to_line: usize,
) -> Option<String> {
    use crate::tui::operant_model::WrappedLineMap;

    let mut text = String::new();
    // The raw line the previous emitted group came from, so a new logical
    // line starts a new output line; `usize::MAX` seeds "no group yet".
    let mut prev_raw: usize = usize::MAX;
    let mut wrote_any = false;

    'lines: for line in from_line..=to_line {
        // Find the section owning this content line.
        let section = frame.sections.iter().find(|s| {
            line >= s.line_start && line < s.line_start + s.prepared.wrapped_lines.len()
        })?;
        let local = line - section.line_start;
        let prepared = &section.prepared;
        let map: WrappedLineMap = *prepared.wrapped_line_map.get(local)?;
        let raw = prepared.raw_plain_lines.get(map.raw_line)?;

        // Tool-row filter: the owning message via cumulative boundaries.
        if prepared.message_boundaries.len() == messages.len() {
            if let Some(owner) = prepared
                .message_boundaries
                .iter()
                .position(|b| b.wrapped_len > local)
            {
                if messages.get(owner).is_some_and(|m| m.role == "tool") {
                    // Skip the whole group: mark so the next text line does not
                    // merge with the one before the skipped chrome.
                    prev_raw = usize::MAX;
                    continue 'lines;
                }
            }
        }

        if map.raw_line != prev_raw {
            if wrote_any {
                text.push('\n');
            }
            prev_raw = map.raw_line;
            wrote_any = true;
        }

        // Char-column slice of the source line: WrappedLineMap columns are
        // char offsets into the raw line (the renderer records them while
        // wrapping), so slice on char boundaries.
        let start = map.start_col.min(raw.chars().count());
        let end = map.end_col.min(raw.chars().count()).max(start);
        for (i, ch) in raw.chars().enumerate().skip(start).take(end - start) {
            let _ = i;
            text.push(ch);
        }
    }
    Some(text)
}

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
/// Blend source for a selected cell carrying no background of its own. Same
/// neutral jcode falls back to in `selection_highlight.rs:11`.
const SELECTION_FALLBACK_BG: [u8; 3] = [32, 38, 48];
/// Neutral grey for a ratatui *named* colour, which carries no RGB value.
const NEUTRAL_RGB: [u8; 3] = [128, 128, 128];
/// Selection fill strength. Deliberately strong: a selection is an active
/// interaction, not passive decoration, and it has to stay obvious over syntax
/// highlighting, diff backgrounds, and dim assistant text.
const SELECTION_BLEND: f32 = 0.58;
/// How far a selected cell's foreground is pulled toward white so text stays
/// legible on that fill.
const SELECTION_FG_BLEND: f32 = 0.32;

/// Resolve a ratatui colour to 24-bit RGB.
///
/// Ported from jcode `operant-tui-style/src/theme.rs:145-167`. Indexed colours go
/// through operant's vendored xterm-256 table; a *named* colour carries no RGB
/// value of its own, so it falls back to neutral grey rather than being
/// silently treated as black.
fn color_to_rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(idx) => crate::tui::vendor::style::color::indexed_to_rgb(idx).into(),
        Color::White => [255, 255, 255],
        Color::Black => [0, 0, 0],
        Color::Gray => [192, 192, 192],
        _ => NEUTRAL_RGB,
    }
}

/// Linear blend from `from` toward `to` by `t`, returned as a 24-bit colour.
///
/// Ported from jcode `blend_color`: interpolate each channel in float space and
/// clamp back into byte range.
fn blend_rgb(from: [u8; 3], to: [u8; 3], t: f32) -> Color {
    let mix = |a: u8, b: u8| -> u8 {
        let (a, b) = (f32::from(a), f32::from(b));
        (a + (b - a) * t).clamp(0.0, 255.0) as u8
    };
    Color::Rgb(
        mix(from[0], to[0]),
        mix(from[1], to[1]),
        mix(from[2], to[2]),
    )
}

/// The selected cell's background: its own background blended toward the theme
/// accent, so the highlight reads over whatever the cell already sat on rather
/// than replacing it with one flat colour.
fn selection_bg_for(base_bg: Option<Color>) -> Color {
    let base = base_bg.map_or(SELECTION_FALLBACK_BG, color_to_rgb);
    blend_rgb(base, color_to_rgb(theme_colors::accent()), SELECTION_BLEND)
}

/// The selected cell's foreground: its own foreground pulled toward white. An
/// unset foreground stays unset - the terminal default is already right there.
fn selection_fg_for(base_fg: Option<Color>) -> Option<Color> {
    base_fg.map(|fg| blend_rgb(color_to_rgb(fg), [255, 255, 255], SELECTION_FG_BLEND))
}

/// True when the glyph in `col` is double-width and so also occupies the cell
/// to its right.
fn glyph_spans_next_cell(buf: &Buffer, col: u16, row: u16) -> bool {
    match buf.cell((col, row)) {
        Some(cell) => UnicodeWidthStr::width(cell.symbol()) > 1,
        None => false,
    }
}

/// True when `col` is covered by a double-width glyph whose lead cell sits at
/// `col - 1`.
///
/// Glyph *width*, not the symbol, is the signal here. `Buffer::set_stringn`
/// calls `reset()` on the cells a wide grapheme covers, and a reset cell reads
/// back as a single blank - indistinguishable from an ordinary empty cell. Only
/// the lead cell still carries the glyph, and its width is what says how far
/// that glyph reaches.
fn is_continuation(buf: &Buffer, col: u16, row: u16) -> bool {
    col > 0 && glyph_spans_next_cell(buf, col - 1, row)
}

/// Expand `[from, to]` so a double-width glyph the drag only partly covered is
/// selected in full.
///
/// ratatui stores a wide glyph as a lead cell carrying the symbol plus reset
/// cells behind it. Styling only the cells the drag happened to land on paints
/// half a glyph whenever a boundary falls inside one, so a run touching any cell
/// of a glyph pulls in all of them.
fn expand_selection_columns(
    buf: &Buffer,
    row: u16,
    from: u16,
    to: u16,
    min_col: u16,
) -> (u16, u16) {
    let mut start = from;
    while start > min_col && is_continuation(buf, start, row) {
        start -= 1;
    }
    let mut end = to;
    while glyph_spans_next_cell(buf, end, row) {
        end += 1;
    }
    (start, end)
}
pub(crate) fn apply_selection_highlight(frame: &mut Frame, app: &App) {
    let (anchor, focus) = match (app.selection_anchor, app.selection_focus) {
        (Some(a), Some(f)) => (a, f),
        _ => return,
    };
    if anchor == focus {
        return;
    }

    let selectable_area = app.last_selectable_area.get();
    if selectable_area.width == 0 || selectable_area.height == 0 {
        return;
    }

    // Selection points are scroll-stable content lines `(col, line)`
    // (iter-672). Project them onto the current viewport before painting —
    // the same way jcode resolves its `CopySelectionPoint`s against the
    // rendered viewport — so a selection made before a scroll stays glued
    // to its text instead of sliding off it.
    let scroll = app.last_render_scroll_offset.get() as usize;
    let first_visible = scroll;
    // Exclusive end of the visible content-line range.
    let visible_end = scroll.saturating_add(selectable_area.height as usize);

    let max_col = selectable_area
        .x
        .saturating_add(selectable_area.width)
        .saturating_sub(1);

    // Clamp columns into the frame; clamp lines into the visible window so a
    // partially offscreen selection paints (and copies) its visible part.
    let anchor = (
        anchor.0.clamp(selectable_area.x, max_col),
        anchor.1.clamp(first_visible, visible_end.saturating_sub(1)),
    );
    let focus = (
        focus.0.clamp(selectable_area.x, max_col),
        focus.1.clamp(first_visible, visible_end.saturating_sub(1)),
    );

    // Normalise so start ≤ end (row-major order).
    let (start, end) = if (anchor.1, anchor.0) <= (focus.1, focus.0) {
        (anchor, focus)
    } else {
        (focus, anchor)
    };

    // Clean-copy (P1-6): interior fully-covered lines copy from the pre-wrap
    // source via the frame's line map — wrapped fragments rejoin into
    // logical lines with original spacing and tool-status rows drop out.
    // Must be read before the mutable buffer borrow below. When the frame
    // lacks map data the run degrades to full cell extraction (the loop
    // below) exactly as before.
    let use_reflow = end.1 >= start.1.saturating_add(2);
    let interior_text = if use_reflow {
        crate::tui::operant_ui::last_chat_frame()
            .as_deref()
            .and_then(|f| {
                reflowed_selection_text(f, app.display_messages(), start.1 + 1, end.1 - 1)
            })
    } else {
        None
    };
    let use_reflow = interior_text.is_some();

    let buf = frame.buffer_mut();
    let mut text = String::new();
    let mut wrote_interior = false;
    let last_line = end.1;
    for line in start.1..=last_line {
        let row = selectable_area.y + (line - scroll) as u16;
        let col_from = if line == start.1 {
            start.0
        } else {
            selectable_area.x
        };
        let col_to = if line == end.1 { end.0 } else { max_col };
        // A drag boundary landing inside a double-width glyph would otherwise
        // paint half of it; pull the whole glyph in first.
        let (col_from, col_to) =
            expand_selection_columns(buf, row, col_from, col_to, selectable_area.x);
        // Interior lines with reflow data are painted (highlighted) but their
        // copy text comes from the pre-wrap source, spliced in once between
        // the two cell-extracted edge lines.
        let collect_cells = if use_reflow {
            line == start.1 || line == end.1
        } else {
            true
        };
        if use_reflow && !collect_cells && !wrote_interior {
            // First skipped interior line: emit the source-rebuilt block.
            // The start edge above has already emitted its cell text.
            while text.ends_with(|c: char| c.is_whitespace()) {
                text.pop();
            }
            text.push('\n');
            if let Some(int) = interior_text.as_deref() {
                text.push_str(int);
            }
            wrote_interior = true;
        }
        let mut line_text = String::new();
        for col in col_from..=col_to {
            if let Some(cell) = buf.cell_mut((col, row)) {
                let sym = cell.symbol().to_owned();
                if collect_cells {
                    line_text.push_str(if sym.is_empty() || sym == "\0" {
                        " "
                    } else {
                        &sym
                    });
                }
                // Highlight: the cell's own background blended toward the theme
                // accent and its own foreground pulled toward white. Rebuilt from
                // the cell's existing style rather than Style::default() so bold,
                // italic and underline on the cell survive the selection.
                let base = cell.style();
                let new_style = Style {
                    bg: Some(selection_bg_for(base.bg)),
                    fg: selection_fg_for(base.fg),
                    ..base
                };
                cell.set_style(new_style);
            }
        }
        if collect_cells && wrote_interior && line == end.1 {
            // The reflow block ends without a newline; separate it from the
            // end edge's cell text before that text appends.
            text.push('\n');
        }
        if collect_cells {
            text.push_str(&line_text);
        }
        if line < last_line && collect_cells {
            // Trim trailing spaces from line before newline
            while text.ends_with(' ') {
                text.pop();
            }
            text.push('\n');
        }
    }
    while text.ends_with(|c: char| c.is_whitespace()) {
        text.pop();
    }
    *app.selection_text.borrow_mut() = text;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-row buffer holding `text` from column 0.
    fn buffer_with(text: &str) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        buf.set_string(0, 0, text, Style::default());
        buf
    }

    // -- P1-6: reflowed clean copy ----------------------------------------

    use crate::tui::operant_model::vendor_types::ToolCall;
    use crate::tui::operant_model::{MessageBoundary, PreparedMessages, PreparedSectionKind};
    use ratatui::text::Line;
    use std::sync::Arc;

    fn prepared_body(
        raws: Vec<&str>,
        maps: Vec<(usize, usize, usize)>,
        boundaries: Vec<(u64, usize)>,
    ) -> Arc<PreparedMessages> {
        Arc::new(PreparedMessages {
            wrapped_lines: maps.iter().map(|_| Line::from("x")).collect(),
            wrapped_plain_lines: Arc::new(Vec::new()),
            wrapped_copy_offsets: Arc::new(Vec::new()),
            raw_plain_lines: Arc::new(raws.into_iter().map(str::to_string).collect()),
            wrapped_line_map: Arc::new(
                maps.iter()
                    .map(|&(raw, s, e)| crate::tui::operant_model::WrappedLineMap {
                        raw_line: raw,
                        start_col: s,
                        end_col: e,
                    })
                    .collect(),
            ),
            wrapped_user_indices: Vec::new(),
            wrapped_user_prompt_starts: Vec::new(),
            wrapped_user_prompt_ends: Vec::new(),
            user_prompt_texts: Vec::new(),
            image_regions: Vec::new(),
            edit_tool_ranges: Vec::new(),
            copy_targets: Vec::new(),
            message_boundaries: boundaries
                .into_iter()
                .map(|(h, len)| MessageBoundary {
                    msg_hash: h,
                    wrapped_len: len,
                    raw_len: 0,
                    user_prompt_len: 0,
                })
                .collect(),
            mermaid_pending_epoch: None,
        })
    }

    fn frame_of(p: Arc<PreparedMessages>) -> PreparedChatFrame {
        PreparedChatFrame::from_sections(vec![(PreparedSectionKind::Body, p)])
    }

    #[test]
    fn reflow_joins_wrapped_fragments_into_the_logical_line() {
        // One raw line hard-wrapped into two rows: the copy must rejoin them
        // with the source's own spacing, not the wrap padding.
        let raw = "The quick brown fox jumps";
        let f = frame_of(prepared_body(
            vec![raw],
            vec![(0, 0, 12), (0, 12, 25)],
            vec![(1, 2)],
        ));
        let msgs = vec![DisplayMessage::assistant(raw)];
        assert_eq!(
            reflowed_selection_text(&f, &msgs, 0, 1).as_deref(),
            Some("The quick brown fox jumps")
        );
    }

    #[test]
    fn reflow_separates_logical_lines_and_drops_tool_chrome() {
        // Two assistant lines around a tool status row: the copy carries the
        // text lines only, chrome excluded.
        let f = frame_of(prepared_body(
            vec!["line one", "\u{2713} aft_read \u{b7} 33 tok", "line two"],
            vec![(0, 0, 8), (1, 0, 18), (2, 0, 8)],
            vec![(1, 1), (2, 2), (3, 3)],
        ));
        let msgs = vec![
            DisplayMessage::assistant("line one"),
            DisplayMessage::tool(
                "\u{2713} aft_read \u{b7} 33 tok",
                ToolCall {
                    id: "c1".into(),
                    name: "aft_read".into(),
                    input: serde_json::Value::Null,
                    intent: None,
                    thought_signature: None,
                },
            ),
            DisplayMessage::assistant("line two"),
        ];
        assert_eq!(
            reflowed_selection_text(&f, &msgs, 0, 2).as_deref(),
            Some("line one\nline two")
        );
    }

    #[test]
    fn reflow_falls_back_to_none_without_map_data() {
        let mut p = prepared_body(vec!["x"], vec![(0, 0, 1)], vec![(1, 1)]);
        let f = frame_of(p.clone());
        let msgs = vec![DisplayMessage::assistant("x")];
        Arc::make_mut(&mut p).wrapped_line_map = Arc::new(Vec::new());
        let f = PreparedChatFrame::from_sections(vec![(PreparedSectionKind::Body, p)]);
        assert_eq!(reflowed_selection_text(&f, &msgs, 0, 0), None);
    }

    // -- The blend --------------------------------------------------------

    #[test]
    fn blend_at_zero_is_the_identity() {
        assert_eq!(
            blend_rgb([10, 20, 30], [200, 100, 50], 0.0),
            Color::Rgb(10, 20, 30)
        );
    }

    #[test]
    fn blend_at_one_is_the_target() {
        assert_eq!(
            blend_rgb([10, 20, 30], [200, 100, 50], 1.0),
            Color::Rgb(200, 100, 50)
        );
    }

    #[test]
    fn blend_halfway_is_the_per_channel_midpoint() {
        // Each channel interpolates on its own; 10 -> 20 at 0.5 is 15.
        assert_eq!(
            blend_rgb([0, 10, 20], [100, 20, 40], 0.5),
            Color::Rgb(50, 15, 30)
        );
    }

    #[test]
    fn the_selection_fill_moves_a_real_distance_off_the_cells_own_background() {
        // The whole point of 0.58: the fill has to visibly leave whatever the
        // cell already sat on, not tint it imperceptibly.
        //
        // Pinned against the LITERAL 0.58, not against SELECTION_BLEND. An
        // earlier version recomputed the expectation from the constant, so
        // changing 0.58 -> 0.30 kept this green: a test that derives its own
        // expectation from the code under test cannot fail. The literal is
        // deliberately duplicated here.
        let base = 10u8;
        let blended = selection_bg_for(Some(Color::Rgb(base, base, base)));
        let Color::Rgb(r, g, b) = blended else {
            panic!("expected an rgb blend, got {blended:?}");
        };
        let accent = color_to_rgb(theme_colors::accent());
        for (got, accent_channel) in [(r, accent[0]), (g, accent[1]), (b, accent[2])] {
            let start = f32::from(base);
            let expected = start + (f32::from(accent_channel) - start) * 0.58;
            let drift = (f32::from(got) - expected).abs();
            assert!(
                drift < 1.5,
                "channel {got} is not within rounding of the 58% blend {expected}"
            );
        }
    }

    #[test]
    fn the_blend_strength_is_the_documented_58_percent() {
        // Guards the constant itself, since it now appears as a literal in the
        // test above and would otherwise drift silently.
        assert_eq!(SELECTION_BLEND, 0.58);
    }

    #[test]
    fn a_cell_with_no_background_blends_from_the_documented_fallback() {
        // Also pinned to literals: SELECTION_FALLBACK_BG = [32,38,48] and a 58%
        // pull toward the accent. Computed by hand rather than via blend_rgb, so
        // this stays a check on the code and not a restatement of it.
        //
        // Truncation, not rounding: blend_rgb ends in `as u8`, which floors a
        // positive float. The first version of this test used .round() and was
        // off by one on green (127 vs the real 126). The expected values below
        // are therefore written as the same float expression with a truncating
        // cast, which keeps the arithmetic independent while matching how the
        // port actually behaves.
        let accent = color_to_rgb(theme_colors::accent());
        let trunc = |start: f32, to: f32| (start + (to - start) * 0.58).clamp(0.0, 255.0) as u8;
        let expected = Color::Rgb(
            trunc(32.0, f32::from(accent[0])),
            trunc(38.0, f32::from(accent[1])),
            trunc(48.0, f32::from(accent[2])),
        );
        assert_eq!(selection_bg_for(None), expected);
    }

    #[test]
    fn an_unset_foreground_stays_unset() {
        assert_eq!(selection_fg_for(None), None);
    }

    #[test]
    fn a_selected_foreground_is_pulled_toward_white() {
        let Some(Color::Rgb(r, g, b)) = selection_fg_for(Some(Color::Rgb(0, 0, 0))) else {
            panic!("expected an rgb foreground");
        };
        // Black pulled 32% toward white lands near 82 on every channel.
        for got in [r, g, b] {
            assert!(
                (70..95).contains(&got),
                "expected roughly 82, got {got} - the fg blend is not toward white"
            );
        }
    }

    #[test]
    fn named_colours_resolve_to_their_rgb_and_indexed_goes_through_the_vendor_table() {
        assert_eq!(color_to_rgb(Color::White), [255, 255, 255]);
        assert_eq!(color_to_rgb(Color::Black), [0, 0, 0]);
        let rgb: [u8; 3] = color_to_rgb(Color::Indexed(196));
        let [r, g, b] = rgb;
        assert_eq!(
            [r, g, b],
            <[u8; 3]>::from(crate::tui::vendor::style::color::indexed_to_rgb(196))
        );
    }

    // -- Wide-glyph expansion --------------------------------------------

    #[test]
    fn a_drag_starting_inside_a_wide_glyph_pulls_in_the_lead_cell() {
        // The glyph occupies columns 0 and 1; a drag at 1 is its second half.
        let buf = buffer_with("漢字");
        assert_eq!(expand_selection_columns(&buf, 0, 1, 1, 0), (0, 1));
    }

    #[test]
    fn a_drag_ending_on_a_wide_glyph_lead_pulls_in_the_trailing_cell() {
        // A drag on the lead cell at 2 must reach the glyph trailing cell at 3.
        let buf = buffer_with("漢字");
        assert_eq!(expand_selection_columns(&buf, 0, 2, 2, 0), (2, 3));
    }

    #[test]
    fn a_drag_over_an_ascii_run_is_left_alone() {
        let buf = buffer_with("abc");
        assert_eq!(expand_selection_columns(&buf, 0, 1, 1, 0), (1, 1));
    }

    #[test]
    fn a_drag_already_spanning_a_wide_glyph_is_left_alone() {
        let buf = buffer_with("漢字");
        assert_eq!(expand_selection_columns(&buf, 0, 0, 3, 0), (0, 3));
    }

    #[test]
    fn expansion_never_reaches_left_of_the_selectable_area() {
        let buf = buffer_with("漢字");
        // min_col pins the walk even when the glyph's lead cell sits outside.
        assert_eq!(expand_selection_columns(&buf, 0, 1, 1, 1), (1, 1));
    }

    #[test]
    fn a_reset_cell_is_not_mistaken_for_a_wide_glyph() {
        // Regression guard for the original bug: reset cells read back as a blank
        // of width 1, so a blank run must not be treated as a wide glyph.
        let buf = Buffer::empty(Rect::new(0, 0, 6, 1));
        assert_eq!(expand_selection_columns(&buf, 0, 1, 4, 0), (1, 4));
    }
}
