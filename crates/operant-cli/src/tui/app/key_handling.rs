//! Key event handling methods.

use super::*;

/// How many counted key presses between flushes of the usage store.
///
/// The store is already written on every slash command, so this only bounds
/// the loss window for a long session that never runs one.
const KEY_USAGE_FLUSH_EVERY: u32 = 64;

// ---- 2026-10-14 batch, plan 03: effort wire mapping ----

/// The agent's wire value for a UI effort level. The picker's four levels
/// map onto the request field's ladder; "" (empty) is only reachable via
/// `effort-set off` (Shift+Tab's cycle-off position).
fn effort_wire_value(level: crate::tui::model_effort::EffortLevel) -> String {
    use crate::tui::model_effort::EffortLevel;
    match level {
        EffortLevel::Low => "low".to_string(),
        EffortLevel::Normal => "medium".to_string(),
        EffortLevel::High => "high".to_string(),
        EffortLevel::Max => "xhigh".to_string(),
    }
}

impl App {
    /// Shift+Tab (plan 03 decision 1; plan 10 cleared the permission cycle
    /// off this key): cycle effort off → low → medium → high → xhigh → off,
    /// forwarding each step to the agent via the pending-action channel.
    pub(crate) fn cycle_effort(&mut self) {
        use crate::tui::model_effort::EffortLevel;
        let (next_level, wire) = match self.effort_level {
            EffortLevel::Low => (EffortLevel::Normal, "medium".to_string()),
            EffortLevel::Normal => (EffortLevel::High, "high".to_string()),
            EffortLevel::High => (EffortLevel::Max, "xhigh".to_string()),
            EffortLevel::Max => (EffortLevel::Low, String::new()),
        };
        self.effort_level = next_level;
        self.pending_effort = Some(wire);
        let label = if self.pending_effort.as_deref().is_some_and(|w| w.is_empty()) {
            "off".to_string()
        } else {
            next_level.label().to_string()
        };
        self.set_status_notice(format!("Effort: {label}"));
    }
}

impl App {
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // ── F12: toggle debug overlay (highest priority, never blocked) ──
        if key.code == KeyCode::F(12) {
            self.debug_hub.toggle_overlay();
            self.debug_hub.publish(crate::tui::debug::TuiEvent::Key {
                code: "F12".into(),
                modifiers: 0,
                at: crate::tui::debug::event_bus::now_secs(),
            });
            return false;
        }

        // ── (plan 05B) the unified inline picker owns the keyboard while it
        // is open — nav/filter/Enter/Esc, exactly jcode's semantics — before
        // any other surface (the composer must not eat its letters as
        // filter text).
        if self.inline_interactive.is_some() {
            if self.handle_inline_interactive_key(key.code, key.modifiers) {
                return false;
            }
        }

        // Publish key event to debug bus (no-op when disabled).
        self.debug_hub.publish(crate::tui::debug::TuiEvent::Key {
            code: format!("{:?}", key.code),
            modifiers: key.modifiers.bits(),
            at: crate::tui::debug::event_bus::now_secs(),
        });

        // Count this press against the binding registry (feeds /hotkeys).
        // Placed here so every accepted key is counted exactly once, ahead of
        // the dispatcher below, including presses consumed by an overlay.
        self.record_keybinding_usage(&key);

        // Phase 3.3: Priority-based dialog handling.
        //
        // `dialog_priority()` is NOT the router, and the inline `if
        // state.visible` gates below are. The two orders DIVERGE: an empirical
        // probe (open two dialogs, press the key each should get) disagreed on
        // 5 of 6 pairs. The previous comment here claimed the inline handlers
        // "already follow the correct priority order". That was false, and it
        // is the sentence that would make the next reader trust the chain.
        //
        // So, deliberately, this stays a debug assertion. Converting the chain
        // into the real router, or reordering these gates to match it, are both
        // behaviour changes with no bug report and no failing test justifying
        // them. Both were considered and deferred; see
        // docs/ROADMAP-TUI-FLEET.md section 6.1.
        //
        // What WAS fixed: three surfaces were gated here but absent from the
        // chain entirely (theme_screen, rewind_flow, memory_file_selector), so
        // the chain did not describe the UI. They are now present, and
        // `dialog_covers_every_gated_surface` fails if one is ever dropped again.
        let _priority = self.dialog_priority();

        if self.global_search.visible {
            return self.handle_global_search_key(key);
        }

