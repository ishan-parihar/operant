//! The department registry — SQLite storage for §3.3's staffing mandate.
//!
//! ## Where this table lives, and why it is not a new file
//!
//! `departments` goes in the **existing** `operant_kanban.db`, alongside
//! `notices` and `employees`. §3.3 says so, and §1.2 restates it as the
//! sibling-DB convention: one file per subsystem family, derived as
//! `config.database_path.parent()/operant_kanban.db`. The stated reason is the
//! repo's own scar — `BUGS.md` R5-1, the `memory/` split-brain: a second
//! location for one concept is a cost that has already been paid once. A
//! department is the same concept the kanban board and the employee registry
//! already live beside, so it joins them.
//!
//! ## How the schema is applied without a migration conflict
//!
//! The table is created with `CREATE TABLE IF NOT EXISTS`, and the columns a
//! later version may add are reconciled through
//! [`crate::org::schema::ensure_columns`], which keys on `PRAGMA table_info`
//! and is a no-op when the column is present.
//!
//! It deliberately does **not** go through `crate::migrations::migrate`.
//! `PRAGMA user_version` is file-wide: `operant_kanban.db` sits at version 1
//! because kanban's single `MIGRATIONS` entry claims that counter
//! (`kanban/db.rs:167`). A second family appending to the same counter moves
//! the *board's* version, and the next time kanban opens the file its
//! one-entry family looks downgraded and hard-fails with "refusing to
//! downgrade" — the INVARIANT `migrations.rs` documents after R39-7. So the
//! registry owns no version counter and bumps no PRAGMA. §12 resolves it the
//! same way: *no `PRAGMA user_version` use at all* for the org layer.
//!
//! The reconcile list is not the full column set, and that is deliberate.
//! [`ensure_column`] refuses `NOT NULL` without a default (SQLite rejects it on
//! a populated table) and refuses any primary-key addition, so the structural
//! columns — `dept_key`, `display_name`, `reason`, `created_at`, `updated_at`
//! — are fixed by the `CREATE TABLE` above. The reconcile list holds exactly
//! the columns a future version can add to a table that already has rows.
//!
//! ## Staffing is computed, never stored
//!
//! §3.3 is explicit: "Actual fill = live employees in the department."
//! There is no `filled_headcount` column and there must not be one — a stored
//! fill count is a claim that goes stale the moment somebody joins or leaves,
//! and a stale claim is worse than no claim because it reads as current.
//!
//! So [`DepartmentDb::check_staffing`] takes the **live employee rows** and
//! returns [`StaffingFinding`]s. This is the no-fabrication rule made
//! structural: a department that declares it needs a capability nobody holds,
//! or a head seat nobody occupies, produces a finding. It cannot render as
//! filled, because nothing in this module is able to invent the occupant.
//!
//! Two consequences worth naming, because both are judgement calls a reader
//! would otherwise have to reverse-engineer:
//!
//! - **Only [`STATUS_ACTIVE`] counts as occupying a seat.** A paused
//!   employee's row exists, but a paused agent is not doing the job, so
//!   reporting it as coverage would fabricate it. The conservative direction
//!   is the honest one: a false *finding* costs an operator a look; a missing
//!   finding costs them a department that looks staffed and is not.
//! - **A head seat naming somebody outside the department is a finding, not a
//!   fill.** `head_employee_id` is a pointer, and a pointer can be wrong. A
//!   head who is not an active member of the department they head is the
//!   unstaffed seat wearing a filled seat's clothes, so it reports as
//!   [`UNSTAFFED_HEAD`] rather than as staffed.
//!
//! [`target_headcount`] is stored but deliberately not reported against. §3.3
//! defines staffing findings as *capability coverage*, and being one person
//! short of a plan is a scheduling fact, not a mandate breach. The §11.4 gate
//! is where a plan-vs-actual comparison belongs; until it exists, reporting it
//! here would invent a policy nobody approved.
//!
//! ## Every mutation carries a reason, enforced here
//!
//! §3.3's `reason` column is not decoration. This store rejects a blank or
//! whitespace-only reason at the **store boundary**, not only in the CLI, for
//! the same reason the notice board and the employee registry do: a library
//! caller that bypasses argument validation must not be able to write a row
//! whose audit trail is empty. A blank reason is an error; it is never
//! substituted with a constant, because a constant names a party that did not
//! make the write.

use crate::error::Error;
use crate::org::employee::{Employee, STATUS_ACTIVE};
use crate::org::notice::rfc3339;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

/// [`StaffingFinding::kind`] for a head seat with no occupant.
pub const VACANT_HEAD: &str = "vacant_head";

/// [`StaffingFinding::kind`] for a required capability no active member holds.
pub const UNFILLED_CAPABILITY: &str = "unfilled_capability";

/// [`StaffingFinding::kind`] for a head seat that *names* an occupant who is
/// not an active member of the department. See the module docs.
pub const UNSTAFFED_HEAD: &str = "unstaffed_head";

