// Vendored from jcode (crates/operant-task-types/src/lib.rs + crates/operant-base/src/todo.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// two upstream modules unified into one file (upstream reaches task-types via
// operant-base/src/todo.rs:29's `pub use operant_task_types::{...}`). See
// operant_app/mod.rs for scope.
//!
//! Partial port — the renderer-referenced subset of the todo/feedback type
//! family. Included from operant-task-types/src/lib.rs: the `semantic_state!`
//! macro (:197-:296) and all ten of its invocations — IntentUnderstanding
//! (:299), IterationMaturity (:309, plus its `permits_completion` impl at
//! :325), FeedbackLoopState (:340), FeedbackLoopRelevance (:352),
//! FeedbackLoopCoverage (:364), FeedbackLoopTraceability (:374),
//! ConfidenceState (:384), Difficulty (:395), Autonomy (:410), DeliveryState
//! (:421) — and the structs TodoItem (:433), TodoPlan (:472), TodoPlanField
//! (:501), TodoPlanChange (:509), TodoGoal (:526), TodoGoalField (:619),
//! TodoGoalChange (:635). Included from operant-base/src/todo.rs: the five
//! pass-fns (intent_understanding_passes :73, feedback_loop_passes :78,
//! feedback_loop_relevance_passes :103, feedback_loop_coverage_passes :108,
//! feedback_loop_traceability_passes :123) — which call the private-to-upstream
//! threshold fns required_feedback_loop_relevance (:85),
//! required_feedback_loop_coverage (:95),
//! required_feedback_loop_traceability (:113), ported too — and the two consts
//! TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE (:188) and
//! TODO_OWNERSHIP_CONTINUATION_MESSAGE (:200).
//! [port-excision] Not ported (unreferenced by the ported renderers): Goal*/
//! sanitize_goal_id/slugify/default_pending_status, PersistedCatchupState,
//! CatchupBrief, the PRE_*/LEGACY_* sibling consts, and operant-base/todo.rs's
//! server-side gate logic. operant-tui's `crate::todo::*` resolves here.

use serde::{Deserialize, Serialize};

/// Declare a semantic assessment enum that serializes as a snake_case string
/// but still deserializes legacy 0-100 numeric scores (and numeric strings)
/// from sessions recorded before the semantic-state migration.
macro_rules! semantic_state {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $variant:ident = $label:literal, legacy: $lo:literal ..= $hi:literal, score: $score:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $( $variant, )+
        }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( Self::$variant => $label, )+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value.trim().to_ascii_lowercase().as_str() {
                    $( $label => Some(Self::$variant), )+
                    _ => None,
                }
            }

            /// Map a legacy 0-100 numeric score onto the closest state.
            pub fn from_legacy_score(score: u8) -> Self {
                match score.min(100) {
                    $( $lo..=$hi => Self::$variant, )+
                    // Unreachable: the ranges cover 0..=100 and the input is
                    // clamped, but match exhaustiveness cannot see that.
                    _ => Self::from_legacy_score(100),
                }
            }

            /// Representative 0-100 score for consumers (telemetry) that still
            /// aggregate numerically.
            pub fn legacy_score(&self) -> u8 {
                match self {
                    $( Self::$variant => $score, )+
                }
            }

            /// How many ordered levels separate two states.
            pub fn level(&self) -> u8 {
                *self as u8
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                use serde::de::Error;
                let value = serde_json::Value::deserialize(deserializer)?;
                match value {
                    serde_json::Value::String(raw) => {
                        let trimmed = raw.trim();
                        if let Some(parsed) = Self::parse(trimmed) {
                            return Ok(parsed);
                        }
                        // Legacy numeric score sent as a string.
                        if let Ok(score) = trimmed.parse::<f64>() {
                            if (0.0..=100.0).contains(&score) {
                                return Ok(Self::from_legacy_score(score as u8));
                            }
                        }
                        Err(D::Error::custom(format!(
                            concat!("invalid ", stringify!($name), " state: {}"),
                            raw
                        )))
                    }
                    serde_json::Value::Number(num) => {
                        let score = num
                            .as_f64()
                            .filter(|score| (0.0..=100.0).contains(score))
                            .ok_or_else(|| {
                                D::Error::custom(concat!(
                                    "legacy ",
                                    stringify!($name),
                                    " score out of 0-100 range"
                                ))
                            })?;
                        Ok(Self::from_legacy_score(score as u8))
                    }
                    other => Err(D::Error::custom(format!(
                        concat!(stringify!($name), " must be a state string, got {}"),
                        other
                    ))),
                }
            }
        }
    };
}

semantic_state! {
    /// How well the agent understands what the user actually wants.
    IntentUnderstanding {
        Uncertain = "uncertain", legacy: 0..=59, score: 40,
        Partial = "partial", legacy: 60..=95, score: 80,
        Clear = "clear", legacy: 96..=99, score: 96,
        Complete = "complete", legacy: 100..=100, score: 100,
    }
}

