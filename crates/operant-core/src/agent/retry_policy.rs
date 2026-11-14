//! The two-span agentic retry policy.
//!
//! ## The shape
//!
//! One policy, two spans, one opt-in ceiling. The transport already retries
//! inside a single request (`RateLimitSettings`, 3 x 60 s with exponential
//! backoff) and the mid-stream ladder already bounds a dropped body
//! (`stream_retry_budget`, 45 s). What neither of them answers is the SEAT:
//! a provider that is down for four minutes leaves a chat sitting there with
//! a message accepted and no answer, and the only knob that could stop it was
//! the turn's wall clock — which stops the TURN, not the retry.
//!
//! So the ladder above the transport has two spans:
//!
//! * **small span** (`1..=small_span_retries`): immediate re-issues of the
//!   SAME request, no sleep. A provider that blipped and is fine now costs one
//!   round trip, which is the common case and the one worth being fast about.
//! * **large span** (`> small_span_retries`): one request per `large_span_secs`
//!   — 300 s by default — and it does not stop. An infinite retry is the
//!   default on purpose: a seat that gives up is a seat that goes dark, and
//!   the operator's only other signal is a missing reply.
//! * **ceiling** (`max_span_secs`): opt-in. `0` (the default) is infinite.
//!   When it is set and the elapsed retry span reaches it, the ladder stops and
//!   surfaces the typed error ("gave up after Xm").
//!
//! ## Why the stop is a span and not the turn's wall clock
//!
//! They are different bounds for different questions. The wall clock answers
//! "how long may ONE turn take" and is the caller's business; the retry span
//! answers "how long may the RETRY keep going". A caller that sets a short
//! wall clock is not asking for fewer retries, it is asking for a shorter
//! turn — conflating them is how an operator who sets a 60 s turn silently
//! loses the 5-minute span they never knew they had.
//!
//! ## Rate-limit awareness
//!
//! A `RateLimit` (429) failure carries the provider's own `retry_after`. The
//! flat large span is a GUESS at how long the provider wants; the hint is the
//! provider telling us. The hint wins when it is present and sane (a positive
//! duration), and the flat span applies otherwise — including when the hint is
//! longer than the span, because the provider's word is the authority on its
//! own limiter, not our guess about it.
//!
//! ## What this is not
//!
//! This is not the transport retry (`RateLimitSettings`) and not the
//! mid-stream budget (`stream_retry_budget`). Those are wire-layer failures
//! with their own bounds; this policy is the layer above them, and it is the
//! only one that can wait minutes.

use crate::config::RetrySettings;
use std::time::{Duration, Instant};

/// How long a failure must be left alone before the NEXT attempt, or a stop
/// carrying the elapsed span when the ladder is over.
///
/// Returned by [`RetryPolicy::next_delay`] on every decision so the caller can
/// both sleep and announce from the one answer — a caller that computed the
/// sleep and the announcement separately is how a pill says "retrying in 5
/// minutes" while the loop actually waits 30 seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Re-issue after this delay. `announced` is true for a large-span retry,
    /// which is the one the operator should see.
    Retry { delay: Duration, announced: bool },
    /// Stop: the ladder is out of small-span attempts AND the ceiling (if any)
    /// has been reached. The caller surfaces the last error.
    Stop { elapsed: Duration },
}

impl RetryDecision {
    /// The delay to sleep, or `None` when the decision is `Stop`.
    ///
    /// A `Stop` has no delay by construction: sleeping before giving up is
    /// how a ladder overshoots the ceiling it just honoured.
    #[must_use]
    pub fn to_retry_delay(&self) -> Option<Duration> {
        match self {
            Self::Retry { delay, .. } => Some(*delay),
            Self::Stop { .. } => None,
        }
    }

    /// Whether this decision is a large-span retry the operator should see.
    #[must_use]
    pub fn is_announced(&self) -> bool {
        matches!(
            self,
            Self::Retry {
                announced: true,
                ..
            }
        )
    }
}

/// The two-span policy over a [`RetrySettings`] block.
///
/// Constructed once per retry ladder and advanced one step at a time. The
/// clock is passed IN rather than captured, so the tests can drive it with an
/// injected instant and the production path can pass `Instant::now()`.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    settings: RetrySettings,
}

