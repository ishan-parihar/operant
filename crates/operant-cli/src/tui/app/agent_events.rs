//! Agent event handling methods.

use super::*;

impl App {
    /// Process a query event from the agentic loop.
    /// Handle an AgentEvent from the agent. (iter-114 — replaces
    /// handle_query_event; eliminates the bridge layer.)
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        // Publish to debug bus (no-op when disabled).
        let event_variant = format!("{:?}", std::mem::discriminant(&event));
        let event_summary: String = format!("{:?}", event).chars().take(80).collect();
        self.debug_hub
            .publish(crate::tui::debug::TuiEvent::AgentEvent {
                variant: event_variant,
                summary: event_summary,
                at: crate::tui::debug::event_bus::now_secs(),
            });

        // Auto-dismiss error modal when assistant responds
        match &event {
            AgentEvent::Content { .. }
            | AgentEvent::Thinking { .. }
            | AgentEvent::Reasoning { .. }
            | AgentEvent::Done { .. } => {
                self.dismiss_error_notifications();
            }
            _ => {}
        }

        match event {
            AgentEvent::Thinking { content } | AgentEvent::Reasoning { text: content } => {
                // Route thinking/reasoning to streaming_thinking.
                if !self.is_streaming {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed as u64).to_string());
                    if self.turn_start.is_none() {
                        self.turn_start = Some(std::time::Instant::now());
                    }
                    self.streaming_thinking.clear();
                    self.streaming_text.clear();
                }
                self.is_streaming = true;
                self.turn_state = TurnState::Thinking;
                self.stall_start = None;
                // If we already have streaming text, this is a NEW iteration —
                // the model is thinking again after a tool call. Clear the old
                // text so we don't accumulate duplicate content across iterations.
                // (iter-122 — fixes "double thinking and text streaming" bug.)
                if !self.streaming_text.is_empty() {
                    // Flush the previous iteration's text as a completed message
                    // so it's preserved in the transcript, then start fresh.
                    self.flush_streamed_assistant_message();
                    self.streaming_thinking.clear();
                }
                // Off never shows reasoning anywhere (jcode
                // ReasoningDisplayMode::Off): don't even accumulate — the
                // live tail and the commit path both stay empty.
                if crate::tui::operant_app::config_shim::config()
                    .display
                    .reasoning_display()
                    != crate::tui::operant_app::config_shim::ReasoningDisplayMode::Off
                {
                    self.streaming_thinking.push_str(&content);
                }
                self.invalidate_transcript();
            }

            AgentEvent::Content { text } => {
                // Strip \r carriage returns as a safety net.
                // \r corrupts terminal display by moving cursor to column 0.
                let text = text.replace('\r', "");
                if !self.is_streaming {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed as u64).to_string());
                    if self.turn_start.is_none() {
                        self.turn_start = Some(std::time::Instant::now());
                    }
                    self.streaming_thinking.clear();
                    self.streaming_text.clear();
                }
                self.is_streaming = true;
                self.turn_state = TurnState::Streaming;
                self.stall_start = None;
                // Accumulate streaming text. (Boundary flushes are handled by
                // AgentEvent::Thinking and AgentEvent::ToolStart.)
                self.streaming_text.push_str(&text);
                self.invalidate_transcript();
            }

            AgentEvent::ToolStart {
                tool_call_id,
                name,
                arguments,
            } => {
                if !self.is_streaming && self.spinner_verb.is_none() {
                    let seed = self.frame_count as usize ^ (self.messages.len() * 17);
                    self.spinner_verb = Some(sample_spinner_verb(seed as u64).to_string());
                }
                self.is_streaming = true;
                self.turn_state = TurnState::RunningTool;

                // Per-iteration text grouping (2026-10-09 live audit, P0-2):
                // a mid-stream ToolStart must not split the assistant text.
                // Providers interleave content deltas around tool_call deltas
                // — observed live: "…first 5 lines of AG" / tool_call /
                // "ENTS.md and summarize." — and flushing here rendered the
                // word AGENTS.md split across the tool row. The text buffer now
                // spans the whole iteration and flushes at the next boundary
                // (next iteration's Thinking, Done, or cancel). Tool rows
                // anchor one slot further so they still render after the text
                // that was in flight when the call was parsed: the reservation
                // mirrors flush_streamed_assistant_message's push condition
                // exactly, so a slot is reserved iff the flush will fill it.
                // (iter-785, glitch 2: order is preserved WITHOUT flushing
                // here — reserved rows render in the Streaming section below
                // the live tails (adapter::live_tool_rows), and the seam keeps the
                // AG|ENTS.md word intact across the tool_call delta.)
                let text_slot = if self.streaming_text.trim().is_empty()
                    && self.streaming_thinking.trim().is_empty()
                {
                    0
                } else {
                    1
                };
                let after_index = self.messages.len() + text_slot;
                let tool_id = tool_call_id.clone();
                let tool_name = name.clone();
                let input_json = arguments;
                // The model's display label for this call. Parsed once here
                // rather than on every read of `input_json`, because the
                // arguments are still a partial JSON fragment for most of the
                // stream and a per-frame parse would fail on every in-flight
                // call. A fragment that does not parse carries no intent yet —
                // the next ToolStart (with the complete arguments) fills it.
                let intent = crate::tui::operant_model::vendor_types::ToolCall::intent_from_input(
                    &serde_json::from_str::<serde_json::Value>(&input_json)
                        .unwrap_or(serde_json::Value::Null),
                );
                // The first ToolStart for an id only means "parsed"; the agent
                // re-announces it once it holds a worker permit. So: a new
                // block is Queued, an existing one transitions to Running.
                // The arrival anchor is stamped once at creation and never
                // re-stamped: the row must stay where the call happened in the
                // stream, not follow later messages down the transcript.
                let started = if let Some(existing) =
                    self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id)
                {
                    existing.status = ToolStatus::Running;
                    existing.output_preview = None;
                    existing.input_json = input_json;
                    // Never CLEAR an intent once captured: the re-announcement
                    // carries the same arguments, but refusing to clear guards
                    // against a shorter fragment arriving second.
                    if intent.is_some() {
                        existing.intent = intent;
                    }
                    true
                } else {
                    // First ToolStart for this id — the call is parsed but
                    // still waiting on the concurrent pool. It stays Queued
                    // until the agent re-announces it with a permit in hand.
                    self.tool_use_blocks.push(ToolUseBlock {
                        id: tool_id.clone(),
                        name: tool_name.clone(),
                        after_index,
                        status: ToolStatus::Queued,
                        output_preview: None,
                        input_json,
                        intent,
                    });
                    false
                };
                // Track subagent spawns for the status-bar HUD.
                if tool_name == "delegate_task" || tool_name == "spawn_subagent" {
                    self.agent_status.retain(|(id, _)| id != &tool_id);
                    self.agent_status.push((
                        tool_id,
                        if started { "running" } else { "queued" }.to_string(),
                    ));
                }

                self.invalidate_transcript();
            }

            AgentEvent::ToolComplete { result } => {
                let tool_id = result.tool_call_id.clone();
                let is_error = !result.success;
                let result_text = if result.success {
                    result.content.clone()
                } else {
                    result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string())
                };
                let preview = tool_output_preview(&result_text, is_error);
                self.remember_tool_output(&tool_id, &result_text);
                if let Some(block) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    block.status = if is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Done
                    };
                    block.output_preview = Some(preview);
                }
                // A delegation reports to the HUD by id, even when its transcript
                // block was cleared by /new or /rewind (TUI-9).
                let new_status = if is_error { "error" } else { "done" };
                for (id, st) in self.agent_status.iter_mut() {
                    if id == &tool_id {
                        *st = new_status.to_string();
                    }
                }
                self.invalidate_transcript();
                // The tool settled; the model has to re-plan before the next
                // Content event, so the turn is back to Thinking.
                self.turn_state = TurnState::Thinking;
                // Iteration boundary (2026-10-09 live audit, P0-2): the
                // assistant text streamed before this tool completed belongs
                // to the previous model response — the next Content starts a
                // NEW response. Flush here so the two never glue into one
                // message ("…summarize.The file wasn't found…"), while text
                // interleaved AROUND the tool_call parse mid-response still
                // joins (the AG|ENTS.md seam) because ToolStart no longer
                // flushes.
                self.flush_streamed_assistant_message();
                // Errors route through set_status_notice with its own TTL.
                // A success does NOT wipe the notice: tools run up to 8 in
                // parallel, their events drain as one batch, and one success
                // landing after a failure in the same batch erased the error
                // (and any RateLimit/StreamCorrupted warning) before its
                // 3-second TTL elapsed.
                if is_error {
                    self.set_status_notice(format!("Tool error: {}", result_text));
                }
                // P4-3 (2026-10-09 v2 outline): the todo tool's result carries
                // the full list; keep it on App so the info-widget Todos band,
                // the pinned card and /todos read live state. The counts-only
                // TodoUpdated event stays as the status-line signal — it can
                // never render a list.
                if result.success
                    && result.name == "todo"
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&result.content)
                    && let Some(items) = value.get("todos")
                    && let Ok(parsed) = serde_json::from_value::<
                        Vec<crate::tui::operant_app::todo::TodoItem>,
                    >(items.clone())
                {
                    // Route through the same rebuild the 1s live tick uses,
                    // and re-stamp the fingerprint with the result's own list:
                    // the tool result is authoritative NOW, while the tick may
                    // have been throttled for up to a second and could still
                    // be reading the pre-write file. Sharing the rebuild keeps
                    // the payload's shape in one place.
                    //
                    // Learn the store key from the result. The tool resolves it
                    // as (explicit arg, agent session metadata, "default"), so
                    // its echoed `sessionId` is authoritative — reading it here
                    // is how the tick follows a session that is NOT "default"
                    // instead of polling a bucket nothing writes to. The value
                    // is already parsed as `value`, so no second parse.
                    if let Some(sid) = value.get("sessionId").and_then(|v| v.as_str())
                        && !sid.is_empty()
                    {
                        self.todos_session_id = sid.to_string();
                    }
                    self.todos = parsed;
                    self.rebuild_todos_card_payload();
                    self.todos_fingerprint = todos_fingerprint(&self.todos);
                    self.invalidate_transcript();
                }
                // (iter-209: refresh_turn_diff_from_history removed)
            }

            AgentEvent::ToolError {
                tool_call_id,
                name: _,
                error,
            } => {
                let tool_id = tool_call_id.clone();
                let result_text = error;
                let preview = tool_output_preview(&result_text, true);
                self.remember_tool_output(&tool_id, &result_text);
                if let Some(block) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    block.status = ToolStatus::Error;
                    block.output_preview = Some(preview);

                    if block.name == "delegate_task" || block.name == "spawn_subagent" {
                        for (id, st) in self.agent_status.iter_mut() {
                            if id == &tool_id {
                                *st = "error".to_string();
                            }
                        }
                    }
                }
                self.invalidate_transcript();
                self.turn_state = TurnState::Thinking;
                self.set_status_notice(format!("Tool error: {}", result_text));
                // Iteration boundary, same as ToolComplete (P0-2): a failed
                // tool also ends the response that called it.
                self.flush_streamed_assistant_message();
                // (iter-209: refresh_turn_diff_from_history removed)
            }

            AgentEvent::Done {
                message, reason, ..
            } => {
                // Turn complete — the agent finished.
                // (iter-210: fix BACKEND_TUI_AUDIT.md §3 bug #2 — Done.message
                // was previously discarded with `message: _`. If the agent
                // emitted Done without preceding Content events (e.g. a
                // non-streaming error-recovery path), the user saw an empty
                // assistant message. Now: if streaming_text is empty, use
                // Done.message.content as the source of truth.)
                self.is_streaming = false;
                self.turn_state = TurnState::Idle;
                self.spinner_verb = None;
                // The turn this submission paid for is over and it worked, so
                // there is nothing to give back. Disarming here is what keeps
                // the slot's invariant true by construction — armed only while
                // a turn is in flight — rather than by argument about which
                // later event might or might not follow.
                self.clear_failed_input_recovery();

                // Record elapsed time and pick a completion verb
                let seed = self.frame_count as usize ^ (self.messages.len() * 7);
                let elapsed = self
                    .turn_start
                    .take()
                    .map(|start| format_elapsed_ms(start.elapsed().as_millis()));
                self.last_turn_elapsed = Some(elapsed.unwrap_or_else(|| "0s".to_string()));
                self.last_turn_verb = Some(sample_completion_verb(seed as u64));

                // If we have streamed content, flush it normally. If not,
                // use Done.message as the source of truth (fixes the
                // dropped-message bug for non-streaming paths).
                let before = self.messages.len();
                if self.streaming_text.trim().is_empty()
                    && self.streaming_thinking.trim().is_empty()
                    && !message.content.is_empty()
                {
                    // Non-streaming path: Done carries the full message.
                    let mut blocks = Vec::new();
                    if let Some(reasoning) = &message.reasoning
                        && !reasoning.trim().is_empty()
                        // Full and Current commit the trace (see
                        // flush_streamed_assistant_message); Off never shows it.
                        && crate::tui::operant_app::config_shim::config()
                            .display
                            .reasoning_display()
                            != crate::tui::operant_app::config_shim::ReasoningDisplayMode::Off
                    {
                        blocks.push(ContentBlock::Thinking {
                            thinking: reasoning.clone(),
                            signature: String::new(),
                        });
                    }
                    blocks.push(ContentBlock::Text {
                        text: message.content.clone(),
                    });
                    let msg = Message::assistant_blocks(blocks);
                    self.messages.push(msg);
                    self.invalidate_transcript();
                    self.on_new_message();
                } else {
                    self.flush_streamed_assistant_message();
                }
                // (BUGS.md S5, the tui-bugfix audit) Only TextResponse is a
                // normal completion: a GraceCall/BudgetExhausted/
                // CircuitBreaker reason means the agent STOPPED EARLY and the
                // message is a best-effort partial (possibly empty after the
                // S1 grace quality gate). The core contract says consumers
                // that surface the message to an operator MUST check the
                // reason — the gateway_runner does; the TUI discarded it, so
                // a budget-exhausted turn rendered with the normal completion
                // verb and no notice, and an S1-rejected empty partial hit
                // the "corrupted provider stream" misdiagnosis below. The
                // Interrupted reason already has its own surface (the
                // "Turn stopped." arm of the Error event).
                let early_notice = match reason {
                    operant_core::agent::TurnExitReason::TextResponse
                    | operant_core::agent::TurnExitReason::Interrupted => None,
                    operant_core::agent::TurnExitReason::BudgetExhausted => Some(
                        "⚠️ Turn ended early — iteration budget exhausted. The answer \
                             shown is a best-effort partial; break the task into smaller \
                             steps or raise `max_iterations`.",
                    ),
                    operant_core::agent::TurnExitReason::GraceCall => {
                        // The wrap-up is TOOLLESS and it can fail in two
                        // different ways. A summary that was CUT OFF by the
                        // output limit is a partial answer — "ran out of
                        // room" is true. A summary the S1 quality gate
                        // rejected (empty, or a reasoning-leak artifact) is
                        // NOT out of room: the model answered, the answer
                        // was garbage, and the turn produced nothing. The
                        // live incident (2026-11-05) was the second kind
                        // and the notice blamed the first.
                        if message.content.trim().is_empty() {
                            Some(
                                "⚠️ Turn ended early — the wrap-up produced no usable \
                                 summary. The best-effort partial shown is empty; the \
                                 harness tried to continue the task on its own — reply \
                                 'continue' to pick it up.",
                            )
                        } else {
                            Some(
                                "⚠️ Turn ended early — the wrap-up ran out of room \
                                 mid-summary. The answer shown is a best-effort partial; \
                                 reply 'continue' to resume.",
                            )
                        }
                    }
                    operant_core::agent::TurnExitReason::CircuitBreaker => Some(
                        "⚠️ Turn ended early — the model looped the same failing call \
                             until the circuit breaker stopped it. Partial work may be \
                             complete; ask to continue.",
                    ),
                    operant_core::agent::TurnExitReason::Error => {
                        Some("⚠️ Turn ended with an error — check the logs for details.")
                    }
                };
                let stopped_early = early_notice.is_some();
                if let Some(notice) = early_notice {
                    self.push_system_message(notice.to_string(), SystemMessageStyle::Info);
                    self.set_status_notice("Turn ended early — partial answer.");
                }
                // 2026-10-10 forced-repro finding: a corrupted provider
                // stream (SSE events the parser must drop) ends the turn
                // with NO text and NO tool rows — an empty reply that used
                // to fail silently, which read as "the TUI ate my message".
                // Surface it so the user knows to retry rather than stare
                // at a blank transcript.
                // `before` unchanged means neither branch pushed a message —
                // tool rows are session-lifetime state, so they say nothing
                // about THIS turn and must not gate the notice.
                // A stopped-early turn with an empty partial is NOT a
                // corrupted stream — the wire was fine, the budget ran out —
                // and the stopped-early card above already explains it, so
                // the misdiagnosis stays suppressed there.
                let turn_gained_nothing =
                    self.messages.len() == before && self.streaming_text.trim().is_empty();
                if turn_gained_nothing && !stopped_early {
                    self.set_status_notice(
                        "Empty reply — the provider stream may be corrupted; \
                         retry or switch model"
                            .to_string(),
                    );
                }
                // Mark any remaining pending (queued OR running) blocks as
                // Done — they completed but the ToolComplete event either
                // fired before the Done event or was never emitted (fast tool
                // / race condition). Pruning them silently dropped the tool
                // trail from the user's view.
                for block in &mut self.tool_use_blocks {
                    if block.status.is_pending() {
                        block.status = ToolStatus::Done;
                    }
                }
                self.complete_current_turn_snapshot(false);
                self.invalidate_transcript();
                // (iter-209: refresh_turn_diff_from_history removed)

                // (P4-2 purge: the every-response "Response complete · /copy…"
                // banner is gone — audit complaint 1; /copy stays in the
                // command surface and the context menu.)
            }

            AgentEvent::Error { error } => {
                self.run_error_surfaced = true;
                self.is_streaming = false;
                self.turn_state = TurnState::Idle;
                self.spinner_verb = None;
                // Keep what already streamed: the partial reply stays
                // readable above the card instead of vanishing.
                self.flush_streamed_assistant_message();
                self.invalidate_transcript();
                // The turn died mid-flight: any tool call it started never
                // ran to completion. Mark this turn's still-pending blocks
                // cancelled — the same loop shape as cancel_streaming_turn —
                // so a later Done sweep cannot claim them: with
                // output_preview: None the adapter renders a pending/Done
                // block as a phantom "✓ completed" row. Idempotent with
                // cancel_streaming_turn on the /stop path, where the blocks
                // are already Cancelled.
                for block in &mut self.tool_use_blocks {
                    if block.status.is_pending() {
                        block.status = ToolStatus::Cancelled;
                    }
                }
                // A user stop is not a failure: one quiet notice, no error
                // card, no prompt restore.
                if is_user_interrupt(&error) {
                    self.clear_failed_input_recovery();
                    self.push_system_message("Turn stopped.".to_string(), SystemMessageStyle::Info);
                    return;
                }
                // Give the prompt back before the error is surfaced. This is
                // the arm the error modal is drawn from, so the message and the
                // recovery land in the same frame.
                self.restore_failed_input_to_composer();
                // The restore removed the user row, which leaves every
                // after_index stamped this turn pointing past the
                // transcript — the next pushed prompt would otherwise own
                // the row. Clamp to the post-restore length (the flush's
                // own release arm in flush_streamed_assistant_message
                // mirrors this shape). No row is removed on the /stop path
                // above, so its anchors stay valid unclamped.
                for block in &mut self.tool_use_blocks {
                    block.after_index = block.after_index.min(self.messages.len());
                }
                let err_msg = format!("Error: {}", error);
                // The debug bus carries the LIVE failure, not just the keys
                // and the renders: an event log that records everything the
                // operator did and nothing the agent failed at is no record
                // of the failure.
                self.debug_hub.record_error("agent", &error);
                // Single source of truth per event (P4-2 spec step 4): the
                // inline Error card is THE transcript surface — no duplicate
                // assistant row, no modal. The status line carries recovery.
                self.push_notice(NotificationKind::Error, err_msg);
                self.set_status_notice("Prompt restored to the composer — the turn failed.");
            }

            AgentEvent::Usage {
                input_tokens,
                output_tokens,
                total_tokens,
            } => {
                // Record cost tracking immediately (was deferred to TurnComplete
                // via the bridge's pending_usage — now we record it directly).
                // (iter-210: fix BACKEND_TUI_AUDIT.md §3 bug #5 — total_tokens
                // was previously discarded with `total_tokens: _` and
                // recomputed by CostTracker as input+output. Now we use the
                // agent's authoritative total_tokens, which may include
                // cached/reasoning tokens that input+output misses.)
                let turn_tokens = total_tokens.max(input_tokens + output_tokens);
                // The CONTEXT OCCUPANCY is the prompt the provider just
                // billed — the window is not a running meter of everything
                // the session ever spent. This used to ACCUMULATE the turn
                // totals, so the context overlay showed the session's
                // cumulative spend as if it were the live context size (a
                // 200k window read "full" after one long turn).
                self.context_used_tokens = u64::from(input_tokens);
                // Per-turn footer deltas. Cache tokens aren't reported by this
                // event, so the cache counters stay at 0 and the footer hides
                // them (see the App field docs).
                self.record_turn_usage(input_tokens, output_tokens);
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.record_usage(input_tokens, output_tokens);
                }
                self.cost_usd = self.cost_tracker.total_cost;
                self.token_count = turn_tokens;
                // (iter-1207) `input_tokens` is the size of the prompt the
                // provider just billed — the only context figure that
                // includes the system prompt and the tool schemas. The
                // footer prefers it over the char estimate (which sees
                // neither), so capture it here rather than letting it die
                // with the event. Clear when the provider reports no
                // usage at all: a real 0 is still a measurement, and the
                // estimate is the better guide than a stale number.
                self.last_observed_prompt_tokens =
                    (total_tokens > 0 || input_tokens > 0 || output_tokens > 0)
                        .then_some(u64::from(input_tokens));
                self.check_token_warnings();
            }

            AgentEvent::RateLimitNotice { retry_after_secs } => {
                // T3: surface the rate-limit state as a non-blocking
                // notification (hermes `_capture_rate_limits` parity).
                self.turn_state = TurnState::WaitingForNetwork;
                let msg = match retry_after_secs {
                    Some(secs) if secs > 0 => {
                        format!("Rate limit reached — retry in ~{secs}s")
                    }
                    _ => "Rate limit reached — retrying with backoff".to_string(),
                };
                self.push_notice(NotificationKind::Warning, msg);
            }

            AgentEvent::StreamCorrupted { dropped_events } => {
                // iter-754: the provider's SSE framing was corrupted and the
                // parser dropped unparseable payloads — the rendered turn may
                // be truncated or look interleaved. Surface WHY so the
                // corruption is attributed to the wire, not the agent.
                self.turn_state = TurnState::WaitingForNetwork;
                self.push_notice(
                    NotificationKind::Warning,
                    format!(
                        "⚠ {dropped_events} corrupted stream event(s) dropped by the provider — output may be truncated"
                    ));
            }

            AgentEvent::Cost {
                cost_usd,
                input_tokens,
                output_tokens,
                model,
            } => {
                // R3: wire the model-aware per-request cost into the live
                // tracker instead of discarding it. Falls back to a flat-rate
                // estimate only when the model isn't in the models_dev catalog.
                let cost = cost_usd.unwrap_or({
                    input_tokens as f64 * 0.000003 + output_tokens as f64 * 0.000015
                });
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.record_cost(cost);
                    tracker.set_model(&model);
                }
                self.cost_usd = self.cost_tracker.total_cost;
                if cost_usd.is_some() {
                    debug!(cost_usd = %cost, model = %model, "Per-request cost (models_dev)");
                } else {
                    debug!(cost_usd = %cost, model = %model, "Per-request cost (flat-rate fallback, model not in models_dev catalog)");
                }
            }

            AgentEvent::IterationComplete { iteration } => {
                // Update the current_turn counter for the "iter N" status pill.
                // (iter-209: current_turn field deleted with FileHistory stub.
                // The iteration count is still tracked via frame_count + the
                // IterationComplete event being published to the debug bus.)
                let _ = iteration;
            }

            AgentEvent::ToolPermissionRequest {
                tool_name,
                description,
                ..
            } => {
                // Permission requests are drained by the dedicated permission_rx
                // task. This event is a duplicate — skip it.
                let _ = (tool_name, description);
            }

            AgentEvent::Notice { text } => {
                // Runtime notices (tool-loop guardrail warnings) are status
                // for the operator, never answer text: a system annotation
                // between rows, not appended to the streaming answer.
                self.push_system_message(text, SystemMessageStyle::Info);
            }

            AgentEvent::BackgroundReview { summary, .. } => {
                // Advisor notes and idle recaps share the BackgroundReview
                // channel (both are side-channel LLM output); the prefix the
                // engine stamps picks the card style.
                // (2026-10-28) A CONTINUE-channel advisor review embeds
                // `[ADVISOR_CONTINUATION]` + the model-visible batch in the
                // summary (omp's steer/continuation delivery, host-triggered):
                // show the card AND queue the batch as the next turn's query.
                // The main loop applies it once the run is complete.
                let marker = operant_core::advisor::CONTINUATION_TAG;
                // The CARD and the CONTINUATION PAYLOAD are two different
                // things in one string: the card is what the operator reads
                // (the notes, one block each), the payload is the machine
                // batch the model must receive. Showing the payload in the
                // transcript dumped the boundary guidance and the raw
                // `<advisory>` XML into the card — the "it lumps everything
                // without correct formatting" report. Split at the marker:
                // the card ends there, the payload starts there.
                let (card, continuation) = match summary.find(marker) {
                    Some(pos) => (
                        summary[..pos].trim_end().to_string(),
                        Some(summary[pos..].trim().to_string()),
                    ),
                    None => (summary.clone(), None),
                };
                if let Some(batch) = continuation
                    && !batch.is_empty()
                {
                    // Keep the marker IN the extracted text: it becomes
                    // part of the submitted query, the conversation carries
                    // it, and the engine's continuation latch (advisor.rs)
                    // greps for it on the next delta — the omp cascade guard.
                    self.pending_advisor_continuation = Some(batch);
                }
                if card.starts_with("⚑") {
                    self.push_system_message(card, SystemMessageStyle::Advisor);
                } else if card.starts_with("※") {
                    self.push_system_message(card, SystemMessageStyle::Recap);
                } else {
                    self.push_system_message(card, SystemMessageStyle::Info);
                }
            }

            AgentEvent::AsyncDelegation {
                delegation_id,
                status,
                summary,
            } => {
                self.push_system_message(
                    format!("Async delegation {delegation_id} {status}: {summary}"),
                    SystemMessageStyle::Info,
                );
            }

            AgentEvent::CompactionStarted { tokens_before } => {
                // Compact style = the compact-boundary marker (teardrop rule),
                // the same lane /compact already renders.
                self.turn_state = TurnState::Compacting;
                self.push_system_message(
                    format!("Compacting context ({tokens_before} tokens) …"),
                    SystemMessageStyle::Compact,
                );
            }

            AgentEvent::CompactionCompleted {
                tokens_before,
                tokens_after,
                messages_before,
                messages_after,
            } => {
                // Compaction is done; the (possibly re-sent) request follows,
                // so the turn is back with the model.
                self.turn_state = TurnState::Thinking;
                self.push_system_message(
                    format!(
                        "Compacted context: {tokens_before} → {tokens_after} tokens, \
                         {messages_before} → {messages_after} messages"
                    ),
                    SystemMessageStyle::Compact,
                );
            }

            AgentEvent::RetryScheduled {
                attempt,
                max_attempts,
                reason,
            } => {
                self.turn_state = TurnState::WaitingForNetwork;
                // The died attempt's partial text must not survive into the
                // retry: the provider restarts the response from its top, so
                // any leftover buffer appends the full retry to the partial
                // and the reply renders duplicated (2026-10-09 live-audit
                // P4-2.5 — the org's provider stream-deaths make this the
                // most-hit path in the field). The turn stays live
                // (`is_streaming` untouched — a mid-turn retry keeps the
                // turn's live-stream display semantics, as the existing
                // test pins); only the died attempt's text is discarded.
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                // Reuse the SystemAPIError block so retries get the same
                // boxed renderer as API failures instead of plain text.
                let block = ContentBlock::SystemAPIError {
                    message: format!("{reason} — retry {attempt}/{max_attempts}"),
                    retry_secs: None,
                };
                self.messages.push(Message::assistant_blocks(vec![block]));
                self.invalidate_transcript();
                self.on_new_message();
            }

            AgentEvent::ModelFallback { from, to, reason } => {
                // The fallback request is being re-sent.
                self.turn_state = TurnState::Sending;
                self.push_system_message(
                    format!("Model fallback: {from} → {to} ({reason})"),
                    SystemMessageStyle::Info,
                );
            }

            AgentEvent::SubagentStarted {
                subagent_id,
                role,
                depth,
            } => {
                self.push_system_message(
                    format!("Subagent {subagent_id} started ({role}, depth {depth})"),
                    SystemMessageStyle::Info,
                );
            }

            AgentEvent::SubagentStopped {
                subagent_id,
                status,
                summary,
            } => {
                self.push_system_message(
                    format!("Subagent {subagent_id} {status}: {summary}"),
                    SystemMessageStyle::Info,
                );
            }

            AgentEvent::TodoUpdated {
                total,
                completed,
                in_progress,
            } => {
                self.push_system_message(
                    format!("Todos: {completed}/{total} done, {in_progress} in progress"),
                    SystemMessageStyle::Info,
                );
            }
            // (plan 05) a real user prompt started — nothing to render; the
            // turn id is for the rewind selector's row attachment, which
            // reads marks from the agent on open.
            AgentEvent::TurnStarted { .. } => {}
        }
    }
}

