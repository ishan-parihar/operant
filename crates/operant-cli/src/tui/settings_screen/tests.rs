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
/// FOUR of them (`context_window`, `output_style`, `compact_threshold`,
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

    // (panels plan S2) Every CONFIG row is handled by ONE generic arm that
    // writes any known key through `persist_config_key`, so the per-key grep
    // this test used to run is now the wrong shape: it would demand 385 match
    // arms the design deliberately does not have. What still must not happen is
    // a row the applier DROPS — so the generic arm is asserted to exist, and a
    // row whose key is not a known config key is the failure.
    assert!(
        applier.contains("persist_config_key"),
        "the generic config arm must be present: without it every registry row          renders, stores into pending_changes, and is silently discarded"
    );
    let mut unhandled = Vec::new();
    for entry in &entries {
        // A row that writes `settings.json` needs its own arm; a row that
        // writes `operant.toml` is carried by the generic arm, provided the key
        // is one the loader knows.
        if entry.store == crate::tui::settings_screen::SettingStore::Config {
            if !operant_core::config_registry::KNOWN_KEYS.contains(&entry.key) {
                unhandled.push(entry.key);
            }
            continue;
        }
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
    // (panels plan S2) The panel is a projection of the generated registry, so
    // the count is the registry's own size rather than a hand-kept number. The
    // floor catches a registry that stopped being read; the ceiling catches a
    // row list that grew a second copy of the table.
    // Only rows the generator could type reach the panel: an f64 ratio or a
    // HashMap has no single-line editor, so `Widget::None`/`Unsupported` are
    // omitted by design. The drawn count is therefore BELOW the live count, and
    // the gap is the honest-omission set.
    let live_rows = operant_core::config_registry::SETTING_ROWS
        .iter()
        .filter(|row| row.live)
        .count();
    let drawable = operant_core::config_registry::SETTING_ROWS
        .iter()
        .filter(|row| {
            row.live
                && !matches!(
                    row.widget,
                    operant_core::config_registry::Widget::None
                        | operant_core::config_registry::Widget::Unsupported
                )
        })
        .count();
    assert!(
        entries.len() >= drawable,
        "the panel drew {} rows but the registry offers {} editable ones",
        entries.len(),
        drawable
    );
    // The TUI's own `settings.json` rows sit on top, so a small excess is the
    // five declared here; a large one is a second table.
    assert!(
        entries.len() <= drawable + 10,
        "the panel added {} rows beyond the registry, which is a second table",
        entries.len() - drawable
    );
}

#[test]
fn search_filters_entries_correctly() {
    let screen = SettingsScreen::new();
    let all = all_entries(&screen);
    let filtered: Vec<_> = all
        .iter()
        .filter(|e| e.label.to_lowercase().contains("context window"))
        .collect();
    assert_eq!(
        filtered.len(),
        1,
        "the search must narrow to one row: {filtered:?}"
    );
    assert_eq!(filtered[0].label, "Context window");
}

