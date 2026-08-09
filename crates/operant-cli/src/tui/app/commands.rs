//! Command handling methods.

use super::*;

impl App {
    #[allow(dead_code)] // Used in tests
    pub fn intercept_slash_command_with_args(&mut self, cmd: &str, args: &str) -> bool {
        if cmd == "mcp" && !args.trim().is_empty() {
            return false;
        }
        self.intercept_slash_command_with_args_impl(cmd, args)
    }

    pub fn handle_tui_command(&mut self, cmd: &str, args: &str) -> bool {
        if cmd == "mcp" && !args.trim().is_empty() {
            return false;
        }
        self.intercept_slash_command_with_args_impl(cmd, args)
    }

    /// Backwards-compatible wrapper that takes no args (treats args as empty).
    /// Kept so external callers (and the existing `?` shortcut path) still work.
    #[allow(dead_code)] // Used in tests
    pub fn intercept_slash_command(&mut self, cmd: &str) -> bool {
        self.intercept_slash_command_with_args_impl(cmd, "")
    }

    /// A JSON snapshot of assertable App state for the headless simulator's
    /// `--assert` engine. Dot-path keys (e.g. `overlays.model_picker`,
    /// `messages`, `model`) are navigated by `evaluate_assertions`. This is
    /// the generic replacement for the old hardcoded boolean whitelist.
    /// Single source of truth for the set of dialog/overlay visibilities.
    /// Both `any_modal_open()` and `debug_snapshot()` derive from this one
    /// list so the two can't drift out of sync (the drift that dropped
    /// `effort_picker` from `any_modal_open` in iter-227). Each entry is
    /// `(snapshot_key, is_visible)`. `permission_request` is tracked via
    /// `.is_some()` rather than a `.visible` flag.
    pub(crate) fn overlay_flags(&self) -> [(&'static str, bool); 35] {
        [
            ("help_overlay", self.help_overlay.visible),
            (
                "history_search_overlay",
                self.history_search_overlay.visible,
            ),
            ("global_search", self.global_search.visible),
            ("rewind_flow", self.rewind_flow.visible),
            ("settings_screen", self.settings_screen.visible),
            ("theme_screen", self.theme_screen.visible),
            ("stats_dialog", self.stats_dialog.visible),
            ("mcp_view", self.mcp_view.visible),
            ("agents_menu", self.agents_menu.visible),
            ("diff_viewer", self.diff_viewer.visible),
            ("memory_file_selector", self.memory_file_selector.visible),
            ("skills_view", self.skills_view.visible),
            ("plugins_hub", self.plugins_hub.visible),
            ("journey_view", self.journey_view.visible),
            ("hooks_config_menu", self.hooks_config_menu.visible),
            ("voice_mode_notice", self.voice_mode_notice.visible),
            ("model_picker", self.model_picker.visible),
            ("session_browser", self.session_browser.visible),
            ("session_branching", self.session_branching.visible),
            ("tasks_overlay", self.tasks_overlay.visible),
            ("export_dialog", self.export_dialog.visible),
            ("context_viz", self.context_viz.visible),
            ("mcp_approval", self.mcp_approval.visible),
            (
                "bypass_permissions_dialog",
                self.bypass_permissions_dialog.visible,
            ),
            ("effort_picker", self.effort_picker.visible),
            ("key_input_dialog", self.key_input_dialog.visible),
            (
                "custom_provider_dialog",
                self.custom_provider_dialog.visible,
            ),
            ("free_mode_dialog", self.free_mode_dialog.visible),
            ("device_auth_dialog", self.device_auth_dialog.visible),
            ("connect_dialog", self.connect_dialog.visible),
            ("import_config_picker", self.import_config_picker.visible),
            ("import_config_dialog", self.import_config_dialog.visible),
            ("command_palette", self.command_palette.visible),
            ("ask_user_dialog", self.ask_user_dialog.visible),
            ("permission_request", self.permission_request.is_some()),
        ]
    }

    pub fn debug_snapshot(&self) -> serde_json::Value {
        // Overlays map is derived from `overlay_flags()` (single source of
        // truth), so it can't drift from `any_modal_open()`. Built from a
        // flat tuple array rather than a giant json! literal (which would
        // overflow the macro recursion limit).
        let overlays: serde_json::Map<String, serde_json::Value> = self
            .overlay_flags()
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::Bool(v)))
            .collect();

