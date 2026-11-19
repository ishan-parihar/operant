//! B4 / RT-598 (iter-2658) — the durable surface-question queue.
//!
//! A surface card that waits on the operator lives in two in-memory maps
//! (`PENDING_SURFACE_QUESTIONS`, `PENDING_SURFACE_OPS`): a restart, a
//! cron-fired surface with no TUI, or an hours-late tap all lose the oneshot
//! and resolve to the 120 s timeout string. This module persists the card so
//! the offer survives the process, and resolves a tap that arrives after a
//! restart against the choice the agent asked for.
//!
//! One row per in-flight card (`pending_surfaces`, added by
//! [`crate::database::SURFACE_QUEUE_MIGRATION`]). `answered_at` NULL means
//! still pending; a non-NULL value is the durable record that this card was
//! answered, so a replayed tap is a no-op instead of a double-answer.
//!
//! The queue is deliberately dumb: it stores and retrieves. The TUI-vs-
//! gateway split-brain (two hosts, one card) is a higher layer's job; this
//! module only guarantees that whatever is in flight survives a restart and
//! can be read back by surface_id.

use rusqlite::{Connection, OptionalExtension, params};

/// One persisted surface card.
#[derive(Debug, Clone)]
pub struct PendingSurfaceRow {
    /// The op's stable identity.
    pub surface_id: String,
    /// The `platform:channel_id` the card was sent to.
    pub channel_key: String,
    /// The serialized `SurfaceOp` (the widget tree + the choice ids).
    pub op_json: String,
    /// The offered choice ids, in render order (JSON array of strings).
    pub choice_ids_json: String,
    /// RFC 3339 timestamp of when the card was offered.
    pub created_at: String,
    /// RFC 3339 timestamp of when it was answered, or `None` while pending.
    pub answered_at: Option<String>,
    /// The structured answer JSON (`{"kind":...,"value":...}`), or `None`.
    pub answer_json: Option<String>,
}

impl PendingSurfaceRow {
    /// The offered choice ids as a `Vec<String>`, or empty if the JSON is
    /// malformed (a card with no choices stores `[]`).
    pub fn choice_ids(&self) -> Vec<String> {
        serde_json::from_str(&self.choice_ids_json).unwrap_or_default()
    }
}

/// Register (or replace) a pending card. `op_json` is the serialized
/// `SurfaceOp`; `choice_ids_json` is the offered choice ids in render order.
///
/// Re-offering the same `surface_id` overwrites the row (a re-rendered card
/// is the same card), so a restart that re-emits an unanswered surface does
/// not duplicate it.
pub fn register(
    conn: &Connection,
    surface_id: &str,
    channel_key: &str,
    op_json: &str,
    choice_ids_json: &str,
    created_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pending_surfaces
            (surface_id, channel_key, op_json, choice_ids_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(surface_id) DO UPDATE SET
            channel_key = excluded.channel_key,
            op_json = excluded.op_json,
            choice_ids_json = excluded.choice_ids_json,
            created_at = excluded.created_at,
            answered_at = NULL,
            answer_json = NULL",
        params![
            surface_id,
            channel_key,
            op_json,
            choice_ids_json,
            created_at
        ],
    )?;
    Ok(())
}

/// Mark a pending card answered. Returns `true` if a pending row was
/// transitioned (the first answer wins), `false` if the card was already
/// answered or never registered — so a replayed tap is a no-op, not a
/// double-answer.
pub fn answer(
    conn: &Connection,
    surface_id: &str,
    answer_json: &str,
    answered_at: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE pending_surfaces
            SET answered_at = ?2, answer_json = ?3
          WHERE surface_id = ?1 AND answered_at IS NULL",
        params![surface_id, answered_at, answer_json],
    )?;
    Ok(n == 1)
}

/// Load a pending (unanswered) card by id.
pub fn load(conn: &Connection, surface_id: &str) -> rusqlite::Result<Option<PendingSurfaceRow>> {
    conn.query_row(
        "SELECT surface_id, channel_key, op_json, choice_ids_json, created_at,
                answered_at, answer_json
           FROM pending_surfaces
          WHERE surface_id = ?1 AND answered_at IS NULL",
        params![surface_id],
        |r| {
            Ok(PendingSurfaceRow {
                surface_id: r.get(0)?,
                channel_key: r.get(1)?,
                op_json: r.get(2)?,
                choice_ids_json: r.get(3)?,
                created_at: r.get(4)?,
                answered_at: r.get(5)?,
                answer_json: r.get(6)?,
            })
        },
    )
    .optional()
}

