//! The write barrier — a run does not complete until its record exists.
//!
//! Spec: `docs/ORG-AUTHORITY-ARCHITECTURE.md` §7 ("the write barrier — logs
//! are a precondition of completion"), §7.1 (the ordered rule), §7.3 (a
//! postcondition on `run()`), and §8 (the subjective log and the decision
//! object, which this module also writes).
//!
//! ## The rule, in order (§7.1)
//!
//! 1. Append the **objective** row (`worklog`). Exactly one per run.
//! 2. Append the **subjective** row (`subjective_log`) — the employee's own
//!    account of what it weighed. Exactly one per run.
//! 3. Append **decision objects** for anything that changed organisational
//!    state. **Zero** when nothing did.
//! 4. Only then may the run report success.
//!
//! Steps 1 and 2 are separate writes to separate tables in separate sqlite
//! files, and they are never merged. The worklog is *what happened*; the
//! subjective log is *what the employee thought*; a row carrying both answers
//! neither question. §8.1 states the split as deliberate, and the two stores
//! keep it structural rather than conventional.
//!
//! ## What this module is, and what it is not
//!
//! This is the barrier's **postcondition**, not a subscription. A caller
//! awaits [`WriteBarrier::apply`] at the end of a run and treats an `Err` as a
//! failed run:
//!
//! ```no_run
//! use operant_core::org::write_barrier::WriteBarrier;
//! use operant_core::org::worklog::OutcomeSignals;
//! use operant_core::turn_end::TurnEnd;
//!
//! let barrier = WriteBarrier::for_app(std::path::Path::new("/tmp/database.db"))?;
//! let request = WriteBarrier::request_from(TurnEnd {
//!     turn_id: 0,
//!     session_id: "sess-1".to_string(),
//!     iterations: 3,
//!     tool_calls: 7,
//!     tool_durations_ms: vec![12, 340],
//!     result_summary: "reconciled the ledger".to_string(),
//!     result_truncated: false,
//! })
//! .with_employee("emp-a02e3f692fb0")
//! .with_subjective_reasoning("weighed a rollback against a forward fix; chose forward");
//!
//! match barrier.apply(&request) {
//!     Ok(report) if report.is_clean() => { /* the run may report success */ }
//!     Ok(report) => eprintln!("record written, but: {:?}", report.findings),
//!     Err(e) => return Err(e), // §7.1: a run that cannot write is a failed run
//! }
//! # Ok::<(), operant_core::error::Error>(())
//! ```
//!
//! §7.3 wants this to be a postcondition on `run()` rather than a per-caller
//! convention, and the framework's post-turn seam
//! ([`crate::turn_end::TurnEndBus`], which the agent already emits to once
//! per turn) is where a *background* subscriber would live.
//! [`WriteBarrier::attach_subscriber`] is that subscriber.
//!
//! **The two are deliberately not the same function, and the difference is
//! load-bearing.** A `TurnEndBus` subscriber cannot make a run fail: the
//! seam's own documented contract is that it "never propagates subscriber
//! failure" — a subscriber that errors dies in its own task and the turn has
//! already returned. §7.1 asks for the opposite. So:
//!
//! - [`WriteBarrier::apply`] is the **postcondition**. It returns `Err`, and
//!   the caller fails the run.
//! - [`WriteBarrier::attach_subscriber`] is the **passive observer**. It cannot
//!   fail anything, so a failed write is pushed onto a
//!   [`BarrierFailureLog`] the caller holds and can read, and logged at
//!   `error`. A failure on that path is recorded, never dropped.
//!
//! ## Failure is visible, and nothing is pretended
//!
//! §7.1: "A run that fails to write is a **failed** run, with the write
//! failure as the error — not a successful run with a missing log." So
//! [`WriteBarrier::apply`] returns `Err` on the first failed write, names
//! *which* write failed, and never continues to the next one — writing a
//! persona-substrate entry for a run whose objective record is missing would
//! leave the substrate describing a run the worklog denies happened.
//!
//! The error also says what was **already durable**, because that is what
//! determines what a retry would duplicate: the worklog is append-only and its
//! `id` is a fresh uuid per attempt, so a retry after a partial failure
//! leaves two objective rows for one run. Duplicate, visible, dedupable —
//! which is the documented behaviour of `worklog_db::append`, and the reason
//! the barrier reports the state instead of leaving it to be discovered by
//! counting rows.
//!
//! What the barrier does **not** do is claim it prevented anything. By the
//! time it runs, the run's effects — model spend, tool calls, deliveries —
//! have already landed. The error says so in those words: this is an
//! incomplete record on a completed run, not a prevented run.
//!
//! ## Attribution is resolved here, not asserted by the caller
//!
//! The registry ([`super::employee_db::EmployeeDb`]) is the source of truth
//! for "does this `employee_id` exist". The barrier resolves the id the run
//! claims and:
//!
//! - a row exists → the worklog's `employee` and the subjective entry's
//!   `employee_id` are that id;
//! - no row → both record [`UNKNOWN_EMPLOYEE`] (WAVE1-DECISIONS §3.4: "the
//!   honest value; do not invent one") **and** the report carries a
//!   [`FindingKind::UnresolvedEmployee`];
//! - the store errors → the barrier **fails**. An unreachable registry has not
//!   proven the run's identity, which is the same fail-closed reasoning the
//!   identity gate uses (WAVE1-DECISIONS §3.2).
//!
//! A decision object is held to the same standard: if the run resolved no
//! employee, a decision with no attributable decider is **refused** rather
//! than written under `unknown`, because §8.2 makes the decision object
//! attributable and a decision nobody decided is not a decision.
//!
//! ## No `PRAGMA user_version`, ever
//!
//! The barrier writes through three existing stores in two existing files:
//! [`super::worklog_db`] and [`super::employee_db`] on the kanban sibling,
//! [`super::decisions_db`] on `operant_decisions.db`. It creates no table,
//! adds no migration, and stamps no version counter — that counter is
//! file-wide in SQLite and belongs to the kanban family alone (see
//! [`super::schema`] module docs, and §12: "New databases — fresh files, no
//! migration. So: no `PRAGMA user_version` use at all"). Pinned here by
//! `the_barrier_leaves_the_kanban_user_version_untouched`.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::turn_end::{TurnEnd, TurnEndBus};

use super::decisions_db::{DecisionsDb, OrgDecision, RunKind, SubjectiveEntry};
use super::employee_db::EmployeeDb;
use super::worklog::{OutcomeSignals, TurnObservation, UNKNOWN_EMPLOYEE};
use super::worklog_db::WorklogDb;

/// §7.1's write order, as data. The barrier walks this array; the error path
/// uses it to report which writes were already durable.
pub const BARRIER_WRITE_ORDER: [BarrierWrite; 3] = [
    BarrierWrite::Worklog,
    BarrierWrite::Subjective,
    BarrierWrite::Decision,
];

/// The `run_kind` recorded when a caller does not say what kind of run it was.
///
/// [`RunKind::Manual`] is the enum's own `Default` and its documented choice:
/// "`manual` is the honest default: a run nobody labelled." Guessing `cron`
/// here — the way `worklog.workflow_kind` defaults to `process` — would
/// attribute a human's turn to a scheduler. The barrier uses the value the
/// type already picked; a caller that *knows* says
/// [`WriteBarrierRequest::with_run_kind`].
pub const DEFAULT_BARRIER_RUN_KIND: RunKind = RunKind::Manual;

/// The id prefix on a subjective entry the barrier mints. Matches §8.2's
/// documented `'sl_' || uuid v4`, so a row written here is indistinguishable
/// in form from one written by any other framework caller.
pub const SUBJECTIVE_ID_PREFIX: &str = "sl_";

