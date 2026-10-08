//! The subjective log and the decision object — SQLite storage
//! (`docs/ORG-AUTHORITY-ARCHITECTURE.md` §8).
//!
//! Spec: `docs/ORG-AUTHORITY-ARCHITECTURE.md` §8.1 ("two things, deliberately
//! separate"), §8.2 (the two tables), §8.3 Q3 (dissent is first-class), §9
//! (persona synthesis), §12 (schema versioning).
//!
//! ## Why two tables and not one
//!
//! The owner's framing was "a subjective log which you are terming as decision
//! object", and §8.1 is the argument for keeping them apart rather than
//! merging them under one name:
//!
//! - [`SubjectiveEntry`] is the employee's own account of its internal
//!   working — what it weighed, what it rejected, how confident it is, what it
//!   would do differently. Free-form, cheap, frequent, per-employee. This is
//!   the **personality substrate** §9 folds into `employees.persona`.
//! - [`OrgDecision`] is a structured, attributable, *accepted* organisational
//!   decision: scope, decider, rationale, dissent, bindingness, expiry. Rare,
//!   authoritative, correlated across employees.
//!
//! Conflating them would lose information in one direction or the other: a
//! free-form log is cheap to write and expensive to query, a decision table is
//! the reverse. §8.1's exact reason for the split is that the subjective log
//! stays unstructured (the memory engine's strength) while decisions stay
//! queryable (the database's strength).
//!
//! ## Why a NEW database file, not the shared kanban file
//!
//! The sibling org tables (`employee`, `employees`, `worklog`, `notices`) all
//! live in `operant_kanban.db` — see
//! [`org_db_path`](crate::org::employee_db::org_db_path) and
//! [`worklog_db_path`](super::worklog_db::worklog_db_path). This module is the
//! first org table that does **not** follow that rule, and the reason is
//! mechanical rather than stylistic:
//!
//! `PRAGMA user_version` is **file-wide** in SQLite. `operant_kanban.db` sits
//! at version 1, claimed by the kanban family's single `MIGRATIONS` entry.
//! Putting a second migrating family in that file would move the version, and
//! the next time kanban opened the same file its one-entry family would
//! *look* downgraded — which is exactly the guard `migrations.rs` added after
//! R39-7, firing as "refusing to downgrade" and hard-failing kanban. That is
//! the R39-7 split-brain the invariant exists to prevent, and it is the
//! `REMAINING-GAPS.md` §GAP-4.5 collision §8.2 cites.
//!
//! So this is a **fresh file** (`operant_decisions.db`, via
//! [`decisions_db_path`]) with no prior version claim, which means there is
//! nothing to collide with. The cost is one more file next to `database.db`,
//! which §1.2's "sibling DB next to the domain DB" convention already
//! accepts (that is the same rule that produced `operant_cron.db` and
//! `operant_kanban.db`).
//!
//! ## Why still no `user_version` on a fresh file
//!
//! It would be *legal* to bump `user_version` on a file nothing else claims.
//! It is still the wrong move here, and §12 says so: "**New databases
//! (`operant_decisions.db`) — fresh files, no migration.** So: no
//! `PRAGMA user_version` use at all." The org layer's rule is stated once, for
//! the layer, and the file's freshness is not an occasion to reintroduce a
//! file-wide counter that the rest of the layer refuses to carry. Both tables
//! are applied declaratively with `CREATE TABLE IF NOT EXISTS`, which is
//! idempotent by construction and needs no version marker. See the module docs
//! in [`super::worklog_db`] for the same argument on the kanban side.
//!
//! ## Three invariants this module enforces at the store boundary
//!
//! 1. **Every mutation carries a non-blank `reason`.** `propose`, `accept`,
//!    and `reject` all reject a blank or whitespace-only reason instead of
//!    substituting a default. This is the same rule Wave 1 enforced on the
//!    notice board and the kanban: an audit trail that can record an empty
//!    justification is not an audit trail, and a default string is a lie about
//!    who decided what.
//! 2. **`rationale` is non-blank on propose.** §8.2 calls it out separately
//!    from `reason`: a decision without a reason for *itself* is not a
//!    decision, it is an instruction. `reason` records the *write* ("why this
//!    row exists now"), `rationale` records the *decision* ("why this is the
//!    right call"). Both are required.
//! 3. **Dissent is append-only.** [`DecisionsDb::record_dissent`] reads the
//!    existing JSON array and pushes onto it. It never removes, never
//!    overwrites, never replaces. §8.3 Q3: "A decision that suppresses
//!    dissent is not a decision; it is a suppression, and it is visible."
//!    There is deliberately no `clear_dissent` or `set_dissent` method, and
//!    the acceptance of a decision does not clear the objections to it.
//!
//! Status transitions are equally loud: accepting a decision that is not
//! [`DecisionStatus::Proposed`] is an error naming the current status, not a
//! silent no-op. A second `accept` that returns `Ok(())` would tell the caller
//! the org bound a decision when in fact it did nothing.
//!
//! ## What this module does not do
//!
//! - **No write barrier.** §7's fail-closed "append before reporting success"
//!   postcondition lives in the agent run path, not here.
//! - **No persona synthesis.** §9.2's `PersonaSynthesizer` is a later step;
//!   this table is its input, not its output.
//! - **No tool surface.** Nothing here is registered in `ToolRegistry`. A
//!   model cannot propose or accept a decision because there is no tool to
//!   call — see [`super::worklog`] for the enforcement argument.
//! - **No GC / retention.** §8.2 specifies `expires_at` and `review_due` but
//!   no retention rule. [`DecisionsDb::mark_expired`] is the explicit,
//!   caller-driven transition; nothing sweeps on its own.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::Error;

/// The sqlite file this module's two tables live in, derived from the main app
/// database path by the same sibling-DB rule
/// [`org_db_path`](crate::org::employee_db::org_db_path) and
/// [`worklog_db_path`](super::worklog_db::worklog_db_path) use — same
/// `parent()` fallback, same "one file per concept family" convention.
///
/// The name differs from the kanban sibling (`operant_kanban.db`) precisely
/// because this is a *different* family; see the module docs for the
/// `user_version` argument that forces the split.
pub fn decisions_db_path(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("operant_decisions.db")
}

