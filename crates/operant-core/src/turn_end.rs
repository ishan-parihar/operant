//! Post-turn event seam — the attachment point for reflection, advisor, and
//! dreaming.
//!
//! Exactly one [`TurnEnd`] is emitted per completed turn, at the agent's
//! turn-end chokepoint (`agent/run.rs`, immediately after the memory
//! `sync_turn` / `queue_prefetch` hooks, so observers see the same ordering
//! the memory path does). Post-turn features subscribe to a
//! [`TurnEndBus`] instead of wrapping the agent loop.
//!
//! This module is the **seam only**. It emits, it fans out, it caps. It
//! does not score turns, write memory, or call a model.
//!
//! # Subscription contract
//!
//! * **Transport** — a [`tokio::sync::broadcast`] channel, cloned per
//!   subscriber via [`TurnEndBus::subscribe`]. Every live subscriber sees
//!   every event; there is no filtering and no per-subscriber delivery ack
//!   beyond the return value of [`TurnEndBus::emit`].
//! * **Ordering** — events are delivered in emission order per subscriber.
//!   `turn_id` is a per-bus monotonic counter (`0, 1, 2, …`), so a
//!   subscriber can assert it has not been reordered or replayed.
//! * **Blocking** — [`TurnEndBus::emit`] is synchronous and never awaits.
//!   The agent turn cannot be slowed by a subscriber. A subscriber that
//!   needs a background task uses [`crate::daemon_pool::spawn`], the
//!   existing fire-and-forget fork primitive this seam reuses rather than
//!   re-implementing.
//! * **Lag / overflow** — the channel holds [`SUBSCRIBER_QUEUE`] events. A
//!   subscriber that falls further behind is not blocked and is not fed
//!   stale filler: it receives `Err(RecvError::Lagged(n))` and decides
//!   whether to resync or skip. Separately, per-turn tool durations are
//!   capped at [`MAX_TOOL_DURATIONS`] and further durations are **dropped
//!   with a `tracing::warn`** — the same drop-and-warn discipline
//!   `MemorySyncExecutor::submit_sync_turn` uses when its channel is full.
//! * **Failure isolation** — the seam never propagates subscriber failure.
//!   A panicking or erroring subscriber dies in its own task; the turn
//!   already returned. Subscribers log their own errors at `warn`.
//! * **Late subscribers** — a subscriber attached mid-turn receives
//!   nothing for that turn. The next completed turn is the first event.
//!
//! # Cost with no subscribers
//!
//! Two independent guards, and neither touches the conversation:
//!
//! 1. The agent's field is `Option<TurnEndBus>` and defaults to `None`.
//!    With no bus attached the emit site is a single `None` check —
//!    nothing is constructed, no allocation happens, and the surrounding
//!    sync_turn / prefetch lines are byte-identical to the pre-seam code.
//! 2. When a bus *is* attached but nobody has subscribed, both
//!    [`TurnEndBus::emit`] and [`TurnEndBus::record_tool_duration`] return
//!    on a `receiver_count() == 0` read, before the event is built.
//!
//! **Why no history is ever cloned** (the code-level reason a runtime
//! allocator counter cannot observe it): [`TurnEnd`] has no message or
//! history field of any kind — no `Vec<Message>`, no `Arc<Vec<..>>`. It is
//! built from four scalars plus `&str` slices that are copied through
//! [`summarize`], which truncates to [`RESULT_SUMMARY_LIMIT`] bytes on a
//! char boundary. The emit call site passes `&result.content` — a string
//! the agent already owns — and never touches `messages`. Cloning
//! `messages` at that point would be a strictly larger operation than
//! everything the seam does put together, and nothing in this module is
//! capable of it.
//!
//! # Example
//!
//! ```no_run
//! use operant_core::turn_end::TurnEndBus;
//!
//! let bus = TurnEndBus::new();
//! let mut rx = bus.subscribe();
//! // Run the subscriber off the turn path.
//! operant_core::daemon_pool::spawn("reflection", async move {
//!     while let Ok(event) = rx.recv().await {
//!         let _ = event; // score, advise, dream — in a later lane
//!     }
//! });
//! let delivered = bus.emit("session-1", 3, 7, "assistant reply");
//! assert_eq!(delivered, 1);
//! ```

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use crate::agent::safe_truncate_str;

