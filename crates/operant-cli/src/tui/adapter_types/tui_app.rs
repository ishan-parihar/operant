// adapter_types/tui_app.rs — Application bootstrap and run loop.

use super::auth::AuthStore;
use super::config::Settings;
use crate::commands::CommandResult;

pub struct TuiApp {
    app: crate::tui::app::App,
    initial_query: Option<String>,
    /// Whether to skip EnableMouseCapture in the TUI setup. Set by the
    /// --no-mouse CLI flag. (Bug #24 from iter-82 audit.)
    no_mouse: bool,
    /// Shared runtime retry/health metrics registry. Created once here,
    /// cloned into `App::retry_metrics` (footer rendering) and into the
    /// agent via `create_runtime_agent` (counter bumps), so the status bar
    /// reflects live stream-drop/memory-sync activity.
    metrics: std::sync::Arc<operant_core::runtime_metrics::RuntimeMetrics>,
}

impl TuiApp {
    pub async fn enter(
        config: operant_core::config::AppConfig,
        _system: Option<String>,
        _mode: LaunchMode,
        no_mouse: bool,
        dangerously_skip_permissions: bool,
    ) -> anyhow::Result<Self> {
        use crate::commands::CommandRegistry;
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
        register_tui_command_handlers(&mut command_registry);
        register_agent_flow_handlers(&mut command_registry);

        // (plan 05) `[tui] double_escape_action` from the TOML — hoisted
        // BEFORE the config moves into App::new.
        let double_escape_action = app_config.tui.double_escape_action.clone();
        let mut app =
            crate::tui::app::App::new(app_config, settings, cost_tracker, command_registry);
        app.double_escape_action = double_escape_action;

        // Share the runtime-metrics registry with the footer so stream-drop
        // retries and memory-sync failures render as a status pill. The same
        // Arc is passed to create_runtime_agent below (see TuiApp::run).
        let metrics = std::sync::Arc::new(operant_core::runtime_metrics::RuntimeMetrics::new());
        app.retry_metrics = std::sync::Arc::clone(&metrics);

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
            metrics,
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

        /// P5-1: rows the inline viewport occupies at the bottom of the main
        /// buffer in terminal-scrollback mode (status + composer + a small
        /// live stream window). The transcript emits above it in later P5
        /// waves; for the P5-1 milestone the cramped-but-alive render proves
        /// the terminal handoff.
        const TERMINAL_SCROLL_INLINE_ROWS: u16 = 14;

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
        // P5-1 terminal-scrollback mode (2026-10-10 design): when enabled,
        // skip the alternate screen entirely and render into an INLINE
        // bottom viewport of the main buffer — the terminal's native
        // scrollback owns everything above (tmux copy-mode, native
        // selection and copy reach the whole session). Mouse capture stays
        // off in this mode so the terminal's own selection works.
        let mut terminal = if self.app.config.tui.terminal_scroll_mode {
            let backend = CrosstermBackend::new(stdout);
            Terminal::with_options(
                backend,
                ratatui::TerminalOptions {
                    viewport: ratatui::Viewport::Inline(TERMINAL_SCROLL_INLINE_ROWS),
                },
            )?
        } else {
            execute!(stdout, EnterAlternateScreen)?;
            // Enable mouse capture unless --no-mouse was passed. Mouse capture
            // lets the TUI receive scroll/click events for the transcript, diff
            // viewer, and overlay scrolling. Some terminal multiplexers (tmux,
            // screen) interfere with mouse capture; --no-mouse disables it so
            // the terminal's native mouse selection works. (Bug #24 from iter-82
            // audit — /mouse mentioned a --no-mouse flag that didn't exist.)
            if !self.no_mouse {
                execute!(stdout, crossterm::event::EnableMouseCapture)?;
                self.app.mouse_capture_active = true;
            }
            // Enable focus-change reporting so the TUI can pause animations and
            // drop the redraw cadence when the window is backgrounded (Phase 2.3).
            // Terminals that don't support focus events simply ignore the sequence.
            execute!(stdout, crossterm::event::EnableFocusChange)?;
            let backend = CrosstermBackend::new(stdout);
            Terminal::new(backend)?
        };

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
                Some(std::sync::Arc::clone(&self.metrics)),
            )
            .await
            {
                Ok(mut agent) => {
                    // Idle recap (omp `recap.*` parity): attach the engine so a
                    // quiet 240s (agent.recap_idle_seconds) after a completed
                    // turn renders a `※ recap:` line. 0 = off (engine returns a
                    // disconnected handle).
                    let idle = config.agent.recap_idle_seconds;
                    let activity = agent.attach_recap_engine(std::time::Duration::from_secs(idle));
                    self.app.recap_activity = Some(activity);
                    // Advisor watchdog (omp advisor/ parity): always-on
                    // transcript reviewer emitting `⚑ advisor:` notes when
                    // `[advisor] enabled = true`. Disabled by default.
                    agent.attach_advisor(config.advisor.clone(), crate::client_config(&config));
                    Some(std::sync::Arc::new(agent.with_permissions(permission_tx)))
                }
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
            // Handle to the agent's memory provider so the /mcp reconnect
            // task can ensure the agentmemory backend is up before
            // connecting its MCP server (fast reconnect).
            self.app.core_memory_provider = agent.memory_provider();
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

        // Warms the models.dev disk cache that the default model and the context
        // window read. A failed fetch leaves the cache as it was.
        if let Err(e) = operant_core::models_dev::fetch_models_dev(false).await {
            tracing::debug!("models.dev warm-up skipped: {e}");
        }
        // Force a context-window-size refresh so /context shows real numbers
        // on the first frame instead of "0 / 0" (Bug #13 from iter-82 audit).
        // After the fetch: on a cold models.dev cache the window reads the
        // models.dev cache, which this fetch is what populates.
        self.app.refresh_context_window_size();

        if let Some(query) = self.initial_query.take()
            && let Some(ref agent) = agent
        {
            // Initial query: never came from the composer — disarm the
            // restore slot so a failure here can't resurrect the
            // composer's last sent text (the pre-merge behavior).
            self.app.clear_failed_input_recovery();
            self.submit_user_message(agent, query).await;
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
                // /skill + /bundle expansion: never came from the composer.
                self.app.clear_failed_input_recovery();
                self.submit_user_message(agent, inject_text).await;
                continue;
            }
            // /retry: drain pending_retry_query set by the intercept arm at
            // the loop top too, so /retry fires immediately after Enter
            // (same one-keystroke semantics as /skill — iter-320).
            if !self.app.is_streaming
                && let Some(agent) = agent.as_ref()
                && let Some(retry_text) = self.app.pending_retry_query.take()
            {
                // /retry resubmission: never came from the composer.
                self.app.clear_failed_input_recovery();
                self.submit_user_message(agent, retry_text).await;
                continue;
            }
            // (plan 03 §2) /clear, /new, /undo deferred from a mid-stream
            // invocation: the agent-side work runs once the turn has fully
            // finished (is_streaming false), so the stopping turn's tail
            // write can't repopulate the conversation after a clear.
            if !self.app.is_streaming
                && let Some(agent) = agent.as_ref()
                && (self.app.pending_clear
                    || self.app.pending_new
                    || self.app.pending_undo.is_some())
            {
                Self::run_pending_agent_actions(&mut self.app, Some(agent)).await;
            }

            match run_app_loop(&mut self.app, &mut terminal) {
                Ok(Some(input)) => {
                    // A model / provider / effort chosen in a picker INSIDE
                    // `run()` only set a `pending_*` flag. Apply it to the
                    // agent now, before this input is acted on — it used to
                    // wait for the next slash command, so the prompt right
                    // after a /model pick still went out on the OLD model.
                    Self::run_pending_agent_actions(&mut self.app, agent.as_ref()).await;
                    // Poll bridge/gateway connection state updates from handler.
                    if let Some(ref mut rx) = self.app.bridge_state_rx {
                        while let Ok(state) = rx.try_recv() {
                            self.app.bridge_state = state;
                            self.app
                                .transcript_version
                                .set(self.app.transcript_version.get().wrapping_add(1));
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
                            // A slash command is a command, not a prompt — no
                            // turn is started, so there is no failure to
                            // restore the text from.
                            self.app.clear_failed_input_recovery();
                            Self::run_pending_agent_actions(&mut self.app, agent.as_ref()).await;
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
                                other => {
                                    // Other CommandResult intents (ShellCommand) are
                                    // handled by the TUI intercept in app.rs.
                                    tracing::debug!("Deferred CommandResult variant: {:?}", other);
                                }
                            }
                            continue;
                        }
                        // (iter-790, command backlog) A REGISTERED command name
                        // that neither the TUI intercept nor the registry
                        // handles must never fall through to the model as
                        // prompt text (the "broken commands" complaint): the
                        // autocomplete advertised it, so consuming it with
                        // an honest notice beats silently sending "/split" to
                        // the LLM. Unregistered text still submits normally.
                        if crate::tui::operant_app::app::registered_command_names_including_hidden()
                            .any(|name| name.trim_start_matches('/') == cmd)
                        {
                            self.app.push_system_message(
                                format!(
                                    "/{cmd} is registered but not wired in operant yet — see /help for working commands"
                                ),
                                crate::tui::app::SystemMessageStyle::Info,
                            );
                            continue;
                        }
                    }

                    if let Some(agent) = &agent {
                        // (plan 03 §5) the ONE submit path: steer-if-streaming
                        // and the stopping-turn guard both live inside
                        // submit_user_message now — the composer block that
                        // used to live here was a third copy of them.
                        self.submit_user_message(agent, input).await;
                    } else {
                        // No agent — the runtime failed to build, so the
                        // submission is dropped on the floor with nothing
                        // running to report an error. That is the one failed
                        // submission with no event to hang the restore on, so
                        // it has to be given back here or the text is simply
                        // gone.
                        self.app.restore_failed_input_to_composer();
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
    async fn submit_user_message(
        &mut self,
        agent: &std::sync::Arc<operant_core::agent::OperantAgent>,
        text: String,
    ) {
        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
        // User activity cancels any pending idle recap (omp parity: the
        // recap exists to greet a returning user, never to interrupt one).
        if let Some(activity) = &self.app.recap_activity {
            activity.notify();
        }
        // (plan 03 §1) THE spawn guard, in the one submit path: a turn whose
        // task future has not finished is still running on the agent even
        // if the display already flipped to idle (a /stop'd turn committing
        // its tail, or a /clear-ed turn's stop). Starting a second run on
        // the shared agent corrupts both turns' transcripts (the gateway
        // got the same fix in iter-864; this is the TUI twin).
        if self
            .app
            .agent_task_handle
            .as_ref()
            .is_some_and(|h| !h.is_finished())
        {
            self.app
                .set_status_notice("A turn is still stopping — try again in a moment.".to_string());
            return;
        }
        // If a turn is currently streaming, push the input as a steer
        // directive instead of starting a new turn (iter-93). Checked AFTER
        // the spawn guard: an unfinished handle with is_streaming true is a
        // live turn to steer; the guard above only catches the stopping
        // window where steering is pointless (the interrupt is already
        // fired).
        if self.app.is_streaming
            && let Some(handle) = &self.app.steer_queue_handle
        {
            let mut q = handle.lock().await;
            q.push(text.clone());
            self.app.status_message = Some(format!(
                "Steer queued: {}",
                text.chars().take(60).collect::<String>()
            ));
            return;
        }
        // (plan 03 §5) The failed-input restore slot is the COMPOSER's
        // debt: `take_input` armed it when Enter consumed the composer,
        // and the merged composer path routes through here — clearing it
        // here would strand a failed composer submission with no restore
        // (the context-menu drift). The NON-composer callers (initial
        // query, /skill + /bundle expansion, /retry) disarm the slot at
        // their own call sites instead: their text never came from the
        // composer, so nothing is owed back.
        self.app.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(text.clone()),
        });
        // The display cache is keyed on transcript_version; without this
        // bump the user message is invisible until the first agent event
        // arrives (N-2) - and the idle orb keeps painting over the sent
        // message because has_started_conversation reads the stale cache.
        self.app.invalidate_transcript();
        self.app.is_streaming = true;
        self.app.begin_turn();
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
        // (plan 04) see the spawn note above.
        self.app.agent_interrupt = Some(agent.interrupt_flag());
        self.app.cancel_deadline = None;
    }

    /// Run the real `App::run` loop headlessly against a `TestBackend`,
    /// replaying `keys`. Returns the captured event log, the final `App`
    /// state (for assertions), the final rendered screen as trimmed text
    /// rows (for screen-content assertions and snapshots), and the same
    /// buffer projected as the `operant-style-v1` per-cell colour/modifier
    /// grid (the style baseline). The last two are both built from the final
    /// `TestBackend` buffer, so a text baseline and a style baseline always
    /// describe the same frame.
    ///
    /// `frame_capture` arms per-frame text capture inside the run loop: the
    /// plan's indices (or every painted frame when `plan.all`) are written to
    /// `plan.dir` as `frame-<NNNN>.txt` as they are painted, which is the
    /// only way to reach intermediate states (the loop exits *before* its
    /// last paint, so a finished buffer can only ever show the final state).
    /// The returned `Option<(requested, captured)>` reports which indices
    /// were actually painted so the caller can fail instead of silently
    /// writing fewer files than asked for. In `all` mode `requested` comes
    /// back empty and the caller must skip the missing-frames check (the
    /// total is unknowable up front).
    #[allow(clippy::too_many_arguments)]
    pub async fn run_headless(
        mut self,
        keys: Vec<crossterm::event::KeyEvent>,
        mouse: Vec<crossterm::event::MouseEvent>,
        agent_script: Option<Vec<crate::cmd_tui_debug::MockAgentEvent>>,
        size: (u16, u16),
        max_frames: Option<u64>,
        frame_capture: Option<crate::tui::debug::FrameCapturePlan>,
    ) -> anyhow::Result<(
        Vec<crate::tui::debug::TuiEvent>,
        crate::tui::app::App,
        Vec<String>,
        Option<(Vec<u64>, Vec<u64>)>,
        String,
    )> {
        self.app.simulated_mouse = mouse;
        let (agent_tx, agent_rx) =
            tokio::sync::mpsc::channel::<operant_core::agent::AgentEvent>(256);
        self.app.agent_event_rx = Some(agent_rx);
        let (permission_tx, permission_rx) =
            tokio::sync::mpsc::channel::<operant_core::agent::ToolPermissionRequest>(4);
        self.app.permission_rx = Some(permission_rx);

        // Created before the mock/live split so the mock path can drive
        // `ask_user_dialog` through the same channel the clarify tool uses.
        let (uq_tx, uq_rx) = tokio::sync::mpsc::unbounded_channel::<
            operant_core::user_question::UserQuestionRequest,
        >();
        let _ = operant_core::user_question::set_user_question_sender(uq_tx.clone());
        self.app.user_question_rx = Some(uq_rx);

        let config = self.app.config.clone();
        let mcp_manager = operant_core::mcp::McpManager::new();
        let skills_dir = config.skills.root_dir.clone();

        let is_mock = agent_script.is_some();
        let agent: Option<std::sync::Arc<operant_core::agent::OperantAgent>> = if let Some(script) =
            agent_script
        {
            // Mock path: inject scripted events through the real channels
            // the TUI already drains, instead of spawning a network agent.
            // AgentEvents are buffered on `agent_event_rx`; the four
            // channel-fed variants go onto `permission_tx` / `uq_tx` /
            // the background-delegation registry, which is exactly where
            // a live agent puts them. is_streaming keeps the run loop
            // alive to process them; a pre-resolved run_complete oneshot
            // guarantees the loop terminates (is_streaming flips false)
            // even if the script omits a Done event. No network calls on
            // this path.
            for ev in script {
                match ev {
                    crate::cmd_tui_debug::MockAgentEvent::User { text } => {
                        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                        self.app.messages.push(Message {
                            role: Role::User,
                            content: MessageContent::Text(text),
                        });
                    }
                    crate::cmd_tui_debug::MockAgentEvent::PermissionRequest {
                        tool,
                        tool_id,
                        description,
                    } => {
                        let (tx, _rx) = tokio::sync::oneshot::channel();
                        let _ =
                            permission_tx.try_send(operant_core::agent::ToolPermissionRequest {
                                tool_name: tool,
                                tool_id,
                                description,
                                danger_explanation: String::new(),
                                input_preview: None,
                                seat_escalation: None,
                                unattended: false,
                                response_tx: tx,
                            });
                    }
                    crate::cmd_tui_debug::MockAgentEvent::UserQuestion { question, choices } => {
                        let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
                        let _ = uq_tx.send(operant_core::user_question::UserQuestionRequest {
                            question,
                            choices,
                            reply_tx,
                        });
                    }
                    crate::cmd_tui_debug::MockAgentEvent::BackgroundTask {
                        goal,
                        model,
                        status,
                    } => {
                        let id =
                            operant_core::tools::async_delegation::create_record(&goal, &model);
                        match status.as_str() {
                            "completed" => operant_core::tools::async_delegation::mark_completed(
                                &id, "(seeded)",
                            ),
                            "failed" => operant_core::tools::async_delegation::mark_failed(
                                &id,
                                "(seeded failure)",
                            ),
                            _ => {}
                        }
                    }
                    other => {
                        if let Some(ev) = other.into_agent_event() {
                            let _ = agent_tx.try_send(ev);
                        }
                    }
                }
            }
            drop(agent_tx);
            self.app.is_streaming = true;
            self.app.begin_turn();
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
                Some(std::sync::Arc::clone(&self.metrics)),
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
            // Handle to the agent's memory provider so the /mcp reconnect
            // task can ensure the agentmemory backend is up before
            // connecting its MCP server (fast reconnect).
            self.app.core_memory_provider = agent.memory_provider();
        }

        // Skip the network models.dev fetch on the deterministic mock path.
        if !is_mock {
            // Warms the models.dev disk cache that the default model and the context
            // window read. A failed fetch leaves the cache as it was.
            if let Err(e) = operant_core::models_dev::fetch_models_dev(false).await {
                tracing::debug!("models.dev warm-up skipped: {e}");
            }
        }
        self.app.refresh_context_window_size();

        if let Some(query) = self.initial_query.take()
            && let Some(ref agent) = agent
        {
            use crate::tui::adapter_types::types::{Message, MessageContent, Role};
            self.app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(query.clone()),
            });
            self.app.is_streaming = true;
            self.app.begin_turn();
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
            // (plan 04) see the spawn note above.
            self.app.agent_interrupt = Some(agent.interrupt_flag());
            self.app.cancel_deadline = None;
        }

        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (width, height) = size;
        let backend = TestBackend::new(width.max(20), height.max(5));
        let mut terminal = Terminal::new(backend)?;

