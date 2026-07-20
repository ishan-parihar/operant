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

use std::fmt;

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
    /// Context overflow triggered compression.
    ContextOverflow,
    /// No tool calls — turn completed with empty response.
    EmptyResponse,
}

impl fmt::Display for TurnExitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TextResponse => write!(f, "text_response"),
            Self::BudgetExhausted => write!(f, "budget_exhausted"),
            Self::Interrupted => write!(f, "interrupted"),
            Self::Error => write!(f, "error"),
            Self::ContextOverflow => write!(f, "context_overflow"),
            Self::EmptyResponse => write!(f, "empty_response"),
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
    /// Whether the turn completed successfully.
    pub completed: bool,
    /// Whether the turn was interrupted.
    pub interrupted: bool,
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

/// Returns true if the turn ended with an error and was not interrupted.
/// Matches hermes-agent's pattern: log at WARNING when the turn ended
/// with an error but the user did not interrupt — the agent may be stuck.
pub fn is_error_not_interrupted(&self) -> bool {
    !self.interrupted && self.exit_reason == TurnExitReason::Error
}
}

/// Summary of actions taken by the background review.
///
/// Used by the background review daemon (once wired up) to surface a compact
/// summary of skill/memory changes to the user.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct BackgroundReviewSummary {
    /// Human-readable action descriptions.
    pub actions: Vec<String>,
    /// Whether any skills were created/updated.
    pub skills_changed: bool,
    /// Whether any memory entries were added/updated.
    pub memory_changed: bool,
}

/// Build a compact action summary from background review messages.
///
/// Scans the review agent's messages for successful tool actions and
/// surfaces a compact summary to the user. Matches hermes-agent's
/// `summarize_background_review_actions`.
#[allow(dead_code)]
pub fn summarize_review_actions(
    review_messages: &[String],
    prior_messages: &[String],
) -> BackgroundReviewSummary {
    let mut summary = BackgroundReviewSummary::default();

    // Collect existing tool call IDs from prior messages to avoid re-surfacing
    let prior_tool_ids: std::collections::HashSet<String> = prior_messages
        .iter()
        .filter_map(|m| {
            let data: serde_json::Value = serde_json::from_str(m).ok()?;
            if data.get("role").and_then(|v| v.as_str()) == Some("tool") {
                data.get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            }
        })
        .collect();

    for msg_str in review_messages {
        let data: serde_json::Value = match serde_json::from_str(msg_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if data.get("role").and_then(|v| v.as_str()) != Some("tool") {
            continue;
        }

        if let Some(id) = data.get("tool_call_id").and_then(|v| v.as_str()) {
            if prior_tool_ids.contains(id) {
                continue;
            }
        }

        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let result: serde_json::Value = serde_json::from_str(content).unwrap_or_default();
        if !result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }

        let message = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if message.is_empty() {
            continue;
        }

        let is_skill = message.to_lowercase().contains("skill");
        if is_skill {
            summary.skills_changed = true;
        } else {
            summary.memory_changed = true;
        }

        summary.actions.push(message);
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarize_review_actions_empty() {
        let summary = summarize_review_actions(&[], &[]);
        assert!(summary.actions.is_empty());
        assert!(!summary.skills_changed);
        assert!(!summary.memory_changed);
    }

    #[test]
    fn test_summarize_review_actions_skill() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Updated skill web-search\"}"}"#.to_string()];
        let summary = summarize_review_actions(&review, &[]);
        assert_eq!(summary.actions.len(), 1);
        assert!(summary.skills_changed);
        assert!(!summary.memory_changed);
    }

    #[test]
    fn test_summarize_review_actions_skips_prior() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Saved memory\"}"}"#.to_string()];
        let prior = vec![r#"{"role":"tool","tool_call_id":"tc1"}"#.to_string()];
        let summary = summarize_review_actions(&review, &prior);
        assert!(summary.actions.is_empty());
    }
}