/// One department row. Mirrors the `departments` table in §3.3.
///
/// All fields are public: the struct is a plain record of a row, and the
/// registry does not hide parts of an org's own structure from the code that
/// reads it. Timestamps are RFC3339, matching every other org table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Department {
    /// Canonical slug. Primary key. Renames go through `DEPT_ALIASES`, never
    /// in place, so a slug is stable and every reference to it stays valid.
    pub dept_key: String,
    /// Human label, e.g. `"Platform Infrastructure"`.
    pub display_name: String,
    /// Prose: what this department exists to do and how success is judged.
    /// NULL when the department has never been given one — an unwritten
    /// mandate is visible, not backfilled with a plausible sentence.
    pub mandate: Option<String>,
    /// JSON array of working protocols — the department's SOPs (§3.4 injects
    /// these into each member's system prompt).
    pub protocols: Vec<String>,
    /// JSON array of hard constraints. §3.3 is explicit that this array is
    /// **enforced, not advisory**; storing it as an empty default means
    /// "nothing declared", which the gate will read as "nothing to fail".
    pub rules: Vec<String>,
    /// JSON array of capabilities the seat set must cover. The source of
    /// [`UNFILLED_CAPABILITY`] findings.
    pub required_capabilities: Vec<String>,
    /// `None` = the head seat is vacant. §1's standing rule renders that as
    /// `"VACANT — <dept> (unstaffed)"` rather than omitting the seat.
    pub head_employee_id: Option<String>,
    /// Planned size. Stored for the §11.4 gate; see the module docs for why
    /// this store does not report against it.
    pub target_headcount: Option<i64>,
    /// The `--reason` of the write that last changed this row. Never empty.
    pub reason: String,
    /// RFC3339. Preserved across updates: a department is created once.
    pub created_at: String,
    /// RFC3339. Moves on every write.
    pub updated_at: String,
}

impl Department {
    /// A new department, stamped with the current time.
    pub fn new(
        dept_key: impl Into<String>,
        display_name: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new_at(dept_key, display_name, reason, &rfc3339(chrono::Utc::now()))
    }

    /// [`Department::new`] with an explicit `created_at`/`updated_at`, so a
    /// caller (or a test) controls the timestamps. The registry never
    /// substitutes its own clock for a caller's.
    pub fn new_at(
        dept_key: impl Into<String>,
        display_name: impl Into<String>,
        reason: impl Into<String>,
        now: &str,
    ) -> Self {
        Department {
            dept_key: dept_key.into(),
            display_name: display_name.into(),
            mandate: None,
            protocols: Vec::new(),
            rules: Vec::new(),
            required_capabilities: Vec::new(),
            head_employee_id: None,
            target_headcount: None,
            reason: reason.into(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        }
    }

    /// The canonical slug — the key every other org table references.
    pub fn dept_key(&self) -> &str {
        &self.dept_key
    }

    /// The human label.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The prose mandate, or `None` when the department has never been given
    /// one.
    pub fn mandate(&self) -> Option<&str> {
        self.mandate.as_deref()
    }

    /// Working protocols (SOPs), injected into each member's system prompt
    /// per §3.4.
    pub fn protocols(&self) -> &[String] {
        &self.protocols
    }

    /// Hard constraints. Violating one is a gate failure, not a warning.
    pub fn rules(&self) -> &[String] {
        &self.rules
    }

    /// Capabilities the seat set must cover. An empty slice means the
    /// department has declared no coverage requirement, which is not the same
    /// as being fully staffed — it means nothing was asked for.
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }

    /// The head seat's occupant, or `None` when it is vacant.
    pub fn head_employee_id(&self) -> Option<&str> {
        self.head_employee_id.as_deref()
    }

    /// Planned size. Not currently reported against — see the module docs.
    pub fn target_headcount(&self) -> Option<i64> {
        self.target_headcount
    }

    /// The justification recorded for the last write to this row.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Creation instant, RFC3339.
    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    /// Last-write instant, RFC3339.
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }
}

/// A visible staffing gap.
///
/// §3.3: "Unfilled required capabilities produce a visible finding, per the
/// standing no-fabrication rule." This is that finding. It is a value the
/// caller renders, not a log line — `org check`'s exit contract (§11.4) reads
/// findings, and the TUI shows them, so the shape has to survive both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StaffingFinding {
    /// Which department the gap is in.
    pub dept_key: String,
    /// Machine-readable kind: [`VACANT_HEAD`], [`UNFILLED_CAPABILITY`], or
    /// [`UNSTAFFED_HEAD`].
    pub kind: String,
    /// Human-readable sentence naming the specific gap.
    pub detail: String,
}

impl StaffingFinding {
    /// The head seat has no occupant at all.
    pub fn vacant_head(dept_key: &str) -> Self {
        StaffingFinding {
            dept_key: dept_key.to_string(),
            kind: VACANT_HEAD.to_string(),
            detail: format!(
                "{dept_key}: the head seat is vacant — no employee occupies it, so the \
                 department is running without a Head of Department"
            ),
        }
    }