impl RetryPolicy {
    /// Build the policy from the operator's `[agent.retry]` block.
    ///
    /// The block is already healed at load (`config_heal`), so an out-of-range
    /// value reaching here is a caller that built its own config — the policy
    /// clamps rather than panicking, because a retry ladder that panics on a
    /// bad number takes the turn down with it.
    #[must_use]
    pub fn new(settings: RetrySettings) -> Self {
        Self { settings }
    }

    /// The operator's settings, clamped into the usable range.
    ///
    /// `small_span_retries` is taken as written (0 is legal and means "no
    /// small span"). `large_span_secs` must be > 0: a zero-length span is a
    /// busy loop, which is strictly worse than the hang this policy exists to
    /// remove, so it becomes the default 300 s. `max_span_secs` of 0 is the
    /// documented INFINITE, not a clamp.
    #[must_use]
    pub fn settings(&self) -> &RetrySettings {
        &self.settings
    }

    /// Whether `elapsed` has reached the opt-in ceiling.
    ///
    /// `max_span_secs == 0` is infinite and never reaches it. A span of 1 s or
    /// less is legal (the tests use it to drive the ladder without waiting).
    #[must_use]
    pub fn ceiling_reached(&self, elapsed: Duration) -> bool {
        let max = self.settings.max_span_secs;
        max > 0 && elapsed.as_secs() >= max
    }

    /// Decide the next step for a failure at `attempt` (0-based: the first
    /// failure is attempt 0) that happened after `elapsed` of retrying.
    ///
    /// The small span covers attempts `0..small_span_retries`; from there the
    /// large span applies, spaced by `large_span_secs`. The first large-span
    /// attempt waits the WHOLE span, because the small span has just been
    /// spent at zero delay — the wait is what makes it a different span.
    ///
    /// `retry_after` is the provider's 429 hint; it replaces the flat large
    /// span when present (see the module docs).
    #[must_use]
    pub fn next_delay(
        &self,
        attempt: u32,
        elapsed: Duration,
        retry_after: Option<Duration>,
    ) -> RetryDecision {
        // The ceiling is checked FIRST and unconditionally: a ladder that
        // overshoots the operator's ceiling because it was mid-sleep is a
        // ladder that does not honour the one knob they set.
        if self.ceiling_reached(elapsed) {
            return RetryDecision::Stop { elapsed };
        }
        if attempt < self.settings.small_span_retries {
            // The small span: immediate, unannounced. A provider that blipped
            // does not need a heartbeat telling the operator about 0 seconds.
            return RetryDecision::Retry {
                delay: Duration::ZERO,
                announced: false,
            };
        }
        // The large span. The provider's own hint wins over our guess.
        let delay = match retry_after {
            Some(hint) if !hint.is_zero() => hint,
            _ => Duration::from_secs(self.settings.large_span_secs.max(1)),
        };
        RetryDecision::Retry {
            delay,
            announced: self.settings.announce_large_span,
        }
    }

    /// The delay the ladder would use for `attempt` at `elapsed`, without the
    /// announce flag. A convenience for a caller that only sleeps.
    #[must_use]
    pub fn delay_for(
        &self,
        attempt: u32,
        elapsed: Duration,
        retry_after: Option<Duration>,
    ) -> Duration {
        match self.next_delay(attempt, elapsed, retry_after) {
            RetryDecision::Retry { delay, .. } => delay,
            // A stopped ladder has no delay; the caller must not sleep.
            RetryDecision::Stop { .. } => Duration::ZERO,
        }
    }

