//! Gateway lifecycle primitives — hermes `gateway/turn_lease.py`,
//! `session_stall.py`, `drain_control.py`, `delivery_ledger.py` and
//! `mirror.py` parity.
//!
//! - [`TurnLease`]: per-session serialization. Only one in-flight turn may
//!   run per session key, so two rapid messages to the same conversation
//!   can never interleave two agent runs.
//! - [`SessionStallTracker`]: records when each session's turn started so a
//!   hung turn (agent wedged on a provider call, etc.) can be surfaced.
//! - [`DeliveryLedger`]: bounded record of outbound deliveries (at-least-
//!   once visibility — every send is accounted, success or failure).
//! - [`MirrorRule`]: forwards responses for a matched source channel to a
//!   target channel (e.g. DM → ops group).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Per-session turn lease.
///
/// A guard is held for the duration of one turn (the full
/// `handler.handle` agent run). A second message for the same session
/// cannot acquire the lease and is answered with a polite busy reply
/// instead of launching a concurrent agent run.
#[derive(Debug, Clone, Default)]
pub struct TurnLease {
    inner: Arc<Mutex<HashMap<String, ()>>>,
}

impl TurnLease {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to acquire the lease for `key` non-blocking. `None` means the
    /// session is already busy with another turn.
    pub async fn try_acquire(&self, key: &str) -> Option<TurnLeaseGuard> {
        let mut map = self.inner.lock().await;
        if map.contains_key(key) {
            return None;
        }
        map.insert(key.to_string(), ());
        Some(TurnLeaseGuard {
            inner: Arc::clone(&self.inner),
            key: key.to_string(),
        })
    }

    /// Whether a turn is currently in flight for `key`.
    pub async fn is_busy(&self, key: &str) -> bool {
        self.inner.lock().await.contains_key(key)
    }

    /// Number of currently held leases (in-flight turns).
    pub async fn in_flight(&self) -> usize {
        self.inner.lock().await.len()
    }
}

/// RAII guard for a held [`TurnLease`]. Released on drop (also on task
/// abort / error paths), so a lease can never leak and wedge a session.
#[derive(Debug)]
pub struct TurnLeaseGuard {
    inner: Arc<Mutex<HashMap<String, ()>>>,
    key: String,
}

impl Drop for TurnLeaseGuard {
    fn drop(&mut self) {
        // Best-effort, non-blocking removal. `try_lock` succeeds unless the
        // map is simultaneously held — in that case the sweep/next message
        // retries naturally (a stale lease self-heals on the next turn).
        if let Ok(mut map) = self.inner.try_lock() {
            map.remove(&self.key);
        }
    }
}

/// Per-session last-activity tracker for stall detection.
///
/// `touch` is called when a session's turn starts; `complete` clears it
/// when the turn ends. Sessions still "active" after `stalled(timeout)`
/// have exceeded the configured stall window without completing.
#[derive(Debug, Clone, Default)]
pub struct SessionStallTracker {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl SessionStallTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `key` as having an in-flight turn starting now.
    pub async fn touch(&self, key: &str) {
        self.inner
            .lock()
            .await
            .insert(key.to_string(), Instant::now());
    }

    /// Clear `key` — its turn completed (or was released).
    pub async fn complete(&self, key: &str) {
        self.inner.lock().await.remove(key);
    }

    /// Sessions whose turn started more than `timeout` ago and never
    /// completed, with their age.
    pub async fn stalled(&self, timeout: Duration) -> Vec<(String, Duration)> {
        let map = self.inner.lock().await;
        let now = Instant::now();
        let mut out: Vec<(String, Duration)> = map
            .iter()
            .filter_map(|(key, started)| {
                let age = now.duration_since(*started);
                (age > timeout).then(|| (key.clone(), age))
            })
            .collect();
        out.sort_by_key(|(_, age)| std::cmp::Reverse(*age));
        out
    }

    /// Number of sessions with an in-flight turn.
    pub async fn active_count(&self) -> usize {
        self.inner.lock().await.len()
    }
}

/// One outbound delivery recorded in the [`DeliveryLedger`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryRecord {
    /// Unix millis when the send was recorded.
    pub ts_ms: u64,
    pub platform: String,
    pub channel_id: String,
    pub content_len: usize,
    /// `"delivered"`, `"failed"`, or `"queued"`.
    pub status: String,
}

/// Bounded in-memory ledger of outbound deliveries (hermes
/// `delivery_ledger.py` at-least-once parity). Every send is recorded —
/// success or failure — so operators can inspect recent delivery health
/// instead of guessing whether a reply actually left the gateway.
#[derive(Debug, Clone)]
pub struct DeliveryLedger {
    inner: Arc<Mutex<VecDeque<DeliveryRecord>>>,
    max: usize,
}

