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

/// Ctrl+Enter QUEUES for the next turn: the draft lands in the App's
/// next-turn queue and the agent's LIVE steer queue stays EMPTY.
///
/// This is the load-bearing assertion of the split. The old arm pushed into
/// the steer queue, and the agent drains that queue at every iteration
/// boundary — so a "queued" message was injected into the RUNNING turn
/// whenever the turn had another iteration left. The operator could not
/// predict, and could not see, which happened.
///
/// MUTATION-PROVEN: pushing into the steer queue instead fails this test by
/// name.
#[test]
fn ctrl_enter_queues_for_the_next_turn_and_never_steers() {
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
    assert!(
        queue.try_lock().unwrap().is_empty(),
        "a QUEUED message must not reach the agent's steer queue — the agent \
         drains that between iterations, which would inject it into the \
         RUNNING turn"
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

/// A plain Enter mid-stream is a STEER: the draft reaches the agent's live
/// steer queue, and the next-turn queue stays empty.
///
/// MUTATION-PROVEN: leaving the key dead (the old notice-only arm) fails
/// this test by name.
#[test]
fn a_plain_enter_mid_stream_steers_the_running_turn() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.set_prompt_text("focus on the parser".to_string());
    let submitted = app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));

    assert!(
        !submitted,
        "a steer must not start a SECOND turn while one is running"
    );
    assert_eq!(
        queue.try_lock().unwrap().as_slice(),
        ["focus on the parser".to_string()],
        "the draft must reach the live steer queue"
    );
    assert!(
        app.next_turn_queue.is_empty(),
        "a steer must not be parked for the next turn either"
    );
    assert!(
        app.prompt_input.is_empty(),
        "a delivered steer consumes the composer"
    );
}

/// The band must show a pending STEER too — a steer the operator cannot see
/// is a steer they cannot trust.
#[test]
fn a_pending_steer_is_visible_in_the_queued_band() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.set_prompt_text("focus on the parser".to_string());
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        app.pending_steers(),
        ["focus on the parser".to_string()],
        "the band must render the pending steers"
    );
    assert!(
        app.queued_messages().is_empty(),
        "a steer is not a queued message"
    );
}

/// The recalled message leaves the queue, so the band must lose it too.
#[test]
fn recalling_a_queued_message_updates_the_band() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.queue_for_next_turn("first queued");
    app.queue_for_next_turn("second queued");
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

/// The agent drains the steer queue at its iteration boundaries; the band
/// must follow, or it advertises messages that have already been delivered.
#[test]
fn the_band_follows_the_agents_drain_of_the_steer_queue() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.steer_now("delivered by the agent");
    assert_eq!(app.pending_steers().len(), 1);

    // The agent's drain takes the message out of the real queue.
    queue.try_lock().unwrap().clear();
    app.refresh_steer_queue_mirror();

    assert!(
        app.pending_steers().is_empty(),
        "a drained steer queue must not keep advertising its messages"
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

/// A steered message is a prompt the operator AUTHORED too — same contract.
#[test]
fn a_steered_message_enters_the_prompt_history() {
    let (_env_lock, sandbox) = sandbox_history();
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.set_prompt_text("steered into history".to_string());
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));

    let ladder = app.prompt_input.history.clone();
    let persisted = std::fs::read_to_string(sandbox.join("history.jsonl")).unwrap_or_default();
    drop(_env_lock);
    let _ = std::fs::remove_dir_all(&sandbox);

    assert!(
        ladder.contains(&"steered into history".to_string()),
        "the steered message must be in the recall ladder: {ladder:?}"
    );
    assert!(
        persisted.contains("steered into history"),
        "the steered message must be in the on-disk history: {persisted}"
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
        app.queued_messages(),
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
        app.queued_messages().is_empty(),
        "and the queued band agrees: {:?}",
        app.queued_messages()
    );
}

/// The submit path must NOT steal a pending STEER. The old
/// `take_pending_queued_messages` drained the agent's steer queue, so a
/// steer that had not been injected yet was carried into the NEXT turn
/// instead of the running one — the two actions had become one.
#[test]
fn taking_the_next_turn_queue_never_steals_a_pending_steer() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.steer_now("guidance for the running turn");

    let pending = app.take_pending_queued_messages();

    assert!(
        pending.is_empty(),
        "a steer is not a queued message: {pending:?}"
    );
    assert_eq!(
        queue.try_lock().unwrap().as_slice(),
        ["guidance for the running turn".to_string()],
        "the steer must still be waiting for the agent to inject it"
    );
}

/// Two queued messages keep their order.
#[test]
fn queued_messages_keep_their_order() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));

    app.queue_for_next_turn("first");
    app.queue_for_next_turn("second");

    assert_eq!(
        app.take_pending_queued_messages(),
        vec!["first".to_string(), "second".to_string()],
        "FIFO: the operator's order is the agent's order"
    );
}

/// (iter-2174) A steer leaves a ROW in the transcript.
///
/// The agent injects it into the running conversation and persists it, but
/// the TUI's transcript is its own vec — so without this the operator's words
/// appeared nowhere on screen: the composer cleared, the band entry dropped
/// the moment the agent drained it, and there was no proof they had ever said
/// it. (This is the gap the red team found after iter-2172 shipped.)
#[test]
fn a_steered_message_is_a_row_in_the_transcript() {
    use crate::tui::adapter_types::types::{MessageContent, Role};

    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    app.set_prompt_text("focus on the parser".to_string());
    app.handle_key_event(press_key(KeyCode::Enter, KeyModifiers::NONE));

    let user_rows: Vec<&str> = app
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .filter_map(|m| match &m.content {
            MessageContent::Text(t) => Some(t.as_str()),
            MessageContent::Blocks(_) => None,
        })
        .collect();
    assert_eq!(
        user_rows,
        ["focus on the parser"],
        "the steered words must be a row the operator can see"
    );
}

/// (iter-2174) `/queue` with no text LISTS what is parked — the one way the
/// operator can see the queue before it is delivered.
#[test]
fn the_queue_command_lists_the_parked_messages() {
    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));

    app.queue_for_next_turn("first parked");
    app.queue_for_next_turn("second parked");

    assert!(app.handle_tui_command("queue", ""));
    let notice = app.status_notice().unwrap_or_default();
    assert!(
        notice.contains("first parked") && notice.contains("second parked"),
        "/queue must list the parked messages: {notice}"
    );
}

/// (iter-2217) `/steer` leaves a row in the transcript TOO.
///
/// The row was pushed only at the plain-Enter key site, so the documented,
/// discoverable steer — `/steer <text>` — still sent the operator's words
/// into the running turn without ever showing them on screen. The row is now
/// pushed by `App::steer_now` itself, which every steer entry point reaches.
#[test]
fn the_steer_slash_command_leaves_a_row_in_the_transcript() {
    use crate::tui::adapter_types::types::{MessageContent, Role};

    let mut app = super::tests::make_app();
    let queue = make_steer_queue();
    app.steer_queue_handle = Some(std::sync::Arc::clone(&queue));
    app.is_streaming = true;

    assert!(app.handle_tui_command("steer", "prefer the serde path"));

    assert_eq!(
        queue.try_lock().unwrap().as_slice(),
        ["prefer the serde path".to_string()],
        "the steer must reach the agent's live steer queue"
    );
    let user_rows: Vec<&str> = app
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .filter_map(|m| match &m.content {
            MessageContent::Text(t) => Some(t.as_str()),
            MessageContent::Blocks(_) => None,
        })
        .collect();
    assert_eq!(
        user_rows,
        ["prefer the serde path"],
        "the operator's steered words must be a row they can see"
    );
}
