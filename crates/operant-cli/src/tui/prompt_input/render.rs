// prompt_input/render.rs — Input rendering (input_height, wrap_line,
// render_prompt_input).
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn input_height(state: &PromptInputState, text_width: u16) -> u16 {
    let visual_lines = if state.text.is_empty() {
        1usize
    } else if text_width == 0 {
        state.text.lines().count().max(1)
    } else {
        let mut total = 0usize;
        let logical: Vec<&str> = state.text.split('\n').collect();
        for line in &logical {
            let chunks = wrap_line(line, text_width as usize).len().max(1);
            total += chunks;
        }
        total.max(1)
    };
    // top-line + text rows + breathing room + underline, capped so the prompt
    // never eats more than ~half the screen.
    const MAX_TEXT_ROWS: usize = 10;
    let text_rows = visual_lines.min(MAX_TEXT_ROWS) as u16;
    let base = (text_rows + 3).max(4);
    base + if state.pending_images.is_empty() {
        0
    } else {
        1
    }
}

/// Wrap a logical line into visual chunks of `width` terminal cells. Empty
/// input yields a single empty chunk so the caller can still place a cursor.
pub fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    if line.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in line.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if current_width > 0 && current_width + ch_width > width {
            out.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        out.push(current);
    }

    out
}

/// Render the prompt input widget in the same low-chrome style as Operant:
/// multi-line input rows (one per logical line in the text) plus an accent
/// underline. Suggestions are rendered by the footer, not as a boxed dropdown
/// here.