semantic_state! {
    /// Whether an iterative goal has a defensible reason to stop. Unlike
    /// feedback-loop quality, this records how far that loop has actually been
    /// exercised rather than whether progress can be measured in principle.
    IterationMaturity {
        NotStarted = "not_started", legacy: 0..=12, score: 6,
        Exploring = "exploring", legacy: 13..=29, score: 21,
        Improving = "improving", legacy: 30..=49, score: 40,
        PlateauUnproven = "plateau_unproven", legacy: 50..=69, score: 60,
        OutcomeReached = "outcome_reached", legacy: 70..=79, score: 74,
        ConstraintsExhausted = "constraints_exhausted", legacy: 80..=87, score: 84,
        PlateauConfirmed = "plateau_confirmed", legacy: 88..=95, score: 92,
        BudgetExhausted = "budget_exhausted", legacy: 96..=100, score: 98,
    }
}

semantic_state! {
    /// How much of a goal's correctness its feedback loop can report on its
    /// own, without the agent's or the user's judgment.
    FeedbackLoopState {
        Absent = "absent", legacy: 0..=19, score: 10,
        Weak = "weak", legacy: 20..=49, score: 35,
        Usable = "usable", legacy: 50..=79, score: 65,
        Strong = "strong", legacy: 80..=95, score: 88,
        Closed = "closed", legacy: 96..=100, score: 98,
    }
}

semantic_state! {
    /// How directly a goal's feedback loop represents the behavior or outcome
    /// the user will actually accept.
    FeedbackLoopRelevance {
        Indirect = "indirect", legacy: 0..=24, score: 12,
        Synthetic = "synthetic", legacy: 25..=49, score: 37,
        Representative = "representative", legacy: 50..=79, score: 75,
        AcceptanceBlocked = "acceptance_blocked", legacy: 80..=95, score: 88,
        AcceptanceAligned = "acceptance_aligned", legacy: 96..=100, score: 98,
    }
}

semantic_state! {
    /// How broadly a goal's feedback loop exercises the paths on which the
    /// result can succeed or fail.
    FeedbackLoopCoverage {
        Narrow = "narrow", legacy: 0..=49, score: 25,
        MainPaths = "main_paths", legacy: 50..=95, score: 75,
        EdgeAndIntegrationPaths = "edge_and_integration_paths", legacy: 96..=100, score: 98,
    }
}

semantic_state! {
    /// How completely a goal's explicit requirements and changed public outputs
    /// are connected to concrete checks and observed results.
    FeedbackLoopTraceability {
        Unmapped = "unmapped", legacy: 0..=49, score: 25,
        Partial = "partial", legacy: 50..=95, score: 75,
        Complete = "complete", legacy: 96..=100, score: 98,
    }
}

semantic_state! {
    /// Evidence state behind a todo: from an unexamined guess to a result
    /// verified end to end.
    ConfidenceState {
        Speculative = "speculative", legacy: 0..=59, score: 40,
        Plausible = "plausible", legacy: 60..=95, score: 80,
        Validated = "validated", legacy: 96..=99, score: 96,
        Verified = "verified", legacy: 100..=100, score: 100,
    }
}

semantic_state! {
    /// Intrinsic difficulty of a goal. Descriptive, never gated: it only
    /// calibrates how much delivery follow-through a completion review expects.
    Difficulty {
        Trivial = "trivial", legacy: 0..=12, score: 6,
        Routine = "routine", legacy: 13..=25, score: 19,
        Involved = "involved", legacy: 26..=38, score: 32,
        Complex = "complex", legacy: 39..=51, score: 45,
        Hard = "hard", legacy: 52..=64, score: 58,
        Expert = "expert", legacy: 65..=77, score: 71,
        Research = "research", legacy: 78..=90, score: 84,
        OpenEnded = "open_ended", legacy: 91..=100, score: 95,
    }
}

semantic_state! {
    /// How far beyond the literal request the agent's work extended.
    /// Descriptive, never gated.
    Autonomy {
        RequestedOnly = "requested_only", legacy: 0..=25, score: 12,
        NecessaryFollowthrough = "necessary_followthrough", legacy: 26..=50, score: 38,
        Proactive = "proactive", legacy: 51..=75, score: 62,
        Stewardship = "stewardship", legacy: 76..=100, score: 88,
    }
}

semantic_state! {
    /// How far a goal's result actually traveled toward the user's outcome.
    /// Legacy scores come from the old `end_to_end_ownership` field.
    DeliveryState {
        ChangeMade = "change_made", legacy: 0..=49, score: 25,
        Integrated = "integrated", legacy: 50..=79, score: 65,
        WorkflowValidated = "workflow_validated", legacy: 80..=95, score: 88,
        OutcomeDelivered = "outcome_delivered", legacy: 96..=100, score: 98,
    }
}