    /// A required capability that no active member holds.
    pub fn unfilled_capability(dept_key: &str, capability: &str) -> Self {
        StaffingFinding {
            dept_key: dept_key.to_string(),
            kind: UNFILLED_CAPABILITY.to_string(),
            detail: format!(
                "{dept_key}: requires capability \"{capability}\" and no active employee \
                 in the department holds it"
            ),
        }
    }

    /// The head seat names somebody who is not an active member of this
    /// department. Distinct from [`StaffingFinding::vacant_head`] on purpose:
    /// the pointer is set but wrong, which is a different thing to fix and a
    /// different thing to be told.
    pub fn unstaffed_head(dept_key: &str, head_employee_id: &str) -> Self {
        StaffingFinding {
            dept_key: dept_key.to_string(),
            kind: UNSTAFFED_HEAD.to_string(),
            detail: format!(
                "{dept_key}: names head {head_employee_id}, who is not an active employee \
                 of this department — the seat is effectively unstaffed"
            ),
        }
    }
}

/// Whether an employee currently occupies their seat.
///
/// Only [`STATUS_ACTIVE`] counts. §3.1 carries a non-`active` cron state
/// through verbatim (a paused job stays visibly paused in the registry), so a
/// paused employee has a row but is not working; counting it as coverage
/// would be the fabrication §3.3 forbids.
fn occupies_seat(employee: &Employee) -> bool {
    employee.status == STATUS_ACTIVE
}

/// Staffing findings for `departments` given the **live** `employees` rows.
///
/// Pure — no I/O — so the no-fabrication rule is testable without a database
/// and readable without tracing a query. [`DepartmentDb::check_staffing`] is
/// the store-backed wrapper that loads the departments first.
///
/// Findings come back in a deterministic order: departments in `dept_key`
/// order, head finding before capability findings, capabilities in declared
/// order. A caller that renders findings should get the same report twice from
/// the same state.
pub fn staffing_findings(
    departments: &[Department],
    employees: &[Employee],
) -> Vec<StaffingFinding> {
    let mut findings = Vec::new();
    for dept in departments {
        let key = dept.dept_key.as_str();

        // The head seat. NULL is a vacancy; a pointer to somebody who is not
        // an active member of *this* department is a vacancy with extra steps.
        match dept.head_employee_id.as_deref() {
            None => findings.push(StaffingFinding::vacant_head(key)),
            Some(head_id) => {
                let holds_seat = employees.iter().any(|e| {
                    e.employee_id == head_id
                        && occupies_seat(e)
                        && e.department.as_deref() == Some(key)
                });
                if !holds_seat {
                    findings.push(StaffingFinding::unstaffed_head(key, head_id));
                }
            }
        }

        // Capability coverage, computed from the live rows.
        let held: HashSet<&str> = employees
            .iter()
            .filter(|e| occupies_seat(e) && e.department.as_deref() == Some(key))
            .flat_map(|e| e.skills.iter().map(String::as_str))
            .collect();
        for capability in &dept.required_capabilities {
            if !held.contains(capability.as_str()) {
                findings.push(StaffingFinding::unfilled_capability(key, capability));
            }
        }
    }
    findings
}

/// The department registry store.
pub struct DepartmentDb {
    conn: Arc<Mutex<Connection>>,
}

impl DepartmentDb {
    /// Open (or create) the registry in an existing sqlite file.
    ///
    /// `path` is expected to be the sibling kanban DB
    /// ([`NOTICE_BOARD_DB_FILE`](crate::org::notice::NOTICE_BOARD_DB_FILE)),
    /// but nothing here enforces that — the registry is a table, not a file,
    /// and tests legitimately point it at a temp path. The parent directory is
    /// created first, because the kanban path is derived from a configured
    /// database path and a configured path may name a directory that does not
    /// exist yet.
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("departments: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("departments: open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Attach the registry to a connection somebody else already owns.
    ///
    /// The registry is a table inside a database another subsystem owns, so
    /// this is the path the rest of operant should use. Opening a second
    /// `Connection` to the same file just to create one table would trade a
    /// separate-store problem for a separate-lock problem.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        Self::from_shared_connection(Arc::new(Mutex::new(conn)))
    }

