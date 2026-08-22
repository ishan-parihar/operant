//! `dispatch` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use anyhow::Result;
use operant_memory;
use operant_providers::reliable::{scope_provider_fallback, take_last_provider_fallback};
use operant_providers::{self, ChatMessage, Provider};
use operant_runtime::agent::loop_::{
    clear_model_switch_request, get_model_switch_state, is_model_switch_requested,
    run_tool_call_loop, scope_session_key, scope_thread_id, scrub_credentials,
};
use operant_runtime::observability::traits::ObserverEvent;
use operant_runtime::observability::{Observer, runtime_trace};
use operant_runtime::security::AutonomyLevel;
use operant_runtime::util::truncate_with_ellipsis;
use portable_atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::*;

pub(crate) async fn process_channel_message(
    ctx: Arc<ChannelRuntimeContext>,
    msg: operant_api::channel::ChannelMessage,
    cancellation_token: CancellationToken,
) {
    if cancellation_token.is_cancelled() {
        return;
    }

    println!(
        "  💬 [{}] from {}: {}",
        msg.channel,
        msg.sender,
        truncate_with_ellipsis(&msg.content, 80)
    );
    runtime_trace::record_event(
        "channel_message_inbound",
        Some(msg.channel.as_str()),
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "sender": msg.sender,
            "message_id": msg.id,
            "reply_target": msg.reply_target,
            "content_preview": truncate_with_ellipsis(&msg.content, 160),
        }),
    );

    // ── Hook: on_message_received (modifying) ────────────
    let mut msg = if let Some(hooks) = &ctx.hooks {
        match hooks.run_on_message_received(msg).await {
            operant_runtime::hooks::HookResult::Cancel(reason) => {
                tracing::info!(%reason, "incoming message dropped by hook");
                return;
            }
            operant_runtime::hooks::HookResult::Continue(modified) => modified,
        }
    } else {
        msg
    };

    // ── Media pipeline: enrich inbound message with media annotations ──
    if ctx.media_pipeline.enabled && !msg.attachments.is_empty() {
        let vision = ctx.provider.supports_vision();
        let pipeline = media_pipeline::MediaPipeline::new(
            &ctx.media_pipeline,
            &ctx.transcription_config,
            vision,
        );
        msg.content = Box::pin(pipeline.process(&msg.content, &msg.attachments)).await;
    }

    // ── Link enricher: prepend URL summaries before agent sees the message ──
    let le_config = &ctx.prompt_config.link_enricher;
    if le_config.enabled {
        let enricher_cfg = link_enricher::LinkEnricherConfig {
            enabled: le_config.enabled,
            max_links: le_config.max_links,
            timeout_secs: le_config.timeout_secs,
        };
        let enriched = link_enricher::enrich_message(&msg.content, &enricher_cfg).await;
        if enriched != msg.content {
            tracing::info!(
                channel = %msg.channel,
                sender = %msg.sender,
                "Link enricher: prepended URL summaries to message"
            );
            msg.content = enriched;
        }
    }

    let target_channel = ctx
        .channels_by_name
        .get(&msg.channel)
        .or_else(|| {
            // Multi-room channels use "name:qualifier" format (e.g. "matrix:!roomId");
            // fall back to base channel name for routing.
            msg.channel
                .split_once(':')
                .and_then(|(base, _)| ctx.channels_by_name.get(base))
        })
        .cloned();
    if let Err(err) = maybe_apply_runtime_config_update(ctx.as_ref()).await {
        tracing::warn!("Failed to apply runtime config update: {err}");
    }
    if handle_runtime_command_if_needed(ctx.as_ref(), &msg, target_channel.as_ref()).await {
        return;
    }

    let history_key = conversation_history_key(&msg);
    let mut route = get_route_selection(ctx.as_ref(), &history_key);

    // ── Query classification: override route when a rule matches ──
    if let Some(hint) =
        operant_runtime::agent::classifier::classify(&ctx.query_classification, &msg.content)
        && let Some(matched_route) = ctx
            .model_routes
            .iter()
            .find(|r| r.hint.eq_ignore_ascii_case(&hint))
    {
        tracing::info!(
            target: "query_classification",
            hint = hint.as_str(),
            provider = matched_route.provider.as_str(),
            model = matched_route.model.as_str(),
            channel = %msg.channel,
            "Channel message classified — overriding route"
        );
        route = ChannelRouteSelection {
            provider: matched_route.provider.clone(),
            model: matched_route.model.clone(),
            api_key: matched_route.api_key.clone(),
        };
    }

    let runtime_defaults = runtime_defaults_snapshot(ctx.as_ref());
    let mut active_provider =
        match get_or_create_provider(ctx.as_ref(), &route.provider, route.api_key.as_deref()).await
        {
            Ok(provider) => provider,
            Err(err) => {
                let safe_err = operant_providers::sanitize_api_error(&err.to_string());
                let message = channel_runtime_string_with_args(
                    "channel-runtime-provider-unavailable",
                    &[
                        ("provider", route.provider.as_str()),
                        ("details", safe_err.as_str()),
                    ],
                );
                if let Some(channel) = target_channel.as_ref() {
                    let _ = channel
                        .send(
                            &SendMessage::new(message, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
                return;
            }
        };
    if ctx.auto_save_memory
        && msg.content.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS
        && !operant_memory::should_skip_autosave_content(&msg.content)
    {
        let autosave_key = conversation_memory_key(&msg);
        let _ = ctx
            .memory
            .store(
                &autosave_key,
                &msg.content,
                operant_memory::MemoryCategory::Conversation,
                Some(&history_key),
            )
            .await;
    }

    println!("  ⏳ Processing message...");
    let started_at = Instant::now();

    let force_fresh_session = take_pending_new_session(ctx.as_ref(), &history_key);
    if force_fresh_session {
        // `/new` should make the next user turn completely fresh even if
        // older cached turns reappear before this message starts.
        clear_sender_history(ctx.as_ref(), &history_key);
    }

    let had_prior_history = if force_fresh_session {
        false
    } else {
        ctx.conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .peek(&history_key)
            .is_some_and(|turns| !turns.is_empty())
    };

    // Preserve user turn before the LLM call so interrupted requests keep context.
    append_sender_turn(ctx.as_ref(), &history_key, ChatMessage::user(&msg.content));

    // Build history from per-sender conversation cache.
    let prior_turns_raw = if force_fresh_session {
        vec![ChatMessage::user(&msg.content)]
    } else {
        ctx.conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&history_key)
            .cloned()
            .unwrap_or_default()
    };
    let mut prior_turns = normalize_cached_channel_turns(prior_turns_raw);

    // Strip stale tool_result blocks from cached turns so the LLM never
    // sees a `<tool_result>` without a preceding `<tool_call>`, which
    // causes hallucinated output on subsequent heartbeat ticks or sessions.
    for turn in &mut prior_turns {
        if turn.content.contains("<tool_result") {
            turn.content = strip_tool_result_content(&turn.content);
        }
    }

    // Strip [Used tools: ...] prefixes from cached assistant turns so the
    // LLM never sees (and reproduces) this internal summary format (#4400).
    for turn in &mut prior_turns {
        if turn.role == "assistant" && turn.content.starts_with("[Used tools:") {
            turn.content = strip_tool_summary_prefix(&turn.content);
        }
    }

    // Strip [IMAGE:] markers from *older* history messages when the active
    // provider does not support vision. This prevents "history poisoning"
    // where a previously-sent image marker gets reloaded from the JSONL
    // session file and permanently breaks the conversation (fixes #3674).
    // We skip the last turn (the current message) so the vision check can
    // still reject fresh image sends with a proper error.
    if !active_provider.supports_vision() && prior_turns.len() > 1 {
        let last_idx = prior_turns.len() - 1;
        for turn in &mut prior_turns[..last_idx] {
            if turn.content.contains("[IMAGE:") {
                let (cleaned, _refs) =
                    operant_providers::multimodal::parse_image_markers(&turn.content);
                turn.content = cleaned;
            }
        }
        // Drop older turns that became empty after marker removal (e.g. image-only messages).
        // Keep the last turn (current message) intact.
        let current = prior_turns.pop();
        prior_turns.retain(|turn| !turn.content.trim().is_empty());
        if let Some(current) = current {
            prior_turns.push(current);
        }
    }

    // Proactively trim conversation history before sending to the provider
    // to prevent context-window-exceeded errors (bug #3460).
    let dropped = proactive_trim_turns(&mut prior_turns, PROACTIVE_CONTEXT_BUDGET_CHARS);
    if dropped > 0 {
        tracing::info!(
            channel = %msg.channel,
            sender = %msg.sender,
            dropped_turns = dropped,
            remaining_turns = prior_turns.len(),
            "Proactively trimmed conversation history to fit context budget"
        );
    }

    // ── Dual-scope memory recall ──────────────────────────────────
    // Always recall before each LLM call (not just first turn).
    // For group chats: merge sender-scope + group-scope memories.
    // For DMs: recall from the current conversation scope plus sender scope.
    let is_group_chat = is_group_reply_target(&msg.reply_target);

    let mem_recall_start = Instant::now();
    let sender_session_ids = sender_memory_session_ids(&msg, &history_key);
    let sender_session_id_refs: Vec<Option<&str>> = sender_session_ids
        .iter()
        .map(|s| Some(s.as_str()))
        .collect();
    let sender_memory_fut = build_memory_context_for_sessions(
        ctx.memory.as_ref(),
        &msg.content,
        ctx.min_relevance_score,
        sender_session_id_refs.as_slice(),
    );

    let (sender_memory, group_memory) = if is_group_chat {
        let group_memory_fut = build_memory_context(
            ctx.memory.as_ref(),
            &msg.content,
            ctx.min_relevance_score,
            Some(&history_key),
        );
        tokio::join!(sender_memory_fut, group_memory_fut)
    } else {
        (sender_memory_fut.await, String::new())
    };
    #[allow(clippy::cast_possible_truncation)]
    let mem_recall_ms = mem_recall_start.elapsed().as_millis() as u64;
    tracing::info!(
        mem_recall_ms,
        sender_empty = sender_memory.is_empty(),
        group_empty = group_memory.is_empty(),
        "⏱ Memory recall completed"
    );

    // Merge sender and group memory context blocks.
    let memory_context = if group_memory.is_empty() {
        sender_memory
    } else if sender_memory.is_empty() {
        group_memory
    } else {
        format!("{sender_memory}\n{group_memory}")
    };

    // Use refreshed system prompt for new sessions (master's /new support),
    // and inject memory into system prompt (not user message) so it
    // doesn't pollute session history and is re-fetched each turn.
    let base_system_prompt = if had_prior_history {
        ctx.system_prompt.as_str().to_string()
    } else {
        refreshed_new_session_system_prompt(ctx.as_ref())
    };
    let mut system_prompt = build_channel_system_prompt(
        &base_system_prompt,
        &msg.channel,
        &msg.reply_target,
        &msg.sender,
    );
    if !memory_context.is_empty() {
        let _ = write!(system_prompt, "\n\n{memory_context}");
    }
    let mut history = vec![ChatMessage::system(system_prompt)];
    history.extend(prior_turns);

    // ── Proactive context compression ────────────────────────────
    // Use the existing ContextCompressor to summarize older history
    // before the LLM call, preventing context-window-exceeded errors
    // and preserving key decisions through LLM-driven summarization.
    {
        let cc_config = ctx.prompt_config.agent.context_compression.clone();
        let compressor = operant_runtime::agent::context_compressor::ContextCompressor::new(
            cc_config,
            ctx.context_token_budget,
        )
        .with_memory(Arc::clone(&ctx.memory));
        match compressor
            .compress_if_needed(&mut history, active_provider.as_ref(), route.model.as_str())
            .await
        {
            Ok(result) if result.compressed => {
                tracing::info!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    tokens_before = result.tokens_before,
                    tokens_after = result.tokens_after,
                    passes = result.passes_used,
                    "Proactive context compression applied before LLM call"
                );
            }
            Err(e) => {
                tracing::warn!("Context compression failed, proceeding without: {e}");
            }
            _ => {}
        }
    }

    // ── Reply-intent precheck ────────────────────────────────────────
    let precheck_cfg = ctx.prompt_config.agent.precheck.clone();
    let reply_intent = if precheck_cfg.enabled {
        let precheck_model = precheck_cfg
            .model
            .as_deref()
            .unwrap_or(route.model.as_str());
        let precheck_start = Instant::now();
        let precheck_timeout = Duration::from_secs(precheck_cfg.timeout_secs);
        match tokio::time::timeout(
            precheck_timeout,
            classify_channel_reply_intent(
                active_provider.as_ref(),
                history[0].content.as_str(),
                &history,
                precheck_model,
                runtime_defaults.temperature,
            ),
        )
        .await
        {
            Ok(Ok(outcome)) => {
                #[allow(clippy::cast_possible_truncation)]
                let elapsed_ms = precheck_start.elapsed().as_millis() as u64;
                tracing::info!(
                    elapsed_ms,
                    model = precheck_model,
                    "⏱ Reply-intent precheck completed"
                );
                outcome
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    error = %e,
                    model = precheck_model,
                    "Reply-intent precheck failed, defaulting to REPLY"
                );
                AssistantChannelOutcome::Reply(String::new())
            }
            Err(_) => {
                #[allow(clippy::cast_possible_truncation)]
                let elapsed_ms = precheck_start.elapsed().as_millis() as u64;
                tracing::warn!(
                    elapsed_ms,
                    timeout_secs = precheck_cfg.timeout_secs,
                    model = precheck_model,
                    "Reply-intent precheck timed out, defaulting to REPLY"
                );
                AssistantChannelOutcome::Reply(String::new())
            }
        }
    } else {
        AssistantChannelOutcome::Reply(String::new())
    };

    if let AssistantChannelOutcome::NoReply { kind, reason } = reply_intent {
        let history_response = AssistantChannelOutcome::NoReply {
            kind,
            reason: reason.clone(),
        }
        .history_marker();
        append_sender_turn(
            ctx.as_ref(),
            &history_key,
            ChatMessage::assistant(&history_response),
        );
        // Surface the no-reply decision in chat with an emoji on the user's
        // message so the chatter isn't left wondering whether the bot saw
        // the message. Same `ack_reactions` gate as the 👀 → ✅/⚠️ ack/done
        // pattern so operators with reactions disabled don't suddenly see
        // them. Best-effort: log on failure, never propagate. Channels that
        // don't implement add_reaction get the trait's no-op default.
        if ctx.ack_reactions
            && let Some(channel) = target_channel.as_ref()
        {
            let emoji = kind.emoji();
            if let Err(e) = channel
                .add_reaction(&msg.reply_target, &msg.id, emoji)
                .await
            {
                tracing::debug!(
                    "Failed to add {emoji} no-reply reaction on {}: {e}",
                    channel.name()
                );
            }
        }
        runtime_trace::record_event(
            "channel_message_no_reply",
            Some(msg.channel.as_str()),
            Some(route.provider.as_str()),
            Some(route.model.as_str()),
            None,
            Some(true),
            reason.as_deref(),
            serde_json::json!({
                "sender": msg.sender,
                "elapsed_ms": started_at.elapsed().as_millis(),
                "phase": "precheck",
                "kind": format!("{kind:?}"),
            }),
        );
        println!(
            "  🤖 No reply [{kind:?}] ({}ms): {}",
            started_at.elapsed().as_millis(),
            reason.as_deref().unwrap_or("no reason provided")
        );
        return;
    }

    let use_draft_streaming = target_channel
        .as_ref()
        .is_some_and(|ch| ch.supports_draft_updates());

    tracing::debug!(
        channel = %msg.channel,
        has_target_channel = target_channel.is_some(),
        use_draft_streaming,
        "Streaming decision"
    );

    // Partial mode: delta channel for draft updates (progress + text).
    let (delta_tx, delta_rx) = if use_draft_streaming {
        let (tx, rx) = tokio::sync::mpsc::channel::<operant_runtime::agent::loop_::DraftEvent>(64);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    // Partial mode: send an initial draft message for progressive editing.
    let draft_message_id = if use_draft_streaming {
        if let Some(channel) = target_channel.as_ref() {
            match channel
                .send_draft(
                    &SendMessage::new("...", &msg.reply_target).in_thread(msg.thread_ts.clone()),
                )
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::debug!("Failed to send draft on {}: {e}", channel.name());
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Spawn the appropriate handler for the delta channel.
    let draft_updater = if use_draft_streaming {
        // Partial: accumulate text and edit a single draft message.
        if let (Some(mut rx), Some(draft_id_ref), Some(channel_ref)) = (
            delta_rx,
            draft_message_id.as_deref(),
            target_channel.as_ref(),
        ) {
            let channel = Arc::clone(channel_ref);
            let reply_target = msg.reply_target.clone();
            let draft_id = draft_id_ref.to_string();
            Some(tokio::spawn(async move {
                use operant_runtime::agent::loop_::StreamDelta;
                let mut accumulated = String::new();
                while let Some(event) = rx.recv().await {
                    match event {
                        StreamDelta::Status(text) => {
                            let visible = strip_think_tags_inline(&text);
                            if let Err(e) = channel
                                .update_draft_progress(&reply_target, &draft_id, &visible)
                                .await
                            {
                                tracing::debug!("Draft progress update failed: {e}");
                            }
                        }
                        StreamDelta::Text(text) => {
                            accumulated.push_str(&text);
                            let visible = strip_think_tags_inline(&accumulated);
                            if let Err(e) = channel
                                .update_draft(&reply_target, &draft_id, &visible)
                                .await
                            {
                                tracing::debug!("Draft update failed: {e}");
                            }
                        }
                    }
                }
            }))
        } else {
            None
        }
    } else {
        None
    };

    // React with 👀 to acknowledge the incoming message
    if ctx.ack_reactions
        && let Some(channel) = target_channel.as_ref()
        && let Err(e) = channel
            .add_reaction(&msg.reply_target, &msg.id, "\u{1F440}")
            .await
    {
        tracing::debug!("Failed to add reaction: {e}");
    }

    // Skip typing only for Partial mode — the draft message itself provides
    // visual feedback. MultiMessage and Off both keep typing active.
    let is_partial_draft = target_channel
        .as_ref()
        .is_some_and(|ch| ch.supports_draft_updates() && !ch.supports_multi_message_streaming());
    let typing_cancellation = if is_partial_draft {
        None
    } else {
        target_channel.as_ref().map(|_| CancellationToken::new())
    };
    let typing_task = match (target_channel.as_ref(), typing_cancellation.as_ref()) {
        (Some(channel), Some(token)) => Some(spawn_scoped_typing_task(
            Arc::clone(channel),
            msg.reply_target.clone(),
            token.clone(),
        )),
        _ => None,
    };

    // Wrap observer to forward tool events as live thread messages
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let notify_observer: Arc<ChannelNotifyObserver> = Arc::new(ChannelNotifyObserver {
        inner: Arc::clone(&ctx.observer),
        tx: notify_tx,
        tools_used: AtomicBool::new(false),
    });
    let notify_observer_flag = Arc::clone(&notify_observer);
    let notify_channel = target_channel.clone();
    let notify_reply_target = msg.reply_target.clone();
    let notify_thread_root = followup_thread_id(&msg);
    let notify_task = if msg.channel == "cli" || !ctx.show_tool_calls {
        Some(tokio::spawn(async move {
            while notify_rx.recv().await.is_some() {}
        }))
    } else {
        Some(tokio::spawn(async move {
            let thread_ts = notify_thread_root;
            while let Some(text) = notify_rx.recv().await {
                if let Some(ref ch) = notify_channel {
                    let _ = ch
                        .send(
                            &SendMessage::new(&text, &notify_reply_target)
                                .in_thread(thread_ts.clone()),
                        )
                        .await;
                }
            }
        }))
    };

    enum LlmExecutionResult {
        Completed(Result<Result<String, anyhow::Error>, tokio::time::error::Elapsed>),
        Cancelled,
    }

    let model_switch_callback = get_model_switch_state();
    let scale_cap = ctx
        .pacing
        .message_timeout_scale_max
        .unwrap_or(CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP);
    let timeout_budget_secs = channel_message_timeout_budget_secs_with_cap(
        ctx.message_timeout_secs,
        ctx.max_tool_iterations,
        scale_cap,
    );
    let cost_tracking_context = ctx.cost_tracking.clone().map(|state| {
        operant_runtime::agent::loop_::ToolLoopCostTrackingContext::new(state.tracker, state.prices)
    });
    let llm_call_start = Instant::now();
    #[allow(clippy::cast_possible_truncation)]
    let elapsed_before_llm_ms = started_at.elapsed().as_millis() as u64;
    tracing::info!(elapsed_before_llm_ms, "⏱ Starting LLM call");
    // Per-turn collector. `tool_execution::execute_one_tool` pushes
    // `<tool_name>: <receipt>` here whenever a receipt is generated, so the
    // orchestrator can render the trailing `Tool receipts:` block after the
    // loop returns. Wrapped in `Arc` so the same handle can be shared into
    // `TOOL_LOOP_RECEIPT_CONTEXT` for subagent forwarding (#6182). Inert when
    // `receipt_generator` is `None`.
    let tool_receipts_collector: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let receipt_scope = ctx.receipt_generator.as_ref().map(|generator| {
        operant_runtime::agent::tool_receipts::ReceiptScope {
            generator: generator.clone(),
            collector: std::sync::Arc::clone(&tool_receipts_collector),
        }
    });
    let (llm_result, fallback_info) = scope_provider_fallback(async {
        let llm_result = loop {
            let loop_result = tokio::select! {
                () = cancellation_token.cancelled() => LlmExecutionResult::Cancelled,
                result = tokio::time::timeout(
                    Duration::from_secs(timeout_budget_secs),
                    scope_thread_id(
                        msg.interruption_scope_id.clone()
                            .or_else(|| msg.thread_ts.clone())
                            .or_else(|| Some(msg.id.clone())),
                    scope_session_key(
                        Some(history_key.clone()),
                        operant_runtime::agent::loop_::TOOL_LOOP_COST_TRACKING_CONTEXT.scope(
                            cost_tracking_context.clone(),
                        operant_runtime::agent::tool_receipts::TOOL_LOOP_RECEIPT_CONTEXT.scope(
                            receipt_scope.clone(),
                        run_tool_call_loop(
                        active_provider.as_ref(),
                        &mut history,
                        ctx.tools_registry.as_ref(),
                        notify_observer.as_ref() as &dyn Observer,
                        route.provider.as_str(),
                        route.model.as_str(),
                        runtime_defaults.temperature,
                        true,
                        Some(&*ctx.approval_manager),
                        msg.channel.as_str(),
                        Some(msg.reply_target.as_str()),
                        &ctx.multimodal,
                        ctx.max_tool_iterations,
                        Some(cancellation_token.clone()),
                        delta_tx.clone(),
                        ctx.hooks.as_deref(),
                        if msg.channel == "cli"
                            || ctx.autonomy_level == AutonomyLevel::Full
                        {
                            &[]
                        } else {
                            ctx.non_cli_excluded_tools.as_ref()
                        },
                        ctx.tool_call_dedup_exempt.as_ref(),
                        ctx.activated_tools.as_ref(),
                        Some(model_switch_callback.clone()),
                        &ctx.pacing,
                        ctx.max_tool_result_chars,
                        ctx.context_token_budget,
                        None, // shared_budget
                        target_channel.as_deref(),
                        ctx.receipt_generator.as_ref(),
                        // Collector is meaningful only when the generator is
                        // active. Pass None when receipts are disabled so the
                        // call site reflects that coupling explicitly.
                        ctx.receipt_generator
                            .as_ref()
                            .map(|_| tool_receipts_collector.as_ref()),
                    ),
                    ),
                    ),
                    ),
                    ),
                ) => LlmExecutionResult::Completed(result),
            };

            // Handle model switch: re-create the provider and retry
            if let LlmExecutionResult::Completed(Ok(Err(ref e))) = loop_result
                && let Some((new_provider, new_model)) = is_model_switch_requested(e)
            {
                tracing::info!(
                    "Model switch requested, switching from {} {} to {} {}",
                    route.provider,
                    route.model,
                    new_provider,
                    new_model
                );

                match create_resilient_provider_nonblocking(
                    &new_provider,
                    ctx.api_key.clone(),
                    ctx.api_url.clone(),
                    ctx.reliability.as_ref().clone(),
                    ctx.provider_runtime_options.clone(),
                )
                .await
                {
                    Ok(new_prov) => {
                        active_provider = Arc::from(new_prov);
                        route.provider = new_provider;
                        route.model = new_model;
                        clear_model_switch_request();

                        ctx.observer.record_event(&ObserverEvent::AgentStart {
                            provider: route.provider.clone(),
                            model: route.model.clone(),
                        });

                        continue;
                    }
                    Err(err) => {
                        tracing::error!("Failed to create provider after model switch: {err}");
                        clear_model_switch_request();
                        // Fall through with the original error
                    }
                }
            }

            break loop_result;
        };
        let fb = take_last_provider_fallback();
        (llm_result, fb)
    })
    .await;

    // Drop all senders so updater tasks can exit (rx.recv() returns None).
    tracing::debug!("Post-loop: dropping delta_tx and awaiting draft updater");
    drop(delta_tx);
    if let Some(handle) = draft_updater {
        let _ = handle.await;
    }
    tracing::debug!("Post-loop: draft updater completed");

    // Thread the final reply only if tools were used (multi-message response)
    if notify_observer_flag.tools_used.load(Ordering::Relaxed) && msg.channel != "cli" {
        msg.thread_ts = followup_thread_id(&msg);
    }
    // Drop the notify sender so the forwarder task finishes
    drop(notify_observer);
    drop(notify_observer_flag);
    if let Some(handle) = notify_task {
        let _ = handle.await;
    }

    #[allow(clippy::cast_possible_truncation)]
    let llm_call_ms = llm_call_start.elapsed().as_millis() as u64;
    #[allow(clippy::cast_possible_truncation)]
    let total_ms = started_at.elapsed().as_millis() as u64;
    tracing::info!(llm_call_ms, total_ms, "⏱ LLM call completed");

    if let Some(token) = typing_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = typing_task {
        log_worker_join_result(handle.await);
    }

    let reaction_done_emoji = match &llm_result {
        LlmExecutionResult::Completed(Ok(Ok(_))) => "\u{2705}", // ✅
        _ => "\u{26A0}\u{FE0F}",                                // ⚠️
    };

    match llm_result {
        LlmExecutionResult::Cancelled => {
            tracing::info!(
                channel = %msg.channel,
                sender = %msg.sender,
                "Cancelled in-flight channel request due to newer message"
            );
            runtime_trace::record_event(
                "channel_message_cancelled",
                Some(msg.channel.as_str()),
                Some(route.provider.as_str()),
                Some(route.model.as_str()),
                None,
                Some(false),
                Some("cancelled due to newer inbound message"),
                serde_json::json!({
                    "sender": msg.sender,
                    "elapsed_ms": started_at.elapsed().as_millis(),
                }),
            );
            if let (Some(channel), Some(draft_id)) =
                (target_channel.as_ref(), draft_message_id.as_deref())
                && let Err(err) = channel.cancel_draft(&msg.reply_target, draft_id).await
            {
                tracing::debug!("Failed to cancel draft on {}: {err}", channel.name());
            }
        }
        LlmExecutionResult::Completed(Ok(Ok(response))) => {
            // ── Hook: on_message_sending (modifying) ─────────
            let mut outbound_response = response;
            if let Some(hooks) = &ctx.hooks {
                match hooks
                    .run_on_message_sending(
                        msg.channel.clone(),
                        msg.reply_target.clone(),
                        outbound_response.clone(),
                    )
                    .await
                {
                    operant_runtime::hooks::HookResult::Cancel(reason) => {
                        tracing::info!(%reason, "outgoing message suppressed by hook");
                        if let (Some(channel), Some(draft_id)) =
                            (target_channel.as_ref(), draft_message_id.as_deref())
                        {
                            let _ = channel.cancel_draft(&msg.reply_target, draft_id).await;
                        }
                        return;
                    }
                    operant_runtime::hooks::HookResult::Continue((
                        hook_channel,
                        hook_recipient,
                        mut modified_content,
                    )) => {
                        if hook_channel != msg.channel || hook_recipient != msg.reply_target {
                            tracing::warn!(
                                from_channel = %msg.channel,
                                from_recipient = %msg.reply_target,
                                to_channel = %hook_channel,
                                to_recipient = %hook_recipient,
                                "on_message_sending attempted to rewrite channel routing; only content mutation is applied"
                            );
                        }

                        let modified_len = modified_content.chars().count();
                        if modified_len > CHANNEL_HOOK_MAX_OUTBOUND_CHARS {
                            tracing::warn!(
                                limit = CHANNEL_HOOK_MAX_OUTBOUND_CHARS,
                                attempted = modified_len,
                                "hook-modified outbound content exceeded limit; truncating"
                            );
                            modified_content = truncate_with_ellipsis(
                                &modified_content,
                                CHANNEL_HOOK_MAX_OUTBOUND_CHARS,
                            );
                        }

                        if modified_content != outbound_response {
                            tracing::info!(
                                channel = %msg.channel,
                                sender = %msg.sender,
                                before_len = outbound_response.chars().count(),
                                after_len = modified_content.chars().count(),
                                "outgoing message content modified by hook"
                            );
                        }

                        outbound_response = modified_content;
                    }
                }
            }

            let sanitized_response =
                sanitize_channel_response(&outbound_response, ctx.tools_registry.as_ref());
            let mut delivered_response =
                if sanitized_response.is_empty() && !outbound_response.trim().is_empty() {
                    channel_runtime_string("channel-runtime-malformed-tool-output")
                } else {
                    sanitized_response
                };

            // Append a footer when the response was served by a different provider family.
            // Intra-family fallbacks (e.g. minimax → minimax-cn) are suppressed.
            if let Some(fb) = fallback_info.as_ref() {
                let req_base = fb.requested_provider.split(':').next().unwrap_or("");
                let act_base = fb.actual_provider.split(':').next().unwrap_or("");
                let same_family = req_base == act_base
                    || req_base.starts_with(act_base)
                    || act_base.starts_with(req_base);
                if !same_family {
                    delivered_response.push_str("\n\n");
                    delivered_response.push_str(&channel_runtime_string_with_args(
                        "channel-runtime-fallback-footer",
                        &[
                            ("requested_provider", fb.requested_provider.as_str()),
                            ("actual_provider", fb.actual_provider.as_str()),
                            ("actual_model", fb.actual_model.as_str()),
                        ],
                    ));
                }
            }

            runtime_trace::record_event(
                "channel_message_outbound",
                Some(msg.channel.as_str()),
                Some(route.provider.as_str()),
                Some(route.model.as_str()),
                None,
                Some(true),
                None,
                serde_json::json!({
                    "sender": msg.sender,
                    "elapsed_ms": started_at.elapsed().as_millis(),
                    "response": scrub_credentials(&delivered_response),
                }),
            );

            // Persist intermediate tool-call/result messages from this turn
            // so the model retains concrete "I used tools" examples in
            // context, preventing drift toward tool-less responses (#4827).
            let keep_tool_turns = ctx.prompt_config.agent.keep_tool_context_turns;
            if keep_tool_turns > 0 {
                // Find tool messages for the current turn: everything after
                // the last user message up to (but not including) the final
                // assistant response that matches our delivered text.
                let tool_messages: Vec<ChatMessage> = extract_current_turn_tool_messages(&history);
                for tool_msg in tool_messages {
                    append_sender_turn(ctx.as_ref(), &history_key, tool_msg);
                }
            }

            let history_response = delivered_response.clone();
            append_sender_turn(
                ctx.as_ref(),
                &history_key,
                ChatMessage::assistant(&history_response),
            );

            // Strip tool-call messages from turns older than
            // keep_tool_context_turns to prevent unbounded growth.
            if keep_tool_turns > 0 {
                strip_old_tool_context(ctx.as_ref(), &history_key, keep_tool_turns);
            }

            // Fire-and-forget LLM-driven memory consolidation.
            if ctx.auto_save_memory && msg.content.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS {
                let provider = Arc::clone(&ctx.provider);
                let model = ctx.model.to_string();
                let memory = Arc::clone(&ctx.memory);
                let user_msg = msg.content.clone();
                let assistant_resp = delivered_response.clone();
                tokio::spawn(async move {
                    if let Err(e) = operant_memory::consolidation::consolidate_turn(
                        provider.as_ref(),
                        &model,
                        memory.as_ref(),
                        &user_msg,
                        &assistant_resp,
                    )
                    .await
                    {
                        tracing::debug!("Memory consolidation skipped: {e}");
                    }
                });
            }

            println!(
                "  🤖 Reply ({}ms): {}",
                started_at.elapsed().as_millis(),
                truncate_with_ellipsis(&delivered_response, 80)
            );
            // Build the trailing `Tool receipts:` block from the per-turn
            // collector. Empty when receipts are disabled or no tool ran.
            // Includes receipts from delegate sub-agents because the same
            // `Arc<Mutex<Vec<String>>>` is forwarded via
            // `TOOL_LOOP_RECEIPT_CONTEXT` into sub-loops (see #6182).
            let receipts_block = if ctx.show_receipts_in_response {
                let receipts = tool_receipts_collector
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if receipts.is_empty() {
                    None
                } else {
                    let mut block = channel_runtime_string("channel-runtime-tool-receipts-header");
                    for r in receipts.iter() {
                        write!(block, "\n  {r}").ok();
                    }
                    Some(block)
                }
            } else {
                None
            };

            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    if let Err(e) = channel
                        .finalize_draft(&msg.reply_target, draft_id, &delivered_response)
                        .await
                    {
                        tracing::warn!("Failed to finalize draft: {e}; sending as new message");
                        let _ = channel
                            .send(
                                &SendMessage::new(&delivered_response, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                } else if let Err(e) = channel
                    .send(
                        &SendMessage::new(&delivered_response, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone())
                            .with_cancellation(cancellation_token.clone()),
                    )
                    .await
                {
                    eprintln!("  ❌ Failed to reply on {}: {e}", channel.name());
                }
                // Send tool receipts as a separate message in the same thread.
                // The block is the operator-facing audit surface for the feature,
                // so a dropped send must leave a log signal rather than silently
                // disappear.
                if let Some(ref block) = receipts_block
                    && let Err(e) = channel
                        .send(
                            &SendMessage::new(block, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await
                {
                    tracing::warn!(
                        channel = channel.name(),
                        error = %e,
                        "failed to send tool receipts block"
                    );
                }
            }
        }
        LlmExecutionResult::Completed(Ok(Err(e))) => {
            if operant_runtime::agent::loop_::is_tool_loop_cancelled(&e)
                || cancellation_token.is_cancelled()
            {
                tracing::info!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    "Cancelled in-flight channel request due to newer message"
                );
                runtime_trace::record_event(
                    "channel_message_cancelled",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("cancelled during tool-call loop"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                    }),
                );
                if let (Some(channel), Some(draft_id)) =
                    (target_channel.as_ref(), draft_message_id.as_deref())
                    && let Err(err) = channel.cancel_draft(&msg.reply_target, draft_id).await
                {
                    tracing::debug!("Failed to cancel draft on {}: {err}", channel.name());
                }
            } else if is_context_window_overflow_error(&e) {
                let compacted = compact_sender_history(ctx.as_ref(), &history_key);
                let error_text = if compacted {
                    channel_runtime_string("channel-runtime-context-window-exceeded-compacted")
                } else {
                    channel_runtime_string("channel-runtime-context-window-exceeded")
                };
                eprintln!(
                    "  ⚠️ Context window exceeded after {}ms; sender history compacted={}",
                    started_at.elapsed().as_millis(),
                    compacted
                );
                runtime_trace::record_event(
                    "channel_message_error",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("context window exceeded"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                        "history_compacted": compacted,
                    }),
                );
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel
                            .finalize_draft(&msg.reply_target, draft_id, &error_text)
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(error_text, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }
            } else {
                eprintln!(
                    "  ❌ LLM error after {}ms: {e}",
                    started_at.elapsed().as_millis()
                );

                // Evict cached provider on auth errors so the next request
                // re-creates it with fresh OAuth credentials (#5219).
                if operant_providers::reliable::is_auth_error(&e) {
                    let cache_key = provider_cache_key(&route.provider, route.api_key.as_deref());
                    let mut cache = ctx.provider_cache.lock().unwrap_or_else(|p| p.into_inner());
                    if cache.remove(&cache_key).is_some() {
                        tracing::info!(
                            provider = %route.provider,
                            "Evicted cached provider after auth error; next request will re-create with fresh credentials"
                        );
                    }
                }
                let safe_error = operant_providers::sanitize_api_error(&e.to_string());
                runtime_trace::record_event(
                    "channel_message_error",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some(&safe_error),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                    }),
                );
                let should_rollback_user_turn = should_rollback_failed_user_turn(&e);
                let rolled_back = should_rollback_user_turn
                    && rollback_orphan_user_turn(ctx.as_ref(), &history_key, &msg.content);

                if !rolled_back {
                    // Close the orphan user turn so subsequent messages don't
                    // inherit this failed request as unfinished context.
                    append_sender_turn(
                        ctx.as_ref(),
                        &history_key,
                        ChatMessage::assistant("[Task failed — not continuing this request]"),
                    );
                }
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel
                            .finalize_draft(&msg.reply_target, draft_id, &format!("⚠️ Error: {e}"))
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(format!("⚠️ Error: {e}"), &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }
            }
        }
        LlmExecutionResult::Completed(Err(_)) => {
            let timeout_msg = format!(
                "LLM response timed out after {}s (base={}s, max_tool_iterations={})",
                timeout_budget_secs, ctx.message_timeout_secs, ctx.max_tool_iterations
            );
            runtime_trace::record_event(
                "channel_message_timeout",
                Some(msg.channel.as_str()),
                Some(route.provider.as_str()),
                Some(route.model.as_str()),
                None,
                Some(false),
                Some(&timeout_msg),
                serde_json::json!({
                    "sender": msg.sender,
                    "elapsed_ms": started_at.elapsed().as_millis(),
                }),
            );
            eprintln!(
                "  ❌ {} (elapsed: {}ms)",
                timeout_msg,
                started_at.elapsed().as_millis()
            );
            // Close the orphan user turn so subsequent messages don't
            // inherit this timed-out request as unfinished context.
            append_sender_turn(
                ctx.as_ref(),
                &history_key,
                ChatMessage::assistant("[Task timed out — not continuing this request]"),
            );
            if let Some(channel) = target_channel.as_ref() {
                let error_text = channel_runtime_string("channel-runtime-request-timed-out");
                if let Some(ref draft_id) = draft_message_id {
                    let _ = channel
                        .finalize_draft(&msg.reply_target, draft_id, &error_text)
                        .await;
                } else {
                    let _ = channel
                        .send(
                            &SendMessage::new(error_text, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
        }
    }

    // Swap 👀 → ✅ (or ⚠️ on error) to signal processing is complete
    if ctx.ack_reactions
        && let Some(channel) = target_channel.as_ref()
    {
        let _ = channel
            .remove_reaction(&msg.reply_target, &msg.id, "\u{1F440}")
            .await;
        let _ = channel
            .add_reaction(&msg.reply_target, &msg.id, reaction_done_emoji)
            .await;
    }
}

/// Shared worker body extracted so both the normal path and the debounce path
/// can reuse the same in-flight tracking / cancellation / process logic.
pub(crate) async fn dispatch_worker(
    ctx: Arc<ChannelRuntimeContext>,
    msg: operant_api::channel::ChannelMessage,
    in_flight: Arc<tokio::sync::Mutex<HashMap<String, InFlightSenderTaskState>>>,
    task_sequence: Arc<AtomicU64>,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let _permit = permit;
    let interrupt_enabled = ctx
        .interrupt_on_new_message
        .enabled_for_channel(msg.channel.as_str());
    let sender_scope_key = interruption_scope_key(&msg);
    let cancellation_token = CancellationToken::new();
    let completion = Arc::new(InFlightTaskCompletion::new());
    let task_id = task_sequence.fetch_add(1, Ordering::Relaxed);

    let register_in_flight = msg.channel != "cli";

    if register_in_flight {
        let previous = {
            let mut active = in_flight.lock().await;
            active.insert(
                sender_scope_key.clone(),
                InFlightSenderTaskState {
                    task_id,
                    cancellation: cancellation_token.clone(),
                    completion: Arc::clone(&completion),
                },
            )
        };

        if interrupt_enabled && let Some(previous) = previous {
            tracing::info!(
                channel = %msg.channel,
                sender = %msg.sender,
                "Interrupting previous in-flight request for sender"
            );
            previous.cancellation.cancel();
            previous.completion.wait().await;
        }
    }

    process_channel_message(ctx, msg, cancellation_token).await;

    if register_in_flight {
        let mut active = in_flight.lock().await;
        if active
            .get(&sender_scope_key)
            .is_some_and(|state| state.task_id == task_id)
        {
            active.remove(&sender_scope_key);
        }
    }

    completion.mark_done();
}

pub(crate) async fn run_message_dispatch_loop(
    mut rx: tokio::sync::mpsc::Receiver<operant_api::channel::ChannelMessage>,
    ctx: Arc<ChannelRuntimeContext>,
    max_in_flight_messages: usize,
) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_in_flight_messages));
    let mut workers = tokio::task::JoinSet::new();
    let in_flight_by_sender = Arc::new(tokio::sync::Mutex::new(HashMap::<
        String,
        InFlightSenderTaskState,
    >::new()));
    let task_sequence = Arc::new(AtomicU64::new(1));

    while let Some(msg) = rx.recv().await {
        // Fast path: /stop cancels the in-flight task for this sender scope without
        // spawning a worker or registering a new task. Handled here — before semaphore
        // acquisition — so the target task is still in the store and is never replaced.
        if msg.channel != "cli" && is_stop_command(&msg.content) {
            let scope_key = interruption_scope_key(&msg);
            let previous = {
                let mut active = in_flight_by_sender.lock().await;
                active.remove(&scope_key)
            };
            let reply = if let Some(state) = previous {
                state.cancellation.cancel();
                channel_runtime_string("channel-runtime-stop-sent")
            } else {
                channel_runtime_string("channel-runtime-stop-none")
            };
            let channel = ctx
                .channels_by_name
                .get(&msg.channel)
                .or_else(|| {
                    // Multi-room channels use "name:qualifier" format (e.g. "matrix:!roomId");
                    // fall back to base channel name for routing.
                    msg.channel
                        .split_once(':')
                        .and_then(|(base, _)| ctx.channels_by_name.get(base))
                })
                .cloned();
            if let Some(channel) = channel {
                let reply_target = msg.reply_target.clone();
                let thread_ts = msg.thread_ts.clone();
                tokio::spawn(async move {
                    let _ = channel
                        .send(&SendMessage::new(reply, &reply_target).in_thread(thread_ts))
                        .await;
                });
            } else {
                tracing::warn!(
                    channel = %msg.channel,
                    "stop command: no registered channel found for reply"
                );
            }
            continue;
        }

        // ── Debounce: accumulate rapid messages per sender ──────────
        // CLI messages bypass debouncing so the interactive loop stays responsive.
        let msg = if msg.channel != "cli" && ctx.debouncer.enabled() {
            let debounce_key = conversation_history_key(&msg);
            match ctx.debouncer.debounce(&debounce_key, &msg.content).await {
                operant_infra::debounce::DebounceResult::Pending(rx) => {
                    // Spawn a lightweight task that waits for the debounce window
                    // to expire, then feeds the combined message through the normal
                    // worker path below.
                    let debounce_ctx = Arc::clone(&ctx);
                    let debounce_in_flight = Arc::clone(&in_flight_by_sender);
                    let debounce_semaphore = Arc::clone(&semaphore);
                    let debounce_task_seq = Arc::clone(&task_sequence);
                    let mut debounce_msg = msg;
                    workers.spawn(async move {
                        let combined = match rx.await {
                            Ok(combined) => combined,
                            Err(_) => {
                                // Receiver dropped — a newer message superseded this one.
                                return;
                            }
                        };
                        debounce_msg.content = combined;
                        tracing::info!(
                            channel = %debounce_msg.channel,
                            sender = %debounce_msg.sender,
                            "Debounced message ready — dispatching combined message"
                        );

                        let permit = match debounce_semaphore.acquire_owned().await {
                            Ok(permit) => permit,
                            Err(_) => return,
                        };

                        dispatch_worker(
                            debounce_ctx,
                            debounce_msg,
                            debounce_in_flight,
                            debounce_task_seq,
                            permit,
                        )
                        .await;
                    });
                    continue;
                }
                operant_infra::debounce::DebounceResult::Passthrough(content) => {
                    let mut m = msg;
                    m.content = content;
                    m
                }
            }
        } else {
            msg
        };

        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };

        let worker_ctx = Arc::clone(&ctx);
        let in_flight = Arc::clone(&in_flight_by_sender);
        let task_sequence = Arc::clone(&task_sequence);
        workers.spawn(async move {
            dispatch_worker(worker_ctx, msg, in_flight, task_sequence, permit).await;
        });

        while let Some(result) = workers.try_join_next() {
            log_worker_join_result(result);
        }
    }

    while let Some(result) = workers.join_next().await {
        log_worker_join_result(result);
    }
}