impl IterationMaturity {
    /// Terminal bases that can justify completing a goal. `BudgetExhausted` is
    /// deliberately terminal but not "better" than a confirmed plateau; the
    /// ordering exists only for legacy numeric compatibility.
    pub fn permits_completion(self) -> bool {
        matches!(
            self,
            Self::OutcomeReached
                | Self::ConstraintsExhausted
                | Self::PlateauConfirmed
                | Self::BudgetExhausted
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub priority: String,
    pub id: String,
    /// Optional group label. Todos that share a group are displayed together
    /// under a single header. Use one group per coherent goal; when work is
    /// steered into a new area, start a new group instead of renaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Forward-looking evidence state that this todo can be completed
    /// correctly. Legacy sessions stored a 0-100 score; numbers map onto the
    /// closest state on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceState>,
    /// Evidence state recorded when the todo is marked completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_confidence: Option<ConfidenceState>,
    /// Every distinct confidence state this todo has carried, oldest first,
    /// ending with the current one. Maintained by the todo tool (not the
    /// model): the first entry is the planning-time assessment, later entries
    /// record how it evolved while the item was worked on. This preserves the
    /// planning signal even after the model overwrites `confidence` when
    /// marking the item done.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub confidence_history: Vec<ConfidenceState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
}

/// Plan-level understanding of what the user actually wants, covering the
/// whole todo list rather than one group.
///
/// Intent is a property of the request, not of an individual group of steps,
/// so it is recorded once per plan: what the user is really after, and how
/// faithfully the plan and its feedback loops represent that.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPlan {
    /// The user's underlying reason and desired outcome for this work, kept
    /// distinct from the agent's steps and validation loops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_intention: Option<String>,
    /// How well the agent understands what the user actually wants and how
    /// faithfully this plan represents it. It does not measure implementation
    /// progress. Older payloads stored a 0-100 score under this name or
    /// `alignment_score`; numbers map onto the closest state on load.
    #[serde(
        default,
        alias = "alignment_score",
        alias = "user_intention_alignment",
        skip_serializing_if = "Option::is_none"
    )]
    pub understands_user_intent: Option<IntentUnderstanding>,
    /// Every distinct `understands_user_intent` state this plan has carried,
    /// oldest first, ending with the current one. Maintained by the todo tool,
    /// not the model: understanding of a request typically starts low and rises
    /// as the agent explores, so the trajectory distinguishes an agent that
    /// resolved the ambiguity by investigating from one that never did.
    /// Model-supplied values are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub understands_user_intent_history: Vec<IntentUnderstanding>,
}

/// A plan field changed by a todo-tool update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoPlanField {
    UserIntention,
    #[serde(alias = "alignment_score", alias = "user_intention_alignment")]
    UnderstandsUserIntent,
}

/// Before/after state for the plan-level intent assessment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPlanChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<TodoPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<TodoPlan>,
    pub fields: Vec<TodoPlanField>,
}

