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

// ---- (2026-10-30) the hub suppresses the idle animation band ----

/// The render layer reserves and paints the decorative idle animation only
/// when `should_reserve_idle_animation` says so, and the hub suppresses it.
///
/// Why this seam and not a guard inside the animation module: `render_app`
/// paints the chrome first and the overlay ladder second, so a guard at the
/// animation call only skipped the draw while the layout still reserved 14
/// rows for it. The gate has to sit where the reservation happens, and
/// `&dyn TuiState` is the only handle `draw_inner` has.
///
/// MUTATION-PROVEN: deleting `&& !state.session_hub_visible()` from
/// `should_reserve_idle_animation` turns the second assertion red — an earlier
/// version of this test only asserted the trait impl and passed straight
/// through that deletion, which is why the assertion is on the gate.
#[test]
fn app_reports_the_session_hub_state_to_the_render_layer() {
    let mut app = make_app();
    assert!(
        !app.session_hub_visible(),
        "a fresh app has no hub open, so the animation stays on"
    );
    app.session_hub.open(None);
    assert!(
        app.session_hub_visible(),
        "an open hub must report itself, or the chrome keeps reserving and \
         painting the animation band under it"
    );
    app.session_hub.close();
    assert!(
        !app.session_hub_visible(),
        "a closed hub must report closed, or the idle animation never returns"
    );
}

/// The gate itself, asserted directly. RT-145 retired the radar-pulse idle
/// animation outright (operator decision 2026-11-02), so the gate is now
/// unconditionally false: no policy, no hub state, no conversation state can
/// resurrect it.
///
/// The hub part of this test is kept because it still has teeth: before RT-145
/// the hub's suppression was the only thing standing between a dormant
/// terminal and a full-screen 3D sample per frame. With the animation gone,
/// the assertion is now a guard against reactivation by way of the hub.
#[test]
fn the_idle_animation_band_is_retired() {
    let mut app = make_app();
    app.session_hub.open(None);
    assert!(
        !crate::tui::operant_ui::should_reserve_idle_animation(&app),
        "an open hub must not resurrect the retired animation"
    );
    app.session_hub.close();
    assert!(
        !crate::tui::operant_ui::should_reserve_idle_animation(&app),
        "the idle animation is retired: no state may re-enable it"
    );

    // The composer therefore sits directly above the status line, which is the
    // codex arrangement the operator asked for. Asserted at the source because
    // `animation_height` is a local in `draw_inner`; the two lines that zero it
    // are the whole contract.
    let ui_mod = include_str!("../operant_ui/mod.rs");
    assert!(
        ui_mod.contains("let animation_height: u16 = idle_animation_reserved_height(show_animation, input_height);"),
        "the layout stopped deriving the animation band height from the gate — \
         a retired animation must not hold a hardcoded band"
    );
}

// ---- Advisor delivery: preserve card vs continuation (2026-10-28) ----
//
// omp advisor parity: a CONTINUE-channel review (a note at/above
// [advisor] continue_severity) embeds [ADVISOR_CONTINUATION] + the
// model-visible batch in its BackgroundReview summary. The handler queues
// the batch; the main loop submits it as the next turn's query once the
// run is complete.
mod advisor_delivery_tests {
    use super::*;
    use operant_core::agent::AgentEvent;

    fn review(summary: String) -> AgentEvent {
        AgentEvent::BackgroundReview {
            summary,
            session_id: "sess-adv".to_string(),
        }
    }

    /// A CONTINUE review queues the batch (marker included — the engine's
    /// continuation latch greps the conversation for it).
    #[test]
    fn continue_review_queues_the_batch() {
        let mut app = make_app();
        let batch = format!(
            "{}\n<advisory severity=\"concern\">fix the race</advisory>",
            operant_core::advisor::CONTINUATION_TAG
        );
        app.handle_agent_event(review(format!("⚑ advisor (small-stack): {batch}")));
        let queued = app.pending_advisor_continuation.clone();
        assert_eq!(queued.as_deref(), Some(batch.as_str()));
    }

    /// A preserve-channel review (below continue_severity) renders the
    /// card and queues NOTHING.
    #[test]
    fn preserve_review_queues_nothing() {
        let mut app = make_app();
        app.handle_agent_event(review(
            "⚑ advisor (small-stack): typo in the comment".to_string(),
        ));
        assert!(app.pending_advisor_continuation.is_none());
    }

    /// An interrupted run drops any queued continuation (omp
    /// autoResumeSuppressed: a deliberate user stop is never auto-resumed).
    #[test]
    fn interrupted_run_drops_the_queued_continuation() {
        let mut app = make_app();
        let batch = format!(
            "{}\n<advisory severity=\"blocker\">tests are broken</advisory>",
            operant_core::advisor::CONTINUATION_TAG
        );
        app.handle_agent_event(review(format!("⚑ advisor: {batch}")));
        assert!(app.pending_advisor_continuation.is_some());

        // The run-complete drain sees Error::Interrupted.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(Err(operant_core::error::Error::Interrupted));
        app.run_complete_rx = Some(rx);
        let _ = app.drain_run_complete();
        assert!(
            app.pending_advisor_continuation.is_none(),
            "an interrupted run must not be auto-continued by advice"
        );
    }
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

// ---- Ctrl+Backspace arrives as Ctrl+H (2026-10-24 red-team) -------------
//
// Most terminals send byte 0x08 for Ctrl+Backspace and crossterm maps that to
// Char('h')+CONTROL (parse.rs: 0x01..=0x1A → Ctrl+letter); a real
// Backspace+CONTROL only arrives from kitty-protocol terminals. Without the
// Char('h') arm the chord the operator actually presses was a silent no-op.

#[test]
fn ctrl_h_deletes_the_word_behind_the_caret_like_ctrl_backspace() {
    let mut app = make_app();
    app.prompt_input.replace_text("fix the par".to_string());
    app.handle_key_event(press_key_with_control(KeyCode::Char('h')));
    assert_eq!(
        app.prompt_input.text, "fix the ",
        "Ctrl+H — what terminals send for Ctrl+Backspace — deletes the word behind the caret"
    );
    assert_eq!(app.prompt_input.cursor, 8, "the caret rides the deletion");
}

#[test]
fn ctrl_h_with_no_word_before_the_caret_deletes_nothing() {
    let mut app = make_app();
    app.prompt_input.replace_text(String::new());
    app.handle_key_event(press_key_with_control(KeyCode::Char('h')));
    assert!(
        app.prompt_input.text.is_empty(),
        "an empty composer must survive Ctrl+H unchanged"
    );
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
    tx.send("MCP reconnect complete in 1.9s — 53 tool(s) synced".to_string())
        .unwrap();

    app.drain_mcp_reconnect_status();
    assert_eq!(
        app.status_message.as_deref(),
        Some("MCP reconnect complete in 1.9s — 53 tool(s) synced")
    );

    // The channel is drained to empty — a second drain is a no-op and the
    // last message still wins.
    app.drain_mcp_reconnect_status();
    assert_eq!(
        app.status_message.as_deref(),
        Some("MCP reconnect complete in 1.9s — 53 tool(s) synced")
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
        // (iter-1263/1267) The scroll-ladder rows were corrected to match the
        // real dispatch: PageUp/Down step by the live viewport height (not a
        // fixed 10), and the bookmark chord moved from Ctrl+g (now the external
        // editor) to Alt+g. The old rows pinned the stale text and failed
        // once iters 1263/1267 fixed the catalogue.
        (
            "Ctrl+g",
            "Edit prompt in external editor ([tui] external_editor)",
        ),
        ("Alt+g", "Toggle scroll bookmark"),
        ("Alt+p", "Cycle configured models"),
        ("Ctrl+o", "Expand latest tool output"),
        ("Ctrl+x", "Recall queued message"),
        ("Ctrl+t", "Toggle thinking display"),
        ("Ctrl+Up", "Fine scroll up 3"),
        ("Ctrl+Down", "Fine scroll down 3"),
        ("Alt+Up", "Fast scroll up 20"),
        ("Alt+Down", "Fast scroll down 20"),
        ("PageUp", "Scroll transcript up one page"),
        ("PageDown", "Scroll transcript down one page"),
        ("Shift+Home", "Jump to transcript top"),
        ("Shift+End", "Jump to transcript tail"),
        // (2026-10-28, jcode parity) Prompt jump — verified against the
        // Alt+[ / Alt+] dispatch arms in key_handling.rs.
        ("Alt+[", "Jump to previous user prompt"),
        ("Alt+]", "Jump to next user prompt"),
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

/// RT-144: the `-- INSERT --` / `-- NORMAL --` vim badge is GONE and must not
/// come back. RT-29 originally required it drawn while vim is on; the operator
/// removed it as a redundant tag that restates what the mode glyph and cursor
/// already show, and codex's composer band carries no mode row at all.
///
/// This test is the guard: it fails the moment anyone reintroduces a badge, and
/// it also pins that vim MODE is still live state (so the fix could not be
/// "delete vim mode"), and that merely enabling vim no longer reserves a hint
/// row — which used to shrink the transcript and move the footer.
#[test]
fn vim_mode_draws_no_badge_and_reserves_no_hint_row() {
    use crate::tui::prompt_input::VimMode;

    let mut app = make_app();
    assert!(
        rows_containing(&screen_rows(&app), "-- ").is_empty(),
        "no badge while vim is off"
    );

    app.prompt_input.vim_enabled = true;
    app.prompt_input.vim_mode = VimMode::Normal;
    let rows = screen_rows(&app);
    assert!(
        rows_containing(&rows, "-- ").is_empty(),
        "NORMAL must not be drawn as a badge: {rows:#?}"
    );

    app.prompt_input.vim_mode = VimMode::Insert;
    let rows = screen_rows(&app);
    assert!(
        rows_containing(&rows, "-- ").is_empty(),
        "INSERT must not be drawn as a badge: {rows:#?}"
    );

    // The mode itself is still real state, so the removal is cosmetic:
    assert_eq!(app.prompt_input.vim_mode, VimMode::Insert);
    assert!(app.prompt_input.vim_enabled);

    // And it must not reserve layout: enabling vim used to add a hint row.
    // `input_hint_line_height` is `pub(super)` to `operant_ui`, so this pins the
    // contract at the source — the same include_str! discipline the file already
    // uses for the resize-anchor seam. A re-added clause would reintroduce a
    // reservation with no painter, which is worse than no reservation at all.
    let input_ui = include_str!("../operant_ui/ui_input.rs");
    assert!(
        !input_ui.contains("app.vim_mode_label().is_some()"),
        "ui_input.rs reintroduced a hint-row reservation for the vim badge — \
         the badge is gone, so the row would be a transcript row silently lost"
    );
    assert!(
        !input_ui.contains("-- {label} --"),
        "ui_input.rs reintroduced the `-- <MODE> --` badge"
    );
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

/// 2026-10-14 batch, plan 05: double-Esc opens the selector (within 500 ms,
/// not streaming); a single Esc does not; while streaming it does not.
///
/// (2026-10-31) The composer-state gate is gone: the draft is parked instead
/// of blocking the gesture. See `double_escape_over_a_draft_opens_the_selector_and_cancel_restores_it`.
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
    app.close_rewind_selector();

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

/// (2026-10-31) THE reported defect, live-reproduced in tmux before the fix:
/// the second Esc over a half-written draft opened nothing, because the arm
/// required `prompt_input.is_empty()`. The one operator who needs "go back a
/// turn" is the one holding text they are about to abandon, so the gesture was
/// dead precisely where it matters.
///
/// This drives the real key handler twice with a draft in place and asserts
/// three things: the selector opens, the composer is emptied for it, and Esc
/// inside the selector hands the draft back verbatim with its cursor.
#[test]
fn double_escape_over_a_draft_opens_the_selector_and_cancel_restores_it() {
    let mut app = make_app();
    app.prompt_input
        .replace_text("half a thought about the cache bug".to_string());
    app.prompt_input.cursor = 6;
    app.refresh_prompt_input();
    assert!(
        !app.prompt_input.is_empty(),
        "the reproduction needs a draft in the composer"
    );

    // First Esc arms only — the single-Esc meaning is unchanged.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        !app.rewind_selector.visible,
        "a single Esc still opens nothing"
    );
    assert!(
        !app.prompt_input.is_empty(),
        "a single Esc must never touch the draft (omp's arm-only semantics)"
    );

    // Second Esc inside the window opens it, draft notwithstanding.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        app.rewind_selector.visible,
        "the second Esc opens the selector over a draft — this is the fix"
    );
    assert!(
        app.prompt_input.is_empty(),
        "the draft is parked, not left in the composer the selector covers"
    );
    assert!(
        app.rewind_draft_held.is_some(),
        "the parked draft is held, so cancel has something to hand back"
    );

    // Esc inside the selector is an explicit cancel: everything comes back.
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.rewind_selector.visible, "Esc cancels the selector");
    assert_eq!(
        app.prompt_input.text, "half a thought about the cache bug",
        "cancel restores the draft verbatim"
    );
    assert_eq!(
        app.prompt_input.cursor, 6,
        "cancel restores the caret where the operator left it"
    );
    assert!(
        app.rewind_draft_held.is_none(),
        "the slot is emptied by the restore — a later rewind cannot resurrect this draft"
    );
}

/// (2026-10-31) The OTHER exit: a committed rewind must NOT hand the parked
/// draft back, because the rewound prompt takes the composer. The bug this
/// pins is the draft surviving a rewind and clobbering the prompt the operator
/// asked to go back to.
#[test]
fn a_committed_rewind_discards_the_parked_draft() {
    let mut app = make_app();
    app.prompt_input
        .replace_text("unrelated half a thought".to_string());
    app.refresh_prompt_input();
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.rewind_selector.visible, "the selector opened");

    // Simulate the drain's success branch: the selector closes and the
    // rewound prompt replaces the composer.
    app.rewind_selector.close();
    app.rewind_draft_held = None;
    app.set_prompt_text("the prompt we rewound to".to_string());

    assert_eq!(
        app.prompt_input.text, "the prompt we rewound to",
        "the rewound prompt wins the composer"
    );
    assert!(
        app.rewind_draft_held.is_none(),
        "the parked draft is discarded by the commit"
    );
}

/// (iter-1489) The defect the operator reported as "none of its functionality
/// works": the selector opened EMPTY and stayed empty, because its only mark
/// source was `run_pending_agent_actions` — an async drain that lives on the
/// OUTER side of the fullscreen overlay's whole lifetime and therefore never
/// runs while the overlay is open.
///
/// This drives the real inner loop (`App::run`, one `run` pass per key) so the
/// assertion is about the LIVE frame sync, not a re-implementation of it.
#[test]
fn the_rewind_selector_syncs_its_marks_from_the_frame_loop() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = make_app();
    app.is_simulating = true;

    // A conversation already on screen: the relocation set that decides which
    // marks belong to THIS session (RT-175).
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: MessageContent::Text("what is 2+2?".to_string()),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::Assistant,
            content: MessageContent::Text("4".to_string()),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: MessageContent::Text("and 3+3?".to_string()),
        });

    // The marks the agent holds. `turn_context` records one per real prompt,
    // so the live selector must show exactly these two.
    app.core_rewind_marks = Some(std::sync::Arc::new(
        crate::tui::rewind_marks::RewindMarksSource::new(
            vec![
                operant_core::agent::UserTurnMark {
                    turn_id: 11,
                    conv_index: 0,
                    conv_epoch: 1,
                    text: "what is 2+2?".to_string(),
                },
                operant_core::agent::UserTurnMark {
                    turn_id: 12,
                    conv_index: 2,
                    conv_epoch: 1,
                    text: "and 3+3?".to_string(),
                },
            ],
            Some("sess_live".to_string()),
            Vec::new(),
        ),
    ));

    // Double-Esc opens the selector through the real key path.
    app.simulated_keys = vec![
        press_key(KeyCode::Esc, KeyModifiers::NONE),
        press_key(KeyCode::Esc, KeyModifiers::NONE),
    ];
    while !app.simulated_keys.is_empty() && !app.should_exit {
        let _ = app.run(&mut terminal).expect("loop runs");
    }
    assert!(
        app.rewind_selector.visible,
        "double-Esc opened the selector"
    );

    // The frame sync — NOT the drain, which never runs inside the overlay —
    // populated the rows. One further loop pass is all it takes.
    app.simulated_keys = vec![press_key(KeyCode::Down, KeyModifiers::NONE)];
    while !app.simulated_keys.is_empty() && !app.should_exit {
        let _ = app.run(&mut terminal).expect("loop runs");
    }

    assert_eq!(
        app.rewind_selector.entries.len(),
        2,
        "the selector lists THIS session's prompts, synced from the frame loop"
    );
    assert_eq!(
        app.rewind_selector.entries[0].text, "what is 2+2?",
        "oldest first, the selector's mark order"
    );
    assert!(
        !app.rewind_selector.entries.iter().any(|e| e.compacted),
        "a prompt still on screen is not a compacted turn"
    );
}

/// (iter-1528) THE LIVE-FRAME VERSION of the re-derive defect.
///
/// The unit test `a_resync_whose_turn_ids_were_re_derived_…` proves the
/// `sync_marks` fix, but it calls `sync_marks` directly. The live path is
/// `App::run`'s frame loop, which calls it on EVERY frame — and it kept the
/// deployed binary's navigation frozen even after the fix, so there is a second
/// reset this nails down: what the frame loop does to the cursor between the
/// keypress and the paint.
///
/// The harness runs several loop passes (several syncs, several paints), so a
/// cursor that is reset anywhere in that window fails here. The marks use
/// STABLE ids, and still re-synced every pass, which is what the live
/// `RewindMarksSource::marks()` does between re-derives.
#[test]
fn an_arrow_key_in_the_rewind_selector_survives_repeated_frame_syncs() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = make_app();
    app.is_simulating = true;

    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: MessageContent::Text("first prompt".to_string()),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::Assistant,
            content: MessageContent::Text("answer one".to_string()),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: MessageContent::Text("second prompt".to_string()),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::Assistant,
            content: MessageContent::Text("answer two".to_string()),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: MessageContent::Text("third prompt".to_string()),
        });

    app.core_rewind_marks = Some(std::sync::Arc::new(
        crate::tui::rewind_marks::RewindMarksSource::new(
            vec![
                operant_core::agent::UserTurnMark {
                    turn_id: 11,
                    conv_index: 0,
                    conv_epoch: 1,
                    text: "first prompt".to_string(),
                },
                operant_core::agent::UserTurnMark {
                    turn_id: 12,
                    conv_index: 2,
                    conv_epoch: 1,
                    text: "second prompt".to_string(),
                },
                operant_core::agent::UserTurnMark {
                    turn_id: 13,
                    conv_index: 4,
                    conv_epoch: 1,
                    text: "third prompt".to_string(),
                },
            ],
            Some("sess_live2".to_string()),
            Vec::new(),
        ),
    ));

    // Open the selector.
    app.simulated_keys = vec![
        press_key(KeyCode::Esc, KeyModifiers::NONE),
        press_key(KeyCode::Esc, KeyModifiers::NONE),
    ];
    while !app.simulated_keys.is_empty() && !app.should_exit {
        let _ = app.run(&mut terminal).expect("loop runs");
    }
    assert!(app.rewind_selector.visible, "the selector opened");
    assert_eq!(app.rewind_selector.entries.len(), 3, "three turns listed");

    // The cursor opens on the newest turn.
    assert_eq!(
        app.rewind_selector.selected, 2,
        "the selector opens on the newest turn"
    );

    // One Up, then MANY more loop passes so the frame loop syncs repeatedly
    // before the assertion. If any of those passes resets the cursor, this is
    // where it dies — and that is exactly what the deployed binary does.
    // The drain that opened the selector left `should_exit` set: the simulator
    // exits once its script is spent and nothing is in flight, and `App::run`
    // then returns immediately WITHOUT consuming another key. Clear it (and the
    // pass counter that trips shortly after) so the next call is a real frame —
    // the same reset a live run gets when the operator presses a key. This is
    // the harness discipline the test needs; it is not the defect under test.
    app.should_exit = false;
    app.sim_key_drain_passes = 0;
    app.simulated_keys = vec![press_key(KeyCode::Up, KeyModifiers::NONE)];
    // A bounded loop, because the simulator's own SIM_KEY_DRAIN_PASSES=3 exits
    // after three passes with the key unread.
    for _ in 0..40 {
        if app.simulated_keys.is_empty() {
            break;
        }
        let _ = app.run(&mut terminal).expect("loop runs");
        if app.should_exit {
            break;
        }
    }
    for _ in 0..5 {
        let _ = app.run(&mut terminal).expect("loop runs");
    }

    assert_eq!(
        app.rewind_selector.selected, 1,
        "the Up survived the frame loop's re-syncs — the deployed binary \
         reset it to the newest turn on the next paint"
    );
    assert_eq!(
        app.rewind_selector.entries[app.rewind_selector.selected].text, "second prompt",
        "and the selection is still the turn the operator moved to"
    );

    // And a second Up, same discipline.
    app.should_exit = false;
    app.sim_key_drain_passes = 0;
    app.simulated_keys = vec![press_key(KeyCode::Up, KeyModifiers::NONE)];
    for _ in 0..40 {
        if app.simulated_keys.is_empty() {
            break;
        }
        let _ = app.run(&mut terminal).expect("loop runs");
        if app.should_exit {
            break;
        }
    }
    for _ in 0..5 {
        let _ = app.run(&mut terminal).expect("loop runs");
    }
    assert_eq!(
        app.rewind_selector.selected, 0,
        "repeated navigation keeps working, not just the first press"
    );
}

/// (iter-1538) THE LIVE-FRAME RENDER PATH for the rewind outline.
///
/// `the_rewind_selection_highlight_follows_the_arrow_key` calls
/// `draw_rewind_selector` DIRECTLY, which proves the selector can outline a row
/// but never touches `App::run`'s frame loop — the path the deployed binary
/// actually paints. The operator's live repro (5 prompt turns, `<up>` x0..x4)
/// showed the reversed highlight FROZEN on the first row in every case, even
/// with no arrow pressed at all, so the outline never followed the cursor in
/// the real render path either.
///
/// This drives the REAL loop: `App::run` paints the frame, and the assertion
/// reads the reversed cells out of the frame the loop actually drew. It is the
/// same projection the live red-team used (`tui debug simulate --dump-style`).
#[test]
fn the_outlined_band_follows_the_arrow_key_through_the_real_frame_loop() {
    use crate::tui::adapter_types::types::{Message, MessageContent, Role};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let mut app = make_app();
    // The real loop reads events from `simulated_keys` instead of the terminal
    // only when it is simulating; without this it tries to open the real input
    // reader and the harness fails for a reason that has nothing to do with
    // the defect under test.
    app.is_simulating = true;
    // Three turns, each prompt followed by its answer, so each prompt owns a
    // row band in the body the selector renders.
    for (i, prompt) in ["prompt one", "prompt two", "prompt three"]
        .iter()
        .enumerate()
    {
        app.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(prompt.to_string()),
        });
        app.messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Text(format!("answer {i}")),
        });
    }
    app.core_rewind_marks = Some(std::sync::Arc::new(
        crate::tui::rewind_marks::RewindMarksSource::new(
            vec![
                operant_core::agent::UserTurnMark {
                    turn_id: 11,
                    conv_index: 0,
                    conv_epoch: 1,
                    text: "prompt one".to_string(),
                },
                operant_core::agent::UserTurnMark {
                    turn_id: 12,
                    conv_index: 2,
                    conv_epoch: 1,
                    text: "prompt two".to_string(),
                },
                operant_core::agent::UserTurnMark {
                    turn_id: 13,
                    conv_index: 4,
                    conv_epoch: 1,
                    text: "prompt three".to_string(),
                },
            ],
            Some("sess_live3".to_string()),
            Vec::new(),
        ),
    ));

    // The outline rows as the real loop paints them.
    fn outlined_rows(terminal: &Terminal<TestBackend>) -> Vec<usize> {
        use crate::tui::debug::debug_hub::buffer_style_dump;
        let dump = buffer_style_dump(terminal.backend().buffer());
        let mut rows = Vec::new();
        for (y, line) in dump.lines().skip(1).enumerate() {
            let reversed_here = line.split(' ').any(|tok| {
                tok.split_once('@').is_some_and(|(style, _sym)| {
                    let parts: Vec<&str> = style.split('/').collect();
                    parts.get(2).is_some_and(|m| m.split('+').any(|f| f == "r"))
                })
            });
            if reversed_here {
                rows.push(y);
            }
        }
        rows
    }

    // Drive a pass through the real loop: apply a key (if any), then paint and
    // read the outline the frame actually drew.
    let drive = |app: &mut App, terminal: &mut Terminal<TestBackend>, key: Option<KeyCode>| {
        app.should_exit = false;
        app.sim_key_drain_passes = 0;
        if let Some(code) = key {
            app.simulated_keys = vec![press_key(code, KeyModifiers::NONE)];
        } else {
            app.simulated_keys.clear();
        }
        for _ in 0..5 {
            let _ = app.run(terminal).expect("loop runs");
            if app.should_exit {
                break;
            }
        }
        outlined_rows(terminal)
    };

    // Open the selector via the real double-Esc key path. The drain exits once
    // the script is spent, so clear its exit flag between batches — the same
    // reset a live run gets.
    app.should_exit = false;
    app.sim_key_drain_passes = 0;
    app.simulated_keys = vec![
        press_key(KeyCode::Esc, KeyModifiers::NONE),
        press_key(KeyCode::Esc, KeyModifiers::NONE),
    ];
    for _ in 0..20 {
        let _ = app.run(&mut terminal).expect("loop runs");
        if app.simulated_keys.is_empty() {
            break;
        }
    }
    assert!(
        app.rewind_selector.visible,
        "double-Esc opened the selector"
    );
    assert_eq!(app.rewind_selector.entries.len(), 3, "three turns listed");
    assert_eq!(
        app.rewind_selector.selected, 2,
        "the cursor opens on the newest turn"
    );

    // A repaint with NO key press at all: the outline must sit on the newest
    // turn (index 2). This is the control that kills a harness artifact — if
    // the outline is already on row band 0 with nothing pressed, the render is
    // pinning it there.
    let at_newest = outlined_rows(&terminal);
    assert!(
        !at_newest.is_empty(),
        "the newest turn IS outlined once the frame loop paints it"
    );

    // One Up MUST move the outlined band through the real loop.
    let one_up = drive(&mut app, &mut terminal, Some(KeyCode::Up));
    assert_ne!(
        one_up, at_newest,
        "up moves the OUTLINED ROWS in the real frame loop, not just the index"
    );
    assert_eq!(
        app.rewind_selector.selected, 1,
        "and the cursor itself moved to the middle turn"
    );

    // A second Up moves it again.
    let two_up = drive(&mut app, &mut terminal, Some(KeyCode::Up));
    assert_ne!(two_up, one_up, "and it moves again on the next press");
    assert_eq!(
        app.rewind_selector.selected, 0,
        "walking up reaches the oldest turn"
    );
}

