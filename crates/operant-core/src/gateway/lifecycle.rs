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
//! - [`TurnExitReason`] + [`classify_turn_error`]: UNWIRED (RT-368, named not shipped) —
//!   a machine-readable exit classifier with nothing calling it.
//!   The doc used to claim it was "written to the turn-state journal at close
//!   (the exit_code/exit_reason columns)" and that "retries, breakers, and
//!   audits branch on THIS". Neither is true: `classify_turn_error` has ZERO
//!   production callers (the only references are its own tests), and the
//!   classifier is not re-exported past this module. The LIVE per-turn
//!   classification is a different enum — `crate::agent::TurnExitReason`
//!   (TextResponse/BudgetExhausted/GraceCall/CircuitBreaker/Interrupted/
//!   Error), built by `agent::turn_finalizer` and written through
//!   `TurnDiagnostics::exit_reason`. This classifier's marker tables and
//!   kill-over-auth precedence are correct and useful; wiring it INTO that
//!   path (or deleting it) is a product decision, named rather than implied.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Machine-readable reason a turn closed.
///
/// UNWIRED (RT-368, named not shipped): the doc used to claim this is "the
/// turn-state journal's exit_code column" and that "retries, breakers, and
/// audits branch on THIS" — nothing writes this enum anywhere, and nothing
/// branches on it (zero production callers; see the module doc for the live
/// `agent::TurnExitReason` path). The classifier itself is correct; the
/// claim of its wiring was not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnExitReason {
    /// The turn returned a final response.
    Complete,
    /// Killed mid-turn by a restart, interrupt, or steer request-stop —
    /// never a provider/tool fault.
    Interrupted,
    /// Credential/authentication failure on the provider or channel.
    Auth,
    /// Turn- or call-level timeout.
    Timeout,
    /// Rate limit / quota (retryable, never an auth problem).
    RateLimit,
    /// Any other failure class.
    AgentError,
}

/// Substrings that indicate an auth-class failure (lowercase match).
const AUTH_EXIT_MARKERS: &[&str] = &[
    "unauthorized",
    "authentication",
    "credentials",
    "forbidden",
    "401",
    "403",
    "token expired",
    "not authenticated",
];

/// Unambiguous signal-kill / OOM markers (trinity #904's lesson). These
/// SHORT-CIRCUIT the auth classification: a SIGKILL is evidence the
/// subprocess died from outside, not a real auth response on the wire —
/// treating it as auth would skip-list a credential that is actually
/// fine and mask the real failure.
const KILL_EXIT_MARKERS: &[&str] = &[
    "sigkill",
    "sigterm",
    "sigint",
    "exit code -9",
    "exit code -15",
    "exit code -2",
    "exit code 137",
    "exit code 143",
    "exit code 130",
    "terminated by",
    "killed by",
    "out of memory",
    "oom",
];

const RATE_LIMIT_EXIT_MARKERS: &[&str] =
    &["429", "rate limit", "quota exceeded", "too many requests"];
const TIMEOUT_EXIT_MARKERS: &[&str] = &["timeout", "timed out", "deadline elapsed"];

/// Classify a turn-terminal error string into a [`TurnExitReason`].
/// Kill markers win over auth markers; rate-limit and timeout follow;
/// everything else is [`TurnExitReason::AgentError`].
pub fn classify_turn_error(error: &str) -> TurnExitReason {
    let lower = error.to_ascii_lowercase();
    if lower.is_empty() {
        return TurnExitReason::AgentError;
    }
    if KILL_EXIT_MARKERS.iter().any(|m| lower.contains(m)) {
        // A signal death is a hard kill of the process tree — the honest
        // bucket is the generic fault, and it must never burn auth logic.
        return TurnExitReason::AgentError;
    }
    if AUTH_EXIT_MARKERS.iter().any(|m| lower.contains(m)) {
        return TurnExitReason::Auth;
    }
    if RATE_LIMIT_EXIT_MARKERS.iter().any(|m| lower.contains(m)) {
        return TurnExitReason::RateLimit;
    }
    if TIMEOUT_EXIT_MARKERS.iter().any(|m| lower.contains(m)) {
        return TurnExitReason::Timeout;
    }
    TurnExitReason::AgentError
}

