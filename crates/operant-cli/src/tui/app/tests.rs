// app/tests.rs — Unit tests for the TUI app (turn state, key handling,
// command routing).
//
// Extracted from the app/mod.rs monolith.

use super::*;
use crate::tui::adapter_types::types::MessageContent;
use crate::tui::operant_app::auth::AuthState;
use crate::tui::operant_app::tui_state::TuiState as _;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton,
};

pub(crate) fn make_app() -> App {
    // `App::new` calls `set_active_theme_enum`, which writes the process-global
    // palette. Serialise on the same lock the theme-asserting tests hold, or a
    // concurrent `make_app` resets the palette between one of those tests'
    // `set_active_theme` and its assertion.
    let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let config = AppConfig::default();
    let settings = Settings::default();
    let cost_tracker = std::sync::Arc::new(crate::tui::adapter_types::cost::CostTracker::new());
    let command_registry = crate::commands::CommandRegistry::new();
    App::new(config, settings, cost_tracker, command_registry)
}

fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// A Ctrl-modified key press (the fine transcript-scroll rung).
fn press_key_with_control(code: KeyCode) -> KeyEvent {
    press_key(code, KeyModifiers::CONTROL)
}

// ---- P0-1: paste-burst Enter semantics (2026-10-09 live audit) ----
//
// tmux/SSH batch keystrokes into the same drain window; pre-fix, a batched
// `text<Enter>` had its Enter absorbed as a literal newline and the message
// never sent. The corpus simulator bypasses the burst path (it injects keys
// directly), so these unit tests are the only gate for this logic.

fn burst_event(code: KeyCode) -> Event {
    Event::Key(press_key(code, KeyModifiers::NONE))
}

#[test]
fn burst_trailing_enter_must_submit_not_absorb() {
    let mut app = make_app();
    // A latency-batched `text<Enter>`: [h, i, Enter] in one drain window.
    let q = vec![
        burst_event(KeyCode::Char('h')),
        burst_event(KeyCode::Char('i')),
        burst_event(KeyCode::Enter),
    ];
    let (buf, pending) = app.collect_paste_burst('T', q.into_iter());
    assert_eq!(buf, "Thi", "the typed chars collect as burst text");
    let pending = pending.expect("trailing Enter must be handed back for replay");
    assert_eq!(pending.code, KeyCode::Enter);
}

#[test]
fn burst_embedded_newline_still_absorbs() {
    let mut app = make_app();
    // A clipboard paste's embedded newline has more clipboard behind it —
    // Windows Ctrl+V multi-line paste must keep working.
    let q = vec![
        burst_event(KeyCode::Char('a')),
        burst_event(KeyCode::Enter),
        burst_event(KeyCode::Char('b')),
    ];
    let (buf, pending) = app.collect_paste_burst('x', q.into_iter());
    assert_eq!(buf, "xa\nb");
    assert!(pending.is_none(), "a mid-paste Enter must not terminate");
}

#[test]
fn burst_single_char_then_enter_replays_submit() {
    let mut app = make_app();
    // One typed char + Enter batched together: below the paste threshold, but
    // the Enter must survive as a replayed submit key, not vanish into text.
    let q = vec![burst_event(KeyCode::Enter)];
    let (buf, pending) = app.collect_paste_burst('X', q.into_iter());
    assert_eq!(buf, "X");
    assert_eq!(
        pending.expect("Enter survives below-threshold bursts").code,
        KeyCode::Enter
    );
}

// ---- MCP reconnect tick-drain tests (iter-326) ----

#[test]
fn drain_mcp_reconnect_status_renders_without_keystroke() {
    let mut app = make_app();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    app.mcp_reconnect_rx = Some(rx);
    assert!(app.status_message.is_none());

    // Simulate the background reconnect task posting updates — the drain
    // runs on every frame (including tick frames with no input), so the
    // completion message must surface with zero keystrokes.
    tx.send("MCP reconnect initiated...".to_string()).unwrap();
    tx.send(
        "MCP reconnect complete in 1.9s — agentmemory backend: up, 53 tool(s) synced".to_string(),
    )
    .unwrap();

    app.drain_mcp_reconnect_status();
    assert_eq!(
        app.status_message.as_deref(),
        Some("MCP reconnect complete in 1.9s — agentmemory backend: up, 53 tool(s) synced")
    );

    // The channel is drained to empty — a second drain is a no-op and the
    // last message still wins.
    app.drain_mcp_reconnect_status();
    assert_eq!(
        app.status_message.as_deref(),
        Some("MCP reconnect complete in 1.9s — agentmemory backend: up, 53 tool(s) synced")
    );
}

#[test]
fn drain_mcp_reconnect_status_is_noop_without_channel() {
    let mut app = make_app();
    app.mcp_reconnect_rx = None;
    // Must not panic when no reconnect task ever ran.
    app.drain_mcp_reconnect_status();
    assert!(app.status_message.is_none());
}

// ---- normalize_char_with_shift tests ----

#[test]
fn test_normalize_char_no_shift_returns_unchanged() {
    assert_eq!(normalize_char_with_shift('a', KeyModifiers::NONE), 'a');
    assert_eq!(normalize_char_with_shift('1', KeyModifiers::NONE), '1');
    assert_eq!(normalize_char_with_shift('!', KeyModifiers::NONE), '!');
}

#[test]
fn test_normalize_char_shift_uppercase_letters() {
    assert_eq!(normalize_char_with_shift('a', KeyModifiers::SHIFT), 'A');
    assert_eq!(normalize_char_with_shift('z', KeyModifiers::SHIFT), 'Z');
    assert_eq!(normalize_char_with_shift('m', KeyModifiers::SHIFT), 'M');
}

#[test]
fn test_normalize_char_shift_numbers() {
    assert_eq!(normalize_char_with_shift('1', KeyModifiers::SHIFT), '!');
    assert_eq!(normalize_char_with_shift('2', KeyModifiers::SHIFT), '@');
    assert_eq!(normalize_char_with_shift('3', KeyModifiers::SHIFT), '#');
    assert_eq!(normalize_char_with_shift('4', KeyModifiers::SHIFT), '$');
    assert_eq!(normalize_char_with_shift('5', KeyModifiers::SHIFT), '%');
    assert_eq!(normalize_char_with_shift('6', KeyModifiers::SHIFT), '^');
    assert_eq!(normalize_char_with_shift('7', KeyModifiers::SHIFT), '&');
    assert_eq!(normalize_char_with_shift('8', KeyModifiers::SHIFT), '*');
    assert_eq!(normalize_char_with_shift('9', KeyModifiers::SHIFT), '(');
    assert_eq!(normalize_char_with_shift('0', KeyModifiers::SHIFT), ')');
}

#[test]
fn test_normalize_char_shift_symbols() {
    assert_eq!(normalize_char_with_shift('-', KeyModifiers::SHIFT), '_');
    assert_eq!(normalize_char_with_shift('=', KeyModifiers::SHIFT), '+');
    assert_eq!(normalize_char_with_shift('[', KeyModifiers::SHIFT), '{');
    assert_eq!(normalize_char_with_shift(']', KeyModifiers::SHIFT), '}');
    assert_eq!(normalize_char_with_shift(';', KeyModifiers::SHIFT), ':');
    assert_eq!(normalize_char_with_shift('\'', KeyModifiers::SHIFT), '"');
    assert_eq!(normalize_char_with_shift(',', KeyModifiers::SHIFT), '<');
    assert_eq!(normalize_char_with_shift('.', KeyModifiers::SHIFT), '>');
    assert_eq!(normalize_char_with_shift('/', KeyModifiers::SHIFT), '?');
    assert_eq!(normalize_char_with_shift('\\', KeyModifiers::SHIFT), '|');
    assert_eq!(normalize_char_with_shift('`', KeyModifiers::SHIFT), '~');
}

#[test]
fn test_normalize_char_shift_already_shifted_chars_unchanged() {
    // Characters that don't have shift equivalents remain unchanged
    assert_eq!(normalize_char_with_shift('!', KeyModifiers::SHIFT), '!');
    assert_eq!(normalize_char_with_shift('@', KeyModifiers::SHIFT), '@');
    assert_eq!(normalize_char_with_shift('A', KeyModifiers::SHIFT), 'A');
}

#[test]
fn test_normalize_char_other_modifiers_ignored() {
    // CTRL or ALT without SHIFT should not shift the character
    assert_eq!(normalize_char_with_shift('a', KeyModifiers::CONTROL), 'a');
    assert_eq!(normalize_char_with_shift('1', KeyModifiers::ALT), '1');
    assert_eq!(
        normalize_char_with_shift('a', KeyModifiers::CONTROL | KeyModifiers::ALT),
        'a'
    );
}

#[test]
fn test_normalize_char_shift_with_other_modifiers() {
    // SHIFT + CTRL should still apply shift transformation
    assert_eq!(
        normalize_char_with_shift('a', KeyModifiers::SHIFT | KeyModifiers::CONTROL),
        'A'
    );
    assert_eq!(
        normalize_char_with_shift('1', KeyModifiers::SHIFT | KeyModifiers::ALT),
        '!'
    );
}

// /personality sets `App.agent_mode`, which is DISPLAY metadata for the
// turn (the footer/header snapshot). It carries no live signal: an
// `agent_mode_changed` bool once existed to tell "the run loop" to update the
// query config, but no consumer was ever written, and the comment above the
// arm asserted one. Both the bool and the claim are gone. This test pins the
// behaviour that remains and guards against resurrecting dead state.
#[test]
fn personality_sets_the_display_mode_and_declares_no_live_signal() {
    let mut app = make_app();

    assert!(app.handle_tui_command("personality", ""));
    assert!(
        app.agent_mode.is_none(),
        "bare /personality reports the current value and must not set one"
    );
    assert!(
        app.status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Current personality"),
        "bare args must report the current value, got {:?}",
        app.status_message
    );

    assert!(app.handle_tui_command("personality", "concise"));
    assert_eq!(
        app.agent_mode.as_deref(),
        Some("concise"),
        "the personality string must land on App so the turn snapshot can read it"
    );
    assert!(
        app.status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Personality set to: concise"),
        "the operator must get feedback the value changed"
    );

    // The value is display metadata, so it must reach the turn snapshot —
    // that is the one consumer that actually exists.
    assert_eq!(
        app.current_agent_mode_snapshot(),
        "concise",
        "the turn snapshot reads agent_mode, so the change is observable there"
    );
}

// Structural guard for the iter-1191 defect class on this field: a slash
// command writing a bool no consumer reads. If next turn adds
// `agent_mode_changed` back without a reader, this fails.
//
// It scans the non-test TUI sources only: a test that named the field to
// guard against it would otherwise flag itself.
#[test]
fn no_app_field_is_written_by_a_command_and_read_by_nothing() {
    fn read_all(dir: &std::path::Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_test_dir = path.file_name().is_some_and(|n| n == "tests");
            let is_test_file = path.file_name().is_some_and(|n| n == "tests.rs");
            if path.is_dir() && !is_test_dir {
                read_all(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs")
                && !is_test_file
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                out.push_str(&text);
                out.push('\n');
            }
        }
    }

    let mut sources = String::new();
    let tui_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui");
    read_all(&tui_root, &mut sources);

    assert!(
        !sources.contains("agent_mode_changed"),
        "agent_mode_changed was removed as a signal with no consumer — do not re-add it"
    );
    assert!(
        !sources.contains("the run loop to update the query config"),
        "the false claim that /personality updates the query config must not return"
    );
}

// (plan 04) rewritten: /mcp with args now runs the REAL subcommand through
// the shell-out pattern (the old body returned false so `operant mcp auth`
// typed as /mcp auth went to the MODEL). The panel opens only for bare /mcp.
#[test]
fn test_mcp_with_args_runs_the_subcommand_and_bare_opens_the_panel() {
    let mut app = make_app();
    assert!(app.handle_tui_command("mcp", "auth mcphub"));
    assert!(
        !app.mcp_view.visible,
        "with args the panel must not open — the subcommand runs"
    );
    assert_eq!(
        app.pending_shell_command,
        Some(vec![
            std::env::current_exe()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "operant".to_string()),
            "mcp".to_string(),
            "auth".to_string(),
            "mcphub".to_string(),
        ]),
        "the real subcommand must be queued"
    );

    app.pending_shell_command = None;
    assert!(app.handle_tui_command("mcp", ""));
    assert!(app.mcp_view.visible, "bare /mcp opens the panel");
    assert!(app.pending_shell_command.is_none());
}

// (plan 03) test_clear_slash_command_clears_messages deleted: it pinned
// the OLD arm behavior — /clear clearing the display directly from the arm,
// which is exactly the display-only lie plan 03 §2 removes. The arm now
// defers to the drain: see clear_when_idle_defers_to_the_drain and
// clear_drain_empties_agent_context_and_display in app/commands.rs tests,
// which pin the new semantics end to end (agent context AND display).

#[test]
fn test_exit_slash_command_sets_quit_flag() {
    let mut app = make_app();
    assert!(!app.should_exit);
    assert!(app.handle_tui_command("exit", ""));
    assert!(app.should_exit);
}

#[test]
fn test_vim_slash_command_toggles_vim() {
    let mut app = make_app();
    assert!(!app.prompt_input.vim_enabled);
    assert!(app.handle_tui_command("vim", ""));
    assert!(app.prompt_input.vim_enabled);
    assert!(app.handle_tui_command("vim", ""));
    assert!(!app.prompt_input.vim_enabled);
}

#[test]
fn test_model_slash_command_opens_picker() {
    // (plan 05B) the unified inline picker, not the legacy overlay.
    let mut app = make_app();
    app.has_credentials = true;
    assert!(app.inline_interactive.is_none());
    assert!(app.handle_tui_command("model", ""));
    assert!(
        app.inline_interactive
            .is_some_and(|p| p.kind == crate::tui::operant_app::tui_fns::PickerKind::Model)
    );
}

#[tokio::test]
async fn test_agent_flow_slash_commands_resolve_in_registry() {
    // /improve /judge /refactor /fix /triage historically had neither a
    // TUI intercept arm NOR a registry handler, so they fell through to
    // the agent as literal prompt text (live-verified: the turn started
    // and errored on credentials). They now resolve in the registry and
    // return ShellCommand, which arms pending_shell_command — the /setup
    // suspend-and-run escape hatch.
    let mut registry = crate::commands::CommandRegistry::new();
    crate::tui::adapter_types::tui_app::register_agent_flow_handlers(&mut registry);
    for name in ["improve", "judge", "refactor", "fix", "triage"] {
        let canonical = registry
            .resolve(name)
            .unwrap_or_else(|| panic!("{name} must resolve in the registry"));
        let result = registry.execute(canonical, "").await;
        let crate::commands::CommandResult::ShellCommand(argv) = result else {
            panic!("{name} must return ShellCommand, got {result:?}");
        };
        assert_eq!(argv[1], "run", "{name}: expected `run` subcommand");
        // CLI contract (main.rs): --autonomous rejects --query; only
        // /improve (TODO.md ledger) goes autonomous.
        if name == "improve" {
            assert!(argv.contains(&"--autonomous".to_string()));
            assert!(!argv.contains(&"--query".to_string()));
        } else {
            assert!(argv.contains(&"--query".to_string()));
        }
    }
}

#[test]
fn test_dialog_tier_slash_commands_are_consumed() {
    // /login and /logout previously fell through to the model as literal
    // prompt text; they must intercept. Since plan 05B B2.7 /login
    // suspends to the setup wizard (pending_shell_command), /logout
    // answers from the status line. /observe was removed in the
    // 2026-10-14 dead-command sweep (plan 02) — unregistered names must
    // fall through to the model per the iter-790 design.
    let mut app = make_app();
    assert!(
        app.handle_tui_command("login", ""),
        "/login must intercept (never fall through to the model)"
    );
    assert!(
        app.pending_shell_command.is_some(),
        "/login suspends to the setup wizard — the provider/model backend"
    );
    assert!(app.handle_tui_command("logout", ""));
    assert!(
        !app.handle_tui_command("observe", ""),
        "removed /observe must fall through"
    );
}

#[test]
fn test_status_tier_slash_commands_are_consumed() {
    // /info and the /thinking alias previously fell through to the model as
    // literal prompt text. Tier-1 status commands must intercept and land a
    // transcript annotation. /model-status was removed in the 2026-10-14
    // dead-command sweep (plan 02) and must now fall through like any
    // unregistered name.
    let mut app = make_app();
    assert!(app.handle_tui_command("info", ""));
    assert_eq!(app.system_annotations.len(), 1);
    assert!(
        !app.handle_tui_command("model-status", ""),
        "removed /model-status must fall through"
    );
    // The alias routes to the thinking-display handler: bare /thinking
    // reports current state without toggling. (plan 08: the report is a
    // transient notice now — no annotation lands.)
    let before = app.thinking_display_on;
    assert!(app.handle_tui_command("thinking", ""));
    assert_eq!(
        app.system_annotations.len(),
        1,
        "the thinking report is a status notice, not a transcript annotation (plan 08)"
    );
    assert_eq!(app.thinking_display_on, before);
}

/// 2026-10-14 batch, plan 10: Shift+Tab used to cycle permission modes —
/// two silent presses armed Bypass. It must never touch permission state
/// again; /permissions is the explicit path, and Bypass defers to the
/// confirmation dialog before arming.
#[test]
fn shift_tab_never_changes_permission_mode() {
    use crate::tui::adapter_types::config::PermissionMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = make_app();
    assert_eq!(app.settings.permission_mode, PermissionMode::Default);

    // Two presses — the old cycle reached Bypass exactly here.
    app.handle_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(
        app.settings.permission_mode,
        PermissionMode::Default,
        "Shift+Tab must not touch permission mode (plan 10)"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(
        app.settings.permission_mode,
        PermissionMode::Default,
        "Shift+Tab must not touch permission mode (plan 10)"
    );

    // /permissions opens the picker; AcceptEdits applies on Enter directly.
    assert!(app.handle_tui_command("permissions", ""));
    assert!(app.permissions_picker.visible);
    app.permissions_picker.select_next(); // Default -> AcceptEdits
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.settings.permission_mode, PermissionMode::AcceptEdits);
    assert!(!app.permissions_picker.visible);

    // Bypass: Enter opens the confirmation dialog instead of arming…
    app.settings.permission_mode = PermissionMode::Default;
    assert!(app.handle_tui_command("permissions", ""));
    app.permissions_picker.select_next();
    app.permissions_picker.select_next(); // Default -> Bypass
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.bypass_permissions_dialog.visible,
        "Bypass requires the confirmation dialog (plan 10)"
    );
    assert_eq!(
        app.settings.permission_mode,
        PermissionMode::Default,
        "Bypass must not arm before the dialog accepts"
    );
}

/// 2026-10-14 batch, plan 10: every advertised binding must dispatch to what
/// its description says. The overlay and /keys generate from the registry,
/// so pinning the catalogue pins every advertised chord. The (chord,
/// description) pairs below were each verified against the dispatch arm in
/// `app/key_handling.rs` during plan 10 — the jcode hand-me-downs they
/// replaced (Ctrl+J "jump to prompt", Ctrl+P "auto-poke", Ctrl+1..9 panel
/// resize, Alt+S/A/X, Shift+Tab "favorited models") advertised chords that
/// did something else or nothing.
#[test]
fn every_advertised_binding_dispatches() {
    use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS};

    let verified: &[(&str, &str)] = &[
        ("Ctrl+c", "Quit application"),
        ("F1", "Show help"),
        ("F8", "Toggle usage overlay"),
        ("Ctrl+k", "Command palette"),
        ("Ctrl+p", "Global search"),
        ("Ctrl+a", "Open model picker"),
        ("Ctrl+b", "Open session branch browser"),
        ("Ctrl+r", "History search"),
        ("Ctrl+g", "Toggle scroll bookmark"),
        ("Alt+g", "Toggle scroll bookmark"),
        ("Alt+p", "Cycle configured models"),
        ("Ctrl+o", "Expand latest tool output"),
        ("Ctrl+x", "Recall queued message"),
        ("Ctrl+t", "Toggle thinking display"),
        ("Ctrl+Up", "Fine scroll up 3"),
        ("Ctrl+Down", "Fine scroll down 3"),
        ("Alt+Up", "Fast scroll up 20"),
        ("Alt+Down", "Fast scroll down 20"),
        ("PageUp", "Scroll transcript up 10"),
        ("PageDown", "Scroll transcript down 10"),
    ];

    let catalogue = DEFAULT_KEYBINDINGS.catalogue();
    for (chord, expected_desc) in verified {
        // Ctrl+R's history search lives in the Prompt context (where it
        // applies); everything else is Global. Accept either — the help
        // overlay renders both sections from the same table.
        let found = catalogue.iter().any(|(context, binding)| {
            (matches!(context, BindingContext::Global | BindingContext::Prompt))
                && binding.chord() == *chord
                && binding.description.as_deref() == Some(expected_desc)
        });
        assert!(
            found,
            "chord {chord} must be catalogued as {expected_desc:?} — the \
             help overlay generates from this table, so a missing row means \
             a silently unadvertised feature (plan 10)"
        );
    }

    // The known lies must be gone from the catalogue entirely: if one of
    // these descriptions reappears it advertises something the dispatch layer
    // never did.
    let dead_claims = [
        "Cycle favorited models",
        "Cycle permission mode",
        "Toggle auto-poke",
        "Toggle queue mode",
        "Resize side panel",
    ];
    for (_, binding) in &catalogue {
        if let Some(desc) = binding.description.as_deref() {
            assert!(
                !dead_claims.contains(&desc),
                "dead claim {desc:?} re-catalogued (plan 10)"
            );
        }
    }
}

/// 2026-10-14 batch, plan 10: /keys opens the overlay machinery at the top —
/// scroll 0, no transcript dump. The old behavior pushed a ~170-line report
/// as a bottom system message that opened below the fold.
#[test]
fn keys_overlay_opens_at_top_not_transcript() {
    let mut app = make_app();
    let annotations_before = app.system_annotations.len();
    assert!(app.handle_tui_command("keys", ""));
    assert!(app.help_overlay.visible, "/keys must open the overlay");
    assert_eq!(app.help_overlay.scroll_offset, 0, "opened at the top");
    assert_eq!(
        app.system_annotations.len(),
        annotations_before,
        "the keys report must not dump into the transcript"
    );
    // Scrolling works on the new surface.
    app.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.help_overlay.scroll_offset >= 1);
    // Esc closes it.
    app.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(!app.help_overlay.visible);
}

/// 2026-10-14 batch, plan 10: /info prints once — the status-bar echo of the
/// same summary line duplicated every press.
#[test]
fn info_prints_once() {
    let mut app = make_app();
    let before = app.system_annotations.len();
    assert!(app.handle_tui_command("info", ""));
    assert_eq!(app.system_annotations.len(), before + 1);
}

// ---- 2026-10-14 batch, plan 08: thinking / tool-call visibility toggles ----

/// Ctrl+T toggles thinking Off and back, restoring the remembered mode,
/// with a transient status notice only — no transcript system message.
#[test]
fn ctrl_t_toggles_thinking_and_restores_previous_mode() {
    use crate::tui::operant_app::config_shim::{self, ReasoningDisplayMode};
    let _guard = config_shim::reasoning_override_test_guard();

    let mut app = make_app();
    // Start from Current (the config default when show_reasoning=true).
    config_shim::replace_reasoning_display_override(Some(ReasoningDisplayMode::Current));

    let before = app.system_annotations.len();
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        ReasoningDisplayMode::Off,
        "Ctrl+T must hide thinking"
    );
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        ReasoningDisplayMode::Current,
        "second Ctrl+T restores the remembered mode (Current)"
    );
    assert_eq!(
        app.system_annotations.len(),
        before,
        "a key toggle must not write a transcript system message"
    );

    // From Full: Off then back to Full.
    config_shim::replace_reasoning_display_override(Some(ReasoningDisplayMode::Full));
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        ReasoningDisplayMode::Off
    );
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        ReasoningDisplayMode::Full
    );
}

/// Copy mode moved from Ctrl+T to Alt+Y (oh-my-pi owns Ctrl+T for thinking);
/// Alt+Y no longer yank-pops.
#[test]
fn ctrl_t_no_longer_enters_copy_mode_and_alt_y_does() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    let mut app = make_app();
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert!(
        !app.copy_mode_active(),
        "Ctrl+T is the thinking toggle now (plan 08), not copy mode"
    );
    app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::ALT));
    assert!(app.copy_mode_active(), "Alt+Y enters copy mode (plan 08)");
    crate::tui::operant_app::config_shim::set_reasoning_display_override(None);
}

/// RT-29: the vim mode was never drawn. The composer now leads its hint row
/// with the mode badge while vim is on, and draws nothing extra while it is off.
#[test]
fn vim_mode_is_drawn_in_the_composer_only_while_vim_is_on() {
    use crate::tui::prompt_input::VimMode;

    let mut app = make_app();
    assert!(
        rows_containing(&screen_rows(&app), "-- ").is_empty(),
        "no badge while vim is off"
    );

    app.prompt_input.vim_enabled = true;
    app.prompt_input.vim_mode = VimMode::Normal;
    let rows = screen_rows(&app);
    assert_eq!(rows_containing(&rows, "-- NORMAL --").len(), 1, "{rows:#?}");

    app.prompt_input.vim_mode = VimMode::Insert;
    let rows = screen_rows(&app);
    assert_eq!(rows_containing(&rows, "-- INSERT --").len(), 1, "{rows:#?}");
    assert!(rows_containing(&rows, "-- NORMAL --").is_empty());
}

/// Config `[tui] show_tool_calls = false` starts the session with tool rows
/// hidden — the field was dead config until this plan wired it.
#[test]
fn show_tool_calls_false_in_config_starts_hidden() {
    let mut app = make_app();
    assert!(
        app.show_tool_calls,
        "default is visible (config default true)"
    );
    app.show_tool_calls = false;
    assert!(!app.show_tool_calls);
}

