// prompt_input/history.rs — History navigation, paste, and kill-ring/yank methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;

impl PromptInputState {
    /// Navigate history up (older).
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_pos {
            None => {
                self.history_draft = self.text.clone();
                self.history_pos = Some(self.history.len() - 1);
            }
            Some(0) => {}
            Some(n) => {
                self.history_pos = Some(n - 1);
            }
        }
        if let Some(pos) = self.history_pos {
            self.text = self.history[pos].clone();
            self.cursor = self.text.len();
            self.update_token_estimate();
        }
    }

    /// Navigate history down (newer).
    pub fn history_down(&mut self) {
        match self.history_pos {
            None => {}
            Some(n) if n + 1 >= self.history.len() => {
                self.history_pos = None;
                self.text = self.history_draft.clone();
                self.cursor = self.text.len();
                self.update_token_estimate();
            }
            Some(n) => {
                self.history_pos = Some(n + 1);
                self.text = self.history[n + 1].clone();
                self.cursor = self.text.len();
                self.update_token_estimate();
            }
        }
    }

    /// Handle a paste event.
    pub fn paste(&mut self, content: &str) {
        let (text, stored) = handle_paste(content, &mut self.paste_counter);
        if let Some(stored_content) = stored {
            self.paste_contents
                .insert(self.paste_counter, stored_content);
        }
        for c in text.chars() {
            self.text.insert(self.cursor, c);
            self.cursor += c.len_utf8();
        }
        self.update_token_estimate();
        self.kill_ring.mark_non_kill();
    }

    /// Ctrl+Y: Paste from kill ring (most recent).
    pub fn yank(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        if let Some(text) = self.kill_ring.get_current() {
            for c in text.chars() {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            self.update_token_estimate();
            self.kill_ring.mark_non_kill();
        }
    }

    /// Alt+Y: Cycle through kill ring backward.
    pub fn yank_pop(&mut self) {
        if self.mode == InputMode::Readonly {
            return;
        }
        self.kill_ring.cycle_backward();
    }

    // ---- Named registers ----

    /// Store `text` in the named register `register`.
    pub fn yank_to_register(&mut self, register: char, text: &str) {
        self.vim_registers.insert(register, text.to_string());
    }

    /// Retrieve text from the named register `register`, if any.
    pub fn paste_from_register(&mut self, register: char) -> Option<String> {
        self.vim_registers.get(&register).cloned()
    }
}
