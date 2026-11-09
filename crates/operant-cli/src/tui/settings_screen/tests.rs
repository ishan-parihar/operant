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
/// `file_injection_enabled: false` and
/// `OutputFormat::default()` instead of carrying the persisted values
/// forward. The save appeared to succeed, so nothing surfaced it until
/// the two sets were compared side by side.
///
/// Deriving the key list from `all_entries` (rather than hand-writing it)
/// means the test keeps pace with the UI: a newly added entry that no
/// applier handles fails here, which is exactly the drift it guards.
#[test]
fn every_settings_entry_has_an_applier() {
    // (iter-1895) ONE screen is checked: there are no conditional rows any
    // more. The two file-injection rows the second screen used to offer were
    // dead — nothing ever read their values — and the @-autocomplete's own
    // two rows, which DO do something, were only offered when that dead
    // toggle was on. With no conditional branch left, the single screen
    // covers every row, and the "the second screen must offer MORE rows"
    // assertion (which only made sense while a conditional existed) goes
    // with it.
    //
    // The shape it replaced is worth keeping in mind: a guard that derives
    // its list from the UI is only as complete as the UI STATE it derives it
    // from, and the two-screen form was the fix for exactly that.
    let screen = with_sandbox_home(SettingsScreen::new);
    let entries = all_entries(&screen);
    let entries: Vec<&SettingsEntry> = entries.iter().collect();
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
/// value forward or overwrite it with a constant. (iter-1176 found five such
/// constants; iter-1900 removed the fields that held two of them along with
/// their dead rows, so the preferences this pins are the two that survive:
/// `vim_enabled` and the `show_cwd` tri-state.)
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
            // (iter-1898) `show_cwd` is a top-level Settings field, not an
            // InnerConfig one, and it is a tri-state.
            show_cwd: Some(true),
            ..Default::default()
        };

        let mut screen = SettingsScreen::new();
        // A change to an UNRELATED entry: the settings screen is one screen, and
        // the operator edits one row at a time.
        // (iter-1900) "notifications" was here until its dead row went;
        // reduce_motion is live (it drives the render loop's animations), so
        // the unrelated edit is still a real one.
        screen
            .pending_changes
            .insert("reduce_motion".to_string(), "true".to_string());
        screen.apply_and_save(&mut config, &mut settings);

        assert!(
            settings.vim_enabled,
            "vim mode was cleared by saving an unrelated setting"
        );
        assert_eq!(
            settings.show_cwd,
            Some(true),
            "show_cwd was cleared by saving an unrelated setting"
        );
        assert!(
            screen.settings_snapshot.vim_enabled,
            "the snapshot written to settings.json also lost vim mode"
        );
    })
}
/// The @-autocomplete's own settings are what the operator actually needs,
/// and they must apply.
///
/// (iter-1895) They used to be offered ONLY when the dead "File injection (@)"
/// toggle was on, so turning that toggle off hid them while the feature they
/// configure kept working. This walks the operator's real path: edit the
/// limit, save, and the value must land in the settings the app reads.
#[test]
fn the_file_autocomplete_rows_apply() {
    with_sandbox_home(|| {
        let mut screen = SettingsScreen::new();
        let mut config = AppConfig::default();
        let mut settings = Settings::default();

        assert!(
            all_entries(&screen)
                .iter()
                .any(|entry| entry.key == "fileAutocompleteLimit"),
            "the @-autocomplete limit row must always be offered — it is what \
             the operator configures the feature they actually use"
        );

        screen
            .pending_changes
            .insert("fileAutocompleteLimit".to_string(), "7".to_string());
        screen.apply_and_save(&mut config, &mut settings);
        assert_eq!(
            settings.config.file_autocomplete_limit, 7,
            "the edited autocomplete limit was dropped on save — the screen \
             showed the new value and the settings file kept the old one"
        );
    })
}

