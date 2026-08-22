//! `startup` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use anyhow::Result;
use operant_config::schema::Config;
use operant_memory::{self, Memory};
use operant_providers::{self, ChatMessage, Provider};
use operant_runtime::agent::loop_::build_tool_instructions_for_names;
use operant_runtime::approval::ApprovalManager;
use operant_runtime::observability::{self, Observer};
use operant_runtime::platform;
use operant_runtime::security::{AutonomyLevel, SecurityPolicy};
use operant_runtime::tools::{self, Tool};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;

#[expect(
    clippy::unwrap_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Start all configured channels and route messages to the agent
#[allow(clippy::too_many_lines)]
pub async fn start_channels(
    config: Config,
    canvas_store: Option<operant_runtime::tools::CanvasStore>,
) -> Result<()> {
    // No model resolves yet — the user has channels configured but hasn't
    // finished onboarding their provider. Returning Ok() here lets the
    // daemon supervisor mark the channels component "done" instead of
    // restart-looping on the bail in `resolved_default_model`. The user
    // completes onboarding at /onboard and reloads via /admin/reload to
    // bring channels up.
    if resolved_default_model(&config).is_err() {
        tracing::warn!(
            "Channels supervisor exiting: no model configured but \
             channels are present. Complete browser onboarding at \
             /onboard (or set [providers.models.<name>] model = \"...\" \
             and reload the daemon) before channels can route messages."
        );
        return Ok(());
    }

    let provider_name = resolved_default_provider(&config);
    let provider_runtime_options = operant_providers::provider_runtime_options_from_config(&config);
    let provider: Arc<dyn Provider> = Arc::from(
        create_resilient_provider_nonblocking(
            &provider_name,
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.clone()),
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.base_url.clone()),
            config.reliability.clone(),
            provider_runtime_options.clone(),
        )
        .await?,
    );

    // Warm up the provider connection pool (TLS handshake, DNS, HTTP/2 setup)
    // so the first real message doesn't hit a cold-start timeout.
    if let Err(e) = provider.warmup().await {
        tracing::warn!("Provider warmup failed (non-fatal): {e}");
    }

    let initial_stamp = config_file_stamp(&config.config_path).await;
    {
        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.insert(
            config.config_path.clone(),
            RuntimeConfigState {
                defaults: runtime_defaults_from_config(&config)?,
                last_applied_stamp: initial_stamp,
            },
        );
    }

    let observer: Arc<dyn Observer> =
        Arc::from(observability::create_observer(&config.observability));
    let runtime: Arc<dyn platform::RuntimeAdapter> =
        Arc::from(platform::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let model = resolved_default_model(&config)?;
    let temperature = config
        .providers
        .fallback_provider()
        .and_then(|e| e.temperature)
        .unwrap_or(0.7);
    let mem: Arc<dyn Memory> = Arc::from(operant_memory::create_memory_with_storage_and_routes(
        &config.memory,
        &config.providers.embedding_routes,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref()),
    )?);
    let (composio_key, composio_entity_id) = if config.composio.enabled {
        (
            config.composio.api_key.as_deref(),
            Some(config.composio.entity_id.as_str()),
        )
    } else {
        (None, None)
    };
    // Build system prompt from workspace identity files + skills
    let workspace = config.workspace_dir.clone();
    let (
        mut built_tools,
        delegate_handle_ch,
        reaction_handle_ch,
        _channel_map_handle,
        ask_user_handle_ch,
        escalate_handle_ch,
    ) = tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        Arc::clone(&mem),
        composio_key,
        composio_entity_id,
        &config.browser,
        &config.http_request,
        &config.web_fetch,
        &workspace,
        &config.agents,
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref()),
        &config,
        // Share the gateway's canvas store so frames pushed from
        // channel-side agents reach the same WebSocket subscribers and
        // REST snapshots the gateway serves (#5356). When `None`, the
        // tool registry creates an orphaned store that nothing can
        // observe — the original silent-failure shape.
        canvas_store,
    );

    // Wire MCP tools into the registry before freezing — non-fatal.
    // When `deferred_loading` is enabled, MCP tools are NOT added eagerly.
    // Instead, a `tool_search` built-in is registered for on-demand loading.
    let mut deferred_section = String::new();
    let mut ch_activated_handle: Option<
        std::sync::Arc<std::sync::Mutex<operant_runtime::tools::ActivatedToolSet>>,
    > = None;
    if config.mcp.enabled && !config.mcp.servers.is_empty() {
        tracing::info!(
            "Initializing MCP client — {} server(s) configured",
            config.mcp.servers.len()
        );
        match operant_runtime::tools::McpRegistry::connect_all(&config.mcp.servers).await {
            Ok(registry) => {
                let registry = std::sync::Arc::new(registry);
                if config.mcp.deferred_loading {
                    let deferred_set = operant_runtime::tools::DeferredMcpToolSet::from_registry(
                        std::sync::Arc::clone(&registry),
                    )
                    .await;
                    tracing::info!(
                        "MCP deferred: {} tool stub(s) from {} server(s)",
                        deferred_set.len(),
                        registry.server_count()
                    );
                    deferred_section =
                        operant_runtime::tools::build_deferred_tools_section(&deferred_set);
                    let activated = std::sync::Arc::new(std::sync::Mutex::new(
                        operant_runtime::tools::ActivatedToolSet::new(),
                    ));
                    ch_activated_handle = Some(std::sync::Arc::clone(&activated));
                    built_tools.push(Box::new(operant_runtime::tools::ToolSearchTool::new(
                        deferred_set,
                        activated,
                    )));
                } else {
                    let names = registry.tool_names();
                    let mut registered = 0usize;
                    for name in names {
                        if let Some(def) = registry.get_tool_def(&name).await {
                            let wrapper: std::sync::Arc<dyn Tool> =
                                std::sync::Arc::new(operant_runtime::tools::McpToolWrapper::new(
                                    name,
                                    def,
                                    std::sync::Arc::clone(&registry),
                                ));
                            if let Some(ref handle) = delegate_handle_ch {
                                handle.write().push(std::sync::Arc::clone(&wrapper));
                            }
                            built_tools.push(Box::new(operant_runtime::tools::ArcToolRef(wrapper)));
                            registered += 1;
                        }
                    }
                    tracing::info!(
                        "MCP: {} tool(s) registered from {} server(s)",
                        registered,
                        registry.server_count()
                    );
                }
            }
            Err(e) => {
                // Non-fatal — daemon continues with the tools registered above.
                tracing::error!("MCP registry failed to initialize: {e:#}");
            }
        }
    }

    let skills = operant_runtime::skills::load_skills_with_config(&workspace, &config);

    // Register skill-defined tools so the gateway can execute them (not just
    // describe them in the prompt). Without this, skill tools like email.send
    // appear in the system prompt but return "Unknown tool" when called.
    operant_runtime::tools::register_skill_tools(&mut built_tools, &skills, security.clone());

    // Extract (name, description) specs from built tools for channel command registration.
    let tool_specs: Vec<(String, String)> = built_tools
        .iter()
        .map(|t| (t.name().to_string(), t.description().to_string()))
        .collect();

    let tools_registry = Arc::new(built_tools);

    // ── Initialize locale-aware tool descriptions ──────────────────
    let i18n_locale = config
        .locale
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(operant_runtime::i18n::detect_locale);
    operant_runtime::i18n::init(&i18n_locale);

    // Collect tool descriptions for the prompt
    let mut tool_descs: Vec<(&str, &str)> = vec![
        (
            "shell",
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.",
        ),
        (
            "file_read",
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.",
        ),
        (
            "file_write",
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.",
        ),
        (
            "memory_store",
            "Save to memory. Use when: preserving durable preferences, decisions, key context. Don't use when: information is transient/noisy/sensitive without need.",
        ),
        (
            "memory_recall",
            "Search memory. Use when: retrieving prior decisions, user preferences, historical context. Don't use when: answer is already in current context.",
        ),
        (
            "memory_forget",
            "Delete a memory entry. Use when: memory is incorrect/stale or explicitly requested for removal. Don't use when: impact is uncertain.",
        ),
    ];

    if matches!(
        config.skills.prompt_injection_mode,
        operant_config::schema::SkillsPromptInjectionMode::Compact
    ) {
        tool_descs.push((
            "read_skill",
            "Load the full source for an available skill by name. Use when: compact mode only shows a summary and you need the complete skill instructions.",
        ));
    }

    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in system browser (allowlist-only, no scraping)",
        ));
    }
    if config.composio.enabled {
        tool_descs.push((
            "composio",
            "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). Use action='list' to discover actions, 'list_accounts' to retrieve connected account IDs, 'execute' to run (optionally with connected_account_id), and 'connect' for OAuth.",
        ));
    }
    tool_descs.push((
        "schedule",
        "Manage scheduled tasks (create/list/get/cancel/pause/resume). Supports recurring cron and one-shot delays.",
    ));
    tool_descs.push((
        "pushover",
        "Send a Pushover notification to your device. Requires PUSHOVER_TOKEN and PUSHOVER_USER_KEY in .env file.",
    ));
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a subtask to a specialized agent. Use when: a task benefits from a different model (e.g. fast summarization, deep reasoning, code generation). The sub-agent runs a single prompt and returns its response.",
        ));
    }

    // Filter out tools excluded for non-CLI channels so the system prompt
    // does not advertise them for channel-driven runs.
    // Skip this filter when autonomy is `Full` — full-autonomy agents keep
    // all tools available regardless of channel.
    let excluded = &config.autonomy.non_cli_excluded_tools;
    if !excluded.is_empty() && config.autonomy.level != AutonomyLevel::Full {
        tool_descs.retain(|(name, _)| !excluded.iter().any(|ex| ex == name));
    }
    let effective_tool_names: HashSet<&str> = tools_registry
        .iter()
        .map(|tool| tool.name())
        .filter(|name| {
            config.autonomy.level == AutonomyLevel::Full
                || !excluded.iter().any(|excluded| excluded.as_str() == *name)
        })
        .collect();
    tool_descs.retain(|(name, _)| effective_tool_names.contains(name));

    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let native_tools = provider.supports_native_tools();
    let mut system_prompt = build_system_prompt_with_mode_and_autonomy(
        &workspace,
        &model,
        &tool_descs,
        &skills,
        Some(&config.identity),
        bootstrap_max_chars,
        Some(&config.autonomy),
        native_tools,
        config.skills.prompt_injection_mode,
        config.agent.compact_context,
        config.agent.max_system_prompt_chars,
    );
    if !native_tools {
        system_prompt.push_str(&build_tool_instructions_for_names(
            tools_registry.as_ref(),
            &effective_tool_names,
        ));
    }

    // Append deferred MCP tool names so the LLM knows what is available
    if !deferred_section.is_empty() {
        system_prompt.push('\n');
        system_prompt.push_str(&deferred_section);
    }

    // Append the receipt-echo instruction so the model carries
    // `[receipt: zc-receipt-...]` markers verbatim into its response.
    if config.agent.tool_receipts.enabled && config.agent.tool_receipts.inject_system_prompt {
        system_prompt.push_str(
            "\n## Tool Execution Receipts\n\n\
             Every tool result includes a `[receipt: ...]` field. This is a cryptographic \
             signature proving the tool actually executed. You must include the receipt \
             verbatim when referencing tool results. Do not modify, omit, or fabricate receipts. \
             A missing or invalid receipt indicates a fabricated tool call.\n\n",
        );
    }

    if !skills.is_empty() {
        println!(
            "  🧩 Skills:   {}",
            skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Collect active channels from a shared builder to keep startup and doctor parity.
    #[allow(unused_mut)]
    let mut channels: Vec<Arc<dyn Channel>> =
        collect_configured_channels(&config, "runtime startup", &tool_specs)
            .into_iter()
            .map(|configured| configured.channel)
            .collect();

    #[cfg(all(feature = "channel-nostr", feature = "channels-vendor"))]
    if let Some(ref ns) = config.channels.nostr {
        channels.push(Arc::new(
            NostrChannel::new(&ns.private_key, ns.relays.clone(), &ns.allowed_pubkeys).await?,
        ));
    }
    if channels.is_empty() {
        println!("No channels configured. Run `operant onboard` to set up channels.");
        return Ok(());
    }

    println!("🦀 Operant Channel Server");
    println!("  🤖 Model:    {model}");
    let effective_backend = operant_memory::effective_memory_backend_name(
        &config.memory.backend,
        Some(&config.storage.provider.config),
    );
    println!(
        "  🧠 Memory:   {} (auto-save: {})",
        effective_backend,
        if config.memory.auto_save { "on" } else { "off" }
    );
    println!(
        "  📡 Channels: {}",
        channels
            .iter()
            .map(|c| c.name())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();
    println!("  Listening for messages... (Ctrl+C to stop)");
    println!();

    operant_runtime::health::mark_component_ok("channels");

    let initial_backoff_secs = config
        .reliability
        .channel_initial_backoff_secs
        .max(DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS);
    let max_backoff_secs = config
        .reliability
        .channel_max_backoff_secs
        .max(DEFAULT_CHANNEL_MAX_BACKOFF_SECS);

    // Single message bus — all channels send messages here
    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(100);

    // Spawn a listener for each channel
    let mut handles = Vec::new();
    for ch in &channels {
        handles.push(spawn_supervised_listener(
            ch.clone(),
            tx.clone(),
            initial_backoff_secs,
            max_backoff_secs,
        ));
    }
    drop(tx); // Drop our copy so rx closes when all channels stop

    let channels_by_name = Arc::new(
        channels
            .iter()
            .map(|ch| (ch.name().to_string(), Arc::clone(ch)))
            .collect::<HashMap<_, _>>(),
    );
    let _ = CRON_CHANNEL_REGISTRY.set(Arc::clone(&channels_by_name));

    // Populate the reaction tool's channel map now that channels are initialized.
    if let Some(ref handle) = reaction_handle_ch {
        let mut map = handle.write();
        for (name, ch) in channels_by_name.as_ref() {
            map.insert(name.clone(), Arc::clone(ch));
        }
    }

    // Populate the ask_user tool's channel map now that channels are initialized.
    if let Some(ref handle) = ask_user_handle_ch {
        let mut map = handle.write();
        for (name, ch) in channels_by_name.as_ref() {
            map.insert(name.clone(), Arc::clone(ch));
        }
    }

    // Populate the escalate_to_human tool's channel map now that channels are initialized.
    if let Some(ref handle) = escalate_handle_ch {
        let mut map = handle.write();
        for (name, ch) in channels_by_name.as_ref() {
            map.insert(name.clone(), Arc::clone(ch));
        }
    }

    let max_in_flight_messages = compute_max_in_flight_messages(channels.len());

    println!("  🚦 In-flight message limit: {max_in_flight_messages}");

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert(provider_name.clone(), Arc::clone(&provider));
    let message_timeout_secs =
        effective_channel_message_timeout_secs(config.channels.message_timeout_secs);
    let interrupt_on_new_message = config
        .channels
        .telegram
        .as_ref()
        .is_some_and(|tg| tg.interrupt_on_new_message);
    let interrupt_on_new_message_slack = config
        .channels
        .slack
        .as_ref()
        .is_some_and(|sl| sl.interrupt_on_new_message);
    let interrupt_on_new_message_discord = config
        .channels
        .discord
        .as_ref()
        .is_some_and(|dc| dc.interrupt_on_new_message);
    let interrupt_on_new_message_mattermost = config
        .channels
        .mattermost
        .as_ref()
        .is_some_and(|mm| mm.interrupt_on_new_message);
    let interrupt_on_new_message_matrix = config
        .channels
        .matrix
        .as_ref()
        .is_some_and(|mx| mx.interrupt_on_new_message);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name,
        provider: Arc::clone(&provider),
        default_provider: Arc::new(provider_name),
        prompt_config: Arc::new(config.clone()),
        memory: Arc::clone(&mem),
        tools_registry: Arc::clone(&tools_registry),
        observer,
        system_prompt: Arc::new(system_prompt),
        model: Arc::new(model.clone()),
        temperature,
        auto_save_memory: config.memory.auto_save,
        max_tool_iterations: config.agent.max_tool_iterations,
        min_relevance_score: config.memory.min_relevance_score,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.clone()),
        api_url: config
            .providers
            .fallback_provider()
            .and_then(|e| e.base_url.clone()),
        reliability: Arc::new(config.reliability.clone()),
        provider_runtime_options,
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: interrupt_on_new_message,
            slack: interrupt_on_new_message_slack,
            discord: interrupt_on_new_message_discord,
            mattermost: interrupt_on_new_message_mattermost,
            matrix: interrupt_on_new_message_matrix,
        },
        multimodal: config.multimodal.clone(),
        media_pipeline: config.media_pipeline.clone(),
        transcription_config: config.transcription.clone(),
        hooks: if config.hooks.enabled {
            let mut runner = operant_runtime::hooks::HookRunner::new();
            if config.hooks.builtin.command_logger {
                runner.register(Box::new(
                    operant_runtime::hooks::builtin::CommandLoggerHook::new(),
                ));
            }
            if config.hooks.builtin.webhook_audit.enabled {
                runner.register(Box::new(
                    operant_runtime::hooks::builtin::WebhookAuditHook::new(
                        config.hooks.builtin.webhook_audit.clone(),
                    ),
                ));
            }
            Some(Arc::new(runner))
        } else {
            None
        },
        non_cli_excluded_tools: Arc::new(config.autonomy.non_cli_excluded_tools.clone()),
        autonomy_level: config.autonomy.level,
        tool_call_dedup_exempt: Arc::new(config.agent.tool_call_dedup_exempt.clone()),
        model_routes: Arc::new(config.providers.model_routes.clone()),
        query_classification: config.query_classification.clone(),
        ack_reactions: config.channels.ack_reactions,
        show_tool_calls: config.channels.show_tool_calls,
        session_store: if config.channels.session_persistence {
            match operant_infra::make_session_backend(
                &config.workspace_dir,
                &config.channels.session_backend,
            ) {
                Ok(backend) => {
                    tracing::info!(
                        "📂 Session persistence enabled (backend: {})",
                        config.channels.session_backend
                    );
                    Some(backend)
                }
                Err(e) => {
                    tracing::warn!("Session persistence disabled: {e}");
                    None
                }
            }
        } else {
            None
        },
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(&config.autonomy)),
        activated_tools: ch_activated_handle,
        cost_tracking: operant_runtime::cost::CostTracker::get_or_init_global(
            config.cost.clone(),
            &config.workspace_dir,
        )
        .map(|tracker| ChannelCostTrackingState {
            tracker,
            prices: Arc::new(config.combined_pricing()),
        }),
        pacing: config.pacing.clone(),
        max_tool_result_chars: config.agent.max_tool_result_chars,
        context_token_budget: config.agent.max_context_tokens,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::from_millis(config.channels.debounce_ms),
        )),
        receipt_generator: if config.agent.tool_receipts.enabled {
            Some(operant_runtime::agent::tool_receipts::ReceiptGenerator::new())
        } else {
            None
        },
        show_receipts_in_response: config.agent.tool_receipts.show_in_response,
    });

    // Hydrate in-memory conversation histories from persisted JSONL session files.
    // Cap to MAX_CONVERSATION_SENDERS sessions (sorted by file mtime, most recent first)
    // and trim each to MAX_CHANNEL_HISTORY turns to bound startup memory.
    // If the last persisted turn is a user message (orphan from a crash mid-query),
    // close it with a marker so the LLM doesn't try to continue the old request.
    if let Some(ref store) = runtime_ctx.session_store {
        let mut hydrated = 0usize;
        let mut orphans_closed = 0usize;

        // Sort by last activity (most recent first) for predictable hydration.
        // The SessionBackend trait carries last_activity in metadata, so any
        // backend (JSONL, SQLite) can answer this question without a side
        // call to a backend-specific mtime method.
        let mut metadata = store.list_sessions_with_metadata();
        metadata.sort_by_key(|m| std::cmp::Reverse(m.last_activity));
        metadata.truncate(MAX_CONVERSATION_SENDERS);
        let session_keys: Vec<String> = metadata.into_iter().map(|m| m.key).collect();

        let mut histories = runtime_ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for key in session_keys {
            let mut msgs = store.load(&key);
            if msgs.is_empty() {
                continue;
            }
            // Trim to MAX_CHANNEL_HISTORY turns (keep most recent).
            if msgs.len() > MAX_CHANNEL_HISTORY {
                msgs.drain(..msgs.len() - MAX_CHANNEL_HISTORY);
            }
            // Close orphaned user turns from crashed sessions.
            if msgs.last().is_some_and(|m| m.role == "user") {
                let closure =
                    ChatMessage::assistant("[Session interrupted — not continuing this request]");
                if let Err(e) = store.append(&key, &closure) {
                    tracing::debug!("Failed to persist orphan closure for {key}: {e}");
                }
                msgs.push(closure);
                orphans_closed += 1;
            }
            // Self-heal: strip orphaned tool_result messages left by a prior
            // compaction that dropped the assistant tool_use without its paired
            // tool_result. Must run LAST, after every other mutation, so any
            // future trim step inserted above is covered by the same guard.
            // Without this, the session is bricked until the file is deleted
            // because every API call fails with 400 "unexpected tool_use_id
            // in tool_result blocks". See #5813.
            operant_runtime::agent::history_pruner::remove_orphaned_tool_messages(&mut msgs);
            hydrated += 1;
            histories.push(key, msgs);
        }
        drop(histories);
        if hydrated > 0 {
            tracing::info!("📂 Restored {hydrated} session(s) from disk");
        }
        if orphans_closed > 0 {
            tracing::info!(
                "🔒 Closed {orphans_closed} orphaned session turn(s) from previous crash"
            );
        }
    }

    run_message_dispatch_loop(rx, runtime_ctx, max_in_flight_messages).await;

    // Wait for all channel tasks
    for h in handles {
        let _ = h.await;
    }

    Ok(())
}
