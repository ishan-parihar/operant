//! Per-agent iteration budget — thread-safe consume/refund counter.
//!
//! Ported from `hermes-agent/agent/iteration_budget.py`. Each `OperantAgent`
//! instance holds an `IterationBudget`; the parent's cap comes from
//! `max_iterations` (default 90).
//!
//! `refund()` gives back iterations consumed by compression or other
//! non-productive calls, matching hermes-agent's budget management.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Thread-safe iteration counter for an agent.
///
/// Each agent (parent or subagent) gets its own `IterationBudget`.
/// The parent's budget is capped at `max_iterations`.
///
/// `refund` gives back iterations consumed by execute_code or compression
/// turns so they don't eat into the budget.
pub struct IterationBudget {
    max_total: usize,
    used: AtomicUsize,
}

impl IterationBudget {
    /// Create a new budget with the given maximum.
    pub fn new(max_total: usize) -> Self {
        Self {
            max_total,
            used: AtomicUsize::new(0),
        }
    }

    /// Try to consume one iteration. Returns `true` if allowed.
    ///
    /// Uses `compare_exchange` to avoid the brief over-count window
    /// of a `fetch_add` + rollback pattern.
    pub fn consume(&self) -> bool {
        loop {
            let current = self.used.load(Ordering::SeqCst);
            if current >= self.max_total {
                return false;
            }
            if self
                .used
                .compare_exchange(
                    current,
                    current + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return true;
            }
            // CAS failed — another thread modified `used`, retry.
        }
    }

    /// Give back one iteration (e.g. for execute_code turns or compression).
    ///
    /// Uses `compare_exchange` to avoid going negative when two threads
    /// refund concurrently at `used == 1`.
    pub fn refund(&self) {
        loop {
            let current = self.used.load(Ordering::SeqCst);
            if current == 0 {
                return; // nothing to refund
            }
            if self
                .used
                .compare_exchange(
                    current,
                    current - 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return;
            }
            // CAS failed — retry.
        }
    }

    /// Number of iterations consumed so far.
    pub fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }

    /// Number of iterations remaining.
    pub fn remaining(&self) -> usize {
        let used = self.used.load(Ordering::SeqCst);
        self.max_total.saturating_sub(used)
    }

    /// Maximum total iterations.
    pub fn max_total(&self) -> usize {
        self.max_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_basic() {
        let budget = IterationBudget::new(5);
        assert_eq!(budget.remaining(), 5);
        assert_eq!(budget.used(), 0);

        assert!(budget.consume());
        assert_eq!(budget.used(), 1);
        assert_eq!(budget.remaining(), 4);
    }

    #[test]
    fn test_budget_exhaustion() {
        let budget = IterationBudget::new(2);
        assert!(budget.consume());
        assert!(budget.consume());
        assert!(!budget.consume()); // should fail
        assert_eq!(budget.used(), 2);
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_refund() {
        let budget = IterationBudget::new(3);
        budget.consume();
        budget.consume();
        budget.consume();
        assert!(!budget.consume());

        budget.refund();
        assert_eq!(budget.remaining(), 1);
        assert!(budget.consume());
        assert!(!budget.consume());
    }

    #[test]
    fn test_budget_refund_underflow() {
        let budget = IterationBudget::new(5);
        // Refund when nothing consumed should be a no-op
        budget.refund();
        assert_eq!(budget.used(), 0);
    }
}
