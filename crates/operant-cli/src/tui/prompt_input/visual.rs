// prompt_input/visual.rs — Visual cursor positioning and multi-line layout methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;
use unicode_width::UnicodeWidthStr;

impl PromptInputState {
    /// Map the current cursor (byte offset) to a (visual_row, visual_col) pair
    /// given the wrap width. `width` is the usable column count for text.
    pub fn cursor_visual_pos(&self, width: usize) -> (usize, usize) {
        if width == 0 {
            return (0, 0);
        }
        let mut byte = 0usize;
        let mut row = 0usize;
        for line in self.text.split('\n') {
            let line_end = byte + line.len();
            if self.cursor <= line_end {
                let intra_byte = self.cursor - byte;
                let intra_byte = intra_byte.min(line.len());
                // walk to char-boundary
                let mut b = intra_byte;
                while b > 0 && !line.is_char_boundary(b) {
                    b -= 1;
                }
                let display_col = UnicodeWidthStr::width(&line[..b]);
                let chunk_idx = if display_col == 0 {
                    0
                } else {
                    display_col / width
                };
                let chunk_col = display_col % width;
                return (row + chunk_idx, chunk_col);
            }
            let chunks = wrap_line(line, width).len().max(1);
            row += chunks;
            byte = line_end + 1; // newline
        }
        (row.saturating_sub(1), 0)
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
        if self.text.is_empty() || width == 0 {
            return 1;
        }
        let mut total = 0usize;
        for line in self.text.split('\n') {
            total += wrap_line(line, width).len().max(1);
        }
        total.max(1)
    }

    fn set_cursor_at_visual(&mut self, target_row: usize, target_col: usize, width: usize) {
        if width == 0 {
            return;
        }
        let mut byte = 0usize;
        let mut row = 0usize;
        for line in self.text.split('\n').collect::<Vec<_>>() {
            let chunks = wrap_line(line, width).len().max(1);
            if target_row < row + chunks {
                let intra_chunk = target_row - row;
                let chunk_char_start = intra_chunk * width;
                let line_chars: Vec<(usize, char)> = line.char_indices().collect();
                let chunk_chars_len = line_chars.len().saturating_sub(chunk_char_start).min(width);
                let col = target_col.min(chunk_chars_len);
                let target_char_idx = chunk_char_start + col;
                let intra_byte = line_chars
                    .get(target_char_idx)
                    .map(|(b, _)| *b)
                    .unwrap_or(line.len());
                self.cursor = byte + intra_byte;
                self.history_pos = None;
                return;
            }
            row += chunks;
            byte += line.len() + 1; // newline
        }
        self.cursor = self.text.len();
        self.history_pos = None;
    }
}
