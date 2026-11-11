//! `stream` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::{ChatResponse, Message, ToolCall, Usage};
use crate::error::{Error, Result};
use crate::observer::{Observer, ObserverEvent};
use crate::org::seat_policy::SeatDecision;
use crate::parser::{ToolCallParser, ToolCallStreamParser};
use crate::tools::{ToolContext, ToolResult};
use futures::StreamExt;
use futures::stream::BoxStream;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use super::*;

impl OperantAgent {
    /// The tool context every tool call runs with: session-id keyed (Plan
    /// 015) and, for unattended runs, flagged so interactive tools can fail
    /// fast instead of routing into a human's chat (BUGS.md D-6 — the
    /// user-question channel is process-global and the gateway drains it
    /// into whatever chat is active; a scheduled job must never block on
    /// it).
    ///
    /// `tool_name` also decides the one approval flag the tools read
    /// themselves: when the operator's allowlist covers this tool, the
    /// context says so. Without it the self-extension tools
    /// (`harness_mount` / `harness_unmount`) could never run — they are not
    /// permission-gated, so they are never prompted for, and the flag they
    /// gate on was never set by anyone. An operator who approved
    /// `harness_mount` in the allowlist got a refusal that told them to
    /// approve it in the allowlist.
    pub(crate) fn tool_context(&self, tool_name: &str) -> ToolContext {
        let mut ctx = ToolContext::default().with_metadata(
            "session_id",
            self.session_id().unwrap_or_else(|| "default".to_string()),
        );
        if self.unattended {
            ctx = ctx.with_metadata("unattended", "true");
        }
        if self.tool_allowed_by_allowlist(tool_name) {
            ctx = ctx.with_metadata("approval", "true");
        }
        ctx
    }

    /// Record one completed tool call's wall-clock duration under its call ID
    /// (iter-1505). Feeds the `ObserverEvent::ToolCall` emit in `run.rs`, which
    /// pairs one tool name with one duration and so cannot use
    /// `TurnEndBus::tool_durations_ms` — that is a name-less `Vec<u64>` in
    /// completion order.
    ///
    /// Silently does nothing when the lock is poisoned, matching
    /// `TurnEndBus::record_tool_duration`: a lost timing must never fail a
    /// turn that otherwise succeeded.
    pub(crate) fn record_tool_duration(&self, tool_call_id: &str, ms: u64) {
        if let Ok(mut map) = self.tool_durations_by_call.lock() {
            map.insert(tool_call_id.to_string(), ms);
        }
    }

    /// The recorded duration for one tool call, or `Duration::ZERO` when the
    /// call was never timed (an early-return path that skipped the executor).
    pub(crate) fn tool_duration_for(&self, tool_call_id: &str) -> Duration {
        self.tool_durations_by_call
            .lock()
            .ok()
            .and_then(|map| map.get(tool_call_id).copied())
            .map(Duration::from_millis)
            .unwrap_or(Duration::ZERO)
    }

    /// Total time this turn spent inside tools, or `None` when no tool call
    /// was ever timed.
    ///
    /// Row 3 (RT-209). The per-call map is keyed by id so the observer can pair
    /// ONE name with ONE duration; attribution needs the per-TURN total instead,
    /// which is a sum, not a lookup. Returning `None` rather than
    /// `Duration::ZERO` when nothing was timed is the whole point: "no tools ran"
    /// and "tools took no time" are different answers, and the second is the RT-199
    /// bug.
    ///
    /// Note the sum is of CONCURRENT executions: the 8-worker pool runs tools
    /// in parallel, so a turn's tools span can exceed its wall clock. That is
    /// correct (it is what the model waited on, roughly) but not exclusive, so
    /// `TurnBreakdown::is_exclusive()` reports `false` for any turn with more
    /// than one tool in it rather than pretending the arithmetic partitions.
    pub(crate) fn turn_tool_duration(&self) -> Option<Duration> {
        let map = self.tool_durations_by_call.lock().ok()?;
        if map.is_empty() {
            return None;
        }
        Some(Duration::from_millis(map.values().sum()))
    }

    /// Park the rendered arguments' byte size for one in-flight call
    /// (iter-1551). The arguments exist as a string only at the
    /// `ToolCallStart` emit site, and `ToolResult` carries the output but not
    /// the input, so the size is carried here until the call completes.
    ///
    /// Silent on a poisoned lock, matching `record_tool_duration`: a lost byte
    /// count must never fail a turn that otherwise succeeded.
    pub(crate) fn record_tool_input_bytes(&self, tool_call_id: &str, bytes: usize) {
        if let Ok(mut map) = self.tool_input_bytes_by_call.lock() {
            map.insert(tool_call_id.to_string(), bytes);
        }
    }

    /// The parked input size for one call, or 0 when it was never recorded (an
    /// early-return path that skipped the executor).
    pub(crate) fn tool_input_bytes_for(&self, tool_call_id: &str) -> usize {
        self.tool_input_bytes_by_call
            .lock()
            .ok()
            .and_then(|map| map.get(tool_call_id).copied())
            .unwrap_or(0)
    }

    /// Drop the per-turn tool-duration map (iter-1505). Called at the start of
    /// `run()` alongside `stream_emitted_tool_starts`, so a long-lived agent
    /// does not accumulate one entry per tool call for its whole life.
    pub(crate) fn clear_tool_durations(&self) {
        if let Ok(mut map) = self.tool_durations_by_call.lock() {
            map.clear();
        }
        if let Ok(mut map) = self.tool_input_bytes_by_call.lock() {
            map.clear();
        }
    }