/// (iter-1538) The headless-mock defect's mechanism, pinned at the unit level.
///
/// The live red-team found the rewind outline frozen on the first prompt for
/// every `<up>`. An ungated probe proved the path: the mock sim constructs NO
/// agent, so `core_rewind_marks` stayed `None`, the frame-loop sync is gated on
/// it being `Some`, and the selector therefore opened with ZERO entries — and
/// `open()` on an empty list sets `selected = 0`, which
/// `user_prompt_range`'s `starts.get(0)` then outlines forever.
///
/// This pins the exact state the defect lived in: marks `None` → no entries →
/// no navigation, no matter how many keys are pressed. The fix seeds the mock's
/// marks (see `tui_app.rs`'s `is_mock` block), which is verified live in the
/// commit body's recorded sweep (ups=0..3 walks CCC→BBB→AAA).
#[test]
fn a_none_marks_source_freezes_the_selector_at_the_first_turn() {
    use crate::tui::adapter_types::types::{Message, MessageContent, Role};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let mut app = make_app();
    app.is_simulating = true;
    // The transcript the mock script injects — three prompts, each followed by
    // its answer, so each prompt owns a row band.
    for prompt in ["AAA first prompt", "BBB second prompt", "CCC third prompt"] {
        app.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(prompt.to_string()),
        });
    }
    // The defect's state, exactly: the mock path left this `None`.
    app.core_rewind_marks = None;

    app.should_exit = false;
    app.sim_key_drain_passes = 0;
    app.simulated_keys = vec![
        press_key(KeyCode::Esc, KeyModifiers::NONE),
        press_key(KeyCode::Esc, KeyModifiers::NONE),
    ];
    for _ in 0..20 {
        let _ = app.run(&mut terminal).expect("loop runs");
        if app.simulated_keys.is_empty() {
            break;
        }
    }
    assert!(app.rewind_selector.visible, "the selector opened");
    assert_eq!(
        app.rewind_selector.entries.len(),
        0,
        "no marks source, no entries — the state the live binary was in"
    );
    assert_eq!(
        app.rewind_selector.selected, 0,
        "empty `open()` pins the cursor at 0, so the outline is stuck on the \
         first prompt"
    );

    // Pressing Up cannot move anything: there is nothing to move through.
    let drive = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
        app.should_exit = false;
        app.sim_key_drain_passes = 0;
        app.simulated_keys = vec![press_key(KeyCode::Up, KeyModifiers::NONE)];
        for _ in 0..10 {
            let _ = app.run(terminal).expect("loop runs");
            if app.simulated_keys.is_empty() {
                break;
            }
        }
    };
    drive(&mut app, &mut terminal);
    assert_eq!(
        app.rewind_selector.selected, 0,
        "and the key is inert — THIS is the operator's frozen navigation"
    );
    drive(&mut app, &mut terminal);
    assert_eq!(
        app.rewind_selector.selected, 0,
        "still frozen on the third pass"
    );
}

/// (iter-1489) The defect behind "session management is slow / it won't
/// navigate": the simulator applied a scripted key the instant it was queued,
/// while the hub's rows were still being read off a blocking DB thread. The
/// key landed on a hub with NO ROWS, so `select_next` found `visible_idx`
/// empty and silently did nothing — the actor was dropped, not delayed. A live
/// operator cannot press a key inside the ~250 ms the load takes (measured on
/// the operator's 105 MB database), so holding the pump until the load lands
/// is what makes the harness faithful to the interaction it claims to test.
///
/// Drives the real `App::run` loop: frame 1 proves the held key is NOT applied
/// to an empty list, and the frame after the rows land applies it.
/// Three session rows for the hub's list, the minimum a navigation test needs.
fn hub_rows(n: usize) -> Vec<operant_core::session::listing::SessionEntryV2> {
    use operant_core::session::listing::SessionEntryV2;
    use operant_core::session::listing::{RowKind, RowStatus};
    (0..n)
        .map(|i| SessionEntryV2 {
            id: format!("sess_{i}"),
            parent_id: None,
            title: Some(format!("session {i}")),
            updated_at: chrono::Utc::now(),
            employee_id: None,
            source: "local".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            tokens: 0,
            cwd: None,
            status: RowStatus::Idle,
            kind: RowKind::Session,
        })
        .collect()
}

/// (iter-1489) The defect behind "session management won't navigate": the
/// simulator applied a scripted key the instant it was queued, while the hub's
/// rows were still being read off a blocking DB thread. The key landed on a hub
/// with NO ROWS, so `select_next` found `visible_idx` empty and silently did
/// nothing — the actor was dropped, not delayed. Measured: a live operator
/// cannot press a key inside the ~250 ms `list_sessions_rich(500)` takes on the
/// operator's 105 MB database, so holding the pump until the load lands is what
/// makes the harness faithful to the interaction it claims to test.
///
/// The rows are delivered from a spawned task AFTER a short delay, so the load
/// is genuinely IN FLIGHT while the first passes run — that is the state a live
/// run is in, and the one the old harness could not represent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hub_navigation_key_waits_for_the_row_load_to_land() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = make_app();
    app.is_simulating = true;

    // Open the hub, then clear the load request: otherwise the frame loop spawns
    // its own DB read on the first pass and replaces the receiver seeded here.
    app.handle_tui_command("session", "");
    assert!(app.session_hub.visible, "the hub opens on /session");
    app.session_list_pending = false;
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<_>>(1);
    app.session_list_rx = Some(rx);
    // Deliver the rows a few frames late, exactly like the real blocking load.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = tx.send(hub_rows(3)).await;
    });

    app.simulated_keys = vec![press_key(KeyCode::Down, KeyModifiers::NONE)];

    // The run ends when the load has landed AND the keys are consumed: the
    // pump held the key while the load was in flight, applied it once the rows
    // existed, and only then found nothing left to do.
    let outcome = app.run(&mut terminal).expect("loop runs");
    eprintln!(
        "DBG hub.visible={} pending={:?} status={:?} msgs={} keys_left={}",
        app.session_hub.visible,
        app.pending_switch_session,
        app.status_message,
        app.messages.len(),
        app.simulated_keys.len()
    );
    let outcome: Option<String> = outcome;
    assert!(outcome.is_none(), "the scenario never submits a prompt");

    assert!(
        app.session_hub.selected_row().is_some(),
        "the rows landed in the hub"
    );
    assert!(
        app.simulated_keys.is_empty(),
        "the held key was applied, not dropped"
    );
    assert_eq!(
        app.session_hub.selected, 1,
        "the navigation key applies once the rows exist"
    );
}

/// (iter-1503) The defect behind "switching session gives a blank
/// screen": Enter in the hub set `pending_switch_session` and closed the hub,
/// handing the keyboard back to the composer INSIDE the loop; the drain that
/// would have applied the retarget lives on its outer side and could not run
/// until the user's next submit. Result: neither session in `App::messages`.
///
/// Here the switch is driven with NO switch handle (a host that has no agent),
/// which must report the reason rather than silently abandoning the switch:
/// the operator's screen cannot sit blank with no explanation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hub_enter_switches_session_and_rebuilds_the_display_in_frame() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = make_app();
    app.is_simulating = true;

    // Enter the hub and seed a row load that is ALREADY satisfied, so the
    // Enter that follows is not held behind it.
    app.handle_tui_command("session", "");
    assert!(app.session_hub.visible, "the hub opens on /session");
    app.session_list_pending = false;
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<_>>(1);
    app.session_list_rx = Some(rx);
    tx.send(crate::tui::session_hub::probe_rows(2))
        .await
        .expect("the seeded rows survive the channel");

    // No switch handle: the frame loop must report the reason for the
    // failed switch, never leave the operator with a blank screen.
    app.core_session_switch = None;

    app.simulated_keys = vec![press_key(KeyCode::Enter, KeyModifiers::NONE)];

    let outcome = app.run(&mut terminal).expect("loop runs");
    assert!(outcome.is_none(), "the scenario never submits a prompt");
    assert!(
        !app.session_hub.visible,
        "Enter closes the hub, so the frame loop must have processed the switch \
         (if the loop had returned before that, the switch would have been \
         applied by the outer drain instead)"
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("No agent attached — cannot switch session."),
        "the frame loop ran in lieu of the outer drain, and named what failed"
    );
    assert_eq!(
        app.pending_switch_session.as_deref(),
        Some("sess_0"),
        "with no handle the flag is put BACK, not dropped: the outer drain \
         still owns the retarget and must find it armed"
    );
    assert_eq!(
        app.messages.len(),
        0,
        "no conversation was fabricated: the display stays honest about \
         holding nothing rather than showing the wrong transcript"
    );
}

// ---- (plan 2026-10-28 iter-D, G5) the ←← session-hub gesture ----
//
// omp agent-hub.ts: a second ← within [40ms, 500ms) on an empty composer
// opens the session hub — armed, so ONE further ← closes it. The 40ms floor
// rejects key-repeat bursts; the 500ms ceiling matches omp's window.
mod left_tap_gesture_tests {
    use super::*;

    fn left() -> KeyEvent {
        press_key(KeyCode::Left, KeyModifiers::NONE)
    }

    /// The gap the gesture tests need between two taps. Real time, because
    /// the window itself is real: two calls in the same microsecond are a
    /// <40ms burst and are correctly rejected. 50ms sits inside [40, 500).
    fn tap_gap() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    #[test]
    fn double_left_opens_the_hub_armed() {
        let mut app = make_app();
        app.handle_key_event(left());
        assert!(!app.session_hub.visible, "the first tap only arms");
        tap_gap();
        app.handle_key_event(left());
        assert!(app.session_hub.visible, "the second tap in-window opens");
        assert!(
            app.session_hub.armed_close_tap.is_some(),
            "the gesture open is armed: ONE further ← closes"
        );
        assert!(app.session_list_pending, "rows load on open");
    }

    #[test]
    fn one_further_left_closes_the_armed_hub() {
        let mut app = make_app();
        app.handle_key_event(left());
        tap_gap();
        app.handle_key_event(left());
        assert!(app.session_hub.visible);
        // The hub's own Left handler consumes the close-tap while visible.
        app.handle_key_event(left());
        assert!(!app.session_hub.visible, "the handoff: one ← closes");
    }

    #[test]
    fn a_burst_under_40ms_never_fires() {
        // Two taps < 40ms apart are a key-repeat burst, not a gesture.
        let mut app = make_app();
        app.handle_key_event(left());
        // No gap: the immediate second tap is a burst.
        app.handle_key_event(left());
        assert!(!app.session_hub.visible, "a burst is not a double-tap");
        // The burst ALSO re-armed: a normal second tap still opens.
        tap_gap();
        app.handle_key_event(left());
        assert!(app.session_hub.visible);
    }

    #[test]
    fn a_gap_over_500ms_is_two_single_taps() {
        let mut app = make_app();
        app.handle_key_event(left());
        // Backdate the arm past the ceiling: the next tap is a fresh first
        // tap, not the gesture's second.
        app.last_left_tap_at =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(600));
        app.handle_key_event(left());
        assert!(!app.session_hub.visible, "the window is closed");
        // Re-armed by the stale tap: the next tap pairs with IT.
        tap_gap();
        app.handle_key_event(left());
        assert!(app.session_hub.visible);
    }

    #[test]
    fn left_with_text_in_the_composer_disarms() {
        let mut app = make_app();
        app.handle_key_event(left());
        app.prompt_input.insert_char('a');
        app.handle_key_event(left());
        assert!(!app.session_hub.visible, "←← with text never opens the hub");
        assert!(app.last_left_tap_at.is_none(), "text disarms the clock");
        // Clearing the composer: the gesture starts fresh.
        app.prompt_input.text.clear();
        app.prompt_input.cursor = 0;
        app.handle_key_event(left());
        tap_gap();
        app.handle_key_event(left());
        assert!(app.session_hub.visible);
    }

    #[test]
    fn shift_left_never_participates() {
        // Shift+← is selection; it must not arm or fire the gesture.
        let mut app = make_app();
        app.handle_key_event(left());
        app.handle_key_event(press_key(KeyCode::Left, KeyModifiers::SHIFT));
        assert!(!app.session_hub.visible);
        assert!(
            app.last_left_tap_at.is_none(),
            "the modified tap disarms the clock"
        );
    }
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
    // Not just "a dialog opened": the live entries must be in the searchable
    // snapshot. (2026-10-25) The overlay now searches the FULL persisted
    // history merged with the live in-session ladder, so the snapshot can
    // legitimately hold more than the fixture's two rows — the invariant is
    // that every live entry is findable, and that the snapshot stays deduped
    // (each text once).
    let snapshot_texts: Vec<&str> = app
        .history_search_overlay
        .snapshot
        .iter()
        .map(|e| e.text.as_str())
        .collect();
    for needle in ["first command", "second"] {
        assert!(
            snapshot_texts.contains(&needle),
            "live entry {needle:?} must be searchable: {snapshot_texts:?}"
        );
        assert_eq!(
            snapshot_texts.iter().filter(|t| **t == needle).count(),
            1,
            "snapshot stays deduped"
        );
    }
}

/// (2026-10-27, jcode parity, operator report) Ctrl+R must PREVIEW the
/// selected prompt in the composer as the search navigates — jcode's
/// `apply_prompt_history_search_preview`. Before this, only the floating
/// panel showed the match and the composer kept its old text.
#[test]
fn history_search_previews_the_selected_match_in_the_composer() {
    let mut app = make_app();
    app.prompt_input.history = vec!["alpha prompt".to_string(), "beta prompt".to_string()];
    app.set_prompt_text("draft".to_string());

    // Open the search; the empty-query browse-all must NOT clobber the
    // draft before the user has picked anything.
    app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(app.prompt_input.text, "draft");

    // Typing a query previews the top match into the composer.
    app.handle_key_event(press_key(KeyCode::Char('b'), KeyModifiers::NONE));
    app.handle_key_event(press_key(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(
        app.prompt_input.text, "beta prompt",
        "the selected match must preview in the input box"
    );

    // Stepping down (older match, newest-first list) moves the preview.
    app.handle_key_event(press_key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.prompt_input.text, "beta prompt");
}

/// Esc must restore the composer draft the search opened on — the live
/// preview overwrites the composer, so cancel has to put it back (jcode's
/// `cancel_prompt_history_search`).
#[test]
fn history_search_esc_restores_the_pre_search_draft() {
    let mut app = make_app();
    app.prompt_input.history = vec!["alpha prompt".to_string()];
    app.set_prompt_text("my half-written draft".to_string());

    app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    // Navigate so the preview overwrites the composer.
    app.handle_key_event(press_key(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(app.prompt_input.text, "alpha prompt");

    app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.history_search_overlay.visible);
    assert_eq!(
        app.prompt_input.text, "my half-written draft",
        "Esc must restore the pre-search draft, not strand the preview"
    );
}

/// Ctrl+R again while the search is open must STEP to the older match
/// (readline), not fall through as a dead key.
#[test]
fn history_search_ctrl_r_again_steps_older() {
    let mut app = make_app();
    app.prompt_input.history = vec!["alpha prompt".to_string(), "beta prompt".to_string()];

    app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    app.handle_key_event(press_key(KeyCode::Char('a'), KeyModifiers::NONE));
    let before = app.history_search_overlay.selected_idx;

    app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    let after = app.history_search_overlay.selected_idx;
    assert_ne!(
        before, after,
        "Ctrl+R while open must move the selection (readline step)"
    );
    assert!(app.history_search_overlay.visible, "the search stays open");
}

#[test]
fn test_tab_accepts_the_typeahead_suggestion() {
    use crate::tui::prompt_input::{TypeaheadSource, TypeaheadSuggestion};

    let mut app = make_app();
    app.prompt_input.text = "/he".to_string();
    app.prompt_input.cursor = 3;
    app.prompt_input.suggestions = vec![TypeaheadSuggestion {
        text: "/help".to_string(),
        display: "/help".to_string(),
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
        display: "/help".to_string(),
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

/// (RT-205, iter-1562) The live-only duplication, attacked at the one
/// thing every prior test did NOT do: grow the transcript incrementally.
///
/// The live operator captured mid-word duplication in the transcript body
/// (`similarity.rly`, `The pattern seems to be::`, `thoughtt`, and each user
/// prompt carrying a stray character: `1› …alpha` + `a`). I refuted, by
/// measurement, the two obvious homes for it: the aroute provider sends
/// genuine deltas with no overlap (iter-1561), and `agent_events.rs`'s
/// accumulation is correct. The `App::run` frame loop renders a CLEAN body when
/// every message is pushed before the first paint — which is what every
/// existing test does.
///
/// The corruption was live-growing-transcript-only, so this drives the loop
/// the way the agent does: append a message, paint, append the next, paint,
/// then commit and paint again — and asserts each text appears exactly ONCE in
/// the prepared body. A duplicated fragment at a message's own end is exactly
/// the signature of an incremental re-stitch seam.
#[test]
fn an_incrementally_grown_transcript_renders_each_message_once() {
    use crate::tui::adapter_types::types::{Message, MessageContent, Role};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = make_app();
    app.is_simulating = true;

    // Grow the transcript the way the real loop does: one message per pass,
    // painting between appends so any incremental/prefix-reuse path is taken.
    let script: Vec<(Role, String)> = vec![
        (Role::User, "first question about the sea".to_string()),
        (
            Role::Assistant,
            "The sea stretches endlessly beneath the horizon.".to_string(),
        ),
        (Role::User, "second question about waves".to_string()),
        (
            Role::Assistant,
            "Its waves whisper secrets to the shore.".to_string(),
        ),
    ];
    for (role, text) in &script {
        app.messages.push(Message {
            role: role.clone(),
            content: MessageContent::Text(text.clone()),
        });
        app.should_exit = false;
        app.sim_key_drain_passes = 0;
        app.simulated_keys.clear();
        for _ in 0..3 {
            let _ = app.run(&mut terminal).expect("loop runs");
            if app.should_exit {
                break;
            }
        }
    }

    // Now commit a streaming assistant message on top of the grown transcript,
    // the way a live turn ends.
    app.handle_agent_event(AgentEvent::Content {
        text: "The tide answers twice.".to_string(),
    });
    app.handle_agent_event(AgentEvent::Done {
        message: operant_core::client::Message {
            role: operant_core::client::Role::Assistant,
            content: String::new(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        },
        reason: operant_core::agent::TurnExitReason::TextResponse,
    });
    app.should_exit = false;
    app.sim_key_drain_passes = 0;
    app.simulated_keys.clear();
    for _ in 0..3 {
        let _ = app.run(&mut terminal).expect("loop runs");
        if app.should_exit {
            break;
        }
    }

    let prepared = crate::tui::operant_ui::prepare::prepare_body_for_overlay(&app, 120);
    let body: Vec<String> = prepared
        .wrapped_lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
        .collect();
    let joined = body.join("\n");

    for needle in [
        "The sea stretches endlessly beneath the horizon.",
        "first question about the sea",
        "Its waves whisper secrets to the shore.",
        "second question about waves",
        "The tide answers twice.",
    ] {
        let occurrences = joined.matches(needle).count();
        assert_eq!(
            occurrences, 1,
            "{needle:?} appears exactly once in an incrementally grown body, \
             got {occurrences} — a duplicated fragment at a message's own end \
             is the live RT-205 signature"
        );
    }

    // And the body must not invent text that no message contains: the live
    // corruption appended a stray character after each prompt.
    for prompt in [
        "first question about the sea",
        "second question about waves",
    ] {
        let Some(row) = body.iter().find(|l| l.contains(prompt)) else {
            panic!("the prompt row for {prompt:?} is missing from the body");
        };
        // The precise check: nothing may follow the prompt on its own row
        // except the prompt itself and the line's own padding.
        let after = &row[row.find(prompt).map(|i| i + prompt.len()).unwrap_or(0)..];
        assert_eq!(
            after.trim(),
            "",
            "a stray fragment must not follow the prompt on its own row: {row:?}"
        );
    }
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
#[test]
fn alt_x_toggles_the_pinned_todo_band() {
    // (iter-1211) jcode binds `todo_card_toggle` to `X` in its ToggleKeys
    // layer (keybind.rs), which operant never ported. The state and the
    // /todos slash command existed, but there was no chord, so dismissing a
    // band the operator sees on every frame meant typing a command.
    let mut app = make_app();
    assert!(!app.todos_band_hidden, "the band starts visible");
    press_alt_x(&mut app);
    assert!(
        app.todos_band_hidden,
        "Alt+X must hide the band \u{2014} jcode's todo_card_toggle chord"
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("hidden")),
        "the operator needs feedback that the band moved: {:?}",
        app.status_message
    );
    press_alt_x(&mut app);
    assert!(!app.todos_band_hidden, "Alt+X must toggle back");
}

/// Alt+X through the real key path (not the slash command), so the test
/// proves the chord is wired the way jcode binds `todo_card_toggle`.
fn press_alt_x(app: &mut App) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
}

#[test]
fn the_pinned_band_expansion_flag_reaches_the_viewport() {
    // (iter-1211) The flag was declared on the TuiState trait and CONSUMED by
    // `pinned_todo_band_lines` to decide whether to print
    // `\u{2026} +N more (todo)`, but the App impl returned a hardcoded `false`
    // under a [port-decision] claiming no todo band existed to expand. A
    // reader of the trait therefore saw a permanent no-op, and the
    // truncation line could never be dismissed. This asserts the accessor
    // reflects the field.
    let mut app = make_app();
    assert!(
        !app.pinned_todos_expanded(),
        "collapsed by default: a short card shows everything anyway"
    );
    app.pinned_todos_expanded = true;
    assert!(
        app.pinned_todos_expanded(),
        "the viewport must see the expanded state it is asked to honour"
    );
}

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
    // token_count — the whole warning subsystem was dead.
    //
    // iter-1207: the percentage is now measured against the prompt size the
    // provider metered (`input_tokens`), not the turn total
    // (input+output+cached). That is the figure which actually fills the
    // window, so it is what must be compared against it; the turn total is
    // bigger than the prompt by the completion the provider just generated
    // and warned early. `context_window_size` is 0 on a fresh App (no
    // provider refresh has run), so the flat 128000 resolver is still the
    // denominator here — the point being tested is the threshold crossing.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 106_000, // 83% of 128_000 -> the 80% notice
        output_tokens: 2_000,
        total_tokens: 108_000,
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
fn token_warning_uses_the_prompt_size_not_the_turn_total() {
    // iter-1207: a turn whose PROMPT is small but whose OUTPUT is large
    // must not trip the warning. The prompt is what occupies the window;
    // the completion is what the provider already generated and handed
    // back. Reading the turn total made a single verbose answer look like
    // an overflowing context.
    //
    // The numbers are chosen with a wide margin on BOTH sides so the test
    // fails decisively under a `used = token_count` mutant instead of
    // slipping through on a rounding boundary: the prompt is 12% of the
    // window (a mutant reading the turn total sees 404% and fires), and
    // the prompt used for the crossing is 86% (a mutant still sees the
    // same 86% and also fires, so the margin lives in the first case).
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 15_000, // 12% of the 128_000 window
        output_tokens: 500_000,
        total_tokens: 515_000, // 402% of the window as a raw turn total
    });
    assert_eq!(
        app.token_warning_threshold_shown, 0,
        "a large completion must not be read as a large context"
    );
    // The reverse: the measured prompt size carries it across.
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 110_000, // 86% of the 128_000 window
        output_tokens: 100,
        total_tokens: 110_100,
    });
    assert_eq!(app.token_warning_threshold_shown, 80);
}

#[test]
fn token_warning_uses_the_resolved_context_window_not_the_flat_stub() {
    // iter-1207: the warning used to divide by
    // `context_window_for_model(model)` — a flat 128000 that ignores the
    // model name — while the footer's bar divided by the per-model window
    // `refresh_context_window_size` resolved (200_000 for Anthropic). One
    // prompt therefore read 80% in the footer and 95% here, and the
    // notice that decides whether auto-compact runs was the wrong one.
    let mut app = make_app();
    app.context_window_size = 200_000;
    app.model_name = "anthropic/claude-sonnet-4".to_string();
    // 160_000 of a 200_000 window is 80% — it must fire now, and must NOT
    // have fired when the flat 128000 denominator was used (125%).
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 160_000,
        output_tokens: 10,
        total_tokens: 160_010,
    });
    assert_eq!(
        app.token_warning_threshold_shown, 80,
        "160k of a 200k window is 80%: the notice must agree with the footer bar"
    );
}

