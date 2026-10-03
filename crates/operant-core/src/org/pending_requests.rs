//! The `pending_requests` sqlite store — wave-2 slice F1 of
//! `docs/NEXT-IMPLEMENTATION-OUTLINE.md` §11 (wave-2 slice F) and
//! `docs/PERMISSION-SCOPING-PLAN.md` §8: the durable escalation queue.
//!
//! §8's lifecycle is committed here. A request is persisted at escalation
//! time (push-then-queue: a 3am cron escalation is never lost to a sleeping
//! approver, because the queue survives), a senior resolves it (`approved` |
//! `denied`), and unattended TTL expiry marks it `expired` — the row stays
//! for audit in every case. Nothing about the tool call executes at request
//! time; approval later mints a TTL'd grant, which is F2's job, not this
//! store's. This file persists the ask, its resolution, and who made both.
//!
//! Mirrors [`crate::org::authority::GrantDb`] and
//! [`crate::org::seat_policy_db::SeatPolicyDb`]: DDL as a `pub const` schema
//! string, idempotent [`PendingRequestDb::init`], `Arc<Mutex<Connection>>`
//! with poisoning treated as a recoverable error, and the same
//! table-not-a-file choice — `pending_requests` is a table inside the
//! kanban-family database the org modules already share, so there is no
//! second store to split-brain (see the authority module docs for the
//! `BUGS.md` R5-1 history).
//!
//! ## The one invariant this store owns
//!
//! Only a **pending** request may be resolved.
//! [`PendingRequestDb::resolve`] enforces that in the UPDATE's WHERE clause
//! — atomically, so a second connection to the same file cannot race a
//! resolution past the guard — and reports the failure loudly: re-resolving
//! an already-resolved request is an error, never a silent overwrite of the
//! first resolution, which is the one worth keeping. Routing, approval and
//! TTL sweeping are F2's; this transition guard is the store's.
//!
//! ## Status parsing is loud, never defaulting
//!
//! `status` is stored as [`Status::as_str`] and parsed back with
//! [`Status: FromStr`](std::str::FromStr). A row carrying a spelling this
//! build does not know is an **error on read**, not a silent fallback: a
//! typo'd status degrading to `pending` would re-queue a request a senior
//! already settled, which is the exact widening this module exists to make
//! impossible. Same discipline as `seat_policy_db`'s `SeatMode`.

use crate::error::Error;
use crate::org::notice::rfc3339;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

// =====================================================================
// Status
// =====================================================================

/// One request's lifecycle state.
///
/// `Expired` is not `Denied`: §8.3 — TTL expiry denies that attempt only,
/// and the row survives so a re-escalation can reference it ("still
/// pending since…") rather than duplicate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting on a senior. The only state [`PendingRequestDb::resolve`]
    /// accepts.
    Pending,
    /// A senior approved it; `grant_id` names the minted grant.
    Approved,
    /// A senior denied it.
    Denied,
    /// Nobody answered before the TTL lapsed.
    Expired,
}

impl Status {
    /// The stored spelling — one source of truth for every SQL comparison:
    /// the queries bind this, never a hand-typed `'pending'` that could
    /// drift from the enum.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
}

/// Unknown [`Status`] spelling — kept as its own error so a bad row names
/// itself in the audit trail instead of surfacing as a bare string (same
/// shape as [`crate::org::seat_policy::SeatModeParseError`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusParseError {
    /// The raw string the row carried.
    pub raw: String,
}

impl std::fmt::Display for StatusParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown pending-request status '{}' (expected pending | approved | denied | expired)",
            self.raw
        )
    }
}

impl std::error::Error for StatusParseError {}

impl FromStr for Status {
    type Err = StatusParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            "expired" => Ok(Self::Expired),
            _ => Err(StatusParseError { raw: s.to_string() }),
        }
    }
}

// =====================================================================
// Resolution + PendingRequest
// =====================================================================

/// What a senior (or the TTL sweeper) turns a pending request into.
///
/// Minting the grant on approval is F2's; [`Resolution::Approved`] only
/// records the `grant_id` F2 minted, so the audit view can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Approved. `grant_id` is the grant the approval minted.
    Approved { grant_id: String },
    /// Denied outright.
    Denied,
    /// The TTL lapsed before anybody answered (§8.3: the attempt is denied,
    /// the row survives for audit).
    Expired,
}