    /// Process streaming response with early tool detection
    pub(crate) async fn process_stream(
        &self,
        mut stream: BoxStream<'static, Result<StreamChunk>>,
    ) -> Result<(
        String,
        String,
        Vec<ToolCall>,
        Option<serde_json::Value>,
        Option<String>,
        u64,
        Option<String>,
    )> {
        let mut accumulated_extra: Option<serde_json::Value> = None;
        let mut parser = ToolCallStreamParser::new().on_tool_call(|tc| {
            let tc_id = tc.id.clone();
            debug!(tool_call_id = %tc_id, name = %tc.function.name, "Early tool call detected");
        });
        let mut content_router = ThinkBlockRouter::default();
        let mut tool_call_router = ToolCallContentRouter::default();
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // Streaming usage arrives split across chunks (Anthropic reports
        // input_tokens on message_start and output_tokens on message_delta;
        // OpenAI-compatible providers report both together on one trailing
        // chunk when stream_options.include_usage is set). Track whichever
        // halves have arrived and only treat usage as complete once both
        // are known.
        let mut usage_prompt_tokens: Option<u32> = None;
        let mut served_model: Option<String> = None;
        let mut usage_completion_tokens: Option<u32> = None;
        // Capture the original stream error so we can surface it (instead of
        // the generic "Stream processing failed" string) and decide whether
        // to flush partials after the loop.
        let mut stream_error: Option<Error> = None;
        // Provider-reported finish reason from the terminal chunk(s) (T1 —
        // truncation detection). Last non-None wins.
        let mut finish_reason: Option<String> = None;
        // Corrupted-SSE accounting: the openai adapter's terminal chunk
        // carries the count of payloads the parser dropped (merged data
        // lines, concatenated JSON — anything unparseable). Surfaced as one
        // StreamCorrupted event at stream end so the user sees WHY the turn
        // looks truncated instead of the agent eating the blame silently.
        let mut dropped_events: u32 = 0;

        // Per-chunk idle limit (the loop request budget, reasoning floor
        // included): a stream may run as long as it keeps producing data, but
        // a silent one ends as a retryable stall. Replaces the old total
        // ceiling around this whole call, which cut long healthy streams.
        let idle = self.loop_request_timeout();
        loop {
            let chunk_result = match tokio::time::timeout(idle, stream.next()).await {
                Ok(Some(chunk_result)) => chunk_result,
                Ok(None) => break,
                Err(_) => {
                    warn!(
                        idle_secs = idle.as_secs(),
                        "Stream stalled — no chunk within the idle limit"
                    );
                    stream_error = Some(Error::StreamStalled {
                        idle_secs: idle.as_secs(),
                    });
                    break;
                }
            };
            self.note_stream_progress();
            // `/stop` has to be able to cut a stream that is already flowing.
            // Without this the SSE loop ran to completion regardless of the flag,
            // so a stop only took effect between tool calls — which is exactly the
            // reported symptom that `/stop` did not stop the streaming. Breaking
            // here falls through to the flush below, which runs on every exit
            // path, so whatever had already arrived is returned rather than
            // discarded: the user sees the partial answer instead of silence.
            if self.interrupt_flag.is_triggered() {
                tracing::info!("Stream interrupted by user; flushing partial content");
                break;
            }
            match chunk_result {
                Ok(chunk) => {
                    if let Some(n) = chunk.dropped_events {
                        dropped_events = dropped_events.saturating_add(n);
                    }
                    if let Some(model) = chunk.model.as_ref()
                        && !model.is_empty()
                    {
                        served_model = Some(model.clone());
                    }
                    if let Some(u) = chunk.usage {
                        if u.prompt_tokens > 0 {
                            usage_prompt_tokens = Some(u.prompt_tokens);
                        }
                        if u.completion_tokens > 0 {
                            usage_completion_tokens = Some(u.completion_tokens);
                        }
                    }

                    // Process reasoning from StreamChunk.
                    // If the provider sends reasoning natively (via
                    // reasoning_content), use that and DON'T also extract
                    // reasoning from content text — otherwise the same
                    // reasoning appears twice. (iter-123 — fixes duplicate
                    // thinking bug.)
                    let has_native_reasoning = chunk.reasoning.is_some();
                    if let Some(reasoning) = chunk.reasoning {
                        // Preserve \n (reasoning may double as message content
                        // when the final answer is empty); only CR is stripped.
                        let reasoning = reasoning.replace('\r', "");
                        let reasoning = strip_reasoning_tags(&reasoning);
                        if !reasoning.is_empty() {
                            accumulated_reasoning.push_str(&reasoning);
                            self.emit(AgentEvent::Reasoning { text: reasoning }).await;
                        }
                    }

                    // Capture provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = chunk.extra_content
                        && !extra.is_null()
                    {
                        accumulated_extra = Some(extra.clone());
                    }

                    // Process content from StreamChunk
                    // Sanitize provider streaming text: strip carriage returns
                    // (they corrupt terminal display by moving the cursor back
                    // to column 0). Newlines are PRESERVED — they carry the
                    // markdown structure (headers, tables, code fences,
                    // blockquotes) that the Telegram/Discord renderers depend
                    // on. (iter-263 replaced every \n with a space to mask a
                    // provider mid-word newline quirk; that collapsed ALL
                    // gateway responses into single-line blobs and broke every
                    // markdown layout.)
                    if let Some(text) = chunk.content {
                        let text = text.replace('\r', "");
                        let (content_delta, reasoning_delta) = content_router.feed(&text);

                        if !content_delta.is_empty() {
                            let chunk_tool_calls = parser.process_chunk(&content_delta);
                            // Collect new tool calls that need ToolStart emitted.
                            // Lock is dropped before each .await to satisfy Send bounds.
                            let mut pending_tool_starts: Vec<(String, String, String)> = Vec::new();
                            for tc in chunk_tool_calls {
                                if !tool_calls.iter().any(|existing| existing.id == tc.id) {
                                    let already = self
                                        .stream_emitted_tool_starts
                                        .lock()
                                        .map(|mut s| !s.insert(tc.id.clone()))
                                        .unwrap_or(true);
                                    if !already {
                                        pending_tool_starts.push((
                                            tc.id.clone(),
                                            tc.function.name.clone(),
                                            tc.function.arguments.clone(),
                                        ));
                                    }
                                    tool_calls.push(tc);
                                }
                            }
                            // Emit ToolStart outside the lock — chronological
                            // message splitting parity.
                            for (id, name, args) in pending_tool_starts {
                                self.emit(AgentEvent::ToolStart {
                                    tool_call_id: id,
                                    name,
                                    arguments: args,
                                })
                                .await;
                            }

                            let visible_text = tool_call_router.feed(&content_delta);
                            if !visible_text.is_empty() {
                                let scrubbed = strip_memory_context_tags(&visible_text);
                                if !scrubbed.is_empty() {
                                    accumulated_text.push_str(&scrubbed);
                                    self.emit(AgentEvent::Content { text: scrubbed }).await;
                                }
                            }
                        }

                        // Only emit reasoning from content_router if the
                        // provider didn't already send it natively. This
                        // prevents duplicate thinking. (iter-123)
                        if !has_native_reasoning && !reasoning_delta.is_empty() {
                            accumulated_reasoning.push_str(&reasoning_delta);
                            self.emit(AgentEvent::Reasoning {
                                text: reasoning_delta,
                            })
                            .await;
                        }
                    }

                    // Merge native provider tool-call deltas
                    if let Some(chunk_tool_calls) = chunk.tool_calls {
                        let mut native_pending: Vec<(String, String, String)> = Vec::new();
                        for tc in chunk_tool_calls {
                            // RT-466: the lookup after the merge found the
                            // FIRST call with a non-empty name, not the one
                            // that was just merged — so an SSE event
                            // carrying two native tool calls announced the
                            // first and left the second without a
                            // ToolStart (its row appeared late, or never
                            // when the turn was interrupted first). The
                            // XML flush path already used the id.
                            let tc_id = tc.id.clone();
                            let is_new = !tool_calls.iter().any(|e| e.id == tc_id);
                            let has_name = !tc.function.name.is_empty();
                            merge_stream_tool_call(&mut tool_calls, tc);
                            if is_new
                                && has_name
                                && let Some(full) = tool_calls.iter().find(|e| e.id == tc_id)
                            {
                                let id = full.id.clone();
                                let already = self
                                    .stream_emitted_tool_starts
                                    .lock()
                                    .map(|mut s| !s.insert(id.clone()))
                                    .unwrap_or(true);
                                if !already {
                                    native_pending.push((
                                        id,
                                        full.function.name.clone(),
                                        full.function.arguments.clone(),
                                    ));
                                }
                            }
                        }
                        for (id, name, args) in native_pending {
                            self.emit(AgentEvent::ToolStart {
                                tool_call_id: id,
                                name,
                                arguments: args,
                            })
                            .await;
                        }
                    }

                    // Capture the provider finish reason (T1).
                    if let Some(fr) = &chunk.finish_reason {
                        finish_reason = Some(fr.clone());
                    }
                }
                Err(e) => {
                    error!(error = %e, "Stream error");
                    // Capture the original error so we can surface it after
                    // flushing partials. Previously the error was swallowed
                    // and replaced with a generic "Stream processing failed"
                    // string, making debugging impossible.
                    stream_error = Some(e);
                    break;
                }
            }
        }

        // Flush any partial content/tool_calls still buffered in the routers
        // and parser. This runs on both success AND error paths so partial
        // tool calls (e.g. a tool_use block that started but didn't finish
        // before the stream broke) are still extracted and returned to the
        // caller. Previously the error path `break`ed before this flush,
        // dropping all partials.
        let (remaining_content, remaining_reasoning) = content_router.finish();
        if !remaining_content.is_empty() {
            let remaining_calls = parser.process_chunk(&remaining_content);
            let mut flush_pending: Vec<(String, String, String)> = Vec::new();
            for tc in remaining_calls {
                if !tool_calls.iter().any(|existing| existing.id == tc.id) {
                    let already = self
                        .stream_emitted_tool_starts
                        .lock()
                        .map(|mut s| !s.insert(tc.id.clone()))
                        .unwrap_or(true);
                    if !already {
                        flush_pending.push((
                            tc.id.clone(),
                            tc.function.name.clone(),
                            tc.function.arguments.clone(),
                        ));
                    }
                }
                merge_stream_tool_call(&mut tool_calls, tc);
            }
            for (id, name, args) in flush_pending {
                self.emit(AgentEvent::ToolStart {
                    tool_call_id: id,
                    name,
                    arguments: args,
                })
                .await;
            }
            let visible = tool_call_router.feed(&remaining_content);
            if !visible.is_empty() {
                accumulated_text.push_str(&visible);
                // Emit the flushed partial so the TUI sees it even if we're
                // about to return Err — otherwise content streamed right
                // before the error would be silently lost.
                self.emit(AgentEvent::Content {
                    text: strip_memory_context_tags(&visible),
                })
                .await;
            }
        }
        let tail = tool_call_router.finish();
        if !tail.is_empty() {
            let scrubbed_tail = strip_memory_context_tags(&tail);
            accumulated_text.push_str(&scrubbed_tail);
            self.emit(AgentEvent::Content {
                text: scrubbed_tail,
            })
            .await;
        }
        if !remaining_reasoning.is_empty() {
            accumulated_reasoning.push_str(&remaining_reasoning);
            self.emit(AgentEvent::Reasoning {
                text: remaining_reasoning,
            })
            .await;
        } // Also try to extract any remaining tool calls from accumulated text.
        // On the error path we don't want a parser failure to mask the
        // original stream error, so fall back to an empty vec.
        // Normalize CR/CRLF before final processing. Newlines are preserved
        // (markdown structure) — only carriage returns are stripped, matching
        // the per-chunk sanitization above.
        accumulated_text = accumulated_text.replace('\r', "");
        accumulated_reasoning = accumulated_reasoning.replace('\r', "");
        let mut remaining_parser = ToolCallParser::new();
        let remaining_calls = if stream_error.is_some() {
            remaining_parser
                .parse(&accumulated_text)
                .unwrap_or_default()
        } else {
            remaining_parser.parse(&accumulated_text)?
        };

        // Merge tool calls, avoiding duplicates
        for tc in remaining_calls {
            merge_stream_tool_call(&mut tool_calls, tc);
        }

        // (iter-261 pre-validated arguments here and dropped irreparable
        // calls; B2/iter-602 removed that retain — unrepairable args must
        // reach `execute_tools`, which answers them with a named
        // not-executed error instead of silently dropping the call.)

        if let Some(err) = stream_error {
            // Surface the ORIGINAL stream error (e.g. reqwest's "error
            // decoding response body" when a provider closes the SSE
            // connection mid-body) rather than wrapping it in a generic
            // "Stream processing failed" Agent error. The raw variant
            // (`Error::Network`) classifies as retryable, which the run()
            // loop uses to re-issue the request (hermes-parity stream-drop
            // recovery). Trajectory saving is handled by the caller (run())
            // when this error propagates up.
            //
            // R2: a transport failure on a known reasoning model with NO
            // content arrived yet is a thinking-timeout (upstream idle-killed
            // the thinking phase). Record it so the run loop can annotate the
            // final error with guidance once retries are exhausted.
            if crate::reasoning_timeouts::is_thinking_timeout(&self.model(), &err.to_string())
                && accumulated_text.is_empty()
                && accumulated_reasoning.is_empty()
                && tool_calls.is_empty()
            {
                self.thinking_timeout_hit
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                self.emit(AgentEvent::Content {
                    text: "⚠ The model's thinking phase may have exceeded the upstream idle timeout — retrying."
                        .to_string(),
                })
                .await;
            }
            if dropped_events > 0 {
                self.emit(AgentEvent::StreamCorrupted { dropped_events })
                    .await;
            }
            if let Some(prompt_tokens) = usage_prompt_tokens {
                let completion_tokens = usage_completion_tokens.unwrap_or(0);
                let usage = Usage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                };
                self.emit_usage_and_cost(
                    &usage,
                    served_model.as_deref().unwrap_or(&self.effective_model()),
                )
                .await;
            }
            return Err(err);
        }

