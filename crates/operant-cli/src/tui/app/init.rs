//! App constructor and initialization.

use super::*;

impl App {
    pub fn new(
        mut config: AppConfig,
        settings: Settings,
        cost_tracker: Arc<CostTracker>,
        command_registry: crate::commands::CommandRegistry,
    ) -> Self {
        let auth_store = crate::tui::adapter_types::AuthStore::load();
        let has_credentials = auth_store.has_any_key()
            || crate::tui::adapter_types::config::resolve_api_key().is_some();

        // Read persisted TUI state from settings.json so CLI commands like
        // `operant tui effort set high` and `operant tui vim on` take effect
        // on the next TUI launch. (Closes parity gaps #5, #8, #10.)
        let initial_effort = match settings.effort_level.as_deref() {
            Some("low") => EffortLevel::Low,
            Some("high") => EffortLevel::High,
            Some("max") => EffortLevel::Max,
            _ => EffortLevel::Normal,
        };
        let initial_vim = settings.vim_enabled;

        let model_name = {
            let raw = config.agent.model.clone();
            if raw.ends_with("/default") {
                let provider = raw.strip_suffix("/default").unwrap_or(&raw);
                let mut probe_reg = crate::tui::adapter_types::ModelRegistry::new();
                let probe_cache = dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("operant")
                    .join("models.json");
                probe_reg.load_cache(&probe_cache);
                let resolved =
                    crate::tui::model_picker::default_model_for_provider(provider, &probe_reg);
                if resolved != format!("{}/default", provider) {
                    config.agent.model = resolved.clone();
                    resolved
                } else {
                    raw
                }
            } else {
                raw
            }
        };
        let initial_active_provider =
            super::super::provider::infer_provider_from_model(&config.agent.model);
        let (bridge_state_tx, bridge_state_rx) = tokio::sync::mpsc::unbounded_channel::<
            crate::tui::bridge_state::BridgeConnectionState,
        >();
        Self {
            config,
            settings,
            project_dir: std::env::current_dir().ok(),
            is_simulating: false,
            simulated_keys: Vec::new(),
            simulation_max_frames: None,
            cost_tracker,
            debug_hub: crate::tui::debug::TuiDebugHub::new_from_env(),
            command_registry,
            messages: Vec::new(),
            system_annotations: Vec::new(),
            input: String::new(),
            prompt_input: {
                let mut p = PromptInputState::new();
                p.vim_enabled = initial_vim;
                // Load persisted input history so up/down arrow cycling works
                // across sessions. (iter-125 — closes the user-reported
                // "up/down must cycle through previously sent messages"
                // request.)
                p.history = crate::tui::input_history::load();
                p
            },
            scroll_offset: 0,
            is_streaming: false,
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            show_reasoning: false,
            status_message: None,
            spinner_verb: None,
            should_exit: false,
            show_help: false,
            pending_shell_command: None,
            tool_use_blocks: Vec::new(),
            permission_request: None,
            frame_count: 0,
            perf_tier: crate::tui::redraw::PerformanceTier::detect(),
            last_activity: std::time::Instant::now(),
            token_count: 0,
            cost_usd: 0.0,
            model_name,
            active_provider: initial_active_provider,
            has_credentials,
            effort_level: initial_effort,
            fast_mode: false,
            agent_mode: None,
            agent_mode_changed: false,
            accent_color: ACCENT_BUILD,
            agent_status: Vec::new(),
            cursor_pos: 0,
            auto_scroll: true,
            new_messages_while_scrolled: 0,
            token_warning_threshold_shown: 0,
            session_start: std::time::Instant::now(),
            turn_start: None,
            last_turn_elapsed: None,
            last_turn_verb: None,
            turn_metadata: Vec::new(),
            transcript_version: Cell::new(0),
            help_overlay: {
                let mut overlay = HelpOverlay::new();
                overlay.populate_from_commands(help_overlay_entries());
                overlay
            },
            history_search_overlay: HistorySearchOverlay::new(),
            global_search: GlobalSearchState::default(),
            rewind_flow: RewindFlowOverlay::new(),
            notifications: NotificationQueue::new(),
            error_modal_scroll_offset: 0,
            session_title: None,
            remote_session_url: None,
            bridge_state: crate::tui::bridge_state::BridgeConnectionState::Disconnected,
            bridge_state_rx: Some(bridge_state_rx),
            bridge_state_tx: Some(bridge_state_tx),
            core_mcp_manager: None,
            steer_queue_handle: None,
            pending_mcp_reconnect: false,
            pending_mcp_panel_auth: None,
            // (iter-209: file_history + current_turn init deleted)
            slash_usage: crate::tui::slash_usage::UsageStore::load(),
            session_goal: None,
            pending_retry_query: None,
            plan_mode: false,
            stall_start: None,
            settings_screen: SettingsScreen::new(),
            theme_screen: ThemeScreen::new(),
            stats_dialog: StatsDialogState::new(),
            mcp_view: McpViewState::new(),
            agents_menu: AgentsMenuState::new(),
            diff_viewer: DiffViewerState::new(),
            // (iter-211: feedback_survey init deleted)
            memory_file_selector: crate::tui::memory_file_selector::MemoryFileSelectorState::new(),
            skills_view: crate::tui::skills_view::SkillsViewState::new(),
            plugins_hub: crate::tui::plugins_hub::PluginsHubState::new(),
            journey_view: crate::tui::journey_view::JourneyViewState::new(),
            hooks_config_menu: crate::tui::hooks_config_menu::HooksConfigMenuState::new(),
            voice_mode_notice: crate::tui::voice_mode_notice::VoiceModeNoticeState::new(),
            model_picker: ModelPickerState::new(),
            session_browser: SessionBrowserState::new(),
            session_branching: crate::tui::session_branching::SessionBranchingState::new(),
            tasks_overlay: TasksOverlay::new(),
            export_dialog: ExportDialogState::new(),
            context_viz: ContextVizState::new(),
            mcp_approval: McpApprovalDialogState::new(),

            bypass_permissions_dialog:
                crate::tui::bypass_permissions_dialog::BypassPermissionsDialogState::new(),
            effort_picker: crate::tui::effort_picker::EffortPickerState::new(),
            key_input_dialog: crate::tui::key_input_dialog::KeyInputDialogState::new(),
            custom_provider_dialog:
                crate::tui::custom_provider_dialog::CustomProviderDialogState::new(),
            free_mode_dialog: crate::tui::free_mode_dialog::FreeModeDialogState::new(),
            device_auth_dialog: crate::tui::device_auth_dialog::DeviceAuthDialogState::new(),
            device_auth_pending: None,
            model_registry: {
                let mut reg = crate::tui::adapter_types::ModelRegistry::new();
                // Try to load cached models.dev data from disk.
                let cache_path = dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("operant")
                    .join("models.json");
                reg.load_cache(&cache_path);
                reg
            },
            model_picker_fetch_pending: false,
            model_picker_provider_id: None,
            session_list_pending: false,
            session_list_rx: None,
            session_load_pending: None,
            session_load_rx: None,
            auth_store,
            connect_dialog: DialogSelectState::new("Connect a provider", provider_picker_items()),
            import_config_picker: DialogSelectState::new(
                "Import config",
                import_config_picker_items(),
            ),
            import_config_dialog: ImportConfigDialogState::new(),
            command_palette: {
                let items: Vec<SelectItem> = tui_slash_command_data()
                    .iter()
                    .map(|(name, desc)| SelectItem {
                        id: format!("/{}", name),
                        title: format!("/{}", name),
                        description: (*desc).to_string(),
                        category: crate::commands::tui_category(name).to_string(),
                        badge: None,
                    })
                    .collect();
                DialogSelectState::new("Command Palette", items)
            },
            output_style: "auto".to_string(),
            current_dir: std::env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string())),
            git_branch: crate::tui::adapter_types::git_utils::get_repo_root(
                std::env::current_dir()
                    .as_deref()
                    .unwrap_or_else(|_| std::path::Path::new(".")),
            )
            .and_then(|repo_root| {
                crate::tui::adapter_types::git_utils::get_current_branch(&repo_root)
            }),
            auto_compact_enabled: false,
            voice_recorder: {
                // Check whether voice input has been enabled via the /voice command
                // (stored in ~/.operant/ui-settings.json).  We also accept
                // OPERANT_VOICE_ENABLED=1 as an override for easier testing.
                let voice_on = std::env::var("OPERANT_VOICE_ENABLED")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
                    || {
                        let path = crate::tui::adapter_types::config::Settings::config_dir()
                            .join("ui-settings.json");
                        std::fs::read_to_string(&path)
                            .ok()
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                            .and_then(|v| v["voice_enabled"].as_bool())
                            .unwrap_or(false)
                    };
                if voice_on {
                    let recorder = crate::tui::adapter_types::voice::global_voice_recorder();
                    if let Ok(mut r) = recorder.lock() {
                        r.set_enabled(true);
                    }
                    Some(recorder)
                } else {
                    None
                }
            },
            voice_recording: false,
            voice_event_rx: None,
            agent_event_rx: None,
            permission_rx: None,
            pending_permission_response_tx: None,
            run_complete_rx: None,
            agent_task_handle: None,
            pending_key: None,
            model_fetch_rx: None,
            user_question_rx: None,
            ask_user_dialog: crate::tui::ask_user_dialog::AskUserDialogState::new(),
            context_window_size: 0,
            context_used_tokens: 0,
            rate_limit_5h_pct: None,
            rate_limit_7day_pct: None,
            thinking_expanded: std::collections::HashSet::new(),
            last_msg_area: Cell::new(ratatui::layout::Rect::default()),
            last_selectable_area: Cell::new(ratatui::layout::Rect::default()),
            last_input_area: Cell::new(ratatui::layout::Rect::default()),
            footer_right_column_area: Cell::new(ratatui::layout::Rect::default()),
            focus: FocusTarget::Input,
            thinking_row_map: RefCell::new(std::collections::HashMap::new()),
            message_row_map: RefCell::new(std::collections::HashMap::new()),
            last_render_scroll_offset: Cell::new(0),
            selection_anchor: None,
            selection_focus: None,
            selection_text: RefCell::new(String::new()),
            last_row_text: RefCell::new(std::collections::HashMap::new()),
            last_click_time: None,
            last_click_position: None,
            click_count: 0,
            context_menu_state: None,
            scroll_accel: 3.0,
            scroll_last_time: None,
            bash_prefix_allowlist: std::collections::HashSet::new(),
            last_exit_key_warning: None,
            exit_key_sequence_start: None,
        }
    }
}