        // ---- Context menu handling (highest priority for menu navigation) ----
        if self.context_menu_state.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.dismiss_context_menu();
                    return false;
                }
                KeyCode::Up | KeyCode::Down => {
                    self.navigate_context_menu(key.code);
                    return false;
                }
                KeyCode::Enter => {
                    self.execute_context_menu_item();
                    return false;
                }
                _ => {}
            }
        }

        // Bypass-permissions dialog: highest-priority gate — user must accept or the
        // session exits immediately. Mirrors TS BypassPermissionsModeDialog.tsx.
        if self.bypass_permissions_dialog.visible {
            match key.code {
                KeyCode::Char('1') | KeyCode::Esc => {
                    // "No" — decline; close and stay in the current mode.
                    self.bypass_permissions_dialog.dismiss();
                }
                KeyCode::Char('2') => {
                    // "Yes, I accept" — arm bypass-permissions and continue.
                    self.arm_bypass_permissions();
                    self.set_status_notice(
                        "Bypass permissions mode enabled — permissions will be auto-approved. Use with care.".to_string(),
                    );
                    self.bypass_permissions_dialog.dismiss();
                }
                KeyCode::Up | KeyCode::Char('k') => self.bypass_permissions_dialog.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.bypass_permissions_dialog.select_next(),
                KeyCode::Enter => {
                    if self.bypass_permissions_dialog.is_accept_selected() {
                        self.arm_bypass_permissions();
                        self.set_status_notice(
                            "Bypass permissions mode enabled — permissions will be auto-approved. Use with care.".to_string(),
                        );
                    }
                    self.bypass_permissions_dialog.dismiss();
                }
                _ => {}
            }
            return false;
        }

        // Permissions picker (/permissions) — 2026-10-14 batch, plan 10.
        // Enter applies the highlighted mode; Bypass does NOT arm here —
        // it routes through the bypass-permissions confirmation dialog,
        // the same explicit gate the removed Shift+Tab cycle skipped.
        if self.permissions_picker.visible {
            use crate::tui::adapter_types::config::PermissionMode;
            match key.code {
                KeyCode::Esc => self.permissions_picker.close(),
                KeyCode::Up | KeyCode::Char('k') => self.permissions_picker.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.permissions_picker.select_next(),
                KeyCode::Enter => {
                    let chosen = self.permissions_picker.current();
                    self.permissions_picker.close();
                    if chosen == PermissionMode::BypassPermissions {
                        self.bypass_permissions_dialog.show();
                    } else {
                        // Label first — `chosen` moves into the setting below.
                        let label = match &chosen {
                            PermissionMode::Default => "Default permissions",
                            PermissionMode::AcceptEdits => "Accept-edits mode",
                            PermissionMode::BypassPermissions => {
                                unreachable!("bypass routes through the confirmation dialog")
                            }
                            PermissionMode::Plan => "Plan mode",
                        };
                        self.settings.permission_mode = chosen;
                        self.set_status_notice(label.to_string());
                    }
                }
                _ => {}
            }
            return false;
        }

        // Device code / browser auth dialog (GitHub Copilot, Anthropic OAuth)
        if self.device_auth_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.device_auth_dialog.close();
                    self.device_auth_pending = None;
                }
                _ if matches!(
                    self.device_auth_dialog.status,
                    crate::tui::device_auth_dialog::DeviceAuthStatus::Success(_)
                ) =>
                {
                    // Any key after success -> store credential and close
                    if let crate::tui::device_auth_dialog::DeviceAuthStatus::Success(ref token) =
                        self.device_auth_dialog.status
                    {
                        let provider_id = self.device_auth_dialog.provider_id.clone();
                        let provider_name = self.device_auth_dialog.provider_name.clone();
                        let token = token.clone();
                        let credential = if provider_id == "github-copilot" {
                            crate::tui::adapter_types::StoredCredential::OAuthToken {
                                access: token.clone(),
                                refresh: token,
                                expires: 0,
                            }
                        } else {
                            crate::tui::adapter_types::StoredCredential::ApiKey { key: token }
                        };
                        self.auth_store.set(&provider_id, credential);
                        self.device_auth_pending = None;
                        self.device_auth_dialog.close();
                        self.activate_provider(provider_id, provider_name, "Connected to");
                        return false;
                    }
                }
                _ if matches!(
                    self.device_auth_dialog.status,
                    crate::tui::device_auth_dialog::DeviceAuthStatus::Error(_)
                ) =>
                {
                    // Any key after error -> close
                    self.device_auth_dialog.close();
                    self.device_auth_pending = None;
                }
                _ => {} // Ignore other keys while waiting
            }
            return false;
        }

        // API key input dialog (opened from /connect for key-based providers)
        // Ask-user question dialog (AskUserQuestion tool)
        if self.ask_user_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.ask_user_dialog.dismiss();
                }
                KeyCode::Enter => {
                    self.ask_user_dialog.confirm();
                }
                KeyCode::Up | KeyCode::BackTab => {
                    self.ask_user_dialog.select_prev();
                }
                KeyCode::Down | KeyCode::Tab => {
                    self.ask_user_dialog.select_next();
                }
                KeyCode::Char(c)
                    if c.is_ascii_digit()
                        && self.ask_user_dialog.options.is_some()
                        && !self.ask_user_dialog.in_custom_input =>
                {
                    // Digit keys select an option by number ONLY when the user
                    // is not already typing a custom answer.  Once in custom
                    // mode, digits flow through to push_char like any other char.
                    let n = (c as u8 - b'0') as usize;
                    if n >= 1 {
                        self.ask_user_dialog.select_by_number(n);
                    }
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.ask_user_dialog.push_char(c);
                }
                KeyCode::Backspace => {
                    self.ask_user_dialog.pop_char();
                }
                _ => {}
            }
            return false;
        }

        if self.key_input_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.key_input_dialog.close();
                }
                KeyCode::Enter => {
                    let provider_id = self.key_input_dialog.provider_id.clone();
                    let provider_name = self.key_input_dialog.provider_name.clone();
                    let api_key = self.key_input_dialog.take_key();
                    if !api_key.is_empty() {
                        self.auth_store.set(
                            &provider_id,
                            crate::tui::adapter_types::StoredCredential::ApiKey { key: api_key },
                        );
                        self.activate_provider(provider_id, provider_name, "Connected to");
                    }
                }
                KeyCode::Backspace => {
                    self.key_input_dialog.backspace();
                }
                KeyCode::Char('v')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    if let Some(text) = crate::image_paste::read_clipboard_text() {
                        if text.is_empty() {
                            self.push_notice(
                                NotificationKind::Warning,
                                "Clipboard is empty".to_string(),
                            );
                        } else {
                            for ch in text.chars() {
                                self.key_input_dialog.insert_char(ch);
                            }
                        }
                    } else {
                        self.push_notice(
                            NotificationKind::Warning,
                            "Could not read clipboard".to_string(),
                        );
                    }
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.key_input_dialog.insert_char(c);
                }
                _ => {}
            }
            return false;
        }

        // "Free" composite-provider setup dialog (collects any subset of the
        // free-tier upstream keys; min 1 to enable, more = better).
        if self.free_mode_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.free_mode_dialog.close();
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.free_mode_dialog.move_next();
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.free_mode_dialog.move_prev();
                }
                KeyCode::Enter => {
                    if self.free_mode_dialog.can_submit() {
                        let values = self.free_mode_dialog.take_values();
                        for (provider_id, key) in values {
                            self.auth_store.set(
                                provider_id,
                                crate::tui::adapter_types::StoredCredential::ApiKey { key },
                            );
                        }
                        self.activate_provider(
                            "free".to_string(),
                            "Free Mode".to_string(),
                            "Connected to",
                        );
                    } else {
                        self.free_mode_dialog.move_next();
                    }
                }
                KeyCode::Backspace => {
                    self.free_mode_dialog.backspace();
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.free_mode_dialog.insert_char(c);
                }
                _ => {}
            }
            return false;
        }

        // Custom provider dialog (URL + API key for OpenAI-compatible providers)
        if self.custom_provider_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.custom_provider_dialog.close();
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.custom_provider_dialog.move_next_field();
                }
                KeyCode::Up => {
                    self.custom_provider_dialog.move_prev_field();
                }
                KeyCode::Enter => {
                    if self.custom_provider_dialog.can_submit() {
                        let provider_id = self.custom_provider_dialog.provider_id.clone();
                        let provider_name = self.custom_provider_dialog.provider_name.clone();
                        let (base_url, api_key) = self.custom_provider_dialog.take_values();
                        self.persist_custom_provider_base_url(&base_url);
                        self.auth_store.set(
                            &provider_id,
                            crate::tui::adapter_types::StoredCredential::ApiKey { key: api_key },
                        );
                        self.activate_provider(provider_id, provider_name, "Connected to");
                    } else {
                        self.custom_provider_dialog.move_next_field();
                    }
                }
                KeyCode::Backspace => {
                    self.custom_provider_dialog.backspace();
                }
                KeyCode::Char(c) => {
                    let c = normalize_char_with_shift(c, key.modifiers);
                    self.custom_provider_dialog.insert_char(c);
                }
                _ => {}
            }
            return false;
        }

        // Connect-a-provider dialog (/connect command)
        if self.connect_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.connect_dialog.close();
                }
                KeyCode::Home => {
                    self.connect_dialog.move_home();
                }
                KeyCode::End => {
                    self.connect_dialog.move_end();
                }
                KeyCode::Up => {
                    self.connect_dialog.move_up();
                }
                KeyCode::Down => {
                    self.connect_dialog.move_down();
                }
                KeyCode::PageUp => {
                    self.connect_dialog.page_up();
                }
                KeyCode::PageDown => {
                    self.connect_dialog.page_down();
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.connect_dialog.move_up();
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.connect_dialog.move_down();
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.connect_dialog.selected().cloned() {
                        self.connect_dialog.close();

                        match selected.id.as_str() {
                            // Local providers — activate immediately, no key needed
                            "ollama" | "lmstudio" | "llamacpp" => {
                                self.activate_provider(
                                    selected.id.clone(),
                                    selected.title.clone(),
                                    "Switched to",
                                );
                            }
                            // "Free" composite mode — collects any subset of the
                            // free-tier upstreams (min 1; more = better availability).
                            "free" => {
                                let existing: Vec<(&'static str, String)> = crate::tui::adapter_types::FREE_CATALOG
                                    .iter()
                                    .filter_map(|upstream| {
                                        let key = match upstream.id {
                                            "opencode-zen" => self
                                                .auth_store
                                                .api_key_for(crate::tui::adapter_types::ProviderId::OpencodeZen)
                                                .or_else(|| {
                                                    self.auth_store.api_key_for(
                                                        crate::tui::adapter_types::ProviderId::OpencodeGo,
                                                    )
                                                }),
                                            other => self.auth_store.api_key_for(other),
                                        };
                                        key.filter(|k: &String| !k.is_empty())
                                            .map(|k| (upstream.id, k))
                                    })
                                    .collect();
                                self.free_mode_dialog.open(&existing);
                            }
                            "anthropic" => {
                                // Anthropic: use API key from console.anthropic.com
                                // (OAuth requires a registered app which Operant doesn't have)
                                self.key_input_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                            }
                            "custom-openai" => {
                                let current_url = Settings::load_sync().ok().and_then(|settings| {
                                    settings
                                        .providers
                                        .get("custom-openai")
                                        .and_then(|p| p.api_base.clone())
                                });
                                self.custom_provider_dialog.open(
                                    selected.id.clone(),
                                    selected.title.clone(),
                                    current_url,
                                );
                            }
                            "github-copilot" => {
                                // GitHub Copilot: device code flow
                                self.device_auth_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                                self.device_auth_pending = Some("github-copilot".to_string());
                            }
                            "codex" | "openai-codex" => {
                                // OpenAI Codex: browser OAuth flow (spawned by main loop)
                                self.device_auth_dialog
                                    .open("openai-codex".into(), "OpenAI Codex".into());
                                self.device_auth_pending = Some("openai-codex".to_string());
                            }
                            // AWS Bedrock — accept a bearer token via key input dialog
                            "amazon-bedrock" => {
                                self.key_input_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                            }
                            // All other providers — open API key input dialog
                            _ => {
                                self.key_input_dialog
                                    .open(selected.id.clone(), selected.title.clone());
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.connect_dialog.filter_pop();
                }
                KeyCode::Char(c) => {
                    self.connect_dialog.filter_push(c);
                }
                _ => {}
            }
            return false;
        }

        // Import-config source picker
        if self.import_config_picker.visible {
            match key.code {
                KeyCode::Esc => {
                    self.import_config_picker.close();
                }
                KeyCode::Home => {
                    self.import_config_picker.move_home();
                }
                KeyCode::End => {
                    self.import_config_picker.move_end();
                }
                KeyCode::Up => {
                    self.import_config_picker.move_up();
                }
                KeyCode::Down => {
                    self.import_config_picker.move_down();
                }
                KeyCode::PageUp => {
                    self.import_config_picker.page_up();
                }
                KeyCode::PageDown => {
                    self.import_config_picker.page_down();
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.import_config_picker.move_up();
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.import_config_picker.move_down();
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.import_config_picker.selected().cloned() {
                        self.import_config_picker.close();
                        if let Some(selection) = Self::import_selection_from_picker(&selected.id) {
                            self.open_import_config_preview(selection);
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.import_config_picker.filter_pop();
                }
                KeyCode::Char(c) => {
                    self.import_config_picker.filter_push(c);
                }
                _ => {}
            }
            return false;
        }

        // Import-config preview dialog
        if self.import_config_dialog.visible {
            match key.code {
                KeyCode::Esc => self.import_config_dialog.close(),
                KeyCode::Enter => self.perform_import_config(),
                _ => {}
            }
            return false;
        }

        // Command palette (Ctrl+K)
        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc => {
                    self.command_palette.close();
                }
                KeyCode::Home => {
                    self.command_palette.move_home();
                }
                KeyCode::End => {
                    self.command_palette.move_end();
                }
                KeyCode::Up => {
                    self.command_palette.move_up();
                }
                KeyCode::Down => {
                    self.command_palette.move_down();
                }
                KeyCode::PageUp => {
                    self.command_palette.page_up();
                }
                KeyCode::PageDown => {
                    self.command_palette.page_down();
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.command_palette.move_up();
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.command_palette.move_down();
                }
                KeyCode::Enter => {
                    if let Some(selected) = self.command_palette.selected().cloned() {
                        self.command_palette.close();
                        // Put the command in the input and signal for execution
                        self.prompt_input.replace_text(selected.id.clone());
                        return true; // signal to submit this as input
                    }
                }
                KeyCode::Backspace => {
                    self.command_palette.filter_pop();
                }
                KeyCode::Char(c) => {
                    self.command_palette.filter_push(c);
                }
                _ => {}
            }
            return false;
        }

        // Invalid-config dialog intercepts Enter/Esc to dismiss

        // Session branching overlay intercepts navigation and Esc
        if self.session_branching.visible {
            use crate::tui::session_branching::BranchBrowserMode;
            match self.session_branching.mode {
                BranchBrowserMode::Browse => match key.code {
                    KeyCode::Esc => self.session_branching.cancel(),
                    KeyCode::Up => self.session_branching.select_prev(),
                    KeyCode::Down => self.session_branching.select_next(),
                    KeyCode::Char('n') => self.session_branching.start_create_new(),
                    KeyCode::Char('d') => self.session_branching.start_delete_confirm(),
                    KeyCode::Enter => {
                        if let Some(branch) = self.session_branching.selected_branch() {
                            // (plan 04) real switch: the drain retargets the
                            // agent and rebuilds the display (the old body
                            // printed "Switched to branch" and did nothing).
                            self.pending_switch_session = Some(branch.id.clone());
                            self.session_branching.close();
                        }
                    }
                    _ => {}
                },
                BranchBrowserMode::CreateNew => match key.code {
                    KeyCode::Esc => self.session_branching.cancel(),
                    KeyCode::Enter => {
                        // (plan 04) a named fork at the current point IS
                        // /rewind's documented behavior ("forks the session,
                        // old path kept") — the old body printed "Created
                        // branch" and did nothing. Close; the fork happens
                        // through /rewind.
                        self.session_branching.cancel();
                        self.set_status_notice(
                            "Fork at a point: use /rewind (forks the session, old path kept)."
                                .to_string(),
                        );
                    }
                    KeyCode::Backspace => self.session_branching.pop_create_char(),
                    KeyCode::Char(c) => self.session_branching.push_create_char(c),
                    _ => {}
                },
                BranchBrowserMode::ConfirmDelete => match key.code {
                    KeyCode::Esc | KeyCode::Char('n') => self.session_branching.cancel(),
                    KeyCode::Enter | KeyCode::Char('y') => {
                        // (plan 04) the old body printed "Deleted branch" and
                        // never touched the store — sessions are the durable
                        // record (\resume), deletion belongs to the CLI where
                        // it can be deliberate: `operant sessions delete`.
                        self.session_branching.cancel();
                        self.set_status_notice(
                            "Deleting sessions: `operant sessions delete <id>` (they are kept for /resume by design).".to_string(),
                        );
                    }
                    _ => {}
                },
            }
            return false;
        }

        // Session browser intercepts navigation and Esc
        if self.session_browser.visible {
            use crate::tui::session_browser::SessionBrowserMode;
            match self.session_browser.mode {
                SessionBrowserMode::Browse => {
                    match key.code {
                        KeyCode::Esc => self.session_browser.close(),
                        KeyCode::Up => self.session_browser.select_prev(),
                        KeyCode::Down => self.session_browser.select_next(),
                        KeyCode::Char('r') => self.session_browser.start_rename(),
                        // Enter: load the selected session's messages from the
                        // database and replace app.messages. The actual load
                        // happens asynchronously in the run loop via
                        // session_load_pending → session_load_rx.
                        KeyCode::Enter => {
                            if let Some(entry) = self
                                .session_browser
                                .sessions
                                .get(self.session_browser.selected_idx)
                                .cloned()
                            {
                                self.session_browser.close();
                                self.session_load_pending = Some(entry.id.clone());
                                self.status_message =
                                    Some(format!("Loading session '{}'…", entry.title));
                            }
                        }
                        _ => {}
                    }
                }
                SessionBrowserMode::Rename => match key.code {
                    KeyCode::Esc => self.session_browser.cancel(),
                    KeyCode::Enter => {
                        if let Some((_id, name)) = self.session_browser.confirm_rename() {
                            self.session_title = Some(name.clone());
                            self.set_status_notice(format!("Renamed to: {}", name));
                        }
                    }
                    KeyCode::Backspace => self.session_browser.pop_rename_char(),
                    KeyCode::Char(c) => self.session_browser.push_rename_char(c),
                    _ => {}
                },
            }
            return false;
        }

        // Export dialog key handling
        if self.export_dialog.visible {
            match key.code {
                KeyCode::Esc => {
                    self.export_dialog.dismiss();
                }
                KeyCode::Enter => {
                    if let Some(path) = self.perform_export() {
                        self.push_notice(NotificationKind::Info, format!("Exported to {}", path));
                    } else {
                        self.push_notice(
                            NotificationKind::Warning,
                            "Export failed: could not write file.".to_string(),
                        );
                    }
                }
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    self.export_dialog.toggle();
                }
                KeyCode::Char('1') => {
                    self.export_dialog.selected = ExportFormat::Json;
                }
                KeyCode::Char('2') => {
                    self.export_dialog.selected = ExportFormat::Markdown;
                }
                _ => {}
            }
            return false;
        }

        // Context visualization overlay key handling
        if self.context_viz.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.context_viz.close();
                }
                _ => {}
            }
            return false;
        }

        // MCP approval dialog
        if self.mcp_approval.visible {
            let result = crate::dialogs::handle_mcp_approval_key(&mut self.mcp_approval, key);
            if result.is_some() {
                // Result processed by CLI loop via take_mcp_approval_result()
            }
            return false;
        }

        // (iter-211: feedback_survey key handler deleted — no telemetry backend)

        // Memory file selector intercepts navigation and Esc
        if self.memory_file_selector.visible {
            match key.code {
                KeyCode::Esc => self.memory_file_selector.close(),
                KeyCode::Up => self.memory_file_selector.select_prev(),
                KeyCode::Down => self.memory_file_selector.select_next(),
                KeyCode::Enter => {
                    self.memory_file_selector.close();
                }
                _ => {}
            }
            return false;
        }

        // Skills view intercepts navigation and Esc
        if self.skills_view.visible {
            match key.code {
                KeyCode::Esc => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::Detail {
                        self.skills_view.back_to_list();
                    } else {
                        self.skills_view.close();
                    }
                }
                KeyCode::Backspace => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::Detail {
                        self.skills_view.back_to_list();
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::List {
                        self.skills_view.select_prev();
                    } else {
                        self.skills_view.scroll_up();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::List {
                        self.skills_view.select_next();
                    } else {
                        // Use the last rendered viewport height (set by
                        // render_list_stage) instead of a hardcoded 24.
                        // (Bug #16 fix.)
                        let vh = self.skills_view.last_viewport_height.get().max(1);
                        self.skills_view.scroll_down(vh);
                    }
                }
                KeyCode::PageUp => {
                    for _ in 0..6 {
                        self.skills_view.scroll_up();
                    }
                }
                KeyCode::PageDown => {
                    let vh = self.skills_view.last_viewport_height.get().max(1);
                    for _ in 0..6 {
                        self.skills_view.scroll_down(vh);
                    }
                }
                KeyCode::Enter
                    if self.skills_view.stage == crate::tui::skills_view::SkillsStage::List =>
                {
                    self.skills_view.open_detail();
                }
                _ => {}
            }
            return false;
        }

        // Plugins hub intercepts navigation, toggle, and Esc
        if self.plugins_hub.visible {
            // Resolve plugins_dir once for the toggle action.
            let plugins_dir = crate::cmd_plugins::plugins_dir(&self.config).unwrap_or_else(|_| {
                dirs::data_dir()
                    .unwrap_or_default()
                    .join("operant")
                    .join("plugins")
            });
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.plugins_hub.close(),
                KeyCode::Up | KeyCode::Char('k') => self.plugins_hub.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.plugins_hub.select_next(),
                KeyCode::Enter | KeyCode::Char('t') | KeyCode::Char(' ') => {
                    self.plugins_hub.toggle_selected(&plugins_dir);
                }
                _ => {}
            }
            return false;
        }

        // Journey view intercepts navigation, pane-switch, and Esc
        if self.journey_view.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.journey_view.close(),
                KeyCode::Up | KeyCode::Char('k') => self.journey_view.cursor_up(),
                KeyCode::Down | KeyCode::Char('j') => self.journey_view.cursor_down(),
                KeyCode::Tab | KeyCode::BackTab => self.journey_view.switch_pane(),
                _ => {}
            }
            return false;
        }

        // Hooks config menu intercepts navigation and Esc
        if self.hooks_config_menu.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.hooks_config_menu.back(),
                KeyCode::Enter => self.hooks_config_menu.enter(),
                KeyCode::Up | KeyCode::Char('k') => self.hooks_config_menu.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.hooks_config_menu.select_next(),
                _ => {}
            }
            return false;
        }

        if self.diff_viewer.visible {
            self.handle_diff_viewer_key(key);
            return false;
        }

        if self.agents_menu.visible {
            self.handle_agents_menu_key(key);
            return false;
        }

        if self.mcp_view.visible {
            return self.handle_mcp_view_key(key);
        }

        if self.stats_dialog.visible {
            self.handle_stats_dialog_key(key);
            return false;
        }

        // Settings screen intercepts keys
        if self.settings_screen.visible {
            crate::settings_screen::handle_settings_key(
                &mut self.settings_screen,
                &mut self.config,
                &mut self.settings,
                key,
            );
            // `reduce_motion` is the one setting with a live effect on the
            // render loop, so apply it the moment the screen writes it rather
            // than waiting for the next launch. The screen owns the live value
            // (it writes the snapshot file, not `self.settings`).
            let reduce_motion = self.settings_screen.reduce_motion;
            crate::tui::redraw::set_reduce_motion(reduce_motion);
            self.perf_tier =
                crate::tui::redraw::PerformanceTier::detect().with_reduce_motion(reduce_motion);
            return false;
        }

        // Theme picker intercepts keys
        if self.theme_screen.visible {
            if let Some(theme_name) =
                crate::theme_screen::handle_theme_key(&mut self.theme_screen, key)
            {
                self.apply_theme(&theme_name);
            }
            return false;
        }

        // Privacy screen intercepts keys
        // Rewind selector intercepts keys first (plan 05)
        if self.rewind_selector.visible {
            return self.handle_rewind_selector_key(key);
        }

        // Help overlay intercepts keys next
        if self.help_overlay.visible {
            return self.handle_help_overlay_key(key);
        }

        // New history-search overlay
        if self.history_search_overlay.visible {
            return self.handle_history_search_overlay_key(key);
        }

        if self.global_search.visible {
            return self.handle_global_search_key(key);
        }

        // (iter-155: legacy history_search.is_some() check deleted — always None)

        // Permission dialog mode intercepts most keys
        if self.permission_request.is_some() {
            self.handle_permission_key(key);
            return false;
        }

        // (iter-143: plugin_hints dismiss handler deleted — Vec was always empty)

        // Overage upsell dismiss — the overage_upsell dialog was deleted in
        // iter-58; this block is kept as a placeholder for future dismiss
        // handlers. No-op until a replacement dialog is wired.

        // Desktop upsell startup dialog

        // Memory update notification dismiss — the memory_update_notification
        // dialog was deleted in iter-58; this block is kept as a placeholder
        // for future dismiss handlers. No-op until a replacement is wired.

        // MCP elicitation dialog — highest priority modal

        // (iter-163: KeybindingResolver processor deleted — process() always
        // returned NoMatch, has_pending_chord() always returned false, and
        // cancel_chord() was a no-op. The entire block was dead code that
        // always fell through to the hardcoded handlers.)

        // ---- Drag-select copy mode (Ctrl+T) ----------------------------
        // Placed after every modal handler so an open dialog still owns its
        // own keys. While copy mode is on, composer keys are swallowed and the
        // exit chords still work; leaving copy mode restores normal key
        // handling because the gate is the only thing that took it away.
        if self.copy_mode_active() {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            match key.code {
                KeyCode::Esc => {
                    self.exit_copy_mode();
                }
                KeyCode::Char('t') if ctrl => {
                    self.exit_copy_mode();
                }
                // (plan 08) copy mode moved from Ctrl+T (the thinking toggle)
                // to Alt+Y — the exit chord follows the entry chord.
                KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.exit_copy_mode();
                }
                KeyCode::Char('c') if ctrl => {
                    self.copy_current_selection();
                }
                // ---- operant-parity keyboard navigation (iter-672) ----------
                // Cursor = selection focus (falling back to the anchor, then
                // the first visible line). Plain moves collapse the selection
                // onto the new cursor; SHIFT moves extend from the anchor.
                KeyCode::Char('h') | KeyCode::Left => {
                    self.move_copy_cursor(-1, 0, shift);
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    self.move_copy_cursor(1, 0, shift);
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.move_copy_cursor(0, 1, shift);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.move_copy_cursor(0, -1, shift);
                }
                KeyCode::Home => {
                    self.move_copy_cursor_to_edge(false, shift);
                }
                KeyCode::End => {
                    self.move_copy_cursor_to_edge(true, shift);
                }
                KeyCode::PageUp => {
                    let page = self.copy_cursor_page();
                    self.move_copy_cursor(0, -(page as i64), shift);
                }
                KeyCode::PageDown => {
                    let page = self.copy_cursor_page();
                    self.move_copy_cursor(0, page as i64, shift);
                }
                KeyCode::Char('g') => {
                    self.move_copy_cursor_to_line_edge(false, shift);
                }
                KeyCode::Char('G') => {
                    self.move_copy_cursor_to_line_edge(true, shift);
                }
                // Select everything currently on screen.
                KeyCode::Char('a') if !ctrl => {
                    self.copy_select_all_visible();
                }
                // Copy the cursor ± 4 lines of context (jcode's
                // COPY_VIEWPORT_CONTEXT_LINES) and exit the mode.
                KeyCode::Char('a') if ctrl => {
                    self.copy_viewport_context_and_exit();
                }
                // Enter/Y: copy the selection.
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.copy_current_selection();
                }
                _ => {}
            }
            return true;
        }

        // Clear any active text selection on key press, except the chords
        // that act *on* a selection: Ctrl+C copies it, Alt+Y enters copy
        // mode (which has to hand the prior selection back on exit).
        // (plan 08: copy mode moved from Ctrl+T — the thinking toggle now.)
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let is_copy = key.code == KeyCode::Char('c') && ctrl;
        let is_copy_mode = key.code == KeyCode::Char('y') && alt;
        if !is_copy && !is_copy_mode && self.selection_anchor.is_some() {
            self.selection_anchor = None;
            self.selection_focus = None;
            *self.selection_text.borrow_mut() = String::new();
        }

        // Composer shift-selection (jcode textarea selection, 2026-10-09
        // visual audit): Shift+arrows extend it; Ctrl+C copies it; every other
        // key collapses it.
        let is_shift_arrow = key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
            );
        if !is_copy && !is_shift_arrow && self.prompt_input.sel_anchor.is_some() {
            self.prompt_input.selection_clear();
        }

        // ---- Ctrl+V / Cmd+V — clipboard paste (image first, then text fallback) ----
        // Only fires when NOT in vim Normal/Visual/VisualBlock mode (where \x16 is
        // already consumed by the vim handler above to enter VisualBlock mode).
        if key.code == KeyCode::Char('v')
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !matches!(
                self.prompt_input.vim_mode,
                crate::prompt_input::VimMode::Normal
                    | crate::prompt_input::VimMode::Visual
                    | crate::prompt_input::VimMode::VisualBlock
            )
        {
            use crate::tui::image_paste::{
                describe_rendered, read_clipboard_image, read_clipboard_text, read_primary_text,
                render_attachment,
            };
            if let Some(img) = read_clipboard_image() {
                // Render (or degrade to a named placeholder) before the image
                // is moved into the prompt's attachment list.
                let msg = describe_rendered(&img, &render_attachment(&img));
                self.prompt_input.add_image(img.clone());
                // Queued for the post-paint emit pass in `render_app`.
                self.pending_inline_images.borrow_mut().push(img);
                self.push_notice(NotificationKind::Info, msg);
            } else if let Some(text) = read_clipboard_text().or_else(read_primary_text) {
                self.handle_paste_data(text);
                self.refresh_prompt_input();
            }
            return false;
        }

        // ---- Shift+Insert — selection/clipboard paste fallback -------------
        if key.code == KeyCode::Insert && key.modifiers.contains(KeyModifiers::SHIFT) {
            let _ = self.paste_primary_into_prompt();
            return false;
        }

        // ---- Ctrl+L — terminal-style clear (jcode parity) --------------------
        // Collapse the chat like a terminal `clear`: nothing is deleted, the
        // transcript chunk just renders zero-height (see
        // `terminal_clear_collapsed`). Scrolling up, new output, or streaming
        // ends it; `/clear` drops context instead. No side pane claims plain
        // Ctrl+L today (copy mode's bare `l` is cursor-right, not this).
        if key.code == KeyCode::Char('l') && ctrl {
            self.clear_view_terminal_style();
            return false;
        }

        // ---- Focus state machine: transcript mode --------------------------
        // When the transcript pane has focus, intercept Escape and scroll keys.
        // Printable characters switch focus back to Input and fall through so the
        // keystroke is processed normally by the prompt editor below.
        if self.focus == FocusTarget::Transcript {
            match key.code {
                KeyCode::Esc => {
                    self.focus = FocusTarget::Input;
                    return false;
                }
                KeyCode::PageUp | KeyCode::PageDown => {
                    // Let these fall through to the normal scroll handling below.
                }
                KeyCode::Char(_)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    // Printable char: switch focus to Input and process normally.
                    self.focus = FocusTarget::Input;
                }
                _ => {}
            }
        }

        match key.code {
            // ---- ESC: cancel streaming (status bar advertises "esc interrupt") ----
            KeyCode::Esc if self.is_streaming => {
                self.cancel_streaming_turn();
            }

            // ---- /rewind <n> confirmation ---------------------------------
            // (plan 05) y/n in the status line; any other key cancels it.
            KeyCode::Char('y') | KeyCode::Char('Y') if self.pending_rewind_confirm.is_some() => {
                let n = self.pending_rewind_confirm.take();
                // Turn n → mark index n-1 (marks are prompt-ordered).
                if let Some(n) = n {
                    let marks_len = self.rewind_selector.entries.len();
                    if n == 0 || n > marks_len {
                        self.set_status_notice(format!("No prompt #{n} to rewind to."));
                    } else {
                        let entry = self.rewind_selector.entries[n - 1].clone();
                        if entry.compacted {
                            self.set_status_notice(
                                "That turn was folded by compaction — pick a later one."
                                    .to_string(),
                            );
                        } else {
                            self.pending_rewind = Some(entry.turn_id);
                        }
                    }
                }
                return false;
            }
            KeyCode::Char(c) if self.pending_rewind_confirm.is_some() && (c == 'n' || c == 'N') => {
                self.pending_rewind_confirm = None;
                self.set_status_notice("Rewind cancelled.".to_string());
                return false;
            }

            // ---- Double-Esc on an empty composer → rewind selector ----
            // (plan 05) A second Esc within 500 ms, not streaming, composer
            // empty: open the fullscreen rewind selector. A single Esc keeps
            // all of its current meanings.
            KeyCode::Esc if !self.is_streaming => {
                let now = std::time::Instant::now();
                let double = self
                    .last_esc_at
                    .is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(500));
                self.last_esc_at = Some(now);
                if double && self.prompt_input.is_empty() && self.double_escape_action == "rewind" {
                    self.last_esc_at = None;
                    self.open_rewind_selector();
                    return false;
                }
                // Fall through to the single-Esc meanings below.
            }

            // ---- Quit / cancel ----------------------------------------
            // Accept both 'c' and 'C' so Shift+Ctrl+C also triggers copy
            // (issue #149 follow-up).
            KeyCode::Char(c)
                if (c == 'c' || c == 'C') && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // Composer shift-selection (jcode textarea selection): the
                // in-input selection wins — copy it and stop, never exit.
                if let Some(sel) = self.prompt_input.selection_text() {
                    if !sel.is_empty() {
                        let outcome = crate::tui::clipboard::copy(&sel);
                        self.prompt_input.selection_clear();
                        self.push_notice(
                            if outcome.is_copied() {
                                NotificationKind::Info
                            } else {
                                NotificationKind::Warning
                            },
                            outcome.status_message(),
                        );
                        return false;
                    }
                }
                // If text is selected, copy it to clipboard instead of quitting.
                let sel_text = self.refresh_selection_text();
                if self.selection_anchor.is_some() && !sel_text.is_empty() {
                    // Text is selected: copy to clipboard.
                    let outcome = crate::tui::clipboard::copy(&sel_text);
                    self.selection_anchor = None;
                    self.selection_focus = None;
                    *self.selection_text.borrow_mut() = String::new();
                    self.push_notice(
                        if outcome.is_copied() {
                            NotificationKind::Info
                        } else {
                            NotificationKind::Warning
                        },
                        outcome.status_message(),
                    );
                } else if self.is_streaming {
                    // (plan 04) Ctrl+C while streaming is the same cancel as
                    // Esc — the interrupt flag, not the invisible
                    // "is_streaming=false" that used to leave the agent
                    // running and spending.
                    self.cancel_streaming_turn();
                } else {
                    // No text selected and not streaming: handle exit confirmation sequence.
                    // Always clear the prompt input on Ctrl+C.
                    if !self.prompt_input.is_empty() {
                        self.prompt_input.clear();
                        self.refresh_prompt_input();
                    }
                    self.handle_exit_key_confirmation('c');
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+D on empty input: trigger two-press exit confirmation (like Ctrl+C).
                if self.prompt_input.is_empty() {
                    self.handle_exit_key_confirmation('d');
                }
            }

            // ---- Model picker (Ctrl+A) -----------------------------------
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // (plan 05B) the unified inline picker — same surface as
                // /model, no provider-guessing.
                if !self.is_streaming && self.credentials_live() {
                    self.open_model_picker_inline();
                }
            }

            // ---- History search ----------------------------------------
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let overlay = HistorySearchOverlay::open(&self.prompt_input.history);
                self.history_search_overlay = overlay;
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.global_search.open();
                self.refresh_global_search();
            }

            // ---- Session branching (Ctrl+B) -----------------------------
            // Bug #6 from iter-82 audit: Ctrl+B was documented in the help
            // overlay comment but had no keybinding. session_branching.open()
            // was never called from anywhere. Now it opens the branch browser
            // seeded with the current message count.
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.session_branching.open(vec![], self.messages.len());
                // (plan 05) populate the browser with the fork tree on the
                // next agent-touching pass (the DB lives on the agent).
                self.pending_branch_list = true;
            }

            // ---- Scroll bookmark (Ctrl+G) -------------------------------
            // Toggles: first press remembers the current spot, second press
            // jumps back to it and disarms. The status row names the state so
            // the toggle is discoverable without opening the help overlay.
            // Ctrl+G: edit the prompt in the external editor (oh-my-pi
            // `app.editor.external`); the scroll bookmark moved to Alt+G.
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.pending_external_editor = true;
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.toggle_scroll_bookmark();
            }
            // Alt+P: cycle the configured models (`[agent] model` +
            // `fallback_models`) for this session; Ctrl+A is the full picker.
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.cycle_configured_model();
            }
            // F5 / Alt+R: retry — resubmit the composer (a failed turn puts
            // its prompt back there) or, when empty, the last prompt sent.
            KeyCode::F(5) | KeyCode::Char('r')
                if key.code == KeyCode::F(5) || key.modifiers.contains(KeyModifiers::ALT) =>
            {
                if self.is_streaming {
                    return false;
                }
                if self.prompt_input.text.trim().is_empty() {
                    match self.prompt_input.history.last().cloned() {
                        Some(last) => {
                            self.prompt_input.text = last;
                            self.prompt_input.cursor = self.prompt_input.text.len();
                        }
                        None => {
                            self.set_status_notice("Nothing to retry yet.".to_string());
                            return false;
                        }
                    }
                }
                return true;
            }
            // Ctrl+Shift+O (plan 08): toggle tool-call visibility — must sit
            // ABOVE the plain Ctrl+O arm, which would otherwise win on the
            // same char+CONTROL when the terminal reports Shift too.
            KeyCode::Char('o')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.toggle_tool_visibility();
            }
            // Ctrl+O: expand / collapse the latest tool row's full output.
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_latest_tool_expansion();
            }

            // ---- Context menu (Ctrl+Shift+M) ----------------------------
            KeyCode::Char('m')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.open_context_menu_at_cursor();
            }

            // ---- Command palette (Ctrl+K) -------------------------------
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.command_palette.open();
            }

            // ---- Help overlay ------------------------------------------
            KeyCode::F(1) => {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }
            KeyCode::Char('?')
                if !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }
            // With the kitty keyboard protocol, Shift+/ is reported as Char('/') with
            // SHIFT rather than Char('?'), so also accept that form for the help toggle.
            KeyCode::Char('/')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !self.is_streaming
                    && self.prompt_input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.show_help = !self.show_help;
                self.help_overlay.toggle();
            }

            // ---- Usage overlay ------------------------------------------
            // Persistent session usage panel (cost, cache-prefix hit rate,
            // per-turn token deltas). Not modal, so it stays up until the
            // next press. Mirrored in keybindings/defaults.rs as
            // KeyAction::Custom(1) so /keys advertises it.
            KeyCode::F(8) => {
                self.usage_overlay.toggle();
            }

            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.kill_line_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.kill_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.yank();
                self.refresh_prompt_input();
            }

            // ---- Prompt bindings the catalogue advertised but nothing
            // implemented (iter-429). Each of these is listed in
            // keybindings/defaults.rs under BindingContext::Prompt, so /keys
            // told users they worked, and pressing them did nothing. All three
            // are standard readline, and none conflicts with an existing
            // binding, which is why they are implemented rather than removed —
            // the iter-409 resolution for Ctrl+H/Ctrl+L was to drop the claim,
            // but those two could never fire reliably because the terminal or
            // the OS took them. These can.
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.move_to_end();
                self.refresh_prompt_input();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.move_word_forward();
                self.refresh_prompt_input();
            }
            // Ctrl+N as history-next, matching the guard the Down arm uses for
            // history_down. The four OTHER Ctrl+N arms in this file are all
            // scoped to a dialog or picker (connect_dialog,
            // import_config_picker, command_palette, model_picker), so nothing
            // else claims this chord with a focused prompt.
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.prompt_input.history_pos.is_some() {
                    self.prompt_input.history_down();
                }
                self.refresh_prompt_input();
            }

            // ---- Input stash (Ctrl+S) -----------------------------------
            // jcode parity. Parks the composer so a half-written prompt is
            // not lost when the composer has to be free; the same chord
            // restores it. Allowed while streaming — the composer is
            // editable mid-turn, so parking it is equally safe.
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_input_stash();
            }
            // ---- Composer burst undo (Ctrl+Z) ---------------------------
            // Reverts a run of edits to the previous snapshot, not one
            // keystroke (see UNDO_COALESCE in prompt_input).
            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.undo_input_burst();
            }
            // ---- Queued-message recall (Ctrl+X) -------------------------
            // Pulls the next message from the queue /steer and /queue write
            // to, so it can be edited instead of spent on the agent. Ctrl+Q
            // would be the mnemonic choice but the (non-dispatched) binding
            // table already documents it as "Quit application", so this
            // takes the one Ctrl letter nothing claims.
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.recall_queued_message();
            }
            // ---- Drag-select copy mode (Ctrl+T) -------------------------
            // Every other Ctrl letter in the composer is taken (a b c d g k
            // n p r s u w x y z); t is the one that reads as "text".
            // ---- Thinking / tool visibility (plan 08) -----------------
            // Ctrl+T is the thinking toggle (oh-my-pi parity); copy mode
            // moved to Alt+Y. Ctrl+Shift+O's arm sits above plain Ctrl+O;
            // Alt+O is the everywhere fallback — same action either way.
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_thinking();
                // (plan 07 §2) a display toggle changes what settled rows
                // SHOULD look like — replay re-emits history under the new
                // mode instead of "applying to new output".
                self.request_native_replay();
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.toggle_tool_visibility();
                self.request_native_replay();
            }
            // (plan 07 §2) Alt+L: manual native replay — re-emit the whole
            // transcript at the current width (native scrollback cannot
            // reflow, so this is the honest "redraw my history" key).
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.request_native_replay();
                self.set_status_notice("Replaying transcript at current width…".to_string());
            }

            // ---- Alt/Meta key text editing operations -------------------
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.enter_copy_mode();
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete_word_backward();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_forward();
                self.refresh_prompt_input();
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_backward();
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.move_word_forward();
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.delete_word_at_cursor();
                self.refresh_prompt_input();
            }

            // ---- Text entry (allowed while streaming so users can queue
            // the next message; submission queues via Enter at the CLI layer).
            KeyCode::Char(c) => {
                let c = normalize_char_with_shift(c, key.modifiers);
                if self.prompt_input.vim_enabled && self.prompt_input.vim_mode != VimMode::Insert {
                    self.prompt_input.vim_command(&c.to_string());
                } else {
                    self.prompt_input.insert_char(c);
                }
                self.refresh_prompt_input();
            }
            KeyCode::Backspace => {
                self.prompt_input.backspace();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete();
                self.refresh_prompt_input();
            }
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_input.delete_word_forward();
                self.refresh_prompt_input();
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // jcode textarea selection: anchor + extend left.
                    self.prompt_input.selection_begin_or_extend();
                    self.prompt_input.move_left();
                } else if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = 0;
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_backward();
                } else {
                    self.prompt_input.move_left();
                }
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // jcode textarea selection: anchor + extend right.
                    self.prompt_input.selection_begin_or_extend();
                    self.prompt_input.move_right();
                } else if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = self.prompt_input.text.len();
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_forward();
                } else {
                    self.prompt_input.move_right();
                }
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Home => {
                self.prompt_input.cursor = 0;
                self.sync_legacy_prompt_fields();
            }
            KeyCode::End => {
                self.prompt_input.cursor = self.prompt_input.text.len();
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Tab => {
                if !self.prompt_input.suggestions.is_empty() {
                    // Accept slash-command suggestion. Allowed while streaming
                    // so the typeahead popup is interactive even when a turn
                    // is in flight — Enter then queues the completed command.
                    if self.prompt_input.suggestion_index.is_none() {
                        self.prompt_input.suggestion_index = Some(0);
                    }
                    self.prompt_input.accept_suggestion();
                    self.refresh_prompt_input();
                }
            }

            // ---- Shift+Tab ----------------------------------------------
            // 2026-10-14 batch, plan 10 removed the permission-mode cycle
            // (two silent presses armed Bypass); plan 03 rebinds the key to
            // the effort cycle — the oh-my-pi "effort ladder" muscle memory.
            KeyCode::BackTab if !self.is_streaming => {
                self.cycle_effort();
            }

            // ---- Submit ------------------------------------------------
            // Shift+Enter / Alt+Enter / Ctrl+Enter / Ctrl+J insert a literal
            // newline so users can compose multi-line prompts before sending.
            // Ctrl+J is the traditional Unix "newline" key and is what
            // hermes-agent uses for line breaks in the TUI.
            // (iter-120 — user-requested: Ctrl+J was not working.)
            KeyCode::Enter
                if !self.is_streaming
                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.modifiers.contains(KeyModifiers::ALT)
                        || key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                self.prompt_input.insert_newline();
                self.refresh_prompt_input();
            }
            // Plain Enter submits. The gate opens for `/steer` + `/queue`
            // even while streaming: those are the one input that is
            // meaningful in both states — streaming, they feed the agent's
            // steer queue which the loop drains at the next iteration
            // boundary; idle, they queue for the next turn. The modifier
            // arm above stays `!is_streaming` so composing a multi-line
            // prompt mid-stream is unaffected.
            KeyCode::Enter
                if !self.is_streaming
                    || (key.modifiers == KeyModifiers::NONE
                        && is_steer_command_input(&self.prompt_input.text)) =>
            {
                use crate::tui::prompt_input::AcceptForSubmitOutcome;
                // (plan 04) An EXACT command submits as typed — Enter must
                // never replace the user's typed command with a different
                // one that merely shares the prefix (usage-recency can rank
                // /effort-set above a typed /effort, or /context-files above
                // /context; completion is for PARTIAL input only). Exact is
                // judged on the first word, so "/goal ship it" still counts
                // as an exact /goal with args.
                let typed_first_word = self
                    .prompt_input
                    .text
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let exact_command = !typed_first_word.is_empty()
                    && crate::tui::app::helpers::tui_slash_command_data()
                        .iter()
                        .any(|(name, _)| *name == typed_first_word);
                if !exact_command {
                    // Phase 1.3: Auto-select first suggestion when visible but none selected.
                    if !self.prompt_input.suggestions.is_empty()
                        && self.prompt_input.suggestion_index.is_none()
                    {
                        self.prompt_input.suggestion_index = Some(0);
                    }
                    match self.prompt_input.accept_suggestion_for_submit() {
                        AcceptForSubmitOutcome::ExtendInput => {
                            self.refresh_prompt_input();
                            return false;
                        }
                        AcceptForSubmitOutcome::Submit => return true,
                        AcceptForSubmitOutcome::NoSuggestion => {}
                    }
                }
                // Auto-dismiss all error notifications when user sends a message
                self.dismiss_error_notifications();
                // New user input: snap back to bottom.
                self.auto_scroll = true;
                self.new_messages_while_scrolled = 0;
                self.scroll_offset = 0;
                return true;
            }

            // ---- Message boundary navigation (Alt+Up/Alt+Down) ----------
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
                // Jump up by ~20 lines (approximate message boundary).
                let prev = self.scroll_offset;
                self.scroll_offset = self.scroll_offset.saturating_add(20);
                self.auto_scroll = false;
                self.note_user_scroll(prev);
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
                // Jump down by ~20 lines (approximate message boundary).
                let prev = self.scroll_offset;
                let new_off = self.scroll_offset.saturating_sub(20);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
                self.note_user_scroll(prev);
            }

            // ---- Fine transcript scroll (Ctrl+Up/Ctrl+Down, jcode parity) -----
            // Completes the keyboard scroll ladder: ±3 here, ±10 on
            // PageUp/PageDown, ±20 on Alt+Up/Alt+Down. Without the fine rung a
            // mouseless session (tmux without mouse-mode, plain tty) jumps in
            // 10-line quanta only — the "chat window is pinned" complaint
            // (2026-10-09 visual audit) from users without wheel events.
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let prev = self.scroll_offset;
                self.scroll_offset = self.scroll_offset.saturating_add(3);
                self.auto_scroll = false;
                self.note_user_scroll(prev);
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let prev = self.scroll_offset;
                let new_off = self.scroll_offset.saturating_sub(3);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
                self.note_user_scroll(prev);
            }

            // ---- Input history navigation ------------------------------
            // For multi-line / wrapped prompts: Up/Down move the cursor by
            // one visual row first, only falling through to history recall
            // when the cursor is already on the first/last visual row
            // (issue #149 follow-up).
            // Also, if suggestions are visible (text starts with '/' or has file ref),
            // allow suggestion navigation with Up/Down.
            // In vim Visual mode, Shift+Up/Shift+Down extend the selection.
            KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(
                        self.prompt_input.vim_mode,
                        crate::prompt_input::VimMode::Visual
                            | crate::prompt_input::VimMode::VisualLine
                            | crate::prompt_input::VimMode::VisualBlock
                    )
                {
                    // Shift+Up in visual mode: extend selection up
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_up(width);
                } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // jcode textarea selection in normal mode: anchor + extend
                    // up one visual row (vim Visual keeps its own path above).
                    self.prompt_input.selection_begin_or_extend();
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_up(width);
                } else if !self.prompt_input.suggestions.is_empty()
                    && (self.prompt_input.text.starts_with('/')
                        || self.prompt_input.has_active_file_ref())
                {
                    // Suggestions visible: navigate them
                    self.prompt_input.suggestion_prev();
                } else if !self.prompt_input.text.contains('\n') {
                    // Single-line input: always navigate history (like hermes-agent).
                    // (iter-124 — was only navigating when move_visual_up failed,
                    // which meant Up did nothing on single-line input.)
                    if !self.prompt_input.history.is_empty() {
                        self.prompt_input.history_up();
                    }
                } else {
                    // Multi-line input: move cursor up within the text.
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_up(width);
                }
                self.refresh_prompt_input();
            }
            KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(
                        self.prompt_input.vim_mode,
                        crate::prompt_input::VimMode::Visual
                            | crate::prompt_input::VimMode::VisualLine
                            | crate::prompt_input::VimMode::VisualBlock
                    )
                {
                    // Shift+Down in visual mode: extend selection down
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_down(width);
                } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // jcode textarea selection in normal mode: anchor + extend
                    // down one visual row (vim Visual keeps its own path above).
                    self.prompt_input.selection_begin_or_extend();
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_down(width);
                } else if !self.prompt_input.suggestions.is_empty()
                    && (self.prompt_input.text.starts_with('/')
                        || self.prompt_input.has_active_file_ref())
                {
                    // Suggestions visible: navigate them
                    self.prompt_input.suggestion_next();
                } else if !self.prompt_input.text.contains('\n') {
                    // Single-line input: always navigate history.
                    if self.prompt_input.history_pos.is_some() {
                        self.prompt_input.history_down();
                    }
                } else {
                    // Multi-line input: move cursor down within the text.
                    let area = self.last_input_area.get();
                    let width = area.width.saturating_sub(4) as usize;
                    self.prompt_input.move_visual_down(width);
                }
                self.refresh_prompt_input();
            }

            // ---- Scroll ------------------------------------------------
            KeyCode::PageUp => {
                let prev = self.scroll_offset;
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                // Scrolling up disables auto-follow.
                self.auto_scroll = false;
                self.note_user_scroll(prev);
            }
            KeyCode::PageDown => {
                let prev = self.scroll_offset;
                let new_off = self.scroll_offset.saturating_sub(10);
                self.scroll_offset = new_off;
                if new_off == 0 {
                    // Scrolled all the way back to bottom — re-enable auto-follow.
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
                self.note_user_scroll(prev);
            }

            // ---- Toggle last thinking block (t key) -------------------
            // (Removed: shadowed by KeyCode::Char(c) prompt input handler.)
            _ => {}
        }

        // Reset exit confirmation sequence if user presses any key other than Ctrl+C or Ctrl+D.
        let is_exit_key = key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char(c) if c == 'c' || c == 'd' || c == 'C' || c == 'D');
        if !is_exit_key {
            self.last_exit_key_warning = None;
            self.exit_key_sequence_start = None;
        }

        false
    }

    // (iter-164: fn current_key_context deleted — unused after keybinding processor removal)

    /// Count one accepted key press against the binding registry, so `/hotkeys`
    /// can split the catalogue into muscle memory vs undiscovered.
    ///
    /// Only chords that actually resolve to a binding are counted. A bare
    /// modifier press, a function key the table does not bind, and ordinary
    /// free text all fail the lookup and are dropped — otherwise the report
    /// would fill up with noise the user never chose to bind.
    ///
    /// Bounded by construction: a counter only ever appears for a chord the
    /// table already defines, so the map can hold at most one entry per
    /// binding (~120), never unbounded.
    ///
    /// Counters live in the shared `UsageStore` under a `key:` namespace,
    /// which is what makes them survive a restart — same JSON file, same
    /// load/save path as the slash-command stats.
    fn record_keybinding_usage(&mut self, key: &KeyEvent) {
        use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS, usage_key};

        // `find` checks the given context first, then the global set, so one
        // lookup covers globals too.
        let context = if self.any_modal_open() {
            BindingContext::Dialog
        } else {
            BindingContext::Prompt
        };
        let Some(binding) = DEFAULT_KEYBINDINGS.find(key, context) else {
            return;
        };
        let name = usage_key(&binding.chord());
        self.slash_usage.record(&name);
        // The store is flushed by every slash command already; this only
        // bounds the extra loss window for a session that never runs one.
        if self
            .slash_usage
            .frequency_rank(&name)
            .is_multiple_of(KEY_USAGE_FLUSH_EVERY)
        {
            self.slash_usage.save();
        }
    }

    // -------------------------------------------------------------------
    // New overlay key handlers
    // -------------------------------------------------------------------

    fn handle_stats_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.stats_dialog.close(),
            KeyCode::Tab | KeyCode::Right => self.stats_dialog.next_tab(),
            KeyCode::BackTab | KeyCode::Left => self.stats_dialog.prev_tab(),
            KeyCode::Char('r') => self.stats_dialog.cycle_range(),
            KeyCode::Up => self.stats_dialog.scroll = self.stats_dialog.scroll.saturating_sub(1),
            KeyCode::Down => self.stats_dialog.scroll = self.stats_dialog.scroll.saturating_add(1),
            _ => {}
        }
    }

    fn handle_mcp_view_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mcp_view.close(),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => self.mcp_view.switch_pane(),
            KeyCode::Up => self.mcp_view.select_prev(),
            KeyCode::Down => self.mcp_view.select_next(),
            KeyCode::Backspace => self.mcp_view.pop_search_char(),
            KeyCode::Char('e') => self.mcp_view.toggle_error_detail(),
            KeyCode::Char('a')
                if self.mcp_view.active_pane == crate::mcp_view::McpViewPane::ServerList =>
            {
                let selected_server = self
                    .mcp_view
                    .servers
                    .get(self.mcp_view.selected_server)
                    .map(|server| server.name.clone());
                if let Some(server_name) = selected_server {
                    self.pending_mcp_panel_auth = Some(server_name);
                    self.mcp_view.close();
                    self.set_status_notice("Starting MCP auth...".to_string());
                }
            }
            KeyCode::Char('r') => {
                self.pending_mcp_reconnect = true;
                self.set_status_notice("Reconnecting MCP runtime...".to_string());
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty()
                    && self.mcp_view.active_pane != crate::mcp_view::McpViewPane::ServerList =>
            {
                self.mcp_view.push_search_char(c);
            }
            _ => {}
        }
        false
    }

    fn handle_agents_menu_key(&mut self, key: KeyEvent) {
        if matches!(self.agents_menu.route, AgentsRoute::Editor(_)) {
            match key.code {
                KeyCode::Esc => self.agents_menu.go_back(),
                KeyCode::Tab | KeyCode::Down => self.agents_menu.editor_next_field(),
                KeyCode::BackTab | KeyCode::Up => self.agents_menu.editor_prev_field(),
                KeyCode::Enter => self.agents_menu.editor_insert_newline(),
                KeyCode::Backspace => self.agents_menu.editor_backspace(),
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match self.agents_menu.save_editor() {
                        Ok(msg) => self.set_status_notice(msg),
                        Err(err) => {
                            self.agents_menu.editor.error = Some(err.clone());
                            self.agents_menu.editor.saved_message = None;
                            self.set_status_notice(err);
                        }
                    }
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let ch = normalize_char_with_shift(ch, key.modifiers);
                    self.agents_menu.editor_insert_char(ch);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.agents_menu.go_back(),
            KeyCode::Up => self.agents_menu.select_prev(),
            KeyCode::Down => self.agents_menu.select_next(),
            KeyCode::Enter | KeyCode::Right => self.agents_menu.confirm_selection(),
            KeyCode::Left => self.agents_menu.go_back(),
            _ => {}
        }
    }

    fn handle_diff_viewer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.diff_viewer.close(),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => self.diff_viewer.switch_pane(),
            KeyCode::Char('d') => {
                let root = self.project_root();
                self.diff_viewer.toggle_diff_type(&root);
            }
            KeyCode::Up => {
                if self.diff_viewer.active_pane == DiffPane::FileList {
                    self.diff_viewer.select_prev();
                } else {
                    self.diff_viewer.scroll_detail_up();
                }
            }
            KeyCode::Down => {
                if self.diff_viewer.active_pane == DiffPane::FileList {
                    self.diff_viewer.select_next();
                } else {
                    self.diff_viewer.scroll_detail_down();
                }
            }
            KeyCode::PageUp => self.diff_viewer.scroll_detail_up(),
            KeyCode::PageDown => self.diff_viewer.scroll_detail_down(),
            KeyCode::Char(' ') if self.diff_viewer.active_pane == DiffPane::FileList => {
                self.diff_viewer.toggle_file_collapse();
            }
            _ => {}
        }
    }

    fn handle_help_overlay_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => {
                self.help_overlay.close();
                self.show_help = false;
            }
            KeyCode::Char('?')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.help_overlay.close();
                self.show_help = false;
            }
            KeyCode::Up => {
                self.help_overlay.scroll_up();
            }
            KeyCode::Down => {
                let max = 50u16; // generous upper bound; renderer will clamp
                self.help_overlay.scroll_down(max);
            }
            KeyCode::Backspace => {
                self.help_overlay.pop_filter_char();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.help_overlay.push_filter_char(c);
            }
            _ => {}
        }
        false
    }

    fn handle_history_search_overlay_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.history_search_overlay.close();
            }
            KeyCode::Enter => {
                if let Some(entry) = self
                    .history_search_overlay
                    .current_entry(&self.prompt_input.history)
                {
                    self.set_prompt_text(entry.to_string());
                }
                self.history_search_overlay.close();
            }
            KeyCode::Up => {
                self.history_search_overlay.select_prev();
            }
            KeyCode::Down => {
                self.history_search_overlay.select_next();
            }
            KeyCode::Backspace => {
                let history = self.prompt_input.history.clone();
                self.history_search_overlay.pop_char(&history);
            }
            // 'p' with no modifiers and an empty query = pin/unpin the selected entry.
            // When the query is non-empty 'p' is treated as a filter character so
            // the user can still search for prompts containing the letter 'p'.
            KeyCode::Char('p')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.history_search_overlay.query.is_empty() =>
            {
                self.history_search_overlay.toggle_pin();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = normalize_char_with_shift(c, key.modifiers);
                let history = self.prompt_input.history.clone();
                self.history_search_overlay.push_char(c, &history);
            }
            _ => {}
        }
        false
    }

    /// (plan 05) Keys inside the fullscreen rewind selector.
    fn handle_rewind_selector_key(&mut self, key: KeyEvent) -> bool {
        // Filter mode eats printable keys until Enter/Esc.
        if self.rewind_selector.filter_open {
            match key.code {
                KeyCode::Esc => {
                    self.rewind_selector.filter_open = false;
                }
                KeyCode::Enter => {
                    // Enter in the filter jumps to the first match.
                    let matches = self.rewind_selector.filtered_indices();
                    if let Some(&first) = matches.first() {
                        self.rewind_selector.selected = first;
                    }
                    self.rewind_selector.filter_open = false;
                    self.queue_rewind_preview();
                }
                KeyCode::Backspace => {
                    self.rewind_selector.filter.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.rewind_selector.filter.push(c);
                    // Live-jump: keep the cursor on the first match.
                    let matches = self.rewind_selector.filtered_indices();
                    if let Some(&first) = matches.first() {
                        self.rewind_selector.selected = first;
                    }
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.rewind_selector.close();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.rewind_selector.select_prev();
                self.queue_rewind_preview();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.rewind_selector.select_next();
                self.queue_rewind_preview();
            }
            KeyCode::PageUp => {
                self.rewind_selector.scroll = self.rewind_selector.scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.rewind_selector.scroll += 10;
            }
            KeyCode::Char('f') | KeyCode::Char('/') => {
                self.rewind_selector.filter_open = true;
            }
            KeyCode::Enter => {
                // Refuse while a turn is running (the plan contract): the
                // fork writes the store and must not race a live turn.
                if self.is_streaming {
                    self.rewind_selector.notice =
                        Some("A turn is still running — wait for it to finish.".to_string());
                    return false;
                }
                if let Some(entry) = self
                    .rewind_selector
                    .entries
                    .get(self.rewind_selector.selected)
                {
                    if entry.compacted {
                        self.rewind_selector.notice = Some(
                            "That turn was folded by compaction — pick a later one.".to_string(),
                        );
                        return false;
                    }
                    self.pending_rewind = Some(entry.turn_id);
                }
            }
            _ => {}
        }
        false
    }

    fn handle_global_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.global_search.close();
            }
            KeyCode::Enter => {
                if let Some(selected) = self.global_search.selected_ref() {
                    self.set_prompt_text(selected);
                }
                self.global_search.close();
            }
            KeyCode::Up => self.global_search.select_prev(),
            KeyCode::Down => self.global_search.select_next(),
            KeyCode::Backspace => {
                self.global_search.pop_char();
                self.refresh_global_search();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let c = normalize_char_with_shift(c, key.modifiers);
                self.global_search.push_char(c);
                self.refresh_global_search();
            }
            _ => {}
        }
        false
    }

    fn handle_exit_key_confirmation(&mut self, mut key_char: char) {
        fn exit_message(key: char) -> &'static str {
            if key == 'c' {
                "Press Ctrl+C again to exit"
            } else {
                "Press Ctrl+D again to exit"
            }
        }

        // Check if we have an active warning within the timeout
        if let Some(warning_time) = self.last_exit_key_warning
            && warning_time.elapsed().as_secs_f64() <= 2.0
        {
            if self.exit_key_sequence_start == Some(key_char) {
                // Matching key - exit
                self.should_exit = true;
                self.last_exit_key_warning = None;
                self.exit_key_sequence_start = None;
                return;
            }
            if let Some(other_key) = self.exit_key_sequence_start {
                // Wrong key pressed - show message for the original key and reset timer
                key_char = other_key;
            }
        }

        // Start new sequence (or show message for wrong key)
        self.push_notice(NotificationKind::Info, exit_message(key_char).to_string());
        self.last_exit_key_warning = Some(std::time::Instant::now());
        self.exit_key_sequence_start = Some(key_char);
    }

    // (iter-164: fn handle_keybinding_action deleted — unused after keybinding processor removal)

    // Resolve the currently-shown permission dialog by mapping the selected
    // option to a `ToolPermissionResponse` and sending it to the agent.
    // Drops the dialog state and the response sender regardless of whether
    // the agent is still listening (send fails silently if the agent has
    // already timed out / been dropped).
    //
    // Option key mapping:
    //   `y` → AllowOnce
    //   `Y` → AllowSession
    //   `p` (persistent) → AllowSession (no persistent store wired yet —
    //       session-scoped is the closest equivalent)
}