        let usage_tokens: u64 = match (usage_prompt_tokens, usage_completion_tokens) {
            (Some(p), Some(c)) => u64::from(p) + u64::from(c),
            _ => 0,
        };
        // iter-247: emit Usage/Cost for streaming the same way
        // process_response does for non-streaming, now that both usage
        // halves are available. If the provider never sent usage data (or
        // only sent one half), silently skip rather than reporting
        // incomplete numbers.
        if let (Some(prompt_tokens), Some(completion_tokens)) =
            (usage_prompt_tokens, usage_completion_tokens)
        {
            let usage = Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            };
            self.emit_usage_and_cost(
                &usage,
                served_model.as_deref().unwrap_or(&self.effective_model()),
            )
            .await;
        }

        if dropped_events > 0 {
            self.emit(AgentEvent::StreamCorrupted { dropped_events })
                .await;
        }

        Ok((
            accumulated_text,
            accumulated_reasoning,
            tool_calls,
            accumulated_extra,
            finish_reason,
            usage_tokens,
            served_model,
        ))
    }

    pub(crate) async fn process_response(
        &self,
        response: ChatResponse,
    ) -> Result<(String, String, Vec<ToolCall>, Option<String>, u64)> {
        let mut choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::ParseResponse("response had no choices".to_string()))?;
        // Provider-reported finish reason (T1 — truncation detection).
        let finish_reason = choice.finish_reason.take();

        let message = choice.message;
        let raw_content = message.content.unwrap_or_default();
        let content = strip_tool_call_markup(&raw_content);
        let reasoning = message
            .reasoning_content
            .map(|value| strip_reasoning_tags(&value))
            .unwrap_or_default();
        let mut tool_calls = extract_tool_calls_from_choice(message.tool_calls);
        let mut xml_parser = ToolCallParser::new();
        if let Ok(xml_tool_calls) = xml_parser.parse(&raw_content) {
            for tool_call in xml_tool_calls {
                merge_stream_tool_call(&mut tool_calls, tool_call);
            }
        }

        if !content.is_empty() {
            self.emit(AgentEvent::Content {
                text: strip_memory_context_tags(&content),
            })
            .await;
        }
        if !reasoning.is_empty() {
            self.emit(AgentEvent::Reasoning {
                text: reasoning.clone(),
            })
            .await;
        }

        self.emit_usage_and_cost(&response.usage, &response.model)
            .await;

        let usage_tokens = u64::from(response.usage.total_tokens);
        Ok((content, reasoning, tool_calls, finish_reason, usage_tokens))
    }

    /// Wave-2 harvest: feed real executed results to the guardrail
    /// tracker's post-execution rungs — the no-progress ladder (identical-
    /// result streak + run-wide recurrence ledger), the per-tool failure
    /// ladder (warn 8 / halt 12), and the hard-reject halt (2). Synthetic
    /// guardrail skips are excluded — their text is identical by
    /// construction and would fake an identical-result streak. A Halt
    /// verdict triggers the interrupt flag and surfaces the root-cause
    /// summary as final content.
    async fn observe_guardrail_results(&self, results: &mut [ToolResult]) {
        use crate::tool_guardrails::{GuardrailDecision, RepeatPattern};

        // Phase 1 (sync, under the lock): classify every result. The lock
        // is dropped before any await so no other arm can block on it.
        let mut notices: Vec<(usize, String, GuardrailDecision, Option<RepeatPattern>)> =
            Vec::new();
        {
            let mut g = self
                .tool_guardrails
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (idx, r) in results.iter().enumerate() {
                if r.content
                    .starts_with(crate::tool_guardrails::SKIP_MESSAGE_PREFIX)
                {
                    continue;
                }
                let decision = g.observe_result(&r.name, &r.content, r.success);
                let is_halt = matches!(decision, GuardrailDecision::Halt(_));
                let pattern = g.last_pattern();
                notices.push((idx, r.name.clone(), decision, pattern));
                if is_halt {
                    break;
                }
            }
        }

        // Phase 2 (async, lock-free). hermes `_append_guardrail_observation`:
        // the guidance rides the tool RESULT the model reads next — a warning
        // the model never sees cannot change its course. The operator gets a
        // Notice (a system annotation), never answer text.
        for (idx, name, decision, pattern) in notices {
            match decision {
                GuardrailDecision::Allow | GuardrailDecision::Skip => {}
                GuardrailDecision::Warn => {
                    warn!(
                        tool = %name,
                        pattern = ?pattern,
                        "Post-result guardrail ladder — warning model"
                    );
                    let message =
                        crate::tool_guardrails::pattern_warning_message(&name, pattern.as_ref());
                    append_guidance(&mut results[idx], "Tool loop warning", &message);
                    self.emit(AgentEvent::Notice { text: message }).await;
                }
                GuardrailDecision::Halt(summary) => {
                    warn!(
                        tool = %name,
                        "Guardrail ladder exhausted — halting the turn: {summary}"
                    );
                    append_guidance(&mut results[idx], "Tool loop hard stop", &summary);
                    if let Ok(mut halt) = self.guardrail_halt.lock() {
                        halt.get_or_insert(summary);
                    }
                    return;
                }
            }
        }
    }

    /// Take the guardrail halt recorded this iteration, if any.
    pub(crate) fn take_guardrail_halt(&self) -> Option<String> {
        self.guardrail_halt.lock().ok().and_then(|mut h| h.take())
    }

    /// Guardian-LLM classification for one flagged tool call (hermes
    /// `approval_smart.py` parity): strip comments, wrap the command as
    /// data, one small non-streaming chat, STRICT verdict parse.
    /// `None` = call failed or reply unparseable → the caller treats it as
    /// ESCALATE (today's channel behavior). The command extraction reuses
    /// the smart gate's own extractor so the classifier and the gate can
    /// never disagree about what the command is.
    async fn guardian_classify(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<crate::approval::GuardianVerdict> {
        use crate::approval::{
            GuardianVerdict, guardian_messages, parse_guardian_reply, strip_shell_comments,
        };
        let command = strip_shell_comments(&crate::approval::extract_command_from_args(name, args));
        if command.trim().is_empty() {
            return Some(GuardianVerdict::Escalate);
        }
        let request = crate::agent::ChatRequest::new(
            self.config.model.clone(),
            guardian_messages(name, &command),
        );
        let request = ChatRequest {
            max_tokens: Some(32),
            temperature: Some(0.0),
            ..request
        };
        match self.client.chat(request).await {
            Ok(response) => {
                let text = response
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_deref())
                    .unwrap_or("");
                let verdict = parse_guardian_reply(text);
                debug!(tool = %name, ?verdict, "Guardian-LLM classified");
                Some(verdict)
            }
            Err(e) => {
                warn!(tool = %name, error = %e, "Guardian-LLM call failed — escalating");
                None
            }
        }
    }

    /// Feed one completed iteration to the output-side successful-repeat
    /// rung (openhuman `record_output`, iter-688): the canonical signature
    /// of the assistant's narration + tool batch, hashed by the caller
    /// BEFORE the batch moved into [`Self::execute_tools`]. A Warn verdict
    /// surfaces the repetition warning as feed content; the skip backstop
    /// lands on the next `observe()` call.
    ///
    /// Synthetic guardrail skips are excluded from the success check (a
    /// batch that was partially skipped is not clean progress), and an
    /// all-exempt (polling) batch resets the streak — a poller re-narrating
    /// itself is legitimate progress.
    pub(crate) async fn observe_iteration_output(
        &self,
        signature: u64,
        narration: &str,
        results: &[ToolResult],
    ) {
        use crate::tool_guardrails::{self, GuardrailDecision};

        let decision = {
            let mut g = self
                .tool_guardrails
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let batch_exempt = results.iter().all(|r| g.is_exempt(&r.name));
            let mut saw_real = false;
            let mut all_ok = true;
            for r in results {
                if r.content.starts_with(tool_guardrails::SKIP_MESSAGE_PREFIX) {
                    continue;
                }
                saw_real = true;
                if !r.success {
                    all_ok = false;
                }
            }
            let claimed_progress = tool_guardrails::text_claims_progress(narration);
            let batch_successful = saw_real && all_ok;
            g.observe_output(
                signature,
                batch_successful,
                batch_exempt,
                claimed_progress,
                saw_real,
            )
        };
        if matches!(decision, GuardrailDecision::Warn) {
            let (count, is_progress_token) = {
                let g = self
                    .tool_guardrails
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                g.last_pattern()
                    .map(|p| match p {
                        crate::tool_guardrails::RepeatPattern::IdenticalOutput { count } => {
                            (count, false)
                        }
                        crate::tool_guardrails::RepeatPattern::ProgressTokenStall { count } => {
                            (count, true)
                        }
                        _ => (0, false),
                    })
                    .unwrap_or((0, false))
            };
            let warning = if is_progress_token {
                crate::tool_guardrails::progress_token_stall_warning_message(count)
            } else {
                crate::tool_guardrails::output_repeat_warning_message(count)
            };
            warn!(
                count,
                is_progress_token, "Output-side repeat/stall rung — warning model"
            );
            self.emit(AgentEvent::Content { text: warning }).await;
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "masked_tools is held only for short non-panicking sections; poisoning is treated as a bug"
    )]
    /// Execute tools and handle self-healing
    pub(crate) async fn execute_tools(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        // ── Concurrent tool execution (iter-56) ──────────────────────────
        // Previously this was a sequential for-loop. Now it's two phases:
        //
        // Phase 1 (sequential): Pre-flight checks — interrupt flag, arg
        //   parsing, tool validation, approval gate, permission prompts.
        //   These MUST be sequential because permission prompts are
        //   interactive (the user sees one dialog at a time).
        //
        // Phase 2 (concurrent): Execute all approved tools concurrently
        //   using FuturesUnordered with a semaphore (max 8, matching
        //   hermes's _MAX_TOOL_WORKERS). Independent tool calls (e.g.
        //   4 web searches) now run in parallel instead of serially.
        //
        // Results are collected in the SAME ORDER as the input tool_calls
        // (the model expects results in the same order as the calls).

        use futures::stream::{self, StreamExt};
        use std::sync::Arc as StdArc;
        use tokio::sync::Semaphore;

        // ── Phase 1: Pre-flight (sequential) ────────────────────────────
        let mut pending: Vec<(usize, ToolCall, serde_json::Value)> = Vec::new();
        let mut early_results: Vec<Option<ToolResult>> = vec![None; tool_calls.len()];
        // Pre-flight rejections build results with no name; the guardrail
        // keys its ladders on the name, so every rejection used to pool under
        // tool "" (2026-10-13: "Tool '' has failed 12 times"). Stamped back
        // from the call at collection.
        let call_names: Vec<String> = tool_calls.iter().map(|c| c.function.name.clone()).collect();
        // T6: within-batch dedupe (hermes `_deduplicate_tool_calls`) — only
        // the first occurrence of each (tool, arguments) pair in a single
        // assistant message executes; exact duplicates are skipped with a
        // synthetic result so result ordering is preserved and degenerate
        // batches don't double-mutate.
        let mut seen_tool_calls: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        // Pre-flight failure-streak gate bookkeeping: parse-refusals from
        // EARLIER calls in this same batch, per tool. The post-result
        // ladder observes results only after the batch drains, so without
        // this counter a 64-call parallel batch would answer every call
        // with a refusal before the gate could say stop (RC3, plan
        // 2026-10-25 tooling-surface).
        let mut batch_parse_fails: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for (idx, mut tool_call) in tool_calls.into_iter().enumerate() {
            // Check interrupt flag
            if self.interrupt_flag.is_triggered() {
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    "Skipped: interrupted by user (Ctrl-C)".to_string(),
                ));
                continue;
            }

            // A call with no function name cannot run; say so plainly instead
            // of a "tool not found" for the empty string.
            if tool_call.function.name.trim().is_empty() {
                warn!("Tool call with an empty function name — refused");
                early_results[idx] = Some(ToolResult::error_with_name(
                    "(unnamed)",
                    &tool_call.id,
                    "This tool call had no function name, so nothing ran. Re-issue it \
                     with the exact name of the tool you meant to call."
                        .to_string(),
                ));
                continue;
            }

            // T6: skip exact duplicates within this batch.
            let dup_key = (
                tool_call.function.name.clone(),
                tool_call.function.arguments.trim().to_string(),
            );
            if !seen_tool_calls.insert(dup_key) {
                warn!(
                    tool = %tool_call.function.name,
                    "Duplicate tool call with identical arguments — skipped (T6 within-batch dedupe)"
                );
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    format!(
                        "Duplicate tool call '{}' with identical arguments skipped — already \
                         invoked once in this batch.",
                        tool_call.function.name
                    ),
                ));
                continue;
            }

            // ── S2 timeout-breaker mask ────────────────────────────────
            // A tool that timed out 3× consecutively this turn is refused
            // here WITHOUT executing — no fourth timeout burn — and the
            // model gets a named error it can act on. The schema list also
            // omits it (`tools_for_turn`), so a well-behaved model never
            // reaches this arm; this is the backstop for one that calls it
            // anyway.
            if self
                .masked_tools
                .lock()
                .expect("masked_tools lock poisoned")
                .contains(&tool_call.function.name)
            {
                warn!(
                    tool = %tool_call.function.name,
                    "Tool masked by the timeout breaker — refusing call without executing"
                );
                early_results[idx] = Some(ToolResult::error_with_name(
                    &tool_call.function.name,
                    &tool_call.id,
                    format!(
                        "Tool '{}' is disabled for the rest of this turn after repeated \
                          timeouts — take a different approach.",
                        tool_call.function.name
                    ),
                ));
                continue;
            }

            // ── Pre-flight failure-streak gate ─────────────────────────
            // The post-result ladder only informs the NEXT iteration: the
            // 2026-10-24 storm dispatched 64 parallel `process` calls and
            // executed 32 of them before the ladder halted at 12 (plan
            // 2026-10-25 tooling-surface, RC3). Gate the batch itself:
            // once this tool has failed the warn threshold's worth of
            // times — from earlier turns (the live streak) or from
            // parse-refusals earlier in THIS batch — later same-tool
            // calls in the batch are answered without executing. The
            // skip text carries the guardrail prefix, so the post-batch
            // observation excludes it (no double count).
            {
                let global_streak = self
                    .tool_guardrails
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .failure_streak(&tool_call.function.name);
                let batch_fails = batch_parse_fails
                    .get(&tool_call.function.name)
                    .copied()
                    .unwrap_or(0);
                let prior_fails = global_streak + batch_fails;
                if prior_fails >= crate::tool_guardrails::FAILURE_WARN_THRESHOLD {
                    warn!(
                        tool = %tool_call.function.name,
                        prior_fails,
                        "Batch streak gate — refusing call without executing"
                    );
                    early_results[idx] = Some(ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        name: tool_call.function.name.clone(),
                        // A deliberate guardrail skip is guidance, not a
                        // tool failure: success-shaped like the
                        // GuardrailDecision::Skip arm below, with the
                        // prefix in `content` so the post-batch observation
                        // excludes it. The skip text in `error` (the
                        // error_with_name shape) would be observed as a
                        // failure and the ladder would halt on skips.
                        success: true,
                        content: format!(
                            "{}tool '{}' has already failed {prior_fails} times in a row this \
                             turn and the rest of this batch's calls to it are skipped. Take \
                             a different approach or report back instead of retrying.",
                            crate::tool_guardrails::SKIP_MESSAGE_PREFIX,
                            tool_call.function.name
                        ),
                        error: None,
                        timed_out: false,
                    });
                    continue;
                }
            }

            let mut name = tool_call.function.name.clone();
            let raw_args = tool_call.function.arguments.clone();
            let trimmed = raw_args.trim();
            let args_str = if trimmed.is_empty() {
                "{}".to_string()
            } else {
                raw_args
            };

            debug!(tool = %name, args = %args_str, "Executing tool");

            // Emit ToolCallStart observer event
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::ToolCallStart {
                    tool: name.clone(),
                    arguments: Some(args_str.clone()),
                });
                // Park the input size for the completion emit, where
                // `ObserverEvent::ToolCall` reports both ends of the payload.
                self.record_tool_input_bytes(&tool_call.id, args_str.len());
            }

            // Skip emitting ToolStart if it was already emitted during
            // streaming (process_stream extracts XML tool calls and emits
            // ToolStart early so the gateway can split messages chronologically).
            let already_streamed = self
                .stream_emitted_tool_starts
                .lock()
                .map(|s| s.contains(&tool_call.id))
                .unwrap_or(false);
            if !already_streamed {
                self.emit(AgentEvent::ToolStart {
                    tool_call_id: tool_call.id.clone(),
                    name: name.clone(),
                    arguments: args_str.clone(),
                })
                .await;
            }

            // Parse arguments — with auto-repair for common truncation issues.
            // (iter-123 — fixes "Invalid JSON: EOF while parsing" errors
            // caused by streaming tool-call argument fragmentation.)
            let mut args: serde_json::Value = match serde_json::from_str(&args_str) {
                Ok(a) => a,
                Err(orig_parse_err) => {
                    // Try to repair common truncation issues:
                    // 1. Missing closing brace — append }
                    // 2. Missing closing bracket — append ]
                    // 3. Truncated string value — append "
                    //
                    // S4 (iter-602): when no repair succeeds, do NOT
                    // execute on dummied-up `{}` — the pre-B2 last resort
                    // did exactly that, and the resulting
                    // schema-validation failure was retried into a
                    // 1,938-substitution cascade in the measured window.
                    // Answer the call with a named not-executed error the
                    // model can correct from in one round trip; it never
                    // reaches phase 2. The log carries the parse error and
                    // lengths only — raw model output does not go into
                    // logs, but the RESULT quotes a truncated preview (the
                    // model already owns its own args, and the 2026-10-24
                    // storm proved a content-free refusal cannot break a
                    // retry loop: 12 identical dumps, zero corrections).
                    let repaired = message_safety::repair_tool_call_arguments(&args_str, &name);
                    let repaired_value = match &repaired {
                        // Fixed is parseable by construction (the repair
                        // only returns strings it validated); the second
                        // from_str routes an invariant break to the same
                        // honest refusal instead of executing garbage.
                        message_safety::RepairOutcome::Fixed(fixed) => {
                            serde_json::from_str(fixed).ok()
                        }
                        message_safety::RepairOutcome::Unrepairable => None,
                    };
                    match repaired_value {
                        Some(a) => {
                            debug!(tool = %name, "Tool arguments auto-repaired");
                            a
                        }
                        None => {
                            warn!(
                                tool = %name,
                                args_len = args_str.len(),
                                error = %orig_parse_err,
                                "Unrepairable tool arguments — call not executed, error returned to the model"
                            );
                            // Feed the batch gate: later same-tool calls
                            // in THIS batch must see this refusal before
                            // the post-batch ladder ever runs.
                            *batch_parse_fails.entry(name.clone()).or_insert(0) += 1;
                            early_results[idx] = Some(ToolResult::error_with_name(
                                &name,
                                &tool_call.id,
                                message_safety::unrepairable_args_message(
                                    &orig_parse_err,
                                    &args_str,
                                ),
                            ));
                            continue;
                        }
                    }
                }
            };

            // ── Tool-call guardrails (R4 — hermes tool_guardrails.py) ──
            // Detect retry storms: the model calling the same tool with
            // identical args repeatedly within one turn. Side-effecting tools
            // are skipped on the 3rd identical call; no-effect tools (cheap,
            // read-only) get a warning then skip on the 4th. The synthetic
            // result tells the model to stop repeating, saving round-trips and
            // preventing repeated mutations.
            {
                use crate::tool_guardrails::GuardrailDecision;
                let (decision, pattern) = {
                    let mut g = self
                        .tool_guardrails
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let d = g.observe(&name, &args_str);
                    (d, g.last_pattern())
                };
                match decision {
                    GuardrailDecision::Allow => {}
                    // observe() (pre-execution) never returns Halt — only the
                    // post-result rungs do; handled in observe_guardrail_results.
                    GuardrailDecision::Halt(_) => {}
                    GuardrailDecision::Warn => {
                        warn!(
                            tool = %name,
                            pattern = ?pattern,
                            "Repetitive tool-call pattern — warning model"
                        );
                        self.emit(AgentEvent::Content {
                            text: crate::tool_guardrails::pattern_warning_message(
                                &name,
                                pattern.as_ref(),
                            ),
                        })
                        .await;
                    }
                    GuardrailDecision::Skip => {
                        warn!(
                            tool = %name,
                            pattern = ?pattern,
                            "Repetitive tool-call pattern — skipping duplicate"
                        );
                        self.metrics.record_guardrail_skip();
                        early_results[idx] = Some(ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            name: name.to_string(),
                            // A deliberate guardrail skip is guidance, not a
                            // tool failure: success-shaped so the degenerate-
                            // loop circuit (consecutive failed iterations)
                            // doesn't count skips as failures and break turns
                            // whose models merely re-emit calls.
                            success: true,
                            content: crate::tool_guardrails::build_pattern_skip_message(
                                &name,
                                pattern.as_ref(),
                            ),
                            error: None,
                            timed_out: false,
                        });
                        continue;
                    }
                }
            }

            // Validate tool exists — with bounded edit-distance recovery for
            // hallucinated / near-miss names. A model that writes
            // `read_files` for `read_file`, or a name from an MCP server
            // that has since disconnected, otherwise gets a bare
            // "not found" and retries the same wrong name forever. Recovery
            // rewrites the name in the ToolCall that flows into `pending`, so
            // BOTH execution branches (single-tool and the concurrent pool)
            // dispatch the canonical name without a second hook. A tie at the
            // minimum distance is refused, never guessed. The caller detects
            // the recovery by comparing the result's stamped name with the
            // name the model asked for, and tells the model about it.
            if !self.registry.contains(&name).await {
                let recovery =
                    crate::tools::tool_search::name_recovery::resolve(&self.registry, &name).await;
                match &recovery {
                    crate::tools::tool_search::name_recovery::Recovery::Recovered {
                        canonical,
                        distance,
                        ..
                    } => {
                        warn!(
                            tool = %name,
                            canonical = %canonical,
                            distance,
                            "Recovered near-miss tool name"
                        );
                        tool_call.function.name.clone_from(canonical);
                        name.clone_from(canonical);
                    }
                    crate::tools::tool_search::name_recovery::Recovery::Exact => {}
                    refused => {
                        error!(tool = %name, "Tool not found and not recoverable");
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            crate::tools::tool_search::name_recovery::refusal_message(
                                &name, refused,
                            ),
                        ));
                        continue;
                    }
                }
            }

            // ── Centralized argument validation (iter-262) ───────────
            // Validate required fields BEFORE calling tool.execute().
            // Without this, each tool's serde_json::from_value() would fail
            // independently with opaque "missing field 'query'" errors.
            // Centralized validation gives a clear, consistent error message
            // and prevents truncated tool calls from reaching the tool impl.
            if let Some(tool) = self.registry.get(&name).await {
                let schema = tool.schema();
                schema.sanitize_args(&mut args);
                if let Err(e) = schema.validate_args(&args) {
                    warn!(tool = %name, error = %e, "Tool argument validation failed");
                    // Use the schema validation error message directly — it
                    // already includes the field name (e.g. "Missing required
                    // field: query"). Avoid duplicating the tool name.
                    early_results[idx] = Some(ToolResult::error(&tool_call.id, e.to_string()));
                    continue;
                }
            }

            // Smart approval gate
            let mut guardian_approved = false;
            // R48-2: `approval_result` is scoped to the gate block; hoist the
            // verdict the headless deny below needs.
            let mut gate_flagged = false;
            if self.config.approval_mode != "off" {
                let approval_result = crate::approval::check_tool_approval(
                    &name,
                    &args,
                    Some(&self.config.approval_mode),
                );
                if approval_result.verdict == "requires_approval" {
                    gate_flagged = true;
                }
                match approval_result.verdict.as_str() {
                    "blocked" => {
                        warn!(tool = %name, "Tool call blocked by approval guard");
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            format!(
                                "{}: {}",
                                crate::tool_guardrails::HARD_REJECT_PREFIX,
                                approval_result
                                    .reason
                                    .unwrap_or_else(|| "blocked".to_string())
                            ),
                        ));
                        continue;
                    }
                    "requires_approval" => {
                        warn!(tool = %name, "Tool call flagged — will prompt user");
                        // ── Guardian-LLM arm (hermes `approval_smart.py`
                        // parity, opt-in via `OPERANT_GUARDIAN_LLM` at
                        // construction) ──────
                        // The auxiliary classifier may APPROVE a flagged
                        // command (skipping the channel) or DENY it (shaped
                        // like a smart-gate block). Anything else —
                        // ESCALATE, call failure, unarmed env — is today's
                        // behavior: the channel below decides. Seat policy
                        // keeps precedence: the guardian sits in the
                        // UNGOVERNED chain only (an Escalate/Deny from a
                        // policy row is never overridden — the classifier
                        // classifies, the policy decides).
                        if self.guardian_llm {
                            match self.guardian_classify(&name, &args).await {
                                Some(crate::approval::GuardianVerdict::Approve) => {
                                    info!(
                                        tool = %name,
                                        "Guardian-LLM approved the flagged command — skipping the permission prompt"
                                    );
                                    guardian_approved = true;
                                }
                                Some(crate::approval::GuardianVerdict::Deny) => {
                                    warn!(
                                        tool = %name,
                                        "Tool call blocked by the guardian-LLM classifier"
                                    );
                                    early_results[idx] = Some(ToolResult::error(
                                        &tool_call.id,
                                        format!(
                                            "{}: the guardian-LLM classifier denied this command",
                                            crate::tool_guardrails::HARD_REJECT_PREFIX
                                        ),
                                    ));
                                    continue;
                                }
                                _ => {
                                    debug!(
                                        tool = %name,
                                        "Guardian-LLM escalated — the permission channel decides"
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            // ── P1/P2 seat policy (permission-bylaws slices E + F2) ──────
            // With a seat authority attached AND a session id (the employee
            // id), `decide()` owns the verdict for this seat: `Run` skips the
            // prompt outright (a yolo mode or a seat allow entry), `Escalate`
            // enters the channel below with the policy's `why` as the danger
            // explanation (in-flight asks resolve interactively; the durable
            // approvals queue retired to Holon's board), and `Deny` is
            // returned shaped like a declined permission. Everything sits
            // behind the Option: no authority, no session id, or no policy
            // row for this seat → `None` → the branches below are today's
            // allowlist-then-hardcoded-list logic, unchanged.
            //
            // The consult is hoisted ABOVE the permission-channel block so
            // unattended agents (which may still carry a channel nobody
            // drains — see `with_unattended`) consult the bylaws too.
            //
            // `blocked` is `false` because the smart approval gate above
            // already denied blocked tools — the hardline floor fires before
            // any seat mode can see (let alone widen) a blocked call.
            // `dangerous` reuses `is_permission_gated_tool`, the same list
            // the prompt below has always keyed on (one list so the gate and
            // the predicate cannot drift apart).
            let seat: Option<SeatDecision> = match (
                self.seat_authority.as_ref(),
                // Wave 2: the seat is the bound EMPLOYEE when the gateway set
                // one; the session id otherwise (cron already runs as its
                // derived employee id, so its path is unchanged).
                self.seat_id().or_else(|| self.session_id()),
            ) {
                (Some(authority), Some(seat)) => {
                    authority.consult(&seat, &name, is_permission_gated_tool(&name))
                }
                _ => None,
            };

            // ── F2 unattended semantics ─────────────────────────────────
            // An unattended run (cron) never waits on an interactive
            // approver: a governed Escalate is queued (once) and denied THIS
            // run; the next run consults the grant the senior minted between
            // ticks (§8's two timelines). The UNGOVERDED path is not here at
            // all — `seat == None` keeps today's channel behaviour
            // byte-for-byte, including the dispatcher's no-active-channel
            // auto-AllowSession arm.
            if let Some(SeatDecision::Escalate(why)) = &seat
                && self.unattended
            {
                warn!(tool = %name, "Tool call escalated by seat policy — unattended run: denied");
                // O-17 (W3): in a Holon run the refusal is recorded for the
                // result — `result.refusals[]` is the blocker the kernel and
                // the Chair read; there is no grant to consult (an approved
                // plan IS the grant, phase C). A personal unattended run
                // keeps today's behaviour: queue + deny (if an authority is
                // attached at all).
                if let Some(run_dir) = crate::platform::holon_run_dir() {
                    crate::org::record_refusal(&run_dir, &name, why, "policy.json:escalate");
                    early_results[idx] = Some(ToolResult::error(
                        &tool_call.id,
                        format!(
                            "Refused under policy.json: {why}. Report this refusal \
                             in your result's `refusals` so the stage records the \
                             blocker"
                        ),
                    ));
                    continue;
                }
                let session_id = self.session_id().unwrap_or_default();
                let queued = self
                    .seat_authority
                    .as_ref()
                    .map(|authority| authority.escalate(&session_id, &name, why))
                    .unwrap_or_else(|| crate::org::seat_authority::QueuedAsk {
                        escalation: None,
                        note: String::from("could not be persisted (no seat authority attached)"),
                    });
                let reference = if queued.note.is_empty() {
                    format!(
                        "was queued as {}",
                        queued
                            .escalation
                            .as_ref()
                            .map(|e| e.request_id.as_str())
                            .unwrap_or("(unrecorded)")
                    )
                } else {
                    queued.note.clone()
                };
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    format!(
                        "Permission denied by seat policy: {why}. No interactive \
                         approver is attached to this unattended run, so this \
                         attempt is denied; the ask {reference} and the next \
                         run consults the grant the approver mints"
                    ),
                ));
                continue;
            }

            // Permission guard for dangerous tools (interactive — sequential)
            //
            // Phase C (docs/agentic-os/): a GOVERNED Escalate with no interactive
            // channel attached — `operant run` builds the agent without a
            // permission channel — must deny, never fall through to execution.
            // The same holds for a governed call that needs a prompt: without a
            // consumer the 120s wait would stall a headless run for nothing.
            // The unattended arm above already covers the cron path; this guard
            // covers the headless-run path. Ungoverned (`seat == None`) keeps
            // today's behavior byte-identical: run whatever the tool config
            // allows (invariant 5 — personal mode is untouched).
            if let Some(SeatDecision::Escalate(why)) = &seat
                && self.permission_tx.is_none()
            {
                warn!(tool = %name, "Tool call escalated by seat policy — no interactive approver attached: deny");
                // O-17 (W3, the multi-turn e2e's catch): in a Holon run the
                // refusal is RECORDED for the result — the same org-mode
                // semantics the unattended path carries. The queue-then-wait
                // behavior ("the next run consults the grant the approver
                // mints") is the PERSONAL-run path; in org mode there is no
                // approver and no grant — an approved plan IS the grant, and
                // result.refusals[] is the blocker the kernel and the Chair
                // read.
                if let Some(run_dir) = crate::platform::holon_run_dir() {
                    crate::org::record_refusal(&run_dir, &name, why, "policy.json:escalate");
                    early_results[idx] = Some(ToolResult::error(
                        &tool_call.id,
                        format!(
                            "Refused under policy.json (escalated): {why}. Report this \
                             refusal in your result's `refusals` so the stage records the blocker"
                        ),
                    ));
                    continue;
                }
                let session_id = self.session_id().unwrap_or_default();
                let queued = self
                    .seat_authority
                    .as_ref()
                    .map(|authority| authority.escalate(&session_id, &name, why))
                    .unwrap_or_else(|| crate::org::seat_authority::QueuedAsk {
                        escalation: None,
                        note: String::from("could not be persisted (no seat authority attached)"),
                    });
                let reference = if queued.note.is_empty() {
                    format!(
                        "was queued as {}",
                        queued
                            .escalation
                            .as_ref()
                            .map(|e| e.request_id.as_str())
                            .unwrap_or("(unrecorded)")
                    )
                } else {
                    queued.note.clone()
                };
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    format!(
                        "Permission denied by seat policy: {why}. No interactive \
                         approver is attached to this run, so this \
                         attempt is denied; the ask {reference} and the next \
                         run consults the grant the approver mints"
                    ),
                ));
                continue;
            }

            // ── R48-2 (2026-10-25 ponytail round 2): headless flagged-call deny ──
            // A call the smart gate FLAGGED (verdict `requires_approval`) with
            // NO interactive approver attached must deny, never fall through
            // to execution — the governed Escalate path already denies above;
            // this is its ungoverned twin. The flag used to log "will prompt
            // user" and then execute silently in every headless run (`operant
            // run`, sub-agent children) and for every tool outside the prompt
            // list in attended ones. Scope is deliberately narrow: benign
            // calls (verdict "allowed") keep invariant 5's headless behavior
            // byte-for-byte, allowlisted calls run (the operator already
            // approved them), a guardian approval runs, and a seat `Run`
            // decision runs — only the flagged-and-nobody-to-ask case denies.
            if !guardian_approved
                && gate_flagged
                && seat.is_none()
                && self.permission_tx.is_none()
                && !self.tool_allowed_by_allowlist(&name)
            {
                warn!(
                    tool = %name,
                    "Smart gate flagged the call — no interactive approver attached: deny"
                );
                early_results[idx] = Some(ToolResult::error_with_name(
                    &name,
                    &tool_call.id,
                    "Refused: this call was flagged for approval and no interactive \
                     approver is attached to this run (headless). Run interactively \
                     to approve it once, or add it to your approval allowlist."
                        .to_string(),
                ));
                continue;
            }

            if let Some(permission_tx) = &self.permission_tx {
                // Whether this call prompts, and with what danger sentence.
                // The ungoverned arm is today's logic, lifted unchanged:
                // hermes parity — a tool covered by the session or permanent
                // allowlist (`command_allowlist` / `always`) never prompts,
                // checked before the hardcoded dangerous-tool list so
                // allowlisted tools bypass the gate
                // (`_command_matches_permanent_allowlist` fires before
                // detection, with only the hardline floor above it).
                //
                // F2: a governed Escalate persists its ask to the queue
                // BEFORE the prompt leaves (§8.1 push-then-queue) — the row
                // below is what the approver's /approve mints a grant
                // against and resolves.
                let mut seat_escalation: Option<crate::org::seat_authority::SeatEscalation> = None;
                let (needs_prompt, danger): (bool, String) = if let Some(decision) = seat {
                    match decision {
                        SeatDecision::Run(_) => (false, String::new()),
                        SeatDecision::Escalate(why) => {
                            let session_id = self.session_id().unwrap_or_default();
                            seat_escalation = self.seat_authority.as_ref().and_then(|authority| {
                                authority.escalate(&session_id, &name, &why).escalation
                            });
                            (true, why)
                        }
                        SeatDecision::Deny(why) => {
                            warn!(tool = %name, "Tool call denied by seat policy");
                            // O-17: a Holon run's denial is a recorded
                            // refusal, for the result's `refusals[]`.
                            if let Some(run_dir) = crate::platform::holon_run_dir() {
                                crate::org::record_refusal(
                                    &run_dir,
                                    &name,
                                    &why,
                                    "policy.json:deny",
                                );
                                early_results[idx] = Some(ToolResult::error(
                                    &tool_call.id,
                                    format!(
                                        "Refused under policy.json: {why}. Report this \
                                         refusal in your result's `refusals` so the stage \
                                         records the blocker"
                                    ),
                                ));
                                continue;
                            }
                            early_results[idx] = Some(ToolResult::error(
                                &tool_call.id,
                                format!("Permission denied by seat policy: {why}"),
                            ));
                            continue;
                        }
                    }
                } else if guardian_approved {
                    // The guardian-LLM approved this specific call — skip the
                    // prompt, keep the approval's own audit trail (the info!
                    // above).
                    (false, String::new())
                } else if self.tool_allowed_by_allowlist(&name) {
                    // allowed by allowlist — no prompt
                    (false, String::new())
                } else {
                    let needs_permission = is_permission_gated_tool(&name);
                    if needs_permission {
                        (
                            true,
                            match name.as_str() {
                                "bash" | "terminal" | "execute_command" => {
                                    "This runs a shell command on your system".to_string()
                                }
                                "code_execution" => {
                                    "This runs code on your system with the operant process's permissions (not sandboxed)".to_string()
                                }
                                "file_read" => "This reads a file from your system".to_string(),
                                "file_write" => "This writes content to a file".to_string(),
                                "file_edit" | "patch" => "This modifies an existing file".to_string(),
                                "process" => "This manages background processes".to_string(),
                                "browser" => "This opens and interacts with a browser".to_string(),
                                "memory_store" | "skill_manage" | "learning_manage" => {
                                    "This changes what the agent remembers or how it behaves in later turns"
                                        .to_string()
                                }
                                _ => "This tool may modify your system".to_string(),
                            },
                        )
                    } else {
                        (false, String::new())
                    }
                };
                if needs_prompt {
                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                    let description = format!("Execute {} tool", name);
                    let input_preview = Some(args_str.clone());
                    let _ = permission_tx
                        .send(ToolPermissionRequest {
                            tool_name: name.clone(),
                            tool_id: tool_call.id.clone(),
                            description,
                            danger_explanation: danger,
                            seat_escalation,
                            // D-2: the receiver's unattended-posture consult
                            // keys on this; attended runs never carry it.
                            unattended: self.unattended,
                            input_preview,
                            response_tx: resp_tx,
                        })
                        .await;
                    let response = tokio::select! {
                        r = resp_rx => r.unwrap_or(ToolPermissionResponse::Deny),
                        _ = tokio::time::sleep(Duration::from_secs(120)) => ToolPermissionResponse::Deny,
                    };
                    match response {
                        ToolPermissionResponse::AllowOnce => {}
                        ToolPermissionResponse::AllowSession => {
                            // hermes `approve_session`: remember the tool
                            // for the rest of this agent instance so it
                            // never prompts again this session.
                            self.session_allowlist
                                .write()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(name.clone());
                        }
                        ToolPermissionResponse::AllowAlways => {
                            // hermes `approve_permanent` +
                            // `save_permanent_allowlist`: remember forever
                            // and persist to disk so later sessions honor
                            // the choice too.
                            self.session_allowlist
                                .write()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(name.clone());
                            let patterns = {
                                let mut guard = self
                                    .persistent_allowlist
                                    .write()
                                    .unwrap_or_else(|e| e.into_inner());
                                guard.insert(name.clone());
                                guard.clone()
                            };
                            persist_approval_allowlist(
                                self.config.approval_allowlist_path.as_deref(),
                                &patterns,
                            );
                        }
                        ToolPermissionResponse::Deny => {
                            early_results[idx] = Some(ToolResult::error(
                                &tool_call.id,
                                "Permission denied by user".to_string(),
                            ));
                            continue;
                        }
                    }
                }
            }

            // Tool passed all pre-flight checks — queue for concurrent execution
            pending.push((idx, tool_call, args));
        }

        // Phase 2: Execute. Batches that can interfere with each other run
        // sequentially; everything else uses the 8-worker pool.
        // If only 1 tool is pending, skip the overhead and execute directly.
        if pending.is_empty() {
            // All tools were handled in pre-flight (errors/blocked/denied)
            // (iter-141 — fixed A20/A21: was .unwrap() which panics if a
            // future was cancelled. Use flatten() to gracefully skip None.)
            let mut results = stamp_result_names(early_results, &call_names);
            self.observe_guardrail_results(&mut results).await;
            return Ok(results);
        }

        if pending.len() == 1 {
            // Single tool — no concurrency overhead
            let (idx, tool_call, args) = pending
                .into_iter()
                .next()
                .expect("pending non-empty in single-tool branch");
            let name = tool_call.function.name.clone();
            // Plan 015: kernel tools key kernels/harness by session id.
            let tool_ctx = self.tool_context(&name);
            // Permit is already in hand (single-tool path skips the pool), so
            // re-announce the call: the TUI shows Queued from the first
            // ToolStart and flips to Running on this one.
            self.emit(AgentEvent::ToolStart {
                tool_call_id: tool_call.id.clone(),
                name: name.clone(),
                arguments: normalized_tool_args(&tool_call),
            })
            .await;
            let tool_started = std::time::Instant::now();
            let tool_future = self.registry.execute(&name, &tool_call.id, args, tool_ctx);
            // Interactive tools (clarify / approval_request) block waiting
            // for a human — the generic tool timeout (30s) would kill the
            // dialog before the user can respond. Long-running tools
            // (delegate_task) spawn a child agent with its own timeout. Both
            // get a generous defensive wrapper instead: the user-question
            // receiver resolves dialogs on their own (120s timeout reply),
            // and the child timeout governs delegation — the wrapper is only
            // a backstop against a wedged receiver/child.
            let limit = if is_interactive_tool(&name) || is_long_running_tool(&name) {
                LONG_RUNNING_TOOL_TIMEOUT
            } else {
                self.config.tool_timeout
            };
            // (plan 04) race the tool against the interrupt flag too: a
            // blocking tool must die on cancel, not ride its timeout out.
            // The interrupt arm yields a truthful "Cancelled…" result —
            // NOT the timeout marker, which would feed the S2 breaker a
            // cancellation it didn't cause.
            let interrupt_flag = self.interrupt_flag.clone();
            let cancelled_msg = "Cancelled by the user before this call finished.";
            let result = tokio::select! {
                r = timeout(limit, tool_future) => r,
                _ = interrupt_flag.wait_triggered() => Ok(Ok(ToolResult::error(
                    &tool_call.id,
                    cancelled_msg.to_string(),
                ))),
            };
            if let Some(ref bus) = self.turn_end_bus {
                bus.record_tool_duration(tool_started.elapsed().as_millis() as u64);
            }
            self.record_tool_duration(&tool_call.id, tool_started.elapsed().as_millis() as u64);
            early_results[idx] = Some(match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                // S2: the flag is what the per-tool breaker keys on — a
                // timeout result built here without it would slip past the
                // breaker when the outer wrapper fires before the
                // registry's own (e.g. a per-tool override extends the
                // inner deadline past this one).
                Err(_) => ToolResult::timeout(&name, &tool_call.id, limit),
            });
        } else if !batch_allows_parallel_execution(
            pending.iter().map(|(_, tc, _)| tc.function.name.as_str()),
        ) {
            // Interfering batch (multiple file mutations, or an
            // approval/permission-gated call): run one at a time so a
            // mutation can't race another read/write of the same path and
            // an approved destructive call can't overlap unrelated work.
            // Same per-call body as the pool below, awaited in order.
            for (idx, tool_call, args) in pending {
                let name = tool_call.function.name.clone();
                self.emit(AgentEvent::ToolStart {
                    tool_call_id: tool_call.id.clone(),
                    name: name.clone(),
                    arguments: normalized_tool_args(&tool_call),
                })
                .await;
                let tool_started = std::time::Instant::now();
                let tool_ctx = self.tool_context(&name);
                let exec = self.registry.execute(&name, &tool_call.id, args, tool_ctx);
                let limit = if is_interactive_tool(&name) || is_long_running_tool(&name) {
                    LONG_RUNNING_TOOL_TIMEOUT
                } else {
                    self.config.tool_timeout
                };
                let result = timeout(limit, exec).await;
                if let Some(ref bus) = self.turn_end_bus {
                    bus.record_tool_duration(tool_started.elapsed().as_millis() as u64);
                }
                self.record_tool_duration(&tool_call.id, tool_started.elapsed().as_millis() as u64);
                early_results[idx] = Some(match result {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                    // S2: mark the flag (see single-tool branch above).
                    Err(_) => ToolResult::timeout(&name, &tool_call.id, limit),
                });
            }
        } else {
            // Multiple independent tools — execute concurrently with semaphore
            let semaphore = StdArc::new(Semaphore::new(8));
            let tool_timeout = self.config.tool_timeout;

            // (plan 04) ids captured before `pending` is consumed — the
            // interrupt path pairs unfinished calls by id.
            //
            // RT-462: the ids were collected from `pending` (the calls that
            // SURVIVED pre-flight) but indexed with the position in
            // `call_names` (ALL the calls). A pre-flight rejection therefore
            // shifted every pairing after it — the wrong `tool_call_id` on
            // the cancelled results, which strict providers reject — and a
            // batch with more calls than pending indexed PAST the end of
            // the vector: an out-of-bounds panic that killed the turn task.
            // The original index travels with the id now.
            let tool_call_ids = ids_by_call_index(call_names.len(), &pending);
            let futures: Vec<_> = pending
                .into_iter()
                .map(|(idx, tool_call, args)| {
                    let sem = semaphore.clone();
                    let registry = &self.registry;
                    let interrupt_flag = &self.interrupt_flag;
                    async move {
                        // Acquire semaphore permit (limits to 8 concurrent)
                        // (iter-141 — fixed A20: was .unwrap() which panics
                        // if the semaphore closes during shutdown. Use
                        // ok() + early return on failure.)
                        let _permit = match sem.acquire().await {
                            Ok(p) => p,
                            Err(_) => {
                                return (
                                    idx,
                                    ToolResult::error(
                                        &tool_call.id,
                                        "Skipped: semaphore closed during shutdown".to_string(),
                                    ),
                                );
                            }
                        };

                        // Check interrupt flag before execution
                        if interrupt_flag.is_triggered() {
                            return (
                                idx,
                                ToolResult::error(
                                    &tool_call.id,
                                    "Skipped: interrupted".to_string(),
                                ),
                            );
                        }

                        let name = tool_call.function.name.clone();
                        // Re-announce the call now that a permit is held: the
                        // first ToolStart (emitted at parse time) only means
                        // "queued", this one means "running". Without it a tool
                        // that sat in the pool reads as queued until it finishes.
                        // ponytail: rides the existing ToolStart event instead of
                        // a new variant — swap for AgentEvent::ToolRunning if
                        // AgentEvent is ever free to change again.
                        self.emit(AgentEvent::ToolStart {
                            tool_call_id: tool_call.id.clone(),
                            name: name.clone(),
                            arguments: normalized_tool_args(&tool_call),
                        })
                        .await;
                        // Plan 015: session-keyed ToolContext for kernel tools.
                        let tool_ctx = self.tool_context(&name);
                        let exec_started = std::time::Instant::now();
                        let exec = registry.execute(&name, &tool_call.id, args, tool_ctx);
                        // Interactive tools exempt from the generic tool
                        // timeout (see is_interactive_tool); long-running
                        // tools like delegate_task carry their own child
                        // timeout. Both get the generous backstop — the
                        // user-question receiver resolves dialogs on their
                        // own 120s timeout and the child timeout governs
                        // delegation.
                        let limit = if is_interactive_tool(&name) || is_long_running_tool(&name) {
                            LONG_RUNNING_TOOL_TIMEOUT
                        } else {
                            tool_timeout
                        };
                        let result = timeout(limit, exec).await;

                        if let Some(ref bus) = self.turn_end_bus {
                            bus.record_tool_duration(exec_started.elapsed().as_millis() as u64);
                        }
                        self.record_tool_duration(
                            &tool_call.id,
                            exec_started.elapsed().as_millis() as u64,
                        );

                        (
                            idx,
                            match result {
                                Ok(Ok(r)) => r,
                                Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                                // S2: mark the flag (see single-tool branch).
                                Err(_) => ToolResult::timeout(&name, &tool_call.id, limit),
                            },
                        )
                    }
                })
                .collect();

            // Execute all futures concurrently and collect results.
            // (plan 04) The drain races the interrupt flag: completed
            // results are kept; when the flag fires, the pool is dropped
            // (running tool futures are cancelled — child processes die
            // with kill_on_drop) and every call that never produced a
            // result becomes a truthful "Cancelled by the user…" result,
            // so the API pairing rule holds without message_safety
            // repairs.
            let mut got = Vec::new();
            let mut pool = stream::iter(futures).buffer_unordered(8);
            let interrupted = loop {
                tokio::select! {
                    maybe = pool.next() => {
                        match maybe {
                            Some(pair) => got.push(pair),
                            None => break false,
                        }
                    }
                    _ = self.interrupt_flag.wait_triggered() => {
                        break true;
                    }
                }
            };

            // Place results in the correct position
            for (idx, result) in got {
                early_results[idx] = Some(result);
            }
            if interrupted {
                warn!("Interrupt during tool batch — cancelled unfinished calls");
                for (idx, call) in call_names.iter().enumerate() {
                    if early_results[idx].is_none() {
                        let _ = call; // name only for logging
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call_ids[idx],
                            "Cancelled by the user before this call finished.".to_string(),
                        ));
                    }
                }
            }
        }

        // Collect results in original order
        // (iter-141 — fixed A20/A21: was .unwrap() which panics if a
        // future was cancelled. Use flatten() to gracefully skip None.)
        let mut results = stamp_result_names(early_results, &call_names);
        self.observe_guardrail_results(&mut results).await;
        Ok(results)
    }

    /// Run agent and handle self-healing on tool errors.
    ///
    /// Each retry is a DISTINCT turn with the same user query. The normal
    /// turn prologue already deduplicates the user row when the last
    /// conversation entry is that exact query, so this method records the
    /// recovery hint explicitly and never appends the query itself.
    pub async fn run_with_healing(&self, user_query: String) -> Result<Message> {
        let mut iteration = 0;
        let max_healing_attempts = self.config.max_healing_attempts;

        loop {
            iteration += 1;

            match self.run(user_query.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) if e.is_self_healing() && iteration <= max_healing_attempts => {
                    warn!(iteration, error = %e, "Self-healing: re-prompting LLM");

                    // Add error context as a system message
                    let error_msg = format!(
                        "{} {}. Please correct your approach and try again.",
                        super::HEALING_RETRY_HINT_PREFIX,
                        e.user_message()
                    );

                    self.add_message(Message::system(&error_msg)).await;
                }
                Err(e) => {
                    error!(error = %e, "Agent run failed");
                    return Err(e);
                }
            }
        }
    }
}

