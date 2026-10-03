// Vendored from jcode (crates/jcode-task-types/src/lib.rs + crates/jcode-base/src/todo.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// two upstream modules unified into one file (upstream reaches task-types via
// jcode-base/src/todo.rs:29's `pub use jcode_task_types::{...}`). See
// jcode_app/mod.rs for scope.
//!
//! Partial port — the renderer-referenced subset of the todo/feedback type
//! family. Included from jcode-task-types/src/lib.rs: the `semantic_state!`
//! macro (:197-:296) and all ten of its invocations — IntentUnderstanding
//! (:299), IterationMaturity (:309, plus its `permits_completion` impl at
//! :325), FeedbackLoopState (:340), FeedbackLoopRelevance (:352),
//! FeedbackLoopCoverage (:364), FeedbackLoopTraceability (:374),
//! ConfidenceState (:384), Difficulty (:395), Autonomy (:410), DeliveryState
//! (:421) — and the structs TodoItem (:433), TodoPlan (:472), TodoPlanField
//! (:501), TodoPlanChange (:509), TodoGoal (:526), TodoGoalField (:619),
//! TodoGoalChange (:635). Included from jcode-base/src/todo.rs: the five
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
//! CatchupBrief, the PRE_*/LEGACY_* sibling consts, and jcode-base/todo.rs's
//! server-side gate logic. jcode-tui's `crate::todo::*` resolves here.

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

