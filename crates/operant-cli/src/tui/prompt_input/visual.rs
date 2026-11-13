// prompt_input/visual.rs — Visual cursor positioning and multi-line layout methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;
use unicode_width::UnicodeWidthChar as _;

/// One visual row of the wrapped input: the text it holds and the character
/// range of the whole input it covers.
///
/// This is the ONE wrap model. The painter lays its rows out with it
/// (`operant_ui::input_ui`) and the caret's visual position is resolved with
/// it, so the caret can never land on a row the painter did not draw.
///
/// The two used to disagree: the caret model assumed every row was exactly
/// `width` cells wide (`display_col / width`), which a CJK row never is, and
/// it put a caret sitting at a row's end on the NEXT row. The painter's map
/// was right; the caret's was the approximation (2026-11-12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRow {
    /// The row's text.
    pub text: String,
    /// Character index of the row's first character in the whole input.
    pub start_char: usize,
    /// Character index just past the row's last character.
    pub end_char: usize,
    /// The row's display width in terminal cells.
    pub display_width: usize,
}

/// Wrap `input` into the visual rows the composer paints: logical lines
/// separated by `\n`, each soft-wrapped at `line_width` terminal cells.
///
/// A character wider than the whole row gets a row of its own, so no row is
/// ever empty except the empty lines between two newlines.
pub fn wrap_input_rows(input: &str, line_width: usize) -> Vec<VisualRow> {
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty() {
        return vec![VisualRow {
            text: String::new(),
            start_char: 0,
            end_char: 0,
            display_width: 0,
        }];
    }
    let mut rows: Vec<VisualRow> = Vec::new();
    let mut pos = 0usize;
    let mut char_count = 0usize;
    while pos <= chars.len() {
        let newline_pos = chars[pos..].iter().position(|&c| c == '\n');
        let segment_end = match newline_pos {
            Some(rel_pos) => pos + rel_pos,
            None => chars.len(),
        };
        let segment = &chars[pos..segment_end];
        let mut seg_pos = 0usize;
        loop {
            let mut display_width = 0usize;
            let mut end = seg_pos;
            while end < segment.len() {
                let cw = segment[end].width().unwrap_or(0);
                if display_width + cw > line_width {
                    break;
                }
                display_width += cw;
                end += 1;
            }
            if end == seg_pos && seg_pos < segment.len() {
                end = seg_pos + 1;
                display_width = segment[seg_pos].width().unwrap_or(0);
            }
            let text: String = segment[seg_pos..end].iter().collect();
            let start_char = char_count;
            let end_char = char_count + (end - seg_pos);
            rows.push(VisualRow {
                text,
                start_char,
                end_char,
                display_width,
            });
            char_count = end_char;
            if end >= segment.len() {
                break;
            }
            seg_pos = end;
        }
        if newline_pos.is_some() {
            char_count += 1;
            pos = segment_end + 1;
        } else {
            break;
        }
    }
    rows
}

impl PromptInputState {
    /// Map the current cursor (byte offset) to a (visual_row, visual_col) pair
    /// given the wrap width. `width` is the usable column count for text.
    ///
    /// The caret at the END of a row belongs to that row — the painter draws
    /// it there, and a caret that resolved to the next row would make Up/Down
    /// walk a row the operator cannot see.
    pub fn cursor_visual_pos(&self, width: usize) -> (usize, usize) {
        if width == 0 {
            return (0, 0);
        }
        let rows = wrap_input_rows(&self.text, width);
        let caret_char = self.text[..self.cursor].chars().count();
        for (index, row) in rows.iter().enumerate() {
            if caret_char >= row.start_char && caret_char <= row.end_char {
                let offset = caret_char - row.start_char;
                let col = row
                    .text
                    .chars()
                    .take(offset)
                    .map(|c| c.width().unwrap_or(0))
                    .sum::<usize>();
                return (index, col);
            }
        }
        (rows.len().saturating_sub(1), 0)
    }

    /// Move the cursor to the same visual column on the row above. Returns
    /// `true` if the cursor actually moved (i.e. there was a row above).
    pub fn move_visual_up(&mut self, width: usize) -> bool {
        if width == 0 {
            return false;
        }
        let (row, col) = self.cursor_visual_pos(width);
        if row == 0 {
            return false;
        }
        self.set_cursor_at_visual(row - 1, col, width);
        true
    }

    /// Move the cursor to the same visual column on the row below. Returns
    /// `true` if the cursor actually moved (i.e. there was a row below).
    pub fn move_visual_down(&mut self, width: usize) -> bool {
        if width == 0 {
            return false;
        }
        let (row, col) = self.cursor_visual_pos(width);
        let total_rows = self.visual_row_count(width);
        if row + 1 >= total_rows {
            return false;
        }
        self.set_cursor_at_visual(row + 1, col, width);
        true
    }

    fn visual_row_count(&self, width: usize) -> usize {
        if width == 0 {
            return 1;
        }
        wrap_input_rows(&self.text, width).len().max(1)
    }

    /// Put the caret on `target_row` at `target_col` DISPLAY columns.
    ///
    /// The column is a display column, not a character index: a CJK character
    /// is two cells wide, so the old "every row is `width` characters" model
    /// put the caret in the wrong place on every row that held one.
    fn set_cursor_at_visual(&mut self, target_row: usize, target_col: usize, width: usize) {
        if width == 0 {
            return;
        }
        let rows = wrap_input_rows(&self.text, width);
        let Some(row) = rows.get(target_row) else {
            self.cursor = self.text.len();
            self.history_pos = None;
            return;
        };
        let mut col = 0usize;
        let mut offset = 0usize;
        for c in row.text.chars() {
            if col >= target_col {
                break;
            }
            col += c.width().unwrap_or(0);
            offset += 1;
        }
        let byte = self
            .text
            .char_indices()
            .nth(row.start_char + offset)
            .map(|(b, _)| b)
            .unwrap_or(self.text.len());
        self.cursor = byte;
        self.history_pos = None;
    }
}