/// Re-announcement payload for a `ToolStart`: the same arguments string the
/// first (parse-time) `ToolStart` carried, with the empty case normalised to
/// `"{}"` so consumers can always parse it as JSON.
fn normalized_tool_args(tool_call: &ToolCall) -> String {
    if tool_call.function.arguments.trim().is_empty() {
        "{}".to_string()
    } else {
        tool_call.function.arguments.clone()
    }
}

/// Append guardrail guidance to the part of a tool result the model reads:
/// the content of a success, the error text of a failure.
fn append_guidance(result: &mut ToolResult, label: &str, message: &str) {
    let note = format!("\n\n[{label}: {message}]");
    if result.success {
        result.content.push_str(&note);
    } else {
        let error = result.error.get_or_insert_with(|| "Error".to_string());
        error.push_str(&note);
    }
}

/// Collect results in call order, stamping each nameless result with its
/// call's name (empty names become "(unnamed)").
/// The id of each call, indexed by its ORIGINAL position in the batch.
///
/// RT-462: the ids used to be collected from `pending` alone (the calls
/// that survived pre-flight) and then indexed with the position in
/// `call_names` (ALL the calls) — so a pre-flight rejection shifted every
/// pairing after it, and a batch with more calls than pending indexed
/// past the end of the vector (a panic that killed the turn task). The
/// vector covers every call, and each id sits at its own call's index.
fn ids_by_call_index(
    call_count: usize,
    pending: &[(usize, ToolCall, serde_json::Value)],
) -> Vec<String> {
    let mut ids = vec![String::new(); call_count];
    for (idx, tool_call, _) in pending {
        if *idx < ids.len() {
            ids[*idx] = tool_call.id.clone();
        }
    }
    ids
}