    /// Whether a large-span retry is due at `elapsed`, given when the last
    /// large-span attempt happened.
    ///
    /// Kept separate from [`Self::next_delay`] because the cron seam drives
    /// the span from a persisted `last_attempt` rather than from a step count:
    /// a daemon that restarts has forgotten the attempts and only remembers
    /// when it last tried.
    #[must_use]
    pub fn large_span_due(&self, last_attempt: Instant, now: Instant) -> bool {
        let span = Duration::from_secs(self.settings.large_span_secs.max(1));
        now.duration_since(last_attempt) >= span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(small: u32, large: u64, max: u64) -> RetryPolicy {
        RetryPolicy::new(RetrySettings {
            small_span_retries: small,
            large_span_secs: large,
            max_span_secs: max,
            announce_large_span: true,
        })
    }

    #[test]
    fn the_small_span_retries_immediately_and_unannounced() {
        let p = policy(3, 300, 0);
        for attempt in 0..3 {
            assert_eq!(
                p.next_delay(attempt, Duration::ZERO, None),
                RetryDecision::Retry {
                    delay: Duration::ZERO,
                    announced: false,
                },
                "attempt {attempt} must be an immediate, unannounced re-issue"
            );
        }
    }

    #[test]
    fn the_large_span_starts_after_the_small_span_is_spent() {
        let p = policy(3, 300, 0);
        // Attempt 3 is the first one past the small span.
        assert_eq!(
            p.next_delay(3, Duration::ZERO, None),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: true,
            }
        );
        // And every attempt after it.
        assert_eq!(
            p.next_delay(4, Duration::ZERO, None),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: true,
            }
        );
    }

    #[test]
    fn the_large_span_is_infinite_by_default() {
        let p = policy(3, 300, 0);
        // An hour of retrying is nowhere near any ceiling: max_span_secs = 0
        // is documented INFINITE, not a clamp to some default.
        assert_eq!(
            p.next_delay(99, Duration::from_secs(3600), None),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: true,
            }
        );
    }

    #[test]
    fn the_opt_in_ceiling_stops_the_ladder_and_names_the_elapsed_span() {
        let p = policy(3, 300, 600);
        assert_eq!(
            p.next_delay(4, Duration::from_secs(599), None),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: true,
            }
        );
        assert_eq!(
            p.next_delay(4, Duration::from_secs(600), None),
            RetryDecision::Stop {
                elapsed: Duration::from_secs(600)
            }
        );
        // The ceiling is checked even inside the SMALL span: an operator who
        // set a 1 s ceiling wants a 1 s ladder, not a 1 s ceiling that only
        // applies to the large span.
        assert_eq!(
            p.next_delay(0, Duration::from_secs(600), None),
            RetryDecision::Stop {
                elapsed: Duration::from_secs(600)
            }
        );
    }

    #[test]
    fn a_zero_large_span_is_clamped_to_one_second_not_a_busy_loop() {
        // The policy must never produce a zero-length sleep for the large
        // span: that is a busy loop against a dead provider, strictly worse
        // than the hang the policy exists to remove.
        let p = policy(0, 0, 0);
        assert_eq!(
            p.next_delay(0, Duration::ZERO, None),
            RetryDecision::Retry {
                delay: Duration::from_secs(1),
                announced: true,
            }
        );
    }

    #[test]
    fn a_zero_small_span_goes_straight_to_the_large_span() {
        let p = policy(0, 300, 0);
        assert_eq!(
            p.next_delay(0, Duration::ZERO, None),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: true,
            }
        );
    }

    #[test]
    fn the_providers_retry_after_wins_over_the_flat_span() {
        let p = policy(0, 300, 0);
        assert_eq!(
            p.next_delay(0, Duration::ZERO, Some(Duration::from_secs(2))),
            RetryDecision::Retry {
                delay: Duration::from_secs(2),
                announced: true,
            }
        );
    }

    #[test]
    fn a_zero_retry_after_is_ignored_in_favour_of_the_flat_span() {
        // A 429 with a zero retry-after is a provider saying "now", which is
        // not a delay hint; treating it as 0 would busy-loop.
        let p = policy(0, 300, 0);
        assert_eq!(
            p.next_delay(0, Duration::ZERO, Some(Duration::ZERO)),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: true,
            }
        );
    }

    #[test]
    fn the_delay_helper_agrees_with_the_decision() {
        let p = policy(1, 60, 0);
        assert_eq!(
            p.delay_for(0, Duration::ZERO, None),
            p.next_delay(0, Duration::ZERO, None)
                .to_retry_delay()
                .unwrap()
        );
    }

    #[test]
    fn a_stopped_ladder_has_no_delay_to_sleep() {
        let p = policy(0, 60, 30);
        assert_eq!(
            p.delay_for(0, Duration::from_secs(30), None),
            Duration::ZERO
        );
    }

    #[test]
    fn announce_can_be_turned_off() {
        let p = RetryPolicy::new(RetrySettings {
            small_span_retries: 0,
            large_span_secs: 300,
            max_span_secs: 0,
            announce_large_span: false,
        });
        assert_eq!(
            p.next_delay(0, Duration::ZERO, None),
            RetryDecision::Retry {
                delay: Duration::from_secs(300),
                announced: false,
            }
        );
    }

    #[test]
    fn the_large_span_is_due_after_the_span_not_before() {
        let p = policy(0, 300, 0);
        let start = Instant::now();
        assert!(!p.large_span_due(start, start + Duration::from_secs(299)));
        assert!(p.large_span_due(start, start + Duration::from_secs(300)));
    }
}
