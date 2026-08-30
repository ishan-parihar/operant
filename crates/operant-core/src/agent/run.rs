//! `run` — method-group impl block extracted verbatim from agent/mod.rs.

use self::turn_finalizer::{
    PREFLIGHT_DECAY_CONSTANT, PREFLIGHT_DECAY_H50, PREFLIGHT_THRESHOLD_PERCENT, TurnDiagnostics,
    TurnExitReason, file_mutation_verifier_footer,
};
use self::turn_rules::{AssistantTurn, EmptyResponseCounter};
use crate::client::{Message, Role};
use crate::error::{Error, Result};
use crate::observer::{Observer, ObserverEvent, ObserverMetric};
use crate::turn_end_heuristics;
use std::time::Duration;
use tracing::{debug, error, info, instrument, warn};

use super::*;

impl OperantAgent {
    /// Attempt a "grace call" — a toolless summary request to the model.
    ///
    /// Called when the iteration budget or max_iterations is exhausted.
    /// The model gets one final chance to summarize its progress without
    /// tools, giving the user a partial answer instead of a hard error.
    ///
    /// Returns `Ok(Message)` on success, or `Err(MaxIterationsExceeded)`
    /// if the grace call also fails.
    pub(crate) async fn attempt_grace_call(
        &self,
        messages: &[Message],
        session_id: &str,
        iterations: usize,
        tool_calls: usize,
        final_response: Option<&Message>,
    ) -> Result<Message> {
        let grace_request = ChatRequest::new(self.effective_model(), messages.to_vec())
            .with_stream(self.config.stream);

        let grace_result = if self.config.stream {
            let stream = self.client.chat_streaming(grace_request).await?;
            let (text, reasoning, _tcs, _extra, _finish_reason) =
                self.process_stream(stream).await?;
            Ok((text, reasoning))
        } else {
            let response = self.client.chat(grace_request).await?;
            self.process_response(response)
                .await
                .map(|(t, r, _, _)| (t, r))
        };

        match grace_result {
            Ok((text, _reasoning)) => {
                let result = Message::assistant(&text);
                if self.record_trajectories {
                    self.save_trajectory(
                        session_id,
                        messages,
                        iterations,
                        tool_calls,
                        false,
                        final_response,
                    )
                    .await;
                }
                // ── Eager LCM ingest (budget-exhausted path) ──────────────
                // Mirrors the TextResponse exit: commit the turn (history +
                // grace response) so the DAG is up to date before returning.
                if let Some(engine) = &self.context_engine {
                    let mut turn = messages.to_vec();
                    turn.push(result.clone());
                    if let Err(e) = engine.ingest_turn(session_id, &turn).await {
                        tracing::warn!(
                            error = %e,
                            "LCM eager turn ingest failed (non-fatal)"
                        );
                    }
                }
                self.emit(AgentEvent::Done {
                    message: result.clone(),
                })
                .await;
                if let Some(ref obs) = self.observer {
                    let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                    obs.record_event(&ObserverEvent::AgentEnd {
                        provider: self.config.model.clone(),
                        model: self.model(),
                        duration: std::time::Duration::from_secs(0),
                        tokens_used: None,
                        cost_usd: if cost > 0.0 { Some(cost) } else { None },
                    });
                }
                if let Some(ref hooks) = self.hook_registry {
                    hooks
                        .emit(
                            crate::gateway_pipeline::HookEvent::AgentEnd,
                            crate::gateway_pipeline::HookContext::new().with_session(session_id),
                        )
                        .await;
                }
                Ok(result)
            }
            Err(e) => {
                warn!(error = %e, "Grace call failed — returning hard error");
                // Emit AgentEnd observer event for failure
                if let Some(ref obs) = self.observer {
                    let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                    obs.record_event(&ObserverEvent::AgentEnd {
                        provider: self.config.model.clone(),
                        model: self.model(),
                        duration: std::time::Duration::from_secs(0),
                        tokens_used: None,
                        cost_usd: if cost > 0.0 { Some(cost) } else { None },
                    });
                }
                if let Some(ref hooks) = self.hook_registry {
                    hooks
                        .emit(
                            crate::gateway_pipeline::HookEvent::AgentEnd,
                            crate::gateway_pipeline::HookContext::new().with_session(session_id),
                        )
                        .await;
                }
                if self.record_trajectories {
                    self.save_trajectory(session_id, messages, iterations, tool_calls, false, None)
                        .await;
                }
                Err(Error::MaxIterationsExceeded {
                    max: self.config.max_iterations,
                })
            }
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Run the agent with a user query
    #[instrument(skip(self), fields(model = % self.config.model))]
    pub async fn run(&self, user_query: String) -> Result<Message> {
        info!("Starting agent run");

        // Emit AgentStart observer event
        if let Some(ref obs) = self.observer {
            obs.record_event(&ObserverEvent::AgentStart {
                provider: self.config.model.clone(),
                model: self.model(),
            });
        }

        // Emit AgentStart hook
        if let Some(ref hooks) = self.hook_registry {
            let ctx = crate::gateway_pipeline::HookContext::new()
                .with_session(self.persistent_session_id.as_deref().unwrap_or(""));
            hooks
                .emit(crate::gateway_pipeline::HookEvent::AgentStart, ctx)
                .await;
        }

        // ── Phase 3: Turn Context Prologue ──────────────────────────────
        // Extract per-turn setup into a structured, testable module.
        // Handles: interrupt flag reset, session ID resolution, evolution
        // state hydration, user message dedup, DB persistence, message
        // building. Matches hermes-agent's build_turn_context() pattern.
        let turn_ctx = turn_context::build_turn_context(self, &user_query).await?;
        let session_id = turn_ctx.session_id;

        // ── Tool-call guardrail reset (R4) ────────────────────────────
        // Identical-call repeat detection is per-USER-TURN, not per-iteration:
        // the model may legitimately call the same tool across iterations of
        // one task, but a retry storm repeats the exact same call within a
        // few iterations. Reset at the start of each run().
        self.tool_guardrails
            .lock()
            .expect("tool_guardrails lock poisoned")
            .reset();

        // ── Iteration budget reset (hermes parity) ─────────────────────
        // Each user turn gets a fresh per-turn budget.  Without this,
        // gateway turns that share one agent instance see the budget
        // permanently exhausted after the first turn exhausts it, causing
        // every subsequent turn to short-circuit to a grace call.
        self.iteration_budget.reset();

        // Clear streaming ToolStart dedup set — each turn starts fresh.
        if let Ok(mut set) = self.stream_emitted_tool_starts.lock() {
            set.clear();
        }

        let mut messages = turn_ctx.messages;

        // ── TurnStart lifecycle hook ─────────────────────────────────────
        // Emit TurnStart so external code (e.g., prefetch queues,
        // telemetry, skill scaffolding) can react to per-turn events.
        if let Some(ref hooks) = self.hook_registry {
            let ctx = crate::gateway_pipeline::HookContext::new()
                .with_session(&session_id)
                .with_metadata("user_query", &user_query);
            hooks
                .emit(crate::gateway_pipeline::HookEvent::TurnStart, ctx)
                .await;
        }
        let mut iteration = 0;
        let mut total_tool_calls: usize = 0;
        // Turn-level wall clock for the R5 accounting line (the observer's
        // AgentEnd duration; the per-iteration `llm_start` only covers the
        // model call).
        let turn_start = std::time::Instant::now();

        // ── Memory provider: on_turn_start ──────────────────────────────
        // Notify the memory provider of the new turn so it can do per-turn
        // bookkeeping (turn counting, scope management, periodic maintenance).
        if let Some(provider) = &self.memory_provider {
            provider.on_turn_start(iteration + 1, &user_query);
        }

        // ── Self-evolution: memory review (per-turn cadence) ────────────
        // Bump the memory turn counter once per user turn and check whether
        // a background memory review should fire. Mirrors hermes-agent's
        // turn_context.py which bumps `_turns_since_memory` once per turn
        // (NOT per iteration) and gates on the memory tool being available
        // plus a memory provider being present (`"memory" in
        // valid_tool_names and agent._memory_store` in hermes) — so we never
        // spawn a review when nothing can persist it.
        //
        // Scope the MutexGuard so it's dropped before the .await below.
        let should_review_memory = {
            let memory_tool_active = !self
                .registry
                .get_available_schemas_filtered(&[
                    "memory_store".to_string(),
                    "memory_search".to_string(),
                    "memory_recall".to_string(),
                ])
                .await
                .is_empty();
            let memory_provider_present = self.memory_provider.is_some();
            let memory_active = memory_tool_active && memory_provider_present;

            if !memory_active {
                false
            } else {
                let mut evo = self
                    .evolution_state
                    .lock()
                    .expect("evolution_state mutex poisoned — programmer error");
                let trigger = turn_finalizer::advance_memory_trigger(&mut evo);
                if trigger.should_review_memory {
                    info!(
                        turns = trigger.turns_since_memory,
                        interval = self.config.memory_review_interval,
                        "Memory review triggered — spawning background review"
                    );
                }
                // Persist evolution counters so the next run() can hydrate.
                if self.persistent_session_id.is_some() {
                    for (key, val) in evo.persist_counters() {
                        let _ = self.database.set_session_metadata(&session_id, key, &val);
                    }
                }
                trigger.should_review_memory
            }
        }; // MutexGuard dropped here — safe to .await
        if should_review_memory {
            self.spawn_background_review(&messages, &session_id, false, true)
                .await;
        }

        let mut retry_state = turn_retry_state::TurnRetryState::new(Some(self.config.max_retries));
        // Plan 006: empty-content retry counter is the shared
        // EmptyResponseCounter from agent/turn_rules.rs — same logic the
        // runtime Agent uses (no more silent divergence on max_retries).
        let mut empty_content_retries = EmptyResponseCounter::new(self.config.max_retries);
        // Truncation-continuation retries (T1 — hermes caps at 4).
        let mut length_continue_retries: usize = 0;
        // Whether we've already tried a fallback model after empty retries.
        let mut fallback_attempted = false;
        // Degenerate-loop circuit breaker: consecutive iterations whose tool
        // batch produced ONLY failures (guardrail skips, malformed args,
        // validation errors). Live incident: 4397 guardrail skips in one turn
        // as the model re-emitted identical `process` calls for 21 minutes.
        let mut consecutive_failed_iters: usize = 0;
        // Cross-iteration repetition guard (R35): consecutive iterations
        // whose executed calls are IDENTICAL (same name + args signature).
        // Live incident: 408 unrepairable `process` calls repaired to `{}`
        // succeeded one after another for 73 iterations — the all-failure
        // breaker above never fired because each call "succeeded".
        let (mut last_call_sig, mut identical_streak): (Option<u64>, usize) = (None, 0);

        // Reset provider registry to primary at turn start.
        // Matches hermes-agent's restore_primary_runtime() pattern —
        // ensures provider fallback is temporary, not permanent.
        if let Some(ref registry) = self.provider_registry {
            registry.reset_to_primary();
        }

        loop {
            // ── Iteration budget enforcement ────────────────────────────
            // Consume one iteration from the thread-safe budget counter before
            // starting the loop body. This matches hermes-agent's
            // IterationBudget.consume() pattern and provides a foundation for
            // future compression-refund support.
            if !self.iteration_budget.consume() {
                warn!(
                    budget_used = self.iteration_budget.used(),
                    budget_max = self.iteration_budget.max_total(),
                    "Iteration budget exhausted — attempting grace call"
                );
                return self
                    .attempt_grace_call(&messages, &session_id, iteration, total_tool_calls, None)
                    .await;
            }

            iteration += 1;
            debug!(iteration, "Agent iteration");

            // ── Graceful interrupt check (Ctrl-C) ──
            // If the interrupt flag has been triggered (e.g. by a Ctrl-C
            // signal handler in the TUI/CLI), exit the loop cleanly instead
            // of starting another LLM round-trip + tool execution cycle.
            if self.interrupt_flag.is_triggered() {
                // ── Turn diagnostics (interrupt exit) ────────────────────
                let diag = TurnDiagnostics {
                    exit_reason: TurnExitReason::Interrupted,
                    model: self.model(),
                    api_calls: iteration,
                    max_iterations: self.config.max_iterations,
                    budget_used: self.iteration_budget.used(),
                    budget_max: self.iteration_budget.max_total(),
                    tool_turns: total_tool_calls,
                    response_len: 0,
                    session_id: session_id.clone(),
                };
                warn!("{}", diag.log_message());
                if self.record_trajectories {
                    self.save_trajectory(
                        &session_id,
                        &messages,
                        iteration,
                        total_tool_calls,
                        false,
                        None,
                    )
                    .await;
                }
                self.emit(AgentEvent::Error {
                    error: "Interrupted by user".to_string(),
                })
                .await;
                let _ = message_safety::close_interrupted_tool_sequence(&mut messages, None);
                return Err(Error::Agent("Interrupted by user".to_string()));
            }

            if iteration > self.config.max_iterations {
                // ── Turn diagnostics (budget exhaustion) ─────────────────
                let diag = TurnDiagnostics {
                    exit_reason: TurnExitReason::BudgetExhausted,
                    model: self.model(),
                    api_calls: iteration,
                    max_iterations: self.config.max_iterations,
                    budget_used: self.iteration_budget.used(),
                    budget_max: self.iteration_budget.max_total(),
                    tool_turns: total_tool_calls,
                    response_len: 0,
                    session_id: session_id.clone(),
                };
                warn!("{}", diag.log_message());
                // ── Grace call (iter-57) ────────────────────────────────
                // When max_iterations is exceeded, hermes-agent makes one
                // extra "grace call" with tools stripped, asking the model
                // to summarize what it has so far. This gives the user a
                // partial answer instead of a hard error.
                return self
                    .attempt_grace_call(&messages, &session_id, iteration, total_tool_calls, None)
                    .await;
            }

            // Log iteration progress (not as a Thinking event — that pollutes
            // the TUI's thinking display with debug text. Use tracing instead.)
            // (iter-120 — user-reported bug: "Iteration 1/90: Requesting LLM
            // response..." was appearing in the thinking block.)
            tracing::debug!(
                iteration,
                max = self.config.max_iterations,
                "Requesting LLM response"
            );

            // Get tool schemas
            let tools = self
                .registry
                .get_schemas_for_request(&self.config.tool_search, self.config.context_window)
                .await;

            let request = ChatRequest::new(self.effective_model(), messages.clone())
                .with_tools(tools)
                .with_stream(self.config.stream);

            // Emit LlmRequest observer event
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::LlmRequest {
                    provider: self.config.model.clone(),
                    model: self.model(),
                    messages_count: messages.len(),
                });
            }

            let llm_start = std::time::Instant::now();
            let mut stream_extra_content = None;
            let response = if request.stream {
                // The run loop's own per-request budget (request_timeout,
                // raised to the R2 reasoning floor) — the client's transport
                // timeout is the wire-level guard, this is the loop ceiling.
                let mut stream = match self
                    .call_with_loop_timeout(self.client.chat_streaming(request))
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        // T2: bail out on interrupt instead of classifying.
                        if self.interrupt_flag.is_triggered() {
                            return Err(e);
                        }
                        // ── Context overflow auto-compression (iter-63) ───────
                        // When the provider returns a context_overflow error,
                        // compress the conversation using context_management
                        // and retry once. This prevents hard failures on long
                        // sessions that exceed the context window.
                        let classified = FallbackModelClient::classify_error(&e);
                        self.emit_rate_limit_notice(&classified, &e).await;
                        if classified.should_compress && !retry_state.compress_attempted {
                            retry_state.compress_attempted = true;
                            warn!(reason = %classified.reason, "Context overflow detected — compressing and retrying");
                            // Try LLM summarization first (intelligent compression),
                            // fall back to deterministic decay/eviction.
                            messages = self.compress_context_overflow(messages).await;
                            // Refund the iteration since the original LLM call
                            // was wasted on a context overflow — the retry gets
                            // a fresh consume() on the next loop iteration.
                            self.iteration_budget.refund();
                            retry_state.consume_retry();
                            // Rebuild request with compressed messages
                            let tools = self
                                .registry
                                .get_schemas_for_request(
                                    &self.config.tool_search,
                                    self.config.context_window,
                                )
                                .await;
                            let retry_request =
                                ChatRequest::new(self.effective_model(), messages.clone())
                                    .with_tools(tools)
                                    .with_stream(self.config.stream);
                            self.client.chat_streaming(retry_request).await?
                        } else if classified.should_rotate_credential
                            && !retry_state.rotate_attempted
                        {
                            // Credential rotation: invalidate current key,
                            // select next from pool, update client, and retry.
                            retry_state.rotate_attempted = true;
                            warn!(
                                reason = %classified.reason,
                                retry = retry_state.retry_count,
                                max = retry_state.max_retries,
                                "Auth/rate-limit error — rotating credential and retrying"
                            );
                            if self.try_rotate_credential().is_some() {
                                self.iteration_budget.refund();
                                retry_state.consume_retry();
                                let tools = self
                                    .registry
                                    .get_schemas_for_request(
                                        &self.config.tool_search,
                                        self.config.context_window,
                                    )
                                    .await;
                                let retry_request =
                                    ChatRequest::new(self.effective_model(), messages.clone())
                                        .with_tools(tools)
                                        .with_stream(self.config.stream);
                                self.client.chat_streaming(retry_request).await?
                            } else {
                                warn!("No more credentials to rotate — returning original error");
                                return Err(e);
                            }
                        } else {
                            return Err(e);
                        }
                    }
                };
                // ── Mid-stream drop recovery (hermes parity) ─────────────
                // Providers that close the SSE connection before the full
                // body arrives surface a transport error (reqwest's "error
                // decoding response body") from the stream. hermes-agent
                // explicitly retries these drops (_log_stream_retry /
                // _emit_stream_drop) instead of aborting the turn; we mirror
                // that here with the turn's existing retry budget.
                //
                // Rotate-classified mid-stream errors (429/401 chunks) are
                // retried too: the pooled client has already benched the
                // failed key via its stream wrapper, so the re-issued
                // request rotates to the next available key — rotation
                // fires on the retry (hermes wraps the whole stream
                // lifecycle with mark_exhausted_and_rotate, not just
                // connection establishment).
                let processed = loop {
                    // T2: the whole stream consumption runs under the loop
                    // budget ceiling (with the R2 reasoning floor) and the
                    // interrupt flag, so a Ctrl-C mid-stream aborts the
                    // request immediately instead of waiting for the turn
                    // to finish.
                    match self
                        .call_with_loop_timeout(self.process_stream(stream))
                        .await
                    {
                        Ok(processed) => break processed,
                        Err(e) => {
                            // T2: never classify/retry an interrupt-aborted
                            // request — propagate the interrupted error up.
                            if self.interrupt_flag.is_triggered() {
                                return Err(e);
                            }
                            let classified = FallbackModelClient::classify_error(&e);
                            let retryable = classified.retryable
                                && !classified.should_compress
                                && (classified.should_rotate_credential
                                    || !retry_state.rotate_attempted);
                            if retryable && retry_state.consume_retry() {
                                self.iteration_budget.refund();
                                // Aggregation hook: bump the shared retry
                                // counters so the TUI status pill can show
                                // stream-drop activity live. (The warn! below
                                // is the log side of the same event.)
                                self.metrics.record_stream_drop();
                                self.metrics.record_stream_retry();
                                warn!(
                                    error = %e,
                                    retry = retry_state.retry_count,
                                    max = retry_state.max_retries,
                                    "Stream dropped mid-read — re-issuing LLM request"
                                );
                                let tools = self
                                    .registry
                                    .get_schemas_for_request(
                                        &self.config.tool_search,
                                        self.config.context_window,
                                    )
                                    .await;
                                let retry_request =
                                    ChatRequest::new(self.effective_model(), messages.clone())
                                        .with_tools(tools)
                                        .with_stream(self.config.stream);
                                stream = self
                                    .call_with_loop_timeout(
                                        self.client.chat_streaming(retry_request),
                                    )
                                    .await?;
                            } else {
                                return Err(self.annotate_thinking_timeout(e));
                            }
                        }
                    }
                };
                let (text, reasoning, tcs, extra, finish_reason) = processed;
                stream_extra_content = extra;
                Ok((text, reasoning, tcs, finish_reason))
            } else {
                let response = match self.call_with_loop_timeout(self.client.chat(request)).await {
                    Ok(r) => r,
                    Err(e) => {
                        // T2: bail out on interrupt instead of classifying.
                        if self.interrupt_flag.is_triggered() {
                            return Err(e);
                        }
                        let classified = FallbackModelClient::classify_error(&e);
                        self.emit_rate_limit_notice(&classified, &e).await;
                        if classified.should_compress && !retry_state.compress_attempted {
                            retry_state.compress_attempted = true;
                            warn!(reason = %classified.reason, "Context overflow detected — compressing and retrying");
                            // Try LLM summarization first (intelligent compression),
                            // fall back to deterministic decay/eviction.
                            messages = self.compress_context_overflow(messages).await;
                            // Refund the iteration since the original LLM call
                            // was wasted on a context overflow.
                            self.iteration_budget.refund();
                            retry_state.consume_retry();
                            let tools = self
                                .registry
                                .get_schemas_for_request(
                                    &self.config.tool_search,
                                    self.config.context_window,
                                )
                                .await;
                            let retry_request =
                                ChatRequest::new(self.effective_model(), messages.clone())
                                    .with_tools(tools)
                                    .with_stream(self.config.stream);
                            self.call_with_loop_timeout(self.client.chat(retry_request))
                                .await?
                        } else if classified.should_rotate_credential
                            && !retry_state.rotate_attempted
                        {
                            // Credential rotation: same as streaming path.
                            retry_state.rotate_attempted = true;
                            warn!(
                                reason = %classified.reason,
                                retry = retry_state.retry_count,
                                max = retry_state.max_retries,
                                "Auth/rate-limit error — rotating credential and retrying"
                            );
                            if self.try_rotate_credential().is_some() {
                                self.iteration_budget.refund();
                                retry_state.consume_retry();
                                let tools = self
                                    .registry
                                    .get_schemas_for_request(
                                        &self.config.tool_search,
                                        self.config.context_window,
                                    )
                                    .await;
                                let retry_request =
                                    ChatRequest::new(self.effective_model(), messages.clone())
                                        .with_tools(tools)
                                        .with_stream(self.config.stream);
                                self.call_with_loop_timeout(self.client.chat(retry_request))
                                    .await?
                            } else {
                                warn!("No more credentials to rotate — returning original error");
                                return Err(self.annotate_thinking_timeout(e));
                            }
                        } else {
                            return Err(self.annotate_thinking_timeout(e));
                        }
                    }
                };
                self.process_response(response).await
            };

