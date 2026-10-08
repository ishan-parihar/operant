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

    /// Emit `AgentEvent::RetryScheduled` when the turn loop is about to
    /// re-issue the LLM request. One-line wrapper so each of the five retry
    /// sites in `run.rs` stays a single call.
    pub(crate) async fn emit_retry_scheduled(
        &self,
        attempt: usize,
        max_attempts: usize,
        reason: &str,
    ) {
        self.emit(AgentEvent::RetryScheduled {
            attempt,
            max_attempts,
            reason: reason.to_string(),
        })
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
    /// Clear the conversation for the **current session** and reset per-session
    /// state. Called on /new, /reset, and session switches.
    ///
    /// This wipes only the session the agent is currently addressing, not a
    /// global slot. A session that was already handed back to the store on a
    /// retarget keeps its own transcript, so two sessions can no longer wipe
    /// each other by sharing one agent.
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
        drop(conv);
        // Drop the wiped session from the store too. Without this the store
        // would still hold the pre-wipe transcript and the very next acquire
        // would rehydrate the history /new was called to discard.
        if let Some(id) = self.session_id() {
            let key = crate::session::SessionKey::new(id);
            self.sessions.discard(&key);
        }
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
            let old_id = self.session_id().unwrap_or_default();
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
    /// Takes &self (not &mut self) so it works through `Arc<OperantAgent>`.
    /// Uses `Arc<RwLock<String>>` for the model override, checked at each
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

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// The session every persistence site in this agent writes to, or
    /// `None` when no host has assigned one.
    ///
    /// This is the single reader for the agent's session identity: the
    /// turn prologue, evolution-metadata hydration, tool context,
    /// compression persistence and the memory provider all resolve
    /// through it, so one turn can never write its trajectory to a
    /// different id than the one a host will reload.
    pub fn session_id(&self) -> Option<String> {
        self.session_id
            .read()
            .expect("session_id RwLock poisoned — programmer error")
            .clone()
    }

    /// Wave 2 (ORGANISM-ARCHITECTURE §2): the cast employee this agent
    /// executes as. The genome consult resolves through this FIRST — the
    /// seat a chat session runs under is its bound employee's id, not the
    /// `gw_<hash>` conversation id. `None` keeps today's behavior (consult
    /// on the session id, which is what cron runs already key on).
    #[expect(clippy::expect_used, reason = "RwLock poison recovery — a poisoned seat_id lock is a programmer error, not a runtime condition")]
    pub fn seat_id(&self) -> Option<String> {
        self.seat_id
            .read()
            .expect("seat_id RwLock poisoned — programmer error")
            .clone()
    }

    /// Assign the employee this agent executes as (the genome seat). Set by
    /// the gateway turn from the session's employee binding; mirrors
    /// [`Self::set_session_id`] through `Arc<OperantAgent>`.
    pub fn set_seat_id(&self, seat_id: impl Into<String>) {
        let new_id = seat_id.into();
        tracing::debug!(seat_id = %new_id, "Agent genome seat set at runtime");
        if let Ok(mut guard) = self.seat_id.write() {
            *guard = Some(new_id);
        }
    }

    /// The bound employee's charter (org system prompt), if the gateway set
    /// one. Appended to the frozen prefix — see [`Self::build_frozen_prefix`].
    #[expect(clippy::expect_used, reason = "RwLock poison recovery — a poisoned charter lock is a programmer error, not a runtime condition")]
    pub fn charter(&self) -> Option<String> {
        self.charter
            .read()
            .expect("charter RwLock poisoned — programmer error")
            .clone()
    }

    /// Set the employee charter the frozen prefix carries. Set with
    /// [`Self::set_seat_id`] so the pair — seat + charter — changes atomically
    /// enough for prompt-cache purposes (both flip only when the bound
    /// employee changes, which is rare per conversation).
    pub fn set_charter(&self, charter: Option<String>) {
        if let Ok(mut guard) = self.charter.write() {
            *guard = charter;
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Assign the session this agent persists into.
    ///
    /// Takes `&self` (not `&mut self`) so it works through
    /// `Arc<OperantAgent>`, mirroring [`Self::set_model`]. A host that
    /// owns session identity — the gateway, which derives a stable
    /// `gw_<hash>` id per chat and reloads the last 20 rows of THAT id
    /// on restart — calls this with its own id so the agent's full tool
    /// trajectory lands in the same namespace the host reloads, instead
    /// of a fresh `sess_<uuid>` per turn.
    pub fn set_session_id(&self, session_id: impl Into<String>) {
        let new_id = session_id.into();
        tracing::debug!(session_id = %new_id, "Agent session id set at runtime");
        let previous = self
            .session_id
            .read()
            .expect("session_id RwLock poisoned — programmer error")
            .clone();
        // A retarget must not carry one session's turns into the next. The
        // hot conversation slot follows the session id: the outgoing session's
        // turns go back to the store (which keeps them durable and warm), and
        // the incoming session's turns are rehydrated in their place. Without
        // this swap, retargeting the agent leaks the previous conversation
        // into the new session — the D2 defect.
        if previous.as_deref() != Some(new_id.as_str()) {
            // Load the INCOMING session's transcript. This must key off
            // `new_id` explicitly, not `self.session_id()` — at this point the
            // id field still holds the outgoing value, so reading it here
            // would rehydrate the session we are leaving.
            let incoming = self.transcript_for(&new_id);
            let outgoing = std::mem::replace(
                &mut *self
                    .conversation
                    .try_write()
                    .expect("conversation RwLock poisoned — programmer error"),
                incoming,
            );
            if let Some(prev) = previous {
                self.sessions
                    .persist(&crate::session::SessionKey::new(prev), &outgoing);
            }
            // Reset the LLM compressor alongside the conversation swap. Its
            // summary state is per-session, so carrying it across a retarget
            // would let one session's summary bleed into the next session's
            // compression context. This used to live only in `clear_history`,
            // which meant a host that retargeted via `set_session_id` silently
            // kept the old summary — the cron path now uses exactly that, so the
            // reset has to follow the id change rather than the clear.
            self.reset_compressor_for_retarget();

            // `clear_history` keeps its own reset: it can fire on an UNCHANGED
            // session id, which this guard deliberately does not cover.
        }
        *self
            .session_id
            .write()
            .expect("session_id RwLock poisoned — programmer error") = Some(new_id);
    }

    /// Drop the LLM compressor's summary state on a session retarget.
    ///
    /// A non-blocking companion to `clear_history`'s reset, for the retarget
    /// path in `set_session_id`. That path is a **synchronous** `pub fn`
    /// (it takes `&self` so hosts can call it through `Arc<OperantAgent>`), so
    /// it cannot `await` the compressor mutex. Making it async is not an option:
    /// `set_session_id` is called from async contexts that already hold session
    /// locks, and an awaited mutex there risks blocking the scheduler.
    ///
    /// So this uses `try_lock`, the same shape `notify_memory_write` uses for
    /// the same reason (`events.rs:247`). On contention the reset is DROPPED,
    /// which is the safe direction: a stale summary degrades compression quality
    /// for one session, whereas waiting would stall every cron tick behind
    /// whatever compression is in flight.
    fn reset_compressor_for_retarget(&self) {
        let Some(compressor) = self.llm_compressor.as_ref() else {
            return;
        };
        match compressor.try_lock() {
            Ok(mut guard) => guard.reset(),
            Err(_) => {
                debug!("llm_compressor lock contended on session retarget — reset dropped");
            }
        }
    }

    /// Rehydrate `session_id`'s transcript from disk.
    ///
    /// Takes the id as an argument rather than reading it from the agent: the
    /// only caller is [`Self::set_session_id`]'s swap, and at that moment the
    /// agent's id field still names the session being left.
    fn transcript_for(&self, session_id: &str) -> Vec<Message> {
        let key = crate::session::SessionKey::new(session_id);
        self.sessions.acquire(&key);
        self.sessions.peek(&key)
    }

    /// The durable per-session store backing this agent's transcripts.
    pub fn session_store(&self) -> &std::sync::Arc<crate::session::SessionStore> {
        &self.sessions
    }

    /// Cache statistics for the session substrate.
    pub fn session_stats(&self) -> crate::session::SessionStoreStats {
        self.sessions.stats()
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
        // Wave 2: the bound employee's charter — the role definition the
        // org layer gives the seat this agent runs as. Appended AFTER the
        // base prompt, BEFORE skills, and byte-stable across turns (it only
        // changes when the bound employee changes), so the frozen-prefix
        // cache discipline holds.
        if let Some(charter) = self.charter() {
            if !charter.is_empty() {
                frozen.push_str("\n\n<employee_charter>\n");
                frozen.push_str(&charter);
                frozen.push_str("\n</employee_charter>");
            }
        }
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
        // C1 — kernel-evolved prompt sections. Appended to the frozen
        // prefix (not to the per-turn system message) so provider
        // installs change the same bytes the review fork inherits, and a
        // provider swap invalidates the cache exactly like a skill list
        // change does.
        if let Some(slot) = &self.harness_prompt_slot {
            let sections = slot.render();
            if !sections.trim().is_empty() {
                frozen.push_str("\n\n");
                frozen.push_str(&sections);
            }
        }
        frozen
    }
}
