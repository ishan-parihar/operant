// prompt_input/editing.rs — Character/word editing and cursor movement methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::vim::char_idx_to_byte;
use super::word_nav::{stop_backward, stop_forward};
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

    /// Ctrl+E / End: move to the end of the current line.
    ///
    /// End of LINE, not end of text — the prompt is multi-line capable
    /// (Shift+Enter, and vim's `o`/`O` insert newlines), and readline's Ctrl+E
    /// is line-scoped, so collapsing the whole buffer onto one line would make
    /// a multi-line draft unnavigable.
    pub fn move_to_end(&mut self) {
        let line_end = self.text[self.cursor..]
            .find('\n')
            .map(|p| self.cursor + p)
            .unwrap_or(self.text.len());
        self.cursor = line_end;
    }

    /// Home: move to the start of the current line (plan 2026-10-24 M1 —
    /// OMP line-scoped Home; it used to jump to the start of the whole
    /// buffer, which on a multi-line draft abandons the line being edited).
    /// Ctrl+A stays the model picker, so this has no chord of its own —
    /// the Home key dispatches it directly.
    pub fn move_to_start(&mut self) {
        let line_start = self.text[..self.cursor]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        self.cursor = line_start;
    }

    /// Ctrl+W: Cut previous word and save to kill ring.
    ///
    /// Word boundaries are the OMP classes (`word_nav`): punctuation runs
    /// are one kill, and joiners stay inside words (`don't` kills whole).
    pub fn kill_word_backward(&mut self) {
        if self.mode == InputMode::Readonly || self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let idx = stop_backward(&chars, chars.len());
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
        let idx = stop_backward(&chars, chars.len());
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
        let idx = stop_forward(&chars, 0);
        let delete_end = self.cursor + char_idx_to_byte(rest, idx);
        if delete_end > self.cursor {
            self.text.drain(self.cursor..delete_end);
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+B / Ctrl+Left: Jump to previous word.
    pub fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let chars: Vec<char> = before.chars().collect();
        let idx = stop_backward(&chars, chars.len());
        self.cursor = char_idx_to_byte(before, idx);
    }

    /// Alt+F / Ctrl+Right: Jump to next word start (readline-style: the
    /// word run AND the whitespace after it are consumed, so the cursor
    /// lands on the first character of the next word).
    pub fn move_word_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let rest = &self.text[self.cursor..];
        let chars: Vec<char> = rest.chars().collect();
        let mut idx = stop_forward(&chars, 0);
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
        let idx = stop_forward(&chars, 0);
        let delete_end = self.cursor + char_idx_to_byte(rest, idx);
        if delete_end > self.cursor {
            self.text.drain(self.cursor..delete_end);
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }
}
