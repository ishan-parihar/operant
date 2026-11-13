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
            // (2026-10-25) A recalled `/…` entry must not re-open the
            // typeahead popup — that is what stranded history browsing in
            // slash-completions (the next Up/Down fed the popup instead of
            // stepping history). OMP opens autocomplete only on a TYPED
            // trigger character; recall is programmatic.
            self.suppress_suggestions_once = true;
            self.suggestions.clear();
            self.suggestion_index = None;
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
                self.suppress_suggestions_once = true;
                self.suggestions.clear();
                self.suggestion_index = None;
            }
            Some(n) => {
                self.history_pos = Some(n + 1);
                self.text = self.history[n + 1].clone();
                self.cursor = self.text.len();
                self.update_token_estimate();
                self.suppress_suggestions_once = true;
                self.suggestions.clear();
                self.suggestion_index = None;
            }
        }
    }

    /// Handle a paste event.
    ///
    /// The pasted text is sanitized before it reaches the buffer. Clipboard
    /// content copied out of a terminal carries that terminal's colour and
    /// cursor escapes, and a cell holding `ESC` is not a glyph — the terminal
    /// executes it, which smears the frame and can move the cursor anywhere
    /// (measured 2026-11-13: `ESC[200~` from a paste painted as raw cells).
    /// A newline is the one control a prompt wants; a tab becomes spaces so
    /// pasted indentation survives; every other control character is dropped.
    pub fn paste(&mut self, content: &str) {
        let (text, stored) = handle_paste(content, &mut self.paste_counter);
        if let Some(stored_content) = stored {
            self.paste_contents
                .insert(self.paste_counter, stored_content);
        }
        for c in sanitize_pasted_text(&text).chars() {
            self.text.insert(self.cursor, c);
            self.cursor += c.len_utf8();
        }
        self.update_token_estimate();
        self.kill_ring.mark_non_kill();
    }

    /// Ctrl+Y: Paste from kill ring (most recent).
    /// Paste the kill ring's current entry at the cursor.
    ///
    /// Returns whether anything was pasted. The caller owns the "why not", per
    /// RT-472's rule that an affordance either works or says why not: this used
    /// to return nothing at all, so `Ctrl+Y` with an empty kill ring moved the
    /// operator no closer to a paste and told them nothing. The dialog paste
    /// path already does this ("Clipboard is empty"), which is the precedent.
    pub fn yank(&mut self) -> bool {
        if self.mode == InputMode::Readonly {
            return false;
        }
        let Some(text) = self.kill_ring.get_current() else {
            return false;
        };
        for c in text.chars() {
            self.text.insert(self.cursor, c);
            self.cursor += c.len_utf8();
        }
        self.update_token_estimate();
        self.kill_ring.mark_non_kill();
        true
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_drops_terminal_escapes_but_keeps_newlines_and_indentation() {
        let mut state = PromptInputState::new();
        // Copied out of a coloured terminal: SGR colour around RED.
        state.paste("a\x1b[31mRED\x1b[0m\n\tb");
        assert_eq!(
            state.text, "aRED\n    b",
            "the escapes must not become cells; the newline and the tab's indentation must survive"
        );
    }

    #[test]
    fn a_paste_keeps_ordinary_text_and_two_lines_inline() {
        let mut state = PromptInputState::new();
        state.paste("line one\nline two");
        assert_eq!(state.text, "line one\nline two");
    }
}

/// What a paste is allowed to put in the composer: printable text, newlines,
/// and tabs widened to four spaces. See `paste` for why the rest is dropped.
fn sanitize_pasted_text(text: &str) -> String {
    // Whole sequences first, so `ESC[31m` leaves no `[31m` behind as visible
    // text — the renderer already uses this scanner for captured output.
    let text = crate::tui::operant_app::message::strip_ansi_escape_sequences(text);
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
