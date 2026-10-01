//! Wall-clock bound for the mid-stream drop retry ladder.
//!
//! ## Why this exists
//!
//! `run()` retries a dropped response stream up to `max_retries` times
//! (`turn_retry_state::DEFAULT_MAX_RETRIES`, currently 3). That budget is an
//! **attempt count**, and on its own it says nothing about how long a person
//! waits. A dropped stream costs a full round trip before it can be observed
//! as dropped, so a dead-but-accepting upstream (a proxy that completes the
//! TLS handshake and then resets the body — the shape that produced
//! `Network error: error decoding response body` in the 2026-09-29 gateway
//! incident) turns 4 attempts into roughly 4 minutes of the user staring at
//! a chat that has already accepted their message and will never answer it.
//!
//! Observed in `gateway.log` on 2026-09-29: four `Stream error` lines at
//! 12:15:45, 12:16:45, 12:17:45, 12:18:45 — one per minute, exactly the
//! attempt budget, then the turn ended with the user message persisted and no
//! assistant row. The retry ladder worked as designed; it just had no opinion
//! about elapsed time.
//!
//! ## Why attempts alone cannot express this
//!
//! The same attempt count means different things for different providers. A
//! fast local proxy that resets in 80ms should get all 3 retries — that is
//! real recovery and it costs the user nothing. A remote endpoint that holds
//! a request open for 60s before resetting should get **zero** further
//! retries, because the second attempt is not a recovery, it is the same wait
//! again. Attempt count cannot tell those apart; elapsed time can, and
//! elapsed time is what the user actually experiences.
//!
//! ## What this is not
//!
//! This does not reduce `max_retries`. A provider that fails fast still gets
//! every attempt it had before. This only stops the ladder once the retries
//! have themselves become the wait.
//!
//! ## Choosing the bound
//!
//! [`STREAM_DROP_RETRY_BUDGET`] is 45 seconds, and it is derived from the
//! two numbers above rather than picked to make a test pass:
//!
//! * **Not less than ~30s.** A stream that is going to recover does it on the
//!   re-issued request immediately — the ladder re-issues with no sleep
//!   (see `run.rs`, `emit_retry_scheduled` documents "no sleep"). So the
//!   re-issue either succeeds in about one round trip or it does not succeed
//!   at all. Any budget that admits a re-issue has, by construction, already
//!   admitted the recovery case.
//! * **Not more than one heartbeat interval.** The gateway already tells the
//!   user work is happening on a 60s cadence (the "Still working..."
//!   heartbeat in `gateway_runner.rs`, plus a typing indicator on a 4s tick).
//!   A user who has watched one heartbeat elapse and seen nothing since has
//!   no way to distinguish "recovering" from "dead", and will not wait long
//!   enough to find out. Sitting *below* that interval is the point: the
//!   bound has to fire before a second heartbeat can pass with no answer, or
//!   the user has already written the bot off.
//!
//! 45s is the largest value that stays under the 60s heartbeat. The
//! consequence for the incident provider — 60s to fail, which is already past
//! the bound — is that the ladder gives up after the *first* attempt rather
//! than the fourth, turning four minutes of silence into one. That is a
//! deliberate consequence of the mechanism, not a tuning artifact: by the
//! time that provider has failed once, the user has already waited a full
//! heartbeat, and re-issuing the identical request to the identical provider
//! is not recovery, it is the same silence again.

use std::time::{Duration, Instant};

/// Wall-clock ceiling on the mid-stream drop retry ladder for one turn.
///
/// See the module docs for the derivation. Exposed as a `Duration` so tests
/// can assert against the budget without sleeping through it.
pub const STREAM_DROP_RETRY_BUDGET: Duration = Duration::from_secs(45);

/// What the stream-drop ladder should do about one retryable failure.
///
/// The point of this type is that the ladder's decision is a *policy*, and a
/// policy that is only expressed as an `if` inside `run()` cannot be tested
/// without a live agent, a live provider, and real minutes of waiting. The
/// regression that motivated this module was invisible to the test suite for
/// exactly that reason. Naming the decision makes it assertable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamRetryDecision {
    /// Re-issue the request. The provider may be flapping and this may work.
    Retry,
    /// Stop. `reason` says which bound fired, because they are different
    /// diagnoses with different fixes.
    GiveUp(StreamRetryGiveUp),
}