/// Maximum size of [`TurnEnd::result_summary`], in bytes. The full assistant
/// reply is *not* carried — a subscriber that needs it re-reads the
/// session, or asks for the field to be widened here deliberately.
pub const RESULT_SUMMARY_LIMIT: usize = 512;

/// Maximum number of per-tool durations retained for one turn. Beyond this
/// the duration is dropped and a `tracing::warn` is emitted, mirroring
/// `MemorySyncExecutor`'s channel-full drop discipline. Prevents a runaway
/// turn from growing the bus without bound.
pub const MAX_TOOL_DURATIONS: usize = 256;

/// Per-subscriber queue depth. A subscriber that lags past this many events
/// gets `RecvError::Lagged` rather than stalling the turn.
pub const SUBSCRIBER_QUEUE: usize = 64;

/// A completed turn, as observed by post-turn subscribers.
///
/// Deliberately small and history-free — see the module docs. Every field
/// is either a counter, a short string, or a bounded vector, so cloning a
/// `TurnEnd` (which the broadcast transport does per subscriber) costs a
/// few hundred bytes at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEnd {
    /// Per-bus monotonic turn counter, starting at 0. Cheap to produce
    /// (one relaxed atomic increment) and enough for a subscriber to
    /// detect reordering, replay, or a missed turn.
    pub turn_id: u64,
    /// Session the turn belonged to.
    pub session_id: String,
    /// Model round-trips consumed by the turn.
    pub iterations: usize,
    /// Tool calls the model requested during the turn. This counts
    /// *requests*; `tool_durations_ms` only has entries for calls that
    /// actually reached the agent's tool-execution phase, so the two can
    /// differ when a call was rejected in pre-flight (unparseable args,
    /// guardrail skip, unknown tool, or user-denied approval).
    pub tool_calls: usize,
    /// Per-tool wall-clock durations in milliseconds, in completion order.
    /// Concurrency means this order need not match the call order. Bounded
    /// by [`MAX_TOOL_DURATIONS`]; the tail is dropped with a warning.
    pub tool_durations_ms: Vec<u64>,
    /// First [`RESULT_SUMMARY_LIMIT`] bytes of the assistant's final reply,
    /// cut on a UTF-8 char boundary.
    pub result_summary: String,
    /// True when the reply was longer than [`RESULT_SUMMARY_LIMIT`] and
    /// `result_summary` is therefore a prefix.
    pub result_truncated: bool,
}

/// Cloneable handle to the post-turn event seam.
///
/// Cheap to clone (`broadcast::Sender` is an `Arc` bump plus a small
/// refcount); hand clones to features that need to subscribe from their
/// own scope.
#[derive(Clone)]
pub struct TurnEndBus {
    tx: broadcast::Sender<TurnEnd>,
    durations: Arc<Mutex<Vec<u64>>>,
    turn_seq: Arc<AtomicU64>,
}

