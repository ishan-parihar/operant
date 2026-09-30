//! The worklog store: sqlite DDL, the framework-only append, and reads.
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.4 (DDL verbatim, field provenance,
//! the AF-AD-007 framework-authorship rule). Types live in
//! [`super::worklog`].
//!
//! ## Storage location (§1.2, Q2)
//!
//! The table lives in the **existing** `operant_kanban.db`, the same sibling
//! file the employee registry and the notice board use. No new database
//! file, no JSONL — the "no second store" rule.
//!
//! Like the employee registry, this DDL does **not** go through
//! `crate::migrations::migrate`. That runner stamps `PRAGMA user_version`,
//! which is **file-wide** in SQLite; `migrations.rs:31-47` documents the
//! invariant that each migration family owns a dedicated file. The kanban
//! file already belongs to the kanban family, so a second `migrate` call
//! there would be the R39-7 split-brain. Plain `CREATE TABLE IF NOT EXISTS`
//! is already idempotent, so it needs no version tracking and leaves the
//! kanban family's `user_version` untouched.
//!
//! ## Append-only is enforced twice, deliberately
//!
//! §3.4's organism citation is "One row in a dept worklog. **Append-only.**"
//! That is enforced at two independent layers:
//!
//! 1. **API shape** — this module exposes [`WorklogDb::append`] and read
//!    methods. There is no `update`, no `delete`, no `clear`, and no method
//!    that takes a row id and mutates.
//! 2. **Storage** — the DDL installs `BEFORE UPDATE` and `BEFORE DELETE`
//!    triggers that `RAISE(ABORT)`. A caller who goes around the API with a
//!    raw `Connection` still cannot rewrite or erase history.
//!
//! Layer 2 is the one that matters for an *audit* log: layer 1 is a
//! convention that a later contributor can route around, while a trigger
//! holds until someone edits the schema. Adding a future retention/GC path
//! means explicitly dropping the trigger, which is a visible, reviewable act
//! rather than a silent capability that appeared in a diff.
//!
//! ## What this store does not do
//!
//! - **It does not write board-routing fields.** There is no `to` and no
//!   `ack_required` column, per `agent_fabric.py:144-146`. The objective log
//!   and the subjective board stay separate stores even inside one file.
//! - **It does not expose a tool.** Nothing here is registered in
//!   `ToolRegistry`. A model cannot append a row because there is no tool to
//!   call; see [`super::worklog`] for the full enforcement argument.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::Error;
use crate::turn_end::RESULT_SUMMARY_LIMIT;

use super::worklog::{Outcome, UNKNOWN_EMPLOYEE, WorklogEntry, WorklogRecord};

/// The §3.4 DDL, plus the three indexes.
///
/// **Column count: 21, not 20.** The 20 are the organism's
/// `WorklogEntry` attributes (`agent_fabric.py:141-167`); `ts_iso` is the
/// 21st and the *only* one with no organism counterpart — §3.4: "`ts_iso`
/// is the one column with no organism counterpart. It is a read-side
/// convenience so `operant org` can render a timestamp without a `datetime`
/// conversion on every row; `ts` remains the stored truth." So the column
/// list below is 21 columns of which 20 are organism fields. §2 of the
/// decision doc records that the *plan* claimed 15 in prose, enumerated 16,
/// and the organism actually has 20 — three different numbers, which is why
/// the count is spelled out here rather than left implicit.
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS worklog (
        id                    TEXT PRIMARY KEY,  -- uuid v4
        ts                    INTEGER NOT NULL,  -- unix seconds
        ts_iso                TEXT NOT NULL,     -- RFC3339, for humans
        employee              TEXT NOT NULL,     -- emp-<hex>; 'unknown' if unbackfilled
        department            TEXT,              -- NULL in Wave 1 (no dept concept yet)
        job_id                TEXT,              -- cron job id, NULL for interactive turns
        iteration             INTEGER NOT NULL DEFAULT 0,
        workflow_kind         TEXT NOT NULL DEFAULT 'process',  -- gather|process|publish|monitor|remediate|synthesize|coordinate
        what_done             TEXT NOT NULL,     -- result_summary, truncated
        outcome               TEXT NOT NULL,     -- success|partial|failure|noop
        artifacts             TEXT NOT NULL DEFAULT '[]',  -- JSON array
        tokens_in             INTEGER NOT NULL DEFAULT 0,
        tokens_out            INTEGER NOT NULL DEFAULT 0,
        tool_calls            INTEGER NOT NULL DEFAULT 0,
        duration_s            REAL NOT NULL DEFAULT 0.0,
        next_intent           TEXT NOT NULL DEFAULT '',
        blockers              TEXT NOT NULL DEFAULT '[]',  -- JSON array
        improvement_proposal  TEXT,              -- NULL, the kaizen field
        correlation_id        TEXT,              -- framework-only FK to notices
        notice_ids            TEXT NOT NULL DEFAULT '[]',  -- framework-only, JSON array
        session_id            TEXT NOT NULL      -- the operant session that produced this
    );

    CREATE INDEX IF NOT EXISTS idx_worklog_employee ON worklog(employee, ts);
    CREATE INDEX IF NOT EXISTS idx_worklog_session  ON worklog(session_id);
    CREATE INDEX IF NOT EXISTS idx_worklog_dept     ON worklog(department, ts);
"#;

/// The SQLite file the worklog table lives in. Delegates to the employee
/// registry's path helper so the "one sibling kanban file" rule is stated
/// once rather than re-derived per module.
pub fn worklog_db_path(database_path: &Path) -> PathBuf {
    super::employee_db::org_db_path(database_path)
}

/// A filter for [`WorklogDb::list`]. Every field is `None` = "no filter on
/// this dimension"; an unfiltered list is the newest-first page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorklogQuery {
    /// Match `worklog.employee`.
    pub employee: Option<String>,
    /// Match `worklog.session_id`.
    pub session_id: Option<String>,
    /// Match `worklog.department`. `Some("")` is not "any department" —
    /// it is a literal match against the (currently always NULL) column,
    /// which correctly returns nothing in Wave 1.
    pub department: Option<String>,
    /// Match `worklog.job_id`.
    pub job_id: Option<String>,
    /// Match `worklog.outcome`.
    pub outcome: Option<Outcome>,
    /// Return rows with `ts >= this`.
    pub since: Option<i64>,
    /// Return rows with `ts <= this`.
    pub until: Option<i64>,
    /// Return rows that have a non-NULL `improvement_proposal`.
    pub has_improvement_proposal: bool,
    /// Cap on returned rows.
    pub limit: Option<usize>,
}