    /// Borrow an already-open `Arc<Mutex<Connection>>` (the shape
    /// [`KanbanDb::conn`](crate::kanban::db::KanbanDb::conn) returns) so the
    /// registry shares the kanban handle instead of opening a second writer on
    /// the same file.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.ensure_schema()?;
        Ok(db)
    }

    /// The underlying connection, for callers that need to share one
    /// connection with kanban, the notice board, or the employee registry.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Lock the connection, converting mutex poisoning into a recoverable
    /// error instead of panicking (same pattern as `kanban/db.rs:141`,
    /// `cronjobs/db.rs:101`, and [`NoticeBoard`](crate::org::notice_db::NoticeBoard)).
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("departments db mutex poisoned".to_string()))
    }

    /// §3.3's table, verbatim.
    ///
    /// Applied declaratively and idempotently — see the module docs for why
    /// this is not a `crate::migrations::migrate` entry.
    const DEPARTMENTS_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS departments (
            dept_key              TEXT PRIMARY KEY,         -- canonical slug; renames go via DEPT_ALIASES
            display_name          TEXT NOT NULL,            -- human label
            mandate               TEXT,                     -- prose: what this dept exists to do
            protocols             TEXT NOT NULL DEFAULT '[]', -- JSON array of working protocols (SOPs)
            rules                 TEXT NOT NULL DEFAULT '[]', -- JSON array of hard constraints (§3.3: enforced)
            required_capabilities TEXT NOT NULL DEFAULT '[]', -- JSON array the seat set must cover
            head_employee_id      TEXT,                     -- NULL = head seat vacant (§1: stays visible)
            target_headcount      INTEGER,                  -- planned size
            reason                TEXT NOT NULL,            -- the --reason of the last write
            created_at            TEXT NOT NULL,            -- RFC3339
            updated_at            TEXT NOT NULL             -- RFC3339
        );
    "#;

    /// The columns a future version can add to a populated table, with the
    /// definitions to add them under.
    ///
    /// The structural columns are absent on purpose: `ensure_column` rejects a
    /// primary key and rejects `NOT NULL` without a default, so `dept_key`,
    /// `display_name`, `reason`, `created_at`, and `updated_at` are fixed by
    /// [`Self::DEPARTMENTS_SCHEMA`]. Everything listed here is either nullable
    /// or carries a default, so each one is addable to a table that already has
    /// rows. See the module docs.
    const RECONCILE_COLUMNS: &[(&str, &str)] = &[
        ("mandate", "TEXT"),
        ("protocols", "TEXT NOT NULL DEFAULT '[]'"),
        ("rules", "TEXT NOT NULL DEFAULT '[]'"),
        ("required_capabilities", "TEXT NOT NULL DEFAULT '[]'"),
        ("head_employee_id", "TEXT"),
        ("target_headcount", "INTEGER"),
    ];

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::DEPARTMENTS_SCHEMA)
            .map_err(|e| Error::Agent(format!("departments: schema: {e}")))?;
        crate::org::schema::ensure_columns(&conn, "departments", Self::RECONCILE_COLUMNS)
            .map_err(|e| Error::Agent(format!("departments: reconcile columns: {e}")))?;
        Ok(())
    }

    /// Every column, in [`Department`] field order. Shared by `get` and
    /// `list` so a column can never be added to one and forgotten in the other.
    const SELECT_COLUMNS: &'static str = "dept_key, display_name, mandate, protocols, rules, \
         required_capabilities, head_employee_id, target_headcount, reason, created_at, updated_at";

    /// Insert or update a department.
    ///
    /// Idempotent on `dept_key`: writing the same department twice is one row,
    /// not two and not an error. `created_at` is **preserved** across an
    /// update — a department is created once, and an update that reset the
    /// creation instant would make every org report wrong about its own age.
    /// Every other column, including `reason` and `updated_at`, is taken
    /// verbatim from `dept`, so a round trip through the store is lossless.
    ///
    /// # Errors
    ///
    /// Rejects a blank `dept_key` (a row nobody can address) and a blank
    /// `reason` (see the module docs), before touching the database.
    pub fn upsert(&self, dept: &Department) -> Result<(), Error> {
        let key = dept.dept_key.trim();
        if key.is_empty() {
            return Err(Error::Agent(
                "departments: dept_key is required — a department with no key cannot be \
                 addressed by a notice, an assignment, or a gate"
                    .to_string(),
            ));
        }
        let reason = validated_reason(&dept.reason)?;
        let protocols = encode_list(&dept.protocols)?;
        let rules = encode_list(&dept.rules)?;
        let required_capabilities = encode_list(&dept.required_capabilities)?;

        let conn = self.lock_conn()?;
        conn.execute(
            &format!(
                "INSERT INTO departments ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(dept_key) DO UPDATE SET
                    display_name          = excluded.display_name,
                    mandate               = excluded.mandate,
                    protocols             = excluded.protocols,
                    rules                 = excluded.rules,
                    required_capabilities = excluded.required_capabilities,
                    head_employee_id      = excluded.head_employee_id,
                    target_headcount      = excluded.target_headcount,
                    reason                = excluded.reason,
                    updated_at            = excluded.updated_at",
                Self::SELECT_COLUMNS
            ),
            params![
                key,
                dept.display_name,
                dept.mandate,
                protocols,
                rules,
                required_capabilities,
                dept.head_employee_id,
                dept.target_headcount,
                reason,
                dept.created_at,
                dept.updated_at,
            ],
        )
        .map_err(|e| Error::Agent(format!("departments: upsert {key}: {e}")))?;
        Ok(())
    }

    /// One department by key, or `None` when there is no such row.
    ///
    /// # Errors
    ///
    /// A malformed JSON column is an error naming the column, never a silently
    /// empty list: a `rules` array that fails to parse must not read as "no
    /// rules declared", because §3.3 makes a rule a gate failure and losing
    /// one here would silently un-gate a department.
    pub fn get(&self, dept_key: &str) -> Result<Option<Department>, Error> {
        let conn = self.lock_conn()?;
        conn.query_row(
            &format!(
                "SELECT {} FROM departments WHERE dept_key = ?1",
                Self::SELECT_COLUMNS
            ),
            params![dept_key],
            row_to_department,
        )
        .optional()
        .map_err(|e| Error::Agent(format!("departments: get {dept_key}: {e}")))
    }

    /// Every department, ordered by `dept_key`.
    ///
    /// The order is deliberate: `org check` and the TUI both render this list,
    /// and a registry whose rows move between two identical reads is a
    /// registry nobody can diff.
    pub fn list(&self) -> Result<Vec<Department>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM departments ORDER BY dept_key",
                Self::SELECT_COLUMNS
            ))
            .map_err(|e| Error::Agent(format!("departments: prepare list: {e}")))?;
        let rows = stmt
            .query_map([], row_to_department)
            .map_err(|e| Error::Agent(format!("departments: query list: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Agent(format!("departments: read row: {e}")))?);
        }
        Ok(out)
    }

    /// Fill, vacate, or reassign a department's head seat.
    ///
    /// `head_employee_id = None` vacates the seat, which is a real and
    /// attributable act: it is what makes a department show up in
    /// [`staffing_findings`] as [`VACANT_HEAD`]. §1's rule is that an
    /// unstaffed seat stays *visible*, so vacating is recorded rather than
    /// papered over.
    ///
    /// `reason` is required and validated here, not only in the CLI. The row's
    /// `reason` column moves to this write's reason, so the row always names
    /// the party accountable for its current state.
    ///
    /// # Errors
    ///
    /// Rejects a blank key or reason, and a key that matches no department —
    /// assigning a head to a seat that does not exist would otherwise be a
    /// silent no-op that reads as success.
    pub fn set_head(
        &self,
        dept_key: &str,
        head_employee_id: Option<&str>,
        reason: &str,
    ) -> Result<(), Error> {
        let key = dept_key.trim();
        if key.is_empty() {
            return Err(Error::Agent(
                "departments: set_head requires a dept_key".to_string(),
            ));
        }
        let reason = validated_reason(reason)?;
        let now = rfc3339(chrono::Utc::now());
        let conn = self.lock_conn()?;
        let changed = conn
            .execute(
                "UPDATE departments
                    SET head_employee_id = ?1, reason = ?2, updated_at = ?3
                  WHERE dept_key = ?4",
                params![head_employee_id, reason, now, key],
            )
            .map_err(|e| Error::Agent(format!("departments: set_head {key}: {e}")))?;
        if changed == 0 {
            return Err(Error::Agent(format!(
                "departments: set_head: no department with key `{key}` — create the \
                 department before filling its head seat"
            )));
        }
        Ok(())
    }

    /// Staffing findings for every stored department, given the **live**
    /// employee rows.
    ///
    /// This is the load-bearing call behind §11.4's `org check` and the §1
    /// rule that an unstaffed seat stays visible. It reads the departments and
    /// derives coverage from `employees`; it never persists a fill count,
    /// because a stored count would be a claim that goes stale silently.
    pub fn check_staffing(&self, employees: &[Employee]) -> Result<Vec<StaffingFinding>, Error> {
        let departments = self.list()?;
        Ok(staffing_findings(&departments, employees))
    }
}