impl TurnEndBus {
    /// Create a bus with [`SUBSCRIBER_QUEUE`] slots per subscriber and no
    /// subscribers attached. An unobserved bus costs nothing beyond this
    /// allocation — see the module docs.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(SUBSCRIBER_QUEUE);
        Self {
            tx,
            durations: Arc::new(Mutex::new(Vec::new())),
            turn_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Attach a subscriber. Every event emitted from now on is delivered
    /// to it, in order, until the returned receiver is dropped.
    pub fn subscribe(&self) -> broadcast::Receiver<TurnEnd> {
        self.tx.subscribe()
    }

    /// Number of live subscribers. `0` means both [`Self::emit`] and
    /// [`Self::record_tool_duration`] are no-ops.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Record one tool call's wall-clock duration for the turn currently
    /// in flight. Called by the agent's tool executor; the values are
    /// drained into the next [`TurnEnd`].
    ///
    /// A no-op when nobody is subscribed, so an observed-but-unwatched bus
    /// does not accumulate timings nobody will read.
    pub fn record_tool_duration(&self, ms: u64) {
        if self.tx.receiver_count() == 0 {
            return;
        }
        match self.durations.lock() {
            Ok(mut queue) => {
                if queue.len() >= MAX_TOOL_DURATIONS {
                    tracing::warn!(
                        limit = MAX_TOOL_DURATIONS,
                        "TurnEndBus: tool duration queue full — duration dropped"
                    );
                    return;
                }
                queue.push(ms);
            }
            Err(_) => {
                tracing::warn!("TurnEndBus: tool duration queue poisoned — duration dropped");
            }
        }
    }

    /// Emit one [`TurnEnd`] for a completed turn. Returns the number of
    /// subscribers the event was handed to.
    ///
    /// Synchronous by design: the agent loop must not await a subscriber.
    /// Returns `0` immediately when there are no subscribers — the event
    /// is never constructed, so this costs one `Option` check at the call
    /// site and one `receiver_count()` read here.
    pub fn emit(
        &self,
        session_id: &str,
        iterations: usize,
        tool_calls: usize,
        result: &str,
    ) -> usize {
        if self.tx.receiver_count() == 0 {
            return 0;
        }
        let (result_summary, result_truncated) = summarize(result);
        let event = TurnEnd {
            turn_id: self.turn_seq.fetch_add(1, Ordering::Relaxed),
            session_id: session_id.to_string(),
            iterations,
            tool_calls,
            tool_durations_ms: self.take_durations(),
            result_summary,
            result_truncated,
        };
        // Racing subscribers — every receiver dropped between the count
        // check and the send — fall back to 0: the event is dropped, never
        // cloned, and the turn is unaffected.
        self.tx.send(event).unwrap_or_default()
    }

    /// Fork a subscriber's work off the turn path.
    ///
    /// Thin delegation to [`crate::daemon_pool::spawn`] — the seam reuses
    /// the existing fire-and-forget primitive (handles tracked, panics
    /// contained, `drain_for_label` for tests) rather than growing a
    /// second one. Errors inside `fut` must be logged by `fut` at `warn`;
    /// the pool only guarantees the task never breaks the caller.
    pub fn fork<F>(&self, label: impl Into<String>, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        crate::daemon_pool::spawn(label, fut);
    }

    /// Take everything recorded since the last drain.
    fn take_durations(&self) -> Vec<u64> {
        match self.durations.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        }
    }
}

