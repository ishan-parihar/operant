// adapter_types/tui_app.rs — Application bootstrap and run loop.

use super::auth::AuthStore;
use super::config::Settings;
use crate::commands::CommandResult;

/// Send-safe copy of an MCP server config, moved into spawned tasks:
/// (name, transport, url, auth_token, command, args, env, enabled). Aliased
/// so the spawned-task tuple isn't repeated inline (clippy::type_complexity).
type McpServerConfigTuple = (
    String,
    operant_core::config::McpTransportKind,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<String>,
    std::collections::HashMap<String, String>,
    bool,
);

pub struct TuiApp {
    app: crate::tui::app::App,
    initial_query: Option<String>,
    /// Whether to skip EnableMouseCapture in the TUI setup. Set by the
    /// --no-mouse CLI flag. (Bug #24 from iter-82 audit.)
    no_mouse: bool,
}

impl TuiApp {
    pub async fn enter(
        config: operant_core::config::AppConfig,
        _system: Option<String>,
        _mode: LaunchMode,
        no_mouse: bool,
        dangerously_skip_permissions: bool,
    ) -> anyhow::Result<Self> {
        use crate::commands::{CommandContext, CommandHandler, CommandRegistry, CommandResult};
        use crate::tui::adapter_types::cost::CostTracker;
        use std::sync::Arc;

        let initial_query = match &_mode {
            LaunchMode::Query(q) => Some(q.clone()),
            _ => None,
        };
        let mut app_config = config;
        let settings = Settings::load_sync().unwrap_or_default();

        // Layer in the user's saved settings.json (written by App::persist_provider_and_model).
        // provider+model are now exclusively stored in operant.toml (config.agent.model /
        // config.client.base_url). The settings.json only carries visual prefs (theme, vim, etc.).

        if let Some(entry) = settings.providers.get("custom-openai")
            && let Some(ref base) = entry.api_base
        {
            app_config.client.base_url = base.clone();
        }

        let cost_tracker = Arc::new(CostTracker::new());

        let mut command_registry = CommandRegistry::new();
        // Register handlers for commands that were previously falling through to the agent
        struct CompactHandler;
        #[async_trait::async_trait]
        impl CommandHandler for CompactHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("Context compaction is handled automatically by the agent.")
            }
        }
        command_registry
            .register_handler("compact", Box::new(CompactHandler))
            .ok();

        struct DoctorHandler;
        #[async_trait::async_trait]
        impl CommandHandler for DoctorHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let mut report = String::from("Operant Diagnostics:\n");
                report.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));
                report.push_str(&format!(
                    "  Config dir: {:?}\n",
                    crate::tui::adapter_types::config::Settings::config_dir()
                ));
                let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok()
                    || std::env::var("OPENAI_API_KEY").is_ok();
                report.push_str(&format!("  API key configured: {}\n", api_key_set));
                report.push_str("  Status: OK\n");
                CommandResult::message(report)
            }
        }
        command_registry
            .register_handler("doctor", Box::new(DoctorHandler))
            .ok();

        struct InitHandler;
        #[async_trait::async_trait]
        impl CommandHandler for InitHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let agentic_dir = std::path::PathBuf::from("AGENTS.md");
                if agentic_dir.exists() {
                    CommandResult::message("AGENTS.md already exists in this project.")
                } else {
                    match std::fs::write(&agentic_dir, "# Project Agent Memory\n\n") {
                        Ok(_) => CommandResult::message("Created AGENTS.md in current directory."),
                        Err(e) => {
                            CommandResult::message(format!("Failed to create AGENTS.md: {}", e))
                        }
                    }
                }
            }
        }
        command_registry
            .register_handler("init", Box::new(InitHandler))
            .ok();

        struct LoginHandler;
        #[async_trait::async_trait]
        impl CommandHandler for LoginHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(
                    "Set your API key: export ANTHROPIC_API_KEY=sk-... or export OPENAI_API_KEY=sk-...",
                )
            }
        }
        command_registry
            .register_handler("login", Box::new(LoginHandler))
            .ok();

        struct LogoutHandler;
        #[async_trait::async_trait]
        impl CommandHandler for LogoutHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("Clear your API key: unset ANTHROPIC_API_KEY OPENAI_API_KEY")
            }
        }
        command_registry
            .register_handler("logout", Box::new(LogoutHandler))
            .ok();

        struct RefreshHandler;
        #[async_trait::async_trait]
        impl CommandHandler for RefreshHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("Provider auth and model caches cleared.")
            }
        }
        command_registry
            .register_handler("refresh", Box::new(RefreshHandler))
            .ok();

        struct ProvidersHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ProvidersHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let auth = AuthStore::load();
                let mut report = String::from("Available providers:\n");
                for p in crate::provider::PROVIDERS {
                    let has_key = auth.api_key_for(p.name).is_some();
                    let env_key = !p.env_var.is_empty() && std::env::var(p.env_var).is_ok();
                    let configured = has_key || env_key;
                    report.push_str(&format!(
                        "  {}: {}\n",
                        p.display_name,
                        if configured {
                            "configured"
                        } else {
                            "not configured"
                        }
                    ));
                }
                report.push_str("\nUsage: /provider <name> — switch LLM provider");
                CommandResult::message(report)
            }
        }
        command_registry
            .register_handler("providers", Box::new(ProvidersHandler))
            .ok();

        struct StatusHandler;
        #[async_trait::async_trait]
        impl CommandHandler for StatusHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let model = std::env::var("OPERANT_MODEL").unwrap_or_else(|_| "gpt-4".to_string());
                let anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok();
                let openai = std::env::var("OPENAI_API_KEY").is_ok();
                CommandResult::message(format!(
                    "Session Status:\n  Model: {}\n  Anthropic: {}\n  OpenAI: {}",
                    model,
                    if anthropic {
                        "configured"
                    } else {
                        "not configured"
                    },
                    if openai {
                        "configured"
                    } else {
                        "not configured"
                    }
                ))
            }
        }
        command_registry
            .register_handler("status", Box::new(StatusHandler))
            .ok();

        struct VersionHandler;
        #[async_trait::async_trait]
        impl CommandHandler for VersionHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(format!("operant v{}", env!("CARGO_PKG_VERSION")))
            }
        }
        command_registry
            .register_handler("version", Box::new(VersionHandler))
            .ok();

        struct TimeHandler;
        #[async_trait::async_trait]
        impl CommandHandler for TimeHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
            }
        }
        command_registry
            .register_handler("time", Box::new(TimeHandler))
            .ok();

        struct DebugHandler;
        #[async_trait::async_trait]
        impl CommandHandler for DebugHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let mut info = String::from("Debug Info:\n");
                info.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));
                info.push_str(&format!(
                    "  Config dir: {:?}\n",
                    crate::tui::adapter_types::config::Settings::config_dir()
                ));
                info.push_str(&format!(
                    "  Rust version: {}\n",
                    env!("CARGO_PKG_RUST_VERSION")
                ));
                CommandResult::message(info)
            }
        }
        command_registry
            .register_handler("debug", Box::new(DebugHandler))
            .ok();

        // (iter-270: NewHandler, HistoryHandler, RetryHandler, UndoHandler,
        // StopHandler removed — these commands are now intercepted in
        // app.rs intercept_slash_command_with_args_impl and directly
        // mutate App state. The registry fallback path is no longer reached.)

        struct CompressHandler;
        #[async_trait::async_trait]
        impl CommandHandler for CompressHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("Context compaction is handled automatically by the agent.")
            }
        }
        command_registry
            .register_handler("compress", Box::new(CompressHandler))
            .ok();

        // (iter-270: RollbackHandler removed — /rollback is intercepted in app.rs)
        // (iter-270: BranchHandler removed — /branch falls through to /session browser)
        // (iter-270: GoalHandler removed — /goal is intercepted in app.rs)
        // (iter-270: YoloHandler removed — /yolo is intercepted in app.rs)
        // (iter-270: PersonalityHandler removed — /personality is intercepted in app.rs)
        // (iter-270: ReasoningHandler removed — /reasoning is intercepted in app.rs)
        // (iter-270: SkillsHandler removed — /skills is intercepted in app.rs)
        // (iter-270: CreditsHandler removed — /credits is intercepted in app.rs)
        // (iter-270: BillingHandler removed — /billing is intercepted in app.rs)
        // (iter-270: SessionsHandler removed — /sessions is intercepted in app.rs)

        struct ProviderHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ProviderHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                if ctx.args.is_empty() {
                    CommandResult::message("Usage: /provider <name> — switch LLM provider")
                } else {
                    CommandResult::message(format!("Provider switched to: {}", ctx.args))
                }
            }
        }
        command_registry
            .register_handler("provider", Box::new(ProviderHandler))
            .ok();

        struct ToolsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ToolsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(
                    "Available tools: memory, web_search, web_fetch, bash, and more. Use /toolsets for the full list.",
                )
            }
        }
        command_registry
            .register_handler("tools", Box::new(ToolsHandler))
            .ok();

        struct BundlesHandler;
        #[async_trait::async_trait]
        impl CommandHandler for BundlesHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(
                    "Skill bundles: curated sets of skills for specific workflows. (No bundles installed)",
                )
            }
        }
        command_registry
            .register_handler("bundles", Box::new(BundlesHandler))
            .ok();

        struct UsageHandler;
        #[async_trait::async_trait]
        impl CommandHandler for UsageHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(
                    "Token usage and rate limits are displayed in the stats dialog. Use /stats to view.",
                )
            }
        }
        command_registry
            .register_handler("usage", Box::new(UsageHandler))
            .ok();

        struct InsightsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for InsightsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(
                    "Insights: session analysis and conversation statistics. Use /stats for details.",
                )
            }
        }
        command_registry
            .register_handler("insights", Box::new(InsightsHandler))
            .ok();

        struct UpdateHandler;
        #[async_trait::async_trait]
        impl CommandHandler for UpdateHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message(format!(
                    "Current version: {}. Check https://github.com/operant-ai/operant-rs for updates.",
                    env!("CARGO_PKG_VERSION")
                ))
            }
        }
        command_registry
            .register_handler("update", Box::new(UpdateHandler))
            .ok();

        struct WhoamiHandler;
        #[async_trait::async_trait]
        impl CommandHandler for WhoamiHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("Access level: admin (local TUI session)")
            }
        }
        command_registry
            .register_handler("whoami", Box::new(WhoamiHandler))
            .ok();

        let mut app =
            crate::tui::app::App::new(app_config, settings, cost_tracker, command_registry);

        // Wire the voice-mode notice: if audio input is available (e.g. not
        // an SSH session, ffmpeg/arecord installed) and the user hasn't
        // enabled voice mode yet, show a one-time hint on startup.
        let audio_env = operant_core::voice::detect_audio_environment();
        app.voice_mode_notice
            .show_if_available(audio_env.available, false);

        // First-run onboarding: if no credentials and onboarding hasn't been
        // completed, auto-open the connect dialog so the user is guided to
        // set up a provider. (P0-2 from UX audit — was silently dropping the
        // user onto a blank welcome screen with no guidance.)
        if !app.has_credentials {
            let settings = Settings::load_sync().unwrap_or_default();
            if !settings.has_completed_onboarding {
                app.connect_dialog.open();
                app.status_message =
                    Some("Welcome to Operant! Connect a provider to get started.".to_string());
            }
        }

        // --dangerously-skip-permissions: show the bypass-permissions
        // confirmation dialog at startup. Bypass mode is NOT enabled yet —
        // it's applied only when the user accepts (see app.rs dialog handler).
        if dangerously_skip_permissions {
            app.bypass_permissions_dialog.show();
        }

        Ok(Self {
            app,
            initial_query,
            no_mouse,
        })
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        // Create the agent event channel directly — no bridge.
        // (iter-114 — eliminates the bridge layer. The TUI now receives
        // AgentEvent directly and handles it in handle_agent_event.)
        let (agent_tx, agent_rx) =
            tokio::sync::mpsc::channel::<operant_core::agent::AgentEvent>(256);
        self.app.agent_event_rx = Some(agent_rx);

        use crossterm::execute;
        use crossterm::terminal::{
            EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        };
        use ratatui::Terminal;
        use ratatui::backend::CrosstermBackend;

        enable_raw_mode()?;

        // Install a panic hook that restores the terminal before printing
        // the panic message. Without this, any panic between enable_raw_mode
        // and disable_raw_mode leaves the user's terminal in raw mode +
        // alternate screen (broken terminal, garbled output).
        let no_mouse = self.no_mouse;
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            if !no_mouse {
                let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
            }
            let _ = execute!(std::io::stdout(), crossterm::event::DisableFocusChange);
            prev_hook(info);
        }));

        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        // Enable mouse capture unless --no-mouse was passed. Mouse capture
        // lets the TUI receive scroll/click events for the transcript, diff
        // viewer, and overlay scrolling. Some terminal multiplexers (tmux,
        // screen) interfere with mouse capture; --no-mouse disables it so
        // the terminal's native mouse selection works. (Bug #24 from iter-82
        // audit — /mouse mentioned a --no-mouse flag that didn't exist.)
        if !self.no_mouse {
            execute!(stdout, crossterm::event::EnableMouseCapture)?;
        }
        // Enable focus-change reporting so the TUI can pause animations and
        // drop the redraw cadence when the window is backgrounded (Phase 2.3).
        // Terminals that don't support focus events simply ignore the sequence.
        execute!(stdout, crossterm::event::EnableFocusChange)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // agent_tx was created above; agent_rx is stored on app.
        // No bridge — the agent sends AgentEvent directly to the TUI.

        let (permission_tx, permission_rx) =
            tokio::sync::mpsc::channel::<operant_core::agent::ToolPermissionRequest>(4);
        self.app.permission_rx = Some(permission_rx);

        let config = self.app.config.clone();
        let mcp_manager = operant_core::mcp::McpManager::new();
        let skills_dir = config.skills.root_dir.clone();

        let agent: Option<std::sync::Arc<operant_core::agent::OperantAgent>> =
            match crate::create_runtime_agent(
                &config,
                &config.agent,
                None,
                agent_tx,
                &mcp_manager,
                &skills_dir,
            )
            .await
            {
                Ok(agent) => Some(std::sync::Arc::new(agent.with_permissions(permission_tx))),
                Err(e) => {
                    self.app.status_message = Some(format!("Agent init failed: {}", e));
                    None
                }
            };

        // Store the real McpManager + steer queue handle on the App so the
        // run loop can act on /mcp reconnect and /steer. (iter-93 — closes
        // the /mcp reconnect + /steer parity gaps.)
        self.app.core_mcp_manager = Some(std::sync::Arc::new(mcp_manager));
        if let Some(ref agent) = agent {
            self.app.steer_queue_handle = Some(agent.steer_queue_handle());
            // Clone of the agent's live registry so /mcp reconnect can
            // materialize MCP tools mid-session via sync_tools_to_registry.
            self.app.core_tool_registry = Some(agent.registry());
        }

        // Create the user-question channel and register the sender with
        // operant_core::user_question. The clarify tool will push
        // UserQuestionRequest { question, choices, reply_tx } to this
        // channel; the TUI drains it in the run loop and opens the
        // ask_user_dialog. (iter-97 — closes Bug #2 from iter-82 audit.)
        let (uq_tx, uq_rx) = tokio::sync::mpsc::unbounded_channel::<
            operant_core::user_question::UserQuestionRequest,
        >();
        let _ = operant_core::user_question::set_user_question_sender(uq_tx);
        self.app.user_question_rx = Some(uq_rx);

        // Attach the MCP manager + file-history + current-turn counter to the
        // App. Without these, /mcp always shows "Disconnected" for every
        // server, /changes always shows "No changes", the "iter N" status
        // pill never renders, and the subagent HUD never renders. (Bug #3
        // from iter-82 audit.)
        // (iter-208: stub TUI McpManager deleted. load_mcp_servers now reads
        // directly from core_mcp_manager, which is set below at line ~2148.)

        // (iter-209: FileHistory + current_turn stub creation deleted.
        // The turn-diff feature never worked — /changes now uses git-diff.)

        // Force a context-window-size refresh so /context shows real numbers
        // on the first frame instead of "0 / 0" (Bug #13 from iter-82 audit).
        self.app.refresh_context_window_size();

        self.app.model_registry.load_models_dev().await;

        if let Some(query) = self.initial_query.take()
            && let Some(ref agent) = agent
        {
            self.submit_user_message(agent, query);
        }

        let result = loop {
            // /skill <name> and /bundle <name>: drain pending_user_message set
            // by the intercept arm at the TOP of the loop so the expansion
            // submits immediately after Enter — no further keystroke needed.
            // (iter-320 — hermes-parity skill invocation expansion.)
            // Guard order matters: check streaming/agent BEFORE .take() so a
            // pending message is never consumed and dropped when a turn is
            // already running (it stays queued for the next idle iteration).
            if !self.app.is_streaming
                && let Some(agent) = agent.as_ref()
                && let Some(inject_text) = self.app.pending_user_message.take()
            {
                self.submit_user_message(agent, inject_text);
                continue;
            }
            // /retry: drain pending_retry_query set by the intercept arm at
            // the loop top too, so /retry fires immediately after Enter
            // (same one-keystroke semantics as /skill — iter-320).
            if !self.app.is_streaming
                && let Some(agent) = agent.as_ref()
                && let Some(retry_text) = self.app.pending_retry_query.take()
            {
                self.submit_user_message(agent, retry_text);
                continue;
            }

            match self.app.run(&mut terminal) {
                Ok(Some(input)) => {
                    // Poll pending MCP state set by /mcp 'a' (panel auth) and
                    // 'r' (reconnect) keys. Without this, the keys set state
                    // that the run loop never reads, so panel-auth + reconnect
                    // are no-ops. (Bug #7 from iter-82 audit.)
                    // For now we surface a status message acknowledging the
                    // request; a real implementation would spawn the MCP
                    // panel-auth flow / reconnect the MCP runtime.
                    if let Some(server_name) = self.app.take_pending_mcp_panel_auth() {
                        self.app.status_message = Some(format!(
                            "MCP panel auth requested for '{}' (not yet wired — restart operant to re-authenticate).",
                            server_name
                        ));
                    }
                    if self.app.take_pending_mcp_reconnect() {
                        // Real MCP reconnect: re-add all configured servers,
                        // then sync their tools into the live ToolRegistry.
                        // (iter-93 — closes the /mcp reconnect parity gap.)
                        // We extract the server configs into a plain Vec of
                        // Send-safe tuples first (so the async block doesn't
                        // capture the AppConfig, which contains non-Send
                        // tracing types via the McpManager's internal spans).
                        if let Some(ref mcp) = self.app.core_mcp_manager {
                            let mcp_clone = std::sync::Arc::clone(mcp);
                            // Send-safe ToolRegistry handle so deferred MCP
                            // servers materialize their tools mid-session.
                            let registry_clone = self.app.core_tool_registry.clone();
                            // Extract server configs into Send-safe tuples.
                            let server_configs: Vec<McpServerConfigTuple> = self
                                .app
                                .config
                                .mcp
                                .servers
                                .iter()
                                .map(|s| {
                                    (
                                        s.name.clone(),
                                        s.transport.clone(),
                                        s.url.clone(),
                                        s.auth_token.clone(),
                                        s.command.clone(),
                                        s.args.clone(),
                                        s.env.clone(),
                                        s.enabled,
                                    )
                                })
                                .collect();
                            // Status channel so the background task can report
                            // what reconnected (and what failed) back to the
                            // run loop, which drains it like bridge_state_rx.
                            let (reconnect_tx, reconnect_rx) =
                                tokio::sync::mpsc::unbounded_channel::<String>();
                            self.app.mcp_reconnect_rx = Some(reconnect_rx);
                            tokio::spawn(async move {
                                use operant_core::config::McpTransportKind;
                                let mut reconnected = Vec::new();
                                let mut failures = Vec::new();
                                for (
                                    name,
                                    transport,
                                    url,
                                    auth_token,
                                    command,
                                    args,
                                    env,
                                    enabled,
                                ) in server_configs
                                {
                                    if !enabled {
                                        continue;
                                    }
                                    // Remove first (no-op if not present).
                                    let _ = mcp_clone.remove_server(&name).await;
                                    // Re-add based on transport.
                                    let result = match transport {
                                        McpTransportKind::Http
                                        | McpTransportKind::StreamableHttp => {
                                            if let Some(url) = url {
                                                mcp_clone.add_server(&name, url, auth_token).await
                                            } else {
                                                Err(operant_core::error::Error::Agent(format!(
                                                    "no URL configured for {name}"
                                                )))
                                            }
                                        }
                                        McpTransportKind::Stdio => match command {
                                            Some(command) => {
                                                mcp_clone
                                                    .add_stdio_server(&name, command, args, env)
                                                    .await
                                            }
                                            None => {
                                                Err(operant_core::error::Error::Agent(format!(
                                                    "no command configured for stdio server {name}"
                                                )))
                                            }
                                        },
                                    };
                                    match result {
                                        Ok(()) => reconnected.push(name),
                                        Err(e) => failures.push(format!("{name}: {e}")),
                                    }
                                }
                                // Materialize tools for every connected server
                                // (incl. any deferred ones the user just
                                // reconnected) into the live registry so the
                                // next turn sees them without a restart.
                                let mut tools_synced = false;
                                if let Some(registry) = registry_clone {
                                    mcp_clone.sync_tools_to_registry(&registry).await;
                                    tools_synced = true;
                                }
                                let mut msg = format!(
                                    "MCP reconnect complete — {} server(s) reconnected: {}.",
                                    reconnected.len(),
                                    if reconnected.is_empty() {
                                        "none".to_string()
                                    } else {
                                        reconnected.join(", ")
                                    }
                                );
                                if !tools_synced {
                                    msg.push_str(" (tools not synced — no live registry attached)");
                                }
                                if !failures.is_empty() {
                                    msg.push_str(&format!(" Failed: {}", failures.join("; ")));
                                }
                                let _ = reconnect_tx.send(msg);
                            });
                            self.app.status_message = Some(
                                "MCP reconnect initiated — servers will reconnect in the background.".to_string()
                            );
                        } else {
                            self.app.status_message = Some(
                                "MCP reconnect requested but no McpManager is attached."
                                    .to_string(),
                            );
                        }
                    }

                    // Poll device_auth_pending set by /connect for github-copilot
                    // and openai-codex. Without this, the device-code dialog
                    // shows "waiting for code" forever because no background
                    // device-flow task is ever spawned. (Bug #15 from iter-82
                    // audit — partial fix; full fix needs the device-flow task
                    // spawned here.)
                    if let Some(provider) = self.app.device_auth_pending.take() {
                        self.app.status_message = Some(format!(
                            "Device auth initiated for '{}' — open the provider's URL in a browser to complete. (Background polling not yet wired; restart operant after authenticating.)",
                            provider
                        ));
                    }

                    // Poll bridge/gateway connection state updates from handler.
                    if let Some(ref mut rx) = self.app.bridge_state_rx {
                        while let Ok(state) = rx.try_recv() {
                            self.app.bridge_state = state;
                            self.app
                                .transcript_version
                                .set(self.app.transcript_version.get().wrapping_add(1));
                        }
                    }

                    // Poll MCP reconnect status messages from the background
                    // reconnect task (what reconnected, what failed).
                    if let Some(ref mut rx) = self.app.mcp_reconnect_rx {
                        while let Ok(msg) = rx.try_recv() {
                            self.app.status_message = Some(msg);
                        }
                    }

                    // If a slash command set a pending shell command on a
                    // *previous* iteration, run it BEFORE processing the next
                    // input. (Slash commands set the field inside
                    // handle_tui_command → intercept_slash_command, then we
                    // `continue` to the next loop iteration; we run the shell
                    // command at the top of the next iteration so the TUI
                    // gets a chance to redraw the "Launching…" status message
                    // before we suspend.)
                    if let Some(argv) = self.app.pending_shell_command.take() {
                        if let Err(e) = run_suspended_shell_command(&mut terminal, &argv) {
                            self.app.status_message = Some(format!("Shell command failed: {}", e));
                        } else {
                            self.app.status_message = Some("Returned to operant.".to_string());
                        }
                        // Force a redraw on the next frame so the status
                        // message + restored terminal show immediately.
                        self.app
                            .transcript_version
                            .set(self.app.transcript_version.get().wrapping_add(1));
                    }

                    if crate::input::is_slash_command(&input) {
                        let (cmd, args) = crate::input::parse_slash_command(&input);
                        if self.app.handle_tui_command(cmd, args) {
                            // If the slash command set a pending shell command,
                            // we need to run it on the NEXT iteration — but
                            // app.run() will block waiting for input. To avoid
                            // that, run it NOW if it was set.
                            if let Some(argv) = self.app.pending_shell_command.take() {
                                if let Err(e) = run_suspended_shell_command(&mut terminal, &argv) {
                                    self.app.status_message =
                                        Some(format!("Shell command failed: {}", e));
                                } else {
                                    self.app.status_message =
                                        Some("Returned to operant.".to_string());
                                }
                                self.app
                                    .transcript_version
                                    .set(self.app.transcript_version.get().wrapping_add(1));
                            }
                            continue;
                        }
                        if let Some(canonical) = self.app.command_registry.resolve(cmd) {
                            match self.app.command_registry.execute(canonical, args).await {
                                CommandResult::Message(output) => {
                                    self.app.push_system_message(
                                        output,
                                        crate::tui::app::SystemMessageStyle::Info,
                                    );
                                }
                                CommandResult::Error(e) => {
                                    self.app.status_message = Some(format!("Command error: {}", e));
                                }
                                other => {
                                    // Other CommandResult intents (OpenHelp, Exit, etc.)
                                    // are handled by the TUI intercept in app.rs.
                                    tracing::debug!("Deferred CommandResult variant: {:?}", other);
                                }
                            }
                            continue;
                        }
                    }

                    if let Some(ref agent) = agent {
                        // If a turn is currently streaming, push the input as
                        // a steer directive instead of starting a new turn.
                        // The agent drains steers at the next iteration boundary
                        // and injects them as user-role messages. (iter-93 —
                        // closes the /steer parity gap.)
                        if self.app.is_streaming
                            && let Some(ref handle) = self.app.steer_queue_handle
                        {
                            let mut q = handle.lock().await;
                            q.push(input.clone());
                            self.app.status_message = Some(format!(
                                "Steer queued: {}",
                                input.chars().take(60).collect::<String>()
                            ));
                            continue;
                        }
                        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                        self.app.messages.push(Message {
                            role: Role::User,
                            content: MessageContent::Text(input.clone()),
                        });
                        self.app.is_streaming = true;
                        self.app.streaming_text.clear();
                        self.app.streaming_thinking.clear();

                        let agent_clone = std::sync::Arc::clone(agent);
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let handle = tokio::spawn(async move {
                            let result = agent_clone.run(input).await.map(|_| ());
                            let _ = tx.send(result);
                        });
                        self.app.run_complete_rx = Some(rx);
                        self.app.agent_task_handle = Some(handle);
                    }
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };

        disable_raw_mode()?;
        if !self.no_mouse {
            let _ = execute!(
                terminal.backend_mut(),
                crossterm::event::DisableMouseCapture
            );
        }
        let _ = execute!(terminal.backend_mut(), crossterm::event::DisableFocusChange);
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        result
    }

    /// Push `text` as a user message and spawn an agent turn for it.
    /// Shared by the initial-query path, /retry, and the /skill + /bundle
    /// expansion so the submission wiring lives in exactly one place.
    /// (iter-320 — dedupe three near-identical submission blocks.)
    fn submit_user_message(
        &mut self,
        agent: &std::sync::Arc<operant_core::agent::OperantAgent>,
        text: String,
    ) {
        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
        self.app.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(text.clone()),
        });
        self.app.is_streaming = true;
        self.app.streaming_text.clear();
        self.app.streaming_thinking.clear();
        let agent_clone = std::sync::Arc::clone(agent);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let result = agent_clone.run(text).await.map(|_| ());
            let _ = tx.send(result);
        });
        self.app.run_complete_rx = Some(rx);
        self.app.agent_task_handle = Some(handle);
    }

    /// Run the real `App::run` loop headlessly against a `TestBackend`,
    /// replaying `keys`. Returns the captured event log, the final `App`
    /// state (for assertions), and the final rendered screen as trimmed
    /// text rows (for screen-content assertions and snapshots).
    pub async fn run_headless(
        mut self,
        keys: Vec<crossterm::event::KeyEvent>,
        agent_script: Option<Vec<operant_core::agent::AgentEvent>>,
        size: (u16, u16),
        max_frames: Option<u64>,
    ) -> anyhow::Result<(
        Vec<crate::tui::debug::TuiEvent>,
        crate::tui::app::App,
        Vec<String>,
    )> {
        let (agent_tx, agent_rx) =
            tokio::sync::mpsc::channel::<operant_core::agent::AgentEvent>(256);
        self.app.agent_event_rx = Some(agent_rx);

        let (permission_tx, permission_rx) =
            tokio::sync::mpsc::channel::<operant_core::agent::ToolPermissionRequest>(4);
        self.app.permission_rx = Some(permission_rx);

        let config = self.app.config.clone();
        let mcp_manager = operant_core::mcp::McpManager::new();
        let skills_dir = config.skills.root_dir.clone();

        let is_mock = agent_script.is_some();
        let agent: Option<std::sync::Arc<operant_core::agent::OperantAgent>> =
            if let Some(script) = agent_script {
                // Mock path: inject scripted AgentEvents through the real
                // agent_event_rx channel instead of spawning a network agent.
                // Events are buffered; is_streaming keeps the run loop alive to
                // process them; a pre-resolved run_complete oneshot guarantees
                // the loop terminates (is_streaming flips false) even if the
                // script omits a Done event. No network calls on this path.
                for ev in script {
                    let _ = agent_tx.try_send(ev);
                }
                drop(agent_tx);
                self.app.is_streaming = true;
                let (done_tx, done_rx) = tokio::sync::oneshot::channel();
                let _ = done_tx.send(Ok(()));
                self.app.run_complete_rx = Some(done_rx);
                None
            } else {
                match crate::create_runtime_agent(
                    &config,
                    &config.agent,
                    None,
                    agent_tx,
                    &mcp_manager,
                    &skills_dir,
                )
                .await
                {
                    Ok(agent) => Some(std::sync::Arc::new(agent.with_permissions(permission_tx))),
                    Err(e) => {
                        self.app.status_message = Some(format!("Agent init failed: {}", e));
                        None
                    }
                }
            };

        self.app.core_mcp_manager = Some(std::sync::Arc::new(mcp_manager));
        if let Some(ref agent) = agent {
            self.app.steer_queue_handle = Some(agent.steer_queue_handle());
            // Clone of the agent's live registry so /mcp reconnect can
            // materialize MCP tools mid-session via sync_tools_to_registry.
            self.app.core_tool_registry = Some(agent.registry());
        }

        let (uq_tx, uq_rx) = tokio::sync::mpsc::unbounded_channel::<
            operant_core::user_question::UserQuestionRequest,
        >();
        let _ = operant_core::user_question::set_user_question_sender(uq_tx);
        self.app.user_question_rx = Some(uq_rx);

        self.app.refresh_context_window_size();
        // Skip the network models.dev fetch on the deterministic mock path.
        if !is_mock {
            self.app.model_registry.load_models_dev().await;
        }

        if let Some(query) = self.initial_query.take()
            && let Some(ref agent) = agent
        {
            use crate::tui::adapter_types::types::{Message, MessageContent, Role};
            self.app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(query.clone()),
            });
            self.app.is_streaming = true;
            self.app.streaming_text.clear();
            self.app.streaming_thinking.clear();

            let agent_clone = std::sync::Arc::clone(agent);
            let (tx, rx) = tokio::sync::oneshot::channel();
            let handle = tokio::spawn(async move {
                let result = agent_clone.run(query).await.map(|_| ());
                let _ = tx.send(result);
            });
            self.app.run_complete_rx = Some(rx);
            self.app.agent_task_handle = Some(handle);
        }

        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (width, height) = size;
        let backend = TestBackend::new(width.max(20), height.max(5));
        let mut terminal = Terminal::new(backend)?;

        self.app.is_simulating = true;
        self.app.simulation_max_frames = max_frames;
        self.app.simulated_keys = keys;
        self.app.debug_hub.event_bus().set_enabled(true);

        loop {
            match self.app.run(&mut terminal) {
                Ok(Some(input)) => {
                    // Intercept slash commands exactly like the interactive
                    // run loop (see TuiApp::run) so the simulator is faithful:
                    // submitting "/help" opens the help overlay instead of
                    // being sent to the agent as a prompt. This also lets the
                    // SlashCommand debug event fire on the headless path.
                    if crate::input::is_slash_command(&input) {
                        let (cmd, args) = crate::input::parse_slash_command(&input);
                        if self.app.handle_tui_command(cmd, args) {
                            continue;
                        }
                    }
                    if let Some(ref agent) = agent {
                        if self.app.is_streaming
                            && let Some(ref handle) = self.app.steer_queue_handle
                        {
                            let mut q = handle.lock().await;
                            q.push(input.clone());
                            continue;
                        }
                        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                        self.app.messages.push(Message {
                            role: Role::User,
                            content: MessageContent::Text(input.clone()),
                        });
                        self.app.is_streaming = true;
                        self.app.streaming_text.clear();
                        self.app.streaming_thinking.clear();

                        let agent_clone = std::sync::Arc::clone(agent);
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let handle = tokio::spawn(async move {
                            let result = agent_clone.run(input).await.map(|_| ());
                            let _ = tx.send(result);
                        });
                        self.app.run_complete_rx = Some(rx);
                        self.app.agent_task_handle = Some(handle);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    self.app
                        .debug_hub
                        .record_error("headless_simulation", &e.to_string());
                    break;
                }
            }
            tokio::task::yield_now().await;
        }

        // The run loop's exit check fires at the top of the loop before the
        // draw, so the last key's effect (and, with zero keys, the landing
        // screen) is never painted. Do one final render of the terminal state
        // so the captured screen reflects the final App state after all keys.
        let _ = terminal.draw(|f| crate::tui::render::render_app(f, &self.app));

        // Capture the final rendered screen as trimmed text rows. The
        // TestBackend buffer is row-major; chunk the flat cell slice by width.
        let screen = {
            let buf = terminal.backend().buffer();
            let width = (buf.area.width as usize).max(1);
            buf.content()
                .chunks(width)
                .map(|row| {
                    row.iter()
                        .map(|c| c.symbol())
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<String>>()
        };
        let events = self.app.debug_hub.event_bus().recent(1000);
        Ok((events, self.app, screen))
    }
}

/// Suspend the TUI, run a shell command with inherited stdio, then resume.
///
/// Used by slash commands like `/setup` that need to launch an interactive
/// subprocess (the operant setup wizard, an editor, etc.). The terminal is
/// left in alt-screen + raw mode by the TUI; this function:
///   1. leaves alt screen + disables raw mode (restoring the user's terminal)
///   2. spawns the command with inherited stdin/stdout/stderr
///   3. waits for it to complete
///   4. re-enters alt screen + re-enables raw mode
///   5. forces a terminal resize detection + full redraw on the next frame
///
/// Errors are returned if any of the crossterm operations fail or the spawn
/// fails. A non-zero exit code from the subprocess is NOT an error (the user
/// may have hit Ctrl+C in the wizard); we surface it via the returned
/// `Ok(ExitStatus)` so the caller can decide whether to message the user.
fn run_suspended_shell_command(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    argv: &[String],
) -> anyhow::Result<std::process::ExitStatus> {
    use crossterm::execute;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use std::io::Write;

    if argv.is_empty() {
        anyhow::bail!("run_suspended_shell_command: empty argv");
    }

    // 1. Leave alt screen + disable raw mode.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    // Flush so the user sees the wizard's first prompt immediately.
    let _ = std::io::stdout().flush();

    // 2. Spawn the subprocess with inherited stdio.
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let result = cmd.spawn()?.wait();

    // 3. Re-enter alt screen + re-enable raw mode regardless of subprocess
    //    outcome. If we skip this, the TUI is permanently broken.
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    // Force ratatui to forget its cached buffer sizes — the terminal may have
    // been resized while we were suspended. ratatui::Terminal::resize takes a
    // Rect; we read the current size and convert.
    let size = terminal.size()?;
    let _ = terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height));

    let status = result?;
    Ok(status)
}

#[derive(Debug, Clone)]
pub enum LaunchMode {
    Landing,
    Query(String),
}