#[test]
fn toggle_bool_entry_flips_value() {
    let mut screen = SettingsScreen::new();
    // (iter-1900) "notifications" was here until its dead row went; reduce
    // motion is live, so the row this exercises is a real one.
    screen.reduce_motion = false;
    screen.open(AppConfig::default().agent.context_window, &AppConfig::default());

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

// ---------------------------------------------------------------------------
// Panels plan S2: the panel is a projection of the generated registry. These
// generalize the hand-kept dead-key list below to the registry itself.
// ---------------------------------------------------------------------------

#[test]
fn no_row_the_config_calls_unread_reaches_the_panel() {
    // The phantom-surface guard at the panel edge. A row whose key the config
    // reports as accepted-but-unread is a control that does nothing, and the
    // registry marks it `live: false` for exactly this reason. The list below
    // is the hand-found version of this test; this one asks the registry, so a
    // key parked tomorrow is caught without anyone adding it here.
    let screen = with_sandbox_home(SettingsScreen::new);
    for entry in all_entries(&screen) {
        let Some(row) = operant_core::config_registry::SETTING_ROWS
            .iter()
            .find(|row| row.key == entry.key)
        else {
            continue; // a TUI-local `settings.json` row: no registry entry by design
        };
        assert!(
            row.live,
            "/settings offers `{}`, which the config reports as accepted but unread",
            entry.key
        );
    }
}

#[test]
fn every_config_row_is_writable_through_the_one_seam() {
    // A CONFIG row whose key the loader does not know is refused by the generic
    // arm, so the operator's edit renders, stores, and vanishes.
    let screen = with_sandbox_home(SettingsScreen::new);
    for entry in all_entries(&screen) {
        if entry.store != crate::tui::settings_screen::SettingStore::Config {
            continue;
        }
        assert!(
            operant_core::config_registry::KNOWN_KEYS.contains(&entry.key),
            "`{}` is drawn as a config row but is not a known config key",
            entry.key
        );
    }
}

#[test]
fn the_panel_shows_the_registry_rather_than_a_handful() {
    // The point of S2. A revert to a small literal list fails here.
    let screen = with_sandbox_home(SettingsScreen::new);
    let entries = all_entries(&screen);
    assert!(
        entries.len() > 100,
        "the panel drew only {} rows — the registry is not being read",
        entries.len()
    );
    let sections: std::collections::HashSet<&str> = entries
        .iter()
        .filter_map(|e| e.key.split('.').next())
        .collect();
    assert!(
        sections.len() >= 5,
        "the panel covers only {} config sections: {sections:?}",
        sections.len()
    );
}

#[test]
fn a_config_write_reaches_the_running_process_not_only_the_file() {
    // (panels plan S5, seam 1) The defect this guards: the surgical writer lands
    // on DISK, and the running process holds its own `AppConfig` behind one
    // `RwLock` with a single whole-struct writer. A disk write the process never
    // re-reads is a value the panel shows and the runtime ignores — the exact
    // "applied but not in effect" gap the two report states exist to prevent.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("operant.toml");
    std::fs::write(&path, "[agent]\nmodel = \"gpt-4\"\n").expect("seed the config");
    let loaded = operant_core::config::load_app_config(Some(path.as_path()))
        .expect("the seeded config loads");
    operant_core::config::install_runtime_config(loaded.config.clone());

    // The panel's own write path for a config key.
    operant_core::config::persist_config_key_at(&path, "agent.model", "claude-opus")
        .expect("the write lands");

    let after = operant_core::config::load_app_config(Some(path.as_path()))
        .expect("the file reloads");
    assert_eq!(
        after.config.agent.model, "claude-opus",
        "the file the panel wrote must carry the new value"
    );
    // And the live process must be re-installed from it, which is the half the
    // panel owes beyond the write. The reload path is the one `apply_and_save`
    // takes after a successful persist.
    operant_core::config::install_runtime_config(after.config.clone());
    assert_eq!(
        operant_core::config::runtime_config().agent.model,
        "claude-opus",
        "the running process must report the value the panel wrote"
    );
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
        "agent.context_window",
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
    screen.open(AppConfig::default().agent.context_window, &AppConfig::default());
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
/// `/settings` → "Context window" → type `abc` → Enter saved nothing and said
/// nothing: the config kept its old value, the displayed value kept the old
/// string, and the operator had no way to tell their edit had been dropped.
/// Same class as /rewind's silently discarded `[N]` (iter-1947).
#[test]
fn an_unreadable_number_says_so_instead_of_saving_silently() {
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());

    // A valid number still applies and clears the error. This test has no
    // config source, so the value is live-only; persistence is covered by the
    // explicit-path restart test below.
    let mut screen = SettingsScreen::new();
    screen
        .pending_changes
        .insert("agent.context_window".to_string(), "200000".to_string());
    screen.apply_and_save(&mut config, &mut settings);
    assert_eq!(config.agent.context_window, 200000);
    assert_eq!(screen.apply_error, None, "a valid value reports no error");

    // An algebraically impossible window is rejected before it can affect
    // the live agent or the operator file.
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());
    let mut screen = SettingsScreen::new();
    screen
        .pending_changes
        .insert("agent.context_window".to_string(), "4096".to_string());
    screen.apply_and_save(&mut config, &mut settings);
    assert_eq!(
        config.agent.context_window,
        AppConfig::default().agent.context_window,
        "an impossible window must not touch the live config"
    );
    let err = screen
        .apply_error
        .clone()
        .expect("the rejected window says why");
    assert!(err.contains("agent.context_window"), "{err}");

    // An invalid one is reported, and nothing is saved.
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());
    let mut screen = SettingsScreen::new();
    screen
        .pending_changes
        .insert("agent.context_window".to_string(), "abc".to_string());
    screen.apply_and_save(&mut config, &mut settings);
    assert_eq!(
        config.agent.context_window,
        AppConfig::default().agent.context_window,
        "an unreadable value must not touch the config"
    );
    let err = screen.apply_error.clone().expect("and must say why");
    assert!(
        err.contains("Context window") && err.contains("abc"),
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

    screen.open(AppConfig::default().agent.context_window, &AppConfig::default());
    screen.apply_error =
        Some("Not a number: Context window \"abc\" — kept the previous value".to_string());
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

/// (iter-1953) A settings row's description is a contract: it must name a
/// mechanism the TUI actually runs.
///
/// `/settings` offered "Auto-compact threshold — Context usage % at which to
/// trigger auto-compact (0-100)". Nothing read it. The TUI's own auto-compact
/// warnings (`check_token_warnings`, messaging.rs) fire at hardcoded 100, 95
/// and 80 percent, and no code path anywhere in `tui/` reads
/// `config.agent.context_compression_threshold`. The value is also not
/// persisted: `apply_and_save` writes it onto the in-memory `AppConfig` and then
/// saves `Settings` (settings.json), which has no such field.
///
/// So an operator could set the row to 50 and watch nothing change — the
/// classic phantom control the audit's rule removes rather than invents for.
///
/// The pin is the reverse of the row: it asserts the surface no longer offers
/// it, so re-adding the row without wiring a consumer fails here.
#[test]
fn the_settings_screen_offers_no_row_whose_value_changes_nothing() {
    let mut screen = SettingsScreen::new();
    screen.open(AppConfig::default().agent.context_window, &AppConfig::default());
    let rows = all_entries(&screen);
    assert!(
        !rows.iter().any(|r| r.key == "compact_threshold"),
        "the auto-compact threshold row is a phantom control: the TUI's warnings are \
         hardcoded at 100/95/80 and no path in tui/ reads the field"
    );

    // A second, property-shaped half is deliberately NOT added: the warnings this
    // row claimed to control are pinned by their own tests, and a negative
    // control that cannot fail is decoration. The assertion that carries the
    // contract is the row's absence, above.
}

/// (iter-1954) A settings row's edit must survive a restart.
///
/// `/settings`' "Context window" row writes `config.agent.context_window`, which
/// `check_token_warnings` reads live as the fallback window — so the row has a
/// working CONSUMER (round 69's class). Its SAVE path was open on both ends:
/// `apply_and_save` also copies the value into `InnerConfig.max_tokens` and
/// saves settings.json, but NOTHING reads `InnerConfig.max_tokens` back, and
/// `config.agent.context_window` is never written to operant.toml — the only
/// file the process actually loads. The repo already has the one surgical
/// writer this needs (`persist_config_key`, used for `agent.model`).
///
/// So an operator set Context window to 200000, saw the footer agree, restarted,
/// and silently got the shipped default back.
#[test]
fn a_settings_edit_survives_the_restart_it_promised() {
    with_sandbox_home(|| {
        let mut config = AppConfig::default();
        let mut settings = Settings::default();

        // A real config file, loaded the way the process loads one, so
        // persist_config_key has a target. The guard is the repo's own: two
        // tests loading temp configs in parallel race on the global path, and
        // the loser's write landed in a neighbour's file once already.
        let _cfg_guard = operant_core::config::config_path_test_guard();
        let sandbox = std::path::PathBuf::from(
            std::env::var("HERMES_HOME").expect("the sandbox sets HERMES_HOME"),
        );
        let toml_path = sandbox.join("operant.toml");
        std::fs::write(
            &toml_path,
            "[agent]\nmodel = \"gpt-4o\"\n\n[client]\nbase_url = \"http://localhost:9/v1\"\n",
        )
        .expect("write config");
        let loaded = operant_core::config::load_app_config(Some(&toml_path)).expect("load");
        operant_core::config::install_runtime_config(loaded.config.clone());
        config.config_path = Some(toml_path.clone());

        let mut screen = SettingsScreen::new();
        screen
            .pending_changes
            .insert("agent.context_window".to_string(), "200000".to_string());
        screen.apply_and_save(&mut config, &mut settings);

        // The live consumer sees it.
        assert_eq!(config.agent.context_window, 200000);

        // And the value the operator typed is in the FILE the process loads,
        // so a restart reads it back — not only in the in-memory config and
        // not only in settings.json, which nothing reads for this field.
        let persisted = std::fs::read_to_string(&toml_path).expect("read the config back");
        assert!(
            persisted.contains("context_window = 200000"),
            "Context window must persist into the loaded config: {persisted}"
        );

        // The ERROR half: a persist that cannot land is NAMED, not swallowed.
        // The live value still applies — withholding it would turn a persistence
        // gap into a setting that does nothing at all in any environment that
        // loaded no config file — but the operator is told the change will not
        // survive a restart, which is the part they cannot see.
        let mut screen = SettingsScreen::new();
        std::fs::remove_file(&toml_path).expect("remove the config so the write must fail");
        screen
            .pending_changes
            .insert("agent.context_window".to_string(), "300000".to_string());
        screen.apply_and_save(&mut config, &mut settings);
        let warn = screen
            .persist_warning
            .clone()
            .expect("a failed persist must say so, not be discarded");
        assert!(
            warn.contains("Context window")
                && warn.contains("300000")
                && warn.contains("not persisted"),
            "the warning names the row, the value, and that it will not persist: {warn:?}"
        );
        assert_eq!(
            config.agent.context_window, 300000,
            "the live value still applies; only its persistence failed"
        );
        assert!(
            screen.apply_error.is_none(),
            "a value that WAS applied must not also report as unread: {:?}",
            screen.apply_error
        );
    });
}

// ---------------------------------------------------------------------------
// The bar-style row (plan 2026-11-15 U1)
// ---------------------------------------------------------------------------

/// Installs a separator style for one test, and serializes against every other
/// test that touches the same process-global.
///
/// Restoring on drop is not enough on its own: two of these tests running
/// concurrently each restore whatever they captured, so the second drop
/// clobbers the first test's install and the row reads the wrong style. The
/// crate-wide `test_env_lock` is the same serialisation `with_sandbox_home`
/// uses for the identical reason, so the guard holds it for the whole test.
struct BarStyleGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<crate::tui::status_line::style::SeparatorStyle>,
}