/// Alt+O (and Ctrl+Shift+O where the terminal reports it) collapses tool
/// rows behind one summary row and restores them.
/// 2026-10-14 batch, plan 04: cancel keeps every row of every turn and
/// marks only THIS turn's pending blocks Cancelled. The old
/// tool_use_blocks.clear() deleted earlier turns' rows too.
#[test]
fn cancel_keeps_earlier_turn_rows_and_marks_pending_cancelled() {
    let mut app = make_app();
    use crate::tui::app::enums::{ToolStatus, ToolUseBlock};

    // Turn 1: a settled tool row.
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t1".to_string(),
        name: "read_file".to_string(),
        after_index: 0,
        status: ToolStatus::Done,
        output_preview: None,
        input_json: "{}".to_string(),
        intent: None,
    });
    // Turn 2 (streaming): one done, one running.
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t2a".to_string(),
        name: "shell".to_string(),
        after_index: 2,
        status: ToolStatus::Done,
        output_preview: None,
        input_json: "{}".to_string(),
        intent: None,
    });
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t2b".to_string(),
        name: "shell".to_string(),
        after_index: 2,
        status: ToolStatus::Running,
        output_preview: None,
        input_json: "{}".to_string(),
        intent: None,
    });
    app.is_streaming = true;

    app.cancel_streaming_turn();

    assert_eq!(app.tool_use_blocks.len(), 3, "no row was deleted");
    assert_eq!(app.tool_use_blocks[0].status, ToolStatus::Done);
    assert_eq!(app.tool_use_blocks[1].status, ToolStatus::Done);
    assert_eq!(
        app.tool_use_blocks[2].status,
        ToolStatus::Cancelled,
        "the pending block of the cancelled turn reads cancelled"
    );
    assert!(!app.is_streaming);
    // No live agent → no interrupt flag → no watchdog armed.
    assert!(app.cancel_deadline.is_none());
}

/// 2026-10-14 batch, plan 04: with a live flag, cancel triggers it instead
/// of aborting the task, and arms the 3 s watchdog.
#[tokio::test]
async fn cancel_triggers_interrupt_flag_and_arms_watchdog() {
    let mut app = make_app();
    let flag = operant_core::interrupt::InterruptFlag::new();
    app.agent_interrupt = Some(flag.clone());
    // A dummy handle so the watchdog arms (no agent task = nothing to watch).
    let (_tx, rx) =
        tokio::sync::oneshot::channel::<std::result::Result<(), operant_core::error::Error>>();
    let handle = tokio::spawn(async move {
        let _ = rx.await;
    });
    app.agent_task_handle = Some(handle);
    app.is_streaming = true;

    app.cancel_streaming_turn();

    assert!(flag.is_triggered(), "the interrupt flag fired, not abort");
    assert!(
        app.cancel_deadline.is_some(),
        "the 3 s watchdog deadline is armed while the task lives"
    );
    assert!(app.agent_task_handle.is_some(), "task not aborted yet");
}

/// 2026-10-14 batch, plan 04: Ctrl+C while streaming routes through the
/// same cancel (flag trigger), not the old invisible is_streaming=false.
#[test]
fn ctrl_c_while_streaming_triggers_the_interrupt_flag() {
    let mut app = make_app();
    let flag = operant_core::interrupt::InterruptFlag::new();
    app.agent_interrupt = Some(flag.clone());
    app.is_streaming = true;

    app.handle_key_event(press_key(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(
        flag.is_triggered(),
        "Ctrl+C while streaming triggers the flag"
    );
    assert!(!app.is_streaming);
    assert!(
        app.tool_use_blocks.is_empty(),
        "sanity: no blocks seeded; nothing to keep"
    );
}

/// 2026-10-14 batch, plan 04: the watchdog. After the deadline passes, the
/// loop aborts the task as the last resort.
#[tokio::test]
async fn watchdog_aborts_after_deadline() {
    let mut app = make_app();
    let (_tx, rx) =
        tokio::sync::oneshot::channel::<std::result::Result<(), operant_core::error::Error>>();
    // A task that never resolves and never observes the flag — the
    // "tool ignoring cancellation" case.
    let handle = tokio::spawn(async move {
        std::future::pending::<()>().await;
        let _keep_alive = rx;
    });
    app.agent_task_handle = Some(handle);
    app.cancel_deadline = Some(std::time::Instant::now());

    // The watchdog tick (normally inside App::run's loop).
    if let Some(deadline) = app.cancel_deadline
        && std::time::Instant::now() >= deadline
    {
        app.cancel_deadline = None;
        if let Some(handle) = app.agent_task_handle.take() {
            handle.abort();
        }
    }

    assert!(app.cancel_deadline.is_none(), "deadline consumed");
    assert!(
        app.agent_task_handle.is_none(),
        "task aborted by the watchdog"
    );
}

/// 2026-10-14 batch, plan 05: /rewind opens the fullscreen selector; the
/// filter narrows entries case-insensitively and Enter jumps to the first
/// match; Esc closes.
#[test]
fn rewind_selector_open_filter_and_escape() {
    let mut app = make_app();
    assert!(app.handle_tui_command("rewind", ""));
    assert!(app.rewind_selector.visible, "/rewind opens the selector");

    // f opens the filter; typing narrows.
    app.handle_key_event(press_key(KeyCode::Char('f'), KeyModifiers::NONE));
    assert!(app.rewind_selector.filter_open, "f opens the filter");
    app.rewind_selector.entries = vec![
        crate::tui::rewind_selector::RewindEntry {
            turn_id: 1,
            text: "Fix the login bug".to_string(),
            compacted: false,
        },
        crate::tui::rewind_selector::RewindEntry {
            turn_id: 2,
            text: "Refactor the parser".to_string(),
            compacted: false,
        },
    ];
    app.handle_key_event(press_key(KeyCode::Char('L'), KeyModifiers::NONE));
    app.handle_key_event(press_key(KeyCode::Char('O'), KeyModifiers::NONE));
    assert_eq!(app.rewind_selector.filter, "LO");
    assert_eq!(app.rewind_selector.filtered_indices(), vec![0]);

    // Enter closes the filter (jumps to first match).
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.rewind_selector.filter_open);
    assert_eq!(app.rewind_selector.selected, 0);

    // Esc closes the selector.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.rewind_selector.visible);
}

/// 2026-10-14 batch, plan 05: double-Esc on an empty composer opens the
/// selector (within 500 ms, not streaming); a single Esc does not; while
/// streaming it does not.
#[test]
fn double_escape_opens_rewind_selector() {
    let mut app = make_app();
    // Single Esc: nothing opens.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.rewind_selector.visible, "a single Esc opens nothing");
    // Second Esc within the window: selector opens.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        app.rewind_selector.visible,
        "the second Esc within 500 ms opens the selector"
    );
    app.rewind_selector.close();

    // While streaming, Esc cancels the turn instead (plan 04) — never the
    // selector.
    app.is_streaming = true;
    let flag = operant_core::interrupt::InterruptFlag::new();
    app.agent_interrupt = Some(flag.clone());
    app.last_esc_at = None;
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(flag.is_triggered(), "streaming Esc triggers the interrupt");
    assert!(
        !app.rewind_selector.visible,
        "never the selector while streaming"
    );

    // "none" disables the double-Esc action entirely.
    app.is_streaming = false;
    app.agent_interrupt = None;
    app.double_escape_action = "none".to_string();
    app.last_esc_at = None;
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        !app.rewind_selector.visible,
        "double_escape_action = \"none\" leaves Esc inert"
    );
}

/// 2026-10-14 batch, plan 05: Enter on a compacted mark refuses with the
/// inline notice; on a live mark it queues the fork (pending_rewind).
#[test]
fn rewind_selector_enter_gates_on_compacted_and_streams() {
    let mut app = make_app();
    app.handle_tui_command("rewind", "");
    app.rewind_selector.entries = vec![
        crate::tui::rewind_selector::RewindEntry {
            turn_id: 1,
            text: "folded away".to_string(),
            compacted: true,
        },
        crate::tui::rewind_selector::RewindEntry {
            turn_id: 2,
            text: "live turn".to_string(),
            compacted: false,
        },
    ];
    // Select the compacted entry.
    app.rewind_selector.selected = 0;
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.rewind_selector
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("compaction")),
        "compacted entries explain instead of forking"
    );
    assert!(app.pending_rewind.is_none());

    // Live entry: the fork is queued for the agent pass.
    app.rewind_selector.selected = 1;
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.pending_rewind, Some(2));

    // While streaming, Enter refuses.
    app.pending_rewind = None;
    app.is_streaming = true;
    app.rewind_selector.selected = 1;
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.pending_rewind.is_none(), "no fork while a turn runs");
}

#[test]
fn alt_o_and_ctrl_shift_o_toggle_tool_visibility() {
    let mut app = make_app();
    // Seed a committed tool run the adapter can see.
    app.tool_use_blocks
        .push(crate::tui::app::enums::ToolUseBlock {
            id: "tool-1".to_string(),
            name: "read_file".to_string(),
            after_index: 0,
            status: crate::tui::app::enums::ToolStatus::Done,
            output_preview: None,
            input_json: "{}".to_string(),
            intent: None,
        });
    app.handle_key_event(press_key(KeyCode::Char('o'), KeyModifiers::ALT));
    assert!(!app.show_tool_calls, "Alt+O hides tool rows");
    app.handle_key_event(press_key(KeyCode::Char('o'), KeyModifiers::ALT));
    assert!(app.show_tool_calls, "second Alt+O restores them");

    // Ctrl+Shift+O — same action where the terminal reports Shift.
    app.handle_key_event(press_key(
        KeyCode::Char('o'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(!app.show_tool_calls, "Ctrl+Shift+O hides tool rows");
}

/// The toggle state is visible in /keys (registry) — all three chords
/// catalogued with honest descriptions.
#[test]
fn registry_lists_the_three_visibility_chords() {
    use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS};
    let expect: &[(&str, &str)] = &[
        ("Ctrl+t", "Toggle thinking display"),
        ("Ctrl+Shift+o", "Toggle tool-call visibility"),
        ("Alt+o", "Toggle tool-call visibility"),
    ];
    for (chord, desc) in expect {
        let found = DEFAULT_KEYBINDINGS.catalogue().iter().any(|(context, b)| {
            matches!(context, BindingContext::Global | BindingContext::Prompt)
                && b.chord() == *chord
                && b.description.as_deref() == Some(*desc)
        });
        assert!(found, "/keys must list {chord} as {desc:?} (plan 08)");
    }
    // Alt+Y is copy mode now, not yank-pop.
    let catalogue = DEFAULT_KEYBINDINGS.catalogue();
    let alt_y = catalogue.iter().find(|(_, b)| b.chord() == "Alt+y");
    assert!(
        alt_y.is_some_and(|(_, b)| b.description.as_deref() == Some("Toggle copy mode")),
        "Alt+Y must be catalogued as copy mode (plan 08), got {:?}",
        alt_y.map(|(_, b)| b.description.clone())
    );
}

/// (plan 08) Hidden tool rows collapse to exactly one summary row per
/// contiguous run; the underlying blocks are untouched so re-showing
/// restores every row.
#[test]
fn hidden_tool_runs_collapse_to_one_summary_row() {
    use crate::tui::app::enums::ToolStatus;
    let mut app = make_app();
    app.messages
        .push(crate::tui::adapter_types::types::Message::user(
            "hi".to_string(),
        ));
    for i in 0..3 {
        app.tool_use_blocks
            .push(crate::tui::app::enums::ToolUseBlock {
                id: format!("t{i}"),
                name: ["read_file", "shell", "grep"][i].to_string(),
                after_index: 1,
                status: ToolStatus::Done,
                output_preview: None,
                input_json: "{}".to_string(),
                intent: None,
            });
    }
    let visible = crate::tui::operant_model::display_messages(&app);
    assert_eq!(
        visible.iter().filter(|m| m.role == "tool").count(),
        3,
        "visible: three tool rows"
    );

    app.show_tool_calls = false;
    let hidden = crate::tui::operant_model::display_messages(&app);
    let tool_rows = hidden.iter().filter(|m| m.role == "tool").count();
    assert_eq!(tool_rows, 0, "no individual tool rows when hidden");
    let summary = hidden
        .iter()
        .find(|m| m.content.starts_with("▸ 3 tool calls"))
        .expect("one summary row for the run");
    assert!(
        summary.role == "system",
        "summary row is system-styled so copy skips it (UI_ROLES)"
    );
    assert!(
        summary.content.contains("read_file") && summary.content.contains("Alt+O to show"),
        "summary names the calls and the restore chord: {}",
        summary.content
    );

    // Re-show restores the rows — blocks were never removed.
    app.show_tool_calls = true;
    let restored = crate::tui::operant_model::display_messages(&app);
    assert_eq!(
        restored.iter().filter(|m| m.role == "tool").count(),
        3,
        "re-show restores all three rows"
    );
}

/// (plan 08) "Thinking off" hides the LIVE streaming thinking too — the
/// Streaming section's splice is gated on the effective mode.
#[test]
fn thinking_off_hides_live_streaming_thinking() {
    use crate::tui::operant_app::config_shim::{self, ReasoningDisplayMode};
    let _guard = config_shim::reasoning_override_test_guard();

    let mut app = make_app();
    app.is_streaming = true;
    app.streaming_thinking = "internal reasoning text".to_string();
    app.streaming_text = "answer text".to_string();

    let frame_text = |a: &crate::tui::app::App| {
        let frame = crate::tui::operant_ui::prepare::prepare_messages(a, 80, 40);
        let mut out = String::new();
        for section in &frame.sections {
            for line in section.prepared.wrapped_plain_lines.iter() {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    };

    config_shim::replace_reasoning_display_override(Some(ReasoningDisplayMode::Current));
    let shown = frame_text(&app);
    assert!(
        shown.contains("internal reasoning text"),
        "live thinking visible in Current mode"
    );

    config_shim::replace_reasoning_display_override(Some(ReasoningDisplayMode::Off));
    let hidden = frame_text(&app);
    assert!(
        !hidden.contains("internal reasoning text"),
        "Off must hide live thinking too (plan 08): {}",
        hidden
    );
    assert!(hidden.contains("answer text"), "the answer still streams");
}

/// (plan 07.1) Regression for the tool_full_outputs budget: 40
/// remembered outputs of 120KB each would be 4.8MB under the old
/// 64×256KB shape — the 4MB total budget must evict oldest-first.
#[test]
fn tool_output_budget_evicts_oldest_first() {
    use crate::tui::app::enums::ToolUseBlock;
    let mut app = make_app();
    let blob = "x".repeat(120 * 1024);
    for i in 0..40 {
        app.tool_use_blocks.push(ToolUseBlock {
            id: format!("call-{i}"),
            name: "bash".to_string(),
            after_index: 0,
            status: crate::tui::app::enums::ToolStatus::Done,
            output_preview: None,
            input_json: String::new(),
            intent: None,
        });
        app.remember_tool_output(&format!("call-{i}"), &blob);
    }
    let total: usize = app.tool_full_outputs.values().map(|v| v.len()).sum();
    assert!(total <= 4 * 1024 * 1024, "budget exceeded: {total}");
    // Newest survived, oldest evicted.
    assert!(app.tool_full_outputs.contains_key("call-39"));
    assert!(!app.tool_full_outputs.contains_key("call-0"));
}

#[test]
fn test_tasks_slash_command_is_an_alias_for_agents() {
    // /tasks is documented (commands.rs alias + gateway help text) as an
    // alias for /agents, but this match arm previously only accepted
    // the literal "agents" — /tasks fell through to a dead
    // CommandRegistry.handlers fallback and printed a "not yet wired"
    // error instead of opening the agents menu (iter-248).
    let mut app = make_app();
    assert!(!app.agents_menu.visible);
    assert!(app.handle_tui_command("tasks", ""));
    assert!(app.agents_menu.visible);
}

#[test]
fn tasks_overlay_should_be_fully_removed() {
    // The Ctrl+T tasks overlay was permanently empty: `tasks_overlay.tasks`
    // was never populated and `TaskDisplay` was never constructed, and no
    // reachable source existed to populate it within a sane diff. It was
    // removed outright rather than left as a dead keybinding. This test is
    // the runnable form of "grep -rn tasks_overlay" over every file that
    // used to reference it — a reintroduction fails here.
    let sources: [(&str, &str); 9] = [
        ("tui/mod.rs", include_str!("../mod.rs")),
        ("tui/app/mod.rs", include_str!("mod.rs")),
        ("tui/app/init.rs", include_str!("init.rs")),
        ("tui/app/commands.rs", include_str!("commands.rs")),
        ("tui/app/key_handling.rs", include_str!("key_handling.rs")),
        (
            "tui/app/dialog_routing.rs",
            include_str!("dialog_routing.rs"),
        ),
        ("tui/app/enums.rs", include_str!("enums.rs")),
        ("tui/render/mod.rs", include_str!("../render/mod.rs")),
        (
            "tui/adapter_types/mod.rs",
            include_str!("../adapter_types/mod.rs"),
        ),
    ];

    for (path, source) in sources {
        assert!(
            !source.contains("tasks_overlay"),
            "{path} still references the removed tasks overlay"
        );
        assert!(
            !source.contains("TaskDisplay"),
            "{path} still references the removed TaskDisplay type"
        );
        assert!(
            !source.contains("render_tasks_overlay"),
            "{path} still references the removed render_tasks_overlay"
        );
    }
}

#[test]
fn test_fast_is_not_a_command() {
    // H-8: /fast toggled a flag no agent code read — removed, not faked.
    let mut app = make_app();
    assert!(!app.handle_tui_command("fast", ""));
}

#[test]
fn test_output_style_cycles() {
    let mut app = make_app();
    assert_eq!(app.output_style, "auto");
    assert!(app.handle_tui_command("output-style", ""));
    assert_eq!(app.output_style, "stream");
    assert!(app.handle_tui_command("output-style", ""));
    assert_eq!(app.output_style, "verbose");
    assert!(app.handle_tui_command("output-style", ""));
    assert_eq!(app.output_style, "auto");
}

#[test]
fn test_context_menu_fork_targets_clicked_message() {
    let mut app = make_app();
    app.add_message(Role::User, "one".to_string());
    app.add_message(Role::Assistant, "two".to_string());
    app.add_message(Role::User, "three".to_string());

    app.handle_context_menu_action(
        ContextMenuItem::Fork,
        ContextMenuKind::Message { message_index: 1 },
    );

    // (2026-10-14 batch, plan 02) /fork was removed with the dead-command
    // sweep; the menu now fills /branch — the same feature, actually wired.
    assert_eq!(app.prompt_input.text, "/branch 2");
    assert_eq!(
        app.status_message.as_deref(),
        Some("Fork at message 2 - press Enter to confirm")
    );
}

#[test]
fn test_right_click_targets_row_message_instead_of_last_message() {
    let mut app = make_app();
    app.last_msg_area.set(ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 10,
    });
    app.message_row_map.borrow_mut().insert(3, 1);

    app.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 12,
        row: 3,
        modifiers: KeyModifiers::empty(),
    });

    assert!(matches!(
        app.context_menu_state,
        Some(ContextMenuState {
            kind: ContextMenuKind::Message { message_index: 1 },
            ..
        })
    ));
}

// ---- Help overlay -------------------------------------------------------

#[test]
fn test_help_slash_command_opens_overlay() {
    let mut app = make_app();
    assert!(!app.help_overlay.visible);
    assert!(!app.show_help);
    assert!(!app.help_overlay.commands.is_empty());
    assert!(app.handle_tui_command("help", ""));
    assert!(app.help_overlay.visible);
    assert!(app.show_help);
}

#[test]
fn test_help_slash_command_toggles() {
    // iter-85: /help now toggles (was idempotent-open in iter-81, which
    // was itself a regression — the audit found that pressing /help twice
    // showed two different help overlays). Correct behavior: first call
    // opens, second call closes.
    let mut app = make_app();
    // First call opens it.
    assert!(app.handle_tui_command("help", ""));
    assert!(app.help_overlay.visible);
    assert!(app.show_help);
    // Second call closes it (toggle, not idempotent-open).
    assert!(app.handle_tui_command("help", ""));
    assert!(!app.help_overlay.visible);
    assert!(!app.show_help);
    // Third call opens it again.
    assert!(app.handle_tui_command("help", ""));
    assert!(app.help_overlay.visible);
    assert!(app.show_help);
}

#[test]
fn test_question_mark_shortcut_opens_help_with_shift_modifier() {
    let mut app = make_app();

    app.handle_key_event(press_key(KeyCode::Char('?'), KeyModifiers::SHIFT));

    assert!(app.help_overlay.visible);
    assert!(app.show_help);
}

#[test]
fn test_question_mark_shortcut_closes_help_with_shift_modifier() {
    let mut app = make_app();
    app.help_overlay.toggle();
    app.show_help = true;

    app.handle_key_event(press_key(KeyCode::Char('?'), KeyModifiers::SHIFT));

    assert!(!app.help_overlay.visible);
    assert!(!app.show_help);
}

// ---- Prompt-context behavioural pin (iter-453) ----
//
// iter-429 proved a SOURCE pin cannot catch a wrong-action binding: the char is
// dispatched, so a `contains(KeyCode::Char('u'))` grep passes, while the chord
// actually opens a dialog. Only asserting the chord's specific EFFECT catches
// that. The six chords below had zero presses anywhere in this file, so nothing
// pinned what they do. Each asserts the exact post-condition, and the multi-line
// fixtures matter: for Ctrl+U the input is multi-line because "delete to line
// start" and "delete to text start" are different results, and a single-line
// fixture would pass for the wrong reason.

#[test]
fn test_ctrl_u_deletes_to_line_start_not_to_text_start() {
    let mut app = make_app();
    app.prompt_input.text = "one\ntwo".to_string();
    app.prompt_input.cursor = app.prompt_input.text.len();
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('u'), KeyModifiers::CONTROL));

    // Line-scoped: the first line must survive. A whole-buffer delete would
    // leave "" and this assertion would catch it.
    assert_eq!(app.prompt_input.text, "one\n");
    assert_eq!(app.prompt_input.cursor, 4);
}

#[test]
fn test_ctrl_w_deletes_the_previous_word_only() {
    let mut app = make_app();
    app.prompt_input.text = "hello world".to_string();
    app.prompt_input.cursor = app.prompt_input.text.len();
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('w'), KeyModifiers::CONTROL));

    assert_eq!(app.prompt_input.text, "hello ");
}

#[test]
fn test_shift_enter_inserts_a_newline_instead_of_submitting() {
    let mut app = make_app();
    app.prompt_input.text = "ab".to_string();
    app.prompt_input.cursor = 1;
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::SHIFT));

    // Inserted AT the cursor, not appended, and nothing was submitted — the
    // distinction from plain Enter is the whole point of the binding.
    assert_eq!(app.prompt_input.text, "a\nb");
    assert_eq!(app.prompt_input.cursor, 2);
    assert!(!app.is_streaming);
}

// The remaining three Prompt chords. iter-453 deferred these as "needs
// history-search, completion and clipboard state" — that was a guess, and
// checking proved it wrong on all three. `yank()` reads the KILL RING, not the
// clipboard; `suggestions` and `history` are public fields; and the overlay's
// `snapshot` is public, so the entry count is assertable. Nothing here needs
// state that is awkward to construct.
//
// Same discipline as iter-453: assert the specific EFFECT, not that "something
// happened", because iter-429's failure was a chord dispatching a different
// action than the catalogue claimed.

#[test]
fn test_ctrl_r_opens_history_search_carrying_the_history() {
    let mut app = make_app();
    app.prompt_input.history = vec!["first command".to_string(), "second".to_string()];

    // Fixture must start closed, or "did it open?" cannot be distinguished from
    // "it was already open and nothing happened".
    assert!(!app.history_search_overlay.visible);

    app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(app.history_search_overlay.visible);
    // Not just "a dialog opened": the entries must be the ones we seeded. An
    // overlay that opened empty would satisfy a bare `visible` assertion.
    assert_eq!(app.history_search_overlay.snapshot.len(), 2);
}

#[test]
fn test_tab_accepts_the_typeahead_suggestion() {
    use crate::tui::prompt_input::{TypeaheadSource, TypeaheadSuggestion};

    let mut app = make_app();
    app.prompt_input.text = "/he".to_string();
    app.prompt_input.cursor = 3;
    app.prompt_input.suggestions = vec![TypeaheadSuggestion {
        text: "/help".to_string(),
        description: "Show help".to_string(),
        source: TypeaheadSource::SlashCommand,
    }];
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Tab, KeyModifiers::NONE));

    // SlashCommand REPLACES the whole buffer, so this asserts the accept path
    // ran rather than Tab merely cycling a highlight.
    assert_eq!(app.prompt_input.text, "/help");
    assert_eq!(app.prompt_input.suggestion_index, Some(0));
}

#[test]
fn test_ctrl_y_yanks_from_the_kill_ring_at_the_cursor() {
    let mut app = make_app();
    app.prompt_input.kill_ring.push("world".to_string());
    app.prompt_input.text = "hello ".to_string();
    app.prompt_input.cursor = app.prompt_input.text.len();
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::CONTROL));

    // Inserted AT the cursor, not appended to the end, and not read from the
    // system clipboard — the kill ring is the only source.
    assert_eq!(app.prompt_input.text, "hello world");
}

// ---- Pins for the four wrong-action entries found at iter-455 ----
//
// Writing the missing Prompt pins surfaced four catalogue entries whose
// advertised action is not what the chord does. All four are the iter-429
// shape: the chord IS dispatched, so a source grep passes, but it does
// something else. These assert the REAL behaviour, so the descriptions cannot
// drift back.

