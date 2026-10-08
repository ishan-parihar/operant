//! Key event handling methods.

use super::*;

/// How many counted key presses between flushes of the usage store.
///
/// The store is already written on every slash command, so this only bounds
/// the loss window for a long session that never runs one.
const KEY_USAGE_FLUSH_EVERY: u32 = 64;

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

        // Dismiss error modal with Esc
        if key.code == KeyCode::Esc && self.notifications.current_is_error() {
            self.dismiss_error_notifications();
            return false;
        }

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
                    self.status_message = Some(
                        "Bypass permissions mode enabled — permissions will be auto-approved. Use with care.".to_string(),
                    );
                    self.bypass_permissions_dialog.dismiss();
                }
                KeyCode::Up | KeyCode::Char('k') => self.bypass_permissions_dialog.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.bypass_permissions_dialog.select_next(),
                KeyCode::Enter => {
                    if self.bypass_permissions_dialog.is_accept_selected() {
                        self.arm_bypass_permissions();
                        self.status_message = Some(
                            "Bypass permissions mode enabled — permissions will be auto-approved. Use with care.".to_string(),
                        );
                    }
                    self.bypass_permissions_dialog.dismiss();
                }
                _ => {}
            }
            return false;
        }

        // Effort picker dialog (/effort).
        if self.effort_picker.visible {
            match key.code {
                KeyCode::Esc => self.effort_picker.close(),
                KeyCode::Up | KeyCode::Char('k') => self.effort_picker.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.effort_picker.select_next(),
                KeyCode::Enter => {
                    let chosen = self.effort_picker.current();
                    self.effort_level = chosen;
                    self.effort_picker.close();
                    self.status_message = Some(format!(
                        "Effort set to {} {}.",
                        chosen.symbol(),
                        chosen.label()
                    ));
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
                            self.push_notification(
                                NotificationKind::Warning,
                                "Clipboard is empty".to_string(),
                                Some(2),
                            );
                        } else {
                            for ch in text.chars() {
                                self.key_input_dialog.insert_char(ch);
                            }
                        }
                    } else {
                        self.push_notification(
                            NotificationKind::Warning,
                            "Could not read clipboard".to_string(),
                            Some(2),
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

        // Model picker intercepts navigation and Esc
        if self.model_picker.visible {
            match key.code {
                KeyCode::Esc => self.model_picker.close(),
                KeyCode::Home => self.model_picker.select_first(),
                KeyCode::End => self.model_picker.select_last(),
                KeyCode::Up => self.model_picker.select_prev(),
                KeyCode::Down => self.model_picker.select_next(),
                KeyCode::Left => self.model_picker.effort_prev(),
                KeyCode::Right => self.model_picker.effort_next(),
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.model_picker.select_prev()
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.model_picker.select_next()
                }
                KeyCode::Enter => {
                    if let Some((model_id, effort)) = self.model_picker.confirm() {
                        // If user picked a model other than the fast-mode model
                        // while fast mode was active, turn fast mode off.
                        if self.fast_mode
                            && !self.model_picker.is_selected_fast_mode_model(&model_id)
                        {
                            self.fast_mode = false;
                        }
                        if let Some(e) = effort {
                            self.effort_level = e;
                        }
                        // Store explicit selections in the canonical
                        // "provider/model" form for non-Anthropic providers.
                        // The "free" composite's picker entries already carry
                        // a routing prefix (`free/…`, `zen/…`, `openrouter/…`)
                        // so re-prefixing would produce nonsense like
                        // `free/free/auto`. Also, OpenRouter catalog entries
                        // are already prefixed with `openrouter/…` — check
                        // for that to avoid `openrouter/openrouter/anthropic/…`.
                        // (Bug #14 from iter-82 audit.)
                        let provider = self.active_provider.as_deref().unwrap_or("anthropic");
                        let prefix = format!("{}/", provider);
                        let full_model = if provider == "anthropic" || provider == "free" {
                            model_id.clone()
                        } else if model_id.starts_with(&prefix) {
                            // Already prefixed (e.g. openrouter/anthropic/claude-…).
                            model_id.clone()
                        } else {
                            format!("{}/{}", provider, model_id)
                        };
                        self.set_model(full_model.clone());
                        self.persist_provider_and_model();
                        let effort_hint = effort
                            .map(|e| format!(" [{}]", e.label()))
                            .unwrap_or_default();
                        self.status_message = Some(format!("Model: {}{}", full_model, effort_hint));
                    }
                }
                KeyCode::Backspace => self.model_picker.pop_filter_char(),
                KeyCode::Char(c) => self.model_picker.push_filter_char(c),
                _ => {}
            }
            return false;
        }

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
                            self.status_message =
                                Some(format!("Switched to branch: {}", branch.name));
                            self.session_branching.close();
                        }
                    }
                    _ => {}
                },
                BranchBrowserMode::CreateNew => match key.code {
                    KeyCode::Esc => self.session_branching.cancel(),
                    KeyCode::Enter => {
                        if let Some((name, at_msg)) = self.session_branching.confirm_create_new() {
                            self.status_message =
                                Some(format!("Created branch: {} at message {}", name, at_msg));
                            self.session_branching.close();
                        }
                    }
                    KeyCode::Backspace => self.session_branching.pop_create_char(),
                    KeyCode::Char(c) => self.session_branching.push_create_char(c),
                    _ => {}
                },
                BranchBrowserMode::ConfirmDelete => match key.code {
                    KeyCode::Esc | KeyCode::Char('n') => self.session_branching.cancel(),
                    KeyCode::Enter | KeyCode::Char('y') => {
                        if let Some(branch_id) = self.session_branching.confirm_delete() {
                            self.status_message = Some(format!("Deleted branch: {}", branch_id));
                        }
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
                            self.status_message = Some(format!("Renamed to: {}", name));
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
                        self.push_notification(
                            NotificationKind::Info,
                            format!("Exported to {}", path),
                            Some(4),
                        );
                    } else {
                        self.push_notification(
                            NotificationKind::Warning,
                            "Export failed: could not write file.".to_string(),
                            Some(4),
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
        // Rewind flow overlay intercepts keys first
        if self.rewind_flow.visible {
            return self.handle_rewind_flow_key(key);
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

        // Notification dismiss
        if key.code == KeyCode::Esc && !self.notifications.is_empty() {
            self.notifications.dismiss_current();
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

        // Clear any active text selection on key press, except the two chords
        // that act *on* a selection: Ctrl+C copies it, Ctrl+T enters copy mode
        // (which has to hand the prior selection back on exit).
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let is_copy = key.code == KeyCode::Char('c') && ctrl;
        let is_copy_mode = key.code == KeyCode::Char('t') && ctrl;
        if !is_copy && !is_copy_mode && self.selection_anchor.is_some() {
            self.selection_anchor = None;
            self.selection_focus = None;
            *self.selection_text.borrow_mut() = String::new();
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
                self.push_notification(NotificationKind::Info, msg, Some(3));
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
                self.is_streaming = false;
                self.spinner_verb = None;
                // Flush in-flight streaming text to messages BEFORE snapshot
                // so the response is preserved in the transcript.
                self.flush_streamed_assistant_message();
                self.status_message = Some("Cancelled.".to_string());
                // Abort the background agent task so it actually stops.
                if let Some(handle) = self.agent_task_handle.take() {
                    handle.abort();
                }
                // Snapshot AFTER flushing so tool trail is preserved.
                self.complete_current_turn_snapshot(true);
                self.tool_use_blocks.clear();
            }

            // ---- Quit / cancel ----------------------------------------
            // Accept both 'c' and 'C' so Shift+Ctrl+C also triggers copy
            // (issue #149 follow-up).
            KeyCode::Char(c)
                if (c == 'c' || c == 'C') && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // If text is selected, copy it to clipboard instead of quitting.
                let sel_text = self.selection_text.borrow().clone();
                if self.selection_anchor.is_some() && !sel_text.is_empty() {
                    // Text is selected: copy to clipboard.
                    let outcome = crate::tui::clipboard::copy(&sel_text);
                    self.selection_anchor = None;
                    self.selection_focus = None;
                    *self.selection_text.borrow_mut() = String::new();
                    self.push_notification(
                        if outcome.is_copied() {
                            NotificationKind::Info
                        } else {
                            NotificationKind::Warning
                        },
                        outcome.status_message(),
                        Some(2),
                    );
                } else if self.is_streaming {
                    // Cancel streaming.
                    self.is_streaming = false;
                    self.spinner_verb = None;
                    // Flush in-flight streaming text to messages BEFORE snapshot
                    // so the response is preserved in the transcript.
                    self.flush_streamed_assistant_message();
                    self.status_message = Some("Cancelled.".to_string());
                    self.complete_current_turn_snapshot(true);
                    self.tool_use_blocks.clear();
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
                if !self.is_streaming && self.has_credentials {
                    self.open_model_picker_for_provider(
                        &self.active_provider.clone().unwrap_or_default(),
                        None,
                    );
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
            }

            // ---- Scroll bookmark (Ctrl+G) -------------------------------
            // Toggles: first press remembers the current spot, second press
            // jumps back to it and disarms. The status row names the state so
            // the toggle is discoverable without opening the help overlay.
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_scroll_bookmark();
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
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.enter_copy_mode();
            }

            // ---- Alt/Meta key text editing operations -------------------
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.prompt_input.yank_pop();
                self.refresh_prompt_input();
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
                if key.modifiers.contains(KeyModifiers::SUPER) {
                    self.prompt_input.cursor = 0;
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.prompt_input.move_word_backward();
                } else {
                    self.prompt_input.move_left();
                }
                self.sync_legacy_prompt_fields();
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SUPER) {
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

            // ---- Shift+Tab: cycle permission mode ----------------------
            // Default → AcceptEdits → BypassPermissions → Default
            // Mirrors TS bottom-left indicator cycling behaviour.
            KeyCode::BackTab if !self.is_streaming => {
                use crate::tui::adapter_types::config::PermissionMode;
                self.settings.permission_mode = match self.settings.permission_mode {
                    PermissionMode::Default => PermissionMode::AcceptEdits,
                    PermissionMode::AcceptEdits => PermissionMode::BypassPermissions,
                    PermissionMode::BypassPermissions => PermissionMode::Default,
                    PermissionMode::Plan => PermissionMode::Default,
                };
                let label = match self.settings.permission_mode {
                    PermissionMode::Default => "Default permissions",
                    PermissionMode::AcceptEdits => "Accept-edits mode",
                    PermissionMode::BypassPermissions => "Bypass permissions (dangerous)",
                    PermissionMode::Plan => "Plan mode",
                };
                self.status_message = Some(label.to_string());
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
                    self.status_message = Some("Starting MCP auth...".to_string());
                }
            }
            KeyCode::Char('r') => {
                self.pending_mcp_reconnect = true;
                self.status_message = Some("Reconnecting MCP runtime...".to_string());
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
                        Ok(msg) => self.status_message = Some(msg),
                        Err(err) => {
                            self.agents_menu.editor.error = Some(err.clone());
                            self.agents_menu.editor.saved_message = None;
                            self.status_message = Some(err);
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

    fn handle_rewind_flow_key(&mut self, key: KeyEvent) -> bool {
        use crate::tui::overlays::RewindStep;
        match &self.rewind_flow.step {
            RewindStep::Selecting => match key.code {
                KeyCode::Esc => {
                    self.rewind_flow.close();
                }
                KeyCode::Enter => {
                    self.rewind_flow.confirm_selection();
                }
                KeyCode::Up => {
                    self.rewind_flow.selector.select_prev();
                }
                KeyCode::Down => {
                    self.rewind_flow.selector.select_next();
                }
                _ => {}
            },
            RewindStep::Confirming { .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(idx) = self.rewind_flow.accept_confirm() {
                        // Truncate conversation to the selected message index.
                        self.messages.truncate(idx);
                        // Remove system annotations placed after the truncation point.
                        self.system_annotations.retain(|a| a.after_index <= idx);
                        self.push_notification(
                            NotificationKind::Success,
                            format!("Rewound to message #{}", idx),
                            Some(4),
                        );
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.rewind_flow.reject_confirm();
                }
                _ => {}
            },
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
        self.push_notification(
            NotificationKind::Info,
            exit_message(key_char).to_string(),
            Some(2),
        );
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