        serde_json::json!({
            "should_exit": self.should_exit,
            "is_streaming": self.is_streaming,
            "is_simulating": self.is_simulating,
            "plan_mode": self.plan_mode,
            "show_help": self.show_help,
            "show_reasoning": self.show_reasoning,
            "fast_mode": self.fast_mode,
            "messages": self.messages.len(),
            "status_message": self.status_message,
            "model": self.model_name,
            "provider": self.active_provider,
            "focus": format!("{:?}", self.focus),
            "token_count": self.token_count,
            "any_modal_open": self.any_modal_open(),
            "overlays": serde_json::Value::Object(overlays),
        })
    }

    /// Push `text` into the live steer queue if the agent is streaming, and
    /// return a status string describing the outcome. Mirrors the live steer
    /// path in adapter_types.rs, but uses `try_lock` because this runs on the
    /// sync slash-command path while the queue is a tokio Mutex.
    /// (iter-240 — wires /steer and /queue <text> to the real steer queue.)
    fn queue_steer(&mut self, text: &str) -> String {
        const NOT_STREAMING: &str = "Steer is only available while the agent is streaming.";
        if !self.is_streaming {
            return NOT_STREAMING.to_string();
        }
        match self.steer_queue_handle.as_ref() {
            Some(handle) => match handle.try_lock() {
                Ok(mut q) => {
                    q.push(text.to_string());
                    format!("Steer queued: {}", text)
                }
                Err(_) => NOT_STREAMING.to_string(),
            },
            None => NOT_STREAMING.to_string(),
        }
    }

    /// Implementation that receives both cmd and args. Most slash commands
    /// ignore args; a few (like /personality <name>) consume them.
    fn intercept_slash_command_with_args_impl(&mut self, cmd: &str, args: &str) -> bool {
        self.close_secondary_views();
        self.dismiss_error_notifications();
        // Record slash-command usage for smart ordering of `/` suggestions.
        // (iter-125 — recency + frequency ranking.)
        self.slash_usage.record(cmd);
        self.slash_usage.save();
        self.debug_hub
            .publish(crate::tui::debug::TuiEvent::SlashCommand {
                name: cmd.to_string(),
                args_preview: args.chars().take(40).collect(),
                at: crate::tui::debug::event_bus::now_secs(),
            });
        match cmd {
            "config" | "settings" => {
                self.settings_screen.open();
                true
            }
            "theme" | "skin" => {
                let current = match &self.settings.theme {
                    Theme::Dark => "dark",
                    Theme::Light => "light",
                    Theme::Default => "default",
                    Theme::Deuteranopia => "deuteranopia",
                    Theme::Custom(s) => s.as_str(),
                };
                self.theme_screen.open(current);
                true
            }
            "stats" | "cost" => {
                self.stats_dialog.open();
                true
            }
            "mcp" => {
                let servers = self.load_mcp_servers();
                self.mcp_view.open(servers);
                true
            }
            "agents" | "tasks" => {
                self.open_agents_menu();
                true
            }
            "diff" | "review" => {
                let root = self.project_root();
                self.diff_viewer.open(&root);
                true
            }
            "changes" => {
                let root = self.project_root();
                // (iter-209: refresh_turn_diff_from_history removed — turn-diff stub deleted)
                self.diff_viewer.open_turn(&root);
                true
            }
            "search" | "find" => {
                self.global_search.open();
                true
            }
            // (iter-211: survey/feedback slash command deleted — no telemetry backend)
            "memory" => {
                let root = self.project_root();
                self.memory_file_selector.open(&root);
                true
            }
            // /skill <name> — expand a skill into the turn (hermes
            // `build_skill_invocation_message` parity): the full preprocessed
            // SKILL.md becomes the user message so the model treats it as
            // active guidance. Bare `/skill` (or `/skills`) opens the overlay.
            "skill" => {
                let name = args.trim();
                if name.is_empty() {
                    let skills_dir = self.config.skills.root_dir.clone();
                    self.skills_view.open(skills_dir);
                    return true;
                }
                // Optional trailing instruction after the skill name:
                // `/skill gitcrawl summarize the repo`
                let (skill_name, instruction) = match name.split_once(char::is_whitespace) {
                    Some((n, rest)) => (n, rest.trim().to_string()),
                    None => (name, String::new()),
                };
                match operant_core::agent::skill_preprocessing::build_skill_invocation_message_in(
                    &self.config.skills.root_dir,
                    skill_name,
                    &instruction,
                ) {
                    Some(msg) => {
                        // Inject as a fresh user message on the next loop
                        // iteration (pending_user_message is drained by the
                        // run loop like pending_retry_query). Returning true
                        // marks the slash command as fully handled, so the
                        // registry fallback is never reached.
                        self.pending_user_message = Some(msg);
                        true
                    }
                    None => {
                        self.status_message = Some(format!(
                            "Skill '{}' not found. Use /skills to browse installed skills.",
                            skill_name
                        ));
                        true
                    }
                }
            }
            // /bundle <name> — expand a skill bundle (multiple skills) into
            // the turn. hermes parity: `skill_bundles.py` + slash bundles.
            "bundle" => {
                let name = args.trim();
                if name.is_empty() {
                    let bundles = operant_core::agent::skill_bundle::list_bundles();
                    if bundles.is_empty() {
                        self.status_message =
                            Some("No skill bundles found in ~/.operant/skill-bundles/".to_string());
                    } else {
                        let names: Vec<&str> = bundles.iter().map(|b| b.slug.as_str()).collect();
                        self.status_message = Some(format!(
                            "Available bundles: {} — usage: /bundle <name>",
                            names.join(", ")
                        ));
                    }
                    true
                } else {
                    let (bundle_name, instruction) = match name.split_once(char::is_whitespace) {
                        Some((n, rest)) => (n, rest.trim().to_string()),
                        None => (name, String::new()),
                    };
                    let key =
                        operant_core::agent::skill_bundle::resolve_bundle_command_key(bundle_name)
                            .unwrap_or_else(|| bundle_name.to_string());
                    match operant_core::agent::skill_bundle::build_bundle_invocation_message(
                        &key,
                        &instruction,
                    ) {
                        Some((msg, _loaded, _missing)) => {
                            self.pending_user_message = Some(msg);
                            true
                        }
                        None => {
                            self.status_message = Some(format!(
                                "Bundle '{}' not found (no loadable skills).",
                                bundle_name
                            ));
                            true
                        }
                    }
                }
            }
            "skills" => {
                let skills_dir = self.config.skills.root_dir.clone();
                self.skills_view.open(skills_dir);
                true
            }
            "plugins" => {
                let plugins_dir =
                    crate::cmd_plugins::plugins_dir(&self.config).unwrap_or_else(|_| {
                        dirs::data_dir()
                            .unwrap_or_default()
                            .join("operant")
                            .join("plugins")
                    });
                self.plugins_hub.open(plugins_dir);
                true
            }
            "hooks" => {
                self.hooks_config_menu.open();
                true
            }
            "import-config" => {
                self.open_import_config_picker();
                true
            }
            "connect" => {
                self.connect_dialog.open();
                true
            }
            "model" => {
                if !self.has_credentials {
                    self.connect_dialog.open();
                    self.status_message = Some("Connect a provider to choose a model.".to_string());
                    return true;
                }
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "anthropic".to_string());
                self.open_model_picker_for_provider(&provider, None);
                true
            }
            "session" | "resume" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                true
            }
            "clear" => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_use_blocks.clear();
                self.turn_metadata.clear();
                self.cost_usd = 0.0;
                // Reset streaming + scroll + token state so new input isn't
                // silently dropped. Without this, /clear mid-stream leaves
                // is_streaming=true, so the prompt input handler rejects
                // new queries.
                self.is_streaming = false;
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.token_count = 0;
                self.invalidate_transcript();
                self.status_message = Some("Conversation cleared.".to_string());
                true
            }
            "exit" | "quit" => {
                self.should_exit = true;
                true
            }
            "vim" => {
                self.prompt_input.vim_enabled = !self.prompt_input.vim_enabled;
                let status = if self.prompt_input.vim_enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                self.status_message = Some(format!("Vim mode {}.", status));
                self.refresh_prompt_input();
                true
            }
            "fast" => {
                self.fast_mode = !self.fast_mode;
                let status = if self.fast_mode {
                    "enabled"
                } else {
                    "disabled"
                };
                self.status_message = Some(format!("Fast mode {}.", status));
                true
            }
            "plan" => {
                use crate::tui::adapter_types::config::PermissionMode;
                self.plan_mode = !self.plan_mode;
                self.settings.permission_mode = if self.plan_mode {
                    PermissionMode::Plan
                } else {
                    PermissionMode::Default
                };
                self.status_message = Some(if self.plan_mode {
                    "Plan mode ON — Operant will plan before acting.".to_string()
                } else {
                    "Plan mode OFF.".to_string()
                });
                true
            }
            // /stop — cancel the live streaming turn, exactly as Esc does.
            // (iter-270: wired to real streaming cancel path.)
            "stop" => {
                if self.is_streaming {
                    self.is_streaming = false;
                    self.spinner_verb = None;
                    // Flush in-flight streaming text to messages BEFORE snapshot
                    // so the response is preserved in the transcript.
                    self.flush_streamed_assistant_message();
                    self.status_message = Some("Stopped.".to_string());
                    self.complete_current_turn_snapshot(true);
                    self.tool_use_blocks.clear();
                } else {
                    self.status_message = Some("Nothing to stop — no turn is running.".to_string());
                }
                true
            }

            // /new — start a completely fresh session (same as /clear but
            // also resets cost and turn counter).
            // (iter-270: wired to real state clear.)
            "new" | "fresh" => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_use_blocks.clear();
                self.turn_metadata.clear();
                self.session_goal = None;
                self.cost_usd = 0.0;
                self.is_streaming = false;
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.token_count = 0;
                self.session_title = None;
                self.invalidate_transcript();
                self.status_message = Some("New session started.".to_string());
                true
            }

            // /undo — remove the last user + assistant exchange from the
            // transcript. Safe no-op if fewer than 2 messages.
            // (iter-270: wired to real message state.)
            "undo" => {
                // Find last user message index from the end.
                let last_user = self
                    .messages
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, m)| m.role == Role::User)
                    .map(|(i, _)| i);
                if let Some(idx) = last_user {
                    // Remove all messages from that user message to end.
                    self.messages.truncate(idx);
                    // Also discard the trailing assistant turn metadata entry.
                    self.turn_metadata.pop();
                    self.invalidate_transcript();
                    self.status_message = Some("Last turn undone.".to_string());
                } else {
                    self.status_message = Some("Nothing to undo.".to_string());
                }
                true
            }

            // /retry — resubmit the last user message. Queues it via
            // pending_retry_query so the adapter_types run loop can spawn
            // the agent call (App::run is sync, agent.run is async).
            // (iter-270: wired to real state.)
            "retry" => {
                if self.is_streaming {
                    self.status_message =
                        Some("Cannot retry while a turn is running. Stop first.".to_string());
                    return true;
                }
                let last_user_text = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.get_all_text());
                if let Some(text) = last_user_text {
                    // Remove all messages from the last user message onward
                    // so the turn is truly retried (not duplicated).
                    let last_user_idx = self
                        .messages
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, m)| m.role == Role::User)
                        .map(|(i, _)| i);
                    if let Some(idx) = last_user_idx {
                        self.messages.truncate(idx);
                    }
                    self.pending_retry_query = Some(text);
                    self.invalidate_transcript();
                    self.status_message = Some("Retrying last message…".to_string());
                } else {
                    self.status_message = Some("No previous message to retry.".to_string());
                }
                true
            }

            // /save — alias for /export (opens the export dialog).
            // (iter-270: fixed broken stub.)
            "save" => {
                self.export_dialog.open();
                true
            }

            // /goal <text> — set a standing session goal. Shown in the status
            // bar and injected as a system annotation so the agent sees it.
            // /goal with no args shows the current goal.
            // (iter-270: wired to real state.)
            "goal" | "subgoal" => {
                let trimmed = args.trim();
                if trimmed.is_empty() {
                    // Show current goal.
                    let cur = self
                        .session_goal
                        .clone()
                        .unwrap_or_else(|| "(none)".to_string());
                    self.status_message = Some(format!(
                        "Session goal: {}. Use /goal <text> to change.",
                        cur
                    ));
                } else {
                    self.session_goal = Some(trimmed.to_string());
                    // Inject as a system annotation so it appears in transcript.
                    self.push_system_message(
                        format!("🎯 Goal set: {}", trimmed),
                        crate::tui::app::SystemMessageStyle::Info,
                    );
                    self.status_message = Some(format!("Goal set: {}", trimmed));
                }
                true
            }

            // /sessions — alias for /session / /resume (open session browser).
            // (iter-270: fixed broken stub.)
            "sessions" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                true
            }

            "compact" => false,
            "copy" => {
                // Copy last assistant message to clipboard. Attempt arboard; fall back to notification.
                let last = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                    .map(|m| m.get_all_text());
                if let Some(text) = last {
                    // Try xclip/xsel/pbcopy/clip.exe for clipboard; fall back to notification.
                    let copied = try_copy_to_clipboard(&text);
                    if copied {
                        self.push_notification(
                            NotificationKind::Info,
                            "Copied to clipboard.".to_string(),
                            Some(3),
                        );
                    } else {
                        self.push_notification(
                            NotificationKind::Info,
                            format!(
                                "Last response: {} chars (clipboard unavailable)",
                                text.len()
                            ),
                            Some(5),
                        );
                    }
                } else {
                    self.push_notification(
                        NotificationKind::Warning,
                        "No assistant message to copy.".to_string(),
                        Some(3),
                    );
                }
                true
            }
            "output-style" | "verbose" => {
                self.output_style = match self.output_style.as_str() {
                    "auto" => "stream".to_string(),
                    "stream" => "verbose".to_string(),
                    _ => "auto".to_string(),
                };
                self.status_message = Some(format!("Output style: {}.", self.output_style));
                true
            }
            "effort" => {
                // Open the picker dialog so users can pick an effort level
                // visually instead of cycling/typing the level (issue #149).
                self.effort_picker.open(self.effort_level);
                true
            }
            "voice" => {
                let was_on = self.voice_recorder.is_some();
                if was_on {
                    // Stop any active recording before disabling.
                    if self.voice_recording {
                        self.voice_recording = false;
                        self.voice_event_rx = None;
                        if let Some(ref recorder_arc) = self.voice_recorder {
                            let recorder = recorder_arc.clone();
                            tokio::task::spawn_blocking(move || {
                                if let Ok(mut r) = recorder.lock() {
                                    tokio::runtime::Handle::current()
                                        .block_on(r.stop_recording())
                                        .ok();
                                }
                            });
                        }
                    }
                    self.voice_recorder = None;
                    self.voice_mode_notice.dismiss();
                    self.status_message = Some("Voice mode disabled.".to_string());
                } else {
                    let recorder = crate::tui::adapter_types::voice::global_voice_recorder();
                    if let Ok(mut r) = recorder.lock() {
                        r.set_enabled(true);
                    }
                    self.voice_recorder = Some(recorder);
                    self.voice_mode_notice =
                        crate::tui::voice_mode_notice::VoiceModeNoticeState::new();
                    self.status_message =
                        Some("Voice mode enabled. Press Alt+V to start recording.".to_string());
                }
                true
            }
            "doctor" => false,
            "rewind" => {
                self.open_rewind_flow();
                true
            }
            "export" => {
                self.export_dialog.open();
                true
            }
            "context" => {
                self.context_viz.toggle();
                true
            }
            "rename" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                self.session_browser.start_rename();
                true
            }
            "init" | "login" | "logout" => false,
            "keybindings" => {
                // Open the keybindings.json file in the external editor
                let keybindings_path = crate::tui::adapter_types::config::Settings::config_dir()
                    .join("keybindings.json");

                if let Err(e) = open_file_externally(&keybindings_path) {
                    eprintln!("Failed to open keybindings file: {}", e);
                }
                true
            }
            "help" => {
                // Toggle the help overlay (same as pressing `?` or F1).
                // Bug #8 from iter-82 audit: previously only opened (never
                // closed), so pressing /help twice showed two different
                // help overlays (the rich one + the legacy show_help fallback).
                self.help_overlay.toggle();
                self.show_help = self.help_overlay.visible;
                true
            }
            // ── Backfilled slash commands (iter-77) ───────────────────────────
            // These previously appeared in PROMPT_SLASH_COMMANDS but were never
            // intercepted — they fell through to the basic command registry,
            // which printed a one-line help text and felt broken. Most map to
            // existing App / Settings state; the rest return a polite status
            // message so the user knows operant heard them.

            // /yolo — toggle bypass-permissions mode by flipping the
            // permission_mode setting between 'Default' and 'BypassPermissions'.
            "yolo" => {
                let mut settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                let new_mode = if matches!(
                    settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                ) {
                    crate::tui::adapter_types::config::PermissionMode::default()
                } else {
                    crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                };
                settings.permission_mode = new_mode.clone();
                let _ = settings.save_sync();
                self.settings.permission_mode = new_mode.clone();
                self.status_message = Some(
                    if matches!(
                        new_mode,
                        crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                    ) {
                        "YOLO mode armed — permissions will be auto-approved. Use with care."
                            .to_string()
                    } else {
                        "YOLO mode disarmed — permissions will prompt.".to_string()
                    },
                );
                true
            }

            // /busy — toggle "busy" indicator (we map to auto_compact to avoid
            // adding a new state field; busy = compact aggressively).
            "busy" => {
                self.auto_compact_enabled = !self.auto_compact_enabled;
                self.status_message = Some(format!(
                    "Auto-compact {}.",
                    if self.auto_compact_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                true
            }

            // /verbose — cycle output-style between auto/stream/verbose.

            // /reasoning — toggle whether thinking/reasoning blocks are
            // expanded by default in the transcript. (Bug #18 from iter-82
            // audit — previously just printed a status message without
            // toggling anything.)
            "reasoning" => {
                self.show_reasoning = !self.show_reasoning;
                self.invalidate_transcript();
                self.status_message = Some(format!(
                    "Reasoning blocks {} by default.",
                    if self.show_reasoning {
                        "expanded"
                    } else {
                        "collapsed"
                    }
                ));
                true
            }

            // /personality — set agent personality from args.
            // The actual personality string is consumed by the agent loop on
            // the next turn (it reads app.agent_mode).
            "personality" => {
                let trimmed = args.trim();
                if trimmed.is_empty() {
                    // No args — show current value.
                    let cur = self
                        .agent_mode
                        .clone()
                        .unwrap_or_else(|| "default".to_string());
                    self.status_message = Some(format!(
                        "Current personality: {}. Use /personality <name> to change.",
                        cur
                    ));
                } else {
                    // Set the new personality. agent_mode_changed=true signals
                    // the run loop to update the query config on the next turn.
                    self.agent_mode = Some(trimmed.to_string());
                    self.agent_mode_changed = true;
                    self.status_message = Some(format!("Personality set to: {}.", trimmed));
                }
                true
            }

            // /steer <message> — inject guidance into the live steer queue so
            // the agent picks it up at the next iteration boundary mid-turn.
            // (iter-240 — wires to the real steer_queue_handle backend.)
            "steer" => {
                self.status_message = Some(if args.is_empty() {
                    "Usage: /steer <message> (inject guidance while the agent is streaming)"
                        .to_string()
                } else {
                    self.queue_steer(args)
                });
                true
            }

            // /queue — list the live steer queue; /queue <text> is an alias for
            // /steer <text>. operant has no separate pending-input queue — the
            // steer queue IS the queue. (iter-240.)
            "queue" => {
                let msg = if !args.is_empty() {
                    self.queue_steer(args)
                } else {
                    match self.steer_queue_handle.as_ref() {
                        Some(handle) => match handle.try_lock() {
                            Ok(q) if q.is_empty() => "Queue is empty".to_string(),
                            Ok(q) => format!("Queued ({}): {}", q.len(), q.join("; ")),
                            Err(_) => "Queue is busy (agent is draining it).".to_string(),
                        },
                        None => {
                            "Nothing queued (queue is active only while streaming).".to_string()
                        }
                    }
                };
                self.status_message = Some(msg);
                true
            }

            // /background <prompt> — operant's TUI runs a single agent
            // synchronously: App holds no agent handle and there is exactly one
            // event channel + run_complete_rx, so spawning a second agent.run()
            // in-session would interleave into (and corrupt) the live
            // transcript. Rather than a bare no-op, echo the exact working
            // detached command with the user's prompt filled in.
            // (ponytail: in-session background turn needs a second isolated
            // agent via create_runtime_agent with its own session id + event
            // channel, threaded through the run loop — invasive and not
            // headless-testable, so we point at `operant run --query ... &`.)
            "background" => {
                let trimmed = args.trim();
                self.status_message = Some(if trimmed.is_empty() {
                    "Usage: /background <prompt> — operant runs one agent synchronously; this prints the command to run it detached.".to_string()
                } else {
                    let escaped = trimmed.replace('"', "\\\"");
                    format!(
                        "Operant runs synchronously in-session. Background it with: operant run --query \"{}\" &",
                        escaped
                    )
                });
                true
            }

            // /rollback — surface the existing /rewind flow (which IS
            // implemented) instead of silently dropping /rollback.
            "rollback" => {
                let root = self.project_root();
                // (iter-209: refresh_turn_diff_from_history removed — turn-diff stub deleted)
                self.diff_viewer.open_turn(&root);
                self.status_message =
                    Some("Rollback: review last turn diff. Use /rewind to step back.".to_string());
                true
            }

            // /reload-mcp — request a live MCP reconnect. The run loop drains
            // pending_mcp_reconnect (adapter_types.rs) and reconnects the MCP
            // servers without restarting the TUI. (iter-240 — wires to the
            // pending_mcp_reconnect backend.)
            "reload-mcp" => {
                self.pending_mcp_reconnect = true;
                self.status_message = Some("Reconnecting MCP servers…".to_string());
                true
            }

            // /reload — re-read TUI settings from disk and re-apply the visual /
            // preference subset that is safe to swap live (theme, output style,
            // permission mode). We intentionally do NOT hot-swap the provider /
            // model client mid-session, so those changes only take effect on
            // restart. Ref: hermes-agent cli.py reload_env().
            "reload" => {
                let new_settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                // Detect whether the provider/model changed on disk so the
                // status line can be honest about what applies now vs. on
                // restart.
                let disk_model = operant_core::config::load_app_config(None)
                    .map(|c| c.config.agent.model)
                    .unwrap_or_else(|_| self.config.agent.model.clone());
                let model_changed = disk_model != self.config.agent.model;

                self.settings = new_settings;
                self.plan_mode = matches!(
                    self.settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::Plan
                );
                self.output_style = match self.settings.output_style.as_deref() {
                    Some("stream") => "stream".to_string(),
                    Some("verbose") => "verbose".to_string(),
                    _ => "auto".to_string(),
                };

                self.status_message = Some(if model_changed {
                    "Config reloaded (provider/model changes apply on restart).".to_string()
                } else {
                    "Configuration reloaded.".to_string()
                });
                true
            }

            // /reload-skills — re-scan the skills directory and repopulate the
            // /skills overlay's backing data. The running agent was built with a
            // fixed SkillManager at startup (main.rs `with_skill_manager`) and
            // exposes no runtime setter, so rescanned skills reach the model only
            // after a restart; the status stays honest about that. Ref:
            // hermes-agent cli.py reload_skills().
            "reload-skills" => {
                let skills_dir = self.config.skills.root_dir.clone();
                let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
                match mgr.load_all() {
                    Ok(mut loaded) => {
                        // Same (category, name) sort skills_view.open() uses so
                        // the overlay renders identically after a rescan.
                        loaded.sort_by(|a, b| {
                            a.category
                                .cmp(&b.category)
                                .then_with(|| a.name.cmp(&b.name))
                        });
                        let count = loaded.len();
                        self.skills_view.skills = loaded;
                        if self.skills_view.selected >= count {
                            self.skills_view.selected = 0;
                        }
                        self.status_message = Some(format!(
                            "Rescanned {} skill{}. Browse with /skills (agent picks up changes on restart).",
                            count,
                            if count == 1 { "" } else { "s" }
                        ));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Failed to reload skills: {}", e));
                    }
                }
                true
            }

            // /browser — surface that operant has a Camofox browser backend
            // but no in-TUI browser launcher (it's a tool the agent calls).
            "browser" => {
                self.status_message = Some(
                    "Browser: operant uses Camofox as the default. The agent invokes it via the browser tool — no in-TUI browser panel.".to_string()
                );
                true
            }

            // /indicator, /statusbar — toggle Settings.terminal_progress_bar
            // (operant has one status-bar toggle, not the two hermes has).
            "indicator" | "statusbar" => {
                let mut settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                settings.terminal_progress_bar = !settings.terminal_progress_bar;
                let _ = settings.save_sync();
                self.status_message = Some(format!(
                    "Status bar {}.",
                    if settings.terminal_progress_bar {
                        "shown"
                    } else {
                        "hidden"
                    }
                ));
                true
            }

            // /mouse — report the mouse-capture state. Capture is enabled on
            // startup unless --no-mouse was passed; that flag lives on the
            // TuiApp runner (adapter_types.rs) and is not threaded into App,
            // so this reports the real startup default. (iter-240.)
            "mouse" => {
                self.status_message = Some(
                    "Mouse capture: enabled (use --no-mouse to disable, e.g. inside tmux)"
                        .to_string(),
                );
                true
            }

            // /terminal-setup — surface that operant auto-detects terminal
            // capabilities at startup (OSC8, truecolor, etc.).
            "terminal-setup" => {
                self.status_message = Some(
                    "Terminal capabilities are auto-detected at startup. No manual setup needed."
                        .to_string(),
                );
                true
            }

            // /redraw — force a full redraw by bumping the transcript
            // version counter (which invalidates cached render state).
            "redraw" => {
                self.transcript_version
                    .set(self.transcript_version.get().wrapping_add(1));
                self.status_message = Some("Screen redrawn.".to_string());
                true
            }

            // /billing, /credits — surface that operant doesn't track
            // provider billing/credits (it's BYOK); point users at /stats
            // for local token usage tracking.
            "billing" | "credits" => {
                self.status_message = Some(format!(
                    "{}: operant is BYOK and doesn't track provider billing. Use /stats for local token usage.",
                    cmd
                ));
                true
            }

            // /update — point at `operant update` (the TUI can't self-update
            // without restarting).
            "update" => {
                self.status_message = Some(
                    "Run `operant update` from a shell to check for and install a new release."
                        .to_string(),
                );
                true
            }

            // /heapdump, /mem — debug diagnostics; surface a snapshot of
            // turn count + token count + cost as a memory/heap summary.
            "heapdump" | "mem" => {
                self.status_message = Some(format!(
                    "Turns: {} | Tokens: {} | Cost: ${:.4} | Agent status entries: {}",
                    self.turn_metadata.len(),
                    self.token_count,
                    self.cost_usd,
                    self.agent_status.len()
                ));
                true
            }

            // /pet — Easter-egg. (iter-144: rustle pose trigger deleted
            // since the pose system was dead code. Still shows the message.)
            "pet" => {
                self.status_message = Some("Rustle wags its tail. 🐕".to_string());
                true
            }

            // /journey, /replay, /replay-diff — these need their own overlays
            // (planned for a later iteration). Surface a "coming soon" status
            // rather than silently dropping.
            "journey" => {
                let skills_dir = self.config.skills.root_dir.clone();
                let memory_dir = operant_core::platform::operant_home().join("memory");
                self.journey_view.open(skills_dir, memory_dir);
                true
            }
            "replay" | "replay-diff" => {
                self.status_message = Some(format!(
                    "/{} overlay is planned. For now, use /agents to view the spawn tree.",
                    cmd
                ));
                true
            }

            // /setup — suspend the TUI and shell out to `operant setup` so the
            // user gets the full interactive wizard. The run loop in
            // TuiApp::run polls pending_shell_command after each frame and,
            // if set, leaves alt screen + raw mode, spawns the command with
            // inherited stdio, waits for it, then re-enters alt screen + raw
            // mode and clears the field.
            "setup" => {
                // Use the current binary (so the version matches) with the
                // `setup` subcommand. If operant was launched via a wrapper,
                // fall back to the literal "operant" name on PATH.
                let exe = std::env::current_exe()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "operant".to_string());
                self.pending_shell_command = Some(vec![exe, "setup".to_string()]);
                self.status_message = Some("Launching setup wizard…".to_string());
                true
            }
            // /whoami — show what the agent knows about the user.
            // (P1-9 from UX audit — transparency + trust.)
            "whoami" => {
                let mem_dir = operant_core::platform::operant_home().join("memory");
                let store = operant_core::memory::MemoryStore::new(mem_dir);
                match store.read_memories() {
                    Ok(map) if map.is_empty() => {
                        self.status_message = Some(
                            "I don't know much about you yet. Chat with me and I'll start remembering.".to_string()
                        );
                    }
                    Ok(map) => {
                        let blocks: Vec<_> = map.into_values().collect();
                        let mut summary = format!(
                            "Here's what I know about you ({} memories):\n\n",
                            blocks.len()
                        );
                        for block in blocks.iter().take(10) {
                            let preview: String = block
                                .content
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(80)
                                .collect();
                            summary.push_str(&format!(
                                "  [{:>3}] {:<10} {}\n",
                                block.importance, block.block_type, preview,
                            ));
                        }
                        if blocks.len() > 10 {
                            summary.push_str(&format!("\n  ...and {} more\n", blocks.len() - 10));
                        }
                        self.push_system_message(
                            summary,
                            crate::tui::app::SystemMessageStyle::Info,
                        );
                    }
                    Err(_) => {
                        self.status_message = Some(
                            "No memory store found. Use /memory to manage memory files."
                                .to_string(),
                        );
                    }
                }
                true
            }
            _ => {
                // Fallback: try the command registry for any unhandled command.
                // This wires up the unified CommandRegistry so commands defined
                // in commands.rs but not yet added to the intercept match arms
                // can still be dispatched via their registered handlers.
                //
                // Only dispatch if the command is actually registered in the
                // registry — truly unknown commands (e.g. `/survey` after its
                // deletion) should NOT be intercepted so the test
                // `test_feedback_survey_removed` can verify they fall through.
                if self.command_registry.resolve(cmd).is_none() {
                    return false;
                }
                let cmd_name = cmd.to_string();
                let args_owned = args.to_string();
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        self.command_registry.execute(&cmd_name, &args_owned).await
                    })
                });
                self.handle_command_result(result)
            }
        }
    }

    /// Interpret a [`CommandResult`] and apply the corresponding side effect
    /// in the TUI. This is the single dispatch point for all slash commands
    /// that go through the `CommandRegistry`.
    ///
    /// Returns `true` if the command was intercepted (even if it only showed
    /// a message), `false` if it should fall through to the agent.
    fn handle_command_result(&mut self, result: crate::commands::CommandResult) -> bool {
        use crate::commands::CommandResult;
        match result {
            // ── Display ────────────────────────────────────────────────────
            CommandResult::Message(text) => {
                self.push_system_message(text, crate::tui::app::SystemMessageStyle::Info);
                true
            }
            CommandResult::Error(text) => {
                self.status_message = Some(text);
                true
            }
            CommandResult::Silent => true,

            // ── Conversation ───────────────────────────────────────────────
            CommandResult::UserMessage(msg) => {
                self.input = msg;
                // Signal the caller that the user message should be submitted.
                false
            }
            CommandResult::ClearConversation => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.is_streaming = false;
                self.tool_use_blocks.clear();
                self.invalidate_transcript();
                self.status_message = Some("Conversation cleared.".to_string());
                true
            }
            CommandResult::NewSession => {
                self.messages.clear();
                self.system_annotations.clear();
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.is_streaming = false;
                self.tool_use_blocks.clear();
                self.invalidate_transcript();
                self.status_message = Some("New session started.".to_string());
                true
            }
            CommandResult::SetMessages(msgs) => {
                self.messages.clear();
                self.system_annotations.clear();
                // Reconstruct messages alternating user/assistant roles.
                for (i, text) in msgs.iter().enumerate() {
                    let role = if i % 2 == 0 {
                        crate::tui::adapter_types::types::Role::User
                    } else {
                        crate::tui::adapter_types::types::Role::Assistant
                    };
                    let message = crate::tui::adapter_types::types::Message {
                        role,
                        content: crate::tui::adapter_types::types::MessageContent::Text(
                            text.clone(),
                        ),
                    };
                    self.messages.push(message);
                }
                self.invalidate_transcript();
                self.status_message = Some(format!("Restored {} messages.", msgs.len()));
                true
            }

            // ── Configuration ──────────────────────────────────────────────
            CommandResult::ToggleSetting { name, enabled } => {
                self.status_message =
                    Some(format!("{}: {}", name, if enabled { "on" } else { "off" }));
                true
            }
            CommandResult::CycleSetting { name, current } => {
                self.status_message = Some(format!("{}: {}", name, current));
                true
            }
            CommandResult::SetGoal(goal) => {
                self.session_goal = goal;
                self.status_message = Some("Session goal updated.".to_string());
                true
            }

            // ── Overlay / UI ───────────────────────────────────────────────
            CommandResult::OpenHelp => {
                self.show_help = true;
                true
            }
            CommandResult::OpenModelPicker => {
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "anthropic".to_string());
                self.open_model_picker_for_provider(&provider, None);
                true
            }
            CommandResult::OpenThemePicker => {
                let theme = self.settings.theme.as_str();
                self.theme_screen.open(theme);
                true
            }
            CommandResult::OpenSessionBrowser => {
                self.session_list_pending = true;
                true
            }
            CommandResult::OpenStats => {
                self.stats_dialog.open();
                true
            }
            CommandResult::OpenMcp => {
                // TODO: populate with live MCP server data from core_mcp_manager.
                self.mcp_view.open(vec![]);
                true
            }
            CommandResult::OpenAgents => {
                let root = self.project_dir.clone().unwrap_or_default();
                self.agents_menu.open(&root);
                true
            }
            CommandResult::OpenDiff => {
                let root = self.project_dir.clone().unwrap_or_default();
                self.diff_viewer.open(&root);
                true
            }
            CommandResult::OpenMemory => {
                let root = self.project_dir.clone().unwrap_or_default();
                self.memory_file_selector.open(&root);
                true
            }
            CommandResult::OpenSkills => {
                self.skills_view
                    .open(operant_core::platform::operant_skills_dir());
                true
            }
            CommandResult::OpenPlugins => {
                let dir = crate::cmd_plugins::plugins_dir(&self.config).unwrap_or_default();
                self.plugins_hub.open(dir);
                true
            }
            CommandResult::OpenHooks => {
                self.hooks_config_menu.open();
                true
            }
            CommandResult::OpenImportConfig => {
                self.open_import_config_picker();
                true
            }
            CommandResult::OpenExport => {
                self.export_dialog.open();
                true
            }
            CommandResult::OpenEffortPicker => {
                self.effort_picker.open(self.effort_level);
                true
            }
            CommandResult::OpenConnect => {
                self.connect_dialog.open();
                true
            }
            CommandResult::OpenSearch => {
                self.global_search.open();
                true
            }
            CommandResult::OpenSettings => {
                self.settings_screen.open();
                true
            }
            CommandResult::OpenContext => {
                self.context_viz.toggle();
                true
            }
            CommandResult::OpenJourney => {
                let skills_dir = operant_core::platform::operant_skills_dir();
                let memory_dir = skills_dir
                    .join("../memory")
                    .canonicalize()
                    .unwrap_or_else(|_| skills_dir.join("../memory"));
                self.journey_view.open(skills_dir, memory_dir);
                true
            }

            // ── Session state ──────────────────────────────────────────────
            CommandResult::StopStreaming => {
                self.is_streaming = false;
                self.flush_streamed_assistant_message();
                true
            }
            CommandResult::Retry => {
                // Set pending_retry_query so the run loop resubmits the last user msg.
                if let Some(last_user) = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| matches!(m.role, crate::tui::adapter_types::types::Role::User))
                {
                    let text = last_user.text_content();
                    if !text.is_empty() {
                        self.pending_retry_query = Some(text);
                    }
                }
                true
            }
            CommandResult::Undo => {
                // Remove the last user+assistant pair.
                let mut removed = 0;
                // Remove trailing assistant message
                if self
                    .messages
                    .last()
                    .map(|m| matches!(m.role, crate::tui::adapter_types::types::Role::Assistant))
                    .unwrap_or(false)
                {
                    self.messages.pop();
                    removed += 1;
                }
                // Remove trailing user message
                if self
                    .messages
                    .last()
                    .map(|m| matches!(m.role, crate::tui::adapter_types::types::Role::User))
                    .unwrap_or(false)
                {
                    self.messages.pop();
                    removed += 1;
                }
                self.invalidate_transcript();
                self.status_message = Some(format!("Undid {} messages.", removed));
                true
            }

            // ── Clipboard ──────────────────────────────────────────────────
            CommandResult::CopyLastResponse => {
                if let Some(last_assistant) =
                    self.messages.iter().rev().find(|m| {
                        matches!(m.role, crate::tui::adapter_types::types::Role::Assistant)
                    })
                {
                    // Filter out thinking blocks — only copy visible text.
                    let text: String = last_assistant
                        .content_blocks()
                        .into_iter()
                        .filter_map(|block| match block {
                            crate::tui::adapter_types::types::ContentBlock::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect();
                    if try_copy_to_clipboard(&text) {
                        self.status_message = Some("Copied to clipboard.".to_string());
                    } else {
                        self.status_message = Some("Failed to copy to clipboard.".to_string());
                    }
                }
                true
            }

            // ── Shell ──────────────────────────────────────────────────────
            CommandResult::ShellCommand(cmds) => {
                self.pending_shell_command = Some(cmds);
                true
            }

            // ── Exit ───────────────────────────────────────────────────────
            CommandResult::Exit => {
                self.should_exit = true;
                true
            }
        }
    }

    // NOTE (iter-237 / Phase B1): intentionally NOT derived from
    // `overlay_flags()`. This is a deliberate *subset* of the overlay set
    // (it omits permission_request, rewind_flow, help_overlay,
    // history_search_overlay, global_search, voice_mode_notice,
    // effort_picker, mcp_approval, bypass_permissions_dialog, ask_user_dialog)
    // and uses `.dismiss()` for export_dialog rather than `.close()`. Unifying
    // it with a loop would change behavior, so it's left explicit; migrate it
    // to the overlay registry in a later iteration once close semantics are
    // normalized.
    pub(super) fn close_secondary_views(&mut self) {
        self.stats_dialog.close();
        self.mcp_view.close();
        self.agents_menu.close();
        self.diff_viewer.close();
        // (iter-211: feedback_survey.close() deleted)
        self.memory_file_selector.close();
        self.skills_view.close();
        self.plugins_hub.close();
        self.journey_view.close();
        self.hooks_config_menu.close();
        self.model_picker.close();
        self.session_browser.close();
        self.session_branching.close();
        self.tasks_overlay.close();
        self.export_dialog.dismiss();
        self.context_viz.close();
        self.connect_dialog.close();
        self.import_config_picker.close();
        self.import_config_dialog.close();
        self.command_palette.close();
        self.key_input_dialog.close();
        self.custom_provider_dialog.close();
        self.free_mode_dialog.close();
        self.device_auth_dialog.close();
        self.settings_screen.close();
        self.theme_screen.close();
    }

    pub fn any_modal_open(&self) -> bool {
        // Derived from `overlay_flags()` (single source of truth) so this
        // can't drift from `debug_snapshot()`. The two extras below are not
        // overlays with a `.visible` flag: `show_help` is a legacy boolean
        // and `context_menu_state` is a popup, both of which still count as
        // "a modal is open" for input gating.
        self.overlay_flags().iter().any(|(_, v)| *v)
            || self.show_help
            || self.context_menu_state.is_some()
    }

    pub(super) fn dismiss_error_notifications(&mut self) {
        while self.notifications.current_is_error() {
            self.notifications.dismiss_current();
        }
        self.error_modal_scroll_offset = 0;
    }

    /// Perform the export based on the selected format. Returns the path written.
    pub fn perform_export(&mut self) -> Option<String> {
        use crate::tui::export_dialog::{export_as_json, export_as_markdown};
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let (filename, content) = match self.export_dialog.selected {
            ExportFormat::Json => {
                let json = export_as_json(&self.messages, self.session_title.as_deref());
                let s = serde_json::to_string_pretty(&json).unwrap_or_default();
                (format!("claude-export-{}.json", ts), s)
            }
            ExportFormat::Markdown => {
                let md = export_as_markdown(&self.messages, self.session_title.as_deref());
                (format!("claude-export-{}.md", ts), md)
            }
        };
        if std::fs::write(&filename, &content).is_ok() {
            self.export_dialog.dismiss();
            Some(filename)
        } else {
            None
        }
    }

    pub(super) fn project_root(&self) -> std::path::PathBuf {
        self.project_dir
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }

    pub(super) fn refresh_global_search(&mut self) {
        let root = self.project_root();
        self.global_search.run_search(&root);
    }

    pub(super) fn load_mcp_servers(&self) -> Vec<McpServerView> {
        // Phase 3a (iter-208): rewired to use the REAL core_mcp_manager
        // instead of the deleted stub. The stub always returned empty data,
        // so /mcp showed 0 tools and all servers Disconnected. Now we read
        // the real connection state from operant_core::mcp::McpManager.
        //
        // The core API is async (tokio::sync::RwLock), but load_mcp_servers
        // is called from the sync render path. We use block_in_place +
        // Handle::block_on to safely call the async methods from within
        // the TUI's tokio runtime. This is the same pattern used by
        // operant's other sync→async bridges.
        if let Some(core_manager) = self.core_mcp_manager.as_ref() {
            // Try to get a runtime handle. If we're not in a tokio context
            // (e.g. unit tests), fall back to the config-only path below.
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                return self.load_mcp_servers_config_only();
            };
            let result = tokio::task::block_in_place(|| {
                handle.block_on(async {
                    let server_names = core_manager.server_names().await;
                    let all_servers = core_manager.all_servers().await;
                    (server_names, all_servers)
                })
            });

            let (server_names, all_servers) = result;
            return self
                .config
                .mcp
                .servers
                .iter()
                .map(|server| {
                    let transport = server
                        .url
                        .as_ref()
                        .map(|_| format!("{:?}", server.transport).to_lowercase())
                        .or_else(|| server.command.as_ref().map(|_| "stdio".to_string()))
                        .unwrap_or_else(|| format!("{:?}", server.transport).to_lowercase());

                    // Check if this server is connected in the core manager.
                    let connected = all_servers.contains_key(&server.name);

                    // Collect tools from the core transport if connected.
                    let tools: Vec<McpToolView> = if connected {
                        // Use block_in_place again for the async get_tools call.
                        let handle = tokio::runtime::Handle::try_current().ok();
                        if let Some(handle) = handle {
                            let transport_tools = tokio::task::block_in_place(|| {
                                handle.block_on(async {
                                    if let Some(t) = all_servers.get(&server.name) {
                                        t.get_tools().await
                                    } else {
                                        Vec::new()
                                    }
                                })
                            });
                            transport_tools
                                .into_iter()
                                .map(|t| {
                                    let def = t.definition();
                                    McpToolView {
                                        name: def.name.clone(),
                                        server: server.name.clone(),
                                        description: def.description.clone(),
                                        input_schema: Some(
                                            serde_json::to_string(&def.input_schema)
                                                .unwrap_or_default(),
                                        ),
                                    }
                                })
                                .collect()
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    };

                    let (status, error_message) = if connected {
                        (McpViewStatus::Connected, None)
                    } else if server_names.contains(&server.name) {
                        (McpViewStatus::Connecting, None)
                    } else {
                        (McpViewStatus::Disconnected, None)
                    };

                    McpServerView {
                        name: server.name.clone(),
                        transport,
                        status,
                        tool_count: tools.len(),
                        resource_count: 0,
                        prompt_count: 0,
                        resources: vec![],
                        prompts: vec![],
                        error_message,
                        tools,
                    }
                })
                .collect();
        }

        self.load_mcp_servers_config_only()
    }

    /// Fallback: build McpServerView list from config only (no live data).
    /// Used when core_mcp_manager is None or when not in a tokio runtime.
    fn load_mcp_servers_config_only(&self) -> Vec<McpServerView> {
        self.config
            .mcp
            .servers
            .iter()
            .map(|server| {
                let transport = server
                    .url
                    .as_ref()
                    .map(|_| format!("{:?}", server.transport).to_lowercase())
                    .or_else(|| server.command.as_ref().map(|_| "stdio".to_string()))
                    .unwrap_or_else(|| format!("{:?}", server.transport).to_lowercase());
                let description = if let Some(url) = &server.url {
                    format!("Endpoint: {}", url)
                } else if let Some(command) = &server.command {
                    let args = if server.args.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", server.args.join(" "))
                    };
                    format!("Command: {}{}", command, args)
                } else {
                    "Configured server".to_string()
                };
                McpServerView {
                    name: server.name.clone(),
                    transport,
                    status: McpViewStatus::Disconnected,
                    tool_count: 0,
                    resource_count: 0,
                    prompt_count: 0,
                    resources: vec![],
                    prompts: vec![],
                    error_message: None,
                    tools: vec![McpToolView {
                        name: "connection".to_string(),
                        server: server.name.clone(),
                        description,
                        input_schema: None,
                    }],
                }
            })
            .collect()
    }

    fn open_agents_menu(&mut self) {
        let root = self.project_root();
        self.agents_menu.open(&root);
        self.agents_menu.active_agents = self
            .agent_status
            .iter()
            .map(|(name, status)| AgentInfo {
                name: name.clone(),
                status: match status.as_str() {
                    "running" => AgentStatus::Running,
                    "waiting" | "waiting_for_tool" => AgentStatus::WaitingForTool,
                    "complete" | "completed" | "done" => AgentStatus::Complete,
                    "failed" | "error" => AgentStatus::Failed,
                    _ => AgentStatus::Idle,
                },
            })
            .collect();
    }

    // Add a message directly (e.g. from a non-streaming source).
}