/// One `pending_requests` row — the request, and once resolved, its
/// resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// `'pr_' || uuid v4`.
    pub request_id: String,
    /// The **requesting** seat — the employee whose `decide()` escalated.
    pub employee_id: String,
    /// `decide()`'s `why` — the sentence that explains the ask. Required
    /// and non-blank (enforced in [`PendingRequestDb::enqueue`]): the
    /// approver judges the note, so an escalation without one is
    /// unjudgeable.
    pub requester_note: String,
    /// The tool the seat wants to run, e.g. `shell`.
    pub tool: String,
    /// RFC3339, via [`rfc3339`] so string comparison is chronological.
    pub requested_at: String,
    pub status: Status,
    /// The resolving seat id or `'operator'`; `None` while pending.
    pub resolved_by: Option<String>,
    /// RFC3339; `None` while pending.
    pub resolved_at: Option<String>,
    /// The minted grant, set on approval only.
    pub grant_id: Option<String>,
}

// =====================================================================
// PendingRequestDb
// =====================================================================

/// The `pending_requests` store.
///
/// One row per escalation ask. Dumb by design: routing, approval and TTL
/// sweeping are F2's — this store persists the ask, guards the
/// pending → resolved transition, and reads it back.
pub struct PendingRequestDb {
    conn: Arc<Mutex<Connection>>,
}

impl PendingRequestDb {
    /// The DDL, applied declaratively and idempotently.
    ///
    /// Same "never `crate::migrations::migrate`, never `PRAGMA user_version`"
    /// discipline as [`crate::org::authority::GrantDb::GRANTS_SCHEMA`] — the
    /// counter belongs to the kanban family that owns the file.
    pub const PENDING_REQUESTS_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS pending_requests (
            request_id     TEXT PRIMARY KEY,  -- 'pr_' || uuid v4
            employee_id    TEXT NOT NULL,     -- the REQUESTING seat
            requester_note TEXT NOT NULL,     -- decide()'s why: the ask, judgeable
            tool           TEXT NOT NULL,
            requested_at   TEXT NOT NULL,     -- RFC3339, fixed millisecond precision
            status         TEXT NOT NULL,     -- Status::as_str()
            resolved_by    TEXT,              -- seat id or 'operator'; NULL while pending
            resolved_at    TEXT,              -- RFC3339; NULL while pending
            grant_id       TEXT               -- the minted grant on approval; NULL otherwise
        );

        CREATE INDEX IF NOT EXISTS idx_pending_requests_status
            ON pending_requests(status);
        CREATE INDEX IF NOT EXISTS idx_pending_requests_employee
            ON pending_requests(employee_id);
    "#;

    /// Open (or create) the pending_requests table in a sqlite file.
    ///
    /// Tests point this at a temp path; production points it at the shared
    /// kanban-family file, or attaches to it via
    /// [`PendingRequestDb::from_shared_connection`].
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("pending requests: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("pending requests: open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Attach the store to a connection somebody else already owns.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.ensure_schema()?;
        Ok(db)
    }