/// The §8.2 DDL for both tables, plus the indexes `recent_for_employee` and
/// `list_by_status` need.
///
/// Applied declaratively and idempotently — see the module docs for why this
/// is not a `crate::migrations::migrate` entry even on a fresh file.
pub const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS subjective_log (
        entry_id             TEXT PRIMARY KEY,      -- 'sl_' || uuid v4
        employee_id          TEXT NOT NULL,         -- author; the persona substrate key
        ts                   INTEGER NOT NULL,      -- unix seconds; the ordering key
        ts_iso               TEXT NOT NULL,         -- RFC3339; the stored human form
        session_id           TEXT NOT NULL,         -- the run that produced this
        run_kind             TEXT NOT NULL,         -- cron|dm|ceo_meeting|manual
        reasoning_digest     TEXT NOT NULL,         -- what it weighed and rejected
        confidence           REAL,                  -- self-reported, NULL if unstated
        would_do_differently TEXT,                  -- the personality growth vector
        traits_delta         TEXT,                  -- JSON object: traits reinforced/shifted
        correlation_id       TEXT,                  -- joins to the board thread
        decision_ids         TEXT NOT NULL DEFAULT '[]'  -- decisions this entry produced
    );

    CREATE INDEX IF NOT EXISTS idx_subjective_employee ON subjective_log(employee_id, ts DESC);
    CREATE INDEX IF NOT EXISTS idx_subjective_session  ON subjective_log(session_id);

    CREATE TABLE IF NOT EXISTS org_decisions (
        decision_id    TEXT PRIMARY KEY,        -- 'd_' || uuid v4
        subject        TEXT NOT NULL,           -- one line
        scope          TEXT NOT NULL,           -- Self|Department|Descendants|Org
        target_dept    TEXT,                    -- NULL for org-wide
        decided_by     TEXT NOT NULL,           -- employee id
        rationale      TEXT NOT NULL,           -- required, non-blank (§8.2)
        amend_seat     TEXT,                    -- Slice 10: seat whose charter this amends; NULL = not an amendment
        amend_charter   TEXT,                    -- Slice 10: the full replacement charter text
        dissent        TEXT NOT NULL DEFAULT '[]',  -- JSON array of {employee_id, position, reason}
        binding        INTEGER NOT NULL DEFAULT 1,   -- bool, see §8.3 Q3
        status         TEXT NOT NULL DEFAULT 'proposed',  -- proposed|accepted|rejected|expired|superseded
        expires_at     TEXT,                    -- RFC3339; decisions must expire
        review_due     TEXT,                    -- RFC3339; dissent feeds this
        correlation_id TEXT,                    -- the meeting that produced it
        artifact_ids   TEXT NOT NULL DEFAULT '[]',  -- JSON array
        reason         TEXT NOT NULL,           -- the --reason of the proposing write
        created_at     TEXT NOT NULL,           -- RFC3339
        updated_at     TEXT NOT NULL            -- RFC3339
    );

    CREATE INDEX IF NOT EXISTS idx_decisions_status ON org_decisions(status);
    CREATE INDEX IF NOT EXISTS idx_decisions_by     ON org_decisions(decided_by);
"#;

/// Columns selected by every read, in [`OrgDecision`] field order.
/// `amend_seat`/`amend_charter` sit after `rationale`, mirroring the DDL.
const DECISION_COLUMNS: &str = "decision_id, subject, scope, target_dept, decided_by, \
     rationale, amend_seat, amend_charter, dissent, binding, status, expires_at, review_due, \
     correlation_id, artifact_ids, reason, created_at, updated_at";

/// Columns selected by every subjective read, in [`SubjectiveEntry`] field
/// order.
const ENTRY_COLUMNS: &str = "entry_id, employee_id, ts, ts_iso, session_id, run_kind, \
     reasoning_digest, confidence, would_do_differently, traits_delta, \
     correlation_id, decision_ids";

/// What kind of run produced a [`SubjectiveEntry`] (§8.2's `run_kind`).
///
/// The four members are the four places an employee can be made to think out
/// loud: a scheduled job, a direct message from a peer, a CEO alignment
/// meeting, and a human at a terminal. Nothing else writes here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// A cron-spawned run.
    Cron,
    /// A peer DM (§10).
    Dm,
    /// A CEO alignment meeting (§11).
    CeoMeeting,
    /// A human-driven run at a terminal or in the TUI.
    Manual,
}

impl RunKind {
    /// The wire form stored in `subjective_log.run_kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cron => "cron",
            Self::Dm => "dm",
            Self::CeoMeeting => "ceo_meeting",
            Self::Manual => "manual",
        }
    }

    /// Parse a stored `run_kind`. `None` for an unrecognized value — an unknown
    /// kind reads as unknown, never as a guess.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cron" => Some(Self::Cron),
            "dm" => Some(Self::Dm),
            "ceo_meeting" => Some(Self::CeoMeeting),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

impl Default for RunKind {
    /// `manual` is the honest default: a run nobody labelled. §8.2 gives the
    /// column no schema default, and inventing one (`cron`, the middle
    /// member, like `worklog.workflow_kind`) would silently attribute a
    /// human's turn to a scheduler. A caller that knows better sets it.
    fn default() -> Self {
        Self::Manual
    }
}

/// A decision's lifecycle state (§8.2's `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Written, not yet bound. The state [`DecisionsDb::propose`] leaves a row
    /// in.
    Proposed,
    /// Bound and in force. Only reachable through [`DecisionsDb::accept`].
    Accepted,
    /// Considered and turned down, with the reason recorded.
    Rejected,
    /// Past `expires_at`, retired by [`DecisionsDb::mark_expired`].
    Expired,
    /// Replaced by a later decision. No writer in this module sets it — §8.2
    /// specifies the state but not the transition — so it exists so a future
    /// supersession path reads the column rather than inventing a sixth value.
    Superseded,
}

impl DecisionStatus {
    /// The wire form stored in `org_decisions.status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
            Self::Superseded => "superseded",
        }
    }

    /// Parse a stored `status`. `None` for an unrecognized value.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "proposed" => Some(Self::Proposed),
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            "expired" => Some(Self::Expired),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// One dissent record against a decision (§8.2: `{employee_id, position,
/// reason}`).
///
/// `position` is the dissenter's stance — §8.3 Q3 only fixes that dissent is
/// recorded and non-blocking, not what the allowed positions are, so this
/// stores the string verbatim rather than inventing an enum that would have to
/// be widened the first time an employee invents a new position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DissentEntry {
    /// The employee whose objection this is.
    pub employee_id: String,
    /// Their stance, verbatim.
    pub position: String,
    /// Why. Required to be non-blank at write time — see
    /// [`DecisionsDb::record_dissent`].
    pub reason: String,
}

impl DissentEntry {
    /// Build a dissent record, rejecting a blank employee, position, or reason.
    ///
    /// Constructing here rather than only at the store keeps the guarantee
    /// true for every caller: there is no way to hold a `DissentEntry` that
    /// could not be written.
    pub fn new(
        employee_id: impl Into<String>,
        position: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, Error> {
        let employee_id = employee_id.into();
        if employee_id.trim().is_empty() {
            return Err(Error::Agent(
                "dissent requires a non-empty employee_id: an objection nobody holds \
                 is not dissent, it is noise"
                    .to_string(),
            ));
        }
        let position = position.into();
        if position.trim().is_empty() {
            return Err(Error::Agent(
                "dissent requires a non-empty position: §8.3 Q3 makes dissent a column, \
                 so the column has to say what the dissenter's stance actually was"
                    .to_string(),
            ));
        }
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(Error::Agent(
                "dissent requires a non-empty reason: an objection with no stated \
                 reason cannot be weighed at review_due, and unstated objections \
                 are how dissent gets quietly dropped"
                    .to_string(),
            ));
        }
        Ok(Self {
            employee_id,
            position,
            reason,
        })
    }
}

/// One row of the subjective log: an employee's own account of one run.
///
/// This is the **personality substrate** (§9). It is cheap and append-only on
/// purpose — §9.2's `PersonaSynthesizer` folds these entries into the
/// `employees.persona` JSON, and the honesty of the traits that come out of
/// that depends on the log being a record of what actually happened rather
/// than a curated summary of it.
///
/// Fields `None`-able are exactly the ones §8.2 leaves optional:
/// `confidence` (the employee may have no honest self-estimate), and the three
/// correlation fields.
#[derive(Debug, Clone, PartialEq)]
pub struct SubjectiveEntry {
    /// `'sl_' || uuid`. Minted by the caller — the write barrier runs at the
    /// end of `run()` and may want a stable id before the store is reached.
    pub entry_id: String,
    /// The authoring employee. This is the persona key.
    pub employee_id: String,
    /// Unix seconds — the ordering key.
    pub ts: i64,
    /// RFC3339 of the same instant, stored so readers do not convert per row.
    pub ts_iso: String,
    /// The run that produced this entry.
    pub session_id: String,
    /// What kind of run it was.
    pub run_kind: RunKind,
    /// What the employee weighed and what it rejected. The substance of the
    /// row; there is no "fine, moved on" default.
    pub reasoning_digest: String,
    /// Self-reported confidence in `[0, 1]`, if stated.
    pub confidence: Option<f64>,
    /// Free text — the personality growth vector. §8.2's own framing: this is
    /// the field that makes the log more than a transcript.
    pub would_do_differently: Option<String>,
    /// JSON object of traits this run reinforced or shifted.
    pub traits_delta: Option<String>,
    /// Joins to the notice-board thread this ran inside.
    pub correlation_id: Option<String>,
    /// Decisions this entry produced, as a JSON array.
    pub decision_ids: Vec<String>,
}