impl Default for TurnEndBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate the assistant reply to [`RESULT_SUMMARY_LIMIT`] bytes on a UTF-8
/// char boundary, reporting whether anything was cut.
fn summarize(result: &str) -> (String, bool) {
    let truncated = result.len() > RESULT_SUMMARY_LIMIT;
    (
        safe_truncate_str(result, RESULT_SUMMARY_LIMIT).to_string(),
        truncated,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// Unique daemon labels: `drain_for_label` claims handles out of one
    /// global pool, so tests sharing a label would steal each other's
    /// subscriber.
    const COLLECTOR_A: &str = "turn-end-test-collector-a";
    const COLLECTOR_B: &str = "turn-end-test-collector-b";
    const COLLECTOR_C: &str = "turn-end-test-collector-c";
    const COLLECTOR_D: &str = "turn-end-test-collector-d";

    /// A subscriber driven by the seam's fork primitive, collecting every
    /// event it is handed. Stands in for reflection / advisor / dreaming:
    /// the loop is all a subscriber needs, and the feature logic is a
    /// later lane.
    ///
    /// Stops after `expect` events so `drain_for_label` — which awaits the
    /// task's handle — can complete. A real subscriber instead runs until
    /// the channel closes.
    ///
    /// `label` must be unique per test: `drain_for_label` claims handles
    /// globally, so a shared label would let one test drain another's
    /// subscriber.
    fn spawn_collector(
        bus: &TurnEndBus,
        seen: Arc<Mutex<Vec<TurnEnd>>>,
        expect: usize,
        label: &str,
    ) {
        let mut rx = bus.subscribe();
        bus.fork(label.to_string(), async move {
            for _ in 0..expect {
                match rx.recv().await {
                    Ok(event) => {
                        let mut seen = match seen.lock() {
                            Ok(seen) => seen,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        seen.push(event);
                    }
                    Err(_) => return,
                }
            }
        });
    }

    async fn collected(seen: &Arc<Mutex<Vec<TurnEnd>>>, label: &str) -> Vec<TurnEnd> {
        let drained = crate::daemon_pool::drain_for_label(label, Duration::from_secs(10)).await;
        assert_eq!(drained, 1, "collector daemon must have run");
        let guard = seen.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    #[tokio::test]
    async fn test_subscriber_receives_event_with_full_payload() {
        let bus = TurnEndBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        spawn_collector(&bus, seen.clone(), 1, COLLECTOR_A);
        assert_eq!(bus.subscriber_count(), 1);

        bus.record_tool_duration(12);
        bus.record_tool_duration(340);
        bus.record_tool_duration(7);

        let delivered = bus.emit("sess-1", 3, 7, "all done");
        assert_eq!(delivered, 1, "one subscriber, one delivery");

        let events = collected(&seen, COLLECTOR_A).await;
        assert_eq!(events.len(), 1, "one emit must produce one event");
        let event = &events[0];
        assert_eq!(event.turn_id, 0);
        assert_eq!(event.session_id, "sess-1");
        assert_eq!(event.iterations, 3);
        assert_eq!(event.tool_calls, 7);
        assert_eq!(event.tool_durations_ms, vec![12, 340, 7]);
        assert_eq!(event.result_summary, "all done");
        assert!(!event.result_truncated);
    }

    #[tokio::test]
    async fn test_event_fires_once_per_turn_in_order() {
        let bus = TurnEndBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        spawn_collector(&bus, seen.clone(), 3, COLLECTOR_B);

        for turn in 0..3u64 {
            assert_eq!(bus.emit("sess-2", 1, 0, "reply"), 1);
            assert_eq!(bus.turn_seq.load(Ordering::Relaxed), turn + 1);
        }

        let events = collected(&seen, COLLECTOR_B).await;
        assert_eq!(events.len(), 3, "one event per completed turn");
        assert_eq!(
            events.iter().map(|e| e.turn_id).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "turn ids must be monotonic and in emission order"
        );
        // Durations drain per turn: turn N never sees turn N+1's timings.
        assert!(events.iter().all(|e| e.tool_durations_ms.is_empty()));
    }

    #[tokio::test]
    async fn test_two_subscribers_both_receive() {
        let bus = TurnEndBus::new();
        let a = Arc::new(Mutex::new(Vec::new()));
        let b = Arc::new(Mutex::new(Vec::new()));
        let mut rx_b = bus.subscribe();
        let b_clone = b.clone();
        bus.fork(COLLECTOR_C.to_string(), async move {
            if let Ok(event) = rx_b.recv().await {
                let mut guard = b_clone.lock().unwrap_or_else(|e| e.into_inner());
                guard.push(event);
            }
        });
        spawn_collector(&bus, a.clone(), 1, COLLECTOR_D);
        assert_eq!(bus.subscriber_count(), 2);

        assert_eq!(bus.emit("sess-3", 1, 0, "hi"), 2);

        let drained =
            crate::daemon_pool::drain_for_label(COLLECTOR_C, Duration::from_secs(10)).await;
        assert_eq!(drained, 1);
        let _ = collected(&a, COLLECTOR_D).await;

        assert_eq!(a.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
        assert_eq!(b.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
    }

    #[test]
    fn test_result_summary_is_capped_and_flagged() {
        let long = "é".repeat(RESULT_SUMMARY_LIMIT * 4);
        let (summary, truncated) = summarize(&long);
        assert!(truncated);
        assert!(
            summary.len() <= RESULT_SUMMARY_LIMIT,
            "summary must never exceed the cap, got {}",
            summary.len()
        );
        // Multi-byte input must not panic on the cut and must stay valid.
        assert!(summary.chars().all(|c| c == 'é'));

        let exact = "a".repeat(RESULT_SUMMARY_LIMIT);
        let (summary, truncated) = summarize(&exact);
        assert!(!truncated, "exactly-at-cap is not truncation");
        assert_eq!(summary.len(), RESULT_SUMMARY_LIMIT);
    }

    #[test]
    fn test_emit_without_subscribers_is_free_and_records_nothing() {
        let bus = TurnEndBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        // No subscriber: emit is a no-op, and it does not consume the
        // turn counter or leave state behind for a later subscriber.
        assert_eq!(bus.emit("sess-4", 2, 5, "reply"), 0);
        assert_eq!(bus.turn_seq.load(Ordering::Relaxed), 0);
        // Nor does an unobserved tool duration accumulate.
        for _ in 0..MAX_TOOL_DURATIONS * 2 {
            bus.record_tool_duration(1);
        }
        assert!(
            bus.durations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty(),
            "durations must not accumulate while nobody is subscribed"
        );
    }

    #[test]
    fn test_duration_queue_is_bounded_and_drops_the_tail() {
        let bus = TurnEndBus::new();
        // Keep one subscriber alive so recording is not short-circuited.
        let _rx = bus.subscribe();
        for _ in 0..(MAX_TOOL_DURATIONS + 10) {
            bus.record_tool_duration(5);
        }
        let queued = bus.durations.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            queued.len(),
            MAX_TOOL_DURATIONS,
            "overflow must drop, not grow without bound"
        );
    }

    #[test]
    fn test_one_emission_site_in_the_turn_loop() {
        // "Fires exactly once per completed turn" is a property of the call
        // site, which no runtime test of the bus can observe. Assert it at
        // the source instead: run.rs references the bus exactly once, at
        // the turn-end chokepoint. A second site (e.g. wiring the
        // budget-exhausted grace path) must be a deliberate change here.
        let run_rs = include_str!("agent/run.rs");
        assert_eq!(
            run_rs.matches("self.turn_end_bus").count(),
            1,
            "run.rs must hold exactly one turn-end emission site"
        );
        // The timings producer is the tool executor, and nothing else.
        // All three execute_tools branches time their call: single,
        // serialized-interfering-batch, and the concurrent pool.
        let stream_rs = include_str!("agent/stream.rs");
        assert_eq!(
            stream_rs.matches("record_tool_duration").count(),
            3,
            "every execute_tools branch (single + sequential + concurrent) must time its tool call"
        );
    }

    #[tokio::test]
    async fn test_fork_runs_off_the_caller_and_failures_stay_local() {
        let bus = TurnEndBus::new();
        let ran = Arc::new(AtomicUsize::new(0));
        let r = ran.clone();
        // A subscriber that panics must not affect the emitter.
        bus.fork("turn-end-test-boom", async move {
            r.fetch_add(1, Ordering::SeqCst);
            panic!("subscriber blew up");
        });
        let kept = bus.subscribe();
        assert_eq!(bus.emit("sess-5", 1, 0, "still fine"), 1);
        let drained =
            crate::daemon_pool::drain_for_label("turn-end-test-boom", Duration::from_secs(10))
                .await;
        assert_eq!(drained, 1);
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(bus.emit("sess-5", 2, 0, "and again"), 1);
        assert_eq!(
            kept.len(),
            2,
            "the panicking fork left the bus delivering every turn"
        );
    }
}
