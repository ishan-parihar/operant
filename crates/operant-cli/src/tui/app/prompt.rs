//! Prompt input and voice handling methods.

use super::*;

impl App {
    pub(super) fn prompt_mode(&self) -> InputMode {
        // Note: previously returned Readonly while streaming, but the prompt
        // now accepts input during streaming so the user can compose / queue
        // a follow-up message. Plan mode still wins.
        if self.plan_mode {
            InputMode::Plan
        } else {
            InputMode::Default
        }
    }

    pub(super) fn sync_legacy_prompt_fields(&mut self) {
        self.input = self.prompt_input.text.clone();
        self.cursor_pos = self.prompt_input.cursor;
    }

    /// Check if any modal dialog is open that should block suggestion updates.
    /// Mirrors claurst's file_injection_dialog guard for suggestion updates.
    pub(super) fn should_block_suggestions(&self) -> bool {
        self.connect_dialog.visible
            || self.import_config_picker.visible
            || self.import_config_dialog.visible
            || self.command_palette.visible
            || self.model_picker.visible
            || self.settings_screen.visible
            || self.export_dialog.visible
            || self.bypass_permissions_dialog.visible
            || self.key_input_dialog.visible
            || self.custom_provider_dialog.visible
            || self.free_mode_dialog.visible
            || self.device_auth_dialog.visible
            || self.ask_user_dialog.visible
    }

    pub fn refresh_prompt_input(&mut self) {
        self.prompt_input.mode = self.prompt_mode();
        // Every caller of this method follows a composer mutation, which
        // makes it the one choke point for the burst-undo counter. The
        // counter — not this call — decides when a snapshot is taken, so a
        // run of typing still collapses into a single undo step.
        self.prompt_input.record_edit();
        // Skip suggestion updates when a modal dialog is open (Phase 1.4).
        if !self.should_block_suggestions() {
            let file_autocomplete_limit = self.settings.config.file_autocomplete_limit;
            let file_autocomplete_show_hidden =
                self.settings.config.file_autocomplete_show_hidden_files;
            self.prompt_input.update_suggestions(
                &tui_slash_command_data(),
                file_autocomplete_limit,
                file_autocomplete_show_hidden,
            );
        }
        self.sync_legacy_prompt_fields();
    }

    pub fn set_prompt_text(&mut self, text: String) {
        self.prompt_input.replace_text(text);
        self.refresh_prompt_input();
    }

    // -----------------------------------------------------------------------
    // Input affordances: stash / burst undo / queued-message recall
    // -----------------------------------------------------------------------

    /// Ctrl+S — park the composer contents so a half-written prompt is not
    /// lost when the composer has to be free; the same chord brings it back
    /// and clears the stash.
    ///
    /// Not gated on `is_streaming`: the composer is deliberately editable
    /// mid-turn (text entry is allowed while streaming so the user can
    /// compose the next message), so parking and restoring it is exactly as
    /// safe there as it is while idle.
    pub fn toggle_input_stash(&mut self) {
        if self.prompt_input.stash.is_some() {
            if self.prompt_input.restore_stash() {
                self.refresh_prompt_input();
                self.status_message = Some("Stash restored into the composer.".to_string());
            }
            return;
        }
        if self.prompt_input.is_empty() {
            self.status_message = Some("Nothing to stash.".to_string());
            return;
        }
        self.prompt_input.stash_input();
        self.refresh_prompt_input();
        self.status_message = Some("Input stashed — Ctrl+S brings it back.".to_string());
    }

    /// Ctrl+Z — revert the last run of composer edits. Reports the state
    /// either way so the key is never silently inert.
    pub fn undo_input_burst(&mut self) {
        if self.prompt_input.undo_burst() {
            self.refresh_prompt_input();
            self.status_message = Some("Reverted the last edit burst.".to_string());
        } else {
            self.status_message = Some("Nothing to undo.".to_string());
        }
    }