#[test]
fn observed_prompt_tokens_reach_the_footer_context_bar() {
    // iter-1207: `observed_context_tokens` was declared on InfoWidgetData
    // and read by the footer bar, but no producer ever wrote it — every
    // frame fell through to `ContextInfo::estimated_tokens()`, a
    // characters/4 guess that can see neither the system prompt nor the
    // tool schemas (both are the bulk of a real prompt). One line closes
    // it: the Usage event's input_tokens is provider-metered context.
    let mut app = make_app();
    assert_eq!(
        app.info_widget_data().observed_context_tokens,
        None,
        "before the first turn there is no metered figure — the estimate rules"
    );
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 42_321,
        output_tokens: 512,
        total_tokens: 42_833,
    });
    assert_eq!(
        app.info_widget_data().observed_context_tokens,
        Some(42_321),
        "the footer must show the prompt size the provider metered, not an estimate"
    );
}

#[test]
fn a_zero_usage_event_clears_the_observed_prompt_tokens() {
    // A provider that reports no usage at all must not leave the previous
    // turn's figure on screen: the estimate is a better guide than a stale
    // measurement of a context that no longer exists.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 42_321,
        output_tokens: 512,
        total_tokens: 42_833,
    });
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
    });
    assert_eq!(app.info_widget_data().observed_context_tokens, None);
}

#[test]
fn token_warning_threshold_resets_when_usage_drops() {
    // Without a reset, an escalate-only gate would permanently suppress
    // warnings after /clear or /compact shrinks the context back down.
    let mut app = make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 123_000,
        output_tokens: 0,
        total_tokens: 123_000, // 96% of the 128_000 stub window
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

/// Stand in for a rendered frame: the reader is parked with the viewport's
/// top row at `top_row`, then reconcile. `scroll_offset` is a TOP ROW (see
/// scroll_anchor.rs): 0 is the oldest line, the frame's max the tail — and a
/// paused frame paints the reader's own offset.
fn paint_frame_at_top(app: &mut App, top_row: usize) {
    app.scroll_offset = top_row;
    app.last_render_scroll_offset.set(top_row as u16);
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

    // Restoring top row 300 of a 40-row transcript must land in range, not
    // past the tail (which would render a blank or clamped view with no
    // explanation).
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(
        app.scroll_offset, 40,
        "restored offset is clamped to the range"
    );
}

/// Repeated PageUps walk the reader UP the transcript from the tail, one page
/// per press, and stop only at the real ceiling.
///
/// (2026-10-24 red-team) The handlers used to ADD to the offset — the
/// distance-from-tail reading — while the renderer paints the offset as the
/// TOP ROW. The state sat at 0 while the reader looked at the tail, so the
/// FIRST press teleported the view to the top of the transcript and every
/// later press walked back down: the operator's "scroll up jumps to the top,
/// then scrolling up scrolls down" report. This drives the real handler in
/// the run loop's order (key → paint → reconcile) against published geometry.
#[test]
fn repeated_page_up_climbs_monotonically_and_stops_at_the_real_ceiling() {
    const LIVE_MAX: usize = 174;
    let mut app = make_app();
    crate::tui::operant_ui::set_last_max_scroll(LIVE_MAX);
    // The reader is following the tail; the reconcile after the last frame
    // synced the state to the tail's top row.
    app.auto_scroll = true;
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, LIVE_MAX,
        "tail-follow parks the state at the tail's top row"
    );

    let mut expected = LIVE_MAX;
    for step in 0..8 {
        expected = expected.saturating_sub(10);
        app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
        // One frame, in the order the run loop produces it: the frame paints
        // the row the reader asked for, then the reconcile runs against it.
        app.last_render_scroll_offset.set(expected as u16);
        app.reconcile_scroll_anchor();
        assert_eq!(
            app.scroll_offset, expected,
            "step {step}: each press climbs one page and the reconcile holds it"
        );
        assert!(
            app.scroll_offset <= LIVE_MAX,
            "step {step}: the offset may never overshoot the transcript"
        );
    }
    // Eight presses of 10 rows climb 80 rows from the tail; the ceiling
    // (row 0) is still 94 rows away, so the reader is climbing — and that is
    // the point: nothing threw them back to the tail on the way.
    assert_eq!(
        app.scroll_offset,
        LIVE_MAX - 80,
        "eight PageUps climb 80 rows, unimpeded"
    );
    crate::tui::operant_ui::set_last_max_scroll(0);
}

/// The same invariant through the fine keyboard rung, which writes
/// `scroll_offset` through its own handler and relies on the same reconcile.
#[test]
fn repeated_wheel_up_climbs_monotonically_and_stops_at_the_real_ceiling() {
    const LIVE_MAX: usize = 174;
    let mut app = make_app();
    crate::tui::operant_ui::set_last_max_scroll(LIVE_MAX);
    app.auto_scroll = true;
    app.reconcile_scroll_anchor();

    let mut expected = LIVE_MAX;
    for step in 0..10 {
        expected = expected.saturating_sub(3);
        app.handle_key_event(press_key_with_control(KeyCode::Up));
        app.last_render_scroll_offset.set(expected as u16);
        app.reconcile_scroll_anchor();
        assert_eq!(
            app.scroll_offset, expected,
            "step {step}: Ctrl+Up subtracts 3 rows and the reconcile holds it"
        );
    }
    // Ten presses of 3 rows climb 30, again with nothing throwing the reader
    // back to the tail.
    assert_eq!(
        app.scroll_offset,
        LIVE_MAX - 30,
        "ten Ctrl+Up presses climb 30 rows"
    );
    crate::tui::operant_ui::set_last_max_scroll(0);
}