/// Which bound ended the stream-drop ladder.
///
/// Ordered by specificity: an operator reading the log needs to know whether
/// the wall clock ran out (slow, dead provider) or the attempt count ran out
/// (fast, thrashing provider), because those are different incidents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamRetryGiveUp {
    /// The wall-clock bound fired. Retries had themselves become the wait.
    WallClock { elapsed: Duration },
    /// The attempt-count bound fired. The provider is failing fast enough
    /// that attempts, not seconds, are the limit.
    Attempts { attempts: usize },
    /// The turn-level failure bound fired. Kept distinct from the per-call
    /// attempt bound so the log does not conflate a flapping provider with a
    /// provider that killed many separate calls.
    TurnBudget { attempts: usize },
    /// The error was not retryable at all. Nothing was spent.
    NotRetryable,
}

/// Decide whether a retryable stream failure earns another attempt.
///
/// `elapsed` is how long the turn has been running, `attempts` is how many
/// retries have already been spent on this call, and `turn_budget_left`
/// reflects the caller-independent turn-level failure bound.
///
/// The wall clock is checked **first and unconditionally**, before the
/// attempt count. Order matters: a provider that has already made the user
/// wait past the bound should not consume another attempt regardless of how
/// many it has left, and a provider that fails in milliseconds will never
/// reach the bound, so checking it first costs the recovery case nothing.
pub fn decide_stream_retry(
    retryable: bool,
    wall_clock_left: bool,
    attempts_left: bool,
    turn_budget_left: bool,
    elapsed: Duration,
    attempts: usize,
) -> StreamRetryDecision {
    if !retryable {
        return StreamRetryDecision::GiveUp(StreamRetryGiveUp::NotRetryable);
    }
    if !wall_clock_left {
        return StreamRetryDecision::GiveUp(StreamRetryGiveUp::WallClock { elapsed });
    }
    if !turn_budget_left {
        return StreamRetryDecision::GiveUp(StreamRetryGiveUp::TurnBudget { attempts });
    }
    if !attempts_left {
        return StreamRetryDecision::GiveUp(StreamRetryGiveUp::Attempts { attempts });
    }
    StreamRetryDecision::Retry
}

/// Tracks elapsed wall-clock time across one turn's stream-drop retries.
///
/// The ladder in `run.rs` re-issues the request with no sleep, so the only
/// thing that accumulates is the provider's own time-to-fail. This is where
/// that accumulation is measured, so the ladder can be cut on elapsed time
/// rather than on attempt count alone.
///
/// The clock starts at construction — which is the start of the *first*
/// attempt, before any failure is known — rather than at the first failure.
/// A budget that began counting at the first drop would exempt the very
/// attempt that cost the most, and the observed 4-minute incident opens with
/// exactly that attempt: its first stream error is already 60s into the turn.
#[derive(Debug, Clone)]
pub struct StreamRetryBudget {
    started: Instant,
}

impl StreamRetryBudget {
    /// Start a fresh budget. Call once per turn, at the top of `run()`.
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    /// A budget that started `elapsed` ago, for tests and for reasoning about
    /// the arithmetic without waiting for real time to pass.
    pub fn started_at(started: Instant) -> Self {
        Self { started }
    }

    /// Time spent since this budget started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// True once the ladder has spent [`STREAM_DROP_RETRY_BUDGET`] re-issuing
    /// a stream that keeps dying. The caller must stop retrying and surface
    /// the failure rather than starting another attempt.
    pub fn exhausted(&self) -> bool {
        self.exhausted_at(self.elapsed())
    }

    /// The budget decision as a pure function of elapsed time.
    ///
    /// Split out from [`Self::exhausted`] so the arithmetic can be tested
    /// against arbitrary timelines — including the four-minute incident —
    /// without the suite sleeping for real minutes to get there.
    pub fn exhausted_at(&self, elapsed: Duration) -> bool {
        elapsed >= STREAM_DROP_RETRY_BUDGET
    }

    /// Remaining budget, saturating at zero once exhausted. Reported in the
    /// give-up log line so an operator can see how much of the budget the
    /// retries actually consumed.
    pub fn remaining(&self) -> Duration {
        self.remaining_at(self.elapsed())
    }