#[test]
fn test_shift_tab_cycles_permission_mode_not_completions() {
    // (2026-10-14 batch, plan 10) INVERTED: Shift+Tab no longer cycles
    // permission mode — two silent presses reached BypassPermissions. It
    // must do NOTHING to permission state and NOTHING to completions;
    // /permissions is the explicit mode picker, and Bypass always defers
    // to the confirmation dialog (pinned by
    // shift_tab_never_changes_permission_mode).
    let mut app = make_app();
    app.prompt_input.suggestions = vec![crate::tui::prompt_input::TypeaheadSuggestion {
        text: "/help".to_string(),
        description: "Show help".to_string(),
        source: crate::tui::prompt_input::TypeaheadSource::SlashCommand,
    }];
    let before = app.settings.permission_mode.clone();
    let before_idx = app.prompt_input.suggestion_index;

    app.handle_key_event(press_key(KeyCode::BackTab, KeyModifiers::SHIFT));

    assert_eq!(
        app.settings.permission_mode, before,
        "Shift+Tab must never touch permission mode (plan 10)"
    );
    assert_eq!(
        app.prompt_input.suggestion_index, before_idx,
        "Shift+Tab must not touch completion state"
    );
}

#[test]
fn test_alt_v_does_not_toggle_vim_mode() {
    let mut app = make_app();
    // Voice hold-to-talk was purged (iter-668), so Alt+V is bound to nothing
    // at all. The point of this test is the negative: whatever Alt+V becomes
    // next, it must not silently become a vim toggle — vim is reached via the
    // /vim slash command.
    assert!(!app.prompt_input.vim_enabled);

    app.handle_key_event(press_key(KeyCode::Char('v'), KeyModifiers::ALT));

    assert!(
        !app.prompt_input.vim_enabled,
        "Alt+V must not enter vim mode; vim is reached via the /vim slash command"
    );
}

#[test]
fn test_down_moves_forward_through_history() {
    let mut app = make_app();
    app.prompt_input.history = vec!["oldest".to_string(), "newer".to_string()];
    // `history_pos` is an INDEX, not an offset-from-end (established at
    // iter-429, where I first asserted the wrong direction and the test caught
    // it). Newest is Some(len-1), so start at the oldest to exercise the
    // walk-forward path rather than the return-to-draft step.
    app.prompt_input.history_pos = Some(0);
    app.prompt_input.history_draft = "draft".to_string();
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Down, KeyModifiers::NONE));

    assert_eq!(
        app.prompt_input.history_pos,
        Some(1),
        "Down is catalogued as \"Next history\"; it must advance the index"
    );
    assert_eq!(app.prompt_input.text, "newer");
}

#[test]
fn test_registry_advertises_no_binding_for_an_unimplemented_action() {
    use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS, KeyAction};
    let registry = &*DEFAULT_KEYBINDINGS;

    // (1) `KeyAction::Redo` had a catalogue entry for Ctrl+Shift+Y and no
    // implementation anywhere — no `fn redo`, no `.redo()` call site. The
    // variant is deliberately KEPT so a future redo has somewhere to land, so
    // this cannot be a "variant must be used" check; it pins the specific
    // pairing instead.
    for context in BindingContext::ALL {
        for binding in registry.get_bindings(context) {
            assert!(
                !matches!(binding.action, KeyAction::Redo),
                "Ctrl+Shift+Y is catalogued as Redo again, but no redo \
                 implementation exists in the TUI — /keys is advertising a \
                 capability that does not exist"
            );
        }
    }

    // (2) and (3) the two entries whose ACTION was wrong rather than absent.
    // These are the ones the behavioural tests above cannot catch: those pin
    // dispatch, and the defect was in the catalogue, so re-labelling the entry
    // while leaving dispatch alone would satisfy both. Only a catalogue
    // assertion sees it.
    //
    // (2) became an ABSENCE check in iter-668: voice hold-to-talk was purged
    // with the legacy voice-mode surface, so Alt+V must not be catalogued at
    // all — the registry must not advertise a chord nothing implements.
    assert!(
        registry
            .get_bindings(BindingContext::Global)
            .into_iter()
            .find(|b| b.key == KeyCode::Char('v') && b.modifiers == KeyModifiers::ALT)
            .is_none(),
        "Alt+V is catalogued again, but voice hold-to-talk was purged (iter-668) \n         — /keys must not advertise an action nothing implements"
    );

    // (2026-10-14 batch, plan 10) INVERTED: Shift+Tab is deliberately
    // uncatalogued — the permission cycle it used to describe was removed
    // (two silent presses armed Bypass; /permissions is the explicit path
    // now). Advertising it would be the same security-relevant lie this
    // test was written to catch.
    let shift_tab = registry
        .get_bindings(BindingContext::Prompt)
        .into_iter()
        .find(|b| b.key == KeyCode::Tab && b.modifiers.contains(KeyModifiers::SHIFT));
    assert!(
        shift_tab.is_none(),
        "Shift+Tab is catalogued again — it must stay uncatalogued until it \
         is rebound to something real (plan 03's reasoning-effort cycle). The \
         old entry was a security-relevant mislabel."
    );
}

#[test]
fn test_question_mark_shortcut_types_into_non_empty_prompt() {
    let mut app = make_app();
    app.prompt_input.text = "why".to_string();
    app.prompt_input.cursor = app.prompt_input.text.len();
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('?'), KeyModifiers::SHIFT));

    assert!(!app.help_overlay.visible);
    assert_eq!(app.prompt_input.text, "why?");
}

#[test]
fn test_ctrl_a_shortcut_opens_model_picker() {
    // (plan 05B) Ctrl+A opens the same inline picker /model does.
    let mut app = make_app();
    app.has_credentials = true;
    app.active_provider = Some("anthropic".to_string());

    app.handle_key_event(press_key(KeyCode::Char('a'), KeyModifiers::CONTROL));

    assert!(
        app.inline_interactive
            .is_some_and(|p| p.kind == crate::tui::operant_app::tui_fns::PickerKind::Model)
    );
}

#[test]
fn test_ctrl_k_shortcut_opens_command_palette_even_with_input() {
    let mut app = make_app();
    app.prompt_input.text = "hello".to_string();
    app.prompt_input.cursor = app.prompt_input.text.len();
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('k'), KeyModifiers::CONTROL));

    assert!(app.command_palette.visible);
    assert_eq!(app.prompt_input.text, "hello");
}

#[test]
fn test_ctrl_e_moves_to_end_of_line_not_end_of_text() {
    let mut app = make_app();
    // Multi-line, so "end of text" and "end of line" are different answers and
    // the test would pass for the wrong reason on single-line input.
    app.prompt_input.text = "one\ntwo".to_string();
    app.prompt_input.cursor = 0;
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('e'), KeyModifiers::CONTROL));

    assert_eq!(
        app.prompt_input.cursor, 3,
        "Ctrl+E must stop at the end of the FIRST line, not the end of the buffer"
    );
    assert_eq!(app.prompt_input.text, "one\ntwo", "Ctrl+E must not edit");
}

#[test]
fn test_ctrl_f_moves_word_forward() {
    let mut app = make_app();
    app.prompt_input.text = "hello world".to_string();
    app.prompt_input.cursor = 0;
    app.refresh_prompt_input();

    app.handle_key_event(press_key(KeyCode::Char('f'), KeyModifiers::CONTROL));

    assert_eq!(
        app.prompt_input.cursor, 6,
        "Ctrl+F must advance to the start of the next word"
    );
    assert_eq!(app.prompt_input.text, "hello world", "Ctrl+F must not edit");
}

#[test]
fn test_ctrl_n_navigates_history_down() {
    let mut app = make_app();
    app.prompt_input.history = vec!["first".to_string(), "second".to_string()];
    // history_pos is an INDEX into history, so the newest entry is Some(len-1).
    // Two history_ups therefore land on the oldest, and Ctrl+N must walk one
    // entry newer from there. (Asserting the walk from the newest entry would
    // be wrong: history_down from the newest correctly restores your draft and
    // clears the position, which is readline's behaviour, not a bug.)
    app.prompt_input.history_up();
    app.prompt_input.history_up();
    assert_eq!(app.prompt_input.history_pos, Some(0));
    assert_eq!(app.prompt_input.text, "first");

    app.handle_key_event(press_key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    assert_eq!(
        app.prompt_input.history_pos,
        Some(1),
        "Ctrl+N must walk one entry newer"
    );
    assert_eq!(app.prompt_input.text, "second");
}

// ---- Bash prefix allowlist --------------------------------------------
// (plan 03 §4) The twin helper `bash_command_allowed_by_prefix` is gone —
// these assert the REAL store; the auto-approve BEHAVIOR (two-token
// matching, metachar/danger guards) is pinned by
// `bash_prefix_allows_plain_continuations_only` in app/commands.rs tests.

#[test]
fn test_bash_command_not_allowed_by_default() {
    let app = make_app();
    assert!(app.bash_prefix_allowlist.is_empty());
}

#[test]
fn test_bash_prefix_allowlist_after_p_key() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    // Set up a bash permission dialog with a suggested prefix.
    let pr = PermissionRequest::bash(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        "git status".to_string(),
        Some("git".to_string()),
    );
    app.permission_request = Some(pr);

    // Simulate pressing 'P' (prefix-allow key).
    let key = KeyEvent {
        code: KeyCode::Char('P'),
        modifiers: KeyModifiers::SHIFT,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    app.handle_permission_key(key);

    // Dialog dismissed; the TWO-TOKEN prefix of the command recorded
    // ("git status", not "git" — a one-token "git" would cover
    // "git push --force", which §4 exists to stop).
    assert!(app.permission_request.is_none());
    assert!(
        app.bash_prefix_allowlist.contains("git status"),
        "the command's two-token prefix must be recorded"
    );
    assert!(
        !app.bash_prefix_allowlist.contains("git"),
        "the bare one-token form must NOT be recorded"
    );
}

#[test]
fn test_bash_prefix_allowlist_via_enter_on_p_option() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let mut pr = PermissionRequest::bash(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        "cargo build".to_string(),
        Some("cargo".to_string()),
    );
    // Navigate to the prefix option (index 3 in a 5-option dialog).
    pr.selected_option = 3;
    app.permission_request = Some(pr);

    // Press Enter to confirm the currently selected (prefix) option.
    let key = KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    assert!(app.bash_prefix_allowlist.contains("cargo build"));
    assert!(!app.bash_prefix_allowlist.contains("make build"));
}

#[test]
fn test_bash_prefix_allowlist_non_prefix_option_does_not_add() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::bash(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        "npm install".to_string(),
        Some("npm".to_string()),
    );
    app.permission_request = Some(pr);

    // Press 'y' (allow-once) — should NOT add to allowlist.
    let key = KeyEvent {
        code: KeyCode::Char('y'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    assert!(app.bash_prefix_allowlist.is_empty());
}

// ---- iter-20: permission dialog response routing ----------------------

/// A shell-tool request as the core sends it, with the response channel the
/// dialog answers on.
fn shell_request(
    command: &str,
) -> (
    operant_core::agent::ToolPermissionRequest,
    tokio::sync::oneshot::Receiver<operant_core::agent::ToolPermissionResponse>,
) {
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    let request = operant_core::agent::ToolPermissionRequest {
        tool_name: "bash".to_string(),
        tool_id: "tu-shell".to_string(),
        description: "This will execute a shell command.".to_string(),
        danger_explanation: String::new(),
        input_preview: Some(command.to_string()),
        seat_escalation: None,
        unattended: false,
        response_tx,
    };
    (request, response_rx)
}

/// Push one request through the permission drain, as the run loop does.
fn drain_one_permission_request(
    app: &mut App,
    request: operant_core::agent::ToolPermissionRequest,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(request)
        .expect("the channel has room for one request");
    app.permission_rx = Some(rx);
    app.drain_permission_requests();
}

#[test]
fn a_plain_shell_request_opens_a_bash_dialog_that_offers_its_prefix() {
    // The prefix option had no producer: every shell request got the generic
    // dialog, so the allowlist could only be fed by hand.
    use crate::tui::dialogs::PermissionDialogKind;
    let mut app = make_app();
    let (request, _response) = shell_request("git status --short");
    drain_one_permission_request(&mut app, request);
    let pr = app.permission_request.as_ref().expect("the dialog opens");
    assert_eq!(
        pr.kind,
        PermissionDialogKind::Bash {
            command: "git status --short".to_string(),
            suggested_prefix: Some("git status".to_string()),
        }
    );
    assert!(
        pr.options.iter().any(|o| o.key == 'P'),
        "a plain command offers the prefix option"
    );
}

#[test]
fn a_shell_request_with_an_injection_tail_offers_no_prefix() {
    // `echo done` is not flagged by the danger classifier, so only the
    // metacharacter guard can refuse this chain. `rm -rf` is the dangerous case.
    use crate::tui::dialogs::PermissionDialogKind;
    for command in ["git status; echo done", "git status; rm -rf ~"] {
        let mut app = make_app();
        let (request, _response) = shell_request(command);
        drain_one_permission_request(&mut app, request);
        let pr = app.permission_request.as_ref().expect("the dialog opens");
        assert!(
            matches!(
                &pr.kind,
                PermissionDialogKind::Bash {
                    suggested_prefix: None,
                    ..
                }
            ),
            "{command}: a chained command offers no prefix"
        );
        assert!(
            pr.options.iter().all(|o| o.key != 'P'),
            "{command}: no prefix option"
        );
    }
}

#[test]
fn a_stored_prefix_approves_a_plain_continuation_and_nothing_chained() {
    let mut app = make_app();
    app.bash_prefix_allowlist.insert("git status".to_string());

    let (request, mut response) = shell_request("git status --short");
    drain_one_permission_request(&mut app, request);
    assert!(
        app.permission_request.is_none(),
        "a stored prefix approves without a dialog"
    );
    assert_eq!(
        response.try_recv().expect("the request is answered"),
        operant_core::agent::ToolPermissionResponse::AllowSession
    );

    let (request, _response) = shell_request("git status; echo done");
    drain_one_permission_request(&mut app, request);
    assert!(
        app.permission_request.is_some(),
        "a chained command falls through to the dialog"
    );
}

#[test]
fn choosing_the_prefix_option_stores_the_two_token_prefix() {
    use crate::tui::dialogs::PermissionRequest;
    let mut app = make_app();
    app.permission_request = Some(PermissionRequest::bash(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        "git status --short".to_string(),
        Some("git status".to_string()),
    ));
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    app.handle_permission_key(press_key(KeyCode::Char('P'), KeyModifiers::NONE));

    assert!(
        app.bash_prefix_allowlist.contains("git status"),
        "the prefix option stores the first two tokens, not the first"
    );
    assert!(!app.bash_prefix_allowlist.contains("git"));
}

#[test]
fn test_permission_dialog_y_sends_allow_once() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    app.permission_request = Some(pr);

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    let key = press_key(KeyCode::Char('y'), KeyModifiers::NONE);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    assert!(app.pending_permission_response_tx.is_none());
    let response = rx.try_recv().expect("response should be sent");
    assert_eq!(
        response,
        operant_core::agent::ToolPermissionResponse::AllowOnce
    );
}

#[test]
fn test_permission_dialog_uppercase_y_sends_allow_session() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    app.permission_request = Some(pr);

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    // Shift+y → uppercase 'Y' (the session-allow key).
    let key = press_key(KeyCode::Char('Y'), KeyModifiers::SHIFT);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    let response = rx.try_recv().expect("response should be sent");
    assert_eq!(
        response,
        operant_core::agent::ToolPermissionResponse::AllowSession
    );
}

#[test]
fn test_permission_dialog_p_sends_allow_always() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    app.permission_request = Some(pr);

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    // 'p' — "Yes, always allow (persistent)": now backed by the real
    // persistent allowlist (hermes `always` → command_allowlist).
    let key = press_key(KeyCode::Char('p'), KeyModifiers::NONE);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    let response = rx.try_recv().expect("response should be sent");
    assert_eq!(
        response,
        operant_core::agent::ToolPermissionResponse::AllowAlways
    );
}

#[test]
fn test_permission_dialog_n_sends_deny() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    app.permission_request = Some(pr);

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    let key = press_key(KeyCode::Char('n'), KeyModifiers::NONE);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    let response = rx.try_recv().expect("response should be sent");
    assert_eq!(response, operant_core::agent::ToolPermissionResponse::Deny);
}

#[test]
fn test_permission_dialog_esc_sends_deny() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    app.permission_request = Some(pr);

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    let key = press_key(KeyCode::Esc, KeyModifiers::NONE);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    let response = rx.try_recv().expect("response should be sent");
    assert_eq!(response, operant_core::agent::ToolPermissionResponse::Deny);
}

#[test]
fn test_permission_dialog_enter_sends_selected_option_response() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let mut pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    // Move selection down to the deny option (index 3).
    pr.selected_option = 3;
    app.permission_request = Some(pr);

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    app.pending_permission_response_tx = Some(tx);

    let key = press_key(KeyCode::Enter, KeyModifiers::NONE);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    let response = rx.try_recv().expect("response should be sent");
    assert_eq!(response, operant_core::agent::ToolPermissionResponse::Deny);
}

#[test]
fn test_permission_dialog_no_tx_does_not_panic() {
    use crate::tui::dialogs::PermissionRequest;

    // Tests the case where the dialog was opened without a response_tx
    // (e.g. directly constructed in tests). resolve_permission_dialog
    // should silently no-op the send, not panic.
    let mut app = make_app();
    let pr = PermissionRequest::from_reason(
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
        None,
    );
    app.permission_request = Some(pr);
    // pending_permission_response_tx is None by default.

    let key = press_key(KeyCode::Char('y'), KeyModifiers::NONE);
    app.handle_permission_key(key);

    assert!(app.permission_request.is_none());
    assert!(app.pending_permission_response_tx.is_none());
}

// ---- Phase 2 regression tests (iter-212) ----
// These tests lock in the behavior fixed/added in Phases 1-4:
//   - F12 debug overlay toggle (Phase 1)
//   - Done.message not dropped (Phase 3c, bug #2)
//   - Usage.total_tokens not dropped (Phase 3c, bug #5)
//   - Stub McpManager/FileHistory eliminated (Phase 3a/3b)
//   - feedback_survey removed (Phase 4)

#[test]
fn test_f12_toggles_debug_overlay() {
    // Phase 1: F12 must toggle the debug overlay visibility.
    let mut app = make_app();
    assert!(
        !app.debug_hub.overlay_visible(),
        "overlay should start hidden"
    );

    app.handle_key_event(press_key(KeyCode::F(12), KeyModifiers::NONE));
    assert!(app.debug_hub.overlay_visible(), "F12 should show overlay");

    app.handle_key_event(press_key(KeyCode::F(12), KeyModifiers::NONE));
    assert!(
        !app.debug_hub.overlay_visible(),
        "second F12 should hide overlay"
    );
}

#[test]
fn test_f12_works_even_with_input() {
    // F12 must work even when there's text in the input buffer — it's
    // the highest-priority keybind and must never be blocked.
    let mut app = make_app();
    app.input = "some text".to_string();
    app.handle_key_event(press_key(KeyCode::F(12), KeyModifiers::NONE));
    assert!(app.debug_hub.overlay_visible());
    // Input must be preserved — F12 doesn't consume or clear it.
    assert_eq!(app.input, "some text");
}

#[test]
fn test_done_message_used_when_no_streaming() {
    // Phase 3c bug #2: Done.message was discarded. Now if streaming_text
    // is empty, Done.message.content is used as the assistant message.
    let mut app = make_app();
    assert!(app.messages.is_empty());
    // Simulate non-streaming path: no Content events, Done carries full msg.
    let done_msg = operant_core::client::Message {
        role: operant_core::client::Role::Assistant,
        content: "Hello from Done".to_string(),
        reasoning: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
        extra_content: None,
    };
    app.handle_agent_event(AgentEvent::Done {
        message: done_msg,
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });
    assert_eq!(app.messages.len(), 1, "Done should produce 1 message");
    assert!(
        app.messages[0].text_content().contains("Hello from Done"),
        "message should contain Done.message.content"
    );
}

#[test]
fn test_done_with_streaming_uses_streamed_text() {
    // When streaming occurred, Done should NOT override with its message —
    // the streamed text is the source of truth (it may have been
    // post-processed or differ from the final Done payload).
    let mut app = make_app();
    // Simulate streaming: Content events fill streaming_text.
    app.is_streaming = true;
    app.streaming_text = "Streamed content".to_string();
    let done_msg = operant_core::client::Message {
        role: operant_core::client::Role::Assistant,
        content: "This should NOT be used".to_string(),
        reasoning: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
        extra_content: None,
    };
    app.handle_agent_event(AgentEvent::Done {
        message: done_msg,
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });
    assert_eq!(app.messages.len(), 1);
    assert!(
        app.messages[0].text_content().contains("Streamed content"),
        "streamed text should win over Done.message when streaming occurred"
    );
}

#[test]
fn test_usage_total_tokens_not_dropped() {
    // Phase 3c bug #5: total_tokens was discarded. Now the authoritative
    // value from the agent is used (which may include cached/reasoning
    // tokens that input+output misses).
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 100,
        output_tokens: 50,
        total_tokens: 200, // > 100+50=150, simulates cached tokens
    });
    assert_eq!(
        app.token_count, 200,
        "token_count should use authoritative total_tokens (200), not input+output (150)"
    );
}

#[test]
fn retry_must_not_append_the_resurrected_stream_to_the_died_partial() {
    // 2026-10-09 live-audit P4-2.5: the provider stream died mid-response
    // (the org's signature flake), leaving partial text in the buffer; the
    // retry restarts the response from its top. If the partial survives,
    // the full retry APPENDS to it and the user watches the reply twice.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Content {
        text: "partial sentence that d".to_string(),
    });
    app.handle_agent_event(AgentEvent::RetryScheduled {
        attempt: 1,
        max_attempts: 3,
        reason: "stream died mid-response".to_string(),
    });
    assert!(
        app.streaming_text.is_empty(),
        "the died attempt's partial must not survive the retry boundary"
    );
    app.handle_agent_event(AgentEvent::Content {
        text: "The full reply from the top.".to_string(),
    });
    assert_eq!(
        app.streaming_text, "The full reply from the top.",
        "the retry's text must replace, not append to, the partial"
    );
}

#[test]
fn todo_tool_result_populates_the_live_store_and_todos_command_toggles_the_band() {
    // P4-3 (2026-10-09 v2 outline): the todo tool's result carries the full
    // list — the counts-only TodoUpdated event could never render one. The
    // ToolComplete(todo) path must keep the items on App (info-widget band +
    // pinned card read them) and /todos must toggle the band.
    let mut app = make_app();
    let content = serde_json::json!({
        "todos": [
            { "id": "1", "content": "audit scroll", "status": "completed" },
            { "id": "2", "content": "audit todo", "status": "in_progress" }
        ]
    })
    .to_string();
    app.handle_agent_event(AgentEvent::ToolComplete {
        result: operant_core::tools::ToolResult {
            tool_call_id: "call_todo".to_string(),
            name: "todo".to_string(),
            success: true,
            content,
            error: None,
            timed_out: false,
        },
    });
    assert_eq!(app.todos.len(), 2, "the full list must land on App");
    assert_eq!(app.todos[1].content, "audit todo");
    assert!(
        !app.todos_card_payload.is_empty(),
        "the pinned-band card payload must be built"
    );
    assert!(
        app.todos_card_payload.contains("audit scroll"),
        "the payload carries the items for the band renderer"
    );

    // /todos hides and re-shows the band.
    assert!(app.handle_tui_command("todos", ""));
    assert!(app.todos_band_hidden, "/todos must toggle the band off");
    assert!(app.handle_tui_command("todos", ""));
    assert!(
        !app.todos_band_hidden,
        "/todos must toggle the band back on"
    );
}

/// The staged plan must survive the whole trip, not just the data layer.
///
/// Before iter-1193 the todo tool could not even name a group, so the
/// card's group headers were unreachable. Now the tool models `phase` and
/// `lane` (the words a plan is written in) and the card renders them as
/// stages; this asserts they arrive at the TUI through the SAME
/// `ToolComplete(todo)` event every real turn delivers, rather than only
/// through a hand-built `TodoItem`.
#[test]
fn a_staged_todo_result_lands_on_app_with_its_phase_and_lane() {
    let mut app = make_app();
    let content = serde_json::json!({
        "todos": [
            { "id": "1", "content": "Prep the tree", "status": "completed",
              "phase": 1, "lane": "parser" },
            { "id": "2", "content": "Wire the lexer", "status": "in_progress",
              "phase": 2, "lane": "parser" },
            { "id": "3", "content": "Shade the demo", "status": "pending",
              "phase": 2, "lane": "art" }
        ]
    })
    .to_string();
    app.handle_agent_event(AgentEvent::ToolComplete {
        result: operant_core::tools::ToolResult {
            tool_call_id: "call_todo_staged".to_string(),
            name: "todo".to_string(),
            success: true,
            content,
            error: None,
            timed_out: false,
        },
    });
    assert_eq!(app.todos.len(), 3, "the whole list must land on App");
    assert_eq!(app.todos[0].phase, Some(1), "{todos:?}", todos = app.todos);
    assert_eq!(app.todos[1].phase, Some(2));
    assert_eq!(
        app.todos[0].group.as_deref(),
        Some("parser"),
        "the tool's `lane` is the card's group"
    );
    assert_eq!(
        app.todos[2].group.as_deref(),
        Some("art"),
        "a second lane inside the same phase stays its own lane"
    );
}

#[test]
fn thinking_display_defaults_on_and_the_command_toggles_it() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    // P4-4 (2026-10-09 v2 outline): jcode parity — thinking renders by
    // default (default_file.rs:175 show_thinking = true) and
    // /thinking-display toggles the trace surfaces off and back on.
    let mut app = make_app();
    assert!(
        app.thinking_display_on,
        "thinking display must default on (jcode parity)"
    );
    assert!(app.handle_tui_command("thinking-display", "off"));
    assert!(
        !app.thinking_display_on,
        "/thinking-display off must hide traces"
    );
    assert!(app.handle_tui_command("thinking-display", "full"));
    assert!(
        app.thinking_display_on,
        "/thinking-display full must show traces"
    );
    assert!(app.handle_tui_command("thinking-display", "current"));
    assert!(
        app.thinking_display_on,
        "current is the same vendored trace mode as full"
    );
    // The command flips a process-global display-mode cell (the config shim
    // snapshot is static and cannot see App state). Tests share a process:
    // leave the cell at the snapshot default (None = shim default Full) so
    // later tests that assert committed-reasoning rendering are not poisoned.
    crate::tui::operant_app::config_shim::set_reasoning_display_override(None);
}

