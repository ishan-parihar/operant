//! G8 — observability for the harness.
//!
//! `tracing::instrument` is added to `Harness::mount_with_config`,
//! `replace`, and `unmount` (see `harness.rs`). This module adds a
//! counter for each state transition so a host can wire it into its
//! own metrics pipeline (Prometheus, OTel, etc.) without taking on a
//! new dependency here.
//!
//! ponytail: stdlib `AtomicU64` counters, no `prometheus` crate. The
//! host can wrap the snapshot in a Prometheus encoder.

use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counters for kernel lifecycle events.
#[derive(Debug, Default)]
pub struct HarnessMetrics {
    /// mount calls that produced an Active entry.
    pub mount_success: AtomicU64,
    /// mount calls that produced a Pending entry (late-bound).
    pub mount_pending: AtomicU64,
    /// mount calls that failed.
    pub mount_failed: AtomicU64,
    /// replace calls that succeeded.
    pub replace_success: AtomicU64,
    /// replace calls that left the old instance intact.
    pub replace_failed: AtomicU64,
    /// unmount calls (each counts the root, not the cascade).
    pub unmount_calls: AtomicU64,
    /// effect unwinds invoked.
    pub unwind_invocations: AtomicU64,
}

impl HarnessMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            mount_success: self.mount_success.load(Ordering::Relaxed),
            mount_pending: self.mount_pending.load(Ordering::Relaxed),
            mount_failed: self.mount_failed.load(Ordering::Relaxed),
            replace_success: self.replace_success.load(Ordering::Relaxed),
            replace_failed: self.replace_failed.load(Ordering::Relaxed),
            unmount_calls: self.unmount_calls.load(Ordering::Relaxed),
            unwind_invocations: self.unwind_invocations.load(Ordering::Relaxed),
        }
    }
}

/// Plain-data snapshot of the counters, safe to serialize to JSON / Prometheus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MetricsSnapshot {
    pub mount_success: u64,
    pub mount_pending: u64,
    pub mount_failed: u64,
    pub replace_success: u64,
    pub replace_failed: u64,
    pub unmount_calls: u64,
    pub unwind_invocations: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reflects_increments() {
        let m = HarnessMetrics::new();
        m.mount_success.fetch_add(2, Ordering::Relaxed);
        m.unmount_calls.fetch_add(1, Ordering::Relaxed);
        let s = m.snapshot();
        assert_eq!(s.mount_success, 2);
        assert_eq!(s.unmount_calls, 1);
        assert_eq!(s.replace_success, 0);
    }
}
