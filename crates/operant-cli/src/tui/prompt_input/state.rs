// prompt_input/state.rs — Core state-management methods (new, clear, take, normalize).
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;

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
            paste_counter: 0,
            paste_contents: std::collections::HashMap::new(),
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
            pending_images: Vec::new(),
            kill_ring: KillRing::new(),
            stash: None,
            burst_undo: Vec::new(),
            burst_anchor: (String::new(), 0),
            burst_edits: 0,
        }
    }

    /// Add a clipboard image attachment to the pending list.
    pub fn add_image(&mut self, img: crate::image_paste::PastedImage) {
        self.pending_images.push(img);
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
        self.token_estimate = 0;
        self.vim_pending = VimPendingState::None;
        self.visual_anchor = None;
        self.vim_command_buf.clear();
        self.vim_search_buf.clear();
    }

    /// Take the current text, clearing the input.
    pub fn take(&mut self) -> String {
        let text = self.text.clone();
        self.clear();
        text
    }

    /// Normalize cursor and metadata after external field updates.
    pub fn normalize(&mut self) {
        self.cursor = self.cursor.min(self.text.len());
        while self.cursor > 0 && !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        self.update_token_estimate();
    }

    /// Rough token estimate: ~4 chars per token.
    pub(crate) fn update_token_estimate(&mut self) {
        self.token_estimate = self.text.len().div_ceil(4);
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
        self.stash = Some(std::mem::take(&mut self.text));
        self.clear();
    }

    /// Bring the held stash back and clear it. `false` when nothing is held.
    pub fn restore_stash(&mut self) -> bool {
        match self.stash.take() {
            Some(text) => {
                self.text = text;
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
        self.update_token_estimate();
        self.burst_anchor = (self.text.clone(), self.cursor);
        self.burst_edits = 0;
    }
}