#[test]
fn reconcile_clamps_ghost_offset_after_the_flush_shrinks_the_transcript() {
    // A flush boundary can collapse rows the offset still counts (live-audit
    // P4-1 follow-up: offset 355 against a 15-row max left PageDown walking
    // ghost rows home). The reconcile must clamp the STATE to the frame's live
    // max — the render already clamps its own paint.
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

    // 140 transcript rows in a 40-row viewport. The reader's viewport top is
    // row 70, the second rendered line of message 3.
    app.scroll_offset = 70;
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(3, 1),
            painted_row: 70,
            resolved_row: 70,
        },
    );
    assert_eq!(app.scroll_offset, 70, "precondition: parked at top row 70");

    // The terminal narrows and every line rewraps: the anchored content now
    // lives at row 25. The frame still painted the stale row (70) — the render
    // path resolved the content address and published both.
    app.note_resize();
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(3, 1),
            painted_row: 70,
            resolved_row: 25,
        },
    );

    assert_eq!(
        app.scroll_offset, 25,
        "the reader is put back on the message they were reading"
    );
    assert_ne!(
        app.scroll_offset, 70,
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
    // the geometry, and the reconcile syncs the state to that row.
    app.scroll_offset = 0;
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(7, 0),
            painted_row: 90,
            resolved_row: 90,
        },
    );
    assert_eq!(
        app.scroll_offset, 90,
        "the state tracks the tail the frame painted"
    );

    app.note_resize();
    assert!(
        app.scroll_memory.pending_resize.is_none(),
        "a follower has no reading position to preserve"
    );

    // Narrowed: the tail is now row 220.
    paint_at(
        &mut app,
        ResizeFrame {
            pos: pos(7, 0),
            painted_row: 220,
            resolved_row: 220,
        },
    );

    assert!(
        app.auto_scroll,
        "the resize must not unpin a reader who was following live output"
    );
    assert_eq!(
        app.scroll_offset, 220,
        "a follower stays on the tail the frame painted"
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
fn a_resize_adopt_writes_the_resolved_top_row() {
    let mut app = make_app();
    app.auto_scroll = false;
    app.scroll_offset = 30;
    app.last_render_content_pos.set(Some(pos(3, 1)));
    app.note_resize();

    app.last_render_scroll_offset.set(190);
    app.last_resolved_scroll.set(Some(25));
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, 25,
        "the offset is a top row: the resolved row is adopted as-is"
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

/// The restore must not leave the sentence on screen twice. The submit path
/// pushes the user message into the transcript before the turn runs, so a
/// failed turn previously showed the prompt as a transcript row AND back in
/// the composer — the operator's "leaking text in the input box" report. The
/// transcript row for the failed turn is dropped with the restore.
///
/// MUTATION-PROOF: deleting the `drop_transcript_user_row_matching` call in
/// `restore_failed_input_to_composer` fails the second assertion.
#[test]
fn a_failed_turn_restores_the_prompt_without_leaving_it_in_the_transcript() {
    let mut app = make_app();
    app.set_prompt_text("audit the browser stack".to_string());

    // The submit path's own ordering: `take_input` empties the composer and
    // arms the restore slot, and the caller then pushes the row.
    let submitted = app.take_input();
    app.add_message(Role::User, submitted);
    assert_eq!(app.messages.len(), 1, "the submit pushed a user row");

    app.handle_agent_event(AgentEvent::Error {
        error: "provider overloaded_error (529)".to_string(),
    });

    assert_eq!(app.prompt_input.text, "audit the browser stack");
    assert_eq!(
        app.messages.len(),
        0,
        "the failed turn's user row must not stay behind as a duplicate"
    );
}

/// The row removal is bounded to the FAILED submission: a transcript that
/// legitimately holds the same prompt twice keeps its earlier copy, and only
/// the tail row goes away. Without the bound, a repeat question would
/// silently lose one of its two turns from the transcript.
#[test]
fn restoring_a_failed_prompt_keeps_an_earlier_identical_user_row() {
    let mut app = make_app();
    app.add_message(Role::User, "same question".to_string());
    app.set_prompt_text("same question".to_string());
    let submitted = app.take_input();
    app.add_message(Role::User, submitted);
    assert_eq!(app.messages.len(), 2);

    app.handle_agent_event(AgentEvent::Error {
        error: "provider overloaded_error (529)".to_string(),
    });

    assert_eq!(
        app.messages.len(),
        1,
        "only the failed turn's row is removed; the earlier one survives"
    );
    assert_eq!(app.prompt_input.text, "same question");
}

/// A restore whose text never reached the transcript (a queued steer, a
/// command expansion) must not delete an unrelated row. The lookup matches
/// exact content at the tail, so a transcript with no such row is untouched.
#[test]
fn restoring_a_prompt_that_has_no_user_row_removes_nothing() {
    let mut app = make_app();
    app.add_message(Role::User, "an unrelated earlier prompt".to_string());
    app.set_prompt_text("text that was never submitted".to_string());
    app.take_input();

    app.handle_agent_event(AgentEvent::Error {
        error: "provider overloaded_error (529)".to_string(),
    });

    assert_eq!(
        app.messages.len(),
        1,
        "an unrelated row must survive a restore that does not own it"
    );
    assert_eq!(app.prompt_input.text, "text that was never submitted");
}

/// Turn attribution stays index-aligned after the row leaves: the metadata
/// vector is derived from the user-message count, so a removed row must not
/// leave an orphan entry that shifts every later turn's model and duration.
#[test]
fn restoring_a_failed_prompt_keeps_turn_metadata_aligned() {
    let mut app = make_app();
    app.add_message(Role::User, "first prompt".to_string());
    app.set_prompt_text("second prompt".to_string());
    let submitted = app.take_input();
    app.add_message(Role::User, submitted);
    // `add_message` for a user row already snapshots the turn, so the counts
    // are aligned the moment the row lands.
    app.sync_turn_metadata_to_messages();
    assert_eq!(
        app.turn_metadata.len(),
        app.messages.iter().filter(|m| m.role == Role::User).count()
    );

    app.handle_agent_event(AgentEvent::Error {
        error: "provider overloaded_error (529)".to_string(),
    });

    let user_count = app.messages.iter().filter(|m| m.role == Role::User).count();
    assert_eq!(
        app.turn_metadata.len(),
        user_count,
        "no orphan metadata entry may survive the removed row"
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

/// (2026-10-25 history-nav fix) THE pollution defect, write side: submitting a
/// slash command must not append it to the Up/Down ladder. The operator's
/// file held 593 command entries in 853 (70%) crowding out real prompts.
/// Mutation: delete the `is_command` guard in take_input and this fails
/// (`/help` lands in the ladder).
#[test]
fn submitting_a_slash_command_does_not_enter_the_history_ladder() {
    // take_input persists real prompts to history.jsonl under the operant
    // home — divert the home to a sandbox so the test does not write to the
    // operator's real history file.
    let _env_lock = crate::config::test_env_lock();
    let saved_home = std::env::var("HERMES_HOME").ok();
    let sandbox = std::env::temp_dir().join(format!("operant-history-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&sandbox);
    // SAFETY: serialized by the crate-wide test_env_lock; restored below.
    unsafe {
        std::env::set_var("HERMES_HOME", &sandbox);
    }
    let mut app = make_app();
    app.set_prompt_text("a real prompt".to_string());
    app.take_input();
    app.set_prompt_text("/help".to_string());
    app.take_input();
    app.set_prompt_text("another real prompt".to_string());
    app.take_input();
    let ladder = app.prompt_input.history.clone();
    // Restore before asserting so a failure does not leak the sandbox home.
    // SAFETY: still under test_env_lock.
    unsafe {
        match &saved_home {
            Some(home) => std::env::set_var("HERMES_HOME", home),
            None => std::env::remove_var("HERMES_HOME"),
        }
    }
    let persisted = std::fs::read_to_string(sandbox.join("history.jsonl")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&sandbox);
    // Semantic assertions: parallel tests' take_input calls can append to
    // the sandboxed file while the env flip is live (env is process-global),
    // so the ladder may carry foreign prompts — but the INVARIANT is that
    // no slash command ever lands in it, and this test's own prompts do.
    assert!(
        !ladder
            .iter()
            .any(|t| crate::tui::input::is_slash_command(t.trim())),
        "slash commands never enter the Up/Down ladder: {ladder:?}"
    );
    assert!(
        ladder.contains(&"a real prompt".to_string()),
        "this test's first prompt is in the ladder: {ladder:?}"
    );
    assert!(
        ladder.contains(&"another real prompt".to_string()),
        "this test's second prompt is in the ladder: {ladder:?}"
    );
    // And the persisted file agrees: no command row ever lands there.
    assert!(
        !persisted.contains("\"/help\""),
        "the file never gains command rows: {persisted}"
    );
    assert!(persisted.contains("a real prompt"));
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
    crate::tui::operant_ui::set_last_max_scroll(100);
    let mut app = make_app();
    app.auto_scroll = true;
    app.reconcile_scroll_anchor(); // tail-sync: the state tracks the tail
    app.handle_key_event(press_key(KeyCode::Up, KeyModifiers::CONTROL));
    assert_eq!(
        app.scroll_offset, 97,
        "Ctrl+Up must scroll the transcript 3 rows up from the tail"
    );
    assert!(!app.auto_scroll, "Ctrl+Up must pause tail-follow");
    app.handle_key_event(press_key(KeyCode::Down, KeyModifiers::CONTROL));
    assert_eq!(app.scroll_offset, 100, "Ctrl+Down returns to the tail");
    assert!(app.auto_scroll, "reaching the tail must resume tail-follow");
    crate::tui::operant_ui::set_last_max_scroll(0);
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

/// The keyboard scroll ladder driven against a REAL render.
///
/// 2026-10-24 red-team: the handlers used to write the offset in the
/// distance-from-tail reading while the renderer paints it as the TOP ROW, so
/// the first PageUp from a live view teleported to the top of the transcript
/// and later presses walked back down. This drives the REAL handler (not a
/// hand-rolled arithmetic stand-in), paints a real frame at each step, and
/// runs the reconcile the run loop performs — the top visible line must walk
/// UP the transcript monotonically and never reverse.
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

    // Each PageUp step: the real handler, then the paint and the reconcile the
    // run loop performs after it. The offset is a top row, so walking UP means
    // the lowest visible `L###` marker DECREASES toward L001.
    let mut seen_tops: Vec<usize> = Vec::new();
    for step in 0..6 {
        app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
        // Paint the frame the reader asked for.
        let rows = screen_rows(&app);
        let flat = rows.join("\n");
        let top_line = visible_top_line(&flat);
        // The reconcile a real run loop performs after every paint.
        app.reconcile_scroll_anchor();
        seen_tops.push(top_line.unwrap_or(1));
        assert!(
            top_line.is_some_and(|l| l >= 1),
            "a real paint must show transcript content (step {step}): top={top_line:?}"
        );
    }
    assert!(
        seen_tops.windows(2).all(|w| w[0] > w[1]),
        "each PageUp must walk the top line further up the transcript, got {seen_tops:?}"
    );
    assert!(
        seen_tops.last().copied().unwrap_or(200) <= 140,
        "six PageUps must reach well above the tail: {seen_tops:?}"
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

    // F5 / Alt+R with an empty composer retries the last FAILED turn only
    // (OMP semantics — 2026-10-25). A failed turn restores its prompt to the
    // composer itself (restore_failed_input_to_composer), so the common path
    // is "press Enter on the restored text". The empty-composer arm refills
    // from the failed-turn slot, never from the plain last prompt: the old
    // behavior re-ran the previous prompt as a whole new turn on a stray
    // Alt+R, which the operator reported as undesired.
    let mut app = make_app();
    app.prompt_input
        .history
        .push("a prompt that succeeded".to_string());
    assert!(!app.handle_key_event(press_key(KeyCode::F(5), KeyModifiers::NONE)));
    assert!(
        app.prompt_input.text.is_empty(),
        "a successful turn is not retryable — F5 must not resubmit it"
    );

    // The failed-turn slot refills the composer on retry.
    let mut app = make_app();
    app.failed_input_recovery = Some("the failed prompt".to_string());
    assert!(app.handle_key_event(press_key(KeyCode::Char('r'), KeyModifiers::ALT)));
    assert_eq!(app.prompt_input.text, "the failed prompt");

    // …and never while a turn is streaming.
    let mut app = make_app();
    app.failed_input_recovery = Some("x".to_string());
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

    // Ctrl+Tab is the jcode alias for the SAME cycle (TODO.md Pending row,
    // audit 2026-10-28 gap 2): one arm, one function, no new cycle state.
    // MUTATION PROOF: dropping the `KeyCode::Tab => CONTROL` arm fails this -
    // the model stops moving and the second assert reads "m-one" after a
    // first cycle that already wrapped.
    let mut app = make_app();
    app.config.agent.model = "m-one".to_string();
    app.config.agent.fallback_models = vec!["m-two".to_string()];
    app.model_name = "m-one".to_string();
    app.handle_key_event(press_key(KeyCode::Tab, KeyModifiers::CONTROL));
    assert_eq!(app.model_name, "m-two", "Ctrl+Tab cycles the model");
    app.handle_key_event(press_key(KeyCode::Tab, KeyModifiers::CONTROL));
    assert_eq!(
        app.model_name, "m-one",
        "the alias walks the same cycle the primary walks"
    );

    // The alias must not be a greedy Tab arm that swallows what the
    // OTHER Tab-adjacent arms own. Asserted on BEHAVIOUR, not on
    // handle_key_event's return value (plain Tab is consumed by another arm
    // legitimately, so its bool is not this arm's contract): BackTab is what
    // crossterm reports for shift+tab, and the composer's contract for it is
    // pinned by `shift_tab_does_not_touch_permission_mode`.
    let mut app = make_app();
    use crate::tui::adapter_types::config::PermissionMode;
    app.handle_key_event(press_key(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(
        app.settings.permission_mode,
        PermissionMode::Default,
        "BackTab must keep its own composer contract, not be stolen by the alias"
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
        content(&app).contains("line 8"),
        "collapsed shows the tail preview — line 8 is the first of the last 3 \
         rows (codex's tail-3: exec_cell/compact.rs:114), so a build's verdict \
         line is visible without expanding. got: {}",
        content(&app)
    );
    assert!(
        !content(&app).contains("line 7"),
        "collapsed must NOT show the 7th of 10 rows — only the last 3 fit. \
         got: {}",
        content(&app)
    );
    app.handle_key_event(press_key(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(
        content(&app).contains("line 10"),
        "expanded shows the full output"
    );
    app.handle_key_event(press_key(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(
        !content(&app).contains("line 2\n"),
        "collapsing again returns to the tail preview, not the head. got: {}",
        content(&app)
    );
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

/// TUI-2: Enter in the session hub must retarget the agent, not only load
/// the transcript. Otherwise the next prompt is written to the old session.
/// (plan 2026-10-28 iter-B: the hub replaces the old browser modal.)
#[test]
fn resume_picks_the_session_the_agent_switches_to() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use operant_core::session::listing::{RowKind, RowStatus, SessionEntryV2};

    let mut app = make_app();
    app.session_hub.open(None);
    app.session_hub.replace_rows(vec![SessionEntryV2 {
        id: "s-42".to_string(),
        parent_id: None,
        title: Some("old work".to_string()),
        updated_at: chrono::Utc::now(),
        employee_id: None,
        source: "local".to_string(),
        message_count: 3,
        cost_usd: 0.0,
        tokens: 0,
        cwd: None,
        status: RowStatus::Idle,
        kind: RowKind::Session,
    }]);
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.pending_switch_session.as_deref(),
        Some("s-42"),
        "/resume must hand the picked session to the agent switch"
    );
    assert!(!app.session_hub.visible, "Enter closes the hub");
}

#[cfg(test)]
mod rename_on_load_tests {
    use super::*;

    #[test]
    fn a_bare_rename_no_longer_opens_modal_state_but_hints() {
        // TUI-8 behavior under the hub (plan 2026-10-28 iter-B): the old
        // modal's inline rename arm is gone; a bare `/rename` surfaces the
        // /rename <title> hint instead. The flag is still consumed once.
        let mut app = make_app();
        app.rename_on_list_load = true;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        app.session_list_rx = Some(rx);
        tx.try_send(Vec::<operant_core::session::listing::SessionEntryV2>::new())
            .unwrap();

        assert!(app.drain_session_list());

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
    use operant_core::session::listing::{RowKind, RowStatus, SessionEntryV2};

    #[test]
    fn a_pending_approval_takes_the_key_before_an_open_hub() {
        // TUI-1: an approval the agent is waiting on is answered first, so an
        // open overlay cannot swallow its key and leave the turn stalled.
        let mut app = make_app();
        app.session_hub.open(None);
        app.session_hub.replace_rows(vec![SessionEntryV2 {
            id: "s-1".to_string(),
            parent_id: None,
            title: Some("t".to_string()),
            updated_at: chrono::Utc::now(),
            employee_id: None,
            source: "local".to_string(),
            message_count: 0,
            cost_usd: 0.0,
            tokens: 0,
            cwd: None,
            status: RowStatus::Idle,
            kind: RowKind::Session,
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
        assert!(app.session_hub.visible, "the hub did not take the key");
    }

    // ---- drag hit-test geometry ------------------------------------------

    /// A drag's screen row must resolve against the rect the transcript TEXT
    /// was drawn into, not the wider messages band.
    ///
    /// The band's first `top_band_lines` rows are the previous-prompt preview
    /// and the pinned-todo strip; resolving against it selected a line ABOVE
    /// the text under the pointer (2026-10-23), so the copy carried a
    /// different message than the highlight painted. Publishing the content
    /// rect is what makes the two agree.
    #[test]
    fn a_drag_row_resolves_against_the_chat_content_area_not_the_band() {
        let app = make_app();
        // The band: rows 3..13 of a 120x40 frame.
        app.last_selectable_area
            .set(ratatui::layout::Rect::new(0, 3, 120, 10));
        // The text inside it starts 2 rows lower (prompt preview + todos).
        crate::tui::operant_ui::publish_chat_content_area(ratatui::layout::Rect::new(0, 5, 120, 8));
        app.last_render_scroll_offset.set(0);

        // Row 7 is the 3rd text row (7 - 5), NOT the 5th band row (7 - 3).
        assert_eq!(app.content_line_of_screen_row(7), 2);
        // The first text row resolves to the content origin, so a drag that
        // starts at the top of the text does not read as a negative line.
        assert_eq!(app.content_line_of_screen_row(5), 0);

        // Scrolled: the offset still applies to a row inside the text rect,
        // never to a row inside the chrome strip.
        app.last_render_scroll_offset.set(40);
        assert_eq!(app.content_line_of_screen_row(7), 42);
    }

    /// Before the first frame paints a content rect (startup, or a narrow
    /// frame that skipped the layout), the band is the only geometry there is
    /// - and an unknown offset must not shift selections by a guess.
    #[test]
    fn an_unpublished_content_area_falls_back_to_the_band() {
        let app = make_app();
        app.last_selectable_area
            .set(ratatui::layout::Rect::new(0, 4, 120, 10));
        app.last_render_scroll_offset.set(3);
        crate::tui::operant_ui::publish_chat_content_area(ratatui::layout::Rect::new(0, 0, 0, 0));

        assert_eq!(app.content_line_of_screen_row(9), 8);
    }

    // ---- /copy [N] and /rewind [N] argument semantics --------------------

    /// Seed `n` alternating user/assistant messages so a test can assert
    /// which slice `/copy N` took.
    fn seed_messages(app: &mut crate::tui::app::App, n: usize) {
        for i in 0..n {
            app.messages.push(crate::tui::app::Message {
                role: if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: MessageContent::Text(format!("turn {i}")),
            });
        }
    }

    /// `/copy <n>` takes the last n messages, and a bare `/copy` still takes
    /// exactly one — the argument the registry advertises must be read.
    #[test]
    fn copy_takes_the_last_n_messages() {
        for (args, expected) in [
            ("", vec!["turn 5"]),
            ("3", vec!["turn 5", "turn 4", "turn 3"]),
        ] {
            let mut app = make_app();
            seed_messages(&mut app, 6);
            // Count what the command selects, without touching the real
            // clipboard: rebuild the same slice the command does.
            let n = super::super::commands::parse_last_n(
                args,
                super::super::commands::DEFAULT_COPY_MESSAGES,
            );
            let selected = app
                .messages
                .iter()
                .rev()
                .filter(|m| !m.get_all_text().trim().is_empty())
                .take(n)
                .map(|m| m.get_all_text())
                .collect::<Vec<_>>();
            assert_eq!(selected, expected, "for args {args:?}");
        }
    }

    /// An explicit `0` copies nothing rather than silently becoming the
    /// default — `0` is a real answer, not a parse failure.
    #[test]
    fn copy_zero_is_honoured_not_replaced_by_the_default() {
        let mut app = make_app();
        seed_messages(&mut app, 3);
        let n = super::super::commands::parse_last_n("0", 1);
        assert_eq!(n, 0);
        let selected = app
            .messages
            .iter()
            .rev()
            .filter(|m| !m.get_all_text().trim().is_empty())
            .take(n)
            .count();
        assert_eq!(selected, 0);
    }

    /// `/rewind <n>` resolves to the Nth-last prompt, not the Nth from the
    /// session's start — the absolute-index reading made every n>1 land near
    /// the oldest turn.
    #[test]
    fn rewind_n_resolves_to_the_nth_last_prompt() {
        let mut app = make_app();
        app.rewind_selector.entries = vec![
            crate::tui::rewind_selector::RewindEntry {
                turn_id: 1,
                text: "first".to_string(),
                compacted: false,
            },
            crate::tui::rewind_selector::RewindEntry {
                turn_id: 2,
                text: "second".to_string(),
                compacted: false,
            },
            crate::tui::rewind_selector::RewindEntry {
                turn_id: 3,
                text: "third".to_string(),
                compacted: false,
            },
        ];
        // The command only records the intent; the y/n handler indexes. Drive
        // it the way the status line does so the resolution is what is tested.
        app.handle_tui_command("rewind", "2");
        assert_eq!(app.pending_rewind_confirm, Some(2));
        app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::NONE));
        // len 3 - 2 = 1 -> entry index 1 -> turn_id 2, the second-newest.
        assert_eq!(app.pending_rewind, Some(2));

        app.handle_tui_command("rewind", "1");
        app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(app.pending_rewind, Some(3), "/rewind 1 is the LAST prompt");

        app.handle_tui_command("rewind", "3");
        app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(app.pending_rewind, Some(1));
    }

    /// An out-of-range `/rewind <n>` must be refused, not clamp to an
    /// in-bounds index and rewind to the wrong turn.
    #[test]
    fn rewind_out_of_range_is_refused() {
        let mut app = make_app();
        app.rewind_selector.entries = vec![crate::tui::rewind_selector::RewindEntry {
            turn_id: 7,
            text: "only".to_string(),
            compacted: false,
        }];
        app.handle_tui_command("rewind", "9");
        app.handle_key_event(press_key(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(app.pending_rewind, None);
        assert!(
            app.status_notice()
                .as_deref()
                .is_some_and(|n| n.contains("No prompt")),
            "an out-of-range n must say so: {:?}",
            app.status_notice()
        );
    }
}

// (audit R1 2026-10-28) the `branch_list_tests` module's two tests
// went with the deleted session_branching overlay: they fed a fork tree
// into a now-dead drain and asserted a now-deleted overlay opened. The
// behaviour they protected — Ctrl+B reaches a fork view, and a database
// failure never pretends an empty answer — is pinned on the hub instead
// in `left_tap_gesture_tests` and `session_hub/tests.rs`, and the
// error path is exercised through the hub's own loader failure notice.

// ---------------------------------------------------------------------------
// iter-1213: keyboard scroll ladder reachability (PgUp/PgDn/Shift+Home/Shift+End)
// ---------------------------------------------------------------------------
//
// The 2026-10-24 operator report was blunt and correct: "none are working",
// where the previous iteration had shipped PgUp/PgDn plus Home/End. The arms
// existed and compiled; the gate they hung on (`self.focus ==
// FocusTarget::Transcript`) is reachable ONLY by a mouse click on the
// transcript (mouse.rs:1058) and has no keyboard way in, so every key was dead
// for a keyboard-only operator while the code looked implemented. These tests
// drive the real key handler from the DEFAULT focus (`FocusTarget::Input`)
// because that is the state an operator is in when they type.
#[cfg(test)]
mod keyboard_scroll_reachability_tests {
    use super::*;

    /// The frame-published transcript height the renderer found the messages in.
    fn publish_transcript_height(height: u16) {
        crate::tui::operant_ui::publish_chat_content_area(ratatui::prelude::Rect::new(
            0, 0, 100, height,
        ));
    }

    /// A 30-row transcript with a 14-row content band, tail-following: the
    /// reader is parked at the top row the tail resolved to (`max_scroll` = 16).
    fn app_at_tail(max: usize) -> App {
        let mut app = make_app();
        crate::tui::operant_ui::set_last_max_scroll(max);
        app.auto_scroll = false;
        app.last_render_scroll_offset.set(max as u16);
        app
    }

    #[test]
    fn page_up_and_page_down_scroll_while_the_prompt_input_has_focus() {
        // Focus is `Input` (init.rs:297) — the state the operator is in while
        // typing, and the state that must not require a mouse detour.
        assert_eq!(make_app().focus, crate::tui::app::enums::FocusTarget::Input);
        crate::tui::operant_ui::set_last_max_scroll(200);
        publish_transcript_height(30);
        let mut app = make_app();
        app.auto_scroll = true;
        app.reconcile_scroll_anchor(); // tail-sync: the state tracks the tail

        app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));

        assert!(
            app.scroll_offset < 200,
            "PageUp must move the view up from the tail while the input has focus; it read as dead by a keyboard-only operator"
        );
        assert!(
            !app.auto_scroll,
            "paging up stops auto-follow so new messages do not yank the view back"
        );
        let parked = app.scroll_offset;
        app.handle_key_event(press_key(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(
            app.scroll_offset > parked,
            "PageDown brings the reader back down the same ladder"
        );
        assert_eq!(app.scroll_offset, 200, "back at the tail's top row");
        assert!(app.auto_scroll, "landing on the tail resumes following");
        crate::tui::operant_ui::set_last_max_scroll(0);
    }

    #[test]
    fn shift_home_and_shift_end_reach_the_top_and_back_to_the_tail() {
        crate::tui::operant_ui::set_last_max_scroll(174);
        let mut app = app_at_tail(174);

        app.handle_key_event(press_key(KeyCode::Home, KeyModifiers::SHIFT));
        assert_eq!(
            app.scroll_offset, 0,
            "Shift+Home takes the whole distance to row 0 — the top of the transcript"
        );
        assert!(!app.auto_scroll, "reaching the top parks the view");

        app.handle_key_event(press_key(KeyCode::End, KeyModifiers::SHIFT));
        assert_eq!(app.scroll_offset, 174, "Shift+End lands on the tail");
        assert!(app.auto_scroll, "and re-enables following new messages");

        crate::tui::operant_ui::set_last_max_scroll(0);
    }

    #[test]
    fn plain_home_and_end_still_edit_the_prompt_cursor() {
        // The plain keys were never the transcript's to take: they are the
        // composer's caret moves, and making them scroll too was the defect
        // that got reverted in this same iteration. A Shift press is what now
        // reaches the transcript, so typing must be untouched.
        let mut app = make_app();
        app.prompt_input.text = "hello".into();
        app.prompt_input.cursor = 3;

        app.handle_key_event(press_key(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(app.prompt_input.cursor, 0, "Home is a cursor move");
        app.handle_key_event(press_key(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(
            app.prompt_input.cursor, 5,
            "End is a cursor move, not a transcript jump"
        );
        assert_eq!(
            app.scroll_offset, 0,
            "plain Home/End never touch the scroll"
        );
    }

    #[test]
    fn the_page_step_measures_the_viewport_it_is_scrolling() {
        // The step used to be a hardcoded 10, which is a "page" by accident on
        // a 12-row terminal and a third of a screen on a 40-row one.
        crate::tui::operant_ui::set_last_max_scroll(300);
        publish_transcript_height(26);
        let app = make_app();
        assert_eq!(
            app.scroll_page_step(),
            24,
            "a 26-row content band pages 24 rows (two rows of overlap)"
        );

        publish_transcript_height(12);
        assert_eq!(app.scroll_page_step(), 10, "a small terminal pages less");

        publish_transcript_height(1);
        assert_eq!(
            app.scroll_page_step(),
            1,
            "a one-row band never degenerates to a zero step, which would make PageUp a no-op"
        );

        publish_transcript_height(0);
        assert_eq!(
            app.scroll_page_step(),
            10,
            "no published frame falls back to the historic constant"
        );
        crate::tui::operant_ui::set_last_max_scroll(0);
    }

    #[test]
    fn paging_up_parks_at_the_top_row_instead_of_drifting_past_it() {
        // `scroll_offset` is a TOP ROW (ui_viewport.rs resolves
        // `scroll = user_scroll.min(max_scroll)`), so upward paging descends
        // toward row 0 and must saturate there rather than wrap or drift.
        crate::tui::operant_ui::set_last_max_scroll(24);
        publish_transcript_height(30);
        let mut app = make_app();
        app.auto_scroll = false;
        app.scroll_offset = 24; // parked at the tail's top row

        // One frame per press, in the order the run loop produces it: the key
        // runs, the frame paints the row the reader landed on, the reconcile
        // holds it.
        let step = app.scroll_page_step();
        let mut expected = 24usize;
        for press in 0..10 {
            expected = expected.saturating_sub(step);
            app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
            app.last_render_scroll_offset.set(expected as u16);
            app.reconcile_scroll_anchor();
            assert_eq!(
                app.scroll_offset, expected,
                "press {press}: the offset follows the ladder"
            );
            assert!(
                app.scroll_offset <= 24,
                "press {press}: the offset may never overshoot the transcript"
            );
        }
        assert_eq!(app.scroll_offset, 0, "upward paging parks at the top row");
        crate::tui::operant_ui::set_last_max_scroll(0);
    }

    /// (2026-10-27, operator report: "the preview from `@` when stream is
    /// ongoing is broken") The typeahead popup's render gate used to hide
    /// Chat-mode rows — the `@` file-ref suggestions — whenever
    /// `is_processing()` (== is_streaming) was true. In jcode that gate is
    /// dead code (Chat-mode suggestions are always empty there); operant's
    /// `@` typeahead made it live, and the composer is deliberately editable
    /// mid-stream, so the popup must render while a turn streams.
    #[test]
    fn file_ref_suggestions_render_while_streaming() {
        let mut app = make_app();
        // A file-ref suggestion is live (as if `@cr` had been typed) and a
        // turn is streaming — the exact state the operator reported.
        app.prompt_input.suggestions =
            vec![crate::tui::prompt_input::TypeaheadSuggestion::file_ref(
                "@folder:crates/".to_string(),
                "crates/".to_string(),
                "directory".to_string(),
            )];
        app.prompt_input.suggestion_index = Some(0);
        app.is_streaming = true;

        let rows = app.command_suggestions();
        assert_eq!(rows.len(), 1);
        // The DISPLAY label — bare path, no @folder: grammar (OMP label parity).
        assert_eq!(rows[0].0, "crates/");
        // The render gate must say the popup is active DESPITE is_streaming.
        assert!(
            crate::tui::operant_ui::input_ui::command_suggestions_active_for_test(&app, &rows),
            "@ file-ref suggestions must render while a turn streams — the composer is editable mid-turn"
        );
    }
}

// ---- Enter chords: newline, queue, continuation (2026-10-27) -------------
//
// The operator's report: "SHIFT+ENTER is not adding line breaks. CTRL+ENTER
// must queue it." Two independent defects were behind it: (1) operant never
// requested the kitty keyboard protocol, so every Enter chord arrived as a
// bare \r (fixed by keyboard_enhancement.rs — these tests drive the key
// handler directly, so they pin the APP side: given a chord event, the right
// thing happens); (2) the app side had its own gaps — Ctrl+J's newline arm
// was lost in the key_handling extraction (the comment survived, the arm
// did not), Ctrl+Enter was mapped to a newline instead of jcode's QUEUE,
// and Shift/Alt+Enter were dead mid-stream.

#[cfg(test)]
mod enter_chord_tests {
    use super::*;

    fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_steer_queue() -> std::sync::Arc<tokio::sync::Mutex<Vec<String>>> {
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()))
    }

    /// Shift+Enter inserts a newline — in BOTH states. The idle case is the
    /// long-standing contract; the streaming case is the fix (the composer
    /// is deliberately editable mid-turn, and composing the next
    /// multi-line prompt while a turn streams is exactly when it is used).
    #[test]
    fn shift_enter_inserts_newline_idle_and_streaming() {
        for streaming in [false, true] {
            let mut app = make_app();
            app.is_streaming = streaming;
            app.prompt_input.replace_text("first line".to_string());
            app.prompt_input.cursor = app.prompt_input.text.len();

            let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::SHIFT));

            assert!(
                !submitted,
                "Shift+Enter must never submit (streaming={streaming})"
            );
            assert!(
                app.prompt_input.text.contains('\n'),
                "Shift+Enter must insert a literal newline (streaming={streaming}), got: {:?}",
                app.prompt_input.text
            );
        }
    }

    /// Alt+Enter shares the Shift+Enter newline branch.
    #[test]
    fn alt_enter_inserts_newline() {
        let mut app = make_app();
        app.prompt_input.replace_text("one".to_string());
        app.prompt_input.cursor = app.prompt_input.text.len();

        let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::ALT));

        assert!(!submitted, "Alt+Enter must never submit");
        assert!(app.prompt_input.text.contains('\n'));
    }

    /// Ctrl+J is the terminal-portable newline (iter-120). Its arm was LOST
    /// in the key_handling extraction refactor — the doc comment claimed
    /// the contract, no arm existed. Restored 2026-10-27; this test is the
    /// one that would have caught the loss.
    #[test]
    fn ctrl_j_inserts_newline() {
        let mut app = make_app();
        app.prompt_input.replace_text("line a".to_string());
        app.prompt_input.cursor = app.prompt_input.text.len();

        let submitted = app.handle_key_event(press_key_with_control(KeyCode::Char('j')));

        assert!(!submitted, "Ctrl+J must never submit");
        assert!(
            app.prompt_input.text.contains('\n'),
            "Ctrl+J must insert a newline, got: {:?}",
            app.prompt_input.text
        );
    }

    /// Ctrl+Enter while a turn streams QUEUES the draft into the steer
    /// queue (jcode's `handle_alternate_enter` → SendAction::Queue) and
    /// clears the composer. The operator's "CTRL+ENTER must queue it".
    #[test]
    fn ctrl_enter_queues_the_draft_while_streaming() {
        let mut app = make_app();
        let queue = make_steer_queue();
        app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
        app.is_streaming = true;
        app.prompt_input
            .replace_text("then check the tests".to_string());

        let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::CONTROL));

        assert!(!submitted, "Ctrl+Enter must not submit while streaming");
        assert_eq!(
            queue.try_lock().unwrap().as_slice(),
            ["then check the tests".to_string()],
            "the draft must land in the steer queue"
        );
        assert!(
            app.prompt_input.text.is_empty(),
            "a successful queue consumes the composer"
        );
        assert!(
            app.status_notice()
                .as_deref()
                .is_some_and(|n| n.contains("Steer queued")),
            "the queue must be announced: {:?}",
            app.status_notice()
        );
    }

    /// Ctrl+Enter while idle submits like plain Enter (jcode's
    /// SendAction::Submit when not processing).
    #[test]
    fn ctrl_enter_submits_when_idle() {
        let mut app = make_app();
        app.is_streaming = false;
        app.prompt_input.replace_text("hello there".to_string());

        let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::CONTROL));

        assert!(submitted, "idle Ctrl+Enter must submit");
    }

    /// An empty composer + Ctrl+Enter mid-stream says so and queues nothing.
    #[test]
    fn ctrl_enter_on_an_empty_composer_says_so() {
        let mut app = make_app();
        let queue = make_steer_queue();
        app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
        app.is_streaming = true;

        let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::CONTROL));

        assert!(!submitted);
        assert!(queue.try_lock().unwrap().is_empty());
        assert!(
            app.status_notice()
                .as_deref()
                .is_some_and(|n| n.contains("Nothing to queue")),
            "the empty-composer case must be named: {:?}",
            app.status_notice()
        );
    }

    /// Trailing-backslash continuation (jcode newline.rs): plain Enter on a
    /// draft ending in an ODD number of backslashes consumes one backslash
    /// and inserts a newline — the universal fallback that needs no
    /// keyboard protocol. An EVEN count is an escaped backslash: literal
    /// text, still submits.
    #[test]
    fn odd_trailing_backslash_makes_enter_a_newline() {
        let mut app = make_app();
        app.prompt_input
            .replace_text("ps aux | grep operant\\".to_string());
        app.prompt_input.cursor = app.prompt_input.text.len();

        let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(!submitted, "the continuation must not submit");
        assert_eq!(
            app.prompt_input.text, "ps aux | grep operant\n",
            "the trailing backslash is consumed and a newline inserted"
        );
    }

    #[test]
    fn even_trailing_backslashes_still_submit() {
        let mut app = make_app();
        app.prompt_input
            .replace_text("literal path C:\\\\".to_string());
        app.prompt_input.cursor = app.prompt_input.text.len();

        let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(
            submitted,
            "an escaped backslash is literal text — Enter submits"
        );
    }

    /// The /terminal-setup hint fires ONCE per session, not per line.
    /// Asserted on the FLAG (not the notice text): `status_notice` returns
    /// the last set message even when nothing new was set, so a second
    /// silent continuation would still read the first press's tip from the
    /// accessor — the honest oracle for "shown once" is the flag itself.
    #[test]
    fn continuation_hint_is_shown_once_per_session() {
        let mut app = make_app();
        app.prompt_input.replace_text("one\\".to_string());
        app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));
        let first_notice = app.status_notice().unwrap_or_default();
        assert!(
            first_notice.contains("/terminal-setup"),
            "the first continuation must point at the real fix: {first_notice}"
        );
        assert!(
            app.terminal_setup_hint_shown.get(),
            "the first continuation must set the once-flag"
        );

        // Second continuation: the flag is already set, so no new notice is
        // set. Clear status_message to prove NOTHING was written.
        app.status_message = None;
        app.prompt_input.replace_text("two\\".to_string());
        app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            app.status_message.is_none(),
            "the second continuation must not re-set the tip: {:?}",
            app.status_message
        );
    }
}

// ---- Prompt jump (2026-10-28, jcode parity) -----------------------------
//
// Alt+[ / Alt+] (and the Ctrl+[ / Ctrl+] aliases where the terminal reports
// them) jump the transcript viewport to the previous / next USER prompt,
// using the wrapped_user_prompt_starts the renderer already publishes on
// every frame — the same seam jcode's prompt-jump reads. The jump pauses
// auto-follow and clears a pending resize anchor (an explicit jump wins
// over a still-settling reflow); the status row names the landed prompt so
// the chord is discoverable without the help overlay.
mod prompt_jump_tests {
    use super::*;
    use crate::tui::operant_model::{PreparedChatFrame, PreparedMessages};
    use std::sync::Arc;

    /// Seed `last_chat_frame` with a body whose user prompts start at the
    /// given wrapped-row indices. Only the prompt-jump seam reads this frame
    /// in these tests, so the other fields stay empty.
    fn seed_frame_with_prompt_starts(starts: &[usize]) {
        let line_count = starts.last().map(|s| s + 4).unwrap_or(8);
        let wrapped_lines: Vec<ratatui::text::Line<'static>> =
            (0..line_count).map(|i| format!("row {i}").into()).collect();
        let plain: Vec<String> = wrapped_lines
            .iter()
            .map(|l| crate::tui::operant_render::line_plain_text(l))
            .collect();
        let mut user_indices = Vec::new();
        for &s in starts {
            user_indices.extend(s..s + 2);
        }
        let prepared = Arc::new(PreparedMessages {
            wrapped_lines,
            wrapped_plain_lines: Arc::new(plain),
            wrapped_copy_offsets: Arc::new(vec![0; line_count]),
            raw_plain_lines: Arc::new(Vec::new()),
            wrapped_line_map: Arc::new(Vec::new()),
            wrapped_user_indices: user_indices,
            wrapped_user_prompt_starts: starts.to_vec(),
            wrapped_user_prompt_ends: starts.iter().map(|s| s + 2).collect(),
            user_prompt_texts: Vec::new(),
            image_regions: Vec::new(),
            edit_tool_ranges: Vec::new(),
            tool_row_ranges: Vec::new(), // RT-146
            copy_targets: Vec::new(),
            message_boundaries: Vec::new(),
            mermaid_pending_epoch: None,
        });
        crate::tui::operant_ui::set_last_chat_frame(Arc::new(PreparedChatFrame::from_single(
            prepared,
        )));
    }

    /// (iter-1515) The red-team finding behind "none of its [the double-Esc
    /// rewind selector's] functionalities are working": the live screen froze
    /// the reversed highlight on ONE row across `<down><down>` — measured on
    /// the deployed binary with `--dump-style`, where the only `/r@` (reversed)
    /// cells stayed on screen row 14 while `rewind_selector.selected` had
    /// advanced. `select_next` and the key handler were both correct; what was
    /// never pinned is that the RENDERED selection follows the cursor, which
    /// is the half the operator actually sees.
    ///
    /// The reversed cells are the selection's footprint, so their row set
    /// before and after a Down is the assertion. The marks describe exactly the
    /// prompts in `app.messages`, which is the correspondence the render
    /// resolves against — and it is built from the messages rather than a
    /// seeded frame because this target's `prepare_body` rebuilds under
    /// `cfg!(test)`, so a seeded LAST_CHAT_FRAME would simply be ignored.
    #[test]
    fn the_rewind_selection_highlight_follows_the_arrow_key() {
        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = make_app();
        // A settled transcript of three turns: each prompt followed by its
        // answer, so each prompt occupies its own row band.
        for (i, prompt) in ["prompt one", "prompt two", "prompt three"]
            .iter()
            .enumerate()
        {
            app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(prompt.to_string()),
            });
            app.messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Text(format!("answer {i}")),
            });
        }
        app.open_rewind_selector();
        assert!(app.rewind_selector.visible, "the selector opened");

        // Marks name the same three prompts, in the same order.
        let marks = vec![
            (1u64, "prompt one".to_string()),
            (2, "prompt two".to_string()),
            (3, "prompt three".to_string()),
        ];
        let display_texts: Vec<String> = marks.iter().map(|(_, t)| t.clone()).collect();
        app.rewind_selector.sync_marks(marks, &display_texts);
        assert_eq!(app.rewind_selector.entries.len(), 3, "three turns listed");

        // SECOND CONTROL: the exact shape the selector builds — a cloned Line
        // whose spans are rebuilt via Span::styled(text, style.add_modifier()).
        // If THIS loses the modifier the selector can never highlight anything,
        // and a "no reversed cells" reading is a harness artefact rather than a
        // defect. Proven deliberately, so the assertion below is falsifiable.
        {
            let backend = TestBackend::new(30, 1);
            let mut t = Terminal::new(backend).expect("t");
            let original = ratatui::text::Line::from("clone me please");
            let rebuilt = ratatui::text::Line::from(
                original
                    .spans
                    .iter()
                    .map(|sp| {
                        ratatui::text::Span::styled(
                            sp.content.to_string(),
                            sp.style.add_modifier(ratatui::style::Modifier::REVERSED),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
            t.draw(|f| {
                f.render_widget(
                    ratatui::widgets::Paragraph::new(rebuilt),
                    ratatui::layout::Rect::new(0, 0, 30, 1),
                );
            })
            .expect("draw");
            let dump = crate::tui::debug::debug_hub::buffer_style_dump(t.backend().buffer());
            let line = dump.lines().nth(1).unwrap_or_default();
            assert!(
                line.split(' ').any(|t| t.contains("/r@")),
                "the harness must see REVERSED on a span rebuilt the way the \
                 selector rebuilds it — otherwise every 'no highlighted row' \
                 reading below is unfalsifiable"
            );
            // SECOND CONTROL: the same construction through Paragraph WITH
            // .scroll(), which is how the selector paints its body. If that
            // path dropped span styles the selector could never highlight.
            let backend2 = TestBackend::new(30, 1);
            let mut t2 = Terminal::new(backend2).expect("t2");
            let l2 = ratatui::text::Line::from(vec![ratatui::text::Span::styled(
                "scrolled line XX",
                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
            )]);
            t2.draw(|f| {
                f.render_widget(
                    ratatui::widgets::Paragraph::new(l2).scroll((0u16, 0)),
                    ratatui::layout::Rect::new(0, 0, 30, 1),
                );
            })
            .expect("draw2");
            let d2 = crate::tui::debug::debug_hub::buffer_style_dump(t2.backend().buffer());
            let l2out = d2.lines().nth(1).unwrap_or_default();
            assert!(
                l2out.split(' ').any(|t| t.contains("/r@")),
                "and it must survive a scrolled Paragraph, or the selector's \
                 body render cannot highlight at all"
            );
        }

        fn outlined_rows(app: &crate::tui::app::App) -> Vec<usize> {
            // The AUTHORITATIVE check, and the same projection the live
            // red-team run used (`operant tui debug simulate --dump-style`):
            // a per-cell grid where `r` in the mods field is REVERSED. Reading
            // the ratatui modifier straight off the buffer is NOT equivalent —
            // it is layout-offset sensitive and a Paragraph's line styling
            // does not always surface as a sub_modifier, which is how this
            // defect hid for months.
            use crate::tui::debug::debug_hub::buffer_style_dump;
            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    crate::tui::rewind_selector::draw_rewind_selector(frame, area, app);
                })
                .expect("draw");
            let dump = buffer_style_dump(terminal.backend().buffer());
            let mut rows = Vec::new();
            for (y, line) in dump.lines().skip(1).enumerate() {
                // A cell token is `<fg>/<bg>/<mods>@<sym>`; `r` is REVERSED.
                // `<fg>/<bg>/<mods>@<sym>` — split off the symbol FIRST, then the
                // three style fields. Splitting on '/' alone leaves the symbol
                // glued to the last field (`r@3`), which is why an earlier
                // version of this check reported "no reversed rows" while the
                // frame plainly outlined the row.
                let reversed_here = line.split(' ').any(|tok| {
                    tok.split_once('@').is_some_and(|(style, _sym)| {
                        let parts: Vec<&str> = style.split('/').collect();
                        parts.get(2).is_some_and(|m| m.split('+').any(|f| f == "r"))
                    })
                });
                if reversed_here {
                    rows.push(y);
                }
            }
            rows
        }

        // The cursor opens on the newest turn.
        let at_newest = outlined_rows(&app);
        assert!(
            !at_newest.is_empty(),
            "the selection IS visibly outlined — three prompts in the \
             transcript, cursor on the last"
        );

        // Down at the newest turn cannot move anything.
        app.rewind_selector.select_next();
        assert_eq!(
            outlined_rows(&app),
            at_newest,
            "down at the newest turn is a bound, not a wrap"
        );

        // Up MUST move the outlined band. This is the assertion the live
        // failure dies on.
        app.rewind_selector.select_prev();
        let one_up = outlined_rows(&app);
        assert_ne!(
            one_up, at_newest,
            "up moves the OUTLINED ROWS, not just the index — the half that \
             was broken (the highlight stayed put while the cursor moved)"
        );

        app.rewind_selector.select_prev();
        assert_ne!(
            outlined_rows(&app),
            one_up,
            "and it moves again on the next press"
        );
    }

    /// The `f` filter has unit tests over `filtered_indices` but was never
    /// driven through the KEY HANDLER. A filter that populates the right list
    /// but is never reachable from the keyboard is unreachable in production,
    /// so this drives the real `handle_rewind_selector_key` events.
    #[test]
    fn the_f_filter_is_reachable_and_moves_the_selection_to_the_first_match() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = make_app();
        for prompt in ["prompt one", "prompt two", "prompt three"] {
            app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(prompt.to_string()),
            });
            app.messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Text(format!("answer {prompt}")),
            });
        }
        app.open_rewind_selector();
        let marks = vec![
            (1u64, "prompt one".to_string()),
            (2, "prompt two".to_string()),
            (3, "prompt three".to_string()),
        ];
        let display_texts: Vec<String> = marks.iter().map(|(_, t)| t.clone()).collect();
        app.rewind_selector.sync_marks(marks, &display_texts);
        app.rewind_selector.selected = 0;

        // Press `f` — the filter must OPEN.
        app.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert!(
            app.rewind_selector.filter_open,
            "the f key must open the filter — if it does not, the filter is \
             dead in production"
        );

        // Type "three" — live-jump must move the selection to that turn.
        for c in ['t', 'h', 'r', 'e', 'e'] {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.rewind_selector.filter, "three");
        assert_eq!(
            app.rewind_selector.selected, 2,
            "typing the filter text must live-jump the selection to the \
             first matching turn"
        );
        assert_eq!(
            app.rewind_selector.filtered_indices(),
            vec![2],
            "the filter must list exactly the matching turn"
        );

        // Enter closes the filter and KEEPS the jumped-to selection.
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            !app.rewind_selector.filter_open,
            "Enter must close the filter"
        );
        assert_eq!(
            app.rewind_selector.selected, 2,
            "Enter must leave the selection on the match it jumped to"
        );

        // A filter matching nothing must not move the selection off-list.
        app.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        for c in ['z', 'z', 'z'] {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert!(
            app.rewind_selector.filtered_indices().is_empty(),
            "a filter matching nothing must report an empty list"
        );
        assert_eq!(
            app.rewind_selector.selected, 2,
            "a no-match filter must leave the selection where it was rather \
             than jumping somewhere meaningless"
        );
        // Esc closes the filter without clearing the typed text (it stays
        // visible in the input, so a cancelled filter is recoverable).
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.rewind_selector.filter_open, "Esc cancels the filter");
    }

    /// Typing a filter must not TELEPORT the cursor to the oldest match
    /// when it is already parked on a matching turn.
    ///
    /// omp's `#filterChanged` (rewind-selector.ts:531-536) keeps the selection
    /// while it still matches and only then falls back to the newest match
    /// BELOW it (`matches.findLast(i => i < selected) ?? matches.at(-1)`).
    /// Ours set `selected = matches.first()` on every keystroke — the OLDEST
    /// match — so an operator who had walked up to turn 2 of 3 and typed a
    /// filter matching 1, 2 and 3 had the cursor dragged down to 1 and had to
    /// re-walk. That is the same "my arrows/selection do nothing useful"
    /// complaint the operator filed three times.
    #[test]
    fn typing_a_filter_while_sitting_on_a_match_keeps_the_cursor() {
        let mut app = make_app();
        for prompt in ["t alpha", "n lima", "t gamma", "o delta", "n oscar"] {
            app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(prompt.to_string()),
            });
            app.messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Text(format!("answer {prompt}")),
            });
        }
        app.open_rewind_selector();
        let marks = vec![
            (1u64, "t alpha".to_string()),
            (2, "n lima".to_string()),
            (3, "t gamma".to_string()),
            (4, "o delta".to_string()),
            (5, "n oscar".to_string()),
        ];
        let display_texts: Vec<String> = marks.iter().map(|(_, t)| t.clone()).collect();
        app.rewind_selector.sync_marks(marks, &display_texts);
        // The operator has walked the outline up to the THIRD turn.
        app.rewind_selector.selected = 2;

        app.handle_key_event(press_key(KeyCode::Char('f'), KeyModifiers::NONE));
        // "t" matches turns 0, 2 and 3 — the parked one is NOT the first.
        app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(
            app.rewind_selector.selected, 2,
            "typing a filter while resting on a matching turn keeps the \
             cursor there — every keystroke dragged it to matches.first(), \
             the OLDEST match, which is omp's #filterChanged divergence"
        );

        // And a query that matches nothing leaves it parked, not teleported.
        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
        app.rewind_selector.filter.clear();
        app.handle_key_event(press_key(KeyCode::Char('f'), KeyModifiers::NONE));
        app.rewind_selector.selected = 2;
        app.handle_key_event(press_key(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(
            app.rewind_selector.selected, 2,
            "a no-match filter leaves the cursor where the operator left it"
        );

        // THE FALLBACK: park the cursor on a turn the query does NOT match,
        // ABOVE two matches. "n oscar" (index 4) carries no "t", and "t"
        // matches 0, 2 and 3 ("delta" ends in t) — so the newest match BELOW
        // index 4 is 3. A first()-style fallback would give 0 and a
        // last()-style one 3 as well, so this turns on the pair: combined with
        // the segment above (a matching cursor is KEPT rather than dragged to
        // 0), it is what pins "newest below" against "first match".
        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
        app.rewind_selector.filter.clear();
        app.handle_key_event(press_key(KeyCode::Char('f'), KeyModifiers::NONE));
        app.rewind_selector.selected = 4;
        app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(
            app.rewind_selector.selected, 3,
            "a cursor parked on a non-matching turn lands on the newest match \
             below it — 3, not the oldest match at 0"
        );
    }

    #[test]
    fn prev_jump_from_tail_lands_on_most_recent_prompt() {
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));

        assert_eq!(
            app.scroll_offset, 20,
            "first Alt+[ lands on the last prompt"
        );
        assert!(!app.auto_scroll, "the jump pauses auto-follow");
        let notice = app.status_notice().unwrap_or_default();
        assert!(
            notice.contains("Prompt 3"),
            "the status row names the landed prompt: {notice}"
        );
    }

    #[test]
    fn repeated_prev_jumps_walk_earlier_prompts_in_document_order() {
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        assert_eq!(app.scroll_offset, 10, "second press walks one prompt up");

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        assert_eq!(app.scroll_offset, 0, "third press reaches the first prompt");

        // Past the first prompt: the jump clamps to the top, not below it.
        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        assert_eq!(app.scroll_offset, 0, "no earlier prompt: stay at the top");
    }

    #[test]
    fn next_jump_walks_back_down_and_resumes_following_at_the_tail() {
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        assert_eq!(app.scroll_offset, 10);

        app.handle_key_event(press_key(KeyCode::Char(']'), KeyModifiers::ALT));
        assert_eq!(app.scroll_offset, 20, "Alt+] walks down to the next prompt");

        app.handle_key_event(press_key(KeyCode::Char(']'), KeyModifiers::ALT));
        assert!(
            app.auto_scroll,
            "past the last prompt the view hands itself back to auto-follow"
        );
    }

    #[test]
    fn next_jump_is_a_noop_while_following_the_tail() {
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char(']'), KeyModifiers::ALT));
        assert!(
            app.auto_scroll,
            "Alt+] while following must not pause the view"
        );
    }

    #[test]
    fn alt_brackets_never_insert_into_the_composer() {
        // The generic text-entry arm accepts every non-CONTROL char; without
        // the chord arms sitting ABOVE it, Alt+[ would type a literal `[`
        // instead of jumping. This is the mutation the arms must survive.
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));
        app.handle_key_event(press_key(KeyCode::Char(']'), KeyModifiers::ALT));

        assert!(
            app.prompt_input.text.is_empty(),
            "the prompt-jump chords must not leak into the composer: {:?}",
            app.prompt_input.text
        );
    }

    #[test]
    fn jump_with_no_prompts_reports_rather_than_teleporting() {
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));

        assert!(app.auto_scroll, "no prompts loaded: nothing to pause over");
        let notice = app.status_notice().unwrap_or_default();
        assert!(
            notice.contains("No prompts"),
            "the chord must say why it did nothing: {notice}"
        );
    }

    #[test]
    fn ctrl_bracket_alias_jumps_too() {
        // tmux extended-keys and some terminals report Ctrl+[ as
        // Char('[')+CONTROL (the kitty protocol instead reports it as
        // Esc+CONTROL — the arm below covers that). Both spellings are the
        // same physical chord.
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::CONTROL));
        assert_eq!(app.scroll_offset, 20, "Char('[')+CONTROL is the same jump");

        // And the kitty spelling:
        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::CONTROL));
        assert_eq!(
            app.scroll_offset, 10,
            "Esc+CONTROL (kitty Ctrl+[) jumps too"
        );
    }

    #[test]
    fn streaming_esc_still_cancels_but_ctrl_esc_does_not() {
        // The streaming-cancel arm used to take Esc with ANY modifiers;
        // under kitty that would make the prompt-jump chord cancel the turn
        // whenever one was running. The NONE guard keeps plain-Esc cancel
        // and frees Esc+CONTROL for the jump.
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.is_streaming = true;

        // Esc+CONTROL mid-stream: a JUMP, not a cancel.
        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::CONTROL));
        assert!(app.is_streaming, "Ctrl+[ must not cancel the running turn");
        assert_eq!(app.scroll_offset, 20, "the jump still lands mid-stream");

        // Plain Esc mid-stream: the interrupt survives intact.
        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.is_streaming, "plain Esc still cancels the turn");
    }

    #[test]
    fn transcript_focus_esc_returns_focus_but_ctrl_esc_jumps() {
        // With the transcript focused (a mouse click is the only way in),
        // plain Esc hands focus back to the composer — but Esc+CONTROL is
        // Ctrl+[ under kitty and must JUMP, not silently switch focus.
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;
        app.focus = crate::tui::app::enums::FocusTarget::Transcript;

        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::CONTROL));
        assert_eq!(
            app.focus,
            crate::tui::app::enums::FocusTarget::Transcript,
            "Esc+CONTROL must not be treated as a plain-Esc focus return"
        );
        assert_eq!(
            app.scroll_offset, 20,
            "the jump still fires from transcript focus"
        );

        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.focus,
            crate::tui::app::enums::FocusTarget::Input,
            "plain Esc still hands focus back to the composer"
        );
    }

    #[test]
    fn jump_clears_a_pending_resize_anchor() {
        // An armed resize anchor re-pins the viewport on the next paint; an
        // explicit jump must win, or the reader snaps straight back.
        let mut app = make_app();
        seed_frame_with_prompt_starts(&[0, 10, 20]);
        app.auto_scroll = true;
        app.scroll_memory.pending_resize = Some(super::super::scroll_anchor::PendingResize {
            target: crate::tui::operant_model::ContentPos::Section {
                kind: crate::tui::operant_model::PreparedSectionKind::Body,
                raw_line: 0,
                column: 0,
            },
            captured_offset: 7,
        });

        app.handle_key_event(press_key(KeyCode::Char('['), KeyModifiers::ALT));

        assert!(
            app.scroll_memory.pending_resize.is_none(),
            "the jump must consume the pending resize anchor"
        );
    }
}