            // Emit LlmResponse observer event with timing
            let llm_duration = llm_start.elapsed();
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::LlmResponse {
                    provider: self.config.model.clone(),
                    model: self.model(),
                    duration: llm_duration,
                    success: response.is_ok(),
                    error_message: response.as_ref().err().map(|e| e.to_string()),
                    input_tokens: None,
                    output_tokens: None,
                });
                obs.record_metric(&ObserverMetric::RequestLatency(llm_duration));
            }

            // Collect tool names before the match so they're accessible
            // in the self-evolution check after the match block.
            #[allow(unused_assignments)]
            let mut tool_names: Vec<String> = Vec::new();

            match response {
                Ok((response_text, reasoning_text, tool_calls, finish_reason)) => {
                    // Reset retry state on successful LLM response.
                    retry_state.reset_on_success();

                    // ── Truncation continuation (T1 — hermes parity) ──────
                    // When the provider reports a cut-off response
                    // (finish_reason="length", or a suspicious stop on
                    // Ollama-GLM), don't surface the partial answer as
                    // final: append a continuation prompt and re-loop,
                    // bounded by MAX_LENGTH_CONTINUE_RETRIES (hermes uses
                    // the same cap) and the iteration budget.
                    if tool_calls.is_empty()
                        && length_continue_retries < MAX_LENGTH_CONTINUE_RETRIES
                    {
                        let truncated = finish_reason.as_deref() == Some("length")
                            || turn_end_heuristics::should_treat_stop_as_truncated(
                                &self.config.model,
                                finish_reason.as_deref(),
                                &response_text,
                                messages.iter().any(|m| m.role == Role::Tool),
                                false,
                            );
                        if truncated {
                            // Thinking-exhausted: the model burned the whole
                            // output budget on reasoning with nothing visible
                            // left — continuation retries are pointless, give
                            // a targeted error (hermes conversation_loop.py
                            // thinking-exhausted detection).
                            if turn_end_heuristics::thinking_exhausted(&response_text) {
                                return Err(Error::Agent(
                                    "Model used all output tokens on reasoning with none left \
                                     for the response. Try lowering reasoning effort or \
                                     increasing max_tokens."
                                        .to_string(),
                                ));
                            }
                            length_continue_retries += 1;
                            self.metrics.record_truncation_continuation();
                            warn!(
                                finish_reason = ?finish_reason,
                                "Response truncated — requesting continuation ({}/{})",
                                length_continue_retries,
                                MAX_LENGTH_CONTINUE_RETRIES
                            );
                            self.emit(AgentEvent::Content {
                                text: format!(
                                    "↻ Response truncated — requesting continuation ({}/{})",
                                    length_continue_retries, MAX_LENGTH_CONTINUE_RETRIES
                                ),
                            })
                            .await;
                            let continue_msg =
                                Message::user(turn_end_heuristics::continuation_prompt());
                            messages.push(continue_msg.clone());
                            self.add_message(continue_msg).await;
                            // Refund the consumed iteration — the LLM call
                            // was wasted on a truncated turn; the continuation
                            // is the same logical turn.
                            self.iteration_budget.refund();
                            continue;
                        }
                    }

                    // ── Empty-content recovery (hermes parity) ─────────────
                    // If the model produced no visible text, no reasoning, and
                    // no tool calls, it has emitted an empty turn (free-tier
                    // providers do this intermittently). Rather than surfacing
                    // an empty reply as the final answer, retry up to
                    // max_retries times with the empty assistant turn appended
                    // to the conversation, exactly like hermes-agent's
                    // conversation_loop.py empty-retry loop and
                    // hermes-agent-ultra's methods_run_stream.rs inner_empty
                    let has_visible_text = !response_text.trim().is_empty();
                    let has_reasoning = !reasoning_text.trim().is_empty();
                    // Plan 006: route the empty-response decision through
                    // the shared rule so core and runtime stay in lockstep.
                    let turn = AssistantTurn {
                        final_text: &response_text,
                        reasoning: if has_reasoning {
                            Some(reasoning_text.as_str())
                        } else {
                            None
                        },
                        has_tool_calls: !tool_calls.is_empty(),
                    };
                    if empty_content_retries.should_retry(turn) {
                        self.metrics.record_empty_content_retry();
                        warn!(
                            "Empty assistant response — retrying ({}/{})",
                            empty_content_retries.count, empty_content_retries.max
                        );
                        // Log the retry but do NOT emit as a visible Content
                        // event — these retry messages should not clutter the
                        // user's Telegram chat. They are operational signals,
                        // not assistant output.
                        // Append the empty assistant turn so the model sees its
                        // own empty reply and is nudged to actually respond.
                        messages.push(Message::assistant(""));
                        self.add_message(Message::assistant("")).await;
                        // Refund the consumed iteration — the LLM call was
                        // wasted on an empty turn.
                        self.iteration_budget.refund();
                        continue;
                    }
                    // After retries exhausted on empty content, try fallback
                    // models before giving up (hermes fallback_on_errors parity).
                    if tool_calls.is_empty()
                        && !has_visible_text
                        && !has_reasoning
                        && !self.config.fallback_models.is_empty()
                        && self.config.fallback_on_errors
                        && !fallback_attempted
                    {
                        fallback_attempted = true;
                        let next_model = &self.config.fallback_models[0];
                        warn!(
                            "Empty response after {} retries — switching to fallback model: {}",
                            self.config.max_retries, next_model
                        );
                        self.set_model(next_model.clone());
                        self.iteration_budget.refund();
                        continue;
                    }
                    // Reasoning-only turn (hermes _CODEX_INCOMPLETE_NUDGE
                    // parity): the model produced reasoning but no visible
                    // answer and no tool calls — reasoning models on loaded
                    // free-tier endpoints stop after thinking. Ending the turn
                    // here hands the user an empty reply; nudge instead,
                    // bounded by the same retry budget.
                    // Plan 006: this case is *not* an empty turn (reasoning
                    // is present and non-blank) — the shared `should_retry`
                    // returns false for it. We handle the nudge manually
                    // here, but still bound the retries via the same
                    // counter so the budget is shared.
                    if tool_calls.is_empty()
                        && !has_visible_text
                        && has_reasoning
                        && empty_content_retries.count < empty_content_retries.max
                    {
                        empty_content_retries.count += 1;
                        self.metrics.record_empty_content_retry();
                        warn!(
                            "Reasoning-only response — nudging for a visible answer ({}/{})",
                            empty_content_retries.count, empty_content_retries.max
                        );
                        // Persist the reasoning-only assistant turn so the
                        // conversation stays coherent, then ask for the answer.
                        messages
                            .push(Message::assistant("").with_reasoning(reasoning_text.clone()));
                        self.add_message(
                            Message::assistant("").with_reasoning(reasoning_text.clone()),
                        )
                        .await;
                        let nudge = Message::user(
                            "[System: Your previous response contained only internal reasoning \
                             and never produced a visible answer or tool call. Do not keep \
                             thinking. Produce your final answer as plain text now.]",
                        );
                        messages.push(nudge.clone());
                        self.add_message(nudge).await;
                        self.iteration_budget.refund();
                        continue;
                    }
                    // Add assistant message to conversation
                    // When tool calls are present, any text before them is typically
                    // model thinking/planning that shouldn't be shown to the user.
                    let effective_text = if !tool_calls.is_empty() {
                        String::new()
                    } else {
                        response_text.clone()
                    };
                    let mut assistant_msg = Message::assistant(&effective_text);
                    if !reasoning_text.is_empty() {
                        assistant_msg = assistant_msg.with_reasoning(reasoning_text);
                    }
                    if !tool_calls.is_empty() {
                        assistant_msg = assistant_msg.with_tool_calls(tool_calls.clone());
                    }
                    // Attach provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = stream_extra_content
                        && !extra.is_null()
                    {
                        assistant_msg = assistant_msg.with_extra_content(extra.clone());
                    }

                    messages.push(assistant_msg.clone());
                    self.add_message(assistant_msg.clone()).await;

                    // Persist assistant message — use save_message_full when
                    // the message has tool_calls so they're not lost on reload.
                    // Previously save_message (4-arg) dropped tool_calls, which
                    // meant reloaded sessions lost the assistant's tool-call
                    // context (the tool results became orphaned).
                    let timestamp = chrono::Utc::now().to_rfc3339();
                    if assistant_msg.tool_calls.is_some() {
                        let tool_calls_json = assistant_msg
                            .tool_calls
                            .as_ref()
                            .and_then(|tcs| serde_json::to_string(tcs).ok());
                        let msg_data = crate::database::MessageData {
                            id: 0,
                            session_id: session_id.clone(),
                            role: "assistant".to_string(),
                            content: Some(effective_text.clone()),
                            tool_call_id: None,
                            tool_calls: tool_calls_json,
                            tool_name: None,
                            timestamp,
                            token_count: None,
                            // T1: persist the provider's real finish reason
                            // (previously hardcoded to "tool_calls").
                            finish_reason: finish_reason.clone(),
                            reasoning: assistant_msg.reasoning.clone(),
                            reasoning_content: None,
                            reasoning_details: None,
                            codex_reasoning_items: None,
                            codex_message_items: None,
                            platform_message_id: None,
                            observed: None,
                            active: 1,
                        };
                        if let Err(e) = self.database.save_message_full(&msg_data) {
                            tracing::warn!(error = %e, "failed to persist assistant message");
                        }
                    } else {
                        if let Err(e) = self.database.save_message(
                            &session_id,
                            "assistant",
                            &effective_text,
                            &timestamp,
                        ) {
                            tracing::warn!(error = %e, "failed to persist assistant message");
                        }
                    }
                    self.database
                        .save_session(
                            &session_id,
                            None,
                            "agent",
                            &chrono::Utc::now().to_rfc3339(),
                            &chrono::Utc::now().to_rfc3339(),
                        )
                        .ok();
                    if let Ok(total) = self.session_cost_usd.read() {
                        self.database.update_session_cost(&session_id, *total).ok();
                    }

                    // If no tool calls, we're done
                    if tool_calls.is_empty() {
                        let result = assistant_msg.clone();
                        self.spawn_session_distillation(messages.clone());

                        // Save trajectory if recording is enabled.
                        if self.record_trajectories {
                            self.save_trajectory(
                                &session_id,
                                &messages,
                                iteration,
                                total_tool_calls,
                                true,
                                Some(&result),
                            )
                            .await;
                        }

                        // ── Turn diagnostics (final response) ───────────────────
                        // Log structured diagnostics at turn completion, matching
                        // hermes-agent's turn-exit diagnostic log pattern.
                        {
                            let diag = TurnDiagnostics {
                                exit_reason: TurnExitReason::TextResponse,
                                model: self.model(),
                                api_calls: iteration,
                                max_iterations: self.config.max_iterations,
                                budget_used: self.iteration_budget.used(),
                                budget_max: self.iteration_budget.max_total(),
                                tool_turns: total_tool_calls,
                                response_len: result.content.len(),
                                session_id: session_id.clone(),
                            };
                            info!("{}", diag.log_message());
                        }

                        // ── TurnEnd lifecycle hook ───────────────────────────────
                        // Emit TurnEnd with iteration and tool call counts.
                        if let Some(ref hooks) = self.hook_registry {
                            let ctx = crate::gateway_pipeline::HookContext::new()
                                .with_session(&session_id)
                                .with_metadata("iterations", iteration.to_string())
                                .with_metadata("tool_calls", total_tool_calls.to_string());
                            hooks
                                .emit(crate::gateway_pipeline::HookEvent::TurnEnd, ctx)
                                .await;
                        }

                        self.emit(AgentEvent::Done {
                            message: assistant_msg,
                        })
                        .await;

                        // R6 — durable session activity heartbeat (hermes
                        // session_activity.py parity): stamp the session as
                        // active so gateway/session liveness views see work
                        // even when the session never sends a message.
                        self.touch_session_activity(&session_id, "turn complete")
                            .await;

                        // Memory provider: sync_turn + queue_prefetch hooks.
                        // sync_turn persists the completed turn to graph memory
                        // (entity extraction + auto-wiring). queue_prefetch
                        // queues background recall for the next turn.
                        // This is the native equivalent of the hermes-agent
                        // MemoryManager.sync_all() + queue_prefetch_all() pattern.
                        //
                        // Uses the MemorySyncExecutor for ordered, non-blocking
                        // background writes. Falls back to direct spawn when the
                        // executor isn't available (e.g. no memory provider).
                        if let Ok(exec_guard) = self.memory_sync_executor.try_lock() {
                            if let Some(executor) = exec_guard.as_ref() {
                                executor.submit_sync_turn(&user_query, &result.content);
                            } else if let Some(provider) = &self.memory_provider {
                                let user_text = user_query.clone();
                                let assistant_text = result.content.clone();
                                let provider_clone = provider.clone();
                                crate::daemon_pool::spawn("memory-sync", async move {
                                    if let Err(e) =
                                        provider_clone.sync_turn(&user_text, &assistant_text).await
                                    {
                                        tracing::warn!(error = %e, "Memory provider sync_turn hook failed");
                                    }
                                });
                            }
                        }
                        // Memory provider: queue background recall for the
                        // NEXT turn (hermes `queue_prefetch_all` call-site
                        // parity). The authoritative search runs in prefetch()
                        // at the top of the next turn; queue_prefetch is a
                        // non-blocking hook the provider can use to warm its
                        // backend — a slow provider can never block the
                        // turn-completion path.
                        if let Some(provider) = &self.memory_provider {
                            provider.queue_prefetch(&user_query);
                        }

                        // Emit AgentEnd hook
                        if let Some(ref hooks) = self.hook_registry {
                            hooks
                                .emit(
                                    crate::gateway_pipeline::HookEvent::AgentEnd,
                                    crate::gateway_pipeline::HookContext::new()
                                        .with_session(&session_id),
                                )
                                .await;
                        }

                        // Emit AgentEnd observer event (R5 turn-summary feed —
                        // the observer prints the per-turn accounting line;
                        // the grace/budget-exhausted path already emits this).
                        if let Some(ref obs) = self.observer {
                            let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                            obs.record_event(&ObserverEvent::AgentEnd {
                                provider: self.config.model.clone(),
                                model: self.model(),
                                duration: turn_start.elapsed(),
                                tokens_used: None,
                                cost_usd: if cost > 0.0 { Some(cost) } else { None },
                            });
                        }

                        // ── Eager LCM ingest (hermes context_engine parity) ──
                        // Commit the COMPLETED turn into the lossless DAG NOW
                        // (not only at the next build_messages), so the final
                        // assistant response is immediately recallable by the
                        // following turn via lcm_recall. Idempotent by
                        // (session, position, content_hash) — safe to run.
                        if let Some(engine) = &self.context_engine {
                            // Borrow — `messages` is not used after this point.
                            if let Err(e) = engine.ingest_turn(&session_id, &messages).await {
                                tracing::warn!(
                                    error = %e,
                                    "LCM eager turn ingest failed (non-fatal)"
                                );
                            }
                        }

                        return Ok(result);
                    }

                    total_tool_calls += tool_calls.len();

                    // Collect tool names before execute_tools() consumes
                    // the Vec, so the self-evolution check can detect
                    // skill_manage calls without holding a reference to the
                    // moved tool_calls.
                    tool_names = tool_calls
                        .iter()
                        .map(|tc| tc.function.name.clone())
                        .collect();

                    // Build a lookup map from tool_call_id → arguments so
                    // we can extract file paths / task descriptions when
                    // mirroring memory writes and delegation results.
                    let call_args: std::collections::HashMap<String, String> = tool_calls
                        .iter()
                        .map(|tc| (tc.id.clone(), tc.function.arguments.clone()))
                        .collect();

                    // ── Progressive LCM ingest (hermes context_engine parity) ──
                    // Commit the accumulated conversation (including the
                    // assistant message just pushed above) into the lossless
                    // DAG BEFORE executing tools, so a same-iteration tool
                    // call like `lcm_recall` can find statements the model
                    // just made. Idempotent by (session, position,
                    // content_hash) — safe to run every iteration.
                    if let Some(engine) = &self.context_engine
                        && let Err(e) = engine.ingest_turn(&session_id, &messages).await
                    {
                        tracing::warn!(
                            error = %e,
                            "LCM progressive ingest failed (non-fatal)"
                        );
                    }

                    // Capture the executed-call signature BEFORE the batch is
                    // moved into execute_tools (repetition guard below).
                    let executed_sig: Option<u64> = tool_calls.first().map(|tc| {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        tc.function.name.hash(&mut h);
                        tc.function.arguments.hash(&mut h);
                        h.finish()
                    });

                    // Execute tools and add results
                    let tool_results = self.execute_tools(tool_calls).await?;

                    // ── Degenerate-loop circuit breaker ────────────────
                    // When EVERY tool call in several consecutive iterations
                    // fails (guardrail skip / malformed args / execution
                    // error), the model is in a repetition loop: nudging once,
                    // then force-ending the turn with partial results.
                    if !tool_results.is_empty() && tool_results.iter().all(|r| !r.success) {
                        consecutive_failed_iters += 1;
                    } else {
                        consecutive_failed_iters = 0;
                    }

                    // ── Cross-iteration repetition guard (R35) ─────────
                    // Identical (name + args) calls repeated across
                    // consecutive iterations are pathological even when they
                    // succeed — nudge, then force-end the turn.
                    match executed_sig {
                        Some(sig) if last_call_sig == Some(sig) => {
                            identical_streak += 1;
                        }
                        Some(sig) => {
                            last_call_sig = Some(sig);
                            identical_streak = 1;
                        }
                        None => {
                            identical_streak = 0;
                        }
                    }
                    if identical_streak >= 6 {
                        warn!(
                            identical_streak,
                            "Identical tool call repeated across iterations — ending turn"
                        );
                        let abort_msg = Message::assistant(format!(
                            "⚠️ I stopped early: I repeated the same tool call {} times in a \
                             row. Partial work may be complete — ask me to continue if needed.",
                            identical_streak
                        ));
                        self.add_message(abort_msg.clone()).await;
                        return Ok(abort_msg);
                    }
                    if identical_streak == 4 {
                        warn!(
                            identical_streak,
                            "Identical tool call repeated — nudging model to stop"
                        );
                        let nudge = Message::user(
                            "[System: You have repeated the exact same tool call 4 times in a \
                             row with the same arguments. It will keep returning the same \
                             result. Stop calling tools and produce your final answer as \
                             plain text now.]",
                        );
                        messages.push(nudge.clone());
                        self.add_message(nudge).await;
                    }
                    if consecutive_failed_iters == 3 {
                        warn!(
                            consecutive_failed_iters,
                            "Degenerate tool-call loop — nudging model to stop repeating"
                        );
                        let nudge = Message::user(
                            "[System: Your last 3 tool-call iterations ALL failed or repeated \
                             identical calls. Do not call this tool again. Produce your final \
                             answer as plain text now.]",
                        );
                        messages.push(nudge.clone());
                        self.add_message(nudge).await;
                    } else if consecutive_failed_iters >= 6 {
                        warn!(
                            consecutive_failed_iters,
                            "Degenerate tool-call loop persists — ending turn with partial results"
                        );
                        let abort_msg = Message::assistant(format!(
                            "⚠️ I stopped early: my last {} tool iterations kept failing or \
                             repeating identically. Partial work may be complete — ask me to \
                             continue if needed.",
                            consecutive_failed_iters
                        ));
                        self.add_message(abort_msg.clone()).await;
                        return Ok(abort_msg);
                    }

                    // Add tool results to messages and persist them (truncated)
                    for result in tool_results {
                        // Secret redaction (hermes `redact.py` parity): tool
                        // output can carry env assignments, API keys, JWT
                        // tokens, connection strings, etc. from terminal
                        // output or file reads. Redact before the text is
                        // pushed to the LLM-bound message list, persisted to
                        // the session DB, or written to the trajectory.
                        let content = if result.success {
                            crate::redaction::redact_sensitive_text_if_enabled(
                                &truncate_tool_result(&result.name, &result.content),
                            )
                        } else {
                            crate::redaction::redact_sensitive_text_if_enabled(
                                result.error.as_deref().unwrap_or("Error"),
                            )
                        };

                        // ── Memory write mirroring (hermes parity) ────────
                        // When a built-in memory tool writes an entry
                        // (write_file to MEMORY.md/USER.md, patch, create_file),
                        // mirror the write to the memory provider so the graph
                        // stays in sync. Ported from hermes-agent's
                        // MemoryManager.notify_memory_tool_write() pattern.
                        // Only fires for memory-related file paths, not all writes.
                        if result.success
                            && (result.name == "write_file"
                                || result.name == "patch"
                                || result.name == "create_file")
                            && let Some(args_str) = call_args.get(&result.tool_call_id)
                            && let Ok(args_val) =
                                serde_json::from_str::<serde_json::Value>(args_str)
                        {
                            let path = args_val
                                .get("path")
                                .or_else(|| args_val.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            // Only mirror writes to memory-related files
                            let is_memory = path.ends_with("MEMORY.md")
                                || path.ends_with("USER.md")
                                || path.contains("/MEMORY.")
                                || path.contains("/USER.");
                            if is_memory {
                                self.notify_memory_write(&result.name, path, &result.content);
                            }
                        }

                        // ── Delegation observation (hermes parity) ────────
                        // When a subagent tool completes (delegate_task,
                        // spawn_subagent), notify the memory provider so the
                        // parent's graph captures delegated work. Ported from
                        // hermes-agent's MemoryManager.on_delegation() pattern.
                        if result.success
                            && (result.name == "delegate_task" || result.name == "spawn_subagent")
                        {
                            let task_desc = call_args
                                .get(&result.tool_call_id)
                                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                .and_then(|v| {
                                    v.get("task")
                                        .or_else(|| v.get("prompt"))
                                        .and_then(|t| t.as_str())
                                        .map(String::from)
                                })
                                .unwrap_or_else(|| result.name.clone());
                            self.notify_delegation(&task_desc, &result.content);
                        }

                        // Persist tool result (truncated)
                        if let Err(e) = self.database.save_message(
                            &session_id,
                            "tool",
                            &content,
                            &chrono::Utc::now().to_rfc3339(),
                        ) {
                            tracing::warn!(error = %e, "failed to persist tool result");
                        }
                        self.database
                            .save_session(
                                &session_id,
                                None,
                                "agent",
                                &chrono::Utc::now().to_rfc3339(),
                                &chrono::Utc::now().to_rfc3339(),
                            )
                            .ok();
                        if let Ok(total) = self.session_cost_usd.read() {
                            self.database.update_session_cost(&session_id, *total).ok();
                        }
                        // Emit ToolCall observer event
                        if let Some(ref obs) = self.observer {
                            obs.record_event(&ObserverEvent::ToolCall {
                                tool: result.name.clone(),
                                duration: Duration::from_millis(0),
                                success: result.success,
                            });
                        }

                        if result.success {
                            self.emit(AgentEvent::ToolComplete {
                                result: result.clone(),
                            })
                            .await;
                        } else {
                            self.emit(AgentEvent::ToolError {
                                tool_call_id: result.tool_call_id.clone(),
                                name: result.name.clone(),
                                error: result.error.clone().unwrap_or_default(),
                            })
                            .await;
                        }

                        messages.push(Message::tool(&result.tool_call_id, &content));
                        self.add_message(Message::tool(&result.tool_call_id, &content))
                            .await;
                    }

                    // ── File mutation advisory footer ──────────────────────────
                    // After all tool results are processed, scan for failed file
                    // mutations (write_file, patch, create_file) and log an advisory.
                    // The footer is logged for observability — the model will see the
                    // tool results with error messages on the next iteration anyway.
                    // Matches hermes-agent's _format_file_mutation_failure_footer pattern.
                    if let Some(footer) = file_mutation_verifier_footer(&messages) {
                        tracing::warn!(footer = %footer, "File mutation advisory");
                    }
                }
                Err(e) => {
                    // ── Turn diagnostics (error exit) ─────────────────────
                    {
                        let diag = TurnDiagnostics {
                            exit_reason: TurnExitReason::Error,
                            model: self.model(),
                            api_calls: iteration,
                            max_iterations: self.config.max_iterations,
                            budget_used: self.iteration_budget.used(),
                            budget_max: self.iteration_budget.max_total(),
                            tool_turns: total_tool_calls,
                            response_len: 0,
                            session_id: session_id.clone(),
                        };
                        warn!("{}", diag.log_message());
                    }
                    error!(error = %e, "Error processing stream");
                    self.emit(AgentEvent::Error {
                        error: e.user_message(),
                    })
                    .await;
                    if self.record_trajectories {
                        self.save_trajectory(
                            &session_id,
                            &messages,
                            iteration,
                            total_tool_calls,
                            false,
                            None,
                        )
                        .await;
                    }
                    // Emit AgentEnd observer event on error
                    if let Some(ref obs) = self.observer {
                        let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                        obs.record_event(&ObserverEvent::AgentEnd {
                            provider: self.config.model.clone(),
                            model: self.model(),
                            duration: llm_duration,
                            tokens_used: None,
                            cost_usd: if cost > 0.0 { Some(cost) } else { None },
                        });
                    }
                    return Err(e);
                }
            }

            self.emit(AgentEvent::IterationComplete { iteration }).await;

            // Emit TurnComplete observer event
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::TurnComplete);
            }

            // ── Self-evolution: skill nudge (per-iteration cadence) ──
            // After each completed iteration, bump the skill counter and check
            // if a skill-review should fire. Mirrors hermes-agent's
            // turn_finalizer.py logic where _iters_since_skill is checked after
            // the tool-calling loop — bumped per *iteration*, NOT per turn.
            // (Memory review is on a separate per-turn cadence handled at the
            // turn boundary above.)
            //
            // When skill_manage is called, the skill counter resets immediately
            // so the nudge window restarts from zero.
            //
            // Scope the MutexGuard so it's dropped before the .await below.
            // A std::sync::MutexGuard held across an await point makes the
            // future !Send, which breaks tokio::spawn.
            let should_review_skills = {
                let skill_manage_called = tool_names.iter().any(|n| n == "skill_manage");
                let mut evo = self
                    .evolution_state
                    .lock()
                    .expect("evolution_state mutex poisoned — programmer error");

                let trigger = turn_finalizer::advance_skill_trigger(&mut evo, skill_manage_called);

                if trigger.should_review_skills {
                    info!(
                        iters = trigger.iters_since_skill,
                        interval = self.config.skill_nudge_interval,
                        "Skill nudge triggered — spawning background review"
                    );
                }

                // ── Persist evolution counters to session metadata ──
                // After bumping, persist so the next run() can hydrate.
                if self.persistent_session_id.is_some() {
                    for (key, val) in evo.persist_counters() {
                        let _ = self.database.set_session_metadata(&session_id, key, &val);
                    }
                }

                trigger.should_review_skills
            }; // MutexGuard dropped here — safe to .await
            if should_review_skills {
                self.spawn_background_review(&messages, &session_id, true, false)
                    .await;
            }

            // ── /steer directive drain (iter-65) ──────────────────────────
            // Between iterations, check if the user queued any steer
            // directives. If so, inject them as a user-role message so
            // the model sees the real-time guidance on the next iteration.
            // This mirrors hermes-agent's /steer drain which injects into
            // the last tool-role message to preserve role alternation.
            if let Some(steer_text) = self.drain_steers().await {
                info!(steer = %steer_text, "Injecting steer directive");
                let steer_msg = Message::user(format!(
                    "[STEER] {}\n\nPlease adjust your approach based on this guidance.",
                    steer_text
                ));
                messages.push(steer_msg.clone());
                self.add_message(steer_msg).await;
            }
        }
    }

    /// Build messages including system prompt.
    ///
    /// Applies context management (decay + eviction) to fit within the
    /// context window budget. When the estimated token count exceeds
    /// 80% of the budget, aggressive preflight compression fires to
    /// prevent wasted LLM calls that would fail with
    /// context_length_exceeded.
    ///
    /// Ported from hermes-agent's `turn_context.py` preflight compression
    /// pattern: estimate → check threshold → compress → fit within budget.
    pub(crate) async fn build_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        // ── Prompt-cache stability (iter-39) ─────────────────────────────
        // Split the system prompt into TWO messages:
        //   1. FROZEN PREFIX: base system prompt + skills. These rarely
        //      change across turns, so keeping them byte-stable lets
        //      Anthropic's prompt cache hit (cache reads cost ~10x less
        //      than fresh prompt tokens).
        //   2. VOLATILE SUFFIX: memory context + workspace context. These
        //      change each turn (memory grows, workspace files change),
        //      so they go in a separate message AFTER the frozen prefix.
        //      The frozen prefix stays cache-stable; only the volatile
        //      suffix + conversation history are re-processed each turn.
        //
        // This is a simplified version of magic-context's m[0]/m[1]
        // cache layout. The full m[0]/m[1] scheme uses HARD/SOFT/SOFT+
        // pass taxonomy + byte-identical replay; this implementation
        // just splits into frozen vs volatile, which captures ~80% of
        // the cache benefit with ~10% of the complexity.

        // Build the frozen prefix (base system prompt + skills + memory
        // provider status line). Uses the shared helper to avoid
        // duplicating the prefix logic with spawn_background_review's
        // cache parity path.
        let mut frozen_prefix = self.build_frozen_prefix();
        if let Some(provider) = &self.memory_provider {
            let block = provider.system_prompt_block().await;
            if !block.trim().is_empty() {
                frozen_prefix.push_str("\n\n");
                frozen_prefix.push_str(block.trim());
            }
        }
        messages.push(Message::system(frozen_prefix));

        // Volatile suffix: memory context + workspace context. These
        // change each turn, so they're a separate message that doesn't
        // bust the frozen prefix's cache entry.
        let mut volatile_suffix = String::new();
        if let Some(memory_manager) = &self.memory_manager {
            let memory_context = memory_manager.build_memory_context(2048).await;
            let memory_context = memory_context.trim();
            if !memory_context.is_empty() {
                volatile_suffix.push_str("\n\n<long_term_memory>\n");
                volatile_suffix.push_str(memory_context);
                volatile_suffix.push_str("\n</long_term_memory>");
            }
        }

        // Memory provider: per-turn semantic recall (prefetch).
        // Runs with an 8s timeout — matches hermes-agent's prefetch
        // timeout pattern. Results land under <memory_context> tags
        // (distinct from the file-backed <long_term_memory> block).
        if let Some(provider) = &self.memory_provider {
            let last_user = {
                let conv = self.conversation.read().await;
                conv.iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.content.clone())
            };
            if let Some(query) = last_user {
                let provider_context =
                    tokio::time::timeout(Duration::from_secs(8), provider.prefetch(&query))
                        .await
                        .unwrap_or_default();
                let provider_context = provider_context.trim();
                if !provider_context.is_empty() {
                    volatile_suffix.push_str("\n\n<memory_context>\n");
                    volatile_suffix.push_str(provider_context);
                    volatile_suffix.push_str("\n</memory_context>");
                }
            }
        }

        let context_files = self.load_context_file_prompt();
        if !context_files.trim().is_empty() {
            volatile_suffix.push_str("\n\n<workspace_context>\n");
            volatile_suffix.push_str(context_files.trim());
            volatile_suffix.push_str("\n</workspace_context>");
        }

        if !volatile_suffix.trim().is_empty() {
            messages.push(Message::system(volatile_suffix.trim().to_string()));
        }

        // ── MoA guidance injection (G5, hermes moa_loop.py parity) ─────
        // Per-turn Mixture-of-Agents guidance computed before run(); injected
        // as a system message after the volatile suffix so the acting loop
        // sees it for every iteration of this turn. Drained — it never leaks
        // into the next turn (a plain turn with no MoA is byte-identical to
        // before, preserving prompt-cache stability).
        if let Some(guidance) = self.drain_moa_guidance() {
            messages.push(Message::system(guidance));
        }

        // Add conversation history
        let conv = self.conversation.read().await;
        messages.extend(conv.clone());
        drop(conv);

        // Apply context management: decay-render old messages + evict
        // if over budget. Without this, any long-running session would
        // eventually exceed the context window and 400-error.
        //
        // Preflight compression (proactive): estimate tokens before the
        // LLM call. If the estimated count exceeds 80% of the context
        // window, apply aggressive decay to compress older messages.
        // This prevents wasted LLM calls that would fail with
        // context_length_exceeded. Ported from hermes-agent's
        // turn_context.py preflight compression pattern.
        let budget = self.config.context_window;
        let reserve = 4096; // tokens reserved for the model's response
        let effective_budget = budget.saturating_sub(reserve);

        let estimated_tokens = self.estimate_current_tokens(&messages);
        let preflight_threshold = budget * PREFLIGHT_THRESHOLD_PERCENT as usize / 100;
        if estimated_tokens > preflight_threshold {
            info!(
                estimated = estimated_tokens,
                threshold = preflight_threshold,
                budget,
                "Preflight compression: estimated tokens exceed threshold"
            );
            // Memory provider: on_pre_compress hook.
            // Extract insights from messages about to be compressed and
            // prepend them as a user context block so the downstream
            // compression/decay preserves what the memory provider still
            // considers important. Mirrors hermes-agent's plugin behavior
            // (insert `[agentmemory context before compaction]` at index 0).
            if let Some(provider) = &self.memory_provider {
                let insights = provider.on_pre_compress(&messages);
                if !insights.is_empty() {
                    tracing::debug!(
                        insights_len = insights.len(),
                        "Memory provider pre-compress insights captured"
                    );
                    messages.insert(
                        0,
                        crate::client::Message::user(format!(
                            "[memory context before compaction]\n{insights}"
                        )),
                    );
                }
            }
            messages = crate::context_management::decay_render(
                messages,
                PREFLIGHT_DECAY_H50,
                PREFLIGHT_DECAY_CONSTANT,
            );
        }

        // Context engine hook (hermes-lcm parity): when a lossless engine is
        // attached, it assembles the final list (D0 fresh tail kept verbatim,
        // older context compacted into the DAG and recallable) INSTEAD of the
        // lossy eviction below.
        //
        // The session key is the one resolved by turn_context for this run
        // (NOT a `"default"` fallback) so DAG ingestion uses the SAME key as
        // the loop's progressive/eager ingest — otherwise the same turn is
        // stored twice under two session keys (wasted storage + scoped recall
        // misses).
        if let Some(engine) = &self.context_engine {
            // `assemble` consumes `messages`; on failure fall back to lossy
            // eviction over the raw conversation history (rare error path).
            match engine
                .assemble(session_id, messages, effective_budget)
                .await
            {
                Ok(assembled) => messages = assembled,
                Err(e) => {
                    warn!(error = %e, engine = engine.name(),
                          "context engine assemble failed — falling back to lossy eviction");
                    let history = self.conversation.read().await;
                    messages = crate::context_management::evict_to_budget(
                        history.clone(),
                        effective_budget,
                    );
                }
            }
        } else {
            // Standard eviction: remove oldest messages within tiers until
            // the total fits within the effective budget.
            messages = crate::context_management::evict_to_budget(messages, effective_budget);
        }

        let seq_repairs = message_safety::repair_message_sequence(&mut messages);
        if seq_repairs > 0 {
            info!(
                repairs = seq_repairs,
                "Repaired message sequence violations"
            );
        }

        // Drop thinking-only assistant messages and merge consecutive user
        // messages. Needed for Anthropic models that emit reasoning as
        // separate empty-content assistant messages.
        messages = message_safety::drop_thinking_only_and_merge_users(&messages);

        // Sanitize tool calls for strict API providers (Gemini, Claude
        // strict mode) that enforce stricter name/argument validation.
        let tool_sans = message_safety::sanitize_tool_calls_for_strict_api(&mut messages);
        if tool_sans > 0 {
            debug!(
                sanitizations = tool_sans,
                "Sanitized tool calls for strict API"
            );
        }

        Ok(messages)
    }
}