    /// Borrow an already-open `Arc<Mutex<Connection>>` (the shape
    /// [`crate::org::notice_db::NoticeBoard::conn`] returns) so the queue
    /// shares that handle instead of opening a second writer.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.ensure_schema()?;
        Ok(db)
    }

    /// The underlying connection.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Lock the connection, turning poisoning into a recoverable error rather
    /// than a panic (same as `authority.rs` and `seat_policy_db.rs`).
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("pending requests db mutex poisoned".to_string()))
    }

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::PENDING_REQUESTS_SCHEMA)
            .map_err(|e| Error::Agent(format!("pending requests: schema: {e}")))
    }

    /// Persist one escalation ask. §8.1: the request is committed at
    /// escalation time, before anybody has seen it.
    ///
    /// Returns the fresh `request_id`. `requester_note` must be non-blank
    /// for the reason given on [`PendingRequest::requester_note`] — the
    /// rejection lives here at the store boundary, not at the caller's.
    pub fn enqueue(
        &self,
        employee_id: &str,
        tool: &str,
        requester_note: &str,
    ) -> Result<String, Error> {
        require_non_blank("requester note", requester_note)?;
        let request_id = format!("pr_{}", uuid::Uuid::new_v4());
        let requested_at = rfc3339(chrono::Utc::now());
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO pending_requests (
                request_id, employee_id, requester_note, tool, requested_at, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                request_id,
                employee_id,
                requester_note,
                tool,
                requested_at,
                Status::Pending.as_str(),
            ],
        )
        .map_err(|e| Error::Agent(format!("pending requests: enqueue {request_id}: {e}")))?;
        Ok(request_id)
    }

    /// The queue, oldest ask first — the `/pending` pull of §8.2.
    pub fn pending(&self) -> Result<Vec<PendingRequest>, Error> {
        self.select_requests("WHERE status = ?1", params![Status::Pending.as_str()])
    }

    /// Resolve one request: a senior's verdict, or the TTL sweeper's lapse.
    ///
    /// Returns `true` when a row was resolved, `false` when the id is
    /// unknown. A **known** id that is no longer pending is a loud error —
    /// the one invariant this store owns (see the module docs): the guard
    /// lives in the UPDATE's WHERE clause, so no second connection can race
    /// a resolution past it, and the failure names the current status.
    pub fn resolve(
        &self,
        request_id: &str,
        resolution: Resolution,
        resolved_by: &str,
    ) -> Result<bool, Error> {
        require_non_blank("resolved_by", resolved_by)?;
        let (status, grant_id) = match &resolution {
            Resolution::Approved { grant_id } => (Status::Approved.as_str(), Some(grant_id)),
            Resolution::Denied => (Status::Denied.as_str(), None),
            Resolution::Expired => (Status::Expired.as_str(), None),
        };
        let resolved_at = rfc3339(chrono::Utc::now());
        let conn = self.lock_conn()?;
        let updated = conn
            .execute(
                "UPDATE pending_requests \
                    SET status = ?2, resolved_by = ?3, resolved_at = ?4, grant_id = ?5 \
                  WHERE request_id = ?1 AND status = ?6",
                params![
                    request_id,
                    status,
                    resolved_by,
                    resolved_at,
                    grant_id,
                    Status::Pending.as_str(),
                ],
            )
            .map_err(|e| Error::Agent(format!("pending requests: resolve {request_id}: {e}")))?;
        if updated > 0 {
            return Ok(true);
        }
        // No row moved. Either the id is unknown (nothing to resolve, same
        // contract as `GrantDb::revoke`), or the request is already
        // resolved — which must be loud, never a silent overwrite.
        let current: Option<String> = conn
            .query_row(
                "SELECT status FROM pending_requests WHERE request_id = ?1",
                params![request_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                Error::Agent(format!(
                    "pending requests: resolve lookup {request_id}: {e}"
                ))
            })?;
        match current {
            None => Ok(false),
            Some(current) => Err(Error::Agent(format!(
                "pending requests: {request_id} is already {current}; \
                 only a pending request may be resolved"
            ))),
        }
    }

    /// Ids still `pending` whose `requested_at` is strictly before `now` —
    /// the TTL sweeper's input (F2 denies the attempt; it does not delete
    /// the row).
    ///
    /// RFC3339 string comparison is chronological, same as grants' expiry:
    /// [`rfc3339`] stamps fixed-millisecond UTC strings for exactly this.
    pub fn expired_as_of(&self, now: &str) -> Result<Vec<String>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT request_id FROM pending_requests \
                  WHERE status = ?1 AND requested_at < ?2 \
                  ORDER BY requested_at ASC, request_id ASC",
            )
            .map_err(|e| Error::Agent(format!("pending requests: expired prepare: {e}")))?;
        let rows = stmt
            .query_map(params![Status::Pending.as_str(), now], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| Error::Agent(format!("pending requests: expired query: {e}")))?;
        let mut out = Vec::new();
        for id in rows {
            out.push(id.map_err(|e| Error::Agent(format!("pending requests: expired row: {e}")))?);
        }
        Ok(out)
    }

    /// Every request one seat has made, resolved or not — the audit view.
    pub fn list_for_employee(&self, employee_id: &str) -> Result<Vec<PendingRequest>, Error> {
        self.select_requests("WHERE employee_id = ?1", params![employee_id])
    }

    /// Shared body of the two full-row reads: SELECT the columns in `col`
    /// order, map raw strings inside the rusqlite closure, then validate via
    /// [`columns_to_request`] outside it.
    fn select_requests(
        &self,
        filter: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<PendingRequest>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT request_id, employee_id, requester_note, tool, requested_at, \
                        status, resolved_by, resolved_at, grant_id \
                   FROM pending_requests {filter} \
                  ORDER BY requested_at ASC, request_id ASC"
            ))
            .map_err(|e| Error::Agent(format!("pending requests: select prepare: {e}")))?;
        let rows = stmt
            .query_map(params, Self::row_to_raw)
            .map_err(|e| Error::Agent(format!("pending requests: select query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let raw =
                row.map_err(|e| Error::Agent(format!("pending requests: select row: {e}")))?;
            out.push(columns_to_request(raw)?);
        }
        Ok(out)
    }

    /// Pull one row's raw columns, in [`col`] order. The loud `status`
    /// parse deliberately happens outside the closure, in
    /// [`columns_to_request`] — the `columns_to_policy` idiom.
    fn row_to_raw(row: &Row<'_>) -> rusqlite::Result<RawRow> {
        Ok((
            row.get(col::REQUEST_ID)?,
            row.get(col::EMPLOYEE_ID)?,
            row.get(col::REQUESTER_NOTE)?,
            row.get(col::TOOL)?,
            row.get(col::REQUESTED_AT)?,
            row.get(col::STATUS)?,
            row.get(col::RESOLVED_BY)?,
            row.get(col::RESOLVED_AT)?,
            row.get(col::GRANT_ID)?,
        ))
    }
}