/// Reject a blank reason at the store boundary, returning the trimmed value.
///
/// Whitespace-only counts as blank: `"   "` is what an unset optional CLI flag
/// looks like once it has been through a shell, and accepting it would put an
/// effectively empty audit trail in a column the design calls mandatory.
fn validated_reason(reason: &str) -> Result<String, Error> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Err(Error::Agent(
            "departments: every write requires a non-empty reason — §3.3 makes the reason \
             a mandatory column, and an empty audit trail is not a reason"
                .to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn encode_list(values: &[String]) -> Result<String, Error> {
    serde_json::to_string(values).map_err(|e| Error::Agent(format!("departments: encode: {e}")))
}

/// Read one JSON-array column, naming the column in the failure.
///
/// A `rules` array that will not parse is reported as a decode error rather
/// than degrading to an empty list, and the message names the column instead
/// of its ordinal: "index 4" tells a reader nothing, "rules" tells them which
/// of the department's three arrays is corrupt. The ordinal is kept alongside
/// so a caller that wants the position still has it.
fn decode_list(row: &Row<'_>, column: &str) -> rusqlite::Result<Vec<String>> {
    let raw: String = row.get(column)?;
    let idx = row.as_ref().column_index(column).unwrap_or(0);
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(MalformedColumn {
                column: column.to_string(),
                source: e,
            }),
        )
    })
}

/// The error behind a malformed JSON column: which column, and why.
#[derive(Debug)]
struct MalformedColumn {
    column: String,
    source: serde_json::Error,
}

impl std::fmt::Display for MalformedColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "column `{}` is not a JSON array: {}",
            self.column, self.source
        )
    }
}