impl SubjectiveEntry {
    /// A minimally-populated entry for `employee_id`/`session_id`, with the
    /// timestamps stamped to now and everything optional empty.
    ///
    /// The caller is expected to set [`reasoning_digest`](Self::reasoning_digest)
    /// before writing: it is the one field with no honest default, and the
    /// store rejects a blank one.
    pub fn new(
        entry_id: impl Into<String>,
        employee_id: impl Into<String>,
        session_id: impl Into<String>,
        run_kind: RunKind,
        reasoning_digest: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            entry_id: entry_id.into(),
            employee_id: employee_id.into(),
            ts: now.timestamp(),
            ts_iso: crate::org::notice::rfc3339(now),
            session_id: session_id.into(),
            run_kind,
            reasoning_digest: reasoning_digest.into(),
            confidence: None,
            would_do_differently: None,
            traits_delta: None,
            correlation_id: None,
            decision_ids: Vec::new(),
        }
    }
}

/// One attributable, accepted organisational decision (§8.2).
///
/// `reason`, `created_at`, and `updated_at` are the standard mutation fields:
/// `reason` records why *this write* happened, `rationale` records why *the
/// decision* is right. They are different questions and both are required.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrgDecision {
    /// `'d_' || uuid`.
    pub decision_id: String,
    /// One line describing what was decided.
    pub subject: String,
    /// `Self` | `Department` | `Descendants` | `Org`.
    pub scope: String,
    /// The department this binds, or `None` for org-wide.
    pub target_dept: Option<String>,
    /// The deciding employee. §8.3 Q2: the CEO is an ordinary staff row, so
    /// this is an employee id with no special meaning.
    pub decided_by: String,
    /// Why the decision is right. Required, non-blank.
    pub rationale: String,
    /// Recorded objections, in the order they were made. Append-only.
    pub dissent: Vec<DissentEntry>,
    /// Whether the decision binds its descendants (§8.3 Q3's default is
    /// `true`; an organisation that cannot bind a decision has a suggestion
    /// loop, not an alignment loop).
    pub binding: bool,
    /// Lifecycle state.
    pub status: DecisionStatus,
    /// RFC3339; decisions must expire or be reviewed.
    pub expires_at: Option<String>,
    /// RFC3339; the review dissent feeds.
    pub review_due: Option<String>,
    /// The meeting or thread that produced it.
    pub correlation_id: Option<String>,
    /// **Slice 10 (identitarian evolution):** the seat whose charter this
    /// decision amends, if it is a charter-amendment decision. `None` on an
    /// ordinary decision. The amendment is applied **at accept time** by the
    /// CLI accept path — a proposal mutates nothing.
    pub amend_seat: Option<String>,
    /// The full replacement charter text. `Some` iff [`Self::amend_seat`] is
    /// `Some`; the pair is validated as both-or-neither at the CLI seam.
    pub amend_charter: Option<String>,
    /// Artifacts the decision rests on.
    pub artifact_ids: Vec<String>,
    /// The reason for the write that created this row. Required, non-blank.
    pub reason: String,
    /// RFC3339 of the proposing write.
    pub created_at: String,
    /// RFC3339 of the last mutation.
    pub updated_at: String,
}

