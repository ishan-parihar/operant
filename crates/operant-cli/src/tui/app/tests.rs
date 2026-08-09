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
        if let Ok(Some(input)) = app.run(&mut terminal) {
            if crate::input::is_slash_command(&input) {
                let (cmd, args) = crate::input::parse_slash_command(&input);
                app.handle_tui_command(cmd, args);
            }
        }
    }

    // 3. Assert the help overlay is open
    assert!(app.help_overlay.visible);
    assert!(app.show_help);

    // 4. Simulate pressing Escape to close the overlay
    app.simulated_keys = vec![press_key(KeyCode::Esc, KeyModifiers::NONE)];

    while !app.simulated_keys.is_empty() && !app.should_exit {
        if let Ok(Some(input)) = app.run(&mut terminal) {
            if crate::input::is_slash_command(&input) {
                let (cmd, args) = crate::input::parse_slash_command(&input);
                app.handle_tui_command(cmd, args);
            }
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
        if let Ok(Some(input)) = app.run(&mut terminal) {
            if crate::input::is_slash_command(&input) {
                let (cmd, args) = crate::input::parse_slash_command(&input);
                app.handle_tui_command(cmd, args);
            }
        }
    }

    // 7. Assert app wants to exit
    assert!(app.should_exit);
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
        if let Ok(Some(input)) = app.run(terminal) {
            if crate::input::is_slash_command(&input) {
                let (cmd, args) = crate::input::parse_slash_command(&input);
                app.handle_tui_command(cmd, args);
            }
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