impl Drop for BarStyleGuard {
    fn drop(&mut self) {
        crate::tui::status_line::style::set_style(self.previous);
    }
}

fn with_bar_style(style: crate::tui::status_line::style::SeparatorStyle) -> BarStyleGuard {
    let lock = crate::config::test_env_lock();
    let previous = crate::tui::status_line::style::style();
    crate::tui::status_line::style::set_style(Some(style));
    BarStyleGuard {
        _lock: lock,
        previous: Some(previous),
    }
}

/// The row must offer exactly the style vocabulary, in its order.
///
/// `SeparatorStyle::ALL` is the one list `key()` enumerates; a second literal
/// list here is how `theme_screen.rs:85-95` produced a picker row that could
/// only ever render the fallback palette. The row's value must be the style the
/// status line is actually drawing, so the operator cannot be shown one thing
/// and given another.
#[test]
fn the_bar_style_row_offers_every_style_and_shows_the_live_one() {
    use crate::tui::status_line::style::SeparatorStyle;

    let _guard = with_bar_style(SeparatorStyle::Pipe);
    let screen = SettingsScreen::new();
    let entry = all_entries(&screen)
        .into_iter()
        .find(|entry| entry.key == "tui.bar_style")
        .expect("the settings screen offers a bar-style row");
    let SettingKind::Enum { options } = entry.kind else {
        panic!("the bar-style row must be an enum, not a free-text field");
    };
    let expected: Vec<&str> = SeparatorStyle::ALL.iter().map(|s| s.key()).collect();
    assert_eq!(
        options, expected,
        "the row offers exactly the vocabulary, in ALL's order"
    );
    assert_eq!(
        entry.value,
        SeparatorStyle::Pipe.key(),
        "the row shows the style in force, not a hardcoded default"
    );
}