fn stamp_result_names(early: Vec<Option<ToolResult>>, call_names: &[String]) -> Vec<ToolResult> {
    early
        .into_iter()
        .enumerate()
        .filter_map(|(idx, result)| {
            let mut result = result?;
            if result.name.is_empty() {
                result.name = match call_names.get(idx).map(|n| n.trim()) {
                    Some(name) if !name.is_empty() => name.to_string(),
                    _ => "(unnamed)".to_string(),
                };
            }
            Some(result)
        })
        .collect()
}

#[cfg(test)]
mod rt462_tests {
    use super::*;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            function: crate::client::ToolCallFunction {
                name: "demo".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    /// RT-462: the ids were collected from `pending` (the calls that
    /// survived pre-flight) and then indexed with the position in
    /// `call_names` (ALL the calls) — so a pre-flight rejection shifted
    /// every pairing after it, and a batch with more calls than pending
    /// indexed PAST the end of the vector: an out-of-bounds panic that
    /// killed the turn task.
    ///
    /// BEHAVIORAL: the id vector must cover every call and carry each id
    /// at its own call's index.
    #[test]
    fn the_cancelled_results_pair_with_their_own_call_ids() {
        // Three calls; the first was rejected in pre-flight, so only the
        // last two reached `pending`.
        let pending: Vec<(usize, ToolCall, serde_json::Value)> = vec![
            (1, call("call-2"), serde_json::json!({})),
            (2, call("call-3"), serde_json::json!({})),
        ];
        let ids = ids_by_call_index(3, &pending);
        assert_eq!(
            ids.len(),
            3,
            "the vector must cover EVERY call — the old one had two entries \
             for three calls, and the drain indexed all three"
        );
        assert_eq!(ids[1], "call-2", "each id sits at its own call's index");
        assert_eq!(ids[2], "call-3", "each id sits at its own call's index");
    }
}

#[cfg(test)]
mod rt466_tests {
    use super::*;
    use crate::agent::model_client::ChatRequest;
    use crate::agent::model_client::StreamChunk;
    use crate::client::{ToolCall, ToolCallFunction};
    use tokio::sync::mpsc;

