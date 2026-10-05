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
use tracing::{debug, error, warn};

use super::*;

impl OperantAgent {
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
        let mut usage_completion_tokens: Option<u32> = None;
        // Capture the original stream error so we can surface it (instead of
        // the generic "Stream processing failed" string) and decide whether
        // to flush partials after the loop.
        let mut stream_error: Option<Error> = None;
        // Provider-reported finish reason from the terminal chunk(s) (T1 —
        // truncation detection). Last non-None wins.
        let mut finish_reason: Option<String> = None;

        while let Some(chunk_result) = stream.next().await {
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
                            let is_new = !tool_calls.iter().any(|e| e.id == tc.id);
                            let has_name = !tc.function.name.is_empty();
                            merge_stream_tool_call(&mut tool_calls, tc);
                            if is_new
                                && has_name
                                && let Some(full) =
                                    tool_calls.iter().find(|e| !e.function.name.is_empty())
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
            self.emit_usage_and_cost(&usage).await;
        }

        Ok((
            accumulated_text,
            accumulated_reasoning,
            tool_calls,
            accumulated_extra,
            finish_reason,
            usage_tokens,
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

        self.emit_usage_and_cost(&response.usage).await;

        let usage_tokens = u64::from(response.usage.total_tokens);
        Ok((content, reasoning, tool_calls, finish_reason, usage_tokens))
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
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
        // T6: within-batch dedupe (hermes `_deduplicate_tool_calls`) — only
        // the first occurrence of each (tool, arguments) pair in a single
        // assistant message executes; exact duplicates are skipped with a
        // synthetic result so result ordering is preserved and degenerate
        // batches don't double-mutate.
        let mut seen_tool_calls: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        for (idx, mut tool_call) in tool_calls.into_iter().enumerate() {
            // Check interrupt flag
            if self.interrupt_flag.is_triggered() {
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    "Skipped: interrupted by user (Ctrl-C)".to_string(),
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
                Err(_) => {
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
                    // reaches phase 2. Log lengths only — raw model
                    // output does not go into logs.
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
                                "Unrepairable tool arguments — call not executed, error returned to the model"
                            );
                            early_results[idx] = Some(ToolResult::error_with_name(
                                &name,
                                &tool_call.id,
                                "Arguments could not be parsed and were NOT executed. \
                                 Reply with corrected arguments on the next turn if the user still wants this.",
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
                let decision = {
                    let mut g = self
                        .tool_guardrails
                        .lock()
                        .expect("tool_guardrails lock poisoned");
                    g.observe(&name, &args_str)
                };
                match decision {
                    GuardrailDecision::Allow => {}
                    GuardrailDecision::Warn => {
                        let count = self
                            .tool_guardrails
                            .lock()
                            .expect("tool_guardrails lock poisoned")
                            .count_of(&name, &args_str);
                        warn!(
                            tool = %name,
                            count,
                            "Repeated identical tool call — warning model"
                        );
                        self.emit(AgentEvent::Content {
                            text: format!(
                                "⚠ Tool '{name}' has been called with identical arguments {count} times this turn."
                            ),
                        })
                        .await;
                    }
                    GuardrailDecision::Skip => {
                        let count = self
                            .tool_guardrails
                            .lock()
                            .expect("tool_guardrails lock poisoned")
                            .count_of(&name, &args_str);
                        warn!(
                            tool = %name,
                            count,
                            "Repeated identical tool call — skipping duplicate"
                        );
                        self.metrics.record_guardrail_skip();
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            crate::tool_guardrails::build_skip_message(&name, count),
                        ));
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
            if self.config.approval_mode != "off" {
                let approval_result = crate::approval::check_tool_approval(
                    &name,
                    &args,
                    Some(&self.config.approval_mode),
                );
                match approval_result.verdict.as_str() {
                    "blocked" => {
                        warn!(tool = %name, "Tool call blocked by approval guard");
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            format!(
                                "Blocked by security policy: {}",
                                approval_result
                                    .reason
                                    .unwrap_or_else(|| "blocked".to_string())
                            ),
                        ));
                        continue;
                    }
                    "requires_approval" => {
                        warn!(tool = %name, "Tool call flagged — will prompt user");
                    }
                    _ => {}
                }
            }

            // ── P1/P2 seat policy (permission-genome slices E + F2) ──────
            // With a seat authority attached AND a session id (the employee
            // id), `decide()` owns the verdict for this seat: `Run` skips the
            // prompt outright (a yolo mode, a seat allow entry, or — since
            // F2 — a standing grant in the ledger), `Escalate` persists the
            // ask to the `pending_requests` queue (deduplicated: a second
            // ask for the same (seat, tool) references the first row, never
            // duplicates it) and then enters the channel below with the
            // policy's `why` as the danger explanation, and `Deny` is
            // returned shaped like a declined permission. Everything sits
            // behind the Option: no authority, no session id, or no policy
            // row for this seat → `None` → the branches below are today's
            // allowlist-then-hardcoded-list logic, unchanged.
            //
            // The consult is hoisted ABOVE the permission-channel block so
            // unattended agents (which may still carry a channel nobody
            // drains — see `with_unattended`) consult the genome too.
            //
            // `blocked` is `false` because the smart approval gate above
            // already denied blocked tools — the hardline floor fires before
            // any seat mode can see (let alone widen) a blocked call.
            // `dangerous` reuses `is_permission_gated_tool`, the same list
            // the prompt below has always keyed on (one list so the gate and
            // the predicate cannot drift apart).
            //
            // `has_grant` is REAL since F2 (plan §4 rule 6 completed): the
            // ledger's unexpired, unrevoked grant naming THIS tool exactly.
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
                warn!(tool = %name, "Tool call escalated by seat policy — unattended run: queue + deny");
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
                            early_results[idx] = Some(ToolResult::error(
                                &tool_call.id,
                                format!("Permission denied by seat policy: {why}"),
                            ));
                            continue;
                        }
                    }
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
            let results = early_results.into_iter().flatten().collect();
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
            let tool_ctx = ToolContext::default().with_metadata(
                "session_id",
                self.session_id().unwrap_or_else(|| "default".to_string()),
            );
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
            let result = timeout(limit, tool_future).await;
            if let Some(ref bus) = self.turn_end_bus {
                bus.record_tool_duration(tool_started.elapsed().as_millis() as u64);
            }
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
                let tool_ctx = ToolContext::default().with_metadata(
                    "session_id",
                    self.session_id().unwrap_or_else(|| "default".to_string()),
                );
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
                        let tool_ctx = ToolContext::default().with_metadata(
                            "session_id",
                            self.session_id().unwrap_or_else(|| "default".to_string()),
                        );
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

            // Execute all futures concurrently and collect results
            let results = stream::iter(futures)
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;

            // Place results in the correct position
            for (idx, result) in results {
                early_results[idx] = Some(result);
            }
        }

        // Collect results in original order
        // (iter-141 — fixed A20/A21: was .unwrap() which panics if a
        // future was cancelled. Use flatten() to gracefully skip None.)
        let results = early_results.into_iter().flatten().collect();
        Ok(results)
    }

    /// Run agent and handle self-healing on tool errors
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
                        "Note: The previous attempt encountered an error: {}. \
                        Please correct your approach and try again.",
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
