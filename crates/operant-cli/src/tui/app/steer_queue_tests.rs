// app/steer_queue_tests.rs — the steer queue the operator can SEE, and the
// prompt-history entries for the messages they SENT.
//
// Split out of `tests.rs` so this contract has its own home: the queue was a
// live, working mechanism (the agent drained it every iteration) and a DEAD
// surface for the operator (the TUI's `queued_messages` was a `&[]` stub), so
// a queued message vanished from the screen the moment it left the composer.
//
// The four symptoms these pin are one report:
///
///   * "shortcuts like ctrl+enter must queue the message, but it is not working"
///   * "sending queue and goal messages are not showing up in the prompt-history"
use super::*;
use crate::tui::operant_app::tui_state::TuiState as _;
use crossterm::event::{KeyEventKind, KeyEventState};

fn make_steer_queue() -> std::sync::Arc<tokio::sync::Mutex<Vec<String>>> {
    std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()))
}

fn press_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// A writable scratch directory for the history sandbox.
///
/// `std::env::temp_dir()` follows `TMPDIR`, which on some hosts points at a
/// read-only mount. The history writer ignores write errors, so a read-only
/// scratch directory would surface as an EMPTY file and a failing assertion
/// that names the app, not the filesystem. Probe the candidates instead.
fn writable_temp_dir() -> std::path::PathBuf {
    for dir in [std::env::temp_dir(), std::path::PathBuf::from("/tmp")] {
        let probe = dir.join(format!("operant-scratch-probe-{}", std::process::id()));
        if std::fs::create_dir_all(&probe).is_ok() {
            let _ = std::fs::remove_dir_all(&probe);
            return dir;
        }
    }
    std::env::temp_dir()
}

/// Point the operant home at a throwaway directory so the history write never
/// touches the operator's real `history.jsonl`. Returns the sandbox path; the
/// caller removes it.
fn sandbox_history() -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    let env_lock = crate::config::test_env_lock();
    let sandbox = writable_temp_dir().join(format!(
        "operant-queue-history-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(&sandbox);
    // SAFETY: serialized by the crate-wide test_env_lock.
    unsafe {
        std::env::set_var("HERMES_HOME", &sandbox);
    }
    (env_lock, sandbox)
}

/// The queued band must show what the agent has queued. Before this the
/// `TuiState::queued_messages` seam returned `&[]`, so the band NEVER
/// rendered a queued message: the operator's Ctrl+Enter produced a transient
/// notice and then silence.
///
/// MUTATION-PROVEN: restoring the `&[]` stub fails the first assertion by name.
#[test]
fn a_queued_message_is_visible_in_the_queued_band() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.set_prompt_text("check the parser".to_string());
    let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::CONTROL));

    assert!(
        !submitted,
        "Ctrl+Enter while streaming must NOT submit the prompt"
    );
    assert_eq!(
        queue.try_lock().unwrap().as_slice(),
        ["check the parser".to_string()],
        "the message must really be in the queue the agent drains"
    );
    assert_eq!(
        app.queued_messages(),
        ["check the parser".to_string()],
        "the queued band must render the real queue — an invisible queue looks dead"
    );
    assert!(
        app.prompt_input.is_empty(),
        "the composer is emptied only after the message is safely queued"
    );
}

/// The recalled message leaves the queue, so the band must lose it too.
#[test]
fn recalling_a_queued_message_updates_the_band() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.queue_steer("first queued");
    app.queue_steer("second queued");
    assert_eq!(app.queued_messages().len(), 2);

    app.recall_queued_message();

    assert_eq!(
        app.prompt_input.text, "first queued",
        "the head of the queue comes back for editing"
    );
    assert_eq!(
        app.queued_messages(),
        ["second queued".to_string()],
        "the recalled message must leave the band with the queue"
    );
}

