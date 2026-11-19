#![allow(clippy::expect_used, clippy::unwrap_used)]

//! RT-285(2): `SessionStore::discard` must destroy the transcript on disk, not
//! only the resident copy.
//!
//! Until iter-2681 `discard` did `self.warm().remove(key)` and nothing else,
//! while its doc promised "memory **and disk**" and the sole production caller
//! (`agent/events.rs`, the `/new` path) believed it — its own comment said
//! "without this the very next acquire would rehydrate the history /new was
//! called to discard", and that is exactly what happened. `clear_history()`
//! did NOT durably wipe a session: switching away and back brought the whole
//! conversation back.
//!
//! The negative control that shipped before this (`cron_session_isolation.rs`)
//! proved nothing, because it used the memory-only `add_message` and then
//! re-selected the SAME id, which `acquire` skips once the id is set. These
//! tests insert through the DATABASE path and reopen a SECOND store over the
//! same file, so the resident-copy-only body cannot pass.
//!
//! Mutation-proven: restoring the in-memory-only body reddens
//! `discarding_a_session_destroys_the_transcript_on_disk` by name.

use std::sync::Arc;

use operant_core::database::Database;
use operant_core::session::{SessionKey, SessionStore};

/// Insert `n` messages for `session_id` through the DATABASE, so the
/// transcript is durable rather than merely resident (`SessionStore::append`
/// writes only to the warm map, which is exactly why the old negative control
/// passed vacuously). `save_session` creates the session row (message inserts
/// are FK-constrained against it). Returns a store over the db.
fn persisted(
    db: &Arc<Database>,
    session_id: &str,
    n: usize,
) -> Result<(), operant_core::error::Error> {
    db.save_session(
        session_id,
        None,
        "test",
        "2026-11-19T00:00:00Z",
        "2026-11-19T00:00:00Z",
    )?;
    for i in 0..n {
        db.save_message(
            session_id,
            "user",
            &format!("message {i}"),
            "2026-11-19T00:00:00Z",
        )?;
    }
    Ok(())
}

#[test]
fn discarding_a_session_destroys_the_transcript_on_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    let sid = "wipe-me";
    persisted(&db, sid, 12).expect("seed");
    let store = SessionStore::new(Arc::clone(&db));

    // Acquire so the transcript is resident, and confirm it is really there.
    store.acquire(&SessionKey::new(sid));
    let warm = store.peek(&SessionKey::new(sid));
    assert_eq!(
        warm.len(),
        12,
        "the test must start from a resident transcript -- if this is 0 the insert \
         above did not land and every assertion below is vacuous"
    );

    // Discard it.
    let deleted = store
        .discard(&SessionKey::new(sid))
        .expect("discard is a destructive operation that must not fail on a real session");
    assert_eq!(
        deleted, 1,
        "discard must report the database row it deleted; 0 means it reached no \
         rows and the transcript is still on disk"
    );

    // A SECOND store over the SAME database -- the resident copy is gone, so
    // anything still visible must have come back from disk.
    let reopened = SessionStore::new(Arc::clone(&db));
    reopened.acquire(&SessionKey::new(sid));
    let after = reopened.peek(&SessionKey::new(sid));
    assert!(
        after.is_empty(),
        "the transcript survived the discard: the resident-copy-only body was \
         restored. /new does not durably wipe a session when this fails. \
         Rehydrated {} message(s).",
        after.len()
    );
}

#[test]
fn discarding_a_memory_only_session_is_a_successful_no_op() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    let store = SessionStore::new(Arc::clone(&db));

    // A session created only through the warm path is still persisted by
    // `acquire`'s `ON CONFLICT(id) DO UPDATE` upsert, so discard has one row
    // to remove. The point of this test is that a memory-only discard is NOT
    // an error: there is nothing on disk to lose, and the call still succeeds.
    let key = SessionKey::new("memory-only");
    store.acquire(&key);
    let deleted = store.discard(&key).expect("a memory-only discard succeeds");
    assert_eq!(
        deleted, 1,
        "acquire's upsert created one session row; discard removes exactly that"
    );
    assert!(store.peek(&key).is_empty(), "the resident copy must go too");
}

#[test]
fn a_discarded_session_does_not_take_its_siblings_with_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    for sid in ["keep-me", "drop-me"] {
        persisted(&db, sid, 4).expect("seed");
    }
    let store = SessionStore::new(Arc::clone(&db));

    let deleted = store
        .discard(&SessionKey::new("drop-me"))
        .expect("discard the second session");
    assert_eq!(deleted, 1, "only the discarded session's row goes");

    // The surviving session is still durable, read through a fresh store so the
    // resident copy cannot be what answers.
    let reopened = SessionStore::new(Arc::clone(&db));
    reopened.acquire(&SessionKey::new("keep-me"));
    let kept = reopened.peek(&SessionKey::new("keep-me"));
    assert_eq!(
        kept.len(),
        4,
        "discarding one session must not disturb another session's transcript"
    );
}