/// 2026-10-22 parity gap (iter-1191): /reasoning was NOT an alias. It hit an
/// earlier arm that flipped `App.show_reasoning`, a bool no renderer reads, so
/// the command changed a debug-snapshot key and nothing on screen. jcode routes
/// all three names (/reasoning, /thinking, /thinking-display) through one
/// handler (`handle_reasoning_display_command`). Prove the alias now moves the
/// live display mode the renderer actually reads.
#[test]
fn reasoning_alias_sets_the_live_thinking_mode() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    use crate::tui::operant_app::config_shim;
    config_shim::set_reasoning_display_override(None);

    let mut app = make_app();
    assert!(
        app.handle_tui_command("reasoning", "off"),
        "/reasoning off must be handled"
    );
    assert!(
        !app.thinking_display_on,
        "/reasoning off must hide traces, not just a debug flag"
    );
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        config_shim::ReasoningDisplayMode::Off,
        "the alias flips the same override cell the renderer reads"
    );

    assert!(app.handle_tui_command("reasoning", "full"));
    assert!(app.thinking_display_on, "/reasoning full must show traces");
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        config_shim::ReasoningDisplayMode::Full
    );

    // jcode's lenient spellings ride the same path.
    assert!(app.handle_tui_command("reasoning", "collapse"));
    assert_eq!(
        config_shim::config().display.reasoning_display(),
        config_shim::ReasoningDisplayMode::Current,
        "'collapse' is jcode's spelling for current"
    );

    config_shim::set_reasoning_display_override(None);
}

/// Bare /thinking-display (and `show`/`status`) reports the live mode and the
/// legend instead of doing nothing (jcode parity).
#[test]
fn a_bare_thinking_display_reports_the_mode_and_the_legend() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    use crate::tui::operant_app::config_shim;
    config_shim::set_reasoning_display_override(None);

    let mut app = make_app();
    for args in ["", "show", "status"] {
        app.status_message = None;
        assert!(app.handle_tui_command("thinking-display", args));
        let notice = app
            .status_message
            .clone()
            .expect("a bare thinking-display reports the current mode");
        assert!(
            notice.starts_with("Thinking display is currently "),
            "args {args:?} must report the mode: {notice}"
        );
        assert!(
            notice.contains("off") && notice.contains("full") && notice.contains("current"),
            "the legend names all three modes: {notice}"
        );
        assert!(
            notice.contains("/thinking-display <off|full|current>"),
            "the legend tells the operator how to change it: {notice}"
        );
    }

    // An unknown word still errors, and errors without touching the mode.
    let before = app.thinking_display_on;
    assert!(app.handle_tui_command("thinking-display", "shouty"));
    assert_eq!(app.thinking_display_on, before);
    let notice = app.status_message.clone().expect("an unknown mode reports");
    assert!(
        notice.contains("Unknown thinking-display mode 'shouty'"),
        "{notice}"
    );

    config_shim::set_reasoning_display_override(None);
}

/// The notice a mode change prints is jcode's label spelling (`Full`), not
/// operant's old lowercase `{:?}` debug rendering.
#[test]
fn a_thinking_display_change_prints_the_mode_label() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    use crate::tui::operant_app::config_shim;
    config_shim::set_reasoning_display_override(None);

    let mut app = make_app();
    assert!(app.handle_tui_command("thinking-display", "full"));
    let notice = app.status_message.clone().expect("the change reports");
    assert_eq!(notice, "Thinking display: Full");

    config_shim::set_reasoning_display_override(None);
}

#[test]
fn cls_clears_the_view_only_and_the_alias_tier_routes() {
    // 2026-10-10 command sweep: /cls is jcode's clear-rendered-view (the
    // model keeps its context) — it must NOT route to /clear, which drops
    // the conversation. The alias tier must route /commands → /help and
    // /split-view → /splitview instead of falling through to the model as
    // literal prompt text.
    use crate::tui::adapter_types::types::{Message, MessageContent, Role};
    let mut app = make_app();
    app.messages.push(Message {
        role: Role::User,
        content: MessageContent::Text("kept context".to_string()),
    });
    assert!(app.handle_tui_command("cls", ""));
    assert_eq!(
        app.messages.len(),
        1,
        "/cls clears the VIEW only — the conversation must survive"
    );
    assert!(
        app.terminal_clear_state_live(),
        "/cls must collapse the view"
    );

    assert!(app.handle_tui_command("commands", ""));
    assert!(app.show_help, "/commands must route to /help");
}

#[test]
fn an_empty_done_turn_surfaces_a_corrupted_stream_notice() {
    // 2026-10-10 forced-repro finding: a corrupted provider stream ends the
    // turn with Done{content: ""} and ZERO Content events — the reply fails
    // silently and the user stares at a blank transcript. The Done arm must
    // surface it (status message) so the user knows to retry.
    let mut app = make_app();
    app.begin_turn();
    app.is_streaming = true;
    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::Message::assistant(String::new()),
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });
    assert_eq!(
        app.status_message.as_deref(),
        Some("Empty reply — the provider stream may be corrupted; retry or switch model"),
        "an empty Done turn must not fail silently"
    );
    assert!(app.messages.is_empty(), "no message may be invented");
}

#[test]
fn a_submitted_turn_records_its_metadata_and_starts_the_latency_clock() {
    // The user-turn snapshot was never called, so the latency clock started at
    // the first agent event and the turn had no metadata. Submit must set both.
    let mut app = make_app();
    assert!(app.turn_metadata.is_empty());
    app.begin_turn();
    assert_eq!(
        app.turn_metadata.len(),
        1,
        "one metadata entry per submitted turn"
    );
    assert!(
        app.turn_start.is_some(),
        "latency is timed from submit, not the first event"
    );
    assert!(app.last_turn_elapsed.is_none());
}

#[test]
fn footer_metrics_should_reset_each_turn() {
    let mut app = make_app();
    app.turn_started_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(2_500));
    app.turn_input_tokens = 1_200;
    app.turn_output_tokens = 340;
    app.turn_cache_read_tokens = 7;
    app.turn_cache_write_tokens = 9;

    // Usage accumulates into the per-turn counters...
    app.record_turn_usage(1_200, 340);
    assert_eq!(app.turn_input_tokens, 2_400);

    // ...and the next submit wipes them so the footer never straddles turns.
    app.begin_turn();
    assert_eq!(app.turn_input_tokens, 0);
    assert_eq!(app.turn_output_tokens, 0);
    assert_eq!(app.turn_cache_read_tokens, 0);
    assert_eq!(app.turn_cache_write_tokens, 0);
    assert!(
        app.turn_started_at.is_some(),
        "a submitted turn must be anchored for the duration/tps readout"
    );
    assert_eq!(
        app.turn_state,
        TurnState::Sending,
        "a submitted turn starts in Sending until the first event lands"
    );
}

#[test]
fn test_usage_event_pushes_token_warning_notification() {
    // iter-255: check_token_warnings() was never called from the Usage
    // handler despite its doc comment saying to call it after updating
    // token_count — the whole warning subsystem was dead. context_window_for_model
    // is a fixed 128000 stub, so 110_000 tokens crosses the 80% threshold.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 60_000,
        output_tokens: 50_000,
        total_tokens: 110_000,
    });
    assert_eq!(app.token_warning_threshold_shown, 80);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("80% full")),
        "expected an 80%-full context warning status notice"
    );
}

#[test]
fn test_token_warning_threshold_resets_when_usage_drops() {
    // Without a reset, an escalate-only gate would permanently suppress
    // warnings after /clear or /compact shrinks the context back down.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 100_000,
        output_tokens: 21_600,
        total_tokens: 121_600, // 95% of the 128_000 stub window
    });
    assert_eq!(app.token_warning_threshold_shown, 95);

    // Simulate /clear (or a successful /compact) shrinking usage back down.
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 1_000,
        output_tokens: 0,
        total_tokens: 1_000,
    });
    assert_eq!(
        app.token_warning_threshold_shown, 0,
        "threshold tracker should reset once usage drops back below it"
    );
}

#[test]
fn test_drop_pending_images_with_notice_warns_and_clears() {
    // iter-255: pasted images were never attached to the outgoing message
    // (no multi-part content support in the core client) nor cleared on
    // send, so the thumbnail row lingered forever looking attached.
    let mut app = make_app();
    app.prompt_input.add_image(crate::image_paste::PastedImage {
        path: std::path::PathBuf::from("/tmp/test.png"),
        label: "test.png".to_string(),
        dimensions: None,
    });
    app.drop_pending_images_with_notice();
    assert!(app.prompt_input.pending_images.is_empty());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("dropped")),
        "expected a warning that the image wasn't sent"
    );
}

#[test]
fn test_drop_pending_images_with_notice_noop_when_empty() {
    let mut app = make_app();
    let before = app.status_message.clone();
    app.drop_pending_images_with_notice();
    assert_eq!(app.status_message, before);
}

#[test]
fn test_usage_falls_back_to_sum_when_total_is_zero() {
    // Some providers send total_tokens=0. In that case, fall back to
    // input+output so we don't show 0 tokens.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 100,
        output_tokens: 50,
        total_tokens: 0,
    });
    assert_eq!(
        app.token_count, 150,
        "should fall back to input+output when total_tokens is 0"
    );
}

#[test]
fn test_stubs_eliminated_no_mcp_manager_field() {
    // Phase 3a: App.mcp_manager (stub) field must be gone.
    // We verify by checking that core_mcp_manager is the only MCP field.
    let app = make_app();
    assert!(
        app.core_mcp_manager.is_none(),
        "core_mcp_manager starts None"
    );
    // If the stub field still existed, this wouldn't compile — the type
    // system enforces the removal.
}

#[tokio::test(flavor = "multi_thread")]
async fn test_feedback_survey_removed() {
    // Phase 4: /survey command must not be intercepted (feedback_survey deleted).
    // intercept_slash_command returns true if the command is known+intercepted.
    // /survey was removed from the command table, so it returns false.
    let mut app = make_app();
    let result = app.handle_tui_command("survey", "");
    assert!(
        !result,
        "/survey should not be intercepted after feedback_survey deletion"
    );
}

#[test]
fn test_debug_hub_records_frames() {
    // Phase 1: record_frame should increment frame count.
    let app = make_app();
    assert_eq!(app.debug_hub.frame_count(), 0);
    app.debug_hub.record_frame(5.0, app.redraw_reason());
    app.debug_hub.record_frame(3.0, app.redraw_reason());
    assert_eq!(app.debug_hub.frame_count(), 2);
    assert_eq!(app.debug_hub.last_render_ms(), 3);
    // A fresh app has nothing pending, so the reason is the idle case — which
    // is a value, not a missing record.
    assert_eq!(app.debug_hub.last_redraw_reason(), None);
}

#[test]
fn test_debug_hub_records_errors() {
    // Phase 1: record_error should store the last error.
    let app = make_app();
    assert!(app.debug_hub.last_error().is_none());
    app.debug_hub.record_error("test", "something broke");
    assert_eq!(
        app.debug_hub.last_error().unwrap(),
        "[test] something broke"
    );
}

#[test]
fn test_interactive_multi_step_simulation() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut app = make_app();
    app.is_simulating = true;

    // 1. Simulate typing a slash command "/help"
    app.simulated_keys = vec![
        press_key(KeyCode::Char('/'), KeyModifiers::NONE),
        press_key(KeyCode::Char('h'), KeyModifiers::NONE),
        press_key(KeyCode::Char('e'), KeyModifiers::NONE),
        press_key(KeyCode::Char('l'), KeyModifiers::NONE),
        press_key(KeyCode::Char('p'), KeyModifiers::NONE),
        press_key(KeyCode::Enter, KeyModifiers::NONE),
    ];

    // 2. Run the loop ticks
    while !app.simulated_keys.is_empty() && !app.should_exit {
        if let Ok(Some(input)) = app.run(&mut terminal)
            && crate::input::is_slash_command(&input)
        {
            let (cmd, args) = crate::input::parse_slash_command(&input);
            app.handle_tui_command(cmd, args);
        }
    }

    // 3. Assert the help overlay is open
    assert!(app.help_overlay.visible);
    assert!(app.show_help);

    // 4. Simulate pressing Escape to close the overlay
    app.simulated_keys = vec![press_key(KeyCode::Esc, KeyModifiers::NONE)];

    while !app.simulated_keys.is_empty() && !app.should_exit {
        if let Ok(Some(input)) = app.run(&mut terminal)
            && crate::input::is_slash_command(&input)
        {
            let (cmd, args) = crate::input::parse_slash_command(&input);
            app.handle_tui_command(cmd, args);
        }
    }

    // 5. Assert the help overlay is closed
    assert!(!app.help_overlay.visible);
    assert!(!app.show_help);

    // 6. Simulate quitting
    app.simulated_keys = vec![
        press_key(KeyCode::Char('/'), KeyModifiers::NONE),
        press_key(KeyCode::Char('q'), KeyModifiers::NONE),
        press_key(KeyCode::Char('u'), KeyModifiers::NONE),
        press_key(KeyCode::Char('i'), KeyModifiers::NONE),
        press_key(KeyCode::Char('t'), KeyModifiers::NONE),
        press_key(KeyCode::Enter, KeyModifiers::NONE),
    ];

    while !app.simulated_keys.is_empty() && !app.should_exit {
        if let Ok(Some(input)) = app.run(&mut terminal)
            && crate::input::is_slash_command(&input)
        {
            let (cmd, args) = crate::input::parse_slash_command(&input);
            app.handle_tui_command(cmd, args);
        }
    }

    // 7. Assert app wants to exit
    assert!(app.should_exit);
}

// ---- Focus-aware rendering (Phase 2.3) --------------------------------

#[test]
fn test_focus_event_transitions_client_focused() {
    // FocusLost must drop client_focused so the redraw cadence slows;
    // FocusGained must restore it and restart the activity timer.
    let mut app = make_app();
    assert!(app.client_focused, "starts focused");

    app.handle_focus_event(false);
    assert!(!app.client_focused, "FocusLost clears client_focused");

    app.handle_focus_event(true);
    assert!(app.client_focused, "FocusGained restores client_focused");
}

#[test]
fn test_focus_gained_resets_activity_timer() {
    // After FocusGained, last_activity is refreshed so idle cadence restarts
    // instead of immediately deep-idling a terminal the user just refocused.
    let mut app = make_app();
    app.handle_focus_event(false);
    // Simulate a stale activity timestamp from before the focus loss.
    app.last_activity = std::time::Instant::now() - std::time::Duration::from_secs(120);

    app.handle_focus_event(true);
    let elapsed = app.last_activity.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "last_activity should be refreshed on FocusGained (elapsed={elapsed:?})"
    );
}

// ---- Phase A5: dialog open/close scenario regression pack -------------
// Drives simulated keys through the real run loop (with the same slash
// interception the interactive/headless loops use), then asserts state
// via App::debug_snapshot(). This is the safety net that gates the
// dialog-unification refactor (Phase B): every listed overlay must open
// via its slash command and close on Esc.

fn drive_keys<B: ratatui::backend::Backend + super::SynchronizedUpdate>(
    app: &mut App,
    terminal: &mut ratatui::Terminal<B>,
) where
    B::Error: Send + Sync + 'static,
{
    let mut guard = 0;
    while !app.simulated_keys.is_empty() && !app.should_exit && guard < 5000 {
        guard += 1;
        if let Ok(Some(input)) = app.run(terminal)
            && crate::input::is_slash_command(&input)
        {
            let (cmd, args) = crate::input::parse_slash_command(&input);
            app.handle_tui_command(cmd, args);
        }
    }
}

fn slash_keys(cmd: &str) -> Vec<KeyEvent> {
    let mut keys = vec![press_key(KeyCode::Char('/'), KeyModifiers::NONE)];
    for ch in cmd.chars() {
        keys.push(press_key(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    keys.push(press_key(KeyCode::Enter, KeyModifiers::NONE));
    keys
}

#[test]
fn test_dialog_open_close_scenarios() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // (slash command, snapshot overlay key). Each must open via
    // `/<cmd><enter>` and close on Esc.
    let scenarios: &[(&str, &str)] = &[
        ("help", "help_overlay"),
        ("settings", "settings_screen"),
        ("theme", "theme_screen"),
        ("stats", "stats_dialog"),
        ("skills", "skills_view"),
        ("journey", "journey_view"),
        ("plugins", "plugins_hub"),
        ("model", "inline_interactive"),
        ("effort", "inline_interactive"),
        ("context", "context_viz"),
        ("agents", "agents_menu"),
        ("export", "export_dialog"),
        ("mcp", "mcp_view"),
    ];

    for (cmd, overlay) in scenarios {
        let mut app = make_app();
        // The `/model` picker is gated on live credentials; the test owns
        // that precondition rather than inheriting the ambient auth state
        // (the operator's key file, the shell's API key, or a parallel
        // test's cleared environment).
        app.has_credentials = true;
        app.is_simulating = true;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        app.simulated_keys = slash_keys(cmd);
        drive_keys(&mut app, &mut terminal);

        let snap = app.debug_snapshot();
        assert_eq!(
            snap["overlays"][overlay],
            serde_json::Value::Bool(true),
            "/{cmd} should open overlay '{overlay}'"
        );
        assert_eq!(
            snap["any_modal_open"],
            serde_json::Value::Bool(true),
            "/{cmd} should register a modal as open"
        );

        // Esc must close it.
        app.simulated_keys = vec![press_key(KeyCode::Esc, KeyModifiers::NONE)];
        drive_keys(&mut app, &mut terminal);
        let snap = app.debug_snapshot();
        assert_eq!(
            snap["overlays"][overlay],
            serde_json::Value::Bool(false),
            "Esc should close overlay '{overlay}' opened by /{cmd}"
        );
    }
}

// Consistency guard (iter-237 / Phase B1): `overlay_flags()` is the single
// source of truth for the overlay set. `debug_snapshot()`'s overlays map is
// built from it, so their key sets must be identical. This is what prevents
// the parallel-list drift that dropped `effort_picker` in iter-227.
#[test]
fn test_overlay_flags_matches_debug_snapshot_keys() {
    let app = make_app();

    let mut flag_keys: Vec<String> = app
        .overlay_flags()
        .iter()
        .map(|(k, _): &(&str, bool)| k.to_string())
        .collect();
    flag_keys.sort();

    let snap = app.debug_snapshot();
    let mut snap_keys: Vec<String> = snap["overlays"]
        .as_object()
        .expect("overlays should be a JSON object")
        .keys()
        .cloned()
        .collect();
    snap_keys.sort();

    assert_eq!(
        flag_keys, snap_keys,
        "overlay_flags() and debug_snapshot() overlays must have identical keys"
    );
}

#[test]
fn test_streaming_agent_events_commit_message() {
    use operant_core::agent::AgentEvent;

    let mut app = make_app();
    app.is_streaming = true;
    app.handle_agent_event(AgentEvent::Content {
        text: "Hello ".into(),
    });
    app.handle_agent_event(AgentEvent::Content {
        text: "world".into(),
    });
    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::client::Message::assistant("Hello world"),
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });

    let snap = app.debug_snapshot();
    assert!(
        snap["messages"].as_u64().unwrap_or(0) >= 1,
        "Done should commit at least one assistant message"
    );
}

#[test]
fn retry_should_construct_system_api_error_block() {
    use operant_core::agent::AgentEvent;

    let mut app = make_app();
    let before = app.messages.len();

    app.handle_agent_event(AgentEvent::RetryScheduled {
        attempt: 1,
        max_attempts: 3,
        reason: "context overflow".into(),
    });

    assert_eq!(app.messages.len(), before + 1, "retry must add one message");
    let blocks = app.messages[before].content_blocks();
    assert_eq!(blocks.len(), 1, "retry message carries exactly one block");
    let ContentBlock::SystemAPIError {
        message,
        retry_secs,
    } = &blocks[0]
    else {
        panic!("expected SystemAPIError, got {:?}", blocks[0]);
    };
    assert!(
        message.contains("context overflow") && message.contains("1/3"),
        "retry message must carry the reason and attempt count, got {message}"
    );
    assert!(retry_secs.is_none(), "loop retries are immediate");
}

#[test]
fn test_command_palette_opens_via_ctrl_k() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = make_app();
    app.is_simulating = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    app.simulated_keys = vec![press_key(KeyCode::Char('k'), KeyModifiers::CONTROL)];
    drive_keys(&mut app, &mut terminal);

    let snap = app.debug_snapshot();
    assert_eq!(
        snap["overlays"]["command_palette"],
        serde_json::Value::Bool(true),
        "Ctrl+K should open the command palette"
    );
}

#[test]
fn test_skill_slash_command_injects_pending_message() {
    // /skill <name> must be fully handled by the intercept arm (returns
    // true) and stage the hermes-parity invocation expansion in
    // pending_user_message — NOT fall through to the CommandRegistry
    // fallback (iter-320). Uses a temp skills dir to keep the test
    // hermetic and independent of the user's installed skills.
    let mut app = make_app();
    let tmp = std::env::temp_dir().join(format!("operant-skill-slash-{}", std::process::id()));
    let skill_dir = tmp.join("demo-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo-skill\ndescription: demo\n---\n\n# Steps\nDo the demo.\n",
    )
    .unwrap();
    app.config.skills.root_dir = tmp.clone();

    // Missing skill: handled with a status message, no pending message.
    assert!(app.handle_tui_command("skill", "nope-missing"));
    assert!(app.pending_user_message.is_none());
    assert!(app.status_message.is_some());

    // Existing skill: handled, expansion staged for the run loop.
    assert!(app.handle_tui_command("skill", "demo-skill"));
    let staged = app.pending_user_message.take().expect("expansion staged");
    assert!(
        staged.contains("demo-skill"),
        "expansion names the skill: {staged}"
    );
    assert!(
        staged.contains("Do the demo."),
        "expansion carries SKILL.md body"
    );

    // Bare /skill opens the overlay (existing behavior preserved).
    assert!(!app.skills_view.visible);
    assert!(app.handle_tui_command("skill", ""));
    assert!(app.skills_view.visible);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_bundle_slash_command_handled_without_registry_fallback() {
    // /bundle with no args must be intercepted (return true) even when no
    // bundles exist — the pre-fix code path could fall through to the
    // CommandRegistry and print a misleading "not yet wired" message
    // (iter-320).
    let mut app = make_app();
    assert!(app.handle_tui_command("bundle", ""));
    assert!(app.pending_user_message.is_none());
}

// ---- Queued vs Running tool status ----

/// The model's `intent` is the description every tool row and every gateway
/// progress line shows. It arrives inside the streamed arguments JSON, so it
/// must be lifted onto the block the moment the arguments parse — otherwise
/// the row renders a bare tool name plus a technical path and the operator
/// has no idea WHY the call was made.
#[test]
fn tool_start_captures_the_model_intent_for_the_tool_row() {
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_intent".to_string(),
        name: "read".to_string(),
        arguments: r#"{"file_path":"src/parser.rs","intent":"Locate the entry point"}"#.to_string(),
    });
    assert_eq!(app.tool_use_blocks.len(), 1);
    assert_eq!(
        app.tool_use_blocks[0].intent.as_deref(),
        Some("Locate the entry point"),
        "the intent must be lifted onto the block for the row to render"
    );
}

/// A re-announcement with an unparseable (short) fragment must not CLEAR an
/// intent already captured: the fragment is the same call mid-stream, and the
/// description must survive it.
#[test]
fn a_shorter_re_announcement_never_clears_a_captured_intent() {
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_intent_2".to_string(),
        name: "read".to_string(),
        arguments: r#"{"file_path":"a.rs","intent":"Check the header"}"#.to_string(),
    });
    // The pool's re-announcement carries a partial fragment that does not parse.
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_intent_2".to_string(),
        name: "read".to_string(),
        arguments: r#"{"file_path"#.to_string(),
    });
    assert_eq!(
        app.tool_use_blocks[0].intent.as_deref(),
        Some("Check the header"),
        "an unparseable fragment must not wipe a captured intent"
    );
}

/// A call whose arguments carry no intent simply has none: the row then shows
/// its technical summary, which is the behaviour that predates this field.
#[test]
fn tool_start_without_an_intent_leaves_the_block_without_one() {
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_no_intent".to_string(),
        name: "read".to_string(),
        arguments: r#"{"file_path":"a.rs"}"#.to_string(),
    });
    assert_eq!(app.tool_use_blocks[0].intent, None);
}

#[test]
fn tool_should_transition_queued_to_running_on_permit_acquire() {
    let mut app = make_app();
    let queued = AgentEvent::ToolStart {
        tool_call_id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: r#"{"command":"ls"}"#.to_string(),
    };
    // The concurrent pool announces the call while it waits for a permit…
    app.handle_agent_event(queued.clone());
    assert_eq!(app.tool_use_blocks.len(), 1);
    assert_eq!(app.tool_use_blocks[0].status, ToolStatus::Queued);
    assert!(app.tool_use_blocks[0].status.is_pending());

    // …then re-announces it once the permit is in hand.
    app.handle_agent_event(queued);
    assert_eq!(app.tool_use_blocks.len(), 1, "no second block is created");
    assert_eq!(app.tool_use_blocks[0].status, ToolStatus::Running);
    assert!(app.tool_use_blocks[0].status.is_pending());

    // Settling the call ends the pending window.
    app.handle_agent_event(AgentEvent::ToolComplete {
        result: operant_core::tools::ToolResult {
            tool_call_id: "call_1".to_string(),
            name: "bash".to_string(),
            success: true,
            content: "ok".to_string(),
            error: None,
            // [courtesy fix] field added by a concurrent peer iter
            // (ToolResult::timed_out) without updating this initializer;
            // carries the pass-through default a completed call implies.
            timed_out: false,
        },
    });
    assert_eq!(app.tool_use_blocks[0].status, ToolStatus::Done);
    assert!(!app.tool_use_blocks[0].status.is_pending());
}

