//! `run` — extracted verbatim from the former loop_.rs monolith.
//! Re-exported from `loop_` so every import path is unchanged.

use crate::approval::ApprovalManager;
use operant_api::agent::TurnEvent;

/// CLI channel factory, injected by the binary. Returns a `Box<dyn Channel>` for interactive mode.
use super::*;

/// Map every [`RunOverrides`] field onto the facade's
/// [`crate::agent::reconciled::FacadeTurnOverrides`], field for field:
/// the two override strings, temperature, the peripheral-tool list, the
/// interactive flag, the session-state path and the tool allowlist all
/// carry straight across. The observer is NOT carried: the facade's
/// observer is construction-bound and is already the caller's observer
/// (passed to `build_facade_agent`); the override form exists for the
/// Wave-3 per-turn swap hook, so it stays `None` here rather than
/// producing a warn on every turn.
fn facade_overrides(overrides: &RunOverrides) -> crate::agent::reconciled::FacadeTurnOverrides {
    crate::agent::reconciled::FacadeTurnOverrides {
        provider_override: overrides.provider_override.clone(),
        model_override: overrides.model_override.clone(),
        temperature: Some(overrides.temperature),
        peripheral_overrides: overrides.peripheral_overrides.clone(),
        interactive: overrides.interactive,
        session_state_file: overrides.session_state_file.clone(),
        allowed_tools: overrides.allowed_tools.clone(),
        observer: None,
    }
}

/// W1.8b: facade turns park permission requests on the TurnEvent
/// channel; an unanswered request sits until core's 120s deadline
/// denies it. The CLI/daemon paths have no transport consumer, so
/// their drain loops answer every request with the SAME policy the
/// old tool-loop [`ApprovalManager`] applied: `approval_requirement()`
/// decides, and `Prompt` means the terminal (interactive run) or
/// denial (no operator on the daemon paths). Decisions flow through
/// `record_decision` so the session allowlist ("always") and the audit
/// log keep their old behavior.
pub(super) async fn answer_approval_request(
    approvals: &crate::agent::reconciled::ReconciledApprovals,
    policy: &ApprovalManager,
    interactive: bool,
    request_id: &str,
    tool_name: &str,
    arguments_summary: &str,
) {
    use crate::approval::ApprovalRequirement;
    use operant_api::channel::ChannelApprovalResponse;

    let (response, legacy) = match policy.approval_requirement(tool_name) {
        ApprovalRequirement::Approved | ApprovalRequirement::NotRequired => (
            ChannelApprovalResponse::Approve,
            crate::approval::ApprovalResponse::Yes,
        ),
        ApprovalRequirement::Prompt if !interactive => (
            ChannelApprovalResponse::Deny,
            crate::approval::ApprovalResponse::No,
        ),
        ApprovalRequirement::Prompt => {
            eprintln!("\n\u{1b}[1m{tool_name}\u{1b}[0m requires approval:\n  {arguments_summary}");
            // W1.8b-fix: the read runs on a blocking task — an inline
            // `read_line` in the forwarder arm parks a tokio worker for the
            // user's whole think-time.
            let answer = tokio::task::spawn_blocking(|| {
                let mut line = String::new();
                std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)
                    .ok()
                    .map(|_| line.trim().to_string())
            })
            .await
            .unwrap_or(None);
            match answer.as_deref() {
                Some("a") | Some("always") => (
                    ChannelApprovalResponse::AlwaysApprove,
                    crate::approval::ApprovalResponse::Always,
                ),
                Some("y") | Some("yes") => (
                    ChannelApprovalResponse::Approve,
                    crate::approval::ApprovalResponse::Yes,
                ),
                _ => (
                    ChannelApprovalResponse::Deny,
                    crate::approval::ApprovalResponse::No,
                ),
            }
        }
    };
    policy.record_decision(
        tool_name,
        &serde_json::Value::String(arguments_summary.to_string()),
        legacy,
        if interactive { "cli" } else { "daemon" },
    );
    if !approvals.resolve(request_id, response) {
        tracing::warn!(
            request_id,
            tool_name,
            "approval request expired before answer"
        );
    }
}

/// Facade construction for the Loop C CLI/daemon adapters
/// (`loop_::run`, `loop_::process_message`). W1.8b: the turn itself now
/// runs on the reconciled facade (Loop A under the hood) instead of
/// `run_tool_call_loop`.
///
/// The inputs are the same ones the old call site handed the tool loop:
/// the caller's already-resolved `(provider, model)` pair (the facade
/// must not re-resolve it — `run`'s model-switch loop and the channels
/// route selection both own that choice), the caller's own runtime tool
/// registry (Loop C's tool surface), the observer, the memory-session id
/// and the cost-tracking context. Every other facade input
/// (`AgentConfig` from the process behavior settings, core's builtin tool
/// registry, MCP, evolution intervals) is the shared
/// `from_config_with_tools` construction.
///
/// `system_prompt` becomes the core agent's frozen system prompt so the
/// facade's turn carries Loop C's exact prompt rather than a
/// default — the same text `run_tool_call_loop` received as
/// `history[0]`.
///
/// `temperature` (thinking-adjusted, already clamped) rides the client
/// default — the only seam core exposes (W1.8b-fix).
/// `allowed_tools` restricts the facade registry at construction — the
/// caller's `RunOverrides.allowed_tools` (cron jobs set it), so user-
/// authored job allowlists keep their meaning (W1.8b-fix).
/// W1.8b-fix also pins the core session id to `memory_session_id` (which
/// `run` derives from `session_state_file`): without it every REPL turn
/// rehydrated a fresh empty transcript, because the facade is built per
/// turn while core's conversation lives in the DB keyed by session id.
pub(super) async fn build_facade_agent(
    config: &Config,
    provider_name: &str,
    provider: &Arc<dyn Provider>,
    model_name: &str,
    observer: Arc<dyn Observer>,
    memory_session_id: Option<&str>,
    cost_tracking_context: Option<ToolLoopCostTrackingContext>,
    system_prompt: &str,
    temperature: Option<f64>,
    allowed_tools: Option<Vec<String>>,
) -> Result<crate::agent::reconciled::ReconciledAgent> {
    let facade = crate::agent::reconciled::ReconciledAgent::from_config_with(
        config,
        Some(observer),
        memory_session_id,
        true,
        Some((
            provider_name.to_string(),
            Arc::clone(provider),
            model_name.to_string(),
        )),
        Some(system_prompt.to_string()),
        temperature,
        allowed_tools,
    )
    .await?;
    if let Some(id) = memory_session_id {
        facade.core_agent().set_session_id(id);
    }
    Ok(facade.with_cost_tracking_context(cost_tracking_context))
}

