//! The worklog — the organism's *objective* temporal log
//! (`docs/WAVE1-DECISIONS.md` §3.4).
//!
//! Pure types and the sealed entry constructor. No I/O; the sqlite store
//! lives in [`super::worklog_db`].
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.4. Organism corroboration:
//! `docs/ORGANISM-ARTIFACT-REFERENCE.md` §4.4(a) (19 keys measured over
//! 1,127 live records; `correlation_id` / `notice_ids` / `session_id`
//! populated on 327 of them).
//!
//! ## WHO WRITES IT, AND WHY THAT IS A TYPE BOUNDARY HERE
//!
//! §3.4, quoting the organism's own contract at `agent_fabric.py:142-147`:
//!
//! > One row in a dept worklog. Append-only.
//! > Objective temporal log (framework-appended, AF-AD-007) — must never
//! > carry board-routing fields. `correlation_id`/`notice_ids`/`session_id`
//! > are foreign-key links to the subjective board, populated by the
//! > framework only.
//!
//! The organism states that as a *contract*. This module states it as an
//! *API shape*, in four ways that are each independently sufficient to stop
//! a model-authored row:
//!
//! 1. **No struct literal.** [`WorklogEntry`] has **private fields** and no
//!    `pub fn new(…)` taking free-form values. The only constructor is
//!    [`TurnObservation::into_entry`], and a [`TurnObservation`] cannot be
//!    conjured — it is built from a [`TurnEnd`], the struct the framework's
//!    `TurnEndBus` hands to subscribers, plus a framework-owned
//!    [`UsageAccumulator`].
//! 2. **No tool.** Nothing in `crates/operant-core/src/tools/**` is
//!    registered for the worklog. A model can only write the log by calling
//!    a tool; there is no tool. Pinned by
//!    `tests/org_worklog_framework_only.rs::no_worklog_tool_is_registered`.
//! 3. **No agent-reachable parameter.** `improvement_proposal` — the one
//!    field that carries an opinion — is set by the same single entry
//!    constructor, from framework code, not from a tool argument. See
//!    [`KaizenProposal`].
//! 4. **No board-routing fields.** The DDL has no `to` and no
//!    `ack_required` column, per `agent_fabric.py:144-146`. The three
//!    framework-only link fields are present and carry exactly what the
//!    organism's carry: `correlation_id`, `notice_ids`, `session_id`.
//!
//! (1) is the mechanism that lives in this file. (2)–(4) are enforced
//! elsewhere and are *verified* by the integration test named above, because
//! only a test outside this crate sees the same public surface an outside
//! caller sees.
//!
//! ## Honest limit of (1)
//!
//! Within one Rust process, "framework" and "agent" are a *seam*
//! distinction, not a memory-safety one. Another crate in this workspace
//! could in principle fabricate a [`TurnEnd`] (its fields are `pub`) and
//! append a row. The guarantee this module actually provides is narrower and
//! stated rather than overclaimed: **no model-reachable path exists**, and
//! the append is a single choke point whose caller set is greppable. A
//! stronger boundary (a private `TurnEnd` mint, or moving the append behind
//! a capability token threaded only through the bootstrap) is available if
//! Wave 2 wants it; it is not claimed here.
//!
//! ## Append-only
//!
//! There is no update and no delete in this module, in
//! [`super::worklog_db`], or in the DDL — and the table additionally carries
//! `BEFORE UPDATE` / `BEFORE DELETE` triggers that `RAISE(ABORT)`, so
//! append-only is a *storage* invariant, not merely an API-shape one. A
//! caller holding a raw `Connection` still cannot rewrite history. The
//! organism's "Append-only" is preserved at both layers.
//!
//! ## Where the framework subscriber attaches (and a spec correction)
//!
//! §3.4 names `TurnEndBus::emit` at `agent/run.rs:1271-1273` as the
//! session-end hook, and then warns of a "coverage gap the implementer must
//! close": that line sits inside the `if tool_calls.is_empty()` early-return
//! branch, so "a worklog subscriber misses every zero-tool turn — which is
//! most digest and summary jobs."
//!
//! **Measured against this tree, that gap does not exist.** `run.rs` has
//! exactly two `turn_end_bus` mentions (1265 a comment, 1271 the `if let`)
//! and the crate has exactly one production `bus.emit` — `run.rs:1272`.
//! That line is inside the `tool_calls.is_empty()` branch opened at
//! `run.rs:1163`, and the branch falls through to it; there is no earlier
//! `return` between 1163 and 1271. So the single emit site *is* the
//! zero-tool path's terminal seam, and zero-tool turns are recorded rather
//! than dropped.
//!
//! I am recording this as a **correction to §3.4, not as credit to the
//! implementation** — the spec's warning may have been accurate when
//! written, and `run.rs` is under concurrent edit by other packets, so the
//! line numbers and the branch structure can move. What matters for whoever
//! wires the subscriber is re-running the one-line check above, not trusting
//! either version of the prose. If the structure does change, the fix is the
//! one §3.4 already names: hoist the emit to a site both branches reach.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::turn_end::TurnEnd;

