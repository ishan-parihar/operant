// app/turn_state.rs — Per-turn snapshot and metadata sync helpers.
//
// Extracted from the app/mod.rs monolith. Turn lifecycle helpers: agent-mode
// snapshots, user-turn begin/complete, transcript metadata sync, rewind flow
// entry, and onboarding persistence.

use super::*;

impl App {
    pub(crate) fn current_agent_mode_snapshot(&self) -> String {
        self.agent_mode
            .clone()
            .unwrap_or_else(|| if self.plan_mode { "plan" } else { "build" }.to_string())
    }

    #[allow(dead_code)] // Prepared for turn metadata tracking
    pub(crate) fn begin_user_turn_snapshot(&mut self) {
        self.turn_metadata.push(TurnMetadata {
            model_name: Some(self.model_name.clone()),
            agent_mode: Some(self.current_agent_mode_snapshot()),
            duration: None,
            interrupted: false,
        });
        // Start the latency timer now — at prompt-submission time — so it
        // measures actual round-trip time even when the provider buffers its
        // full response before yielding any stream events (e.g. Gemini flash).
        self.turn_start = Some(std::time::Instant::now());
        self.last_turn_elapsed = None;
        self.last_turn_verb = None;
    }

    pub(crate) fn sync_turn_metadata_to_messages(&mut self) {
        let user_count = self
            .messages
            .iter()
            .filter(|msg| msg.role == Role::User)
            .count();

        if self.turn_metadata.len() > user_count {
            self.turn_metadata.truncate(user_count);
            return;
        }

        while self.turn_metadata.len() < user_count {
            self.turn_metadata.push(TurnMetadata::default());
        }
    }

    pub(crate) fn complete_current_turn_snapshot(&mut self, interrupted: bool) {
        if let Some(index) = self.current_user_turn_index() {
            if self.turn_metadata.len() <= index {
                self.sync_turn_metadata_to_messages();
            }

            let model_name = self.model_name.clone();
            let agent_mode = self.current_agent_mode_snapshot();
            if let Some(meta) = self.turn_metadata.get_mut(index) {
                meta.duration = self.last_turn_elapsed.clone();
                meta.interrupted = interrupted;
                if meta.model_name.is_none() {
                    meta.model_name = Some(model_name);
                }
                if meta.agent_mode.is_none() {
                    meta.agent_mode = Some(agent_mode);
                }
            }
        }
    }

    pub(crate) fn scroll_step(&mut self) -> usize {
        let now = std::time::Instant::now();
        let elapsed_ms = self
            .scroll_last_time
            .map(|t| now.duration_since(t).as_millis())
            .unwrap_or(u128::MAX);
        self.scroll_last_time = Some(now);
        if elapsed_ms < 40 {
            // Trackpad burst — gradually accelerate
            self.scroll_accel = (self.scroll_accel + 0.4).min(6.0);
        } else {
            // Mouse click or first event — reset to base
            self.scroll_accel = 3.0;
        }
        self.scroll_accel.round() as usize
    }

    /// Open the rewind flow with the current message list converted to
    /// `SelectorMessage` entries.
    pub(crate) fn open_rewind_flow(&mut self) {
        let selector_msgs: Vec<SelectorMessage> = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let text = m.get_all_text();
                let preview: String = text.chars().take(80).collect();
                let has_tool_use = !m.get_tool_use_blocks().is_empty();
                SelectorMessage {
                    idx: i,
                    role: format!("{:?}", m.role).to_lowercase(),
                    preview,
                    has_tool_use,
                }
            })
            .collect();
        self.rewind_flow.open(selector_msgs);
    }

    // -------------------------------------------------------------------
    // Event handling
    // -------------------------------------------------------------------

    /// Persist `has_completed_onboarding = true` to the settings file.
    /// Best-effort: failures are silently ignored to not disrupt the session.
    pub(crate) fn persist_onboarding_complete() -> anyhow::Result<()> {
        let mut settings = crate::tui::adapter_types::config::Settings::load_sync()?;
        settings.has_completed_onboarding = true;
        settings.save_sync()
    }

    /// Enable bypass-permissions mode and persist it — the "arm" half of the
    /// `/yolo` toggle, shared with the `--dangerously-skip-permissions` startup
    /// dialog accept path.
    pub(crate) fn arm_bypass_permissions(&mut self) {
        use crate::tui::adapter_types::config::PermissionMode;
        let mut settings = crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
        settings.permission_mode = PermissionMode::BypassPermissions;
        let _ = settings.save_sync();
        self.settings.permission_mode = PermissionMode::BypassPermissions;
    }

    // ---- Advanced mouse interaction helpers --------------------------------

    // -------------------------------------------------------------------
    // Query event handling
    // -------------------------------------------------------------------

    /// Push a completed assistant message and trigger auto-scroll bookkeeping.
    pub(crate) fn push_assistant_message(&mut self, text: String) {
        let msg = Message::assistant(text);
        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }
}