    struct UnusedClient;

    #[async_trait::async_trait]
    impl crate::agent::model_client::ModelClient for UnusedClient {
        async fn chat(&self, _request: ChatRequest) -> Result<crate::client::ChatResponse> {
            Err(crate::error::Error::Agent("unused".into()))
        }
        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(crate::error::Error::Agent("unused".into()))
        }
        fn provider_name(&self) -> &str {
            "unused"
        }
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            function: ToolCallFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    /// A failed stream that already reported an input-token half must not
    /// discard that metered usage, and must attribute it to the route that
    /// served it rather than the configured startup model.
    #[tokio::test]
    async fn a_failed_stream_still_emits_its_known_usage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db =
            crate::database::Database::init(temp_dir.path().join("failed_usage.sqlite")).unwrap();
        let (tx, mut rx) = mpsc::channel(64);

        struct UnusedClient;

        #[async_trait::async_trait]
        impl crate::agent::model_client::ModelClient for UnusedClient {
            async fn chat(&self, _request: ChatRequest) -> Result<crate::client::ChatResponse> {
                Err(crate::error::Error::Agent("unused".into()))
            }

            async fn chat_streaming(
                &self,
                _request: ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                Err(crate::error::Error::Agent("unused".into()))
            }

            fn provider_name(&self) -> &str {
                "unused"
            }
        }