/// The core's user-interrupt error texts: the run loop's interrupt exit
/// (`agent/run.rs`) and the in-flight abort (`agent/events.rs`), possibly
/// wrapped as "Agent error: …" by the run-completion drain.
fn is_user_interrupt(error: &str) -> bool {
    error
        .trim_start_matches("Agent error: ")
        .starts_with("Interrupted by user")
}

/// Takes the preview from the TAIL, the way codex does it (`compact.rs:114`:
/// `output.lines().rev().take(DETAIL_PREVIEW_LINES)`) — because the line the
/// user wants for a build, a test run, or a migration is the one that says what
/// happened, and that line is LAST. The head-3 form this replaces showed
/// preamble instead: `Compiling`, packrat progress, curl noise.
///
/// Only the slice changes; the `… N more lines` suffix keeps its exact shape so
/// an operator's reading of the row is unchanged, and the count now names the
/// lines dropped from the TOP rather than the ones dropped from the bottom.
///
/// The `(no output)` row is codex's `compact.rs:127`: a successful call that
/// produced nothing used to fall through operant's renderer to an EMPTY tool
/// row — a blank line that reads as nothing rather than as a silent success.
const TOOL_PREVIEW_LINES: usize = 3;

/// The collapsed preview for a finished tool call: the LAST
/// [`TOOL_PREVIEW_LINES`] rows in reading order, plus a `… N more lines` suffix
/// for the head it dropped. An empty success renders as one dim `(no output)`
/// row so the row is not a blank line. An empty failure keeps its empty
/// preview untouched (the renderer invents `✗ failed` for it).
///
/// Before this took the tail it took `all_lines[..3]` — the FIRST three rows,
/// which for a build or a test run is the `Compiling` preamble and never the
/// verdict line.
///
/// `pub(super)` because the session-display converter (a sibling module)
/// builds the same preview for a restored tool row as the live
/// `ToolComplete` path does — one preview rule, both callers.
pub(super) fn tool_output_preview(result_text: &str, is_error: bool) -> String {
    let all_lines: Vec<&str> = result_text.lines().collect();
    if all_lines.is_empty() {
        // Codex says so with a dim row (`compact.rs:127`). The empty-failure
        // case deliberately stays empty: `adapter.rs:395` renders `✗ failed`
        // for it, and replacing that placeholder here would double the marker.
        return if is_error {
            String::new()
        } else {
            "(no output)".to_string()
        };
    }
    let tail_start = all_lines.len().saturating_sub(TOOL_PREVIEW_LINES);
    let mut preview = all_lines[tail_start..].join("\n");
    if tail_start > 0 {
        preview.push_str(&format!("\n\u{2026} {} more lines", tail_start));
    }
    preview
}

impl super::App {
    /// Clears the transcript's tool blocks. A delegation still running then
    /// never reports a completion, so the HUD marks it abandoned (TUI-9).
    pub(crate) fn clear_tool_blocks(&mut self) {
        self.tool_use_blocks.clear();
        for (_, status) in self.agent_status.iter_mut() {
            if status == "running" || status == "queued" {
                *status = "abandoned".to_string();
            }
        }
    }
}
