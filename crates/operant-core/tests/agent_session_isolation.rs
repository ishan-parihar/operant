//! Integration tests for per-session conversation isolation.
//!
//! The defect these cover is real and was live: `OperantAgent` held one global
//! `conversation: Arc<RwLock<Vec<Message>>>`. Two sessions served by one agent
//! shared it. The gateway "solved" this by calling `clear_history()` and
//! reloading the last 20 rows on **every** session switch
//! (`gateway_runner.rs`), which means two concurrent gateway chats wipe each
//! other's turns — each message destroys the other's context.
//!
//! The substrate makes the hot conversation follow `session_id`. These tests
//! assert that through the public agent API, not through the store's internals:
//! the store's own unit tests cover cache mechanics; what could still regress
//! is the agent wiring, which is exactly where the original bug lived.
//!
//! Every test drives the real `OperantAgent` over a real `Database`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use operant_core::agent::{AgentConfig, OperantAgent};
use operant_core::client::{ClientConfig, Message, OpenAIClient};
use operant_core::database::Database;
use operant_core::tools::ToolRegistry;

/// An agent over its own temporary database.
struct Fixture {
    _dir: tempfile::TempDir,
    db: Arc<Database>,
    agent: Arc<OperantAgent>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    let agent = Arc::new(OperantAgent::new(
        AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(OpenAIClient::new(
                ClientConfig::default(),
            )),
        ),
        ToolRegistry::new(Duration::from_secs(1)),
        Arc::clone(&db),
    ));
    Fixture {
        _dir: dir,
        db,
        agent,
    }
}

/// Persist a user turn the way the agent's turn prologue does, so the
/// rehydrate path has rows to find.
fn seed(db: &Database, session: &str, contents: &[&str]) {
    db.save_session(session, None, "test", "t0", "t0")
        .expect("save session");
    for (i, c) in contents.iter().enumerate() {
        db.save_message(session, "user", c, &format!("t{i:03}"))
            .expect("save message");
    }
}

#[tokio::test]
async fn retarget_carries_no_turns_into_the_next_session() {
    let f = fixture();

    f.agent.set_session_id("emp-a");
    f.agent.add_message(Message::user("a's secret turn")).await;
    assert_eq!(
        f.agent.conversation().await.len(),
        1,
        "a's turn is in a's slot"
    );

    f.agent.set_session_id("emp-b");

    assert!(
        f.agent.conversation().await.is_empty(),
        "b must not inherit a's turns, got {:?}",
        f.agent.conversation().await
    );
}

#[tokio::test]
async fn returning_to_a_session_restores_its_own_turns() {
    let f = fixture();

    f.agent.set_session_id("emp-a");
    f.agent.add_message(Message::user("a1")).await;
    f.agent.add_message(Message::user("a2")).await;

    f.agent.set_session_id("emp-b");
    f.agent.add_message(Message::user("b1")).await;
    assert_eq!(
        f.agent.conversation().await.len(),
        1,
        "b holds only its own turn"
    );

    f.agent.set_session_id("emp-a");

    let contents: Vec<String> = f
        .agent
        .conversation()
        .await
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        contents,
        vec!["a1".to_string(), "a2".to_string()],
        "returning to a must see a's turns, not b's"
    );
}

#[tokio::test]
async fn a_retargeted_session_survives_a_cold_eviction() {
    let f = fixture();
    let store = f.agent.session_store().clone();

    f.agent.set_session_id("emp-a");
    f.agent.add_message(Message::user("a1")).await;
    f.agent.add_message(Message::user("a2")).await;

    // Force a's turns out of the store entirely.
    assert!(
        store.evict(&operant_core::session::SessionKey::new("emp-a")),
        "a was resident and is now evicted"
    );

    f.agent.set_session_id("emp-b");
    f.agent.set_session_id("emp-a");

    let contents: Vec<String> = f
        .agent
        .conversation()
        .await
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        contents,
        vec!["a1".to_string(), "a2".to_string()],
        "an evicted session rehydrates from the store with its turns intact"
    );
}

#[tokio::test]
async fn clear_history_wipes_only_the_addressed_session() {
    let f = fixture();

    f.agent.set_session_id("emp-a");
    f.agent.add_message(Message::user("a1")).await;
    f.agent.set_session_id("emp-b");
    f.agent.add_message(Message::user("b1")).await;

    // /new on b.
    f.agent.clear_history().await;
    assert!(f.agent.conversation().await.is_empty(), "b is wiped");

    f.agent.set_session_id("emp-a");
    assert_eq!(
        f.agent.conversation().await.len(),
        1,
        "wiping b must not wipe a"
    );
}

#[tokio::test]
async fn a_retarget_to_the_same_id_is_a_no_op() {
    let f = fixture();

    f.agent.set_session_id("emp-a");
    f.agent.add_message(Message::user("keep")).await;
    f.agent.set_session_id("emp-a");

    assert_eq!(
        f.agent.conversation().await.len(),
        1,
        "re-assigning the same id must not drop the live conversation"
    );
}

#[tokio::test]
async fn a_cold_session_rehydrates_its_persisted_turns() {
    let f = fixture();
    // An agent that has never been retargeted starts on an empty slot.
    assert!(f.agent.conversation().await.is_empty());

    seed(&f.db, "emp-restarted", &["earlier1", "earlier2"]);
    f.agent.set_session_id("emp-restarted");

    let contents: Vec<String> = f
        .agent
        .conversation()
        .await
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        contents,
        vec!["earlier1".to_string(), "earlier2".to_string()],
        "a returning session must rehydrate its persisted transcript"
    );
}

#[tokio::test]
async fn two_agents_sharing_a_database_keep_separate_sessions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("shared.sqlite")).expect("Database::init"));
    let build = || {
        Arc::new(OperantAgent::new(
            AgentConfig::default(),
            Box::new(
                operant_core::agent::clients::openai::OpenAIModelClient::new(OpenAIClient::new(
                    ClientConfig::default(),
                )),
            ),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::clone(&db),
        ))
    };
    let one = build();
    let two = build();

    one.set_session_id("emp-a");
    one.add_message(Message::user("a only")).await;

    two.set_session_id("emp-b");
    assert!(
        two.conversation().await.is_empty(),
        "b must not see a's turn even sharing one database"
    );
}

#[tokio::test]
async fn session_store_starts_with_nothing_warm() {
    let f = fixture();
    let stats = f.agent.session_stats();
    assert_eq!(stats.warm, 0, "load-on-demand: nothing warm before use");
    assert_eq!(stats.cold_loads, 0);
}