/// The `reason` stamped on a decision object whose caller supplied none.
///
/// [`DecisionsDb::propose`] refuses a blank `reason` — "an audit trail that
/// accepts a blank justification is not an audit trail". So the barrier has to
/// supply one, and the only honest thing it can say about a write it performed
/// is *that it performed it*. This is the framework's provenance, **not** the
/// decision's justification: that is [`OrgDecision::rationale`], which the
/// caller must still set and which the barrier never fills in. A caller with a
/// better reason passes one to [`WriteBarrierRequest::with_decision_reason`].
pub const BARRIER_DECISION_REASON: &str =
    "write barrier: decision recorded by the run that made it";

/// The `reasoning_digest` written when a run supplied no reasoning *and* the
/// turn produced no result summary.
///
/// A named constant rather than an empty string, because
/// [`DecisionsDb::append`] refuses a blank digest and because "this run
/// recorded no reasoning" and "this row is corrupt" must not look the same to
/// a reader of the persona substrate. The barrier raises a
/// [`FindingKind::EmptyReasoning`] alongside it: §7.1's step 2 was satisfied
/// literally and its intent was not, and the operator should learn that from
/// the report rather than by reading every digest.
pub const NO_REASONING_RECORDED: &str = "run recorded no reasoning";

/// Which of the barrier's three writes this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarrierWrite {
    /// The objective `worklog` row — §7.1 step 1.
    Worklog,
    /// The `subjective_log` row — §7.1 step 2.
    Subjective,
    /// One or more `org_decisions` rows — §7.1 step 3. Reached only when the
    /// run actually changed something organisational.
    Decision,
}

impl BarrierWrite {
    /// The wire form, for logs and `org check` output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worklog => "worklog",
            Self::Subjective => "subjective",
            Self::Decision => "decision",
        }
    }

    /// Parse a wire form. `None` for an unrecognized value — an unknown write
    /// reads as unknown, never as a guess.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "worklog" => Some(Self::Worklog),
            "subjective" => Some(Self::Subjective),
            "decision" => Some(Self::Decision),
            _ => None,
        }
    }

    /// The writes §7.1 orders before this one. The failure path uses it to
    /// report what was already durable.
    fn all_before(self) -> &'static [BarrierWrite] {
        let cut = BARRIER_WRITE_ORDER
            .iter()
            .position(|w| *w == self)
            .unwrap_or(0);
        &BARRIER_WRITE_ORDER[..cut]
    }
}

impl std::fmt::Display for BarrierWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Something the operator needs to know that is not a hard `Err`.
///
/// Distinct from a failed write on purpose. §7.1 is fail-closed on a *write*
/// failure and says nothing about a run that wrote fine but whose identity
/// could not be resolved. Both are visible; they are not the same severity.
/// A write failure means the audit trail has a hole. A finding means the trail
/// exists and one of its fields is honestly empty — which WAVE1-DECISIONS §3.4
/// calls the right answer rather than a defect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingKind {
    /// A **passive** barrier's write failed. Only ever produced by
    /// [`WriteBarrier::attach_subscriber`], which cannot return an `Err` to a
    /// run that has already ended. On the [`WriteBarrier::apply`] path the
    /// same failure is an `Err`, never a finding.
    WriteFailed {
        /// Which write failed.
        write: BarrierWrite,
    },
    /// The run named an `employee_id` that has no registry row, or named none
    /// at all, so both the worklog and the subjective row recorded
    /// [`UNKNOWN_EMPLOYEE`]. The rows exist; their attribution does not.
    UnresolvedEmployee {
        /// The id the run claimed, when it claimed one.
        claimed: Option<String>,
    },
    /// The run supplied no reasoning and produced no result summary, so the
    /// digest is [`NO_REASONING_RECORDED`].
    EmptyReasoning,
    /// The run supplied no reasoning but did produce a summary, so the digest
    /// fell back to that summary. Recorded because §9.2 folds these rows into
    /// `employees.persona`, and a persona assembled from summaries is worse
    /// than a smaller one that says what was actually weighed.
    ReasoningFellBackToSummary,
}

/// One barrier finding, with the context a reader needs to act on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarrierFinding {
    /// Machine-readable kind, so `org check` can filter rather than grep.
    pub kind: FindingKind,
    /// The session the finding is about.
    pub session_id: String,
    /// One human-readable line. Stable enough to grep.
    pub message: String,
}

/// What the barrier wrote, in the order it wrote it.
///
/// The counts are the *result*, which is what makes "the barrier ran and
/// everything landed" assertable without re-reading both databases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarrierReport {
    /// `worklog` rows appended. 0 or 1.
    pub worklog_rows: usize,
    /// `subjective_log` rows appended. 0 or 1.
    pub subjective_rows: usize,
    /// `org_decisions` rows proposed. Zero when the run changed nothing
    /// organisational, which is the common case.
    pub decision_rows: usize,
    /// The id of the objective row, so a caller can correlate a notice or a
    /// DM with exactly this record. `None` only on the passive failure path,
    /// where there is no single row to point at.
    pub worklog_id: Option<String>,
    /// Non-fatal things worth reporting. Never silently empty: the barrier
    /// always says what it could not attribute.
    pub findings: Vec<BarrierFinding>,
}

impl BarrierReport {
    /// True when no finding was raised. A caller that wants to fail loudly on
    /// *anything* checks this, not just the `Result`.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// Findings of one kind, in the order they were raised.
    pub fn findings_of(&self, kind: FindingKind) -> impl Iterator<Item = &BarrierFinding> {
        self.findings.iter().filter(move |f| f.kind == kind)
    }
}

/// The shared sink a **passive** barrier reports failures into.
///
/// It exists because [`WriteBarrier::attach_subscriber`] runs on a
/// `TurnEndBus` subscriber task that cannot return an `Err` to a run which has
/// already returned (§7.1 needs a failure to be visible; the seam's contract
/// says a subscriber's failure cannot propagate). Without a sink, that path
/// would have exactly one option — swallow — which is the failure mode this
/// whole module exists to prevent. So the caller holds this, the subscriber
/// writes to it, and whoever asked for the barrier can read it back.
///
/// Cloning shares the sink, which is what lets a caller hand a clone to the
/// subscriber and keep the original.
#[derive(Clone, Default)]
pub struct BarrierFailureLog {
    findings: Arc<Mutex<Vec<BarrierFinding>>>,
}

