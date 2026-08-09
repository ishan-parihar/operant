// prompt_input/suggestions.rs — Suggestion computation and navigation methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;

impl PromptInputState {
    /// Returns true if the text (up to cursor) contains a word-boundary `@` token,
    /// meaning an `@file` reference is actively being typed.
    pub fn has_active_file_ref(&self) -> bool {
        let text = &self.text[..self.cursor];
        text.rfind('@').is_some_and(|at_idx| {
            at_idx == 0
                || text[..at_idx]
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_whitespace())
        })
    }

    /// Update typeahead suggestions for slash commands and file references in the current text.
    pub fn update_suggestions(
        &mut self,
        slash_commands: &[(&str, &str)],
        file_autocomplete_limit: usize,
        file_autocomplete_show_hidden: bool,
    ) {
        // Only look at text up to the cursor — text after the cursor belongs to a
        // different editing position and would confuse rfind('@') / rfind('/').
        let text_before_cursor = &self.text[..self.cursor];
        self.suggestions = compute_typeahead(
            text_before_cursor,
            slash_commands,
            file_autocomplete_limit,
            file_autocomplete_show_hidden,
        );

        if self.suggestions.is_empty() {
            self.suggestion_index = None;
        } else {
            let idx = self
                .suggestion_index
                .unwrap_or(0)
                .min(self.suggestions.len() - 1);
            self.suggestion_index = Some(idx);
        }
    }

    /// Select the next suggestion.
    pub fn suggestion_next(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        self.suggestion_index = Some(
            self.suggestion_index
                .map_or(0, |i| (i + 1) % self.suggestions.len()),
        );
    }

    /// Select the previous suggestion.
    pub fn suggestion_prev(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        self.suggestion_index = Some(self.suggestion_index.map_or(0, |i| {
            if i == 0 {
                self.suggestions.len() - 1
            } else {
                i - 1
            }
        }));
    }

    /// Accept the current suggestion during an Enter (submit) keypress.
    ///
    /// Unlike `accept_suggestion`, this is the state machine for the submit path:
    /// it both fills the text with the chosen suggestion AND signals to the caller
    /// whether the next step is to keep editing (e.g. trailing space after a
    /// file-ref) or to submit the message/slash-command.
    ///
    /// Returns `NoSuggestion` when there is no suggestion to accept — callers
    /// should then fall back to the normal submit path (queue the message).
    pub fn accept_suggestion_for_submit(&mut self) -> AcceptForSubmitOutcome {
        if self.suggestions.is_empty() || self.suggestion_index.is_none() {
            return AcceptForSubmitOutcome::NoSuggestion;
        }
        let source = self
            .suggestions
            .get(self.suggestion_index.unwrap_or(0))
            .map(|s| s.source.clone());
        self.accept_suggestion();
        match source {
            Some(TypeaheadSource::FileRef) => {
                // File refs end with a trailing space so the user can keep typing
                // the instruction that follows the @file reference.
                self.insert_char(' ');
                AcceptForSubmitOutcome::ExtendInput
            }
            Some(TypeaheadSource::SlashCommand) | None => {
                // Slash commands replace the whole input — Enter submits the command.
                AcceptForSubmitOutcome::Submit
            }
        }
    }

    /// Accept the current suggestion.
    pub fn accept_suggestion(&mut self) {
        if let Some(idx) = self.suggestion_index {
            if let Some(s) = self.suggestions.get(idx) {
                let new_cursor = match s.source {
                    TypeaheadSource::SlashCommand => {
                        // Replace entire text; discard anything after cursor too.
                        self.text = s.text.clone();
                        self.text.len()
                    }
                    TypeaheadSource::FileRef => {
                        // Replace from the last word-boundary @ up to the cursor.
                        // Preserve any text that was already after the cursor.
                        let tail = self.text[self.cursor..].to_string();
                        if let Some(at_idx) = self.text[..self.cursor].rfind('@') {
                            let at_word_boundary = at_idx == 0
                                || self.text[..at_idx]
                                    .chars()
                                    .last()
                                    .map(|c| c.is_whitespace())
                                    .unwrap_or(false);
                            if at_word_boundary {
                                let mut new_text = self.text[..at_idx].to_string();
                                new_text.push_str(&s.text);
                                let cursor = new_text.len();
                                new_text.push_str(&tail);
                                self.text = new_text;
                                cursor
                            } else {
                                let mut new_text = s.text.clone();
                                let cursor = new_text.len();
                                new_text.push_str(&tail);
                                self.text = new_text;
                                cursor
                            }
                        } else {
                            let mut new_text = s.text.clone();
                            let cursor = new_text.len();
                            new_text.push_str(&tail);
                            self.text = new_text;
                            cursor
                        }
                    }
                };
                self.cursor = new_cursor;
                self.suggestions.clear();
                self.suggestion_index = None;
                self.update_token_estimate();
            }
        }
    }

    /// Replace the full text buffer and move the cursor to the end.
    pub fn replace_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.history_pos = None;
        self.suggestion_index = None;
        self.update_token_estimate();
    }
}