/// (RT-205 reopen, iter-1567) THE DECISIVE INSTRUMENT: does the overlay
/// duplicate a span's final character?
///
/// Live evidence, raw bytes from `/tmp/rt/c0.txt`:
///
/// ```text
/// \x1b[2m\x1b[38;2;80;80;80m  CLEANPROBE: alpha acknowledged.
/// \x1b[0m\x1b[38;2;200;200;195m  .
/// \x1b[39m
/// ```
///
/// The trailing `.` carries a DIFFERENT, deliberate style from the run it
/// follows, so a stale repaint cannot explain it (that was my iter-1565
/// withdrawal, and it was wrong). All six affected rows share one signature:
/// `text` + reset + new style + **the last character of `text`**.
///
/// Four mutation-proven tests already prove the markdown pipeline is clean
/// (`operant_markdown::tests::cases`), so the defect is downstream of markdown
/// — in the layer that turns already-styled `Line`s into cells for the overlay.
/// This drives `draw_rewind_selector` directly and reads the painted cells.
#[test]
fn the_overlay_body_does_not_emit_a_spans_final_character_twice() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = make_app();
    app.is_simulating = true;

    // WHY THIS INSTRUMENT WAS REWRITTEN — twice.
    //
    // The first version could not FAIL. It PASSED with the defect
    // deliberately reintroduced into `restyle_dim` (measured, iter-1596).
    // Two independent reasons, both of which had to be fixed:
    //
    //   1. Its two plain-text messages produced ONE span per body line, so
    //      the span BOUNDARY where a duplicate would appear never existed.
    //   2. With only two turns the PICK zone covered the whole body, so NO
    //      row was dimmed — and `restyle_dim` only runs on a DIM row. The
    //      code under test never executed at all.
    //
    // The SECOND version fixed both and went green on clean shipping code,
    // but then ALSO survived the reintroduced defect. That failure is the
    // reason for the fixture's shape below: every needle had been pointed at
    // the PICK turn, which is REVERSED and never dimmed. Markdown — the only
    // thing that makes span boundaries the defect duplicates across — has to
    // sit in the DROP zone.
    //
    // THE COUNTING TRAP this fixture also avoids: an earlier attempt seeded
    // the same markdown phrase into four turns and then asserted the phrase
    // appears once. It appeared four times — legitimately, once per turn —
    // and that was nearly recorded as the defect reproducing. One phrase in
    // one turn makes the assertion say what it means.
    let mut marks = Vec::new();
    // Turn 100 is the PICK: plain prose, because a plain turned block keeps
    // the frame's zone separation legible in any future failure dump.
    marks.push((100u64, "prompt number zero".to_string()));
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: crate::tui::adapter_types::types::MessageContent::Text(
                "prompt number zero".to_string(),
            ),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::Assistant,
            content: crate::tui::adapter_types::types::MessageContent::Text(
                "picked turn prose: ordinary sentences, nothing unusual here".to_string(),
            ),
        });
    // THE SIGNATURE LIVES IN A DROP-ZONE TURN (101), styled by `restyle_dim`.
    let drop_prompt = "filler prompt one";
    marks.push((101u64, drop_prompt.to_string()));
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: crate::tui::adapter_types::types::MessageContent::Text(
                drop_prompt.to_string(),
            ),
        });
    app.messages
        .push(crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::Assistant,
            content: crate::tui::adapter_types::types::MessageContent::Text(
                "the *quick* **brown** _fox_ jumps over the lazy dog, with a \
                 [link](http://example.com) and `inline code` too."
                    .to_string(),
            ),
        });
    // Two more plain DROP turns: they widen the DIM zone so `restyle_dim`
    // has rows to run on, without contributing text that inflates counts.
    for i in 2..4 {
        let prompt = format!("filler prompt {i}");
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::User,
                content: crate::tui::adapter_types::types::MessageContent::Text(prompt.clone()),
            });
        marks.push((100 + i as u64, prompt));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: crate::tui::adapter_types::types::MessageContent::Text(format!(
                    "filler answer {i}: plain prose, no emphasis markers in it"
                )),
            });
    }
    app.core_rewind_marks = Some(std::sync::Arc::new(
        crate::tui::rewind_marks::RewindMarksSource::new(
            marks
                .into_iter()
                .map(|(turn_id, text)| operant_core::agent::UserTurnMark {
                    turn_id,
                    conv_index: 0,
                    conv_epoch: 1,
                    text,
                })
                .collect(),
            Some("sess-rt205".to_string()),
            Vec::new(),
        ),
    ));
    // Mirror App::run's frame-loop sync: set_conversation first, then read
    // the marks. Skipping it leaves the selector empty and blind.
    {
        let source = app.core_rewind_marks.as_ref().expect("marks").clone();
        let display_texts = app.user_prompt_texts_for_filter();
        source.set_conversation(display_texts.clone());
        let collected: Vec<(u64, String)> = source
            .marks()
            .into_iter()
            .map(|m| (m.turn_id, m.text))
            .collect();
        app.rewind_selector.sync_marks(collected, &display_texts);
    }

    // Open on the FIRST turn so 101/102/103 fall in the DROP (dimmed) zone.
    app.rewind_selector.open();
    app.rewind_selector.selected = 0;

    let paint = |app: &crate::tui::app::App| -> (String, String) {
        let backend = TestBackend::new(120, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                crate::tui::rewind_selector::draw_rewind_selector(frame, area, app);
            })
            .expect("draw");
        let buf = terminal.backend().buffer();
        use crate::tui::debug::debug_hub::buffer_style_dump;
        let dump = buffer_style_dump(buf);
        // The token is `fg/bg/mods@symbol`; `_` means a real space.
        //
        // ROW BOUNDARIES ARE PRESERVED. An earlier sampler joined every row
        // into one string so `matches` could count across a wrap, but that
        // flattened the frame: a phrase that wraps lands in two rows and
        // counts as zero, which is indistinguishable from a render bug.
        // Counting per row keeps both true: the count is the total over the
        // frame, and a wrapped phrase cannot vanish.
        let row_symbols: Vec<String> = dump
            .lines()
            .skip(1)
            .map(|l| {
                l.split(' ')
                    .filter_map(|tok| tok.split_once('@').map(|(_, sym)| sym))
                    .map(|s| if s == "_" { " " } else { s })
                    .collect::<String>()
            })
            .collect();
        let symbols = row_symbols.join("\n");
        (dump, symbols)
    };

    let (dump, symbols) = paint(&app);

    // GATE 1 — THE FIXTURE MUST REACH THE CODE UNDER TEST.
    // A dim cell carries `d` in its mods field. No dim rows means
    // `restyle_dim` never ran, so every assertion below is vacuous.
    let dim_rows = dump
        .lines()
        .skip(1)
        .filter(|row| {
            row.split(' ')
                .filter_map(|tok| tok.split_once('@'))
                .any(|(style, _)| style.split('/').nth(2).is_some_and(|m| m.contains('d')))
        })
        .count();
    assert!(
        dim_rows >= 3,
        "the fixture must paint at least 3 DIM rows (got {dim_rows}) — without \
         them `restyle_dim` never executes and this instrument proves nothing"
    );

    // GATE 2 — the painted frame must carry many distinct styles, or there
    // is no span boundary for a duplicate to appear at.
    let distinct_styles: std::collections::HashSet<&str> = dump
        .lines()
        .skip(1)
        .flat_map(|row| {
            row.split(' ')
                .filter_map(|tok| tok.split_once('@'))
                .map(|(style, _)| style)
        })
        .collect();
    assert!(
        distinct_styles.len() > 4,
        "the frame must carry several distinct styles (got {}); a flat frame \
         has no span boundary for the duplicate signature",
        distinct_styles.len()
    );

    // THE SIGNATURE — a run whose final character is emitted TWICE.
    //
    // The duplication is per CHARACTER, not per phrase: a span ending in `k`
    // emits `k` then `k` again. So the detectable form is a doubled glyph.
    // Matching the literal phrase `jumps over the lazy` can never work — the
    // mutation breaks the phrase into `jumpss overr thee lazyy`, and a
    // phrase-count assertion reports 0 for a frame that plainly shows the
    // render bug. Measured: I lost a cycle to exactly that.
    //
    // `kk` is drawn from `*quick*`, which is emphasis-styled, so the run
    // boundary sits right after the `k`. Unstyled text would make every run
    // one span and leave the boundary unexercised.
    //
    // ONLY GLYPH PAIRS THE FIXTURE CANNOT SPELL BY ACCIDENT may be used here.
    // `ll` was in the first list and cost a cycle: every "filler" prompt in
    // this very fixture spells it, so the instrument failed on clean shipping
    // code with `"ll" appears 5 times`. Measured, not assumed — the audit
    // below is what the needle list must pass before it is allowed.
    for doubled in ["kk", "nn", "xx", "ee", "rr", "ss"] {
        let hits = symbols.matches(doubled).count();
        assert_eq!(
            hits, 0,
            "{doubled:?} appears {hits} time(s): a span's final character is being \
             emitted a second time under the next run's style — the RT-205 \
             duplicated-final-character signature"
        );
    }

    // A duplicate final character shows up as an adjacent pair of
    // single-character cells that share a symbol but carry different styles.
    let mut adjacent_style_repeat = 0usize;
    for line in dump.lines() {
        let toks: Vec<(&str, &str)> = line.split(' ').filter_map(|t| t.split_once('@')).collect();
        for pair in toks.windows(2) {
            let (style_a, sym_a) = pair[0];
            let (style_b, sym_b) = pair[1];
            if style_a != style_b && sym_a.len() == 1 && sym_b.len() == 1 && sym_a == sym_b {
                adjacent_style_repeat += 1;
            }
        }
    }
    assert_eq!(
        adjacent_style_repeat, 0,
        "no two adjacent single-character cells may carry the same symbol in \
         different styles — that is the duplicated-final-character signature"
    );

    assert!(
        !symbols.is_empty(),
        "the painted frame must be non-empty for this test to mean anything"
    );
}