impl OrgDecision {
    /// A minimally-populated `proposed` decision with timestamps stamped to
    /// now, `binding = true`, and no dissent.
    ///
    /// `subject`, `scope`, `decided_by`, `rationale`, and `reason` are all
    /// required by the store; the caller sets them on the returned value.
    pub fn new(
        decision_id: impl Into<String>,
        subject: impl Into<String>,
        scope: impl Into<String>,
        decided_by: impl Into<String>,
        rationale: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let now = crate::org::notice::rfc3339(chrono::Utc::now());
        Self {
            decision_id: decision_id.into(),
            subject: subject.into(),
            scope: scope.into(),
            target_dept: None,
            decided_by: decided_by.into(),
            rationale: rationale.into(),
            dissent: Vec::new(),
            binding: true,
            status: DecisionStatus::Proposed,
            expires_at: None,
            review_due: None,
            correlation_id: None,
            amend_seat: None,
            amend_charter: None,
            artifact_ids: Vec::new(),
            reason: reason.into(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// **Slice 10:** mark this decision as a charter amendment for `seat`.
    ///
    /// The pair is stored, not applied — [`DecisionsDb::propose`] writes the
    /// row in `proposed` like any other, and the mutation of the registry
    /// happens at accept time in the CLI accept path (the only writer the
    /// authority predicate gates).
    pub fn with_amendment(mut self, seat: impl Into<String>, charter: impl Into<String>) -> Self {
        self.amend_seat = Some(seat.into());
        self.amend_charter = Some(charter.into());
        self
    }
}

/// The subjective log and decision store — one sqlite file, two tables.
pub struct DecisionsDb {
    conn: Arc<Mutex<Connection>>,
}

impl DecisionsDb {
    /// Open the store for the app, deriving the path from the main database
    /// exactly like [`NoticeBoard::for_app`](crate::org::notice_db::NoticeBoard::for_app)
    /// delegates to its own helper.
    pub fn for_app(database_path: &Path) -> Result<Self, Error> {
        Self::init(decisions_db_path(database_path))
    }

    /// Open (or create) the store at `path`.
    ///
    /// The parent directory is created first because this file is genuinely
    /// new: unlike the kanban sibling there is no other subsystem that has
    /// already created the directory, so a caller pointing at a fresh
    /// `~/.operant` would otherwise get a bare "unable to open database file"
    /// with no hint about the missing directory.
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("decisions db: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("decisions db: open {}: {e}", path.display())))?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.ensure_schema()?;
        Ok(db)
    }

    /// The underlying connection, for a caller that needs to share one handle.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Lock the connection, converting mutex poisoning into a recoverable error
    /// instead of panicking (same pattern as `worklog_db.rs` and `notice_db.rs`).
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("decisions db mutex poisoned".to_string()))
    }

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| Error::Agent(format!("decisions db: schema: {e}")))?;
        // Slice 10: `amend_seat`/`amend_charter` joined the DDL after the
        // first live deployments, so a file created by an older binary lacks
        // the pair. `CREATE TABLE IF NOT EXISTS` cannot add columns to an
        // existing table, so add them explicitly when absent. There is no
        // `user_version` on this file (see module docs), so the probe is a
        // `PRAGMA table_info` check — idempotent and cheap.
        for col in ["amend_seat", "amend_charter"] {
            let present = conn
                .prepare(&format!(
                    "SELECT COUNT(*) FROM pragma_table_info('org_decisions') WHERE name = '{col}'"
                ))
                .and_then(|mut stmt| stmt.query_row([], |r| r.get::<_, i64>(0)))
                .map(|n| n > 0)
                .map_err(|e| Error::Agent(format!("decisions db: column probe: {e}")))?;
            if !present {
                conn.execute(
                    &format!("ALTER TABLE org_decisions ADD COLUMN {col} TEXT"),
                    [],
                )
                .map_err(|e| Error::Agent(format!("decisions db: add {col}: {e}")))?;
            }
        }
        Ok(())
    }

    // -------------------------------------------------------- subjective_log

    /// Total rows in the subjective log. Diagnostics and test affordance.
    pub fn count(&self) -> Result<i64, Error> {
        let conn = self.lock_conn()?;
        conn.query_row("SELECT COUNT(*) FROM subjective_log", [], |r| r.get(0))
            .map_err(|e| Error::Agent(format!("decisions db: count: {e}")))
    }

    /// Append one subjective entry.
    ///
    /// Append-only at both layers, like the worklog: there is no `update` and
    /// no `delete` in this module's API, and a rewrite of history is never a
    /// repair — it is a falsification of the persona substrate.
    ///
    /// A blank `reasoning_digest` is rejected: the whole value of the table is
    /// that the row says what the employee considered, and an empty digest is
    /// indistinguishable from a run that thought about nothing.
    pub fn append(&self, entry: &SubjectiveEntry) -> Result<(), Error> {
        if entry.reasoning_digest.trim().is_empty() {
            return Err(Error::Agent(
                "subjective entry requires a non-empty reasoning_digest: §8.1 makes this \
                 table the record of what the employee weighed and rejected, and an \
                 empty digest records neither"
                    .to_string(),
            ));
        }
        if entry.employee_id.trim().is_empty() {
            return Err(Error::Agent(
                "subjective entry requires a non-empty employee_id: §9.2 folds these rows \
                 into that employee's persona, and an unowned entry has no persona to \
                 land in"
                    .to_string(),
            ));
        }
        let decision_ids = serde_json::to_string(&entry.decision_ids)
            .map_err(|e| Error::Agent(format!("decisions db: decision_ids encode: {e}")))?;

        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO subjective_log (
                 entry_id, employee_id, ts, ts_iso, session_id, run_kind,
                 reasoning_digest, confidence, would_do_differently, traits_delta,
                 correlation_id, decision_ids
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                entry.entry_id,
                entry.employee_id,
                entry.ts,
                entry.ts_iso,
                entry.session_id,
                entry.run_kind.as_str(),
                entry.reasoning_digest,
                entry.confidence,
                entry.would_do_differently,
                entry.traits_delta,
                entry.correlation_id,
                decision_ids,
            ],
        )
        .map_err(|e| Error::Agent(format!("decisions db: append: {e}")))?;
        Ok(())
    }

    /// Newest-first page of one employee's subjective entries.
    ///
    /// `ORDER BY ts DESC, entry_id DESC` so a same-second page is still
    /// stable: two entries written in the same unix second have no defined
    /// order otherwise and would swap between calls.
    ///
    /// This is the §9.2 synthesizer's read. It is bounded by `limit` because
    /// persona synthesis wants recent behaviour, not the whole history — §9.3
    /// makes the same point about the objective log, which is never injected
    /// verbatim.
    pub fn recent_for_employee(
        &self,
        employee_id: &str,
        limit: usize,
    ) -> Result<Vec<SubjectiveEntry>, Error> {
        let conn = self.lock_conn()?;
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM subjective_log \
                          WHERE employee_id = ?1 ORDER BY ts DESC, entry_id DESC LIMIT ?2"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Agent(format!("decisions db: recent prepare: {e}")))?;
        let rows = stmt
            .query_map(params![employee_id, limit as i64], row_to_entry)
            .map_err(|e| Error::Agent(format!("decisions db: recent query: {e}")))?;
        collect_entries(rows)
    }

    // ---------------------------------------------------------- org_decisions

    /// Propose a decision. The row is written in [`DecisionStatus::Proposed`].
    ///
    /// Rejects a blank `reason`, a blank `rationale`, and a blank `subject`
    /// (a decision with no subject line cannot be listed, reviewed, or
    /// expired — §8.2's `subject` is "one line", which presumes it exists).
    /// `decided_by` is checked for the same reason: §8.2 calls the object
    /// *attributable*, and an unattributable decision is not one.
    pub fn propose(&self, decision: &OrgDecision) -> Result<(), Error> {
        require_text(
            &decision.subject,
            "subject",
            "a decision with no subject cannot be listed, reviewed, or expired",
        )?;
        require_text(
            &decision.decided_by,
            "decided_by",
            "§8.2 makes the decision object attributable; an unattributable decision is not a decision",
        )?;
        require_text(
            &decision.rationale,
            "rationale",
            "a decision without a stated reason for itself is an instruction, not a decision",
        )?;
        require_text(
            &decision.reason,
            "reason",
            "every mutation records why it happened; a default reason is a lie about who decided what",
        )?;

        let dissent = serde_json::to_string(&decision.dissent)
            .map_err(|e| Error::Agent(format!("decisions db: dissent encode: {e}")))?;
        let artifact_ids = serde_json::to_string(&decision.artifact_ids)
            .map_err(|e| Error::Agent(format!("decisions db: artifact_ids encode: {e}")))?;

        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO org_decisions (
                 decision_id, subject, scope, target_dept, decided_by, rationale,
                 amend_seat, amend_charter, dissent, binding, status, expires_at, review_due,
                 correlation_id, artifact_ids, reason, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                decision.decision_id,
                decision.subject,
                decision.scope,
                decision.target_dept,
                decision.decided_by,
                decision.rationale,
                decision.amend_seat,
                decision.amend_charter,
                dissent,
                decision.binding,
                decision.status.as_str(),
                decision.expires_at,
                decision.review_due,
                decision.correlation_id,
                artifact_ids,
                decision.reason,
                decision.created_at,
                decision.updated_at,
            ],
        )
        .map_err(|e| Error::Agent(format!("decisions db: propose: {e}")))?;
        Ok(())
    }

    /// One decision by id. `None` when the id is unknown.
    pub fn get(&self, decision_id: &str) -> Result<Option<OrgDecision>, Error> {
        let conn = self.lock_conn()?;
        let sql = format!("SELECT {DECISION_COLUMNS} FROM org_decisions WHERE decision_id = ?1");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Agent(format!("decisions db: get prepare: {e}")))?;
        let row = stmt
            .query_row(params![decision_id], row_to_decision)
            .optional()
            .map_err(|e| Error::Agent(format!("decisions db: get: {e}")))?;
        Ok(row)
    }

    /// Every decision in `status`, oldest first, then by id so the order is
    /// stable within a timestamp.
    pub fn list_by_status(&self, status: DecisionStatus) -> Result<Vec<OrgDecision>, Error> {
        let conn = self.lock_conn()?;
        let sql = format!(
            "SELECT {DECISION_COLUMNS} FROM org_decisions WHERE status = ?1 \
             ORDER BY created_at ASC, decision_id ASC"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Agent(format!("decisions db: list prepare: {e}")))?;
        let rows = stmt
            .query_map(params![status.as_str()], row_to_decision)
            .map_err(|e| Error::Agent(format!("decisions db: list query: {e}")))?;
        collect_decisions(rows)
    }

    /// Accept a proposed decision: `status` becomes
    /// [`DecisionStatus::Accepted`] and `reason` is replaced by the reason this
    /// acceptance happened.
    ///
    /// Errs if the decision is unknown or is not `proposed` — the error names
    /// the current status so a caller that double-accepts sees *why* it did
    /// nothing instead of receiving a cheerful `Ok(())` it would read as "the
    /// org bound this".
    ///
    /// Dissent is deliberately left in place: §8.3 Q3 makes it non-blocking
    /// but visible, and acceptance is not a vote that erases the objections.
    pub fn accept(&self, decision_id: &str, reason: &str) -> Result<(), Error> {
        require_mutation_reason(reason)?;
        self.transition(
            decision_id,
            DecisionStatus::Proposed,
            DecisionStatus::Accepted,
            reason,
        )
    }

    /// Reject a proposed decision: `status` becomes
    /// [`DecisionStatus::Rejected`].
    ///
    /// Same transition guard as [`Self::accept`], and dissent is likewise
    /// preserved — a rejected decision's objections are part of its record.
    pub fn reject(&self, decision_id: &str, reason: &str) -> Result<(), Error> {
        require_mutation_reason(reason)?;
        self.transition(
            decision_id,
            DecisionStatus::Proposed,
            DecisionStatus::Rejected,
            reason,
        )
    }

    /// The single `proposed -> {accepted, rejected}` write, shared so both
    /// transitions enforce the identical guard. The read and the write happen
    /// under one lock, so two concurrent accepts cannot both observe
    /// `proposed`.
    fn transition(
        &self,
        decision_id: &str,
        from: DecisionStatus,
        to: DecisionStatus,
        reason: &str,
    ) -> Result<(), Error> {
        let now = crate::org::notice::rfc3339(chrono::Utc::now());
        let conn = self.lock_conn()?;
        let current: Option<String> = conn
            .query_row(
                "SELECT status FROM org_decisions WHERE decision_id = ?1",
                params![decision_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| Error::Agent(format!("decisions db: transition read: {e}")))?;

        let current = current.ok_or_else(|| {
            Error::Agent(format!("decisions db: no decision with id {decision_id:?}"))
        })?;
        if current != from.as_str() {
            return Err(Error::Agent(format!(
                "decisions db: {decision_id} is {current}, not {} — cannot move it to {}",
                from.as_str(),
                to.as_str()
            )));
        }

        conn.execute(
            "UPDATE org_decisions SET status = ?2, reason = ?3, updated_at = ?4 \
             WHERE decision_id = ?1",
            params![decision_id, to.as_str(), reason, now],
        )
        .map_err(|e| Error::Agent(format!("decisions db: transition: {e}")))?;
        Ok(())
    }

    /// Record one objection against a decision. **Append-only**: the existing
    /// array is read, the new entry is pushed onto the end, and the whole
    /// array is written back. No entry is ever removed or reordered.
    ///
    /// This is §8.3 Q3 made structural — "a decision that suppresses dissent
    /// is not a decision; it is a suppression, and it is visible" — so the
    /// only way to lose a dissent record is to stop writing the API that
    /// records one.
    ///
    /// Works against any status, including `accepted` and `rejected`. Dissent
    /// that arrives late is still dissent, and the point of `review_due` is
    /// that a bound decision can still be contested.
    pub fn record_dissent(&self, decision_id: &str, entry: DissentEntry) -> Result<(), Error> {
        let now = crate::org::notice::rfc3339(chrono::Utc::now());
        let conn = self.lock_conn()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT dissent FROM org_decisions WHERE decision_id = ?1",
                params![decision_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| Error::Agent(format!("decisions db: dissent read: {e}")))?;
        let existing = existing.ok_or_else(|| {
            Error::Agent(format!("decisions db: no decision with id {decision_id:?}"))
        })?;

        // A corrupt or hand-edited array is replaced by a one-element array
        // rather than silently swallowing the new objection — losing a dissent
        // record is worse than losing a malformed one.
        let mut items: Vec<DissentEntry> = serde_json::from_str(&existing).unwrap_or_default();
        items.push(entry);
        let encoded = serde_json::to_string(&items)
            .map_err(|e| Error::Agent(format!("decisions db: dissent encode: {e}")))?;

        conn.execute(
            "UPDATE org_decisions SET dissent = ?2, updated_at = ?3 WHERE decision_id = ?1",
            params![decision_id, encoded, now],
        )
        .map_err(|e| Error::Agent(format!("decisions db: dissent write: {e}")))?;
        Ok(())
    }

    /// Retire every `accepted` decision whose `expires_at` is at or before
    /// `now_iso`. Returns how many rows moved.
    ///
    /// The comparison is a string compare, which is sound because
    /// [`rfc3339`](crate::org::notice::rfc3339) emits fixed-millisecond
    /// precision with a `Z` suffix precisely so lexicographic order equals
    /// chronological order — the property that function's own doc claims, and
    /// the reason `mark_expired` does not pay for a `datetime()` call in SQL
    /// on a sweep. **`now_iso` must therefore be in that same form** (the
    /// output of [`rfc3339`](crate::org::notice::rfc3339), e.g.
    /// `2026-10-01T00:00:00.000Z`); an offset form like `+00:00` sorts
    /// lexicographically after every `Z` instant of the same day and would
    /// expire the whole day's decisions. That is a caller's contract, not
    /// something guessed at here — the store has no way to know which form it
    /// was handed.
    ///
    /// Only `accepted` rows are swept. A `proposed` decision past its expiry
    /// was never bound, and a `rejected`/`superseded` one is already out of
    /// force — marking those would rewrite history to say something the status
    /// did not already say.
    pub fn mark_expired(&self, now_iso: &str) -> Result<usize, Error> {
        let now = crate::org::notice::rfc3339(chrono::Utc::now());
        let conn = self.lock_conn()?;
        let changed = conn
            .execute(
                "UPDATE org_decisions SET status = ?1, updated_at = ?2 \
                 WHERE status = ?3 AND expires_at IS NOT NULL AND expires_at <= ?4",
                params![
                    DecisionStatus::Expired.as_str(),
                    now,
                    DecisionStatus::Accepted.as_str(),
                    now_iso,
                ],
            )
            .map_err(|e| Error::Agent(format!("decisions db: mark_expired: {e}")))?;
        Ok(changed)
    }
}