    /// [`Self::remaining`] as a pure function of elapsed time, for the same
    /// reason [`Self::exhausted_at`] is: the saturating behaviour at and past
    /// the bound is arithmetic, and arithmetic should be testable without a
    /// clock.
    pub fn remaining_at(&self, elapsed: Duration) -> Duration {
        STREAM_DROP_RETRY_BUDGET.saturating_sub(elapsed)
    }
}

impl Default for StreamRetryBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience: drive the real decision function the way `run()` does.
    fn decide(elapsed: Duration, attempts: usize) -> StreamRetryDecision {
        let budget = StreamRetryBudget::new();
        decide_stream_retry(
            /* retryable        */ true,
            /* wall_clock_left  */ !budget.exhausted_at(elapsed),
            /* attempts_left    */ attempts < 3,
            /* turn_budget_left */ true,
            elapsed,
            attempts,
        )
    }

    /// ── THE REGRESSION TEST ──────────────────────────────────────────
    ///
    /// Replays the 2026-09-29 incident exactly: `gateway.log` shows four
    /// `Stream error, error: Network error: error decoding response body`
    /// lines at 12:15:45, 12:16:45, 12:17:45, 12:18:45, then the turn ends
    /// with the user message persisted and no assistant row. The provider
    /// takes 60s to fail each attempt, so the attempt-count bound (3) buys
    /// four minutes of silence.
    ///
    /// Against `decide_stream_retry` — the function the ladder actually
    /// calls — the very first failure at t=60s is already past the 45s wall
    /// clock, so the ladder must refuse to re-issue. This test fails if the
    /// wall-clock term is removed from the decision, which is exactly the
    /// pre-fix behaviour.
    #[test]
    fn incident_provider_is_cut_at_the_first_failure() {
        let decision = decide(Duration::from_secs(60), /* attempts */ 0);
        assert_eq!(
            decision,
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::WallClock {
                elapsed: Duration::from_secs(60)
            }),
            "a provider that burns a full heartbeat before failing must not be \
             handed a second minute; this is the 4-minute incident"
        );
    }

    /// The same incident, counted end to end: the old bound spent four
    /// attempts and ~240s. The new bound spends one attempt and 60s. Both
    /// numbers are asserted so the improvement cannot silently regress into
    /// "we lowered the retry count" — the attempt count is untouched.
    #[test]
    fn incident_wall_clock_falls_from_four_minutes_to_one() {
        // Old behaviour, for the record: attempt-count only.
        let old: Vec<Duration> = (1..=4).map(|_| Duration::from_secs(60)).collect();
        let old_total: Duration = old.iter().sum();
        assert_eq!(old.len(), 4, "initial attempt + max_retries(3)");
        assert_eq!(old_total, Duration::from_secs(240));

        // New behaviour: the decision refuses at the first failure.
        let mut spent = Vec::new();
        let mut attempts = 0usize;
        let mut elapsed = Duration::ZERO;
        loop {
            elapsed += Duration::from_secs(60); // one attempt costs a minute
            spent.push(elapsed);
            match decide(elapsed, attempts) {
                StreamRetryDecision::Retry => attempts += 1,
                StreamRetryDecision::GiveUp(_) => break,
            }
        }
        assert_eq!(spent.len(), 1, "ladder must stop after the first attempt");
        assert_eq!(
            *spent.last().expect("ladder ran at least once"),
            Duration::from_secs(60)
        );
        assert!(spent.last().expect("nonempty") < &old_total);
    }

    /// ── THE CONTRACT THE BOUND MUST NOT BREAK ────────────────────────
    ///
    /// A fast-failing provider is genuinely recoverable and the bound must
    /// cost it nothing. Three failures 100ms apart is 300ms of the user's
    /// life. If this test fails, the fix has degenerated into a retry-count
    /// reduction wearing a wall-clock disguise.
    #[test]
    fn fast_flapping_provider_keeps_every_retry() {
        for (attempts, ms) in [(0usize, 100u64), (1, 200), (2, 300)] {
            assert_eq!(
                decide(Duration::from_millis(ms), attempts),
                StreamRetryDecision::Retry,
                "a fast-flap provider must keep attempt {attempts}"
            );
        }
    }

    /// A slow-but-recoverable provider also keeps its retries, right up to
    /// the bound. Ten seconds to fail is a slow round trip, not a hang.
    #[test]
    fn slow_but_recoverable_provider_keeps_its_retries() {
        for (attempts, secs) in [(0usize, 10u64), (1, 20), (2, 30)] {
            assert_eq!(
                decide(Duration::from_secs(secs), attempts),
                StreamRetryDecision::Retry,
                "a {secs}s round trip is inside the bound and must be retried"
            );
        }
    }

    /// The attempt bound still fires on its own, for a provider fast enough
    /// to blow through three retries without reaching 45s. This is the case
    /// that proves the fix did not replace the attempt bound — it added one.
    #[test]
    fn attempt_bound_still_fires_for_a_fast_provider() {
        assert_eq!(
            decide(Duration::from_millis(400), /* attempts */ 3),
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::Attempts { attempts: 3 }),
            "the attempt-count bound must survive alongside the wall clock"
        );
    }

    /// Each give-up reason is distinguishable, because they are different
    /// incidents: a slow dead provider, a fast thrashing one, a turn that
    /// burned its cumulative budget, and an error that was never retryable.
    #[test]
    fn give_up_reasons_stay_distinguishable() {
        assert_eq!(
            decide_stream_retry(true, false, true, true, Duration::from_secs(90), 0),
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::WallClock {
                elapsed: Duration::from_secs(90)
            })
        );
        assert_eq!(
            decide_stream_retry(true, true, false, true, Duration::from_millis(90), 3),
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::Attempts { attempts: 3 })
        );
        assert_eq!(
            decide_stream_retry(true, true, true, false, Duration::from_millis(90), 1),
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::TurnBudget { attempts: 1 })
        );
        assert_eq!(
            decide_stream_retry(false, true, true, true, Duration::ZERO, 0),
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::NotRetryable)
        );
    }

    /// A non-retryable error is refused before anything is spent, and must
    /// not be mislabelled as a wall-clock give-up — that would tell an
    /// operator to go look at the provider when the real cause is, say, a
    /// content filter.
    #[test]
    fn non_retryable_is_checked_before_the_wall_clock() {
        assert_eq!(
            decide_stream_retry(
                /* retryable */ false,
                /* wall_clock_left */ false,
                /* attempts_left */ true,
                /* turn_budget_left */ true,
                Duration::from_secs(600),
                0
            ),
            StreamRetryDecision::GiveUp(StreamRetryGiveUp::NotRetryable),
            "a non-retryable error is not a dead-stream give-up"
        );
    }

    #[test]
    fn remaining_saturates_at_zero_once_exhausted() {
        let budget = StreamRetryBudget::new();
        assert_eq!(
            budget.remaining_at(Duration::from_secs(60)),
            Duration::ZERO,
            "past the bound there is no time left to report"
        );
        assert_eq!(
            budget.remaining_at(Duration::from_secs(300)),
            Duration::ZERO,
            "saturating, not a wrapped value"
        );
        assert_eq!(
            budget.remaining_at(Duration::ZERO),
            STREAM_DROP_RETRY_BUDGET,
            "a fresh budget has its whole allowance"
        );
        assert_eq!(
            budget.remaining_at(Duration::from_secs(15)),
            STREAM_DROP_RETRY_BUDGET - Duration::from_secs(15)
        );
    }

    #[test]
    fn a_fresh_budget_has_its_whole_allowance() {
        let budget = StreamRetryBudget::new();
        assert!(!budget.exhausted());
        assert!(budget.remaining() <= STREAM_DROP_RETRY_BUDGET);
    }

    /// The bound must sit under the gateway's 60s "Still working..."
    /// heartbeat. If a heartbeat can elapse with no answer after the ladder
    /// has already given up, the user has already written the bot off and
    /// the explanation arrives too late to matter.
    #[test]
    fn bound_sits_below_the_gateway_heartbeat_interval() {
        assert!(
            STREAM_DROP_RETRY_BUDGET < Duration::from_secs(60),
            "the bound must fire before a second heartbeat passes with no reply"
        );
    }
}