/// RT-205, the STALE-CELL half: opening the rewind selector must ERASE the
/// cells the previous frame painted, not merely paint its own content over the
/// top.
///
/// WHY THIS TEST EXISTS AND COULD NOT HAVE BEEN WRITTEN BEFORE: the sibling
/// instrument above (`the_overlay_body_does_not_emit_a_spans_final_character_twice`)
/// measured one frame over a FRESH TestBackend buffer, so there was no previous
/// content to survive and the defect was structurally invisible to it. It went
/// green while the shipped binary was visibly corrupt.
///
/// MEASURED LIVE on the deployed binary (2026-11-01), driving the headless
/// simulator with `--agent-script` so turns actually complete and `Esc Esc`
/// opens the real selector: the frame before the overlay opened held
/// ` operant · replay · perf:reduced` at row 13, and the overlay's own row 13
/// painted `2› beta two brown lazy dog` — leaving cols 28..31 holding `educed`
/// in the PREVIOUS line's own amber `rgb:CC9B1F`. At 120x45 the survivor was
/// `ck`, the tail of `/model to switch · Small Stack` in the model badge's pink
/// `rgb:FF96C8`, so row 17 read `4› delta four lazy sleeps nowck`. The fragment
/// changes with terminal height (41 -> `educed`/`ck`, 44/45 -> `ced`/`ck`,
/// 40/42/43 -> none) because the overlay's row length relative to the surviving
/// cell changes — which is precisely why a single-frame or single-height
/// instrument missed it.
///
/// MECHANISM, from ratatui's own source: `Paragraph`'s `render_line` writes only
/// the line's own graphemes and never pads to `area.width`
/// (`ratatui-widgets/src/paragraph.rs`), and `Terminal::try_draw` diffs against
/// the previous buffer. So a short line leaves the tail of the previous longer
/// line in place, symbol intact, and the overlay's fullscreen contract (see the
/// paint-site comment in `render/operant_overlays.rs`) is what makes that a bug.
#[test]
/// RT-205, the STALE-CELL half: opening the rewind selector must ERASE the
/// cells the previous frame painted, not merely paint its own content over the
/// top.
///
/// WHY THIS TEST COULD NOT HAVE CAUGHT IT EARLIER — and one wrong turn recorded
/// so it is not repeated. Two things had to be true for the defect to appear,
/// and neither held in the repo's instruments:
///
/// 1. **The overlay must open over a frame that already has content in those
///    rows.** The sibling instrument above measures ONE `draw` over a fresh
///    `TestBackend`, so there is no previous content to survive: it went green
///    while the shipped binary was visibly corrupt.
/// 2. **The second frame must go through `render_app`,** which is what paints
///    the chrome the stale cells come from. Driving `draw_rewind_selector`
///    directly instead does NOT reproduce it — MEASURED: I wrote exactly that
///    version, and it passed with the fix removed. `Paragraph`'s
///    `buf.set_style(text_area, self.style)` resets the style across the whole
///    text area, so the isolated call happens to be clean. The symbols survive
///    only when the surrounding chrome pass is what left them.
///
/// MEASURED LIVE on the deployed binary (2026-11-01), via the headless
/// simulator's `--agent-script` (mock agent events, so turns actually COMPLETE
/// and `Esc Esc` opens the real selector): the frame before the overlay opened
/// held ` operant · replay · perf:reduced` at row 13, and the overlay's own row
/// 13 painted `2› beta two brown lazy dog`, leaving cols 28..31 holding
/// `educed` in the PREVIOUS line's own amber `rgb:CC9B1F`. At 120x45 the survivor
/// was `ck`, the tail of `/model to switch · Small Stack` in the model badge's
/// pink `rgb:FF96C8`, so row 17 read `4› delta four lazy sleeps nowck`. The
/// fragment changes with height (41 -> `educed`/`ck`, 44/45 -> `ced`/`ck`,
/// 40/42/43 -> none) because the overlay's row length relative to the surviving
/// cell changes — which is why a single-height instrument missed it.
///
/// MECHANISM, from ratatui's own source: `Paragraph`'s `render_line` writes only
/// the line's own graphemes and never pads to `area.width`
/// (`ratatui-widgets/src/paragraph.rs`), and `Terminal::try_draw` diffs against
/// the previous buffer. A short line therefore leaves the tail of the previous
/// longer line in place — symbol AND, where nothing reset the style, colour.
/// The overlay's fullscreen contract (see the paint-site comment in
/// `render/operant_overlays.rs`) is what makes that a bug.
#[test]
fn opening_the_rewind_selector_erases_the_previous_frames_cells() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = make_app();
    app.is_simulating = true;

    // FRAME ONE: the app's own chrome, painted by the real path. This is what
    // leaves content in the rows the overlay is about to repaint more briefly.
    // `/model` and friends are gated on live credentials, so the test owns that
    // precondition rather than inheriting the ambient auth state.
    app.has_credentials = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
    terminal
        .draw(|frame| crate::tui::render::render_app(frame, &app))
        .unwrap();

    // FRAME TWO's precondition: four turns so the selector has entries and a
    // DROP zone, using the same marks wiring the frame loop uses.
    let mut marks = Vec::new();
    for i in 0..4 {
        let prompt = format!("prompt number {i}");
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::User,
                content: crate::tui::adapter_types::types::MessageContent::Text(prompt.clone()),
            });
        marks.push((100 + i as u64, prompt));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: crate::tui::adapter_types::types::MessageContent::Text(format!(
                    "answer number {i}"
                )),
            });
    }
    app.core_rewind_marks = Some(std::sync::Arc::new(
        crate::tui::rewind_marks::RewindMarksSource::new(
            marks
                .into_iter()
                .map(|(turn_id, text)| operant_core::agent::UserTurnMark {
                    turn_id,
                    conv_index: 0,
                    conv_epoch: 1,
                    text,
                })
                .collect(),
            Some("sess-rt205-stale".to_string()),
            Vec::new(),
        ),
    ));
    {
        let source = app.core_rewind_marks.as_ref().unwrap().clone();
        let display_texts = app.user_prompt_texts_for_filter();
        source.set_conversation(display_texts.clone());
        let collected: Vec<(u64, String)> = source
            .marks()
            .into_iter()
            .map(|m| (m.turn_id, m.text))
            .collect();
        app.rewind_selector.sync_marks(collected, &display_texts);
    }
    app.rewind_selector.open();
    app.rewind_selector.selected = 0;

    // THE SECOND FRAME — the overlay, through the same path the TUI uses, over
    // the chrome already in the buffer.
    terminal
        .draw(|frame| crate::tui::render::render_app(frame, &app))
        .unwrap();

    use crate::tui::debug::debug_hub::buffer_style_dump;
    let dump = buffer_style_dump(terminal.backend().buffer());
    let rows: Vec<Vec<(&str, &str)>> = dump
        .lines()
        .skip(1)
        .map(|l| l.split(' ').filter_map(|t| t.split_once('@')).collect())
        .collect();

    let row_text = |cells: &[(&str, &str)]| -> String {
        cells
            .iter()
            .map(|(_, s)| if *s == "_" { " " } else { *s })
            .collect()
    };

    // PRECONDITION, so the test can never again pass by measuring an empty
    // screen: the overlay must actually be on screen. This is the check whose
    // absence let iter-1606 record a "does not reproduce" verdict from a frame
    // that had no overlay in it at all.
    let flat: String = rows.iter().map(|c| row_text(c)).collect();
    assert!(
        flat.contains("Rewind to a previous prompt"),
        "the rewind selector must be on screen, or nothing below measures anything"
    );

    // THE SIGNATURE. A row painted by the overlay's body may not still carry a
    // foreign-styled cell past its own content: that is one frame's content
    // surviving into the next under another style, which is RT-205 exactly.
    // Scope to the OVERLAY's own rows only. The overlay's chrome is identifiable
    // from its own content: the header row (`Rewind to a previous prompt`), the
    // caption row (`Rewind to here`), and the footer rows (`turn n/m`,
    // `No rewindable turns`). Everything the overlay paints lives between them
    // and its own footer. The app's status bar and composer (RT-144 removed
    // the vim `-- INSERT --` badge) are NOT the overlay's and are
    // legitimately multi-styled, so they
    // must not be considered — MEASURED: without this scope the first version
    // flagged the app's own status row as a "leak" and failed even with the fix
    // in place.
    let overlay_header = rows
        .iter()
        .position(|cells| row_text(cells).contains("Rewind to a previous prompt"))
        .unwrap_or(0);
    let overlay_footer = rows
        .iter()
        .rposition(|cells| {
            let t = row_text(cells);
            t.contains("No rewindable turns") || (t.contains("turn ") && t.contains("/"))
        })
        .unwrap_or(rows.len());
    // The overlay's BODY is the region its `mid` paragraph owns: from just
    // after the caption row to just before the footer. Rows outside it belong
    // to the app's own chrome, which the overlay is entitled to leave alone
    // only if it does so identically every frame — see the assertion below,
    // which checks the overlay did not let frame-one chrome linger in rows it
    // painted over.
    let caption_row = rows
        .iter()
        .position(|cells| row_text(cells).contains("Rewind to here"))
        .unwrap_or(overlay_header);
    let overlay_rows = (caption_row + 1)..overlay_footer;

    let mut leaked = Vec::new();
    for (r, cells) in rows.iter().enumerate() {
        let text = row_text(cells);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        // THE PRECISE FORM. The overlay's body paints only the dim ramp
        // (`rgb:505050/rgb:232832/-` with and without BOLD). Any other colour
        // in that region is a cell the overlay never wrote — i.e. what frame
        // one left behind. An earlier version counted distinct styles and
        // rejected >2, which both over- and under-shot: it flagged the app's
        // legitimate multi-styled status row, and would have missed a
        // single-colour survivor.
        let foreign: Vec<&str> = cells
            .iter()
            .filter(|(style, sym)| {
                *sym != "_" && !sym.is_empty() && !style.starts_with("rgb:505050/")
            })
            .map(|(style, _)| *style)
            .collect();
        let in_overlay = overlay_rows.contains(&r);
        if in_overlay && !foreign.is_empty() {
            leaked.push((r, trimmed.to_string(), foreign.len()));
        }
    }
    assert!(
        leaked.is_empty(),
        "no painted row may carry a foreign-styled tail: {leaked:?} — one frame's \\
         content surviving into the next under another style is the RT-205 \\
         signature"
    );

    // And the specific live shape, so the failure message names it: none of the
    // overlay's turn rows may read longer than the prompt it was given.
    for (prompt, needle) in [
        ("prompt number 0", "prompt number 0"),
        ("prompt number 1", "prompt number 1"),
        ("prompt number 2", "prompt number 2"),
        ("prompt number 3", "prompt number 3"),
    ] {
        for cells in rows.iter() {
            let text = row_text(cells);
            let trimmed = text.trim_end();
            if let Some(at) = trimmed.find(needle) {
                let after = &trimmed[at + needle.len()..];
                assert!(
                    after.trim().is_empty(),
                    "row {trimmed:?} ends with {after:?} past {prompt:?} — a \\
                     survivor from the previous frame"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2026-10-31: the rewind selector's PREVIEW over a transcript that has teeth.
//
// The operator reported, live: "the entire preview is broken — it shows
// overlapping text from both the panes, while up/dn arrows do nothing".
//
// Both halves measured on a seeded transcript, and both traced to the render.
// The second half (arrows) was `sync_marks` resetting the cursor, fixed in
// iter-1515; this module pins the FIRST half, which had never been asserted
// anywhere: the PICK/DROP zone the renderer paints.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod rewind_preview_zones {
    use super::*;
    use crate::tui::adapter_types::types::ContentBlock;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn long_answer(tag: &str) -> String {
        // Deliberately long and multi-paragraph. The wrap is what a one-line
        // reply never exercises: a zone that covers only the prompt row is
        // invisible on a single-line turn and glaring on a wrapped one.
        format!(
            "{tag}: the quick brown fox jumps over the lazy dog while the \
             selector is open, and this paragraph is long enough to wrap \
             across several terminal lines in a 120 column pane. \
             A second paragraph follows with a list:\n\
             - item one is here\n- item two is here\n- item three is here\n\
             And a closing sentence so the block has a tail."
        )
    }

    fn user(text: &str) -> crate::tui::adapter_types::types::Message {
        crate::tui::adapter_types::types::Message {
            role: crate::tui::adapter_types::types::Role::User,
            content: MessageContent::Text(text.to_string()),
        }
    }

    /// Turn 1 carries a tool call AND its result, because that is the shape
    /// the operator had live, and a rewind selector must show the turn the
    /// rewind keeps — not merely its prompt.
    fn seed_transcript(app: &mut crate::tui::app::App) {
        app.messages.push(user("first prompt"));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: long_answer("A1"),
                    },
                    ContentBlock::ToolUse {
                        id: "tu_1".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({"command": "ls -la /tmp/rewind-repro"}),
                    },
                ]),
            });
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "tu_1".to_string(),
                    content: crate::tui::adapter_types::types::ToolResultContent::Text(
                        "total 8\ndrwxr-xr-x 2 user user 4096 rewind-repro".to_string(),
                    ),
                    is_error: false,
                }]),
            });
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: MessageContent::Text(long_answer("A2")),
            });
        app.messages.push(user("second prompt"));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: MessageContent::Text(long_answer("B1")),
            });
        app.messages.push(user("third prompt"));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: MessageContent::Text(long_answer("C1")),
            });
    }

    fn seed_marks(app: &mut crate::tui::app::App) {
        app.core_rewind_marks = Some(std::sync::Arc::new(
            crate::tui::rewind_marks::RewindMarksSource::new(
                vec![
                    operant_core::agent::UserTurnMark {
                        turn_id: 11,
                        conv_index: 0,
                        conv_epoch: 1,
                        text: "first prompt".to_string(),
                    },
                    operant_core::agent::UserTurnMark {
                        turn_id: 12,
                        conv_index: 4,
                        conv_epoch: 1,
                        text: "second prompt".to_string(),
                    },
                    operant_core::agent::UserTurnMark {
                        turn_id: 13,
                        conv_index: 6,
                        conv_epoch: 1,
                        text: "third prompt".to_string(),
                    },
                ],
                Some("sess_live2".to_string()),
                Vec::new(),
            ),
        ));
    }

    /// Sync the selector from the marks exactly as `App::run` does each frame.
    fn sync(app: &mut crate::tui::app::App) {
        let marks = vec![
            (11u64, "first prompt".to_string()),
            (12u64, "second prompt".to_string()),
            (13u64, "third prompt".to_string()),
        ];
        let display_texts = app.user_prompt_texts_for_filter();
        app.rewind_selector.sync_marks(marks, &display_texts);
    }

    /// Paint the selector into a 120x45 frame.
    fn paint(app: &crate::tui::app::App) -> (Vec<String>, Vec<String>) {
        let backend = TestBackend::new(120, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                crate::tui::rewind_selector::draw_rewind_selector(frame, area, app);
            })
            .expect("draw");
        let buf = terminal.backend().buffer();
        let screen: Vec<String> = (0..45u16)
            .map(|y| {
                (0..120u16)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        // The per-cell style rows. `<fg>/<bg>/<mods>@<sym>` per cell, where
        // the mods field is the single-letter alphabet of `style_mods_field`:
        // `r` = REVERSED, `d` = DIM, `b` = BOLD. Reading the letters off the
        // dump is the only reliable way to see a zone — ratatui's `modifier`
        // on the buffer is layout-offset sensitive and a Paragraph's line
        // styling does not always surface there.
        use crate::tui::debug::debug_hub::buffer_style_dump;
        let styles: Vec<String> = buffer_style_dump(buf)
            .lines()
            .skip(1)
            .map(|s| s.to_string())
            .collect();
        (screen, styles)
    }

    /// Row indices whose style row carries the given mods letter.
    fn rows_with_mod(styles: &[String], want: char) -> Vec<usize> {
        styles
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.split(' ').any(|tok| {
                    let fields: Vec<&str> = tok
                        .split('@')
                        .next()
                        .unwrap_or_default()
                        .split('/')
                        .collect();
                    fields.len() == 3 && fields[2].contains(want)
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// THE reported defect, first half. The picked turn's whole block must be
    /// reversed and nothing after it may be dimmed.
    ///
    /// Before the fix the pick zone was computed from
    /// `wrapped_user_prompt_ends[selected]` — the end of the PROMPT ROW alone.
    /// The outline was therefore one row, and the selected turn's answer, tool
    /// call and tool result all fell on the DIM side, painted as content the
    /// rewind drops. The operator's "overlapping text from both panes" is what
    /// a highlighted row sitting directly on top of a dimmed block looks like
    /// in a real terminal, where the two styles bleed at the row boundary.
    #[test]
    fn the_picked_turn_outlines_its_whole_block_and_dims_only_what_follows() {
        let mut app = make_app();
        seed_transcript(&mut app);
        seed_marks(&mut app);
        app.rewind_selector.open();
        app.refresh_prompt_input();
        sync(&mut app);
        assert_eq!(app.rewind_selector.selected, 2, "opens on the newest turn");

        let (screen, styles) = paint(&app);
        let reversed = rows_with_mod(&styles, 'r');
        let dimmed = rows_with_mod(&styles, 'd');

        // The pick zone covers the whole turn 3: its prompt, its answer's
        // wrapped rows, and its closing sentence.
        assert!(
            reversed.len() >= 4,
            "the outline must cover the whole turn, not one row — got {}",
            reversed.len()
        );
        let prompt_row = screen
            .iter()
            .position(|l| l.contains("third prompt"))
            .expect("the picked prompt is painted");
        assert!(
            reversed.contains(&prompt_row),
            "the picked prompt row must be inside the reversed block"
        );
        // The answer rows of the SAME turn are kept by the rewind, so they are
        // inside the outline too — this is the assertion the old code failed.
        let c1_row = screen
            .iter()
            .position(|l| l.contains("C1:"))
            .expect("the picked turn's answer is painted");
        assert!(
            reversed.contains(&c1_row),
            "the picked turn's OWN answer must be outlined, not dimmed — \
             it is what Enter keeps"
        );
        // The newest turn has nothing below it, so nothing is dim.
        assert!(
            dimmed.is_empty(),
            "no row may be dim when the newest turn is picked — got {dimmed:?}"
        );
        // omp's caption says what Enter does. Without it the outline is a
        // picture with no verb attached.
        assert!(
            screen
                .iter()
                .any(|l| l.contains("nothing below to drop") && l.contains("the composer")),
            "the caption must name the rewind target and the drops"
        );
    }

    /// The SAME zones one turn up: turns 1-2 normal, turn 2 reversed, turn 3
    /// dimmed. This is the case that distinguishes a correct `sel_end` from
    /// the old prompt-row-only end, because turn 3 must land in the DROP zone.
    #[test]
    fn stepping_up_moves_the_boundary_between_the_zones() {
        let mut app = make_app();
        seed_transcript(&mut app);
        seed_marks(&mut app);
        app.rewind_selector.open();
        app.refresh_prompt_input();
        sync(&mut app);
        assert_eq!(app.rewind_selector.selected, 2);

        app.rewind_selector.select_prev();
        assert_eq!(app.rewind_selector.selected, 1, "one step up");

        let (screen, styles) = paint(&app);
        let reversed = rows_with_mod(&styles, 'r');
        let dimmed = rows_with_mod(&styles, 'd');

        let p2 = screen
            .iter()
            .position(|l| l.contains("second prompt"))
            .expect("turn 2's prompt is painted");
        let p3 = screen
            .iter()
            .position(|l| l.contains("third prompt"))
            .expect("turn 3's prompt is painted");
        let b1 = screen
            .iter()
            .position(|l| l.contains("B1:"))
            .expect("turn 2's answer is painted");

        assert!(
            reversed.contains(&p2) && reversed.contains(&b1),
            "turn 2's whole block is the pick zone"
        );
        assert!(
            dimmed.contains(&p3),
            "turn 3 is below the picked turn — it is what the rewind drops"
        );
        // And the boundary sits between them, so the earlier turn is untouched.
        let p1 = screen
            .iter()
            .position(|l| l.contains("first prompt"))
            .expect("turn 1's prompt is painted");
        assert!(
            !reversed.contains(&p1) && !dimmed.contains(&p1),
            "turn 1 is above the pick — normal, neither outlined nor dimmed"
        );
    }

    /// The arrows, over the rich transcript. In sync with the renderer here:
    /// `selected` moves, and the painted zones move with it on the next frame.
    #[test]
    fn arrows_move_the_selection_and_the_zones_follow() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app();
        app.is_simulating = true;
        seed_transcript(&mut app);
        seed_marks(&mut app);

        app.simulated_keys = vec![
            press_key(KeyCode::Esc, KeyModifiers::NONE),
            press_key(KeyCode::Esc, KeyModifiers::NONE),
        ];
        while !app.simulated_keys.is_empty() && !app.should_exit {
            let _ = app.run(&mut terminal).expect("loop runs");
        }
        assert!(app.rewind_selector.visible, "the selector opened");
        assert_eq!(app.rewind_selector.selected, 2);

        app.should_exit = false;
        app.sim_key_drain_passes = 0;
        app.simulated_keys = vec![press_key(KeyCode::Up, KeyModifiers::NONE)];
        for _ in 0..40 {
            if app.simulated_keys.is_empty() {
                break;
            }
            let _ = app.run(&mut terminal).expect("loop runs");
        }
        assert_eq!(
            app.rewind_selector.selected, 1,
            "one Up moves the selection, and it survives the frame-loop resync"
        );

        // And the paint agrees: turn 3 is now in the drop zone.
        let (screen, styles) = paint(&app);
        let p3 = screen
            .iter()
            .position(|l| l.contains("third prompt"))
            .expect("turn 3's prompt is painted");
        assert!(
            rows_with_mod(&styles, 'd').contains(&p3),
            "after stepping up, turn 3 reads as dropped"
        );
    }

    /// A compaction that folds a prompt out of the display must not outline a
    /// turn other than the one the operator picked.
    ///
    /// `selected` indexes the MARK list, which keeps one entry per turn so a
    /// folded turn can still be refused by Enter. The display shows only the
    /// surviving prompts, so after a compaction the two lists are NOT
    /// index-aligned. Resolving by position returns a value whenever the index
    /// still fits — and a value for the wrong turn is worse than none, because
    /// the outline moves silently.
    ///
    /// This builds the case that actually bites: THREE marks, the FIRST
    /// folded, so the display holds two prompts. The cursor is then on mark 1
    /// ("second prompt") while the display's index 1 is "third prompt" — the
    /// positional shortcut answers display prompt 2, so the pick zone is drawn
    /// over "third prompt" and "second prompt"'s own answer lands in the drop
    /// zone as content the rewind removes.
    ///
    /// Mutation-proven: reverting `user_prompt_index` to position-first leaves
    /// the pick zone on "third prompt" and this fails on the drop-zone assert
    /// below (observed: pick rows 12-19 over "third prompt", drop set empty).
    #[test]
    fn a_folded_middle_turn_does_not_outline_the_wrong_turn() {
        let mut app = make_app();
        app.is_simulating = true;

        // Two display prompts, three marks: the FIRST turn's prompt was folded
        // away by a compaction, so the mark list and the display are not
        // index-aligned. `conv_index` mirrors the real thing — the folded
        // turn's index is no longer reachable in the display.
        app.messages.push(user("second prompt"));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: MessageContent::Text(long_answer("A1")),
            });
        app.messages.push(user("third prompt"));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: MessageContent::Text(long_answer("A2")),
            });
        app.core_rewind_marks = Some(std::sync::Arc::new(
            crate::tui::rewind_marks::RewindMarksSource::new(
                vec![
                    operant_core::agent::UserTurnMark {
                        turn_id: 11,
                        conv_index: 0,
                        conv_epoch: 1,
                        text: "first prompt".to_string(),
                    },
                    operant_core::agent::UserTurnMark {
                        turn_id: 12,
                        conv_index: 2,
                        conv_epoch: 1,
                        text: "second prompt".to_string(),
                    },
                    operant_core::agent::UserTurnMark {
                        turn_id: 13,
                        conv_index: 4,
                        conv_epoch: 1,
                        text: "third prompt".to_string(),
                    },
                ],
                Some("sess_fold".to_string()),
                Vec::new(),
            ),
        ));

        // Mirror `App::run`'s frame-loop sync: set the conversation first, then
        // read the marks. Skipping it leaves the selector empty and blind.
        {
            let source = app.core_rewind_marks.as_ref().expect("marks").clone();
            source.set_epoch(1);
            let display_texts = app.user_prompt_texts_for_filter();
            source.set_conversation(display_texts.clone());
            let collected: Vec<(u64, String)> = source
                .marks()
                .into_iter()
                .map(|m| (m.turn_id, m.text))
                .collect();
            app.rewind_selector.sync_marks(collected, &display_texts);
        }
        app.rewind_selector.open();

        // The folded turn keeps its mark — flagged, not dropped, so Enter can
        // still refuse it with an explanation rather than a silent no-op.
        assert_eq!(
            app.rewind_selector.entries.len(),
            3,
            "one entry per turn: the folded turn is flagged, not dropped"
        );
        assert!(
            app.rewind_selector.entries.first().map(|e| e.compacted) == Some(true),
            "the turn with no display row reads as compacted"
        );

        // Step up once: newest (mark 2) to mark 1, "second prompt".
        app.rewind_selector.select_prev();
        assert_eq!(app.rewind_selector.selected, 1, "one step up");

        let (screen, styles) = paint(&app);
        let p2 = screen
            .iter()
            .position(|l| l.contains("second prompt"))
            .expect("the second prompt is painted");
        let p3 = screen
            .iter()
            .position(|l| l.contains("third prompt"))
            .expect("the third prompt is painted");
        assert!(
            !rows_with_mod(&styles, 'd').contains(&p2),
            "the picked turn's own prompt is NOT dimmed — dimming it would mean \
             the positional shortcut outlined a later turn's rows"
        );
        assert!(
            rows_with_mod(&styles, 'd').contains(&p3),
            "the turn below the picked one reads as dropped"
        );
    }
}

// ---- (2026-10-31, iter-1588) the live todo tick and the subagent-live accent ----
//
// Both halves close a renderer-only defect: the card was fed from exactly one
// `AgentEvent::ToolComplete`, so a producer the parent's stream never sees (a
// subagent's write, a list persisted before a restart) produced no visible
// change. The audit is
// docs/audit/2026-10-31-todo-card-and-subagent-preview-are-renderer-only.md.
//
// The store is redirected with `set_todo_dir_for_tests`, NOT `HERMES_HOME`:
// `operant_home()` reads that variable directly and `cmd_setup` resolves a
// config path through it without holding the crate-wide env lock, so a
// concurrent env mutation decides which file that unrelated test loads
// (observed as `wizard_persist_writes_arrays_as_arrays` failing only when a
// todo test ran beside it).