/// Reject a blank/whitespace-only `reason` on any mutation.
///
/// This is the org layer's law, not a style preference: the same guard
/// `employee_db.rs::backfill_from_cron_jobs` applies to a reason that claims
/// to create employee rows. It lives at the store boundary, not in the caller,
/// because a caller that forgets is exactly the case the rule exists for.
fn require_mutation_reason(reason: &str) -> Result<(), Error> {
    // iter-637: the guard lives in org::require_non_blank; this keeps the
    // decisions-db audit-trail message verbatim.
    super::require_non_blank(reason, || {
        "every decision mutation requires a non-empty reason: an audit trail that \
         accepts a blank justification is not an audit trail, and a default reason is \
         a lie about who decided what"
            .to_string()
    })
}

/// Reject a blank/whitespace-only required column, with the caller supplying the
/// one-line justification of *why* that column cannot be empty.
fn require_text(value: &str, field: &str, why: &str) -> Result<(), Error> {
    // iter-637: the guard lives in org::require_non_blank; this keeps the
    // decisions-db field message verbatim.
    super::require_non_blank(value, || {
        format!("decision requires a non-empty {field}: {why}")
    })
}

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<SubjectiveEntry> {
    let decision_ids: String = row.get(11)?;
    Ok(SubjectiveEntry {
        entry_id: row.get(0)?,
        employee_id: row.get(1)?,
        ts: row.get(2)?,
        ts_iso: row.get(3)?,
        session_id: row.get(4)?,
        run_kind: RunKind::parse(&row.get::<_, String>(5)?).unwrap_or_default(),
        reasoning_digest: row.get(6)?,
        confidence: row.get(7)?,
        would_do_differently: row.get(8)?,
        traits_delta: row.get(9)?,
        correlation_id: row.get(10)?,
        // A row written by an older version (or a hand-edited one) must not
        // make the read fail: an unreadable array reads as empty, and the
        // alternative is one bad row poisoning the persona synthesis.
        decision_ids: serde_json::from_str(&decision_ids).unwrap_or_default(),
    })
}