/// The worklog store: one sqlite table, append-only, framework-written.
pub struct WorklogDb {
    conn: Arc<Mutex<Connection>>,
}

impl WorklogDb {
    /// Open (or create) the worklog and apply the DDL.
    ///
    /// Takes the main app database path and derives the sibling kanban file,
    /// matching `employee_db::org_db_path` — the same derivation
    /// `main.rs:967-971` and `cmd_cron.rs:82-92` already use. Callers that
    /// already hold a `KanbanDb` can use [`WorklogDb::from_shared_connection`]
    /// to share one handle rather than opening a second writer.
    pub fn init(database_path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(worklog_db_path(database_path))
            .map_err(|e| Error::Agent(format!("Failed to open worklog database: {e}")))?;
        Self::from_connection(conn)
    }

    /// Open the worklog on a connection the caller already owns.
    ///
    /// This is the form a framework subscriber wants when it is handed the
    /// live `Arc<Mutex<Connection>>` from `KanbanDb::conn` — one file, one
    /// handle, no second writer contending for the lock.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.setup_schema()?;
        Ok(db)
    }

    /// Borrow an already-open `Arc<Mutex<Connection>>` (the shape
    /// `KanbanDb::conn` returns) so the worklog shares the kanban handle.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.setup_schema()?;
        Ok(db)
    }

    /// The shared connection, for a caller that needs to read alongside the
    /// other org tables in one query.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("worklog db mutex poisoned".to_string()))
    }

    /// §3.4 DDL, then the append-only triggers, then nothing else. Never
    /// stamps `user_version` — see the module docs.
    fn setup_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| Error::Agent(format!("worklog schema failed: {e}")))?;
        for (name, ddl) in TRIGGERS {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = ?1)",
                    params![name],
                    |row| row.get(0),
                )
                .map_err(|e| Error::Agent(format!("worklog trigger probe failed: {e}")))?;
            if exists {
                continue;
            }
            conn.execute_batch(ddl).map_err(|e| {
                Error::Agent(format!("worklog trigger {name} creation failed: {e}"))
            })?;
        }
        Ok(())
    }

    /// **The framework-only write.** Append one row.
    ///
    /// This is the single mutation in the module. It is `pub` because the
    /// framework's session-end subscriber lives in another module, not
    /// because it is generally callable — the *only* way to obtain the
    /// [`WorklogEntry`] it takes is [`super::worklog::TurnObservation::
    /// into_entry`], which requires a framework-minted `TurnEnd`.
    ///
    /// Idempotency: the `id` is a uuid v4 minted at
    /// `into_entry`, so a retry after a transport error writes a *second*
    /// row rather than silently overwriting the first. That is the correct
    /// behavior for an append-only log — a duplicate is visible and
    /// dedupable, a silent overwrite is neither.
    pub fn append(&self, entry: &WorklogEntry) -> Result<(), Error> {
        let artifacts = serde_json::to_string(entry.artifacts())
            .map_err(|e| Error::Agent(format!("worklog artifacts encode failed: {e}")))?;
        let blockers = serde_json::to_string(entry.blockers())
            .map_err(|e| Error::Agent(format!("worklog blockers encode failed: {e}")))?;
        let notice_ids = serde_json::to_string(entry.notice_ids())
            .map_err(|e| Error::Agent(format!("worklog notice_ids encode failed: {e}")))?;

        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO worklog (
                 id, ts, ts_iso, employee, department, job_id, iteration,
                 workflow_kind, what_done, outcome, artifacts, tokens_in,
                 tokens_out, tool_calls, duration_s, next_intent, blockers,
                 improvement_proposal, correlation_id, notice_ids, session_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
                entry.id(),
                entry.ts(),
                entry.ts_iso(),
                entry.employee(),
                entry.department(),
                entry.job_id(),
                entry.iteration(),
                entry.workflow_kind().as_str(),
                entry.what_done(),
                entry.outcome().as_str(),
                artifacts,
                entry.tokens_in(),
                entry.tokens_out(),
                entry.tool_calls(),
                entry.duration_s(),
                entry.next_intent(),
                blockers,
                entry.improvement_proposal(),
                entry.correlation_id(),
                notice_ids,
                entry.session_id(),
            ],
        )
        .map_err(|e| Error::Agent(format!("worklog append failed: {e}")))?;
        Ok(())
    }

    /// Total rows. Cheap enough for a CLI status line; not on a hot path.
    pub fn count(&self) -> Result<i64, Error> {
        let conn = self.lock_conn()?;
        conn.query_row("SELECT COUNT(*) FROM worklog", [], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("worklog count failed: {e}")))
    }

    /// Newest-first page of rows matching `query`.
    ///
    /// `ORDER BY ts DESC, id DESC` so a same-second page is still stable:
    /// two rows written in the same unix second have no defined order
    /// otherwise, and the reader would see them swap between calls.
    pub fn list(&self, query: &WorklogQuery) -> Result<Vec<WorklogRecord>, Error> {
        // `SELECT_COLUMNS` (the `WHERE 1=1` base), *not* `SELECT_COLUMNS_FROM`
        // — the filters below append `AND …` clauses, which only compose
        // onto a neutral predicate. Building on the by-id variant left every
        // filtered read with both `id = ?1` and the filter, and then failed
        // to bind `:employee` at all (see the SQL constant's docs).
        let mut sql = String::from(SELECT_COLUMNS);
        if query.employee.is_some() {
            sql.push_str(" AND employee = :employee");
        }
        if query.session_id.is_some() {
            sql.push_str(" AND session_id = :session_id");
        }
        if query.department.is_some() {
            sql.push_str(" AND department = :department");
        }
        if query.job_id.is_some() {
            sql.push_str(" AND job_id = :job_id");
        }
        if query.outcome.is_some() {
            sql.push_str(" AND outcome = :outcome");
        }
        if query.since.is_some() {
            sql.push_str(" AND ts >= :since");
        }
        if query.until.is_some() {
            sql.push_str(" AND ts <= :until");
        }
        if query.has_improvement_proposal {
            sql.push_str(" AND improvement_proposal IS NOT NULL");
        }
        sql.push_str(" ORDER BY ts DESC, id DESC");
        if query.limit.is_some() {
            sql.push_str(" LIMIT :limit");
        }

        let conn = self.lock_conn()?;
        let mut stmt = conn_statement(&conn, &sql)?;
        let rows = bind_and_collect(&mut stmt, query)
            .map_err(|e| Error::Agent(format!("worklog list failed: {e}")))?;
        Ok(rows)
    }

    /// One row by its uuid. `None` when the id is unknown.
    pub fn get(&self, id: &str) -> Result<Option<WorklogRecord>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(SELECT_COLUMNS_FROM)
            .map_err(|e| Error::Agent(format!("worklog get prepare failed: {e}")))?;
        let row = stmt
            .query_row(params![id], map_row)
            .optional()
            .map_err(|e| Error::Agent(format!("worklog get failed: {e}")))?;
        Ok(row)
    }

    /// Every `improvement_proposal` recorded so far, oldest first.
    ///
    /// This is the Wave 4 DEPT-loop input per §3.4 — "it is a **Wave 4
    /// DEPT-loop input**, not a Wave 1 deliverable". Wave 1 ships the column
    /// and this read, and the read is the *only* place the kaizen field is
    /// ever pulled out of the objective log for a consumer that is not the
    /// log itself.
    pub fn improvement_proposals(&self, limit: Option<usize>) -> Result<Vec<String>, Error> {
        let conn = self.lock_conn()?;
        let mut sql = String::from(
            "SELECT improvement_proposal FROM worklog \
             WHERE improvement_proposal IS NOT NULL ORDER BY ts ASC, id ASC",
        );
        // The `:limit` binding is conditional for the same reason
        // `bind_and_collect`'s is: rusqlite rejects a bound name the
        // statement does not contain, so binding it unconditionally would
        // make the unbounded read — the common case, `None` — fail at
        // prepare. The `push_str` and the binding must be added together.
        let limit_value: Option<Box<dyn rusqlite::ToSql>> =
            limit.map(|l| Box::new(l as i64) as Box<dyn rusqlite::ToSql>);
        if limit_value.is_some() {
            sql.push_str(" LIMIT :limit");
        }
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Agent(format!("worklog proposals prepare failed: {e}")))?;
        let named: Vec<(&str, &dyn rusqlite::ToSql)> = limit_value
            .as_ref()
            .map(|v| vec![(":limit", v.as_ref())])
            .unwrap_or_default();
        let mut out = stmt
            .query_map(named.as_slice(), |row| row.get::<_, String>(0))
            .map_err(|e| Error::Agent(format!("worklog proposals query failed: {e}")))?;
        let mut proposals = Vec::new();
        for row in out.by_ref() {
            proposals
                .push(row.map_err(|e| Error::Agent(format!("worklog proposal read failed: {e}")))?);
        }
        Ok(proposals)
    }

    /// Rows whose `employee` is still the [`super::worklog::UNKNOWN_EMPLOYEE`]
    /// placeholder, i.e. sessions that never got a backfilled employee.
    ///
    /// Exposed because §3.4 makes the `employee` default a *statement* about
    /// identity, and a reader of the worklog needs to be able to check
    /// whether the backfill has caught up. Not a mutation.
    pub fn unbackfilled(&self, limit: usize) -> Result<Vec<WorklogRecord>, Error> {
        self.list(&WorklogQuery {
            employee: Some(UNKNOWN_EMPLOYEE.to_string()),
            limit: Some(limit),
            ..Default::default()
        })
    }
}