/// The settings screen must not offer a row whose value nothing reads.
///
/// (iter-1895) "File injection (@)" — "Auto-inject @file references into
/// message context" — and "File injection max size" — "Max file size to
/// auto-inject" — were ported in with the claurst settings surface, but
/// NOTHING ever read `file_injection_enabled` or `file_injection_max_size`:
/// the @-file suggestions insert a PATH (the agent reads the file with its
/// own tools), and no code path injects a file's CONTENT. The operator could
/// toggle the switch, edit the number, watch both persist into
/// settings.json, and change nothing at all.
///
/// The pin is the screen's own row list: the two dead keys must not be
/// offered, and the two @-autocomplete rows that DO do something must be.
#[test]
fn the_settings_screen_offers_no_row_nothing_reads() {
    let screen = SettingsScreen::new();
    let entries = all_entries(&screen);
    for dead in ["fileInjectionEnabled", "fileInjectionMaxSize"] {
        assert!(
            !entries.iter().any(|entry| entry.key == dead),
            "/settings offers {dead} again — nothing reads its value, so the \
             operator's edit is a promise nothing keeps"
        );
    }
    for live in ["fileAutocompleteLimit", "fileAutocompleteShowHiddenFiles"] {
        assert!(
            entries.iter().any(|entry| entry.key == live),
            "/settings must offer {live}: the @-autocomplete reads it"
        );
    }
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
    // (iter-1900) Eight rows survive three sweeps that REMOVED rows rather
    // than adding them: the two file-injection rows, the display flags' dead
    // siblings, the four rows nothing read at all and the five more the last
    // sweep found. The floor still catches an empty list; the ceiling catches
    // a runaway re-port of dead rows.
    assert!(
        entries.len() >= 6,
        "Should have at least 12 editable settings, got {}",
        entries.len()
    );
    assert!(
        entries.len() <= 10,
        "Should have at most 16 editable settings, got {}",
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
    // (iter-1900) "notifications" was here until its dead row went; reduce
    // motion is live, so the row this exercises is a real one.
    screen.reduce_motion = false;
    screen.open();

    let initial = screen.reduce_motion;
    let all = all_entries(&screen);
    // (iter-1899) The row's position moves whenever a row is deleted: find it
    // by its key instead of a hardcoded index, so deleting a row cannot
    // silently re-point this test at a different setting.
    let entry = all
        .iter()
        .find(|e| e.key == "reduce_motion")
        .expect("the screen offers the Reduce motion row");
    assert_eq!(entry.label, "Reduce motion");

    // Simulate toggle (manually, since toggle_or_cycle_current modifies internal state)
    screen.reduce_motion = !screen.reduce_motion;
    assert_ne!(screen.reduce_motion, initial);
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

/// (iter-1899) The screen must not offer a row whose value nothing reads.
///
/// This is the family iter-1896 opened (`fileInjectionEnabled` /
/// `fileInjectionMaxSize`) and iter-1899 finishes. Each key below was
/// offered by `all_entries`, persisted by the screen and displayed with a
/// value, and NO code outside the settings screen read it — so the operator
/// could change it, watch it save, and change nothing at all:
///
///   auto_compact          "Automatically compact turns at threshold."
///   show_turn_duration    "Display elapsed time per turn in status bar."
///   terminal_progress_bar "Show progress during tool use."
///   disable_claude_mds    "Ignore CLAUDE.md files in projects."
///
/// The four lived in the ported claurst settings surface and were never
/// wired. `terminal_progress_bar` was the only one with a command (/statusbar,
/// which printed "Status bar shown." while nothing changed) — that command is
/// retired with the setting.
///
/// The pin is the screen's own row list, and the doc comment's claim is
/// mechanical: a grep for each key outside the settings screen module finds
/// nothing, which the ledger row cites with the command that produced it.
#[test]
fn the_settings_screen_offers_no_unread_row() {
    const DEAD: &[&str] = &[
        "auto_compact",
        "show_turn_duration",
        "terminal_progress_bar",
        "disable_claude_mds",
        "notifications",
        "verbose",
        "cursor_blink_enabled",
        "auto_commits",
        "output_format",
    ];
    let entries = all_entries(&SettingsScreen::new());
    for key in DEAD {
        assert!(
            !entries.iter().any(|entry| entry.key == *key),
            "/settings offers {key} again — nothing reads its value, so the \
             operator's edit is a promise nothing keeps"
        );
    }
    // And the rows that DO something are still there.
    for live in [
        "max_tokens",
        "output_style",
        "reduce_motion",
        "show_cwd",
        "show_git_branch",
        "fileAutocompleteLimit",
        "fileAutocompleteShowHiddenFiles",
    ] {
        assert!(
            entries.iter().any(|entry| entry.key == live),
            "/settings must still offer {live}"
        );
    }
}

/// (iter-1899) The dead fields must be GONE from the persisted settings, not
/// merely hidden — a settings.json still carrying `auto_compact` (and every
/// old file does) must load and round-trip without it, thanks to the struct's
/// `#[serde(default)]`.
#[test]
fn the_dead_settings_fields_are_gone_from_the_persisted_shape() {
    // The exact shape an old settings.json carries.
    let legacy = r#"{"auto_compact":true,"show_turn_duration":true,
        "terminal_progress_bar":false,"disable_claude_mds":true,
        "theme":"Default","permission_mode":"Default"}"#;
    let settings: Settings = serde_json::from_str(legacy).expect("an old settings.json must load");
    // The JSON on disk keeps the stale keys (serde ignores unknown fields —
    // `Settings` does not deny them), and the loaded value must not resurrect
    // a field the Rust type no longer has.
    let reloaded = serde_json::to_string(&settings).expect("the loaded settings re-serialise");
    for key in [
        "auto_compact",
        "show_turn_duration",
        "terminal_progress_bar",
        "disable_claude_mds",
    ] {
        assert!(
            !reloaded.contains(key),
            "{key} survived the round-trip — the field is still in the struct"
        );
    }
}

/// (iter-1917) The settings list paints the SELECTED row on a short terminal.
///
/// The keyboard tracked the window with a hardcoded estimate of ten rows
/// ("Rough estimate, will be actual in real usage") while the render sliced
/// with the real height — two windows, two numbers. At 80x24 the list paints
/// five rows, so the estimate never advanced: the operator walked ↓ past the
/// fifth row, the ► marker vanished, and the row could not be seen, read or
/// toggled. Three of the eight rows were unreachable that way.
///
/// This renders the REAL surface at the REAL narrow size and drives the
/// operator's real key, so a second window cannot reappear without failing
/// here: whatever the selected row is, it must be on screen.
#[test]
fn the_settings_list_paints_the_selected_row_on_a_short_terminal() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // 80x24 is the corpus's narrow variant — the size where the list paints
    // five rows and the old estimate (ten) disagreed with the real window.
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut screen = SettingsScreen::new();
    screen.open();
    let total = all_entries(&screen).len();

    for step in 0..total {
        screen.select_next(total);
        terminal
            .draw(|frame| {
                render_settings_screen(frame, &screen, frame.area());
            })
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect::<String>();
        let selected = &all_entries(&screen)[screen.selected_idx];
        assert!(
            rendered.contains(selected.label),
            "step {step}: the selected row {:?} must be painted at 80x24 — the \
             window let the selection run off the end",
            selected.label
        );
        assert!(
            rendered.contains('►'),
            "step {step}: the ► marker must be painted — the selection left \
             the visible window"
        );
    }
}

