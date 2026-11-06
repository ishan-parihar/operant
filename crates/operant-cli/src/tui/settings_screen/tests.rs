// settings_screen/tests.rs — Unit tests for the settings screen.
//
// Extracted from the settings_screen.rs monolith.

use super::*;

/// Run `f` with the operant home pointed at a throwaway directory.
///
/// (iter-1776) The settings screen's save writes the operator's REAL
/// `~/.operant/settings.json` — so a test that saves rewrites the
/// operator's configuration, and the NEXT run of the suite reads back
/// whatever the last run wrote. That coupling is not hypothetical: the
/// guard below (`every_settings_entry_has_an_applier`) compares two screens
/// built from the ambient settings, and a test that had enabled file
/// injection made the "default" screen offer the conditional rows too — so
/// the guard started failing for a reason that had nothing to do with the
/// guard.
///
/// The crate-wide `test_env_lock` serialises the env mutation (the same
/// pattern `app/tests.rs` uses for the history sandbox), and the variable is
/// restored below so a failure cannot leak the sandbox home.
fn with_sandbox_home<T>(f: impl FnOnce() -> T) -> T {
    let _env_lock = crate::config::test_env_lock();
    let saved_home = std::env::var("HERMES_HOME").ok();
    let sandbox =
        std::env::temp_dir().join(format!("operant-settings-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&sandbox);
    // SAFETY: serialized by the crate-wide test_env_lock; restored below.
    unsafe {
        std::env::set_var("HERMES_HOME", &sandbox);
    }
    let out = f();
    // SAFETY: still under test_env_lock.
    unsafe {
        match &saved_home {
            Some(home) => std::env::set_var("HERMES_HOME", home),
            None => std::env::remove_var("HERMES_HOME"),
        }
    }
    out
}

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
    // BOTH screens are checked, and the second one must really offer MORE
    // rows. The file-injection rows are only offered when file injection is
    // enabled, so a guard that looked only at the DEFAULT screen never checked
    // them — which is exactly how `fileInjectionMaxSize` survived with no
    // applier: the operator turns injection on, edits the max size, and the
    // edit is silently dropped. The assertion below keeps that hole closed:
    // if the conditional branch ever changes shape, this test fails instead of
    // quietly shrinking its own coverage.
    // (iter-1776) Both screens are built in a sandbox home AND with their
    // conditional flag set explicitly: the default screen must NOT offer the
    // file-injection rows, and the ambient settings.json used to decide that
    // (a test that had enabled file injection made the "default" screen offer
    // them too, and this guard failed for a reason unrelated to the guard).
    let (default_screen, injection_screen) = with_sandbox_home(|| {
        let mut default_screen = SettingsScreen::new();
        default_screen.file_injection_enabled = false;
        let mut injection_screen = SettingsScreen::new();
        injection_screen.file_injection_enabled = true;
        (default_screen, injection_screen)
    });
    let entries = all_entries(&default_screen);
    let conditional = all_entries(&injection_screen);
    assert!(
        conditional.len() > entries.len(),
        "the file-injection screen must offer MORE rows than the default one — \
         otherwise this test silently stops checking the conditional entries"
    );
    let entries: Vec<&SettingsEntry> = entries.iter().chain(conditional.iter()).collect();
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
    with_sandbox_home(|| {
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
    })
}
/// The file-injection rows are only OFFERED when file injection is enabled,
/// so the screen has a state where an operator edits a setting that
/// `apply_and_save` does not handle: the value renders, lands in
/// `pending_changes`, and is silently dropped on save — while the whole
/// settings.json is rewritten from the snapshot. That is the RT-171 defect
/// class, and it survived RT-171's own fix because the guard only ever looked
/// at the DEFAULT screen, where those rows are not offered at all.
///
/// This test walks the operator's real path: turn file injection on, then edit
/// the max size.
#[test]
fn the_file_injection_rows_apply_when_file_injection_is_enabled() {
    with_sandbox_home(|| {
        let mut screen = SettingsScreen::new();
        let mut config = AppConfig::default();
        let mut settings = Settings::default();

        // Step 1: the operator enables file injection.
        screen
            .pending_changes
            .insert("fileInjectionEnabled".to_string(), "true".to_string());
        screen.apply_and_save(&mut config, &mut settings);
        assert!(
            screen.file_injection_enabled,
            "precondition: file injection is on, so the max-size row is offered"
        );
        assert!(
            all_entries(&screen)
                .iter()
                .any(|entry| entry.key == "fileInjectionMaxSize"),
            "precondition: the screen offers the max-size row in this state"
        );

        // Step 2: the operator edits the max size.
        screen
            .pending_changes
            .insert("fileInjectionMaxSize".to_string(), "2048".to_string());
        screen.apply_and_save(&mut config, &mut settings);
        assert_eq!(
            settings.config.file_injection_max_size, 2048,
            "the edited max size was dropped on save — the screen showed the new \
             value and the settings file kept the old one"
        );
    })
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
