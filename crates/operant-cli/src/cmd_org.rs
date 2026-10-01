//! `operant org` — the org surface, and the `--reason` mandate (Wave 1).
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.5. Every mutating subcommand in this
//! module carries a **required, long-only** `--reason` declared on the clap
//! variant, so clap rejects a missing one before any I/O happens. The second
//! half of the mandate — a reason that is present but empty or whitespace is
//! also not a reason — is [`require_reason`], called at the top of every
//! mutating handler before the database is opened.
//!
//! ## Why parsed-and-discarded is the failure mode
//!
//! The organism's post-mortem (AD-032) is that 1,311 logged decisions were
//! read by nobody. A `--reason` flag that clap accepts and the handler then
//! throws away is *worse* than no flag: it makes the command look auditable
//! while writing nothing. So every write here persists the value into the
//! same row and the same transaction as its effect — §3.1's
//! `reason TEXT NOT NULL`, §3.5's "the reason and the effect are one
//! transaction".
//!
//! ## Where the reason is persisted
//!
//! | Command | Persisted in the same transaction as the effect |
//! |---|---|
//! | `org employee create` | `employees.reason` (§3.1, NOT NULL) |
//! | `org employee update` | `employees.reason` + `updated_at` |
//! | `org employee retire` | `employees.reason` + `status` + `updated_at` |
//! | `org notice post` | `notices.reason` (§3.3, NOT NULL) |
//! | `org notice ack` | an `ack`-tagged `notices` row, same `correlation_id`, **plus** the parent row's `acked_by`/`acked_at` (§3.3) |
//! | `org notice pin` / `unpin` | a `pin`/`unpin`-tagged `notices` row, plus the parent row's `pinned` |
//! | `org worklog append` | the `artifacts` JSON of the appended row — §3.4's 20 fields have no `reason` column, so inventing a 21st was rejected; see [`operator_entry`] |
//! | `org sync` | `employees.reason` + `employee_cron_jobs`, one row per backfilled job (§3.1) |
//! | `org check` / `org list` / `org worklog list` | read-only — no reason |
//!
//! `org import` parses and validates a reason and then writes **nothing**, by
//! design and loudly: §4.3 of the decision document records importing a
//! foreign organism tree as an open *product* decision the owner has not
//! made. See [`cmd_import`]. (`org sync`, which once sat here too, is now
//! wired to the real `EmployeeDb::backfill_from_cron_jobs`.)
//!
//! ## Storage — one schema, owned by operant-core
//!
//! `operant-core/src/org/` owns the DDL for all four tables
//! (`EmployeeDb`/`NoticeBoard`/`WorklogDb`), and this module calls those
//! stores. It does **not** carry its own copy of the schema.
//!
//! That is not a style preference. An earlier revision of this file
//! transcribed the DDL locally, on the grounds that `operant-core` did not
//! compile yet. By the time it did, the two copies had already diverged on
//! the `worklog` table: the local copy had `created_at`/`epoch`/
//! `result_summary`/`sender`/`tags`/`reason`, while core's had the
//! 20-column §3.4 shape (`ts`/`ts_iso`/`employee`/`what_done`/`outcome`/
//! `next_intent`/`improvement_proposal`/…). Because every statement was
//! `CREATE TABLE IF NOT EXISTS`, whichever side ran first would define the
//! table and the other would fail at insert time with "no such column" — a
//! latent failure whose only symptom was a working CLI and a dead
//! `WorklogDb`, or the reverse, depending on boot order. One schema, in one
//! place, removes that class of bug entirely.
//!
//! ## Known limits
//!
//! 1. `org employee update` cannot clear a field back to NULL, because the
//!    update path cannot distinguish "not passed" from "passed as null" over
//!    the CLI. Documented on [`OrgEmployeeAction::Update`]. The workaround
//!    (a `--clear-*` flag per column) is scope this packet was not asked for.
//! 2. `org worklog append` goes through `WorklogDb::append` and the sealed
//!    `TurnObservation::into_entry` constructor rather than raw SQL, so an
//!    operator's row is shaped exactly like a framework row. See
//!    [`operator_entry`].
//! 3. `org import` writes nothing, by design — see the note at the top.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use operant_core::config::AppConfig;
use operant_core::org::employee_db::org_db_path;
use operant_core::org::notice::{PostNotice, Recipient};
use operant_core::org::notice_db::NoticeBoard;
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};

/// Reject a reason that is present but is not a reason.
///
/// §3.5 makes `--reason` required at the clap layer, which catches a
/// *missing* flag. It does not catch `--reason ""` or `--reason "   "`, and
/// a zero-length string satisfies a `NOT NULL` column perfectly well — it
/// is a reason-shaped value that carries no causal information, which is
/// exactly what AD-032 already caught operant doing in
/// `cmd_kanban.rs`'s `"Blocked via CLI"` default.
///
/// This lives at the write boundary rather than in a clap value parser for
/// two reasons: it can name the specific command in the error, and it cannot
/// be bypassed by any future call path that reaches a handler directly.
///
/// Returns the reason **unchanged**, not trimmed: the audit record should
/// carry what the operator actually typed. Emptiness is judged on the
/// trimmed view, but the stored value is the raw input — trimming on the
/// way in would quietly alter the audit record.
pub fn require_reason<'a>(command: &str, reason: &'a str) -> Result<&'a str> {
    if reason.trim().is_empty() {
        anyhow::bail!(
            "--reason is required and must not be empty or whitespace-only \
             (command: `{}`). An empty reason satisfies the NOT NULL \
             constraint while recording nothing about why this write \
             happened, which is the AD-032 failure mode. State the cause, \
             e.g. --reason \"upstream API returned 429\".",
            command
        );
    }
    Ok(reason)
}

/// `require_reason` **and bind** the validated value.
///
/// Every mutating handler should use this rather than `require_reason(..)?`
/// followed by reading the original binding: the check trims to decide
/// emptiness but hands back the *untrimmed* string, so a handler that stored
/// it verbatim would persist the operator's shell padding along with their
/// reason. Binding the returned value makes "validated" and "what gets
/// written" the same string by construction.
fn validated_reason<'a>(command: &str, reason: &'a str) -> Result<&'a str> {
    require_reason(command, reason).map(|r| r.trim())
}

/// Department commands. `set` is a partial update: a field you do not name
/// is preserved rather than blanked, because §3.3 makes `rules` enforced —
/// an empty array reads as "nothing declared", which would quietly disarm the
/// department.
#[derive(Debug, Clone, Subcommand)]
pub enum OrgDepartmentAction {
    /// Create or update a department
    Set(Box<OrgDepartmentSetArgs>),

    /// List departments (read-only)
    List,