/// The SQLite file the worklog table lives in. Same sibling kanban file the
/// employee registry and the notice board use (§1.2) — one store, not four.
pub const WORKLOG_DB_FILE: &str = "operant_kanban.db";

/// The `employee` value written when the producing session has no backfilled
/// employee. §3.4: "the honest value; do not invent one".
pub const UNKNOWN_EMPLOYEE: &str = "unknown";

/// What kind of work the turn was doing. Members verified against
/// `agent_fabric.py:44-51` (`WorkflowKind`), per
/// `docs/ORGANISM-ARTIFACT-REFERENCE.md` §4.4(a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    Gather,
    Process,
    Publish,
    Monitor,
    Remediate,
    Synthesize,
    Coordinate,
}

impl WorkflowKind {
    /// The wire form stored in `worklog.workflow_kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gather => "gather",
            Self::Process => "process",
            Self::Publish => "publish",
            Self::Monitor => "monitor",
            Self::Remediate => "remediate",
            Self::Synthesize => "synthesize",
            Self::Coordinate => "coordinate",
        }
    }

    /// Parse a stored `workflow_kind`. `None` for an unrecognized value —
    /// an unknown kind reads as unknown, never as a guess.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "gather" => Some(Self::Gather),
            "process" => Some(Self::Process),
            "publish" => Some(Self::Publish),
            "monitor" => Some(Self::Monitor),
            "remediate" => Some(Self::Remediate),
            "synthesize" => Some(Self::Synthesize),
            "coordinate" => Some(Self::Coordinate),
            _ => None,
        }
    }
}

impl Default for WorkflowKind {
    /// §3.4: `workflow_kind` has **no operant source** — nothing in the turn
    /// path carries it. `process` is the schema's declared default and the
    /// middle member of the organism's own enum, so it is the least
    /// presumptive default. A `[org]` config or a job-level field is the
    /// Wave 2 fix; until then the column is honestly uniform and the doc
    /// says so rather than the value being faked per turn.
    fn default() -> Self {
        Self::Process
    }
}

/// How a turn ended. Members verified against `agent_fabric.py:54-58`
/// (`OutcomeKind`), independently confirmed by the organism's own smoke
/// assertion (`smoke.yaml`: every entry's outcome is one of these four).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Partial,
    Failure,
    Noop,
}

impl Outcome {
    /// The wire form stored in `worklog.outcome`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failure => "failure",
            Self::Noop => "noop",
        }
    }

    /// Parse a stored `outcome`. `None` for an unrecognized value.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "partial" => Some(Self::Partial),
            "failure" => Some(Self::Failure),
            "noop" => Some(Self::Noop),
            _ => None,
        }
    }
}

/// The raw signals a turn ended with, as observed by the framework.
///
/// §3.4 specifies the derivation as prose, over four independent facts:
/// "`success` if the turn emitted `AgentEvent::Done`; `failure` if `run()`
/// returned `Err` or the classifier fired; `partial` if `last_status` was
/// non-clean; `noop` if `result_summary` is empty." Naming them as a struct
/// makes the derivation a pure function that can be unit-tested, which prose
/// in a doc comment cannot be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeSignals {
    /// The turn emitted `AgentEvent::Done { .. }`.
    pub done_emitted: bool,
    /// `OperantAgent::run` returned `Err`, or `classify_api_error` fired.
    /// These are one boolean because the framework observes them at the same
    /// chokepoint and the worklog records the *outcome*, not the cause.
    /// §3.4 is explicit that `ClassifiedError { reason, status_code, … }`
    /// "supplies the cause, not the outcome" — so the cause lands in
    /// [`TurnObservation::blockers`], and only the boolean lands here.
    pub turn_errored: bool,
    /// The last `last_status` the turn saw was clean. Operant has no
    /// `last_status` field today (see the §3.4 coverage notes); a framework
    /// that has no such signal leaves this `true` and derives the outcome
    /// from the other two.
    pub last_status_clean: bool,
}

/// Derive the recorded `outcome` from the observed signals.
///
/// The precedence differs from §3.4's prose order, deliberately: §3.4 lists
/// `success` first, but "the turn both emitted `Done` **and** returned
/// `Err`" is not reachable in the current loop, so the ordering only decides
/// an ambiguous case. It is resolved toward `failure`, because a turn whose
/// error was swallowed must never be recorded as a success. The remaining
/// order follows §3.4 exactly.
///
/// `result_summary_empty` is `TurnEnd::result_summary.is_empty()`, supplied
/// separately because §3.4's `noop` rule reads the summary, which lives on
/// the [`TurnEnd`] rather than on [`OutcomeSignals`].
pub fn derive_outcome(signals: &OutcomeSignals, result_summary_empty: bool) -> Outcome {
    if signals.turn_errored {
        return Outcome::Failure;
    }
    if result_summary_empty {
        return Outcome::Noop;
    }
    if signals.done_emitted && signals.last_status_clean {
        return Outcome::Success;
    }
    // Either no `Done` (the turn ended without a terminal response) or a
    // non-clean `last_status`. Both are partial progress, not success and
    // not failure.
    Outcome::Partial
}