/// The column list every read shares, in the exact order [`map_row`] reads
/// them. One constant so a column added to the DDL and a column added here
/// cannot drift apart silently — §1.3 Finding 3 is the measured hazard of
/// positional row mapping in this repo, and the mitigation is to have a
/// single column list and a single mapper, both named.
///
/// This is the base for [`WorklogDb::list`]: the neutral `WHERE 1=1` is what
/// the filter clauses (`AND employee = :employee`, …) append onto.
const SELECT_COLUMNS: &str = "SELECT id, ts, ts_iso, employee, department, job_id, iteration, \
     workflow_kind, what_done, outcome, artifacts, tokens_in, tokens_out, tool_calls, \
     duration_s, next_intent, blockers, improvement_proposal, correlation_id, notice_ids, \
     session_id FROM worklog WHERE 1=1";

/// The same column list with the by-id predicate, for the single-row lookup,
/// which replaces the `WHERE` rather than adding to it.
const SELECT_COLUMNS_FROM: &str = "SELECT id, ts, ts_iso, employee, department, job_id, iteration, \
     workflow_kind, what_done, outcome, artifacts, tokens_in, tokens_out, tool_calls, \
     duration_s, next_intent, blockers, improvement_proposal, correlation_id, notice_ids, \
     session_id FROM worklog WHERE id = ?1";