/// A goal-level assessment attached to a todo group (or, for an ungrouped
/// flat list, the whole list as one implicit goal with `group: None`).
///
/// A closed feedback loop is a property of an objective, not of individual
/// steps: "optimize grep latency" can close its loop because progress has a
/// metric, while "design an onboarding screen" cannot because success is a
/// taste judgment. Items like "read the auth code" have no meaningful score of
/// their own, so the score lives here instead of on `TodoItem`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoGoal {
    /// Group label this goal describes. `None` covers the ungrouped list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// How much of this goal's correctness `feedback_loop` can report on its
    /// own, without the agent's judgment or the user's. Legacy sessions stored
    /// a 0-100 score; numbers map onto the closest state on load.
    #[serde(
        default,
        alias = "hill_climbability",
        skip_serializing_if = "Option::is_none"
    )]
    pub closed_feedback_loop: Option<FeedbackLoopState>,
    /// Every distinct `closed_feedback_loop` state this goal has carried, oldest
    /// first. Tool-maintained; model-supplied values are ignored.
    #[serde(
        default,
        alias = "hill_climbability_history",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub closed_feedback_loop_history: Vec<FeedbackLoopState>,
    /// The concrete feedback loop used to judge whether each iteration improves
    /// the outcome (e.g. a benchmark command and the metric it reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_loop: Option<String>,
    /// How directly `feedback_loop` represents the public behavior and outcome
    /// the user will actually judge, rather than a proxy or internal detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_loop_relevance: Option<FeedbackLoopRelevance>,
    /// Every distinct `feedback_loop_relevance` state this goal has carried,
    /// oldest first. Tool-maintained; model-supplied values are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback_loop_relevance_history: Vec<FeedbackLoopRelevance>,
    /// How broadly `feedback_loop` exercises main paths, integration boundaries,
    /// edge cases, packaging, and likely failure modes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_loop_coverage: Option<FeedbackLoopCoverage>,
    /// Every distinct `feedback_loop_coverage` state this goal has carried,
    /// oldest first. Tool-maintained; model-supplied values are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback_loop_coverage_history: Vec<FeedbackLoopCoverage>,
    /// Whether every explicit requirement and changed public output is mapped to
    /// a concrete check and its observed result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_loop_traceability: Option<FeedbackLoopTraceability>,
    /// Every distinct `feedback_loop_traceability` state this goal has carried,
    /// oldest first. Tool-maintained; model-supplied values are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback_loop_traceability_history: Vec<FeedbackLoopTraceability>,
    /// How far the goal's result actually traveled toward the user's outcome:
    /// from a bare change through integration and workflow validation to a
    /// delivered outcome. Replaces the legacy 0-100 `end_to_end_ownership`
    /// score, which maps onto the closest state on load.
    #[serde(
        default,
        alias = "end_to_end_ownership",
        skip_serializing_if = "Option::is_none"
    )]
    pub delivery_state: Option<DeliveryState>,
    /// Every distinct `delivery_state` this goal has carried, oldest first.
    /// Tool-maintained; model-supplied values are ignored.
    #[serde(
        default,
        alias = "end_to_end_ownership_history",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub delivery_state_history: Vec<DeliveryState>,
    /// Intrinsic difficulty of this goal. Descriptive, never gated: it only
    /// calibrates how much delivery follow-through a completion review expects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
    /// How far beyond the literal request the work extended. Completion is
    /// gated at `necessary_followthrough` so consequential adjacent work is not
    /// silently left to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<Autonomy>,
    /// How far an iterative feedback loop has progressed and, at completion,
    /// the semantic basis for stopping. This is distinct from loop quality: a
    /// perfectly measurable loop may still be actively improving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration_maturity: Option<IterationMaturity>,
    /// Evidence that an open-ended search has reached a defensible stopping
    /// point, such as a plateau across distinct approaches, exhausted
    /// hypotheses, or an explicit budget limit. Required only when a
    /// research or open-ended goal is marked complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopping_evidence: Option<String>,
}

/// A goal field changed by a todo-tool update. This lets transcript renderers
/// show a concise quality-gate refinement instead of repeating the full plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoGoalField {
    #[serde(alias = "hill_climbability")]
    ClosedFeedbackLoop,
    FeedbackLoop,
    FeedbackLoopRelevance,
    FeedbackLoopCoverage,
    FeedbackLoopTraceability,
    #[serde(alias = "end_to_end_ownership")]
    DeliveryState,
    Autonomy,
    IterationMaturity,
    StoppingEvidence,
}

/// Before/after state for one changed todo goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoGoalChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<TodoGoal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<TodoGoal>,
    pub fields: Vec<TodoGoalField>,
}

// [port-decision] dedup/excision followup: PersistedCatchupState's derive and
// its HashMap import were left behind when the struct was excised per the
// file-header [port-excision]; deleted both (unused after excision).

/// Minimum directness expected from a completion check. More involved goals
/// need checks aligned with acceptance behavior rather than a representative
/// proxy alone.
pub fn required_feedback_loop_relevance(difficulty: Option<Difficulty>) -> FeedbackLoopRelevance {
    if difficulty.is_some_and(|difficulty| difficulty >= Difficulty::Involved) {
        FeedbackLoopRelevance::AcceptanceAligned
    } else {
        FeedbackLoopRelevance::Representative
    }
}

/// Minimum breadth expected from a completion check. More involved goals must
/// include edge cases and integration boundaries as well as their main paths.
pub fn required_feedback_loop_coverage(difficulty: Option<Difficulty>) -> FeedbackLoopCoverage {
    if difficulty.is_some_and(|difficulty| difficulty >= Difficulty::Involved) {
        FeedbackLoopCoverage::EdgeAndIntegrationPaths
    } else {
        FeedbackLoopCoverage::MainPaths
    }
}

pub fn required_feedback_loop_traceability(
    difficulty: Option<Difficulty>,
) -> FeedbackLoopTraceability {
    if difficulty.is_some_and(|difficulty| difficulty >= Difficulty::Involved) {
        FeedbackLoopTraceability::Complete
    } else {
        FeedbackLoopTraceability::Partial
    }
}

/// Whether the plan's intent understanding is solid enough to work against.
pub fn intent_understanding_passes(state: Option<IntentUnderstanding>) -> bool {
    state.is_some_and(|state| state >= IntentUnderstanding::Clear)
}

/// Whether a goal's feedback loop reports back on the requirements by itself.
pub fn feedback_loop_passes(state: Option<FeedbackLoopState>) -> bool {
    state.is_some_and(|state| state >= FeedbackLoopState::Closed)
}

