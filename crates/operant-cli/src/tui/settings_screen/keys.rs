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
            select_prev_visible(screen);
        }
        KeyCode::Down => {
            select_next_visible(screen);
        }
        KeyCode::Char(c) => {
            screen.push_search_char(c);
        }
        _ => {}
    }
    *settings = screen.settings_snapshot.clone();
    true
}

/// Move the settings screen's selection UP one row — the rule ↑ uses.
///
/// (iter-1856) The wheel needs the SAME rule as the arrow keys: the visible
/// count is the FILTERED entry count (the search query hides rows), and the
/// cursor must stay in view. Both facts live in this module — the key layer
/// owns the rule, and the wheel is a vertical key.
pub(crate) fn select_prev_visible(screen: &mut SettingsScreen) {
    screen.select_prev();
    update_scroll_offset_for_selection(screen);
}

/// Move the settings screen's selection DOWN one row — the rule ↓ uses.
/// See `select_prev_visible` for why this lives here.
pub(crate) fn select_next_visible(screen: &mut SettingsScreen) {
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
                    "reduce_motion" => {
                        screen.reduce_motion = new_value;
                        screen.settings_snapshot.reduce_motion = new_value;
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_cwd" => {
                        screen.show_cwd = new_value;
                        screen.settings_snapshot.show_cwd = Some(new_value);
                        let _ = screen.settings_snapshot.save_sync();
                    }
                    "show_git_branch" => {
                        screen.show_git_branch = new_value;
                        screen.settings_snapshot.show_git_branch = Some(new_value);
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

                // (iter-1900) `output_format` was the other arm here until
                // its dead row went; with one arm left, a match is a
                // single-match and clippy says so.
                if entry.key == "output_style" {
                    screen.output_style = new_value.to_string();
                    screen.settings_snapshot.config.output_style = Some(new_value.to_string());
                    let _ = screen.settings_snapshot.save_sync();
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
