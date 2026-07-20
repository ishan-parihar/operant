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
}


