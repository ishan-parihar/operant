//! Message handling and notification methods.

use super::*;

impl App {
    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    pub(super) fn flush_streamed_assistant_message(&mut self) {
        if self.streaming_text.trim().is_empty() && self.streaming_thinking.trim().is_empty() {
            self.streaming_text.clear();
            self.streaming_thinking.clear();
            return;
        }

        let thinking = std::mem::take(&mut self.streaming_thinking);
        let text = std::mem::take(&mut self.streaming_text);

        let mut blocks = Vec::new();
        if !thinking.trim().is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking,
                signature: String::new(),
            });
        }
        if !text.is_empty() {
            blocks.push(ContentBlock::Text { text });
        }

        let msg = match blocks.len() {
            0 => return,
            1 => match blocks.pop().unwrap() {
                ContentBlock::Text { text } => Message::assistant(text),
                block => Message::assistant_blocks(vec![block]),
            },
            _ => Message::assistant_blocks(blocks),
        };

        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }
    /// Handle slash commands that should open UI screens rather than execute
    /// Add a message directly (e.g. from a non-streaming source).
    #[allow(dead_code)] // Used in tests + gateway_runner
    pub fn add_message(&mut self, role: Role, text: String) {
        let msg = match role {
            Role::User => Message::user(text),
            Role::Assistant => Message::assistant(text),
            Role::System => Message {
                role: Role::System,
                content: crate::tui::adapter_types::types::MessageContent::Text(text),
            },
        };
        if role == Role::User {
            self.begin_user_turn_snapshot();
        }
        self.messages.push(msg);
        self.invalidate_transcript();
        self.on_new_message();
    }

    /// Terminal-style clear (Ctrl+L, jcode parity): collapse the chat like a
    /// terminal after `clear` — the transcript chunk renders zero-height and
    /// the composer sits at the top. Nothing is deleted: any new output
    /// (which bumps `transcript_version`), streaming, or scrolling up
    /// immediately restores the full layout. Contrast `/clear`, which drops
    /// context entirely.
    pub fn clear_view_terminal_style(&mut self) {
        self.terminal_clear_version.set(Some(self.transcript_version.get()));
        // Snap to the bottom so the collapsed frame shows the composer at top.
        self.scroll_offset = 0;
        self.auto_scroll = true;
        self.new_messages_while_scrolled = 0;
    }

    /// Whether usable credentials exist RIGHT NOW, not a snapshot taken at
    /// `App::new`. The init-time `has_credentials` boolean goes stale the
    /// moment a credential arrives from anywhere other than the boot-time
    /// auth-store/env scan (omp/custom base URLs, `/login` mid-session on a
    /// fresh install) — which left the jcode header pinned in
    /// "/login to add provider" mode for an entire working session. The
    /// header transition bug (2026-10-09 visual audit, complaint 2) traced
    /// to exactly this: `auth_status()` mapped the stale boolean onto the
    /// provider matrix and nothing ever refreshed it.
    pub fn credentials_live(&self) -> bool {
        self.has_credentials
            || self.auth_store.has_any_key()
            || crate::tui::adapter_types::config::resolve_api_key().is_some()
    }

    /// Whether the terminal-clear state is live right now. Derived the way
    /// jcode derives its spacer-tail check, adapted to operant's
    /// version-keyed display cache: the clear holds only while the version
    /// captured at Ctrl+L still matches and the same idle conditions hold.
    pub fn terminal_clear_state_live(&self) -> bool {
        self.terminal_clear_version.get() == Some(self.transcript_version.get())
            && self.auto_scroll
            && !self.is_streaming
            && self.streaming_text.is_empty()
    }

    /// Push a synthetic system annotation into the conversation pane.
    /// It will appear after the current last message.
    /// Push a notification and, for Error-kind notifications, reset the error
    /// modal scroll offset so a newly arrived error is always shown from the top.
    pub fn push_notification(
        &mut self,
        kind: NotificationKind,
        msg: String,
        duration_secs: Option<u64>,
    ) {
        if kind == NotificationKind::Error {
            self.error_modal_scroll_offset = 0;
        }
        self.notifications.push(kind, msg, duration_secs);
    }

    pub fn push_system_message(&mut self, text: String, style: SystemMessageStyle) {
        self.system_annotations.push(SystemAnnotation {
            after_index: self.messages.len(),
            text,
            style,
        });
        self.invalidate_transcript();
    }

    /// Called whenever a new message is appended to `messages`.
    /// Manages the auto-scroll / new-message-counter state.
    pub(super) fn on_new_message(&mut self) {
        if self.auto_scroll {
            // Auto-scroll: keep offset at 0 so render shows the bottom.
            self.scroll_offset = 0;
        } else {
            self.new_messages_while_scrolled = self.new_messages_while_scrolled.saturating_add(1);
        }
    }

    pub fn invalidate_transcript(&self) {
        self.transcript_version
            .set(self.transcript_version.get().wrapping_add(1));
    }

    /// Check current token usage and push token warning notifications as
    /// appropriate.  Call this after updating `token_count`.
    pub fn check_token_warnings(&mut self) {
        let window = crate::tui::adapter_types::context_window_for_model(&self.model_name) as u32;
        if window == 0 {
            return;
        }
        let pct = (self.token_count as f64 / window as f64 * 100.0) as u8;

        // Usage dropped back below the last-shown threshold (e.g. /clear or
        // /compact shrank the context) — reset so warnings can re-fire on
        // the way back up instead of being suppressed forever.
        if pct < self.token_warning_threshold_shown {
            self.token_warning_threshold_shown = 0;
        }

        // Only escalate — never repeat a threshold already shown.
        if pct >= 100 && self.token_warning_threshold_shown < 100 {
            self.token_warning_threshold_shown = 100;
            self.push_notification(
                NotificationKind::Error,
                "Context window full. Running auto-compact\u{2026}".to_string(),
                None,
            );
        } else if pct >= 95 && self.token_warning_threshold_shown < 95 {
            self.token_warning_threshold_shown = 95;
            self.push_notification(
                NotificationKind::Error,
                "Context window 95% full! Run /compact now.".to_string(),
                None, // persistent until dismissed
            );
        } else if pct >= 80 && self.token_warning_threshold_shown < 80 {
            self.token_warning_threshold_shown = 80;
            self.push_notification(
                NotificationKind::Warning,
                "Context window 80% full. Consider /compact.".to_string(),
                Some(30),
            );
        }
    }

    /// Drain any pasted images waiting to be attached and, if there were any,
    /// warn that they weren't actually sent. Call this once a message has
    /// been submitted. Images can't be attached yet because the core
    /// client's request path has no multi-part content support — without
    /// this, the thumbnail row would linger forever and look like the
    /// image was sent when it silently wasn't.
    pub fn drop_pending_images_with_notice(&mut self) {
        let dropped = self.prompt_input.clear_images();
        if !dropped.is_empty() {
            self.push_notification(
                NotificationKind::Warning,
                format!(
                    "Image attachments aren't sent to the model yet — {} image(s) dropped.",
                    dropped.len()
                ),
                Some(6),
            );
        }
    }

    /// Decide what the composer holds after a failed submission.
    ///
    /// Three rules, in order:
    ///
    /// 1. **Nothing was submitted** (or only whitespace) → [`ComposerRecovery::Nothing`].
    /// 2. **The composer already holds text** → [`ComposerRecovery::KeepExisting`].
    ///    Never clobber and never append: appending would splice a stale
    ///    fragment onto the failed prompt and produce a prompt the user never
    ///    wrote, while clobbering would destroy what they just typed. The
    ///    failed text is not lost either way — `take_input` already pushed it
    ///    onto in-session and on-disk history, so Up-arrow / Ctrl+R reaches it.
    /// 3. Otherwise → [`ComposerRecovery::Restore`], the text unchanged.
    ///
    /// No length bound is applied. The composer has no cap of its own anywhere
    /// in this TUI (the only truncation is render-time width clipping in the
    /// dialogs and the diff viewer), so truncating on the way back in would
    /// invent a policy the buffer does not have and would hand back a prompt
    /// that no longer says what the user typed.
    pub(crate) fn resolve_composer_after_failure(
        submitted: Option<&str>,
        current: &str,
    ) -> ComposerRecovery {
        let Some(text) = submitted else {
            return ComposerRecovery::Nothing;
        };
        if text.trim().is_empty() {
            return ComposerRecovery::Nothing;
        }
        if !current.trim().is_empty() {
            return ComposerRecovery::KeepExisting;
        }
        ComposerRecovery::Restore(text.to_string())
    }

    /// Hand the text of a failed submission back to the composer, so a turn
    /// that dies on a bad model id or a dead endpoint costs the user their
    /// prompt rather than their prompt *and* the turn.
    ///
    /// Restore-once falls out of the `take()`: the same failure can reach this
    /// through two doors — the agent emits `AgentEvent::Error` itself
    /// (`agent/run.rs`) and then returns `Err`, which makes
    /// `drain_run_complete` synthesize a second one — and the first call has
    /// already emptied the slot, so the second is a no-op instead of a double
    /// paste.
    pub(crate) fn restore_failed_input_to_composer(&mut self) {
        let submitted = self.failed_input_recovery.take();
        match Self::resolve_composer_after_failure(submitted.as_deref(), &self.prompt_input.text) {
            ComposerRecovery::Nothing => {}
            ComposerRecovery::KeepExisting => {
                self.failed_input_recovery = None;
            }
            ComposerRecovery::Restore(text) => {
                self.prompt_input.replace_text(text);
                self.refresh_prompt_input();
                self.status_message =
                    Some("Prompt restored to the composer — the turn failed.".to_string());
            }
        }
    }

    /// Drop the held submission without restoring it.
    ///
    /// Called on every path that consumes a submission without producing a
    /// turn outcome of its own, so a failure in some *later* turn can never
    /// resurrect an unrelated prompt from an earlier one.
    pub(crate) fn clear_failed_input_recovery(&mut self) {
        self.failed_input_recovery = None;
    }

    /// Take the current input buffer, push it to history, and return it.
    pub fn take_input(&mut self) -> String {
        let input = self.prompt_input.take();
        // Arm the restore slot for the turn this submission is about to start.
        // A later submission overwrites it, so the slot can only ever hold the
        // text of the turn currently in flight.
        self.failed_input_recovery = (!input.trim().is_empty()).then(|| input.clone());
        if !input.is_empty() {
            self.prompt_input.history.push(input.clone());
            self.prompt_input.history_pos = None;
            self.prompt_input.history_draft.clear();
            // Persist the new entry to ~/.operant/history.jsonl so it
            // survives restarts. (iter-125 — persistent input history.)
            crate::tui::input_history::append(&input);
        }
        self.refresh_prompt_input();
        input
    }
}