/// Choosing a style must change the bar WITHOUT a restart.
///
/// This is the whole point of U1: the key existed and was installed once at
/// startup, so the operator could set it only by editing TOML and relaunching.
/// The assertion is on the live cell the paint site reads, not on a config
/// field — a change that writes the config but never reaches the bar is the
/// defect this row exists to close.
#[test]
fn choosing_a_bar_style_changes_the_bar_without_a_restart() {
    use crate::tui::status_line::style::SeparatorStyle;

    let _guard = with_bar_style(SeparatorStyle::Dot);
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());
    let mut screen = SettingsScreen::new();
    screen.open(config.agent.context_window, &config);
    let index = all_entries(&screen)
        .iter()
        .position(|entry| entry.key == "tui.bar_style")
        .expect("the bar-style row is offered");
    screen.selected_idx = index;

    let before = crate::tui::status_line::style::style();
    handle_settings_key(
        &mut screen,
        &mut config,
        &mut settings,
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ),
    );

    let after = crate::tui::status_line::style::style();
    assert_ne!(before, after, "Enter on the row must advance the style");
    assert_eq!(
        after.key(),
        config.tui.bar_style,
        "the live cell and the config key agree"
    );
    assert_eq!(
        screen.bar_style,
        after.key(),
        "the row shows what the bar is drawing"
    );
    assert!(
        screen.apply_error.is_none(),
        "a known style reports no error: {:?}",
        screen.apply_error
    );
}

