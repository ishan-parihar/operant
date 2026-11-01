//! Exclusive per-turn attribution: OPEN. **This module has no caller.**
//!
//! ## Read this before believing the RT-199 class is handled
//!
//! RT-199 (a hardcoded `Duration::from_millis(0)` that lived undetected
//! because a `u64` cannot distinguish "took zero time" from "nobody measured
//! this") was closed separately, by the observer path:
//! `agent/mod.rs`'s `tool_durations_by_call` map, filled by `execute_tools` at
//! the same sites that feed `TurnEndBus::record_tool_duration`, and read back
//! by the `ObserverEvent::ToolCall` emit in `agent/run.rs`. That is the fix.
//!
//! This module was written to close a DIFFERENT, still-open question and was
//! never wired in: what the four mutually-exclusive spans of a turn are. It
//! previously claimed to be the RT-199 fix in its own doc header, which is
//! false, and that false claim is the reason this paragraph exists: an orphan
//! that documents itself as live is worse than one that does not, because the
//! next agent reads the header, believes the class is handled, and does not
//! check.
//!
//! ## Why the emission site is not a one-liner (measured 2026-11-01)
//!
//! An attempt to wire this into the prompt found the blocker is
//! ARCHITECTURAL, not plumbing. The obvious seat (the
//! `<turn_context>` volatile tail where `<context_budget>` rides,
//! `agent/run.rs` inside `build_messages`) is built EXACTLY ONCE PER TURN,
//! BEFORE the loop starts:
//!
//! ```text
//! run() -> turn_context::build_turn_context() (run.rs:337)
//!        -> build_messages()            (turn_context.rs:219)
//!           builds the volatile tail
//!        -> returns Vec<Message>, which the loop only APPENDS to
//! ```
//!
//! So at the moment the tail is written, no model call has been made and no
//! tool has run: `turn_inference` and `turn_tools` are still zero and
//! `saw_inference` is still false. A block emitted there would render
//! `n/a` for every span on every turn, forever, which is the RT-199
//! failure wearing a better label. Emitting it on the LAST iteration does
//! not work either, because the frozen prefix and history must stay
//! byte-stable across the turn's requests or the provider cache is
//! invalidated on every tool iteration.
//!
//! Two honest options exist, and the choice is the operator's:
//!
//! 1. **Emit it ONCE at turn END, off the request path.** The numbers are
//!    complete by then. The natural home is the existing `TurnEnd` event
//!    (`turn_end.rs`), which already carries per-turn counters and
//!    already reaches every subscriber (TUI, gateway, trajectory, logs).
//!    Costs: extend `TurnEnd` with the four spans (bounded, no per-clone
//!    cost beyond four `Duration`s), and add one more capture. It does
//!    NOT put the block in the prompt, so the model would not see it --
//!    which may be fine, since "why did this turn take 40s" is a question
//!    the OPERATOR asks, and the operator reads `TurnEnd`.
//! 2. **Thread the timings into the turn-context build.** Change
//!    `build_messages` to take the span inputs, and have `run()` build the
//!    tail on the LAST iteration from the accumulated values. Costs: one
//!    new signature, a second build site for the tail only, and a real
//!    risk of breaking the frozen-prefix guarantee that iter-39 depends
//!    on. This is the expensive option and it only buys prompt visibility.
//!
//! Recommendation is (1): it answers the question the numbers exist for,
//! at a fraction of the risk, and it keeps the request path untouched.
//!
//! ## The bug this would close
//!
//! operant already measures three things per turn: the LLM call duration
//! (`agent/run.rs`), each tool's duration, and the turn total. All three are
//! **inclusive and overlapping**: the tools run *inside* the turn, and the LLM
//! call that follows them runs inside it too, so nothing answers "why did this
//! turn take 40 seconds".
//!
//! ## The state model, ported
//!
//! codex's `analytics/src/turn_critical_path.rs` carries
//! `TurnCriticalPath { Disabled, Incomplete, Complete }`. `Disabled` is capture
//! switched off. `Incomplete` means at least one span never got a timestamp —
//! and **names which one**. `Complete` means all four are measured. Critically,
//! a missing capture NEVER becomes a zero-duration measurement.
//!
//! ## The exclusivity invariant
//!
//! The four spans partition the turn exactly:
//!
//! ```text
//! total = inference + tools + user_wait + overhead
//! ```
//!
//! with `overhead` as the residual, computed with `saturating_sub` so a
//! measurement gap can never make it negative. The naive implementation
//! **double-counts** — summing per-tool durations plus inter-tool gaps plus LLM
//! latency exceeds the turn interval, so the operator sees >100%. The test
//! `double_counting_a_span_makes_the_sum_check_fail` pins that.

