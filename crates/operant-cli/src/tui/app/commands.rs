//! Command handling methods.

use super::*;

impl App {
    pub fn intercept_slash_command_with_args(&mut self, cmd: &str, args: &str) -> bool {
        self.intercept_slash_command_with_args_impl(cmd, args)
    }

    pub fn handle_tui_command(&mut self, cmd: &str, args: &str) -> bool {
        self.intercept_slash_command_with_args_impl(cmd, args)
    }

    /// Backwards-compatible wrapper that takes no args (treats args as empty).
    /// Kept so external callers (and the existing `?` shortcut path) still work.
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
    pub(crate) fn overlay_flags(&self) -> [(&'static str, bool); 32] {
        [
            ("help_overlay", self.help_overlay.visible),
            (
                "history_search_overlay",
                self.history_search_overlay.visible,
            ),
            ("global_search", self.global_search.visible),
            ("rewind_selector", self.rewind_selector.visible),
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
            // (plan 05B) the unified inline picker replaced the legacy
            // model_picker overlay.
            ("inline_interactive", self.inline_interactive.is_some()),
            ("session_browser", self.session_browser.visible),
            ("session_branching", self.session_branching.visible),
            ("export_dialog", self.export_dialog.visible),
            ("context_viz", self.context_viz.visible),
            ("mcp_approval", self.mcp_approval.visible),
            (
                "bypass_permissions_dialog",
                self.bypass_permissions_dialog.visible,
            ),
            ("permissions_picker", self.permissions_picker.visible),
            ("key_input_dialog", self.key_input_dialog.visible),
            (
                "custom_provider_dialog",
                self.custom_provider_dialog.visible,
            ),
            ("free_mode_dialog", self.free_mode_dialog.visible),
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
            "terminal_clear_collapsed": self.terminal_clear_state_live(),
            "is_simulating": self.is_simulating,
            "plan_mode": self.plan_mode,
            "show_help": self.show_help,
            "show_reasoning": self.show_reasoning,
            "messages": self.messages.len(),
            "status_message": self.status_message,
            "model": self.model_name,
            "provider": self.active_provider,
            "focus": format!("{:?}", self.focus),
            "token_count": self.token_count,
            "any_modal_open": self.any_modal_open(),
            "overlays": serde_json::Value::Object(overlays),
            // Non-modal surfaces, deliberately NOT in `overlays`:
            // `any_modal_open()` is derived from `overlay_flags()`, and both
            // of these render below every modal (render/mod.rs) without
            // gating input. Folding them in would make `any_modal_open`
            // claim a modal is up when none is, which changes real
            // behaviour — it gates error-modal precedence, toast
            // suppression, and key routing.
            "usage_overlay": self.usage_overlay.visible,
            "debug_overlay": self.debug_hub.overlay_visible(),
            // Selection state — the scenario corpus asserts on these to prove
            // a simulated drag actually anchored/focused a range instead of
            // goldenning an unpainted screen (selection-highlight scenario).
            "selection_anchor": self.selection_anchor.map(|(c, r)| format!("{c},{r}")),
            "selection_focus": self.selection_focus.map(|(c, r)| format!("{c},{r}")),
            "selection_anchor_row": self.selection_anchor.map(|(_, r)| r),
            "selection_focus_row": self.selection_focus.map(|(_, r)| r),
            "selection_area_x": self.last_selectable_area.get().x,
            "selection_area_y": self.last_selectable_area.get().y,
            "selection_area_w": self.last_selectable_area.get().width,
            "selection_area_h": self.last_selectable_area.get().height,
            "selection_text": self.selection_text.borrow().clone(),
            // Terminal-scroll mode (P5): scenarios assert the mode is
            // live from config and that emission never re-emits (the
            // watermark only advances). insert_before is inert on the
            // headless fullscreen TestBackend, so scrollback history
            // itself is live-verified territory, not corpus territory.
            "terminal_scroll_mode": self.terminal_scroll_mode,
            "scroll_emitted_through": self.scroll_emitted_through.get(),
            "scroll_cls_divider": self.scroll_cls_divider.get()
        })
    }

    /// Push `text` into the live steer queue and return a status string
    /// describing the outcome. The same queue serves both states: while
    /// streaming the agent drains it at the next iteration boundary, while
    /// idle the loop drains it on the next turn — so /steer and /queue
    /// are meaningful either way. Mirrors the live steer path in
    /// adapter_types.rs, but uses `try_lock` because this runs on the
    /// sync slash-command path while the queue is a tokio Mutex.
    /// (iter-240 — wires /steer and /queue `<text>` to the real steer queue.)
    fn queue_steer(&mut self, text: &str) -> String {
        const NO_QUEUE: &str = "Steer is unavailable (no live agent).";
        const QUEUE_BUSY: &str = "Queue is busy (agent is draining it).";
        match self.steer_queue_handle.as_ref() {
            Some(handle) => match handle.try_lock() {
                Ok(mut q) => {
                    q.push(text.to_string());
                    if self.is_streaming {
                        format!("Steer queued: {}", text)
                    } else {
                        format!("Steer queued for the next turn: {}", text)
                    }
                }
                Err(_) => QUEUE_BUSY.to_string(),
            },
            None => NO_QUEUE.to_string(),
        }
    }

