// prompt_input/editing.rs — Character/word editing and cursor movement methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::vim::{char_idx_to_byte, is_word_char};
use super::*;

impl PromptInputState {
    /// Insert a character at cursor position.
    pub fn insert_char(&mut self, c: char) {
        if self.mode == InputMode::Readonly {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.update_token_estimate();
    }

    /// Insert a newline (Shift+Enter).
    pub fn insert_newline(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        self.insert_char('\n');
    }

    /// Delete the character before cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 || self.mode == InputMode::Readonly {
            return;
        }
        let prev = self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.remove(prev);
        self.cursor = prev;
        self.update_token_estimate();
    }

    /// Delete the character at cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() || self.mode == InputMode::Readonly {
            return;
        }
        self.text.remove(self.cursor);
        self.update_token_estimate();
    }

    /// Move cursor left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor = prev;
        }
    }

    /// Move cursor right.
    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.text.len());
            self.cursor = next;
        }
    }

    /// Ctrl+U: Cut from line start to cursor and save to kill ring.
    pub fn kill_line_backward(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        let line_start = self.text[..self.cursor]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);

        if self.cursor > line_start {
            let killed = self.text.drain(line_start..self.cursor).collect::<String>();
            self.kill_ring.push(killed);
            self.cursor = line_start;
            self.update_token_estimate();
        }
    }

    /// Ctrl+W: Cut previous word and save to kill ring.
    pub fn kill_word_backward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let mut idx = chars.len();
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 {
            return;
        }
        if is_word_char(chars[idx - 1]) {
            while idx > 0 && is_word_char(chars[idx - 1]) {
                idx -= 1;
            }
        } else {
            while idx > 0 && !is_word_char(chars[idx - 1]) && !chars[idx - 1].is_whitespace() {
                idx -= 1;
            }
        }
        let kill_start = char_idx_to_byte(before, idx);
        if kill_start < self.cursor {
            let killed = self.text.drain(kill_start..self.cursor).collect::<String>();
            self.kill_ring.push(killed);
            self.cursor = kill_start;
            self.update_token_estimate();
        }
    }

    /// Alt+Backspace: Delete word backward.
    pub fn delete_word_backward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let mut idx = chars.len();
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 {
            return;
        }
        if is_word_char(chars[idx - 1]) {
            while idx > 0 && is_word_char(chars[idx - 1]) {
                idx -= 1;
            }
        } else {
            while idx > 0 && !is_word_char(chars[idx - 1]) && !chars[idx - 1].is_whitespace() {
                idx -= 1;
            }
        }
        let delete_start = char_idx_to_byte(before, idx);
        if delete_start < self.cursor {
            self.text.drain(delete_start..self.cursor);
            self.cursor = delete_start;
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+Delete: Delete word forward.
    pub fn delete_word_forward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = 0;
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() {
            return;
        }
        if is_word_char(chars[idx]) {
            while idx < chars.len() && is_word_char(chars[idx]) {
                idx += 1;
            }
        } else {
            while idx < chars.len() && !is_word_char(chars[idx]) && !chars[idx].is_whitespace() {
                idx += 1;
            }
        }
        let delete_end = self.cursor + char_idx_to_byte(rest, idx);
        if delete_end > self.cursor {
            self.text.drain(self.cursor..delete_end);
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+B: Jump to previous word.
    pub fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let mut idx = chars.len();
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 {
            return;
        }
        if is_word_char(chars[idx - 1]) {
            while idx > 0 && is_word_char(chars[idx - 1]) {
                idx -= 1;
            }
        } else {
            while idx > 0 && !is_word_char(chars[idx - 1]) && !chars[idx - 1].is_whitespace() {
                idx -= 1;
            }
        }
        self.cursor = char_idx_to_byte(before, idx);
    }

    /// Alt+F: Jump to next word.
    pub fn move_word_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = 0;
        if idx < chars.len() {
            if is_word_char(chars[idx]) {
                while idx < chars.len() && is_word_char(chars[idx]) {
                    idx += 1;
                }
            } else if !chars[idx].is_whitespace() {
                while idx < chars.len() && !is_word_char(chars[idx]) && !chars[idx].is_whitespace()
                {
                    idx += 1;
                }
            }
        }
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        self.cursor += char_idx_to_byte(rest, idx);
    }

    /// Alt+D: Delete word after cursor.
    pub fn delete_word_at_cursor(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = 0;
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() {
            return;
        }
        if is_word_char(chars[idx]) {
            while idx < chars.len() && is_word_char(chars[idx]) {
                idx += 1;
            }
        } else {
            while idx < chars.len() && !is_word_char(chars[idx]) && !chars[idx].is_whitespace() {
                idx += 1;
            }
        }
        let delete_end = self.cursor + char_idx_to_byte(rest, idx);
        if delete_end > self.cursor {
            self.text.drain(self.cursor..delete_end);
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }
}