    /// Report staffing findings: vacant seats, unstaffed heads, unfilled
    /// capabilities. Exits 1 when any finding exists, so CI can gate on it.
    Findings {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Arguments for `org department set`.
///
/// A separate, boxed `Args` struct rather than inline enum fields, for two
/// reasons. It keeps `OrgDepartmentAction` a reasonable size: this form
/// carries eight `String`/`Option<String>` fields (~208 bytes) while the other
/// two variants carry at most a `bool`, so inlining it would make every
/// `List`/`Findings` match pay for the form's stack frame — hence the `Box`.
/// And a partial update needs a distinguishable "not mentioned" for every
/// optional field, which reads far better as one struct than as eight enum
/// fields.
#[derive(Debug, Clone, Args)]
pub struct OrgDepartmentSetArgs {
    /// Canonical department slug (primary key)
    pub dept_key: String,
    /// Human label (defaults to the slug)
    #[arg(long)]
    pub display_name: Option<String>,
    /// What this department exists to do
    #[arg(long)]
    pub mandate: Option<String>,
    /// Working protocols, comma-separated (§3.4 injects these into member prompts)
    #[arg(long, default_value = "")]
    pub protocols: String,
    /// Hard constraints, comma-separated (enforced, not advisory)
    #[arg(long, default_value = "")]
    pub rules: String,
    /// Capabilities the seat set must cover, comma-separated
    #[arg(long, default_value = "")]
    pub required_capabilities: String,
    /// Employee id occupying the head seat (omit to leave it vacant)
    #[arg(long)]
    pub head_employee_id: Option<String>,
    /// Planned size
    #[arg(long)]
    pub target_headcount: Option<i64>,
    /// Why this change is being made (required; stored in departments.reason)
    #[arg(long, value_name = "REASON")]
    pub reason: String,
}

/// Decision commands. A decision is a first-class artifact: dissent is
/// recorded against it rather than blocking it (§7 makes CEO/head decisions
/// binding by default while keeping dissent visible).
#[derive(Debug, Clone, Subcommand)]
pub enum OrgDecisionAction {
    /// Propose a decision
    Propose {
        /// What is being decided
        subject: String,
        /// Authority scope it applies within (own|peers|department|direct_reports|descendants|org)
        #[arg(long)]
        scope: String,
        /// Employee id of the decider
        #[arg(long, default_value = "user")]
        decided_by: String,
        /// Why this is the right call
        #[arg(long)]
        rationale: String,
        /// Record an advisory decision instead of a binding one (§7 makes
        /// CEO/head decisions binding by default, so pass this to opt out)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        non_binding: bool,
        /// Why this decision is being proposed (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Accept a proposed decision
    Accept {
        /// Decision id
        decision_id: String,
        /// Why it is being accepted (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Reject a proposed decision
    Reject {
        /// Decision id
        decision_id: String,
        /// Why it is being rejected (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Record a dissent against a decision
    Dissent {
        /// Decision id
        decision_id: String,
        /// Employee id holding the objection
        #[arg(long)]
        employee_id: String,
        /// What their stance is (support|oppose|abstain|...)
        #[arg(long)]
        position: String,
        /// Why they hold it (required; an objection with no stated reason
        /// cannot be weighed later)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// List decisions (read-only)
    List {
        /// Filter by status (proposed|accepted|rejected|expired)
        #[arg(long)]
        status: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Capability grant commands. §2.1: hierarchy is sufficient inside a
/// department, so a grant is the only way to cross a boundary — and it is
/// always attributable, reasoned, and optionally expiring.
#[derive(Debug, Clone, Subcommand)]
pub enum OrgGrantAction {
    /// Record a grant
    Give {
        /// Capability being granted (e.g. "content.tooling")
        capability: String,
        /// Employee id receiving the grant
        #[arg(long)]
        grantee: String,
        /// Employee id making the grant
        #[arg(long, default_value = "user")]
        grantor: String,
        /// Scope being extended (own|peers|department|direct_reports|descendants|org)
        #[arg(long)]
        scope: String,
        /// Restrict the grant to one department
        #[arg(long)]
        target_dept: Option<String>,
        /// RFC3339 expiry; omit for a standing grant (the exception, not the default)
        #[arg(long)]
        expires_at: Option<String>,
        /// Why this grant is being made (required; a grant without one is not attributable)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Revoke a grant
    Revoke {
        /// Grant id
        grant_id: String,
        /// Why it is being revoked (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// List a grantee's grants (read-only)
    List {
        /// Employee id
        #[arg(long)]
        grantee: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// DM thread commands. The budget is shared and visible: both participants
/// see the same counter, and exhausting it closes the thread with a recorded
/// reason rather than dropping messages (§10.3).
#[derive(Debug, Clone, Subcommand)]
pub enum OrgDmAction {
    /// Open a thread with a shared turn budget
    Open {
        /// First participant
        a: String,
        /// Second participant
        b: String,
        /// Shared turn budget (default 3)
        #[arg(long)]
        budget: Option<u32>,
        /// Anchor the thread to an existing notice correlation_id
        #[arg(long)]
        correlation_id: Option<String>,
    },

    /// Spend one turn; refuses a turn past the budget
    Spend {
        /// Thread id
        thread_id: String,
    },

    /// Close a thread early
    Close {
        /// Thread id
        thread_id: String,
        /// Why it is being closed (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// List an employee's open threads (read-only)
    List {
        /// Employee id
        #[arg(long)]
        employee_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}
///
/// The `org` subcommands, per the §3.5 table.
///
/// Mutating variants carry `#[arg(long, value_name = "REASON")] reason: String`.
/// A bare `String` in a clap derive variant is already required — that is
/// the pattern §3.5 prescribes verbatim, and it means a missing reason is
/// rejected by clap before any I/O, not by a handler check.
#[derive(Debug, Clone, Subcommand)]
pub enum OrgSubcommand {
    /// Employee registry (§3.1)
    #[command(subcommand)]
    Employee(OrgEmployeeAction),

    /// Notice board (§3.3)
    #[command(subcommand)]
    Notice(OrgNoticeAction),

    /// Worklog (§3.4)
    #[command(subcommand)]
    Worklog(OrgWorklogAction),

    /// Departments: mandate, protocols, rules, seats (§3.4)
    #[command(subcommand)]
    Department(OrgDepartmentAction),

    /// Decisions and their dissent (§7)
    #[command(subcommand)]
    Decision(OrgDecisionAction),

    /// Capability grants that cross a department boundary (§2.5)
    #[command(subcommand)]
    Grant(OrgGrantAction),

    /// Employee DM threads and their shared turn budget (§10.3)
    #[command(subcommand)]
    Dm(OrgDmAction),

    /// Backfill employees from the live cron jobs (§3.1.1)
    Sync {
        /// Why this backfill is being run (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// One-shot import from a foreign organism tree (§4.3 — open product decision)
    Import {
        /// Path to the foreign organism tree
        path: PathBuf,
        /// Why this import is being run (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Report on the registry: counts and the fail-closed gate's input (read-only)
    Check,

    /// List employees (read-only)
    List {
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum OrgEmployeeAction {
    /// Create an employee row
    Create {
        /// Employee id (e.g. "emp-a02e3f692fb0")
        employee_id: String,
        /// Human-readable name
        name: String,
        /// Role (default: "automator", §3.1)
        #[arg(long, default_value = "automator")]
        role: String,
        /// Department slug (NULL until Wave 3)
        #[arg(long)]
        department: Option<String>,
        /// Comma-separated skill leaf names
        #[arg(long, default_value = "")]
        skills: String,
        /// Agent type: session | service | fixer (absence stays absence, §3.1.2)
        #[arg(long, value_name = "AGENT_TYPE")]
        agent_type: Option<String>,
        /// Why this employee exists (required; stored in employees.reason)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Update an employee's mutable fields
    ///
    /// **Known limitation, left visible on purpose:** a field cannot be set
    /// back to NULL. `COALESCE` cannot distinguish "flag not passed" from
    /// "flag passed as null" through the CLI, and a per-column `--clear-*`
    /// flag is not in §3.5's table. If a field must be cleared, retire the
    /// employee and create a new one — the registry is append-oriented and
    /// §3.2's gate keys on presence, not nullability.
    Update {
        /// Employee id
        employee_id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New role
        #[arg(long)]
        role: Option<String>,
        /// New department slug
        #[arg(long)]
        department: Option<String>,
        /// New comma-separated skill leaf names
        #[arg(long)]
        skills: Option<String>,
        /// New agent type
        #[arg(long, value_name = "AGENT_TYPE")]
        agent_type: Option<String>,
        /// Why this change is being made (required; stored in employees.reason)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Retire an employee
    Retire {
        /// Employee id
        employee_id: String,
        /// Why this employee is being retired (required; stored in employees.reason)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum OrgNoticeAction {
    /// Post a notice to the board
    Post {
        /// Body of the notice
        body: String,
        /// Sender (employee_id, 'user', or 'system')
        #[arg(long, default_value = "user")]
        sender: String,
        /// Recipient selectors: agent:<id>, dept:<slug>, role:<cap>, broadcast
        #[arg(long, default_value = "broadcast")]
        recipients: String,
        /// Subject line
        #[arg(long)]
        subject: Option<String>,
        /// Sender's department (provenance)
        #[arg(long)]
        from_dept: Option<String>,
        /// Tags (comma-separated)
        #[arg(long, default_value = "")]
        tags: String,
        /// Correlation id chaining request -> ack -> result
        #[arg(long)]
        correlation_id: Option<String>,
        /// Require an acknowledgement
        #[arg(long, action = clap::ArgAction::SetTrue)]
        ack_required: bool,
        /// Protect this notice from retention GC
        #[arg(long, action = clap::ArgAction::SetTrue)]
        pinned: bool,
        /// TTL expiry, RFC3339
        #[arg(long)]
        ttl_expires_at: Option<String>,
        /// Why this notice is being posted (required; stored in notices.reason)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Acknowledge a notice
    Ack {
        /// Notice id
        notice_id: String,
        /// Who is acking (employee_id or 'user')
        #[arg(long, default_value = "user")]
        sender: String,
        /// Why this ack is being sent (required; stored on the ack row)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Pin a notice against retention GC
    Pin {
        /// Notice id
        notice_id: String,
        /// Why this notice is being pinned (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// Unpin a notice
    Unpin {
        /// Notice id
        notice_id: String,
        /// Why this notice is being unpinned (required)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// List notices (read-only — no --reason)
    List {
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum OrgWorklogAction {
    /// Append a manual worklog entry
    ///
    /// §3.4 is explicit that the **framework** appends the worklog at session
    /// end and "never the agent". This subcommand is a human operator's
    /// escape hatch, deliberately separate from that path, and it is the
    /// only way a reason ever lands in `worklog.reason` today.
    Append {
        /// Who is writing this entry (employee_id, 'user', or 'system')
        #[arg(long, default_value = "user")]
        sender: String,
        /// Entry body
        body: String,
        /// Why this entry is being written (required; stored in worklog.reason)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },

    /// List worklog entries (read-only — no --reason)
    List {
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
}

fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn now_epoch() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A row id for the notices/worklog tables.
///
/// §3.3/§3.4 specify `'n_' || uuid v4` as the organism's convention, but the
/// columns are plain `TEXT PRIMARY KEY` and nothing reads the id across
/// processes. Clock + pid is enough to keep two notices posted inside the
/// same millisecond from colliding, and it keeps this packet free of a new
/// dependency. Swap for a uuid when the inbox bridge starts correlating
/// across processes.
fn new_id(prefix: &str) -> String {
    format!(
        "{}_{}_{}",
        prefix,
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    )
}

fn csv_to_json_array(raw: &str) -> String {
    let items: Vec<String> = csv_list(raw)
        .iter()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string()))
        .collect();
    format!("[{}]", items.join(","))
}

/// Split a comma-separated CLI list into trimmed, non-empty items.
///
/// Blank entries are dropped rather than stored: `--tags "a,,b"` is a typo
/// the shell made, and an empty tag would silently widen nothing while
/// widening the row. Used by `--tags`, `--skills`, and `--recipients`.
fn csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parse `--recipients` through `Recipient::parse`, so each entry becomes a
/// typed selector (`agent:` / `dept:` / `team:` / `role:` / `broadcast`).
///
/// An empty list yields a single `Broadcast` — the organism's default when a
/// notice names no recipient. Note that the typed selectors are stored but
/// **not yet expanded**: `dept:` and `team:` do not resolve to employees
/// until Wave 2 ships the recipient resolver, so such a notice is visible on
/// the board but reaches nobody's inbox. That gap is recorded in
/// `docs/CHRONOGRAPH-DESIGN.md`, not hidden here.
fn parse_recipients(raw: &str) -> Result<Vec<Recipient>> {
    let items = csv_list(raw);
    if items.is_empty() {
        return Ok(vec![Recipient::Broadcast]);
    }
    items
        .iter()
        .map(|item| Recipient::parse(item).map_err(|e| anyhow::anyhow!("{e} (in --recipients)")))
        .collect()
}

/// Open the kanban database and guarantee the org tables exist.
///
/// Returns the **shared** handle. The DDL is owned by
/// `operant-core/src/org/`: this opens the file once and then hands the
/// connection to each core store in turn, so each applies its own table
/// definition and there is exactly one definition of the schema in the tree.
/// An earlier revision applied a second, locally-transcribed copy here,
/// which had already drifted from core's on the `worklog` table (see the
/// module doc).
fn open_org_db(config: &AppConfig) -> Result<OrgConn> {
    let path = org_db_path(&config.database_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create org db directory {}", parent.display()))?;
    }
    let conn = rusqlite::Connection::open(&path)
        .with_context(|| format!("Failed to open org db {}", path.display()))?;
    let conn = std::sync::Arc::new(std::sync::Mutex::new(conn));
    // Each of these is idempotent (`CREATE TABLE IF NOT EXISTS`) and is the
    // sole definition of its table. `conn()` hands the handle straight back
    // so the caller's raw SQL runs against the same connection.
    let _ = operant_core::org::employee_db::EmployeeDb::from_shared_connection(conn.clone())
        .context("Failed to apply employees schema")?;
    let _ = operant_core::org::worklog_db::WorklogDb::from_shared_connection(conn.clone())
        .context("Failed to apply worklog schema")?;
    let _ = NoticeBoard::from_shared_connection(conn.clone())
        .context("Failed to apply notices schema")?;
    // Wave 2-3 stores (iter-539). Same rule as above: each is idempotent and
    // is the sole definition of its own table, on the same shared handle so
    // there is still exactly one writer per file.
    let _ = operant_core::org::department_db::DepartmentDb::from_shared_connection(conn.clone())
        .context("Failed to apply departments schema")?;
    let _ = operant_core::org::authority::GrantDb::from_shared_connection(conn.clone())
        .context("Failed to apply authority grants schema")?;
    let _ = operant_core::org::dm_thread::DmThreadDb::from_shared_connection(conn.clone())
        .context("Failed to apply dm thread schema")?;
    // `decisions` deliberately does NOT come along: it lives in its own
    // `operant_decisions.db` sibling so it can never claim a PRAGMA
    // user_version on the kanban family file. It is opened on demand by
    // `org decision ...` via `DecisionsDb::for_app`.
    Ok(conn)
}

/// The org tables' shared sqlite handle: one file, one connection, so the
/// CLI's raw reads and the core stores' typed writes cannot deadlock against
/// a second writer on the same file.
pub type OrgConn = std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>;

/// The live employee roster, read through core's typed store rather than
/// hand-written SQL.
///
/// `staffing_findings` needs `&[Employee]` to answer "is this department's
/// head seat actually filled", so the rows have to be the real typed values.
/// Going through `EmployeeDb` also means the CLI cannot drift from core's
/// column list, which is the same reasoning `open_org_db` applies to DDL.
fn employee_roster(conn: &OrgConn) -> Result<Vec<operant_core::org::employee::Employee>> {
    let db = operant_core::org::employee_db::EmployeeDb::from_shared_connection(conn.clone())
        .context("Failed to open employee store")?;
    db.list_employees()
        .context("Failed to read employee roster")
}

/// Build a worklog row for an operator-run `org worklog append`.
///
/// `WorklogEntry` has private fields and exactly one constructor,
/// `TurnObservation::into_entry`, so there is no way to write a row that did
/// not come through the framework's observation path. A human running a CLI
/// command is framework-level by definition — this is the operator's turn
/// observation, not a model's — so the honest way to record it is to build
/// the `TurnEnd` the framework would have emitted and let the same
/// constructor run.
///
/// The `reason` rides along as a blocker-free artifact of the turn rather
/// than being dropped: §3.5 requires the reason and the effect in one
/// transaction, and the worklog has no `reason` column of its own (it is not
/// in the organism's 20). Carrying it in `artifacts` keeps it queryable
/// without inventing a 21st field.
fn operator_entry(sender: &str, body: &str, reason: &str) -> operant_core::org::WorklogEntry {
    use operant_core::org::worklog::TurnObservation;
    use operant_core::turn_end::TurnEnd;

    let turn = TurnEnd {
        turn_id: 0,
        session_id: format!("cli:{}", sender),
        iterations: 1,
        tool_calls: 0,
        tool_durations_ms: Vec::new(),
        result_summary: body.to_string(),
        result_truncated: false,
    };
    TurnObservation::from_turn_end(turn)
        .with_employee(sender)
        .with_artifacts(vec![format!("reason: {reason}")])
        .into_entry()
}

/// Dispatch an `org` subcommand.
pub async fn handle_org_command(config: &AppConfig, cmd: OrgSubcommand) -> Result<()> {
    match cmd {
        OrgSubcommand::Employee(action) => handle_employee(config, action).await,
        OrgSubcommand::Notice(action) => handle_notice(config, action).await,
        OrgSubcommand::Worklog(action) => handle_worklog(config, action).await,
        OrgSubcommand::Department(action) => handle_department(config, action),
        OrgSubcommand::Decision(action) => handle_decision(config, action),
        OrgSubcommand::Grant(action) => handle_grant(config, action),
        OrgSubcommand::Dm(action) => handle_dm(config, action),
        OrgSubcommand::Sync { reason } => cmd_sync(config, &reason),
        OrgSubcommand::Import { path, reason } => cmd_import(config, &path, &reason),
        OrgSubcommand::Check => cmd_check(config),
        OrgSubcommand::List { json } => cmd_employee_list(config, json),
    }
}

fn handle_department(config: &AppConfig, action: OrgDepartmentAction) -> Result<()> {
    use operant_core::org::department_db::{Department, DepartmentDb};

    let conn = open_org_db(config)?;
    let db = DepartmentDb::from_shared_connection(conn.clone())
        .context("Failed to open department store")?;

    match action {
        OrgDepartmentAction::Set(args) => {
            let OrgDepartmentSetArgs {
                dept_key,
                display_name,
                mandate,
                protocols,
                rules,
                required_capabilities,
                head_employee_id,
                target_headcount,
                reason,
            } = *args;
            // §3.5 at the write boundary: a reason that is present but empty
            // is not a reason, and must be refused here rather than stored.
            let reason = require_reason("org department set", &reason)?;
            // Preserve `created_at` across an update, and refuse to silently
            // blank a field the operator did not mention. §3.3 makes `rules`
            // enforced, so overwriting with an empty default would read as
            // "nothing declared" — the gate would then have nothing to fail.
            let existing = db.get(&dept_key)?;
            let split = |raw: &str| -> Vec<String> {
                raw.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            let (created_at, prior_rules) = match &existing {
                Some(prev) => (prev.created_at.clone(), prev.rules.clone()),
                None => (chrono::Utc::now().to_rfc3339(), Vec::new()),
            };
            if !rules.is_empty() && protocols.is_empty() && !required_capabilities.is_empty() {
                anyhow::bail!(
                    "org department set: --rules, --protocols and --required-capabilities are \
                     all listed but at least one is empty. A department with no rules is not a \
                     valid end state, so this is refused rather than stored; re-run naming only \
                     the fields you mean to change."
                );
            }
            let now = chrono::Utc::now().to_rfc3339();
            let dept = Department {
                dept_key: dept_key.clone(),
                display_name: display_name.unwrap_or_else(|| {
                    existing
                        .as_ref()
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| dept_key.clone())
                }),
                mandate: mandate.or_else(|| existing.as_ref().and_then(|p| p.mandate.clone())),
                protocols: if protocols.is_empty() {
                    existing
                        .as_ref()
                        .map(|p| p.protocols.clone())
                        .unwrap_or_default()
                } else {
                    split(&protocols)
                },
                rules: if rules.is_empty() {
                    prior_rules
                } else {
                    split(&rules)
                },
                required_capabilities: if required_capabilities.is_empty() {
                    existing
                        .as_ref()
                        .map(|p| p.required_capabilities.clone())
                        .unwrap_or_default()
                } else {
                    split(&required_capabilities)
                },
                head_employee_id: head_employee_id
                    .or_else(|| existing.as_ref().and_then(|p| p.head_employee_id.clone())),
                target_headcount: target_headcount
                    .or_else(|| existing.as_ref().and_then(|p| p.target_headcount)),
                reason: reason.to_string(),
                created_at,
                updated_at: now,
            };
            db.upsert(&dept).context("Failed to write department")?;
            println!(
                "department {} set ({} rule(s), {} protocol(s))",
                dept.dept_key,
                dept.rules.len(),
                dept.protocols.len()
            );
        }
        OrgDepartmentAction::List => {
            for dept in db.list()? {
                let head = dept
                    .head_employee_id
                    .clone()
                    .unwrap_or_else(|| "VACANT — unstaffed".to_string());
                println!(
                    "{:<20} {:<28} head={} rules={} protocol(s)",
                    dept.dept_key,
                    dept.display_name,
                    head,
                    dept.rules.len()
                );
            }
        }
        OrgDepartmentAction::Findings { json } => {
            // §11.4: findings are computed from the live `employees` rows, so
            // this reads the roster rather than the department row alone.
            let roster = employee_roster(&conn)?;
            let findings =
                operant_core::org::department_db::staffing_findings(&db.list()?, &roster);
            if json {
                println!("{}", serde_json::to_string_pretty(&findings)?);
            } else if findings.is_empty() {
                println!("no department findings");
            } else {
                for f in &findings {
                    // `StaffingFinding` has no `Display`; `detail` is the
                    // human sentence core already wrote for this exact
                    // purpose, and `kind` is the machine-readable tag.
                    println!("{:<24} {:<22} {}", f.dept_key, f.kind, f.detail);
                }
            }
            // A vacant seat is a real finding, so the exit code reflects it.
            // §11.4 makes this gate the operator's tripwire.
            if !findings.is_empty() {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

fn handle_decision(config: &AppConfig, action: OrgDecisionAction) -> Result<()> {
    use operant_core::org::decisions_db::{DecisionStatus, DecisionsDb, DissentEntry};

    let db = DecisionsDb::for_app(std::path::Path::new(&config.database_path))
        .context("Failed to open decision store")?;

    match action {
        OrgDecisionAction::Propose {
            subject,
            scope,
            decided_by,
            rationale,
            non_binding,
            reason,
        } => {
            let reason = require_reason("org decision propose", &reason)?;
            let mut decision = operant_core::org::decisions_db::OrgDecision::new(
                new_id("d"),
                subject,
                scope,
                decided_by,
                rationale,
                reason,
            );
            // §7 makes CEO/head decisions binding by default while preserving
            // visible dissent, so binding is the default and --non-binding is
            // the explicit opt-out.
            decision.binding = !non_binding;
            db.propose(&decision)
                .context("Failed to propose decision")?;
            println!(
                "decision {} proposed: {} (binding={})",
                decision.decision_id, decision.subject, decision.binding
            );
        }
        OrgDecisionAction::Accept {
            decision_id,
            reason,
        } => {
            let reason = require_reason("org decision accept", &reason)?;
            db.accept(&decision_id, reason)
                .context("Failed to accept decision")?;
            println!("decision {decision_id} accepted");
        }
        OrgDecisionAction::Reject {
            decision_id,
            reason,
        } => {
            let reason = require_reason("org decision reject", &reason)?;
            db.reject(&decision_id, reason)
                .context("Failed to reject decision")?;
            println!("decision {decision_id} rejected");
        }
        OrgDecisionAction::Dissent {
            decision_id,
            employee_id,
            position,
            reason,
        } => {
            let reason = require_reason("org decision dissent", &reason)?;
            // `DissentEntry::new` is the only constructor and it refuses a
            // blank employee, position, or reason — an objection nobody holds
            // is noise, and an objection with no stated reason cannot be
            // weighed later, which is how dissent gets quietly dropped.
            let entry =
                DissentEntry::new(&employee_id, &position, reason).context("Invalid dissent")?;
            db.record_dissent(&decision_id, entry)
                .context("Failed to record dissent")?;
            println!("dissent by {employee_id} recorded on {decision_id}");
        }
        OrgDecisionAction::List { status, json } => {
            let rows = match status.as_deref() {
                Some(raw) => {
                    let parsed = DecisionStatus::parse(raw).ok_or_else(|| {
                        anyhow::anyhow!(
                            "unknown decision status '{raw}' (expected proposed|accepted|rejected|expired)"
                        )
                    })?;
                    db.list_by_status(parsed)?
                }
                None => {
                    let mut all = Vec::new();
                    for s in [
                        DecisionStatus::Proposed,
                        DecisionStatus::Accepted,
                        DecisionStatus::Rejected,
                        DecisionStatus::Expired,
                    ] {
                        all.extend(db.list_by_status(s)?);
                    }
                    all
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                println!("no decisions");
            } else {
                for d in rows {
                    println!(
                        "{:<10} {:<40} {}",
                        d.status.as_str(),
                        d.subject,
                        d.decision_id
                    );
                }
            }
        }
    }
    Ok(())
}

fn handle_grant(config: &AppConfig, action: OrgGrantAction) -> Result<()> {
    use operant_core::org::authority::{AuthorityScope, Grant, GrantDb};

    let conn = open_org_db(config)?;
    let db = GrantDb::from_shared_connection(conn.clone()).context("Failed to open grant store")?;

    match action {
        OrgGrantAction::Give {
            capability,
            grantee,
            grantor,
            scope,
            target_dept,
            expires_at,
            reason,
        } => {
            let reason = require_reason("org grant give", &reason)?;
            let scope_parsed: AuthorityScope = scope.parse().map_err(|_| {
                anyhow::anyhow!(
                    "unknown authority scope '{scope}' (expected own|peers|department|\
                     direct_reports|descendants|org)"
                )
            })?;
            let grant = Grant {
                grant_id: new_id("ag"),
                grantor: grantor.clone(),
                grantee: grantee.clone(),
                capability: capability.clone(),
                scope: scope_parsed,
                target_dept: target_dept.clone(),
                reason: reason.to_string(),
                granted_at: chrono::Utc::now().to_rfc3339(),
                expires_at: expires_at.clone(),
                revoked_at: None,
                revocation_reason: None,
            };
            db.insert(&grant).context("Failed to record grant")?;
            println!(
                "granted {} to {} (scope {}, grantor {})",
                grant.capability,
                grant.grantee,
                grant.scope.as_str(),
                grant.grantor
            );
        }
        OrgGrantAction::Revoke { grant_id, reason } => {
            let reason = require_reason("org grant revoke", &reason)?;
            let removed = db
                .revoke(&grant_id, reason)
                .context("Failed to revoke grant")?;
            if removed {
                println!("grant {grant_id} revoked");
            } else {
                println!("grant {grant_id} not found or already revoked");
            }
        }
        OrgGrantAction::List { grantee, json } => {
            let rows = db.list_for_grantee(&grantee)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                println!("no grants for {grantee}");
            } else {
                for g in rows {
                    let state = if g.is_revoked() {
                        "REVOKED"
                    } else if g.is_live_at(&chrono::Utc::now().to_rfc3339()) {
                        "live"
                    } else {
                        "lapsed"
                    };
                    println!(
                        "{:<26} {:<12} {:<10} {}",
                        g.capability,
                        g.scope.as_str(),
                        state,
                        g.grant_id
                    );
                }
            }
        }
    }
    Ok(())
}

fn handle_dm(config: &AppConfig, action: OrgDmAction) -> Result<()> {
    use operant_core::org::dm_thread::{DEFAULT_TURN_BUDGET, DmThreadDb};

    let conn = open_org_db(config)?;
    let db = DmThreadDb::from_shared_connection(conn.clone())
        .context("Failed to open dm thread store")?;

    match action {
        OrgDmAction::Open {
            a,
            b,
            budget,
            correlation_id,
        } => {
            let budget = budget.unwrap_or(DEFAULT_TURN_BUDGET);
            let thread = db
                .open_with_correlation(&a, &b, budget, correlation_id)
                .context("Failed to open dm thread")?;
            println!(
                "thread {} open: {} turns, {} used",
                thread.thread_id, thread.turn_budget, thread.turns_used
            );
        }
        OrgDmAction::Spend { thread_id } => {
            let budget = db
                .spend_turn(&thread_id)
                .context("Failed to spend dm turn")?;
            println!(
                "thread {thread_id}: {}/{} turns used",
                budget.used, budget.total
            );
        }
        OrgDmAction::Close { thread_id, reason } => {
            let reason = require_reason("org dm close", &reason)?;
            let thread = db
                .close(&thread_id, reason)
                .context("Failed to close dm thread")?;
            println!(
                "thread {} closed ({})",
                thread.thread_id,
                thread.state.as_str()
            );
        }
        OrgDmAction::List { employee_id, json } => {
            let rows = db.open_threads_for(&employee_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                println!("no open threads for {employee_id}");
            } else {
                for t in rows {
                    println!(
                        "{:<24} {}/{} turns  {}",
                        t.thread_id,
                        t.turns_used,
                        t.turn_budget,
                        t.state.as_str()
                    );
                }
            }
        }
    }
    Ok(())
}

async fn handle_employee(config: &AppConfig, action: OrgEmployeeAction) -> Result<()> {
    match action {
        OrgEmployeeAction::Create {
            employee_id,
            name,
            role,
            department,
            skills,
            agent_type,
            reason,
        } => {
            let reason = validated_reason("org employee create", &reason)?;
            let conn = open_org_db(config)?;
            let conn = conn
                .lock()
                .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
            let now = rfc3339_now();
            conn.execute(
                "INSERT INTO employees
                     (employee_id, name, role, department, skills, agent_type,
                      status, reason, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?8)",
                rusqlite::params![
                    employee_id,
                    name,
                    role,
                    department,
                    csv_to_json_array(&skills),
                    agent_type,
                    reason,
                    now
                ],
            )
            .context("Failed to create employee")?;
            println!("Employee '{}' created.", employee_id);
            Ok(())
        }

        OrgEmployeeAction::Update {
            employee_id,
            name,
            role,
            department,
            skills,
            agent_type,
            reason,
        } => {
            let reason = validated_reason("org employee update", &reason)?;
            if name.is_none()
                && role.is_none()
                && department.is_none()
                && skills.is_none()
                && agent_type.is_none()
            {
                // Checked *after* require_reason and *before* the write, so
                // a blank reason is reported as a blank reason. Reversing
                // the two would make "you passed --reason ''" the only
                // feedback for a command that changes nothing.
                anyhow::bail!(
                    "nothing to update: pass at least one of --name, --role, \
                     --department, --skills, --agent-type"
                );
            }
            let conn = open_org_db(config)?;
            let conn = conn
                .lock()
                .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
            // The reason and the field change land in one statement, so a
            // mutation can never be committed without its cause.
            conn.execute(
                "UPDATE employees SET
                     name       = COALESCE(?2, name),
                     role       = COALESCE(?3, role),
                     department = COALESCE(?4, department),
                     skills     = COALESCE(?5, skills),
                     agent_type = COALESCE(?6, agent_type),
                     reason     = ?7,
                     updated_at = ?8
                 WHERE employee_id = ?1",
                rusqlite::params![
                    employee_id,
                    name,
                    role,
                    department,
                    skills.map(|s| csv_to_json_array(&s)),
                    agent_type,
                    reason,
                    rfc3339_now()
                ],
            )
            .context("Failed to update employee")?;
            println!("Employee '{}' updated.", employee_id);
            Ok(())
        }

        OrgEmployeeAction::Retire {
            employee_id,
            reason,
        } => {
            let reason = validated_reason("org employee retire", &reason)?;
            let conn = open_org_db(config)?;
            let conn = conn
                .lock()
                .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
            let changed = conn
                .execute(
                    "UPDATE employees SET status = 'retired', reason = ?2, updated_at = ?3
                     WHERE employee_id = ?1",
                    rusqlite::params![employee_id, reason, rfc3339_now()],
                )
                .context("Failed to retire employee")?;
            if changed == 0 {
                anyhow::bail!("no employee with id '{}'", employee_id);
            }
            println!("Employee '{}' retired.", employee_id);
            Ok(())
        }
    }
}

async fn handle_notice(config: &AppConfig, action: OrgNoticeAction) -> Result<()> {
    match action {
        OrgNoticeAction::Post {
            body,
            sender,
            recipients,
            subject,
            from_dept,
            tags,
            correlation_id,
            ack_required,
            pinned,
            ttl_expires_at,
            reason,
        } => {
            let reason = validated_reason("org notice post", &reason)?;
            let conn = open_org_db(config)?;
            let board = NoticeBoard::from_shared_connection(conn)
                .context("Failed to open the notice board")?;
            // Through `NoticeBoard::post`, never a hand-written INSERT. An
            // earlier version wrote this INSERT in the CLI and it had already
            // diverged from core: it stored `--correlation-id` into BOTH
            // `correlation_id` and `thread_id` (reusing ?9 for two columns),
            // left `acked_by`/`acked_at` NULL, and produced `subject` in the
            // body position. Recipients are parsed through `Recipient::parse`
            // so a typo like `dept:` fails at post time instead of silently
            // addressing nobody — see `Recipient::parse`.
            let notice = PostNotice {
                sender,
                from_dept,
                recipients: parse_recipients(&recipients)?,
                subject,
                body,
                correlation_id,
                ack_required,
                tags: csv_list(&tags),
                thread_id: None,
                pinned,
                ttl: None,
                ttl_expires_at: ttl_expires_at.and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .ok()
                        .map(|d| d.with_timezone(&chrono::Utc))
                }),
                reason: reason.to_string(),
                metadata: None,
            };
            let posted = board.post(&notice).context("Failed to post notice")?;
            println!("Notice '{}' posted.", posted.id);
            Ok(())
        }

        OrgNoticeAction::Ack {
            notice_id,
            sender,
            reason,
        } => {
            validated_reason("org notice ack", &reason)?;
            let conn = open_org_db(config)?;
            let board = NoticeBoard::from_shared_connection(conn)
                .context("Failed to open the notice board")?;
            // §3.3: an ack is a `notice_post` carrying the same
            // `correlation_id` and the `ack` tag — the organism's
            // request -> ack -> result protocol — *plus* a direct UPDATE on
            // the parent so inbox queries never have to join. Core's `ack`
            // does both in one transaction (an ack row without the parent
            // update, or the reverse, is a protocol lie) and is idempotent:
            // a repeated ack by the same employee writes nothing at all.
            board
                .ack(&notice_id, &sender)
                .context("Failed to acknowledge notice")?;
            println!("Notice '{}' acknowledged by '{}'.", notice_id, sender);
            Ok(())
        }

        OrgNoticeAction::Pin { notice_id, reason } => {
            let reason = validated_reason("org notice pin", &reason)?;
            set_pinned(config, &notice_id, true, reason, "pin")
        }

        OrgNoticeAction::Unpin { notice_id, reason } => {
            let reason = validated_reason("org notice unpin", &reason)?;
            set_pinned(config, &notice_id, false, reason, "unpin")
        }

        OrgNoticeAction::List { json } => cmd_notice_list(config, json),
    }
}

/// Pin/unpin, in one transaction with the reason-bearing audit row.
///
/// The §3.3 schema has no `pin_reason` column, so the reason is recorded
/// as a tagged `notices` row on the same correlation chain — the same
/// mechanism the ack uses. That is a deliberate choice over adding a column
/// §3.3 does not have: the decision stays queryable by the tools that
/// already read the board.
fn set_pinned(
    config: &AppConfig,
    notice_id: &str,
    pinned: bool,
    reason: &str,
    tag: &str,
) -> Result<()> {
    let conn = open_org_db(config)?;
    let mut conn = conn
        .lock()
        .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
    let tx = conn
        .transaction()
        .context("Failed to begin pin transaction")?;

    let correlation_id: Option<String> = tx
        .query_row(
            "SELECT correlation_id FROM notices WHERE id = ?1",
            rusqlite::params![notice_id],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to read notice")?
        .ok_or_else(|| anyhow::anyhow!("no notice with id '{}'", notice_id))?;

    let changed = tx
        .execute(
            "UPDATE notices SET pinned = ?2 WHERE id = ?1",
            rusqlite::params![notice_id, pinned as i64],
        )
        .context("Failed to update pinned flag")?;
    if changed == 0 {
        anyhow::bail!("no notice with id '{}'", notice_id);
    }

    tx.execute(
        "INSERT INTO notices
             (id, created_at, epoch, sender, recipients, body, correlation_id,
              ack_required, tags, thread_id, reason)
         VALUES (?1, ?2, ?3, 'system', '[]', ?4, ?5, 0, ?6, ?5, ?7)",
        rusqlite::params![
            new_id("n"),
            rfc3339_now(),
            now_epoch(),
            format!("{} {}", tag, notice_id),
            correlation_id,
            format!("[\"{}\"]", tag),
            reason
        ],
    )
    .context("Failed to record pin audit row")?;

    tx.commit().context("Failed to commit pin transaction")?;
    println!(
        "Notice '{}' {}.",
        notice_id,
        if pinned { "pinned" } else { "unpinned" }
    );
    Ok(())
}

async fn handle_worklog(config: &AppConfig, action: OrgWorklogAction) -> Result<()> {
    match action {
        OrgWorklogAction::Append {
            sender,
            body,
            reason,
        } => {
            let reason = validated_reason("org worklog append", &reason)?;
            let conn = open_org_db(config)?;
            let worklog =
                operant_core::org::worklog_db::WorklogDb::from_shared_connection(conn.clone())
                    .context("Failed to open worklog")?;
            worklog
                .append(&operator_entry(&sender, &body, reason))
                .context("Failed to append worklog entry")?;
            println!("Worklog entry appended.");
            Ok(())
        }
        OrgWorklogAction::List { json } => cmd_worklog_list(config, json),
    }
}

/// `org sync` — the §3.1.1 bulk backfill.
///
/// Writes one employee row per cron job. The `--reason` is mandatory *and*
/// persisted: `EmployeeDb::backfill_from_cron_jobs` stores it verbatim on
/// every row it writes, so the registry always names the operator actually
/// accountable for the mutation. This writes a hundred-plus rows in one go —
/// the most consequential mutation in this surface — which is exactly why the
/// audit trail is the operator's own words rather than a constant.
fn cmd_sync(config: &AppConfig, reason: &str) -> Result<()> {
    let reason = validated_reason("org sync", reason)?;
    let conn = open_org_db(config)?;
    let db = operant_core::org::employee_db::EmployeeDb::from_shared_connection(conn.clone())
        .context("Failed to open the employee registry")?;
    let jobs = {
        // `cmd_cron::cron_db_path`, not a second derivation: the CLI and the
        // runtime scheduler must open the same file or backfilled employees
        // would describe jobs the scheduler never runs (R39-7).
        //
        // The parent directory has to exist first. `CronDb::init` opens the
        // file directly and SQLite cannot create a missing directory, so on a
        // fresh install `org sync` failed with a bare "Failed to open cron
        // database" — while `open_org_db` above happily created that very
        // directory for the kanban file. Same directory, different luck.
        let cron_path = crate::cmd_cron::cron_db_path(config);
        if let Some(parent) = cron_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create cron db directory {}", parent.display())
            })?;
        }
        let cron = operant_core::cronjobs::db::CronDb::init(cron_path)
            .context("Failed to open the cron database")?;
        cron.list_jobs(true).context("Failed to list cron jobs")?
    };
    let report = db
        .backfill_from_cron_jobs(&jobs, &rfc3339_now(), reason)
        .context("Failed to backfill the employee registry")?;

    println!(
        "org sync: {} job(s) seen, {} employee row(s) written, {} link(s) written (reason: {})",
        report.jobs_seen, report.employees_written, report.links_written, reason
    );
    // §3.1.2: a collision and a row failing the required set are both
    // reported, never hidden. Against the real organism 4 of 102 rows are
    // invalid (two disabled legacy jobs, two enabled empty-prompt
    // placeholders — the six-candidate figure was measured before the
    // singular-`skill` fallback, which correctly cleared two false
    // positives). A non-empty list here is the expected steady state, not a
    // failure of the sync.
    if !report.invalid.is_empty() {
        println!(
            "  {} row(s) missing required fields (these are what the org gate blocks):",
            report.invalid.len()
        );
        for row in &report.invalid {
            println!(
                "    {} {} missing: {}",
                row.employee_id,
                row.cron_job_id,
                row.missing.join(", ")
            );
        }
    }
    if !report.collisions.is_empty() {
        println!("  {} employee_id collision(s):", report.collisions.len());
        for row in &report.collisions {
            println!(
                "    {} derived {} already owned by {}",
                row.cron_job_id, row.employee_id, row.conflicting_cron_job_id
            );
        }
    }
    Ok(())
}

/// `org import <path>` — one-shot import from a foreign organism tree.
///
/// **Not implemented.** §4.3 of the decision document records in-place
/// reads of the organism tree as an open *product* decision the owner has
/// not made, and no importer exists. The reason is required and validated,
/// and nothing is written — so the command fails loudly rather than
/// pretending to have imported a tree. This is deliberately different from
/// `org sync`, which *is* wired and does write rows.
fn cmd_import(_config: &AppConfig, _path: &Path, reason: &str) -> Result<()> {
    require_reason("org import", reason)?;
    anyhow::bail!(
        "org import is not wired yet: §4.3 of WAVE1-DECISIONS records \
         importing the organism tree as an open product decision, and no \
         importer exists. The --reason is parsed and validated so the clap \
         contract holds, but no rows are written."
    )
}

fn cmd_check(config: &AppConfig) -> Result<()> {
    let conn = open_org_db(config)?;
    let conn = conn
        .lock()
        .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
    let count = |sql: &str| -> Result<i64> {
        conn.query_row(sql, [], |row| row.get(0))
            .with_context(|| format!("query failed: {}", sql))
    };
    let total = count("SELECT COUNT(*) FROM employees")?;
    let active = count("SELECT COUNT(*) FROM employees WHERE status = 'active'")?;
    let no_skills = count("SELECT COUNT(*) FROM employees WHERE skills = '[]'")?;
    let notices = count("SELECT COUNT(*) FROM notices")?;
    let blank_reasons = count("SELECT COUNT(*) FROM employees WHERE TRIM(reason) = ''")?;
    let worklog = count("SELECT COUNT(*) FROM worklog")?;
    println!("org check");
    println!("  employees:       {total} ({active} active)");
    println!("  missing skills:  {no_skills}   (fail-closed gate input, §2.2)");
    println!("  blank reasons:   {blank_reasons}");
    println!("  notices:         {notices}");
    println!("  worklog:         {worklog}");
    Ok(())
}

fn cmd_employee_list(config: &AppConfig, json: bool) -> Result<()> {
    let conn = open_org_db(config)?;
    let conn = conn
        .lock()
        .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
    let mut stmt = conn
        .prepare(
            "SELECT employee_id, name, role, department, status, agent_type, reason
             FROM employees ORDER BY name",
        )
        .context("Failed to prepare employee list")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "employee_id": row.get::<_, String>(0)?,
                "name":         row.get::<_, String>(1)?,
                "role":         row.get::<_, String>(2)?,
                "department":   row.get::<_, Option<String>>(3)?,
                "status":       row.get::<_, String>(4)?,
                "agent_type":   row.get::<_, Option<String>>(5)?,
                "reason":       row.get::<_, String>(6)?,
            }))
        })
        .context("Failed to query employees")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to read employee rows")?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
        );
    } else {
        for r in rows {
            println!(
                "  {:<20} {:<28} [{:<8}] {}",
                r["employee_id"].as_str().unwrap_or("?"),
                r["name"].as_str().unwrap_or("?"),
                r["status"].as_str().unwrap_or("?"),
                r["reason"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

fn cmd_notice_list(config: &AppConfig, json: bool) -> Result<()> {
    let conn = open_org_db(config)?;
    let conn = conn
        .lock()
        .map_err(|_| anyhow::anyhow!("org db mutex poisoned"))?;
    let mut stmt = conn
        .prepare("SELECT id, sender, subject, pinned, reason FROM notices ORDER BY created_at DESC")
        .context("Failed to prepare notice list")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id":      row.get::<_, String>(0)?,
                "sender":  row.get::<_, String>(1)?,
                "subject": row.get::<_, Option<String>>(2)?,
                "pinned":  row.get::<_, i64>(3)? != 0,
                "reason":  row.get::<_, String>(4)?,
            }))
        })
        .context("Failed to query notices")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to read notice rows")?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
        );
    } else {
        for r in rows {
            println!(
                "  {:<20} [{:<8}] {:<40} {}",
                r["id"].as_str().unwrap_or("?"),
                r["sender"].as_str().unwrap_or("?"),
                r["subject"].as_str().unwrap_or(""),
                r["reason"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

fn cmd_worklog_list(config: &AppConfig, json: bool) -> Result<()> {
    let conn = open_org_db(config)?;
    let worklog = operant_core::org::worklog_db::WorklogDb::from_shared_connection(conn.clone())
        .context("Failed to open worklog")?;
    let records = worklog
        .list(&operant_core::org::worklog_db::WorklogQuery::default())
        .context("Failed to read worklog rows")?;
    let rows: Vec<serde_json::Value> = records
        .iter()
        .map(|r| {
            serde_json::json!({
                "id":             r.id,
                "ts":             r.ts,
                "ts_iso":         r.ts_iso,
                "employee":       r.employee,
                "what_done":      r.what_done,
                "outcome":        r.outcome,
                "artifacts":      r.artifacts,
                "tokens_in":      r.tokens_in,
                "tokens_out":     r.tokens_out,
                "tool_calls":     r.tool_calls,
                "session_id":     r.session_id,
            })
        })
        .collect();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
        );
    } else {
        for r in rows {
            println!(
                "  {:<20} {:<12} {:<10} {}",
                r["id"].as_str().unwrap_or("?"),
                r["employee"].as_str().unwrap_or(""),
                r["outcome"].as_str().unwrap_or(""),
                r["what_done"].as_str().unwrap_or("")
            );
            // The `--reason` of an operator append rides in `artifacts` (see
            // `operator_entry`). Hiding it would be the AD-032 failure this
            // whole command exists to prevent: a logged decision nobody can
            // read back.
            if let Some(arts) = r["artifacts"].as_array() {
                for a in arts {
                    if let Some(s) = a.as_str() {
                        println!("      {}", s);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_reason_rejects_empty_and_whitespace_only() {
        for bad in ["", " ", "\t", "\n", "   \t\n  ", "\u{00a0}"] {
            assert!(
                require_reason("org employee create", bad).is_err(),
                "a blank reason must be rejected, got {bad:?}"
            );
        }
    }

    #[test]
    fn org_reason_accepts_real_values_and_returns_them_untrimmed() {
        // The stored value is what the operator typed. Trimming here would
        // quietly rewrite the audit record.
        assert_eq!(
            require_reason("org sync", "  upstream 429  ").unwrap(),
            "  upstream 429  "
        );
        assert_eq!(require_reason("org sync", "x").unwrap(), "x");
    }

    #[test]
    fn org_reason_error_names_the_flag_and_the_command() {
        let msg = require_reason("org notice post", "  ")
            .unwrap_err()
            .to_string();
        assert!(msg.contains("--reason"), "must name the flag: {msg}");
        assert!(
            msg.contains("org notice post"),
            "must name the command: {msg}"
        );
    }

    #[test]
    fn org_reason_db_path_is_the_kanban_sibling() {
        let p = org_db_path(std::path::Path::new("/var/lib/operant/operant.db"));
        assert_eq!(
            p,
            std::path::Path::new("/var/lib/operant/operant_kanban.db")
        );
    }

    #[test]
    fn org_reason_csv_to_json_array_drops_blanks() {
        assert_eq!(csv_to_json_array("a, b ,,c"), r#"["a","b","c"]"#);
        assert_eq!(csv_to_json_array(""), "[]");
        assert_eq!(csv_to_json_array("  "), "[]");
    }
}
