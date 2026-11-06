//! Dialog routing, permission handling, and key context methods.

use super::channel_drains::plain_command_prefix;
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
        if self.connect_dialog.visible {
            return Some(DialogPriority::Connect);
        }
        if self.import_config_picker.visible {
            return Some(DialogPriority::ImportConfigPicker);
        }
        if self.command_palette.visible {
            return Some(DialogPriority::CommandPalette);
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
        if self.session_hub.visible {
            return Some(DialogPriority::SessionHub);
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
        if self.rewind_selector.visible {
            return Some(DialogPriority::RewindFlow);
        }
        if self.memory_file_selector.visible {
            return Some(DialogPriority::MemoryFileSelector);
        }
        // (RT-257) Three MORE surfaces were gated inline and absent from the
        // chain — the same omission the three above were recovered from, and
        // it happened again because the guard the comment in key_handling.rs
        // names (`dialog_covers_every_gated_surface`) was never written. They
        // are appended for the same reason: the chain's order already diverges
        // from the inline order, so claiming a "correct" slot would be false
        // precision. PRESENCE is what matters, and the guard now enforces it
        // against the overlay catalogue rather than against a hand-kept list.
        if self.inline_interactive.is_some() {
            return Some(DialogPriority::InlineInteractive);
        }
        if self.permissions_picker.visible {
            return Some(DialogPriority::PermissionsPicker);
        }
        if self.permission_request.is_some() {
            return Some(DialogPriority::PermissionRequest);
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
        // (plan 03 §3) LAST: the next queued request, if one arrived while
        // this dialog was open, opens now — a parallel batch's second gated
        // tool used to be silently denied here. Opening sets the turn back
        // to WaitingForApproval, so it must run after the state reset above.
        self.open_next_permission_request();
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
                    // '0' is not an option key (options number from 1): the
                    // old saturating_sub mapped it to index 0, silently
                    // selecting the FIRST option — the one action '0' most
                    // plausibly was not meant to take.
                    if digit >= 1 {
                        let idx = (digit as usize).saturating_sub(1);
                        if idx < pr.options.len() {
                            pr.selected_option = idx;
                        }
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
        // The same guard that decided the option was offered decides what is
        // stored, so the entry is the two-token prefix ("git status", not
        // "git": a one-token "git" would approve "git push --force").
        if let PermissionDialogKind::Bash { command, .. } = &pr.kind
            && let Some(entry) = plain_command_prefix(command)
        {
            self.bash_prefix_allowlist.insert(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Variant names of `DialogPriority`, read from its own source so the
    /// guard cannot go stale when a variant is added.
    ///
    /// (RT-257) This reader used to take the enum's DOC-COMMENT lines and
    /// keep only alphanumeric ones — and every doc comment in this enum
    /// contains a space, so it kept NOTHING and the guard below compared an
    /// empty list: it passed while checking no variant at all. The variant
    /// lines are the ones that matter (`Name = value,`), so the reader takes
    /// those and skips the prose lines.
    fn priority_variants() -> Vec<String> {
        include_str!("enums.rs")
            .lines()
            .skip_while(|l| !l.contains("pub enum DialogPriority"))
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('}'))
            .filter_map(|l| {
                let t = l.trim();
                if t.starts_with("///") || t.starts_with("//") {
                    return None;
                }
                let name = t.split('=').next()?.trim().to_string();
                let looks_like_variant = !name.is_empty()
                    && name.chars().next()?.is_uppercase()
                    && name.chars().all(|c| c.is_alphanumeric());
                looks_like_variant.then_some(name)
            })
            .collect()
    }

    /// How many variant lines the enum declares — counted INDEPENDENTLY of
    /// the reader above, so a reader that silently returns nothing cannot
    /// make the guard vacuous. (RT-257: the reader used to return an empty
    /// list — it read the prose lines — and the guard passed while checking
    /// no variant at all.)
    fn declared_variant_lines() -> usize {
        include_str!("enums.rs")
            .lines()
            .skip_while(|l| !l.contains("pub enum DialogPriority"))
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('}'))
            .filter(|l| {
                let t = l.trim();
                !t.starts_with("//") && t.contains('=') && t.ends_with(',')
            })
            .count()
    }

    /// Every `DialogPriority` variant except `None` must be REACHABLE — the
    /// chain must be able to return it.
    ///
    /// Precise by construction: it compares the enum against the gates in the
    /// same file, so it cannot pick up unrelated `if self.X.is_some()` checks
    /// that live in `key_handling.rs` for input and scrolling.
    #[test]
    fn every_priority_variant_has_a_gate() {
        let chain = include_str!("dialog_routing.rs");
        let variants = priority_variants();
        assert_eq!(
            variants.len(),
            declared_variant_lines(),
            "the variant reader returned {variants:?} — it must read the enum's VARIANT \
             lines, not its prose (a reader that returns nothing makes this guard vacuous)"
        );
        let mut missing: Vec<String> = variants
            .into_iter()
            .filter(|v| v != "None" && !chain.contains(&format!("Some(DialogPriority::{v})")))
            .collect();
        missing.sort();
        assert!(
            missing.is_empty(),
            "DialogPriority names a variant the chain can never return: {missing:?}"
        );
    }

    /// Every surface in the overlay CATALOGUE must be gated in BOTH places
    /// that route keys: the chain here and the inline chain in
    /// key_handling.rs.
    ///
    /// (RT-257) The comment at the top of key_handling.rs's dispatcher named
    /// this guard — "`dialog_covers_every_gated_surface` fails if one is ever
    /// dropped again" — and the guard was never written. What stood in its
    /// place was a hard-coded list of the THREE surfaces the first repair had
    /// recovered, so the chain drifted again: the inline picker, the
    /// permission request and the permissions picker were gated in the
    /// dispatcher and absent from the chain, exactly as theme_screen,
    /// rewind_flow and memory_file_selector once were.
    ///
    /// The catalogue (`App::overlay_flags()`) is the ONE list of surfaces the
    /// UI has, so the guard is derived from it: a surface added to the
    /// catalogue without a gate in either place fails here, and a gate
    /// dropped from either chain fails here.
    #[test]
    fn dialog_covers_every_gated_surface() {
        let app = crate::tui::app::tests::make_app();
        let chain = include_str!("dialog_routing.rs");
        let dispatcher = include_str!("key_handling.rs");

        let mut problems: Vec<String> = Vec::new();
        for (surface, _) in app.overlay_flags() {
            let in_chain = chain.contains(&format!("self.{surface}.visible"))
                || chain.contains(&format!("self.{surface}.is_some()"));
            let in_dispatcher = dispatcher.contains(&format!("if self.{surface}.visible"))
                || dispatcher.contains(&format!("if self.{surface}.is_some()"));
            if !in_chain {
                problems.push(format!(
                    "{surface} is in the overlay catalogue but the priority chain does not gate it"
                ));
            }
            if !in_dispatcher {
                problems.push(format!(
                    "{surface} is in the overlay catalogue but the dispatcher does not gate it"
                ));
            }
        }
        assert!(
            problems.is_empty(),
            "the overlay catalogue and the key-routing chains disagree:\n{}",
            problems.join("\n")
        );
    }
}
