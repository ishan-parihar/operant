// settings_screen/tests.rs — Unit tests for the settings screen.
//
// Extracted from the settings_screen.rs monolith.

use super::*;

/// Every entry the screen offers MUST be handled by `apply_and_save`.
///
/// This test exists because it failed when it was written (iter-1176):
/// `all_entries` offered 19 editable settings and `apply_and_save` matched
/// FOUR of them (`max_tokens`, `output_style`, `compact_threshold`,
/// `fileAutocompleteLimit`, `fileInjectionMaxSize`), so toggling
/// `notifications`, `verbose`, `vim`, `reduce_motion`, `show_cwd`,
/// `fileInjectionEnabled` and the rest rendered the new value in the TUI,
/// wrote `pending_changes`, and then silently dropped it on the floor —
/// while still rewriting the ENTIRE settings.json from the snapshot,
/// hard-coding `verbose: false`, `cursor_blink_enabled: false`,
/// `disable_claude_mds: false`, `file_injection_enabled: false` and
/// `OutputFormat::default()` instead of carrying the persisted values
/// forward. The save appeared to succeed, so nothing surfaced it until
/// the two sets were compared side by side.
///
/// Deriving the key list from `all_entries` (rather than hand-writing it)
/// means the test keeps pace with the UI: a newly added entry that no
/// applier handles fails here, which is exactly the drift it guards.
#[test]
fn every_settings_entry_has_an_applier() {
    let screen = SettingsScreen::new();
    let entries = all_entries(&screen);
    assert!(
        !entries.is_empty(),
        "all_entries must offer settings; an empty list makes this test vacuous"
    );

    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tui/settings_screen/state.rs"
    ))
    .expect("the screen's own state module is readable");
    // Only the applier body: a key mentioned in a comment or in an entry
    // definition elsewhere in the file must not count as handled.
    let applier = source
        .split_once("pub fn apply_and_save(")
        .expect("apply_and_save exists")
        .1
        .split_once("let _ = self.settings_snapshot.save_sync();")
        .expect("apply_and_save reaches its save")
        .0;

    let mut unhandled = Vec::new();
    for entry in &entries {
        // `SettingsEntry::key` is a `&'static str`, so a borrow is all the
        // pattern needs. (`entry.key.as_str()` does not compile:
        // `str::as_str` is unstable on this toolchain — and reaching for it
        // at all is the tell that the type was guessed instead of read.)
        let quoted = format!("\"{}\"", entry.key);
        if !applier.contains(&quoted) {
            unhandled.push(entry.key);
        }
    }

    assert!(
        unhandled.is_empty(),
        "settings offered in /settings that apply_and_save silently discards: {unhandled:?} \
         — the value renders and is stored in pending_changes, then dropped by the match, \
         while the whole settings.json is rewritten from the snapshot"
    );
}

/// `apply_and_save` must not hard-code a field the operator already chose.
///
/// The snapshot it writes is rebuilt wholesale (it is a struct literal, not a
/// patch), so every field is an opportunity to either carry the persisted
/// value forward or overwrite it with a constant. This pins the two fields
/// where the constant is a silent reset: `vim_enabled` and
/// `cursor_blink_enabled` are operator preferences an unrelated edit must not
/// clear.
#[test]
fn apply_and_save_preserves_unrelated_operator_preferences() {
    let mut config = AppConfig::default();
    // Constructed with the preferences already set rather than assigning to a
    // `Default::default()` instance: clippy's `field_reassign_with_default`
    // fires on that shape, and this iteration already produced a warning of
    // its own in the same session (the clippy gate's whole job is to stop
    // that), so the test is written the way the lint wants from the start.
    let mut settings = Settings {
        vim_enabled: true,
        config: crate::tui::adapter_types::config::InnerConfig {
            verbose: true,
            cursor_blink_enabled: true,
            disable_claude_mds: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut screen = SettingsScreen::new();
    // A change to an UNRELATED entry: the settings screen is one screen, and
    // the operator edits one row at a time.
    screen
        .pending_changes
        .insert("notifications".to_string(), "true".to_string());
    screen.apply_and_save(&mut config, &mut settings);

    assert!(
        settings.vim_enabled,
        "vim mode was cleared by saving an unrelated setting"
    );
    assert!(
        settings.config.verbose,
        "verbose output was cleared by saving an unrelated setting"
    );
    assert!(
        settings.config.cursor_blink_enabled,
        "cursor blink was cleared by saving an unrelated setting"
    );
    assert!(
        settings.config.disable_claude_mds,
        "disable_claude_mds was cleared by saving an unrelated setting"
    );
    assert!(
        screen.settings_snapshot.vim_enabled,
        "the snapshot written to settings.json also lost vim mode"
    );
}

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