pub fn render_prompt_input(
    state: &PromptInputState,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
    mode: InputMode,
    accent_override: Color,
    cursor_blink_enabled: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // If images are pending, render a pill row above everything else and shrink area.
    let (area, image_row_y) = if !state.pending_images.is_empty() && area.height > 1 {
        let pill_y = area.y;
        let rest = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };
        (rest, Some(pill_y))
    } else {
        (area, None)
    };

    if let Some(pill_y) = image_row_y {
        let mut pills: Vec<Span<'static>> = Vec::new();
        for img in &state.pending_images {
            let label = if let Some((w, h)) = img.dimensions {
                format!(" \u{f03e} {} {}x{} ", img.label, w, h) // nerd-font image icon, fallback to plain text
            } else {
                format!(" \u{f03e} {} ", img.label)
            };
            pills.push(Span::styled(
                label,
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ));
            pills.push(Span::raw(" "));
        }
        if !pills.is_empty() {
            Paragraph::new(Line::from(pills)).render(
                Rect {
                    x: area.x,
                    y: pill_y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }

    let accent = match mode {
        InputMode::Readonly => ACCENT_PRIMARY, // locked while streaming — always pink
        _ => accent_override,                  // use mode-aware accent color
    };
    let prompt_prefix = format!("{PROMPT_POINTER} ");
    let prefix_width = UnicodeWidthStr::width(prompt_prefix.as_str()) as u16;
    // Reserve a 2-cell right margin so wrapped text doesn't kiss the right edge
    // of the box (issue #149: padding too tight).
    let right_pad: u16 = 2;
    let available_width = area
        .width
        .saturating_sub(prefix_width)
        .saturating_sub(right_pad) as usize;
    let cursor_visible = if cursor_blink_enabled {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        (ms / 530).is_multiple_of(2)
    } else {
        true
    };
    // Render cursor as an overlay so its blink state never shifts the
    // underlying text (issue #149: cursor blink shifted the prompt).
    let show_cursor = focused && cursor_visible;

    // Use the raw text — no inline cursor character — so layout is stable.
    let display_text: String = if state.text.is_empty() {
        if focused {
            String::new()
        } else if mode == InputMode::Default {
            "How can I help you?".to_string()
        } else {
            String::new()
        }
    } else {
        state.text.clone()
    };

    // Top separator line (matches bottom underline — visual "box" around the prompt).
    if area.height > 0 {
        Paragraph::new(Line::from(vec![Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(accent),
        )]))
        .render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    // Text rows start 1 row below the top separator.
    let text_start_y = area.y + 1;

    // Split into logical lines; guarantee at least one.
    let logical_lines: Vec<String> = {
        let collected: Vec<String> = display_text.lines().map(|l| l.to_string()).collect();
        if display_text.ends_with('\n') || collected.is_empty() {
            let mut v = collected;
            v.push(String::new());
            v
        } else {
            collected
        }
    };

    let text_style = if state.text.is_empty() && !focused {
        Style::default().fg(Color::DarkGray).bg(Color::Black)
    } else {
        Style::default().fg(Color::White).bg(Color::Black)
    };

    // Wrap each logical line into visual rows that fit `available_width`,
    // and remember the (logical_idx, intra_line_display_col) for each row
    // so we can later compute where the cursor lives.
    let mut visual_rows: Vec<(usize, usize, String)> = Vec::new();
    for (li, line_text) in logical_lines.iter().enumerate() {
        let chunks = wrap_line(line_text, available_width.max(1));
        let mut col_offset = 0usize;
        for chunk in chunks {
            let chunk_len = UnicodeWidthStr::width(chunk.as_str());
            visual_rows.push((li, col_offset, chunk));
            col_offset += chunk_len;
        }
    }

    // Compute cursor's visual (row, col) within `visual_rows`.
    // We map state.cursor (a byte offset into state.text) to
    // (logical_line, display column).
    let cursor_pos: Option<(usize, usize)> = if focused && !state.text.is_empty() {
        let mut byte_idx = 0usize;
        let mut found: Option<(usize, usize)> = None;
        'outer: for (li, line_text) in logical_lines.iter().enumerate() {
            let line_bytes = line_text.len();
            // The +1 accounts for the '\n' between logical lines (last line has no trailing \n).
            let line_end_byte = byte_idx + line_bytes;
            if state.cursor <= line_end_byte {
                let intra_byte = state.cursor - byte_idx;
                let display_col = UnicodeWidthStr::width(&line_text[..intra_byte.min(line_bytes)]);
                found = Some((li, display_col));
                break 'outer;
            }
            byte_idx = line_end_byte + 1; // newline
        }
        // Fallback: cursor at end of text.
        found.or_else(|| {
            let li = logical_lines.len().saturating_sub(1);
            let col = logical_lines
                .get(li)
                .map(|s| UnicodeWidthStr::width(s.as_str()))
                .unwrap_or(0);
            Some((li, col))
        })
    } else if focused && state.text.is_empty() {
        Some((0, 0))
    } else {
        None
    };

    let cursor_visual: Option<(usize, usize)> = cursor_pos.and_then(|(li, col)| {
        // Find the visual row whose logical_idx == li and contains `col`.
        let mut last_match: Option<(usize, usize)> = None;
        for (vi, (row_li, row_col_start, chunk)) in visual_rows.iter().enumerate() {
            if *row_li != li {
                continue;
            }
            let chunk_len = UnicodeWidthStr::width(chunk.as_str());
            let row_col_end = row_col_start + chunk_len;
            if col >= *row_col_start && col <= row_col_end {
                last_match = Some((vi, col - row_col_start));
            }
        }
        last_match
    });

    // Render each visual row (truncated to area height).
    // Ensure at least 1 text row so the input is always visible.
    // (iter-120 — user-reported bug: input text was not showing up,
    // appearing as a "black bar". Root cause: max_text_rows was 0 when
    // area.height <= 2, so no text rows were rendered.)
    let max_text_rows = ((area.height as usize).saturating_sub(2)).max(1);
    // Scroll so the cursor row is visible.
    let scroll_offset = match cursor_visual {
        Some((vi, _)) if visual_rows.len() > max_text_rows && vi >= max_text_rows => {
            vi + 1 - max_text_rows
        }
        _ => 0,
    };

    for (display_idx, (vi, (li, _col_start, chunk))) in visual_rows
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_text_rows)
        .map(|(idx, item)| (idx - scroll_offset, item))
        .enumerate()
        .map(|(d, (idx, item))| (d, (idx + scroll_offset, item)))
    {
        let _ = vi;
        let _ = li;
        let row_y = text_start_y + display_idx as u16;

        // Determine if this is the first visual row of the first logical line —
        // that's the only row that gets the prompt prefix; continuation rows
        // (whether from logical line breaks or wrapping) get whitespace.
        let is_first_row_of_first_logical = display_idx == 0 && scroll_offset == 0;

        let spans: Vec<Span<'static>> = if is_first_row_of_first_logical {
            vec![
                Span::styled(
                    prompt_prefix.clone(),
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(chunk.clone(), text_style),
            ]
        } else {
            vec![
                Span::raw(" ".repeat(prefix_width as usize)),
                Span::styled(chunk.clone(), text_style),
            ]
        };

        Paragraph::new(Line::from(spans)).render(
            Rect {
                x: area.x,
                y: row_y,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    // Overlay the cursor on top of the rendered text. Instead of replacing
    // the character with a solid block (which made the input look like a
    // "black bar" when the user typed), we use reverse video: swap the
    // foreground and background of the character at the cursor position.
    // This shows the character AND the cursor simultaneously.
    // (iter-121 — user-reported bug: input text was invisible because the
    // solid block cursor covered the typed character.)
    if show_cursor {
        if let Some((vi, col_in_row)) = cursor_visual {
            if vi >= scroll_offset {
                let display_idx = vi - scroll_offset;
                if display_idx < max_text_rows {
                    let row_y = text_start_y + display_idx as u16;
                    let x = area.x + prefix_width + col_in_row as u16;
                    if x < area.x + area.width && row_y < area.y + area.height {
                        let cell = &mut buf[(x, row_y)];
                        // Get the current character at this position
                        let current_symbol = cell.symbol().to_string();
                        if current_symbol.is_empty() || current_symbol == " " {
                            // Empty position — show a cursor bar
                            cell.set_symbol("▏");
                            cell.set_style(Style::default().fg(Color::White).bg(Color::Black));
                        } else {
                            // Non-empty position — reverse video
                            cell.set_style(
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            );
                        }
                    }
                }
            }
        }
    }

    // Vim command / search row (shown below text lines, before underline).
    let text_rows_rendered = visual_rows
        .len()
        .saturating_sub(scroll_offset)
        .min(max_text_rows);
    let cmd_line: Option<Line<'static>> = match state.vim_mode {
        VimMode::Command => {
            let buf_text = format!(":{}\u{2588}", state.vim_command_buf);
            Some(Line::from(vec![Span::styled(
                buf_text,
                Style::default().fg(Color::Cyan),
            )]))
        }
        VimMode::Search => {
            let buf_text = format!("/{}\u{2588}", state.vim_search_buf);
            Some(Line::from(vec![Span::styled(
                buf_text,
                Style::default().fg(Color::Yellow),
            )]))
        }
        _ => None,
    };

    let (cmdline_row, underline_row) = if let Some(ref _cl) = cmd_line {
        let cmd_y = text_start_y + text_rows_rendered as u16;
        let ul_y = cmd_y + 1;
        (Some(cmd_y), ul_y)
    } else {
        (None, text_start_y + text_rows_rendered as u16)
    };

    if let (Some(row), Some(cl)) = (cmdline_row, cmd_line) {
        if row < area.y + area.height {
            Paragraph::new(cl).render(
                Rect {
                    x: area.x,
                    y: row,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }

    if underline_row < area.y + area.height {
        Paragraph::new(Line::from(vec![Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(accent),
        )]))
        .render(
            Rect {
                x: area.x,
                y: underline_row,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    // Token estimate overlay on the first text row (top-right corner).
    // Format mirrors TS formatTokens: compact "1.3k" for ≥1000, raw number below that.
    if state.text.len() > 1000 && area.height > 1 {
        let n = state.token_estimate;
        let formatted = if n >= 1000 {
            let k = n as f64 / 1000.0;
            // One decimal place, suppress trailing ".0" (e.g. 2000 → "2k", 1300 → "1.3k")
            if (k * 10.0).round() % 10.0 == 0.0 {
                format!("~{}k", k as u64)
            } else {
                format!("~{:.1}k", k)
            }
        } else {
            format!("~{}", n)
        };
        let count_str = formatted;
        let x = area.x + area.width.saturating_sub(count_str.len() as u16);
        Paragraph::new(Line::from(vec![Span::styled(
            count_str,
            Style::default().fg(Color::DarkGray),
        )]))
        .render(
            Rect {
                x,
                y: text_start_y,
                width: area.width.saturating_sub(x.saturating_sub(area.x)),
                height: 1,
            },
            buf,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