/// (iter-1948) A settings screen that accepts an edit must say when it could
/// not take effect.
///
/// The edit buffer accepts ANY character (`KeyCode::Char(c) => push(c)`), and
/// the three numeric rows resolved with `if let Ok(n) = value.parse()`. So
/// `/settings` → "Max Tokens" → type `abc` → Enter saved nothing and said
/// nothing: the config kept its old value, the displayed value kept the old
/// string, and the operator had no way to tell their edit had been dropped.
/// Same class as /rewind's silently discarded `[N]` (iter-1947).
#[test]
fn an_unreadable_number_says_so_instead_of_saving_silently() {
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());

    // A valid number still applies and clears the error.
    let mut screen = SettingsScreen::new();
    screen
        .pending_changes
        .insert("max_tokens".to_string(), "4242".to_string());
    screen.apply_and_save(&mut config, &mut settings);
    assert_eq!(config.agent.context_window, 4242);
    assert_eq!(screen.apply_error, None, "a valid value reports no error");

    // An invalid one is reported, and nothing is saved.
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());
    let mut screen = SettingsScreen::new();
    screen
        .pending_changes
        .insert("max_tokens".to_string(), "abc".to_string());
    screen.apply_and_save(&mut config, &mut settings);
    assert_eq!(
        config.agent.context_window,
        AppConfig::default().agent.context_window,
        "an unreadable value must not touch the config"
    );
    let err = screen.apply_error.clone().expect("and must say why");
    assert!(
        err.contains("Max Tokens") && err.contains("abc"),
        "the error names the row and the value the operator typed: {err:?}"
    );
    assert!(
        screen.pending_changes.is_empty(),
        "a failed apply is still consumed, so it cannot be retried forever"
    );

    // The PAINTED half: the operator receives the footer, so the footer IS
    // the error while one stands, and the key hints return once it clears.
    let mut screen = SettingsScreen::new();
    fn painted(screen: &SettingsScreen) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                render_settings_screen(frame, screen, frame.area());
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    screen.open();
    screen.apply_error =
        Some("Not a number: Max Tokens \"abc\" — kept the previous value".to_string());
    let errored = painted(&screen);
    assert!(
        errored.contains("abc"),
        "the painted footer carries the unread value: {errored:?}"
    );
    assert!(
        !errored.contains("navigate"),
        "and the key hints yield to it while it stands: {errored:?}"
    );
    screen.apply_error = None;
    let cleared = painted(&screen);
    assert!(
        cleared.contains("navigate"),
        "the key hints return once no error stands: {cleared:?}"
    );
}