/// True when the prompt holds a `/steer` or `/queue` command — the only
/// input the loop steers on rather than starting a new turn, so Enter
/// must reach the submit path even while a turn is streaming.
fn is_steer_command_input(text: &str) -> bool {
    let Some(rest) = text.trim().strip_prefix('/') else {
        return false;
    };
    matches!(
        rest.split_whitespace().next().unwrap_or_default(),
        "steer" | "queue"
    )
}

// ── (plan 05B) The unified inline picker's key handling ─────────────────────
impl App {
    /// jcode's `handle_inline_interactive_key`, the Model-picker subset:
    /// Up/Down navigate rows (or the active row's options in column ≥ 1),
    /// Left/Right/Tab cycle the column, typing filters (case-insensitive
    /// substring — jcode's fuzzy matcher is not ported), Enter drills into
    /// the row's options and applies on the last column, Esc clears the
    /// filter first and closes second. Returns true when the key was
    /// consumed (the caller must not forward it).
    pub(super) fn handle_inline_interactive_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> bool {
        let Some(picker) = self.inline_interactive.as_mut() else {
            return false;
        };
        match code {
            KeyCode::Esc => {
                if !picker.filter.is_empty() {
                    picker.filter.clear();
                    Self::apply_inline_interactive_filter(picker);
                } else {
                    self.inline_interactive = None;
                }
                true
            }
            KeyCode::Up => {
                if picker.column == 0 {
                    picker.selected = picker.selected.saturating_sub(1);
                } else if let Some(&idx) = picker.filtered.get(picker.selected) {
                    let entry = &mut picker.entries[idx];
                    entry.selected_option = entry.selected_option.saturating_sub(1);
                }
                true
            }
            KeyCode::Down => {
                if picker.column == 0 {
                    if picker.selected + 1 < picker.filtered.len() {
                        picker.selected += 1;
                    }
                } else if let Some(&idx) = picker.filtered.get(picker.selected) {
                    let entry = &mut picker.entries[idx];
                    if entry.selected_option + 1 < entry.options.len() {
                        entry.selected_option += 1;
                    }
                }
                true
            }
            KeyCode::Left => {
                if picker.column > 0 {
                    picker.column -= 1;
                }
                true
            }
            KeyCode::Tab | KeyCode::Right => {
                let max_col = picker
                    .filtered
                    .get(picker.selected)
                    .and_then(|&idx| picker.entries.get(idx))
                    .map(|e| e.options.len().saturating_sub(1))
                    .unwrap_or(0);
                if picker.column < max_col {
                    picker.column += 1;
                }
                true
            }
            KeyCode::Backspace => {
                picker.filter.pop();
                Self::apply_inline_interactive_filter(picker);
                true
            }
            KeyCode::Enter => {
                // Apply the selected row. `picker` is borrowed mutably
                // above; take the clone out, then act on `self`.
                let Some(&idx) = picker.filtered.get(picker.selected) else {
                    return true;
                };
                let entry = picker.entries[idx].clone();
                // Column drill (jcode): a multi-option row steps into its
                // options before applying; a one-option row applies at once.
                if picker.column == 0 && entry.options.len() > 1 {
                    picker.column = 1;
                    return true;
                }
                if picker.column == 1 && entry.options.len() > 2 {
                    picker.column = 2;
                    return true;
                }
                self.inline_interactive = None;
                self.apply_inline_picker_entry(&entry);
                true
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                picker.filter.push(c);
                Self::apply_inline_interactive_filter(picker);
                true
            }
            _ => true,
        }
    }

