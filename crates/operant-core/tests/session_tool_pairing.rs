//! A resumed session keeps its tool history.
//!
//! The writer has carried `tool_call_id` on a tool row since iter-1769, but
//! every tool row written BEFORE that has none — and an id-less tool row
//! cannot be paired with the assistant turn that requested it, so the
//! rehydration's orphan repair DROPS it. The resumed session then loses its
//! ENTIRE tool history, and the assistant turns that asked for those tools
//! lose their calls too: the model sees a conversation with no tools in it.
//!
//! MEASURED on the operator's live database (2026-11-06): 407 of the 437 tool
//! rows in one session carried no id, and that turn's own repair log counted
//! 818 dropped halves.
//!
//! The transcript's ORDER carries the pairing — an assistant turn's tool
//! results follow it, one per call — so the rehydration recovers the missing
//! ids from the order.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use operant_core::database::Database;
use operant_core::session::{SessionKey, SessionStore, SessionStoreConfig};

/// The assistant turn's request, as the writer stores it.
fn tool_calls_json(ids: &[&str]) -> String {
    let calls: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "type": "function",
                "function": {"name": "read_file", "arguments": "{}"}
            })
        })
        .collect();
    serde_json::to_string(&calls).expect("serialize the calls")
}

/// One assistant turn with `ids` as its calls, followed by one tool row per
/// call — the rows WITHOUT an id, exactly as the old writer stored them.
fn seed_turn(db: &Database, session_id: &str, timestamp: &str, ids: &[&str]) {
    db.save_session(session_id, None, "test", timestamp, timestamp)
        .expect("save session");
    db.save_message_full(&operant_core::database::MessageData {
        id: 0,
        session_id: session_id.to_string(),
        role: "assistant".to_string(),
        content: Some(String::new()),
        full_content: None,
        tool_call_id: None,
        tool_calls: Some(tool_calls_json(ids)),
        tool_name: None,
        timestamp: timestamp.to_string(),
        token_count: None,
        finish_reason: Some("tool_calls".to_string()),
        reasoning: None,
        reasoning_content: None,
        reasoning_details: None,
        codex_reasoning_items: None,
        codex_message_items: None,
        platform_message_id: None,
        observed: None,
        active: 1,
    })
    .expect("save the assistant turn");
    for (i, _) in ids.iter().enumerate() {
        db.save_message_full(&operant_core::database::MessageData {
            id: 0,
            session_id: session_id.to_string(),
            role: "tool".to_string(),
            content: Some(format!("result {i}")),
            full_content: None,
            tool_call_id: None,
            tool_name: Some("read_file".to_string()),
            tool_calls: None,
            timestamp: timestamp.to_string(),
            token_count: None,
            finish_reason: None,
            reasoning: None,
            reasoning_content: None,
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: None,
            active: 1,
        })
        .expect("save the tool result");
    }
}

/// Seed an assistant tool request followed by one result per call.
fn seed_group(
    db: &Database,
    session_id: &str,
    timestamp: &str,
    ids: &[&str],
    result_ids: &[Option<&str>],
) {
    db.save_session(session_id, None, "test", timestamp, timestamp)
        .expect("save session");
    db.save_message_full(&operant_core::database::MessageData {
        id: 0,
        session_id: session_id.to_string(),
        role: "assistant".to_string(),
        content: Some(String::new()),
        full_content: None,
        tool_call_id: None,
        tool_calls: Some(tool_calls_json(ids)),
        tool_name: None,
        timestamp: timestamp.to_string(),
        token_count: None,
        finish_reason: Some("tool_calls".to_string()),
        reasoning: None,
        reasoning_content: None,
        reasoning_details: None,
        codex_reasoning_items: None,
        codex_message_items: None,
        platform_message_id: None,
        observed: None,
        active: 1,
    })
    .expect("save assistant");
    for (index, id) in result_ids.iter().enumerate() {
        db.save_message_full(&operant_core::database::MessageData {
            id: 0,
            session_id: session_id.to_string(),
            role: "tool".to_string(),
            content: Some(format!("result {index}")),
            full_content: None,
            tool_call_id: id.map(str::to_string),
            tool_name: Some("read_file".to_string()),
            tool_calls: None,
            timestamp: timestamp.to_string(),
            token_count: None,
            finish_reason: None,
            reasoning: None,
            reasoning_content: None,
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: None,
            active: 1,
        })
        .expect("save tool result");
    }
}

fn store_with_tail(db: &Arc<Database>, tail: usize) -> SessionStore {
    SessionStore::with_config(
        Arc::clone(db),
        SessionStoreConfig {
            warm_capacity: 4,
            rehydrate_tail: tail,
            rehydrate_bytes: 1024 * 1024,
        },
    )
}

fn store(db: &Arc<Database>) -> SessionStore {
    SessionStore::with_config(
        Arc::clone(db),
        SessionStoreConfig {
            warm_capacity: 4,
            rehydrate_tail: 200,
            rehydrate_bytes: 1024 * 1024,
        },
    )
}

