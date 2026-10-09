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
                self.streaming_thinking.push_str(&content);
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
                    });
                    false
                };
                self.status_message = Some(format!(
                    "{} {}…",
                    if started { "Running" } else { "Queued" },
                    tool_name
                ));

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
                let all_lines: Vec<&str> = result_text.lines().collect();
                let preview_lines = all_lines.len().min(3);
                let mut preview = all_lines[..preview_lines].join("\n");
                let remaining = all_lines.len().saturating_sub(preview_lines);
                if remaining > 0 {
                    preview.push_str(&format!("\n\u{2026} {} more lines", remaining));
                }
                if let Some(block) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    block.status = if is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Done
                    };
                    block.output_preview = Some(preview);

                    if block.name == "delegate_task" || block.name == "spawn_subagent" {
                        let new_status = if is_error { "error" } else { "done" };
                        for (id, st) in self.agent_status.iter_mut() {
                            if id == &tool_id {
                                *st = new_status.to_string();
                            }
                        }
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
                if is_error {
                    self.status_message = Some(format!("Tool error: {}", result_text));
                } else {
                    self.status_message = None;
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
                let all_lines: Vec<&str> = result_text.lines().collect();
                let preview_lines = all_lines.len().min(3);
                let mut preview = all_lines[..preview_lines].join("\n");
                let remaining = all_lines.len().saturating_sub(preview_lines);
                if remaining > 0 {
                    preview.push_str(&format!("\n\u{2026} {} more lines", remaining));
                }
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
                self.status_message = Some(format!("Tool error: {}", result_text));
                // Iteration boundary, same as ToolComplete (P0-2): a failed
                // tool also ends the response that called it.
                self.flush_streamed_assistant_message();
                // (iter-209: refresh_turn_diff_from_history removed)
            }

            AgentEvent::Done { message, .. } => {
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
                if self.streaming_text.trim().is_empty()
                    && self.streaming_thinking.trim().is_empty()
                    && !message.content.is_empty()
                {
                    // Non-streaming path: Done carries the full message.
                    let mut blocks = Vec::new();
                    if let Some(reasoning) = &message.reasoning
                        && !reasoning.trim().is_empty()
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

                // Show a "copy" hint after each response so the user knows
                // they can copy the last response with /copy.
                // (iter-122 — user-requested: copy button at end of response.)
                self.push_notification(
                    NotificationKind::Info,
                    "Response complete · /copy to copy · Ctrl+J for line break".to_string(),
                    Some(4),
                );
            }

            AgentEvent::Error { error } => {
                self.is_streaming = false;
                self.turn_state = TurnState::Idle;
                self.spinner_verb = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.invalidate_transcript();
                // Give the prompt back before the error is surfaced. This is
                // the arm the error modal is drawn from, so the message and the
                // recovery land in the same frame.
                self.restore_failed_input_to_composer();
                let err_msg = format!("Error: {}", error);
                self.push_assistant_message(err_msg.clone());
                self.push_notification(NotificationKind::Error, err_msg, None);
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
                self.context_used_tokens =
                    self.context_used_tokens.saturating_add(turn_tokens as u64);
                // Per-turn footer deltas. Cache tokens aren't reported by this
                // event, so the cache counters stay at 0 and the footer hides
                // them (see the App field docs).
                self.record_turn_usage(input_tokens, output_tokens);
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.record_usage(input_tokens, output_tokens);
                }
                self.cost_usd = self.cost_tracker.total_cost;
                self.token_count = turn_tokens;
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
                self.push_notification(NotificationKind::Warning, msg, None);
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

            AgentEvent::BackgroundReview { summary } => {
                self.push_system_message(summary, SystemMessageStyle::Info);
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
        }
    }
}