#[allow(clippy::too_many_lines)]
pub async fn run(
    config: Config,
    message: Option<String>,
    overrides: RunOverrides,
) -> Result<String> {
    let RunOverrides {
        provider_override,
        model_override,
        temperature,
        peripheral_overrides,
        interactive,
        session_state_file,
        allowed_tools,
        observer,
    } = overrides;
    // W1.8b: the facade maps `RunOverrides` field-by-field onto
    // `FacadeTurnOverrides`. Keep a snapshot of the originals so each
    // turn can be re-mapped (the interactive loop runs many turns, and
    // the non-interactive path can retry after a model switch).
    let overrides_snapshot = RunOverrides {
        provider_override: provider_override.clone(),
        model_override: model_override.clone(),
        temperature,
        peripheral_overrides: peripheral_overrides.clone(),
        interactive,
        session_state_file: session_state_file.clone(),
        allowed_tools: allowed_tools.clone(),
        observer: None,
    };
    // ── Wire up agnostic subsystems ──────────────────────────────
    let observer: Arc<dyn Observer> = observer
        .unwrap_or_else(|| Arc::from(observability::create_observer(&config.observability)));
    let runtime: Arc<dyn platform::RuntimeAdapter> =
        Arc::from(platform::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));

    let fallback_provider_loop = config.providers.fallback_provider();

    // ── Memory (the brain) ────────────────────────────────────────
    let mem: Arc<dyn Memory> = Arc::from(operant_memory::create_memory_with_storage_and_routes(
        &config.memory,
        &config.providers.embedding_routes,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        fallback_provider_loop.and_then(|e| e.api_key.as_deref()),
    )?);
    tracing::info!(backend = mem.name(), "Memory initialized");

    // ── Peripherals (merge peripheral tools into registry) ─
    if !peripheral_overrides.is_empty() {
        tracing::info!(
            peripherals = ?peripheral_overrides,
            "Peripheral overrides from CLI (config boards take precedence)"
        );
    }

    // ── Tools (including memory tools and peripherals) ────────────
    let (composio_key, composio_entity_id) = if config.composio.enabled {
        (
            config.composio.api_key.as_deref(),
            Some(config.composio.entity_id.as_str()),
        )
    } else {
        (None, None)
    };
    let (
        mut tools_registry,
        delegate_handle,
        _reaction_handle,
        _channel_map_handle,
        _ask_user_handle,
        _escalate_handle,
    ) = tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        mem.clone(),
        composio_key,
        composio_entity_id,
        &config.browser,
        &config.http_request,
        &config.web_fetch,
        &config.workspace_dir,
        &config.agents,
        fallback_provider_loop.and_then(|e| e.api_key.as_deref()),
        &config,
        None,
    );

    let peripheral_tools: Vec<Box<dyn Tool>> = if let Some(f) = PERIPHERAL_TOOLS_FN.get() {
        f(config.peripherals.clone()).await.unwrap_or_default()
    } else {
        vec![]
    };
    if !peripheral_tools.is_empty() {
        tracing::info!(count = peripheral_tools.len(), "Peripheral tools added");
        tools_registry.extend(peripheral_tools);
    }

    // ── Capability-based tool access control ─────────────────────
    // When `allowed_tools` is `Some(list)`, restrict the tool registry to only
    // those tools whose name appears in the list. Unknown names are silently
    // ignored. When `None`, all tools remain available (backward compatible).
    if let Some(ref allow_list) = allowed_tools {
        tools_registry.retain(|t| allow_list.iter().any(|name| name == t.name()));
        tracing::info!(
            allowed = allow_list.len(),
            retained = tools_registry.len(),
            "Applied capability-based tool access filter"
        );
    }

    // ── Wire MCP tools (non-fatal) — CLI path ────────────────────
    // NOTE: MCP tools are injected after built-in tool filtering
    // (filter_primary_agent_tools_or_fail / agent.allowed_tools / agent.denied_tools).
    // MCP servers are user-declared external integrations; the built-in allow/deny
    // filter is not appropriate for them and would silently drop all MCP tools when
    // a restrictive allowlist is configured. Keep this block after any such filter call.
    //
    // When `deferred_loading` is enabled, MCP tools are NOT added to the registry
    // eagerly. Instead, a `tool_search` built-in is registered so the LLM can
    // fetch schemas on demand. This reduces context window waste.
    let mut deferred_section = String::new();
    // W1.8b: deferred-MCP activation is core-owned on the facade path;
    // the registry (still built for the system prompt + skill tools)
    // keeps its ToolSearchTool registration, so the handle stays live
    // but no loop consumes it here. Underscored, not removed, until
    // W1.10 deletes the Loop C registry construction entirely.
    let mut _activated_handle: Option<
        std::sync::Arc<std::sync::Mutex<crate::tools::ActivatedToolSet>>,
    > = None;
    if config.mcp.enabled && !config.mcp.servers.is_empty() {
        tracing::info!(
            "Initializing MCP client — {} server(s) configured",
            config.mcp.servers.len()
        );
        match crate::tools::McpRegistry::connect_all(&config.mcp.servers).await {
            Ok(registry) => {
                let registry = std::sync::Arc::new(registry);
                if config.mcp.deferred_loading {
                    // Deferred path: build stubs and register tool_search
                    let deferred_set = crate::tools::DeferredMcpToolSet::from_registry(
                        std::sync::Arc::clone(&registry),
                    )
                    .await;
                    tracing::info!(
                        "MCP deferred: {} tool stub(s) from {} server(s)",
                        deferred_set.len(),
                        registry.server_count()
                    );
                    deferred_section = crate::tools::build_deferred_tools_section(&deferred_set);
                    let activated = std::sync::Arc::new(std::sync::Mutex::new(
                        crate::tools::ActivatedToolSet::new(),
                    ));
                    _activated_handle = Some(std::sync::Arc::clone(&activated));
                    tools_registry.push(Box::new(crate::tools::ToolSearchTool::new(
                        deferred_set,
                        activated,
                    )));
                } else {
                    // Eager path: register all MCP tools directly
                    let names = registry.tool_names();
                    let mut registered = 0usize;
                    for name in names {
                        if let Some(def) = registry.get_tool_def(&name).await {
                            let wrapper: std::sync::Arc<dyn Tool> =
                                std::sync::Arc::new(crate::tools::McpToolWrapper::new(
                                    name,
                                    def,
                                    std::sync::Arc::clone(&registry),
                                ));
                            if let Some(ref handle) = delegate_handle {
                                handle.write().push(std::sync::Arc::clone(&wrapper));
                            }
                            tools_registry.push(Box::new(crate::tools::ArcToolRef(wrapper)));
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
                tracing::error!("MCP registry failed to initialize: {e:#}");
            }
        }
    }

    // ── Resolve provider ─────────────────────────────────────────
    let mut provider_name = provider_override
        .as_deref()
        .or(config.providers.fallback.as_deref())
        .unwrap_or("openrouter")
        .to_string();

    let mut model_name = model_override
        .as_deref()
        .or(fallback_provider_loop.and_then(|e| e.model.as_deref()))
        .unwrap_or("anthropic/claude-sonnet-4")
        .to_string();

    let provider_runtime_options = operant_providers::provider_runtime_options_from_config(&config);

    let mut provider: Arc<dyn Provider> =
        Arc::from(operant_providers::create_routed_provider_with_options(
            &provider_name,
            fallback_provider_loop.and_then(|e| e.api_key.as_deref()),
            fallback_provider_loop.and_then(|e| e.base_url.as_deref()),
            &config.reliability,
            &config.providers.model_routes,
            &model_name,
            &provider_runtime_options,
        )?);

    // W1.8b-fix: the model-switch callback clone is gone — on the facade path
    // the switch is detected from the turn's `Err` (is_model_switch_requested,
    // the same signal both call sites already branch on), so this binding
    // had no consumer. The accessor itself stays for the W1.10 deletion
    // pass to remove.

    observer.record_event(&ObserverEvent::AgentStart {
        provider: provider_name.to_string(),
        model: model_name.to_string(),
    });

    // ── Hardware RAG (datasheet retrieval when peripherals + datasheet_dir) ──
    let hardware_rag: Option<crate::rag::HardwareRag> = config
        .peripherals
        .datasheet_dir
        .as_ref()
        .filter(|d| !d.trim().is_empty())
        .map(|dir| crate::rag::HardwareRag::load(&config.workspace_dir, dir.trim()))
        .and_then(Result::ok)
        .filter(|r: &crate::rag::HardwareRag| !r.is_empty());
    if let Some(ref rag) = hardware_rag {
        tracing::info!(chunks = rag.len(), "Hardware RAG loaded");
    }

    let board_names: Vec<String> = config
        .peripherals
        .boards
        .iter()
        .map(|b| b.board.clone())
        .collect();

    // ── Initialize locale-aware tool descriptions ──────────────────
    let i18n_locale = config
        .locale
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(crate::i18n::detect_locale);
    crate::i18n::init(&i18n_locale);

    // ── Build system prompt from workspace MD files (OpenClaw framework) ──
    let skills = crate::skills::load_skills_with_config(&config.workspace_dir, &config);

    // Register skill-defined tools as callable tool specs in the tool registry
    // so the LLM can invoke them via native function calling, not just XML prompts.
    tools::register_skill_tools(&mut tools_registry, &skills, security.clone());

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
    tool_descs.push((
        "cron_add",
        "Create a cron job. Supports schedule kinds: cron, at, every; and job types: shell or agent.",
    ));
    tool_descs.push((
        "cron_list",
        "List all cron jobs with schedule, status, and metadata.",
    ));
    tool_descs.push(("cron_remove", "Remove a cron job by job_id."));
    tool_descs.push((
        "cron_update",
        "Patch a cron job (schedule, enabled, command/prompt, model, delivery, session_target).",
    ));
    tool_descs.push((
        "cron_run",
        "Force-run a cron job immediately and record a run history entry.",
    ));
    tool_descs.push(("cron_runs", "Show recent run history for a cron job."));
    tool_descs.push((
        "screenshot",
        "Capture a screenshot of the current screen. Returns file path and base64-encoded PNG. Use when: visual verification, UI inspection, debugging displays.",
    ));
    tool_descs.push((
        "image_info",
        "Read image file metadata (format, dimensions, size) and optionally base64-encode it. Use when: inspecting images, preparing visual data for analysis.",
    ));
    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in system browser (allowlist-only, no scraping)",
        ));
    }
    if config.composio.enabled {
        tool_descs.push((
            "composio",
            "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). Use action='list' to discover, 'execute' to run (optionally with connected_account_id), 'connect' to OAuth.",
        ));
    }
    tool_descs.push((
        "schedule",
        "Manage scheduled tasks (create/list/get/cancel/pause/resume). Supports recurring cron and one-shot delays.",
    ));
    tool_descs.push((
        "model_routing_config",
        "Configure default model, scenario routing, and delegate agents. Use for natural-language requests like: 'set conversation to kimi and coding to gpt-5.3-codex'.",
    ));
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a sub-task to a specialized agent. Use when: task needs different model/capability, or to parallelize work.",
        ));
    }
    if config.peripherals.enabled && !config.peripherals.boards.is_empty() {
        tool_descs.push((
            "gpio_read",
            "Read GPIO pin value (0 or 1) on connected hardware (STM32, Arduino). Use when: checking sensor/button state, LED status.",
        ));
        tool_descs.push((
            "gpio_write",
            "Set GPIO pin high (1) or low (0) on connected hardware. Use when: turning LED on/off, controlling actuators.",
        ));
        tool_descs.push((
            "arduino_upload",
            "Upload agent-generated Arduino sketch. Use when: user asks for 'make a heart', 'blink pattern', or custom LED behavior on Arduino. You write the full .ino code; Operant compiles and uploads it. Pin 13 = built-in LED on Uno.",
        ));
        tool_descs.push((
            "hardware_memory_map",
            "Return flash and RAM address ranges for connected hardware. Use when: user asks for 'upper and lower memory addresses', 'memory map', or 'readable addresses'.",
        ));
        tool_descs.push((
            "hardware_board_info",
            "Return full board info (chip, architecture, memory map) for connected hardware. Use when: user asks for 'board info', 'what board do I have', 'connected hardware', 'chip info', or 'what hardware'.",
        ));
        tool_descs.push((
            "hardware_memory_read",
            "Read actual memory/register values from Nucleo via USB. Use when: user asks to 'read register values', 'read memory', 'dump lower memory 0-126', 'give address and value'. Params: address (hex, default 0x20000000), length (bytes, default 128).",
        ));
        tool_descs.push((
            "hardware_capabilities",
            "Query connected hardware for reported GPIO pins and LED pin. Use when: user asks what pins are available.",
        ));
    }
    retain_registered_tool_descriptions(&mut tool_descs, &tools_registry);
    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let native_tools = provider.supports_native_tools();
    let mut system_prompt = crate::agent::system_prompt::build_system_prompt_with_mode_and_autonomy(
        &config.workspace_dir,
        &model_name,
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

    // Append structured tool-use instructions with schemas (only for non-native providers)
    if !native_tools {
        system_prompt.push_str(&build_tool_instructions(&tools_registry));
    }

    // Append deferred MCP tool names so the LLM knows what is available
    if !deferred_section.is_empty() {
        system_prompt.push('\n');
        system_prompt.push_str(&deferred_section);
    }

    // ── Approval policy (W1.8b) ───────────────────────────────────
    // The facade drain answers permission requests with this manager:
    // interactive turns prompt the terminal (old tool-loop behavior),
    // non-interactive turns apply the daemon policy (auto_approve
    // passes, ask-gated denies — no operator).
    let approval_policy = if interactive {
        ApprovalManager::from_config(&config.autonomy)
    } else {
        ApprovalManager::for_non_interactive(&config.autonomy)
    };
    let memory_session_id = session_state_file.as_deref().and_then(|path| {
        let raw = path.to_string_lossy().trim().to_string();
        if raw.is_empty() {
            None
        } else {
            // Match the sanitized form persisted by memory backend migrations.
            Some(operant_api::session_keys::sanitize_session_key(&format!(
                "cli:{raw}"
            )))
        }
    });

    // ── Cost tracking context (scoped for CLI / cron / web agents) ──
    let cost_tracking_context: Option<ToolLoopCostTrackingContext> =
        crate::cost::CostTracker::get_or_init_global(config.cost.clone(), &config.workspace_dir)
            .map(|tracker| {
                ToolLoopCostTrackingContext::new(tracker, Arc::new(config.combined_pricing()))
            });

    // ── Execute ──────────────────────────────────────────────────
    let start = Instant::now();

    let mut final_output = String::new();

    // Save the base system prompt before any thinking modifications so
    // the interactive loop can restore it between turns.
    let base_system_prompt = system_prompt.clone();

    if let Some(msg) = message {
        // ── Parse thinking directive from user message ─────────
        let (thinking_directive, effective_msg) =
            match crate::agent::thinking::parse_thinking_directive(&msg) {
                Some((level, remaining)) => {
                    tracing::info!(thinking_level = ?level, "Thinking directive parsed from message");
                    (Some(level), remaining)
                }
                None => (None, msg.clone()),
            };
        let thinking_level = crate::agent::thinking::resolve_thinking_level(
            thinking_directive,
            None,
            &config.agent.thinking,
        );
        let thinking_params = crate::agent::thinking::apply_thinking_level(thinking_level);
        let effective_temperature = crate::agent::thinking::clamp_temperature(
            temperature + thinking_params.temperature_adjustment,
        );

        // Prepend thinking system prompt prefix when present.
        if let Some(ref prefix) = thinking_params.system_prompt_prefix {
            system_prompt = format!("{prefix}\n\n{system_prompt}");
        }

        if let Some(suggestion) = crate::skills::render_missing_skill_install_suggestion(
            &effective_msg,
            &skills,
            &config.workspace_dir,
            config.skills.install_suggestions.enabled,
        ) {
            final_output = suggestion.clone();
            println!("{suggestion}");
            observer.record_event(&ObserverEvent::TurnComplete);
            return Ok(final_output);
        }

        // Auto-save user message to memory (skip short/trivial messages)
        if config.memory.auto_save
            && effective_msg.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS
            && !operant_memory::should_skip_autosave_content(&effective_msg)
        {
            let user_key = autosave_memory_key("user_msg");
            let _ = mem
                .store(
                    &user_key,
                    &effective_msg,
                    MemoryCategory::Conversation,
                    memory_session_id.as_deref(),
                )
                .await;
        }

        // Inject memory + hardware RAG context into user message.
        // For non-interactive runs (cron, daemon heartbeat), exclude
        // Conversation-category memories so chat history does not leak
        // into autonomous executions. See #5415 / #5456.
        let mem_context = build_context(
            mem.as_ref(),
            &effective_msg,
            config.memory.min_relevance_score,
            memory_session_id.as_deref(),
            !interactive,
        )
        .await;
        let rag_limit = if config.agent.compact_context { 2 } else { 5 };
        let hw_context = hardware_rag
            .as_ref()
            .map(|r| build_hardware_context(r, &effective_msg, &board_names, rag_limit))
            .unwrap_or_default();
        let context = format!("{mem_context}{hw_context}");
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
        let enriched = if context.is_empty() {
            format!("[{now}] {effective_msg}")
        } else {
            format!("{context}[{now}] {effective_msg}")
        };

        let mut history = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&enriched),
        ];

        // Prune history for token efficiency (when enabled).
        if config.agent.history_pruning.enabled {
            let _stats = crate::agent::history_pruner::prune_history(
                &mut history,
                &config.agent.history_pruning,
            );
        }

        // Per-turn MCP exclusion from `tool_filter_groups` is Loop C-only
        // guardrail plumbing. The facade's core registry is
        // construction-bound (per-turn allowlist is the Wave-3 hook —
        // `FacadeTurnOverrides::allowed_tools` is the entry point), so the
        // exclusion set is computed for the warning and recorded in
        // BUGS.md (S8) rather than silently dropped.
        let excluded_tools = compute_excluded_mcp_tools(
            &tools_registry,
            &config.agent.tool_filter_groups,
            &effective_msg,
        );
        if !excluded_tools.is_empty() {
            tracing::warn!(
                count = excluded_tools.len(),
                "tool_filter_groups matched tools this turn; facade turn runs unfiltered until the W3 per-turn allowlist"
            );
        }

        #[allow(unused_assignments)]
        let mut response = String::new();
        loop {
            // W1.8b — the turn runs on the reconciled facade (Loop A under
            // the hood). Same inputs the tool loop took: this caller's
            // provider/model, its tool registry, its observer, its system
            // prompt, and the cost-tracking context (which the facade now
            // wraps around `core.run` itself, so the explicit
            // `TOOL_LOOP_COST_TRACKING_CONTEXT.scope` the old call site
            // applied is carried by `.with_cost_tracking_context`).
            let facade_result = {
                let mut facade = build_facade_agent(
                    &config,
                    &provider_name,
                    &provider,
                    &model_name,
                    Arc::clone(&observer),
                    memory_session_id.as_deref(),
                    cost_tracking_context.clone(),
                    &system_prompt,
                    Some(effective_temperature),
                    allowed_tools.clone(),
                )
                .await?;
                // No TurnEvent consumer on the one-shot CLI path, but
                // permission requests still arrive on this channel and
                // park core's turn until answered (or its 120s deadline
                // denies) — so the drain answers them per the daemon
                // policy; everything else is discarded (the response
                // string is `run`'s only output).
                let approvals_handle = facade.approvals().clone();
                let (facade_tx, mut facade_rx) = tokio::sync::mpsc::channel::<TurnEvent>(64);
                // W1.8b-fix: `&enriched` (memory RAG + hardware board
                // context + `[timestamp]` prefix) is what Loop C fed as
                // `history[1]`; passing the raw message silently dropped
                // all three. The temperature rides the client default
                // (`build_facade_agent`), so the warn-only per-turn
                // temperature override is no longer set here.
                let turn = facade.turn_streamed_with_overrides(
                    &enriched,
                    facade_tx,
                    None,
                    facade_overrides(&overrides_snapshot),
                );
                let (result, _) = tokio::join!(turn, async {
                    while let Some(event) = facade_rx.recv().await {
                        if let TurnEvent::ApprovalRequest {
                            request_id,
                            tool_name,
                            arguments_summary,
                            ..
                        } = event
                        {
                            answer_approval_request(
                                &approvals_handle,
                                &approval_policy,
                                false,
                                &request_id,
                                &tool_name,
                                &arguments_summary,
                            )
                            .await;
                        }
                    }
                });
                result
            };
            match facade_result {
                Ok(resp) => {
                    response = resp;
                    break;
                }
                Err(e) => {
                    if let Some((new_provider, new_model)) = is_model_switch_requested(&e) {
                        tracing::info!(
                            "Model switch requested, switching from {} {} to {} {}",
                            provider_name,
                            model_name,
                            new_provider,
                            new_model
                        );

                        provider =
                            Arc::from(operant_providers::create_routed_provider_with_options(
                                &new_provider,
                                fallback_provider_loop.and_then(|e| e.api_key.as_deref()),
                                fallback_provider_loop.and_then(|e| e.base_url.as_deref()),
                                &config.reliability,
                                &config.providers.model_routes,
                                &new_model,
                                &provider_runtime_options,
                            )?);

                        provider_name = new_provider;
                        model_name = new_model;

                        clear_model_switch_request();

                        observer.record_event(&ObserverEvent::AgentStart {
                            provider: provider_name.to_string(),
                            model: model_name.to_string(),
                        });

                        continue;
                    }
                    return Err(e);
                }
            }
        }

        // After successful multi-step execution, attempt autonomous skill creation.
        if config.skills.skill_creation.enabled {
            let tool_calls = crate::skills::creator::extract_tool_calls_from_history(&history);
            if tool_calls.len() >= 2 {
                let creator = crate::skills::creator::SkillCreator::new(
                    config.workspace_dir.clone(),
                    config.skills.skill_creation.clone(),
                );
                match creator.create_from_execution(&msg, &tool_calls, None).await {
                    Ok(Some(slug)) => {
                        tracing::info!(slug, "Auto-created skill from execution");
                    }
                    Ok(None) => {
                        tracing::debug!("Skill creation skipped (duplicate or disabled)");
                    }
                    Err(e) => tracing::warn!("Skill creation failed: {e}"),
                }
            }
        }
        final_output = response.clone();
        println!("{response}");
        observer.record_event(&ObserverEvent::TurnComplete);
    } else {
        println!("🦀 Operant Interactive Mode");
        println!("Type /help for commands.\n");
        #[expect(
            clippy::expect_used,
            reason = "startup invariant: register_cli_channel_fn is called before interactive entry"
        )]
        let cli = CLI_CHANNEL_FN
            .get()
            .expect("CLI channel factory not registered — call register_cli_channel_fn at startup")(
        );

        // Persistent conversation history across turns
        let mut history = if let Some(path) = session_state_file.as_deref() {
            load_interactive_session_history(path, &system_prompt)?
        } else {
            vec![ChatMessage::system(&system_prompt)]
        };

        loop {
            print!("> ");
            let _ = std::io::stdout().flush();

            // Read raw bytes to avoid UTF-8 validation errors when PTY
            // transport splits multi-byte characters at frame boundaries
            // (e.g. CJK input with spaces over kubectl exec / SSH).
            let mut raw = Vec::new();
            match std::io::BufRead::read_until(&mut std::io::stdin().lock(), b'\n', &mut raw) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("\nError reading input: {e}\n");
                    break;
                }
            }
            let input = String::from_utf8_lossy(&raw).into_owned();

            let user_input = input.trim().to_string();
            if user_input.is_empty() {
                continue;
            }
            match user_input.as_str() {
                "/quit" | "/exit" => break,
                "/help" => {
                    println!("Available commands:");
                    println!("  /help             Show this help message");
                    println!("  /clear /new       Clear conversation history");
                    println!("  /quit /exit       Exit interactive mode");
                    println!(
                        "  /think:<level>    Set reasoning depth (off|minimal|low|medium|high|max)\n"
                    );
                    continue;
                }
                "/clear" | "/new" => {
                    println!(
                        "This will clear the current conversation and delete all session memory."
                    );
                    println!("Core memories (long-term facts/preferences) will be preserved.");
                    print!("Continue? [y/N] ");
                    let _ = std::io::stdout().flush();

                    let mut confirm_raw = Vec::new();
                    if std::io::BufRead::read_until(
                        &mut std::io::stdin().lock(),
                        b'\n',
                        &mut confirm_raw,
                    )
                    .is_err()
                    {
                        continue;
                    }
                    let confirm = String::from_utf8_lossy(&confirm_raw);
                    if !matches!(confirm.trim().to_lowercase().as_str(), "y" | "yes") {
                        println!("Cancelled.\n");
                        continue;
                    }

                    history.clear();
                    history.push(ChatMessage::system(&system_prompt));
                    // Clear conversation and daily memory
                    let mut cleared = 0;
                    for category in [MemoryCategory::Conversation, MemoryCategory::Daily] {
                        let entries = mem.list(Some(&category), None).await.unwrap_or_default();
                        for entry in entries {
                            if mem.forget(&entry.key).await.unwrap_or(false) {
                                cleared += 1;
                            }
                        }
                    }
                    if cleared > 0 {
                        println!("Conversation cleared ({cleared} memory entries removed).\n");
                    } else {
                        println!("Conversation cleared.\n");
                    }
                    if let Some(path) = session_state_file.as_deref() {
                        save_interactive_session_history(path, &history)?;
                    }
                    continue;
                }
                _ => {}
            }

            // ── Parse thinking directive from interactive input ───
            let (thinking_directive, effective_input) =
                match crate::agent::thinking::parse_thinking_directive(&user_input) {
                    Some((level, remaining)) => {
                        tracing::info!(thinking_level = ?level, "Thinking directive parsed");
                        (Some(level), remaining)
                    }
                    None => (None, user_input.clone()),
                };
            let thinking_level = crate::agent::thinking::resolve_thinking_level(
                thinking_directive,
                None,
                &config.agent.thinking,
            );
            let thinking_params = crate::agent::thinking::apply_thinking_level(thinking_level);
            let turn_temperature = crate::agent::thinking::clamp_temperature(
                temperature + thinking_params.temperature_adjustment,
            );

            // For non-Medium levels, temporarily patch the system prompt with prefix.
            let turn_system_prompt;
            if let Some(ref prefix) = thinking_params.system_prompt_prefix {
                turn_system_prompt = format!("{prefix}\n\n{system_prompt}");
                // Update the system message in history for this turn.
                if let Some(sys_msg) = history.first_mut()
                    && sys_msg.role == "system"
                {
                    sys_msg.content = turn_system_prompt.clone();
                }
            }

            if let Some(suggestion) = crate::skills::render_missing_skill_install_suggestion(
                &effective_input,
                &skills,
                &config.workspace_dir,
                config.skills.install_suggestions.enabled,
            ) {
                final_output = suggestion.clone();
                if let Err(e) = operant_api::channel::Channel::send(
                    &*cli,
                    &operant_api::channel::SendMessage::new(format!("\n{suggestion}\n"), "user"),
                )
                .await
                {
                    eprintln!("\nError sending CLI response: {e}\n");
                }
                observer.record_event(&ObserverEvent::TurnComplete);
                if thinking_params.system_prompt_prefix.is_some()
                    && let Some(sys_msg) = history.first_mut()
                    && sys_msg.role == "system"
                {
                    sys_msg.content.clone_from(&base_system_prompt);
                }
                continue;
            }

            // Auto-save conversation turns (skip short/trivial messages)
            if config.memory.auto_save
                && effective_input.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS
                && !operant_memory::should_skip_autosave_content(&effective_input)
            {
                let user_key = autosave_memory_key("user_msg");
                let _ = mem
                    .store(
                        &user_key,
                        &effective_input,
                        MemoryCategory::Conversation,
                        memory_session_id.as_deref(),
                    )
                    .await;
            }

            // Inject memory + hardware RAG context into user message.
            // Interactive REPL: keep Conversation memories (user is actively
            // chatting in this session and may want their own history recalled).
            let mem_context = build_context(
                mem.as_ref(),
                &effective_input,
                config.memory.min_relevance_score,
                memory_session_id.as_deref(),
                false,
            )
            .await;
            let rag_limit = if config.agent.compact_context { 2 } else { 5 };
            let hw_context = hardware_rag
                .as_ref()
                .map(|r| build_hardware_context(r, &effective_input, &board_names, rag_limit))
                .unwrap_or_default();
            let context = format!("{mem_context}{hw_context}");
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
            let enriched = if context.is_empty() {
                format!("[{now}] {effective_input}")
            } else {
                format!("{context}[{now}] {effective_input}")
            };

            history.push(ChatMessage::user(&enriched));

            // Per-turn MCP exclusion from `tool_filter_groups` — same
            // treatment as the non-interactive path: warn + BUGS.md S8,
            // W3 per-turn allowlist is the carry target.
            let excluded_tools = compute_excluded_mcp_tools(
                &tools_registry,
                &config.agent.tool_filter_groups,
                &effective_input,
            );
            if !excluded_tools.is_empty() {
                tracing::warn!(
                    count = excluded_tools.len(),
                    "tool_filter_groups matched tools this turn; facade turn runs unfiltered until the W3 per-turn allowlist"
                );
            }

            // Set up streaming channel so tool progress and response
            // content are printed progressively instead of buffered.
            let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel::<DraftEvent>(64);
            let content_was_streamed =
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let content_streamed_flag = content_was_streamed.clone();
            let is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());

            let consumer_handle = tokio::spawn(async move {
                use std::io::Write;
                while let Some(event) = delta_rx.recv().await {
                    match event {
                        StreamDelta::Status(text) => {
                            if is_tty {
                                let _ = write!(std::io::stderr(), "\x1b[2m{text}\x1b[0m");
                            } else {
                                let _ = write!(std::io::stderr(), "{text}");
                            }
                            let _ = std::io::stderr().flush();
                        }
                        StreamDelta::Text(text) => {
                            content_streamed_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                            print!("{text}");
                            let _ = std::io::stdout().flush();
                        }
                    }
                }
            });

            // Ctrl+C cancels the in-flight turn instead of killing the process.
            let cancel_token = CancellationToken::new();
            let cancel_token_clone = cancel_token.clone();
            let ctrlc_handle = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    cancel_token_clone.cancel();
                }
            });

            let response = loop {
                // W1.8b — interactive turn on the reconciled facade. The
                // draft sink rides through `FacadeSink::pair`, so the
                // `StreamDelta::Text` deltas the CLI consumer prints
                // arrive exactly as the tool loop's `on_delta` produced
                // them (`TurnEvent::Chunk`/`Thinking` -> `DraftEvent`).
                // `TOOL_LOOP_COST_TRACKING_CONTEXT` is applied by the
                // facade (`with_cost_tracking_context` wraps `core.run`).
                let facade_result = {
                    let mut facade = build_facade_agent(
                        &config,
                        &provider_name,
                        &provider,
                        &model_name,
                        Arc::clone(&observer),
                        memory_session_id.as_deref(),
                        cost_tracking_context.clone(),
                        &system_prompt,
                        Some(turn_temperature),
                        allowed_tools.clone(),
                    )
                    .await?;
                    // The facade writes into `facade_tx`; the forwarder
                    // below pumps each event through `FacadeSink::send`,
                    // which is what fans `TurnEvent::Chunk`/`Thinking`
                    // into the CLI's `DraftEvent` consumer. The consumer
                    // `event_tx` leg is drained (the CLI has no
                    // TurnEvent consumer of its own — it renders the
                    // draft channel).
                    let (facade_tx, mut facade_rx) = tokio::sync::mpsc::channel::<TurnEvent>(64);
                    let (consumer_tx, mut consumer_rx) =
                        tokio::sync::mpsc::channel::<TurnEvent>(64);
                    let sink = crate::agent::reconciled::FacadeSink::pair(
                        consumer_tx,
                        Some(delta_tx.clone()),
                    );
                    let approvals_handle = facade.approvals().clone();
                    // W1.8b-fix: `&enriched` (Loop C's `history[1]`: memory
                    // RAG + hardware board context + timestamp prefix) and
                    // the thinking-adjusted temperature rides the client
                    // default, so the warn-only per-turn temperature
                    // override is not set.
                    let turn = facade.turn_streamed_with_overrides(
                        &enriched,
                        facade_tx,
                        Some(cancel_token.clone()),
                        facade_overrides(&overrides_snapshot),
                    );
                    // Permission requests are answered on the terminal
                    // here (the old tool-loop prompt), everything else
                    // fans out to the CLI draft consumer as before.
                    let (result, _) = tokio::join!(turn, async {
                        while let Some(event) = facade_rx.recv().await {
                            if let TurnEvent::ApprovalRequest {
                                request_id,
                                tool_name,
                                arguments_summary,
                                ..
                            } = event
                            {
                                answer_approval_request(
                                    &approvals_handle,
                                    &approval_policy,
                                    true,
                                    &request_id,
                                    &tool_name,
                                    &arguments_summary,
                                )
                                .await;
                                continue;
                            }
                            sink.send(event).await;
                        }
                    });
                    while consumer_rx.try_recv().is_ok() {}
                    result
                };
                match facade_result {
                    Ok(resp) => break resp,
                    Err(e) => {
                        if is_tool_loop_cancelled(&e) {
                            eprintln!("\n\x1b[2m(cancelled)\x1b[0m");
                            break String::new();
                        }
                        if let Some((new_provider, new_model)) = is_model_switch_requested(&e) {
                            tracing::info!(
                                "Model switch requested, switching from {} {} to {} {}",
                                provider_name,
                                model_name,
                                new_provider,
                                new_model
                            );

                            provider =
                                Arc::from(operant_providers::create_routed_provider_with_options(
                                    &new_provider,
                                    fallback_provider_loop.and_then(|e| e.api_key.as_deref()),
                                    fallback_provider_loop.and_then(|e| e.base_url.as_deref()),
                                    &config.reliability,
                                    &config.providers.model_routes,
                                    &new_model,
                                    &provider_runtime_options,
                                )?);

                            provider_name = new_provider;
                            model_name = new_model;

                            clear_model_switch_request();

                            observer.record_event(&ObserverEvent::AgentStart {
                                provider: provider_name.to_string(),
                                model: model_name.to_string(),
                            });

                            continue;
                        }
                        // Context overflow recovery: compress and retry
                        // Context overflow recovery: compress and retry.
                        // W1.4: one compressor — the core pair, driven via
                        // the reconciled facade (fresh probe window per
                        // error, matching the per-error construction the
                        // deleted engine used).
                        if operant_providers::reliable::is_context_window_exceeded(&e) {
                            tracing::warn!(
                                "Context overflow in interactive loop, attempting recovery"
                            );
                            let mut window = config.agent.max_context_tokens;
                            let error_msg = format!("{e}");
                            match crate::agent::reconciled::compress_on_error(
                                &mut history,
                                provider.as_ref(),
                                &model_name,
                                &error_msg,
                                &crate::agent::reconciled::PreflightConfig::default(),
                                &mut window,
                                Some(&mem),
                                memory_session_id.as_deref(),
                            )
                            .await
                            {
                                Ok(true) => {
                                    tracing::info!(
                                        "Context recovered via compression, retrying turn"
                                    );
                                    continue;
                                }
                                Ok(false) => {
                                    tracing::warn!("Compression ran but couldn't reduce enough");
                                }
                                Err(compress_err) => {
                                    tracing::warn!(
                                        error = %compress_err,
                                        "Compression failed during recovery"
                                    );
                                }
                            }
                        }

                        eprintln!("\nError: {e}\n");
                        break String::new();
                    }
                }
            };

            // Clean up: stop the Ctrl+C listener and flush streaming events.
            ctrlc_handle.abort();
            drop(delta_tx);
            let _ = consumer_handle.await;

            final_output = response.clone();
            if content_was_streamed.load(std::sync::atomic::Ordering::Relaxed) {
                println!();
            } else if let Err(e) = operant_api::channel::Channel::send(
                &*cli,
                &operant_api::channel::SendMessage::new(format!("\n{response}\n"), "user"),
            )
            .await
            {
                eprintln!("\nError sending CLI response: {e}\n");
            }
            observer.record_event(&ObserverEvent::TurnComplete);

            // Context compression before hard trimming to preserve long-context signal.
            // W1.4: the core pair via the reconciled facade — todo re-injection
            // now also folds the active todo list back in here.
            {
                match crate::agent::reconciled::compress_if_needed(
                    &mut history,
                    &provider,
                    &model_name,
                    &crate::agent::reconciled::PreflightConfig::default(),
                    config.agent.max_context_tokens,
                    Some(&mem),
                    memory_session_id.as_deref(),
                )
                .await
                {
                    Ok(result) if result.compressed => {
                        tracing::info!(
                            passes = result.passes_used,
                            before = result.tokens_before,
                            after = result.tokens_after,
                            "Context compression complete"
                        );
                    }
                    Ok(_) => {} // No compression needed
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Context compression failed, falling back to history trim"
                        );
                        trim_history(&mut history, config.agent.max_history_messages / 2);
                    }
                }
            }

            // Hard cap as a safety net.
            trim_history(&mut history, config.agent.max_history_messages);

            // Restore base system prompt (remove per-turn thinking prefix).
            if thinking_params.system_prompt_prefix.is_some()
                && let Some(sys_msg) = history.first_mut()
                && sys_msg.role == "system"
            {
                sys_msg.content.clone_from(&base_system_prompt);
            }

            if let Some(path) = session_state_file.as_deref() {
                save_interactive_session_history(path, &history)?;
            }
        }
    }

    let duration = start.elapsed();
    observer.record_event(&ObserverEvent::AgentEnd {
        provider: provider_name.to_string(),
        model: model_name.to_string(),
        duration,
        tokens_used: None,
        cost_usd: None,
    });

    Ok(final_output)
}

