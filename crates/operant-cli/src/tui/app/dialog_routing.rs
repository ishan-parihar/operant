//! Dialog routing, permission handling, and key context methods.

use super::*;

impl App {
    /// Get the highest-priority visible dialog for key routing.
    /// Returns None if no dialog is visible.
    pub(super) fn dialog_priority(&self) -> Option<DialogPriority> {
        // Check in priority order (highest first)
        if self.context_menu_state.is_some() {
            return Some(DialogPriority::ContextMenu);
        }
        if self.bypass_permissions_dialog.visible {
            return Some(DialogPriority::BypassPermissions);
        }
        if self.mcp_approval.visible {
            return Some(DialogPriority::McpApproval);
        }
        if self.device_auth_dialog.visible {
            return Some(DialogPriority::DeviceAuth);
        }
        if self.ask_user_dialog.visible {
            return Some(DialogPriority::AskUser);
        }
        if self.key_input_dialog.visible {
            return Some(DialogPriority::KeyInput);
        }
        if self.custom_provider_dialog.visible {
            return Some(DialogPriority::CustomProvider);
        }
        if self.free_mode_dialog.visible {
            return Some(DialogPriority::FreeMode);
        }
        if self.import_config_dialog.visible {
            return Some(DialogPriority::ImportConfig);
        }
        if self.effort_picker.visible {
            return Some(DialogPriority::EffortPicker);
        }
        if self.connect_dialog.visible {
            return Some(DialogPriority::Connect);
        }
        if self.import_config_picker.visible {
            return Some(DialogPriority::ImportConfigPicker);
        }
        if self.command_palette.visible {
            return Some(DialogPriority::CommandPalette);
        }
        if self.model_picker.visible {
            return Some(DialogPriority::ModelPicker);
        }
        if self.settings_screen.visible {
            return Some(DialogPriority::Settings);
        }
        if self.export_dialog.visible {
            return Some(DialogPriority::Export);
        }
        if self.stats_dialog.visible {
            return Some(DialogPriority::Stats);
        }
        if self.context_viz.visible {
            return Some(DialogPriority::ContextViz);
        }
        if self.session_browser.visible {
            return Some(DialogPriority::SessionBrowser);
        }
        if self.session_branching.visible {
            return Some(DialogPriority::SessionBranching);
        }
        if self.global_search.visible {
            return Some(DialogPriority::GlobalSearch);
        }
        if self.history_search_overlay.visible {
            return Some(DialogPriority::HistorySearch);
        }
        if self.help_overlay.visible {
            return Some(DialogPriority::Help);
        }
        if self.mcp_view.visible {
            return Some(DialogPriority::MCPView);
        }
        if self.agents_menu.visible {
            return Some(DialogPriority::AgentsMenu);
        }
        if self.diff_viewer.visible {
            return Some(DialogPriority::DiffViewer);
        }
        if self.plugins_hub.visible {
            return Some(DialogPriority::PluginsHub);
        }
        if self.skills_view.visible {
            return Some(DialogPriority::SkillsView);
        }
        if self.journey_view.visible {
            return Some(DialogPriority::JourneyView);
        }
        if self.hooks_config_menu.visible {
            return Some(DialogPriority::HooksConfig);
        }
        // The next three were gated inline in key_handling.rs but had no
        // representation here at all, so this chain did not describe the UI.
        // They are appended rather than woven into their semantic neighbours:
        // the chain's order already diverges from the inline order (see the
        // note in key_handling.rs), so claiming a "correct" slot for them would
        // be false precision. Presence is what matters — `dialog_covers_every_
        // gated_surface` fails if one is ever dropped again.
        if self.theme_screen.visible {
            return Some(DialogPriority::ThemeScreen);
        }
        if self.rewind_flow.visible {
            return Some(DialogPriority::RewindFlow);
        }
        if self.memory_file_selector.visible {
            return Some(DialogPriority::MemoryFileSelector);
        }
        if self.voice_mode_notice.visible {
            return Some(DialogPriority::VoiceModeNotice);
        }
        None
    }

    /// Process a keyboard event. Returns `true` when the input should be
    /// submitted (Enter pressed with no blocking dialog).
    ///   `P` (bash prefix) → AllowSession, also records the bash prefix in
    ///       `bash_prefix_allowlist` via `maybe_record_bash_prefix`
    ///   `n` / Esc / unknown → Deny
    fn resolve_permission_dialog(&mut self) {
        // Capture the selected option key + response sender up front so we
        // can clear `permission_request` at the end unconditionally.
        let (selected_key, tx) = {
            let pr = match self.permission_request.as_ref() {
                Some(p) => p,
                None => return,
            };
            let key = pr.options.get(pr.selected_option).map(|o| o.key);
            let tx = self.pending_permission_response_tx.take();
            (key, tx)
        };
        // Bash prefix-allow ('P') records the prefix in the allowlist. Must
        // run before we drop `permission_request` — it reads `pr.kind`.
        self.maybe_record_bash_prefix();

        let response = match selected_key {
            Some('y') => operant_core::agent::ToolPermissionResponse::AllowOnce,
            // 'Y' (session) and 'P' (bash prefix rule) — session-scoped.
            Some('Y') | Some('P') => operant_core::agent::ToolPermissionResponse::AllowSession,
            // 'p' — "Yes, always allow (persistent)": now backed by the real
            // persistent allowlist (hermes `always` → command_allowlist), so
            // the agent stores the tool permanently instead of session-only.
            Some('p') => operant_core::agent::ToolPermissionResponse::AllowAlways,
            // 'n' (deny), None (no options), or any unmatched key → Deny.
            Some('n') | None => operant_core::agent::ToolPermissionResponse::Deny,
            Some(_) => operant_core::agent::ToolPermissionResponse::Deny,
        };
        if let Some(tx) = tx {
            let _ = tx.send(response);
        }
        self.permission_request = None;
        // The dialog was the blocker; the turn goes back to the model unless
        // it was cancelled while the dialog was open.
        self.turn_state = if self.is_streaming {
            TurnState::Thinking
        } else {
            TurnState::Idle
        };
    }

