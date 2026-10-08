//! Chief-of-staff synthesis (organism-memory plan Slice 9).
//!
//! The synthesis loop the org could not form before: compose the org's
//! operational state (notices, decisions, worklog) over a window into one
//! digest, retain it into the **org memory bank** (bank id [`ORG_BANK`],
//! the shared tier of the two-tier design — "anything that changes org
//! state or policy → org bank"), and post it to the board from
//! `chief-of-staff` so every seat's cycle can see it.
//!
//! ## Why a mechanical digest, not an LLM pass
//!
//! The org layer deliberately has no tool surface
//! (`decisions_db.rs`: "a model cannot propose or accept a decision
//! because there is no tool to call") — every org write goes through an
//! auditable CLI seam with a `--reason`. Synthesis keeps that property:
//! the digest is deterministic aggregation of stored rows, so the same
//! window always produces the same bytes, the operator can predict what
//! lands in the org bank before retaining it (`--dry-run`), and the
//! write is attributable to `chief-of-staff` with a recorded reason.
//!
//! ## Routing
//!
//! The digest posts as `broadcast` — the §3.2 global tier. `can_post_to`
//! allows broadcast at any scope, so no cross-department grant is
//! needed for the synthesis to reach every seat, and the digest is
//! exactly the kind of org-wide artefact the broadcast tier exists for.
//!
//! ## Idempotence
//!
//! None, on purpose: each synthesis is a new dated digest row (notice +
//! memory). The board's own `retention_gc` and the memory engine's
//! relevance ranking are the compaction layers; a "skip if one already
//! exists today" guard here would silently drop a genuinely changed
//! org state.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::Error;

use super::employee_db::org_db_path;
use super::notice::{PostNotice, Recipient};
use super::notice_db::NoticeBoard;
use super::worklog_db::{WorklogDb, WorklogQuery};

/// The org-level memory bank id (two-tier design: `org` + `emp-*`).
pub const ORG_BANK: &str = "org";

/// The seat the synthesis posts as. Chief-of-staff is premiere's right
/// hand — the seat whose job is cross-cutting awareness.
pub const SYNTHESIS_SENDER: &str = "chief-of-staff";

/// What one synthesis pass did, for the CLI to print and tests to assert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesisReport {
    /// The digest text itself — retained to the org bank and posted.
    pub digest: String,
    /// Id of the posted notice; `None` on `dry_run`.
    pub notice_id: Option<String>,
    /// Notices inside the window.
    pub notices: usize,
    /// Rows scanned from the notice board, including outside the window.
    pub notices_total: usize,
    /// Proposed decisions at synthesis time.
    pub decisions_proposed: usize,
    /// Accepted decisions at synthesis time.
    pub decisions_accepted: usize,
    /// Worklog rows inside the window.
    pub worklog_entries: usize,
    /// Whether the digest was retained to [`ORG_BANK`] (false on dry-run).
    pub retained: bool,
}