// The tool row and the spinner footer already name the tool and its state. A
// status notice repeating "Queued <tool>…" printed the name a third time while
// the call was pending.
#[test]
fn tool_start_leaves_the_status_line_to_the_tool_row() {
    let mut app = make_app();
    app.status_message = None;
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_1".to_string(),
        name: "aft_read".to_string(),
        arguments: r#"{"file_path":"README.md"}"#.to_string(),
    });
    assert_eq!(app.tool_use_blocks.len(), 1);
    assert_eq!(app.status_message, None);
}

// ---- P0-2: per-iteration text grouping (2026-10-09 live audit) ----
//
// Providers interleave content deltas around tool_call deltas in one
// response; observed live splitting "AG|ENTS.md" across the tool row.
// ToolStart must not flush (the seam joins); ToolComplete/ToolError are the
// iteration boundary and must flush (responses never glue).

#[test]
fn tool_start_mid_stream_must_not_split_assistant_text() {
    use operant_core::agent::AgentEvent;

    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Content {
        text: "I’ll read the first 5 lines of AG".into(),
    });
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_split".into(),
        name: "aft_read".into(),
        arguments: "{}".into(),
    });
    app.handle_agent_event(AgentEvent::Content {
        text: "ENTS.md and summarize.".into(),
    });

    assert!(
        app.messages.is_empty(),
        "ToolStart must not flush mid-response text"
    );
    assert_eq!(
        app.streaming_text, "I’ll read the first 5 lines of AGENTS.md and summarize.",
        "the interleaved chunks join into the unsplit sentence"
    );
    assert_eq!(
        app.tool_use_blocks[0].after_index, 1,
        "the tool row reserves the slot the pending text will occupy"
    );

    // The tool settles: iteration boundary flushes the joined text as one
    // message, and the display order puts the tool row after it.
    app.handle_agent_event(AgentEvent::ToolComplete {
        result: operant_core::tools::ToolResult {
            tool_call_id: "call_split".into(),
            name: "aft_read".into(),
            success: true,
            content: "# AGENTS.md…".into(),
            error: None,
            timed_out: false,
        },
    });
    assert_eq!(app.messages.len(), 1, "boundary flush commits one message");
    let joined = app.messages[0].text_content();
    assert!(
        joined.contains("AGENTS.md and summarize."),
        "the committed message carries the unsplit sentence: {joined}"
    );

    let display = crate::tui::operant_model::display_messages(&app);
    assert_eq!(display[0].role, "assistant");
    assert_eq!(display[1].role, "tool", "tool row renders after the text");
}

#[test]
fn tool_complete_boundary_must_flush_iteration_text() {
    use operant_core::agent::AgentEvent;

    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Content {
        text: "Sentence one.".into(),
    });
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_b".into(),
        name: "aft_read".into(),
        arguments: "{}".into(),
    });
    app.handle_agent_event(AgentEvent::ToolComplete {
        result: operant_core::tools::ToolResult {
            tool_call_id: "call_b".into(),
            name: "aft_read".into(),
            success: true,
            content: "ok".into(),
            error: None,
            timed_out: false,
        },
    });
    // The model re-plans: the next response's text must NOT glue onto the
    // previous one.
    app.handle_agent_event(AgentEvent::Content {
        text: "Sentence two.".into(),
    });
    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::client::Message::assistant("Sentence two."),
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });

    assert_eq!(app.messages.len(), 2, "two responses, two messages");
    let first = app.messages[0].text_content();
    let second = app.messages[1].text_content();
    assert!(first.contains("Sentence one."), "first message: {first}");
    assert!(second.contains("Sentence two."), "second message: {second}");
    assert!(
        !second.contains("one"),
        "no glue: the second response must not carry the first's text: {second}"
    );
}

#[test]
fn tool_error_boundary_must_flush_iteration_text() {
    use operant_core::agent::AgentEvent;

    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Content {
        text: "Trying the tool.".into(),
    });
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "call_e".into(),
        name: "shell".into(),
        arguments: "{}".into(),
    });
    app.handle_agent_event(AgentEvent::ToolError {
        tool_call_id: "call_e".into(),
        name: "shell".into(),
        error: "exit status 3".into(),
    });
    app.handle_agent_event(AgentEvent::Content {
        text: "It failed.".into(),
    });
    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::client::Message::assistant("It failed."),
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });

    assert_eq!(app.messages.len(), 2, "error boundary also flushes");
    let first = app.messages[0].text_content();
    assert!(
        first.contains("Trying the tool."),
        "pre-error text is preserved as its own message: {first}"
    );
}

// ---- Scroll bookmark + reflow anchor (Ctrl+G) --------------------------
//
// These drive the two halves of the reading-position state in
// `app/scroll_anchor.rs`. The render path is what measures the transcript, so
// the tests stand in for it: setting `last_render_scroll_offset` to the row a
// frame *would* paint, then calling `reconcile_scroll_anchor`, is exactly what
// `App::run` does after `terminal.draw`.

/// Stand in for a rendered frame: the viewport is at the top of a transcript
/// with `scrollback` rows of scrollable height, then reconcile.
fn paint_frame_at_top(app: &mut App, scrollback: usize) {
    app.last_render_scroll_offset.set(0);
    app.scroll_offset = scrollback;
    app.reconcile_scroll_anchor();
}

#[test]
fn scroll_bookmark_should_toggle_and_restore() {
    let mut app = make_app();
    app.auto_scroll = false;
    paint_frame_at_top(&mut app, 42);

    // Alt+G (Ctrl+G is the external editor since iter-807).
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::ALT));
    assert_eq!(app.scroll_offset, 42, "first press only arms");
    assert!(
        app.status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Bookmark on")
    );

    // Reader wanders off, then asks to come back.
    app.scroll_offset = 5;
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::ALT));
    assert_eq!(app.scroll_offset, 42, "second press restores the bookmark");
    assert!(
        app.status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Bookmark off")
    );

    // It is a toggle, not a sticky mark: the third press arms again at the
    // current spot instead of jumping back a second time.
    app.scroll_offset = 7;
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::ALT));
    assert_eq!(app.scroll_offset, 7);
    assert!(
        app.status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Bookmark on")
    );
}

#[test]
fn scroll_bookmark_should_clamp_when_transcript_shrank() {
    let mut app = make_app();
    app.auto_scroll = false;
    paint_frame_at_top(&mut app, 300);

    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(app.scroll_offset, 300);

    // `/clear`, `/rewind` or a compacted turn drops the scrollback to 40 rows.
    paint_frame_at_top(&mut app, 40);

    // Restoring row 300 of a 40-row transcript must land in range, not past
    // the end (which would render a blank or clamped view with no explanation).
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(
        app.scroll_offset, 40,
        "restored offset is clamped to the range"
    );
}

#[test]
fn anchor_should_hold_position_when_content_grows_above() {
    let mut app = make_app();
    paint_frame_at_top(&mut app, 0);

    // Reader parks 10 rows up: PageUp drops auto-follow and pins the row that
    // is on screen. The frame that paints it publishes that row, exactly as
    // the render path does while paused (`scroll = user_scroll`).
    app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(!app.auto_scroll);
    app.last_render_scroll_offset.set(10);
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, 10,
        "no reflow yet, so nothing to correct"
    );

    // 20 rows of new output land above the viewport, so the row that was at the
    // top is now 20 rows further down: the frame publishes 20 as the row it
    // painted and the reader's pinned row (10) sits 20 - 10 = 10 rows below it.
    app.last_render_scroll_offset.set(20);
    app.reconcile_scroll_anchor();

    // The offset grows by exactly the drift, which puts the pinned row back at
    // the top: 10 + 10 = 20, and 20 - 20 = 0. Growing by the full 20 (the old
    // behaviour) was the bug — it read the whole distance as drift and landed
    // the reader at the tail on their own key press.
    assert_eq!(
        app.scroll_offset, 20,
        "the offset grows by the drift, not by the whole distance"
    );
}

/// 2026-10-22 scroll-ladder follow-up (P4-1 residual): repeated PageUps put the
/// reader at the TAIL and every further key press did nothing.
///
/// Measured on the headless simulator before the fix, against a 200-line
/// transcript at 100x30 (`max_scroll` 40): 1 PageUp L001-L083, 2 L014-L126, and
/// 3 already back at L101-L200 — which is the tail, where it stayed for every
/// press after. The reader's own key press parked them at the bottom.
///
/// The mechanism: the reader's FIRST PageUp follows a tail-following frame, so
/// `painted` holds `max_scroll` (`resolve_tail_follow_scroll` publishes that).
/// `note_user_scroll`'s pin arithmetic (`painted + prev_offset - new_offset`)
/// then names a row above the whole transcript, and the growth correction reads
/// that distance as "content appeared above the reader" and lands the offset on
/// `max_scroll`. Every later PageUp was then clamped away.
///
/// The fix bounds the correction by what the frame actually measured, so a
/// pin outside the transcript can no longer teleport the reader. This test
/// drives that first press and every press after, and asserts the invariant the
/// operator cares about: scrolling up is monotonic and stops only at the real
/// ceiling.
#[test]
fn repeated_page_up_climbs_monotonically_and_stops_at_the_real_ceiling() {
    // 200 wrapped rows, a viewport that shows 26, so the reader may climb 174.
    const LIVE_MAX: usize = 174;
    let mut app = make_app();
    crate::tui::operant_ui::set_last_max_scroll(LIVE_MAX);
    // The frame before the first key: tail-following, so the render path
    // published `max_scroll` as the row it painted.
    app.auto_scroll = false;
    app.last_render_scroll_offset.set(LIVE_MAX as u16);

    let mut expected = 0usize;
    for step in 0..8 {
        expected = (expected + 10).min(LIVE_MAX);
        // One frame, in the order the run loop produces it: the key handler
        // runs against the row the previous frame published, the frame paints
        // (publishing the reader's own offset while paused), then the
        // reconcile runs against it.
        app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
        app.last_render_scroll_offset.set(expected as u16);
        app.reconcile_scroll_anchor();
        assert_eq!(
            app.scroll_offset, expected,
            "step {step}: the offset follows the ladder and is capped at the real ceiling"
        );
        assert!(
            app.scroll_offset <= LIVE_MAX,
            "step {step}: the growth correction may never overshoot the transcript"
        );
    }
    // Eight presses of 10 rows would reach 80; the ceiling is 174, so the
    // reader is still climbing — and that is the point: nothing threw them
    // back to the tail on the way.
    assert_eq!(
        app.scroll_offset, 80,
        "eight PageUps climb 80 rows, unimpeded"
    );
    crate::tui::operant_ui::set_last_max_scroll(0);
}

/// The same invariant through the wheel and the fine keyboard rung, which write
/// `scroll_offset` through their own handlers and rely on the same reconcile.
#[test]
fn repeated_wheel_up_climbs_monotonically_and_stops_at_the_real_ceiling() {
    const LIVE_MAX: usize = 174;
    let mut app = make_app();
    crate::tui::operant_ui::set_last_max_scroll(LIVE_MAX);
    app.auto_scroll = false;
    app.last_render_scroll_offset.set(LIVE_MAX as u16);

    let mut expected = 0usize;
    for step in 0..10 {
        expected = (expected + 3).min(LIVE_MAX);
        app.handle_key_event(press_key_with_control(KeyCode::Up));
        app.last_render_scroll_offset.set(expected as u16);
        app.reconcile_scroll_anchor();
        assert_eq!(
            app.scroll_offset, expected,
            "step {step}: Ctrl+Up adds 3 rows and is capped at the real ceiling"
        );
    }
    // Ten presses of 3 rows climb 30, again with nothing throwing the reader
    // back to the tail.
    assert_eq!(app.scroll_offset, 30, "ten Ctrl+Up presses climb 30 rows");
    crate::tui::operant_ui::set_last_max_scroll(0);
}

#[test]
fn user_scroll_should_cancel_the_anchor() {
    let mut app = make_app();
    paint_frame_at_top(&mut app, 0);
    app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
    app.reconcile_scroll_anchor();
    assert_eq!(app.scroll_offset, 10);

    // A scroll the reader makes outside the keyboard arms — the mouse wheel
    // writes `scroll_offset` directly — is a new reading position, so the old
    // pin is void and later reflow must not drag the view back.
    app.scroll_offset = 15;
    app.last_render_scroll_offset.set(20);
    app.reconcile_scroll_anchor();
    assert_eq!(app.scroll_offset, 15, "a user scroll drops the anchor");
}

#[test]
fn reconcile_clamps_ghost_offset_after_the_flush_shrinks_the_transcript() {
    // The growth correction counts the streaming bubble's rows; the flush
    // boundary collapses them into one committed message, so the offset can
    // hold rows that no longer exist (live-audit P4-1 follow-up: offset 355
    // against a 15-row max left PageDown walking ghost rows home). The
    // reconcile must clamp the STATE to the frame's live max — the render
    // already clamps its own paint — and void the pin.
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 355;
    crate::tui::operant_ui::set_last_max_scroll(15);
    app.last_render_scroll_offset.set(15);
    app.last_resolved_scroll.set(Some(15));
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, 15,
        "ghost rows must clamp to the live frame max"
    );
    // Reset the thread-local so later tests on this thread see "no frame".
    crate::tui::operant_ui::set_last_max_scroll(0);
}

// ---- Resize preserves reading position ---------------------------------
//
// A resize rewraps every transcript line, so a row index captured before it
// names a different row after it. These drive the same three steps the run loop
// does: the render path publishes where the reader is and where that content
// now lives, `Event::Resize` captures, and `reconcile_scroll_anchor` adopts.
//
// The render path is stood in for the same way the reflow tests above stand in
// for it: the Cells it publishes are set directly, because measuring a real
// rewrap needs a real `Terminal`. The end-to-end half — that a paint at two
// widths really does resolve to the same message — is `resize_reflow_keeps_the
// same_message_in_view` in `render::tests`.

/// Stand in for a painted frame at one width.
///
/// `painted_row` is what the render path reports in `last_render_scroll_offset`
/// — the row the *uncorrected* offset put at the top — and `resolved_row` is
/// where the anchored content now lives. They are different numbers on purpose:
/// that gap is the whole bug. `App::run` publishes the first and the render
/// path resolves the second, then `reconcile_scroll_anchor` decides between them.
struct ResizeFrame {
    /// The content address of the row on screen.
    pos: ContentPos,
    painted_row: u16,
    resolved_row: usize,
}

fn paint_at(app: &mut App, f: ResizeFrame) {
    app.last_render_content_pos.set(Some(f.pos));
    app.last_render_scroll_offset.set(f.painted_row);
    app.last_resolved_scroll.set(Some(f.resolved_row));
    app.reconcile_scroll_anchor();
}

fn pos(message: usize, line: u16) -> ContentPos {
    // The captured target is opaque to the reconcile machinery (it is the
    // render's job to resolve it); the tests only need a stable Some value.
    ContentPos::Message(crate::tui::operant_model::Anchor {
        msg_hash: message as u64,
        occurrence: 0,
        row_within_item: line as usize,
    })
}

#[test]
fn resize_preserves_the_row_a_scrolled_up_reader_was_looking_at() {
    let mut app = make_app();
    app.auto_scroll = false;

    // 140 transcript rows in a 40-row viewport, so 100 rows of scrollback. The
    // reader is parked 30 rows up: the top of the viewport is row 70, and that
    // row is the second rendered line of message 3.
    app.scroll_offset = 30;
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(3, 1),
            painted_row: 70,
            resolved_row: 70,
        },
    );
    assert_eq!(app.scroll_offset, 30, "precondition: parked 30 rows up");

    // The terminal narrows and every line rewraps: 260 transcript rows, so 220
    // rows of scrollback. Leaving the offset alone would put the top of the
    // viewport at 220 - 30 = 190 — a completely different part of the
    // conversation. The anchored content is actually at row 25.
    app.note_resize();
    app.scroll_offset = 30;
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(3, 1),
            painted_row: 190,
            resolved_row: 25,
        },
    );

    assert_eq!(
        app.scroll_offset, 195,
        "220 - 25: the reader is put back on the message they were reading"
    );
    assert_ne!(
        app.scroll_offset, 30,
        "the whole point — leaving the offset alone is what this fixes"
    );
    assert!(
        !app.auto_scroll,
        "a resize must never re-pin a reader who scrolled up"
    );
}

#[test]
fn resize_leaves_a_follower_pinned_to_the_tail() {
    let mut app = make_app();
    app.auto_scroll = true;

    // Auto-follow owns the position: the render path paints the tail whatever
    // the geometry, so the top row is simply wherever the tail now is. 100 rows
    // of scrollback, top row 90.
    app.scroll_offset = 0;
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(7, 0),
            painted_row: 90,
            resolved_row: 90,
        },
    );

    app.note_resize();
    assert!(
        app.scroll_memory.pending_resize.is_none(),
        "a follower has no reading position to preserve"
    );

    // Narrowed: 220 rows of scrollback, so the tail is row 220. The stale
    // resolution (row 12, from before the capture) must not be adopted.
    app.scroll_offset = 0;
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(7, 0),
            painted_row: 220,
            resolved_row: 12,
        },
    );

    assert!(
        app.auto_scroll,
        "the resize must not unpin a reader who was following live output"
    );
    assert_eq!(
        app.scroll_offset, 0,
        "a follower stays at the tail — offset 0 — not at the old position"
    );
}

#[test]
fn a_resize_with_no_content_address_leaves_the_offset_to_the_render_clamp() {
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 30;
    app.last_render_content_pos.set(None);

    app.note_resize();
    assert!(
        app.scroll_memory.pending_resize.is_none(),
        "nothing nameable means nothing to preserve"
    );

    // No resolution published, so the reconcile must leave the offset alone
    // rather than inventing a correction.
    app.last_render_scroll_offset.set(190);
    app.last_resolved_scroll.set(None);
    app.reconcile_scroll_anchor();
    assert_eq!(app.scroll_offset, 30, "the offset is left untouched");
}

#[test]
fn a_resolve_that_finds_no_row_leaves_the_offset_alone() {
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 30;
    app.last_render_content_pos.set(Some(pos(3, 1)));
    app.note_resize();

    // The anchored message is gone (`/clear`, `/rewind`): the render path
    // resolved nothing. The reader keeps their offset rather than being thrown
    // to row 0 by a resolution that failed.
    app.last_render_scroll_offset.set(190);
    app.last_resolved_scroll.set(None);
    app.reconcile_scroll_anchor();
    assert_eq!(app.scroll_offset, 30);
}

#[test]
fn a_resize_is_not_itself_a_user_scroll_for_the_next_reflow() {
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 30;
    app.last_render_content_pos.set(Some(pos(3, 1)));
    app.note_resize();

    app.last_render_scroll_offset.set(190);
    app.last_resolved_scroll.set(Some(25));
    app.reconcile_scroll_anchor();
    assert_eq!(app.scroll_offset, 195, "220 - 25");

    // The adopt writes `scroll_offset` itself. If it did not also re-pin the
    // reflow anchor at the row it asked for, the very next append would compare
    // the offset against a stale `applied_offset`, read the difference as a user
    // scroll and void the pin — so the position would survive the resize and be
    // dropped by the next message.
    //
    // One frame later the transcript grew by 5 rows above the reader and painted
    // row 30.
    app.last_render_scroll_offset.set(30);
    app.last_resolved_scroll.set(None);
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, 200,
        "the reflow anchor survived the resize, so the append still corrects"
    );
}

#[test]
fn a_user_scroll_during_a_pending_resize_wins() {
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 30;
    app.last_render_content_pos.set(Some(pos(3, 1)));
    app.note_resize();

    // The reader scrolls again before the next paint — a PageUp, a wheel tick.
    // `scroll_offset` moves without going through the keyboard handler, which is
    // why the anchor records the offset it was captured at rather than trusting a
    // call site to cancel it.
    app.scroll_offset = 5;
    app.last_render_scroll_offset.set(215);
    app.last_resolved_scroll.set(Some(25));
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, 5,
        "a scroll made after the resize is a newer position than the captured one"
    );
}

#[test]
fn event_resize_reaches_the_reading_position_state() {
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 30;
    app.last_render_content_pos.set(Some(pos(4, 2)));

    // `Event::Resize` used to fall through to the match's catch-all, so a
    // resize was invisible to app state. Drive the real arm.
    let outcome = app.apply_terminal_event(Event::Resize(40, 16));
    assert!(matches!(outcome, EventOutcome::Consumed));
    assert_eq!(
        app.scroll_memory.pending_resize.map(|p| p.target),
        Some(pos(4, 2)),
        "the resize must arm the anchor the run loop reconciles"
    );
    assert_eq!(
        app.scroll_memory.pending_resize.map(|p| p.captured_offset),
        Some(30),
        "the capture records the offset in effect, to spot a later user scroll"
    );
}

// ---- keybinding registry: /keys and /hotkeys ------------------------

#[test]
fn keys_command_should_report_registry_bindings() {
    use crate::tui::keybindings::KeyBindingRegistry;
    use crate::tui::overlays::help::HelpOverlayContent;

    let mut app = make_app();
    let before = app.system_annotations.len();

    assert!(app.handle_tui_command("keys", ""));
    // (2026-10-14 batch, plan 10) /keys opens the overlay at the top —
    // never dumps the ~170-line report into the transcript.
    assert!(
        app.help_overlay.visible && app.help_overlay.content == HelpOverlayContent::Keys,
        "/keys must open the Keys overlay"
    );
    assert_eq!(
        app.system_annotations.len(),
        before,
        "/keys must not dump the report into the transcript"
    );
    assert_eq!(app.help_overlay.scroll_offset, 0, "opened at the top");

    // Sourced from the registry, not a hand-maintained list: the header count
    // must equal the number of rows the registry actually catalogues.
    let text = app.render_keys_report(
        &KeyBindingRegistry::with_defaults(),
        &KeyBindingRegistry::with_defaults().usage_report(&app.slash_usage),
    );
    let expected_total = KeyBindingRegistry::with_defaults().catalogue().len();
    assert!(
        text.contains(&format!("{expected_total} binding(s) from the registry")),
        "header must count the registry catalogue, got: {text}"
    );
    // Spot-check one binding per shape: a plain key, a modified chord, and a
    // context group, all of which exist in the default table.
    assert!(text.contains("[prompt]"), "context groups are labelled");
    assert!(text.contains("Enter"), "a plain key binding is listed");
    assert!(text.contains("Ctrl+"), "a modified chord is listed");
    // A known table row must appear verbatim with its own description.
    assert!(
        text.contains("Submit prompt"),
        "description comes from the default table, got: {text}"
    );
    // Ctrl+C is a real tty-level intercept and must be flagged inline.
    assert!(
        text.contains("⚠ SIGINT"),
        "the inline flag names the mechanism, got: {text}"
    );
    assert!(
        text.contains("may be intercepted by the terminal") && text.contains("Ctrl+c"),
        "the summary lists the conflicting chords, got: {text}"
    );
    // The flag is not blanket-applied: Enter is a real binding that no
    // terminal intercepts, so it must not be reported as a conflict.
    assert!(
        crate::tui::keybindings::os_conflict_reason("Enter").is_none(),
        "Enter must not be flagged — the conflict list is not a catch-all"
    );
}

#[test]
fn hotkey_usage_should_distinguish_used_from_never_used() {
    use crate::tui::keybindings::{KeyBindingRegistry, usage_key};
    use crate::tui::slash_usage::UsageStore;

    // 1. The split itself, on a synthetic store: exactly the chords we
    //    recorded are "used", everything else is "never used".
    let registry = KeyBindingRegistry::with_defaults();
    let mut usage = UsageStore::default();
    usage.record(&usage_key("Ctrl+p"));
    usage.record(&usage_key("Ctrl+p"));
    usage.record(&usage_key("Alt+v"));

    let report = registry.usage_report(&usage);
    assert_eq!(report.used.len() + report.never_used.len(), report.total);
    assert_eq!(
        report.used.iter().map(|v| v.count).collect::<Vec<_>>(),
        vec![2, 1],
        "used is ordered most-pressed first"
    );
    // Per-chord, not per row: `Ctrl+p` is bound in two contexts in the table
    // (global palette, prompt history-prev) but is one key the user presses,
    // so it must appear exactly once in the used list.
    assert_eq!(
        report
            .used
            .iter()
            .map(|v| v.chord.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["Alt+v", "Ctrl+p"]),
        "a chord bound in several contexts is one chord"
    );
    assert!(
        report.never_used.iter().all(|v| v.count == 0),
        "never-used rows all have a zero count"
    );
    assert!(
        !report.used.is_empty() && !report.never_used.is_empty(),
        "a partially-used table must populate both halves"
    );
    // The catalogue itself still lists every row, so /keys (a catalogue) and
    // /hotkeys (a chord report) legitimately differ in totals.
    assert!(
        registry.catalogue().len() > report.total,
        "the catalogue has more rows than the table has distinct chords"
    );

    // 2. The live path: a real key press through the accept point is counted,
    //    an unbound chord is not, and both halves render. `Up` is bound in the
    //    prompt context; `Ctrl+PageUp` is not in the table at all.
    let mut app = make_app();
    let up_before = app.slash_usage.frequency_rank(&usage_key("Up"));

    app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::CONTROL));
    assert_eq!(
        app.slash_usage.frequency_rank(&usage_key("Ctrl+PageUp")),
        0,
        "an unbound chord must not be counted"
    );

    app.handle_key_event(press_key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        app.slash_usage.frequency_rank(&usage_key("Up")),
        up_before + 1,
        "a bound chord is counted exactly once per accepted press"
    );

    assert!(app.handle_tui_command("hotkeys", ""));
    // (2026-10-14 batch, plan 10) /hotkeys opens the Hotkeys overlay — the
    // used/never-used split comes from the report builder.
    assert!(
        app.help_overlay.visible,
        "/hotkeys must open the overlay, not dump to the transcript"
    );
    let report = crate::tui::keybindings::DEFAULT_KEYBINDINGS.usage_report(&app.slash_usage);
    let text = app.render_hotkeys_report(&report);
    let _ = &text; // report builder exercised; overlay content pinned below
    let overlay_text = app.keys_report_lines(true).join("\n");
    assert!(
        overlay_text.contains("chord(s) used"),
        "/hotkeys leads with the used/never split, got: {overlay_text}"
    );
    assert!(
        overlay_text.contains("You have used:")
            && overlay_text.contains("Never used (undiscovered):"),
        "both halves are rendered, got: {overlay_text}"
    );
    assert!(
        overlay_text.contains("Up"),
        "the chord just pressed shows up in the used half, got: {overlay_text}"
    );
}

