// prompt_input/vim_ops.rs — Vim undo, marks, macros, and search methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;

impl PromptInputState {
    /// Push the current (text, cursor) to the undo stack.
    pub fn push_undo(&mut self) {
        self.undo_stack.push((self.text.clone(), self.cursor));
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    // ---- Marks ----

    /// Set mark `name` at the current cursor position.
    pub fn set_mark(&mut self, name: char) {
        self.vim_marks
            .insert(name, (self.text.clone(), self.cursor));
    }

    /// Move cursor to the position recorded for mark `name`, if the text still matches.
    pub fn jump_to_mark(&mut self, name: char) {
        if let Some((_saved_text, saved_cursor)) = self.vim_marks.get(&name).cloned() {
            // Clamp to current text length in case text changed.
            let target = saved_cursor.min(self.text.len());
            // Ensure we land on a char boundary.
            let mut pos = target;
            while pos > 0 && !self.text.is_char_boundary(pos) {
                pos -= 1;
            }
            self.cursor = pos;
        }
    }

    // ---- Macro recording ----

    /// Begin recording a macro into register `register`.
    /// If already recording, stops the current recording first.
    pub fn start_macro_recording(&mut self, register: char) {
        self.vim_macro_recording = Some(register);
        self.vim_macro_content.insert(register, Vec::new());
    }

    /// Stop recording the current macro. Returns the register name that was being recorded.
    pub fn stop_macro_recording(&mut self) -> Option<char> {
        self.vim_macro_recording.take()
    }

    /// Return the recorded key sequence for `register`, or an empty vec.
    pub fn replay_macro(&self, register: char) -> Vec<String> {
        self.vim_macro_content
            .get(&register)
            .cloned()
            .unwrap_or_default()
    }

    // ---- Vim command-line execution ----

    /// Execute a `:` command-line command.
    /// Recognised: `q`/`quit`, `wq`, `set` (no-op), `noh` (clear search highlight).
    pub fn execute_vim_cmdline(&mut self, cmd: &str) {
        match cmd {
            "q" | "quit" | "wq" | "x" => {
                // In prompt context we can only signal quit by clearing + a special flag.
                // We set a dedicated field that the app loop can inspect.
                self.vim_quit_requested = true;
            }
            "noh" | "nohlsearch" => {
                self.vim_search_last = None;
            }
            s if s.starts_with("set ") => {
                // `:set vim` → enable, `:set novim` → disable (runtime toggle)
                let arg = s["set ".len()..].trim();
                match arg {
                    "vim" => {
                        self.vim_enabled = true;
                    }
                    "novim" => {
                        self.vim_enabled = false;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // ---- In-prompt search ----

    /// Move cursor to the next occurrence of `pattern` after `cursor + skip`.
    /// `skip = 0` finds from current position; `skip = 1` finds the *next* one.
    pub fn vim_search_forward(&mut self, pattern: &str, skip: usize) {
        if pattern.is_empty() {
            return;
        }
        let start = if skip > 0 {
            // Start after the current character to avoid re-matching same position

            self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(b, _)| self.cursor + b)
                .unwrap_or(0)
        } else {
            self.cursor
        };
        // Search from `start` forward, then wrap around
        let text_lc = self.text.to_lowercase();
        let pat_lc = pattern.to_lowercase();
        if let Some(pos) = text_lc[start..].find(&pat_lc) {
            self.cursor = start + pos;
            return;
        }
        // Wrap: search from beginning
        if let Some(pos) = text_lc.find(&pat_lc) {
            self.cursor = pos;
        }
    }

    /// Move cursor to the previous occurrence of `pattern` before current cursor.
    pub fn vim_search_backward(&mut self, pattern: &str) {
        if pattern.is_empty() {
            return;
        }
        let text_lc = self.text.to_lowercase();
        let pat_lc = pattern.to_lowercase();
        // Find all occurrences, pick the last one before cursor
        let before = &text_lc[..self.cursor];
        if let Some(pos) = before.rfind(&pat_lc) {
            self.cursor = pos;
            return;
        }
        // Wrap: find last occurrence in whole text
        if let Some(pos) = text_lc.rfind(&pat_lc) {
            self.cursor = pos;
        }
    }
}
