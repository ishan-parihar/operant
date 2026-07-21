//! Per-retry state tracking for the agent turn loop.
//!
//! Ported from hermes-agent's `agent/turn_retry_state.py`. Collapses the
//! scattered loop-local retry variables (`retry_count`, `max_retries`,
//! per-error recovery guards) into a single, testable struct.
//!
//! ## Design
//!
//! The agent's `run()` loop makes multiple LLM requests per turn. Each
//! request can fail with various error types (auth, rate-limit, context
//! overflow, etc.). The `TurnRetryState` tracks:
//!
//! 1. **Retry budget**: How many retries remain before giving up.
//! 2. **Per-error recovery guards**: One-shot flags that prevent infinite
//!    retry loops for the same error class (e.g., don't try to compress
//!    context more than once per turn).
//! 3. **Restart signals**: Flags set by error handlers to signal the outer
//!    loop how to proceed (compress, rotate credential, rebuild messages).
//!
//! ## Reset semantics
//!
//! The retry state is created fresh at the start of each `run()` call
//! (one state per user query). It is NOT persisted across turns.
//!
//! ## YAGNI boundaries
//!
//! Unlike hermes-agent's TurnRetryState (which tracks 15+ provider-specific
//! guards), this implementation starts with the core set:
//! - retry_count / max_retries
//! - compress_attempted / rotate_attempted / fallback_attempted
//! - restart_with_compressed / restart_with_rotated_credential
//!
//! Additional provider-specific guards can be added later as gaps are identified.

use std::fmt;

/// Maximum number of retries before the agent gives up on a turn.
/// Matches hermes-agent's `api_max_retries` default (3).
pub const DEFAULT_MAX_RETRIES: usize = 3;

/// Per-retry state tracking for a single agent turn.
///
/// Created at the start of `run()` and consumed when the turn completes.
/// All fields are mutated in-place (no interior mutability needed since
/// the state is a local variable in `run()`).
#[derive(Debug, Clone)]
pub struct TurnRetryState {
    // ── Retry budget ──────────────────────────────────────────────
    /// Current retry attempt (0-based). Incremented after each failed
    /// LLM call that triggers a retry.
    pub retry_count: usize,
    /// Maximum retries before giving up. Defaults to `DEFAULT_MAX_RETRIES`.
    pub max_retries: usize,

    // ── Per-error recovery guards (one-shot flags) ────────────────
    /// Whether context compression has been attempted this turn.
    /// Prevents infinite compress→fail→compress loops.
    pub compress_attempted: bool,
    /// Whether credential rotation has been attempted this turn.
    /// Prevents burning through the entire credential pool.
    pub rotate_attempted: bool,
    /// Whether model fallback has been attempted this turn.
    /// Prevents cascading fallback across all models.
    pub fallback_attempted: bool,

    // ── Restart signals (set by error handlers, read by outer loop) ──
    /// Signal: restart the LLM call with compressed messages.
    pub restart_with_compressed: bool,
    /// Signal: restart the LLM call with a rotated credential.
    pub restart_with_rotated_credential: bool,
    /// Signal: restart the LLM call with rebuilt messages (e.g., after
    /// content filter stall or message corruption).
    pub restart_with_rebuilt_messages: bool,
}

impl TurnRetryState {
    /// Create a fresh retry state for a new turn.
    pub fn new(max_retries: Option<usize>) -> Self {
        Self {
            retry_count: 0,
            max_retries: max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
            compress_attempted: false,
            rotate_attempted: false,
            fallback_attempted: false,
            restart_with_compressed: false,
            restart_with_rotated_credential: false,
            restart_with_rebuilt_messages: false,
        }
    }

    /// Returns true if the retry budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.retry_count >= self.max_retries
    }

    /// Returns true if any restart signal is set.
    pub fn has_restart_signal(&self) -> bool {
        self.restart_with_compressed
            || self.restart_with_rotated_credential
            || self.restart_with_rebuilt_messages
    }

    /// Consume one retry from the budget. Returns false if exhausted.
    pub fn consume_retry(&mut self) -> bool {
        if self.is_exhausted() {
            false
        } else {
            self.retry_count += 1;
            true
        }
    }

    /// Reset all restart signals (called after the outer loop processes them).
    pub fn clear_restart_signals(&mut self) {
        self.restart_with_compressed = false;
        self.restart_with_rotated_credential = false;
        self.restart_with_rebuilt_messages = false;
    }

    /// Reset the retry state for a new LLM call within the same turn.
    /// Called when a retry succeeds or when the model returns a valid response.
    pub fn reset_on_success(&mut self) {
        self.retry_count = 0;
        self.clear_restart_signals();
    }
}

impl fmt::Display for TurnRetryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RetryState(retry={}/{}, compress={}, rotate={}, fallback={})",
            self.retry_count,
            self.max_retries,
            self.compress_attempted,
            self.rotate_attempted,
            self.fallback_attempted,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state_defaults() {
        let state = TurnRetryState::new(None);
        assert_eq!(state.retry_count, 0);
        assert_eq!(state.max_retries, DEFAULT_MAX_RETRIES);
        assert!(!state.is_exhausted());
        assert!(!state.has_restart_signal());
    }

    #[test]
    fn test_new_state_custom_max() {
        let state = TurnRetryState::new(Some(5));
        assert_eq!(state.max_retries, 5);
    }

    #[test]
    fn test_consume_retry() {
        let mut state = TurnRetryState::new(Some(2));
        assert!(state.consume_retry());
        assert_eq!(state.retry_count, 1);
        assert!(state.consume_retry());
        assert_eq!(state.retry_count, 2);
        assert!(!state.consume_retry()); // exhausted
        assert!(state.is_exhausted());
    }

    #[test]
    fn test_exhausted_at_zero() {
        let mut state = TurnRetryState::new(Some(0));
        assert!(state.is_exhausted());
        assert!(!state.consume_retry());
    }

    #[test]
    fn test_restart_signals() {
        let mut state = TurnRetryState::new(None);
        assert!(!state.has_restart_signal());

        state.restart_with_compressed = true;
        assert!(state.has_restart_signal());

        state.clear_restart_signals();
        assert!(!state.has_restart_signal());
    }

    #[test]
    fn test_reset_on_success() {
        let mut state = TurnRetryState::new(None);
        state.retry_count = 3;
        state.compress_attempted = true;
        state.rotate_attempted = true;
        state.restart_with_compressed = true;

        state.reset_on_success();
        assert_eq!(state.retry_count, 0);
        // Recovery guards are NOT reset (they're one-shot per turn)
        assert!(state.compress_attempted);
        assert!(state.rotate_attempted);
        // Restart signals ARE reset
        assert!(!state.restart_with_compressed);
    }

    #[test]
    fn test_display() {
        let state = TurnRetryState::new(Some(3));
        let display = format!("{state}");
        assert!(display.contains("retry=0/3"));
        assert!(display.contains("compress=false"));
    }

    #[test]
    fn test_clone() {
        let mut state = TurnRetryState::new(None);
        state.retry_count = 2;
        state.compress_attempted = true;
        let cloned = state.clone();
        assert_eq!(cloned.retry_count, 2);
        assert!(cloned.compress_attempted);
    }
}