/// The agent drains the queue at its iteration boundaries; the band must
/// follow, or it advertises messages that have already been delivered.
#[test]
fn the_band_follows_the_agents_drain_of_the_queue() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));

    app.queue_steer("delivered by the agent");
    assert_eq!(app.queued_messages().len(), 1);

    // The agent's drain takes the message out of the real queue.
    queue.try_lock().unwrap().clear();
    app.refresh_steer_queue_mirror();

    assert!(
        app.queued_messages().is_empty(),
        "a drained queue must not keep advertising its messages"
    );
}

/// A queued message is a prompt the operator AUTHORED: it must be reachable
/// from the recall ladder (Up/Down) and from the on-disk history (Ctrl+R).
#[test]
fn a_queued_message_enters_the_prompt_history() {
    let (_env_lock, sandbox) = sandbox_history();
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.set_prompt_text("queued into history".to_string());
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::CONTROL));

    let ladder = app.prompt_input.history.clone();
    let persisted = std::fs::read_to_string(sandbox.join("history.jsonl")).unwrap_or_default();
    drop(_env_lock);
    let _ = std::fs::remove_dir_all(&sandbox);

    assert!(
        ladder.contains(&"queued into history".to_string()),
        "the queued message must be in the recall ladder: {ladder:?}"
    );
    assert!(
        persisted.contains("queued into history"),
        "the queued message must be in the on-disk history: {persisted}"
    );
}

/// The goal message is a prompt the operator authored too — same contract.
#[test]
fn the_goal_message_enters_the_prompt_history() {
    let (_env_lock, sandbox) = sandbox_history();
    let mut app = super::tests::make_app();

    assert!(app.handle_tui_command("goal", "ship the release"));

    let ladder = app.prompt_input.history.clone();
    let persisted = std::fs::read_to_string(sandbox.join("history.jsonl")).unwrap_or_default();
    drop(_env_lock);
    let _ = std::fs::remove_dir_all(&sandbox);

    assert_eq!(app.session_goal.as_deref(), Some("ship the release"));
    assert!(
        ladder.contains(&"ship the release".to_string()),
        "the goal must be in the recall ladder: {ladder:?}"
    );
    assert!(
        persisted.contains("ship the release"),
        "the goal must be in the on-disk history: {persisted}"
    );
}

/// A message the operator QUEUED while the TUI was IDLE must be delivered
/// WITH the next turn, ahead of the draft they just submitted.
///
/// The agent's steer drain runs BETWEEN iterations, so a message queued
/// before a turn had no iteration boundary to wait for: it sat until the
/// turn after the next one — "CTRL+ENTER still does not queue correctly".
///
/// MUTATION-PROVEN: leaving the pending messages in the queue (the old
/// behaviour — the submit path never looked at them) fails this test by name.
#[test]
fn an_idle_queued_message_is_taken_for_the_next_turn() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = false;
    app.prompt_input.replace_text("do this first".to_string());

    // Ctrl+Enter: the operator asks for it to be QUEUED, not sent.
    let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::CONTROL));
    assert!(!submitted, "Ctrl+Enter must not submit — it queues");
    assert_eq!(
        queue.try_lock().unwrap().as_slice(),
        ["do this first".to_string()],
        "the draft is queued"
    );

    // The next turn takes it, in the order it was queued.
    let pending = app.take_pending_queued_messages();
    assert_eq!(
        pending,
        vec!["do this first".to_string()],
        "the pending message is handed to the next turn"
    );
    assert!(
        queue.try_lock().unwrap().is_empty(),
        "the queue is empty once the turn has taken it"
    );
    assert!(
        app.queued_messages().is_empty(),
        "and the queued band agrees: {:?}",
        app.queued_messages()
    );
}

/// Two queued messages keep their order.
#[test]
fn queued_messages_keep_their_order() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));

    app.queue_steer("first");
    app.queue_steer("second");

    assert_eq!(
        app.take_pending_queued_messages(),
        vec!["first".to_string(), "second".to_string()],
        "FIFO: the operator's order is the agent's order"
    );
}
