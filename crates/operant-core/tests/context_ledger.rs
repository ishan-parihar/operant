//! The context meter and the per-request ledger.
//!
//! Until this existed, the context a session used was **unobservable after the
//! fact**. The `sessions` table carried `input_tokens`, `output_tokens` and
//! `api_call_count` columns since v1, and the live path never wrote them: on
//! the operator's database every one of the 3 271 sessions reported zero
//! tokens, so `operant insights sessions` and the `session_insights` tool
//! rendered "Tokens: 0" for a window in which one session had spent hundreds
//! of thousands of context tokens. `messages.token_count` was NULL for all
//! 65 387 message rows, so no row carried a size either.
//!
//! That blindness is what makes a context policy untunable: there is nothing
//! to compare a change against. These tests pin the meter, the ledger, and
//! the per-message token count — the three facts every later policy needs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::client::{Message, Role};
use operant_core::context_management::context_composition;
use operant_core::database::{ContextRequestRecord, Database};

/// The migrations are APPEND-ONLY: v1 (`DESIRED_SCHEMA_SQL`) and v2
/// (`DEFERRED_INDEX_SQL`) are frozen, and a new entry is a new SQL block.
/// The iter-2123 meter was nearly shipped by adding its table to v1 —
/// which MEASURED as "no such table: context_ledger" on every database
/// that already existed. This test is the fixture that reproduces an
/// existing installation and pins the append-only rule.
#[test]
fn a_v2_database_gains_the_context_ledger_when_it_is_opened() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.sqlite");
    {
        // A database at version 2 of the sessions family: the shape an
        // existing install has before this change.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL); \
             CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, source TEXT, \
             started_at TEXT, ended_at TEXT); \
             CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, \
             role TEXT, content TEXT, timestamp TEXT);",
        )
        .unwrap();
        conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        let has: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'context_ledger'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has, 0, "the legacy database does not have the table");
    }

    // Opening it applies the new entry.
    let _db = Database::init(path.clone()).expect("open the legacy database");
    let conn = rusqlite::Connection::open(&path).unwrap();
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'context_ledger'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(has, 1, "the context ledger now exists");
    assert_eq!(version, 3, "the family advanced by exactly one entry");
}

fn db() -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let db = Database::init(temp.path().join("ledger.sqlite")).expect("init the database");
    (temp, db)
}

fn record(session_id: &str, seq: i64, prompt: i64, tool_tokens: i64) -> ContextRequestRecord {
    ContextRequestRecord {
        session_id: session_id.to_string(),
        request_seq: seq,
        created_at: "2026-11-12T00:00:00Z".to_string(),
        prompt_tokens: prompt,
        completion_tokens: 10,
        total_tokens: prompt + 10,
        estimated_context_tokens: prompt * 2,
        message_count: 40,
        tool_result_tokens: tool_tokens,
        user_tokens: 100,
        assistant_tokens: 200,
        tool_call_count: 7,
        budget_tokens: 120_000,
        window_tokens: 1_000_000,
        ladder_fired: seq % 2 == 0,
        note: None,
    }
}

#[test]
fn the_session_meter_banks_the_requests_it_made() {
    let (_temp, db) = db();
    let now = "2026-11-12T00:00:00Z";
    db.save_session("metered", None, "agent", now, now).unwrap();

    // Before the meter, this row reported zero for everything.
    let before = db.session_usage("metered").unwrap();
    assert_eq!(before.input_tokens, 0);
    assert_eq!(before.api_call_count, 0);

    db.add_session_usage("metered", 12_345, 678, 1).unwrap();
    db.add_session_usage("metered", 2_000, 100, 1).unwrap();

    let after = db.session_usage("metered").unwrap();
    assert_eq!(
        after.input_tokens, 14_345,
        "the meter accumulates, not replaces"
    );
    assert_eq!(after.output_tokens, 778);
    assert_eq!(after.api_call_count, 2, "two requests were made");
}

#[test]
fn the_ledger_keeps_the_shape_of_each_request() {
    let (_temp, db) = db();
    db.record_context_request(&record("s1", 1, 1_000, 400))
        .unwrap();
    db.record_context_request(&record("s1", 2, 2_000, 900))
        .unwrap();
    db.record_context_request(&record("s2", 1, 50, 10)).unwrap();

    let rows = db.context_ledger_for_session("s1", 10).unwrap();
    assert_eq!(rows.len(), 2, "the ledger is per session");
    assert_eq!(rows[0].request_seq, 1, "oldest first");
    assert_eq!(rows[1].request_seq, 2);
    assert_eq!(rows[1].tool_result_tokens, 900);
    assert!(rows[1].ladder_fired, "the second request ran the ladder");
    assert!(!rows[0].ladder_fired);

    // The ordinal is per session, so a second session starts at 1 again.
    assert_eq!(db.next_context_request_seq("s1").unwrap(), 3);
    assert_eq!(db.next_context_request_seq("s2").unwrap(), 2);
    assert_eq!(db.next_context_request_seq("fresh").unwrap(), 1);
}

#[test]
fn every_message_row_carries_its_token_count() {
    let (_temp, db) = db();
    let now = "2026-11-12T00:00:00Z";
    db.save_session("counted", None, "agent", now, now).unwrap();
    db.save_message("counted", "user", "hello there", now)
        .unwrap();

    let rows = db.get_session_messages_full("counted").unwrap();
    assert_eq!(rows.len(), 1);
    let count = rows[0]
        .token_count
        .expect("a message row carries its token count");
    assert!(count > 0, "the count is positive: {}", count);
    // The same estimator the eviction uses: 4 tokens of role overhead plus
    // the content at chars/4.
    assert_eq!(
        count as usize,
        4 + operant_core::context_management::estimate_tokens("hello there")
    );
}

#[test]
fn the_composition_separates_tool_output_from_the_conversation() {
    // The shape a long tool-using session has: a little conversation and a
    // great deal of tool output. This is the measurement that says which of
    // the two a policy should keep.
    let messages = vec![
        Message::new(Role::System, "you are operant"),
        Message::new(Role::User, "do the thing"),
        Message::new(Role::Assistant, "calling the tool"),
        Message::new(Role::Tool, "x".repeat(4_000)),
        Message::new(Role::Tool, "y".repeat(4_000)),
    ];
    let composition = context_composition(&messages);
    assert_eq!(composition.messages, 5);
    assert!(composition.tool_result_tokens > composition.user_tokens);
    assert!(
        composition.estimated_tokens >= composition.tool_result_tokens,
        "the total covers the tool results"
    );
    assert_eq!(
        composition.user_tokens,
        4 + 3,
        "the user turn's own estimate"
    );
}

#[test]
fn the_meter_says_whether_this_database_has_any_usage_at_all() {
    let (_temp, db) = db();
    let now = "2026-11-12T00:00:00Z";
    db.save_session("quiet", None, "agent", now, now).unwrap();
    assert_eq!(
        db.sessions_with_recorded_usage().unwrap(),
        0,
        "a fresh database has no metered sessions"
    );
    db.add_session_usage("quiet", 10, 1, 1).unwrap();
    assert_eq!(db.sessions_with_recorded_usage().unwrap(), 1);
}
