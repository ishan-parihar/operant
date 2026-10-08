//! Daily socialization sessions — the scheduler-side coordinator (gap 6,
//! `plan-2026-10-08-socialization-sessions.md`).
//!
//! Design realization note: the design doc sketched the socializer as a
//! `cron_cast_socializer` registry row; the owner's standing ruling is
//! **9 CAST seats only in the registry** (infra jobs live outside it), so
//! the socializer is infra-owned scheduler code — the same posture as
//! dispatcher-retry. No 10th registry row.
//!
//! What lives here: the due computation (a one-row state table in the org
//! sibling db — table-not-a-file, the seat_policies discipline) and the
//! prompt/state helpers. The session driving lives on `CronScheduler`
//! (`cronjobs/scheduler.rs`) because that is where the agent machinery
//! (seat binding, memory, injection, metering) already composes.

use crate::error::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use std::str::FromStr;

/// One row, one bit of state: when the sessions last ran. A CHECK(id = 1)
/// keeps the table honest about being a singleton.
pub const SOCIAL_STATE_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS socialization_state (
        id       INTEGER PRIMARY KEY CHECK (id = 1),
        last_run TEXT NOT NULL
    );
"#;

/// Whether the sessions are due, given the schedule and the last run.
///
/// The most recent fire at-or-before `now` is the candidate; the sessions
/// are due when the last run predates it. A schedule that cannot parse
/// fails OPEN with the caller's warning (never blocks a tick, never
/// silently loop-runs).
pub fn socialization_due(schedule_raw: &str, last_run: Option<&str>, now: DateTime<Utc>) -> bool {
    let Ok(schedule) = cron::Schedule::from_str(schedule_raw) else {
        return false;
    };
    // The most recent fire at-or-before now. Scan a bounded window — a
    // daily schedule's previous fire is within a day; a misconfigured
    // every-second schedule is bounded by the window, not the loop.
    let earlier = now - chrono::Duration::hours(24);
    let Some(last_fire) = schedule.after(&earlier).take_while(|t| *t <= now).last() else {
        return false;
    };
    match last_run {
        None => true,
        Some(run) => {
            DateTime::parse_from_rfc3339(run)
                .map(|t| t.with_timezone(&Utc) < last_fire)
                .unwrap_or(true) // unreadable stamp: run, and let the record repair it
        }
    }
}

/// Read the last run stamp from the state table (`None` when never run).
pub fn last_socialization_run(db_path: &Path) -> Result<Option<String>> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::error::Error::Agent(format!("socialization: open: {e}")))?;
    conn.execute_batch(SOCIAL_STATE_SCHEMA)
        .map_err(|e| crate::error::Error::Agent(format!("socialization: schema: {e}")))?;
    let stamp: Option<String> = conn
        .query_row(
            "SELECT last_run FROM socialization_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| crate::error::Error::Agent(format!("socialization: state read: {e}")))?;
    Ok(stamp)
}

/// Record the run stamp.
pub fn record_socialization_run(db_path: &Path, at: &str) -> Result<()> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::error::Error::Agent(format!("socialization: open: {e}")))?;
    conn.execute_batch(SOCIAL_STATE_SCHEMA)
        .map_err(|e| crate::error::Error::Agent(format!("socialization: schema: {e}")))?;
    conn.execute(
        "INSERT INTO socialization_state (id, last_run) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET last_run = excluded.last_run",
        params![at],
    )
    .map_err(|e| crate::error::Error::Agent(format!("socialization: state write: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_on_first_boot_and_after_the_next_fire_not_before() {
        let now = Utc::now();
        // A schedule that fires every minute inside the window.
        let sched = format!("0 {} * * * *", now.format("%M"));
        assert!(socialization_due(&sched, None, now), "never ran → due");
        assert!(
            socialization_due(
                &sched,
                Some((now - chrono::Duration::minutes(5)).to_rfc3339().as_str()),
                now
            ),
            "ran before the latest fire → due"
        );
        assert!(
            !socialization_due(&sched, Some(now.to_rfc3339().as_str()), now),
            "ran at/after the latest fire → not due"
        );
    }

    #[test]
    fn unparseable_schedule_fails_open_and_the_state_roundtrips() {
        assert!(!socialization_due("not a schedule", None, Utc::now()));
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("state.sqlite");
        assert_eq!(last_socialization_run(&db).unwrap(), None);
        record_socialization_run(&db, "2026-10-08T09:30:00Z").unwrap();
        assert_eq!(
            last_socialization_run(&db).unwrap().as_deref(),
            Some("2026-10-08T09:30:00Z")
        );
    }
}
