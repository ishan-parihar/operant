//! Agent event handling methods.

use super::*;

impl App {
    /// Process a query event from the agentic loop.
    /// Handle an AgentEvent from the agent. (iter-114 — replaces
    /// handle_query_event; eliminates the bridge layer.)
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        // Publish to debug bus (no-op when disabled).
        let event_variant = format!("{:?}", std::mem::discriminant(&event));
        let event_summary: String = format!("{:?}", &event).chars().take(80).collect();
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
                self.status_message = Some(format!("Running {}…", name));

                // When a tool starts, flush any accumulated streaming text/thinking
                // as a completed message. This prevents content from accumulating
                // across iterations (think → tool → think → tool → respond).
                // (iter-123 — fixes duplicate thinking/text in multi-iteration turns.)
                if !self.streaming_text.is_empty() || !self.streaming_thinking.is_empty() {
                    self.flush_streamed_assistant_message();
                }

                let turn_index = self.current_user_turn_index();
                let tool_id = tool_call_id.clone();
                let tool_name = name.clone();
                let input_json = arguments;
                if let Some(existing) = self.tool_use_blocks.iter_mut().find(|b| b.id == tool_id) {
                    existing.turn_index = turn_index;
                    existing.status = ToolStatus::Running;
                    existing.output_preview = None;
                    existing.input_json = input_json;
                } else {
                    self.tool_use_blocks.push(ToolUseBlock {
                        id: tool_id.clone(),
                        name: tool_name.clone(),
                        turn_index,
                        status: ToolStatus::Running,
                        output_preview: None,
                        input_json,
                    });
                }

                // Track subagent spawns for the status-bar HUD.
                if tool_name == "delegate_task" || tool_name == "spawn_subagent" {
                    self.agent_status.retain(|(id, _)| id != &tool_id);
                    self.agent_status.push((tool_id, "running".to_string()));
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
                self.status_message = Some(format!("Tool error: {}", result_text));
                // (iter-209: refresh_turn_diff_from_history removed)
            }

            AgentEvent::Done { message } => {
                // Turn complete — the agent finished.
                // (iter-210: fix BACKEND_TUI_AUDIT.md §3 bug #2 — Done.message
                // was previously discarded with `message: _`. If the agent
                // emitted Done without preceding Content events (e.g. a
                // non-streaming error-recovery path), the user saw an empty
                // assistant message. Now: if streaming_text is empty, use
                // Done.message.content as the source of truth.)
                self.is_streaming = false;
                self.spinner_verb = None;

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
                    if let Some(reasoning) = &message.reasoning {
                        if !reasoning.trim().is_empty() {
                            blocks.push(ContentBlock::Thinking {
                                thinking: reasoning.clone(),
                                signature: String::new(),
                            });
                        }
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
                // Mark any remaining Running blocks as Done — they completed
                // but the ToolComplete event either fired before the Done event
                // or was never emitted (fast tool / race condition). Pruning
                // them silently dropped the tool trail from the user's view.
                for block in &mut self.tool_use_blocks {
                    if block.status == ToolStatus::Running {
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
                self.spinner_verb = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.invalidate_transcript();
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
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.record_usage(input_tokens, output_tokens);
                }
                self.cost_usd = self.cost_tracker.total_cost;
                self.token_count = turn_tokens;
                self.check_token_warnings();
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
        }
    }

}