    /// Render `/keys`: the full active catalogue grouped by context, with the
    /// chords a terminal is likely to intercept flagged inline.
    pub fn render_keys_report(
        &self,
        registry: &crate::tui::keybindings::KeyBindingRegistry,
        report: &crate::tui::keybindings::KeyUsageReport,
    ) -> String {
        use crate::tui::keybindings::{BindingContext, os_conflict_reason};

        let mut out = format!(
            "Keybindings — {} binding(s) from the registry. /hotkeys shows what you have used.\n",
            registry.catalogue().len()
        );
        let mut current: Option<BindingContext> = None;
        for (context, binding) in registry.catalogue() {
            if current != Some(context) {
                current = Some(context);
                out.push_str(&format!("\n[{}]\n", context.label()));
            }
            let flag = match os_conflict_reason(&binding.chord()) {
                Some(why) => format!("  ⚠ {}", why),
                None => String::new(),
            };
            let description = binding.description.as_deref().unwrap_or("");
            let action = format!("{:?}", binding.action);
            out.push_str(&format!(
                "  {:<16} {:<34} {}{}\n",
                binding.chord(),
                description,
                action,
                flag
            ));
        }
        if !report.conflicts.is_empty() {
            out.push_str(&format!(
                "\n⚠ {} binding(s) may be intercepted by the terminal: {}\n",
                report.conflicts.len(),
                report
                    .conflicts
                    .iter()
                    .map(|(c, _)| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        out
    }

    /// Render `/hotkeys`: chords you have pressed vs chords you never have.
    /// The never-used list is the point — it is the undiscovered surface.
    pub fn render_hotkeys_report(
        &self,
        report: &crate::tui::keybindings::KeyUsageReport,
    ) -> String {
        let used = report.used.len();
        let mut out = format!(
            "Keybinding usage — {used} of {} chord(s) used, {} never used.\n",
            report.total,
            report.never_used.len()
        );

        if used == 0 {
            out.push_str(
                "\nNo chords recorded yet — press a bound key, then run /hotkeys again.\n",
            );
        } else {
            out.push_str("\nYou have used:\n");
            for v in &report.used {
                out.push_str(&format!(
                    "  {:<16} {:>4}x  {}\n",
                    v.chord,
                    v.count,
                    if v.description.is_empty() {
                        v.action.as_str()
                    } else {
                        v.description.as_str()
                    }
                ));
            }
        }

        out.push_str("\nNever used (undiscovered):\n");
        if report.never_used.is_empty() {
            out.push_str("  (none — every bound chord has been pressed)\n");
        } else {
            for v in &report.never_used {
                out.push_str(&format!(
                    "  {:<16} [{}] {}\n",
                    v.chord,
                    v.context.label(),
                    if v.description.is_empty() {
                        v.action.as_str()
                    } else {
                        v.description.as_str()
                    }
                ));
            }
        }
        out
    }

    /// (plan 04) Run a REAL CLI subcommand from the TUI: same binary
    /// (version matches), `pending_shell_command` suspends the alt screen and
    /// inherits stdio, so interactive wizards work and plain output lands in
    /// the terminal the TUI returns to. The subcommand-parity arms
    /// (/cron, /kanban, /curator, /suggestions, /profile, /logs, /pause,
    /// /unpause, /org, /platforms, /mcp <args>, /setup) all funnel here —
    /// one pattern, the actual subcommand, zero re-implementation.
    fn shell_out_to_subcommand(&mut self, subcommand: &str, args: &str) {
        self.close_secondary_views();
        self.dismiss_error_notifications();
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "operant".to_string());
        let mut argv = vec![exe, subcommand.to_string()];
        argv.extend(args.split_whitespace().map(str::to_string));
        self.pending_shell_command = Some(argv);
        self.set_status_notice(format!("Running operant {subcommand}…"));
    }

    /// Implementation that receives both cmd and args. Most slash commands
    /// ignore args; a few (like /personality `<name>`) consume them.
    fn intercept_slash_command_with_args_impl(&mut self, cmd: &str, args: &str) -> bool {
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
            // Alias tier (2026-10-10 command sweep): the registry advertises
            // names whose jcode counterparts are the same surface as an
            // already-handled command — route instead of falling through to
            // the model as literal text.
            "commands" => self.intercept_slash_command_with_args_impl("help", args),
            "models" => self.intercept_slash_command_with_args_impl("model", args),
            "split" | "split-view" => {
                self.intercept_slash_command_with_args_impl("splitview", args)
            }

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
                // (plan 04) bare /mcp keeps the panel; with args it runs the
                // real subcommand (the old arm returned false for any args,
                // so `operant mcp list` typed as /mcp list went to the model).
                if args.trim().is_empty() {
                    let servers = self.load_mcp_servers();
                    self.mcp_view.open(servers);
                } else {
                    self.shell_out_to_subcommand("mcp", args);
                }
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
            "context-files" => {
                // (plan 04) the AGENTS.md/CLAUDE.md selector — renamed from
                // /memory, whose old help ("Toggle memory feature") lied:
                // it selects context files, it does not touch memory.
                let root = self.project_root();
                self.memory_file_selector.open(&root);
                true
            }
            "memory" => {
                self.shell_out_to_subcommand("memory", args);
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
                        self.set_status_notice(format!(
                            "Skill '{}' not found. Use /skills to browse installed skills.",
                            skill_name
                        ));
                        true
                    }
                }
            }
            // /bundle <name> — expand a skill bundle (multiple skills) into
            // the turn. hermes parity: `skill_bundles.py` + slash bundles.
            // The listing forces a fresh scan so bundles added mid-session
            // show up without a restart (mtime-aware cache).
            "bundle" => {
                let name = args.trim();
                if name.is_empty() {
                    let bundles = operant_core::agent::skill_bundle::refresh_skill_bundles();
                    if bundles.is_empty() {
                        self.status_message =
                            Some("No skill bundles found in ~/.operant/skill-bundles/".to_string());
                    } else {
                        let names: Vec<&str> = bundles.keys().map(|k| k.as_str()).collect();
                        self.set_status_notice(format!(
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
                            self.set_status_notice(format!(
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
                if !self.credentials_live() {
                    self.connect_dialog.open();
                    self.set_status_notice("Connect a provider to choose a model.".to_string());
                    return true;
                }
                // (plan 05B) the ported jcode unified inline picker over the
                // agent's endpoint catalogue (05A's rows: base_url /models
                // + the configured [agent] models).
                self.open_model_picker_inline();
                true
            }
            // /model-status — status-tier (command backlog tier 1):
            // one-line model health flash in the status bar. /status keeps
            // the full transcript annotation.
            // (2026-10-14 batch, plan 02) "model-status" and "observe" were
            // dead commands — removed from the registry; their arms went with
            // them. Their status surface lives on via /status and /info.
            "session" | "resume" => {
                self.session_browser.open(vec![]);
                self.session_list_pending = true;
                true
            }
            // /branch — open the session branching overlay (hermes /branch
            // parity). The overlay was previously only reachable via a
            // keybinding; the slash arm makes it discoverable.
            "branch" => {
                // (plan 04) the arm opened the overlay with an EMPTY list
                // (it never set pending_branch_list, so the fork-tree backfill
                // in tui_app.rs never ran). One flag: the real feeder builds
                // parent + child forks from the agent's DB.
                self.pending_branch_list = true;
                true
            }
            // /status — session/model/token summary (hermes /status parity).
            // Rendered as a system annotation so it persists in the
            // transcript instead of flashing in the status bar.
            "status" => {
                let model = if self.model_name.is_empty() {
                    "(unset)".to_string()
                } else {
                    self.model_name.clone()
                };
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "(default)".to_string());
                let title = self
                    .session_title
                    .clone()
                    .unwrap_or_else(|| "(untitled)".to_string());
                let streaming = if self.is_streaming {
                    "streaming"
                } else {
                    "idle"
                };
                let summary = format!(
                    "Session status — model: {model} · provider: {provider} · title: {title} · \
                     messages: {} · tokens: {} · cost: ${:.4} · {streaming}",
                    self.messages.len(),
                    self.token_count,
                    self.cost_usd,
                );
                self.push_system_message(
                    summary.clone(),
                    crate::tui::app::enums::SystemMessageStyle::Info,
                );
                self.set_status_notice(summary);
                true
            }
            // /info — status-tier (command backlog tier 1): compact
            // environment identity flash. /status carries session numbers.
            "info" => {
                let model = if self.model_name.is_empty() {
                    "(unset)".to_string()
                } else {
                    self.model_name.clone()
                };
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "(default)".to_string());
                let title = self
                    .session_title
                    .clone()
                    .unwrap_or_else(|| "(untitled)".to_string());
                let summary = format!(
                    "operant v{} · model: {model} · provider: {provider} · title: {title}",
                    env!("CARGO_PKG_VERSION"),
                );
                // (2026-10-14 batch, plan 10) print once: the transcript
                // annotation is the visible surface; the status-bar echo of
                // the same line duplicated it.
                self.push_system_message(summary, crate::tui::app::enums::SystemMessageStyle::Info);
                true
            }
            // /clear — drop the conversation. The agent-side clear is DEFERRED
            // (plan 03 §2): if a turn is streaming, cancel it first and let
            // the loop-top drain run agent.clear_history() once the run has
            // fully finished — otherwise the stopping turn's tail write
            // re-populates the conversation after the clear.
            "clear" => {
                if self.is_streaming {
                    self.cancel_streaming_turn();
                    self.pending_clear = true;
                    self.set_status_notice("Stopping turn, will clear…".to_string());
                } else {
                    self.pending_clear = true;
                }
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
                self.set_status_notice(format!("Vim mode {}.", status));
                self.refresh_prompt_input();
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
                self.set_status_notice(if self.plan_mode {
                    "Plan mode ON — read-only: gated tools will be denied. /permissions to change."
                        .to_string()
                } else {
                    "Plan mode OFF.".to_string()
                });
                true
            }
            // /stop — cancel the live streaming turn, exactly as Esc and
            // /cancel do: trigger the interrupt flag (the agent commits its
            // tail and returns Error::Interrupted) instead of only flipping
            // the display's is_streaming (plan 03 §1 — the old body lied:
            // the agent kept running while the TUI showed idle, and a prompt
            // typed right after raced two concurrent runs on one agent).
            "stop" => {
                if self.is_streaming {
                    self.cancel_streaming_turn();
                } else {
                    self.set_status_notice("Nothing to stop — no turn is running.".to_string());
                }
                true
            }

            // /new — a fresh session: new session id + empty conversation.
            // Same deferral as /clear (plan 03 §1/§2): the agent-side work
            // (new session id + swap) runs in the loop-top drain once any
            // in-flight turn has fully finished. The old session stays in
            // the store and in /resume.
            "new" | "fresh" => {
                if self.is_streaming {
                    self.cancel_streaming_turn();
                    self.pending_new = true;
                    self.set_status_notice("Stopping turn, starting new session…".to_string());
                } else {
                    self.pending_new = true;
                }
                true
            }

            // /undo [N] — rewind the last N turns through the agent's real
            // rewind machinery (which forks and keeps the old path), when
            // the turn has finished (plan 03 §2 — the old hand-rolled
            // messages.truncate edited only the display, so the agent kept
            // the full context and every later turn "remembered" the
            // undone exchange).
            "undo" => {
                let n: usize = args.trim().parse().unwrap_or(1);
                let n = n.max(1);
                if self.is_streaming {
                    self.status_message =
                        Some("Cannot undo while a turn is running. Stop first.".to_string());
                    return true;
                }
                self.pending_undo = Some(n);
                true
            }

            // /retry — undo the last turn (real rewind) and resubmit its
            // prompt. The undo runs in the drain (plan 03 §2); the resubmit
            // rides the existing pending_retry_query loop-top drain, so the
            // resubmitted turn starts only after the rewind has rebuilt the
            // display.
            "retry" => {
                if self.is_streaming {
                    self.status_message =
                        Some("Cannot retry while a turn is running. Stop first.".to_string());
                    return true;
                }
                self.pending_undo = Some(1);
                self.pending_retry_after_undo = true;
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
                    self.set_status_notice(format!(
                        "Session goal: {}. Use /goal <text> to change.",
                        cur
                    ));
                } else {
                    self.session_goal = Some(trimmed.to_string());
                    // (plan 03 §6) The annotation below is DISPLAY-only —
                    // the agent sees the goal because it is SUBMITTED as a
                    // real message on the next turn, through the same
                    // one-keystroke drain /skill uses. The old comment
                    // claimed injection that never reached the agent.
                    self.pending_user_message = Some(format!("[session goal] {}", trimmed));
                    // Inject as a system annotation so it appears in transcript.
                    self.push_system_message(
                        format!("🎯 Goal set: {}", trimmed),
                        crate::tui::app::SystemMessageStyle::Info,
                    );
                    self.set_status_notice(format!("Goal set and sent to the agent: {trimmed}"));
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

            // /compact — fold older completed exchanges of the agent's stored
            // history (recent turns stay verbatim). Runs after this returns:
            // the agent lives in TuiApp (`run_pending_agent_actions`).
            "compact" => {
                if self.is_streaming {
                    self.set_status_notice(
                        "A turn is running — run /compact after it finishes.".to_string(),
                    );
                } else {
                    self.pending_compact = true;
                    self.set_status_notice("Compacting context…".to_string());
                }
                true
            }
            // /cancel — the Esc-while-streaming stop, as a command.
            "cancel" => {
                if self.is_streaming {
                    self.cancel_streaming_turn();
                } else {
                    self.set_status_notice("Nothing is running.".to_string());
                }
                true
            }
            // /transcript — write the session transcript (markdown) to
            // ~/.operant/transcripts/ and open it in the external editor.
            "transcript" => {
                let dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".operant")
                    .join("transcripts");
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default();
                let path = dir.join(format!("transcript-{stamp}.md"));
                let markdown = crate::tui::export_dialog::export_as_markdown(
                    &self.messages,
                    self.session_title.as_deref(),
                );
                match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, markdown)) {
                    Ok(()) => {
                        let mut argv = crate::tui::external_editor::resolve_command(
                            &self.config.tui.external_editor,
                        );
                        argv.push(path.display().to_string());
                        self.pending_shell_command = Some(argv);
                        self.set_status_notice(format!("Transcript: {}", path.display()));
                    }
                    Err(e) => {
                        self.set_status_notice(format!("Could not write the transcript: {e}"))
                    }
                }
                true
            }
            // /changelog — the newest entries of this build's changelog.
            "changelog" => {
                const CHANGELOG: &str = include_str!("../../../../../docs/CHANGELOG.md");
                let titles: Vec<String> = CHANGELOG
                    .lines()
                    .filter_map(|line| line.strip_prefix("- **"))
                    .map(|rest| {
                        let title = rest
                            .split("**")
                            .next()
                            .unwrap_or(rest)
                            .trim_end_matches(':');
                        format!("• {}", title.trim())
                    })
                    .take(12)
                    .collect();
                self.push_system_message(
                    format!("Recent changes:\n{}", titles.join("\n")),
                    crate::tui::app::enums::SystemMessageStyle::Info,
                );
                true
            }
            // /refresh-model-list — drop the cached catalog and re-fetch the
            // active provider's live model list (the picker opens on it).
            "refresh-model-list" => {
                let cache = dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("operant")
                    .join("models.json");
                let _ = std::fs::remove_file(&cache);
                crate::tui::operant_app::provider::catalog_scheduler::bump_catalog_generation();
                if self.credentials_live() {
                    self.open_model_picker_inline();
                    self.set_status_notice("Refreshing the model catalog…".to_string());
                } else {
                    self.connect_dialog.open();
                    self.set_status_notice("Connect a provider to fetch its models.".to_string());
                }
                true
            }
            // /auth — where credentials come from right now.
            "auth" => {
                let provider = self
                    .active_provider
                    .clone()
                    .unwrap_or_else(|| "(default)".to_string());
                let env_or_config = crate::tui::adapter_types::config::resolve_api_key().is_some();
                let stored = self.auth_store.has_any_key();
                let summary = format!(
                    "Auth: provider {provider} · credentials {} · env/config key: {} · stored keys: {} — /login to add or change keys",
                    if self.credentials_live() {
                        "live"
                    } else {
                        "absent"
                    },
                    if env_or_config { "yes" } else { "no" },
                    if stored { "yes" } else { "no" },
                );
                self.push_system_message(summary, crate::tui::app::enums::SystemMessageStyle::Info);
                true
            }
            // /goals — the autonomous task ledger (`[autonomous] todo_path`,
            // TODO.md): what autonomous mode will work on next.
            "goals" => {
                let path = std::env::current_dir()
                    .unwrap_or_default()
                    .join(&self.config.autonomous.todo_path);
                let text = match std::fs::read_to_string(&path) {
                    Ok(raw) => {
                        let ledger = crate::autonomous::parse_todo_ledger(&raw);
                        let mut lines = vec![format!(
                            "Goals ({}): {} pending · {} implemented",
                            path.display(),
                            ledger.pending.len(),
                            ledger.implemented.len()
                        )];
                        lines.extend(
                            ledger
                                .pending
                                .iter()
                                .take(15)
                                .map(|item| format!("• {item}")),
                        );
                        if ledger.pending.len() > 15 {
                            lines.push(format!("… {} more", ledger.pending.len() - 15));
                        }
                        lines.join("\n")
                    }
                    Err(_) => format!(
                        "No task ledger at {} — autonomous mode is idle. Add a `## Pending` list to it.",
                        path.display()
                    ),
                };
                self.push_system_message(text, crate::tui::app::enums::SystemMessageStyle::Info);
                true
            }
            "copy" => {
                // Copy last assistant message to clipboard. Attempt arboard; fall back to notification.
                let last = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                    .map(|m| m.get_all_text());
                if let Some(text) = last {
                    // One clipboard chain, one report: the outcome names the
                    // mechanism that worked, or every mechanism that did not
                    // and why.
                    let outcome = crate::tui::clipboard::copy(&text);
                    if outcome.is_copied() {
                        self.push_notice(NotificationKind::Info, outcome.status_message());
                    } else {
                        self.push_notice(
                            NotificationKind::Info,
                            format!(
                                "Last response: {} chars. {}",
                                text.len(),
                                outcome.status_message()
                            ),
                        );
                    }
                } else {
                    self.push_notice(
                        NotificationKind::Warning,
                        "No assistant message to copy.".to_string(),
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
                self.set_status_notice(format!("Output style: {}.", self.output_style));
                true
            }
            "effort" => {
                // (plan 05B) the same unified inline picker, effort rows:
                // one row per wire level, the current one flagged and
                // preselected. Enter routes through the plan-03
                // pending_effort drain. The legacy effort_picker overlay
                // is retired with this.
                self.open_effort_picker_inline();
                true
            }
            "effort-set" => {
                // (plan 03) hidden command the picker/model-picker confirm
                // path funnels through: sets the wire value on the live agent
                // via the pending-action channel (same seam as /compact).
                // Args: the effort string ("" = off).
                let value = args.trim().to_ascii_lowercase();
                let valid = ["", "minimal", "low", "medium", "high", "xhigh"];
                if !valid.contains(&value.as_str()) {
                    self.set_status_notice(format!(
                        "Unknown effort '{value}' — use off|minimal|low|medium|high|xhigh"
                    ));
                    return true;
                }
                self.pending_effort = Some(value.clone());
                self.effort_level = match value.as_str() {
                    "minimal" | "low" => crate::tui::model_effort::EffortLevel::Low,
                    "medium" => crate::tui::model_effort::EffortLevel::Normal,
                    "xhigh" => crate::tui::model_effort::EffortLevel::Max,
                    "high" => crate::tui::model_effort::EffortLevel::High,
                    _ => self.effort_level,
                };
                true
            }
            "permissions" => {
                // 2026-10-14 batch, plan 10: the explicit replacement for
                // the removed Shift+Tab permission cycle. Bypass still
                // defers to the confirmation dialog (key handling, not
                // here, so the danger gate cannot be skipped).
                self.permissions_picker
                    .open(self.settings.permission_mode.clone());
                true
            }
            "doctor" => {
                // (plan 04) the REAL health checks, not the registry's
                // display-only handler — and it must intercept: the sweep
                // treats a fall-through as a lie.
                self.shell_out_to_subcommand("doctor", args);
                true
            }
            "rewind" => {
                // (plan 05) `/rewind` opens the selector; `/rewind <n>`
                // targets prompt #n directly — confirm via y/n in the status
                // line before the fork.
                if let Ok(n) = args.trim().parse::<usize>()
                    && n > 0
                {
                    self.pending_rewind_confirm = Some(n);
                    self.set_status_notice(format!("Rewind to before prompt #{n}? (y/n)"));
                } else {
                    self.open_rewind_selector();
                }
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
            // /login, /init — dialog tier (command backlog tier 2): the
            // registry advertises them but the handler returned false, so
            // they fell through to the model as literal prompt text. The
            // connect dialog IS the login surface; route like /connect.
            // /login, /init — (plan 05B B2.7, owner-directed) operant's
            // provider/model backend IS the setup wizard (`operant setup
            // provider` — fuzzy provider select, key prompt, model choice,
            // and since iter-893 a SURGICAL persist). The wizard suspends
            // the TUI and runs in the terminal, exactly the /setup
            // suspend-and-run pattern; the in-TUI connect dialog stays
            // reachable at first-run boot and via /connect.
            "init" | "login" => {
                self.shell_out_to_subcommand("setup", "provider");
                true
            }
            // /logout — the TUI holds no credential material to drop
            // (keys live in ~/.operant/.env or the auth store), so say
            // where they are and route back to the same setup backend
            // instead of pretending a state change happened.
            "logout" => {
                self.set_status_notice(
                    "Keys live in ~/.operant/.env — run /login (the setup wizard) to change the provider or key."
                        .to_string(),
                );
                true
            }
            // /observe — registered as "show the latest tool context in
            // the side panel", but no side panel exists in the ported
            // chrome yet. A registered surface must never fall through to
            // the model; until the panel lands, surface the latest tool
            // invocation (from tool_use_blocks) in the transcript.
            // (2026-10-14 batch, plan 02) "observe" removed — see above.
            // (2026-10-14 batch, plan 10) /keybindings removed — it opened a
            // keybindings.json the TUI never loads (load_keybindings_from_
            // config is a stub); /keys is the real catalogue and /config
            // edits settings.
            // /keys — the active binding catalogue, read from the registry.
            // /hotkeys — the same catalogue split by whether you have pressed
            // it. Distinct from /keybindings above, which edits the JSON file.
            "keys" | "hotkeys" => {
                let registry = &*crate::tui::keybindings::DEFAULT_KEYBINDINGS;
                let _report = registry.usage_report(&self.slash_usage);
                // Flush here so a freshly-pressed chord is always reflected,
                // rather than waiting for the next periodic flush.
                self.slash_usage.save();
                // 2026-10-14 batch, plan 10: the catalogue renders as the
                // help-overlay machinery with Keys/Hotkeys content, opened at
                // the top — not as a bottom-dumped transcript message that
                // opens below the fold.
                if cmd == "keys" {
                    self.help_overlay.open_keys();
                } else {
                    self.help_overlay.open_hotkeys();
                }
                self.show_help = self.help_overlay.visible;
                true
            }
            // `?` is the help alias the registry advertises (and the core
            // def lists under "help"); the 2026-10-14 no-unwired test caught
            // it landing the core fallback. Route it like `commands` does.
            "?" => self.intercept_slash_command_with_args_impl("help", args),
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
                // (plan 03 §3) Arming bypass goes through the confirmation
                // dialog — the same one /permissions uses — and persists
                // only on "Yes, I accept" (arm_bypass_permissions). The old
                // arm wrote settings.json directly with no confirmation.
                // Disarming stays direct: removing a permission needs no
                // confirm.
                if matches!(
                    self.settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                ) {
                    let mut settings =
                        crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                    settings.permission_mode =
                        crate::tui::adapter_types::config::PermissionMode::default();
                    let _ = settings.save_sync();
                    self.settings.permission_mode =
                        crate::tui::adapter_types::config::PermissionMode::default();
                    self.set_status_notice(
                        "YOLO mode disarmed — permissions will prompt.".to_string(),
                    );
                } else {
                    self.bypass_permissions_dialog.show();
                    self.set_status_notice("Confirm bypass permissions in the dialog.".to_string());
                }
                true
            }

            // /busy — toggle "busy" indicator (we map to auto_compact to avoid
            // adding a new state field; busy = compact aggressively).
            // /verbose — cycle output-style between auto/stream/verbose.

            // /reasoning — toggle whether thinking/reasoning blocks are
            // expanded by default in the transcript. (Bug #18 from iter-82
            // audit — previously just printed a status message without
            // toggling anything.)
            "reasoning" => {
                self.show_reasoning = !self.show_reasoning;
                self.invalidate_transcript();
                self.set_status_notice(format!(
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
                    self.set_status_notice(format!(
                        "Current personality: {}. Use /personality <name> to change.",
                        cur
                    ));
                } else {
                    // Set the new personality. agent_mode_changed=true signals
                    // the run loop to update the query config on the next turn.
                    self.agent_mode = Some(trimmed.to_string());
                    self.agent_mode_changed = true;
                    self.set_status_notice(format!("Personality set to: {}.", trimmed));
                }
                true
            }

            // /steer <message> — inject guidance into the live steer queue so
            // the agent picks it up at the next iteration boundary mid-turn.
            // (iter-240 — wires to the real steer_queue_handle backend.)
            "steer" => {
                let notice = if args.is_empty() {
                    "Usage: /steer <message> (inject guidance while the agent is streaming)"
                        .to_string()
                } else {
                    self.queue_steer(args)
                };
                self.set_status_notice(notice);
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
                self.set_status_notice(msg);
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
                self.set_status_notice("Reconnecting MCP servers…".to_string());
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

                self.set_status_notice(if model_changed {
                    "Config reloaded (provider/model changes apply on restart).".to_string()
                } else {
                    "Configuration reloaded.".to_string()
                });
                true
            }

            // /reload-skills — re-scan the skills directory and repopulate the
            // /skills overlay's backing data + the /skill + /bundle Tab
            // typeahead (so newly installed skills/bundles complete without a
            // restart). The running agent was built with a fixed SkillManager
            // at startup (main.rs `with_skill_manager`) and exposes no runtime
            // setter, so rescanned skills reach the model only after a restart;
            // the status stays honest about that. Ref: hermes-agent
            // cli.py reload_skills().
            "reload-skills" => {
                let skills_dir = self.config.skills.root_dir.clone();
                let mut mgr = operant_core::skills::SkillManager::new(skills_dir.clone());
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
                        // Refresh the slash typeahead with fresh skill + bundle
                        // names so /skill <Tab> completes newly-installed
                        // skills without a restart.
                        let skill_names = self
                            .skills_view
                            .skills
                            .iter()
                            .map(|s| s.name.clone())
                            .collect();
                        let bundle_names =
                            operant_core::agent::skill_bundle::refresh_skill_bundles()
                                .keys()
                                .map(|k| k.trim_start_matches('/').to_string())
                                .collect();
                        crate::tui::prompt_input::register_typeahead_names(
                            skill_names,
                            bundle_names,
                        );
                        self.set_status_notice(format!(
                            "Rescanned {} skill{}. Browse with /skills (agent picks up changes on restart).",
                            count,
                            if count == 1 { "" } else { "s" }
                        ));
                    }
                    Err(e) => {
                        self.set_status_notice(format!("Failed to reload skills: {}", e));
                    }
                }
                true
            }

            // /browser — surface that operant has a Camofox browser backend
            // but no in-TUI browser launcher (it's a tool the agent calls).
            // /statusbar — toggle Settings.terminal_progress_bar. The
            // /indicator twin is deleted: two names, one toggle, one row
            // (plan 04 — operant has one status-bar toggle, not the two
            // hermes has).
            "statusbar" => {
                let mut settings =
                    crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
                settings.terminal_progress_bar = !settings.terminal_progress_bar;
                let _ = settings.save_sync();
                self.set_status_notice(format!(
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
                self.set_status_notice(
                    "Mouse capture: enabled (use --no-mouse to disable, e.g. inside tmux)"
                        .to_string(),
                );
                true
            }

            // /terminal-setup — operant maps Enter+Shift to a newline in the
            // prompt (key_handling.rs), but most terminals send a bare \r for
            // Shift+Enter, so crossterm cannot see the difference and the key
            // silently submits instead. Nothing in the app can recover that;
            // it has to be fixed terminal-side, so name the fix for the
            // environment we are actually in.
            "terminal-setup" => {
                self.set_status_notice(crate::tui::terminal_setup::render(
                    &crate::tui::terminal_setup::EnvSnapshot::from_process(),
                ));
                true
            }

            // /redraw — force a full redraw by bumping the transcript
            // version counter (which invalidates cached render state).
            "redraw" => {
                self.transcript_version
                    .set(self.transcript_version.get().wrapping_add(1));
                self.set_status_notice("Screen redrawn.".to_string());
                true
            }

            // /billing, /credits — surface that operant doesn't track
            // provider billing/credits (it's BYOK); point users at /stats
            // for local token usage tracking.
            // /update — point at `operant update` (the TUI can't self-update
            // without restarting).
            "update" => {
                self.set_status_notice(
                    "Run `operant update` from a shell to check for and install a new release."
                        .to_string(),
                );
                true
            }

            // /heapdump, /mem — debug diagnostics; surface a snapshot of
            // turn count + token count + cost as a memory/heap summary.
            // /pet — Easter-egg. (iter-144: rustle pose trigger deleted
            // since the pose system was dead code. Still shows the message.)
            // jcode /cls parity (2026-10-10 command sweep): clear the rendered
            // VIEW only — the model keeps its full context. Distinct from
            // /clear, which drops the conversation. Same machinery as Ctrl+L.
            "cls" | "clear-view" => {
                // Exactly Ctrl+L's sequence: the collapse capture must record
                // the CURRENT transcript version, so no invalidate here —
                // an invalidate after the capture bumps the version and the
                // collapsed state reads as dead.
                self.clear_view_terminal_style();
                // (plan 07 §2) native-scroll mode: the view reset also
                // replays — the divider below marks the boundary in the
                // re-emitted history.
                self.request_native_replay();
                self.scroll_cls_divider.set(true);
                self.status_message =
                    Some("View cleared — context kept (/clear drops context)".to_string());
                true
            }

            // P4-3 (2026-10-09 v2 outline): the band itself is live — the
            // list arrives from the todo tool's results; /todos only chooses
            // whether the pinned card is shown.
            // P4-4 (2026-10-09 v2 outline): jcode's /thinking-display
            // (off|full|current). current = the turn's traces until the next
            // prompt; full = every trace; off = none.
            // "thinking" alias: the registered surface name is /thinking
            // (command registry); the muscle-memory ports cover it.
            "thinking-display" | "thinking" => {
                let mode = args.trim();
                match mode {
                    "off" => {
                        self.thinking_display_on = false;
                        // Live flip: committed reasoning stops rendering
                        // immediately (the shim resolver reads this cell).
                        crate::tui::operant_app::config_shim::replace_reasoning_display_override(
                            Some(crate::tui::operant_app::config_shim::ReasoningDisplayMode::Off),
                        );
                    }
                    "current" => {
                        self.thinking_display_on = true;
                        crate::tui::operant_app::config_shim::replace_reasoning_display_override(
                            Some(
                                crate::tui::operant_app::config_shim::ReasoningDisplayMode::Current,
                            ),
                        );
                    }
                    "full" | "on" => {
                        self.thinking_display_on = true;
                        crate::tui::operant_app::config_shim::replace_reasoning_display_override(
                            Some(crate::tui::operant_app::config_shim::ReasoningDisplayMode::Full),
                        );
                    }
                    "" => {}
                    other => {
                        self.set_status_notice(format!(
                            "Unknown thinking-display mode '{other}' — use off|full|current"
                        ));
                        return true;
                    }
                }
                let summary = format!(
                    "Thinking display: {}",
                    format!(
                        "{:?}",
                        crate::tui::operant_app::config_shim::config()
                            .display
                            .reasoning_display()
                    )
                    .to_lowercase()
                );
                // (plan 08) transient notice only — the transcript echo
                // duplicated the status bar line.
                self.set_status_notice(summary);
                self.invalidate_transcript();
                true
            }
            "todos" | "todo" => {
                self.todos_band_hidden = !self.todos_band_hidden;
                self.set_status_notice(if self.todos_band_hidden {
                    "Todo card hidden".to_string()
                } else if self.todos_card_payload.is_empty() {
                    "Todo card pinned (no todos yet)".to_string()
                } else {
                    "Todo card pinned".to_string()
                });
                self.invalidate_transcript();
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
            // /setup — suspend the TUI and shell out to `operant setup` so the
            // user gets the full interactive wizard. The run loop in
            // TuiApp::run polls pending_shell_command after each frame and,
            // if set, leaves alt screen + raw mode, spawns the command with
            // inherited stdio, waits for it, then re-enters alt screen + raw
            // mode and clears the field.
            "setup" => {
                self.shell_out_to_subcommand("setup", args);
                true
            }

            // (plan 04) Subcommand parity: these arms shell out to the REAL
            // CLI subcommand (same binary, same version) via the /setup
            // suspend-and-run pattern — the output lands in the terminal the
            // TUI returns to. The old typeahead suggested them but every one
            // printed "not yet wired to a handler".
            "cron" | "kanban" | "curator" | "suggestions" | "profile" | "logs" | "pause"
            | "unpause" | "org" => {
                self.shell_out_to_subcommand(cmd, args);
                true
            }
            "platforms" => {
                self.shell_out_to_subcommand("gateway", "status");
                true
            }
            "title" => self.intercept_slash_command_with_args_impl("rename", args),
            // /whoami — show what the agent knows about the user.
            // (P1-9 from UX audit — transparency + trust.)
            "whoami" => {
                // (plan 08) ONE engine: memory-wire, the same store the agent
                // recalls from. The old arm read `~/.operant/memory/` via
                // MemoryStore — a third, phantom path: it said "I don't know
                // much about you yet" while memory-wire held hundreds of rows.
                let opened = operant_core::memory_wire::MemoryWireProvider::new(
                    operant_core::platform::operant_home(),
                );
                match opened {
                    Err(_) => {
                        self.set_status_notice(
                            "Memory store unavailable — run operant doctor for the memory-wire line."
                                .to_string(),
                        );
                    }
                    Ok(provider) => {
                        let hits = provider.list_page(10, 0);
                        if hits.is_empty() {
                            self.set_status_notice(
                                "I don't have any memories yet. Chat with me and I'll start remembering."
                                    .to_string(),
                            );
                        } else {
                            let total = provider
                                .memory_bank_stats()
                                .map(|s| s.memories)
                                .unwrap_or(hits.len());
                            let mut summary = format!(
                                "Here's what I remember (showing 10 of {total} memories):

"
                            );
                            for m in &hits {
                                let preview: String = m
                                    .content
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .chars()
                                    .take(80)
                                    .collect();
                                summary.push_str(&format!(
                                    "  {} {preview}
",
                                    m.id
                                ));
                            }
                            if total > hits.len() {
                                summary.push_str(&format!(
                                    "
  ...and {} more
",
                                    total - hits.len()
                                ));
                            }
                            self.push_system_message(
                                summary,
                                crate::tui::app::SystemMessageStyle::Info,
                            );
                        }
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

    /// Interpret a [`CommandResult`](crate::commands::CommandResult) and apply the corresponding side effect
    /// in the TUI. This is the single dispatch point for all slash commands
    /// that go through the `CommandRegistry`.
    ///
    /// Returns `true` if the command was intercepted (even if it only showed
    /// a message), `false` if it should fall through to the agent.
    fn handle_command_result(&mut self, result: crate::commands::CommandResult) -> bool {
        use crate::commands::CommandResult;
        match result {
            CommandResult::Message(text) => {
                self.push_system_message(text, crate::tui::app::SystemMessageStyle::Info);
                true
            }
            CommandResult::ShellCommand(cmds) => {
                self.pending_shell_command = Some(cmds);
                true
            }
        }
    }

    // NOTE (iter-237 / Phase B1): intentionally NOT derived from
    // `overlay_flags()`. This is a deliberate *subset* of the overlay set
    // (it omits permission_request, rewind_flow, help_overlay,
    // history_search_overlay, global_search,
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
        self.session_browser.close();
        self.session_branching.close();
        self.export_dialog.dismiss();
        self.context_viz.close();
        self.connect_dialog.close();
        self.import_config_picker.close();
        self.import_config_dialog.close();
        self.command_palette.close();
        self.key_input_dialog.close();
        self.custom_provider_dialog.close();
        self.free_mode_dialog.close();
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
        // P4-2: errors are inline transcript cards now — nothing to
        // dismiss; the modal and its scroll state are gone. Kept as a
        // no-op seam for the five call sites that cleared the modal
        // pre-purge.
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

// ── (plan 07 step 5) the alternate-screen guard for full-screen overlays ──
impl App {
    /// Which overlays are "full-screen" in jcode chrome terms: they cover
    /// the whole frame, so in native-scroll mode they must NOT paint into
    /// the scroll region (their rows would duplicate history that is
    /// already in scrollback). The unified inline picker is deliberately
    /// NOT here — it renders inline inside the live region by design
    /// (plan 05B).
    fn full_screen_overlay_open(&self) -> bool {
        self.terminal_scroll_mode
            && self.overlay_flags().iter().any(|(name, visible)| {
                *visible
                    && !matches!(
                        *name,
                        "inline_interactive"
                            | "voice_mode_notice"
                            | "selection_highlight"
                            | "selection_row_cache"
                            | "context_menu"
                            | "command_palette"
                            | "permission_request"
                            | "mcp_approval"
                            | "ask_user_dialog"
                    )
            })
    }

    /// Called once per frame before the draw. Takes the alternate screen
    /// when a full-screen overlay opens under it and leaves it when the
    /// last one closes (requesting a native replay — the transcript
    /// re-emits at the current width, so nothing stale survives the
    /// overlay's cover).
    pub(super) fn sync_overlay_alt_screen<
        B: ratatui::backend::Backend + super::SynchronizedUpdate,
    >(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) {
        let want = self.full_screen_overlay_open();
        if want && !self.overlay_alt_active {
            let _ = terminal.backend_mut().enter_alt_screen();
            let _ = terminal.backend_mut().clear();
            self.overlay_alt_active = true;
        } else if !want && self.overlay_alt_active {
            let _ = terminal.backend_mut().leave_alt_screen();
            self.overlay_alt_active = false;
            // (plan 07 §2) the overlay covered native history rows — the
            // watermark still counts them, but they are stale on screen;
            // replay re-emits the whole transcript at the current width.
            self.request_native_replay();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::adapter_types::config::Settings;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use operant_core::config::AppConfig;

    fn make_app() -> App {
        // `App::new` writes the process-global palette — see the note on the
        // twin helper in `app/tests.rs`.
        let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let config = AppConfig::default();
        let settings = Settings::default();
        let cost_tracker = std::sync::Arc::new(crate::tui::adapter_types::cost::CostTracker::new());
        let command_registry = crate::commands::CommandRegistry::new();
        App::new(config, settings, cost_tracker, command_registry)
    }

    fn make_steer_queue() -> std::sync::Arc<tokio::sync::Mutex<Vec<String>>> {
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()))
    }

    fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // ---- /steer must land in the steer queue in BOTH states ----------------
    // The queue is the whole contract: the loop drains it at the next
    // iteration boundary (run.rs `drain_steers`). Previously the Enter arm
    // was gated on `!is_streaming` while `queue_steer` refused when idle,
    // so /steer was a guaranteed no-op in either state.

    #[test]
    fn steer_should_queue_when_streaming() {
        let mut app = make_app();
        let queue = make_steer_queue();
        app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
        app.is_streaming = true;

        let status = app.queue_steer("focus on the parser");

        assert_eq!(
            queue.try_lock().unwrap().as_slice(),
            ["focus on the parser".to_string()]
        );
        assert_eq!(status, "Steer queued: focus on the parser");
    }

    #[test]
    fn steer_should_apply_when_idle() {
        let mut app = make_app();
        let queue = make_steer_queue();
        app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
        app.is_streaming = false;

        let status = app.queue_steer("use the CLI instead");

        // Same queue — idle, the loop drains it on the next turn.
        assert_eq!(
            queue.try_lock().unwrap().as_slice(),
            ["use the CLI instead".to_string()]
        );
        assert_eq!(
            status,
            "Steer queued for the next turn: use the CLI instead"
        );
    }

    #[test]
    fn steer_should_submit_while_streaming() {
        let mut app = make_app();
        app.is_streaming = true;
        app.set_prompt_text("/steer focus on the parser".to_string());

        // Submit must be reachable mid-turn, otherwise the steering text can
        // never reach `queue_steer` while a turn is streaming.
        assert!(app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE)));
    }

    #[test]
    fn plain_prompt_should_not_submit_while_streaming() {
        let mut app = make_app();
        app.is_streaming = true;
        app.set_prompt_text("keep going".to_string());

        // Opening the gate for /steer must not let ordinary prompts start a
        // second turn while one is already streaming.
        assert!(!app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE)));
    }

    // ── plan 03 §1/§2: the context commands drive the agent ────────────

    /// /stop must trigger the REAL interrupt — the old arm only flipped
    /// is_streaming, so the agent kept running while the TUI showed idle
    /// and a prompt typed right after raced a second run on one agent.
    #[test]
    fn stop_mid_stream_triggers_the_real_interrupt() {
        let mut app = make_app();
        app.is_streaming = true;
        let flag = operant_core::interrupt::InterruptFlag::new();
        app.agent_interrupt = Some(flag.clone());

        assert!(app.handle_tui_command("stop", ""));
        assert!(flag.is_triggered(), "/stop must trigger the interrupt flag");
        assert!(!app.is_streaming);
    }

    /// /clear mid-stream cancels the turn and DEFERS the clear: the
    /// display stays until the drain runs `agent.clear_history()` on a
    /// fully-finished turn, so the stopping turn's tail write cannot
    /// repopulate the conversation after the clear.
    #[test]
    fn clear_mid_stream_cancels_and_defers_to_the_drain() {
        let mut app = make_app();
        app.is_streaming = true;
        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
        app.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text("hello".into()),
        });
        let flag = operant_core::interrupt::InterruptFlag::new();
        app.agent_interrupt = Some(flag.clone());

        assert!(app.handle_tui_command("clear", ""));
        assert!(
            flag.is_triggered(),
            "the running turn must be cancelled first"
        );
        assert!(app.pending_clear, "the clear must be deferred to the drain");
        assert!(
            !app.messages.is_empty(),
            "the display must not lie — it stays until the agent is really cleared"
        );
    }

    /// /clear when idle also defers (same drain, one code path).
    #[test]
    fn clear_when_idle_defers_to_the_drain() {
        let mut app = make_app();
        assert!(app.handle_tui_command("clear", ""));
        assert!(app.pending_clear);
    }

    /// /undo [N] parses N and defers; while streaming it is refused.
    #[test]
    fn undo_parses_n_and_defers() {
        let mut app = make_app();
        assert!(app.handle_tui_command("undo", "2"));
        assert_eq!(app.pending_undo, Some(2));
        assert!(app.handle_tui_command("undo", ""));
        assert_eq!(app.pending_undo, Some(1), "no argument means 1");
    }

    #[test]
    fn undo_while_streaming_is_refused() {
        let mut app = make_app();
        app.is_streaming = true;
        assert!(app.handle_tui_command("undo", ""));
        assert!(app.pending_undo.is_none(), "nothing may be undone mid-turn");
    }

    /// /retry is undo-1 + resubmit, through the same rewind machinery.
    #[test]
    fn retry_defers_into_the_rewind_machinery() {
        let mut app = make_app();
        assert!(app.handle_tui_command("retry", ""));
        assert_eq!(app.pending_undo, Some(1));
        assert!(app.pending_retry_after_undo);
    }

    // ── plan 03 §2: the drain reaches the agent ─────────────────────────

    /// The scripted client the drain tests run against — same shape as the
    /// gateway_runner tests'.
    struct EchoClient;

    fn echo_chat_response(content: &str) -> operant_core::client::ChatResponse {
        use operant_core::client::{Choice, MessageDelta, Role, Usage};
        operant_core::client::ChatResponse {
            id: "resp_echo".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "echo".into(),
            choices: vec![Choice {
                index: 0,
                message: MessageDelta {
                    role: Some(Role::Assistant),
                    content: Some(content.to_string()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
        }
    }

    #[async_trait::async_trait]
    impl operant_core::agent::ModelClient for EchoClient {
        fn provider_name(&self) -> &str {
            "echo"
        }

        async fn chat(
            &self,
            request: operant_core::agent::ChatRequest,
        ) -> operant_core::error::Result<operant_core::client::ChatResponse> {
            let echo = request
                .messages
                .iter()
                .rev()
                .find(|m| m.role == operant_core::client::Role::User)
                // The user's own text: the per-turn `<turn_context>` block
                // (workspace files, recall) rides the latest user message.
                .map(|m| {
                    m.content
                        .rsplit("</turn_context>\n\n")
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .unwrap_or_default();
            Ok(echo_chat_response(&format!("answer-{echo}")))
        }

        async fn chat_streaming(
            &self,
            _request: operant_core::agent::ChatRequest,
        ) -> operant_core::error::Result<
            futures::stream::BoxStream<
                'static,
                operant_core::error::Result<operant_core::agent::StreamChunk>,
            >,
        > {
            Err(operant_core::error::Error::Agent(
                "streaming not used in this test".into(),
            ))
        }
    }

    fn scripted_agent(
        dir: &tempfile::TempDir,
    ) -> std::sync::Arc<operant_core::agent::OperantAgent> {
        let db = std::sync::Arc::new(
            operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
        );
        let cfg = operant_core::agent::AgentConfig {
            stream: false,
            ..operant_core::agent::AgentConfig::default()
        };
        std::sync::Arc::new(operant_core::agent::OperantAgent::new(
            cfg,
            Box::new(EchoClient),
            operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(5)),
            db,
        ))
    }

    #[tokio::test]
    async fn clear_drain_empties_agent_context_and_display() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = scripted_agent(&dir);
        agent.run("q1".into()).await.expect("turn 1");
        agent.run("q2".into()).await.expect("turn 2");
        assert_eq!(
            agent.conversation().await.len(),
            4,
            "two turns = four messages before the clear"
        );

        let mut app = make_app();
        app.pending_clear = true;
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        assert!(
            agent.conversation().await.is_empty(),
            "the AGENT context is cleared"
        );
        assert!(
            app.messages.is_empty(),
            "the display is rebuilt from the agent"
        );
        assert!(!app.pending_clear);
    }

    #[tokio::test]
    async fn resume_drain_retargets_the_agent_to_the_picked_session() {
        // TUI-2: a /resume pick moves the agent onto that session, so the next
        // turn is written to and answered from it.
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = scripted_agent(&dir);

        let mut app = make_app();
        app.pending_switch_session = Some("s-42".to_string());
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        assert_eq!(agent.session_id().as_deref(), Some("s-42"));
        assert!(app.pending_switch_session.is_none());
    }

    #[tokio::test]
    async fn new_drain_mints_a_fresh_session_and_empties_the_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = scripted_agent(&dir);
        agent.run("q1".into()).await.expect("turn 1");
        let old_session = agent.session_id().unwrap_or_default();

        let mut app = make_app();
        app.pending_new = true;
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        let new_session = agent.session_id().unwrap_or_default();
        assert_ne!(
            old_session, new_session,
            "/new must mint a fresh session id"
        );
        assert!(
            new_session.starts_with("sess_"),
            "the fork shape, not an ad-hoc id"
        );
        assert!(agent.conversation().await.is_empty());
        assert!(app.messages.is_empty());
        assert!(!app.pending_new);
    }

    #[tokio::test]
    async fn undo_drain_forks_through_the_rewind_machinery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = scripted_agent(&dir);
        agent.run("first-question".into()).await.expect("turn 1");
        agent.run("second-question".into()).await.expect("turn 2");
        let before = agent.session_id().unwrap_or_default();

        let mut app = make_app();
        app.pending_undo = Some(1);
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        let conv = agent.conversation().await;
        assert_eq!(
            conv.len(),
            2,
            "one turn left after undoing the second — the AGENT context is rewound"
        );
        assert!(conv.iter().any(|m| m.content.contains("first-question")));
        assert!(!conv.iter().any(|m| m.content.contains("second-question")));
        assert_ne!(
            agent.session_id().unwrap_or_default(),
            before,
            "rewind forks — the undone path is kept, not overwritten"
        );
        assert_eq!(
            app.prompt_input.text, "second-question",
            "the undone prompt comes back to the composer"
        );
        assert!(app.pending_undo.is_none());
    }

    #[tokio::test]
    async fn retry_drain_queues_the_resubmission() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = scripted_agent(&dir);
        agent.run("the-question".into()).await.expect("turn 1");

        let mut app = make_app();
        app.pending_undo = Some(1);
        app.pending_retry_after_undo = true;
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        assert_eq!(
            app.pending_retry_query.as_deref(),
            Some("the-question"),
            "the rewound prompt is queued for resubmission, not the composer"
        );
        assert_eq!(
            app.prompt_input.text, "",
            "retry never touches the composer"
        );
    }

    /// The drain must NOT consume the pendings while a turn is still
    /// finishing — the loop-top call retries after the run completes.
    #[tokio::test]
    async fn drain_leaves_pendings_alone_while_a_turn_is_still_stopping() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = scripted_agent(&dir);
        agent.run("q1".into()).await.expect("turn 1");

        let mut app = make_app();
        app.is_streaming = true;
        app.pending_clear = true;
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        assert!(
            app.pending_clear,
            "still streaming — the clear must stay deferred"
        );
        assert!(!agent.conversation().await.is_empty());
    }

    /// (plan 03 §6) /goal's claim "so the agent sees it" was display-only;
    /// the goal now rides pending_user_message — the same drain /skill
    /// uses — so the agent receives it as a real message next turn.
    #[test]
    fn goal_submission_reaches_the_agent_context() {
        let mut app = make_app();
        assert!(app.handle_tui_command("goal", "ship the release"));
        assert_eq!(
            app.pending_user_message.as_deref(),
            Some("[session goal] ship the release"),
            "the goal must be queued for the agent, not just the status bar"
        );
        assert_eq!(app.session_goal.as_deref(), Some("ship the release"));
        // No-args shows the goal; the queued message stays queued.
        assert!(app.handle_tui_command("goal", ""));
        assert_eq!(
            app.pending_user_message.as_deref(),
            Some("[session goal] ship the release"),
            "showing the goal must not consume the queued submission"
        );
    }

    // ── plan 03 §3/§4: the permission decision path ────────────────────

    use crate::tui::adapter_types::config::PermissionMode;
    use operant_core::agent::{ToolPermissionRequest, ToolPermissionResponse};

    fn permission_req(
        tool: &str,
        preview: Option<&str>,
    ) -> (
        ToolPermissionRequest,
        tokio::sync::oneshot::Receiver<ToolPermissionResponse>,
    ) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (
            ToolPermissionRequest {
                tool_name: tool.to_string(),
                tool_id: format!("tool-{tool}"),
                description: "test request".to_string(),
                danger_explanation: String::new(),
                input_preview: preview.map(str::to_string),
                seat_escalation: None,
                unattended: false,
                response_tx: tx,
            },
            rx,
        )
    }

    /// Bypass mode IS the decision: AllowSession, no dialog. Pre-§3 the
    /// mode had zero readers on the decision path — it still prompted.
    #[tokio::test]
    async fn bypass_mode_auto_allows_without_a_dialog() {
        let mut app = make_app();
        app.settings.permission_mode = PermissionMode::BypassPermissions;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("rm -rf /tmp/thing"));
        tx.send(req).await.expect("send");
        app.permission_rx = Some(rx);

        app.drain_permission_requests();
        assert_eq!(
            resp_rx.try_recv(),
            Ok(ToolPermissionResponse::AllowSession),
            "bypass must answer AllowSession itself"
        );
        assert!(
            app.permission_request.is_none(),
            "bypass must not open a dialog"
        );
    }

    /// Plan mode is read-only: Deny, no dialog, a visible notice.
    #[tokio::test]
    async fn plan_mode_denies_without_a_dialog() {
        let mut app = make_app();
        app.settings.permission_mode = PermissionMode::Plan;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("cargo build"));
        tx.send(req).await.expect("send");
        app.permission_rx = Some(rx);

        app.drain_permission_requests();
        assert_eq!(
            resp_rx.try_recv(),
            Ok(ToolPermissionResponse::Deny),
            "plan mode must deny gated tools itself"
        );
        assert!(
            app.permission_request.is_none(),
            "plan mode must not open a dialog"
        );
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("read-only")),
            "the denial must be visible"
        );
    }

    /// (plan 05A) a model pick must reach the AGENT and persist — the old
    /// confirm path updated only the display copy, so requests kept using
    /// the old model forever.
    #[tokio::test]
    async fn model_pick_drain_switches_the_agent_and_persists() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A real config file so persist_config_key has a target.
        let cfg_path = dir.path().join("operant.toml");
        std::fs::write(
            &cfg_path,
            "[agent]\nmodel = \"old-model\"\n\n[client]\nbase_url = \"http://localhost:9/v1\"\n",
        )
        .expect("write config");
        let loaded = operant_core::config::load_app_config(Some(&cfg_path)).expect("load");
        operant_core::config::install_runtime_config(loaded.config.clone());

        let agent = scripted_agent(&dir);
        assert_eq!(
            agent.model(),
            "old-model",
            "the scripted agent starts on the configured model"
        );

        let mut app = make_app();
        app.config = loaded.config;
        app.pending_model = Some("small-stack".to_string());
        crate::tui::adapter_types::tui_app::TuiApp::run_pending_agent_actions(
            &mut app,
            Some(&agent),
        )
        .await;

        assert_eq!(
            agent.model(),
            "small-stack",
            "the AGENT must switch — requests read agent.model()"
        );
        assert_eq!(
            app.model_name, "small-stack",
            "display refreshed from the agent"
        );
        assert_eq!(app.config.agent.model, "small-stack");
        assert!(app.pending_model.is_none(), "consumed");
        let persisted = std::fs::read_to_string(&cfg_path).expect("re-read");
        assert!(
            persisted.contains("small-stack"),
            "the pick persists through the surgical writer: {persisted}"
        );
        assert!(
            !persisted.contains("[pk]"),
            "the whole-file rewrite must be gone"
        );
    }

