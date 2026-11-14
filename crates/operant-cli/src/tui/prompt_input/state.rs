// prompt_input/state.rs — Core state-management methods (new, clear, take, normalize).
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;
use operant_core::context_management::estimate_tokens;

impl PromptInputState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            vim_mode: VimMode::Insert,
            vim_enabled: false,
            mode: InputMode::Default,
            suggestions: Vec::new(),
            suggestion_index: None,
            history: Vec::new(),
            history_pos: None,
            history_draft: String::new(),
            suppress_suggestions_once: false,
            pending_pastes: Vec::new(),
            suppress_paste_expand: false,
            paste_expanded: None,
            paste_collapsed: None,
            history_recall: None,
            preferred_col: None,
            yank_buf: String::new(),
            token_estimate: 0,
            vim_pending: VimPendingState::None,
            undo_stack: Vec::new(),
            visual_anchor: None,
            last_find: None,
            vim_registers: std::collections::HashMap::new(),
            vim_macro_recording: None,
            vim_macro_content: std::collections::HashMap::new(),
            vim_marks: std::collections::HashMap::new(),
            vim_dot_action: None,
            vim_insert_text_before: None,
            vim_command_buf: String::new(),
            vim_search_buf: String::new(),
            vim_search_last: None,
            vim_quit_requested: false,
            sel_anchor: None,
            pending_images: Vec::new(),
            kill_ring: KillRing::new(),
            stash: None,
            burst_undo: Vec::new(),
            burst_anchor: (String::new(), 0),
            burst_edits: 0,
            pending_file_query: None,
            ghost: None,
        }
    }

    /// Add a clipboard image attachment to the pending list.
    pub fn add_image(&mut self, img: crate::image_paste::PastedImage) {
        self.pending_images.push(img);
    }

    /// Normalized byte-offset range of the active shift-selection, or None.
    /// The anchor is the fixed end, the cursor the moving end — exactly
    /// jcode's textarea selection model (2026-10-09 visual audit). Consumed
    /// by the vendored composer renderer (ui_input::wrap_input_text) for
    /// reverse-video highlighting and by Ctrl+C for copy.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.sel_anchor.map(|anchor| {
            if anchor <= self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    /// The selected text, clamped to char boundaries (multibyte-safe).
    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let start = self.snap_to_char_boundary(start);
        let end = self.snap_to_char_boundary(end);
        self.text.get(start..end).map(|s| s.to_string())
    }

    fn snap_to_char_boundary(&self, mut idx: usize) -> usize {
        while idx < self.text.len() && !self.text.is_char_boundary(idx) {
            idx += 1;
        }
        idx
    }

    /// Begin or extend a shift-selection: anchor the fixed end if unset.
    pub fn selection_begin_or_extend(&mut self) {
        if self.sel_anchor.is_none() {
            self.sel_anchor = Some(self.cursor);
        }
    }

    /// Collapse the selection (any non-extending edit key).
    pub fn selection_clear(&mut self) {
        self.sel_anchor = None;
    }

    /// Drain and return all pending image attachments (called at send time).
    pub fn clear_images(&mut self) -> Vec<crate::image_paste::PastedImage> {
        std::mem::take(&mut self.pending_images)
    }

    /// Clear the input and reset state.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.suggestions.clear();
        self.suggestion_index = None;
        self.history_pos = None;
        self.history_recall = None;
        self.preferred_col = None;
        self.pending_pastes.clear();
        // A clear ends every paste-scoped one-shot. Leaving the suppress flag
        // armed made the NEXT paste's payload get dropped by a stale flag, and
        // leaving a notice set popped it over an empty composer.
        self.suppress_paste_expand = false;
        self.paste_expanded = None;
        self.paste_collapsed = None;
        self.token_estimate = 0;
        self.vim_pending = VimPendingState::None;
        self.visual_anchor = None;
        self.vim_command_buf.clear();
        self.vim_search_buf.clear();
    }

    /// Take the current text, clearing the input.
    /// Large pastes are shown as placeholders; the taken text carries the
    /// real content (2026-10-13: the store was never read, so the model got
    /// "[Pasted ~N lines #K]").
    pub fn take(&mut self) -> String {
        let text = expand_paste_placeholders(&self.text, &self.pending_pastes);
        // The store holds sanitised payloads (see `paste`), so this is a guard:
        // the composer's invariant is that it never holds a control byte, and
        // the outgoing prompt is the last place to enforce it. A byte the
        // provider's JSON would otherwise carry has no business leaving here.
        let text = super::history::sanitize_pasted_text(&text);
        self.clear();
        text
    }

    /// Drop the payloads whose placeholders are no longer in the text, and
    /// do nothing else.
    ///
    /// Codex's placeholder is an atomic `TextElement` and
    /// `reconcile_deleted_elements` does this the moment the element leaves
    /// the buffer. This store has no ranges, so the invariant is enforced
    /// here instead — called from the composer's refresh choke point, which
    /// every mutation already converges on. A placeholder removed by an edit
    /// takes its clipboard content with it: nothing can resurrect it, and the
    /// number is reclaimed by the next paste of the same shape (RT-559).
    pub fn drop_missing_paste_placeholders(&mut self) {
        if self.pending_pastes.is_empty() {
            return;
        }
        self.pending_pastes
            .retain(|(placeholder, _)| paste::is_live_placeholder(&self.text, placeholder));
    }

    /// Is the history ladder LIVE right now?
    ///
    /// Browsing is owned by the recalled text, not by "we are somewhere in the
    /// ladder": the ladder answers Up/Down only while the composer still holds
    /// what was recalled. An edit after a recall (which does not clear
    /// `history_pos`) used to keep the ladder walking and swallow the edit —
    /// codex's `should_handle_navigation` rule (RT-560).
    pub fn history_is_live(&self) -> bool {
        // A position with no recall recorded is still browsing: the recall is
        // written by every ladder step, and a caller that sets `history_pos`
        // directly (a test, a restore path) is asserting a live browse.
        self.history_pos.is_some()
            && self
                .history_recall
                .as_deref()
                .is_none_or(|recalled| recalled == self.text)
    }

    /// Drop stale browsing state. Called by the ladder when
    /// [`Self::history_is_live`] says the recall was edited away, so the next
    /// navigation starts from the edited text instead of the ladder.
    pub fn end_history_browse(&mut self) {
        self.history_pos = None;
        self.history_recall = None;
    }

    /// Normalize cursor and metadata after external field updates.
    pub fn normalize(&mut self) {
        self.cursor = self.cursor.min(self.text.len());
        while self.cursor > 0 && !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        self.update_token_estimate();
    }

    /// Rough token estimate: the canonical chars/4 estimator.
    pub(crate) fn update_token_estimate(&mut self) {
        self.token_estimate = estimate_tokens(&self.text);
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    // -----------------------------------------------------------------------
    // Stash (Ctrl+S) — park the composer, then bring it back
    // -----------------------------------------------------------------------

    /// Park the current contents and empty the composer. One slot: a stash
    /// held while another is stashed is replaced (not stacked), so a stray
    /// second Ctrl+S can never strand a prompt in a hidden stack. Moved out
    /// with `mem::take`, so parking costs no clone.
    pub fn stash_input(&mut self) {
        // The store rides with the text: `clear()` empties `pending_pastes`, and
        // bringing back a composer full of placeholders with nothing behind them
        // is how the model got `[Pasted ~5 lines]` instead of the paste.
        self.stash = Some((
            std::mem::take(&mut self.text),
            std::mem::take(&mut self.pending_pastes),
        ));
        self.clear();
    }

    /// Bring the held stash back and clear it. `false` when nothing is held.
    pub fn restore_stash(&mut self) -> bool {
        match self.stash.take() {
            Some((text, pastes)) => {
                self.text = text;
                self.pending_pastes = pastes;
                self.cursor = self.text.len();
                self.history_pos = None;
                self.suggestion_index = None;
                self.update_token_estimate();
                true
            }
            None => false,
        }
    }

    // -----------------------------------------------------------------------
    // Burst undo (Ctrl+Z) — revert a run of edits, not one keystroke
    // -----------------------------------------------------------------------

    /// Count one composer mutation. Called from the app's single
    /// post-edit sync point, so it is the one place a snapshot can be
    /// taken. Nothing is cloned here: the stack only grows when a burst
    /// closes, and it stores `burst_anchor` — the state the burst started
    /// from — so one Ctrl+Z reverts the whole run.
    pub fn record_edit(&mut self) {
        self.burst_edits += 1;
        if self.burst_edits < UNDO_COALESCE {
            return;
        }
        self.close_burst();
    }

    /// Snapshot the open burst's start state and re-arm the anchor at the
    /// current state. Skips the push when the anchor already matches the
    /// top of the stack, so a run of no-op edits (backspace at offset 0)
    /// cannot fill the stack with identical states and make Ctrl+Z look
    /// broken.
    fn close_burst(&mut self) {
        self.burst_edits = 0;
        if self
            .burst_undo
            .last()
            .is_none_or(|(text, _)| *text != self.burst_anchor.0)
        {
            self.burst_undo.push(self.burst_anchor.clone());
            if self.burst_undo.len() > UNDO_STACK_MAX {
                self.burst_undo.remove(0);
            }
        }
        self.burst_anchor = (self.text.clone(), self.cursor);
    }

    /// Revert to the previous burst snapshot. `false` when there is nothing
    /// to revert, so the caller can say so instead of silently doing nothing.
    pub fn undo_burst(&mut self) -> bool {
        // The run in progress starts at the anchor, which is by definition
        // not on the stack yet, so the first Ctrl+Z after typing reverts that
        // run instead of waiting for UNDO_COALESCE edits to land.
        if self.burst_edits > 0 && self.burst_anchor.0 != self.text {
            let (text, cursor) = self.burst_anchor.clone();
            self.restore_snapshot(text, cursor);
            return true;
        }
        // Otherwise walk one closed burst back. A run of no-op edits
        // (backspace at offset 0) lands here and reports honestly rather
        // than "reverting" to an identical buffer.
        self.burst_edits = 0;
        let Some((text, cursor)) = self.burst_undo.pop() else {
            return false;
        };
        self.restore_snapshot(text, cursor);
        true
    }

    /// Put a snapshot back into the composer and re-arm the burst anchor at
    /// it, so the next undo walks further back instead of redoing this step.
    fn restore_snapshot(&mut self, text: String, cursor: usize) {
        self.text = text;
        self.cursor = cursor.min(self.text.len());
        self.history_pos = None;
        // An undo can restore a text with no placeholder in it while the store
        // still holds the payload. Drop it here so the invariant ("every stored
        // payload's placeholder is in the text") is true at every exit, not
        // only after the next refresh.
        self.drop_missing_paste_placeholders();
        self.update_token_estimate();
        self.burst_anchor = (self.text.clone(), self.cursor);
        self.burst_edits = 0;
    }
}