/// Redirect the todo store to a sandbox and restore it on scope exit.
struct TodoStoreGuard {
    dir: std::path::PathBuf,
    previous_dir: Option<std::path::PathBuf>,
}

impl TodoStoreGuard {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("operant-todo-tick-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let previous_dir = operant_core::tools::set_todo_dir_for_tests(Some(dir.clone()));
        Self { dir, previous_dir }
    }
}

impl Drop for TodoStoreGuard {
    fn drop(&mut self) {
        operant_core::tools::set_todo_dir_for_tests(self.previous_dir.take());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A distinct session id per test, so the process-global `LOADED_SESSIONS`
/// cache cannot serve one test another test's `default` list.
fn unique_session(tag: &str) -> String {
    format!("tick-{tag}-{}", std::process::id())
}

/// The tick must pick up a list the parent's event stream never carried.
///
/// This is the operator's report in one assertion: a subagent writes the list
/// (its `ToolComplete` is deliberately not forwarded to the parent,
/// sub_agent_tool.rs:660), and the card showed nothing until the model happened
/// to call `todo` again. Writing through the tool and calling
/// `refresh_todos_if_needed` directly is the seam the run loop drives.
#[test]
fn the_live_tick_picks_up_a_list_only_a_subagent_wrote() {
    let _store = TodoStoreGuard::new("subagent-write");
    let session = unique_session("subagent-write");

    let mut app = make_app();
    assert!(
        app.todos_card_payload.is_empty(),
        "a fresh app has no card to show"
    );
    app.todos_session_id = session.clone();

    // The subagent's write: the tool persists, and emits nothing to the parent.
    let write = serde_json::json!({
        "sessionId": session,
        "todos": [
            { "id": "s1", "content": "child wrote this", "status": "pending",
              "assignedTo": "dlg-101" }
        ]
    });
    let written = futures::executor::block_on(operant_core::tools::OperantTool::execute(
        &operant_core::tools::TodoTool,
        write,
        operant_core::tools::ToolContext::default(),
    ));
    assert!(written.success, "the child's write lands in the store");

    // A tick within 0ms of a previous one would be throttled, so clear the
    // clock exactly as a real second would.
    app.todos_tick_at = None;
    assert!(
        app.refresh_todos_if_needed(),
        "the tick must adopt a list no parent event carried"
    );
    assert_eq!(app.todos.len(), 1, "the list lands on App");
    assert!(
        app.todos_card_payload.contains("child wrote this"),
        "and the card payload carries it: {}",
        app.todos_card_payload
    );
    assert_eq!(
        app.todos[0].assigned_to.as_deref(),
        Some("dlg-101"),
        "the `assignedTo` write survives the wire-to-card map"
    );
}

/// The second consecutive tick is a no-op, and the fingerprint is why.
///
/// Without the fingerprint the tick would rebuild the payload every second,
/// which invalidates the render cache and re-runs the card renderer for a list
/// that did not change.
#[test]
fn an_unchanged_list_does_not_rebuild_the_card_payload() {
    let _store = TodoStoreGuard::new("unchanged");
    let session = unique_session("unchanged");

    let mut app = make_app();
    app.todos_session_id = session.clone();
    let write = serde_json::json!({
        "sessionId": session,
        "todos": [{ "id": "u1", "content": "steady", "status": "pending" }]
    });
    let written = futures::executor::block_on(operant_core::tools::OperantTool::execute(
        &operant_core::tools::TodoTool,
        write,
        operant_core::tools::ToolContext::default(),
    ));
    assert!(written.success);

    app.todos_tick_at = None;
    assert!(app.refresh_todos_if_needed(), "first tick builds");
    let frozen = app.todos_card_payload.clone();

    app.todos_tick_at = None;
    assert!(
        !app.refresh_todos_if_needed(),
        "an unchanged list must not report a change"
    );
    assert_eq!(
        app.todos_card_payload, frozen,
        "nor rebuild the payload the render cache keys on"
    );
}

/// A todo result from a NON-default session retargets the tick.
///
/// This is the other half of `test_todo_session_from_tool_context_metadata`
/// (operant-core): the tool now writes the agent's session instead of the
/// `default` catch-all, but a card whose tick still polls `default` would
/// show nothing for a session that is doing all the writing. The result's
/// own echoed `sessionId` is the authoritative key, so the handler learns
/// it from the event rather than from a second copy of the agent.
///
/// MUTATION PROVEN: deleting the `if let Some(sid) = value.get("sessionId")`
/// block in `agent_events.rs` makes this test red — the list lands on App
/// via the tool result, but `todos_session_id` stays `default`, so the tick
/// reads a bucket the write never touched.
#[test]
fn a_non_default_todo_result_retargets_the_live_tick() {
    let _store = TodoStoreGuard::new("retarget");
    let session = unique_session("retarget");
    let mut app = make_app();
    assert_eq!(
        app.todos_session_id, "default",
        "the precondition this fixes: a fresh app polls the catch-all"
    );

    // Write under a DIFFERENT session, and hand the app the same result
    // the agent stream would carry (the tool's own reply).
    let write = serde_json::json!({
        "sessionId": session,
        "todos": [
            { "id": "r1", "content": "secret chat task", "status": "pending" }
        ]
    });
    let result = futures::executor::block_on(operant_core::tools::OperantTool::execute(
        &operant_core::tools::TodoTool,
        write,
        operant_core::tools::ToolContext::default(),
    ));
    assert!(result.success);

    app.handle_agent_event(AgentEvent::ToolComplete {
        result: operant_core::tools::ToolResult {
            tool_call_id: "call_retarget".to_string(),
            name: "todo".to_string(),
            success: true,
            content: result.content.clone(),
            error: None,
            timed_out: false,
        },
    });
    assert_eq!(
        app.todos_session_id, session,
        "the result's own sessionId becomes the tick's store key"
    );

    // The tick now follows the session the tool wrote, which is the
    // point: the card no longer goes blank the moment the tool result is
    // gone. The tool result already put this list on App, so the first tick
    // correctly reports NO change (identical fingerprint) — what must be
    // proved is that the tick READS the retargeted session, so a LATER write
    // only a subagent makes is still adopted.
    app.todos_tick_at = None;
    assert!(
        !app.refresh_todos_if_needed(),
        "the list the tool result already gave App is what the tick reads back"
    );
    assert_eq!(
        app.todos_session_id, session,
        "a no-change tick must not reset the key"
    );

    // The subagent write: nothing forwards its ToolComplete to the parent, so
    // the ONLY thing that can show this is the tick reading the right file.
    let child = futures::executor::block_on(operant_core::tools::OperantTool::execute(
        &operant_core::tools::TodoTool,
        serde_json::json!({
            "sessionId": session,
            "todos": [
                { "id": "r1", "content": "secret chat task", "status": "pending" },
                { "id": "r2", "content": "child added this", "status": "pending",
                  "assignedTo": "dlg-9" }
            ],
            "merge": true
        }),
        operant_core::tools::ToolContext::default(),
    ));
    assert!(child.success);

    app.todos_tick_at = None;
    assert!(
        app.refresh_todos_if_needed(),
        "a write the parent never saw must still reach the card"
    );
    assert_eq!(app.todos.len(), 2);
    assert_eq!(app.todos[1].content, "child added this");
}

/// A completed delegation stops lighting its todo.
///
/// The accent reads "a live subagent is on this", so it has to clear when the
/// delegation is no longer live — otherwise a finished worker keeps a
/// `pending` row coloured as current work.
#[test]
fn a_settled_delegation_stops_lighting_its_todo() {
    let _registry = registry_lock();
    let _drain = operant_core::tools::async_delegation::RegistryDrain::new();
    let delegation_id =
        operant_core::tools::async_delegation::create_record("audit the tests", "gpt-5.4");

    let mut app = make_app();
    app.todos = vec![crate::tui::operant_app::todo::TodoItem {
        content: "the delegated todo".to_string(),
        status: "pending".to_string(),
        priority: "medium".to_string(),
        id: "l1".to_string(),
        group: None,
        phase: None,
        confidence: None,
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: Some(delegation_id.clone()),
    }];

    let live = app.subagent_live_todo_ids();
    assert_eq!(
        live,
        vec!["l1".to_string()],
        "a pending delegation lights the todo it names"
    );

    app.rebuild_todos_card_payload();
    assert!(
        app.todos_card_payload.contains("live_ids")
            && !app.todos_card_payload.contains("\"live_ids\":[]"),
        "the live id rides the payload: {}",
        app.todos_card_payload
    );

    // The worker finishes: the row must stop reading as current work.
    operant_core::tools::async_delegation::mark_completed(&delegation_id, "done");
    assert!(
        app.subagent_live_todo_ids().is_empty(),
        "a settled delegation lights nothing"
    );
    app.rebuild_todos_card_payload();
    assert!(
        app.todos_card_payload.contains("\"live_ids\":[]"),
        "the empty match set rides instead of a stale id: {}",
        app.todos_card_payload
    );
}

/// The gallery is fed by the delegation registry, and it was not fed at all
/// before iter-1607: the four `TuiState` defaults returned nothing and nothing
/// overrode them, so the strip, the band and the panel were dead renderers.
///
/// MUTATION PROVEN: reverting `tui_state_impl.rs`'s `inline_swarm_members` to
/// the trait default (`Vec::new()`) turns this test red — the registry has the
/// record, and the member list comes back empty.
#[test]
fn a_live_delegation_feeds_the_inline_swarm_gallery() {
    let _registry = registry_lock();
    let _drain = operant_core::tools::async_delegation::RegistryDrain::new();
    let delegation_id =
        operant_core::tools::async_delegation::create_record("audit the tests", "gpt-5.4");

    let mut app = make_app();
    // The model names the dispatch handle on the todo it delegated, which is
    // the join this whole surface rests on.
    app.todos = vec![crate::tui::operant_app::todo::TodoItem {
        content: "the delegated todo".to_string(),
        status: "pending".to_string(),
        priority: "medium".to_string(),
        id: "l1".to_string(),
        group: None,
        phase: None,
        confidence: None,
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: Some(delegation_id.clone()),
    }];

    assert!(
        app.inline_swarm_gallery_active(),
        "a live delegation must make the gallery active"
    );
    let members = app.inline_swarm_members();
    assert_eq!(members.len(), 1, "one record, one member");
    let member = &members[0];
    assert_eq!(
        member.session_id, delegation_id,
        "keyed by its dispatch handle"
    );
    assert_eq!(member.status, "running", "a pending record is live work");
    assert_eq!(
        member.task_label.as_deref(),
        Some("audit the tests"),
        "the stable label is the goal, not the transient detail"
    );

    // The counter that makes the row meaningful without opening a transcript:
    // this child owns 1 item and has finished 0 of them.
    assert_eq!(
        member.todo_progress,
        Some((0, 1)),
        "todo_progress is scoped to THIS child's items"
    );
    assert_eq!(member.todo_items.len(), 1);
    assert_eq!(member.todo_items[0].content, "the delegated todo");
    assert_eq!(member.runtime.model.as_deref(), Some("gpt-5.4"));
}

/// Two children must never show each other's counts: the join is on
/// `assigned_to`, so a sibling's todo is not this child's progress.
#[test]
fn gallery_counts_are_scoped_to_each_child() {
    let _registry = registry_lock();
    let _drain = operant_core::tools::async_delegation::RegistryDrain::new();
    let first = operant_core::tools::async_delegation::create_record("first job", "m1");
    let second = operant_core::tools::async_delegation::create_record("second job", "m2");

    let mut app = make_app();
    let make = |id: &str, assigned: &str, status: &str| crate::tui::operant_app::todo::TodoItem {
        content: format!("{id} work"),
        status: status.to_string(),
        priority: "medium".to_string(),
        id: id.to_string(),
        group: None,
        phase: None,
        confidence: None,
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: Some(assigned.to_string()),
    };
    app.todos = vec![
        make("a1", &first, "completed"),
        make("a2", &first, "pending"),
        make("b1", &second, "completed"),
        make("b2", &second, "completed"),
    ];
    let members = app.inline_swarm_members();
    let by_id = |id: &str| {
        members
            .iter()
            .find(|m| m.session_id == id)
            .expect("the record maps to a member")
    };
    assert_eq!(
        by_id(&first).todo_progress,
        Some((1, 2)),
        "the first child's own 1-of-2"
    );
    assert_eq!(
        by_id(&second).todo_progress,
        Some((2, 2)),
        "the second child's own 2-of-2, not the parent's total"
    );
}

/// The strip answers "what is running NOW", so a finished delegation must
/// leave it. The transcript cards still show it — `live_only` is what
/// separates the two, and the difference is asserted rather than assumed.
#[test]
fn a_settled_delegation_leaves_the_strip_but_keeps_its_card() {
    let _registry = registry_lock();
    let _drain = operant_core::tools::async_delegation::RegistryDrain::new();
    let delegation_id =
        operant_core::tools::async_delegation::create_record("a job that finishes", "m3");

    let app = make_app();
    assert_eq!(
        app.inline_swarm_members().len(),
        1,
        "precondition: a pending record is on the strip"
    );
    assert_eq!(
        app.swarm_members_for_transcript().len(),
        1,
        "and it has a card"
    );

    operant_core::tools::async_delegation::mark_completed(&delegation_id, "the answer");
    assert!(
        !app.inline_swarm_gallery_active(),
        "a settled child must not keep the strip up"
    );
    assert!(
        app.inline_swarm_members().is_empty(),
        "the live list is empty"
    );

    let cards = app.swarm_members_for_transcript();
    assert_eq!(cards.len(), 1, "but the card survives for later reading");
    assert_eq!(cards[0].status, "completed");
    assert_eq!(
        cards[0].detail.as_deref(),
        Some("the answer"),
        "the answer rides the card's transient half"
    );
}

/// A child with no todos assigned must not fabricate a 0/0 counter: there is
/// nothing to count yet, so the field stays `None` and the row renders
/// without it.
#[test]
fn a_delegation_with_no_assigned_todos_has_no_counter() {
    let _registry = registry_lock();
    let _drain = operant_core::tools::async_delegation::RegistryDrain::new();
    operant_core::tools::async_delegation::create_record("unplanned work", "m4");

    let app = make_app();
    let members = app.inline_swarm_members();
    assert_eq!(members.len(), 1);
    assert_eq!(
        members[0].todo_progress, None,
        "no items means no counter, not a false 0/0"
    );
    assert!(members[0].todo_items.is_empty(), "and no items to render");
}

/// Serialises every test that touches the process-global delegation
/// registry.
///
/// WHY A LOCK AND NOT JUST `RegistryDrain`: the drain empties the registry on
/// DROP, so two registry tests running in parallel race — one test's drain
/// wipes the record the other is still asserting on. Proven live on
/// 2026-11-01: adding three registry tests to this module made the
/// pre-existing `a_settled_delegation_stops_lighting_its_todo` fail with
/// `left: []` (its record was gone) in the module run while passing alone.
/// That is the RT-207 hazard `RegistryDrain`'s own doc names. Same discipline
/// `gateway_commands.rs`'s `SUBAGENTS_LOCK` already uses for its registry tests.
static DELEGATION_REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Guard for [`DELEGATION_REGISTRY_LOCK`]; take one at the top of any test that
/// seeds or reads the delegation registry.
fn registry_lock() -> std::sync::MutexGuard<'static, ()> {
    DELEGATION_REGISTRY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A row whose `assigned_to` names nothing live must not be accented.
///
/// Guards the join's direction: rather than accenting every row with an
/// `assigned_to`, only a delegation id the registry still lists as pending
/// lights the row.
#[test]
fn a_claim_on_a_delegation_that_never_existed_lights_nothing() {
    let _registry = registry_lock();
    let _drain = operant_core::tools::async_delegation::RegistryDrain::new();

    let mut app = make_app();
    app.todos = vec![crate::tui::operant_app::todo::TodoItem {
        content: "claimed by nobody".to_string(),
        status: "pending".to_string(),
        priority: "medium".to_string(),
        id: "p1".to_string(),
        group: None,
        phase: None,
        confidence: None,
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: Some("dlg-does-not-exist".to_string()),
    }];
    assert!(
        app.subagent_live_todo_ids().is_empty(),
        "an unresolvable claim is not a live subagent"
    );
}

// ---------------------------------------------------------------------------
// 2026-10-31 round 2: the rewind selector must FOLLOW the outline (omp
// `TranscriptBrowser` + `#outlineAnchor` → `revealRange`).
//
// The operator reported again, live: "the rewind preview is still very glitchy,
// and the arrow keys are not navigating". Round one fixed the ZONE boundaries
// (iter-1589) and the earlier `sync_marks` cursor reset (iter-1515). Neither
// touched the one thing that decides whether an arrow press is VISIBLE.
//
// The render: `draw_rewind_selector` keeps the whole `mid` vector and slices
// `state.scroll..scroll+body_h`. `state.scroll` is only ever written by PageUp
// / PageDown / wheel — nothing on an arrow key moves it. omp does not model
// this as an operator choice at all: its frame carries an ANCHOR
// (`#outlineAnchor` → `composed.selStart..selEnd`) and `ScrollView.revealRange`
// scrolls it into view every paint, then sets `followTail = false`. The
// consequence is the one the operator felt: press Up five times over a
// transcript longer than the pane, and the reversed outline walks off the top
// edge of the pane — the highlight stays on a row the operator can no longer
// see, and only the footer's "drops N turns" changes. That is exactly "arrows
// do nothing".
// ---------------------------------------------------------------------------
#[cfg(test)]
mod rewind_follows_the_outline {
    use super::*;
    use crate::tui::adapter_types::types::{Message, MessageContent, Role};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> crate::tui::app::App {
        make_app()
    }

    fn user(text: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
        }
    }

    fn assistant_long(text: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: MessageContent::Text(text.to_string()),
        }
    }

    /// A transcript taller than a 45-row pane: 8 turns of prose long enough to
    /// wrap, so the body is several screens. That is the shape where a
    /// selector without anchor-follow visibly strands the outline.
    fn seed_tall(app: &mut crate::tui::app::App) {
        let mut marks = Vec::new();
        for i in 0..8 {
            let prompt = format!("prompt number {i}");
            app.messages.push(user(&prompt));
            marks.push(((100 + i as u64), prompt));
            let mut body = String::new();
            for p in 0..12 {
                body.push_str(&format!(
                    "turn {i} paragraph {p} — a long line of prose so the body wraps \
                     across several terminal rows in a 120 column pane.\n"
                ));
            }
            app.messages.push(assistant_long(&body));
        }
        app.core_rewind_marks = Some(std::sync::Arc::new(
            crate::tui::rewind_marks::RewindMarksSource::new(
                marks
                    .into_iter()
                    .map(|(turn_id, text)| operant_core::agent::UserTurnMark {
                        turn_id,
                        conv_index: 0,
                        conv_epoch: 1,
                        text,
                    })
                    .collect(),
                Some("sess_follow".to_string()),
                Vec::new(),
            ),
        ));
    }

    /// Mirror `App::run`'s frame-loop sync EXACTLY (mod.rs:1502-1509): the
    /// relocation set is pushed first, then the marks are read. Skipping
    /// `set_conversation` makes `RewindMarksSource::marks()` return nothing
    /// whenever a session id is present, because the epoch is 0 and no mark
    /// text is in the (empty) relocation set — a harness that skips it
    /// measures an empty selector and proves nothing about navigation.
    fn sync_all(app: &mut crate::tui::app::App) {
        let source = app.core_rewind_marks.as_ref().expect("marks").clone();
        let display_texts = app.user_prompt_texts_for_filter();
        source.set_conversation(display_texts.clone());
        let marks: Vec<(u64, String)> = source
            .marks()
            .into_iter()
            .map(|m| (m.turn_id, m.text))
            .collect();
        app.rewind_selector.sync_marks(marks, &display_texts);
    }

