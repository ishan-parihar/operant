//! Message handling and notification methods.

use super::*;

impl App {
    #[expect(
        clippy::unwrap_used,
        reason = "the `1 =>` arm matches a block count of one, so pop yields that block"
    )]
    pub(crate) fn flush_streamed_assistant_message(&mut self) {
        if self.streaming_text.trim().is_empty() && self.streaming_thinking.trim().is_empty() {
            self.streaming_text.clear();
            self.streaming_thinking.clear();
            return;
        }

        let thinking = std::mem::take(&mut self.streaming_thinking);
        let text = std::mem::take(&mut self.streaming_text);

        let mut blocks = Vec::new();
        if !thinking.trim().is_empty()
            // Full and Current both commit the trace where it streamed
            // (jcode anchor_current_reasoning_block); the adapter hides
            // Current traces once the next user prompt lands. Off never
            // accumulates — this guards a mid-turn flip to Off.
            && crate::tui::operant_app::config_shim::config()
                .display
                .reasoning_display()
                != crate::tui::operant_app::config_shim::ReasoningDisplayMode::Off
        {
            blocks.push(ContentBlock::Thinking {
                thinking,
                signature: String::new(),
            });
        }
        if !text.is_empty() {
            blocks.push(ContentBlock::Text { text });
        }

        let msg = match blocks.len() {
            0 => {
                // Nothing to commit (Current mode dropped a thinking-only
                // iteration). Release the slot ToolStart reserved for these
                // tails, or the tool rows stay "reserved" and later anchor
                // below whatever message is pushed next.
                let committed = self.messages.len();
                for block in &mut self.tool_use_blocks {
                    block.after_index = block.after_index.min(committed);
                }
                self.invalidate_transcript();
                return;
            }
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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "AD-034: test seeding helper; production appends through its own paths"
        )
    )]
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
        self.terminal_clear_version
            .set(Some(self.transcript_version.get()));
        // Snap to the bottom so the collapsed frame shows the composer at top.
        self.scroll_offset = 0;
        self.auto_scroll = true;
        self.new_messages_while_scrolled = 0;
        // Terminal-scroll mode: emitted history is append-only, so the
        // clear boundary needs a visible divider row in native scrollback
        // (the design doc's spacer). The emitter prints it before the
        // next settled content; the watermark does NOT reset.
        if self.terminal_scroll_mode {
            self.scroll_cls_divider.set(true);
        }
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

    /// Set the transient status-line notice (jcode `set_status_notice`
    /// parity, P4-2). Single write seam: stamps `status_message_set_at` so
    /// the `status_detail`/`status_notice` accessors can self-hide the
    /// notice after 3s without a tick loop. All `status_message = Some(..)`
    /// writes route here.
    pub fn set_status_notice(&mut self, text: impl Into<String>) {
        self.status_message = Some(text.into());
        self.status_message_set_at = Some(std::time::Instant::now());
    }

    /// P4-2 routing seam — the jcode notification taxonomy:
    /// Info/Success/Warning are transient status notices; Error lands an
    /// inline transcript card (persistent, inspectable, scrollable) plus
    /// the status notice. This replaces the overlay-banner queue.
    pub fn push_notice(&mut self, kind: NotificationKind, msg: String) {
        match kind {
            NotificationKind::Error => {
                self.push_system_message(msg.clone(), SystemMessageStyle::Error);
                self.set_status_notice(msg);
            }
            NotificationKind::Info | NotificationKind::Warning => {
                self.set_status_notice(msg);
            }
        }
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
        if !self.auto_scroll {
            self.new_messages_while_scrolled = self.new_messages_while_scrolled.saturating_add(1);
        }
        // While following, `scroll_offset` is left alone here: the render path
        // paints the tail whatever it holds, and `reconcile_scroll_anchor`
        // syncs the state to that row after the frame. Writing 0 here (the old
        // "0 = bottom" reading) made the next user scroll start from a stale
        // top row instead of the tail.
    }

    pub fn invalidate_transcript(&self) {
        self.transcript_version
            .set(self.transcript_version.get().wrapping_add(1));
    }

    /// Check current token usage and push token warning notifications as
    /// appropriate.  Call this after updating `token_count`.
    pub fn check_token_warnings(&mut self) {
        // (iter-1207) Use the SAME window the footer's context bar divides
        // by: the per-model value `refresh_context_window_size` resolved
        // from models_dev (200k for Anthropic, 1M for Gemini). This
        // function used to call the flat
        // `context_window_for_model(model)`, which ignores the model name
        // and always returns 128000 — so on Anthropic a context at 160k
        // tokens fired the "95% full" notice while the bar in the footer
        // read 80%. Two surfaces, two denominators, one lie about how
        // close the window is to full, and the warning that decides
        // whether auto-compact triggers is the one that was wrong.
        // Fall back to the model resolver only before the first refresh
        // has run (`context_window_size == 0`).
        let window = if self.context_window_size > 0 {
            self.context_window_size as u32
        } else {
            crate::tui::adapter_types::context_window_for_model(&self.model_name) as u32
        };
        if window == 0 {
            return;
        }
        // Compare like with like: the provider-metered prompt size when
        // there is one, else the turn total. The footer renders the prompt
        // size, so the percentage must be measured against that same
        // figure or the notice and the bar disagree.
        let used = self
            .last_observed_prompt_tokens
            .unwrap_or(u64::from(self.token_count));
        let pct = (used as f64 / window as f64 * 100.0) as u8;

        // Usage dropped back below the last-shown threshold (e.g. /clear or
        // /compact shrank the context) — reset so warnings can re-fire on
        // the way back up instead of being suppressed forever.
        if pct < self.token_warning_threshold_shown {
            self.token_warning_threshold_shown = 0;
        }

        // Only escalate — never repeat a threshold already shown.
        if pct >= 100 && self.token_warning_threshold_shown < 100 {
            self.token_warning_threshold_shown = 100;
            self.push_notice(
                NotificationKind::Error,
                "Context window full. Running auto-compact\u{2026}".to_string(),
            );
        } else if pct >= 95 && self.token_warning_threshold_shown < 95 {
            self.token_warning_threshold_shown = 95;
            self.push_notice(
                NotificationKind::Error,
                "Context window 95% full! Run /compact now.".to_string(),
            );
        } else if pct >= 80 && self.token_warning_threshold_shown < 80 {
            self.token_warning_threshold_shown = 80;
            self.push_notice(
                NotificationKind::Warning,
                "Context window 80% full. Consider /compact.".to_string(),
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
            self.push_notice(
                NotificationKind::Warning,
                format!(
                    "Image attachments aren't sent to the model yet — {} image(s) dropped.",
                    dropped.len()
                ),
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
                self.set_status_notice("Prompt restored to the composer — the turn failed.");
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
            // (2026-10-25 history-nav fix) A slash command is a command, not
            // a prompt: it must not enter the Up/Down scroll history (the
            // operator's report — 70% of the persisted history was `/help`-
            // style commands crowding out real prompts, and every recall of
            // one re-opened the completion popup and stranded browsing).
            // OMP keeps ordinary slash commands in history and filters only
            // secret-carrying ones (shouldSkipHistory); the operator's
            // directive here is stricter and wins: commands never enter.
            let is_command = crate::tui::input::is_slash_command(input.trim());
            if !is_command {
                self.prompt_input.history.push(input.clone());
                self.prompt_input.history_pos = None;
                self.prompt_input.history_draft.clear();
                // Persist the new entry to ~/.operant/history.jsonl so it
                // survives restarts. (iter-125 — persistent input history.)
                crate::tui::input_history::append(&input);
            }
            // (plan 2026-10-24 M5) The word predictor learns from every
            // submitted prompt — same data, one submission, one lesson.
            if let Some(engine) = &self.word_predictor {
                engine.observe(&input);
            }
        }
        self.refresh_prompt_input();
        input
    }
}