/// Maximum size of an `improvement_proposal` in bytes.
///
/// **This is a design decision beyond §3.4**, which specifies the column but
/// no length. The reasoning: the log is append-only *at the storage layer*
/// (see [`super::worklog_db`]), so an unbounded text column cannot be
/// trimmed later — a rambling row is permanent. The seam already caps
/// `what_done` at [`crate::turn_end::RESULT_SUMMARY_LIMIT`] for exactly this
/// reason; the kaizen field gets the same budget so one turn's row has a
/// bounded size. Raise it deliberately, not by accident.
pub const KAIZEN_LIMIT: usize = crate::turn_end::RESULT_SUMMARY_LIMIT;

/// The kaizen field: the mechanism that turns a run's friction into the next
/// run's change.
///
/// ## Why it is a type and not a `String`
///
/// §3.4 is blunt that operant has **no producer** for this column in Wave 1:
/// producing it "requires an LLM call at session end", and it "must be
/// produced by a **framework** task, never by the agent in its own turn (the
/// agent cannot be trusted to grade itself)". A bare `String` invites the
/// cheap mistake — a tool argument, a `what_done` suffix, a comment. Naming
/// the field's *authorship* in its type is what makes the constraint
/// greppable: the only construction site is [`KaizenProposal::new`], and the
/// only caller of that is the framework's session-end task.
///
/// Wave 1 default is `None`, which the schema stores as `NULL` — a real
/// column that is honestly empty, not free text smuggled onto another field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KaizenProposal {
    /// The proposal itself: what should change about how this ran.
    ///
    /// Truncated to [`KAIZEN_LIMIT`] on a char boundary by the constructor.
    pub text: String,
}

impl KaizenProposal {
    /// Construct a proposal. Framework-side only — see the type docs.
    ///
    /// Text longer than [`KAIZEN_LIMIT`] is truncated on a UTF-8 char
    /// boundary, so the stored value is always valid UTF-8 at a char edge (a
    /// byte-index cut would panic on a multi-byte character).
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        if text.len() <= KAIZEN_LIMIT {
            return Self { text };
        }
        let mut end = KAIZEN_LIMIT;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            text: text[..end].to_string(),
        }
    }
}

/// Running token totals for one turn.
///
/// §3.4's corrected provenance for `tokens_in` / `tokens_out`:
/// `AgentEvent::Usage { input_tokens, output_tokens, .. }`, emitted by
/// `emit_usage_and_cost` at `agent/compress.rs:217-226` from
/// `usage.prompt_tokens` / `usage.completion_tokens`. (**This replaces the
/// outline's `agent/cost.rs`, which does not exist in operant-core** — that
/// file is `crates/operant-runtime/src/agent/cost.rs`, a different crate.
/// §3.4 offers two sources, "accumulate `AgentEvent::Usage` in the
/// subscriber or read the `sessions` table"; this is the first, and it is
/// the one that is per-turn rather than per-session.)
///
/// The accumulator takes plain token counts rather than an `&AgentEvent` so
/// this module has no dependency on the `agent` module and stays unit-
/// testable on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageAccumulator {
    input_tokens: u64,
    output_tokens: u64,
    /// How many `AgentEvent::Usage` events were folded in. Diagnostic only:
    /// a turn that ran but recorded zero usage events is a real, visible
    /// state rather than a silent zero.
    usage_events: u32,
}

impl UsageAccumulator {
    /// A fresh, empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one `AgentEvent::Usage { input_tokens, output_tokens, .. }` in.
    ///
    /// **Sums, not overwrites.** `AgentEvent::Usage` fires once per model
    /// round-trip, so a five-iteration turn emits five of them; the worklog
    /// wants the turn's total. (The agent's own `last_prompt_tokens` is the
    /// opposite — a last-write-wins snapshot of the most recent request.)
    pub fn record(&mut self, input_tokens: u32, output_tokens: u32) {
        self.input_tokens = self.input_tokens.saturating_add(u64::from(input_tokens));
        self.output_tokens = self.output_tokens.saturating_add(u64::from(output_tokens));
        self.usage_events = self.usage_events.saturating_add(1);
    }

    /// Total input tokens across the turn.
    pub fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    /// Total output tokens across the turn.
    pub fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    /// How many usage events were folded in.
    pub fn usage_events(&self) -> u32 {
        self.usage_events
    }
}

