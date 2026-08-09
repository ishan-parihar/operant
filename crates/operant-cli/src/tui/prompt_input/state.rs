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
}