    /// Ctrl+X — pull the next queued message back into the composer so it can
    /// be edited instead of being spent on the agent.
    pub fn recall_queued_message(&mut self) {
        // Never silently discard what is already in the composer: a recall
        // that overwrote it would lose a half-written prompt.
        if !self.prompt_input.is_empty() {
            self.status_message =
                Some("Composer is not empty — Ctrl+S stashes it before recall.".to_string());
            return;
        }
        match self.pop_queued_message() {
            Err(msg) => self.status_message = Some(msg.to_string()),
            Ok(None) => self.status_message = Some("Queue is empty".to_string()),
            Ok(Some(text)) => {
                self.prompt_input.replace_text(text);
                self.refresh_prompt_input();
                self.status_message = Some("Recalled a queued message for editing.".to_string());
            }
        }
    }

    /// Pop the head of the live steer queue — the same queue `/steer` and
    /// `/queue` write to. FIFO: index 0 is what the agent would consume
    /// next, since `drain_steers` drains the queue from the front.
    fn pop_queued_message(&self) -> Result<Option<String>, &'static str> {
        let Some(handle) = self.steer_queue_handle.as_ref() else {
            return Err("Nothing queued (no live agent).");
        };
        let Ok(mut q) = handle.try_lock() else {
            return Err("Queue is busy (agent is draining it).");
        };
        Ok(if q.is_empty() {
            None
        } else {
            Some(q.remove(0))
        })
    }

    // -----------------------------------------------------------------------
    // Voice PTT helpers
    // -----------------------------------------------------------------------

    /// Start PTT recording: open the microphone capture stream and signal the
    /// UI.  No-op when no voice recorder is attached or recording is already
    /// in progress.
    pub fn handle_voice_ptt_start(&mut self) {
        if self.voice_recording || self.voice_recorder.is_none() {
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        self.voice_event_rx = Some(rx);
        self.voice_recording = true;
        if let Some(ref recorder_arc) = self.voice_recorder {
            let recorder = recorder_arc.clone();
            tokio::task::spawn_blocking(move || {
                if let Ok(mut r) = recorder.lock() {
                    tokio::runtime::Handle::current()
                        .block_on(r.start_recording(tx))
                        .ok();
                }
            });
        }
        self.status_message =
            Some("Recording\u{2026} release V or press Enter to transcribe".to_string());
    }

    /// Stop PTT recording: flip the AtomicBool inside VoiceRecorder so the
    /// capture thread exits, then fire a "Transcribing…" notice.  The
    /// transcript text arrives later via `voice_event_rx` and is injected into
    /// the prompt by the event-loop drain.
    pub fn handle_voice_ptt_stop(&mut self) {
        if !self.voice_recording {
            return;
        }
        self.voice_recording = false;
        if let Some(ref recorder_arc) = self.voice_recorder {
            let recorder = recorder_arc.clone();
            tokio::task::spawn_blocking(move || {
                if let Ok(mut r) = recorder.lock() {
                    tokio::runtime::Handle::current()
                        .block_on(r.stop_recording())
                        .ok();
                }
            });
        }
        self.status_message = Some("Transcribing\u{2026}".to_string());
    }

    // (iter-209: attach_turn_diff_state + refresh_turn_diff_from_history
    // deleted — stub FileHistory removed, turn-diff feature cut as YAGNI.
    // /changes overlay now uses git-diff via diff_viewer's real path.)

    // (iter-208: attach_mcp_manager deleted — stub mcp_manager field removed.
    // load_mcp_servers now reads from core_mcp_manager, which is set directly
    // in TuiApp::enter via self.app.core_mcp_manager = Some(...).)

    pub fn take_pending_mcp_panel_auth(&mut self) -> Option<String> {
        self.pending_mcp_panel_auth.take()
    }

    pub fn take_pending_mcp_reconnect(&mut self) -> bool {
        let pending = self.pending_mcp_reconnect;
        self.pending_mcp_reconnect = false;
        pending
    }

    #[allow(dead_code)] // Called from providers.rs
    pub(super) fn clear_prompt(&mut self) {
        self.prompt_input.clear();
        self.refresh_prompt_input();
    }
}