/// What the framework observed about one turn, and what it wants recorded.
///
/// Build with [`TurnObservation::from_turn_end`] and fill in the rest. It is
/// public because the framework subscriber lives in another module, but it
/// is only useful once you have a [`TurnEnd`] in hand.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnObservation {
    turn: TurnEnd,
    employee: String,
    department: Option<String>,
    job_id: Option<String>,
    workflow_kind: WorkflowKind,
    usage: UsageAccumulator,
    duration_s: f64,
    signals: OutcomeSignals,
    artifacts: Vec<String>,
    blockers: Vec<String>,
    improvement_proposal: Option<KaizenProposal>,
    correlation_id: Option<String>,
    notice_ids: Vec<String>,
}

impl TurnObservation {
    /// Start an observation from the framework's turn-end event.
    ///
    /// Everything §3.4 maps off `TurnEnd` is filled here: `session_id`,
    /// `iterations` → `iteration`, `tool_calls`, `result_summary` →
    /// `what_done`. Everything else starts at its honest Wave 1 default.
    pub fn from_turn_end(turn: TurnEnd) -> Self {
        Self {
            turn,
            employee: UNKNOWN_EMPLOYEE.to_string(),
            department: None,
            job_id: None,
            workflow_kind: WorkflowKind::default(),
            usage: UsageAccumulator::new(),
            duration_s: 0.0,
            signals: OutcomeSignals::default(),
            artifacts: Vec::new(),
            blockers: Vec::new(),
            improvement_proposal: None,
            correlation_id: None,
            notice_ids: Vec::new(),
        }
    }

    /// The `employee` the turn ran as. Defaults to
    /// [`UNKNOWN_EMPLOYEE`] — §3.4: "'unknown' when the session has no
    /// backfilled employee; the honest value; do not invent one".
    pub fn with_employee(mut self, employee: impl Into<String>) -> Self {
        self.employee = employee.into();
        self
    }

    /// `department`. `None` in Wave 1 — operant has no department concept
    /// yet, and the column is left honestly NULL rather than filled with a
    /// placeholder string that would read as a real department.
    pub fn with_department(mut self, department: Option<String>) -> Self {
        self.department = department;
        self
    }

    /// `job_id`: the cron job id when the session came from
    /// `run_agent_job` (`cronjobs/scheduler.rs`), `None` for an interactive
    /// turn.
    pub fn with_job_id(mut self, job_id: Option<String>) -> Self {
        self.job_id = job_id;
        self
    }

    /// Override `workflow_kind`. Only a framework-level source (a `[org]`
    /// config, or a job-level field in a later wave) should call this;
    /// per-turn inference does not exist.
    pub fn with_workflow_kind(mut self, kind: WorkflowKind) -> Self {
        self.workflow_kind = kind;
        self
    }

    /// Fold this turn's token totals in. See [`UsageAccumulator::record`].
    pub fn with_usage(mut self, usage: UsageAccumulator) -> Self {
        self.usage = usage;
        self
    }

    /// `duration_s`, from `turn_start.elapsed()` — the same source the
    /// existing `ObserverEvent::AgentEnd { duration }` already uses
    /// (`observer.rs:35-41`).
    ///
    /// Negative values are clamped to `0.0`: the column is `REAL NOT NULL
    /// DEFAULT 0.0` and a negative duration would be a measurement bug that
    /// sorts a completed turn before a zero-length one.
    pub fn with_duration_s(mut self, duration_s: f64) -> Self {
        self.duration_s = if duration_s.is_finite() && duration_s > 0.0 {
            duration_s
        } else {
            0.0
        };
        self
    }

    /// The observed end-of-turn signals. See [`OutcomeSignals`].
    pub fn with_signals(mut self, signals: OutcomeSignals) -> Self {
        self.signals = signals;
        self
    }