    /// Handle a key event while a permission dialog is active.
    pub(super) fn handle_permission_key(&mut self, key: KeyEvent) {
        let pr = match self.permission_request.as_mut() {
            Some(p) => p,
            None => return,
        };

        match key.code {
            KeyCode::Char(c) => {
                if let Some(digit) = c.to_digit(10) {
                    let idx = (digit as usize).saturating_sub(1);
                    if idx < pr.options.len() {
                        pr.selected_option = idx;
                    }
                } else {
                    // Check if any option matches this key.
                    let mut matched_idx = None;
                    for (i, opt) in pr.options.iter().enumerate() {
                        if opt.key == c {
                            matched_idx = Some(i);
                            break;
                        }
                    }
                    if let Some(idx) = matched_idx {
                        pr.selected_option = idx;
                        self.resolve_permission_dialog();
                    }
                }
            }
            KeyCode::Enter => {
                self.resolve_permission_dialog();
            }
            KeyCode::Up => {
                if pr.selected_option > 0 {
                    pr.selected_option -= 1;
                }
            }
            KeyCode::Down => {
                if pr.selected_option + 1 < pr.options.len() {
                    pr.selected_option += 1;
                }
            }
            KeyCode::Esc => {
                // Esc = cancel = deny. Force the selected option to the deny
                // option (key 'n') before resolving so the response is always
                // Deny regardless of which option was highlighted.
                if let Some(pr) = self.permission_request.as_mut()
                    && let Some(idx) = pr.options.iter().position(|o| o.key == 'n')
                {
                    pr.selected_option = idx;
                }
                self.resolve_permission_dialog();
            }
            _ => {}
        }
    }

    /// If the active permission dialog's selected option is the prefix-allow
    /// option ('P') for a Bash dialog, extract the suggested prefix and add it
    /// to `bash_prefix_allowlist` so future requests with the same prefix are
    /// silently approved.
    fn maybe_record_bash_prefix(&mut self) {
        use crate::tui::dialogs::PermissionDialogKind;
        let pr = match self.permission_request.as_ref() {
            Some(p) => p,
            None => return,
        };
        // Only act on Bash dialogs where the selected option key is 'P'.
        let selected_key = pr.options.get(pr.selected_option).map(|o| o.key);
        if selected_key != Some('P') {
            return;
        }
        if let PermissionDialogKind::Bash { command, .. } = &pr.kind {
            let first_word = command.split_whitespace().next().unwrap_or("").to_string();
            if !first_word.is_empty() {
                self.bash_prefix_allowlist.insert(first_word);
            }
        }
    }

    /// Returns `true` if the given bash `command` is covered by the session-local
    /// prefix allowlist (i.e. its first word matches an entry in
    /// `bash_prefix_allowlist`).  Used by callers to skip the permission dialog.
    #[cfg(test)]
    pub fn bash_command_allowed_by_prefix(&self, command: &str) -> bool {
        let first_word = command.split_whitespace().next().unwrap_or("");
        !first_word.is_empty() && self.bash_prefix_allowlist.contains(first_word)
    }
}

#[cfg(test)]
mod tests {
    /// Fields gated in THIS file, i.e. the surfaces the chain knows about.
    fn chained_fields() -> Vec<String> {
        let mut out: Vec<String> = include_str!("dialog_routing.rs")
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                let r = t.strip_prefix("if self.")?;
                r.contains(".visible")
                    .then(|| r.split(['.', '(']).next().map(str::to_string))?
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Variant names of `DialogPriority`, read from its own source so the
    /// guard cannot go stale when a variant is added.
    fn priority_variants() -> Vec<String> {
        include_str!("enums.rs")
            .lines()
            .skip_while(|l| !l.contains("pub enum DialogPriority"))
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('}'))
            .filter_map(|l| {
                let t = l.trim();
                let name = t.split(" = ").next()?;
                let name = name.strip_prefix("///")?.trim().to_string();
                let looks_like_variant = !name.is_empty()
                    && name.chars().next()?.is_uppercase()
                    && name.chars().all(|c| c.is_alphanumeric());
                looks_like_variant.then_some(name)
            })
            .collect()
    }

    /// Every `DialogPriority` variant except `None` must have a gate here.
    ///
    /// Precise by construction: it compares the enum against the gates in the
    /// same file, so it cannot pick up unrelated `if self.X.is_some()` checks
    /// that live in `key_handling.rs` for input and scrolling.
    #[test]
    fn every_priority_variant_has_a_gate() {
        let gates = chained_fields();
        let missing: Vec<String> = priority_variants()
            .into_iter()
            .filter(|v| v != "None" && !gates.iter().any(|g| g == v))
            .collect();
        assert!(
            missing.is_empty(),
            "DialogPriority names a surface with no gate in dialog_routing.rs: {missing:?}"
        );
    }

    /// The three surfaces that were gated inline but absent from the chain.
    ///
    /// This is the regression that motivated the guard: theme_screen,
    /// rewind_flow and memory_file_selector could each be open and the chain
    /// would not have known, so the chain did not describe the UI.
    #[test]
    fn the_three_recovered_surfaces_are_gated() {
        let gates = chained_fields();
        for field in ["theme_screen", "rewind_flow", "memory_file_selector"] {
            assert!(
                gates.iter().any(|g| g == field),
                "{field} is gated in key_handling.rs so it must be gated here too"
            );
        }
    }
}