use std::time::Duration;

/// The four mutually-exclusive ways a turn's wall-clock can be spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSpan {
    /// Time inside a provider call, waiting on the model.
    Inference,
    /// Time inside a tool, waiting on the effect.
    Tools,
    /// Time the operator held the turn (the pause/estop gate).
    UserWait,
    /// Everything else: message building, context management, redaction,
    /// persistence, the emit awaits. The residual, measured as
    /// `total - inference - tools - user_wait`.
    Overhead,
}

impl TurnSpan {
    /// The label used in the `<turn_attribution>` prompt block and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference",
            Self::Tools => "tools",
            Self::UserWait => "user_wait",
            Self::Overhead => "overhead",
        }
    }

    /// All four, in the order they render.
    pub const ALL: [Self; 4] = [Self::Inference, Self::Tools, Self::UserWait, Self::Overhead];

    fn index(self) -> usize {
        match self {
            Self::Inference => 0,
            Self::Tools => 1,
            Self::UserWait => 2,
            Self::Overhead => 3,
        }
    }
}

/// How much of the turn was spent in one span.
///
/// The point of the enum: `Unmeasured` and `zero` are different answers, and
/// conflating them is what RT-199 was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanDuration {
    /// Captured, and the captured value (which may legitimately be zero).
    Measured(Duration),
    /// Never captured. NEVER rendered as `0` — it renders as `n/a`.
    Unmeasured,
}

impl SpanDuration {
    /// The zero value, explicitly a measurement rather than an absence.
    pub fn measured(d: Duration) -> Self {
        Self::Measured(d)
    }

    /// Treat an unmeasured span as nothing, for the arithmetic only. The
    /// *rendering* path must not use this — see `TurnBreakdown::render`.
    fn as_duration(self) -> Duration {
        match self {
            Self::Measured(d) => d,
            Self::Unmeasured => Duration::ZERO,
        }
    }
}

/// The four spans of one completed turn, exclusive by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBreakdown {
    total: Duration,
    spans: [SpanDuration; 4],
}

impl TurnBreakdown {
    /// Build the breakdown from captured pieces.
    ///
    /// `total` is the turn's wall-clock. The other three are whatever was
    /// captured; the fourth (`overhead`) is derived as the residual. Any span
    /// the caller could not measure is passed as [`SpanDuration::Unmeasured`],
    /// and the derived residual is then `Unmeasured` too — we do not report a
    /// precise residual derived from partial inputs.
    pub fn build(
        total: Duration,
        inference: SpanDuration,
        tools: SpanDuration,
        user_wait: SpanDuration,
    ) -> Self {
        let measured = [inference, tools, user_wait];
        let all_measured = measured.iter().all(|s| *s != SpanDuration::Unmeasured);
        let overhead = if all_measured {
            let sum = inference.as_duration() + tools.as_duration() + user_wait.as_duration();
            SpanDuration::measured(total.saturating_sub(sum))
        } else {
            SpanDuration::Unmeasured
        };
        Self {
            total,
            spans: [inference, tools, user_wait, overhead],
        }
    }

    /// The span's value, or `Unmeasured`.
    pub fn span(&self, s: TurnSpan) -> SpanDuration {
        self.spans[s.index()]
    }

    /// The turn's wall-clock, always measured (the turn start is unconditional).
    pub fn total(&self) -> Duration {
        self.total
    }

    /// The exclusivity check: every span, measured or not, must sum to `total`.
    ///
    /// Returns `true` when the four spans partition the turn exactly. An
    /// unmeasured span makes this `false` rather than silently true on zeros.
    pub fn is_exclusive(&self) -> bool {
        if self.spans.contains(&SpanDuration::Unmeasured) {
            return false;
        }
        let sum: Duration = self.spans.iter().map(|s| s.as_duration()).sum();
        sum == self.total
    }