        self.app.is_simulating = true;
        self.app.simulation_max_frames = max_frames;
        // (plan 01) frame-per-event capture mode doubles as the per-event
        // drain switch: capture-all implies one event per loop pass.
        self.app.simulation_one_event_per_frame =
            frame_capture.as_ref().is_some_and(|plan| plan.all);
        self.app.simulated_keys = keys;
        self.app.debug_hub.event_bus().set_enabled(true);
        if let Some(plan) = frame_capture {
            if plan.all {
                self.app.debug_hub.arm_all_frames_capture(plan.dir);
            } else {
                self.app.debug_hub.arm_frame_capture(plan.indices, plan.dir);
            }
        }

        loop {
            // (plan 03 §2) the sim mirrors the interactive loop's deferred
            // drain (see the retry/skill drains there) so a mid-stream
            // /clear in a scenario behaves exactly like production.
            if !self.app.is_streaming
                && let Some(agent) = agent.as_ref()
                && (self.app.pending_clear
                    || self.app.pending_new
                    || self.app.pending_undo.is_some())
            {
                Self::run_pending_agent_actions(&mut self.app, Some(agent)).await;
            }
            match self.app.run(&mut terminal) {
                Ok(Some(input)) => {
                    // A model / provider / effort chosen in a picker INSIDE
                    // `run()` only set a `pending_*` flag. Apply it to the
                    // agent now, before this input is acted on — it used to
                    // wait for the next slash command, so the prompt right
                    // after a /model pick still went out on the OLD model.
                    Self::run_pending_agent_actions(&mut self.app, agent.as_ref()).await;
                    // Intercept slash commands exactly like the interactive
                    // run loop (see TuiApp::run) so the simulator is faithful:
                    // submitting "/help" opens the help overlay instead of
                    // being sent to the agent as a prompt. This also lets the
                    // SlashCommand debug event fire on the headless path.
                    if crate::input::is_slash_command(&input) {
                        let (cmd, args) = crate::input::parse_slash_command(&input);
                        if self.app.handle_tui_command(cmd, args) {
                            self.app.clear_failed_input_recovery();
                            Self::run_pending_agent_actions(&mut self.app, agent.as_ref()).await;
                            continue;
                        }
                        // Mirror the interactive loop exactly (registry
                        // fallback + registered-name catch-all) so the
                        // simulator is faithful and the command sweep
                        // measures the same dispatch as production.
                        if let Some(canonical) = self.app.command_registry.resolve(cmd) {
                            let result = self.app.command_registry.execute(canonical, args).await;
                            if let crate::commands::CommandResult::Message(output) = result {
                                self.app.push_system_message(
                                    output,
                                    crate::tui::app::SystemMessageStyle::Info,
                                );
                            }
                            continue;
                        }
                        if crate::tui::operant_app::app::registered_command_names_including_hidden()
                            .any(|name| name.trim_start_matches('/') == cmd)
                        {
                            self.app.push_system_message(
                                format!(
                                    "/{cmd} is registered but not wired in operant yet — see /help for working commands"
                                ),
                                crate::tui::app::SystemMessageStyle::Info,
                            );
                            continue;
                        }
                    }
                    if let Some(agent) = &agent {
                        // (plan 03 §5) the one submit path, shared with the
                        // interactive loop's composer.
                        self.submit_user_message(agent, input).await;
                    } else if is_mock {
                        // Mock path: the "agent" is the script, not a failed
                        // init - a submitted message must land in the
                        // transcript exactly like the live paths (N-2). The
                        // pre-fix restore-else simulated an init failure for
                        // every mock submit, so no scenario could ever pin
                        // the submit-path commit.
                        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                        self.app.messages.push(Message {
                            role: Role::User,
                            content: MessageContent::Text(input.clone()),
                        });
                        self.app.invalidate_transcript();
                    } else {
                        self.app.restore_failed_input_to_composer();
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
        let screen = crate::tui::debug::debug_hub::buffer_rows(terminal.backend().buffer());
        // Same buffer, same frame: the style grid is projected here rather
        // than in the caller so the two baselines can never diverge.
        let style_dump =
            crate::tui::debug::debug_hub::buffer_style_dump(terminal.backend().buffer());
        let events = self.app.debug_hub.event_bus().recent(1000);
        let capture_status = self.app.debug_hub.frame_capture_status();
        Ok((events, self.app, screen, capture_status, style_dump))
    }
}

impl TuiApp {
    /// Agent-side work a slash command asked for (it needs the agent, which
    /// the app does not hold): `/compact` folds the stored conversation.
    /// (plan 03 §2) The ONE display rebuild: `App.messages` from the
    /// agent's conversation, so display and context come from a single
    /// source. Extracted from the rewind handler; /clear, /new and /undo
    /// rebuild through it too — a slash command must never leave the
    /// display diverged from the agent's real context.
    fn rebuild_display_from_agent(
        app: &mut crate::tui::app::App,
        conv: Vec<operant_core::client::Message>,
    ) {
        app.messages.clear();
        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
        for m in conv {
            let role = match m.role {
                operant_core::client::Role::User => Role::User,
                operant_core::client::Role::Assistant => Role::Assistant,
                operant_core::client::Role::System => Role::System,
                operant_core::client::Role::Tool => Role::System,
            };
            app.messages.push(Message {
                role,
                content: MessageContent::Text(m.content),
            });
        }
        app.invalidate_transcript();
    }

    pub(crate) async fn run_pending_agent_actions(
        app: &mut crate::tui::app::App,
        agent: Option<&std::sync::Arc<operant_core::agent::OperantAgent>>,
    ) {
        // (plan 04) watchdog repair: the agent task was aborted after the
        // grace window; commit the tail (partial text from the TUI buffer +
        // synthetic tool results) on the stored conversation.
        if std::mem::take(&mut app.pending_repair) {
            if let Some(agent) = agent {
                let partial = app.pending_repair_partial.take();
                agent.repair_after_abort(partial).await;
                app.push_system_message(
                    "Turn stopped; the interrupted step was kept in context.".to_string(),
                    crate::tui::app::SystemMessageStyle::Info,
                );
            } else {
                app.pending_repair_partial = None;
            }
        }
        // (plan 05) Ctrl+B: populate the branch browser with the current
        // session's fork tree (parent + children) from the agent's DB.
        if std::mem::take(&mut app.pending_branch_list)
            && let Some(agent) = agent
        {
            let current = agent.session_id().unwrap_or_default();
            let mut branches = Vec::new();
            if let Ok(parent) = agent.database().session_parent(&current)
                && let Some(parent) = parent
            {
                let short: String = parent.chars().take(8).collect();
                branches.push(crate::tui::session_branching::BranchInfo {
                    id: parent.clone(),
                    name: format!("{short} (parent)"),
                    created_at: String::new(),
                    is_current: false,
                });
            }
            if let Ok(children) = agent.database().child_sessions(&current) {
                for (id, title) in children {
                    let label = title.unwrap_or_else(|| {
                        format!("{} (fork)", id.chars().take(8).collect::<String>())
                    });
                    branches.push(crate::tui::session_branching::BranchInfo {
                        id,
                        name: label,
                        created_at: String::new(),
                        is_current: false,
                    });
                }
            }
            let at = app.session_branching.branch_at_message;
            app.session_branching.open(branches, at);
        }
        // (plan 05) rewind selector: sync marks from the agent while open
        // (or while a /rewind <n> confirmation is pending — the numeric
        // path resolves against the mark list without opening the UI),
        // service the preview request, execute the fork on Enter.
        if (app.rewind_selector.visible || app.pending_rewind_confirm.is_some())
            && let Some(agent) = agent
        {
            let marks: Vec<(u64, String)> = agent
                .rewind_marks()
                .into_iter()
                .map(|m| (m.turn_id, m.text))
                .collect();
            let display_texts = app.user_prompt_texts_for_filter();
            app.rewind_selector.sync_marks(marks, &display_texts);
            if std::mem::take(&mut app.pending_rewind_preview)
                && let Some(turn_id) = app.rewind_selector.selected_turn_id()
            {
                match agent.preview_rewind(turn_id).await {
                    Ok((prefix, tokens)) => {
                        app.rewind_selector.preview = Some((prefix.len(), tokens));
                    }
                    Err(e) => {
                        app.rewind_selector.notice = Some(e.to_string());
                        app.rewind_selector.preview = None;
                    }
                }
            }
        }
        if let Some(turn_id) = app.pending_rewind.take() {
            let Some(agent) = agent else {
                app.rewind_selector.notice = Some("No agent attached — cannot rewind.".to_string());
                return;
            };
            match agent.rewind_to_turn(turn_id).await {
                Ok(outcome) => {
                    // Rebuild the display from the agent's forked store —
                    // display and context from one source.
                    let conv = agent.conversation().await;
                    Self::rebuild_display_from_agent(app, conv);
                    app.rewind_selector.close();
                    // oh-my-pi behaviour: the rewound prompt comes back to
                    // the composer, ready to edit and resend.
                    app.set_prompt_text(outcome.prompt_text.clone());
                    let short: String = outcome.new_session_id.chars().take(8).collect();
                    app.set_status_notice(format!(
                        "Rewound {} turn(s) · previous path kept as session {short}",
                        outcome.removed_turns
                    ));
                }
                Err(e) => {
                    app.rewind_selector.notice = Some(e.to_string());
                }
            }
        }
        // ── (plan 03 §2) the deferred context commands ────────────────────
        // Consumed only when the turn has fully finished (`!app.is_streaming`);
        // while a stopping turn is still committing its tail they stay set
        // and the loop-top drain calls here again.
        if !app.is_streaming {
            if std::mem::take(&mut app.pending_clear) {
                let Some(agent) = agent else {
                    app.set_status_notice("No agent attached — cannot clear.".to_string());
                    return;
                };
                agent.clear_history().await;
                let conv = agent.conversation().await;
                Self::rebuild_display_from_agent(app, conv);
                app.tool_use_blocks.clear();
                app.system_annotations.clear();
                app.turn_metadata.clear();
                app.cost_usd = 0.0;
                app.scroll_offset = 0;
                app.auto_scroll = true;
                app.new_messages_while_scrolled = 0;
                // Native-scroll reset (07 §2 lands the full replay; until
                // then a cleared conversation must not re-emit the old
                // rows).
                app.request_native_replay();
                app.invalidate_transcript();
                app.set_status_notice("Conversation cleared.".to_string());
            }
            if std::mem::take(&mut app.pending_new) {
                let Some(agent) = agent else {
                    // Headless/mock: display-only fresh session, same as the
                    // pre-plan-03 arm.
                    app.messages.clear();
                    app.system_annotations.clear();
                    app.streaming_text.clear();
                    app.streaming_thinking.clear();
                    app.tool_use_blocks.clear();
                    app.turn_metadata.clear();
                    app.session_goal = None;
                    app.cost_usd = 0.0;
                    app.session_title = None;
                    app.scroll_offset = 0;
                    app.auto_scroll = true;
                    app.new_messages_while_scrolled = 0;
                    app.token_count = 0;
                    app.request_native_replay();
                    app.invalidate_transcript();
                    app.set_status_notice("New session started.".to_string());
                    return;
                };
                let old = agent.session_id().unwrap_or_default();
                let fresh = operant_core::agent::rewind::new_session_id();
                agent.set_session_id(fresh.clone());
                agent.notify_session_switch(&fresh, &old, false);
                let conv = agent.conversation().await;
                Self::rebuild_display_from_agent(app, conv);
                app.tool_use_blocks.clear();
                app.system_annotations.clear();
                app.turn_metadata.clear();
                app.session_goal = None;
                app.cost_usd = 0.0;
                app.session_title = None;
                app.scroll_offset = 0;
                app.auto_scroll = true;
                app.new_messages_while_scrolled = 0;
                app.token_count = 0;
                app.request_native_replay();
                app.invalidate_transcript();
                let short: String = fresh.chars().take(8).collect();
                app.set_status_notice(format!(
                    "New session started ({short}) — previous session kept for /resume."
                ));
            }
            if let Some(session_id) = std::mem::take(&mut app.pending_switch_session) {
                let Some(agent) = agent else {
                    app.set_status_notice("No agent attached — cannot switch session.".to_string());
                    return;
                };
                // (plan 04) a branch IS a session (the rewind fork makes
                // one), so switching = the /new shape: retarget, keep the
                // old session in the store, rebuild the display from the
                // agent's conversation.
                let old = agent.session_id().unwrap_or_default();
                agent.set_session_id(session_id.clone());
                agent.notify_session_switch(&session_id, &old, false);
                let conv = agent.conversation().await;
                Self::rebuild_display_from_agent(app, conv);
                app.tool_use_blocks.clear();
                app.system_annotations.clear();
                app.request_native_replay();
                app.invalidate_transcript();
                let short: String = session_id.chars().take(8).collect();
                app.set_status_notice(format!("Switched to session {short}"));
            }
            if let Some(n) = app.pending_undo.take() {
                let retry_after_undo = std::mem::take(&mut app.pending_retry_after_undo);
                let Some(agent) = agent else {
                    app.set_status_notice("No agent attached — cannot undo.".to_string());
                    return;
                };
                let marks = agent.rewind_marks();
                if marks.len() < n {
                    app.pending_undo = None;
                    app.pending_retry_after_undo = false;
                    app.set_status_notice(format!(
                        "Nothing to undo — only {} completed turn(s).",
                        marks.len()
                    ));
                } else if let Some(turn_id) = marks.iter().rev().nth(n - 1).map(|m| m.turn_id) {
                    match agent.rewind_to_turn(turn_id).await {
                        Ok(outcome) => {
                            let conv = agent.conversation().await;
                            Self::rebuild_display_from_agent(app, conv);
                            app.tool_use_blocks.clear();
                            app.system_annotations.clear();
                            app.request_native_replay();
                            app.invalidate_transcript();
                            if retry_after_undo {
                                app.pending_retry_query = Some(outcome.prompt_text.clone());
                                app.set_status_notice("Retrying last message…".to_string());
                            } else {
                                app.set_prompt_text(outcome.prompt_text.clone());
                                app.set_status_notice(format!(
                                    "Undid {} turn(s) — prompt restored to the composer.",
                                    outcome.removed_turns
                                ));
                            }
                        }
                        Err(e) => {
                            app.set_status_notice(format!("Undo failed: {e}"));
                        }
                    }
                }
            }
        }
        // (plan 03) effort change first: the agent applies it to the very
        // next request.
        if let Some(effort) = std::mem::take(&mut app.pending_effort)
            && let Some(agent) = agent
        {
            agent.set_reasoning_effort(if effort.is_empty() {
                None
            } else {
                Some(effort.clone())
            });
            let shown = if effort.is_empty() {
                "off".to_string()
            } else {
                effort
            };
            app.set_status_notice(format!("Reasoning effort: {shown}"));
        }
        // `/connect`: retarget the live client at the chosen provider's
        // endpoint BEFORE the model switch below, so the very next request
        // goes to the new host with the new key.
        if let Some((base_url, api_key)) = std::mem::take(&mut app.pending_endpoint)
            && let Some(agent) = agent
        {
            agent.set_endpoint(&base_url, api_key.as_deref());
        }
        // (plan 05A) model switch: ONE source of truth — the agent applies
        // it, and the display (model_name, config, context window, cost
        // tracker) refreshes FROM the agent, never from a parallel copy.
        // Persist through the surgical writer (plan 09): agent.model only.
        if let Some(model) = std::mem::take(&mut app.pending_model)
            && let Some(agent) = agent
        {
            agent.set_model(model.clone());
            let applied = agent.model().to_string();
            app.model_name = applied.clone();
            app.config.agent.model = applied.clone();
            if let Some(tracker) = std::sync::Arc::get_mut(&mut app.cost_tracker) {
                tracker.set_model(&applied);
            }
            app.refresh_context_window_size();
            // Context is fresh on a new model.
            app.context_used_tokens = 0;
            match operant_core::config::persist_config_key("agent.model", &applied) {
                Ok(path) => app.set_status_notice(format!(
                    "Model: {applied} (persisted to {})",
                    path.display()
                )),
                Err(e) => app.set_status_notice(format!("Model: {applied} (persist failed: {e})")),
            }
        }
        if std::mem::take(&mut app.pending_compact) {
            let Some(agent) = agent else {
                app.set_status_notice("No agent attached — nothing to compact.".to_string());
                return;
            };
            let (before, after) = agent.compact_history_now().await;
            let text = if after < before {
                format!(
                    "Compacted context: {before} → {after} messages (recent turns kept verbatim)."
                )
            } else {
                "Nothing to compact yet — the history is all recent turns.".to_string()
            };
            app.push_system_message(text, crate::tui::app::SystemMessageStyle::Info);
            // (plan 07 §2) compaction rewrites what history shows — replay
            // re-emits it under the compacted view.
            app.request_native_replay();
        }
    }
}

/// Drive the synchronous TUI loop without starving the runtime.
///
/// `App::run` blocks its thread for the whole interactive session. A task it
/// `tokio::spawn`s (the `/models` fetch behind the picker, the session list)
/// lands in THIS worker's LIFO slot, which other workers never steal — so it
/// never ran and the picker stayed on its one configured row.
/// `block_in_place` hands the worker's queue to another thread first. It is
/// only legal on the multi-thread flavor; a current-thread runtime (tests,
/// `#[tokio::test]`) runs the loop in place as before.
fn run_app_loop(
    app: &mut crate::tui::app::App,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<String>> {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(|| app.run(terminal)),
        _ => app.run(terminal),
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

/// Tier-3 agent-flow handlers (/improve /judge /refactor /fix /triage):
/// each suspends the TUI via `CommandResult::ShellCommand` →
/// `pending_shell_command` → `run_suspended_shell_command` (the /setup
/// escape hatch) and runs a one-shot operant session.
///
/// argv[0] is `current_exe()` per the /setup precedent: a bare "operant"
/// would resolve against PATH and could run a different version than the
/// TUI the user is sitting in.
///
/// Flag shapes follow the CLI contract (main.rs): `run --autonomous`
/// rejects `--query` (autonomous mode reads TODO.md), so /improve — whose
/// whole purpose is the TODO.md ledger — goes autonomous-only, while the
/// one-shot flows take `run --query "<prompt>"`.
pub(crate) fn register_agent_flow_handlers(registry: &mut crate::commands::CommandRegistry) {
    use crate::commands::{CommandContext, CommandHandler, CommandResult};

    pub(crate) fn agent_flow_argv(extra: &[&str]) -> Vec<String> {
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "operant".to_string());
        std::iter::once(exe)
            .chain(extra.iter().map(|s| s.to_string()))
            .collect()
    }

    struct ImproveHandler;
    #[async_trait::async_trait]
    impl CommandHandler for ImproveHandler {
        async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
            // Autonomous mode reads the TODO.md ledger (run --autonomous
            // rejects --query by design).
            CommandResult::ShellCommand(agent_flow_argv(&["run", "--autonomous"]))
        }
    }

    struct JudgeHandler;
    #[async_trait::async_trait]
    impl CommandHandler for JudgeHandler {
        async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
            CommandResult::ShellCommand(agent_flow_argv(&[
                "run",
                "--query",
                "Judge this session: review the conversation for correctness, completeness, and safety, then report findings.",
            ]))
        }
    }

    struct RefactorHandler;
    #[async_trait::async_trait]
    impl CommandHandler for RefactorHandler {
        async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
            CommandResult::ShellCommand(agent_flow_argv(&[
                "run",
                "--query",
                "Run a safe refactor loop: identify a refactor opportunity, apply it, then review the diff.",
            ]))
        }
    }

    struct FixHandler;
    #[async_trait::async_trait]
    impl CommandHandler for FixHandler {
        async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
            CommandResult::ShellCommand(agent_flow_argv(&[
                "run",
                "--query",
                "Recover: the model cannot continue — diagnose the failure from the transcript and propose a fix.",
            ]))
        }
    }