pub fn feedback_loop_relevance_passes(goal: &TodoGoal) -> bool {
    goal.feedback_loop_relevance
        .is_some_and(|state| state >= required_feedback_loop_relevance(goal.difficulty))
}

pub fn feedback_loop_coverage_passes(goal: &TodoGoal) -> bool {
    goal.feedback_loop_coverage
        .is_some_and(|state| state >= required_feedback_loop_coverage(goal.difficulty))
}

pub fn feedback_loop_traceability_passes(goal: &TodoGoal) -> bool {
    goal.feedback_loop_traceability
        .is_some_and(|state| state >= required_feedback_loop_traceability(goal.difficulty))
}

/// Model-facing continuation for the private closed-feedback-loop check. Names
/// the assessment category without disclosing the score or threshold.
pub const TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE: &str = "[auto] Your feedback loop isn't good enough. Think about what feedback loops you need. Make sure the todo is up to date.";

/// Model-facing continuation for the private end-to-end ownership check. It
/// asks for more work without revealing that an evaluator triggered it.
pub const TODO_OWNERSHIP_CONTINUATION_MESSAGE: &str =
    "[auto] Continue the work below. Keep the todo up to date; do not reply or wait for the user.";

// [port-source] batch-4 extension of this leaf (upstream operant-base/src/todo.rs):
// the auto-poke continuation family the session renderer now classifies with —
// is_auto_poke_message (:743), auto_poke_display_summary (:787), and the
// const set they read: TODO_LONG_SESSION_REVIEW_MESSAGE (:9),
// PRE_COMPACT_TODO_LONG_SESSION_REVIEW_MESSAGE (:10),
// PRE_BUDGET_TODO_LONG_SESSION_REVIEW_MESSAGE (:11),
// LEGACY_TODO_ALIGNMENT_CONTINUATION_MESSAGE (:174),
// TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE (:177),
// PRE_COMPACT_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE (:178),
// PRE_CONCISE_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE (:182),
// PRE_TODO_REMINDER_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE (:183),
// PRE_COMPACT_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE (:189),
// PRE_TODO_REMINDER_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE (:190),
// PRE_BUDGET_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE (:191),
// LEGACY_TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE (:196),
// PRE_COMPACT_TODO_OWNERSHIP_CONTINUATION_MESSAGE (:202),
// LEGACY_TODO_OWNERSHIP_CONTINUATION_MESSAGE (:279),
// TODO_COMPLETION_CONTINUATION_MESSAGE (:282),
// PRE_COMPACT_TODO_COMPLETION_CONTINUATION_MESSAGE (:283),
// TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE (:287),
// PRE_COMPACT_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE (:288),
// TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE (:293),
// PRE_NARROWED_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE (:294),
// PRE_COMPACT_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE (:295),
// PRE_BUDGET_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE (:296),
// TODO_GATE_DIGEST_PREFIX (:347), PRE_COMPACT_TODO_GATE_DIGEST_PREFIX (:348),
// LABELED_TODO_GATE_DIGEST_PREFIX (:349), LEGACY_TODO_CONFIDENCE_SUMMARY_PREFIX
// (:524), LEGACY_TODO_COMPLETION_CONTINUATION_MESSAGE (:528),
// LEGACY_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE (:530),
// PRE_EVIDENCE_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE (:534); plus the
// persisted-io slice the picker reads — load_todos (:849), todos_exist (:854,
// not ported yet), save_todos (:860), todo_path (:871), derive_session_title
// (:956), load_session_title (:988), load_plan (:1008), plan_path (:1027).
// TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE and
// TODO_OWNERSHIP_CONTINUATION_MESSAGE already lived in this leaf (header :540,
// :544) and are not re-ported.

pub const TODO_LONG_SESSION_REVIEW_MESSAGE: &str = "[auto] Re-read the request. Update the todo plan and goal assessments from the evidence gathered so far. Correct anything stale or overstated, then continue the work. Do not reply or wait for the user.";
const PRE_COMPACT_TODO_LONG_SESSION_REVIEW_MESSAGE: &str = "[automated todo assessment review - not a user message] Re-read the request. Update the todo plan and goal assessments from the evidence gathered so far. Correct anything stale or overstated, then continue the work. Do not reply or wait for the user.";
const PRE_BUDGET_TODO_LONG_SESSION_REVIEW_MESSAGE: &str = "[automated todo assessment review - not a user message] Re-read the original request and reconsider the current todo plan and every goal assessment using the evidence gathered during the work so far. Correct anything stale or overstated, including intent understanding, feedback-loop relevance and coverage, autonomy, difficulty, delivery, confidence, iteration maturity, and stopping evidence. Do not reply conversationally or wait for the user. Continue the work after saving an honest updated assessment.";

