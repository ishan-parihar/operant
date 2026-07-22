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

    /// Build a user-facing explanation for why the turn ended abnormally.
    ///
    /// Ported from hermes-agent's `_format_turn_completion_explanation`.
    /// Returns `None` for healthy `TextResponse` exits (no explanation needed).
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
// Message Sequence Repair
// ---------------------------------------------------------------------------

/// Repair role-alternation violations in a message list.
///
/// Ported from hermes-agent's `repair_message_sequence_with_cursor`.
/// Fixes:
/// - `tool → user` violations (tool result followed by user message)
/// - `user → user` violations (consecutive user messages)
/// - `assistant → assistant` violations (consecutive assistant messages)
///
/// Returns the number of repairs made.
pub fn repair_message_sequence(messages: &mut Vec<crate::client::Message>) -> usize {
    if messages.len() <= 2 {
        return 0;
    }

    use crate::client::Role;

    let mut repairs = 0;
    let mut i = 1;

    while i < messages.len() {
        let prev_role = &messages[i - 1].role;
        let curr_role = &messages[i].role;

        let violation = match (prev_role, curr_role) {
            // Tool followed by user — insert a synthetic assistant message
            (Role::Tool, Role::User) => {
                let synthetic = crate::client::Message {
                    role: Role::Assistant,
                    content: "[Continuing after tool result]".to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                    ..Default::default()
                };
                messages.insert(i, synthetic);
                repairs += 1;
                true
            }
            // User followed by user — merge into previous
            (Role::User, Role::User) => {
                let merged = format!("{}\n\n{}", messages[i - 1].content, messages[i].content);
                messages[i - 1].content = merged;
                messages.remove(i);
                repairs += 1;
                true
            }
            // Assistant followed by assistant — merge into previous
            (Role::Assistant, Role::Assistant) => {
                let merged = format!("{}\n\n{}", messages[i - 1].content, messages[i].content);
                messages[i - 1].content = merged;
                messages.remove(i);
                repairs += 1;
                true
            }
            _ => false,
        };

        if !violation {
            i += 1;
        }
    }

    repairs
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
pub fn file_mutation_verifier_footer(messages: &[crate::client::Message]) -> Option<String> {
    use crate::client::Role;

    let mut failed_writes: Vec<String> = Vec::new();

    // Walk messages looking for tool results with failed file mutations
    for msg in messages {
        if msg.role != Role::Tool {
            continue;
        }

        let content = &msg.content;
        let content_lower = content.to_lowercase();

        // Detect failed write_file / patch operations
        let is_file_mutation = content_lower.contains("write_file")
            || content_lower.contains("patch")
            || content_lower.contains("file_path")
            || content_lower.contains("write to file");

        let is_failure = content_lower.contains("error")
            || content_lower.contains("failed")
            || content_lower.contains("could not find")
            || content_lower.contains("no such file")
            || content_lower.contains("permission denied");

        if is_file_mutation && is_failure {
            // Extract a short preview of the failure
            let preview: String = content.chars().take(120).collect();
            failed_writes.push(preview);
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