/// Per-session turn lease.
///
/// A guard is held for the duration of one turn (the full
/// `handler.handle` agent run). A second message for the same session
/// cannot acquire the lease and is answered with a polite busy reply
/// instead of launching a concurrent agent run.
///
/// The key set is a `std::sync::Mutex`, NOT a tokio `Mutex`. The
/// critical sections are pure-CPU (a set insert, a lookup, a remove)
/// and never await; and the release path is `Drop`, a SYNCHRONOUS
/// function that cannot await, so it must never need an async lock.
/// With a tokio `Mutex` the release had to `try_lock` (RT-512): when the
/// map was already held the removal was SKIPPED, the key stayed, and
/// `try_acquire` then refused that chat's every future message with
/// "still working on your previous message" — the comment claimed "a
/// stale lease self-heals on the next turn" and nothing implemented a
/// sweep. A synchronous lock makes the release unconditional.
#[derive(Debug, Clone, Default)]
pub struct TurnLease {
    inner: Arc<std::sync::Mutex<HashSet<String>>>,
}

/// Lock the lease's key set, recovering a poisoned lock.
///
/// A poison means some OTHER holder panicked while inside the critical
/// section — and the critical section is a set insert, so the data is
/// still coherent. Recovering it keeps the release path from ever
/// failing: a lease that cannot be released is the defect RT-512 was.
fn lease_keys(
    mutex: &std::sync::Mutex<HashSet<String>>,
) -> std::sync::MutexGuard<'_, HashSet<String>> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl TurnLease {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to acquire the lease for `key` non-blocking. `None` means the
    /// session is already busy with another turn.
    pub fn try_acquire(&self, key: &str) -> Option<TurnLeaseGuard> {
        let mut keys = lease_keys(&self.inner);
        if !keys.insert(key.to_string()) {
            return None;
        }
        Some(TurnLeaseGuard {
            inner: Arc::clone(&self.inner),
            key: key.to_string(),
        })
    }

    /// Whether a turn is currently in flight for `key`.
    pub fn is_busy(&self, key: &str) -> bool {
        lease_keys(&self.inner).contains(key)
    }

    /// Number of currently held leases (in-flight turns).
    pub fn in_flight(&self) -> usize {
        lease_keys(&self.inner).len()
    }
}

/// RAII guard for a held [`TurnLease`]. Released on drop (also on task
/// abort / error paths), so a lease can never leak and wedge a session.
#[derive(Debug)]
pub struct TurnLeaseGuard {
    inner: Arc<std::sync::Mutex<HashSet<String>>>,
    key: String,
}

impl Drop for TurnLeaseGuard {
    fn drop(&mut self) {
        // Unconditional: the key set is never held across an await, so
        // the lock is free whenever a caller can observe it, and a
        // poisoned lock still holds the real key set. The old
        // `try_lock`-and-skip release left the key behind whenever the
        // map happened to be held, and that key was never swept (RT-512).
        lease_keys(&self.inner).remove(&self.key);
    }
}