impl std::error::Error for MalformedColumn {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn row_to_department(row: &Row<'_>) -> rusqlite::Result<Department> {
    Ok(Department {
        dept_key: row.get("dept_key")?,
        display_name: row.get("display_name")?,
        mandate: row.get("mandate")?,
        protocols: decode_list(row, "protocols")?,
        rules: decode_list(row, "rules")?,
        required_capabilities: decode_list(row, "required_capabilities")?,
        head_employee_id: row.get("head_employee_id")?,
        target_headcount: row.get("target_headcount")?,
        reason: row.get("reason")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A registry on a throwaway sqlite file named exactly like the real
    /// sibling DB, so a test that passed here cannot pass by accident against
    /// a differently-named file.
    fn db() -> (DepartmentDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().join("operant_kanban.db");
        let db = DepartmentDb::init(path).expect("DepartmentDb::init");
        (db, dir)
    }

    fn dept(key: &str) -> Department {
        Department::new_at(
            key,
            format!("{key} display"),
            "stand up the department",
            "2026-10-01T00:00:00.000Z",
        )
    }

    /// A live employee in `dept_key` holding `skills`.
    fn employee(id: &str, dept_key: Option<&str>, skills: &[&str]) -> Employee {
        Employee {
            employee_id: id.to_string(),
            name: id.to_string(),
            role: "automator".to_string(),
            department: dept_key.map(str::to_string),
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            agent_type: None,
            persona: None,
            status: STATUS_ACTIVE.to_string(),
            reason: "test fixture".to_string(),
            created_at: "2026-10-01T00:00:00.000Z".to_string(),
            updated_at: "2026-10-01T00:00:00.000Z".to_string(),
        }
    }

    // ------------------------------------------------------------ the reason

    /// Hard rule 1: a blank reason is an error at the store boundary, not a
    /// row with an empty audit trail.
    #[test]
    fn blank_reason_is_rejected() {
        let (db, _dir) = db();
        let err = db
            .upsert(&Department::new_at(
                "infra",
                "Infra",
                "   ",
                "2026-10-01T00:00:00.000Z",
            ))
            .expect_err("whitespace-only reason must be refused");
        assert!(
            err.to_string().contains("non-empty reason"),
            "unhelpful: {err}"
        );
        assert!(
            db.get("infra").expect("get").is_none(),
            "a refused write must leave no row behind"
        );
    }

    #[test]
    fn empty_reason_is_rejected() {
        let (db, _dir) = db();
        assert!(
            db.upsert(&Department::new_at("infra", "Infra", "", "t"))
                .is_err()
        );
        assert!(db.set_head("infra", Some("emp-1"), "").is_err());
    }

    #[test]
    fn blank_reason_is_rejected_by_set_head_too() {
        let (db, _dir) = db();
        db.upsert(&dept("infra")).expect("seed");
        let err = db
            .set_head("infra", Some("emp-1"), "  \t ")
            .expect_err("set_head must validate as well");
        assert!(
            err.to_string().contains("non-empty reason"),
            "unhelpful: {err}"
        );
    }

    // ----------------------------------------------------------- round trips

    #[test]
    fn upsert_then_get_round_trips() {
        let (db, _dir) = db();
        let mut d = dept("platform-infra");
        d.mandate = Some("keep the platform up".to_string());
        d.protocols = vec!["incident: page first, diagnose second".to_string()];
        d.rules = vec!["must not write outside its own tree".to_string()];
        d.required_capabilities = vec!["kubernetes".to_string(), "terraform".to_string()];
        d.head_employee_id = Some("emp-head".to_string());
        d.target_headcount = Some(4);
        db.upsert(&d).expect("upsert");

        let read = db.get("platform-infra").expect("get").expect("row present");
        assert_eq!(read, d, "round trip must be lossless");
    }

    #[test]
    fn upsert_is_idempotent() {
        let (db, _dir) = db();
        let d = dept("content");
        db.upsert(&d).expect("first");
        db.upsert(&d).expect("second");
        db.upsert(&d).expect("third");
        let all = db.list().expect("list");
        assert_eq!(all.len(), 1, "upsert must not duplicate the row");
        assert_eq!(all[0], d);
    }

    #[test]
    fn upsert_updates_and_preserves_created_at() {
        let (db, _dir) = db();
        db.upsert(&dept("content")).expect("first");
        let mut later = dept("content");
        later.display_name = "Content Studio".to_string();
        later.created_at = "2099-01-01T00:00:00.000Z".to_string();
        later.updated_at = "2026-10-02T00:00:00.000Z".to_string();
        later.reason = "renamed".to_string();
        db.upsert(&later).expect("update");

        let read = db.get("content").expect("get").expect("row");
        assert_eq!(read.display_name, "Content Studio");
        assert_eq!(read.reason, "renamed");
        assert_eq!(read.updated_at, "2026-10-02T00:00:00.000Z");
        assert_eq!(
            read.created_at, "2026-10-01T00:00:00.000Z",
            "an update must not rewrite the creation instant"
        );
    }

    #[test]
    fn get_on_absent_key_is_none_not_an_error() {
        let (db, _dir) = db();
        assert!(db.get("nope").expect("get").is_none());
    }

    #[test]
    fn list_is_ordered_and_skips_unknown_rows() {
        let (db, _dir) = db();
        db.upsert(&dept("zulu")).expect("z");
        db.upsert(&dept("alpha")).expect("a");
        db.upsert(&dept("mike")).expect("m");
        let keys: Vec<String> = db
            .list()
            .expect("list")
            .into_iter()
            .map(|d| d.dept_key)
            .collect();
        assert_eq!(keys, vec!["alpha", "mike", "zulu"]);
    }

    #[test]
    fn blank_dept_key_is_rejected() {
        let (db, _dir) = db();
        let err = db.upsert(&dept("   ")).expect_err("must refuse");
        assert!(err.to_string().contains("dept_key"), "unhelpful: {err}");
    }

    // ------------------------------------------------------------- set_head

    #[test]
    fn set_head_fills_then_vacates_the_seat() {
        let (db, _dir) = db();
        db.upsert(&dept("infra")).expect("seed");
        db.set_head("infra", Some("emp-1"), "promoted")
            .expect("fill");
        let filled = db.get("infra").expect("get").expect("row");
        assert_eq!(filled.head_employee_id.as_deref(), Some("emp-1"));
        assert_eq!(filled.reason, "promoted", "reason must record this write");

        db.set_head("infra", None, "left the org").expect("vacate");
        let vacant = db.get("infra").expect("get").expect("row");
        assert_eq!(vacant.head_employee_id, None, "vacating must be visible");
        assert_eq!(vacant.reason, "left the org");
    }

    #[test]
    fn set_head_on_unknown_department_is_a_named_error() {
        let (db, _dir) = db();
        let err = db
            .set_head("ghost", Some("emp-1"), "hiring")
            .expect_err("must not be a silent no-op");
        assert!(
            err.to_string().contains("no department with key"),
            "unhelpful: {err}"
        );
    }

    // ------------------------------------------------------------- staffing

    /// Hard rule 2: a vacant head is a finding, never rendered as filled.
    #[test]
    fn vacant_head_produces_a_finding() {
        let (db, _dir) = db();
        let mut d = dept("infra");
        d.required_capabilities.clear();
        db.upsert(&d).expect("seed");

        let findings = db
            .check_staffing(&[employee("emp-1", Some("infra"), &["kubernetes"])])
            .expect("staffing");
        assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
        assert_eq!(findings[0].kind, VACANT_HEAD);
        assert_eq!(findings[0].dept_key, "infra");
        assert!(
            findings[0].detail.contains("vacant"),
            "unhelpful: {}",
            findings[0].detail
        );
    }

    #[test]
    fn unfilled_capability_produces_a_finding() {
        let (db, _dir) = db();
        let mut d = dept("infra");
        d.head_employee_id = Some("emp-head".to_string());
        d.required_capabilities = vec!["kubernetes".to_string(), "terraform".to_string()];
        db.upsert(&d).expect("seed");

        let findings = db
            .check_staffing(&[
                employee("emp-head", Some("infra"), &["kubernetes"]),
                employee("emp-2", Some("infra"), &["kubernetes"]),
            ])
            .expect("staffing");
        assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
        assert_eq!(findings[0].kind, UNFILLED_CAPABILITY);
        assert!(
            findings[0].detail.contains("terraform"),
            "the finding must name the missing capability: {}",
            findings[0].detail
        );
    }

    #[test]
    fn a_fully_staffed_department_produces_no_findings() {
        let (db, _dir) = db();
        let mut d = dept("infra");
        d.head_employee_id = Some("emp-head".to_string());
        d.required_capabilities = vec!["kubernetes".to_string(), "terraform".to_string()];
        db.upsert(&d).expect("seed");

        let findings = db
            .check_staffing(&[
                employee("emp-head", Some("infra"), &["kubernetes"]),
                employee("emp-2", Some("infra"), &["terraform"]),
            ])
            .expect("staffing");
        assert!(
            findings.is_empty(),
            "fully staffed must be silent: {findings:?}"
        );
    }

    #[test]
    fn a_department_with_no_members_reports_every_gap() {
        let (db, _dir) = db();
        let mut d = dept("infra");
        d.required_capabilities = vec!["kubernetes".to_string(), "terraform".to_string()];
        db.upsert(&d).expect("seed");

        let findings = db.check_staffing(&[]).expect("staffing");
        assert_eq!(findings.len(), 3, "head + two capabilities: {findings:?}");
        assert_eq!(findings[0].kind, VACANT_HEAD, "head finding comes first");
    }

    #[test]
    fn a_paused_employee_does_not_cover_a_capability() {
        let (db, _dir) = db();
        let mut d = dept("infra");
        d.head_employee_id = Some("emp-head".to_string());
        d.required_capabilities = vec!["kubernetes".to_string()];
        db.upsert(&d).expect("seed");

        let mut paused = employee("emp-head", Some("infra"), &["kubernetes"]);
        paused.status = "paused".to_string();
        let findings = db.check_staffing(&[paused]).expect("staffing");
        assert_eq!(findings.len(), 2, "paused covers nothing: {findings:?}");
        assert_eq!(findings[0].kind, UNSTAFFED_HEAD);
        assert_eq!(findings[1].kind, UNFILLED_CAPABILITY);
    }

    /// A head seat pointing outside the department is an unstaffed seat
    /// wearing a filled seat's clothes, so it must not read as staffed.
    #[test]
    fn a_head_from_another_department_is_a_finding() {
        let (db, _dir) = db();
        let mut d = dept("infra");
        d.head_employee_id = Some("emp-outsider".to_string());
        db.upsert(&d).expect("seed");

        let findings = db
            .check_staffing(&[employee("emp-outsider", Some("content"), &[])])
            .expect("staffing");
        assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
        assert_eq!(findings[0].kind, UNSTAFFED_HEAD);
        assert!(
            findings[0].detail.contains("emp-outsider"),
            "the finding must name who: {}",
            findings[0].detail
        );
    }

    #[test]
    fn findings_are_deterministic_across_calls() {
        let (db, _dir) = db();
        let mut a = dept("alpha");
        a.required_capabilities = vec!["x".to_string(), "y".to_string()];
        let mut b = dept("beta");
        b.required_capabilities = vec!["z".to_string()];
        db.upsert(&a).expect("a");
        db.upsert(&b).expect("b");

        let first = db.check_staffing(&[]).expect("first");
        let second = db.check_staffing(&[]).expect("second");
        assert_eq!(first, second, "the same state must render the same report");
        let keys: Vec<&str> = first.iter().map(|f| f.dept_key.as_str()).collect();
        assert_eq!(keys, vec!["alpha", "alpha", "alpha", "beta", "beta"]);
    }

    #[test]
    fn pure_staffing_needs_no_database() {
        let mut d = dept("infra");
        d.head_employee_id = Some("emp-head".to_string());
        d.required_capabilities = vec!["kubernetes".to_string()];
        let findings = staffing_findings(
            &[d],
            &[employee("emp-head", Some("infra"), &["kubernetes"])],
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    // ------------------------------------------------------------ durability

    /// The table lives in the shared kanban file, so init must be safe to run
    /// on every boot and must not disturb rows an earlier handle wrote.
    #[test]
    fn the_table_survives_being_opened_twice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().join("operant_kanban.db");
        let first = DepartmentDb::init(path.clone()).expect("first open");
        first.upsert(&dept("infra")).expect("write");
        drop(first);

        let reopened = DepartmentDb::init(path.clone()).expect("second open");
        assert_eq!(
            reopened.get("infra").expect("get").expect("row"),
            dept("infra"),
            "rows must survive a reopen"
        );
        // And opening a third time is still a no-op rather than an error.
        let again = DepartmentDb::init(path).expect("third open");
        again.upsert(&dept("infra")).expect("write after reopen");
        assert_eq!(again.list().expect("list").len(), 1);
    }

    #[test]
    fn init_creates_the_parent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b").join("operant_kanban.db");
        assert!(!Path::new(&dir.path().join("a")).exists());
        let db = DepartmentDb::init(nested.clone()).expect("init must create the dir");
        db.upsert(&dept("infra")).expect("write");
        assert!(nested.exists());
    }

    #[test]
    fn from_connection_reuses_a_caller_owned_handle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("operant_kanban.db")).expect("open");
        let shared = Arc::new(Mutex::new(conn));
        let db = DepartmentDb::from_shared_connection(Arc::clone(&shared)).expect("attach");
        db.upsert(&dept("infra")).expect("write");

        let count: i64 = shared
            .lock()
            .expect("lock")
            .query_row("SELECT COUNT(*) FROM departments", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1, "the write must be visible on the shared handle");
    }

    /// A `rules` array that cannot be parsed must surface as a named error,
    /// not as an empty array — losing a rule would silently un-gate a
    /// department.
    #[test]
    fn a_malformed_json_column_is_an_error_not_an_empty_list() {
        let (db, _dir) = db();
        db.upsert(&dept("infra")).expect("seed");
        db.conn()
            .lock()
            .expect("lock")
            .execute(
                "UPDATE departments SET rules = 'not json' WHERE dept_key = 'infra'",
                [],
            )
            .expect("corrupt");
        let err = db
            .get("infra")
            .expect_err("must not decode as an empty list");
        assert!(err.to_string().contains("rules"), "unhelpful: {err}");
    }

    #[test]
    fn defaults_apply_when_a_row_is_written_out_of_band() {
        let (db, _dir) = db();
        db.conn()
            .lock()
            .expect("lock")
            .execute(
                "INSERT INTO departments (dept_key, display_name, reason, created_at, updated_at)
                 VALUES ('bare', 'Bare', 'seeded', 't0', 't0')",
                [],
            )
            .expect("insert");
        let row = db.get("bare").expect("get").expect("row");
        assert!(row.protocols.is_empty());
        assert!(row.rules.is_empty());
        assert!(row.required_capabilities.is_empty());
        assert_eq!(row.head_employee_id, None);
        assert_eq!(row.target_headcount, None);
    }
}
