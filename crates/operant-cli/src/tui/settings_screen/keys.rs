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
            toggle_or_cycle_current(screen, config);
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
        // (plan 2026-11-16 S2) Tab switching. Brackets, not bare letters: every
        // letter is the search field, so `[`/`]` is the only pair that cannot be
        // typed into a query. Home/End jump the strip end to end.
        KeyCode::Char('[') => step_tab(screen, -1),
        KeyCode::Char(']') => step_tab(screen, 1),
        KeyCode::Home => screen.select_tab(SettingsTab::ALL[0]),
        KeyCode::End => screen.select_tab(SettingsTab::ALL[SettingsTab::ALL.len() - 1]),
        KeyCode::Char(c) => {
            screen.push_search_char(c);
        }
        _ => {}
    }
    *settings = screen.settings_snapshot.clone();
    true
}

/// Move one tab left (`-1`) or right (`+1`), wrapping at both ends.
///
/// Wrapping, not clamping: a strip of eight tabs is short enough that an
/// operator who holds `]` expects to come back to the start, and a clamp makes
/// the key appear to die at the last tab. `select_tab` does the clamping of the
/// ROW index, so a tab with nothing on it cannot leave the marker stranded.
fn step_tab(screen: &mut SettingsScreen, delta: isize) {
    let all = SettingsTab::ALL;
    let current = all
        .iter()
        .position(|tab| *tab == screen.selected_tab)
        .unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(all.len() as isize) as usize;
    screen.select_tab(all[next]);
}

/// Move the settings screen's selection UP one row — the rule ↑ uses.
///
/// (iter-1856) The wheel needs the SAME rule as the arrow keys: the visible
/// count is the FILTERED entry count (the search query hides rows), and the
/// cursor must stay in view. Both facts live in this module — the key layer
/// owns the rule, and the wheel is a vertical key.
pub(crate) fn select_prev_visible(screen: &mut SettingsScreen) {
    screen.select_prev();
}

/// Move the settings screen's selection DOWN one row — the rule ↓ uses.
/// See `select_prev_visible` for why this lives here.
pub(crate) fn select_next_visible(screen: &mut SettingsScreen) {
    // (plan 2026-11-16 S2) The ONE filter: tab, then query. This counted its own
    // list while the painter sliced another, which is iter-1917's two-windows
    // defect. Both now read `visible_entries()`.
    let count = screen.visible_entries().len();
    screen.select_next(count);
}

pub(crate) fn toggle_or_cycle_current(screen: &mut SettingsScreen, config: &mut AppConfig) {
    // (iter-2452) ONE APPLY SEAM, NOT TWO.
    //
    // This function used to carry its own copy of every write: seven match arms
    // naming individual keys, and `_ => {}` for everything else. The panel draws
    // ~270 registry rows, so Enter did NOTHING on 269 of them — the rows
    // rendered, the cursor moved, and the operator's edit vanished. `apply_and_save`
    // already had the complete generic path (known-key check, `persist_config_key`
    // through `toml_edit`, reload into the live process, both report states), so
    // the fix is to route through it rather than to grow the duplicate by 269
    // more arms.
    //
    // Only the `settings.json` visual state needs a bespoke write: those keys are
    // deliberately not config keys and must not leak into a shared `operant.toml`.
    let selected = screen
        .visible_entries()
        .get(screen.selected_idx)
        .map(|entry| (entry.key, entry.value.clone(), entry.kind.clone()));

    let Some((key, value, kind)) = selected else {
        return;
    };
    let _ = SettingStore::SettingsJson;

    // The next value this row would take.
    let next = match kind {
        SettingKind::Bool => (value != "true").to_string(),
        SettingKind::Enum { ref options } => {
            if options.is_empty() {
                return;
            }
            let at = options
                .iter()
                .position(|option| *option == value)
                .unwrap_or(0);
            options[(at + 1) % options.len()].to_string()
        }
        // Number and Text open the inline editor; Enter there is handled by the
        // editing branch above, so reaching here means nothing to cycle.
        SettingKind::Number | SettingKind::Text | SettingKind::Unread => return,
    };

    // A key nothing reads is never written. Enter on it says so rather than doing
    // nothing, because a row that silently ignores the operator is the phantom
    // surface the registry's `live` flag exists to keep out of the panel.
    if kind_is_unread(&kind) {
        screen.apply_error = Some(format!(
            "{key} is accepted but unread — no code reads it, so the panel cannot change it."
        ));
        return;
    }

    // Both stores go through `apply_and_save`: its arms name the `settings.json`
    // keys (reduce_motion, show_cwd, …) and its `_` arm is the generic config-key
    // writer. What a row IS decides which arm takes it, so the caller does not
    // need to know — which is the whole point of one seam.
    screen.pending_changes.clear();
    screen.pending_changes.insert(key.to_string(), next);
    let mut settings = screen.settings_snapshot.clone();
    screen.apply_and_save(config, &mut settings);
}

/// Whether this kind is the deliberate no-op the panel uses for env-shadowed keys.
fn kind_is_unread(kind: &SettingKind) -> bool {
    matches!(kind, SettingKind::Unread)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