/// Per-session last-activity tracker for stall detection.
///
/// `touch` is called when a session's turn starts; `complete` clears it
/// when the turn ends. Sessions still "active" after `stalled(timeout)`
/// have exceeded the configured stall window without completing.
///
/// UNWIRED (RT-331, named not shipped): no production callers route it —
/// the lease gate itself (`TurnLease`) is exercised in production, but
/// the detector that would escalate stalled sessions is complete and
/// unread. Wiring a sweep over `stalled()` is a product decision; the
/// doc no longer implies the detector runs.
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
mod rt368_tests {
    /// RT-368: the classifier's docs claimed the journal column and live
    /// branching that do not exist — `classify_turn_error` has zero
    /// production callers and the live classification is
    /// `crate::agent::TurnExitReason`. Source contract, needles
    /// concatenation-built.
    #[test]
    fn the_turn_exit_classifier_names_its_unwired_state() {
        let src = include_str!("lifecycle.rs");
        let named = format!("UNWIRED (RT-368, named not {}shipped)", "");
        assert!(
            src.matches(&named).count() >= 2,
            "both the module doc and the enum doc must name the unwired surface"
        );
        let old_claim = format!("retries, breakers, and audits branch on {}", "THIS");
        assert!(
            !src.contains(&old_claim),
            "the doc must not claim a branching that does not exist"
        );
        let live_path = format!("crate::agent::{}", "TurnExitReason");
        assert!(
            src.contains(&live_path),
            "the doc must name the live classification path so a reader finds the real one"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── iter-691: turn-exit classification (trinity #904 negative-marker
    // precedence — kill markers short-circuit auth, never the reverse) ──

    #[test]
    fn classify_names_auth_failures() {
        assert_eq!(
            classify_turn_error("HTTP 401 Unauthorized from provider"),
            TurnExitReason::Auth
        );
        assert_eq!(
            classify_turn_error("telegram: bot token expired"),
            TurnExitReason::Auth
        );
    }

    #[test]
    fn classify_never_reads_a_signal_death_as_auth() {
        // The trinity #904 case: an OOM/SIGKILL stderr can contain the word
        // "credentials" — the kill markers must win.
        assert_eq!(
            classify_turn_error("child killed by SIGKILL while loading credentials"),
            TurnExitReason::AgentError
        );
        assert_eq!(
            classify_turn_error("process terminated by signal 9 (exit code 137)"),
            TurnExitReason::AgentError
        );
    }

    #[test]
    fn classify_names_rate_limit_and_timeout() {
        assert_eq!(
            classify_turn_error("429 Too Many Requests"),
            TurnExitReason::RateLimit
        );
        assert_eq!(
            classify_turn_error("request timed out after 30s"),
            TurnExitReason::Timeout
        );
    }

    #[test]
    fn classify_defaults_to_agent_error() {
        assert_eq!(classify_turn_error(""), TurnExitReason::AgentError);
        assert_eq!(
            classify_turn_error("tool returned malformed JSON"),
            TurnExitReason::AgentError
        );
    }

    #[tokio::test]
    async fn turn_lease_serializes_a_session_and_releases_on_drop() {
        let lease = TurnLease::new();
        let guard = lease.try_acquire("telegram:u1:c1").expect("first acquire");
        assert!(lease.is_busy("telegram:u1:c1"));
        assert!(
            lease.try_acquire("telegram:u1:c1").is_none(),
            "second turn for the same session must be refused"
        );
        // A different session is unaffected.
        assert!(lease.try_acquire("telegram:u2:c2").is_some());
        drop(guard);
        assert!(
            lease.try_acquire("telegram:u1:c1").is_some(),
            "lease must be releasable after the turn ends"
        );
    }

    #[tokio::test]
    async fn turn_lease_in_flight_counts() {
        let lease = TurnLease::new();
        assert_eq!(lease.in_flight(), 0);
        let g1 = lease.try_acquire("a").unwrap();
        let g2 = lease.try_acquire("b").unwrap();
        assert_eq!(lease.in_flight(), 2);
        drop(g1);
        drop(g2);
        assert_eq!(lease.in_flight(), 0);
    }

    /// RT-512: the release used to be `try_lock`-and-SKIP, so a drop
    /// that raced the map's lock left the key behind — and nothing ever
    /// swept it, so that chat was refused every later message with
    /// "still working on your previous message", forever.
    ///
    /// The release must be unconditional. Here the key set is held by
    /// ANOTHER thread at the instant the guard is dropped, which is the
    /// contention the old implementation lost.
    #[test]
    fn a_dropped_lease_is_released_even_while_the_key_set_is_held() {
        let lease = TurnLease::new();
        let guard = lease.try_acquire("telegram:u1:c1").expect("first acquire");

        // Hold the key set from another thread, as a concurrent
        // acquire/release does. The channel makes the contention
        // CERTAIN: the drop below happens only AFTER the holder has the
        // lock, not merely while it might take it.
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let holder = {
            let lease = lease.clone();
            std::thread::spawn(move || {
                let _keys = lease_keys(&lease.inner);
                held_tx.send(()).ok();
                std::thread::sleep(std::time::Duration::from_millis(200));
            })
        };
        held_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the holder takes the key set");

        drop(guard);
        holder.join().expect("the holder thread joins");

        assert!(
            lease.try_acquire("telegram:u1:c1").is_some(),
            "a lease released while the key set was held must still be re-acquirable"
        );
        assert!(!lease.is_busy("telegram:u1:c1"));
    }

    /// RT-331: the tracker presents `stalled()` as live machinery while
    /// nothing calls it — a detector that would run only if routed.
    /// Source-grep contract (the RT-323 shape), the needle concatenation-
    /// built so this test's own source cannot satisfy it.
    #[test]
    fn the_stall_detector_names_its_unwired_state() {
        let src = include_str!("lifecycle.rs");
        let claim = format!("no production callers {}", "route it");
        assert!(
            src.contains(&claim),
            "the doc must name the unwired detector — a reader must not infer it runs"
        );
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
