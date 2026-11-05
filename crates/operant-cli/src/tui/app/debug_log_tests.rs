//! The debug log sees the surfaces that own the keyboard and the failures.
//!
//! Two gaps: the inline picker consumes its keys BEFORE the debug
//! publication (so the picker's keys were the one surface the event log could
//! not see — the exact surface a "the picker ate my key" bug report needs),
//! and a live agent error was recorded only on the transcript card, never in
//! the log.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}
fn open_inline_picker(app: &mut App) {
    // The same opener the `/model` command uses — a hand-built picker state
    // would test a shape the product never builds.
    app.open_model_picker_inline();
}

/// The picker's keys must reach the debug bus: the publication sits ABOVE
/// the picker's gate now, because the picker returns early and a publication
/// below it never happened.
#[test]
fn the_inline_picker_keys_reach_the_debug_bus() {
    let mut app = super::tests::make_app();
    app.debug_hub.event_bus().set_enabled(true);
    open_inline_picker(&mut app);
    assert!(
        app.inline_interactive.is_some(),
        "sanity: the picker is open and owns the keyboard"
    );

    app.handle_key_event(press_key(KeyCode::Char('a'), KeyModifiers::NONE));

    let saw_key = app
        .debug_hub
        .event_bus()
        .recent(16)
        .iter()
        .any(|event| matches!(event, crate::tui::debug::TuiEvent::Key { .. }));
    assert!(
        saw_key,
        "the picker's key must be in the debug log — the log of the run whose \
         keys the picker ate is the run the operator wants to debug"
    );
}

/// A live agent error belongs in the debug hub: the transcript card shows the
/// operator, the log is what a BUG REPORT can attach.
#[test]
fn a_live_agent_error_is_recorded_in_the_debug_hub() {
    let mut app = super::tests::make_app();
    app.debug_hub.event_bus().set_enabled(true);

    app.handle_agent_event(AgentEvent::Error {
        error: "the provider died mid-stream".to_string(),
    });

    let last = app
        .debug_hub
        .last_error()
        .expect("the failure must be recorded in the hub");
    assert!(
        last.contains("the provider died mid-stream"),
        "the hub must carry the live failure, got: {last}"
    );
    let saw_error = app
        .debug_hub
        .event_bus()
        .recent(16)
        .iter()
        .any(|event| matches!(event, crate::tui::debug::TuiEvent::Error { .. }));
    assert!(
        saw_error,
        "the failure must be an event on the bus, not only a stored string"
    );
}
