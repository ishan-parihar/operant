//! Post-loop turn finalization for `OperantAgent::run()`.
//!
//! Extracted from the agent's main loop as part of the self-evolution pipeline.
//! After each turn completes (successfully or via budget exhaustion), the
//! finalizer checks whether a skill review should be triggered and spawns
//! a background review daemon if so.
//!
//! Ported from `hermes-agent/agent/turn_finalizer.py`.
//!
//! ## Turn Diagnostics
//!
//! Each turn produces a [`TurnDiagnostics`] record that captures why the
//! turn ended, how many iterations were used, what tools were called, and
//! whether the response was successful. This mirrors hermes-agent's
//! turn-exit diagnostic log pattern.
//!
//! ## Preflight Compression Constants
//!
//! Thresholds and decay parameters for proactive context compression,
//! extracted from the agent loop for clarity.

use std::fmt;

use crate::agent::background_review::SelfEvolutionState;
use crate::client::{Message, Role};

// ---------------------------------------------------------------------------
// Preflight Context Compression Constants
// ---------------------------------------------------------------------------

/// Percentage of context window that triggers proactive compression.
/// When estimated tokens exceed this fraction of the budget, aggressive
/// decay fires before the LLM call to prevent context_length_exceeded.
pub const PREFLIGHT_THRESHOLD_PERCENT: u64 = 80;

/// Half-life (in tokens) for aggressive preflight decay. Shorter than
/// the standard 200 to compress older messages more aggressively.
pub const PREFLIGHT_DECAY_H50: usize = 100;

/// Decay constant for preflight compression. Lower = faster decay.
/// Standard is 30.0; preflight uses 20.0 for more aggressive compression.
pub const PREFLIGHT_DECAY_CONSTANT: f64 = 20.0;

// ---------------------------------------------------------------------------
// Turn Diagnostics
// ---------------------------------------------------------------------------

/// Why the turn ended. Matches hermes-agent's `_turn_exit_reason` strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnExitReason {
    /// Model produced a text response (normal completion).
    TextResponse,
    /// Budget exhausted — grace call was made.
    BudgetExhausted,
    /// User interrupted (Ctrl-C or /stop).
    Interrupted,
    /// An error occurred during the turn.
    Error,
}

