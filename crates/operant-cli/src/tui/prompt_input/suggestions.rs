// prompt_input/suggestions.rs — Suggestion computation and navigation methods.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;

impl PromptInputState {
    /// Returns true if the text (up to cursor) contains a word-boundary `@`
    /// token — bare `@`, `@file:`, or `@folder:` (M4) — meaning a file
    /// reference is actively being typed.
    pub fn has_active_file_ref(&self) -> bool {
        parse_file_ref_token(&self.text[..self.cursor]).is_some()
    }

    /// A live `/command` token under the caret — the gate for the popup's
    /// Up/Down navigation. It used to be `text.starts_with('/')`, which
    /// missed a command typed after prose (2026-11-12).
    pub fn has_slash_command_token(&self) -> bool {
        super::typeahead::slash_token_range(&self.text[..self.cursor]).is_some()
    }

    /// Update typeahead suggestions for slash commands and file references in the current text.
    pub fn update_suggestions(
        &mut self,
        slash_commands: &[(&str, &str)],
        usage: &crate::tui::slash_usage::UsageStore,
        file_autocomplete_limit: usize,
        file_autocomplete_show_hidden: bool,
    ) {
        // (2026-10-25 history-nav fix) A history recall just replaced the
        // text; OMP's editor only auto-triggers autocomplete on a TYPED
        // trigger character, and a programmatic recall must not open the
        // popup — a recalled `/help` stranding the next Up/Down in
        // slash-completions is the exact defect this one-shot prevents.
        // Consume the flag and leave the popup shut; the next genuine
        // refresh (a typed character) recomputes normally.
        if self.suppress_suggestions_once {
            self.suppress_suggestions_once = false;
            self.suggestions.clear();
            self.suggestion_index = None;
            self.pending_file_query = None;
            return;
        }
        // Only look at text up to the cursor — text after the cursor belongs to a
        // different editing position and would confuse rfind('@') / rfind('/').
        let text_before_cursor = &self.text[..self.cursor];
        self.suggestions = compute_typeahead(
            text_before_cursor,
            slash_commands,
            usage,
            file_autocomplete_limit,
            file_autocomplete_show_hidden,
        );

        // Fuzzy tier request (plan 2026-10-24 M3): the synchronous pass
        // above is the IMMEDIATE listing; while an `@` token with a rankable
        // tail is live, also describe the recursive walk for the App to
        // spawn. Pure data — this layer spawns nothing.
        self.pending_file_query = active_file_query(text_before_cursor);

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

    /// Take the fuzzy-tier request (M3): the App calls this right after
    /// `update_suggestions` and spawns the walk it describes.
    pub fn take_pending_file_query(&mut self) -> Option<PendingFileQuery> {
        self.pending_file_query.take()
    }

    /// Replace the interim FileRef listing with the async fuzzy tier
    /// (M3). An empty result keeps the interim listing — a walk that found
    /// nothing new must not blank the popup the user is navigating.
    pub fn replace_file_suggestions(&mut self, suggestions: Vec<TypeaheadSuggestion>) {
        if suggestions.is_empty() {
            return;
        }
        self.suggestions = suggestions;
        let idx = self
            .suggestion_index
            .unwrap_or(0)
            .min(self.suggestions.len() - 1);
        self.suggestion_index = Some(idx);
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
                // `accept_suggestion` already appended the trailing space
                // for a FILE value (M4); a directory value chains instead.
                // Either way Enter must keep editing, not submit.
                AcceptForSubmitOutcome::ExtendInput
            }
            Some(TypeaheadSource::SkillRef) => {
                // A mention lives mid-sentence: Tab or Enter drops `$name`
                // into the buffer at the cursor and the instruction follows.
                AcceptForSubmitOutcome::ExtendInput
            }
            Some(TypeaheadSource::SlashCommand) | None => {
                // Slash commands replace the whole input — Enter submits the command.
                AcceptForSubmitOutcome::Submit
            }
        }
    }

    /// Accept the current suggestion.
    ///
    /// FileRef values (M4): files append a trailing space (the instruction
    /// that follows the reference starts immediately); directories end
    /// with `/` and append NOTHING — the subsequent refresh re-lists the
    /// drilled directory, so both Tab and Enter chain.
    pub fn accept_suggestion(&mut self) {
        if let Some(idx) = self.suggestion_index
            && let Some(s) = self.suggestions.get(idx)
        {
            let new_cursor = match s.source {
                TypeaheadSource::SlashCommand => {
                    // Splice the command TOKEN — never the whole buffer.
                    // The text in FRONT of the command and the draft tail
                    // after the caret both survive (2026-11-12). The
                    // whole-buffer rewrite this replaces deleted both: the
                    // `@` arm has always spliced
                    // (`accept_suggestion_file_ref_after_text_preserves_prefix`),
                    // the slash arm never did.
                    let tail = self.text[self.cursor..].to_string();
                    match super::typeahead::slash_token_range(&self.text[..self.cursor]) {
                        Some((start, _end)) => {
                            let prefix = self.text[..start].to_string();
                            self.text = format!("{}{}{}", prefix, s.text, tail);
                            prefix.len() + s.text.len()
                        }
                        // No live token under the caret: an accept must not
                        // touch the buffer (the anchor-less contract the
                        // FileRef arm already has).
                        None => self.cursor,
                    }
                }
                TypeaheadSource::SkillRef => {
                    // Replace the live `$query` token with the chosen
                    // `$name`, keeping the text before it and the tail after
                    // the cursor. A trailing space lets the instruction follow.
                    let tail = self.text[self.cursor..].to_string();
                    match operant_core::skill_mentions::active_skill_token(
                        &self.text[..self.cursor],
                    ) {
                        Some(token) => {
                            let mut new_text = self.text[..token.start].to_string();
                            new_text.push_str(&s.text);
                            if !tail.starts_with(char::is_whitespace) {
                                new_text.push(' ');
                            }
                            let cursor = new_text.len();
                            new_text.push_str(&tail);
                            self.text = new_text;
                            cursor
                        }
                        // The anchor `$` is gone (Home with a stale popup):
                        // nothing to apply, leave the draft alone.
                        None => self.cursor,
                    }
                }
                TypeaheadSource::FileRef => {
                    // Replace from the last word-boundary @ up to the cursor.
                    // Preserve any text that was already after the cursor.
                    let tail = self.text[self.cursor..].to_string();
                    // The popup only offers FileRef values while a parseable
                    // `@` token is live, so the plain rfind is the same `@`
                    // the parser found.
                    if let Some(at_idx) = self.text[..self.cursor].rfind('@') {
                        let mut new_text = self.text[..at_idx].to_string();
                        new_text.push_str(&s.text);
                        // A file value ends the reference: trailing space,
                        // unless the preserved tail already starts with one.
                        // A directory value ends `/` and chains — no space.
                        if !s.text.ends_with('/') && !tail.starts_with(char::is_whitespace) {
                            new_text.push(' ');
                        }
                        let cursor = new_text.len();
                        new_text.push_str(&tail);
                        self.text = new_text;
                        cursor
                    } else {
                        // The anchor `@` is gone — the cursor moved before
                        // the popup's token (reachable via Home with a stale
                        // popup, which had no Esc dismissal either). The old
                        // fallback REPLACED THE WHOLE BUFFER with the bare
                        // suggestion value, destroying the draft. A
                        // suggestion with no anchor is nothing to apply:
                        // leave the text alone; the popup clear below
                        // dismisses it.
                        self.cursor
                    }
                }
            };
            self.cursor = new_cursor;
            self.suggestions.clear();
            self.suggestion_index = None;
            self.pending_file_query = None;
            self.update_token_estimate();
        }
    }

    /// Replace the full text buffer and move the cursor to the end.
    pub fn replace_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.history_pos = None;
        self.suggestion_index = None;
        self.pending_file_query = None;
        self.update_token_estimate();
    }
}