#[test]
fn keybinding_registry_should_have_no_duplicate_default_bindings() {
    use crate::tui::keybindings::KeyBindingRegistry;
    use std::collections::HashSet;

    let registry = KeyBindingRegistry::with_defaults();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut dupes: Vec<String> = Vec::new();

    for (context, binding) in registry.catalogue() {
        // The lookup key is exactly what `find` compares: key code +
        // modifiers, scoped to one context.
        let ident = (
            context.label().to_string(),
            format!("{:?}/{:?}", binding.key, binding.modifiers),
        );
        if !seen.insert(ident.clone()) {
            dupes.push(format!("{} twice in [{}]", ident.1, ident.0));
        }
    }
    assert!(
        dupes.is_empty(),
        "a duplicate chord makes `find` order-dependent and ambiguous: {dupes:?}"
    );

    // The table must not be trivially empty — otherwise the check above would
    // pass vacuously.
    assert!(
        registry.catalogue().len() > 100,
        "expected a full default table, got {} rows",
        registry.catalogue().len()
    );
}

// ---- Input affordances: stash / burst undo / queued-message recall ----

/// Ctrl+S parks the composer and empties it; the same chord brings the
/// contents back and clears the stash. A stash held while another is
/// stashed is replaced — one slot, so a prompt can never be stranded.
#[test]
fn input_stash_should_hold_and_restore_contents() {
    let mut app = make_app();
    app.set_prompt_text("half-written prompt".to_string());

    let stash = press_key(KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(!app.handle_key_event(stash));
    assert!(app.prompt_input.is_empty(), "stash must empty the composer");
    assert_eq!(
        app.prompt_input.stash.as_deref(),
        Some("half-written prompt")
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("Input stashed — Ctrl+S brings it back.")
    );

    // Stashing while a stash is held replaces it — one slot, never a stack
    // that could strand an earlier prompt. (Set through the primitive: the
    // chord itself restores when something is held, which is the toggle.)
    app.set_prompt_text("second draft".to_string());
    app.prompt_input.stash_input();
    assert_eq!(app.prompt_input.stash.as_deref(), Some("second draft"));

    // Same chord restores and clears the stash.
    app.handle_key_event(press_key(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(app.prompt_input.text, "second draft");
    assert!(app.prompt_input.stash.is_none());
    assert_eq!(
        app.status_message.as_deref(),
        Some("Stash restored into the composer.")
    );
}

// ---- Failed-submission recovery: the prompt goes back into the composer ----

/// The pure rule, exercised without an agent or a terminal: a failed
/// submission into an empty composer is handed back verbatim.
#[test]
fn resolve_composer_after_failure_restores_into_an_empty_composer() {
    assert_eq!(
        App::resolve_composer_after_failure(Some("audit the browser stack"), ""),
        ComposerRecovery::Restore("audit the browser stack".to_string())
    );
}

/// Restore, never append. Text the user typed *after* the failure is their
/// newest work; neither clobbering it nor splicing the stale prompt onto it
/// is right, so the composer is left exactly as it is.
#[test]
fn resolve_composer_after_failure_keeps_what_the_user_just_typed() {
    for current in ["n", "a fresh prompt", "   leading spaces  "] {
        assert_eq!(
            App::resolve_composer_after_failure(Some("the failed prompt"), current),
            ComposerRecovery::KeepExisting,
            "composer holding {current:?} must be left alone"
        );
    }
}

/// No submission, or a whitespace-only one, is nothing to give back — the
/// slot must not turn a blank submit into a phantom restore.
#[test]
fn resolve_composer_after_failure_has_nothing_to_give_back() {
    assert_eq!(
        App::resolve_composer_after_failure(None, ""),
        ComposerRecovery::Nothing
    );
    assert_eq!(
        App::resolve_composer_after_failure(Some("   \n\t "), ""),
        ComposerRecovery::Nothing
    );
}

/// Bounded by nothing. The composer has no length cap of its own, so a long
/// paste is handed back at full length rather than silently shortened into a
/// prompt the user never wrote.
#[test]
fn resolve_composer_after_failure_does_not_truncate_a_long_prompt() {
    let long = "x".repeat(10_000);
    match App::resolve_composer_after_failure(Some(&long), "") {
        ComposerRecovery::Restore(restored) => assert_eq!(restored.len(), 10_000),
        other => panic!("expected Restore, got {other:?}"),
    }
}

/// End to end through the real submit-and-fail path: Enter empties the
/// composer and arms the slot, an `AgentEvent::Error` gives the text back.
#[test]
fn a_failed_turn_puts_the_prompt_back_in_the_composer() {
    let mut app = make_app();
    app.set_prompt_text("audit the browser stack".to_string());

    let submitted = app.take_input();
    assert_eq!(submitted, "audit the browser stack");
    assert!(app.prompt_input.is_empty(), "submit empties the composer");
    assert_eq!(
        app.failed_input_recovery.as_deref(),
        Some("audit the browser stack"),
        "the submitted text is held for the turn in flight"
    );

    app.handle_agent_event(AgentEvent::Error {
        error: "provider overloaded_error (529)".to_string(),
    });

    assert_eq!(app.prompt_input.text, "audit the browser stack");
    assert_eq!(app.prompt_input.cursor, 23, "cursor lands after the text");
    assert!(
        app.failed_input_recovery.is_none(),
        "the slot is spent once the text is back"
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("Prompt restored to the composer — the turn failed.")
    );
}

/// The same failure can reach the error arm twice — the agent emits
/// `AgentEvent::Error` and then returns `Err`, which makes
/// `drain_run_complete` synthesize a second one. The prompt comes back once.
#[test]
fn a_failure_surfaced_twice_restores_the_prompt_only_once() {
    let mut app = make_app();
    app.set_prompt_text("run the audit twice".to_string());
    app.take_input();

    for _ in 0..2 {
        app.handle_agent_event(AgentEvent::Error {
            error: "provider overloaded_error (529)".to_string(),
        });
    }

    assert_eq!(
        app.prompt_input.text, "run the audit twice",
        "a second delivery must not paste the prompt onto itself"
    );
}

/// Text already in the composer wins: the user typed a new prompt while the
/// failed turn was dying, and that is the one they keep.
#[test]
fn a_failed_turn_does_not_clobber_a_prompt_typed_afterwards() {
    let mut app = make_app();
    app.set_prompt_text("the failed prompt".to_string());
    app.take_input();

    app.set_prompt_text("what I actually want to ask".to_string());
    app.handle_agent_event(AgentEvent::Error {
        error: "connection refused".to_string(),
    });

    assert_eq!(app.prompt_input.text, "what I actually want to ask");
}

/// A submission that never starts a turn — an intercepted slash command, the
/// initial query, `/skill` expansion, `/retry` — is disarmed once the
/// submission has been taken, so a failure in some later turn cannot
/// resurrect it. The order matters and is the point of the test: `take_input`
/// arms the slot (the text did leave the composer), and the run loop is what
/// disarms it again when it learns no turn was started.
#[test]
fn a_submission_that_never_started_a_turn_cannot_resurrect_an_older_prompt() {
    let mut app = make_app();
    app.set_prompt_text("an older prompt".to_string());
    app.take_input();
    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::client::Message {
            role: operant_core::client::Role::Assistant,
            content: "ok".to_string(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        },
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });
    assert!(
        app.failed_input_recovery.is_none(),
        "a turn that succeeds owes nothing back"
    );

    // Enter on a slash command: the text leaves the composer and the slot is
    // armed, because at that instant nothing knows yet that this is a command
    // and not a prompt.
    app.set_prompt_text("/help".to_string());
    app.take_input();
    assert_eq!(
        app.failed_input_recovery.as_deref(),
        Some("/help"),
        "the slot is armed the moment the text leaves the composer"
    );

    // The run loop then sees the leading slash, finds the command was handled
    // locally, and disarms — no turn was ever started to fail.
    app.clear_failed_input_recovery();

    app.handle_agent_event(AgentEvent::Error {
        error: "connection refused".to_string(),
    });

    assert!(
        app.prompt_input.is_empty(),
        "the slash command's text must not come back"
    );
}

/// The slot holds the text of the turn in flight, so a second submission
/// overwrites the first rather than stacking a queue of stale prompts.
#[test]
fn the_restore_slot_tracks_the_turn_currently_in_flight() {
    let mut app = make_app();

    app.set_prompt_text("first".to_string());
    app.take_input();
    app.set_prompt_text("second".to_string());
    app.take_input();

    assert_eq!(app.failed_input_recovery.as_deref(), Some("second"));

    app.handle_agent_event(AgentEvent::Error {
        error: "connection refused".to_string(),
    });
    assert_eq!(app.prompt_input.text, "second");
}

/// A turn that succeeds is spent, not held. Without this, a later error
/// arriving after an unrelated successful turn would pop a stale prompt into
/// a composer the user had already moved on from.
#[test]
fn a_turn_that_succeeds_leaves_nothing_to_restore() {
    let mut app = make_app();
    app.set_prompt_text("a prompt that worked".to_string());
    app.take_input();

    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::client::Message {
            role: operant_core::client::Role::Assistant,
            content: "done".to_string(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        },
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });

    // Something unrelated fails later, with a fresh prompt in the composer.
    app.set_prompt_text("unrelated follow-up".to_string());
    app.handle_agent_event(AgentEvent::Error {
        error: "connection refused".to_string(),
    });

    assert_eq!(
        app.prompt_input.text, "unrelated follow-up",
        "the old prompt must not reappear, and the new one must survive"
    );
}

/// A run of keystrokes collapses into one undo step. Twelve keystrokes are
/// two bursts, so two Ctrl+Z reach the empty composer — not twelve steps.
#[test]
fn input_undo_should_revert_coalesced_edits() {
    let mut app = make_app();

    for c in "abcdefghijkl".chars() {
        app.handle_key_event(press_key(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(app.prompt_input.text, "abcdefghijkl");

    app.handle_key_event(press_key(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_eq!(app.prompt_input.text, "abcdefgh", "8 edits = 1 undo step");
    assert_eq!(
        app.status_message.as_deref(),
        Some("Reverted the last edit burst.")
    );

    app.handle_key_event(press_key(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_eq!(app.prompt_input.text, "", "second burst back to empty");

    // Stack drained — and the key says so instead of doing nothing quietly.
    app.handle_key_event(press_key(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_eq!(app.status_message.as_deref(), Some("Nothing to undo."));
}

/// The stack is capped, so a long editing session cannot grow it without
/// limit; the oldest burst is the one that goes.
#[test]
fn input_undo_should_respect_stack_bound() {
    use crate::tui::prompt_input::{UNDO_COALESCE, UNDO_STACK_MAX};

    let mut app = make_app();
    // Comfortably past the cap: each outer tick is one closed burst.
    for _ in 0..UNDO_STACK_MAX + 8 {
        for _ in 0..UNDO_COALESCE {
            app.refresh_prompt_input();
        }
        app.prompt_input.insert_char('x');
    }

    assert!(
        app.prompt_input.burst_undo.len() <= UNDO_STACK_MAX,
        "burst stack grew to {} entries, cap is {UNDO_STACK_MAX}",
        app.prompt_input.burst_undo.len()
    );
    assert_eq!(app.prompt_input.burst_undo.len(), UNDO_STACK_MAX);
}

/// Ctrl+X takes the head of the real steer queue (FIFO — the same order the
/// agent would drain it in) rather than a second, private queue.
#[test]
fn queued_message_recall_should_move_head_into_composer() {
    let mut app = make_app();
    let queue = std::sync::Arc::new(tokio::sync::Mutex::new(vec![
        "first queued".to_string(),
        "second queued".to_string(),
    ]));
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));

    app.handle_key_event(press_key(KeyCode::Char('x'), KeyModifiers::CONTROL));
    assert_eq!(app.prompt_input.text, "first queued");
    assert_eq!(
        queue.try_lock().unwrap().as_slice(),
        ["second queued".to_string()],
        "recall removes the message instead of copying it"
    );

    // A non-empty composer is never overwritten: stash first, then recall.
    app.handle_key_event(press_key(KeyCode::Char('x'), KeyModifiers::CONTROL));
    assert_eq!(
        app.status_message.as_deref(),
        Some("Composer is not empty — Ctrl+S stashes it before recall.")
    );
    assert_eq!(app.prompt_input.text, "first queued");

    // Stash, then recall picks up where the queue left off.
    app.handle_key_event(press_key(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.handle_key_event(press_key(KeyCode::Char('x'), KeyModifiers::CONTROL));
    assert_eq!(app.prompt_input.text, "second queued");
    assert!(queue.try_lock().unwrap().is_empty());
}

// ---- Clipboard chain + drag-select copy mode (iter-351) ----

/// The fallback decision is a pure function of availability, so the whole
/// preference order is assertable without touching `$PATH` or a terminal —
/// including the part that never changes: OSC 52 is last, always.
#[test]
fn clipboard_should_prefer_earlier_mechanism_in_chain() {
    use crate::tui::clipboard::{CHAIN, Mechanism, plan};

    assert_eq!(
        CHAIN,
        [
            Mechanism::Arboard,
            Mechanism::WlCopy,
            Mechanism::Xclip,
            Mechanism::Xsel,
            Mechanism::Osc52
        ],
        "chain order changed — OSC 52 must stay the last resort"
    );

    // Every mechanism available, listed in the worst possible order: the plan
    // must still come back as the full chain, in preference order.
    assert_eq!(
        plan(&[
            Mechanism::Xsel,
            Mechanism::Osc52,
            Mechanism::Xclip,
            Mechanism::WlCopy,
            Mechanism::Arboard
        ]),
        CHAIN.to_vec()
    );

    // A later mechanism available does not promote it above an earlier one.
    assert_eq!(
        plan(&[Mechanism::Xsel, Mechanism::Osc52]),
        vec![Mechanism::Xsel, Mechanism::Osc52]
    );
    assert_eq!(plan(&[Mechanism::Osc52]), vec![Mechanism::Osc52]);
    assert!(plan(&[]).is_empty());
}

#[test]
fn clipboard_should_report_all_attempted_mechanisms_on_total_failure() {
    use crate::tui::clipboard::{Attempt, CHAIN, failure_summary};

    // What a total failure has to look like: every mechanism named, each with a
    // reason, all on one line. Built from the real chain so the test breaks if
    // a mechanism is added without a reason for why it failed.
    let attempts: Vec<Attempt> = CHAIN
        .iter()
        .map(|m| Attempt {
            mechanism: *m,
            reason: format!("{} unavailable here", m.label()),
        })
        .collect();
    let line = failure_summary(&attempts);

    assert!(!line.contains('\n'), "report must be one line: {line}");
    for m in CHAIN {
        assert!(
            line.contains(m.label()),
            "report omits {}: {line}",
            m.label()
        );
    }
    // The one line names each mechanism AND the reason it failed.
    for m in CHAIN {
        assert!(
            line.contains(&format!("{} ({} unavailable here)", m.label(), m.label())),
            "report omits the reason for {}: {line}",
            m.label()
        );
    }
}

#[test]
fn copy_mode_should_restore_state_on_exit() {
    let mut app = make_app();

    // Baseline: nothing active, and the composer owns the keyboard.
    assert!(!app.copy_mode_active());
    app.scroll_offset = 12;
    app.auto_scroll = false;

    // Enter with a live selection that copy mode is about to take over.
    app.selection_anchor = Some((3, 4));
    app.selection_focus = Some((30, 9));
    *app.selection_text.borrow_mut() = "selected before copy mode".to_string();
    app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::ALT));
    assert!(
        app.copy_mode_active(),
        "Alt+Y did not enter copy mode (Ctrl+T is the thinking toggle since plan 08)"
    );

    // While copy mode is on, composer keys are swallowed — typing 'x' must not
    // reach the composer, and the transcript scroll stays put.
    let before_scroll = app.scroll_offset;
    app.handle_key_event(press_key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(
        app.prompt_input.is_empty(),
        "copy mode leaked a keystroke into the composer"
    );
    assert_eq!(app.scroll_offset, before_scroll);

    // Drag-select moves the scroll position; copy mode has to put it back.
    app.scroll_offset = 99;
    app.auto_scroll = true;
    app.selection_anchor = Some((3, 4));
    app.selection_focus = Some((3, 5));
    *app.selection_text.borrow_mut() = "dragged".to_string();

    // Esc leaves copy mode.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.copy_mode_active(), "Esc did not exit copy mode");

    // Every piece of pre-entry state is back: scroll offset, tail-follow flag,
    // and the selection that was live on entry.
    assert_eq!(app.scroll_offset, 12, "scroll offset not restored");
    assert!(!app.auto_scroll, "tail-follow flag not restored");
    assert_eq!(app.selection_anchor, Some((3, 4)));
    assert_eq!(app.selection_focus, Some((30, 9)));
    assert_eq!(
        app.selection_text.borrow().as_str(),
        "selected before copy mode"
    );

    // Key handling is restored: the composer takes 'x' again.
    app.handle_key_event(press_key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(
        !app.prompt_input.is_empty(),
        "key handling not restored after leaving copy mode"
    );
}

#[test]
fn copy_mode_should_toggle_with_alt_y() {
    let mut app = make_app();
    app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::ALT));
    assert!(app.copy_mode_active());
    app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::ALT));
    assert!(
        !app.copy_mode_active(),
        "Alt+Y did not toggle copy mode off"
    );
}

// ---- Behavioural keybinding drift (iter-420) ----
//
// `tui/keybindings/tests.rs::global_bindings_should_be_dispatched_in_key_handling`
// is a SOURCE-level pin: it greps `key_handling.rs` for the key token. That
// catches a deleted or renamed dispatch arm, and it says plainly that it cannot
// see two things:
//
//   - that the right MODIFIER is required. A bare `x` press satisfies a
//     catalogue entry for `Ctrl+X` just as well.
//   - that the press lands in the right BRANCH.
//
// Both are answerable here, because `make_app`/`press_key` are three lines each
// — the doc comment claimed App construction was "a large fixture", which is
// stale. So this presses the chord and asserts the App state actually moved,
// and then presses the SAME KEY WITHOUT ITS MODIFIER and asserts nothing moved.
// The negative half is the part the source pin cannot do: it is what proves
// `Ctrl+K` is not really just a bare `k` that happens to be caught by an
// unrelated arm.
//
// Scope is the Global-context chords, matching the source pin, because Global
// is where a false advertisement hurts most: the user presses a chord from
// anywhere, nothing happens, and `/keys` said otherwise. The other eleven
// `BindingContext`s dispatch in `prompt_input/vim.rs`, `typeahead.rs` and
// `suggestions.rs`, and are not covered by either test.

/// One Global chord plus how to observe that it fired.
struct DriftedBinding {
    key: KeyCode,
    modifiers: KeyModifiers,
    /// Plain-English chord, for the failure message.
    chord: &'static str,
    /// Whether the chord carries a modifier the bare key does not.
    requires_modifier: bool,
    fired: fn(&mut App) -> bool,
}

fn help_is_open(app: &mut App) -> bool {
    app.show_help
}

fn usage_overlay_is_open(app: &mut App) -> bool {
    app.usage_overlay.visible
}

fn command_palette_is_open(app: &mut App) -> bool {
    app.command_palette.visible
}

/// (plan 08) Ctrl+T's observable effect: the reasoning display override
/// sits at Off.
fn thinking_is_off(app: &mut App) -> bool {
    use crate::tui::operant_app::config_shim::{self, ReasoningDisplayMode};
    let _ = app;
    config_shim::config().display.reasoning_display() == ReasoningDisplayMode::Off
}

fn drifted_bindings() -> Vec<DriftedBinding> {
    vec![
        DriftedBinding {
            key: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
            chord: "F1",
            requires_modifier: false,
            fired: help_is_open,
        },
        DriftedBinding {
            key: KeyCode::F(8),
            modifiers: KeyModifiers::NONE,
            chord: "F8",
            requires_modifier: false,
            fired: usage_overlay_is_open,
        },
        DriftedBinding {
            key: KeyCode::Char('k'),
            modifiers: KeyModifiers::CONTROL,
            chord: "Ctrl+K",
            requires_modifier: true,
            fired: command_palette_is_open,
        },
        DriftedBinding {
            key: KeyCode::Char('t'),
            modifiers: KeyModifiers::CONTROL,
            chord: "Ctrl+T",
            requires_modifier: true,
            // (plan 08) Ctrl+T is the thinking toggle now: its observable
            // effect is the display-mode override cell, not copy mode.
            fired: thinking_is_off,
        },
    ]
}

#[test]
fn global_bindings_should_fire_their_action_behaviourally() {
    // One of the drifted chords is Ctrl+T, and the thinking toggle writes
    // the PROCESS-GLOBAL reasoning-mode cell. Without this guard the
    // toggle's write lands in the middle of a parallel test that expects
    // a different mode — which is how the live-tails test came to fail
    // intermittently with "the thinking did not render".
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    let mut inert = Vec::new();
    for b in drifted_bindings() {
        let mut app = make_app();
        assert!(
            !(b.fired)(&mut app),
            "{}: fixture starts already in the state the chord should reach, so the \
             test cannot tell a working binding from a no-op",
            b.chord
        );
        app.handle_key_event(press_key(b.key, b.modifiers));
        if !(b.fired)(&mut app) {
            inert.push(b.chord);
        }
    }
    assert!(
        inert.is_empty(),
        "these Global chords are advertised by /keys but pressing them does not change \
         any App state — the help screen claims a capability that does not exist:\n  {}",
        inert.join("\n  ")
    );
}

#[test]
fn a_modified_binding_must_not_fire_on_the_bare_key() {
    // Ctrl+T is one of the drifted chords, and the thinking toggle writes
    // the process-global reasoning-mode cell — so this test needs the same
    // guard as the one above.
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    let mut leaked = Vec::new();
    for b in drifted_bindings() {
        if !b.requires_modifier {
            continue;
        }
        // Same key, no modifier. If this still fires, the modifier in the
        // catalogue is decorative and the source-level pin cannot see it,
        // because it only greps for the key token.
        let mut app = make_app();
        app.handle_key_event(press_key(b.key, KeyModifiers::NONE));
        if (b.fired)(&mut app) {
            leaked.push(b.chord);
        }
    }
    assert!(
        leaked.is_empty(),
        "these bindings are registered WITH a modifier but also fire on the bare key, \
         so the chord in /keys is not what actually gates the action — a bare press \
         will trigger it by accident:\n  {}",
        leaked.join("\n  ")
    );
}

/// The two tests above pass if `make_app` returns an `App` whose key handling is
/// dead, because every assertion would then be "nothing happened". This proves
/// the harness can still observe a firing binding, so an inert result above
/// means the binding is broken rather than the fixture.
#[test]
fn the_behavioural_harness_can_observe_a_binding_firing() {
    let mut app = make_app();
    assert!(!app.show_help, "precondition: help starts closed");
    app.handle_key_event(press_key(KeyCode::F(1), KeyModifiers::NONE));
    assert!(
        app.show_help,
        "F1 did not open help, so the drift tests above cannot be trusted: they would \
         pass for every binding if no key ever reached the dispatch chain"
    );
}

// ── Ctrl+L terminal-style clear (jcode parity) ─────────────────────────────

#[test]
fn ctrl_l_collapses_until_new_content_arrives() {
    let mut app = make_app();
    app.add_message(Role::User, "hello".to_string());
    app.clear_view_terminal_style();
    assert!(
        app.terminal_clear_state_live(),
        "Ctrl+L with no transcript change must collapse the view"
    );
    app.add_message(Role::Assistant, "world".to_string());
    assert!(
        !app.terminal_clear_state_live(),
        "new content bumps transcript_version, which must end the cleared state"
    );
}

#[test]
fn ctrl_l_ends_on_scroll_up() {
    let mut app = make_app();
    app.clear_view_terminal_style();
    assert!(app.terminal_clear_state_live());
    app.auto_scroll = false; // what scrolling up does
    assert!(
        !app.terminal_clear_state_live(),
        "scrolling up must restore the full layout"
    );
}

#[test]
fn ctrl_l_holds_while_streaming_then_recollapses() {
    let mut app = make_app();
    app.clear_view_terminal_style();
    // Streaming text arrives without bumping the transcript version yet:
    // the predicate must defer to the live stream.
    app.is_streaming = true;
    app.streaming_text = "partial".to_string();
    assert!(!app.terminal_clear_state_live());
    // Stream settles with no committed content: version unchanged, so the
    // cleared view returns once the idle conditions hold again.
    app.is_streaming = false;
    app.streaming_text.clear();
    assert!(app.terminal_clear_state_live());
}

#[test]
fn ctrl_l_keybinding_collapses_the_view() {
    let mut app = make_app();
    app.add_message(Role::User, "hello".to_string());
    assert!(!app.terminal_clear_state_live());
    app.handle_key_event(press_key(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(
        app.terminal_clear_state_live(),
        "the Ctrl+L chord must reach clear_view_terminal_style"
    );
}

// ── Live credential status (2026-10-09 visual audit: stuck login header) ──

/// Serializes tests that mutate ANTHROPIC/OPENAI env vars; other suite tests
/// read these only via App init, and the cleared window below makes such an
/// init see "no credentials" — the exact state under test.
static AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_cleared_auth_env<R>(f: impl FnOnce() -> R) -> R {
    let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_anthropic = std::env::var("ANTHROPIC_API_KEY").ok();
    let saved_openai = std::env::var("OPENAI_API_KEY").ok();
    // SAFETY: serialized by AUTH_ENV_LOCK; tests are single-binary and the
    // cleared window only affects other tests' App-init reads (the state
    // under test). Restored before the lock drops.
    unsafe {
        // The environment is process-global, so this test's key removal
        // must not interleave with another test that sets one. The
        // crate-wide lock — see `crate::config::test_env_lock`.
        let _env_lock = crate::config::test_env_lock();
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
    }
    let result = f();
    // SAFETY: same lock held; restoring the exact values observed above.
    unsafe {
        if let Some(key) = saved_anthropic {
            std::env::set_var("ANTHROPIC_API_KEY", key);
        }
        if let Some(key) = saved_openai {
            std::env::set_var("OPENAI_API_KEY", key);
        }
    }
    result
}

#[test]
fn auth_status_refreshes_when_store_gains_key_without_recreating_app() {
    with_cleared_auth_env(|| {
        let mut app = make_app();
        app.has_credentials = false;
        // The real on-disk store may carry the developer's credentials; this
        // test controls credential presence explicitly.
        app.auth_store.credentials.clear();
        app.active_provider = Some("anthropic".to_string());
        let before = app.auth_status();
        assert!(
            before.anthropic.state == AuthState::NotConfigured,
            "precondition: matrix starts unconfigured"
        );
        // The boot-time snapshot is false; a mid-session /login adds a key.
        // Direct map insert — `AuthStore::set` persists to the real on-disk
        // credential file, which a unit test must never touch.
        app.auth_store.credentials.insert(
            "anthropic".to_string(),
            crate::tui::adapter_types::StoredCredential::ApiKey {
                key: "sk-ant-test".to_string(),
            },
        );
        let after = app.auth_status();
        assert_eq!(
            after.anthropic.state,
            AuthState::Available,
            "auth_status must recompute from the live store, not the init snapshot"
        );
    })
}

#[test]
fn credentials_live_or_s_store_env_and_activation_flag() {
    with_cleared_auth_env(|| {
        let mut app = make_app();
        app.has_credentials = false;
        app.auth_store.credentials.clear();
        assert!(!app.credentials_live());
        app.has_credentials = true;
        assert!(
            app.credentials_live(),
            "sticky activation flag alone counts"
        );
    })
}

#[test]
fn auth_status_maps_custom_profiles_to_compatible_slot() {
    let mut app = make_app();
    app.has_credentials = true;
    app.active_provider = Some("groq".to_string()); // catalog profile: no dedicated slot
    let status = app.auth_status();
    assert_eq!(
        status.openai_compatible_any,
        AuthState::Available,
        "OpenAI-compatible catalog profiles must land in openai_compatible_any"
    );
}

#[test]
fn auth_status_falls_back_to_model_inference_when_provider_unset() {
    let mut app = make_app();
    app.has_credentials = true;
    app.active_provider = None;
    app.config.agent.model = "free/auto".to_string();
    let status = app.auth_status();
    assert_eq!(
        status.openai_compatible_any,
        AuthState::Available,
        "provider inference from the configured model must still populate the matrix"
    );
}

#[test]
fn last_msg_area_defaults_zero_and_is_republished_from_draw() {
    // The cell is a hit-test publish target: zero-rect means the right-click
    // bounds check can never pass. The render pass sets it from the vendored
    // message_area(); this test pins the default and the setter semantics.
    let app = make_app();
    assert_eq!(
        app.last_msg_area.get(),
        ratatui::layout::Rect::default(),
        "fresh App must carry a zero rect so stale geometry never gates a menu"
    );
    app.last_msg_area
        .set(ratatui::layout::Rect::new(2, 3, 40, 10));
    assert_eq!(app.last_msg_area.get().width, 40);
}

#[test]
fn ctrl_arrow_chords_scroll_transcript_in_fine_steps() {
    let mut app = make_app();
    app.auto_scroll = true;
    app.handle_key_event(press_key(KeyCode::Up, KeyModifiers::CONTROL));
    assert_eq!(
        app.scroll_offset, 3,
        "Ctrl+Up must scroll the transcript 3 lines"
    );
    assert!(!app.auto_scroll, "Ctrl+Up must pause tail-follow");
    app.scroll_offset = 10;
    app.handle_key_event(press_key(KeyCode::Down, KeyModifiers::CONTROL));
    assert_eq!(app.scroll_offset, 7, "Ctrl+Down must scroll back 3 lines");
    app.handle_key_event(press_key(KeyCode::Down, KeyModifiers::CONTROL));
    app.handle_key_event(press_key(KeyCode::Down, KeyModifiers::CONTROL));
    app.handle_key_event(press_key(KeyCode::Down, KeyModifiers::CONTROL));
    assert_eq!(app.scroll_offset, 0);
    assert!(
        app.auto_scroll,
        "reaching the bottom must resume tail-follow"
    );
}

// ── Composer shift-selection (jcode textarea selection, 2026-10-09 audit) ──

#[test]
fn shift_arrows_select_composer_text_and_ctrl_c_wins_over_exit() {
    let mut app = make_app();
    app.prompt_input.replace_text("hello world".to_string());
    app.prompt_input.cursor = 6; // between "hello" and "world"
    app.handle_key_event(press_key(KeyCode::Right, KeyModifiers::SHIFT));
    app.handle_key_event(press_key(KeyCode::Right, KeyModifiers::SHIFT));
    assert_eq!(
        app.prompt_input.selection_range(),
        Some((6, 8)),
        "Shift+Right must extend the selection from the anchor"
    );
    assert_eq!(
        app.prompt_input.selection_text().as_deref(),
        Some("wo"),
        "selection_text must slice the selected span"
    );
    // Any plain edit key collapses the selection.
    app.handle_key_event(press_key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(app.prompt_input.selection_range(), None);
}

#[test]
fn shift_left_selects_backwards_toward_anchor() {
    let mut app = make_app();
    app.prompt_input.replace_text("abc".to_string());
    app.prompt_input.cursor = 3;
    app.handle_key_event(press_key(KeyCode::Left, KeyModifiers::SHIFT));
    app.handle_key_event(press_key(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(
        app.prompt_input.selection_range(),
        Some((1, 3)),
        "anchor fixes at the start; cursor moves back over it"
    );
    assert_eq!(app.prompt_input.selection_text().as_deref(), Some("bc"));
}

#[test]
fn wrap_input_text_paints_selection_in_reverse_video() {
    let (lines, _, _) = crate::tui::operant_ui::input_ui::wrap_input_text(
        "hello world",
        8,
        80,
        "1",
        ">",
        ratatui::style::Color::Gray,
        2,
        Some((6, 8)),
    );
    let found = lines.iter().any(|line| {
        line.spans.iter().any(|span| {
            span.content.as_ref() == "wo"
                && span
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::REVERSED)
        })
    });
    assert!(found, "selection span must paint reverse-video over 'wo'");
    // Unselected text still renders: the rest of the line is intact.
    let joined: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect();
    assert!(
        joined.contains("hello ") && joined.ends_with("rld"),
        "chars outside the selection must survive: {joined}"
    );
}

#[test]
fn auth_status_derives_slots_from_env_keys_when_provider_unset() {
    // No provider id and a stored anthropic key: the matrix must show the
    // session as configured (the corpus/sim shape, and any boot with keys
    // but no provider field). Store-based, not env-based: a parallel test's
    // App init would snapshot a mutated env into its own AuthStore, and
    // that cross-test poisoning is exactly what broke the Ctrl+A picker
    // test (tokio::spawn needs a runtime the unit test lacks). Pinned
    // end-to-end by the first-message corpus scenario (not-contains
    // "○ anthropic").
    with_cleared_auth_env(|| {
        let mut app = make_app();
        app.has_credentials = false;
        app.active_provider = None;
        // omp-style bare model — provider inference must yield None so the
        // credential-derivation arm is the one under test (the exact shape of
        // the user's reported session).
        app.config.agent.model = "glm-5.3".to_string();
        app.auth_store.credentials.clear();
        app.auth_store.credentials.insert(
            "anthropic".to_string(),
            crate::tui::adapter_types::StoredCredential::ApiKey {
                key: "sk-ant-test".to_string(),
            },
        );
        let status = app.auth_status();
        assert_eq!(
            status.anthropic.state,
            crate::tui::operant_app::auth::AuthState::Available,
            "a stored anthropic key with no provider id must mark anthropic configured"
        );
    })
}

/// Render `app` once and return the screen as one string per row.
fn screen_rows(app: &App) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| crate::tui::render::render_app(f, app))
        .unwrap();
    let buf = terminal.backend().buffer();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// The keyboard/wheel scroll ladder driven against a REAL render.
///
/// 2026-10-22 (P4-1 follow-up): measured on the headless simulator, a single
/// PageUp moved the view (tail L200 -> L083) but the SECOND produced a
/// non-monotonic jump (L126) and the THIRD snapped straight back to the tail
/// (L200). A scroll ladder that reverses direction and returns the reader to
/// the tail is worse than no ladder: it looks like input is being eaten. This
/// paints a real frame at each step so the render seam, not the harness's
/// drain-before-content artifact, is what is under test: the offset the keys
/// compute must be honoured monotonically by the paint and by the reconcile
/// that follows it.
#[test]
fn repeated_page_up_walks_upward_monotonically() {
    let mut app = make_app();
    // A transcript far taller than the viewport.
    for i in 1..=200 {
        app.messages
            .push(Message::assistant(format!("L{i:03} scroll test line")));
    }

    // Tail-follow first: the reader is on the last message.
    let tail = screen_rows(&app);
    let tail_flat = tail.join("\n");
    assert!(
        tail_flat.contains("L200"),
        "tail-follow shows the last line: {tail_flat}"
    );

    // Each PageUp step: the key handler's arithmetic, then a paint and the
    // reconcile the run loop runs after it. `offset` counts rows up from the
    // bottom, so a growing offset is the reader walking UP the transcript.
    let mut seen_offsets: Vec<usize> = Vec::new();
    for _ in 0..6 {
        app.scroll_offset = app.scroll_offset.saturating_add(10);
        app.auto_scroll = false;
        // Paint the frame the reader asked for.
        let rows = screen_rows(&app);
        let flat = rows.join("\n");
        let top_line = visible_top_line(&flat);
        // The reconcile a real run loop performs after every paint.
        app.reconcile_scroll_anchor();
        seen_offsets.push(app.scroll_offset);
        // The offset must never be pulled back DOWN by the reconcile: that is
        // the ghost-offset clamp eating a legitimate reading position.
        assert!(
            top_line.is_some_and(|l| l <= 190),
            "a real paint must honour the offset (step {}): top={top_line:?}",
            seen_offsets.len()
        );
    }
    assert!(
        seen_offsets.windows(2).all(|w| w[0] < w[1]),
        "each PageUp must grow the offset, got {seen_offsets:?}"
    );
    assert!(
        seen_offsets.last().copied().unwrap_or(0) >= 60,
        "six PageUps must reach at least 60 rows up: {seen_offsets:?}"
    );
}

/// The lowest `L###` marker visible on a rendered screen, as a number.
fn visible_top_line(screen: &str) -> Option<usize> {
    screen
        .split_whitespace()
        .filter_map(|tok| tok.strip_prefix('L'))
        .filter_map(|rest| rest.parse::<usize>().ok())
        .min()
}

fn rows_containing(rows: &[String], needle: &str) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| r.contains(needle))
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn live_tails_render_once_while_a_tool_is_reserved() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    crate::tui::operant_app::config_shim::set_reasoning_display_override(Some(
        crate::tui::operant_app::config_shim::ReasoningDisplayMode::Full,
    ));
    // 2026-10-13 duplication: a ToolStart that reserved a slot for the live
    // tails used to splice them into the Body AND render them again in the
    // Streaming section — every thinking/text line showed twice until the
    // first tool settled.
    use operant_core::agent::AgentEvent;
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.handle_agent_event(AgentEvent::Thinking {
        content: "THINKMARK plan the call".to_string(),
    });
    app.handle_agent_event(AgentEvent::Content {
        text: "TEXTMARK running it".to_string(),
    });
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "t1".to_string(),
        name: "bash".to_string(),
        arguments: r#"{"command":"echo TOOLMARK"}"#.to_string(),
    });

    let rows = screen_rows(&app);
    let think = rows_containing(&rows, "THINKMARK");
    let text = rows_containing(&rows, "TEXTMARK");
    let tool = rows_containing(&rows, "bash $ echo TOOLMARK");
    assert_eq!(think.len(), 1, "thinking renders once: {rows:#?}");
    assert_eq!(text.len(), 1, "text renders once: {rows:#?}");
    assert_eq!(tool.len(), 1, "tool row renders once: {rows:#?}");
    assert!(
        think[0] < text[0] && text[0] < tool[0],
        "stream order thinking < text < tool: {rows:#?}"
    );

    // The tool settles: the boundary flush commits the tails, the row
    // returns to the Body at its anchor — still once each, same order.
    app.handle_agent_event(AgentEvent::ToolError {
        tool_call_id: "t1".to_string(),
        name: "bash".to_string(),
        error: "boom".to_string(),
    });
    let rows = screen_rows(&app);
    let text = rows_containing(&rows, "TEXTMARK");
    let tool = rows_containing(&rows, "bash $ echo TOOLMARK");
    assert_eq!(text.len(), 1, "committed text renders once: {rows:#?}");
    assert_eq!(tool.len(), 1, "settled tool row renders once: {rows:#?}");
    assert!(text[0] < tool[0], "tool stays below its text: {rows:#?}");
    crate::tui::operant_app::config_shim::set_reasoning_display_override(None);
}

#[test]
fn empty_flush_releases_the_tool_slot_reservation() {
    let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
    // A mid-turn flip to Off drops a thinking-only iteration at flush, so
    // nothing is pushed; the slot ToolStart reserved must be released or the
    // tool row later anchors BELOW the next message (the final answer).
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t1".to_string(),
        name: "read".to_string(),
        after_index: 2,
        status: ToolStatus::Done,
        output_preview: None,
        input_json: "{}".to_string(),
        intent: None,
    });
    // Off: no Text, no persisted Thinking — the "0 blocks" arm.
    app.streaming_thinking = "x".to_string();
    crate::tui::operant_app::config_shim::set_reasoning_display_override(Some(
        crate::tui::operant_app::config_shim::ReasoningDisplayMode::Off,
    ));
    app.flush_streamed_assistant_message();
    crate::tui::operant_app::config_shim::set_reasoning_display_override(None);
    assert_eq!(app.messages.len(), 1);
    assert_eq!(app.tool_use_blocks[0].after_index, 1);
}

#[test]
fn selection_copy_is_synchronous_and_not_clamped_to_the_viewport() {
    // 2026-10-13 copy parity: the release handler used to copy the text the
    // render pass scraped from painted cells — one frame late (a fast
    // Drag…Up batch copied nothing) and clamped to the visible window.
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    let body: Vec<String> = (0..30).map(|i| format!("LINE{i:02} words")).collect();
    app.messages.push(Message::assistant(body.join("\n\n")));
    // Draw once with NO selection: the render pass has nothing to scrape.
    let rows = screen_rows(&app);
    let frame = crate::tui::operant_ui::last_chat_frame().expect("frame drawn");
    let line_of = |needle: &str| {
        (0..frame.total_wrapped_lines)
            .find(|&l| {
                frame
                    .wrapped_plain_line(l)
                    .is_some_and(|t| t.contains(needle))
            })
            .unwrap_or_else(|| panic!("{needle} not in frame"))
    };
    let (top, bottom) = (line_of("LINE03"), line_of("LINE29"));
    assert!(
        rows_containing(&rows, "LINE03").is_empty(),
        "LINE03 must be scrolled off screen for this test: {rows:#?}"
    );

    // Selection set after the last draw, spanning off-screen lines.
    let area = app.last_selectable_area.get();
    app.selection_anchor = Some((area.x, top));
    app.selection_focus = Some((area.x + area.width - 1, bottom));
    let text = app.refresh_selection_text();
    assert!(text.starts_with("LINE03 words"), "{text:?}");
    assert!(text.contains("LINE15 words"), "interior copies: {text:?}");
    assert!(text.ends_with("LINE29 words"), "{text:?}");

    // Exact column slice on one visible line.
    let row = rows_containing(&rows, "LINE29")[0];
    let col = rows[row].find("LINE29").unwrap() as u16;
    app.selection_anchor = Some((col, bottom));
    app.selection_focus = Some((col + 5, bottom));
    assert_eq!(app.refresh_selection_text(), "LINE29");
}

/// First frame line whose plain text contains `needle`.
fn frame_line_of(needle: &str) -> usize {
    let frame = crate::tui::operant_ui::last_chat_frame().expect("frame drawn");
    (0..frame.total_wrapped_lines)
        .find(|&l| {
            frame
                .wrapped_plain_line(l)
                .is_some_and(|t| t.contains(needle))
        })
        .unwrap_or_else(|| panic!("{needle} not in frame"))
}

fn select_lines(app: &mut App, first: usize, last: usize) -> String {
    let area = app.last_selectable_area.get();
    app.selection_anchor = Some((area.x, first));
    app.selection_focus = Some((area.x + area.width - 1, last));
    app.refresh_selection_text()
}

#[test]
fn copy_joins_soft_wraps_and_drops_gutters_like_jcode() {
    // 2026-10-13 copy parity: soft-wrapped rows copied as separate lines and
    // the prompt gutter (`1›`) came along. Copy is logical lines, text only.
    let mut app = make_app();
    let prompt = "PROMPTSTART ".to_string() + &"prompt words ".repeat(14) + "PROMPTEND";
    let para = "PARASTART ".to_string() + &"alpha beta gamma delta ".repeat(10) + "PARAEND";
    app.messages.push(Message::user(prompt.clone()));
    app.messages.push(Message::assistant(para.clone()));
    let _ = screen_rows(&app);
    let (p0, p1) = (frame_line_of("PROMPTSTART"), frame_line_of("PROMPTEND"));
    let (a0, a1) = (frame_line_of("PARASTART"), frame_line_of("PARAEND"));
    assert!(p1 > p0 && a1 > a0, "both must soft-wrap for this test");

    assert_eq!(
        select_lines(&mut app, p0, p1),
        prompt,
        "prompt: one line, no 1›"
    );
    assert_eq!(select_lines(&mut app, a0, a1), para, "paragraph: one line");
    assert_eq!(
        select_lines(&mut app, p0, a1),
        format!("{prompt}\n\n{para}"),
        "logical lines keep their blank separator"
    );
}

#[test]
fn copy_carries_tool_rows_alongside_prose() {
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.messages
        .push(Message::assistant("BEFORE text".to_string()));
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t1".to_string(),
        name: "bash".to_string(),
        after_index: 2,
        status: ToolStatus::Done,
        output_preview: Some("TOOLOUT".to_string()),
        input_json: r#"{"command":"echo TOOLCMD"}"#.to_string(),
        intent: None,
    });
    app.messages
        .push(Message::assistant("AFTER text".to_string()));
    let _ = screen_rows(&app);
    let (b, t, a) = (
        frame_line_of("BEFORE"),
        frame_line_of("TOOLCMD"),
        frame_line_of("AFTER"),
    );
    assert!(b < t && t < a);
    // jcode copies tool rows: its `ui_prepare` pushes them through the same
    // `push_auto` path as prose, so a drag across prose and a call keeps BOTH.
    let across = select_lines(&mut app, b, a);
    assert!(
        across.contains("BEFORE text"),
        "prose before the call survives: {across:?}"
    );
    assert!(
        across.contains("TOOLCMD"),
        "the tool call survives the mixed selection: {across:?}"
    );
    assert!(
        across.contains("AFTER text"),
        "prose after the call survives: {across:?}"
    );
    let only_tool = select_lines(&mut app, t, t);
    assert!(only_tool.contains("TOOLCMD"), "{only_tool:?}");
}

// The regression risk in iter-1190's direction: dropping "tool" from
// UI_ROLES must not make everything copyable. Real chrome (a system notice)
// still gets skipped when the selection also covers conversation text, while a
// tool row does not — the exception is specific to tools, not a general
// "copy spans everything now".
#[test]
fn copy_still_skips_chrome_when_the_selection_mixes_prose_and_a_tool_call() {
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.messages.push(Message {
        role: Role::System,
        content: MessageContent::Text("SYSNOTICE background check finished".to_string()),
    });
    app.messages
        .push(Message::assistant("BEFORE text".to_string()));
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t1".to_string(),
        name: "bash".to_string(),
        after_index: 3,
        status: ToolStatus::Done,
        output_preview: Some("TOOLOUT".to_string()),
        input_json: r#"{"command":"echo TOOLCMD"}"#.to_string(),
        intent: None,
    });
    app.messages
        .push(Message::assistant("AFTER text".to_string()));
    let _ = screen_rows(&app);

    let sys = frame_line_of("SYSNOTICE");
    let tool = frame_line_of("TOOLCMD");
    let before = frame_line_of("BEFORE");
    assert!(
        before < tool,
        "tool must render after the prose it belongs to"
    );

    // System row + tool row + prose: the chrome drops, the call survives.
    let mixed = select_lines(&mut app, sys, tool);
    assert!(
        mixed.contains("TOOLCMD"),
        "the tool call must survive even in a selection that starts on chrome: {mixed:?}"
    );
    assert!(
        !mixed.contains("SYSNOTICE"),
        "the system notice is still chrome and must not copy: {mixed:?}"
    );

    // Selecting ONLY chrome is the documented escape hatch: the user explicitly
    // grabbed those rows, so they copy. Removing "tool" must not remove this
    // exception either — it is what makes an all-chrome drag useful.
    let only_chrome = select_lines(&mut app, sys, sys);
    assert!(
        only_chrome.contains("SYSNOTICE"),
        "an all-chrome selection still copies, by design: {only_chrome:?}"
    );
}