    /// Two requests in one batch: the second QUEUES, it is not silently
    /// denied. The dialog-pop order follows.
    #[tokio::test]
    async fn a_second_request_while_a_dialog_is_open_queues_instead_of_denying() {
        let mut app = make_app();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req1, mut resp1) = permission_req("bash", Some("cargo test --release"));
        let (req2, mut resp2) = permission_req("file_write", None);
        tx.send(req1).await.expect("send 1");
        tx.send(req2).await.expect("send 2");
        app.permission_rx = Some(rx);

        app.drain_permission_requests();
        assert!(
            app.permission_request.is_some(),
            "the first request opens the dialog"
        );
        assert_eq!(
            app.pending_permission_queue.len(),
            1,
            "the second request queues, never denies"
        );
        assert!(
            resp2.try_recv().is_err(),
            "no answer may be sent for the queued request yet"
        );
        assert!(resp1.try_recv().is_err(), "first request still unanswered");

        // Dialog closes (its own path sends the answer) — the queued one
        // opens next.
        app.permission_request = None;
        assert!(app.open_next_permission_request());
        assert!(
            app.permission_request.is_some(),
            "the queued request opens after the dialog closes"
        );
        assert!(app.pending_permission_queue.is_empty());
    }

    /// (plan 03 §4) the two-token prefix allowlist: plain continuations
    /// auto-approve; a different command, an injected tail, or a dangerous
    /// shape always reaches the dialog.
    #[tokio::test]
    async fn bash_prefix_allows_plain_continuations_only() {
        let mut app = make_app();
        app.bash_prefix_allowlist.insert("git status".to_string());
        app.bash_prefix_allowlist.insert("ls".to_string());

        // Plain continuation of a two-token entry → auto.
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("git status --short"));
        tx.send(req).await.expect("send");
        app.permission_rx = Some(rx);
        app.drain_permission_requests();
        assert_eq!(
            resp_rx.try_recv(),
            Ok(ToolPermissionResponse::AllowSession),
            "a stored two-token prefix covers its plain continuations"
        );
        assert!(app.permission_request.is_none());

        // Single-token legacy entry → plain continuation auto.
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("ls -la /tmp"));
        tx.send(req).await.expect("send");
        app.permission_rx = Some(rx);
        app.drain_permission_requests();
        assert_eq!(
            resp_rx.try_recv(),
            Ok(ToolPermissionResponse::AllowSession),
            "a single-token entry covers its first token"
        );

        // Different command under the same first token → dialog, never auto.
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("git push --force"));
        tx.send(req).await.expect("send");
        app.permission_rx = Some(rx);
        app.drain_permission_requests();
        assert!(
            resp_rx.try_recv().is_err(),
            "git push --force must never auto-approve on a git-status entry"
        );
        assert!(
            app.permission_request.is_some(),
            "it opens the dialog instead"
        );
        app.permission_request = None;

        // Injection tail on a matching entry → dialog.
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("git status; rm -rf /"));
        tx.send(req).await.expect("send");
        app.permission_rx = Some(rx);
        app.drain_permission_requests();
        assert!(
            resp_rx.try_recv().is_err(),
            "a metacharacter tail must never auto-approve"
        );
        assert!(app.permission_request.is_some(), "it opens the dialog");
        app.permission_request = None;

        // Dangerous shape even with a matching single-token entry → dialog.
        let mut app2 = make_app();
        app2.bash_prefix_allowlist.insert("rm".to_string());
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (req, mut resp_rx) = permission_req("bash", Some("rm -rf /"));
        tx.send(req).await.expect("send");
        app2.permission_rx = Some(rx);
        app2.drain_permission_requests();
        assert!(
            resp_rx.try_recv().is_err(),
            "a dangerous shape must never auto-approve, even on a matching entry"
        );
        assert!(app2.permission_request.is_some(), "it opens the dialog");
    }
}
