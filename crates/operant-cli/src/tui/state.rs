//! TuiState trait — the presentation interface for the transcript renderer.
//!
//! Instead of the renderer taking `&App` directly (tight coupling), it takes
//! `&dyn TuiState`. This enables:
//! - **Testability**: mock state for unit testing renderers
//! - **Decoupling**: renderers don't know about App internals
//! - **Composability**: different state implementations for different modes
//!
//! Modeled after jcode's `TuiState` trait but minimal for operant's current
//! feature set — only the methods the renderers actually need today.

use crate::tui::adapter_types::types::Message;

/// Read-only presentation state consumed by the transcript renderer.
///
/// This trait abstracts the minimum surface area that renderers need from App.
/// As operant's TUI grows, new methods are added here — NOT as direct `&App`
/// references in renderer functions.
///
/// Methods marked with default implementations are not yet consumed by any
/// renderer. They exist to keep the trait aligned with the full App surface
/// area and will be used as the rendering pipeline matures.
pub trait TuiState {
    // ---- Transcript (REQUIRED — used by renderers today) ----
    fn messages(&self) -> &[Message];
    fn streaming_text(&self) -> &str;
    fn is_streaming(&self) -> bool;

    // ---- Input (REQUIRED — used by renderers today) ----
    fn input_text(&self) -> &str;

    // ---- Scroll (REQUIRED — used by renderers today) ----
    fn scroll_offset(&self) -> usize;
    fn auto_scroll(&self) -> bool;

    // ---- Provider / model (REQUIRED — used by status bar) ----
    fn model_name(&self) -> &str;

    // ---- Status (REQUIRED — used by renderers today) ----
    fn status_message(&self) -> Option<&str>;
    fn frame_count(&self) -> u64;

    // ---- Messages count (REQUIRED — used by scroll logic) ----
    fn message_count(&self) -> usize;

    // ---- Optional: streaming thinking (default: empty) ----
    fn streaming_thinking(&self) -> &str {
        ""
    }

    // ---- Optional: reasoning visibility (default: hidden) ----
    fn show_reasoning(&self) -> bool {
        false
    }

    // ---- Optional: cursor position (default: 0) ----
    fn cursor_pos(&self) -> usize {
        0
    }

    // ---- Optional: active provider (default: None) ----
    fn active_provider(&self) -> Option<&str> {
        None
    }

    // ---- Optional: effort level label (default: "normal") ----
    fn effort_level_label(&self) -> &str {
        "normal"
    }

    // ---- Optional: fast mode (default: false) ----
    fn fast_mode(&self) -> bool {
        false
    }

    // ---- Optional: spinner verb (default: "thinking") ----
    fn spinner_verb(&self) -> &str {
        "thinking"
    }

    // ---- Optional: streaming active flag (default: same as is_streaming) ----
    fn is_streaming_active(&self) -> bool {
        self.is_streaming()
    }

    // ---- Optional: cost (default: 0.0) ----
    fn cost_usd(&self) -> f64 {
        0.0
    }

    // ---- Optional: token count (default: 0) ----
    fn token_count(&self) -> u32 {
        0
    }

    // ---- Optional: git branch (default: None) ----
    fn git_branch(&self) -> Option<&str> {
        None
    }

    // ---- Optional: current directory (default: None) ----
    fn current_dir(&self) -> Option<&str> {
        None
    }

    // ---- Optional: session goal (default: None) ----
    fn session_goal(&self) -> Option<&str> {
        None
    }

    // ---- Optional: help visible (default: false) ----
    fn help_visible(&self) -> bool {
        false
    }

    // ---- Optional: any modal open (default: false) ----
    fn any_modal_open(&self) -> bool {
        false
    }

    // ---- Optional: session elapsed (default: zero) ----
    fn session_start_elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    // ---- Optional: turn elapsed (default: None) ----
    fn turn_elapsed(&self) -> Option<String> {
        None
    }

    // ---- Optional: last turn verb (default: None) ----
    fn last_turn_verb(&self) -> Option<&'static str> {
        None
    }

    // ---- Optional: client focused (default: true) ----
    fn client_focused(&self) -> bool {
        true
    }

    // ---- Optional: plan mode (default: false) ----
    fn plan_mode(&self) -> bool {
        false
    }

    // ---- Optional: new messages while scrolled (default: 0) ----
    fn new_messages_while_scrolled(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test-only TuiState implementation.
    struct MockState {
        messages: Vec<Message>,
        streaming_text: String,
        is_streaming: bool,
        input: String,
        scroll: usize,
        model: String,
    }

    impl TuiState for MockState {
        fn messages(&self) -> &[Message] {
            &self.messages
        }
        fn streaming_text(&self) -> &str {
            &self.streaming_text
        }
        fn is_streaming(&self) -> bool {
            self.is_streaming
        }
        fn input_text(&self) -> &str {
            &self.input
        }
        fn scroll_offset(&self) -> usize {
            self.scroll
        }
        fn auto_scroll(&self) -> bool {
            true
        }
        fn model_name(&self) -> &str {
            &self.model
        }
        fn status_message(&self) -> Option<&str> {
            None
        }
        fn frame_count(&self) -> u64 {
            0
        }
        fn message_count(&self) -> usize {
            self.messages.len()
        }
    }

    #[test]
    fn mock_state_compiles_with_defaults() {
        let state = MockState {
            messages: vec![],
            streaming_text: String::new(),
            is_streaming: false,
            input: String::new(),
            scroll: 0,
            model: "test-model".to_string(),
        };
        // Required methods
        assert!(state.messages().is_empty());
        assert!(!state.is_streaming());
        assert_eq!(state.model_name(), "test-model");
        assert_eq!(state.message_count(), 0);
        // Default methods
        assert_eq!(state.streaming_thinking(), "");
        assert!(!state.show_reasoning());
        assert_eq!(state.effort_level_label(), "normal");
        assert!(!state.fast_mode());
        assert_eq!(state.spinner_verb(), "thinking");
        assert!(!state.help_visible());
        assert!(!state.any_modal_open());
        assert!(state.client_focused());
        assert!(!state.plan_mode());
    }
}