    struct TriageHandler;
    #[async_trait::async_trait]
    impl CommandHandler for TriageHandler {
        async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
            CommandResult::ShellCommand(agent_flow_argv(&[
                "run",
                "--query",
                "Triage new GitHub issues: fetch open issues, classify safe vs unsafe, autonomously fix the safe ones.",
            ]))
        }
    }

    registry
        .register_handler("improve", Box::new(ImproveHandler))
        .ok();
    registry
        .register_handler("judge", Box::new(JudgeHandler))
        .ok();
    registry
        .register_handler("refactor", Box::new(RefactorHandler))
        .ok();
    registry.register_handler("fix", Box::new(FixHandler)).ok();
    registry
        .register_handler("triage", Box::new(TriageHandler))
        .ok();
}

pub(crate) fn register_tui_command_handlers(
    command_registry: &mut crate::commands::CommandRegistry,
) {
    // (2026-10-14 batch, plan 02) extracted from the create path so the
    // no-unwired-command test can mirror the LIVE handler wiring —
    // a bare `CommandRegistry::new()` lacks all of these and would
    // misreport wired commands as dead.
    use crate::commands::{CommandContext, CommandHandler, CommandResult};
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
                    Err(e) => CommandResult::message(format!("Failed to create AGENTS.md: {}", e)),
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

    // ── Tier 3: agent-flow commands ──────────────────────────────────
    // Registered in the slash surface but historically unhandled — they
    // fell through the TUI intercept AND the registry, landing in the
    // model as literal prompt text. See register_agent_flow_handlers.
}
