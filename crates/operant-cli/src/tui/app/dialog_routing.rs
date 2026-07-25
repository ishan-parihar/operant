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
        if self.tasks_overlay.visible {
            return Some(DialogPriority::Tasks);
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
            Some('Y') | Some('p') | Some('P') => {
                operant_core::agent::ToolPermissionResponse::AllowSession
            }
            // 'n' (deny), None (no options), or any unmatched key → Deny.
            Some('n') | None => operant_core::agent::ToolPermissionResponse::Deny,
            Some(_) => operant_core::agent::ToolPermissionResponse::Deny,
        };
        if let Some(tx) = tx {
            let _ = tx.send(response);
        }
        self.permission_request = None;
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
                if let Some(pr) = self.permission_request.as_mut() {
                    if let Some(idx) = pr.options.iter().position(|o| o.key == 'n') {
                        pr.selected_option = idx;
                    }
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