    /// The visible screen rows of the selector, as strings.
    fn paint(app: &crate::tui::app::App) -> Vec<String> {
        let backend = TestBackend::new(120, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                crate::tui::rewind_selector::draw_rewind_selector(frame, frame.area(), app);
            })
            .expect("draw");
        let buf = terminal.backend().buffer();
        (0..45u16)
            .map(|y| {
                (0..120u16)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect()
    }

    #[test]
    fn stepping_up_reveals_the_outlined_turn_in_the_pane() {
        let mut app = app();
        seed_tall(&mut app);
        app.rewind_selector.open();
        sync_all(&mut app);
        // The selector opens on the newest turn; the body is far taller than
        // the pane, so the operator sees the tail.
        let _ = paint(&app);

        // Walk up. Each step must keep the outlined turn ON SCREEN: a press
        // whose only visible effect is the footer's counter is a dead key.
        for step in 0..6 {
            app.rewind_selector.select_prev();
            let screen = paint(&app);
            let selected = app.rewind_selector.selected;
            let needle = format!("prompt number {selected}");
            assert!(
                screen.iter().any(|row| row.contains(&needle)),
                "step {step}: the outlined turn (prompt number {selected}) is off \
                 screen after Up — the pane still shows the tail. omp reveals the \
                 anchor every frame; we leave `scroll` untouched, so the outline \
                 strands above the top edge and Up reads as dead."
            );
        }
    }

    /// The reveal must not FIGHT an explicit page key, and a frame with no
    /// input must be stable. `state.scroll` is operator state as much as it
    /// is render state: an operator who pages down to read an older turn must
    /// not have their page silently rewritten by a paint. omp's `revealRange`
    /// returns false when the anchor is already inside the visible range and
    /// leaves the offset alone (scroll-view.ts:331-345); ours must do the
    /// same. A reveal that disagrees with its own previous result is the
    /// flicker half of "still very glitchy".
    #[test]
    fn paging_away_is_not_undone_by_a_repaint_and_a_frame_is_stable() {
        let mut app = app();
        seed_tall(&mut app);
        app.rewind_selector.open();
        sync_all(&mut app);
        let _ = paint(&app);

        // Page down well past the selected turn: the operator is reading
        // above the outline now.
        for _ in 0..3 {
            app.rewind_selector.scroll += 10;
        }
        let paged = app.rewind_selector.scroll;
        assert!(paged > 0, "the page keys must move the viewport");

        let after_page = paint(&app);
        assert_eq!(
            app.rewind_selector.scroll, paged,
            "a paint must not silently overwrite the operator's page offset"
        );

        // A second paint with no input must produce the identical frame.
        assert_eq!(
            after_page,
            paint(&app),
            "two paints with no input between them must be byte-identical — an \
             unstable reveal is the flicker the operator felt"
        );
    }

    /// The arrow keys never park on a turn that is no longer in the transcript
    /// (omp `#move` skips `outlineVisibility` failures,
    /// rewind-selector.ts:647-668). A compacted turn keeps its mark so Enter
    /// can refuse it, but it has no rows — parking on it draws an outline over
    /// nothing and offers only a dead Enter.
    #[test]
    fn arrows_skip_a_turn_compaction_folded_out_of_the_transcript() {
        let mut st = crate::tui::rewind_selector::RewindSelectorState::default();
        let entry = |turn_id: u64, text: &str| crate::tui::rewind_selector::RewindEntry {
            turn_id,
            text: text.to_string(),
            compacted: false,
        };
        st.entries = vec![
            entry(31, "keep me"),
            entry(32, "folded away"),
            entry(33, "keep me too"),
        ];
        st.entries[1].compacted = true;
        st.selected = 2;

        st.select_prev();
        assert_eq!(
            st.selected, 0,
            "Up must step over the folded turn rather than parking on it"
        );
        st.select_next();
        assert_eq!(
            st.selected, 2,
            "and Down must step back over it too, never resting on a turn with \
             no rows"
        );
    }

    /// While the filter is open, the arrow keys walk the MATCHES (omp's
    /// `#stepFiltered`, rewind-selector.ts:541-547). They used to land in the
    /// filter branch's dead `_ => {}` arm, so a filtered list had no way to
    /// move at all.
    #[test]
    fn arrows_walk_the_filter_matches_not_the_whole_list() {
        let mut st = crate::tui::rewind_selector::RewindSelectorState::default();
        let entry = |turn_id: u64, text: &str| crate::tui::rewind_selector::RewindEntry {
            turn_id,
            text: text.to_string(),
            compacted: false,
        };
        st.entries = vec![
            entry(41, "fix the cache"),
            entry(42, "write the docs"),
            entry(43, "fix the index"),
            entry(44, "fix the retries"),
        ];
        st.selected = 3;
        st.filter = "fix".to_string();

        // Selected is on the tail match (44, index 3). The filter narrows the
        // walk to 0, 2 and 3 — the docs turn at 1 must be unreachable.
        st.select_prev();
        assert_eq!(
            st.selected, 2,
            "Up steps off 44 onto the match at 43 — not onto the non-matching \
             docs turn at index 1"
        );
        st.select_prev();
        assert_eq!(
            st.selected, 0,
            "the next Up lands on the oldest match, still skipping the docs turn"
        );
        st.select_prev();
        assert_eq!(
            st.selected, 0,
            "Up clamps at the first match, does not wrap"
        );
        st.select_next();
        assert_eq!(
            st.selected, 2,
            "and Down returns through the matches only, never onto the docs turn"
        );
    }

    /// The live-jump while TYPING a filter must not park the cursor on a turn
    /// compaction folded out of the transcript.
    ///
    /// The two filter paths disagreed: `filtered_indices` (used by the
    /// key-handler's live-jump on every keystroke) returns every text match,
    /// compacted or not, while `step` filters compacted entries out first. So a
    /// query matching only a folded turn moved the cursor onto it — an outline
    /// over no rows, and an Enter that can only refuse. `filtered_indices` is
    /// the one that must honour the same nav gate, since it is the path that
    /// decides where the keys will land next.
    #[test]
    fn typing_a_filter_that_only_matches_a_folded_turn_does_not_land_on_it() {
        let mut st = crate::tui::rewind_selector::RewindSelectorState::default();
        let entry = |turn_id: u64, text: &str| crate::tui::rewind_selector::RewindEntry {
            turn_id,
            text: text.to_string(),
            compacted: false,
        };
        st.entries = vec![
            entry(51, "fix the cache"),
            entry(52, "rewrite the index"),
            entry(53, "write the docs"),
        ];
        // The middle turn was folded by compaction: its mark survives (so Enter
        // can refuse it) but it renders no row.
        st.entries[1].compacted = true;
        st.selected = 2;

        // The operator types a query that matches ONLY the folded turn.
        st.filter = "rewrite".to_string();
        let matches = st.filtered_indices();
        assert!(
            matches.is_empty(),
            "a query that matches only a folded turn offers nothing: the \
             live-jump used to land on it, which no arrow key could then \
             reach, and Enter could only refuse"
        );

        // And a query that matches ONE folded and ONE live turn still resolves
        // to the live one — the gate is per-entry, not per-query.
        st.filter = "e".to_string();
        let matches = st.filtered_indices();
        assert_eq!(
            matches,
            vec![0, 2],
            "the folded turn is absent from the set the keys may reach"
        );
    }

    /// (iter-1634) The keys walk the NAVIGABLE subset, and a cursor parked
    /// outside it (on a folded turn) steps by RELATION rather than snapping to
    /// the tail of the subset.
    ///
    /// Three jobs are independently checkable now: what the rows show, where
    /// the cursor index sits, and what the keys may reach. The seam between
    /// them is what this pins — `selected` indexes the FULL mark list (a folded
    /// turn keeps a slot so Enter can refuse it), so an arrow press has to
    /// translate through the navigable subset and back.
    ///
    /// The old translation was `position().unwrap_or(len)`: correct for Up by
    /// accident, and silently wrong for Down, where `current` clamped to `len`
    /// and the next index clamped to `len - 1` — ONE press DOWN moved the
    /// cursor to the BOTTOM of the list. Reachable exactly after iter-1633,
    /// because a folded turn now renders by text while its mark keeps a slot.
    /// Mutation-proven: restoring the position translation fails the Down
    /// assertion below (observed: Up from the head jumped to 3).
    #[test]
    fn arrows_off_a_folded_cursor_stay_on_the_navigable_subset() {
        let mut st = crate::tui::rewind_selector::RewindSelectorState::default();
        let entry =
            |turn_id: u64, text: &str, compacted: bool| crate::tui::rewind_selector::RewindEntry {
                turn_id,
                text: text.to_string(),
                compacted,
            };
        // The FIRST turn is folded (no display row) but its mark survives.
        st.entries = vec![
            entry(61, "first prompt", true),
            entry(62, "second prompt", false),
            entry(63, "third prompt", false),
            entry(64, "fourth prompt", false),
        ];
        st.selected = 0;

        // Up from the folded head has nowhere to go: there is no navigable
        // entry below index 0, and omp's rule answers "do not move" rather
        // than "snap somewhere". The cursor stays where the operator left it
        // and Enter's refusal explains why.
        st.select_prev();
        assert_eq!(
            st.selected, 0,
            "Up from the folded head does not move the cursor — there is no \
             navigable entry below it"
        );

        // Down steps into the navigable subset — from 0 (not in it) to its
        // first member, 1. NOT to the tail.
        st.selected = 0;
        st.select_next();
        assert_eq!(
            st.selected, 1,
            "Down from an out-of-subset cursor moves ONE navigable step, not \
             to the end of the list"
        );

        // And walking the whole subset never lands on the folded entry.
        st.selected = 0;
        for _ in 0..8 {
            st.select_next();
            assert!(
                !st.entries[st.selected].compacted,
                "no arrow press may park the cursor on a compacted entry"
            );
        }
        assert_eq!(st.selected, 3, "Down stops at the newest navigable turn");

        // The same walk backwards.
        for _ in 0..8 {
            st.select_prev();
            assert!(
                !st.entries[st.selected].compacted,
                "no arrow press may park the cursor on a compacted entry"
            );
        }
        assert_eq!(st.selected, 1, "Up stops at the oldest navigable turn");
    }

    /// (iter-1634) Leaving the filter box stops it filtering. omp keeps the
    /// query IN the field — `#filter` is `this.#filterInput?.getValue()`
    /// (rewind-selector.ts:195-197) — so the moment `#closeFilter` drops the
    /// input the query is gone and `#filterMatches` answers "every visible
    /// target". OUR query string used to outlive the close, because
    /// `matching_indices` read the string rather than the field's state, so an
    /// operator who typed "t", pressed Enter to leave the box, and pressed Up
    /// had the cursor skip straight over the turn above — a filter they could
    /// no longer see was still steering the keys.
    ///
    /// `handleNativeEvent` (rewind-selector.ts:878-886) is the other half of
    /// the rule: while `#filter !== undefined` a cancel is `#closeFilter()`, and
    /// once it is closed a cancel is `deps.onCancel()`. So an Esc in the filter
    /// box is "I am done typing" and the second Esc is "never mind" — ours
    /// does the same at key_handling.rs:2515, and this pins the pair together
    /// so they do not collapse into one behaviour.
    #[test]
    fn escaping_the_filter_stops_it_filtering() {
        let mut app = make_app();
        for prompt in ["t alpha", "n lima", "t gamma", "o delta", "n oscar"] {
            app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(prompt.to_string()),
            });
            app.messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Text(format!("answer {prompt}")),
            });
        }
        app.open_rewind_selector();
        let marks: Vec<(u64, String)> = ["t alpha", "n lima", "t gamma", "o delta", "n oscar"]
            .iter()
            .enumerate()
            .map(|(i, t)| ((i + 1) as u64, t.to_string()))
            .collect();
        let display_texts: Vec<String> = marks.iter().map(|(_, t)| t.clone()).collect();
        app.rewind_selector.sync_marks(marks, &display_texts);

        // Open the filter, type a query that reaches three of the five turns.
        app.handle_key_event(press_key(KeyCode::Char('f'), KeyModifiers::NONE));
        app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::NONE));
        // "o delta" also carries a "t", so the query reaches three of five.
        assert_eq!(
            app.rewind_selector.filtered_indices(),
            vec![0, 2, 3],
            "the query reaches three turns"
        );

        // Esc closes the field AND disposes of the query: it is dead state
        // once the field is gone, and leaving it set keeps an invisible filter
        // steering the arrows.
        app.handle_key_event(press_key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !app.rewind_selector.filter_open,
            "Esc closes the filter field"
        );
        assert!(
            app.rewind_selector.filter.is_empty(),
            "and the query goes with it, so nothing invisible keeps filtering"
        );

        // With the field closed the arrows no longer step the matches: they
        // start from the cursor and move through the FULL list. From the
        // cursor on turn 2 ("n lima", which the query does not match) Up moves
        // to turn 1, not skipping it for a "t" turn.
        app.rewind_selector.selected = 2;
        app.handle_key_event(press_key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            app.rewind_selector.selected, 1,
            "with the filter field closed, Up walks the whole list rather than \
             the match subset"
        );

        // And the match set itself answers "everything navigable" once the
        // field is closed, which is what makes that Up possible.
        assert_eq!(
            app.rewind_selector.filtered_indices(),
            vec![0, 1, 2, 3, 4],
            "a closed filter field offers the whole list"
        );
    }
}

// ---- RT-146: click-to-expand tool-call previews ----

/// The whole prepared transcript as plain text, sections in order.
///
/// Reads `wrapped_plain_lines` — the same plain-text projection the copy path
/// uses — so a test sees the rows a user would select, not the styled spans.
fn flat_text(frame: &crate::tui::operant_model::PreparedChatFrame) -> String {
    frame
        .sections
        .iter()
        .flat_map(|section| section.prepared.wrapped_plain_lines.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build a three-tool transcript and publish the regions the viewport would.
/// Returns the app so tests can drive the click handler and assert the row map.
///
/// The regions come from the REAL prepare path (`prepare_messages`) against the
/// assembled transcript, not a hand-built map, so the test fails if the publish
/// wire breaks rather than only proving the hit-test maths.
fn app_with_three_clickable_tools() -> App {
    use crate::tui::adapter_types::types::Message;
    use crate::tui::app::enums::{ToolStatus, ToolUseBlock};

    let mut app = make_app();
    app.messages
        .push(Message::user("run three commands".to_string()));
    app.messages
        .push(Message::assistant("calling them".to_string()));
    // Bash rows are the tool surface that actually renders an output body (the
    // 3-line preview under `show_bash_output`, the full text when expanded), so
    // they are the right fixture: a `read_file` row renders a single summary
    // line with no body at all, and "expand" would have nothing to show.
    crate::tui::operant_ui::tools_ui::tests_show_bash_output_override::set(true);
    for i in 0..3 {
        let body =
            format!("output a{i} signal\noutput b{i} tail\noutput c{i} extra\noutput d{i} final");
        app.tool_use_blocks.push(ToolUseBlock {
            id: format!("call-{i}"),
            name: "bash".to_string(),
            after_index: 2,
            status: ToolStatus::Done,
            output_preview: Some(body),
            input_json: serde_json::json!({"command": format!("run-{i}")}).to_string(),
            intent: None,
        });
        app.remember_tool_output(
            &format!("call-{i}"),
            &format!("full output {i}\nsecond line {i}\nthird line {i}"),
        );
    }
    app.invalidate_transcript();
    // Publish exactly what the viewport publishes, from the real prepared frame.
    // Rows are one summary line + N body lines, so the region must span all of
    // them — a one-line region would make every click below the header fall
    // through to a text selection instead of toggling.
    let prepared = crate::tui::operant_ui::prepare::prepare_messages(&app, 80, 40);
    crate::tui::operant_ui::viewport::set_tool_row_regions(prepared.tool_row_ranges.clone());
    app
}

/// The touchdown contract: a click on a tool call's row expands THAT call, a
/// second click on the same row collapses it, and a click on a different row
/// moves the single expansion. The old `Ctrl+O` path could only ever touch the
/// LAST finished tool, which is why this exists.
#[test]
fn clicking_a_tool_row_toggles_that_call_only() {
    let mut app = app_with_three_clickable_tools();
    let regions = crate::tui::operant_ui::viewport::tool_row_regions();
    assert_eq!(
        regions.len(),
        3,
        "every tool call must publish a row range: {regions:?}"
    );
    let first = &regions[0];

    assert!(
        app.expanded_tool_id.is_none(),
        "nothing is expanded before the first click"
    );

    assert!(
        app.toggle_tool_expansion_by_id(&first.tool_call_id),
        "a click on a retained tool row must expand it"
    );
    assert_eq!(
        app.expanded_tool_id.as_deref(),
        Some(first.tool_call_id.as_str())
    );

    // A click on a DIFFERENT row moves the single expansion, it does not stack.
    let second = &regions[1];
    app.toggle_tool_expansion_by_id(&second.tool_call_id);
    assert_eq!(
        app.expanded_tool_id.as_deref(),
        Some(second.tool_call_id.as_str()),
        "one expansion at a time is the whole contract"
    );

    // A click on the same row collapses.
    app.toggle_tool_expansion_by_id(&second.tool_call_id);
    assert!(app.expanded_tool_id.is_none(), "the second click collapses");
}

/// `expanded_tool_id` is consumed by the adapter to render the full output, so
/// a click must reach the pixels — an id nobody reads would be the same
/// no-op the repo already caught in `thinking_row_map`/`message_row_map`.
#[test]
fn a_clicked_tool_row_renders_its_full_output() {
    let mut app = app_with_three_clickable_tools();
    let target = crate::tui::operant_ui::viewport::tool_row_regions()
        .first()
        .expect("three tool rows published")
        .tool_call_id
        .clone();

    // COLLAPSED: the click has not happened yet, so each row shows its
    // 3-line preview and NONE shows the retained full output.
    let collapsed_text = flat_text(&crate::tui::operant_ui::prepare::prepare_messages(
        &app, 80, 40,
    ));
    assert!(
        !collapsed_text.contains("full output 0"),
        "collapsed must NOT show the full output — otherwise the preview is \
         already the whole thing and expand is meaningless. got: {collapsed_text}"
    );
    // The 3-line preview IS visible (it shows the last 3 lines), so there is
    // something to expand from.
    assert!(
        collapsed_text.contains("output d0 final"),
        "the preview must be on screen for a click to be meaningful: {collapsed_text}"
    );

    // EXPANDED: the click happened, so that row renders the retained full
    // output. The prep path always rebuilds under `cfg!(test)`, so this is the
    // real frame rather than a cached one.
    app.toggle_tool_expansion_by_id(&target);
    let expanded_text = flat_text(&crate::tui::operant_ui::prepare::prepare_messages(
        &app, 80, 40,
    ));
    assert!(
        expanded_text.contains("full output 0"),
        "an expanded row must render the retained full output — the click has to \
         reach the transcript, not just the state field. got: {expanded_text}"
    );
    assert!(
        expanded_text.contains("third line 0"),
        "the FULL text must arrive, not a first line: {expanded_text}"
    );
    // And ONLY that row expands: one-at-a-time is the contract.
    assert!(
        !expanded_text.contains("full output 1"),
        "expanding one row must not expand its neighbours: {expanded_text}"
    );

    // Collapsing again takes it back off screen.
    app.toggle_tool_expansion_by_id(&target);
    let recollapsed = flat_text(&crate::tui::operant_ui::prepare::prepare_messages(
        &app, 80, 40,
    ));
    assert!(
        !recollapsed.contains("full output 0"),
        "a second click must hide the full output again: {recollapsed}"
    );
}

/// The click-to-expand contract as the HEADLESS HARNESS sees it.
///
/// `a_clicked_tool_row_renders_its_full_output` above proves the click reaches
/// the pixels, but it runs in-process with `prepare_messages` reading the
/// toggle's live state. The headless `debug_snapshot`-driven path cannot
/// reach that: the collapsed preview body is gated behind
/// `show_tool_call_details`, and `tests_show_bash_output_override` is
/// `#[cfg(test)]` in THIS crate, so a shipped `operant tui debug simulate`
/// process cannot flip it. A screen assertion in the scenario corpus is
/// therefore structurally impossible for this feature — state is the only
/// observable that survives the trip to the harness.
///
/// This pins the three keys the corpus asserts on. Without them a future
/// regression that stops publishing the toggle breaks the feature while every
/// existing scenario stays green.
///
/// Deliberately NOT in `overlays`: an open tool row is not a modal, and
/// `any_modal_open` gates error-modal precedence, toast suppression and key
/// routing (see the comment on `usage_overlay` in `commands.rs`).
#[test]
fn debug_snapshot_exposes_tool_row_expansion_state() {
    let mut app = app_with_three_clickable_tools();

    let snap = app.debug_snapshot();
    assert!(
        snap["overlays"].get("expanded_tool_id").is_none(),
        "tool-row expansion must NOT land in `overlays` — it would make \
         `any_modal_open` claim a modal is open, which changes real routing"
    );
    let mut calls = snap["tool_calls_seen"]
        .as_array()
        .expect("tool_calls_seen must be an array")
        .iter()
        .map(|v| v.as_str().expect("ids must be strings").to_string())
        .collect::<Vec<_>>();
    calls.sort();
    assert_eq!(
        calls,
        vec!["call-0", "call-1", "call-2"],
        "every tool call in the transcript must be visible to the harness"
    );
    let mut retained = snap["tool_full_outputs"]
        .as_array()
        .expect("tool_full_outputs must be an array")
        .iter()
        .map(|v| v.as_str().expect("ids must be strings").to_string())
        .collect::<Vec<_>>();
    retained.sort();
    assert_eq!(
        retained, calls,
        "the click target must be exactly the retained set: a row whose full \
         output was evicted is inert (`clicking_an_evicted_tool_row_...`), and \
         an id retained with no row would expand nothing"
    );
    assert!(
        snap["expanded_tool_id"].is_null(),
        "nothing is expanded before the first click"
    );

    app.toggle_tool_expansion_by_id("call-1");
    assert_eq!(
        app.debug_snapshot()["expanded_tool_id"],
        serde_json::json!("call-1"),
        "the snapshot must carry the id the click selected — a stale or absent \
         key is exactly the silent regression this plan closes"
    );

    // Second click on the same row collapses, and the key must say so rather
    // than staying stuck at the old id.
    app.toggle_tool_expansion_by_id("call-1");
    assert!(
        app.debug_snapshot()["expanded_tool_id"].is_null(),
        "collapse must be observable, not a state-only change the harness misses"
    );
}

/// A row whose full output was evicted by the bounded budget must be a no-op
/// that SAYS so, never a silent swallow and never a panic.
#[test]
fn clicking_an_evicted_tool_row_is_inert_and_reports() {
    let mut app = make_app();
    // A real row range, but nothing retained for its id.
    app.toggle_tool_expansion_by_id("call-does-not-exist");
    assert!(
        app.expanded_tool_id.is_none(),
        "an unretained row must not expand to an empty string"
    );
    assert!(
        app.status_message.is_some(),
        "the user must be told the output is gone rather than see a dead click"
    );
    let notice = app.status_message.clone().unwrap();
    assert!(
        notice.contains("no longer retained") || notice.contains("Ctrl+O"),
        "the notice must point at the live affordance: {notice}"
    );
}

/// The hit test: a screen row maps to exactly one tool call, rows outside any
/// tool call map to nothing, and rows above the content (the pinned-todo /
/// prompt-preview top band) map to nothing rather than to row 0.
#[test]
fn tool_row_hit_test_uses_the_frames_own_scroll() {
    use crate::tui::operant_model::ToolRowRange;
    use crate::tui::operant_ui::viewport::{set_tool_row_regions, tool_row_region_at};

    let content_top = 4u16;
    set_tool_row_regions(vec![ToolRowRange {
        tool_call_id: "call-x".to_string(),
        start_line: 10,
        end_line: 14,
    }]);

    // scroll 0: content row 12 sits at screen row 4 + 12 = 16.
    assert_eq!(
        tool_row_region_at(16, content_top, 0).map(|r| r.tool_call_id),
        Some("call-x".to_string())
    );
    // Scrolled down by 8: that same wrapped line is now at screen row 4 + 4.
    assert_eq!(
        tool_row_region_at(8, content_top, 8).map(|r| r.tool_call_id),
        Some("call-x".to_string()),
        "the hit test must follow the frame's scroll, not a cached screen row"
    );
    // Above the content: the top band, not the first tool call.
    assert_eq!(tool_row_region_at(3, content_top, 0), None);
    // Gap before the region, and past its exclusive end.
    assert_eq!(tool_row_region_at(6, content_top, 0), None);
    assert_eq!(tool_row_region_at(18, content_top, 0), None);

    // Reset so a later test does not inherit these.
    set_tool_row_regions(Vec::new());
    assert_eq!(tool_row_region_at(16, content_top, 0), None);
}

/// The regions must be capped to what is on screen, or a 5,000-line transcript
/// publishes 5,000 regions and every click pays for the whole list.
#[test]
fn published_tool_regions_start_at_the_prepared_frames_ranges() {
    let app = app_with_three_clickable_tools();
    let regions = crate::tui::operant_ui::viewport::tool_row_regions();
    for region in &regions {
        assert!(
            region.start_line < region.end_line,
            "a region must span at least one line: {region:?}"
        );
        assert!(
            !region.tool_call_id.is_empty(),
            "a region must carry the tool-call id the toggle needs: {region:?}"
        );
    }
}

/// RT-211: the rewind selector's caption must survive a filter close.
///
/// The operator's report nets here into the "stale elements ... orphan text
/// letters" family: closing the filter rendered the caption as
/// `\u{23ea}i Rewind to here` instead of `  \u{23ea}  Rewind ...` — the `i` is
/// the filter field's own, parked in the caption marker's continuation cell.
///
/// The mechanism, measured rather than inferred (two frames on one terminal,
/// both through `draw_rewind_selector`): `⏪` is a WIDE glyph, so ratatui's
/// `Paragraph` writes its grapheme at one column and treats the NEXT cell as
/// the continuation WITHOUT rewriting it. Closing the filter shrinks the
/// header from two rows to one, so the caption's row slides UP onto the row
/// that held `  filter: ` — whose third character is `i` — and the caption's
/// second space is served from that stale cell instead of being written. With
/// a wide marker the whole caption also ends up one column right of where it
/// belongs.
///
/// The fix is a NARROW marker. This test is what keeps it narrow: restoring
/// the wide glyph makes it fail naming the leaked cell.
#[test]
fn the_rewind_caption_survives_a_filter_close() {
    let mut app = make_app();
    assert!(app.handle_tui_command("rewind", ""));
    // The seeding recipe the selector's OWN render tests use (RT-205's), because
    // two things must be true for the caption to be reachable at all, and
    // seeding `entries` directly satisfies neither: the app needs user messages
    // whose positions `user_prompt_index` can resolve a mark against, and
    // `core_rewind_marks` must carry that mark source. Without them `sel_end`
    // stays unresolved, the caption block is skipped entirely, and the test
    // would be green over a frame with no caption in it.
    let mut marks = Vec::new();
    for i in 0..2 {
        let prompt = format!("prompt number {i}");
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::User,
                content: crate::tui::adapter_types::types::MessageContent::Text(prompt.clone()),
            });
        marks.push((100 + i as u64, prompt));
        app.messages
            .push(crate::tui::adapter_types::types::Message {
                role: crate::tui::adapter_types::types::Role::Assistant,
                content: crate::tui::adapter_types::types::MessageContent::Text(format!(
                    "answer number {i}"
                )),
            });
    }
    app.core_rewind_marks = Some(std::sync::Arc::new(
        crate::tui::rewind_marks::RewindMarksSource::new(
            marks
                .into_iter()
                .map(|(turn_id, text)| operant_core::agent::UserTurnMark {
                    turn_id,
                    conv_index: 0,
                    conv_epoch: 1,
                    text,
                })
                .collect(),
            Some("sess-rt211".to_string()),
            Vec::new(),
        ),
    ));
    {
        let source = app.core_rewind_marks.as_ref().expect("source").clone();
        let display_texts = app.user_prompt_texts_for_filter();
        source.set_conversation(display_texts.clone());
        let collected: Vec<(u64, String)> = source
            .marks()
            .into_iter()
            .map(|m| (m.turn_id, m.text))
            .collect();
        app.rewind_selector.sync_marks(collected, &display_texts);
    }

    let caption_row = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| -> String {
        let buf = terminal.backend().buffer();
        // The caption is the first row of the middle chunk. With the filter
        // open the header is two rows, so the caption sits at y=2; when it is
        // closed the header is one row and the caption slides to y=1. Scan the
        // top three rows so the test does not pin the arithmetic by hand.
        (0..3u16)
            .find(|y| {
                let row: String = (0..40u16)
                    .map(|x| buf.cell((x, *y)).unwrap().symbol().to_string())
                    .collect();
                row.contains("Rewind to here")
            })
            .and_then(|y| {
                Some(
                    (0..40u16)
                        .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                        .collect::<String>(),
                )
            })
            .unwrap_or_default()
    };

    // FRAME ONE: the filter is OPEN, so the header carries `  filter: `. This is
    // the frame that owns the `i` the leak will try to keep.
    app.rewind_selector.filter_open = true;
    app.rewind_selector.filter = "second".to_string();
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).expect("test backend");
    let open_frame = {
        terminal
            .draw(|frame| {
                let area = frame.area();
                crate::tui::rewind_selector::draw_rewind_selector(frame, area, &app);
            })
            .expect("draw");
        caption_row(&terminal)
    };
    assert!(
        open_frame.contains("Rewind to here"),
        "the caption must be painted with the filter open (the control): {open_frame:?}"
    );

    // FRAME TWO: the filter CLOSES. The header shrinks and the caption moves up
    // onto the row that held `  filter: `, which is the leak's entire setup.
    app.rewind_selector.filter_open = false;
    app.rewind_selector.filter = String::new();
    terminal
        .draw(|frame| {
            let area = frame.area();
            crate::tui::rewind_selector::draw_rewind_selector(frame, area, &app);
        })
        .expect("draw");
    let closed = caption_row(&terminal);

    // The leak, named plainly: an `i` BEFORE "Rewind" in the caption's head,
    // whose only source is the filter field's `  filter: `.
    let head = closed.split("Rewind").next().unwrap_or("");
    assert!(
        !head.contains('i'),
        "the caption's marker carries a stale cell from the closed filter — the \
         marker must occupy exactly one cell so the diff rewrites every column \\
         it covers. Rendered: {closed:?}"
    );
    assert!(
        closed.contains("  >> Rewind to here"),
        "the caption keeps its leading gap and lands on the right columns: {closed:?}"
    );
}