fn row_to_decision(row: &Row<'_>) -> rusqlite::Result<OrgDecision> {
    let dissent: String = row.get(8)?;
    let artifact_ids: String = row.get(14)?;
    Ok(OrgDecision {
        decision_id: row.get(0)?,
        subject: row.get(1)?,
        scope: row.get(2)?,
        target_dept: row.get(3)?,
        decided_by: row.get(4)?,
        rationale: row.get(5)?,
        amend_seat: row.get(6)?,
        amend_charter: row.get(7)?,
        dissent: serde_json::from_str(&dissent).unwrap_or_default(),
        binding: row.get::<_, i64>(9)? != 0,
        status: DecisionStatus::parse(&row.get::<_, String>(10)?)
            .unwrap_or(DecisionStatus::Proposed),
        expires_at: row.get(11)?,
        review_due: row.get(12)?,
        correlation_id: row.get(13)?,
        artifact_ids: serde_json::from_str(&artifact_ids).unwrap_or_default(),
        reason: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

/// Drain a `query_map` result set into a `Vec`, converting each row error into
/// the org layer's error type.
///
/// Taking the iterator directly (rather than `rusqlite::Result<impl …>`) is
/// what `query_map` hands back: the *prepare/bind* failure is the `Result` the
/// caller has already handled with `?`, and what is left is an iterator whose
/// items are the fallible rows.
fn collect_entries(
    rows: impl Iterator<Item = rusqlite::Result<SubjectiveEntry>>,
) -> Result<Vec<SubjectiveEntry>, Error> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| Error::Agent(format!("decisions db: entry read: {e}")))?);
    }
    Ok(out)
}

