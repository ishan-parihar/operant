// app/tests.rs — Unit tests for the TUI app (turn state, key handling,
// command routing).
//
// Extracted from the app/mod.rs monolith.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton};

fn make_app() -> App {
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

#[test]
fn test_mcp_subcommand_is_not_intercepted() {
    let mut app = make_app();
    assert!(!app.intercept_slash_command_with_args("mcp", "auth mcphub"));
    assert!(!app.mcp_view.visible);
}

#[test]
fn test_clear_slash_command_clears_messages() {
    let mut app = make_app();
    app.add_message(Role::User, "hello".to_string());
    app.add_message(Role::Assistant, "world".to_string());
    assert_eq!(app.messages.len(), 2);
    assert!(app.intercept_slash_command("clear"));
    assert_eq!(app.messages.len(), 0);
}

#[test]
fn test_exit_slash_command_sets_quit_flag() {
    let mut app = make_app();
    assert!(!app.should_exit);
    assert!(app.intercept_slash_command("exit"));
    assert!(app.should_exit);
}

#[test]
fn test_vim_slash_command_toggles_vim() {
    let mut app = make_app();
    assert!(!app.prompt_input.vim_enabled);
    assert!(app.intercept_slash_command("vim"));
    assert!(app.prompt_input.vim_enabled);
    assert!(app.intercept_slash_command("vim"));
    assert!(!app.prompt_input.vim_enabled);
}

#[test]
fn test_model_slash_command_opens_picker() {
    let mut app = make_app();
    app.has_credentials = true;
    assert!(!app.model_picker.visible);
    assert!(app.intercept_slash_command("model"));
    assert!(app.model_picker.visible);
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
    assert!(app.intercept_slash_command("tasks"));
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
fn test_fast_slash_command_toggles_fast_mode() {
    let mut app = make_app();
    assert!(!app.fast_mode);
    assert!(app.intercept_slash_command("fast"));
    assert!(app.fast_mode);
    assert!(app.intercept_slash_command("fast"));
    assert!(!app.fast_mode);
}

#[test]
fn test_output_style_cycles() {
    let mut app = make_app();
    assert_eq!(app.output_style, "auto");
    assert!(app.intercept_slash_command("output-style"));
    assert_eq!(app.output_style, "stream");
    assert!(app.intercept_slash_command("output-style"));
    assert_eq!(app.output_style, "verbose");
    assert!(app.intercept_slash_command("output-style"));
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

    assert_eq!(app.prompt_input.text, "/fork 2");
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
    assert!(app.intercept_slash_command("help"));
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
    assert!(app.intercept_slash_command("help"));
    assert!(app.help_overlay.visible);
    assert!(app.show_help);
    // Second call closes it (toggle, not idempotent-open).
    assert!(app.intercept_slash_command("help"));
    assert!(!app.help_overlay.visible);
    assert!(!app.show_help);
    // Third call opens it again.
    assert!(app.intercept_slash_command("help"));
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
    let mut app = make_app();
    // Seeded with a non-empty suggestion so the "previous completion" reading
    // is actually testable: if Tab-family keys cycled completions, this would
    // change. It must NOT, and the permission mode must move.
    app.prompt_input.suggestions = vec![crate::tui::prompt_input::TypeaheadSuggestion {
        text: "/help".to_string(),
        description: "Show help".to_string(),
        source: crate::tui::prompt_input::TypeaheadSource::SlashCommand,
    }];
    let before = app.settings.permission_mode.clone();
    let before_idx = app.prompt_input.suggestion_index;

    app.handle_key_event(press_key(KeyCode::BackTab, KeyModifiers::SHIFT));

    assert_ne!(
        app.settings.permission_mode, before,
        "Shift+Tab is catalogued as \"Cycle permission mode\" (Custom(5)); it must \
         not be left doing nothing"
    );
    // The security-relevant part: BypassPermissions is one of the states this
    // cycles through, so a user who believed this key only moved through
    // completions would not expect it to change their permission posture.
    assert_eq!(
        app.prompt_input.suggestion_index, before_idx,
        "Shift+Tab must not touch completion state"
    );
}

#[test]
fn test_alt_v_does_not_toggle_vim_mode() {
    let mut app = make_app();
    // No voice recorder is configured on a bare App, which is exactly the
    // condition under which the dispatcher's guard fails. The point of this
    // test is the negative: Alt+V is catalogued as voice (Custom(6)), NOT as
    // ToggleVimMode, and must not silently become a vim toggle.
    assert!(!app.prompt_input.vim_enabled);

    app.handle_key_event(press_key(KeyCode::Char('v'), KeyModifiers::ALT));

    assert!(
        !app.prompt_input.vim_enabled,
        "Alt+V must not enter vim mode; vim is reached via the /vim slash command"
    );
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
    let alt_v = registry
        .get_bindings(BindingContext::Global)
        .into_iter()
        .find(|b| b.key == KeyCode::Char('v') && b.modifiers == KeyModifiers::ALT)
        .expect("Alt+V should still be catalogued — it starts voice recording");
    assert!(
        !matches!(alt_v.action, KeyAction::ToggleVimMode),
        "Alt+V is catalogued as ToggleVimMode again, but the dispatcher runs \
         voice hold-to-talk. Vim mode is reached via the /vim slash command."
    );

    let shift_tab = registry
        .get_bindings(BindingContext::Prompt)
        .into_iter()
        .find(|b| b.key == KeyCode::Tab && b.modifiers.contains(KeyModifiers::SHIFT))
        .expect("Shift+Tab should still be catalogued — it cycles permissions");
    assert!(
        !matches!(shift_tab.action, KeyAction::CompletionPrev),
        "Shift+Tab is catalogued as CompletionPrev again, but the dispatcher \
         cycles the PERMISSION mode. That is security-relevant: a user who \
         thought the key only moved through completions would not expect it to \
         reach BypassPermissions."
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
    let mut app = make_app();
    app.has_credentials = true;
    app.active_provider = Some("anthropic".to_string());

    app.handle_key_event(press_key(KeyCode::Char('a'), KeyModifiers::CONTROL));

    assert!(app.model_picker.visible);
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

// ---- Bash prefix allowlist ----------------------------------------------

#[test]
fn test_bash_command_not_allowed_by_default() {
    let app = make_app();
    assert!(!app.bash_command_allowed_by_prefix("git status"));
    assert!(!app.bash_command_allowed_by_prefix("ls -la"));
    assert!(!app.bash_command_allowed_by_prefix(""));
}

#[test]
fn test_bash_prefix_allowlist_after_p_key() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    // Set up a bash permission dialog with a suggested prefix.
    let pr = PermissionRequest::bash(
        "tu-1".to_string(),
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

    // Dialog should be dismissed and "git" added to the allowlist.
    assert!(app.permission_request.is_none());
    assert!(app.bash_command_allowed_by_prefix("git status"));
    assert!(app.bash_command_allowed_by_prefix("git push origin main"));
    // Other commands should NOT be allowed.
    assert!(!app.bash_command_allowed_by_prefix("rm -rf /tmp"));
}

#[test]
fn test_bash_prefix_allowlist_via_enter_on_p_option() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let mut pr = PermissionRequest::bash(
        "tu-2".to_string(),
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
    assert!(app.bash_command_allowed_by_prefix("cargo test"));
    assert!(!app.bash_command_allowed_by_prefix("make build"));
}

#[test]
fn test_bash_prefix_allowlist_non_prefix_option_does_not_add() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::bash(
        "tu-3".to_string(),
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
    assert!(!app.bash_command_allowed_by_prefix("npm test"));
}

// ---- iter-20: permission dialog response routing ----------------------

#[test]
fn test_permission_dialog_y_sends_allow_once() {
    use crate::tui::dialogs::PermissionRequest;

    let mut app = make_app();
    let pr = PermissionRequest::standard(
        "tu-1".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    let pr = PermissionRequest::standard(
        "tu-2".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    let pr = PermissionRequest::standard(
        "tu-p".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    let pr = PermissionRequest::standard(
        "tu-3".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    let pr = PermissionRequest::standard(
        "tu-4".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    let mut pr = PermissionRequest::standard(
        "tu-5".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    let pr = PermissionRequest::standard(
        "tu-6".to_string(),
        "Bash".to_string(),
        "This will execute a shell command.".to_string(),
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
    app.handle_agent_event(AgentEvent::Done { message: done_msg });
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
    app.handle_agent_event(AgentEvent::Done { message: done_msg });
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
fn turn_state_should_reflect_retry_event() {
    let mut app = make_app();
    assert_eq!(app.turn_state, TurnState::Idle);

    // A retry scheduled outside a live turn must not invent one: the
    // compat shim keeps its pre-existing meaning (untouched by this event).
    app.handle_agent_event(AgentEvent::RetryScheduled {
        attempt: 1,
        max_attempts: 3,
        reason: "stream dropped".to_string(),
    });
    assert_eq!(app.turn_state, TurnState::WaitingForNetwork);
    assert!(
        !app.is_streaming,
        "RetryScheduled must not flip is_streaming — it fires mid-turn, where the flag is already true"
    );
    assert_eq!(
        app.display_turn_state(),
        TurnState::Idle,
        "a state set with no live turn collapses to Idle for display"
    );

    // The real case: a retry inside a live turn keeps the turn streaming and
    // only adds the specificity the status line needs.
    app.handle_agent_event(AgentEvent::Thinking {
        content: "re-planning".to_string(),
    });
    app.handle_agent_event(AgentEvent::RetryScheduled {
        attempt: 2,
        max_attempts: 3,
        reason: "context overflow".to_string(),
    });
    assert!(app.is_streaming, "the turn is still live");
    assert_eq!(app.turn_state, TurnState::WaitingForNetwork);
    assert_eq!(app.display_turn_state(), TurnState::WaitingForNetwork);
    assert_eq!(TurnState::WaitingForNetwork.label(), "Retrying");
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
        app.notifications
            .notifications
            .iter()
            .any(|n| n.message.contains("80% full")),
        "expected an 80%-full context warning notification to be pushed"
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
        app.notifications
            .notifications
            .iter()
            .any(|n| n.message.contains("dropped")),
        "expected a warning that the image wasn't sent"
    );
}

#[test]
fn test_drop_pending_images_with_notice_noop_when_empty() {
    let mut app = make_app();
    let before = app.notifications.notifications.len();
    app.drop_pending_images_with_notice();
    assert_eq!(app.notifications.notifications.len(), before);
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

#[test]
fn test_stubs_eliminated_no_file_history_field() {
    // Phase 3b: App.file_history + current_turn fields must be gone.
    // Verified by compilation — if they existed, referencing them would
    // be needed. Their absence is the test.
    let app = make_app();
    assert!(
        app.diff_viewer.turn_files.is_empty(),
        "no turn-files without stub"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_feedback_survey_removed() {
    // Phase 4: /survey command must not be intercepted (feedback_survey deleted).
    // intercept_slash_command returns true if the command is known+intercepted.
    // /survey was removed from the command table, so it returns false.
    let mut app = make_app();
    let result = app.intercept_slash_command("survey");
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
    app.debug_hub.record_frame(5.0);
    app.debug_hub.record_frame(3.0);
    assert_eq!(app.debug_hub.frame_count(), 2);
    assert_eq!(app.debug_hub.last_render_ms(), 3);
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

fn drive_keys<B: ratatui::backend::Backend>(app: &mut App, terminal: &mut ratatui::Terminal<B>)
where
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
        ("model", "model_picker"),
        ("effort", "effort_picker"),
        ("context", "context_viz"),
        ("agents", "agents_menu"),
        ("export", "export_dialog"),
        ("mcp", "mcp_view"),
    ];

    for (cmd, overlay) in scenarios {
        let mut app = make_app();
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
    assert!(app.intercept_slash_command_with_args("skill", "nope-missing"));
    assert!(app.pending_user_message.is_none());
    assert!(app.status_message.is_some());

    // Existing skill: handled, expansion staged for the run loop.
    assert!(app.intercept_slash_command_with_args("skill", "demo-skill"));
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
    assert!(app.intercept_slash_command("skill"));
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
    assert!(app.intercept_slash_command("bundle"));
    assert!(app.pending_user_message.is_none());
}

// ---- Queued vs Running tool status ----

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
        },
    });
    assert_eq!(app.tool_use_blocks[0].status, ToolStatus::Done);
    assert!(!app.tool_use_blocks[0].status.is_pending());
}

// ---- Batched tool-call grouping ----
//
// A concurrent batch (6 same-name calls in one turn) must render as one block
// with a progress meter, not as N undifferentiated siblings. The two cases
// that matter for correctness are the merge itself and the boundary it must
// never cross.
use crate::tui::render::tools::{render_tool_group_lines, tool_group_ranges};
use crate::tui::transcript_turn::build_transcript_turns;

/// Drive one call through the real event path: the first `ToolStart` means
/// "parsed" (Queued), the pool's re-announcement means the permit is held
/// (Running). Emitting the same `AgentEvent` twice is how a real call reaches
/// Running, so the tests use it rather than hand-building blocks.
fn start_tool(app: &mut App, id: &str, name: &str, arguments: &str) {
    let event = AgentEvent::ToolStart {
        tool_call_id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    };
    app.handle_agent_event(event.clone());
    app.handle_agent_event(event);
}

#[test]
fn tool_group_should_merge_adjacent_same_name_blocks() {
    let mut app = make_app();
    app.messages
        .push(Message::user("read the tree".to_string()));
    start_tool(&mut app, "call_1", "read", r#"{"file_path":"src/a.rs"}"#);
    start_tool(&mut app, "call_2", "read", r#"{"file_path":"src/b.rs"}"#);
    // The double ToolStart is a state transition, not two tools: still one
    // block per call id, so there is nothing to double-count.
    assert_eq!(app.tool_use_blocks.len(), 2, "one block per call id");

    let turns = build_transcript_turns(&app);
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    let groups = tool_group_ranges(&turn.tool_blocks, turn.assistant_messages.len());
    assert_eq!(groups.len(), 1, "adjacent same-name calls form one group");
    assert_eq!(groups[0], 0..2, "the group covers both calls");

    // The group renders as one block: a progress-meter header plus one status
    // row per sub-call, not two sibling blocks.
    let mut lines = Vec::new();
    render_tool_group_lines(&mut lines, &turn.tool_blocks[0..2], 0);
    assert_eq!(lines.len(), 3, "one header + one row per sub-call");
    let header: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(header.contains("read"), "header names the tool: {header}");
    assert!(
        header.contains("0/2 done"),
        "header carries the progress meter: {header}"
    );
    assert!(
        header.contains("running:"),
        "header names what is still running: {header}"
    );
    // Per-sub-call rows identify each call, and both are running.
    for (row, path) in lines[1..].iter().zip(["src/a.rs", "src/b.rs"]) {
        let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(path), "row names its call: {text}");
    }
}

#[test]
fn tool_group_should_not_merge_across_user_message_boundary() {
    let mut app = make_app();
    app.messages.push(Message::user("read a".to_string()));
    start_tool(&mut app, "call_1", "read", r#"{"file_path":"src/a.rs"}"#);
    app.messages.push(Message::user("read b".to_string()));
    start_tool(&mut app, "call_2", "read", r#"{"file_path":"src/b.rs"}"#);
    assert_eq!(app.tool_use_blocks.len(), 2);

    let turns = build_transcript_turns(&app);
    assert_eq!(turns.len(), 2, "one turn per user message");
    assert_eq!(turns[0].tool_blocks.len(), 1);
    assert_eq!(turns[1].tool_blocks.len(), 1);

    // Grouping is per turn, so two same-name calls on either side of a user
    // message stay two groups of one. A transcript-global grouping that ignored
    // the turn would collapse them into a single group of two and fail here.
    let batch_sizes: Vec<usize> = turns
        .iter()
        .flat_map(|turn| {
            tool_group_ranges(&turn.tool_blocks, turn.assistant_messages.len())
                .into_iter()
                .map(|group| group.len())
        })
        .collect();
    assert_eq!(
        batch_sizes,
        vec![1, 1],
        "same-name calls in different turns must not batch together"
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

    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(app.scroll_offset, 42, "first press only arms");
    assert!(
        app.status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Bookmark on")
    );

    // Reader wanders off, then asks to come back.
    app.scroll_offset = 5;
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
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
    app.handle_key_event(press_key(KeyCode::Char('g'), KeyModifiers::CONTROL));
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
    // is on screen.
    app.handle_key_event(press_key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(!app.auto_scroll);
    app.reconcile_scroll_anchor();
    assert_eq!(
        app.scroll_offset, 10,
        "no reflow yet, so nothing to correct"
    );

    // 20 rows of new output land above the viewport. The next paint would
    // therefore show the pinned row 20 lines lower than the reader left it.
    app.last_render_scroll_offset.set(20);
    app.reconcile_scroll_anchor();

    // Offset grew by the same 20, and the transcript is now 20 + 10 = 30 rows
    // of scrollback, so the pinned row paints at 30 - 30 = 0 again: the same
    // content, in the same place.
    assert_eq!(app.scroll_offset, 30);
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

// ---- keybinding registry: /keys and /hotkeys ------------------------

/// Rendered system text of the most recent system annotation.
fn last_system_text(app: &App) -> String {
    app.system_annotations
        .last()
        .map(|a| a.text.clone())
        .unwrap_or_default()
}

#[test]
fn keys_command_should_report_registry_bindings() {
    use crate::tui::keybindings::KeyBindingRegistry;

    let mut app = make_app();
    let before = app.system_annotations.len();

    assert!(app.intercept_slash_command("keys"));
    assert_eq!(
        app.system_annotations.len(),
        before + 1,
        "/keys reports through the transcript, not the status line"
    );

    let text = last_system_text(&app);
    // Sourced from the registry, not a hand-maintained list: the header count
    // must equal the number of rows the registry actually catalogues.
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

    assert!(app.intercept_slash_command("hotkeys"));
    let text = last_system_text(&app);
    assert!(
        text.contains("chord(s) used"),
        "/hotkeys leads with the used/never split, got: {text}"
    );
    assert!(
        text.contains("You have used:") && text.contains("Never used (undiscovered):"),
        "both halves are rendered, got: {text}"
    );
    assert!(
        text.contains("Up"),
        "the chord just pressed shows up in the used half, got: {text}"
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
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert!(app.copy_mode_active(), "Ctrl+T did not enter copy mode");

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
fn copy_mode_should_toggle_with_ctrl_t() {
    let mut app = make_app();
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert!(app.copy_mode_active());
    app.handle_key_event(press_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert!(
        !app.copy_mode_active(),
        "Ctrl+T did not toggle copy mode off"
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

fn copy_mode_is_on(app: &mut App) -> bool {
    app.copy_mode_active()
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
            fired: copy_mode_is_on,
        },
    ]
}

#[test]
fn global_bindings_should_fire_their_action_behaviourally() {
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
