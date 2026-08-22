//! `messages` — extracted verbatim from the former loop_.rs monolith.
//! Re-exported from `loop_` so every import path is unchanged.

use crate::approval::ApprovalManager;

/// CLI channel factory, injected by the binary. Returns a `Box<dyn Channel>` for interactive mode.
use super::*;

pub async fn process_message(
    config: Config,
    message: &str,
    session_id: Option<&str>,
    observer: Option<Arc<dyn Observer>>,
) -> Result<String> {
    let observer: Arc<dyn Observer> = observer
        .unwrap_or_else(|| Arc::from(observability::create_observer(&config.observability)));
    let runtime: Arc<dyn platform::RuntimeAdapter> =
        Arc::from(platform::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let fallback_provider_pm = config.providers.fallback_provider();
    let approval_manager = ApprovalManager::for_non_interactive(&config.autonomy);
    let mem: Arc<dyn Memory> = Arc::from(operant_memory::create_memory_with_storage_and_routes(
        &config.memory,
        &config.providers.embedding_routes,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        fallback_provider_pm.and_then(|e| e.api_key.as_deref()),
    )?);

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
        delegate_handle_pm,
        _reaction_handle_pm,
        _channel_map_handle_pm,
        _ask_user_handle_pm,
        _escalate_handle_pm,
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
        fallback_provider_pm.and_then(|e| e.api_key.as_deref()),
        &config,
        None,
    );
    let peripheral_tools: Vec<Box<dyn Tool>> = if let Some(f) = PERIPHERAL_TOOLS_FN.get() {
        f(config.peripherals.clone()).await.unwrap_or_default()
    } else {
        vec![]
    };
    tools_registry.extend(peripheral_tools);

    // ── Wire MCP tools (non-fatal) — process_message path ────────
    // NOTE: Same ordering contract as the CLI path above — MCP tools must be
    // injected after filter_primary_agent_tools_or_fail (or equivalent built-in
    // tool allow/deny filtering) to avoid MCP tools being silently dropped.
    let mut deferred_section = String::new();
    let mut activated_handle_pm: Option<
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
                    activated_handle_pm = Some(std::sync::Arc::clone(&activated));
                    tools_registry.push(Box::new(crate::tools::ToolSearchTool::new(
                        deferred_set,
                        activated,
                    )));
                } else {
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
                            if let Some(ref handle) = delegate_handle_pm {
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

    // ── Per-platform tool policy (gateway/non-CLI path) ──────────
    // When `gateway.platform_toolsets` is configured for the `api_server`
    // platform (this function is the gateway daemon / webhook path), narrow
    // the registry to the allow-list. Empty config → all tools (legacy
    // behavior), so this is a strict no-op unless explicitly configured.
    let all_names: Vec<String> = tools_registry
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    let allowed: Vec<String> = operant_tool_planning::resolve_platform_tool_names(
        &config.gateway.platform_toolsets,
        "api_server",
        &all_names.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    if allowed.len() < all_names.len() {
        let allowed_set: std::collections::HashSet<&str> =
            allowed.iter().map(String::as_str).collect();
        let before = tools_registry.len();
        tools_registry.retain(|tool| allowed_set.contains(tool.name()));
        tracing::info!(
            "Platform tool policy (api_server): {}/{} tools exposed",
            tools_registry.len(),
            before
        );
    }

    let provider_name = config.providers.fallback.as_deref().unwrap_or("openrouter");
    let model_name = match fallback_provider_pm
        .and_then(|e| e.model.as_deref())
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(m) => m.to_string(),
        None => match config.providers.resolve_default_model() {
            Some(m) => {
                tracing::warn!(
                    provider = provider_name,
                    model = %m,
                    "fallback provider has no `model` set; using first configured \
                     providers.models entry as default. Set [providers.models.{provider_name}] \
                     model = \"...\" to silence this warning.",
                );
                m
            }
            None => {
                anyhow::bail!(
                    "no model configured: providers.fallback = {:?} resolves with no model, \
                     and no [[providers.models.*]] entry has a `model` field set. \
                     Configure at least one [providers.models.<name>] model = \"...\" \
                     or define a [[model_routes]] hint.",
                    config.providers.fallback,
                )
            }
        },
    };
    let provider_runtime_options = operant_providers::provider_runtime_options_from_config(&config);
    let provider: Box<dyn Provider> = operant_providers::create_routed_provider_with_options(
        provider_name,
        fallback_provider_pm.and_then(|e| e.api_key.as_deref()),
        fallback_provider_pm.and_then(|e| e.base_url.as_deref()),
        &config.reliability,
        &config.providers.model_routes,
        &model_name,
        &provider_runtime_options,
    )?;

    let hardware_rag: Option<crate::rag::HardwareRag> = config
        .peripherals
        .datasheet_dir
        .as_ref()
        .filter(|d| !d.trim().is_empty())
        .map(|dir| crate::rag::HardwareRag::load(&config.workspace_dir, dir.trim()))
        .and_then(Result::ok)
        .filter(|r: &crate::rag::HardwareRag| !r.is_empty());
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

    let skills = crate::skills::load_skills_with_config(&config.workspace_dir, &config);

    // Register skill-defined tools as callable tool specs (process_message path).
    tools::register_skill_tools(&mut tools_registry, &skills, security.clone());

    let mut tool_descs: Vec<(&str, &str)> = vec![
        ("shell", "Execute terminal commands."),
        ("file_read", "Read file contents."),
        ("file_write", "Write file contents."),
        ("memory_store", "Save to memory."),
        ("memory_recall", "Search memory."),
        ("memory_forget", "Delete a memory entry."),
        (
            "model_routing_config",
            "Configure default model, scenario routing, and delegate agents.",
        ),
        ("screenshot", "Capture a screenshot."),
        ("image_info", "Read image metadata."),
    ];
    if matches!(
        config.skills.prompt_injection_mode,
        operant_config::schema::SkillsPromptInjectionMode::Compact
    ) {
        tool_descs.push((
            "read_skill",
            "Load the full source for an available skill by name.",
        ));
    }
    if config.browser.enabled {
        tool_descs.push(("browser_open", "Open approved URLs in browser."));
    }
    if config.composio.enabled {
        tool_descs.push(("composio", "Execute actions on 1000+ apps via Composio."));
    }
    if config.peripherals.enabled && !config.peripherals.boards.is_empty() {
        tool_descs.push(("gpio_read", "Read GPIO pin value on connected hardware."));
        tool_descs.push((
            "gpio_write",
            "Set GPIO pin high or low on connected hardware.",
        ));
        tool_descs.push((
            "arduino_upload",
            "Upload Arduino sketch. Use for 'make a heart', custom patterns. You write full .ino code; Operant uploads it.",
        ));
        tool_descs.push((
            "hardware_memory_map",
            "Return flash and RAM address ranges. Use when user asks for memory addresses or memory map.",
        ));
        tool_descs.push((
            "hardware_board_info",
            "Return full board info (chip, architecture, memory map). Use when user asks for board info, what board, connected hardware, or chip info.",
        ));
        tool_descs.push((
            "hardware_memory_read",
            "Read actual memory/register values from Nucleo. Use when user asks to read registers, read memory, dump lower memory 0-126, or give address and value.",
        ));
        tool_descs.push((
            "hardware_capabilities",
            "Query connected hardware for reported GPIO pins and LED pin. Use when user asks what pins are available.",
        ));
    }

    // Filter out tools excluded for non-CLI channels (gateway counts as non-CLI).
    // Skip when autonomy is `Full` — full-autonomy agents keep all tools.
    if config.autonomy.level != AutonomyLevel::Full {
        let excluded = &config.autonomy.non_cli_excluded_tools;
        if !excluded.is_empty() {
            tool_descs.retain(|(name, _)| !excluded.iter().any(|ex| ex == name));
        }
    }
    let effective_tool_names: HashSet<&str> = tools_registry
        .iter()
        .map(|tool| tool.name())
        .filter(|name| {
            config.autonomy.level == AutonomyLevel::Full
                || !config
                    .autonomy
                    .non_cli_excluded_tools
                    .iter()
                    .any(|excluded| excluded.as_str() == *name)
        })
        .collect();
    tool_descs.retain(|(name, _)| effective_tool_names.contains(name));

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
    if !native_tools {
        system_prompt.push_str(&build_tool_instructions_for_names(
            &tools_registry,
            &effective_tool_names,
        ));
    }
    if !deferred_section.is_empty() {
        system_prompt.push('\n');
        system_prompt.push_str(&deferred_section);
    }

    // ── Parse thinking directive from user message ─────────────
    let (thinking_directive, effective_message) =
        match crate::agent::thinking::parse_thinking_directive(message) {
            Some((level, remaining)) => {
                tracing::info!(thinking_level = ?level, "Thinking directive parsed from message");
                (Some(level), remaining)
            }
            None => (None, message.to_string()),
        };
    let thinking_level = crate::agent::thinking::resolve_thinking_level(
        thinking_directive,
        None,
        &config.agent.thinking,
    );
    let thinking_params = crate::agent::thinking::apply_thinking_level(thinking_level);
    let effective_temperature = crate::agent::thinking::clamp_temperature(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7)
            + thinking_params.temperature_adjustment,
    );

    // Prepend thinking system prompt prefix when present.
    if let Some(ref prefix) = thinking_params.system_prompt_prefix {
        system_prompt = format!("{prefix}\n\n{system_prompt}");
    }

    let effective_msg_ref = effective_message.as_str();
    if let Some(suggestion) = crate::skills::render_missing_skill_install_suggestion(
        effective_msg_ref,
        &skills,
        &config.workspace_dir,
        config.skills.install_suggestions.enabled,
    ) {
        return Ok(suggestion);
    }

    // process_message is the channel entrypoint (Discord, Telegram, gateway,
    // etc.) — recall is scoped to the channel's session_id, so retrieving the
    // user's own Conversation history within their session is intended.
    let mem_context = build_context(
        mem.as_ref(),
        effective_msg_ref,
        config.memory.min_relevance_score,
        session_id,
        false,
    )
    .await;
    let rag_limit = if config.agent.compact_context { 2 } else { 5 };
    let hw_context = hardware_rag
        .as_ref()
        .map(|r| build_hardware_context(r, effective_msg_ref, &board_names, rag_limit))
        .unwrap_or_default();
    let context = format!("{mem_context}{hw_context}");
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
    let enriched = if context.is_empty() {
        format!("[{now}] {effective_message}")
    } else {
        format!("{context}[{now}] {effective_message}")
    };

    let mut history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&enriched),
    ];
    let mut excluded_tools = compute_excluded_mcp_tools(
        &tools_registry,
        &config.agent.tool_filter_groups,
        effective_msg_ref,
    );
    if config.autonomy.level != AutonomyLevel::Full {
        excluded_tools.extend(config.autonomy.non_cli_excluded_tools.iter().cloned());
    }

    agent_turn(
        provider.as_ref(),
        &mut history,
        &tools_registry,
        observer.as_ref(),
        provider_name,
        &model_name,
        effective_temperature,
        true,
        "daemon",
        None,
        &config.multimodal,
        config.agent.max_tool_iterations,
        Some(&approval_manager),
        &excluded_tools,
        &config.agent.tool_call_dedup_exempt,
        activated_handle_pm.as_ref(),
        None,
        None, // channel: process_message path has no channel ref
    )
    .await
}