impl BarrierFailureLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one finding. A poisoned mutex is recovered from rather than
    /// panicked on — losing the sink to a poisoned lock is exactly the silent
    /// loss this type prevents — and the caller is already in the
    /// failure path, so there is nothing to escalate to.
    pub fn record(&self, finding: BarrierFinding) {
        match self.findings.lock() {
            Ok(mut guard) => guard.push(finding),
            Err(poisoned) => poisoned.into_inner().push(finding),
        }
    }

    /// Every finding recorded so far, oldest first.
    pub fn findings(&self) -> Vec<BarrierFinding> {
        match self.findings.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// How many failures have been recorded.
    pub fn len(&self) -> usize {
        match self.findings.lock() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    /// True when nothing has failed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What the barrier should write for one run.
///
/// Built with [`WriteBarrier::request_from`] (or
/// [`WriteBarrierRequest::from_turn_end`]), which is the only route to a
/// [`super::worklog::WorklogEntry`]: the sealed constructor in `worklog.rs`
/// remains the single mint point, and this type adds no second one.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteBarrierRequest {
    observation: TurnObservation,
    run_kind: RunKind,
    /// The `employee_id` the run claims to be. `None` = nobody claimed one,
    /// which is the honest state for a bare interactive turn and records
    /// [`UNKNOWN_EMPLOYEE`].
    claimed_employee: Option<String>,
    /// The employee's own account of what it weighed and rejected. `None`
    /// means the caller had no reasoning to record; see
    /// [`Self::resolved_reasoning_digest`].
    reasoning_digest: Option<String>,
    confidence: Option<f64>,
    would_do_differently: Option<String>,
    /// Organisational changes the run made. Empty means "nothing changed",
    /// and the barrier writes nothing to `org_decisions`.
    decisions: Vec<OrgDecision>,
    decision_reason: Option<String>,
    /// `department` when the caller knows one. The worklog's `department`
    /// column is honestly NULL in Wave 1; this exists so a caller that *does*
    /// know can record it, not to give the column a value it has no source
    /// for.
    department_override: Option<String>,
}

impl WriteBarrierRequest {
    /// Start a request from the framework's turn-end event.
    ///
    /// Everything the worklog derives off [`TurnEnd`] — session id, iterations,
    /// tool calls, the already-capped result summary — is filled here by
    /// [`TurnObservation::from_turn_end`]. Everything else starts at its
    /// documented default and is set by the `with_*` builders.
    pub fn from_turn_end(turn: TurnEnd) -> Self {
        Self {
            observation: TurnObservation::from_turn_end(turn),
            run_kind: DEFAULT_BARRIER_RUN_KIND,
            claimed_employee: None,
            reasoning_digest: None,
            confidence: None,
            would_do_differently: None,
            decisions: Vec::new(),
            decision_reason: None,
            department_override: None,
        }
    }

    /// The end-of-turn signals the recorded `outcome` is derived from. See
    /// [`super::worklog::derive_outcome`].
    pub fn with_signals(mut self, signals: OutcomeSignals) -> Self {
        self.observation = self.observation.with_signals(signals);
        self
    }

    /// The `employee_id` this run ran as. **A claim, not a resolution** — the
    /// registry check happens in [`WriteBarrier::apply`], so a caller cannot
    /// assert an identity the registry does not have without a finding
    /// appearing.
    pub fn with_employee(mut self, employee_id: impl Into<String>) -> Self {
        self.claimed_employee = Some(employee_id.into());
        self
    }

    /// What kind of run this was. Defaults to [`DEFAULT_BARRIER_RUN_KIND`].
    pub fn with_run_kind(mut self, run_kind: RunKind) -> Self {
        self.run_kind = run_kind;
        self
    }

    /// The employee's own account of what it weighed and rejected.
    ///
    /// This is the builder a caller reaches for. §8.2's `reasoning_digest` has
    /// no honest default — [`DecisionsDb::append`] rejects a blank one — so
    /// the fallback in [`Self::resolved_reasoning_digest`] exists only so that
    /// a run with nothing to say is *recorded as having nothing to say*
    /// rather than failing or leaving a gap in the persona substrate.
    pub fn with_subjective_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning_digest = Some(reasoning.into());
        self
    }

    /// Self-reported confidence in `[0, 1]`.
    ///
    /// A value outside the range is dropped ("not stated") rather than
    /// clamped: `1.4` is not a slightly-too-high number, it is a number that
    /// was never a probability.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = (0.0..=1.0).contains(&confidence).then_some(confidence);
        self
    }

    /// §8.2's `would_do_differently` — the personality growth vector.
    pub fn with_would_do_differently(mut self, text: impl Into<String>) -> Self {
        self.would_do_differently = Some(text.into());
        self
    }

    /// Record that this run changed organisational state, with the decision
    /// object(s) describing it.
    ///
    /// An empty iterator records **no** decision, which is the normal case.
    /// §7.1 step 3 applies to "anything that changes organisational state",
    /// and a run that changed nothing must not manufacture a decision in the
    /// one table §8.2 makes authoritative. Pinned by
    /// `no_decision_object_is_written_when_nothing_organisational_changed`.
    pub fn with_decisions<I>(mut self, decisions: I) -> Self
    where
        I: IntoIterator<Item = OrgDecision>,
    {
        self.decisions = decisions.into_iter().collect();
        self
    }

    /// The `reason` stamped on this request's decision objects. Defaults to
    /// [`BARRIER_DECISION_REASON`].
    pub fn with_decision_reason(mut self, reason: impl Into<String>) -> Self {
        self.decision_reason = Some(reason.into());
        self
    }

    /// The `department` to record when the registry row does not carry one.
    pub fn with_department(mut self, department: Option<String>) -> Self {
        self.department_override = department;
        self
    }

    /// The cron job this run belongs to. Lands in `worklog.job_id` — the
    /// column exists for exactly one caller, the scheduled-run mount in
    /// `cronjobs/scheduler.rs::run_agent_job`, per
    /// [`TurnObservation::with_job_id`]. `None` for an interactive turn.
    pub fn with_job_id(mut self, job_id: Option<String>) -> Self {
        self.observation = self.observation.with_job_id(job_id);
        self
    }

    /// The `employee_id` the run claims. `None` when it claims none.
    pub fn claimed_employee(&self) -> Option<&str> {
        self.claimed_employee.as_deref()
    }

    /// The decision objects that will be proposed. Empty means "no
    /// organisational change".
    pub fn decisions(&self) -> &[OrgDecision] {
        &self.decisions
    }

    /// The run kind that will be recorded on the subjective entry.
    pub fn run_kind(&self) -> RunKind {
        self.run_kind
    }

    /// The session this request is for.
    pub fn session_id(&self) -> &str {
        &self.observation.turn_end().session_id
    }

    /// The reasoning text that will actually be written.
    ///
    /// The caller's digest when it gave a non-empty one; otherwise the turn's
    /// result summary; otherwise [`NO_REASONING_RECORDED`]. The last case is
    /// the point — see [`WriteBarrier::apply`], which pairs this with a
    /// [`FindingKind::EmptyReasoning`] rather than letting the placeholder read
    /// as deliberation.
    pub fn resolved_reasoning_digest(&self) -> String {
        if let Some(digest) = self.reasoning_digest.as_deref()
            && !digest.trim().is_empty()
        {
            return digest.to_string();
        }
        let summary = &self.observation.turn_end().result_summary;
        if !summary.trim().is_empty() {
            return summary.clone();
        }
        NO_REASONING_RECORDED.to_string()
    }
}

/// The three stores a barrier writes through.
///
/// Held as `Arc`s rather than opened per turn: each store applies its schema
/// on construction, and the barrier runs at the end of every employee run, so
/// re-opening two sqlite files per turn would cost without buying anything. A
/// caller that already holds the shared kanban connection builds
/// [`WorklogDb::from_shared_connection`] and
/// [`EmployeeDb::from_shared_connection`] and gets one writer per file instead
/// of two.
#[derive(Clone)]
pub struct WriteBarrier {
    worklog: Arc<WorklogDb>,
    decisions: Arc<DecisionsDb>,
    employees: Arc<EmployeeDb>,
}

impl WriteBarrier {
    /// Start a request for `turn`. A constructor on the barrier as well as on
    /// the request, so a caller that has a barrier in hand does not have to
    /// import the request type to use it.
    pub fn request_from(turn: TurnEnd) -> WriteBarrierRequest {
        WriteBarrierRequest::from_turn_end(turn)
    }

    /// Open all three stores from the main app database path.
    ///
    /// `database_path` is the *main* path (`~/.operant/database.db`); each
    /// store derives its own sibling from it by the rule the rest of the org
    /// layer uses ([`super::employee_db::org_db_path`],
    /// [`super::decisions_db::decisions_db_path`]). No file is created here
    /// that the three stores do not already create, and no `user_version` is
    /// stamped.
    pub fn for_app(database_path: &std::path::Path) -> Result<Self, Error> {
        Ok(Self {
            worklog: Arc::new(WorklogDb::init(database_path)?),
            decisions: Arc::new(DecisionsDb::for_app(database_path)?),
            employees: Arc::new(EmployeeDb::init(super::employee_db::org_db_path(
                database_path,
            ))?),
        })
    }