    /// Case-insensitive substring filter (jcode's fuzzy matcher is not
    /// ported; the model catalogue is small enough that this is exact).
    pub(crate) fn apply_inline_interactive_filter(
        picker: &mut crate::tui::operant_app::tui_fns::InlineInteractiveState,
    ) {
        if picker.filter.is_empty() {
            picker.filtered = (0..picker.entries.len()).collect();
            return;
        }
        let needle = picker.filter.to_lowercase();
        picker.filtered = picker
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.name.to_lowercase().contains(&needle)
                    || e.options
                        .iter()
                        .any(|o| o.provider.to_lowercase().contains(&needle))
            })
            .map(|(i, _)| i)
            .collect();
        if picker.selected >= picker.filtered.len() {
            picker.selected = 0;
        }
    }

    /// The apply step for the ONE action the picker carries today: a Model
    /// row routes through plan 05A's `pending_model`/`pending_effort`
    /// drain — the agent switches and the config persists from there.
    fn apply_inline_picker_entry(&mut self, entry: &crate::tui::operant_app::tui_fns::PickerEntry) {
        use crate::tui::operant_app::tui_fns::PickerAction;
        match entry.action {
            PickerAction::Model => {
                self.pending_model = Some(entry.name.clone());
                if let Some(effort) = entry.effort.clone() {
                    self.pending_effort = Some(effort);
                }
                self.set_status_notice(format!("Model: {} (applying…)", entry.name));
            }
            _ => {
                self.set_status_notice(format!("{} — no action in this picker", entry.name));
            }
        }
    }
}