/// Prepare `sql` on `conn`, mapping the error into the crate's error type.
fn conn_statement<'a>(conn: &'a Connection, sql: &str) -> Result<rusqlite::Statement<'a>, Error> {
    conn.prepare(sql)
        .map_err(|e| Error::Agent(format!("worklog prepare failed: {e}")))
}

/// Bind `query`'s named parameters and read every row.
///
/// Named (rather than positional) parameters matter here: §1.3 Finding 3 is
/// the measured hazard of positional row mapping in this repo, and the
/// fix is the same in both directions — bind by name so the SQL and the
/// struct cannot drift into a plausible wrong value.
///
/// **Only parameters the SQL actually contains may be bound.** rusqlite
/// errors with `Invalid parameter name: :x` when a bound name is absent
/// from the statement, so the binding list below is built conditionally and
/// mirrors `list`'s `if query.x.is_some()` predicates exactly. The two must
/// move together: add a predicate without its binding and every filtered
/// read fails at prepare; add a binding without its predicate and every
/// read fails the same way. (An earlier revision of this function bound all
/// parameters unconditionally on the belief that a surplus name was
/// ignored — it is not, and five tests caught it.)
fn bind_and_collect(
    stmt: &mut rusqlite::Statement<'_>,
    query: &WorklogQuery,
) -> Result<Vec<WorklogRecord>, rusqlite::Error> {
    // `&[(&str, &dyn ToSql)]` is the shape rusqlite 0.32 actually implements
    // `Params` for in the named case (`params.rs:223`). `params_from_iter`
    // is positional-only for tuples and rejects a `(&str, Value)` item.
    let mut bound: Vec<(&str, Box<dyn rusqlite::ToSql>)> = Vec::new();
    let mut bind = |name: &'static str, value: Option<Box<dyn rusqlite::ToSql>>| {
        if let Some(value) = value {
            bound.push((name, value));
        }
    };

    bind(
        ":employee",
        query
            .employee
            .clone()
            .map(|v| Box::new(v) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":session_id",
        query
            .session_id
            .clone()
            .map(|v| Box::new(v) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":department",
        query
            .department
            .clone()
            .map(|v| Box::new(v) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":job_id",
        query
            .job_id
            .clone()
            .map(|v| Box::new(v) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":outcome",
        query
            .outcome
            .map(|o| Box::new(o.as_str().to_string()) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":since",
        query.since.map(|v| Box::new(v) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":until",
        query.until.map(|v| Box::new(v) as Box<dyn rusqlite::ToSql>),
    );
    bind(
        ":limit",
        query
            .limit
            .map(|l| Box::new(l as i64) as Box<dyn rusqlite::ToSql>),
    );

    let named: Vec<(&str, &dyn rusqlite::ToSql)> = bound
        .iter()
        .map(|(name, value)| (*name, value.as_ref()))
        .collect();
    let mut rows = stmt.query(named.as_slice())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map_row(row)?);
    }
    Ok(out)
}

/// Map one row to a [`WorklogRecord`], reading by the column order in
/// [`SELECT_COLUMNS`].
///
/// The three JSON columns are decoded leniently: a row whose `artifacts` is
/// not a JSON array reads as `[]` rather than failing the whole page. That
/// matters because the arrays are written by this module and are therefore
/// always valid in practice — the lenient path is for a row written by a
/// future migration or a hand-edited DB, and degrading one row is the right
/// response, not erroring the whole command.
fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorklogRecord> {
    let artifacts: Option<String> = row.get(10)?;
    let blockers: Option<String> = row.get(16)?;
    let notice_ids: Option<String> = row.get(19)?;
    let what_done: Option<String> = row.get(8)?;
    // Measured before `what_done` is consumed below, so the borrow order is
    // explicit rather than a clone.
    let result_truncated = what_done
        .as_deref()
        .is_some_and(|s| s.len() >= RESULT_SUMMARY_LIMIT);

    Ok(WorklogRecord {
        id: row.get(0)?,
        ts: row.get(1)?,
        ts_iso: row.get(2)?,
        employee: row.get(3)?,
        department: row.get(4)?,
        job_id: row.get(5)?,
        iteration: row.get(6)?,
        workflow_kind: row.get(7)?,
        what_done: what_done.unwrap_or_default(),
        outcome: row.get(9)?,
        artifacts: decode_array(artifacts.as_deref()),
        tokens_in: row.get(11)?,
        tokens_out: row.get(12)?,
        tool_calls: row.get(13)?,
        duration_s: row.get(14)?,
        next_intent: row.get(15)?,
        blockers: decode_array(blockers.as_deref()),
        improvement_proposal: row.get(17)?,
        correlation_id: row.get(18)?,
        notice_ids: decode_array(notice_ids.as_deref()),
        session_id: row.get(20)?,
        // There is no column for this (see `WorklogRecord::result_truncated`),
        // but the seam only ever truncates *at* `RESULT_SUMMARY_LIMIT`, so a
        // summary sitting exactly on the limit is the detectable residue of a
        // clipped write. Reporting that beats a hardcoded `false` that reads
        // as "not truncated" for a row that demonstrably was.
        result_truncated,
    })
}

/// Decode a JSON-array column, treating `NULL` and a non-array as empty.
fn decode_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default()
}

/// The append-only triggers: `(name, ddl)`, created only when absent.
///
/// SQLite has no `CREATE TRIGGER IF NOT EXISTS`; a second creation raises
/// `trigger already exists`. Probing `sqlite_master` first is what makes
/// [`WorklogDb::setup_schema`] re-runnable, which matters because a second
/// `WorklogDb` can be built on the same file in the same process.
///
/// The abort messages name the operation, so a caller that trips one learns
/// the table is a log and not a cache.
const TRIGGERS: &[(&str, &str)] = &[
    (
        "worklog_no_update",
        "CREATE TRIGGER worklog_no_update BEFORE UPDATE ON worklog
         BEGIN
             SELECT RAISE(ABORT, 'worklog is append-only (AF-AD-007): UPDATE is not permitted');
         END",
    ),
    (
        "worklog_no_delete",
        "CREATE TRIGGER worklog_no_delete BEFORE DELETE ON worklog
         BEGIN
             SELECT RAISE(ABORT, 'worklog is append-only (AF-AD-007): DELETE is not permitted');
         END",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::worklog::{
        KaizenProposal, OutcomeSignals, TurnObservation, UNKNOWN_EMPLOYEE, WORKLOG_DB_FILE,
    };
    use crate::turn_end::TurnEnd;

    fn temp_db() -> (tempfile::TempDir, WorklogDb) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = WorklogDb::init(&dir.path().join("database.db")).expect("worklog init");
        (dir, db)
    }

    fn turn(session: &str, reply: &str) -> TurnEnd {
        TurnEnd {
            turn_id: 0,
            session_id: session.to_string(),
            iterations: 2,
            tool_calls: 4,
            tool_durations_ms: vec![10, 20, 30, 40],
            result_summary: reply.to_string(),
            result_truncated: false,
        }
    }

    fn success_entry(session: &str) -> WorklogEntry {
        TurnObservation::from_turn_end(turn(session, "did the thing"))
            .with_signals(OutcomeSignals {
                done_emitted: true,
                turn_errored: false,
                last_status_clean: true,
            })
            .into_entry()
    }

    #[test]
    fn append_persists_and_reads_back_every_field() {
        let (_dir, db) = temp_db();
        let mut usage = crate::org::worklog::UsageAccumulator::new();
        usage.record(10, 5);
        let entry = TurnObservation::from_turn_end(turn("sess-1", "done"))
            .with_employee("emp-abc")
            .with_job_id(Some("cron-1".to_string()))
            .with_usage(usage)
            .with_duration_s(1.25)
            .with_artifacts(vec!["/tmp/out.txt".to_string()])
            .with_blockers(vec!["rate_limited".to_string()])
            .with_correlation_id(Some("corr-1".to_string()))
            .with_notice_ids(vec!["notice-1".to_string()])
            .with_signals(OutcomeSignals {
                done_emitted: true,
                turn_errored: false,
                last_status_clean: true,
            })
            .into_entry();
        let id = entry.id().to_string();
        db.append(&entry).expect("append");

        let rows = db.list(&WorklogQuery::default()).expect("list");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, id);
        assert!(row.ts > 0);
        assert!(row.ts_iso.ends_with('Z') || row.ts_iso.contains('+'));
        assert_eq!(row.employee, "emp-abc");
        assert_eq!(row.department, None);
        assert_eq!(row.job_id.as_deref(), Some("cron-1"));
        assert_eq!(row.iteration, 2);
        assert_eq!(row.workflow_kind, "process");
        assert_eq!(row.what_done, "done");
        assert_eq!(row.outcome, "success");
        assert_eq!(row.artifacts, vec!["/tmp/out.txt".to_string()]);
        assert_eq!(row.tokens_in, 10);
        assert_eq!(row.tokens_out, 5);
        assert_eq!(row.tool_calls, 4);
        assert!((row.duration_s - 1.25).abs() < 1e-9);
        assert_eq!(row.next_intent, "");
        assert_eq!(row.blockers, vec!["rate_limited".to_string()]);
        assert_eq!(row.correlation_id.as_deref(), Some("corr-1"));
        assert_eq!(row.notice_ids, vec!["notice-1".to_string()]);
        assert_eq!(row.session_id, "sess-1");
        assert_eq!(row.improvement_proposal, None);
    }

    #[test]
    fn improvement_proposal_round_trips_distinctly_and_survives_sql() {
        let (_dir, db) = temp_db();
        let bare = success_entry("sess-bare");
        db.append(&bare).expect("append bare");

        let proposed = TurnObservation::from_turn_end(turn("sess-k", "done"))
            .with_improvement_proposal(Some(KaizenProposal::new(
                "The digest cron should run at 07:00 not 07:30",
            )))
            .into_entry();
        db.append(&proposed).expect("append proposed");

        let rows = db.list(&WorklogQuery::default()).expect("list");
        let with = rows
            .iter()
            .find(|r| r.session_id == "sess-k")
            .expect("proposed row");
        let without = rows
            .iter()
            .find(|r| r.session_id == "sess-bare")
            .expect("bare row");

        assert_eq!(
            with.improvement_proposal.as_deref(),
            Some("The digest cron should run at 07:00 not 07:30")
        );
        assert_eq!(without.improvement_proposal, None);
        // The proposal is its own column, not smuggled into `what_done`.
        assert_eq!(with.what_done, "done");
        // And it is queryable on its own.
        let filtered = db
            .list(&WorklogQuery {
                has_improvement_proposal: true,
                ..Default::default()
            })
            .expect("list filtered");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "sess-k");
        let proposals = db.improvement_proposals(None).expect("proposals");
        assert_eq!(proposals.len(), 1);
    }

    #[test]
    fn update_and_delete_are_refused_by_the_storage_layer() {
        let (_dir, db) = temp_db();
        let entry = success_entry("sess-x");
        let id = entry.id().to_string();
        db.append(&entry).expect("append");

        let conn = db.lock_conn().expect("conn");
        let update = conn.execute(
            "UPDATE worklog SET what_done = 'rewritten history' WHERE id = ?1",
            params![id],
        );
        assert!(update.is_err(), "UPDATE must be refused by the trigger");
        assert!(update.unwrap_err().to_string().contains("append-only"));

        let delete = conn.execute("DELETE FROM worklog WHERE id = ?1", params![id]);
        assert!(delete.is_err(), "DELETE must be refused by the trigger");
        assert!(delete.unwrap_err().to_string().contains("append-only"));
        drop(conn);

        assert_eq!(db.count().expect("count"), 1, "the row survived intact");
        let rows = db.list(&WorklogQuery::default()).expect("list");
        assert_eq!(rows[0].what_done, "did the thing");
    }

    #[test]
    fn setup_is_idempotent_on_an_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("database.db");
        let first = WorklogDb::init(&base).expect("first init");
        first.append(&success_entry("sess-1")).expect("append");
        drop(first);

        // Re-open the same file: DDL re-applied, triggers re-checked.
        let second = WorklogDb::init(&base).expect("second init must not fail");
        assert_eq!(second.count().expect("count"), 1);
    }

    #[test]
    fn filters_narrow_by_employee_session_and_outcome() {
        let (_dir, db) = temp_db();
        let mut usage = crate::org::worklog::UsageAccumulator::new();
        usage.record(1, 1);
        db.append(
            &TurnObservation::from_turn_end(turn("sess-a", "alpha"))
                .with_employee("emp-1")
                .into_entry(),
        )
        .expect("append a");
        let failed = TurnObservation::from_turn_end(turn("sess-b", "beta"))
            .with_employee("emp-2")
            .with_usage(usage)
            .with_signals(OutcomeSignals {
                turn_errored: true,
                ..Default::default()
            })
            .into_entry();
        db.append(&failed).expect("append b");

        assert_eq!(db.count().expect("count"), 2);
        let by_emp = db
            .list(&WorklogQuery {
                employee: Some("emp-1".to_string()),
                ..Default::default()
            })
            .expect("by employee");
        assert_eq!(by_emp.len(), 1);
        assert_eq!(by_emp[0].session_id, "sess-a");

        let by_session = db
            .list(&WorklogQuery {
                session_id: Some("sess-b".to_string()),
                ..Default::default()
            })
            .expect("by session");
        assert_eq!(by_session[0].outcome, "failure");

        let by_outcome = db
            .list(&WorklogQuery {
                outcome: Some(Outcome::Failure),
                ..Default::default()
            })
            .expect("by outcome");
        assert_eq!(by_outcome.len(), 1);
        assert_eq!(by_outcome[0].session_id, "sess-b");

        let capped = db
            .list(&WorklogQuery {
                limit: Some(1),
                ..Default::default()
            })
            .expect("capped");
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn unknown_employee_is_the_honest_default_and_is_findable() {
        let (_dir, db) = temp_db();
        db.append(&success_entry("sess-unknown")).expect("append");
        let unbackfilled = db.unbackfilled(10).expect("unbackfilled");
        assert_eq!(unbackfilled.len(), 1);
        assert_eq!(unbackfilled[0].employee, UNKNOWN_EMPLOYEE);
    }

    #[test]
    fn get_by_id_returns_the_row_and_none_for_unknown() {
        let (_dir, db) = temp_db();
        let entry = success_entry("sess-g");
        let id = entry.id().to_string();
        db.append(&entry).expect("append");
        let found = db.get(&id).expect("get").expect("row present");
        assert_eq!(found.session_id, "sess-g");
        assert!(db.get("no-such-id").expect("get").is_none());
    }

    #[test]
    fn the_table_lands_in_the_sibling_kanban_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("nested").join("database.db");
        std::fs::create_dir_all(base.parent().expect("parent")).expect("mkdir");
        let db = WorklogDb::init(&base).expect("init");
        db.append(&success_entry("sess-path")).expect("append");

        let expected = base.parent().expect("parent").join(WORKLOG_DB_FILE);
        assert!(expected.exists(), "{} must exist", expected.display());
        assert_eq!(
            std::fs::read_dir(base.parent().expect("parent"))
                .expect("readdir")
                .filter_map(Result::ok)
                .count(),
            1,
            "no second database file: only the kanban sibling"
        );
    }

    #[test]
    fn both_append_only_triggers_are_created_with_matching_names() {
        assert_eq!(TRIGGERS.len(), 2);
        for (name, ddl) in TRIGGERS {
            assert!(
                ddl.contains(&format!("CREATE TRIGGER {name} ")),
                "{name} must be created under its own name"
            );
            assert!(ddl.contains("RAISE(ABORT"));
        }
        // The trigger names are part of the module's contract with anyone
        // who later adds a GC path and has to drop them deliberately.
        let names: Vec<&str> = TRIGGERS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["worklog_no_update", "worklog_no_delete"]);
    }

    #[test]
    fn schema_has_exactly_the_twenty_one_organism_columns_and_no_board_routing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = WorklogDb::init(&dir.path().join("database.db")).expect("init");
        let conn = db.lock_conn().expect("conn");
        let mut stmt = conn
            .prepare("PRAGMA table_info(worklog)")
            .expect("prepare pragma");
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query")
            .filter_map(Result::ok)
            .collect();

        let expected = [
            "id",
            "ts",
            "ts_iso",
            "employee",
            "department",
            "job_id",
            "iteration",
            "workflow_kind",
            "what_done",
            "outcome",
            "artifacts",
            "tokens_in",
            "tokens_out",
            "tool_calls",
            "duration_s",
            "next_intent",
            "blockers",
            "improvement_proposal",
            "correlation_id",
            "notice_ids",
            "session_id",
        ];
        assert_eq!(
            columns, expected,
            "column set and order must match the §3.4 DDL exactly (21 columns: \
             the organism's 20 WorklogEntry attributes plus ts_iso)"
        );
        // §3.4: "That constraint is preserved: there is no `to` column."
        assert!(!columns.iter().any(|c| c == "to"));
        assert!(!columns.iter().any(|c| c == "ack_required"));
    }
}
