//! Worklog value types and the cron-job seat-id derivation.
//!
//! The organism's *objective* temporal log (`docs/WAVE1-DECISIONS.md` §3.4)
//! was ported here at iter-106 with its sealed-entry machinery: a private-field
//! `WorklogEntry`, its only constructor `TurnObservation::into_entry`, and the
//! `WorklogRecord` read shape. The sqlite store retired with the org layer
//! (G+1, iter-838); Holon's `runs`, `outputs` and hash-chained `events` are the
//! objective log now. **2026-10-26:** the sealed-entry types were measured to
//! have no producer and no consumer anywhere in the workspace outside this
//! file's own tests, and were cut (ponytail audit of the operant/holon corpus,
//! `docs/plans/2026-10-26-two-repo-ponytail-cut-and-integration-repair.md`).
//!
//! What remains is what still has callers:
//!
//! - [`WorkflowKind`] and [`Outcome`] — the wire forms a run's outcome reads
//!   through, still used by the value types below;
//! - [`derive_outcome`] turning [`OutcomeSignals`] into an [`Outcome`];
//! - [`KaizenProposal`] — the one field of the retired row that carried an
//!   opinion, kept as a type;
//! - [`UsageAccumulator`] — the only part that earns its keep: it folds a
//!   turn's token totals and feeds `result.usage`, read by the personal cron
//!   scheduler and the gateway's usage meter;
//! - [`derive_employee_id`] — the deterministic seat id a cron job derives for
//!   its session key, moved here from the retired `employee` module and still
//!   called by the scheduler, `cmd_cron` and their isolation tests.
//!
//! Spec reference (unchanged, for provenance): `docs/WAVE1-DECISIONS.md` §3.4
//! and `docs/ORGANISM-ARTIFACT-REFERENCE.md` §4.4(a).

use serde::{Deserialize, Serialize};

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
    /// `TurnObservation::blockers`, and only the boolean lands here.
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
/// (append-only is a storage invariant, not merely an API-shape one), so an
/// unbounded text column cannot be
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
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageAccumulator {
    input_tokens: u64,
    output_tokens: u64,
    /// Cumulative models_dev-estimated cost (`AgentEvent::Cost`), USD.
    /// iter-678: folded by the cron scheduler's usage meter — usd-basis
    /// budget envelopes need it, and the future TurnEnd wiring inherits
    /// the field for free. (`Eq` dropped from the derive: f64 is only
    /// `PartialEq`.)
    cost_usd: f64,
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

    /// Fold one `AgentEvent::Cost` estimate in (USD).
    pub fn record_cost(&mut self, cost_usd: f64) {
        self.cost_usd += cost_usd;
    }

    /// Cumulative estimated cost in USD.
    pub fn cost_usd(&self) -> f64 {
        self.cost_usd
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

/// The deterministic seat id a cron job derives (moved from the retired
/// `employee` module, G+1): `emp-` + the first 12 bytes of the job id.
/// Deterministic, stable under re-runs, and reproduces the organism's
/// shape on hex ids. The scheduler still uses it for session keys.
pub fn derive_employee_id(cron_job_id: &str) -> String {
    const EMPLOYEE_ID_PREFIX: &str = "emp-";
    const EMPLOYEE_ID_HEX_LEN: usize = 12;
    let mut out = String::from(EMPLOYEE_ID_PREFIX);
    let mut taken = 0usize;
    for ch in cron_job_id.chars() {
        if taken >= EMPLOYEE_ID_HEX_LEN {
            break;
        }
        let len = ch.len_utf8();
        if taken + len > EMPLOYEE_ID_HEX_LEN {
            break;
        }
        out.push(ch);
        taken += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn usage_accumulator_saturates_instead_of_wrapping() {
        let mut usage = UsageAccumulator::new();
        usage.record(u32::MAX, u32::MAX);
        usage.record(u32::MAX, u32::MAX);
        assert_eq!(usage.input_tokens(), u64::from(u32::MAX) * 2);
        assert_eq!(usage.usage_events(), 2);
    }
}