    /// Build a barrier over stores the caller already opened. The form to use
    /// when the caller holds the shared kanban connection, so the org tables
    /// and the kanban board share one writer rather than contending for two.
    pub fn new(
        worklog: Arc<WorklogDb>,
        decisions: Arc<DecisionsDb>,
        employees: Arc<EmployeeDb>,
    ) -> Self {
        Self {
            worklog,
            decisions,
            employees,
        }
    }

    /// The worklog store, for a caller that reads back the row it just wrote.
    pub fn worklog(&self) -> &WorklogDb {
        &self.worklog
    }

    /// The subjective-log / decision store.
    pub fn decisions(&self) -> &DecisionsDb {
        &self.decisions
    }

    /// The employee registry — the identity source the barrier resolves
    /// against.
    pub fn employees(&self) -> &EmployeeDb {
        &self.employees
    }

    /// **The barrier.** Write all three artifact kinds in §7.1's order and
    /// report what landed.
    ///
    /// Synchronous, because every store it writes through is a blocking
    /// `rusqlite` call and there is nothing to await. A caller at the end of a
    /// run pays the same lock the worklog append would have cost anyway.
    ///
    /// # Errors
    ///
    /// Returns `Err` on the **first** failed write and does not continue.
    /// The error names the write, the session, the employee, what was already
    /// durable when it failed, and the fact that the run's effects had
    /// already landed and were **not** rolled back.
    ///
    /// A registry read error also fails here, before any write: an unreachable
    /// registry has not proven the run's identity.
    pub fn apply(&self, request: &WriteBarrierRequest) -> Result<BarrierReport, Error> {
        let session_id = request.session_id().to_string();
        let mut report = BarrierReport::default();

        // Resolve identity first: a fail-closed barrier must not write an
        // objective row for a run whose author it could not establish, and a
        // poisoned registry is a barrier failure rather than an `unknown`.
        let (employee, identity_finding) = self.resolve_employee(request)?;
        if let Some(finding) = identity_finding {
            report.findings.push(finding);
        }

        // ── §7.1 step 1 — the objective row. Exactly one. ──────────────────
        let entry = self.observe(request, &employee).into_entry();
        let worklog_id = entry.id().to_string();
        if let Err(e) = self.worklog.append(&entry) {
            return Err(self.fail(BarrierWrite::Worklog, &session_id, &employee, &report, e));
        }
        report.worklog_rows = 1;
        report.worklog_id = Some(worklog_id);

        // ── §7.1 step 2 — the subjective row. A different table, a
        // different file, and never merged into the row above. ────────────
        if let Some(finding) = self.reasoning_finding(request) {
            report.findings.push(finding);
        }
        let mut subjective = SubjectiveEntry::new(
            format!("{SUBJECTIVE_ID_PREFIX}{}", uuid::Uuid::new_v4()),
            employee.clone(),
            session_id.clone(),
            request.run_kind,
            request.resolved_reasoning_digest(),
        );
        subjective.confidence = request.confidence;
        subjective.would_do_differently = request.would_do_differently.clone();
        if let Err(e) = self.decisions.append(&subjective) {
            return Err(self.fail(BarrierWrite::Subjective, &session_id, &employee, &report, e));
        }
        report.subjective_rows = 1;

        // ── §7.1 step 3 — decision objects, for organisational change only.
        // An empty `decisions` writes nothing at all. That is the rule, not an
        // oversight: a manufactured decision is a false statement in the one
        // table §8.2 makes authoritative. ─────────────────────────────────
        for (index, mut decision) in request.decisions.iter().cloned().enumerate() {
            if decision.decided_by.trim().is_empty() {
                if employee == UNKNOWN_EMPLOYEE {
                    // §8.2 makes the object attributable and `propose` would
                    // happily take `unknown`. Refuse it here rather than
                    // writing a decision with an author nobody is.
                    return Err(self.fail(
                        BarrierWrite::Decision,
                        &session_id,
                        &employee,
                        &report,
                        Error::Agent(format!(
                            "decision #{index} ({:?}) has no attributable decider: the run \
                             resolved no employee, and a decision object nobody decided is not \
                             a decision",
                            decision.subject
                        )),
                    ));
                }
                decision.decided_by = employee.clone();
            }
            if decision.reason.trim().is_empty() {
                decision.reason = request
                    .decision_reason
                    .clone()
                    .unwrap_or_else(|| BARRIER_DECISION_REASON.to_string());
            }
            if let Err(e) = self.decisions.propose(&decision) {
                return Err(self.fail(BarrierWrite::Decision, &session_id, &employee, &report, e));
            }
            report.decision_rows += 1;
        }

        Ok(report)
    }