const LEGACY_TODO_ALIGNMENT_CONTINUATION_MESSAGE: &str = "Your alignment score is not high enough. Build a requirement inventory from the user's request, including outcomes, deliverables, constraints, prohibited actions, integration paths, edge cases, and necessary follow-through. Revise the plan and its stated user intention to represent every material item. Then map each item to an explicit observation or check in a feedback loop. Generic instructions to run tests, verify, or review count only for requirements those checks actually enforce; add separate checks for non-testable requirements. Reassess the weaker link before continuing the task.";

/// Model-facing continuation for the private intent-understanding check.
pub const TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE: &str = "[auto] Understand the user's intent better. Try to avoid asking the user. Make sure the todo is up to date.";
const PRE_COMPACT_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE: &str = "Understand the user's intent better. Try to avoid asking the user. Make sure the todo is up to date.";

/// Previous verbose wording, retained so persisted sessions still classify it
/// as a hidden quality-gate message after the concise rewrite.
const PRE_CONCISE_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE: &str = "Your understanding of the user's intent is not high enough. Re-read the request and think harder about what the user actually wants and left implicit, using the conversation and codebase as evidence. Form a requirement inventory covering outcomes, deliverables, constraints, prohibited actions, integration paths, edge cases, and necessary follow-through, and check the plan represents every material item. Do not ask the user; resolve the ambiguity yourself, then update the plan's user intention and understands_user_intent.";
const PRE_TODO_REMINDER_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE: &str =
    "Understand the user's intent better. Try to avoid asking the user.";

const PRE_COMPACT_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE: &str = "Improve the goal's feedback loop. Name a concrete check for each requirement and what result will show it passed. Update the todo, then continue the work.";
const PRE_TODO_REMINDER_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE: &str = "Improve the goal's feedback loop. Name a concrete check for each requirement and what result will show it passed. Update the goal, then continue the work.";
const PRE_BUDGET_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE: &str = "Your feedback loop is not closed. First, improve the goal's objective and name the observation that reports back on each requirement, so progress can be measured across iterations. Generic phrases such as run tests, verify, or review count only for requirements those named checks demonstrably enforce; add separate explicit checks for non-testable requirements. Then call the todo tool again with the revised goal before continuing the task. The goal is to create a strong feedback loop you can iterate against.";

/// Pre-rename ("hill-climbability") version of the closed-feedback-loop
/// continuation. Kept only so persisted transcripts still classify it as a
/// synthetic gate message rather than a user turn.
const LEGACY_TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE: &str = "Your hill-climbability is not high enough. First, improve the goal's objective and feedback loop so progress can be measured across iterations. Then call the todo tool again with the revised goal before continuing the task. The goal is to create a strong feedback loop you can iterate against.";

const PRE_COMPACT_TODO_OWNERSHIP_CONTINUATION_MESSAGE: &str = "[automated follow-up - not a user message] Continue the work below. Keep the todo up to date; do not reply or wait for the user.";

const LEGACY_TODO_OWNERSHIP_CONTINUATION_MESSAGE: &str = "[automated todo completion gate - not a user message] Your end-to-end ownership is not high enough to finish this goal.";

/// Model-facing continuation for private completion-confidence checks.
pub const TODO_COMPLETION_CONTINUATION_MESSAGE: &str = "[auto] Do more validation on the work below. Keep the todo up to date; do not reply or wait for the user.";
const PRE_COMPACT_TODO_COMPLETION_CONTINUATION_MESSAGE: &str = "[automated follow-up - not a user message] Do more validation on the work below. Keep the todo up to date; do not reply or wait for the user.";

/// Model-facing continuation identifying the items whose confidence jumped and
/// asking for one explicit double-check without exposing scores or thresholds.
pub const TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE: &str = "[auto] You had a confidence jump in the items below. Double-check that these are correct. Keep the todo up to date; do not reply or wait for the user.";
const PRE_COMPACT_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE: &str = "[automated follow-up - not a user message] You had a confidence jump in the items below. Double-check that these are correct. Keep the todo up to date; do not reply or wait for the user.";

/// Final synthetic turn after no more automatic checks are needed. Gate
/// continuations tell the model not to reply, so without this handoff a cycle
/// can end on a bare tool call or an internal-looking validation response.
pub const TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE: &str = "[auto] Give the user a concise final response now, including any remaining limitations or blockers. Do not call the todo tool or do more work.";
const PRE_NARROWED_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE: &str = "[auto] Quality checks passed. Give the user a concise final response now. Do not call the todo tool or do more work.";
const PRE_COMPACT_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE: &str = "[automated follow-up - not a user message] Quality checks passed. Give the user a concise final response now. Do not call the todo tool or do more work.";
const PRE_BUDGET_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE: &str = "[automated follow-up - not a user message] All work and quality checks are complete. Give the user the final response now. Default to fewer than 5 lines unless the user's request requires more detail. Summarize the outcome clearly; do not call the todo tool or perform more work.";