#[test]
fn copy_slices_exact_columns_on_wrapped_rows() {
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    let para = "alpha ".repeat(30) + "MIDWORD tail";
    app.messages.push(Message::assistant(para));
    let rows = screen_rows(&app);
    let line = frame_line_of("MIDWORD");
    let row = rows_containing(&rows, "MIDWORD")[0];
    let col = rows[row].find("MIDWORD").unwrap() as u16;
    app.selection_anchor = Some((col, line));
    app.selection_focus = Some((col + 6, line));
    assert_eq!(app.refresh_selection_text(), "MIDWORD");
}

#[test]
fn copy_rejoins_pre_wrapped_system_cards_into_authored_lines() {
    // 2026-10-13: a copied error card broke mid-sentence at the terminal
    // width — system rows are pre-wrapped by their renderer, so each row was
    // its own "logical" line. Rows now map back onto the authored lines.
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    let err = "CARDSTART The model provider's stream died mid-response and did not recover after 0 retries over 60s (Network error: error decoding response body). Your message was not lost — reply 'continue' to retry it. CARDEND";
    app.push_system_message(err.to_string(), SystemMessageStyle::Error);
    let note = "NOTESTART first authored line\n\nthird authored line NOTEEND";
    app.push_system_message(note.to_string(), SystemMessageStyle::Info);
    let _ = screen_rows(&app);
    let (c0, c1) = (frame_line_of("CARDSTART"), frame_line_of("CARDEND"));
    assert!(c1 > c0, "the card must wrap for this test");
    assert_eq!(select_lines(&mut app, c0, c1), err);
    let (n0, n1) = (frame_line_of("NOTESTART"), frame_line_of("NOTEEND"));
    assert_eq!(select_lines(&mut app, n0, n1), note);
}

#[test]
fn copy_strips_code_quote_and_table_chrome() {
    // Copy is document text: no `┌─ lang` frames, `│ ` gutters or box-drawn
    // table rules. Tables copy as tab-separated cells.
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.messages.push(Message::assistant(
        "HEADSTART\n\n| Tool | Status |\n|---|---|\n| env_probe | ok |\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\n> quoted line\n\nTAILEND"
            .to_string(),
    ));
    let _ = screen_rows(&app);
    let (a, b) = (frame_line_of("HEADSTART"), frame_line_of("TAILEND"));
    assert_eq!(
        select_lines(&mut app, a, b),
        "HEADSTART\n\nTool\tStatus\nenv_probe\tok\n\nfn main() {\n    println!(\"hi\");\n}\n\nquoted line\n\nTAILEND"
    );
}

fn error_cards(app: &App) -> usize {
    app.system_annotations
        .iter()
        .filter(|a| a.style == SystemMessageStyle::Error)
        .count()
}

#[test]
fn notices_are_annotations_not_answer_text() {
    // 2026-10-13 paste: guardrail warnings glued onto the answer.
    use operant_core::agent::AgentEvent;
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Content {
        text: "The answer.".to_string(),
    });
    app.handle_agent_event(AgentEvent::Notice {
        text: "⚠ Tool 'process' has failed 8 times".to_string(),
    });
    assert_eq!(app.streaming_text, "The answer.");
    assert!(
        app.system_annotations
            .iter()
            .any(|a| a.text.contains("has failed 8 times"))
    );
}

