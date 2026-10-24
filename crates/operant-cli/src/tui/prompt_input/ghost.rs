//! Ghost-text word completion state (plan 2026-10-24 M6).
//!
//! The engine (M5) answers asynchronously; this module owns WHEN a ghost
//! may show (the prose gate — OMP `wordCompletionQuery` port), the
//! projection that keeps a consistent ghost while the user types through
//! it, and the accept/reject key semantics: TAB accepts with a trailing
//! space, Right accepts without, any divergence drops the ghost.

use super::*;

/// The shown ghost: `prefix` is what was typed when it was computed,
/// `suffix` paints dim after the caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostText {
    pub prefix: String,
    pub suffix: String,
}

impl PromptInputState {
    /// The word-completion query at the caret: `(text before the word, the
    /// word)` — or `None` wherever a ghost must not appear: while the
    /// suggestion popup is open, inside an `@` reference, in a
    /// slash-command input, inside a backtick code span, mid-word, or on a
    /// non-alphabetic "word" (paths, numbers).
    pub fn ghost_query(&self) -> Option<(String, String)> {
        if self.mode == InputMode::Readonly {
            return None;
        }
        if !self.suggestions.is_empty() {
            return None; // the popup owns the composer's attention
        }
        let before_cursor = &self.text[..self.cursor];
        if parse_file_ref_token(before_cursor).is_some() {
            return None; // inside an @file:/@folder: reference
        }
        if self.text.trim_start().starts_with('/') {
            return None; // slash-command input
        }
        if before_cursor.matches('`').count() % 2 == 1 {
            return None; // inside a backtick code span
        }
        if self.text[self.cursor..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic())
        {
            return None; // mid-word: the caret must sit at the word's end
        }
        let word: String = before_cursor
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic() || *c == '\u{2019}' || *c == '\'')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // Single letters still ask (`figure o|ut` — OMP's operating point);
        // a leading apostrophe or digit run is not a prose word.
        if word.chars().next().is_some_and(|c| c.is_alphabetic()) {
            let word_start = self.cursor - word.len();
            return Some((self.text[..word_start].to_string(), word));
        }
        None
    }

    /// Apply a fresh engine answer. Called by the App drain only when the
    /// answer's generation and token still match the live query.
    pub fn show_ghost(&mut self, prefix: String, suffix: String) {
        if suffix.is_empty() {
            return;
        }
        self.ghost = Some(GhostText { prefix, suffix });
    }

    /// Projection (OMP `#project`): while the typed word still extends the
    /// ghost's full word, keep showing the remainder without re-asking the
    /// engine. Returns true when the existing ghost was projected.
    pub fn project_ghost(&mut self, word: &str) -> bool {
        let Some(g) = &self.ghost else { return false };
        let full = format!("{}{}", g.prefix, g.suffix).to_lowercase();
        let lower = word.to_lowercase();
        if full.starts_with(&lower) && full.chars().count() > lower.chars().count() {
            let suffix: String = full.chars().skip(lower.chars().count()).collect();
            self.ghost = Some(GhostText {
                prefix: word.to_string(),
                suffix,
            });
            true
        } else {
            self.ghost = None;
            false
        }
    }

    /// Drop the ghost (divergence, cursor moves, popups opening).
    pub fn clear_ghost(&mut self) {
        self.ghost = None;
    }

    /// Accept the shown ghost at the caret. `with_space` appends the
    /// trailing space (TAB); Right accepts without it. Returns false when
    /// no ghost is shown.
    pub fn accept_ghost(&mut self, with_space: bool) -> bool {
        let Some(g) = self.ghost.take() else {
            return false;
        };
        for c in g.suffix.chars() {
            self.insert_char(c);
        }
        if with_space {
            self.insert_char(' ');
        }
        true
    }
}