pub const TODO_GATE_DIGEST_PREFIX: &str = "[auto] Before you treat this turn as finished, double-check the weak points it surfaced. Keep the todo up to date. Do not reply or wait for the user.";
const PRE_COMPACT_TODO_GATE_DIGEST_PREFIX: &str = "Before you treat this turn as finished, double-check the weak points it surfaced. Keep the todo up to date. Do not reply or wait for the user.";
const LABELED_TODO_GATE_DIGEST_PREFIX: &str = "[automated todo quality review - not a user message] Before you treat this turn as finished, double-check the weak points it surfaced. Do not reply conversationally or wait for the user.";

const LEGACY_TODO_CONFIDENCE_SUMMARY_PREFIX: &str = "All todos are done. Todo confidence summary:";
/// Pre-gate-rewrite texts (before the "[automated todo completion gate" prefix)
/// still exist in persisted transcripts; keep detecting them so reload/resume
/// does not re-render them as user prompts.
const LEGACY_TODO_COMPLETION_CONTINUATION_MESSAGE: &str =
    "Your completion confidence is missing or not high enough.";
const LEGACY_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE: &str =
    "Your completion confidence rose too sharply to count as independently validated.";
/// Wording used immediately before the evidence-backed framing. Persisted
/// sessions can still contain it and must keep treating it as a hidden gate.
const PRE_EVIDENCE_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE: &str = "[automated follow-up - not a user message] Independently recheck the work below. Keep the todo up to date; do not reply or wait for the user.";