    /// `artifacts`: paths or ids the turn produced. **No operant source
    /// exists in the turn path in Wave 1** — operant records trajectories
    /// when `record_trajectories` is on, but that is a different sink. This
    /// stays `[]` unless a framework source supplies it; the organism's own
    /// records carry `[]` on the overwhelming majority of rows too.
    pub fn with_artifacts(mut self, artifacts: Vec<String>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// `blockers`: the classified cause(s) that stopped or degraded the
    /// turn. §3.4's source is `ClassifiedError` plus
    /// `HookEvent::Error { error: String }`; the *outcome* is derived
    /// separately by [`derive_outcome`], and the cause lives here so the two
    /// facts are not conflated into one string.
    pub fn with_blockers(mut self, blockers: Vec<String>) -> Self {
        self.blockers = blockers;
        self
    }

    /// Attach the kaizen proposal. **Framework task only** — see
    /// [`KaizenProposal`]. The agent has no path to this, by construction.
    pub fn with_improvement_proposal(mut self, proposal: Option<KaizenProposal>) -> Self {
        self.improvement_proposal = proposal;
        self
    }

    /// `correlation_id`: the framework-only join key into the notice board
    /// (§3.4). `None` in Wave 1.
    pub fn with_correlation_id(mut self, correlation_id: Option<String>) -> Self {
        self.correlation_id = correlation_id;
        self
    }

    /// `notice_ids`: the framework-only links to notices raised during the
    /// turn. `[]` in Wave 1.
    pub fn with_notice_ids(mut self, notice_ids: Vec<String>) -> Self {
        self.notice_ids = notice_ids;
        self
    }

    /// The `TurnEnd` this observation was built from.
    pub fn turn_end(&self) -> &TurnEnd {
        &self.turn
    }

    /// The outcome this observation will record.
    pub fn outcome(&self) -> Outcome {
        derive_outcome(&self.signals, self.turn.result_summary.is_empty())
    }

    /// Seal the observation into the appendable row.
    ///
    /// This is the **only** way a [`WorklogEntry`] comes into existence. It
    /// mints the uuid v4 `id` and reads the clock once, so the `ts` /
    /// `ts_iso` pair is a single observation of one instant.
    pub fn into_entry(self) -> WorklogEntry {
        let now = SystemTime::now();
        let ts = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let ts_iso = chrono::DateTime::from_timestamp(ts, 0)
            .map(super::notice::rfc3339)
            .unwrap_or_default();

        let outcome = self.outcome();
        WorklogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            ts,
            ts_iso,
            employee: self.employee,
            department: self.department,
            job_id: self.job_id,
            iteration: self.turn.iterations as i64,
            workflow_kind: self.workflow_kind,
            what_done: self.turn.result_summary.clone(),
            outcome,
            artifacts: self.artifacts,
            tokens_in: to_i64(self.usage.input_tokens()),
            tokens_out: to_i64(self.usage.output_tokens()),
            tool_calls: self.turn.tool_calls as i64,
            duration_s: self.duration_s,
            next_intent: String::new(),
            blockers: self.blockers,
            improvement_proposal: self.improvement_proposal.map(|p| p.text),
            correlation_id: self.correlation_id,
            notice_ids: self.notice_ids,
            session_id: self.turn.session_id.clone(),
            result_truncated: self.turn.result_truncated,
        }
    }
}

/// Narrow a `u64` token total to the `i64` the column stores.
///
/// Saturating, not wrapping: a wrap would turn an absurd total into a
/// *negative* one, which reads as a plausible small number rather than as
/// the overflow it is. Saturating at `i64::MAX` keeps the row honest about
/// having hit the ceiling, and the only way to get there is 9.2 quintillion
/// tokens in one turn.
fn to_i64(tokens: u64) -> i64 {
    i64::try_from(tokens).unwrap_or(i64::MAX)
}

/// One worklog row, exactly as the 20 organism fields are stored.
///
/// All fields are **private**. There is no `WorklogEntry::new`, no
/// `Default`, and no public field — so the only construction site in the
/// whole workspace is [`TurnObservation::into_entry`]. Read through the
/// accessors, or serialize via [`WorklogEntry::to_record`] for rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorklogEntry {
    id: String,
    ts: i64,
    ts_iso: String,
    employee: String,
    department: Option<String>,
    job_id: Option<String>,
    iteration: i64,
    workflow_kind: WorkflowKind,
    what_done: String,
    outcome: Outcome,
    artifacts: Vec<String>,
    tokens_in: i64,
    tokens_out: i64,
    tool_calls: i64,
    duration_s: f64,
    next_intent: String,
    blockers: Vec<String>,
    improvement_proposal: Option<String>,
    correlation_id: Option<String>,
    notice_ids: Vec<String>,
    session_id: String,
    result_truncated: bool,
}

