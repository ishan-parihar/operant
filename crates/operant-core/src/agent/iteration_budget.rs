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
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
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
                .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
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

    /// Reset the counter to zero so the next `run()` call gets a fresh
    /// per-turn budget.  Called at the start of each agent turn (hermes
    /// parity: `api_call_count = 0` in conversation_loop.py).
    pub fn reset(&self) {
        self.used.store(0, Ordering::SeqCst);
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

    // ── Concurrent-access tests ──────────────────────────────────────
    // These verify that the AtomicUsize-based compare_exchange loops
    // produce consistent results under concurrent access.

    #[test]
    fn test_budget_concurrent_consume() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let budget = Arc::new(IterationBudget::new(100));
        let barrier = Arc::new(Barrier::new(10));
        let mut handles = vec![];

        // Spawn 10 threads, each consuming 20 iterations.
        // Total = 200 attempted, but max is 100, so exactly 100 should succeed.
        // Barrier forces all threads to start simultaneously for true
        // concurrent access to the AtomicUsize CAS loops.
        for _ in 0..10 {
            let b = Arc::clone(&budget);
            let bar = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                bar.wait(); // synchronize start
                let mut succeeded = 0;
                for _ in 0..20 {
                    if b.consume() {
                        succeeded += 1;
                    }
                }
                succeeded
            }));
        }

        let total_consumed: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total_consumed, 100);
        assert_eq!(budget.used(), 100);
        assert_eq!(budget.remaining(), 0);
        assert!(!budget.consume()); // exhausted
    }

    #[test]
    fn test_budget_concurrent_consume_refund() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let budget = Arc::new(IterationBudget::new(50));
        let barrier = Arc::new(Barrier::new(10));
        let barrier2 = Arc::new(Barrier::new(10));
        let barrier3 = Arc::new(Barrier::new(10));
        let mut handles = vec![];

        // 10 threads on ONE atomic, in three phases separated by two barriers.
        //
        // The barriers pin the ORDER OF PHASES, not the order of operations
        // inside a phase: all 35 phase-1 `consume()` calls and all 50 `refund()`
        // calls still race each other on the same `AtomicUsize` through the
        // compare_exchange loops. That contention is the thing under test and
        // it is fully preserved.
        //
        // The previous version released all 10 threads from ONE barrier and then
        // asserted `total_consumed > 50`. That assert was a statement about
        // thread scheduling, not about the budget: if the 5 consumers finished
        // their 15 attempts before any refunder was scheduled, they consumed
        // exactly 50, every refund landed at `used == 0` and was a no-op, and the
        // assert failed. That is a legal interleaving. It reproduced on the first
        // run after a rebuild (a fresh binary on a multi-core box gets genuinely
        // parallel scheduling, which is what makes the interleaving random) and
        // passed on every subsequent run of the same binary.
        //
        // The arithmetic below is exact, so the outcome is the same every run:
        //   phase 1  5 x 7 = 35 consumes, all succeed (35 < 50 cap)   used = 35
        //   refunds  5 x 10 = 50 attempted; 35 land, 15 no-op at zero  used = 0
        //   phase 2  5 x 8 = 40 consumes, all succeed (40 < 50 cap)
        //   total = 35 + 40 = 75
        for i in 0..10 {
            let b = Arc::clone(&budget);
            let bar = Arc::clone(&barrier);
            let bar2 = Arc::clone(&barrier2);
            let bar3 = Arc::clone(&barrier3);
            if i % 2 == 0 {
                handles.push(thread::spawn(move || -> usize {
                    bar.wait();
                    let mut succeeded = 0;
                    for _ in 0..7 {
                        if b.consume() {
                            succeeded += 1;
                        }
                    }
                    // barrier2: everyone has left phase 1. refunders go on to
                    // refund, consumers stop here.
                    bar2.wait();
                    // barrier3: refunders only arrive once all 50 of their refunds
                    // are done, so phase 2 starts from a KNOWN used == 0. Without
                    // this third barrier the refunds race the phase-2 consumes and
                    // knock some of the 40 straight back out again, which is the
                    // `used() == 32` failure this rewrite was written to remove.
                    bar3.wait();
                    for _ in 0..8 {
                        if b.consume() {
                            succeeded += 1;
                        }
                    }
                    succeeded
                }));
            } else {
                handles.push(thread::spawn(move || -> usize {
                    bar.wait();
                    bar2.wait(); // consumers have drained phase 1
                    for _ in 0..10 {
                        b.refund();
                    }
                    // Only now may phase 2 begin.
                    bar3.wait();
                    0 // refunders return 0
                }));
            }
        }

        let total_consumed: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();

        // Schedule-independent: both consumers and refunders always ran, all 75
        // consumer attempts are below the 50 cap per phase, and `used` can never
        // exceed `max_total` nor underflow (refund is a no-op at zero).
        assert_eq!(total_consumed, 75, "every consumer attempt should succeed");
        assert_eq!(
            budget.used(),
            40,
            "35 consumed, 35 refunded, 40 re-consumed"
        );
        assert!(budget.used() <= 50, "used must never exceed max_total");
    }

    #[test]
    fn test_budget_concurrent_exhaustion_boundary() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let budget = Arc::new(IterationBudget::new(10));
        let barrier = Arc::new(Barrier::new(20));
        let mut handles = vec![];

        // Spawn 20 threads, each trying to consume exactly 1 iteration.
        // Exactly 10 should succeed. Barrier forces simultaneous start.
        for _ in 0..20 {
            let b = Arc::clone(&budget);
            let bar = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                bar.wait();
                b.consume()
            }));
        }

        let successes: usize = handles
            .into_iter()
            .map(|h| if h.join().unwrap() { 1 } else { 0 })
            .sum();

        assert_eq!(successes, 10);
        assert_eq!(budget.used(), 10);
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_reset() {
        let budget = IterationBudget::new(5);
        // Exhaust the budget
        for _ in 0..5 {
            assert!(budget.consume());
        }
        assert!(!budget.consume());
        assert_eq!(budget.used(), 5);
        assert_eq!(budget.remaining(), 0);

        // Reset — should get a fresh budget
        budget.reset();
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.remaining(), 5);
        assert!(budget.consume());
        assert_eq!(budget.used(), 1);
    }
}