/// Ordinals of the row columns, so the SELECT list and the mapper cannot
/// drift apart silently. Same defence as `notice_db`'s and `authority`'s
/// `mod col`.
mod col {
    pub(super) const REQUEST_ID: usize = 0;
    pub(super) const EMPLOYEE_ID: usize = 1;
    pub(super) const REQUESTER_NOTE: usize = 2;
    pub(super) const TOOL: usize = 3;
    pub(super) const REQUESTED_AT: usize = 4;
    pub(super) const STATUS: usize = 5;
    pub(super) const RESOLVED_BY: usize = 6;
    pub(super) const RESOLVED_AT: usize = 7;
    pub(super) const GRANT_ID: usize = 8;
}

/// One row's raw column values, in [`col`] order.
type RawRow = (
    String,         // request_id
    String,         // employee_id
    String,         // requester_note
    String,         // tool
    String,         // requested_at
    String,         // status — parsed loudly below
    Option<String>, // resolved_by
    Option<String>, // resolved_at
    Option<String>, // grant_id
);

/// Validate one raw row into a [`PendingRequest`].
///
/// A `status` spelling this build does not know is an error naming the
/// request and the raw string, never a silent default — a typo read as
/// `pending` would re-queue a request a senior already settled.
fn columns_to_request(raw: RawRow) -> Result<PendingRequest, Error> {
    let (
        request_id,
        employee_id,
        requester_note,
        tool,
        requested_at,
        status,
        resolved_by,
        resolved_at,
        grant_id,
    ) = raw;
    let status = Status::from_str(&status).map_err(|e| {
        Error::Agent(format!(
            "pending requests: {request_id}: stored row is unreadable: {e}"
        ))
    })?;
    Ok(PendingRequest {
        request_id,
        employee_id,
        requester_note,
        tool,
        requested_at,
        status,
        resolved_by,
        resolved_at,
        grant_id,
    })
}

