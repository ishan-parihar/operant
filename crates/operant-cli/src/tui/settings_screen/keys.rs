// settings_screen/keys.rs — Settings screen key handling + edit helpers.
//
// Extracted from the settings_screen.rs monolith. handle_settings_key routes
// keys; scroll/toggle helpers update state.

use super::*;

pub fn handle_settings_key(
    screen: &mut SettingsScreen,
    config: &mut AppConfig,
    settings: &mut Settings,
    key: crossterm::event::KeyEvent,
) -> bool {
    use crossterm::event::KeyCode;

    if !screen.visible {
        return false;
    }

    // Editing mode
    if screen.edit_field.is_some() {
        match key.code {
            KeyCode::Enter => {
                screen.commit_edit();
                screen.apply_and_save(config, settings);
            }
            KeyCode::Esc => {
                screen.cancel_edit();
            }
            KeyCode::Backspace => {
                screen.edit_value.pop();
            }
            KeyCode::Char(c) => {
                screen.edit_value.push(c);
            }
            _ => {}
        }
        return true;
    }

    // Navigation mode
    match key.code {
        KeyCode::Enter => {
            toggle_or_cycle_current(screen);
        }
        KeyCode::Esc => {
            if !screen.search_query.is_empty() {
                screen.search_query.clear();
                screen.selected_idx = 0;
            } else {
                screen.close();
            }
        }
        KeyCode::Backspace => {
            screen.pop_search_char();
        }
        KeyCode::Up => {
            screen.select_prev();
            update_scroll_offset_for_selection(screen);
        }
        KeyCode::Down => {
            let all = all_entries(screen);
            let filtered: Vec<_> = all
                .iter()
                .filter(|e| {
                    e.label
                        .to_lowercase()
                        .contains(&screen.search_query.to_lowercase())
                })
                .collect();
            screen.select_next(filtered.len());
            update_scroll_offset_for_selection(screen);
        }
        KeyCode::Char(c) => {
            screen.push_search_char(c);
        }
        _ => {}
    }
    *settings = screen.settings_snapshot.clone();
    true
}

fn update_scroll_offset_for_selection(screen: &mut SettingsScreen) {
    let visible_rows = 10; // Rough estimate, will be actual in real usage
    if screen.selected_idx < screen.scroll_offset {
        screen.scroll_offset = screen.selected_idx;
    } else if screen.selected_idx >= screen.scroll_offset + visible_rows {
        screen.scroll_offset = screen.selected_idx.saturating_sub(visible_rows - 1);
    }
}

fn toggle_or_cycle_current(screen: &mut SettingsScreen) {
    let all = all_entries(screen);
    let filtered: Vec<_> = all
        .iter()
        .filter(|e| {
            e.label
                .to_lowercase()
                .contains(&screen.search_query.to_lowercase())
        })
        .collect();

    if let Some(entry) = filtered.get(screen.selected_idx) {
        match entry.kind {
            SettingKind::Bool => {
                let new_value = entry.value != "true";
                match entry.key {
                    "auto_compact" => {
                        screen.auto_compact = new_value;
                        screen.settings_snapshot.auto_compact = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "notifications" => {
                        screen.notifications = new_value;
                        screen.settings_snapshot.notifications = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_turn_duration" => {
                        screen.show_turn_duration = new_value;
                        screen.settings_snapshot.show_turn_duration = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "reduce_motion" => {
                        screen.reduce_motion = new_value;
                        screen.settings_snapshot.reduce_motion = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "terminal_progress_bar" => {
                        screen.terminal_progress_bar = new_value;
                        screen.settings_snapshot.terminal_progress_bar = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "verbose" => {
                        screen.verbose = new_value;
                        screen.settings_snapshot.config.verbose = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "cursor_blink_enabled" => {
                        screen.cursor_blink_enabled = new_value;
                        screen.settings_snapshot.config.cursor_blink_enabled = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "auto_copy_enabled" => {
                        screen.auto_copy_enabled = new_value;
                        screen.settings_snapshot.auto_copy_on_highlight = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_cwd" => {
                        screen.show_cwd = new_value;
                        screen.settings_snapshot.show_cwd = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_git_branch" => {
                        screen.show_git_branch = new_value;
                        screen.settings_snapshot.show_git_branch = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "auto_commits" => {
                        screen.auto_commits = new_value;
                        screen.settings_snapshot.config.auto_commits =
                            if new_value { Some(true) } else { None };
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "disable_claude_mds" => {
                        screen.disable_claude_mds = new_value;
                        screen.settings_snapshot.config.disable_claude_mds = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "fileInjectionEnabled" => {
                        screen.file_injection_enabled = new_value;
                        screen.settings_snapshot.config.file_injection_enabled = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "fileAutocompleteShowHiddenFiles" => {
                        screen.file_autocomplete_show_hidden_files = new_value;
                        screen
                            .settings_snapshot
                            .config
                            .file_autocomplete_show_hidden_files = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    _ => {}
                }
            }
            SettingKind::Enum { ref options } => {
                let current_idx = options.iter().position(|&o| o == entry.value).unwrap_or(0);
                let next_idx = (current_idx + 1) % options.len();
                let new_value = options[next_idx];

                match entry.key {
                    "output_style" => {
                        screen.output_style = new_value.to_string();
                        screen.settings_snapshot.config.output_style = Some(new_value.to_string());
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "output_format" => {
                        screen.output_format = new_value.to_string();
                        screen.settings_snapshot.config.output_format = match new_value {
                            "json" => crate::tui::adapter_types::config::OutputFormat::Json,
                            "stream_json" => {
                                crate::tui::adapter_types::config::OutputFormat::StreamJson
                            }
                            _ => crate::tui::adapter_types::config::OutputFormat::Text,
                        };
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    _ => {}
                }
            }
            SettingKind::Number => {
                screen.start_edit(entry.key, &entry.value);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