/// Every still-pending card for a channel, oldest first — the replay set a
/// host re-offers after a restart.
pub fn pending_for_channel(
    conn: &Connection,
    channel_key: &str,
) -> rusqlite::Result<Vec<PendingSurfaceRow>> {
    let mut stmt = conn.prepare(
        "SELECT surface_id, channel_key, op_json, choice_ids_json, created_at,
                answered_at, answer_json
           FROM pending_surfaces
          WHERE channel_key = ?1 AND answered_at IS NULL
          ORDER BY created_at",
    )?;
    let rows = stmt
        .query_map(params![channel_key], |r| {
            Ok(PendingSurfaceRow {
                surface_id: r.get(0)?,
                channel_key: r.get(1)?,
                op_json: r.get(2)?,
                choice_ids_json: r.get(3)?,
                created_at: r.get(4)?,
                answered_at: r.get(5)?,
                answer_json: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE pending_surfaces (
                surface_id TEXT PRIMARY KEY,
                channel_key TEXT NOT NULL,
                op_json TEXT NOT NULL,
                choice_ids_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                answered_at TEXT,
                answer_json TEXT
            );",
        )
        .unwrap();
        c
    }

    #[test]
    fn register_then_load_roundtrips() {
        let c = conn();
        register(
            &c,
            "s1",
            "tg:1",
            "{\"op\":\"create\"}",
            "[\"a\",\"b\"]",
            "t0",
        )
        .unwrap();
        let row = load(&c, "s1").unwrap().expect("row present");
        assert_eq!(row.surface_id, "s1");
        assert_eq!(row.channel_key, "tg:1");
        assert_eq!(row.choice_ids(), vec!["a", "b"]);
        assert!(row.answered_at.is_none());
    }

    #[test]
    fn answer_transitions_pending_and_is_idempotent() {
        let c = conn();
        register(&c, "s1", "tg:1", "{}", "[]", "t0").unwrap();
        assert!(answer(&c, "s1", "{\"kind\":\"choice\",\"value\":\"a\"}", "t1").unwrap());
        // A second answer must NOT flip it (first answer wins).
        assert!(!answer(&c, "s1", "{\"kind\":\"choice\",\"value\":\"b\"}", "t2").unwrap());
        // And the answered row is no longer pending.
        assert!(load(&c, "s1").unwrap().is_none());
    }

    #[test]
    fn register_overwrites_and_resets_answer() {
        let c = conn();
        register(&c, "s1", "tg:1", "v1", "[]", "t0").unwrap();
        answer(&c, "s1", "{}", "t1").unwrap();
        // Re-offering the same surface reopens it (a restart re-emits).
        register(&c, "s1", "tg:1", "v2", "[]", "t2").unwrap();
        let row = load(&c, "s1").unwrap().expect("reopened");
        assert_eq!(row.op_json, "v2");
    }

    /// B4 / RT-598 (iter-2666): the durable-resolve flow a late tap takes
    /// when the in-memory oneshot is gone -- read the row, resolve against its
    /// choice_ids, answer it, and a REPLAY must be a no-op. This is the
    /// regression pin for the audit finding that B4 only ANNOTATED a row and
    /// never resolved from it.
    #[test]
    fn a_late_tap_resolves_from_the_persisted_row() {
        let c = conn();
        register(
            &c,
            "s1",
            "tg:1",
            "{\"op\":\"create\"}",
            "[\"a\",\"b\"]",
            "t0",
        )
        .unwrap();
        // The tap handler's read-back: newest-first is oldest-first on purpose,
        // so the CARD the operator is looking at is the one answered.
        let rows = pending_for_channel(&c, "tg:1").unwrap();
        let row = rows.into_iter().next().expect("the card is still pending");
        assert_eq!(row.choice_ids(), vec!["a", "b"]);
        // Resolve `choice:1` (index 1 -> ids[0] = "a") and answer durably.
        assert!(answer(&c, "s1", "{\"kind\":\"choice\",\"choice_id\":\"a\"}", "t1").unwrap());
        // A replayed tap finds no pending row: the answer is durable and a
        // double-tap cannot double-answer.
        assert!(pending_for_channel(&c, "tg:1").unwrap().is_empty());
        assert!(!answer(&c, "s1", "{}", "t2").unwrap());
    }

    #[test]
    fn pending_for_channel_orders_oldest_first() {
        let c = conn();
        register(&c, "s2", "tg:1", "{}", "[]", "t2").unwrap();
        register(&c, "s1", "tg:1", "{}", "[]", "t1").unwrap();
        let rows = pending_for_channel(&c, "tg:1").unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.surface_id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "s2"]
        );
    }
}