/// Compose the digest, retain it to the org bank, and post it as
/// `chief-of-staff` broadcast.
///
/// `database_path` is the main app database (the org sibling file is
/// derived from it, same as every other org store). `memory_store_dir`
/// is the memory-wire storage dir (`platform::operant_home()` in the
/// CLI); the org bank lives in the same `memory_wire.sqlite` the
/// per-session provider writes, distinguished by bank id — one store,
/// bank-isolated, exactly as `memory_wire.rs` describes.
///
/// `window_hours` bounds the "recent" sections; `dry_run` composes and
/// reports without writing anything (no retain, no notice).
pub fn synthesize(
    database_path: &Path,
    memory_store_dir: &Path,
    window_hours: i64,
    dry_run: bool,
) -> Result<SynthesisReport, Error> {
    if window_hours <= 0 {
        return Err(Error::Agent(format!(
            "synthesis window must be positive (got {window_hours}h)"
        )));
    }

    let conn = Arc::new(Mutex::new(
        Connection::open(org_db_path(database_path))
            .map_err(|e| Error::Agent(format!("synthesis: open org db: {e}")))?,
    ));
    let board = NoticeBoard::from_shared_connection(conn.clone())?;
    let worklog = WorklogDb::from_shared_connection(conn.clone())?;
    let decisions = super::decisions_db::DecisionsDb::for_app(database_path)?;

    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::hours(window_hours);
    let cutoff_iso = crate::org::notice::rfc3339(cutoff);

    // -------------------------------------------------------------- reads
    let (notices_total, recent_notices) = {
        let c = conn
            .lock()
            .map_err(|_| Error::Agent("synthesis: org db mutex poisoned".to_string()))?;
        let total: i64 = c
            .query_row("SELECT COUNT(*) FROM notices", [], |r| r.get(0))
            .map_err(|e| Error::Agent(format!("synthesis: notice count: {e}")))?;
        let mut stmt = c
            .prepare(
                "SELECT COALESCE(subject, ''), sender, body FROM notices \
                 WHERE created_at >= ?1 ORDER BY created_at DESC LIMIT 5",
            )
            .map_err(|e| Error::Agent(format!("synthesis: notice prepare: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![cutoff_iso], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("synthesis: notice query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Agent(format!("synthesis: notice read: {e}")))?);
        }
        (total, out)
    };

    let recent_worklog = worklog.list(&WorklogQuery {
        since: Some(cutoff.timestamp()),
        limit: Some(5),
        ..WorklogQuery::default()
    })?;
    let worklog_in_window = worklog
        .list(&WorklogQuery {
            since: Some(cutoff.timestamp()),
            ..WorklogQuery::default()
        })?
        .len();

    let proposed = decisions.list_by_status(super::decisions_db::DecisionStatus::Proposed)?;
    let accepted = decisions.list_by_status(super::decisions_db::DecisionStatus::Accepted)?;

    // ------------------------------------------------------------- compose
    let mut digest = String::new();
    digest.push_str(&format!(
        "# Org synthesis — {}\nwindow: last {window_hours}h (since {cutoff_iso})\n",
        crate::org::notice::rfc3339(now)
    ));

    digest.push_str(&format!(
        "\n## Board\n{notices_total} notices on file; {} in window.\n",
        recent_notices.len()
    ));
    for (subject, sender, body) in &recent_notices {
        let head = body.chars().take(160).collect::<String>();
        if subject.is_empty() {
            digest.push_str(&format!("- {sender}: {head}\n"));
        } else {
            digest.push_str(&format!("- {sender}: {subject} — {head}\n"));
        }
    }

    digest.push_str(&format!(
        "\n## Decisions\n{} proposed, {} accepted.\n",
        proposed.len(),
        accepted.len()
    ));
    for d in proposed.iter().take(10) {
        digest.push_str(&format!(
            "- [proposed] {} (by {}, scope {})\n",
            d.subject, d.decided_by, d.scope
        ));
    }

    digest.push_str(&format!(
        "\n## Worklog\n{worklog_in_window} entries in window.\n"
    ));
    for w in &recent_worklog {
        let head = w.what_done.chars().take(160).collect::<String>();
        digest.push_str(&format!("- {}: {} ({})\n", w.employee, head, w.outcome));
    }

    if dry_run {
        return Ok(SynthesisReport {
            digest,
            notice_id: None,
            notices: recent_notices.len(),
            notices_total: notices_total as usize,
            decisions_proposed: proposed.len(),
            decisions_accepted: accepted.len(),
            worklog_entries: worklog_in_window,
            retained: false,
        });
    }

    // ------------------------------------------------------- org bank write
    let store_path = memory_store_dir.join("memory_wire.sqlite");
    let store = memory_wire::store::SqliteStore::open(&store_path)
        .map_err(|e| Error::Agent(format!("synthesis: memory store open: {e}")))?;
    let service = memory_wire::api::MemoryService::new(store);
    service
        .retain(ORG_BANK, &digest, None)
        .map_err(|e| Error::Agent(format!("synthesis: org bank retain: {e}")))?;

    // ----------------------------------------------------------- board post
    let mut post = PostNotice::new(
        SYNTHESIS_SENDER,
        vec![Recipient::Broadcast],
        digest.clone(),
        format!("chief-of-staff synthesis over the last {window_hours}h"),
    );
    post.subject = Some(format!("Org synthesis {window_hours}h"));
    let notice = board.post(&post)?;

    Ok(SynthesisReport {
        digest,
        notice_id: Some(notice.id),
        notices: recent_notices.len(),
        notices_total: notices_total as usize,
        decisions_proposed: proposed.len(),
        decisions_accepted: accepted.len(),
        worklog_entries: worklog_in_window,
        retained: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_app() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_db = dir.path().join("operant.db");
        (dir, app_db)
    }

    #[test]
    fn empty_org_synthesizes_a_header_only_digest() {
        let (_dir, app_db) = temp_app();
        let mem_dir = tempfile::tempdir().expect("memdir");
        let report = synthesize(&app_db, mem_dir.path(), 24, false).expect("synthesize");
        assert!(
            report.digest.contains("# Org synthesis"),
            "{}",
            report.digest
        );
        assert!(report.digest.contains("0 notices"), "{}", report.digest);
        assert!(report.notice_id.is_some(), "a live run must post");
        assert!(report.retained);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (_dir, app_db) = temp_app();
        let mem_dir = tempfile::tempdir().expect("memdir");
        let report = synthesize(&app_db, mem_dir.path(), 24, true).expect("dry run");
        assert_eq!(report.notice_id, None);
        assert!(!report.retained);
        // No memory rows: the store file is not even created by a dry run.
        assert!(!mem_dir.path().join("memory_wire.sqlite").exists());
        // No notices on the board.
        let board = NoticeBoard::init(org_db_path(&app_db)).expect("board");
        assert_eq!(board.count().expect("count"), 0);
    }

    #[test]
    fn digest_reaches_the_org_bank_and_the_board() {
        let (_dir, app_db) = temp_app();
        let mem_dir = tempfile::tempdir().expect("memdir");
        // Seed one notice so the digest carries real content.
        let board = NoticeBoard::init(org_db_path(&app_db)).expect("board");
        let post = PostNotice::new(
            "user",
            vec![Recipient::Broadcast],
            "ship the port this week",
            "seed",
        );
        board.post(&post).expect("seed post");

        let report = synthesize(&app_db, mem_dir.path(), 24, false).expect("synthesize");
        assert_eq!(report.notices, 1, "the seeded notice is in window");
        assert!(report.digest.contains("ship the port"), "{}", report.digest);

        // Org bank: recall from bank "org" finds the digest.
        let store =
            memory_wire::store::SqliteStore::open(&mem_dir.path().join("memory_wire.sqlite"))
                .expect("store");
        let service = memory_wire::api::MemoryService::new(store);
        let hits = service
            .recall_with_weights(
                ORG_BANK,
                "org synthesis",
                2000,
                &memory_wire::recall::FusionWeights::SHIPPED,
            )
            .expect("recall");
        assert!(!hits.is_empty(), "the digest must reach the org bank");

        // Board: one seeded + one synthesis broadcast.
        assert_eq!(board.count().expect("count"), 2);
        let notice_id = report.notice_id.expect("posted");
        let notice = board.get(&notice_id).expect("get").expect("row");
        assert_eq!(notice.sender, SYNTHESIS_SENDER);
    }

    #[test]
    fn zero_hour_window_is_refused() {
        let (_dir, app_db) = temp_app();
        let mem_dir = tempfile::tempdir().expect("memdir");
        let err = synthesize(&app_db, mem_dir.path(), 0, false).expect_err("must refuse");
        assert!(err.to_string().contains("positive"), "{err}");
    }
}
