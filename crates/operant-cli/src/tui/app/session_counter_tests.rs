// app/session_counter_tests.rs — the per-session counters and the
// session-boundary gate.
//
// Split out of `tests.rs` so the counter contract has its own home: these
// four cases pin the numbers the TUI shows (the context occupancy, the
// session totals) and the moment a session boundary may be applied.

use super::*;
use crate::tui::operant_app::tui_state::TuiState as _;

/// The context overlay must show the PROMPT the provider just billed — the
/// live occupancy of the window — not the sum of every request the session
/// ever made. The old accumulator made a 200k window read "full" after one
/// long turn.
///
/// MUTATION-PROVEN: restoring the accumulation fails the first assertion by
/// name (the second request's prompt is 150k, the accumulated spend 275k).
#[test]
fn the_context_overlay_shows_the_prompt_occupancy_not_the_cumulative_spend() {
    let mut app = super::tests::make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 120_000,
        output_tokens: 2_000,
        total_tokens: 122_000,
    });
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 150_000,
        output_tokens: 3_000,
        total_tokens: 153_000,
    });
    assert_eq!(
        app.context_used_tokens, 150_000,
        "the overlay must show the latest prompt occupancy, not the session's spend"
    );
}

/// The "session: Nk in, Nk out" line must describe the SESSION: cumulative
/// input and cumulative output. The output side used to be the CURRENT
/// turn's output, which `begin_turn` zeroes — so the session figure changed
/// on every turn.
#[test]
fn the_session_totals_are_cumulative_input_and_output() {
    let mut app = super::tests::make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 1_000,
        output_tokens: 100,
        total_tokens: 1_100,
    });
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 2_000,
        output_tokens: 200,
        total_tokens: 2_200,
    });
    // A new turn zeroes the PER-TURN counters; the session totals survive.
    app.begin_turn();
    assert_eq!(app.turn_output_tokens, 0);
    let (input, output) = app.total_session_tokens().expect("session totals");
    assert_eq!(
        (input, output),
        (3_000, 300),
        "the session totals must be the cumulative input and output"
    );
}

/// Every session boundary (clear, new, switch) must zero the per-session
/// counters: a counter that survives labels the NEW session with the OLD
/// session's numbers.
#[test]
fn a_session_boundary_resets_the_counters() {
    let mut app = super::tests::make_app();
    app.handle_agent_event(AgentEvent::Usage {
        input_tokens: 5_000,
        output_tokens: 500,
        total_tokens: 5_500,
    });
    assert!(app.context_used_tokens > 0);
    assert!(app.last_observed_prompt_tokens.is_some());

    app.reset_session_counters();

    assert_eq!(app.context_used_tokens, 0, "the occupancy must not survive");
    assert_eq!(app.token_count, 0);
    assert_eq!(app.turn_input_tokens, 0);
    assert_eq!(app.turn_output_tokens, 0);
    assert_eq!(app.cost_usd, 0.0);
    assert_eq!(app.cost_tracker.input_tokens, 0);
    assert_eq!(app.cost_tracker.output_tokens, 0);
    assert!(
        app.last_observed_prompt_tokens.is_none(),
        "the stale prompt measurement must not survive"
    );
}

/// `/stop` clears `is_streaming` immediately, but the cancelled task keeps
/// committing its tail. A session-boundary command that lands in that window
/// wipes the session the tail is still writing into — so the boundary waits
/// for the task.
///
/// MUTATION-PROVEN: gating on `is_streaming` alone fails the last assertion
/// (the task is still live).
#[tokio::test]
async fn a_session_boundary_waits_for_the_agent_task() {
    let mut app = super::tests::make_app();
    assert!(
        app.session_boundary_ready(),
        "an idle app is ready for a boundary command"
    );

    let (_tx, rx) =
        tokio::sync::oneshot::channel::<std::result::Result<(), operant_core::error::Error>>();
    let handle = tokio::spawn(async move {
        let _ = rx.await;
    });
    app.agent_task_handle = Some(handle);
    // A live agent: the cancel triggers the interrupt flag and the task keeps
    // running until it returns — that is the window this guard covers.
    app.agent_interrupt = Some(operant_core::interrupt::InterruptFlag::new());
    app.is_streaming = true;

    app.cancel_streaming_turn();

    assert!(!app.is_streaming, "sanity: /stop clears the streaming flag");
    assert!(
        !app.session_boundary_ready(),
        "the boundary must wait while the cancelled task is still committing its tail"
    );
}