impl fmt::Display for TurnExitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TextResponse => write!(f, "text_response"),
            Self::BudgetExhausted => write!(f, "budget_exhausted"),
            Self::Interrupted => write!(f, "interrupted"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Per-turn diagnostics record.
///
/// Captures why the turn ended, how many iterations were used, what
/// tools were called, and whether the response was successful. Logged
/// at INFO for every turn completion. Mirrors hermes-agent's turn-exit
/// diagnostic log pattern.
#[derive(Debug, Clone)]
pub struct TurnDiagnostics {
    /// Why the turn ended.
    pub exit_reason: TurnExitReason,
    /// Model name used for this turn.
    pub model: String,
    /// Number of LLM iterations consumed.
    pub api_calls: usize,
    /// Maximum allowed iterations.
    pub max_iterations: usize,
    /// Iteration budget used.
    pub budget_used: usize,
    /// Iteration budget maximum.
    pub budget_max: usize,
    /// Number of tool-calling iterations.
    pub tool_turns: usize,
    /// Length of the final response in characters.
    pub response_len: usize,
    /// Session ID.
    pub session_id: String,
}

impl TurnDiagnostics {
    /// Format the diagnostics as a human-readable log message.
    ///
    /// Matches hermes-agent's `_diag_msg` format:
    /// `"Turn ended: reason=%s model=%s api_calls=%d/%d budget=%d/%d tool_turns=%d response_len=%d session=%s"`
    pub fn log_message(&self) -> String {
        format!(
            "Turn ended: reason={} model={} api_calls={}/{} budget={}/{} tool_turns={} response_len={} session={}",
            self.exit_reason,
            self.model,
            self.api_calls,
            self.max_iterations,
            self.budget_used,
            self.budget_max,
            self.tool_turns,
            self.response_len,
            self.session_id,
        )
    }

    /// Build a user-facing explanation for why the turn ended abnormally.
    ///
    /// Ported from hermes-agent's `_format_turn_completion_explanation`.
    /// Returns `None` for healthy `TextResponse` exits (no explanation needed).
    #[expect(
        dead_code,
        reason = "Prepared for UI integration; will surface turn-exit reasons"
    )]
    pub fn explanation(&self) -> Option<String> {
        match self.exit_reason {
            TurnExitReason::TextResponse => None,
            TurnExitReason::BudgetExhausted => Some(format!(
                "⚠️ Turn ended early — iteration budget exhausted ({}/{} iterations used). \
                 The agent ran out of tool-calling turns before completing the task. \
                 Try breaking the task into smaller steps or increasing `max_iterations` in config.",
                self.api_calls, self.max_iterations,
            )),
            TurnExitReason::Interrupted => Some(
                "⚡ Turn was interrupted by the user. Partial work may have been done — \
                 check what was accomplished before the interruption."
                    .to_string(),
            ),
            TurnExitReason::Error => Some(
                "❌ Turn ended due to an error. The agent encountered an unexpected failure \
                 during processing. Check the logs for details."
                    .to_string(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// File Mutation Verifier
// ---------------------------------------------------------------------------

/// Scan tool results for failed file mutations and return advisory footers.
///
/// Ported from hermes-agent's `_format_file_mutation_failure_footer`.
/// When `write_file` or `patch` calls fail during a turn, this function
/// detects them and produces a human-readable footer to append to the
/// assistant response, preventing over-claiming.
pub fn file_mutation_verifier_footer(messages: &[Message]) -> Option<String> {
    let mut failed_writes: Vec<String> = Vec::new();

    // Walk assistant messages to find tool calls that are file mutations,
    // then match against tool results that indicate failure.
    let mut file_mutation_call_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for msg in messages {
        // Collect tool call IDs from assistant messages that invoked file mutations
        if msg.role == Role::Assistant
            && let Some(ref tool_calls) = msg.tool_calls
        {
            for tc in tool_calls {
                let name = tc.function.name.to_lowercase();
                if name == "write_file" || name == "patch" || name == "create_file" {
                    file_mutation_call_ids.insert(tc.id.clone());
                }
            }
        }

        // Check tool results for failures on those specific call IDs
        if msg.role == Role::Tool {
            if let Some(ref tool_call_id) = msg.tool_call_id {
                if !file_mutation_call_ids.contains(tool_call_id) {
                    continue;
                }
            } else {
                continue;
            }

            let content = &msg.content;
            let content_lower = content.to_lowercase();

            let is_failure = content_lower.contains("error")
                || content_lower.contains("failed")
                || content_lower.contains("could not find")
                || content_lower.contains("no such file")
                || content_lower.contains("permission denied")
                || content_lower.contains("old_string not found");

            if is_failure {
                // Extract a short preview of the failure
                let preview: String = content.chars().take(120).collect();
                failed_writes.push(preview);
            }
        }
    }

    if failed_writes.is_empty() {
        return None;
    }

    let count = failed_writes.len();
    let footer = format!(
        "\n\n---\n⚠️ **File Mutation Advisory**: {} file operation(s) may have failed during this turn:\n{}\nPlease verify that the intended file changes were applied correctly.",
        count,
        failed_writes
            .iter()
            .enumerate()
            .map(|(i, f)| format!("  {}. {}", i + 1, f))
            .collect::<Vec<_>>()
            .join("\n")
    );

    Some(footer)
}

// ---------------------------------------------------------------------------
// Evolution Trigger Check
// ---------------------------------------------------------------------------

/// Result of checking evolution triggers after a completed turn.
///
/// Carries the flags indicating which reviews should fire, plus
/// metadata for logging and persistence. Matches hermes-agent's
/// turn_finalizer.py where _iters_since_skill and _turns_since_memory
/// are checked after the tool-calling loop completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvolutionTriggerResult {
    /// Whether a skill review should be triggered.
    pub should_review_skills: bool,
    /// Whether a memory review should be triggered.
    pub should_review_memory: bool,
    /// Current skill iteration counter (after bump/reset, before persist).
    pub iters_since_skill: usize,
    /// Current memory turn counter (after bump/reset, before persist).
    pub turns_since_memory: usize,
}

/// Check and advance evolution counters after a completed turn.
///
/// Bumps or resets the skill counter based on whether `skill_manage`
/// was called this turn, increments the memory counter, checks both
/// nudge thresholds, and resets any that fired. Returns the result
/// indicating which reviews should be triggered.
///
/// This function is pure (no side effects beyond mutating `evo`),
/// making it easy to test. The caller is responsible for persisting
/// the counters and spawning the review daemon.
///
/// Ported from hermes-agent's `turn_finalizer._check_evolution_triggers`.
/// Advance the skill-evolution counter and check its nudge threshold.
///
/// Called once per **iteration** of the tool loop (NOT per turn) — this
/// mirrors hermes-agent, which bumps `_iters_since_skill` once per API
/// iteration. The memory counter is untouched here; it is advanced by
/// [`advance_memory_trigger`] at the turn boundary instead.
///
/// This function is pure (no side effects beyond mutating `evo`),
/// making it easy to test. The caller is responsible for persisting
/// the counters and spawning the review daemon.
pub fn advance_skill_trigger(
    evo: &mut SelfEvolutionState,
    skill_manage_called: bool,
) -> EvolutionTriggerResult {
    // Skill counter: reset if skill_manage was called, else bump.
    if skill_manage_called {
        evo.reset_skill_counter();
    } else {
        evo.bump_skill_counter();
    }

    let should_review_skills = evo.should_review_skills();

    // Capture counter value BEFORE reset so the caller can log the
    // threshold value that triggered the review (matches hermes-agent's
    // log pattern where the counter is read before reset).
    let iters_at_fire = evo.iters_since_skill;

    // Reset the counter that fired so the next window starts fresh.
    if should_review_skills {
        evo.reset_skill_counter();
    }

    EvolutionTriggerResult {
        should_review_skills,
        should_review_memory: false,
        iters_since_skill: iters_at_fire,
        turns_since_memory: evo.turns_since_memory_review,
    }
}

/// Advance the memory-evolution counter and check its review threshold.
///
/// Called once per **user turn** at the turn boundary (NOT per iteration) —
/// this mirrors hermes-agent's `turn_context.py` which bumps
/// `_turns_since_memory` once per turn. The skill counter is untouched
/// here; it is advanced by [`advance_skill_trigger`] per iteration.
///
/// This function is pure (no side effects beyond mutating `evo`),
/// making it easy to test. The caller is responsible for persisting
/// the counters and spawning the review daemon.
pub fn advance_memory_trigger(evo: &mut SelfEvolutionState) -> EvolutionTriggerResult {
    // Memory counter increments once per completed turn regardless.
    evo.bump_memory_counter();

    let should_review_memory = evo.should_review_memory();

    // Capture counter value BEFORE reset so the caller can log the
    // threshold value that triggered the review.
    let turns_at_fire = evo.turns_since_memory_review;

    // Reset the counter that fired so the next window starts fresh.
    if should_review_memory {
        evo.reset_memory_counter();
    }

    EvolutionTriggerResult {
        should_review_skills: false,
        should_review_memory,
        iters_since_skill: evo.iters_since_skill,
        turns_since_memory: turns_at_fire,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::background_review::BackgroundReviewConfig;

    fn make_evo(skill_interval: usize, memory_interval: usize) -> SelfEvolutionState {
        SelfEvolutionState::new(&BackgroundReviewConfig {
            skill_nudge_interval: skill_interval,
            memory_review_interval: memory_interval,
        })
    }

    #[test]
    fn test_evolution_trigger_no_fire() {
        let mut evo = make_evo(10, 5);
        let result = advance_memory_trigger(&mut evo);
        assert!(!result.should_review_skills);
        assert!(!result.should_review_memory);
        assert_eq!(result.turns_since_memory, 1);
    }

    #[test]
    fn test_evolution_trigger_skill_fires() {
        let mut evo = make_evo(3, 10);
        // Bump 2 times → no fire yet
        for _ in 0..2 {
            let result = advance_skill_trigger(&mut evo, false);
            assert!(!result.should_review_skills);
        }
        // 3rd bump → fires at threshold
        let result = advance_skill_trigger(&mut evo, false);
        assert!(result.should_review_skills);
        assert!(!result.should_review_memory);
        // iters_since_skill should be the threshold value (3), not 0
        assert_eq!(result.iters_since_skill, 3);
    }

    #[test]
    fn test_evolution_trigger_memory_fires() {
        let mut evo = make_evo(100, 3);
        // Bump 2 times → no fire yet
        for _ in 0..2 {
            let result = advance_memory_trigger(&mut evo);
            assert!(!result.should_review_memory);
        }
        // 3rd bump → fires at threshold
        let result = advance_memory_trigger(&mut evo);
        assert!(result.should_review_memory);
        assert!(!result.should_review_skills);
        assert_eq!(result.turns_since_memory, 3);
    }

    #[test]
    fn test_evolution_cadence_isolation() {
        // The two counters must advance independently: skill bumps do NOT
        // move the memory counter and vice-versa. This is the load-bearing
        // hermes parity — memory cadence is per-turn, skill is per-iteration.
        let mut evo = make_evo(10, 10);
        for _ in 0..7 {
            advance_skill_trigger(&mut evo, false);
        }
        assert_eq!(evo.iters_since_skill, 7);
        // Memory counter untouched by skill bumps.
        assert_eq!(evo.turns_since_memory_review, 0);

        let result = advance_memory_trigger(&mut evo);
        assert!(!result.should_review_memory);
        // Memory counter advanced by exactly one turn.
        assert_eq!(result.turns_since_memory, 1);
        // Skill counter untouched by the memory bump.
        assert_eq!(evo.iters_since_skill, 7);
    }

    #[test]
    fn test_evolution_trigger_skill_manage_resets() {
        let mut evo = make_evo(5, 10);
        // Bump 4 times
        for _ in 0..4 {
            advance_skill_trigger(&mut evo, false);
        }
        assert_eq!(evo.iters_since_skill, 4);
        // skill_manage resets the counter
        advance_skill_trigger(&mut evo, true);
        assert_eq!(evo.iters_since_skill, 0);
        assert!(!evo.should_review_skills());
    }

    #[test]
    fn test_evolution_disabled_when_zero() {
        let mut evo = make_evo(0, 0);
        let skill = advance_skill_trigger(&mut evo, false);
        assert!(!skill.should_review_skills);
        let memory = advance_memory_trigger(&mut evo);
        assert!(!memory.should_review_memory);
    }

    #[test]
    fn test_evolution_both_fire() {
        let mut evo = make_evo(1, 1);
        let skill = advance_skill_trigger(&mut evo, false);
        assert!(skill.should_review_skills);
        let memory = advance_memory_trigger(&mut evo);
        assert!(memory.should_review_memory);
    }
}
