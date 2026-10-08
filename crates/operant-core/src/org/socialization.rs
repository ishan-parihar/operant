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
use rusqlite::{Connection, params};
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

/// **Phase 2 — decisions-of-record:** the senior posts the session's close-out
/// to the board, addressed to the junior.
///
/// §9's acceptance: "The senior seat's decision posts as a notice with the
/// junior's ratification; nothing else can." Structurally true — this is the
/// only notice write outside the CLI, and it is reachable only from the
/// session driver. The §2.3.1 consult runs through the same core resolver
/// the CLI seams use ([`crate::org::authority::resolve_actor_scope`] /
/// [`live_grants_for`](crate::org::authority::live_grants_for)), BEFORE any
/// row write; an unregistered senior is a refusal (fail-closed identity),
/// and the caller treats any refusal or error as a skipped post (fail-open —
/// the session itself already succeeded and densified both MEMORY.md files).
///
/// `root` is the seat-memory root (the dir holding `operant_kanban.db`), the
/// same handle the session driver already resolved.
pub fn post_session_outcome(root: &Path, senior: &str, junior: &str, outcome: &str) -> Result<()> {
    use std::sync::{Arc, Mutex};

    use crate::org::authority::{can_post_to, live_grants_for, resolve_actor_scope};
    use crate::org::notice::{PostNotice, Recipient};
    use crate::org::notice_db::NoticeBoard;

    let conn = Arc::new(Mutex::new(
        Connection::open(root.join("operant_kanban.db"))
            .map_err(|e| crate::error::Error::Agent(format!("socialization: open org db: {e}")))?,
    ));
    let board = NoticeBoard::from_shared_connection(conn.clone())?;
    // Fail-closed identity + the §2.3.1 consult, before any row write.
    let (scope, dept) = resolve_actor_scope(&conn, senior)?;
    let grants = live_grants_for(&conn, senior)?;
    let target = Recipient::Agent(junior.to_string());
    can_post_to(scope, dept.as_deref(), &target, &grants)
        .into_result()
        .map_err(|e| crate::error::Error::Agent(e.to_string()))?;

    let post = PostNotice::new(
        senior,
        vec![target],
        format!("Session outcome with {junior}: {outcome}"),
        format!("socialization session {senior}-{junior} close-out"),
    );
    board.post(&post)?;
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

    // ------------------------------------------ phase 2: outcome posting

    use crate::org::employee_db::EmployeeDb;
    use crate::org::notice_db::NoticeBoard;
    use std::sync::{Arc, Mutex};

    /// A root dir with a seeded registered senior. The employees schema
    /// comes up via `EmployeeDb::from_shared_connection`; the row lands via
    /// raw SQL (the registry has no insert-in-tests API — same pattern as
    /// the CLI's cmd_org test fixtures).
    fn seeded_root(senior: &str, register: bool) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let org_db = dir.path().join("operant_kanban.db");
        let conn = Arc::new(Mutex::new(
            rusqlite::Connection::open(&org_db).expect("open"),
        ));
        let employees = EmployeeDb::from_shared_connection(conn.clone()).expect("schema");
        let _ = employees; // schema ensured; the row goes in over the same handle
        if register {
            conn.lock()
                .expect("conn")
                .execute(
                    "INSERT OR REPLACE INTO employees (
                         employee_id, name, role, department, skills, agent_type,
                         persona, system_prompt, status, reason, created_at, updated_at
                     ) VALUES (?1, ?1, 'tester', 'crew', '[]', NULL, NULL, NULL, 'active',
                               'phase2 fixture', '2026-10-08T00:00:00Z',
                               '2026-10-08T00:00:00Z')",
                    rusqlite::params![senior],
                )
                .expect("seed senior");
        }
        (dir, org_db)
    }

    #[test]
    fn outcome_posts_from_a_registered_senior_to_the_junior() {
        let (dir, org_db) = seeded_root("emp-senior", true);
        post_session_outcome(dir.path(), "emp-senior", "emp-junior", "we agreed to ship")
            .expect("a registered senior may post its session outcome");
        let conn = rusqlite::Connection::open(&org_db).expect("reopen");
        let (sender, recipients): (String, String) = conn
            .query_row("SELECT sender, recipients FROM notices", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("one notice row");
        assert_eq!(sender, "emp-senior");
        assert!(
            recipients.contains("emp-junior"),
            "the outcome must be addressed to the junior: {recipients}"
        );
        let body: String = conn
            .query_row("SELECT body FROM notices", [], |r| r.get(0))
            .expect("body");
        assert!(body.contains("we agreed to ship"), "{body}");
    }

    #[test]
    fn outcome_from_an_unregistered_senior_is_refused_and_writes_nothing() {
        let (dir, org_db) = seeded_root("emp-ghost", false);
        let err = post_session_outcome(dir.path(), "emp-ghost", "emp-junior", "x")
            .expect_err("an unregistered senior must be refused");
        assert!(err.to_string().contains("unknown actor"), "{err}");
        let board = NoticeBoard::init(org_db).expect("board");
        assert_eq!(board.count().expect("count"), 0);
    }
}