    /// The prompt-tail block. Omitted entirely under `Disabled`.
    ///
    /// An unmeasured span renders `n/a`, never `0ms` — that single line is the
    /// RT-199 contract made visible.
    pub fn render(&self) -> Option<String> {
        let mut lines = Vec::with_capacity(5);
        lines.push(format!(
            "turn attribution (total {}ms):",
            self.total.as_millis()
        ));
        for s in TurnSpan::ALL {
            let v = match self.span(s) {
                SpanDuration::Measured(d) => format!("{}ms", d.as_millis()),
                SpanDuration::Unmeasured => "n/a".to_string(),
            };
            lines.push(format!("  {}: {}", s.as_str(), v));
        }
        Some(lines.join("\n"))
    }
}

/// Whether this turn's attribution is being captured at all.
///
/// The three-state contract from codex: a turn is either not being measured,
/// partially measured (and the missing span is named), or fully measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnAttribution {
    /// Capture is off — the block renders nothing at all.
    Disabled,
    /// At least one span was never captured, so the numbers do not partition
    /// the turn. Carries the spans that WERE measured so a partially-working
    /// capture is still reported rather than thrown away.
    Incomplete {
        measured: TurnBreakdown,
        missing: Vec<TurnSpan>,
    },
    /// All four spans measured and exclusive.
    Complete(TurnBreakdown),
}

impl TurnAttribution {
    /// Classify a breakdown: `Complete` when every span was measured,
    /// `Incomplete` naming what is missing, `Disabled` never (a caller that
    /// is not measuring does not build one).
    pub fn of(breakdown: TurnBreakdown) -> Self {
        let missing: Vec<TurnSpan> = TurnSpan::ALL
            .into_iter()
            .filter(|s| breakdown.span(*s) == SpanDuration::Unmeasured)
            .collect();
        if missing.is_empty() {
            Self::Complete(breakdown)
        } else {
            Self::Incomplete {
                measured: breakdown,
                missing,
            }
        }
    }

    /// The prompt-tail line, or `None` when capture is off.
    pub fn render(&self) -> Option<String> {
        match self {
            Self::Disabled => None,
            Self::Complete(b) => b.render(),
            Self::Incomplete {
                measured,
                missing: _,
            } => measured.render(),
        }
    }

