//! `tool_loop` — extracted verbatim from the former loop_.rs monolith.
//! Re-exported from `loop_` so every import path is unchanged.

use crate::approval::{ApprovalManager, ApprovalRequest, ApprovalRequirement, ApprovalResponse};

/// CLI channel factory, injected by the binary. Returns a `Box<dyn Channel>` for interactive mode.
use super::*;

#[allow(clippy::too_many_arguments)]
pub async fn run_tool_call_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    approval: Option<&ApprovalManager>,
    channel_name: &str,
    channel_reply_target: Option<&str>,
    multimodal_config: &operant_config::schema::MultimodalConfig,
    max_tool_iterations: usize,
    cancellation_token: Option<CancellationToken>,
    on_delta: Option<tokio::sync::mpsc::Sender<DraftEvent>>,
    hooks: Option<&crate::hooks::HookRunner>,
    excluded_tools: &[String],
    dedup_exempt_tools: &[String],
    activated_tools: Option<&std::sync::Arc<std::sync::Mutex<crate::tools::ActivatedToolSet>>>,
    model_switch_callback: Option<ModelSwitchCallback>,
    pacing: &operant_config::schema::PacingConfig,
    max_tool_result_chars: usize,
    context_token_budget: usize,
    shared_budget: Option<Arc<std::sync::atomic::AtomicUsize>>,
    channel: Option<&dyn Channel>,
    receipt_generator: Option<&crate::agent::tool_receipts::ReceiptGenerator>,
    collected_receipts: Option<&std::sync::Mutex<Vec<String>>>,
) -> Result<String> {
    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    let turn_id = Uuid::new_v4().to_string();
    let loop_started_at = Instant::now();
    let loop_ignore_tools: HashSet<&str> = pacing
        .loop_ignore_tools
        .iter()
        .map(String::as_str)
        .collect();
    let mut consecutive_identical_outputs: usize = 0;
    let mut last_tool_output_hash: Option<u64> = None;
    let mut empty_response_retries: usize = 0;
    // Real-work iteration count. Empty-response retries below "refund" their
    // slot, so the caller's max_iterations budget is reserved for real work
    // (OperantAgent R4 refund parity) while the loop bound keeps headroom for
    // the retry ladder on tiny budgets (e.g. `--max-iterations 2`).
    let mut real_iterations: usize = 0;

    let mut loop_detector = crate::agent::loop_detector::LoopDetector::new(
        crate::agent::loop_detector::LoopDetectorConfig {
            enabled: pacing.loop_detection_enabled,
            window_size: pacing.loop_detection_window_size,
            max_repeats: pacing.loop_detection_max_repeats,
        },
    );

    // Accumulated display text across all tool-loop calls.
    let mut accumulated_display_text = String::new();

    // Reserve retry headroom so empty-response retries don't consume the
    // caller's real iteration budget (OperantAgent R4 refund parity). With a
    // small budget (e.g. `--max-iterations 2`), the bounded retry ladder must
    // be able to run without exhausting it — the extra slots go unused when
    // no empty responses occur, and loop detection still bounds runaways.
    for iteration in 0..(max_iterations + EMPTY_RESPONSE_MAX_RETRIES) {
        let mut seen_tool_signatures: HashSet<(String, String)> = HashSet::new();

        // Real-budget gate: retry passes refund their slot, so only genuine
        // work counts against max_iterations. Exceeding it falls through to
        // the exhaustion path below the loop (same semantics as the original
        // `0..max_iterations` bound for non-retry callers).
        real_iterations += 1;
        if real_iterations > max_iterations {
            break;
        }

        if cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(ToolLoopCancelled.into());
        }

        // Shared iteration budget: parent + subagents share a global counter
        if let Some(ref budget) = shared_budget {
            let remaining = budget.load(std::sync::atomic::Ordering::Relaxed);
            if remaining == 0 {
                tracing::warn!("Shared iteration budget exhausted at iteration {iteration}");
                break;
            }
            budget.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Preemptive context management: trim history before it overflows
        if context_token_budget > 0 {
            let estimated = estimate_history_tokens(history);
            if estimated > context_token_budget {
                tracing::info!(
                    estimated,
                    budget = context_token_budget,
                    iteration = iteration + 1,
                    "Preemptive context trim: estimated tokens exceed budget"
                );
                let chars_saved = fast_trim_tool_results(history, 4);
                if chars_saved > 0 {
                    tracing::info!(chars_saved, "Preemptive fast-trim applied");
                }
                // If still over budget, use the history pruner for deeper cleanup
                let recheck = estimate_history_tokens(history);
                if recheck > context_token_budget {
                    let stats = crate::agent::history_pruner::prune_history(
                        history,
                        &crate::agent::history_pruner::HistoryPrunerConfig {
                            enabled: true,
                            max_tokens: context_token_budget,
                            keep_recent: 4,
                            collapse_tool_results: true,
                        },
                    );
                    if stats.dropped_messages > 0 || stats.collapsed_pairs > 0 {
                        tracing::info!(
                            collapsed = stats.collapsed_pairs,
                            dropped = stats.dropped_messages,
                            "Preemptive history prune applied"
                        );
                    }
                }
            }
        }

        // Remove orphaned tool-role messages whose assistant (tool_calls)
        // counterpart was dropped by proactive trimming, context compression,
        // or session history reloading.  Without this, providers like MiniMax
        // reject the request with "tool result's tool id not found" (bug #5743).
        crate::agent::history_pruner::remove_orphaned_tool_messages(history);
        normalize_system_messages(history);

        // Check if model switch was requested via model_switch tool
        if let Some(ref callback) = model_switch_callback
            && let Ok(guard) = callback.lock()
            && let Some((new_provider, new_model)) = guard.as_ref()
            && (new_provider != provider_name || new_model != model)
        {
            tracing::info!(
                "Model switch detected: {} {} -> {} {}",
                provider_name,
                model,
                new_provider,
                new_model
            );
            return Err(ModelSwitchRequested {
                provider: new_provider.clone(),
                model: new_model.clone(),
            }
            .into());
        }

        // Rebuild tool_specs each iteration so newly activated deferred tools appear.
        let mut tool_specs: Vec<crate::tools::ToolSpec> = tools_registry
            .iter()
            .filter(|tool| !excluded_tools.iter().any(|ex| ex == tool.name()))
            .map(|tool| tool.spec())
            .collect();
        if let Some(at) = activated_tools {
            for spec in at.lock().unwrap().tool_specs() {
                if !excluded_tools.iter().any(|ex| ex == &spec.name) {
                    tool_specs.push(spec);
                }
            }
        }
        let use_native_tools = provider.supports_native_tools() && !tool_specs.is_empty();

        let image_marker_count = multimodal::count_image_markers(history);

        // ── Vision provider routing ──────────────────────────
        // When the default provider lacks vision support but a dedicated
        // vision_provider is configured, create it on demand and use it
        // for this iteration.  Otherwise, preserve the original error.
        let vision_provider_box: Option<Box<dyn Provider>> = if image_marker_count > 0
            && !provider.supports_vision()
        {
            if let Some(ref vp) = multimodal_config.vision_provider {
                let vp_instance = operant_providers::create_provider(vp, None)
                    .map_err(|e| anyhow::anyhow!("failed to create vision provider '{vp}': {e}"))?;
                if !vp_instance.supports_vision() {
                    return Err(ProviderCapabilityError {
                        provider: vp.clone(),
                        capability: "vision".to_string(),
                        message: format!(
                            "configured vision_provider '{vp}' does not support vision input"
                        ),
                    }
                    .into());
                }
                Some(vp_instance)
            } else {
                return Err(ProviderCapabilityError {
                        provider: provider_name.to_string(),
                        capability: "vision".to_string(),
                        message: format!(
                            "received {image_marker_count} image marker(s), but this provider does not support vision input"
                        ),
                    }
                    .into());
            }
        } else {
            None
        };

        let (active_provider, active_provider_name, active_model): (&dyn Provider, &str, &str) =
            if let Some(ref vp_box) = vision_provider_box {
                let vp_name = multimodal_config
                    .vision_provider
                    .as_deref()
                    .unwrap_or(provider_name);
                let vm = multimodal_config.vision_model.as_deref().unwrap_or(model);
                (vp_box.as_ref(), vp_name, vm)
            } else {
                (provider, provider_name, model)
            };

        let prepared_messages =
            multimodal::prepare_messages_for_provider(history, multimodal_config).await?;

        // ── Progress: LLM thinking ────────────────────────────
        if let Some(ref tx) = on_delta {
            let phase = if iteration == 0 {
                "\u{1f914} Thinking...\n".to_string()
            } else {
                format!("\u{1f914} Thinking (round {})...\n", iteration + 1)
            };
            let _ = tx.send(StreamDelta::Status(phase)).await;
        }

        observer.record_event(&ObserverEvent::LlmRequest {
            provider: active_provider_name.to_string(),
            model: active_model.to_string(),
            messages_count: history.len(),
        });
        runtime_trace::record_event(
            "llm_request",
            Some(channel_name),
            Some(active_provider_name),
            Some(active_model),
            Some(&turn_id),
            None,
            None,
            serde_json::json!({
                "iteration": iteration + 1,
                "messages_count": history.len(),
            }),
        );

        let llm_started_at = Instant::now();

        // Fire void hook before LLM call
        if let Some(hooks) = hooks {
            hooks.fire_llm_input(history, model).await;
        }

        // Budget enforcement — block if limit exceeded (no-op when not scoped)
        if let Some(BudgetCheck::Exceeded {
            current_usd,
            limit_usd,
            period,
        }) = check_tool_loop_budget()
        {
            return Err(anyhow::anyhow!(
                "Budget exceeded: ${:.4} of ${:.2} {:?} limit. Cannot make further API calls until the budget resets.",
                current_usd,
                limit_usd,
                period
            ));
        }

        // Unified path via Provider::chat so provider-specific native tool logic
        // (OpenAI/Anthropic/OpenRouter/compatible adapters) is honored.
        let request_tools = if use_native_tools {
            Some(tool_specs.as_slice())
        } else {
            None
        };
        let should_consume_provider_stream = on_delta.is_some()
            && provider.supports_streaming()
            && (request_tools.is_none() || provider.supports_streaming_tool_events());
        tracing::debug!(
            has_on_delta = on_delta.is_some(),
            supports_streaming = provider.supports_streaming(),
            should_consume_provider_stream,
            "Streaming decision for iteration {}",
            iteration + 1,
        );
        let mut streamed_live_deltas = false;

        let chat_result = if should_consume_provider_stream {
            match consume_provider_streaming_response(
                active_provider,
                &prepared_messages.messages,
                request_tools,
                active_model,
                temperature,
                cancellation_token.as_ref(),
                on_delta.as_ref(),
            )
            .await
            {
                Ok(streamed) => {
                    streamed_live_deltas = streamed.forwarded_live_deltas;
                    let reasoning_content = if streamed.reasoning_content.is_empty() {
                        None
                    } else {
                        Some(streamed.reasoning_content)
                    };
                    Ok(operant_providers::ChatResponse {
                        text: Some(streamed.response_text),
                        tool_calls: streamed.tool_calls,
                        usage: streamed.usage,
                        reasoning_content,
                    })
                }
                Err(stream_err) => {
                    tracing::warn!(
                        provider = active_provider_name,
                        model = active_model,
                        iteration = iteration + 1,
                        "provider streaming failed, falling back to non-streaming chat: {stream_err}"
                    );
                    runtime_trace::record_event(
                        "llm_stream_fallback",
                        Some(channel_name),
                        Some(active_provider_name),
                        Some(active_model),
                        Some(&turn_id),
                        Some(false),
                        Some("provider stream failed; fallback to non-streaming chat"),
                        serde_json::json!({
                            "iteration": iteration + 1,
                            "error": scrub_credentials(&stream_err.to_string()),
                        }),
                    );
                    {
                        let chat_future = active_provider.chat(
                            ChatRequest {
                                messages: &prepared_messages.messages,
                                tools: request_tools,
                            },
                            active_model,
                            Some(temperature),
                        );
                        if let Some(token) = cancellation_token.as_ref() {
                            tokio::select! {
                                () = token.cancelled() => Err(ToolLoopCancelled.into()),
                                result = chat_future => result,
                            }
                        } else {
                            chat_future.await
                        }
                    }
                }
            }
        } else {
            // Non-streaming path: wrap with optional per-step timeout from
            // pacing config to catch hung model responses.
            let chat_future = active_provider.chat(
                ChatRequest {
                    messages: &prepared_messages.messages,
                    tools: request_tools,
                },
                active_model,
                Some(temperature),
            );

            match pacing.step_timeout_secs {
                Some(step_secs) if step_secs > 0 => {
                    let step_timeout = Duration::from_secs(step_secs);
                    if let Some(token) = cancellation_token.as_ref() {
                        tokio::select! {
                            () = token.cancelled() => return Err(ToolLoopCancelled.into()),
                            result = tokio::time::timeout(step_timeout, chat_future) => {
                                match result {
                                    Ok(inner) => inner,
                                    Err(_) => anyhow::bail!(
                                        "LLM inference step timed out after {step_secs}s (step_timeout_secs)"
                                    ),
                                }
                            },
                        }
                    } else {
                        match tokio::time::timeout(step_timeout, chat_future).await {
                            Ok(inner) => inner,
                            Err(_) => anyhow::bail!(
                                "LLM inference step timed out after {step_secs}s (step_timeout_secs)"
                            ),
                        }
                    }
                }
                _ => {
                    if let Some(token) = cancellation_token.as_ref() {
                        tokio::select! {
                            () = token.cancelled() => return Err(ToolLoopCancelled.into()),
                            result = chat_future => result,
                        }
                    } else {
                        chat_future.await
                    }
                }
            }
        };

        let (
            response_text,
            parsed_text,
            tool_calls,
            assistant_history_content,
            native_tool_calls,
            _parse_issue_detected,
            response_streamed_live,
            response_reasoning,
        ) = match chat_result {
            Ok(resp) => {
                let (resp_input_tokens, resp_output_tokens) = resp
                    .usage
                    .as_ref()
                    .map(|u| (u.input_tokens, u.output_tokens))
                    .unwrap_or((None, None));

                observer.record_event(&ObserverEvent::LlmResponse {
                    provider: provider_name.to_string(),
                    model: model.to_string(),
                    duration: llm_started_at.elapsed(),
                    success: true,
                    error_message: None,
                    input_tokens: resp_input_tokens,
                    output_tokens: resp_output_tokens,
                });

                // Record cost via task-local tracker (no-op when not scoped)
                let _ = resp
                    .usage
                    .as_ref()
                    .and_then(|usage| record_tool_loop_cost_usage(provider_name, model, usage));

                let response_text = if tool_specs.is_empty() {
                    strip_think_tags(resp.text_or_empty())
                } else {
                    resp.text_or_empty().to_string()
                };
                // First try native structured tool calls (OpenAI-format).
                // Fall back to text-based parsing (XML tags, markdown blocks,
                // GLM format) only if the provider returned no native calls —
                // this ensures we support both native and prompt-guided models.
                let mut calls: Vec<ParsedToolCall> = if tool_specs.is_empty() {
                    Vec::new()
                } else {
                    resp.tool_calls
                        .iter()
                        .map(|call| ParsedToolCall {
                            name: call.name.clone(),
                            arguments: serde_json::from_str::<serde_json::Value>(&call.arguments)
                                .unwrap_or_else(|_| {
                                    serde_json::Value::Object(serde_json::Map::new())
                                }),
                            tool_call_id: Some(call.id.clone()),
                        })
                        .collect()
                };
                let mut parsed_text = String::new();

                if calls.is_empty() && !tool_specs.is_empty() {
                    let (fallback_text, fallback_calls) = parse_tool_calls(&response_text);
                    if !fallback_text.is_empty() {
                        parsed_text = fallback_text;
                    }
                    calls = fallback_calls;
                }

                let parse_issue = if tool_specs.is_empty() {
                    None
                } else {
                    detect_tool_call_parse_issue(&response_text, &calls)
                };
                if let Some(ref issue) = parse_issue {
                    runtime_trace::record_event(
                        "tool_call_parse_issue",
                        Some(channel_name),
                        Some(provider_name),
                        Some(model),
                        Some(&turn_id),
                        Some(false),
                        Some(issue.as_str()),
                        serde_json::json!({
                            "iteration": iteration + 1,
                            "response_excerpt": truncate_with_ellipsis(
                                &scrub_credentials(&response_text),
                                600
                            ),
                        }),
                    );
                }

                runtime_trace::record_event(
                    "llm_response",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(true),
                    None,
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "duration_ms": llm_started_at.elapsed().as_millis(),
                        "input_tokens": resp_input_tokens,
                        "output_tokens": resp_output_tokens,
                        "raw_response": scrub_credentials(&response_text),
                        "native_tool_calls": resp.tool_calls.len(),
                        "parsed_tool_calls": calls.len(),
                    }),
                );

                // Preserve native tool call IDs in assistant history so role=tool
                // follow-up messages can reference the exact call id.
                let reasoning_content = resp.reasoning_content.clone();
                let assistant_history_content = if resp.tool_calls.is_empty() {
                    if use_native_tools {
                        build_native_assistant_history_from_parsed_calls(
                            &response_text,
                            &calls,
                            reasoning_content.as_deref(),
                        )
                        .unwrap_or_else(|| response_text.clone())
                    } else {
                        response_text.clone()
                    }
                } else {
                    build_native_assistant_history(
                        &response_text,
                        &resp.tool_calls,
                        reasoning_content.as_deref(),
                    )
                };

                let native_calls = resp.tool_calls;
                (
                    response_text,
                    parsed_text,
                    calls,
                    assistant_history_content,
                    native_calls,
                    parse_issue.is_some(),
                    streamed_live_deltas,
                    reasoning_content,
                )
            }
            Err(e) => {
                let safe_error = operant_providers::sanitize_api_error(&e.to_string());
                observer.record_event(&ObserverEvent::LlmResponse {
                    provider: provider_name.to_string(),
                    model: model.to_string(),
                    duration: llm_started_at.elapsed(),
                    success: false,
                    error_message: Some(safe_error.clone()),
                    input_tokens: None,
                    output_tokens: None,
                });
                runtime_trace::record_event(
                    "llm_response",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some(&safe_error),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "duration_ms": llm_started_at.elapsed().as_millis(),
                    }),
                );

                // Context overflow recovery: trim history and retry
                if operant_providers::reliable::is_context_window_exceeded(&e) {
                    tracing::warn!(
                        iteration = iteration + 1,
                        "Context window exceeded, attempting in-loop recovery"
                    );

                    // Step 1: fast-trim old tool results (cheap)
                    let chars_saved = fast_trim_tool_results(history, 4);
                    if chars_saved > 0 {
                        tracing::info!(
                            chars_saved,
                            "Context recovery: trimmed old tool results, retrying"
                        );
                        continue;
                    }

                    // Step 2: emergency drop oldest non-system messages
                    let dropped = emergency_history_trim(history, 4);
                    if dropped > 0 {
                        tracing::info!(dropped, "Context recovery: dropped old messages, retrying");
                        continue;
                    }

                    // Nothing left to trim — truly unrecoverable
                    tracing::error!("Context overflow unrecoverable: no trimmable messages");
                }

                return Err(e);
            }
        };

        let display_text = resolve_display_text(
            &response_text,
            &parsed_text,
            !tool_calls.is_empty(),
            !native_tool_calls.is_empty(),
        );
        // ── Progress: LLM responded ─────────────────────────────
        if let Some(ref tx) = on_delta {
            let llm_secs = llm_started_at.elapsed().as_secs();
            if !tool_calls.is_empty() {
                let _ = tx
                    .send(StreamDelta::Status(format!(
                        "\u{1f4ac} Got {} tool call(s) ({llm_secs}s)\n",
                        tool_calls.len()
                    )))
                    .await;
            }
        }

        if tool_calls.is_empty() {
            // R23: empty-response retry (OperantAgent R4 parity). The CLI
            // path retries empty assistant responses up to max_retries times
            // with a nudge; the gateway path previously returned an empty
            // answer to the user immediately. Bounded so a model that
            // persistently returns nothing still terminates.
            if empty_response_retries < EMPTY_RESPONSE_MAX_RETRIES
                && response_text.trim().is_empty()
                && response_reasoning
                    .as_deref()
                    .is_none_or(|reasoning| reasoning.trim().is_empty())
            {
                empty_response_retries += 1;
                // Refund the iteration slot: retries must not consume the
                // caller's real budget (OperantAgent R4 refund parity).
                real_iterations -= 1;
                tracing::warn!(
                    retry = empty_response_retries,
                    max = EMPTY_RESPONSE_MAX_RETRIES,
                    "Empty assistant response — retrying"
                );
                // Append the empty assistant turn so the model sees its own
                // empty reply and is nudged to actually respond.
                history.push(ChatMessage::assistant(String::new()));
                continue;
            }
            runtime_trace::record_event(
                "turn_final_response",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                Some(true),
                None,
                serde_json::json!({
                    "iteration": iteration + 1,
                    "text": scrub_credentials(&display_text),
                }),
            );
            // No tool calls — this is the final response.
            accumulated_display_text.push_str(&display_text);

            // If text wasn't streamed live, send it now via post-hoc chunking.
            // When streamed live, the channel already received the deltas.
            if let Some(ref tx) = on_delta
                && !response_streamed_live
            {
                let mut chunk = String::new();
                for word in display_text.split_inclusive(char::is_whitespace) {
                    if cancellation_token
                        .as_ref()
                        .is_some_and(CancellationToken::is_cancelled)
                    {
                        return Err(ToolLoopCancelled.into());
                    }
                    chunk.push_str(word);
                    if chunk.len() >= STREAM_CHUNK_MIN_CHARS
                        && tx
                            .send(StreamDelta::Text(std::mem::take(&mut chunk)))
                            .await
                            .is_err()
                    {
                        break;
                    }
                }
                if !chunk.is_empty() {
                    let _ = tx.send(StreamDelta::Text(chunk)).await;
                }
            }

            history.push(ChatMessage::assistant(response_text.clone()));
            return Ok(accumulated_display_text);
        }

        // Accumulate text from this iteration (tool calls present, loop continues).
        accumulated_display_text.push_str(&display_text);

        // Native tool-call providers can return assistant text separately from
        // the structured call payload; relay it to draft-capable channels.
        if !display_text.is_empty() {
            if !native_tool_calls.is_empty()
                && let Some(ref tx) = on_delta
            {
                let mut narration = display_text.clone();
                if !narration.ends_with('\n') {
                    narration.push('\n');
                }
                let _ = tx.send(StreamDelta::Text(narration)).await;
            }
            if !silent {
                print!("{display_text}");
                let _ = std::io::stdout().flush();
            }
        }

        // Execute tool calls and build results. `individual_results` tracks per-call output so
        // native-mode history can emit one role=tool message per tool call with the correct ID.
        //
        // When multiple tool calls are present and interactive CLI approval is not needed, run
        // tool executions concurrently for lower wall-clock latency.
        let mut tool_results = String::new();
        let mut individual_results: Vec<(Option<String>, String)> = Vec::new();
        let mut ordered_results: Vec<Option<(String, Option<String>, ToolExecutionOutcome)>> =
            (0..tool_calls.len()).map(|_| None).collect();
        let allow_parallel_execution = should_execute_tools_in_parallel(&tool_calls, approval);
        let mut executable_indices: Vec<usize> = Vec::new();
        let mut executable_calls: Vec<ParsedToolCall> = Vec::new();

        for (idx, call) in tool_calls.iter().enumerate() {
            // ── Hook: before_tool_call (modifying) ──────────
            let mut tool_name = call.name.clone();
            let mut tool_args = call.arguments.clone();
            if let Some(hooks) = hooks {
                match hooks
                    .run_before_tool_call(tool_name.clone(), tool_args.clone())
                    .await
                {
                    crate::hooks::HookResult::Cancel(reason) => {
                        tracing::info!(tool = %call.name, %reason, "tool call cancelled by hook");
                        let cancelled = format!("Cancelled by hook: {reason}");
                        runtime_trace::record_event(
                            "tool_call_result",
                            Some(channel_name),
                            Some(provider_name),
                            Some(model),
                            Some(&turn_id),
                            Some(false),
                            Some(&cancelled),
                            serde_json::json!({
                                "iteration": iteration + 1,
                                "tool": call.name,
                                "arguments": scrub_credentials(&tool_args.to_string()),
                            }),
                        );
                        if let Some(ref tx) = on_delta {
                            let _ = tx
                                .send(StreamDelta::Status(format!(
                                    "\u{274c} {}: {}\n",
                                    call.name,
                                    truncate_with_ellipsis(&scrub_credentials(&cancelled), 200)
                                )))
                                .await;
                        }
                        ordered_results[idx] = Some((
                            call.name.clone(),
                            call.tool_call_id.clone(),
                            ToolExecutionOutcome {
                                output: cancelled,
                                success: false,
                                error_reason: Some(scrub_credentials(&reason)),
                                duration: Duration::ZERO,
                                receipt: None,
                            },
                        ));
                        continue;
                    }
                    crate::hooks::HookResult::Continue((name, args)) => {
                        tool_name = name;
                        tool_args = args;
                    }
                }
            }

            maybe_inject_channel_delivery_defaults(
                &tool_name,
                &mut tool_args,
                channel_name,
                channel_reply_target,
            );

            crate::agent::set_runtime_approved_arg(&tool_name, &mut tool_args, false);

            // ── Approval hook ────────────────────────────────
            let mut approval_requirement = approval
                .map(|mgr| mgr.approval_requirement(&tool_name))
                .unwrap_or(ApprovalRequirement::NotRequired);
            if let Some(mgr) = approval
                && approval_requirement == ApprovalRequirement::Prompt
            {
                let request = ApprovalRequest {
                    tool_name: tool_name.clone(),
                    arguments: tool_args.clone(),
                };

                // Interactive CLI: prompt the operator.
                // Non-interactive (channels): try the channel's inline
                // approval (e.g. Telegram inline keyboard) before falling
                // back to auto-deny.
                let decision = if mgr.is_non_interactive() {
                    let channel_decision = if let Some(ch) = channel {
                        let ch_request = operant_api::channel::ChannelApprovalRequest {
                            tool_name: request.tool_name.clone(),
                            arguments_summary: crate::approval::summarize_args(&request.arguments),
                        };
                        let recipient = channel_reply_target.unwrap_or_default();
                        match ch.request_approval(recipient, &ch_request).await {
                            Ok(Some(r)) => Some(r),
                            Ok(None) => None,
                            Err(e) => {
                                tracing::warn!("Channel approval request failed: {e}");
                                None
                            }
                        }
                    } else {
                        None
                    };
                    match channel_decision {
                        Some(operant_api::channel::ChannelApprovalResponse::Approve) => {
                            ApprovalResponse::Yes
                        }
                        Some(operant_api::channel::ChannelApprovalResponse::AlwaysApprove) => {
                            ApprovalResponse::Always
                        }
                        Some(operant_api::channel::ChannelApprovalResponse::Deny) => {
                            ApprovalResponse::No
                        }
                        // Channel doesn't support approval — auto-deny.
                        None => ApprovalResponse::No,
                    }
                } else {
                    mgr.prompt_cli(&request)
                };

                mgr.record_decision(&tool_name, &tool_args, decision, channel_name);

                if decision == ApprovalResponse::No {
                    let denied = "Denied by user.".to_string();
                    runtime_trace::record_event(
                        "tool_call_result",
                        Some(channel_name),
                        Some(provider_name),
                        Some(model),
                        Some(&turn_id),
                        Some(false),
                        Some(&denied),
                        serde_json::json!({
                            "iteration": iteration + 1,
                            "tool": tool_name.clone(),
                            "arguments": scrub_credentials(&tool_args.to_string()),
                        }),
                    );
                    if let Some(ref tx) = on_delta {
                        let _ = tx
                            .send(StreamDelta::Status(format!(
                                "\u{274c} {}: {}\n",
                                tool_name, denied
                            )))
                            .await;
                    }
                    ordered_results[idx] = Some((
                        tool_name.clone(),
                        call.tool_call_id.clone(),
                        ToolExecutionOutcome {
                            output: denied.clone(),
                            success: false,
                            error_reason: Some(denied),
                            duration: Duration::ZERO,
                            receipt: None,
                        },
                    ));
                    continue;
                }

                if matches!(decision, ApprovalResponse::Yes | ApprovalResponse::Always) {
                    approval_requirement = ApprovalRequirement::Approved;
                }
            }
            crate::agent::set_runtime_approved_arg(
                &tool_name,
                &mut tool_args,
                approval_requirement == ApprovalRequirement::Approved,
            );

            let signature = {
                let canonical_args = canonicalize_json_for_tool_signature(&tool_args);
                let args_json =
                    serde_json::to_string(&canonical_args).unwrap_or_else(|_| "{}".to_string());
                (tool_name.trim().to_ascii_lowercase(), args_json)
            };
            let dedup_exempt = dedup_exempt_tools.iter().any(|e| e == &tool_name);
            if !dedup_exempt && !seen_tool_signatures.insert(signature) {
                let duplicate = format!(
                    "Skipped duplicate tool call '{tool_name}' with identical arguments in this turn."
                );
                runtime_trace::record_event(
                    "tool_call_result",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some(&duplicate),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "tool": tool_name.clone(),
                        "arguments": scrub_credentials(&tool_args.to_string()),
                        "deduplicated": true,
                    }),
                );
                if let Some(ref tx) = on_delta {
                    let _ = tx
                        .send(StreamDelta::Status(format!(
                            "\u{274c} {}: {}\n",
                            tool_name, duplicate
                        )))
                        .await;
                }
                ordered_results[idx] = Some((
                    tool_name.clone(),
                    call.tool_call_id.clone(),
                    ToolExecutionOutcome {
                        output: duplicate.clone(),
                        success: false,
                        error_reason: Some(duplicate),
                        duration: Duration::ZERO,
                        receipt: None,
                    },
                ));
                continue;
            }

            runtime_trace::record_event(
                "tool_call_start",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                None,
                None,
                serde_json::json!({
                    "iteration": iteration + 1,
                    "tool": tool_name.clone(),
                    "arguments": scrub_credentials(&tool_args.to_string()),
                }),
            );

            // ── Progress: tool start ────────────────────────────
            if let Some(ref tx) = on_delta {
                let hint = {
                    let raw = match tool_name.as_str() {
                        "shell" => tool_args.get("command").and_then(|v| v.as_str()),
                        "file_read" | "file_write" => {
                            tool_args.get("path").and_then(|v| v.as_str())
                        }
                        _ => tool_args
                            .get("action")
                            .and_then(|v| v.as_str())
                            .or_else(|| tool_args.get("query").and_then(|v| v.as_str())),
                    };
                    match raw {
                        Some(s) => truncate_with_ellipsis(s, 60),
                        None => String::new(),
                    }
                };
                let progress = if hint.is_empty() {
                    format!("\u{23f3} {}\n", tool_name)
                } else {
                    format!("\u{23f3} {}: {hint}\n", tool_name)
                };
                tracing::debug!(tool = %tool_name, "Sending progress start to draft");
                let _ = tx.send(StreamDelta::Status(progress)).await;
            }

            executable_indices.push(idx);
            executable_calls.push(ParsedToolCall {
                name: tool_name,
                arguments: tool_args,
                tool_call_id: call.tool_call_id.clone(),
            });
        }

        let executed_outcomes = if allow_parallel_execution && executable_calls.len() > 1 {
            execute_tools_parallel(
                &executable_calls,
                tools_registry,
                activated_tools,
                observer,
                cancellation_token.as_ref(),
                receipt_generator,
            )
            .await?
        } else {
            execute_tools_sequential(
                &executable_calls,
                tools_registry,
                activated_tools,
                observer,
                cancellation_token.as_ref(),
                receipt_generator,
            )
            .await?
        };

        for ((idx, call), outcome) in executable_indices
            .iter()
            .zip(executable_calls.iter())
            .zip(executed_outcomes)
        {
            runtime_trace::record_event(
                "tool_call_result",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                Some(outcome.success),
                outcome.error_reason.as_deref(),
                serde_json::json!({
                    "iteration": iteration + 1,
                    "tool": call.name.clone(),
                    "duration_ms": outcome.duration.as_millis(),
                    "output": scrub_credentials(&outcome.output),
                }),
            );

            // ── Hook: after_tool_call (void) ─────────────────
            if let Some(hooks) = hooks {
                let tool_result_obj = crate::tools::ToolResult {
                    success: outcome.success,
                    output: outcome.output.clone(),
                    error: None,
                };
                hooks
                    .fire_after_tool_call(&call.name, &tool_result_obj, outcome.duration)
                    .await;
            }

            // ── Progress: tool completion ───────────────────────
            if let Some(ref tx) = on_delta {
                let secs = outcome.duration.as_secs();
                let progress_msg = if outcome.success {
                    format!("\u{2705} {} ({secs}s)\n", call.name)
                } else if let Some(ref reason) = outcome.error_reason {
                    format!(
                        "\u{274c} {} ({secs}s): {}\n",
                        call.name,
                        truncate_with_ellipsis(reason, 200)
                    )
                } else {
                    format!("\u{274c} {} ({secs}s)\n", call.name)
                };
                tracing::debug!(tool = %call.name, secs, "Sending progress complete to draft");
                let _ = tx.send(StreamDelta::Status(progress_msg)).await;
            }

            ordered_results[*idx] = Some((call.name.clone(), call.tool_call_id.clone(), outcome));
        }

        // Collect tool results and build per-tool output for loop detection.
        // Only non-ignored tool outputs contribute to the identical-output hash.
        let mut detection_relevant_output = String::new();
        // Use enumerate *before* filter_map so result_index stays aligned with
        // tool_calls even when some ordered_results entries are None.
        for (result_index, (tool_name, tool_call_id, outcome)) in ordered_results
            .into_iter()
            .enumerate()
            .filter_map(|(i, opt)| opt.map(|v| (i, v)))
        {
            if !loop_ignore_tools.contains(tool_name.as_str()) {
                detection_relevant_output.push_str(&outcome.output);

                // Feed the pattern-based loop detector with name + args + result.
                let args = tool_calls
                    .get(result_index)
                    .map(|c| &c.arguments)
                    .unwrap_or(&serde_json::Value::Null);
                let det_result = loop_detector.record(&tool_name, args, &outcome.output);
                match det_result {
                    crate::agent::loop_detector::LoopDetectionResult::Ok => {}
                    crate::agent::loop_detector::LoopDetectionResult::Warning(ref msg) => {
                        tracing::warn!(tool = %tool_name, %msg, "loop detector warning");
                        append_or_merge_system_message(history, format!("[Loop Detection] {msg}"));
                    }
                    crate::agent::loop_detector::LoopDetectionResult::Block(ref msg) => {
                        tracing::warn!(tool = %tool_name, %msg, "loop detector blocked tool call");
                        // Replace the tool output with the block message.
                        // We still continue the loop so the LLM sees the block feedback.
                        append_or_merge_system_message(
                            history,
                            format!("[Loop Detection — BLOCKED] {msg}"),
                        );
                    }
                    crate::agent::loop_detector::LoopDetectionResult::Break(msg) => {
                        runtime_trace::record_event(
                            "loop_detector_circuit_breaker",
                            Some(channel_name),
                            Some(provider_name),
                            Some(model),
                            Some(&turn_id),
                            Some(false),
                            Some(&msg),
                            serde_json::json!({
                                "iteration": iteration + 1,
                                "tool": tool_name,
                            }),
                        );
                        anyhow::bail!("Agent loop aborted by loop detector: {msg}");
                    }
                }
            }
            let canonical_output = canonicalize_tool_result_media_markers(&outcome.output);
            let mut result_output = truncate_tool_result(&canonical_output, max_tool_result_chars);
            // Append HMAC receipt to tool result when receipts are enabled (#4830)
            if let Some(ref receipt) = outcome.receipt {
                tracing::debug!(tool = %tool_name, receipt = %receipt, "Tool receipt generated");
                result_output = format!("{result_output}\n\n[receipt: {receipt}]");
                if let Some(store) = collected_receipts
                    && let Ok(mut v) = store.lock()
                {
                    v.push(format!("{tool_name}: {receipt}"));
                }
            }
            individual_results.push((tool_call_id, result_output.clone()));
            let _ = writeln!(
                tool_results,
                "<tool_result name=\"{}\">\n{}\n</tool_result>",
                tool_name, result_output
            );
        }

        // ── Time-gated loop detection ──────────────────────────
        // When pacing.loop_detection_min_elapsed_secs is set, identical-output
        // loop detection activates after the task has been running that long.
        // This avoids false-positive aborts on long-running browser/research
        // workflows while keeping aggressive protection for quick tasks.
        // When not configured, identical-output detection is disabled (preserving
        // existing behavior where only max_iterations prevents runaway loops).
        let loop_detection_active = match pacing.loop_detection_min_elapsed_secs {
            Some(min_secs) => loop_started_at.elapsed() >= Duration::from_secs(min_secs),
            None => false, // disabled when not configured (backwards compatible)
        };

        if loop_detection_active && !detection_relevant_output.is_empty() {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            detection_relevant_output.hash(&mut hasher);
            let current_hash = hasher.finish();

            if last_tool_output_hash == Some(current_hash) {
                consecutive_identical_outputs += 1;
            } else {
                consecutive_identical_outputs = 0;
                last_tool_output_hash = Some(current_hash);
            }

            // Bail if we see 3+ consecutive identical tool outputs (clear runaway).
            if consecutive_identical_outputs >= 3 {
                runtime_trace::record_event(
                    "tool_loop_identical_output_abort",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some("identical tool output detected 3 consecutive times"),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "consecutive_identical": consecutive_identical_outputs,
                    }),
                );
                anyhow::bail!(
                    "Agent loop aborted: identical tool output detected {} consecutive times",
                    consecutive_identical_outputs
                );
            }
        }

        // Add assistant message with tool calls + tool results to history.
        // Native mode: use JSON-structured messages so convert_messages() can
        // reconstruct proper OpenAI-format tool_calls and tool result messages.
        // Prompt mode: use XML-based text format as before.
        history.push(ChatMessage::assistant(assistant_history_content));
        if native_tool_calls.is_empty() {
            let all_results_have_ids = use_native_tools
                && !individual_results.is_empty()
                && individual_results
                    .iter()
                    .all(|(tool_call_id, _)| tool_call_id.is_some());
            if all_results_have_ids {
                for (tool_call_id, result) in &individual_results {
                    let tool_msg = serde_json::json!({
                        "tool_call_id": tool_call_id,
                        "content": result,
                    });
                    history.push(ChatMessage::tool(tool_msg.to_string()));
                }
            } else {
                history.push(ChatMessage::user(format!("[Tool results]\n{tool_results}")));
            }
        } else {
            for (native_call, (_, result)) in
                native_tool_calls.iter().zip(individual_results.iter())
            {
                let tool_msg = serde_json::json!({
                    "tool_call_id": native_call.id,
                    "content": result,
                });
                history.push(ChatMessage::tool(tool_msg.to_string()));
            }
        }
    }

    runtime_trace::record_event(
        "tool_loop_exhausted",
        Some(channel_name),
        Some(provider_name),
        Some(model),
        Some(&turn_id),
        Some(false),
        Some("agent exceeded maximum tool iterations"),
        serde_json::json!({
            "max_iterations": max_iterations,
        }),
    );

    // Graceful shutdown: ask the LLM for a final summary without tools
    tracing::warn!(
        max_iterations,
        "Max iterations reached, requesting final summary"
    );
    history.push(ChatMessage::user(
        "You have reached the maximum number of tool iterations. \
         Please provide your best answer based on the work completed so far. \
         Summarize what you accomplished and what remains to be done."
            .to_string(),
    ));

    let summary_request = operant_providers::ChatRequest {
        messages: history,
        tools: None, // No tools — force a text response
    };
    match provider
        .chat(summary_request, model, Some(temperature))
        .await
    {
        Ok(resp) => {
            let text = resp.text.unwrap_or_default();
            if text.is_empty() {
                anyhow::bail!("Agent exceeded maximum tool iterations ({max_iterations})")
            }
            accumulated_display_text.push_str(&text);
            Ok(accumulated_display_text)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Final summary LLM call failed, bailing");
            anyhow::bail!("Agent exceeded maximum tool iterations ({max_iterations})")
        }
    }
}