        let agent = OperantAgent::with_events(
            AgentConfig {
                model: "startup-route".to_string(),
                ..AgentConfig::default()
            },
            Box::new(UnusedClient),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
            tx,
        );

        let usage_chunk = StreamChunk {
            content: None,
            reasoning: None,
            tool_calls: None,
            extra_content: None,
            usage: Some(crate::client::Usage {
                prompt_tokens: 11,
                completion_tokens: 0,
                total_tokens: 11,
            }),
            finish_reason: None,
            dropped_events: None,
            model: Some("served-route".to_string()),
        };
        let chunks: Vec<Result<StreamChunk>> = vec![
            Ok(usage_chunk),
            Err(Error::Network(
                reqwest::Client::new()
                    .get("http://127.0.0.1:9/")
                    .send()
                    .await
                    .expect_err("connection refused")
                    .into(),
            )),
        ];
        let result = agent
            .process_stream(Box::pin(futures::stream::iter(chunks)))
            .await;
        assert!(result.is_err(), "the stream failure must still surface");

        let mut saw_usage = false;
        let mut saw_cost_model = None;
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentEvent::Usage {
                    input_tokens,
                    output_tokens,
                    ..
                } if input_tokens == 11 && output_tokens == 0 => saw_usage = true,
                AgentEvent::Cost { model, .. } => saw_cost_model = Some(model),
                _ => {}
            }
        }
        assert!(saw_usage, "the known usage must survive the stream failure");
        assert_eq!(
            saw_cost_model.as_deref(),
            Some("served-route"),
            "cost must be attributed to the route that served the stream"
        );
    }

    /// RT-466: after merging a native tool call, the lookup found the FIRST
    /// call with a non-empty name — not the one just merged — so an SSE
    /// event carrying two native tool calls announced only the first, and
    /// the second's row appeared late (or never, when the turn was
    /// interrupted first).
    ///
    /// BEHAVIORAL: both calls in one chunk must get their own ToolStart.
    #[tokio::test]
    async fn two_native_tool_calls_in_one_chunk_each_get_a_tool_start() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::database::Database::init(temp_dir.path().join("rt466.sqlite")).unwrap();
        let (tx, mut rx) = mpsc::channel(64);
        let agent = OperantAgent::with_events(
            AgentConfig {
                reasoning_effort: String::new(),
                reasoning_effort_param: Default::default(),
                context_semantic_compaction: false,
                model: "demo".to_string(),
                max_trajectories: 0,
                show_turn_attribution: true,
                max_iterations: 3,
                tool_timeout: Duration::from_secs(5),
                request_timeout: Duration::from_secs(5),
                system_prompt: None,
                stream: true,
                context_window: 8000,
                max_tool_result_share: crate::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
                max_healing_attempts: 1,
                fallback_models: Vec::new(),
                fallback_on_errors: false,
                loop_detection_enabled: false,
                approval_mode: "off".to_string(),
                approval_allowlist: Vec::new(),
                approval_allowlist_path: None,
                record_trajectories: false,
                guardrail_exempt_tools: Vec::new(),
                skill_nudge_interval: 0,
                memory_review_interval: 0,
                max_retries: 1,
                tool_search: Default::default(),
                ponytail: Default::default(),
            },
            Box::new(UnusedClient),
            crate::ToolRegistry::new(Duration::from_secs(5)),
            std::sync::Arc::new(db),
            tx,
        );

        let chunk = StreamChunk {
            content: None,
            reasoning: None,
            tool_calls: Some(vec![call("call-a", "alpha"), call("call-b", "beta")]),
            extra_content: None,
            usage: None,
            finish_reason: Some("tool_calls".to_string()),
            dropped_events: None,
            model: None,
        };
        let stream: BoxStream<'static, Result<StreamChunk>> =
            Box::pin(futures::stream::once(async move { Ok(chunk) }));
        agent
            .process_stream(stream)
            .await
            .expect("the stream must be processed");

        let mut ids: Vec<String> = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::ToolStart { tool_call_id, .. } = event {
                ids.push(tool_call_id);
            }
        }
        assert!(
            ids.contains(&"call-a".to_string()) && ids.contains(&"call-b".to_string()),
            "both native calls must get their own ToolStart — the lookup after \
             the merge found the FIRST named call: {ids:?}"
        );
    }
}

