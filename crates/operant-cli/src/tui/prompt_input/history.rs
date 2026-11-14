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
        // RT-560: browsing is owned by the RECALLED text, not by "we are
        // somewhere in the ladder". An edit after a recall (which does not
        // clear `history_pos`) used to keep the ladder walking and replace the
        // edit with the next entry; codex gates on `last_history_text` — the
        // ladder answers only while the composer still holds what it recalled.
        // An edited recall is a dead ladder, and a dead ladder's next call is a
        // FRESH start: the edit becomes the draft the operator gets back when
        // they walk back out of history.
        if !self.history_is_live() {
            self.end_history_browse();
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
            // The recalled entry replaces the text WHOLESALE and carries no
            // store of its own: an entry is plain text, and the entry we were
            // browsing may still have had a live paste store behind it. Drop
            // the store with the text. The orphan this prevents is not a
            // mis-delivery — the label numbering hands the next paste its own
            // token, so the payload is never swapped — it is the invariant:
            // the store is the only thing between `[Pasted ~N lines]` and the
            // model, and it must not describe a buffer it was never in.
            // (red-team 2, 2026-11-14; probed, see the audit §2.)
            self.pending_pastes.clear();
            self.text = self.history[pos].clone();
            // Going BACK in time, the caret belongs at the TOP-START: the
            // operator pressed Up to reach an older prompt, and an entry that
            // opens with the caret at its end reads as "this is the newest
            // line", which is the opposite of the gesture. It also makes the
            // next Up walk the entry's own first row instead of immediately
            // stepping further back (operator report 2026-11-14).
            self.cursor = 0;
            self.preferred_col = None;
            self.history_recall = Some(self.text.clone());
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
        // The same gate as the way up: an edited recall is a dead ladder, and
        // a dead ladder's Down is not a "restore the draft" move.
        if !self.history_is_live() {
            self.end_history_browse();
        }
        match self.history_pos {
            None => {}
            Some(n) if n + 1 >= self.history.len() => {
                self.history_pos = None;
                self.pending_pastes.clear();
                self.text = self.history_draft.clone();
                // Back to the draft: the caret belongs at the BOTTOM-END,
                // where the operator left it (operator report 2026-11-14).
                self.cursor = self.text.len();
                self.preferred_col = None;
                self.history_recall = None;
                self.update_token_estimate();
                self.suppress_suggestions_once = true;
                self.suggestions.clear();
                self.suggestion_index = None;
            }
            Some(n) => {
                self.history_pos = Some(n + 1);
                self.pending_pastes.clear();
                self.text = self.history[n + 1].clone();
                // Going FORWARD in time, the caret belongs at the BOTTOM-END:
                // Down reaches toward the present, and the tail is where the
                // operator's eye is after walking back up through an entry.
                self.cursor = self.text.len();
                self.preferred_col = None;
                self.history_recall = Some(self.text.clone());
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
    ///
    /// Large pastes collapse to a placeholder; pasting the SAME content again
    /// expands the placeholder it already holds (RT-562) instead of collapsing
    /// a second one.
    pub fn paste(&mut self, content: &str) {
        // Sanitise ONCE, here, before the content is matched, stored or
        // inserted, so the store holds exactly what the buffer holds. A payload
        // spliced back in by the expand then cannot reintroduce a byte the
        // paste path would have dropped (that was a real defect: the expand
        // wrote raw ESC bytes into the composer, reopening RT-539), and a
        // re-paste of the same clipboard content matches the store by the value
        // the buffer actually carries.
        let content = sanitize_pasted_text(content);
        if let Some(placeholder) = paste::expand_target(&content, &self.pending_pastes) {
            self.expand_paste_placeholder(&placeholder);
            return;
        }
        match paste::PasteOutcome::of(&content, &self.pending_pastes) {
            paste::PasteOutcome::Inline => self.insert_text(&content),
            paste::PasteOutcome::Collapsed {
                placeholder,
                content: stored,
            } => {
                // Counted before the move into the store: the notice has to
                // name the size of what the token swallowed.
                let lines = stored.lines().count();
                self.insert_text(&placeholder);
                self.pending_pastes.push((placeholder, stored));
                // The operator pasted something big and the composer now shows
                // a token instead of their text. Say so, and say how to get it
                // back: the collapse used to be silent, which is half of what
                // RT-562 asked for.
                self.paste_collapsed =
                    Some(format!("Pasted {lines} lines — paste it again to expand."));
            }
        }
    }

    /// Swap one placeholder back for its payload and drop the store entry.
    ///
    /// ONE placeholder in, ONE out: the payload is spliced in where the
    /// placeholder was, so a composer holding two pastes expands the one the
    /// operator re-pasted and keeps the other collapsed. The caret goes to the
    /// end of what was inserted, which is where a paste leaves it.
    ///
    /// The `suppress_paste_expand` flag is what keeps the refresh choke point
    /// from re-collapsing the payload this method just inserted.
    pub(super) fn expand_paste_placeholder(&mut self, placeholder: &str) {
        let Some(index) = self
            .pending_pastes
            .iter()
            .position(|(candidate, _)| candidate == placeholder)
        else {
            return;
        };
        let (_, content) = self.pending_pastes.remove(index);
        // The store holds the SANITISED payload (see `paste`), so this is a
        // guard rather than a second copy of the rule: if a future writer ever
        // puts raw text in the store, the buffer still gets clean text.
        let content = sanitize_pasted_text(&content);
        let lines = content.lines().count();
        let Some(at) = self.text.find(placeholder) else {
            // The store says the placeholder is live and the text disagrees:
            // put the payload back and let the refresh drop it if it cannot
            // land. Nothing is silently swallowed either way.
            self.pending_pastes
                .insert(index, (placeholder.to_string(), content));
            return;
        };
        self.text
            .replace_range(at..at + placeholder.len(), &content);
        self.cursor = at + content.len();
        self.preferred_col = None;
        self.history_pos = None;
        self.suppress_paste_expand = true;
        self.paste_expanded = Some(format!("Pasted {lines} lines expanded."));
        self.update_token_estimate();
    }

    /// Insert text at the caret and leave the caret at the end of what went in.
    ///
    /// Every paste path goes through here — the token, the expanded payload and
    /// a small paste's verbatim text — and the caller has already sanitised, so
    /// this does not sanitise again. A paste is a non-vertical edit, so it ends
    /// any saved-column run (RT-561's rule; the paste path used to skip it).
    fn insert_text(&mut self, text: &str) {
        self.preferred_col = None;
        for c in text.chars() {
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

/// What a paste is allowed to put in the composer: printable text, newlines,
/// and tabs widened to four spaces. See `paste` for why the rest is dropped.
pub(super) fn sanitize_pasted_text(text: &str) -> String {
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