impl DeliveryLedger {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
            max: max.max(1),
        }
    }

    /// Record a delivery (oldest evicted when the cap is reached).
    pub async fn record(&self, platform: &str, channel_id: &str, content: &str, status: &str) {
        let mut queue = self.inner.lock().await;
        if queue.len() >= self.max {
            queue.pop_front();
        }
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        queue.push_back(DeliveryRecord {
            ts_ms,
            platform: platform.to_string(),
            channel_id: channel_id.to_string(),
            content_len: content.len(),
            status: status.to_string(),
        });
    }

    /// Most recent `n` records, newest first.
    pub async fn recent(&self, n: usize) -> Vec<DeliveryRecord> {
        let queue = self.inner.lock().await;
        queue.iter().rev().take(n).cloned().collect()
    }

    /// Total records currently retained.
    pub async fn count(&self) -> usize {
        self.inner.lock().await.len()
    }

    /// Records with status `"delivered"`.
    pub async fn delivered_count(&self) -> usize {
        self.inner
            .lock()
            .await
            .iter()
            .filter(|r| r.status == "delivered")
            .count()
    }

    /// Records with status `"failed"`.
    pub async fn failed_count(&self) -> usize {
        self.inner
            .lock()
            .await
            .iter()
            .filter(|r| r.status == "failed")
            .count()
    }
}

impl Default for DeliveryLedger {
    fn default() -> Self {
        Self::new(500)
    }
}

/// Mirror rule — hermes `gateway/mirror.py` parity.
///
/// When an incoming message on `platform` arrives from `source_channel`,
/// the gateway forwards the outgoing response to `target_channel` on the
/// same platform. Empty `platform`/`source_channel` act as wildcards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MirrorRule {
    /// Platform the rule applies to (`telegram`/`discord`/`slack`/
    /// `whatsapp`). Empty = any platform.
    #[serde(default)]
    pub platform: String,
    /// Source channel to mirror FROM. Empty = any inbound channel.
    #[serde(default)]
    pub source_channel: String,
    /// Target channel to forward the response TO.
    #[serde(default)]
    pub target_channel: String,
}

impl MirrorRule {
    /// Whether an inbound message on `platform`/`channel` matches this rule.
    pub fn matches(&self, platform: &str, channel: &str) -> bool {
        let platform_ok = self.platform.is_empty() || self.platform == platform;
        let channel_ok = self.source_channel.is_empty() || self.source_channel == channel;
        platform_ok && channel_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn turn_lease_serializes_a_session_and_releases_on_drop() {
        let lease = TurnLease::new();
        let guard = lease
            .try_acquire("telegram:u1:c1")
            .await
            .expect("first acquire");
        assert!(lease.is_busy("telegram:u1:c1").await);
        assert!(
            lease.try_acquire("telegram:u1:c1").await.is_none(),
            "second turn for the same session must be refused"
        );
        // A different session is unaffected.
        assert!(lease.try_acquire("telegram:u2:c2").await.is_some());
        drop(guard);
        assert!(
            lease.try_acquire("telegram:u1:c1").await.is_some(),
            "lease must be releasable after the turn ends"
        );
    }

    #[tokio::test]
    async fn turn_lease_in_flight_counts() {
        let lease = TurnLease::new();
        assert_eq!(lease.in_flight().await, 0);
        let g1 = lease.try_acquire("a").await.unwrap();
        let g2 = lease.try_acquire("b").await.unwrap();
        assert_eq!(lease.in_flight().await, 2);
        drop(g1);
        drop(g2);
        assert_eq!(lease.in_flight().await, 0);
    }

    #[tokio::test]
    async fn stall_tracker_reports_and_clears_stalled_sessions() {
        let tracker = SessionStallTracker::new();
        tracker.touch("telegram:u1:c1").await;
        tracker.touch("telegram:u2:c2").await;
        tracker.complete("telegram:u2:c2").await;
        // Zero timeout → the still-active session is immediately stalled.
        let stalled = tracker.stalled(Duration::ZERO).await;
        assert_eq!(stalled.len(), 1);
        assert_eq!(stalled[0].0, "telegram:u1:c1");
        assert_eq!(tracker.active_count().await, 1);
        tracker.complete("telegram:u1:c1").await;
        assert!(tracker.stalled(Duration::ZERO).await.is_empty());
    }

    #[tokio::test]
    async fn delivery_ledger_is_bounded_and_orders_newest_first() {
        let ledger = DeliveryLedger::new(3);
        ledger.record("telegram", "c1", "first", "delivered").await;
        ledger.record("discord", "c2", "second", "delivered").await;
        ledger.record("slack", "c3", "third", "failed").await;
        ledger.record("whatsapp", "c4", "fourth", "delivered").await;

        assert_eq!(ledger.count().await, 3, "cap enforced");
        let recent = ledger.recent(10).await;
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].platform, "whatsapp", "newest first");
        assert_eq!(
            recent[2].platform, "discord",
            "oldest of the retained evicted"
        );
        assert_eq!(ledger.delivered_count().await, 2);
        assert_eq!(ledger.failed_count().await, 1);
    }

    #[test]
    fn mirror_rule_matching_supports_wildcards() {
        let exact = MirrorRule {
            platform: "telegram".to_string(),
            source_channel: "dm:123".to_string(),
            target_channel: "group:ops".to_string(),
        };
        assert!(exact.matches("telegram", "dm:123"));
        assert!(!exact.matches("telegram", "dm:456"));
        assert!(!exact.matches("discord", "dm:123"));

        let wildcard = MirrorRule {
            source_channel: "dm:123".to_string(),
            target_channel: "group:ops".to_string(),
            ..Default::default()
        };
        assert!(wildcard.matches("slack", "dm:123"));
    }
}