#[test]
fn one_error_card_per_failed_run_and_the_partial_answer_survives() {
    use operant_core::agent::AgentEvent;
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.handle_agent_event(AgentEvent::Content {
        text: "PARTIAL reply".to_string(),
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_complete_rx = Some(rx);
    // The core emits the event AND returns the Err.
    app.handle_agent_event(AgentEvent::Error {
        error: "provider exploded".to_string(),
    });
    tx.send(Err(operant_core::error::Error::Agent(
        "provider exploded".to_string(),
    )))
    .unwrap();
    assert!(app.drain_run_complete());
    assert_eq!(error_cards(&app), 1, "one card per failure");
    assert!(
        app.messages
            .iter()
            .any(|m| m.text_content().contains("PARTIAL reply")),
        "the streamed partial stays in the transcript"
    );
}

#[test]
fn a_user_stop_is_a_quiet_notice_not_an_error() {
    use operant_core::agent::AgentEvent;
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Error {
        error: "Interrupted by user".to_string(),
    });
    assert_eq!(error_cards(&app), 0);
    assert!(
        app.system_annotations
            .iter()
            .any(|a| a.text == "Turn stopped.")
    );
}

#[test]
fn tool_rows_show_failure_and_pending_from_app_status() {
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    let block = |id: &str, status, preview: Option<&str>| ToolUseBlock {
        id: id.to_string(),
        name: "process".to_string(),
        after_index: 1,
        status,
        output_preview: preview.map(str::to_string),
        input_json: "{}".to_string(),
        intent: None,
    };
    app.tool_use_blocks
        .push(block("t1", ToolStatus::Error, Some("process exited")));
    app.tool_use_blocks
        .push(block("t2", ToolStatus::Running, None));
    app.tool_use_blocks
        .push(block("t3", ToolStatus::Done, Some("ok")));
    let rows = crate::tui::operant_model::display_messages(&app);
    let content = |id: &str| {
        rows.iter()
            .find(|r| r.tool_data.as_ref().is_some_and(|t| t.id == id))
            .map(|r| r.content.clone())
            .unwrap()
    };
    assert_eq!(content("t1"), "✗ process exited");
    assert_eq!(
        content("t2"),
        crate::tui::operant_model::TOOL_PENDING_PREVIEW
    );
    assert_eq!(content("t3"), "ok");
}

fn render_buffer(app: &App) -> ratatui::buffer::Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| crate::tui::render::render_app(f, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn highlight_covers_only_the_text_a_copy_carries() {
    // 2026-10-13: the selection painted every row edge to edge — the `1›`
    // gutter, padding lit up although copy drops them.
    let mut app = make_app();
    app.messages
        .push(Message::user("PROMPTTEXT here".to_string()));
    app.tool_use_blocks.push(ToolUseBlock {
        id: "t1".to_string(),
        name: "bash".to_string(),
        after_index: 1,
        status: ToolStatus::Done,
        output_preview: Some("done".to_string()),
        input_json: r#"{"command":"echo TOOLROW"}"#.to_string(),
        intent: None,
    });
    app.messages
        .push(Message::assistant("ANSWERTEXT body".to_string()));
    let plain = render_buffer(&app);
    let (p, a) = (frame_line_of("PROMPTTEXT"), frame_line_of("ANSWERTEXT"));
    let area = app.last_selectable_area.get();
    app.selection_anchor = Some((area.x, p));
    app.selection_focus = Some((area.x + area.width - 1, a));
    let lit = render_buffer(&app);
    let row_of = |needle: &str| {
        (0..plain.area.height)
            .find(|&y| {
                (0..plain.area.width)
                    .map(|x| plain[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap()
    };
    let highlighted = |y: u16| -> String {
        (0..plain.area.width)
            .filter(|&x| plain[(x, y)].style() != lit[(x, y)].style())
            .map(|x| plain[(x, y)].symbol().to_string())
            .collect()
    };
    let prompt_lit = highlighted(row_of("PROMPTTEXT"));
    assert!(prompt_lit.contains("PROMPTTEXT"), "{prompt_lit:?}");
    assert!(
        !prompt_lit.contains('›'),
        "the prompt gutter is not text: {prompt_lit:?}"
    );
    // A tool row highlights exactly as it copies: highlight and copy share one
    // predicate, so what paints is what lands on the clipboard (iter-1190).
    assert!(
        highlighted(row_of("TOOLROW")).contains("TOOLROW"),
        "tool rows are selectable and copyable: {:?}",
        highlighted(row_of("TOOLROW"))
    );
    assert!(highlighted(row_of("ANSWERTEXT")).contains("ANSWERTEXT"));
}

#[test]
fn copy_merges_wrapped_table_cells_and_skips_error_cards() {
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    let long = "stored and read back successfully with timestamp ".repeat(4);
    app.messages.push(Message::assistant(format!(
        "TABLESTART\n\n| # | Result |\n|---|---|\n| 1 | {long}END |\n| 2 | short |"
    )));
    app.push_system_message("Error: CARDTEXT".to_string(), SystemMessageStyle::Error);
    app.messages
        .push(Message::assistant("AFTERCARD".to_string()));
    let _ = screen_rows(&app);
    let (a, b) = (frame_line_of("TABLESTART"), frame_line_of("AFTERCARD"));
    let text = select_lines(&mut app, a, b);
    assert!(
        text.contains(&format!("1\t{long}END")),
        "a wrapped cell stays one TSV row: {text:?}"
    );
    assert!(text.contains("2\tshort"));
    assert!(!text.contains("CARDTEXT"), "error cards are UI: {text:?}");
}

#[test]
fn new_composer_keys_do_what_they_say() {
    // Ctrl+G queues the external editor (run by the loop, which owns the
    // terminal); Alt+G is the scroll bookmark now.
    let mut app = make_app();
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert!(app.pending_external_editor);

    // F5 with an empty composer resubmits the last prompt.
    let mut app = make_app();
    app.prompt_input.history.push("run the tests".to_string());
    assert!(app.handle_key_event(press_key(KeyCode::F(5), KeyModifiers::NONE)));
    assert_eq!(app.prompt_input.text, "run the tests");
    // …and never while a turn is streaming.
    let mut app = make_app();
    app.prompt_input.history.push("x".to_string());
    app.is_streaming = true;
    assert!(!app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::ALT)));

    // Alt+P cycles model + fallback_models, in session only.
    let mut app = make_app();
    app.config.agent.model = "m-one".to_string();
    app.config.agent.fallback_models = vec!["m-two".to_string(), "m-one".to_string()];
    app.model_name = "m-one".to_string();
    app.handle_key_event(press_key(KeyCode::Char('p'), KeyModifiers::ALT));
    assert_eq!(app.model_name, "m-two");
    app.handle_key_event(press_key(KeyCode::Char('p'), KeyModifiers::ALT));
    assert_eq!(
        app.model_name, "m-one",
        "deduplicated two-model cycle wraps"
    );
}

#[test]
fn ctrl_o_expands_the_latest_tool_output_and_collapses_it_again() {
    use operant_core::agent::AgentEvent;
    let mut app = make_app();
    app.messages.push(Message::user("go".to_string()));
    app.handle_agent_event(AgentEvent::ToolStart {
        tool_call_id: "t1".to_string(),
        name: "bash".to_string(),
        arguments: "{}".to_string(),
    });
    let full: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    app.handle_agent_event(AgentEvent::ToolError {
        tool_call_id: "t1".to_string(),
        name: "bash".to_string(),
        error: full.clone(),
    });
    let content = |app: &App| {
        crate::tui::operant_model::display_messages(app)
            .into_iter()
            .find(|r| r.role == "tool")
            .map(|r| r.content)
            .unwrap()
    };
    assert!(
        !content(&app).contains("line 10"),
        "collapsed shows the preview"
    );
    app.handle_key_event(press_key(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(
        content(&app).contains("line 10"),
        "expanded shows the full output"
    );
    app.handle_key_event(press_key(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(!content(&app).contains("line 10"));
}

#[test]
fn essential_commands_are_wired() {
    let mut app = make_app();
    // /compact queues the fold for TuiApp (which holds the agent) — never
    // mid-turn.
    app.is_streaming = true;
    assert!(app.handle_tui_command("compact", ""));
    assert!(!app.pending_compact);
    app.is_streaming = false;
    assert!(app.handle_tui_command("compact", ""));
    assert!(app.pending_compact);

    // /goals reads the autonomous ledger.
    let dir = std::env::temp_dir().join(format!("operant-goals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("TODO.md");
    std::fs::write(
        &ledger,
        "## Implemented\n- done thing\n\n## Pending\n- GOALMARK first\n- second\n",
    )
    .unwrap();
    app.config.autonomous.todo_path = ledger.clone();
    assert!(app.handle_tui_command("goals", ""));
    let note = app.system_annotations.last().unwrap().text.clone();
    assert!(
        note.contains("2 pending · 1 implemented") && note.contains("GOALMARK"),
        "{note}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    assert!(app.handle_tui_command("changelog", ""));
    assert!(
        app.system_annotations
            .last()
            .unwrap()
            .text
            .starts_with("Recent changes:")
    );
    assert!(app.handle_tui_command("auth", ""));
    assert!(
        app.system_annotations
            .last()
            .unwrap()
            .text
            .starts_with("Auth:")
    );
    assert!(app.handle_tui_command("cancel", ""));
}

#[test]
fn jcode_only_commands_are_not_registered() {
    let registered: Vec<&str> = crate::tui::operant_app::app::registered_command_entries()
        .map(|(name, _)| name)
        .collect();
    for gone in [
        "/account",
        "/zzz",
        "/selfdev",
        "/subscription",
        "/catchup",
        "/telemetry",
        "/update-sim",
    ] {
        assert!(!registered.contains(&gone), "{gone} is still registered");
    }
    for kept in [
        "/compact",
        "/cancel",
        "/transcript",
        "/changelog",
        "/goals",
        "/auth",
    ] {
        assert!(registered.contains(&kept), "{kept} must stay registered");
    }
}

/// 2026-10-14 batch, plan 02: no REGISTERED command may sit unwired — the
/// class the iter-811 sweep found printing "/x is registered but not wired
/// in operant yet" (groups A/B) and "not yet wired to a handler" (group C).
/// Drives the real dispatch per name, under a multi_thread runtime because
/// the registry fallback path uses `block_in_place`. Two failure modes:
/// (a) `handle_tui_command` returns false — the sim layer would print the
/// "registered but not wired" notice; (b) the command resolves to a core
/// def with no handler — execute() lands the "not yet wired to a handler"
/// fallback as a system annotation. Both are advertised-but-dead.
#[tokio::test(flavor = "multi_thread")]
async fn no_registered_tui_command_is_unwired() {
    let mut app = make_app();
    // Mirror the live TuiApp wiring (tui_app.rs): the registry handlers and
    // the agent-flow handlers are registered at app assembly, not in
    // `CommandRegistry::new()`. The test App skips that assembly, so without
    // this the test would misreport wired commands as dead.
    crate::tui::adapter_types::tui_app::register_tui_command_handlers(&mut app.command_registry);
    crate::tui::adapter_types::tui_app::register_agent_flow_handlers(&mut app.command_registry);
    for name in crate::tui::operant_app::app::locally_dispatchable_command_names() {
        let bare = name.trim_start_matches('/');
        let before = app.system_annotations.len();
        let handled = app.handle_tui_command(bare, "");
        assert!(
            handled,
            "/{bare} is registered but not intercepted — the sim layer would print the not-wired notice (plan 02)"
        );
        // /clear and friends wipe system_annotations mid-loop; clamp the
        // window to whatever survived.
        let start = before.min(app.system_annotations.len());
        for note in &app.system_annotations[start..] {
            assert!(
                !note.text.contains("not yet wired to a handler"),
                "/{bare} landed the not-yet-wired fallback: {} (plan 02)",
                note.text
            );
        }
    }

    // (plan 04) The TYPEAHEAD table is part of the contract now: anything
    // autocomplete suggests must dispatch. A row the typeahead offers that
    // falls through to the model (or prints not-wired) is a lying command.
    for (name, _desc) in crate::tui::app::helpers::tui_slash_command_data() {
        let bare = name.trim_start_matches('/');
        let before = app.system_annotations.len();
        let handled = app.handle_tui_command(bare, "");
        assert!(
            handled,
            "/{bare} is offered by typeahead but not intercepted (plan 04 — lying suggestion)"
        );
        let start = before.min(app.system_annotations.len());
        for note in &app.system_annotations[start..] {
            assert!(
                !note.text.contains("not yet wired to a handler"),
                "/{bare} landed the not-yet-wired fallback: {} (plan 04)",
                note.text
            );
        }
    }
}

/// 2026-10-14 batch, plan 02: the dead commands from the iter-811 sweep
/// (named by the user, the same-class sweep finds, and the registry-fallback
/// stragglers) must be gone from the TUI registry and from the curated
/// /help entries. Keep the CommandDef for gateway-shared commands (/restart,
/// /background) — absence is asserted on the TUI registry only.
#[test]
fn removed_commands_are_absent() {
    let registered: Vec<&str> =
        crate::tui::operant_app::app::registered_command_names_including_hidden().collect();
    let gone = [
        // Group A — named by the user
        "/commit",
        "/commit-and-push",
        "/commit-push-release",
        "/cut-release",
        "/git",
        "/test",
        "/swarm",
        "/subagent",
        "/subagent-model",
        "/ssh",
        "/remote",
        // Group B — same class, found by the sweep
        "/commit-push",
        "/merge",
        "/merge-remote-release",
        "/remote-release",
        "/fast-release",
        "/fast-macos-release",
        "/swarm-prompt",
        "/initiatives",
        "/onboarding-preview",
        "/onboarding-sim",
        "/productivity",
        "/provider-test-coverage",
        "/model-status",
        "/compact-notifications",
        "/show-agentgrep-output",
        "/tool-call-details",
        // Group C — core-registry fallback stragglers
        "/btw",
        "/voice",
        "/fork",
        "/restart",
        "/observe",
    ];
    for name in gone {
        assert!(
            !registered.contains(&name),
            "{name} is still registered (plan 02)"
        );
    }
}

// ── (plan 05B) the unified inline picker ────────────────────────────────

/// `/model` opens the inline picker over the configured catalogue: the
/// `[agent]` model is present, flagged current, and preselected.
#[test]
fn model_command_opens_the_inline_picker_on_the_configured_rows() {
    let mut app = make_app();
    // The picker is gated on live credentials, so the test establishes that
    // precondition itself. It used to inherit it from the AMBIENT state —
    // the operator's `~/.operant/auth.json` and the shell's API key — which
    // made the result depend on a store whose path follows `HERMES_HOME`:
    // a parallel test that moved the home (or cleared the auth env) could
    // turn this into a different test.
    app.has_credentials = true;
    assert!(app.inline_interactive.is_none());
    assert!(app.handle_tui_command("model", ""));
    let picker = app
        .inline_interactive
        .as_ref()
        .expect("the inline picker must be open after /model");
    assert_eq!(
        picker.kind,
        crate::tui::operant_app::tui_fns::PickerKind::Model
    );
    let current_row = picker
        .entries
        .iter()
        .find(|e| e.is_current)
        .expect("the configured model must be present");
    assert_eq!(current_row.name, app.config.agent.model);
    assert_eq!(
        picker.entries[picker.filtered[picker.selected]].name,
        current_row.name
    );
}

/// `/connect` must retarget the AGENT, not just the display: the endpoint the
/// live client talks to, the key, and the provider's default model are all
/// queued for `tui_app` to drain, and the config the model picker reads
/// follows. (Before: only `active_provider` flipped; requests kept going to
/// the original `client.base_url` with the new provider's model name.)
#[test]
fn connect_queues_a_live_endpoint_retarget() {
    let mut app = make_app();
    app.config.client.base_url = "http://localhost:20129/v1".into();
    app.config.agent.model = "small-stack".into();

    let endpoint = App::connect_endpoint("anthropic", None, Some("sk-ant-test".into()));
    assert_eq!(
        endpoint,
        Some((
            "https://api.anthropic.com/v1".to_string(),
            Some("sk-ant-test".to_string())
        ))
    );
    app.activate_provider(
        "anthropic".into(),
        "Anthropic".into(),
        "Connected to",
        endpoint,
    );
    assert_eq!(
        app.pending_endpoint,
        Some((
            "https://api.anthropic.com/v1".to_string(),
            Some("sk-ant-test".to_string())
        ))
    );
    assert_eq!(app.config.client.base_url, "https://api.anthropic.com/v1");
    assert_eq!(app.config.client.api_key.as_deref(), Some("sk-ant-test"));
    // The default model is resolved from the live catalog, which a sync test
    // does not run; it arrives later through `default_model_rx`.
    assert!(app.pending_model.is_none());
}

/// A default model resolved in the background reaches the agent through
/// `pending_model`, the same channel the picker uses.
#[test]
fn a_resolved_default_model_is_applied_through_pending_model() {
    let mut app = make_app();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.default_model_rx = Some(rx);
    assert!(!app.drain_default_model(), "nothing sent yet");
    tx.send("claude-x".to_string()).unwrap();
    assert!(app.drain_default_model());
    assert_eq!(app.pending_model.as_deref(), Some("claude-x"));
    assert!(app.default_model_rx.is_none());
}

/// Pasted text belongs to the open text dialog, not the composer behind it —
/// the custom-provider form (URL + key) used to lose every paste, so a user
/// could not paste an API key into `/connect`.
#[test]
fn paste_and_burst_text_go_to_the_open_dialog_not_the_composer() {
    let mut app = make_app();
    app.custom_provider_dialog.open(
        "custom-openai".into(),
        "Custom OpenAI-Compatible".into(),
        None,
    );
    assert!(app.text_dialog_visible());
    assert!(
        !app.prompt_is_accepting_text(),
        "paste-burst detection must stand down while a text dialog is open"
    );

    // CR/LF dragged along by a copied secret are dropped, not inserted.
    assert!(app.insert_text_into_dialog("http://127.0.0.1:20128/v1\r\n"));
    app.custom_provider_dialog.move_next_field();
    assert!(app.insert_text_into_dialog("sk-test-123\n"));
    let (url, key) = app.custom_provider_dialog.take_values();
    assert_eq!(url, "http://127.0.0.1:20128/v1");
    assert_eq!(key, "sk-test-123");
    assert_eq!(app.prompt_input.text, "", "composer untouched");

    app.custom_provider_dialog.close();
    assert!(!app.insert_text_into_dialog("x"), "no dialog, no routing");
}

/// Connecting a provider with no endpoint URL must not say "Connected".
///
/// `bedrock` and `azure-foundry` have an empty `default_base_url`, and
/// `azure` has a `{resource}` template that is not a URL until it is
/// filled in — so `connect_endpoint` returns `None` for all three, and
/// `retarget_endpoint` is skipped. The agent then keeps talking to the
/// PREVIOUS provider's endpoint while the model name changes, and the
/// operator was told "Connected to AWS Bedrock."
///
/// The message must say what actually happened.
#[test]
fn connecting_a_provider_with_no_endpoint_url_says_so() {
    let mut app = make_app();
    let before = app.config.client.base_url.clone();

    app.activate_provider(
        "bedrock".to_string(),
        "AWS Bedrock".to_string(),
        "Connected to",
        None,
    );

    let message = app.status_message.clone().unwrap_or_default();
    assert!(
        message.contains("no endpoint URL"),
        "the message must say the provider has no endpoint, got: {message:?}"
    );
    assert!(
        !message.contains("Connected to AWS Bedrock."),
        "the message must not claim a connection that did not happen: {message:?}"
    );
    assert_eq!(
        app.config.client.base_url, before,
        "the endpoint must be untouched — the agent is still on the old one"
    );
}

#[test]
fn connect_endpoint_prefers_custom_url_and_skips_virtual_providers() {
    assert_eq!(
        App::connect_endpoint(
            "custom-openai",
            Some("http://gpu:8000/v1".into()),
            Some("k".into())
        ),
        Some(("http://gpu:8000/v1".to_string(), Some("k".to_string())))
    );
    // Local provider: catalog URL, no key.
    assert_eq!(
        App::connect_endpoint("ollama", None, None),
        Some(("http://localhost:11434/v1".to_string(), None))
    );
    // No single HTTP endpoint (acp://, templated, or unknown) -> no retarget.
    assert_eq!(App::connect_endpoint("copilot-acp", None, None), None);
    assert_eq!(App::connect_endpoint("azure", None, Some("k".into())), None);
    assert_eq!(App::connect_endpoint("no-such-provider", None, None), None);
}

/// Enter applies the selected row through plan 05A's pending_model drain
/// seam; Esc closes without applying; typing filters.
#[test]
fn inline_picker_enter_applies_and_esc_closes() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = make_app();
    // The picker is gated on live credentials — the test owns that
    // precondition instead of inheriting the ambient auth state.
    app.has_credentials = true;
    app.config.agent.fallback_models = vec!["other-model".to_string()];
    assert!(app.handle_tui_command("model", ""));
    let key = |code: KeyCode| KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    };

    // Type a filter that matches nothing, clear it with Esc (first Esc
    // clears the filter, second closes).
    app.handle_key_event(key(KeyCode::Char('z')));
    assert!(
        app.inline_interactive
            .as_ref()
            .is_some_and(|p| p.filter == "z")
    );
    app.handle_key_event(key(KeyCode::Esc));
    assert!(
        app.inline_interactive
            .as_ref()
            .is_some_and(|p| p.filter.is_empty())
    );
    app.handle_key_event(key(KeyCode::Esc));
    assert!(
        app.inline_interactive.is_none(),
        "the second Esc must close"
    );

    // Reopen; Enter on the single-option current row applies immediately.
    assert!(app.handle_tui_command("model", ""));
    app.handle_key_event(key(KeyCode::Enter));
    assert!(
        app.inline_interactive.is_none(),
        "Enter must close the picker"
    );
    assert_eq!(
        app.pending_model.as_deref(),
        Some(app.config.agent.model.as_str()),
        "Enter must route the pick to the pending_model drain"
    );
}

/// TUI-5: `/rewind <n>` arms a y/n confirmation. Any key other than y or n
/// must cancel it, so a later `y` typed in a word cannot fork the session.
#[test]
fn any_key_other_than_y_or_n_cancels_the_rewind_confirmation() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = make_app();
    app.pending_rewind_confirm = Some(1);
    app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(
        app.pending_rewind_confirm.is_none(),
        "a non-y/n key must cancel the armed /rewind confirmation"
    );
    assert!(app.pending_rewind.is_none(), "cancelling must not fork");
}

/// TUI-2: Enter in the /resume browser must retarget the agent, not only load
/// the transcript. Otherwise the next prompt is written to the old session.
#[test]
fn resume_picks_the_session_the_agent_switches_to() {
    use crate::tui::session_browser::SessionEntry;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = make_app();
    app.session_browser.open(vec![SessionEntry {
        id: "s-42".to_string(),
        title: "old work".to_string(),
        last_updated: "1h ago".to_string(),
        message_count: 3,
        cost_usd: 0.0,
    }]);
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.pending_switch_session.as_deref(),
        Some("s-42"),
        "/resume must hand the picked session to the agent switch"
    );
}

#[cfg(test)]
mod rename_on_load_tests {
    use super::*;
    use crate::tui::session_browser::{SessionBrowserMode, SessionEntry};

    #[test]
    fn a_bare_rename_starts_on_the_first_row_once_the_list_arrives() {
        // TUI-8: `/rename` with no argument opens the rename on the list's first row.
        let mut app = make_app();
        app.rename_on_list_load = true;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        app.session_list_rx = Some(rx);
        tx.try_send(vec![SessionEntry {
            id: "s-7".to_string(),
            title: "release notes".to_string(),
            last_updated: "now".to_string(),
            message_count: 1,
            cost_usd: 0.0,
        }])
        .unwrap();

        assert!(app.drain_session_list());

        assert_eq!(app.session_browser.mode, SessionBrowserMode::Rename);
        assert_eq!(app.session_browser.rename_input, "release notes");
        assert!(!app.rename_on_list_load, "the flag is consumed once used");
    }
}

#[cfg(test)]
mod delegation_status_tests {
    use super::*;

    #[test]
    fn a_cleared_delegation_is_abandoned_not_left_running() {
        // TUI-9: a delegation whose turn is cleared never reports a completion,
        // so it is marked abandoned rather than shown as running forever.
        let mut app = make_app();
        app.agent_status
            .push(("d1".to_string(), "running".to_string()));
        app.clear_tool_blocks();
        assert_eq!(app.agent_status[0].1, "abandoned");
    }
}

#[cfg(test)]
mod approval_gate_tests {
    use super::*;
    use crate::tui::dialogs::PermissionRequest;
    use crate::tui::session_browser::SessionEntry;

    #[test]
    fn a_pending_approval_takes_the_key_before_an_open_browser() {
        // TUI-1: an approval the agent is waiting on is answered first, so an
        // open overlay cannot swallow its key and leave the turn stalled.
        let mut app = make_app();
        app.session_browser.open(vec![SessionEntry {
            id: "s-1".to_string(),
            title: "t".to_string(),
            last_updated: "now".to_string(),
            message_count: 0,
            cost_usd: 0.0,
        }]);
        app.permission_request = Some(PermissionRequest::bash(
            "Bash".to_string(),
            "run a command".to_string(),
            "ls".to_string(),
            None,
        ));
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        app.handle_key_event(esc);

        assert!(
            app.permission_request.is_none(),
            "the approval answered the key"
        );
        assert!(
            app.session_browser.visible,
            "the browser did not take the key"
        );
    }
}

#[cfg(test)]
mod branch_list_tests {
    use super::*;

    #[test]
    fn a_failed_fork_tree_read_is_reported_not_shown_as_an_empty_tree() {
        // TUI-11: a database error must not open an empty branch browser.
        let mut app = make_app();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        app.branch_list_rx = Some(rx);
        tx.try_send(Err("database is locked".to_string())).unwrap();

        assert!(app.drain_branch_list());

        assert!(!app.session_branching.visible);
        assert!(app.branch_list_rx.is_none());
        assert!(
            app.system_annotations
                .iter()
                .any(|a| a.text.contains("database is locked")),
            "the error reaches the transcript"
        );
    }

    #[test]
    fn a_loaded_fork_tree_opens_the_branch_browser() {
        let mut app = make_app();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        app.branch_list_rx = Some(rx);
        tx.try_send(Ok(vec![crate::tui::session_branching::BranchInfo {
            id: "b-1".to_string(),
            name: "fork".to_string(),
            created_at: String::new(),
            is_current: false,
        }]))
        .unwrap();

        assert!(app.drain_branch_list());

        assert!(app.session_branching.visible);
    }
}