fn collect_decisions(
    rows: impl Iterator<Item = rusqlite::Result<OrgDecision>>,
) -> Result<Vec<OrgDecision>, Error> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| Error::Agent(format!("decisions db: decision read: {e}")))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, DecisionsDb) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = DecisionsDb::init(dir.path().join("operant_decisions.db")).expect("init");
        (dir, db)
    }

    fn decision(subject: &str) -> OrgDecision {
        OrgDecision::new(
            "d-1",
            subject,
            "Org",
            "emp-ceo",
            "the platform team cannot ship both stacks this quarter",
            "recorded from the 2026-09-30 alignment meeting",
        )
    }

    #[test]
    fn amendment_fields_round_trip() {
        let (_dir, db) = temp_db();
        let d = decision("amend the dispatcher charter").with_amendment(
            "emp-dispatcher",
            "You are dispatcher — now with routing oversight.",
        );
        db.propose(&d).expect("propose");
        let got = db.get(&d.decision_id).expect("get").expect("row");
        assert_eq!(got.amend_seat.as_deref(), Some("emp-dispatcher"));
        assert_eq!(
            got.amend_charter.as_deref(),
            Some("You are dispatcher — now with routing oversight.")
        );
    }

    #[test]
    fn a_decision_without_amendment_reads_none_none() {
        let (_dir, db) = temp_db();
        let d = decision("ordinary decision");
        assert_eq!(d.amend_seat, None);
        assert_eq!(d.amend_charter, None);
        db.propose(&d).expect("propose");
        let got = db.get(&d.decision_id).expect("get").expect("row");
        assert_eq!(got.amend_seat, None);
        assert_eq!(got.amend_charter, None);
    }

    #[test]
    fn legacy_file_gains_amendment_columns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant_decisions.db");
        {
            // A pre-Slice-10 file: the org_decisions table without the
            // amend_seat/amend_charter pair, holding one legacy row.
            let conn = Connection::open(&path).expect("open legacy");
            conn.execute_batch(
                "CREATE TABLE org_decisions (
                     decision_id TEXT PRIMARY KEY, subject TEXT NOT NULL,
                     scope TEXT NOT NULL, target_dept TEXT, decided_by TEXT NOT NULL,
                     rationale TEXT NOT NULL, dissent TEXT NOT NULL DEFAULT '[]',
                     binding INTEGER NOT NULL DEFAULT 1,
                     status TEXT NOT NULL DEFAULT 'proposed', expires_at TEXT,
                     review_due TEXT, correlation_id TEXT,
                     artifact_ids TEXT NOT NULL DEFAULT '[]', reason TEXT NOT NULL,
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                 )",
            )
            .expect("legacy schema");
            conn.execute(
                "INSERT INTO org_decisions (decision_id, subject, scope, decided_by, \
                 rationale, reason, created_at, updated_at) \
                 VALUES ('d-old', 'legacy', 'Org', 'user', 'r', 'r', 't', 't')",
                [],
            )
            .expect("legacy row");
        }
        // Opening through the store runs ensure_schema, which must ALTER the
        // legacy table into shape without touching its rows.
        let db = DecisionsDb::init(path).expect("open legacy file");
        let got = db.get("d-old").expect("get").expect("legacy row survives");
        assert_eq!(got.subject, "legacy");
        assert_eq!(got.amend_seat, None, "legacy rows read as non-amendments");
        // And a fresh amendment write lands on the migrated table.
        let d = decision("post-migration amendment").with_amendment("emp-x", "c");
        db.propose(&d).expect("propose after migration");
        let got = db.get(&d.decision_id).expect("get").expect("row");
        assert_eq!(got.amend_seat.as_deref(), Some("emp-x"));
    }

    fn entry(employee: &str, ts: i64) -> SubjectiveEntry {
        SubjectiveEntry::new(
            format!("sl-{employee}-{ts}"),
            employee,
            "sess-1",
            RunKind::Cron,
            "weighed a rollback against a forward fix; chose forward",
        )
        .with_ts(ts)
    }

    impl SubjectiveEntry {
        /// Backdate an entry so ordering tests have distinct seconds without
        /// sleeping.
        fn with_ts(mut self, ts: i64) -> Self {
            self.ts = ts;
            self.ts_iso = format!("1970-01-01T00:{:02}:{:02}Z", ts / 60, ts % 60);
            self
        }
    }

    // ------------------------------------------------------------- path helper

    #[test]
    fn path_is_a_sibling_named_operant_decisions_db() {
        let base = Path::new("/home/u/.operant/database.db");
        let path = decisions_db_path(base);
        assert_eq!(path, Path::new("/home/u/.operant/operant_decisions.db"));
        assert_eq!(path.file_name().unwrap(), "operant_decisions.db");
        // Same directory as the source DB, not a nested one.
        assert_eq!(path.parent(), base.parent());
        // And it is NOT the kanban sibling — the whole point of the split.
        assert_ne!(path, crate::org::employee_db::org_db_path(base));
    }

    #[test]
    fn path_falls_back_to_cwd_when_the_source_has_no_parent() {
        let path = decisions_db_path(Path::new("database.db"));
        // `Path::join` normalises a bare `.` prefix away, so the result is
        // `./operant_decisions.db` and `operant_decisions.db` compared as
        // `Path`s are equal — the file lands in the *current* directory, which
        // is what the fallback is for. Asserting the literal string with a
        // `./` prefix would be asserting a `PathBuf` display detail, not the
        // behaviour.
        assert_eq!(path, Path::new("operant_decisions.db"));
        assert_eq!(
            path.parent(),
            Some(Path::new("")),
            "no parent dir in the source means the cwd, not a subdirectory"
        );
    }

    // ------------------------------------------------------ subjective_log

    #[test]
    fn subjective_entry_round_trips_every_column() {
        let (_dir, db) = temp_db();
        let mut e = SubjectiveEntry::new(
            "sl-1",
            "emp-1",
            "sess-a",
            RunKind::CeoMeeting,
            "considered shipping without staging",
        );
        e.ts = 1_700_000_000;
        e.ts_iso = "2026-01-02T03:04:05.000Z".to_string();
        e.confidence = Some(0.72);
        e.would_do_differently = Some("get staging access earlier".to_string());
        e.traits_delta = Some(r#"{"cautious": 0.1}"#.to_string());
        e.correlation_id = Some("corr-9".to_string());
        e.decision_ids = vec!["d-a".to_string(), "d-b".to_string()];
        db.append(&e).expect("append");

        let rows = db.recent_for_employee("emp-1", 10).expect("recent");
        assert_eq!(rows.len(), 1);
        let got = &rows[0];
        assert_eq!(got.entry_id, "sl-1");
        assert_eq!(got.employee_id, "emp-1");
        assert_eq!(got.ts, 1_700_000_000);
        assert_eq!(got.ts_iso, "2026-01-02T03:04:05.000Z");
        assert_eq!(got.session_id, "sess-a");
        assert_eq!(got.run_kind, RunKind::CeoMeeting);
        assert_eq!(got.reasoning_digest, "considered shipping without staging");
        assert!((got.confidence.expect("confidence") - 0.72).abs() < 1e-9);
        assert_eq!(
            got.would_do_differently.as_deref(),
            Some("get staging access earlier")
        );
        assert_eq!(got.traits_delta.as_deref(), Some(r#"{"cautious": 0.1}"#));
        assert_eq!(got.correlation_id.as_deref(), Some("corr-9"));
        assert_eq!(got.decision_ids, vec!["d-a".to_string(), "d-b".to_string()]);
        assert_eq!(db.count().expect("count"), 1);
    }

    #[test]
    fn recent_is_newest_first_scoped_to_the_employee_and_bounded_by_limit() {
        let (_dir, db) = temp_db();
        for ts in [10, 30, 20] {
            db.append(&entry("emp-a", ts)).expect("append a");
        }
        db.append(&entry("emp-b", 99)).expect("append b");

        let all = db.recent_for_employee("emp-a", 10).expect("recent");
        assert_eq!(
            all.iter().map(|e| e.ts).collect::<Vec<_>>(),
            vec![30, 20, 10],
            "newest first"
        );

        let capped = db.recent_for_employee("emp-a", 2).expect("capped");
        assert_eq!(capped.len(), 2);
        assert_eq!(
            capped.iter().map(|e| e.ts).collect::<Vec<_>>(),
            vec![30, 20]
        );

        let none = db.recent_for_employee("emp-unknown", 5).expect("unknown");
        assert!(none.is_empty());
    }

    #[test]
    fn a_blank_digest_or_owner_is_refused() {
        let (_dir, db) = temp_db();
        let mut blank_digest = entry("emp-a", 1);
        blank_digest.reasoning_digest = "   ".to_string();
        assert!(db.append(&blank_digest).is_err());

        let mut blank_owner = entry("emp-a", 1);
        blank_owner.employee_id = "  ".to_string();
        assert!(db.append(&blank_owner).is_err());

        assert_eq!(db.count().expect("count"), 0, "nothing was written");
    }

    // ---------------------------------------------------------- org_decisions

    #[test]
    fn propose_get_round_trips_every_column() {
        let (_dir, db) = temp_db();
        let mut d = decision("Retire the v1 ingest stack");
        d.target_dept = Some("platform".to_string());
        d.review_due = Some("2026-12-01T00:00:00.000Z".to_string());
        d.expires_at = Some("2027-01-01T00:00:00.000Z".to_string());
        d.correlation_id = Some("corr-1".to_string());
        d.artifact_ids = vec!["art-1".to_string()];
        d.binding = false;
        db.propose(&d).expect("propose");

        let got = db.get("d-1").expect("get").expect("row present");
        assert_eq!(got.decision_id, "d-1");
        assert_eq!(got.subject, "Retire the v1 ingest stack");
        assert_eq!(got.scope, "Org");
        assert_eq!(got.target_dept.as_deref(), Some("platform"));
        assert_eq!(got.decided_by, "emp-ceo");
        assert_eq!(
            got.rationale,
            "the platform team cannot ship both stacks this quarter"
        );
        assert!(got.dissent.is_empty());
        assert!(!got.binding);
        assert_eq!(got.status, DecisionStatus::Proposed);
        assert_eq!(got.expires_at.as_deref(), Some("2027-01-01T00:00:00.000Z"));
        assert_eq!(got.review_due.as_deref(), Some("2026-12-01T00:00:00.000Z"));
        assert_eq!(got.correlation_id.as_deref(), Some("corr-1"));
        assert_eq!(got.artifact_ids, vec!["art-1".to_string()]);
        assert_eq!(got.reason, "recorded from the 2026-09-30 alignment meeting");
        assert_eq!(got.created_at, d.created_at);
        assert_eq!(got.updated_at, d.updated_at);
        assert!(db.get("d-nope").expect("get unknown").is_none());
    }

    #[test]
    fn a_new_decision_defaults_to_proposed_and_binding() {
        let (_dir, db) = temp_db();
        db.propose(&decision("x")).expect("propose");
        let got = db.get("d-1").expect("get").expect("row");
        assert_eq!(got.status, DecisionStatus::Proposed);
        assert!(got.binding, "§8.3 Q3 defaults binding to true");
        let proposed = db
            .list_by_status(DecisionStatus::Proposed)
            .expect("list proposed");
        assert_eq!(proposed.len(), 1);
        assert!(
            db.list_by_status(DecisionStatus::Accepted)
                .expect("list accepted")
                .is_empty()
        );
    }

    #[test]
    fn a_blank_reason_or_rationale_is_refused_at_the_store() {
        let (_dir, db) = temp_db();

        let mut blank_reason = decision("x");
        blank_reason.reason = "   ".to_string();
        let err = db.propose(&blank_reason).expect_err("blank reason refused");
        assert!(err.to_string().contains("reason"), "got: {err}");

        let mut blank_rationale = decision("y");
        blank_rationale.rationale = String::new();
        let err = db
            .propose(&blank_rationale)
            .expect_err("blank rationale refused");
        assert!(err.to_string().contains("rationale"), "got: {err}");

        let mut blank_subject = decision("z");
        blank_subject.subject = " ".to_string();
        assert!(db.propose(&blank_subject).is_err());

        let mut blank_author = decision("w");
        blank_author.decided_by = String::new();
        assert!(db.propose(&blank_author).is_err());

        let proposed = db.list_by_status(DecisionStatus::Proposed).expect("list");
        assert!(proposed.is_empty(), "no partial row survived a refusal");
    }

    #[test]
    fn accept_moves_the_status_and_a_second_accept_names_the_status() {
        let (_dir, db) = temp_db();
        db.propose(&decision("x")).expect("propose");

        db.accept("d-1", "HODs aligned in the meeting")
            .expect("accept");
        let got = db.get("d-1").expect("get").expect("row");
        assert_eq!(got.status, DecisionStatus::Accepted);
        assert_eq!(got.reason, "HODs aligned in the meeting");
        assert!(
            db.list_by_status(DecisionStatus::Proposed)
                .expect("proposed")
                .is_empty()
        );

        // A second accept must not silently succeed: the caller would read Ok(())
        // as "the org bound this" when nothing happened.
        let err = db
            .accept("d-1", "trying again")
            .expect_err("re-accept refused");
        let msg = err.to_string();
        assert!(
            msg.contains("accepted"),
            "error names the current status: {msg}"
        );

        // ...and the row was not disturbed by the refused attempt.
        let after = db.get("d-1").expect("get").expect("row");
        assert_eq!(after.status, DecisionStatus::Accepted);
        assert_eq!(after.reason, "HODs aligned in the meeting");
    }

    #[test]
    fn accept_and_reject_need_a_non_blank_reason() {
        let (_dir, db) = temp_db();
        db.propose(&decision("x")).expect("propose");
        assert!(db.accept("d-1", "").is_err());
        assert!(db.accept("d-1", "   \t ").is_err());
        assert!(db.reject("d-1", "").is_err());
        assert_eq!(
            db.get("d-1").expect("get").expect("row").status,
            DecisionStatus::Proposed,
            "a refused transition leaves the row untouched"
        );
        db.reject("d-1", "the sequencing is unsafe")
            .expect("reject");
        assert_eq!(
            db.get("d-1").expect("get").expect("row").status,
            DecisionStatus::Rejected
        );
        assert!(db.reject("d-1", "and again").is_err(), "re-reject refused");
    }

    #[test]
    fn transitioning_an_unknown_decision_is_an_error_not_a_no_op() {
        let (_dir, db) = temp_db();
        assert!(db.accept("d-missing", "reason").is_err());
        assert!(db.reject("d-missing", "reason").is_err());
        assert!(
            db.record_dissent(
                "d-missing",
                DissentEntry::new("emp-1", "against", "reason").expect("entry")
            )
            .is_err()
        );
    }

    #[test]
    fn dissent_appends_and_never_removes_the_prior_entries() {
        let (_dir, db) = temp_db();
        db.propose(&decision("x")).expect("propose");

        db.record_dissent(
            "d-1",
            DissentEntry::new("emp-1", "against", "no staging access").expect("e1"),
        )
        .expect("first dissent");
        db.record_dissent(
            "d-1",
            DissentEntry::new("emp-2", "abstain", "blocked on a vendor").expect("e2"),
        )
        .expect("second dissent");

        let got = db.get("d-1").expect("get").expect("row");
        assert_eq!(got.dissent.len(), 2, "both entries survive");
        assert_eq!(got.dissent[0].employee_id, "emp-1");
        assert_eq!(got.dissent[0].position, "against");
        assert_eq!(got.dissent[0].reason, "no staging access");
        assert_eq!(got.dissent[1].employee_id, "emp-2");
        assert_eq!(got.dissent[1].position, "abstain");

        // Acceptance is not an erasure: the objections are part of the record.
        db.accept("d-1", "bound anyway, dissent is non-blocking")
            .expect("accept");
        let after = db.get("d-1").expect("get").expect("row");
        assert_eq!(after.status, DecisionStatus::Accepted);
        assert_eq!(after.dissent.len(), 2, "accept did not clear dissent");

        // Late dissent against a decided decision is still recorded.
        db.record_dissent(
            "d-1",
            DissentEntry::new("emp-3", "against", "new evidence").expect("e3"),
        )
        .expect("late dissent");
        assert_eq!(db.get("d-1").expect("get").expect("row").dissent.len(), 3);
    }

    #[test]
    fn a_dissent_entry_needs_an_author_a_position_and_a_reason() {
        assert!(DissentEntry::new("  ", "against", "why").is_err());
        assert!(DissentEntry::new("emp-1", " ", "why").is_err());
        assert!(DissentEntry::new("emp-1", "against", "  ").is_err());
        assert!(DissentEntry::new("emp-1", "against", "why").is_ok());
    }

    #[test]
    fn mark_expired_retires_only_accepted_decisions_that_are_past_expiry() {
        let (_dir, db) = temp_db();

        let mut stale = decision("stale");
        stale.decision_id = "d-stale".to_string();
        stale.expires_at = Some("2026-01-01T00:00:00.000Z".to_string());
        db.propose(&stale).expect("propose stale");
        db.accept("d-stale", "was bound").expect("accept stale");

        let mut fresh = decision("fresh");
        fresh.decision_id = "d-fresh".to_string();
        fresh.expires_at = Some("2099-01-01T00:00:00.000Z".to_string());
        db.propose(&fresh).expect("propose fresh");
        db.accept("d-fresh", "still binding").expect("accept fresh");

        let mut no_expiry = decision("no-expiry");
        no_expiry.decision_id = "d-no-expiry".to_string();
        db.propose(&no_expiry).expect("propose no-expiry");
        db.accept("d-no-expiry", "binds indefinitely")
            .expect("accept");

        let mut pending = decision("pending");
        pending.decision_id = "d-pending".to_string();
        pending.expires_at = Some("2026-01-01T00:00:00.000Z".to_string());
        db.propose(&pending).expect("propose pending");

        // `now_iso` in the form `rfc3339` actually emits — the store's
        // documented caller contract for the lexicographic compare.
        let swept = db
            .mark_expired("2026-10-01T00:00:00.000Z")
            .expect("mark_expired");
        assert_eq!(swept, 1);

        let status = |id: &str| db.get(id).expect("get").expect("row").status;
        assert_eq!(status("d-stale"), DecisionStatus::Expired);
        assert_eq!(status("d-fresh"), DecisionStatus::Accepted);
        assert_eq!(status("d-no-expiry"), DecisionStatus::Accepted);
        assert_eq!(
            status("d-pending"),
            DecisionStatus::Proposed,
            "a decision that was never bound is not 'expired'"
        );
        assert_eq!(
            db.list_by_status(DecisionStatus::Expired)
                .expect("list")
                .len(),
            1
        );
    }

    #[test]
    fn status_and_run_kind_wire_forms_round_trip() {
        for s in [
            DecisionStatus::Proposed,
            DecisionStatus::Accepted,
            DecisionStatus::Rejected,
            DecisionStatus::Expired,
            DecisionStatus::Superseded,
        ] {
            assert_eq!(DecisionStatus::parse(s.as_str()), Some(s));
        }
        for k in [
            RunKind::Cron,
            RunKind::Dm,
            RunKind::CeoMeeting,
            RunKind::Manual,
        ] {
            assert_eq!(RunKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(DecisionStatus::parse("wibble"), None);
        assert_eq!(RunKind::parse("wibble"), None);
        assert_eq!(DecisionStatus::Accepted.as_str(), "accepted");
        assert_eq!(RunKind::CeoMeeting.as_str(), "ceo_meeting");
    }

    #[test]
    fn init_is_idempotent_and_does_not_stamp_a_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("operant_decisions.db");
        let first = DecisionsDb::init(path.clone()).expect("first init creates the dir");
        first.propose(&decision("x")).expect("propose");
        drop(first);

        let second = DecisionsDb::init(path.clone()).expect("re-init must not fail");
        assert_eq!(second.get("d-1").expect("get").expect("row").subject, "x");

        // §12: a fresh database carries no migration, so this file must be
        // left at user_version 0. Reading it is the only proof; the schema is
        // applied purely with CREATE TABLE IF NOT EXISTS.
        let version: i64 = second
            .conn()
            .lock()
            .expect("lock")
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("user_version");
        assert_eq!(version, 0, "no migration family claimed this file");
    }

    #[test]
    fn the_two_tables_are_independent() {
        let (_dir, db) = temp_db();
        db.append(&entry("emp-1", 5)).expect("append");
        db.propose(&decision("x")).expect("propose");
        assert_eq!(db.count().expect("subjective count"), 1);
        assert_eq!(
            db.list_by_status(DecisionStatus::Proposed)
                .expect("list")
                .len(),
            1
        );
    }
}