/// Reject a blank attribution field at the store boundary — same defence as
/// `authority.rs`'s `require_reason`: an escalation without its `why` (or a
/// resolution without its resolver) is unattributable, and an
/// unattributable mutation is indistinguishable from a bug.
fn require_non_blank(field: &str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(Error::Agent(format!(
            "pending requests: {field} is required and must not be blank"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real temp-dir-backed store: every test opens the file `init`
    /// creates, so the DDL and parent-directory creation are exercised on
    /// every run (mirrors `authority.rs`'s init tests).
    fn db() -> (PendingRequestDb, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_pending_requests_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("operant_kanban.db");
        let db = PendingRequestDb::init(path).expect("init");
        (db, dir)
    }

    /// Seed a row with a literal `requested_at`, which `enqueue` (correctly)
    /// stamps with real now — the TTL boundary test needs controlled clocks,
    /// the same way `authority.rs`'s tests build grants with chosen
    /// `expires_at` literals.
    fn seed(db: &PendingRequestDb, id: &str, employee_id: &str, requested_at: &str) {
        db.conn()
            .lock()
            .expect("lock conn")
            .execute(
                "INSERT INTO pending_requests (
                    request_id, employee_id, requester_note, tool, requested_at, status
                 ) VALUES (?1, ?2, 'why: seeded for the TTL boundary', 'shell', ?3, ?4)",
                params![id, employee_id, requested_at, Status::Pending.as_str()],
            )
            .expect("seed");
    }

    // ------------------------------------------------------- enqueue/pending

    #[test]
    fn enqueue_then_pending_lists_it() {
        let (db, dir) = db();
        let id = db
            .enqueue("emp-1", "shell", "need a prod dump for the nightly debug")
            .expect("enqueue");
        assert!(id.starts_with("pr_"), "ids are 'pr_' || uuid v4, got {id}");
        let pending = db.pending().expect("pending");
        assert_eq!(pending.len(), 1, "the queue holds exactly the one ask");
        let r = &pending[0];
        assert_eq!(r.request_id, id);
        assert_eq!(r.employee_id, "emp-1");
        assert_eq!(r.tool, "shell");
        assert_eq!(
            r.requester_note, "need a prod dump for the nightly debug",
            "decide()'s why must travel with the ask"
        );
        assert_eq!(r.status, Status::Pending);
        assert_eq!(r.resolved_by, None);
        assert_eq!(r.resolved_at, None);
        assert_eq!(r.grant_id, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------ resolution

    #[test]
    fn approving_stamps_the_grant_and_the_resolver() {
        let (db, dir) = db();
        let id = db
            .enqueue("emp-1", "shell", "why: the nightly needs the dump")
            .expect("enqueue");
        assert!(
            db.resolve(
                &id,
                Resolution::Approved {
                    grant_id: "ag_1234".to_string(),
                },
                "emp-hod"
            )
            .expect("resolve"),
            "a pending request resolves"
        );
        assert!(
            db.pending().expect("pending").is_empty(),
            "approval empties the queue"
        );
        let row = &db.list_for_employee("emp-1").expect("list")[0];
        assert_eq!(row.status, Status::Approved);
        assert_eq!(row.resolved_by.as_deref(), Some("emp-hod"));
        assert_eq!(row.grant_id.as_deref(), Some("ag_1234"));
        let resolved_at = row.resolved_at.as_deref().expect("resolved_at stamped");
        chrono::DateTime::parse_from_rfc3339(resolved_at).expect("resolved_at is RFC3339");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolving_an_already_resolved_request_is_a_loud_error() {
        let (db, dir) = db();
        let id = db
            .enqueue("emp-1", "shell", "why: the first ask")
            .expect("enqueue");
        db.resolve(&id, Resolution::Denied, "emp-hod")
            .expect("first resolution");
        let err = db
            .resolve(
                &id,
                Resolution::Approved {
                    grant_id: "ag_overwrite".to_string(),
                },
                "emp-hod",
            )
            .expect_err("re-resolving must be an error, not a silent overwrite");
        assert!(
            err.to_string().contains("denied"),
            "the error must name the current status: {err}"
        );
        // The first resolution is the one that survives.
        let row = &db.list_for_employee("emp-1").expect("list")[0];
        assert_eq!(row.status, Status::Denied);
        assert_eq!(row.grant_id, None, "the failed overwrite must not stick");
        // An unknown id is not an error — there is just nothing to resolve.
        assert!(
            !db.resolve("pr_does_not_exist", Resolution::Denied, "emp-hod")
                .expect("resolve unknown"),
            "an unknown id must not be reported as a resolution"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn denied_and_expired_both_resolve_from_pending() {
        let (db, dir) = db();
        let denied_id = db
            .enqueue("emp-1", "shell", "why: ask one")
            .expect("enqueue");
        let expired_id = db
            .enqueue("emp-1", "jcode.write", "why: ask two")
            .expect("enqueue");
        assert!(
            db.resolve(&denied_id, Resolution::Denied, "emp-hod")
                .expect("deny")
        );
        assert!(
            db.resolve(&expired_id, Resolution::Expired, "operator")
                .expect("expire")
        );
        let rows = db.list_for_employee("emp-1").expect("list");
        assert_eq!(rows.len(), 2, "both rows survive for audit");
        let denied_row = rows
            .iter()
            .find(|r| r.request_id == denied_id)
            .expect("denied row");
        assert_eq!(denied_row.status, Status::Denied);
        assert_eq!(denied_row.grant_id, None);
        assert!(denied_row.resolved_at.is_some());
        let expired_row = rows
            .iter()
            .find(|r| r.request_id == expired_id)
            .expect("expired row");
        assert_eq!(expired_row.status, Status::Expired);
        assert_eq!(
            expired_row.grant_id, None,
            "expiry mints nothing — re-escalation is the path (§8.3)"
        );
        assert!(expired_row.resolved_at.is_some());
        assert!(db.pending().expect("pending").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -------------------------------------------------------------- TTL sweep

    #[test]
    fn expired_as_of_returns_only_lapsed_pending_ids() {
        let (db, dir) = db();
        seed(&db, "pr_old", "emp-1", "2026-01-01T00:00:00.000Z");
        seed(&db, "pr_new", "emp-1", "2026-06-01T00:00:00.000Z");
        // A request stamped LATER than `now` is not expired.
        assert_eq!(
            db.expired_as_of("2026-03-01T00:00:00.000Z")
                .expect("expired_as_of"),
            vec!["pr_old".to_string()],
            "only the earlier-than-now ask lapsed"
        );
        // The comparison is strict: at its own timestamp, nothing lapsed.
        assert!(
            db.expired_as_of("2026-01-01T00:00:00.000Z")
                .expect("expired_as_of")
                .is_empty()
        );
        // An approved request never expires, however old its stamp.
        assert!(
            db.resolve(
                "pr_old",
                Resolution::Approved {
                    grant_id: "ag_1".to_string(),
                },
                "emp-hod"
            )
            .expect("approve"),
        );
        assert!(
            db.expired_as_of("2026-03-01T00:00:00.000Z")
                .expect("expired_as_of")
                .is_empty(),
            "the approved row is out of the TTL's reach for good"
        );
        assert_eq!(
            db.expired_as_of("2026-07-01T00:00:00.000Z")
                .expect("expired_as_of"),
            vec!["pr_new".to_string()],
            "the still-pending later row lapses once now passes it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------- audit view

    #[test]
    fn list_for_employee_returns_only_that_seat() {
        let (db, dir) = db();
        db.enqueue("emp-1", "shell", "why: a").expect("enqueue");
        db.enqueue("emp-1", "jcode.write", "why: b")
            .expect("enqueue");
        db.enqueue("emp-2", "shell", "why: c").expect("enqueue");
        let rows = db.list_for_employee("emp-1").expect("list");
        assert_eq!(rows.len(), 2, "emp-2's ask must not leak into emp-1's view");
        assert!(rows.iter().all(|r| r.employee_id == "emp-1"));
        assert_eq!(
            db.list_for_employee("emp-404").expect("list").len(),
            0,
            "a seat with no asks has an empty audit view"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------- boundary guards

    #[test]
    fn a_blank_note_or_resolver_is_rejected_at_the_boundary() {
        let (db, dir) = db();
        assert!(
            db.enqueue("emp-1", "shell", "   ").is_err(),
            "a blank why cannot be judged by the approver"
        );
        let id = db
            .enqueue("emp-1", "shell", "why: a real note")
            .expect("enqueue");
        assert!(
            db.resolve(&id, Resolution::Denied, "  ").is_err(),
            "an unattributable resolution is indistinguishable from a bug"
        );
        assert_eq!(
            db.pending().expect("pending").len(),
            1,
            "the rejected resolve must not have written anything"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_status_string_errors_on_read_not_defaults() {
        let (db, dir) = db();
        seed(&db, "pr_corrupt", "emp-1", "2026-01-01T00:00:00.000Z");
        db.conn()
            .lock()
            .expect("lock conn")
            .execute(
                "UPDATE pending_requests SET status = 'settled' WHERE request_id = 'pr_corrupt'",
                [],
            )
            .expect("corrupt");
        // The corrupt row is invisible to the pending filter (it is not
        // 'pending'), but the full audit read must refuse to invent a state.
        let err = db
            .list_for_employee("emp-1")
            .expect_err("a status spelling this build does not know must not parse");
        assert!(
            err.to_string().contains("settled"),
            "the error must name the raw spelling: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