#[cfg(test)]
mod turn_attribution_tests {
    //! Row 3 (RT-209): the post-turn `TurnEnd` emission seat.
    //!
    //! The prompt tail cannot host this block: `build_messages` runs before the
    //! loop, so at that moment no model call has happened and no tool has run,
    //! and every span would read `n/a` forever. The numbers are therefore
    //! computed at turn END. These tests pin the facts that make it honest.
    //!
    //! The mock client exists only so an `OperantAgent` can be constructed;
    //! none of these tests ever reaches the model, because `turn_attribution`
    //! is called directly.

    use super::*;
    use crate::turn_attribution::{self, SpanDuration, TurnAttribution, TurnSpan};

    fn config() -> AgentConfig {
        AgentConfig {
            reasoning_effort: String::new(),
            reasoning_effort_param: crate::config::ReasoningEffortParam::Auto,
            context_semantic_compaction: false,
            model: "demo".to_string(),
            max_trajectories: 0,
            max_iterations: 3,
            tool_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            system_prompt: Some("You are a test agent.".to_string()),
            stream: true,
            context_window: 8000,
            max_tool_result_share: crate::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
            max_healing_attempts: 1,
            show_turn_attribution: true,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            loop_detection_enabled: true,
            approval_mode: "off".to_string(),
            approval_allowlist: Vec::new(),
            approval_allowlist_path: None,
            record_trajectories: false,
            guardrail_exempt_tools: Vec::new(),
            skill_nudge_interval: 0,
            memory_review_interval: 0,
            max_retries: 3,
            tool_search: Default::default(),
            ponytail: Default::default(),
        }
    }

    /// Never called: a client that exists so an agent can be built.
    struct UnusedClient;

    #[async_trait::async_trait]
    impl crate::agent::model_client::ModelClient for UnusedClient {
        async fn chat(&self, _request: ChatRequest) -> Result<crate::client::ChatResponse> {
            Err(Error::Agent("unused in turn-attribution tests".into()))
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let stream = futures::stream::once(async {
                Err(Error::Agent("unused in turn-attribution tests".into()))
            });
            Ok(Box::pin(stream))
        }

        fn provider_name(&self) -> &str {
            "unused"
        }
    }

    async fn agent(cfg: AgentConfig) -> OperantAgent {
        let temp_dir = tempfile::tempdir().unwrap();
        let db =
            crate::database::Database::init(temp_dir.path().join("attribution.sqlite")).unwrap();
        OperantAgent::new(
            cfg,
            Box::new(UnusedClient),
            crate::ToolRegistry::new(Duration::from_secs(1)),
            std::sync::Arc::new(db),
        )
    }

    #[tokio::test]
    async fn measured_turn_carries_a_breakdown_with_honest_unmeasured_spans() {
        let a = agent(config()).await;
        let start = std::time::Instant::now();
        let inference = Duration::from_millis(30);
        a.record_tool_duration("call-1", 20);
        // Sleep BEFORE building the breakdown so the turn's wall clock
        // genuinely covers the spans handed in below: the turn cannot have
        // taken less than the work done inside it.
        std::thread::sleep(Duration::from_millis(70));

        let attr = a
            .turn_attribution(start, inference, true)
            .expect("the flag is on and a model call happened, so a breakdown exists");
        // Partial capture: three of four spans were measured, and the one that
        // was not must be NAMED rather than silently rendered as a number.
        assert!(
            matches!(&attr, TurnAttribution::Incomplete { missing, .. }
                if missing.as_slice() == [TurnSpan::UserWait, TurnSpan::Overhead]),
            "an unmeasured span must be named, not rounded to zero: {attr:?}"
        );
        let bd = match attr {
            TurnAttribution::Complete(b) | TurnAttribution::Incomplete { measured: b, .. } => b,
            TurnAttribution::Disabled => unreachable!("the flag is on"),
        };

        assert_eq!(
            bd.span(TurnSpan::Inference),
            SpanDuration::Measured(inference)
        );
        assert_eq!(
            bd.span(TurnSpan::Tools),
            SpanDuration::Measured(Duration::from_millis(20))
        );
        // No approval gate was crossed in this test, and nothing measures
        // "user wait" at this seam, so those two spans must be honest about it.
        assert_eq!(bd.span(TurnSpan::UserWait), SpanDuration::Unmeasured);
        assert_eq!(bd.span(TurnSpan::Overhead), SpanDuration::Unmeasured);
        // An unmeasured span means the sum cannot be verified, so `is_exclusive`
        // must report `false` rather than silently `true` on zeros.
        assert!(!bd.is_exclusive());
        // Wall clock is the turn's real elapsed time. The sleep above makes it
        // genuinely cover the spans handed in, so it must be at least their
        // sum: a turn cannot take less than the work done inside it.
        assert!(bd.total() >= inference + Duration::from_millis(20));
    }

    #[tokio::test]
    async fn nothing_captured_is_unmeasured_never_zero() {
        let a = agent(config()).await;
        let start = std::time::Instant::now();
        // A turn that exited before any model call, with no tool ever timed.
        let attr = a
            .turn_attribution(start, Duration::ZERO, false)
            .expect("the flag is on, so a breakdown exists even with nothing measured");
        // Nothing was captured, so the classification is `Incomplete` and it
        // names ALL FOUR spans. That list is the answer to "why is there no
        // attribution here", and it is what a zero would destroy.
        assert!(
            matches!(&attr, TurnAttribution::Incomplete { missing, .. }
                if missing.as_slice() == turn_attribution::TurnSpan::ALL),
            "a turn with no capture must name every missing span: {attr:?}"
        );
        let bd = match attr {
            TurnAttribution::Complete(b) | TurnAttribution::Incomplete { measured: b, .. } => b,
            TurnAttribution::Disabled => unreachable!("the flag is on"),
        };

        // The RT-199 failure this row exists to avoid: a zero standing in for
        // an absent measurement. All three input spans must be `Unmeasured`.
        assert_eq!(bd.span(TurnSpan::Inference), SpanDuration::Unmeasured);
        assert_eq!(bd.span(TurnSpan::Tools), SpanDuration::Unmeasured);
        assert_eq!(bd.span(TurnSpan::UserWait), SpanDuration::Unmeasured);
        assert_eq!(bd.span(TurnSpan::Overhead), SpanDuration::Unmeasured);
        assert!(!bd.is_exclusive());
    }

    #[tokio::test]
    async fn a_genuinely_instant_model_call_is_measured_zero_not_unmeasured() {
        // The mirror image of the test above. A model call that really took no
        // time is `Measured(0)`; conflating that with "no call happened" is the
        // same bug facing the other direction.
        let a = agent(config()).await;
        let start = std::time::Instant::now();

        let attr = a
            .turn_attribution(start, Duration::ZERO, true)
            .expect("saw_inference is true, so the span is a real measurement");
        let bd = match attr {
            TurnAttribution::Complete(b) | TurnAttribution::Incomplete { measured: b, .. } => b,
            TurnAttribution::Disabled => unreachable!("the flag is on"),
        };

        assert_eq!(
            bd.span(TurnSpan::Inference),
            SpanDuration::Measured(Duration::ZERO)
        );
        assert_eq!(bd.span(TurnSpan::Tools), SpanDuration::Unmeasured);
    }

    #[tokio::test]
    async fn tool_totals_accumulate_across_calls() {
        let a = agent(config()).await;
        let start = std::time::Instant::now();
        // Three parallel tool calls: the per-TURN total is the sum, which for
        // concurrent work can exceed the turn's wall clock (documented on
        // `turn_tool_duration`, and why `is_exclusive` reports `false`).
        a.record_tool_duration("call-1", 10);
        a.record_tool_duration("call-2", 15);
        a.record_tool_duration("call-3", 25);

        assert_eq!(
            a.turn_tool_duration(),
            Some(Duration::from_millis(50)),
            "the per-turn tool total is the sum of every recorded call"
        );
        // ...and that total reaches the breakdown.
        let attr = a
            .turn_attribution(start, Duration::from_millis(5), true)
            .unwrap();
        let bd = match attr {
            TurnAttribution::Complete(b) | TurnAttribution::Incomplete { measured: b, .. } => b,
            TurnAttribution::Disabled => unreachable!("the flag is on"),
        };
        assert_eq!(
            bd.span(TurnSpan::Tools),
            SpanDuration::Measured(Duration::from_millis(50))
        );
    }

    #[tokio::test]
    async fn the_kill_switch_returns_none_and_fakes_nothing() {
        let mut cfg = config();
        cfg.show_turn_attribution = false;
        let a = agent(cfg).await;
        let start = std::time::Instant::now();
        a.record_tool_duration("call-1", 20);

        // `None` is the kill-switch's whole contract: a subscriber must be able
        // to tell "attribution is off" from "the spans were zero".
        assert!(
            a.turn_attribution(start, Duration::from_millis(5), true)
                .is_none()
        );
    }
}