    /// Fork a **passive** barrier onto the post-turn bus.
    ///
    /// This is the subscription the bus exists for. It cannot make a run fail
    /// — the seam's contract is that subscriber failure never propagates, and
    /// the turn has already returned — so every failure is pushed onto
    /// `failures` (which the caller holds) and logged at `error`. Nothing on
    /// this path is swallowed; it is merely reported somewhere other than a
    /// return value nobody could still read.
    ///
    /// The subscriber task ends when the bus it was handed is dropped: a
    /// broadcast receiver sees `Err(RecvError::Closed)` at that point, which
    /// is the only clean shutdown signal the seam has. A caller that wants the
    /// barrier to outlive the bus would need a cancellation token of its own —
    /// out of scope here, and §7.1 asks for a postcondition on a run, not for
    /// a permanent daemon.
    ///
    /// `resolve` maps a [`TurnEnd`] to the `employee_id` that ran it. The event
    /// carries no identity — §3.4's `UNKNOWN_EMPLOYEE` default is exactly this
    /// gap — so a subscriber that returns `None` records the objective row
    /// honestly unattributed and raises
    /// [`FindingKind::UnresolvedEmployee`]. Returning `Some(id)` for an id
    /// with no registry row produces the same finding, so the closure cannot
    /// fabricate attribution either.
    ///
    /// `label` names the daemon and must be unique per install:
    /// `daemon_pool::drain_for_label` claims handles out of one global pool, so
    /// two subscribers sharing a label would let one test drain the other.
    pub fn attach_subscriber(
        self: Arc<Self>,
        bus: &TurnEndBus,
        label: impl Into<String>,
        failures: BarrierFailureLog,
        resolve: impl Fn(&TurnEnd) -> Option<String> + Send + Sync + 'static,
    ) {
        let mut rx = bus.subscribe();
        let barrier = self.clone();
        bus.fork(label, async move {
            while let Ok(turn) = rx.recv().await {
                let mut request = WriteBarrierRequest::from_turn_end(turn.clone());
                if let Some(employee_id) = resolve(&turn) {
                    request = request.with_employee(employee_id);
                }
                if let Err(e) = barrier.apply(&request) {
                    let write = write_named_in(&e);
                    tracing::error!(
                        session_id = turn.session_id,
                        "write barrier (passive subscriber): postcondition failure on a run \
                         that had already completed: {e}"
                    );
                    failures.record(BarrierFinding {
                        kind: FindingKind::WriteFailed { write },
                        session_id: turn.session_id.clone(),
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// Resolve the run's claimed `employee_id` against the registry.
    ///
    /// Returns the id to record — the resolved one, or [`UNKNOWN_EMPLOYEE`]
    /// when the claim does not hold — plus a finding when it does not. A
    /// *store* failure is an `Err`: it means the barrier does not know, and
    /// §7.1 is fail-closed.
    fn resolve_employee(
        &self,
        request: &WriteBarrierRequest,
    ) -> Result<(String, Option<BarrierFinding>), Error> {
        let Some(claimed) = request.claimed_employee() else {
            return Ok((
                UNKNOWN_EMPLOYEE.to_string(),
                Some(BarrierFinding {
                    kind: FindingKind::UnresolvedEmployee { claimed: None },
                    session_id: request.session_id().to_string(),
                    message: format!(
                        "run named no employee, so both rows record employee={UNKNOWN_EMPLOYEE:?}: \
                         the records exist and are not attributable"
                    ),
                }),
            ));
        };
        if claimed.trim().is_empty() {
            return Err(Error::Agent(format!(
                "write barrier: run {} claimed a blank employee_id — an identity that is empty is \
                 not an identity that is unknown, and recording it as 'unknown' would launder a \
                 caller's bug into an honest-looking row",
                request.session_id()
            )));
        }
        match self.employees.get_employee(claimed) {
            // The registry holds the id the run claimed, so the row found *is*
            // the resolved identity; there is no second derivation to disagree
            // with it.
            Ok(Some(_)) => Ok((claimed.to_string(), None)),
            Ok(None) => Ok((
                UNKNOWN_EMPLOYEE.to_string(),
                Some(BarrierFinding {
                    kind: FindingKind::UnresolvedEmployee {
                        claimed: Some(claimed.to_string()),
                    },
                    session_id: request.session_id().to_string(),
                    message: format!(
                        "run {} claimed employee {claimed:?}, which has no row in the registry; \
                         both rows record employee={UNKNOWN_EMPLOYEE:?} — the honest value \
                         (WAVE1-DECISIONS §3.4), not an invented one",
                        request.session_id()
                    ),
                }),
            )),
            Err(e) => Err(Error::Agent(format!(
                "write barrier: employee registry read failed for {claimed:?}, so the run's \
                 identity is unproven and no record was written: {e}"
            ))),
        }
    }

    /// Build the worklog observation with the resolved employee, and the
    /// department when the caller supplied one.
    ///
    /// Kept separate from [`Self::resolve_employee`] because the department
    /// only matters at the point the row is sealed. The registry's own
    /// `department` column is NULL on both sides in Wave 1, so this is a
    /// pass-through today; it is read here rather than dropped so a caller
    /// that knows a department can record it when the registry grows one.
    fn observe(&self, request: &WriteBarrierRequest, employee: &str) -> TurnObservation {
        let mut observation = request.observation.clone().with_employee(employee);
        if let Some(department) = request.department_override.clone() {
            observation = observation.with_department(Some(department));
        }
        observation
    }

    /// The finding that goes with a subjective digest the caller did not
    /// supply, or `None` when it did.
    fn reasoning_finding(&self, request: &WriteBarrierRequest) -> Option<BarrierFinding> {
        let supplied = request
            .reasoning_digest
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty());
        if supplied {
            return None;
        }
        let summary_empty = request
            .observation
            .turn_end()
            .result_summary
            .trim()
            .is_empty();
        let (kind, message) = if summary_empty {
            (
                FindingKind::EmptyReasoning,
                format!(
                    "the run supplied no reasoning and produced no result summary, so the \
                     subjective digest is the placeholder {NO_REASONING_RECORDED:?}"
                ),
            )
        } else {
            (
                FindingKind::ReasoningFellBackToSummary,
                "the run supplied no reasoning, so the subjective digest fell back to its result \
                 summary; §9.2 folds these rows into a persona, and a persona built from \
                 summaries answers a different question than 'what did this employee weigh'"
                    .to_string(),
            )
        };
        Some(BarrierFinding {
            kind,
            session_id: request.session_id().to_string(),
            message,
        })
    }

    /// Build the postcondition-failure error.
    ///
    /// Names the failing write, the session, the employee, and the writes that
    /// were already durable — because the worklog is append-only with a fresh
    /// uuid per attempt, so "what was durable" is exactly what tells a
    /// retrying caller which rows it would duplicate.
    fn fail(
        &self,
        write: BarrierWrite,
        session_id: &str,
        employee_id: &str,
        report: &BarrierReport,
        source: Error,
    ) -> Error {
        let durable: Vec<&str> = write
            .all_before()
            .iter()
            .filter(|w| match w {
                BarrierWrite::Worklog => report.worklog_rows > 0,
                BarrierWrite::Subjective => report.subjective_rows > 0,
                BarrierWrite::Decision => report.decision_rows > 0,
            })
            .map(|w| w.as_str())
            .collect();
        let durable = if durable.is_empty() {
            "nothing".to_string()
        } else {
            durable.join(", ")
        };
        tracing::error!(
            write = write.as_str(),
            session_id,
            employee_id,
            durable_at_failure = durable.as_str(),
            "write barrier: postcondition failure on a run that had already completed"
        );
        Error::Agent(format!(
            "write barrier: {write} write failed for session {session_id:?} (employee \
             {employee_id:?}); durable at failure: {durable}. The run's effects had already landed \
             and are NOT rolled back — this is an incomplete record on a completed run, not a \
             prevented run. Retrying appends new rows rather than replacing these, because the \
             worklog is append-only. Cause: {source}"
        ))
    }
}

/// Recover the failing write from a `fail`-shaped error, for the passive path
/// that has an `Error` and a typed field to fill but no return value to
/// inspect.
///
/// Parsing the message rather than threading a second error type through
/// [`WriteBarrier::fail`] is a deliberate trade: the alternative is a
/// `BarrierError { write, source }` enum this crate's `Error` cannot hold
/// without a variant of its own, and this module does not own `error.rs`. The
/// fallback is [`BarrierWrite::Worklog`] — the first write, and the conservative
/// answer when the text does not match — so a malformed message degrades to
/// naming the earliest write rather than to no write at all.
fn write_named_in(error: &Error) -> BarrierWrite {
    let text = error.to_string();
    BARRIER_WRITE_ORDER
        .iter()
        .find(|w| text.contains(&format!("barrier: {w} write failed")))
        .copied()
        .unwrap_or(BarrierWrite::Worklog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cronjobs::db::CronJob;
    use crate::org::decisions_db::DecisionStatus;
    use crate::org::worklog::{Outcome, WorklogRecord};
    use crate::org::worklog_db::WorklogQuery;
    use std::time::Duration;

    /// Unique daemon labels: `drain_for_label` claims handles out of one global
    /// pool, so a shared label would let one test drain another's subscriber.
    const LABEL_MOUNT: &str = "write-barrier-test-mount";
    const LABEL_FAILURE: &str = "write-barrier-test-failure";

    /// The three stores plus the temp dir that owns them, at the real path
    /// shape: a main `database.db` with the kanban and decisions siblings
    /// beside it.
    struct Fixture {
        _dir: tempfile::TempDir,
        barrier: WriteBarrier,
    }

    impl Fixture {
        /// Register one employee so identity resolution has something to find.
        fn register(&self, job_id: &str) -> String {
            self.barrier
                .employees()
                .backfill_from_cron_jobs(
                    &[script_job(job_id)],
                    "2026-10-01T00:00:00.000Z",
                    "write barrier test: seed the registry",
                )
                .expect("backfill");
            format!("emp-{job_id}")
        }

        /// Drop a table out from under a store, so its next write fails for a
        /// real reason (SQLite "no such table") rather than a simulated one.
        fn break_worklog(&self, sql: &str) {
            self.barrier
                .worklog()
                .conn()
                .lock()
                .expect("worklog lock")
                .execute_batch(sql)
                .expect("drop the table");
        }

        fn break_decisions(&self, sql: &str) {
            self.barrier
                .decisions()
                .conn()
                .lock()
                .expect("decisions lock")
                .execute_batch(sql)
                .expect("drop the table");
        }

        fn worklog_rows(&self, session: &str) -> Vec<WorklogRecord> {
            self.barrier
                .worklog()
                .list(&WorklogQuery {
                    session_id: Some(session.to_string()),
                    ..Default::default()
                })
                .expect("read the objective log")
        }
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let barrier = WriteBarrier::for_app(&dir.path().join("database.db"))
            .expect("the barrier opens its three stores");
        Fixture { _dir: dir, barrier }
    }

    /// A minimal `CronJob`. The backfill reads `id`, `skills`, and `state`;
    /// every other field is noise, so it gets the smallest honest value.
    fn script_job(id: &str) -> CronJob {
        CronJob {
            id: id.to_string(),
            name: "Write Barrier Fixture".to_string(),
            prompt: "reconcile the ledger".to_string(),
            schedule: "0 * * * *".to_string(),
            schedule_display: "0 * * * *".to_string(),
            repeat_times: None,
            repeat_completed: 0,
            deliver: "local".to_string(),
            origin_platform: None,
            origin_chat_id: None,
            origin_thread_id: None,
            skill: None,
            skills: Some(vec!["ops".to_string()]),
            model: None,
            provider: None,
            base_url: None,
            script: None,
            context_from: None,
            enabled_toolsets: None,
            workdir: None,
            no_agent: false,
            enabled: true,
            state: "scheduled".to_string(),
            paused_at: None,
            paused_reason: None,
            created_at: "2026-10-01T00:00:00.000Z".to_string(),
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            last_error: None,
            last_delivery_error: None,
        }
    }

    fn turn(session: &str, reply: &str) -> TurnEnd {
        TurnEnd {
            turn_id: 0,
            session_id: session.to_string(),
            iterations: 2,
            tool_calls: 4,
            tool_durations_ms: vec![10, 20],
            result_summary: reply.to_string(),
            result_truncated: false,
        }
    }

    /// A request that should produce a completely clean report: a named
    /// employee, supplied reasoning, clean signals, no decisions.
    fn clean_request(session: &str, employee: &str) -> WriteBarrierRequest {
        WriteBarrier::request_from(turn(session, "reconciled the ledger"))
            .with_signals(OutcomeSignals {
                done_emitted: true,
                turn_errored: false,
                last_status_clean: true,
            })
            .with_employee(employee)
            .with_subjective_reasoning("weighed a rollback against a forward fix; chose forward")
    }

    /// A decision with no author **and no reason**, so the barrier must supply
    /// both. `OrgDecision::new`'s signature is
    /// `(id, subject, scope, decided_by, rationale, reason)`, so the empty
    /// `decided_by` and the empty `reason` are the 4th and 6th arguments —
    /// passing the meeting note in the `reason` slot instead left it non-blank
    /// and the barrier's own-provenance substitution at `write_barrier.rs:701`
    /// never fired.
    fn authorless_decision(id: &str, subject: &str) -> OrgDecision {
        OrgDecision::new(
            id,
            subject,
            "Org",
            "",
            "the platform team cannot ship both stacks this quarter",
            "",
        )
    }

    // ── 1. exactly one objective row per successful run ────────────────────

    #[test]
    fn a_successful_run_writes_exactly_one_worklog_row() {
        let fx = fixture();
        let employee = fx.register("a02e3f692fb0");

        let report = fx
            .barrier
            .apply(&clean_request("sess-one", &employee))
            .expect("the barrier succeeds");

        assert_eq!(report.worklog_rows, 1, "one objective row per run");
        assert_eq!(report.subjective_rows, 1);
        assert_eq!(report.decision_rows, 0);
        assert!(
            report.is_clean(),
            "a clean run raises no finding: {report:?}"
        );
        assert!(report.worklog_id.is_some());

        let rows = fx.worklog_rows("sess-one");
        assert_eq!(
            rows.len(),
            1,
            "exactly one row on disk, not zero and not two"
        );
        let row = &rows[0];
        assert_eq!(row.employee, employee, "attributable, not 'unknown'");
        assert_eq!(row.session_id, "sess-one");
        assert_eq!(row.what_done, "reconciled the ledger");
        assert_eq!(row.outcome, Outcome::Success.as_str());
        assert_eq!(row.iteration, 2);
        assert_eq!(row.tool_calls, 4);
        assert_eq!(row.department, None, "no department is invented");

        // A second run appends rather than replacing: the log is append-only,
        // so "one row per run" is a per-run claim, not a table-size claim.
        fx.barrier
            .apply(&clean_request("sess-two", &employee))
            .expect("the second run succeeds");
        let all = fx
            .barrier
            .worklog()
            .list(&WorklogQuery::default())
            .expect("read every row");
        assert_eq!(all.len(), 2, "one row per run, appended not overwritten");
    }

    // ── 2. the subjective entry is its own artifact ────────────────────────

    #[test]
    fn the_subjective_entry_is_recorded_separately_from_the_worklog_row() {
        let fx = fixture();
        let employee = fx.register("b02e3f692fb0");
        let reasoning = "weighed a rollback against a forward fix; chose forward";

        let report = fx
            .barrier
            .apply(&clean_request("sess-sub", &employee))
            .expect("the barrier succeeds");
        assert_eq!(report.worklog_rows, 1);
        assert_eq!(report.subjective_rows, 1);

        // Two stores, two rows, and the subjective row says something the
        // objective row does not.
        let subjective = fx
            .barrier
            .decisions()
            .recent_for_employee(&employee, 10)
            .expect("read the subjective log");
        assert_eq!(subjective.len(), 1);
        let entry = &subjective[0];
        assert_eq!(entry.session_id, "sess-sub");
        assert_eq!(entry.employee_id, employee);
        assert_eq!(entry.run_kind, RunKind::Manual);
        assert_eq!(entry.reasoning_digest, reasoning);
        assert!(
            entry.entry_id.starts_with(SUBJECTIVE_ID_PREFIX),
            "the subjective id follows §8.2's shape, got {:?}",
            entry.entry_id
        );

        let objective = fx.worklog_rows("sess-sub");
        assert_eq!(objective.len(), 1);
        // The merge this module must not perform: the reasoning is not
        // smuggled into the objective row, and the objective summary is not
        // smuggled into the subjective digest.
        assert_eq!(objective[0].what_done, "reconciled the ledger");
        assert!(
            !objective[0].what_done.contains(reasoning),
            "what_done must stay objective: {}",
            objective[0].what_done
        );
        assert_ne!(entry.reasoning_digest, objective[0].what_done);
        assert_eq!(
            fx.barrier.decisions().count().expect("subjective count"),
            1,
            "exactly one subjective row, and the worklog did not absorb it"
        );
    }

    // ── 3. a write failure is surfaced, not swallowed ──────────────────────

    #[test]
    fn a_failed_worklog_write_is_an_err_and_nothing_else_is_written() {
        let fx = fixture();
        let employee = fx.register("c02e3f692fb0");
        // Remove the table under the barrier: the next append fails for a real
        // SQLite reason rather than a simulated one.
        fx.break_worklog("DROP TABLE worklog");

        let err = fx
            .barrier
            .apply(&clean_request("sess-fail", &employee))
            .expect_err("a failed worklog write must be an Err, never Ok");

        let msg = err.to_string();
        assert!(
            msg.contains("worklog write failed"),
            "names the write: {msg}"
        );
        assert!(msg.contains("sess-fail"), "names the session: {msg}");
        assert!(msg.contains("durable at failure: nothing"), "{msg}");
        assert!(
            msg.contains("NOT rolled back"),
            "says the run completed rather than pretending it was prevented: {msg}"
        );

        // The barrier stopped at the first failure, so the subjective row was
        // never written for a run whose objective record is missing.
        assert_eq!(
            fx.barrier.decisions().count().expect("subjective count"),
            0,
            "§7.1's order is not decoration: no subjective row after a failed objective row"
        );
    }

    #[test]
    fn a_failed_subjective_write_reports_what_was_already_durable() {
        let fx = fixture();
        let employee = fx.register("d02e3f692fb0");
        // The decisions file holds both the subjective and the decision table;
        // drop only the subjective one, so step 1 succeeds and step 2 does not.
        fx.break_decisions("DROP TABLE subjective_log");

        let err = fx
            .barrier
            .apply(&clean_request("sess-partial", &employee))
            .expect_err("a failed subjective write must be an Err, never Ok");

        let msg = err.to_string();
        assert!(msg.contains("subjective write failed"), "{msg}");
        assert!(
            msg.contains("durable at failure: worklog"),
            "names the objective row that did land, so a caller knows a retry duplicates it: {msg}"
        );
        // ...and the row that did land is still there. The barrier neither
        // claims it did not happen nor undoes it.
        assert_eq!(fx.barrier.worklog().count().expect("count"), 1);
    }

    #[test]
    fn a_failed_decision_write_surfaces_instead_of_passing() {
        let fx = fixture();
        let employee = fx.register("e02e3f692fb0");

        // A blank `rationale` is refused by the store; the barrier must report
        // that rather than filling in a justification of its own.
        let mut decision = authorless_decision("d-bad", "retire the v1 ingest");
        decision.rationale = String::new();
        let request = clean_request("sess-bad-decision", &employee).with_decisions([decision]);

        let err = fx
            .barrier
            .apply(&request)
            .expect_err("a refused decision write must be an Err, never Ok");
        let msg = err.to_string();
        assert!(msg.contains("decision write failed"), "{msg}");
        assert!(
            msg.contains("durable at failure: worklog, subjective"),
            "{msg}"
        );
        assert_eq!(
            fx.barrier.decisions().count().expect("count"),
            1,
            "the subjective row landed; the refused decision wrote no partial row"
        );
        assert!(
            fx.barrier.decisions().get("d-bad").expect("get").is_none(),
            "no half-written decision survives a refusal"
        );
    }

    #[test]
    fn a_registry_that_cannot_answer_fails_the_barrier_before_any_write() {
        let fx = fixture();
        // Make the registry genuinely unreachable: `get_employee` prepares a
        // statement against `employees`, and with the table gone that fails
        // for a real SQLite reason. An unanswerable registry has not proven
        // the run's identity, so the barrier must fail rather than record
        // `unknown` — and it must do so before writing anything.
        let registry_path = fx
            .barrier
            .worklog()
            .conn()
            .lock()
            .expect("worklog lock")
            .path()
            .expect("path")
            .to_string();
        let mut registry = rusqlite::Connection::open(registry_path).expect("open the kanban file");
        registry
            .execute_batch("DROP TABLE employees")
            .expect("drop the registry table");

        let err = fx
            .barrier
            .apply(&clean_request("sess-no-registry", "emp-anything"))
            .expect_err("an unproven identity must not produce records");
        let msg = err.to_string();
        assert!(msg.contains("identity is unproven"), "{msg}");
        assert_eq!(
            fx.barrier.worklog().count().expect("count"),
            0,
            "fail-closed: nothing was written"
        );
    }

    // ── 4. no decision object unless something organisational changed ───────

    #[test]
    fn no_decision_object_is_written_when_nothing_organisational_changed() {
        let fx = fixture();
        let employee = fx.register("f02e3f692fb0");

        let request = clean_request("sess-quiet", &employee);
        assert!(
            request.decisions().is_empty(),
            "the run changed nothing, so it declared no decisions"
        );

        let report = fx.barrier.apply(&request).expect("the barrier succeeds");
        assert_eq!(
            report.decision_rows, 0,
            "no decision object is manufactured for a run that changed nothing"
        );
        assert!(
            fx.barrier
                .decisions()
                .list_by_status(DecisionStatus::Proposed)
                .expect("list")
                .is_empty(),
            "the authoritative decision table is untouched"
        );
        // ...while the other two writes still happened, so "no decision" is
        // not "no barrier".
        assert_eq!(report.worklog_rows, 1);
        assert_eq!(report.subjective_rows, 1);
        assert!(report.is_clean(), "{report:?}");
    }

    #[test]
    fn a_run_that_changed_the_organisation_writes_its_decision_objects() {
        let fx = fixture();
        let employee = fx.register("a12e3f692fb0");

        let report = fx
            .barrier
            .apply(
                &clean_request("sess-decided", &employee)
                    .with_decisions([authorless_decision("d-1", "Retire the v1 ingest stack")]),
            )
            .expect("the barrier succeeds");
        assert_eq!(report.decision_rows, 1);

        let written = fx
            .barrier
            .decisions()
            .get("d-1")
            .expect("get")
            .expect("row present");
        assert_eq!(written.decided_by, employee, "attributed to the runner");
        assert_eq!(written.status, DecisionStatus::Proposed);
        assert_eq!(
            written.reason, BARRIER_DECISION_REASON,
            "the barrier records its own provenance, not a default justification"
        );
        assert_eq!(
            written.rationale, "the platform team cannot ship both stacks this quarter",
            "the decision's own justification is the caller's and is never filled in"
        );
    }

    #[test]
    fn a_decision_nobody_decided_is_refused_rather_than_given_a_fake_author() {
        let fx = fixture();
        // No employee claimed and none registered, so the run resolved to
        // `unknown`. §8.2 makes the object attributable.
        let request = WriteBarrier::request_from(turn("sess-anon", "did a thing"))
            .with_decisions([authorless_decision("d-anon", "reorg the platform team")]);

        let err = fx
            .barrier
            .apply(&request)
            .expect_err("an unattributable decision must not be written");
        let msg = err.to_string();
        assert!(msg.contains("decision write failed"), "{msg}");
        assert!(msg.contains("no attributable decider"), "{msg}");
        assert!(
            fx.barrier.decisions().get("d-anon").expect("get").is_none(),
            "no decision row with a fabricated author"
        );
    }

    // ── findings: nothing is silently unattributed or unreasoned ───────────

    #[test]
    fn an_unresolvable_employee_is_a_finding_not_an_invented_identity() {
        let fx = fixture();
        let request = clean_request("sess-ghost", "emp-doesnotexist");

        let report = fx
            .barrier
            .apply(&request)
            .expect("a missing row is not an Err");
        let findings: Vec<_> = report
            .findings_of(FindingKind::UnresolvedEmployee {
                claimed: Some("emp-doesnotexist".to_string()),
            })
            .collect();
        assert_eq!(findings.len(), 1, "the gap is reported: {report:?}");
        assert!(!report.is_clean());
        assert!(findings[0].message.contains("no row in the registry"));

        // The row exists and says `unknown` — the honest value, per §3.4.
        let rows = fx.worklog_rows("sess-ghost");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].employee, UNKNOWN_EMPLOYEE);
    }

    #[test]
    fn a_blank_claimed_employee_id_is_refused_rather_than_recorded_as_unknown() {
        let fx = fixture();
        let err = fx
            .barrier
            .apply(&clean_request("sess-blank", "   "))
            .expect_err("an empty identity is a caller bug, not an unknown employee");
        assert!(err.to_string().contains("blank employee_id"), "got: {err}");
        assert_eq!(fx.barrier.worklog().count().expect("count"), 0);
    }

    #[test]
    fn a_run_with_no_reasoning_records_that_fact_explicitly() {
        let fx = fixture();
        let request = WriteBarrier::request_from(turn("sess-empty", ""));

        let report = fx.barrier.apply(&request).expect("the barrier succeeds");
        assert_eq!(report.subjective_rows, 1, "§7.1 step 2 still ran");
        assert_eq!(
            report.findings_of(FindingKind::EmptyReasoning).count(),
            1,
            "and the report said so: {report:?}"
        );
        let digest = fx
            .barrier
            .decisions()
            .recent_for_employee(UNKNOWN_EMPLOYEE, 1)
            .expect("read")
            .remove(0)
            .reasoning_digest;
        assert_eq!(
            digest, NO_REASONING_RECORDED,
            "a named placeholder, not a blank the store would have refused"
        );
    }

    #[test]
    fn a_summary_substituted_for_missing_reasoning_is_flagged() {
        let fx = fixture();
        // A summary but no `with_subjective_reasoning`.
        let request = WriteBarrier::request_from(turn("sess-fallback", "reconciled the ledger"))
            .with_employee(fx.register("a22e3f692fb0"));

        let report = fx.barrier.apply(&request).expect("the barrier succeeds");
        assert_eq!(
            report
                .findings_of(FindingKind::ReasoningFellBackToSummary)
                .count(),
            1,
            "a persona built from summaries is a finding: {report:?}"
        );
    }

    // ── the mount: the barrier on the real post-turn seam ───────────────────

    #[tokio::test]
    async fn a_bus_subscriber_runs_the_barrier_for_each_emitted_turn() {
        let fx = fixture();
        let employee = fx.register("a32e3f692fb0");
        let barrier = Arc::new(fx.barrier.clone());
        let failures = BarrierFailureLog::new();
        let bus = TurnEndBus::new();
        let kept = bus.subscribe();
        let resolver_employee = employee.clone();

        barrier.attach_subscriber(&bus, LABEL_MOUNT, failures.clone(), move |turn| {
            if turn.session_id.starts_with("sess-") {
                Some(resolver_employee.clone())
            } else {
                None
            }
        });

        assert_eq!(bus.emit("sess-1", 2, 3, "reconciled the ledger"), 2);
        assert_eq!(bus.emit("sess-2", 1, 0, "checked the inbox"), 2);
        assert_eq!(kept.len(), 2, "both turns were delivered");

        // The subscriber runs until the bus closes, so dropping the bus is how
        // the task is told to finish; `drain_for_label` then waits for it.
        drop(bus);
        let drained =
            crate::daemon_pool::drain_for_label(LABEL_MOUNT, Duration::from_secs(10)).await;
        assert_eq!(drained, 1, "the barrier subscriber ran to completion");
        assert!(
            failures.is_empty(),
            "a healthy subscriber records nothing: {:?}",
            failures.findings()
        );
        assert_eq!(
            fx.barrier.worklog().count().expect("count"),
            2,
            "one objective row per emitted turn"
        );
        assert_eq!(fx.barrier.decisions().count().expect("count"), 2);
    }

    #[tokio::test]
    async fn a_passive_subscriber_records_its_failure_instead_of_swallowing_it() {
        let fx = fixture();
        let employee = fx.register("a42e3f692fb0");
        fx.break_decisions("DROP TABLE subjective_log");

        let barrier = Arc::new(fx.barrier.clone());
        let failures = BarrierFailureLog::new();
        let bus = TurnEndBus::new();
        let _kept = bus.subscribe();
        barrier.attach_subscriber(&bus, LABEL_FAILURE, failures.clone(), move |_| {
            Some(employee.clone())
        });

        // The turn ends successfully; the run is over. The barrier cannot fail
        // it, so the failure has to be readable afterwards.
        assert_eq!(bus.emit("sess-1", 1, 0, "did a thing"), 2);
        drop(bus);
        let drained =
            crate::daemon_pool::drain_for_label(LABEL_FAILURE, Duration::from_secs(10)).await;
        assert_eq!(drained, 1);

        let recorded = failures.findings();
        assert_eq!(recorded.len(), 1, "not swallowed: {recorded:?}");
        assert_eq!(
            recorded[0].kind,
            FindingKind::WriteFailed {
                write: BarrierWrite::Subjective
            },
            "names the failing write"
        );
        assert_eq!(recorded[0].session_id, "sess-1");
        assert!(recorded[0].message.contains("NOT rolled back"));
    }

    // ── the storage invariant the layer holds to ───────────────────────────

    #[test]
    fn the_barrier_leaves_the_kanban_user_version_untouched() {
        let fx = fixture();
        let employee = fx.register("a52e3f692fb0");
        fx.barrier
            .apply(&clean_request("sess-version", &employee))
            .expect("the barrier succeeds");

        // Read through a *second* connection to the same file, so this is the
        // stored value and not a cached handle. The org layer never claims the
        // file-wide counter; the kanban family alone owns it (schema.rs).
        let path = fx
            .barrier
            .worklog()
            .conn()
            .lock()
            .expect("lock")
            .path()
            .expect("path")
            .to_string();
        // No `drop(fx)` here: opening a second `Connection` on the same file
        // while the barrier's own connection is still open is valid SQLite, and
        // is exactly what the "second connection, not a cached handle" comment
        // above describes. Dropping the fixture would unlink the database and
        // turn this storage invariant into a `SqliteFailure` on open.
        let version: i64 = rusqlite::Connection::open(&path)
            .expect("reopen")
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("read user_version");
        assert_eq!(
            version, 0,
            "the barrier opened and wrote both org tables without moving a version counter"
        );
    }

    // ── small contracts ────────────────────────────────────────────────────

    #[test]
    fn every_barrier_write_round_trips_through_its_wire_form() {
        for w in BARRIER_WRITE_ORDER {
            assert_eq!(BarrierWrite::parse(w.as_str()), Some(w));
        }
        assert_eq!(BarrierWrite::parse("nonsense"), None);
        assert_eq!(BarrierWrite::Worklog.to_string(), "worklog");
    }

    #[test]
    fn the_write_order_is_the_specs_order() {
        assert_eq!(
            BARRIER_WRITE_ORDER,
            [
                BarrierWrite::Worklog,
                BarrierWrite::Subjective,
                BarrierWrite::Decision
            ],
            "§7.1 numbers them in this order and `all_before` indexes off it"
        );
        assert!(BarrierWrite::Worklog.all_before().is_empty());
        assert_eq!(
            BarrierWrite::Subjective.all_before(),
            &[BarrierWrite::Worklog]
        );
        assert_eq!(
            BarrierWrite::Decision.all_before(),
            &[BarrierWrite::Worklog, BarrierWrite::Subjective]
        );
    }

    #[test]
    fn an_out_of_range_confidence_is_dropped_rather_than_clamped() {
        let high = WriteBarrier::request_from(turn("s", "x")).with_confidence(1.4);
        assert_eq!(high.confidence, None, "1.4 was never a probability");
        let honest = WriteBarrier::request_from(turn("s", "x")).with_confidence(0.75);
        assert_eq!(honest.confidence, Some(0.75));
    }

    #[test]
    fn the_resolved_digest_prefers_the_callers_own_words() {
        let own = WriteBarrier::request_from(turn("s", "the summary"))
            .with_subjective_reasoning("what I weighed")
            .resolved_reasoning_digest();
        assert_eq!(own, "what I weighed", "the caller's words win");

        let summary =
            WriteBarrier::request_from(turn("s", "the summary")).resolved_reasoning_digest();
        assert_eq!(summary, "the summary", "then the turn's own summary");

        let none = WriteBarrier::request_from(turn("s", "  ")).resolved_reasoning_digest();
        assert_eq!(none, NO_REASONING_RECORDED, "then a named placeholder");
    }
}
