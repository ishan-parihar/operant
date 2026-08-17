//! Runtime retry/health metrics — a lock-free aggregation hook shared
//! between the agent loop, the memory sync executor, and the TUI status bar.
//!
//! ## Design
//!
//! The agent loop and `MemorySyncExecutor` increment atomic counters at the
//! exact points where they already log `tracing::warn!` (stream-drop
//! re-issues, empty-content retries, sync_turn failures, channel-full job
//! drops). The TUI holds an `Arc<RuntimeMetrics>` and calls `snapshot()`
//! once per frame to render a compact status pill — no channels, no event
//! bus, no locking, and zero coupling between the rendering side and the
//! hot paths that bump the counters.
//!
//! Every counter is monotonic for the process lifetime; the TUI decides
//! how to present recency from the `last_*_at` millisecond timestamps.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Millisecond-resolution wall clock used for `last_*_at` stamps.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Lock-free aggregation point for runtime retry/health counters.
///
/// Cheap to clone (`Arc`-backed), so the same registry can be shared by the
/// agent, the memory executor, and any number of readers (TUI footer,
/// `operant status`, tests).
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    /// Mid-stream SSE drops detected in the agent run loop.
    stream_drops: AtomicU64,
    /// LLM requests re-issued after a stream drop (bounded by max_retries).
    stream_retries: AtomicU64,
    /// Empty assistant turns retried (free-tier providers emitting no text,
    /// no reasoning, no tool calls).
    empty_content_retries: AtomicU64,
    /// `sync_turn` (POST /observe) failures from the memory sync executor.
    memory_sync_failures: AtomicU64,
    /// Memory jobs dropped because the executor channel was full.
    memory_jobs_dropped: AtomicU64,
    /// Identical tool-call repeats skipped by the R4 guardrail.
    guardrail_skips: AtomicU64,
    /// Truncated responses that triggered a continuation retry (T1).
    truncation_continuations: AtomicU64,
    /// Unix-millis of the last stream-drop retry (0 = never).
    last_stream_retry_at: AtomicU64,
    /// Unix-millis of the last empty-content retry (0 = never).
    last_empty_content_retry_at: AtomicU64,
    /// Unix-millis of the last memory-sync failure (0 = never).
    last_memory_failure_at: AtomicU64,
}

/// Point-in-time copy of every counter, safe to read across threads and to
/// render without holding the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub stream_drops: u64,
    pub stream_retries: u64,
    pub empty_content_retries: u64,
    pub memory_sync_failures: u64,
    pub memory_jobs_dropped: u64,
    pub guardrail_skips: u64,
    pub truncation_continuations: u64,
    pub last_stream_retry_at: u64,
    pub last_empty_content_retry_at: u64,
    pub last_memory_failure_at: u64,
}

impl MetricsSnapshot {
    /// True when any counter is non-zero (used to hide the status pill when
    /// the session has been healthy).
    pub fn has_any(&self) -> bool {
        self.stream_drops > 0
            || self.stream_retries > 0
            || self.empty_content_retries > 0
            || self.memory_sync_failures > 0
            || self.memory_jobs_dropped > 0
            || self.guardrail_skips > 0
            || self.truncation_continuations > 0
    }
}

impl RuntimeMetrics {
    /// Create a fresh, all-zero registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A stream was dropped mid-read; a retry re-issue will follow.
    pub fn record_stream_drop(&self) {
        self.stream_drops.fetch_add(1, Ordering::Relaxed);
    }

    /// An LLM request was re-issued after a stream drop.
    pub fn record_stream_retry(&self) {
        self.stream_retries.fetch_add(1, Ordering::Relaxed);
        self.last_stream_retry_at
            .store(now_millis(), Ordering::Relaxed);
    }

    /// A truncated response triggered a continuation retry (T1).
    pub fn record_truncation_continuation(&self) {
        self.truncation_continuations
            .fetch_add(1, Ordering::Relaxed);
    }

    /// The model returned an empty turn and the loop retried it.
    pub fn record_empty_content_retry(&self) {
        self.empty_content_retries.fetch_add(1, Ordering::Relaxed);
        self.last_empty_content_retry_at
            .store(now_millis(), Ordering::Relaxed);
    }

    /// A `sync_turn` background job failed (backend unreachable / HTTP error).
    pub fn record_memory_sync_failure(&self) {
        self.memory_sync_failures.fetch_add(1, Ordering::Relaxed);
        self.last_memory_failure_at
            .store(now_millis(), Ordering::Relaxed);
    }

    /// A memory job was dropped because the executor channel was full.
    pub fn record_memory_jobs_dropped(&self) {
        self.memory_jobs_dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// An identical tool-call repeat was skipped by the R4 guardrail.
    pub fn record_guardrail_skip(&self) {
        self.guardrail_skips.fetch_add(1, Ordering::Relaxed);
    }

    /// Read every counter into a cheap `Copy` snapshot.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            stream_drops: self.stream_drops.load(Ordering::Relaxed),
            stream_retries: self.stream_retries.load(Ordering::Relaxed),
            empty_content_retries: self.empty_content_retries.load(Ordering::Relaxed),
            memory_sync_failures: self.memory_sync_failures.load(Ordering::Relaxed),
            memory_jobs_dropped: self.memory_jobs_dropped.load(Ordering::Relaxed),
            guardrail_skips: self.guardrail_skips.load(Ordering::Relaxed),
            truncation_continuations: self.truncation_continuations.load(Ordering::Relaxed),
            last_stream_retry_at: self.last_stream_retry_at.load(Ordering::Relaxed),
            last_empty_content_retry_at: self.last_empty_content_retry_at.load(Ordering::Relaxed),
            last_memory_failure_at: self.last_memory_failure_at.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_snapshot_is_empty() {
        let m = RuntimeMetrics::new();
        let s = m.snapshot();
        assert!(!s.has_any());
        assert_eq!(s.stream_drops, 0);
        assert_eq!(s.memory_sync_failures, 0);
        assert_eq!(s.last_stream_retry_at, 0);
    }

    #[test]
    fn test_stream_retry_records_drop_retry_and_stamp() {
        let m = RuntimeMetrics::new();
        m.record_stream_drop();
        m.record_stream_drop();
        m.record_stream_retry();

        let s = m.snapshot();
        assert_eq!(s.stream_drops, 2);
        assert_eq!(s.stream_retries, 1);
        assert!(s.has_any());
        assert!(s.last_stream_retry_at > 0, "retry stamp must be set");
        assert_eq!(s.last_memory_failure_at, 0);
    }

    #[test]
    fn test_memory_failure_records_and_stamps() {
        let m = RuntimeMetrics::new();
        m.record_memory_sync_failure();
        m.record_memory_jobs_dropped();

        let s = m.snapshot();
        assert_eq!(s.memory_sync_failures, 1);
        assert_eq!(s.memory_jobs_dropped, 1);
        assert!(s.last_memory_failure_at > 0);
        assert_eq!(s.stream_retries, 0);
    }

    #[test]
    fn test_shared_across_threads() {
        use std::thread;
        let m = std::sync::Arc::new(RuntimeMetrics::new());
        let mut handles = Vec::new();
        for _ in 0..4 {
            let m = std::sync::Arc::clone(&m);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    m.record_stream_retry();
                    m.record_memory_sync_failure();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let s = m.snapshot();
        assert_eq!(s.stream_retries, 4000);
        assert_eq!(s.memory_sync_failures, 4000);
    }
}
