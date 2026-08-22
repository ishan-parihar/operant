//! `events` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::Message;
use crate::database::Database;
use tracing::{debug, warn};

use super::*;

impl OperantAgent {
    /// Send an event to the channel
    pub(crate) async fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event).await;
        }
    }

    /// Loop-level per-request ceiling — the `request_timeout` config wired as
    /// the run loop's own budget (hermes `request_timeout_secs` parity, the
    /// audit's dead-field fix). Raised to the R2 reasoning stale-timeout floor
    /// for known reasoning models so a long-thinking model is never killed by
    /// the loop ceiling — the floor is a FLOOR, applied as `max(configured,
    /// floor)` exactly like the client's `effective_timeout`.
    pub(crate) fn loop_request_timeout(&self) -> std::time::Duration {
        let configured = self.config.request_timeout;
        match crate::reasoning_timeouts::get_reasoning_stale_timeout_floor(&self.model()) {
            Some(floor) => configured.max(std::time::Duration::from_secs(floor)),
            None => configured,
        }
    }

    /// Run a model call under the loop-level request budget. On expiry, the
    /// future is dropped and a retryable `Agent` error is produced (its
    /// "timed out" text also feeds the R2 thinking-timeout detection for
    /// reasoning models).
    pub(crate) async fn call_with_loop_timeout<F, T>(&self, fut: F) -> crate::error::Result<T>
    where
        F: std::future::Future<Output = crate::error::Result<T>>,
    {
        let budget = self.loop_request_timeout();
        // T2: race the request against BOTH the budget ceiling and the
        // interrupt flag so a Ctrl-C on the one-shot path aborts the
        // in-flight request instead of waiting for it (or the timeout) to
        // complete. The interrupt branch returns an `Interrupted`-style
        // error; the loop's error handlers bail out on the flag before
        // classifying, so it never enters the retry/rotate path.
        let interrupt_flag = self.interrupt_flag.clone();
        let interrupt_fut = async move {
            loop {
                if interrupt_flag.is_triggered() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            crate::error::Error::Agent(
                "Interrupted by user — in-flight LLM request aborted".to_string(),
            )
        };
        tokio::select! {
            result = fut => result,
            _ = tokio::time::sleep(budget) => {
                warn!(budget = ?budget, "LLM request exceeded loop request_timeout ceiling");
                Err(crate::error::Error::Agent(format!(
                    "request timed out after {budget:?} (loop request_timeout ceiling)"
                )))
            }
            interrupted = interrupt_fut => {
                warn!("Interrupt flag triggered — aborting in-flight LLM request");
                Err(interrupted)
            }
        }
    }

    /// R2: append thinking-timeout guidance to a final (post-retry) error when
    /// the failure is a transport error on a known reasoning model with no
    /// content arrived (upstream idle-killed the thinking phase). Only fires
    /// after the retry budget is exhausted — the raw error flows through the
    /// retry loop unannotated so classification is unaffected.
    pub(crate) fn annotate_thinking_timeout(
        &self,
        err: crate::error::Error,
    ) -> crate::error::Error {
        // Streaming path: the flag is set by process_stream when the failure
        // happened with no content arrived. Non-streaming path: detect the
        // transport error on a known reasoning model directly.
        let hit = self
            .thinking_timeout_hit
            .load(std::sync::atomic::Ordering::Relaxed)
            || crate::reasoning_timeouts::is_thinking_timeout(&self.model(), &err.to_string());
        if hit {
            let guidance =
                crate::reasoning_timeouts::build_thinking_timeout_guidance(&self.model());
            warn!(
                error = %err,
                "Thinking-timeout detected on reasoning model — appending guidance"
            );
            crate::error::Error::Agent(format!("{err}{guidance}"))
        } else {
            err
        }
    }

    /// T3: emit a `RateLimitNotice` AgentEvent when the classified failure is
    /// a rate limit (429), surfacing the Retry-After (when known) so the
    /// CLI/TUI can show "limit reached, retry in Ns" (hermes
    /// `_capture_rate_limits` parity). No-op for other failure classes.
    pub(crate) async fn emit_rate_limit_notice(
        &self,
        classified: &ClassifiedError,
        err: &crate::error::Error,
    ) {
        use crate::agent::error_classifier::FailoverReason;
        if !matches!(
            classified.reason,
            FailoverReason::RateLimit | FailoverReason::UpstreamRateLimit
        ) {
            return;
        }
        let retry_after_secs = match err {
            crate::error::Error::RateLimited { retry_after } => Some(retry_after.as_secs()),
            crate::error::Error::Provider {
                retry_after: Some(d),
                ..
            } => Some(d.as_secs()),
            _ => None,
        };
        self.emit(AgentEvent::RateLimitNotice { retry_after_secs })
            .await;
    }

    /// Add a message to the conversation history
    pub async fn add_message(&self, message: Message) {
        let mut conv = self.conversation.write().await;
        conv.push(message);
    }

    /// Add a user message
    pub async fn user_message(&self, content: impl Into<String>) {
        self.add_message(Message::user(content)).await;
    }

    /// Get current conversation
    pub async fn conversation(&self) -> Vec<Message> {
        self.conversation.read().await.clone()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Clear conversation history and reset per-session state.
    /// Called on /new, /reset, and session switches.
    pub async fn clear_history(&self) {
        // Notify memory provider of session end before clearing.
        // This fires at actual session boundaries so the graph captures
        // session-level patterns. Ported from hermes-agent's
        // MemoryManager.on_session_end() pattern.
        //
        // Clone the snapshot under the read lock, then drop it before
        // acquiring the write lock — prevents TOCTOU race where another
        // task modifies the conversation between read() and write().
        let snapshot = {
            let conv = self.conversation.read().await;
            conv.clone()
        };
        // Route through the executor when available for FIFO ordering.
        {
            let exec_guard = self
                .memory_sync_executor
                .lock()
                .expect("memory_sync_executor mutex poisoned — programmer error");
            if let Some(executor) = exec_guard.as_ref() {
                executor.submit_session_end(&snapshot);
            } else if let Some(provider) = &self.memory_provider {
                provider.on_session_end(&snapshot);
            }
        }
        let mut conv = self.conversation.write().await;
        conv.clear();
        // Reset LLM compressor state so the next session starts fresh.
        // Without this, a previous session's summary would bleed into
        // the new session's compression context.
        if let Some(ref compressor) = self.llm_compressor {
            compressor.lock().await.reset();
        }
        // Notify memory provider of session switch (reset=true).
        // This fires on /new, /reset, and session switches so the
        // graph knows the session boundary. Ported from hermes-agent's
        // MemoryManager.on_session_switch() pattern.
        // Use the existing public method for consistency.
        if let Some(provider) = &self.memory_provider {
            let old_id = self.persistent_session_id.clone().unwrap_or_default();
            provider.on_session_switch(&old_id, &old_id, true);
        }
    }

    /// Notify the memory provider that the session_id has rotated.
    /// Ported from hermes-agent's MemoryManager.on_session_switch().
    /// Fires on /resume, /branch, /reset, /new, and context compression.
    pub fn notify_session_switch(
        &self,
        new_session_id: &str,
        parent_session_id: &str,
        reset: bool,
    ) {
        if let Some(provider) = &self.memory_provider {
            provider.on_session_switch(new_session_id, parent_session_id, reset);
        }
    }

    /// Notify the memory provider of a built-in memory write.
    /// Mirrors the write to the memory backend so it stays in sync with
    /// MEMORY.md / USER.md changes. Uses the background executor
    /// when available to avoid blocking the agent loop.
    pub fn notify_memory_write(&self, action: &str, target: &str, content: &str) {
        // Use try_lock() to avoid blocking — if the mutex is held by shutdown,
        // just drop the write silently.
        if let Ok(exec_guard) = self.memory_sync_executor.try_lock() {
            if let Some(executor) = exec_guard.as_ref() {
                executor.submit_memory_write(action, target, content);
            } else if let Some(provider) = &self.memory_provider {
                provider.on_memory_write(action, target, content);
            }
        } else {
            debug!("memory_sync_executor lock contended — memory_write notification dropped");
        }
    }

    /// Notify the memory provider of a delegation result.
    /// The parent's memory provider gets the task+result pair as an
    /// observation of what was delegated and what came back.
    /// Uses the background executor when available.
    pub fn notify_delegation(&self, task: &str, result: &str) {
        if let Ok(exec_guard) = self.memory_sync_executor.try_lock() {
            if let Some(executor) = exec_guard.as_ref() {
                executor.submit_delegation(task, result);
            } else if let Some(provider) = &self.memory_provider {
                provider.on_delegation(task, result);
            }
        } else {
            debug!("memory_sync_executor lock contended — delegation notification dropped");
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Gracefully shut down the memory sync executor.
    /// Drains pending jobs (up to 5s) then abandons remaining work.
    /// Call this during agent shutdown to avoid losing in-flight writes.
    /// Takes `&self` (not `&mut self`) so it works through `Arc<OperantAgent>`.
    pub async fn shutdown_memory_executor(&self) {
        let executor = self
            .memory_sync_executor
            .lock()
            .expect("memory_sync_executor mutex poisoned — programmer error")
            .take();
        if let Some(executor) = executor {
            executor.shutdown().await;
        }
    }

    /// Get a reference to the database
    pub fn db(&self) -> &Database {
        &self.database
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Update the model at runtime. Used by the gateway to apply
    /// per-session model overrides via /model command. (iter-162 —
    /// closes ponytail-audit gap B36: 'model_override is read but
    /// never applied — the agent's config.model is private.')
    ///
    /// Takes &self (not &mut self) so it works through Arc<OperantAgent>.
    /// Uses Arc<RwLock<String>> for the model override, checked at each
    /// run() call.
    pub fn set_model(&self, model: impl Into<String>) {
        let new_model = model.into();
        tracing::info!(model = %new_model, "Agent model override set at runtime");
        *self
            .model_override
            .write()
            .expect("model_override RwLock poisoned — programmer error") = Some(new_model);
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get the current model name (effective model = override or config).
    pub fn model(&self) -> String {
        self.model_override
            .read()
            .expect("model_override RwLock poisoned — programmer error")
            .as_ref()
            .map(|m| m.clone())
            .unwrap_or_else(|| self.config.model.clone())
    }

    /// Get the effective model for API calls. Checks override first.
    pub(crate) fn effective_model(&self) -> String {
        self.model()
    }

    /// Build the frozen prefix (base system prompt + skills).
    ///
    /// This is the byte-stable portion of the system prompt that rarely
    /// changes across turns. Keeping it identical between the parent agent
    /// and the background review fork enables prompt cache hits on
    /// Anthropic/OpenRouter (cache reads cost ~10x less than fresh tokens).
    ///
    /// Extracted from `build_messages()` to share with `spawn_background_review`.
    pub(crate) fn build_frozen_prefix(&self) -> String {
        let mut frozen = self.config.system_prompt.clone().unwrap_or_else(|| {
            "You are Operant, a helpful AI assistant. You have access to tools that you can use to help users. \
                Use the provided tools when needed to accomplish tasks. \
                After receiving tool results, continue reasoning and either call more tools or provide your final response to the user."
                .to_string()
        });
        if let Some(skill_manager) = &self.skill_manager {
            let skills = skill_manager.list();
            if !skills.is_empty() {
                frozen.push_str("\n\n<available_skills>\n");
                for (name, description) in &skills {
                    frozen.push_str(&format!(
                        "  <skill name=\"{}\">{}</skill>\n",
                        name, description
                    ));
                }
                frozen.push_str("</available_skills>");
            }
            // hermes parity: skill-management principles ride the same
            // frozen prefix so the background-review fork inherits them
            // for free (byte-stable => prompt cache hits preserved).
            frozen.push_str(SKILLS_GUIDANCE);
        }
        frozen
    }
}