/// An unknown style is reported, and changes nothing.
///
/// `SeparatorStyle::parse` returns the fallback with a `known` flag precisely
/// so a call site cannot swallow a typo: a row that silently drew `dot` would
/// leave the operator believing they had chosen a style.
#[test]
fn an_unknown_bar_style_is_reported_and_changes_nothing() {
    use crate::tui::status_line::style::SeparatorStyle;

    let _guard = with_bar_style(SeparatorStyle::Dot);
    let (mut config, mut settings) = (AppConfig::default(), Settings::default());
    let before = config.tui.bar_style.clone();

    let mut screen = SettingsScreen::new();
    screen
        .pending_changes
        .insert("tui.bar_style".to_string(), "powerline-ish".to_string());
    screen.apply_and_save(&mut config, &mut settings);

    let error = screen
        .apply_error
        .clone()
        .expect("a rejected style says why");
    assert!(
        error.contains("powerline-ish") && error.contains("dot"),
        "the error names the value and the expected vocabulary: {error:?}"
    );
    assert_eq!(
        config.tui.bar_style, before,
        "a rejected style must not touch the config"
    );
    assert_eq!(
        crate::tui::status_line::style::style(),
        SeparatorStyle::Dot,
        "and must not touch the live bar"
    );
    assert!(
        screen.pending_changes.is_empty(),
        "a failed apply is consumed, so it cannot be retried forever"
    );
}