#[test]
fn a_tool_row_written_without_its_call_id_is_paired_again_on_resume() {
    let temp = tempfile::tempdir().unwrap();
    let db =
        Arc::new(Database::init(temp.path().join("pairing.sqlite")).expect("init the database"));
    let session_id = "tool-pairing";
    seed_turn(&db, session_id, "t1", &["call_a", "call_b"]);

    let store = store(&db);
    store.acquire(&SessionKey::new(session_id));
    let messages = store.peek(&SessionKey::new(session_id));

    let tool_ids: Vec<Option<String>> = messages
        .iter()
        .filter(|m| m.role == operant_core::client::Role::Tool)
        .map(|m| m.tool_call_id.clone())
        .collect();
    assert_eq!(
        tool_ids,
        vec![Some("call_a".to_string()), Some("call_b".to_string())],
        "the resumed transcript must pair each tool result with the call that \
         asked for it — an id-less result is DROPPED by the orphan repair, \
         which is how a resumed session loses its whole tool history"
    );
}

#[test]
fn a_mixed_turn_keeps_the_ids_it_has_and_pairs_the_rest_by_order() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::init(temp.path().join("mixed.sqlite")).expect("init the database"));
    let session_id = "tool-pairing-mixed";
    // The assistant turn asked for two calls; the SECOND result was written
    // by the fixed writer, so it HAS its id, and the first one has none.
    db.save_session(session_id, None, "test", "t1", "t1")
        .expect("save session");
    db.save_message_full(&operant_core::database::MessageData {
        id: 0,
        session_id: session_id.to_string(),
        role: "assistant".to_string(),
        content: Some(String::new()),
        full_content: None,
        tool_call_id: None,
        tool_calls: Some(tool_calls_json(&["call_a", "call_b"])),
        tool_name: None,
        timestamp: "t1".to_string(),
        token_count: None,
        finish_reason: Some("tool_calls".to_string()),
        reasoning: None,
        reasoning_content: None,
        reasoning_details: None,
        codex_reasoning_items: None,
        codex_message_items: None,
        platform_message_id: None,
        observed: None,
        active: 1,
    })
    .expect("save the assistant turn");
    for (i, id) in [None, Some("call_b")].into_iter().enumerate() {
        db.save_message_full(&operant_core::database::MessageData {
            id: 0,
            session_id: session_id.to_string(),
            role: "tool".to_string(),
            content: Some(format!("result {i}")),
            full_content: None,
            tool_call_id: id.map(str::to_string),
            tool_name: Some("read_file".to_string()),
            tool_calls: None,
            timestamp: "t1".to_string(),
            token_count: None,
            finish_reason: None,
            reasoning: None,
            reasoning_content: None,
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: None,
            active: 1,
        })
        .expect("save the tool result");
    }

    let store = store(&db);
    store.acquire(&SessionKey::new(session_id));
    let messages = store.peek(&SessionKey::new(session_id));

    let tool_ids: Vec<Option<String>> = messages
        .iter()
        .filter(|m| m.role == operant_core::client::Role::Tool)
        .map(|m| m.tool_call_id.clone())
        .collect();
    assert_eq!(
        tool_ids,
        vec![Some("call_a".to_string()), Some("call_b".to_string())],
        "the row that already had its id keeps it, and the id-less row takes \
         the call nobody claimed"
    );
}

/// A count-bound cut that begins in the middle of a tool group must not
/// rehydrate an unmatched result. The resumed request either carries the
/// whole assistant/request + results group or none of it.
#[test]
fn a_truncated_rehydration_keeps_complete_tool_groups_only() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::init(temp.path().join("group-boundary.sqlite")).unwrap());
    let session_id = "group-boundary";
    seed_group(&db, session_id, "t1", &["call_a", "call_b"], &[None, None]);
    seed_group(&db, session_id, "t2", &["call_c", "call_d"], &[None, None]);

    let store = store_with_tail(&db, 2);
    store.acquire(&SessionKey::new(session_id));
    let messages = store.peek(&SessionKey::new(session_id));

    let requested: std::collections::HashSet<_> = messages
        .iter()
        .filter(|message| message.role == operant_core::client::Role::Assistant)
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .map(|call| call.id.clone())
        .collect();
    for message in messages
        .iter()
        .filter(|message| message.role == operant_core::client::Role::Tool)
    {
        assert!(
            message
                .tool_call_id
                .as_ref()
                .is_some_and(|id| requested.contains(id)),
            "a result without its request must not be rehydrated: {message:#?}"
        );
    }
    for message in messages
        .iter()
        .filter(|message| message.role == operant_core::client::Role::Assistant)
    {
        let Some(calls) = message.tool_calls.as_ref() else {
            continue;
        };
        for call in calls {
            assert!(
                messages.iter().any(|candidate| {
                    candidate.role == operant_core::client::Role::Tool
                        && candidate.tool_call_id.as_deref() == Some(call.id.as_str())
                }),
                "an assistant request without all of its results must not be rehydrated"
            );
        }
    }
}

/// A complete group at the tail stays intact even when the count bound also
/// includes an older partial group.
#[test]
fn a_truncated_rehydration_keeps_a_complete_tail_group() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::init(temp.path().join("complete-tail.sqlite")).unwrap());
    let session_id = "complete-tail";
    seed_group(&db, session_id, "t1", &["call_a", "call_b"], &[None, None]);
    seed_group(&db, session_id, "t2", &["call_c", "call_d"], &[None, None]);

    let store = store_with_tail(&db, 4);
    store.acquire(&SessionKey::new(session_id));
    let messages = store.peek(&SessionKey::new(session_id));

    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == operant_core::client::Role::Tool)
            .count(),
        2,
        "the complete tail group must survive: {messages:#?}"
    );
}