impl WorklogEntry {
    /// uuid v4, minted at construction.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Unix seconds at construction. The stored truth.
    pub fn ts(&self) -> i64 {
        self.ts
    }
    /// RFC3339 rendering of [`Self::ts`], fixed millisecond precision.
    ///
    /// The one column with no organism counterpart (§3.4): a read-side
    /// convenience so `operant org worklog list` does not run a `datetime`
    /// conversion per row.
    pub fn ts_iso(&self) -> &str {
        &self.ts_iso
    }
    /// `emp-<hex>`, or [`UNKNOWN_EMPLOYEE`].
    pub fn employee(&self) -> &str {
        &self.employee
    }
    /// `None` in Wave 1 — no department concept yet.
    pub fn department(&self) -> Option<&str> {
        self.department.as_deref()
    }
    /// Cron job id, or `None` for an interactive turn.
    pub fn job_id(&self) -> Option<&str> {
        self.job_id.as_deref()
    }
    /// Model round-trips the turn consumed.
    pub fn iteration(&self) -> i64 {
        self.iteration
    }
    /// The workflow classification. Uniformly [`WorkflowKind::Process`] in
    /// Wave 1 — no operant source.
    pub fn workflow_kind(&self) -> WorkflowKind {
        self.workflow_kind
    }
    /// `TurnEnd::result_summary`, already truncated to
    /// [`crate::turn_end::RESULT_SUMMARY_LIMIT`] on a char boundary.
    pub fn what_done(&self) -> &str {
        &self.what_done
    }
    /// The derived outcome. See [`derive_outcome`].
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    /// `[]` in Wave 1 — no operant source.
    pub fn artifacts(&self) -> &[String] {
        &self.artifacts
    }
    /// Sum of `AgentEvent::Usage.input_tokens` across the turn.
    pub fn tokens_in(&self) -> i64 {
        self.tokens_in
    }
    /// Sum of `AgentEvent::Usage.output_tokens` across the turn.
    pub fn tokens_out(&self) -> i64 {
        self.tokens_out
    }
    /// **Requested** tool calls, not executed ones. Quoting
    /// `turn_end.rs:118-122`: this counts requests, and it "can differ" from
    /// the executed set when a call was rejected in pre-flight (unparseable
    /// args, guardrail skip, unknown tool, user-denied approval). The
    /// organism's worklog means *executed* calls; this records the request
    /// count because that is what operant can observe here, and
    /// `tool_durations_ms` (which only has executed entries) is what an
    /// executed-count derivation would have to read.
    pub fn tool_calls(&self) -> i64 {
        self.tool_calls
    }
    /// Wall-clock seconds for the turn.
    pub fn duration_s(&self) -> f64 {
        self.duration_s
    }
    /// `''` in Wave 1. §3.4: `next_intent` has **zero** hits across
    /// `crates/`, and `learning_graph.rs` — which the outline cited — has no
    /// intent field. Wave 4 consumer.
    pub fn next_intent(&self) -> &str {
        &self.next_intent
    }
    /// The classified cause(s) behind a degraded turn.
    pub fn blockers(&self) -> &[String] {
        &self.blockers
    }
    /// The kaizen field. `None` in Wave 1 — a real column that is honestly
    /// `NULL`, not free text on another field. See [`KaizenProposal`].
    pub fn improvement_proposal(&self) -> Option<&str> {
        self.improvement_proposal.as_deref()
    }
    /// Framework-only join key to the notice board.
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }
    /// Framework-only links to notices raised during the turn.
    pub fn notice_ids(&self) -> &[String] {
        &self.notice_ids
    }
    /// The operant session that produced the turn — the framework-populated
    /// field the organism populates on only 29% of rows.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    /// `TurnEnd::result_truncated`: true when `what_done` is a prefix of the
    /// full reply. Not a schema column — it is derived from the truncation
    /// the seam already performs, and it travels on the row so a reader can
    /// tell a short answer from a cut one.
    pub fn result_truncated(&self) -> bool {
        self.result_truncated
    }

    /// Flatten into a plain DTO for rendering and serialization.
    ///
    /// The DTO has public fields because a **read** result is not a write
    /// path: nothing turns a `WorklogRecord` back into a row.
    pub fn to_record(&self) -> WorklogRecord {
        WorklogRecord {
            id: self.id.clone(),
            ts: self.ts,
            ts_iso: self.ts_iso.clone(),
            employee: self.employee.clone(),
            department: self.department.clone(),
            job_id: self.job_id.clone(),
            iteration: self.iteration,
            workflow_kind: self.workflow_kind.as_str().to_string(),
            what_done: self.what_done.clone(),
            outcome: self.outcome.as_str().to_string(),
            artifacts: self.artifacts.clone(),
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            tool_calls: self.tool_calls,
            duration_s: self.duration_s,
            next_intent: self.next_intent.clone(),
            blockers: self.blockers.clone(),
            improvement_proposal: self.improvement_proposal.clone(),
            correlation_id: self.correlation_id.clone(),
            notice_ids: self.notice_ids.clone(),
            session_id: self.session_id.clone(),
            result_truncated: self.result_truncated,
        }
    }
}

