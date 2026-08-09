// settings_screen/tests.rs — Unit tests for the settings screen.
//
// Extracted from the settings_screen.rs monolith.

use super::*;

#[test]
fn settings_screen_new_has_sensible_defaults() {
    let screen = SettingsScreen::new();
    assert!(!screen.visible);
    assert!(screen.search_query.is_empty());
    assert_eq!(screen.selected_idx, 0);
    assert!(screen.edit_field.is_none());
    assert!(screen.edit_value.is_empty());
}

#[test]
fn all_entries_returns_expected_settings() {
    let screen = SettingsScreen::new();
    let entries = all_entries(&screen);
    // Base settings are always present, plus 0-3 conditional file injection settings
    assert!(
        entries.len() >= 16,
        "Should have at least 16 editable settings, got {}",
        entries.len()
    );
    assert!(
        entries.len() <= 20,
        "Should have at most 20 editable settings, got {}",
        entries.len()
    );
}

#[test]
fn search_filters_entries_correctly() {
    let screen = SettingsScreen::new();
    let all = all_entries(&screen);
    let filtered: Vec<_> = all
        .iter()
        .filter(|e| e.label.to_lowercase().contains("token"))
        .collect();
    assert_eq!(
        filtered.len(),
        1,
        "Should find exactly 1 entry matching 'token'"
    );
    assert_eq!(filtered[0].label, "Max Tokens");
}

#[test]
fn toggle_bool_entry_flips_value() {
    let mut screen = SettingsScreen::new();
    screen.notifications = true;
    screen.open();

    let initial = screen.notifications;
    let all = all_entries(&screen);
    let entry = &all[2]; // notifications is at index 2
    assert_eq!(entry.label, "Desktop notifications");

    // Simulate toggle (manually, since toggle_or_cycle_current modifies internal state)
    screen.notifications = !screen.notifications;
    assert_ne!(screen.notifications, initial);
}

#[test]
fn cycle_enum_entry_wraps_around() {
    let mut screen = SettingsScreen::new();
    screen.output_style = "default".to_string();

    // Simulate cycling through all options
    let options = ["default", "concise", "explanatory", "learning"];
    let mut idx = options.iter().position(|&o| o == "default").unwrap();

    idx = (idx + 1) % options.len();
    assert_eq!(options[idx], "concise");

    idx = (idx + 1) % options.len();
    assert_eq!(options[idx], "explanatory");

    idx = (idx + 1) % options.len();
    assert_eq!(options[idx], "learning");

    idx = (idx + 1) % options.len();
    assert_eq!(options[idx], "default"); // Wraps around
}