/// True when a persisted user-role message is a synthetic auto-poke
/// continuation (an incomplete-todos poke or todo confidence summary) rather
/// than a real user prompt.
///
/// These are persisted as `Role::User` so the model treats them as a normal
/// continuation turn, but they are not something the user typed. The live UI
/// hides them (showing an "Auto-poking..." notice instead), and the session
/// renderer uses this to avoid re-rendering them as user prompts on
/// reload/resume/remote attach.
pub fn is_auto_poke_message(message: &str) -> bool {
    let trimmed = message.trim();
    (trimmed.starts_with("You have ")
        && trimmed.contains(" incomplete todo")
        && trimmed.ends_with("update the todo tool."))
        || trimmed.starts_with(TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_TODO_REMINDER_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_BUDGET_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_ALIGNMENT_CONTINUATION_MESSAGE)
        || trimmed.starts_with(TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_TODO_REMINDER_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_CONCISE_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(TODO_OWNERSHIP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_OWNERSHIP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_OWNERSHIP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(TODO_COMPLETION_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_COMPLETION_CONTINUATION_MESSAGE)
        || trimmed.starts_with(TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_NARROWED_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_BUDGET_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_COMPLETION_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_EVIDENCE_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_CONFIDENCE_SUMMARY_PREFIX)
        || trimmed.starts_with(TODO_GATE_DIGEST_PREFIX)
        || trimmed.starts_with(PRE_COMPACT_TODO_GATE_DIGEST_PREFIX)
        || trimmed.starts_with(LABELED_TODO_GATE_DIGEST_PREFIX)
        || trimmed.starts_with(TODO_LONG_SESSION_REVIEW_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_LONG_SESSION_REVIEW_MESSAGE)
        || trimmed.starts_with(PRE_BUDGET_TODO_LONG_SESSION_REVIEW_MESSAGE)
}

/// Short, user-facing stand-in for a synthetic auto-poke/gate continuation.
///
/// The continuations themselves are written for the model and name specific
/// todos and required fields. Showing that wall of instructions in the
/// transcript (on reload/resume, where the live short notice is gone) buries the
/// conversation, so the UI renders this one-liner instead.
pub fn auto_poke_display_summary(message: &str) -> Option<&'static str> {
    let trimmed = message.trim();
    if !is_auto_poke_message(trimmed) {
        return None;
    }
    if trimmed.starts_with(TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_EVIDENCE_TODO_CONFIDENCE_SPIKE_CONTINUATION_MESSAGE)
    {
        return Some("🔍 Double-checking confidence jumps...");
    }
    if trimmed.starts_with(TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_NARROWED_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_BUDGET_TODO_FINAL_RESPONSE_CONTINUATION_MESSAGE)
    {
        return Some("✅ Preparing the final response...");
    }
    if trimmed.starts_with(TODO_COMPLETION_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_COMPLETION_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_COMPLETION_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_CONFIDENCE_SUMMARY_PREFIX)
    {
        return Some("🔍 Double-checking confidence for you...");
    }
    if trimmed.starts_with(TODO_GATE_DIGEST_PREFIX)
        || trimmed.starts_with(PRE_COMPACT_TODO_GATE_DIGEST_PREFIX)
        || trimmed.starts_with(LABELED_TODO_GATE_DIGEST_PREFIX)
    {
        return Some("🔍 Reviewing the weak points of this turn for you...");
    }
    if trimmed.starts_with(TODO_LONG_SESSION_REVIEW_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_LONG_SESSION_REVIEW_MESSAGE)
        || trimmed.starts_with(PRE_BUDGET_TODO_LONG_SESSION_REVIEW_MESSAGE)
    {
        return Some("🔍 Rechecking the plan and assessments after extended work...");
    }
    if trimmed.starts_with(TODO_OWNERSHIP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_OWNERSHIP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_OWNERSHIP_CONTINUATION_MESSAGE)
    {
        return Some("🔍 Checking the delivery state of the finished work...");
    }
    if trimmed.starts_with(TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_TODO_REMINDER_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_CONCISE_TODO_INTENT_UNDERSTANDING_CONTINUATION_MESSAGE)
    {
        return Some("🔍 Re-checking the request was understood...");
    }
    if trimmed.starts_with(TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_COMPACT_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_TODO_REMINDER_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(PRE_BUDGET_TODO_CLOSED_FEEDBACK_LOOP_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_HILL_CLIMBABILITY_CONTINUATION_MESSAGE)
        || trimmed.starts_with(LEGACY_TODO_ALIGNMENT_CONTINUATION_MESSAGE)
    {
        return Some("🔍 Asking for a stronger way to verify this work...");
    }
    // Incomplete-todos poke: the count is genuinely useful, and it is already
    // short, so it keeps its own text.
    None
}

pub fn load_todos(session_id: &str) -> anyhow::Result<Vec<TodoItem>> {
    let path = todo_path(session_id)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    crate::tui::operant_app::storage::read_json(&path).or_else(|_| Ok(Vec::new()))
}

pub fn save_todos(session_id: &str, todos: &[TodoItem]) -> anyhow::Result<()> {
    let path = todo_path(session_id)?;
    crate::tui::operant_app::storage::write_json_fast(&path, todos)?;
    #[cfg(any())]
    // [port-decision] recent-session-index subsystem (refresh_todo_title,
    // operant-base/src/recent_session_index.rs:150) not ported; the indexed title
    // refresh silently no-ops until then. Re-activate at cutover.
    if let Err(error) = crate::recent_session_index::refresh_todo_title(session_id) {
        crate::logging::warn(&format!(
            "Failed to refresh indexed todo title for {session_id}: {error}"
        ));
    }
    Ok(())
}

fn todo_path(session_id: &str) -> anyhow::Result<std::path::PathBuf> {
    let base = crate::tui::operant_app::storage::operant_dir()?;
    Ok(base.join("todos").join(format!("{}.json", session_id)))
}

/// Derive a concise session-title hint from the todo tool's persisted plan.
///
/// Todo groups are intended to name coherent goals, so the group containing the
/// current (or latest incomplete) item is the strongest signal. Ungrouped plans
/// fall back to the plan's user intention, then item text.
pub fn derive_session_title(todos: &[TodoItem], plan: &TodoPlan) -> Option<String> {
    fn non_empty(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    let current = todos
        .iter()
        .rev()
        .find(|todo| todo.status.eq_ignore_ascii_case("in_progress"))
        .or_else(|| {
            todos
                .iter()
                .rev()
                .find(|todo| !todo.status.eq_ignore_ascii_case("completed"))
        })
        .or_else(|| todos.last());

    if let Some(todo) = current {
        if let Some(group) = non_empty(todo.group.as_deref()) {
            return Some(group);
        }

        if let Some(user_intention) = non_empty(plan.user_intention.as_deref()) {
            return Some(user_intention);
        }

        return non_empty(Some(&todo.content));
    }

    non_empty(plan.user_intention.as_deref())
}

/// Load todo state for a session and derive its best title hint.
pub fn load_session_title(session_id: &str) -> Option<String> {
    let todos = load_todos(session_id).ok()?;
    let plan = load_plan(session_id).unwrap_or_default();
    derive_session_title(&todos, &plan)
}

/// The plan-level intent assessment lives in its own file beside the todo list
/// and per-group goals, so each format stays independently readable.
pub fn load_plan(session_id: &str) -> anyhow::Result<TodoPlan> {
    let path = plan_path(session_id)?;
    if !path.exists() {
        return Ok(TodoPlan::default());
    }
    crate::tui::operant_app::storage::read_json(&path).or_else(|_| Ok(TodoPlan::default()))
}

fn plan_path(session_id: &str) -> anyhow::Result<std::path::PathBuf> {
    let base = crate::tui::operant_app::storage::operant_dir()?;
    Ok(base.join("todos").join(format!("{}-plan.json", session_id)))
}