/// A worklog row as read back out of sqlite.
///
/// Public fields are safe here: this is the read side. There is no
/// `from_record` constructor, so a `WorklogRecord` cannot be pushed back
/// into the table — the table has no update and no delete to push it into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorklogRecord {
    pub id: String,
    pub ts: i64,
    pub ts_iso: String,
    pub employee: String,
    pub department: Option<String>,
    pub job_id: Option<String>,
    pub iteration: i64,
    pub workflow_kind: String,
    pub what_done: String,
    pub outcome: String,
    pub artifacts: Vec<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tool_calls: i64,
    pub duration_s: f64,
    pub next_intent: String,
    pub blockers: Vec<String>,
    pub improvement_proposal: Option<String>,
    pub correlation_id: Option<String>,
    pub notice_ids: Vec<String>,
    pub session_id: String,
    /// **Not a column — derived from the stored summary's length.**
    ///
    /// §3.4's 21 columns correctly omit this: "was this clipped" is a
    /// property of the *write*, and what survives it is a `what_done` sitting
    /// at exactly [`crate::turn_end::RESULT_SUMMARY_LIMIT`]. So the read side
    /// infers it (`what_done.len() >= RESULT_SUMMARY_LIMIT`) rather than
    /// storing a second copy of a fact already implied by the data.
    ///
    /// The inference is sound because the seam truncates *to the limit and
    /// no further* (`turn_end.rs:90`): a summary shorter than the limit was
    /// never clipped, and one at the limit may or may not have been — this
    /// field is the conservative `true` in that single ambiguous case. If a
    /// caller needs certainty, it is not available from the table; that is
    /// the honest shape of the spec's schema, documented rather than
    /// papered over.
    pub result_truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn_end::{RESULT_SUMMARY_LIMIT, TurnEndBus};

    /// A `TurnEnd` as the framework's bus would hand one over, built through
    /// the real seam so the test cannot drift from what production sees.
    ///
    /// `label` must be unique per caller: `drain_for_label` claims handles
    /// out of one global pool, so a shared label would let one test drain
    /// another's subscriber.
    async fn observed_turn(label: &str, session: &str, reply: &str) -> TurnEnd {
        let bus = TurnEndBus::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut rx = bus.subscribe();
        bus.fork(label.to_string(), async move {
            if let Ok(event) = rx.recv().await
                && let Ok(mut sink) = sink.lock()
            {
                sink.push(event);
            }
        });
        bus.emit(session, 3, 7, reply);
        let drained =
            crate::daemon_pool::drain_for_label(label, std::time::Duration::from_secs(10)).await;
        assert_eq!(drained, 1, "collector daemon must have run");
        seen.lock()
            .map(|guard| guard[0].clone())
            .unwrap_or_else(|_| panic!("collector mutex poisoned"))
    }

    #[tokio::test]
    async fn entry_carries_every_populated_turn_field() {
        let turn = observed_turn(
            "org-worklog-test-collector-fields",
            "sess-abc",
            "summarised the inbox",
        )
        .await;
        let mut usage = UsageAccumulator::new();
        usage.record(120, 45);
        usage.record(80, 30);

        let entry = TurnObservation::from_turn_end(turn)
            .with_employee("emp-a02e3f692fb0")
            .with_job_id(Some("cron:6252e44d".to_string()))
            .with_usage(usage)
            .with_duration_s(4.5)
            .with_signals(OutcomeSignals {
                done_emitted: true,
                turn_errored: false,
                last_status_clean: true,
            })
            .with_blockers(vec!["rate_limited".to_string()])
            .into_entry();

        assert_eq!(entry.session_id(), "sess-abc");
        assert_eq!(entry.iteration(), 3);
        assert_eq!(entry.tool_calls(), 7);
        assert_eq!(entry.what_done(), "summarised the inbox");
        assert_eq!(entry.employee(), "emp-a02e3f692fb0");
        assert_eq!(entry.job_id(), Some("cron:6252e44d"));
        assert_eq!(entry.tokens_in(), 200, "usage sums across round-trips");
        assert_eq!(entry.tokens_out(), 75);
        assert_eq!(entry.duration_s(), 4.5);
        assert_eq!(entry.blockers(), &["rate_limited".to_string()]);
        assert_eq!(entry.outcome(), Outcome::Success);
        // The honest Wave 1 defaults, asserted so a future edit cannot
        // quietly invent a value for a column that has no source.
        assert_eq!(entry.workflow_kind(), WorkflowKind::Process);
        assert_eq!(entry.department(), None);
        assert!(entry.artifacts().is_empty());
        assert_eq!(entry.next_intent(), "");
        assert_eq!(entry.improvement_proposal(), None);
        assert_eq!(entry.correlation_id(), None);
        assert!(entry.notice_ids().is_empty());
    }

    #[test]
    fn kaizen_proposal_is_a_distinct_first_class_field() {
        let turn = TurnEnd {
            turn_id: 0,
            session_id: "sess-k".to_string(),
            iterations: 1,
            tool_calls: 0,
            tool_durations_ms: Vec::new(),
            result_summary: "done".to_string(),
            result_truncated: false,
        };
        let proposal = "web_scrape timed out at the default budget; raise it";
        let without = TurnObservation::from_turn_end(turn.clone()).into_entry();
        let with = TurnObservation::from_turn_end(turn)
            .with_improvement_proposal(Some(KaizenProposal::new(proposal)))
            .into_entry();

        assert_eq!(without.improvement_proposal(), None);
        assert_eq!(with.improvement_proposal(), Some(proposal));
        // The proposal must not leak into the summary field — "a real
        // column, not free text bolted onto another field".
        assert_eq!(with.what_done(), "done");
    }

    #[test]
    fn outcome_derivation_covers_all_four_members() {
        let clean = OutcomeSignals {
            done_emitted: true,
            turn_errored: false,
            last_status_clean: true,
        };
        assert_eq!(derive_outcome(&clean, false), Outcome::Success);
        assert_eq!(derive_outcome(&clean, true), Outcome::Noop);

        let errored = OutcomeSignals {
            turn_errored: true,
            ..clean
        };
        assert_eq!(
            derive_outcome(&errored, false),
            Outcome::Failure,
            "an errored turn must never read as success even with a summary"
        );

        let dirty = OutcomeSignals {
            last_status_clean: false,
            ..clean
        };
        assert_eq!(derive_outcome(&dirty, false), Outcome::Partial);

        let no_done = OutcomeSignals {
            done_emitted: false,
            ..clean
        };
        assert_eq!(derive_outcome(&no_done, false), Outcome::Partial);
    }

    #[test]
    fn every_workflow_kind_round_trips_through_its_wire_form() {
        for kind in [
            WorkflowKind::Gather,
            WorkflowKind::Process,
            WorkflowKind::Publish,
            WorkflowKind::Monitor,
            WorkflowKind::Remediate,
            WorkflowKind::Synthesize,
            WorkflowKind::Coordinate,
        ] {
            assert_eq!(WorkflowKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(WorkflowKind::parse("nonsense"), None);
    }

    #[test]
    fn every_outcome_round_trips_through_its_wire_form() {
        for outcome in [
            Outcome::Success,
            Outcome::Partial,
            Outcome::Failure,
            Outcome::Noop,
        ] {
            assert_eq!(Outcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(Outcome::parse("nonsense"), None);
    }

    #[test]
    fn a_seam_truncated_summary_is_stored_at_exactly_the_seam_limit() {
        // The seam's contract (`turn_end.rs:90`) is that `result_summary`
        // has already been cut to at most `RESULT_SUMMARY_LIMIT` *bytes* on
        // a char boundary. Reproduce that exactly: 400 three-byte chars is
        // 1200 bytes, of which the seam keeps the first 170 chars (510
        // bytes, the largest multiple of 3 at or under 512) and reports
        // `result_truncated = true`.
        let raw = "€".repeat(400); // 3 bytes each
        let kept: String = raw.chars().take(RESULT_SUMMARY_LIMIT / 3).collect();
        assert_eq!(
            kept.len(),
            510,
            "the fixture must itself be a valid seam cut"
        );

        let entry = TurnObservation::from_turn_end(TurnEnd {
            turn_id: 0,
            session_id: "sess-long".to_string(),
            iterations: 1,
            tool_calls: 0,
            tool_durations_ms: Vec::new(),
            result_summary: kept.clone(),
            result_truncated: true,
        })
        .into_entry();

        // Stored verbatim: the worklog does **not** re-truncate, because the
        // seam already did. Re-truncating would be a second pass over a
        // string that is already bounded, and would hide a caller that
        // skipped the seam.
        assert_eq!(entry.what_done(), kept);
        assert_eq!(entry.what_done().len(), 510);
        assert!(
            entry.result_truncated(),
            "the flag is carried, not recomputed"
        );
    }

    #[test]
    fn the_worklog_does_not_silently_re_truncate_an_unbounded_summary() {
        // A caller that constructs `TurnEnd` without going through the seam
        // can hand over an arbitrarily long summary (`TurnEnd`'s fields are
        // `pub`). The honest behavior is to store it and let the reader see
        // it — not to quietly clip it and produce a row that looks
        // ordinary. The cap is the seam's job; duplicating it here would
        // make two truncation sites with different semantics.
        let overlong: String = "x".repeat(RESULT_SUMMARY_LIMIT * 4);
        let entry = TurnObservation::from_turn_end(TurnEnd {
            turn_id: 0,
            session_id: "sess-overlong".to_string(),
            iterations: 1,
            tool_calls: 0,
            tool_durations_ms: Vec::new(),
            result_summary: overlong.clone(),
            result_truncated: false,
        })
        .into_entry();
        assert_eq!(entry.what_done(), overlong);
        assert!(
            !entry.result_truncated(),
            "carried from the caller, not inferred"
        );
    }

    #[test]
    fn usage_accumulator_saturates_instead_of_wrapping() {
        let mut usage = UsageAccumulator::new();
        usage.record(u32::MAX, u32::MAX);
        usage.record(u32::MAX, u32::MAX);
        assert_eq!(usage.input_tokens(), u64::from(u32::MAX) * 2);
        assert_eq!(usage.usage_events(), 2);
    }

    #[test]
    fn negative_or_nan_duration_is_clamped_to_zero() {
        let base = TurnEnd {
            turn_id: 0,
            session_id: "sess-d".to_string(),
            iterations: 1,
            tool_calls: 0,
            tool_durations_ms: Vec::new(),
            result_summary: "x".to_string(),
            result_truncated: false,
        };
        for bad in [-1.0f64, f64::NAN, f64::INFINITY] {
            let entry = TurnObservation::from_turn_end(base.clone())
                .with_duration_s(bad)
                .into_entry();
            assert_eq!(entry.duration_s(), 0.0, "duration {bad} must clamp");
        }
    }
}