#[cfg(test)]
mod w18b_tests {
    //! W1.8b: the facade's approval answerer must reproduce the old
    //! tool-loop ApprovalManager decisions on the CLI/daemon paths:
    //! auto-approve passes, ask-gated denies (no operator), Full autonomy
    //! approves everything, and an "always" answer persists into the
    //! session allowlist so later requests auto-approve.

    use super::*;
    use crate::approval::{ApprovalManager, ApprovalResponse};

    fn autonomy_cfg() -> operant_config::schema::AutonomyConfig {
        operant_config::schema::AutonomyConfig::default()
    }

    #[tokio::test]
    async fn daemon_auto_approve_passes_and_ask_gated_denies() {
        let mut cfg = autonomy_cfg();
        cfg.level = crate::security::AutonomyLevel::Supervised;
        cfg.auto_approve.push("read_file".to_string());
        cfg.always_ask.push("deny_gated_tool".to_string());
        let policy = ApprovalManager::for_non_interactive(&cfg);
        let approvals = crate::agent::reconciled::ReconciledApprovals::default();

        // auto_approve tool: allowed, recorded as Yes on the daemon channel.
        answer_approval_request(&approvals, &policy, false, "r1", "read_file", "{}").await;
        // ask-gated tool: denied (no operator to prompt), recorded as No.
        answer_approval_request(&approvals, &policy, false, "r2", "deny_gated_tool", "{}").await;

        let log = policy.audit_log();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].tool_name, "read_file");
        assert_eq!(log[0].decision, ApprovalResponse::Yes);
        assert_eq!(log[0].channel, "daemon");
        assert_eq!(log[1].tool_name, "deny_gated_tool");
        assert_eq!(log[1].decision, ApprovalResponse::No);
    }

    #[tokio::test]
    async fn full_autonomy_approves_everything_without_operator() {
        let mut cfg = autonomy_cfg();
        cfg.level = crate::security::AutonomyLevel::Full;
        let policy = ApprovalManager::for_non_interactive(&cfg);
        let approvals = crate::agent::reconciled::ReconciledApprovals::default();

        answer_approval_request(&approvals, &policy, false, "r1", "shell", "rm -rf /").await;
        let log = policy.audit_log();
        assert_eq!(log[0].decision, ApprovalResponse::Yes);
    }

    #[tokio::test]
    async fn always_answer_persists_into_session_allowlist() {
        let mut cfg = autonomy_cfg();
        cfg.level = crate::security::AutonomyLevel::Supervised;
        cfg.auto_approve.retain(|t| t != "grep");
        let policy = ApprovalManager::from_config(&cfg);
        let approvals = crate::agent::reconciled::ReconciledApprovals::default();

        // "always" decision lands in the manager's session allowlist
        // (record_decision); the very next requirement check must now
        // resolve as Approved without prompting.
        policy.record_decision(
            "grep",
            &serde_json::Value::String("{}".to_string()),
            ApprovalResponse::Always,
            "cli",
        );
        assert!(
            policy.approval_requirement("grep") != crate::approval::ApprovalRequirement::Prompt
        );
        assert!(policy.session_allowlist().contains("grep"));
    }

    #[tokio::test]
    async fn resolving_unknown_request_is_a_no_op_warn() {
        let policy = ApprovalManager::for_non_interactive(&autonomy_cfg());
        let approvals = crate::agent::reconciled::ReconciledApprovals::default();
        // No parked request with this id: resolve() must return false
        // (logged warn in the answerer) and NOT panic.
        answer_approval_request(&approvals, &policy, false, "nonexistent", "read_file", "{}").await;
        // The decision was still recorded (audit honesty): one entry.
        assert_eq!(policy.audit_log().len(), 1);
    }
}