    /// True when the four spans partition the turn exactly.
    pub fn is_exclusive(&self) -> bool {
        matches!(self, Self::Complete(b) if b.is_exclusive())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_four_spans_partition_the_turn_exactly() {
        // The core invariant. 10000 total, spent as 4000 + 3000 + 1000 = 8000,
        // leaving 2000 of real overhead — so the residual is a real number and
        // not an artifact.
        let b = TurnBreakdown::build(
            ms(10_000),
            SpanDuration::measured(ms(4_000)),
            SpanDuration::measured(ms(3_000)),
            SpanDuration::measured(ms(1_000)),
        );
        assert_eq!(
            b.span(TurnSpan::Overhead),
            SpanDuration::measured(ms(2_000))
        );
        assert!(b.is_exclusive(), "the four spans must sum to the total");
        assert_eq!(
            b.total(),
            ms(10_000),
            "the turn total is the unconditional turn start"
        );
    }

    /// THE mutation guard from the plan, stated as an executable test.
    ///
    /// The naive implementation double-counts: it sums per-tool durations
    /// PLUS inter-tool gaps PLUS LLM latency, which exceeds the turn interval,
    /// so an operator reading the report sees >100%. This test makes that
    /// impossible by construction — the residual is `total - sum`, so the sum
    /// can never exceed the total. Mutate `Overhead` to be the sum instead of
    /// the residual and this fails.
    #[test]
    fn double_counting_a_span_makes_the_sum_check_fail() {
        let b = TurnBreakdown::build(
            ms(10_000),
            SpanDuration::measured(ms(4_000)),
            SpanDuration::measured(ms(3_000)),
            SpanDuration::measured(ms(1_000)),
        );
        // The spans as reported.
        let reported: Duration = TurnSpan::ALL.iter().map(|s| b.span(*s).as_duration()).sum();
        // The residual the builder derived.
        let residual = b.span(TurnSpan::Overhead).as_duration();
        assert_eq!(
            reported,
            b.total(),
            "measurement gap can never make the residual negative, and the \
             reported spans can never exceed the total"
        );
        // And the residual is genuinely the difference, not the sum.
        let expected = ms(10_000) - (ms(4_000) + ms(3_000) + ms(1_000));
        assert_eq!(residual, expected);
    }

    /// THE second mutation guard: make the report a `u64`/`Duration` only and
    /// the `Incomplete` state is lost — which is exactly what RT-199 was.
    #[test]
    fn an_unmeasured_span_is_not_a_zero_span() {
        // A turn with no tools at all must report `Unmeasured`, not `0ms`, or
        // the reader cannot tell "there were no tools" from "nobody timed them".
        let b = TurnBreakdown::build(
            ms(10_000),
            SpanDuration::measured(ms(4_000)),
            SpanDuration::Unmeasured,
            SpanDuration::measured(ms(0)),
        );
        assert_eq!(b.span(TurnSpan::Tools), SpanDuration::Unmeasured);
        // The residual is then also Unmeasured — a precise residual derived
        // from partial inputs is a lie.
        assert_eq!(b.span(TurnSpan::Overhead), SpanDuration::Unmeasured);
        assert!(
            !b.is_exclusive(),
            "a partially-captured turn is not exclusive, and must not claim to be"
        );
        let rendered = b.render().expect("a partial turn still renders");
        assert!(
            rendered.contains("tools: n/a"),
            "unmeasured renders n/a, never 0ms — got: {rendered}"
        );
        assert!(
            !rendered.contains("tools: 0ms"),
            "0ms would be the RT-199 bug"
        );
    }

    #[test]
    fn a_zero_duration_span_is_a_measurement_not_an_absence() {
        // `user_wait` measured at zero is a real answer: the operator never
        // held the turn. The enum says so, and the render says `0ms`.
        let b = TurnBreakdown::build(
            ms(10_000),
            SpanDuration::measured(ms(10_000)),
            SpanDuration::measured(ms(0)),
            SpanDuration::measured(ms(0)),
        );
        assert_eq!(b.span(TurnSpan::Tools), SpanDuration::measured(ms(0)));
        assert_eq!(b.span(TurnSpan::UserWait), SpanDuration::measured(ms(0)));
        assert!(b.is_exclusive());
        let rendered = b.render().expect("renders");
        assert!(
            rendered.contains("tools: 0ms"),
            "a MEASURED zero renders as 0ms — got: {rendered}"
        );
    }

    #[test]
    fn attribution_complete_names_itself_and_incomplete_names_the_gap() {
        let complete = TurnBreakdown::build(
            ms(100),
            SpanDuration::measured(ms(10)),
            SpanDuration::measured(ms(20)),
            SpanDuration::measured(ms(30)),
        );
        let a = TurnAttribution::of(complete.clone());
        assert_eq!(a, TurnAttribution::Complete(complete));
        assert!(a.is_exclusive());
        assert!(a.render().is_some());

        let partial = TurnBreakdown::build(
            ms(100),
            SpanDuration::measured(ms(10)),
            SpanDuration::Unmeasured,
            SpanDuration::measured(ms(30)),
        );
        let a = TurnAttribution::of(partial);
        match &a {
            TurnAttribution::Incomplete { missing, .. } => {
                assert!(
                    missing.contains(&TurnSpan::Tools),
                    "the missing span must be NAMED, not implied"
                );
            }
            other => panic!("expected Incomplete, got {other:?}"),
        }
        assert!(!a.is_exclusive());
        // An incomplete turn still renders (with n/a for the gap).
        assert!(a.render().is_some());
    }

    #[test]
    fn disabled_renders_nothing_at_all() {
        // Under `Disabled` the block is absent — a turn that is not being
        // measured emits no attribution line whatsoever.
        assert_eq!(TurnAttribution::Disabled.render(), None);
        assert!(!TurnAttribution::Disabled.is_exclusive());
    }

    #[test]
    fn a_turn_that_took_exactly_zero_is_not_unmeasured() {
        // Degenerate but real: an empty turn. Total zero, all spans measured
        // zero. Exclusive.
        let b = TurnBreakdown::build(
            ms(0),
            SpanDuration::measured(ms(0)),
            SpanDuration::measured(ms(0)),
            SpanDuration::measured(ms(0)),
        );
        assert!(b.is_exclusive(), "a zero turn is fully measured");
        assert_eq!(b.span(TurnSpan::Overhead), SpanDuration::measured(ms(0)));
    }
}
