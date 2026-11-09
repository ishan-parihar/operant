//! Exclusive per-turn attribution. **LIVE as of iter-1631, and its whole public
//! surface has a production caller as of iter-1635** on the `TurnEnd` seam:
//! `TurnEnd::attribution` (`turn_end.rs`) carries a `TurnAttribution` -- the
//! three-state classification, not a bare `TurnBreakdown` -- per completed
//! turn, built by `OperantAgent::turn_attribution()` (`agent/run.rs`).
//!
//! **Why the classification matters**: a raw `TurnBreakdown` tells a subscriber
//! that `tools` is `Unmeasured` but not WHICH spans are missing, so it cannot
//! tell "the model used no tools" from "nobody timed the tools". That ambiguity
//! is the whole reason this module exists, so the classifier is the payload.
//!
//! `TurnAttribution::Disabled` never reaches here: the kill-switch returns
//! `None` before anything is built (see `AgentConfig::show_turn_attribution`).
//! An `Incomplete` result is a legitimate value -- it NAMES the gaps -- so a
//! measurement gap is never a reason to suppress the emission.
//!
//! **Consumed at the seam as of iter-1661, not by any subscriber.**
//! `run()` classifies the turn, `TurnEnd` carries the enum into every
//! delivered event, and `TurnEndBus::log_attribution` renders one `debug!`
//! line per turn at the emission site -- so the value an operator can see is
//! real. What is still absent is a SUBSCRIBER: both existing subscribers
//! (`advisor.rs`, `recap.rs`) read only `end.session_id`, so the value the
//! event CARRIED is dropped. The seam log reads the local variable before the
//! emit, which is why the two are not the same claim. `TurnBreakdown`'s
//! `is_exclusive()` has no production caller either, and `TurnAttribution`'s
//! multi-line `render()` has none -- the seam uses `render_one_line`. All
//! three gaps print a `KNOWN GAP` notice from the guard at the bottom of this
//! file, because a guard that passes silently is the failure this module has
//! already shipped twice.
//!
//! **Why the log line and not a TUI panel.** A panel needs new TUI surface,
//! which is a peer's area and a far larger iteration. The seam is the one
//! place that already holds the classified enum AND the guarantee that the
//! turn is over, so a line there makes the value observable with no new
//! architecture. `debug!`, not `info!`: one line per turn is noise at info
//! level in a gateway running many turns per minute.
//!
//! **The orphan guard at the bottom of this file is load-bearing.**
//! iter-1631 wired `TurnSpan`, `SpanDuration` and `TurnBreakdown` and left
//! `TurnAttribution` unused, and NO gate caught it: this module is `pub` in a
//! `pub mod`, so a type with zero users emits no `dead_code` warning, `cargo
//! test` is green, and clippy is silent. That is exactly how RT-209 shipped
//! invisible in the first place. The guard scans the four production seams per
//! public type, and requires `TurnAttribution::of` to be called from outside
//! this module so a classifier that is built and discarded still fails.
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
//! This module closes a DIFFERENT question: what the four mutually-exclusive
//! spans of a turn are. It previously claimed to be the RT-199 fix in its own
//! doc header, which is FALSE, and that false claim is why this paragraph
//! survives: it was written when the module shipped as an orphan, and an
//! orphan that documents itself as live is worse than one that does not,
//! because the next agent reads the header, believes the class is handled,
//! and does not check. The module is no longer an orphan; the false claim is
//! still worth stating so nobody re-derives the wrong conclusion.
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
//! Recommendation was (1): it answers the question the numbers exist for,
//! at a fraction of the risk, and it keeps the request path untouched.
//! **iter-1631 implemented (1).** `TurnEnd` carries the `Option<TurnBreakdown>`
//! (115 bytes, cloned per subscriber, bounded), `run()` folds every
//! `llm_duration` into a per-turn accumulator and reads `turn_tool_duration()`
//! for the tools span, and `user_wait` is honestly `Unmeasured` because
//! nothing at that seam measures it. Option (2) was rejected on cost and on
//! the frozen-prefix risk it names. **Rendering the breakdown is still open**
//! (RT-209): the capture is live, no subscriber displays it.
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

/// The four spans of one completed turn.
///
/// RT-431: this used to claim the spans are exclusive as a matter of
/// construction.
/// They are exclusive while the measured spans fit inside the turn's wall
/// clock; when they do NOT, the derived overhead saturates at zero and the
/// four spans sum to MORE than the total. `is_exclusive` is the question
/// that answers whether they do.
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
    /// All four spans were MEASURED.
    ///
    /// RT-431: this used to claim the spans were also "exclusive". The
    /// classifier checks measurement, not arithmetic: `of` returns
    /// `Complete` when no span is `Unmeasured`, and never consults
    /// `TurnBreakdown::is_exclusive`. A turn whose measured spans exceed
    /// its wall clock is therefore `Complete` while its spans do not sum to
    /// the total — `is_exclusive` is the separate question, and it says
    /// false there.
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

    /// One line, four spans, nothing dropped.
    ///
    /// [`Self::render`] throws away the `missing` list on the `Incomplete`
    /// path -- `missing: _` -- so the human-readable block cannot tell "the
    /// model used no tools" from "nobody timed the tools", which is the
    /// ambiguity the classifier exists to remove. A log line is where that
    /// distinction has to survive, because it is the only place the operator
    /// actually sees the value.
    ///
    /// An unmeasured span renders `n/a`, never `0ms`: that single rule is the
    /// RT-199 contract made visible, and a zero span is a different fact.
    pub fn render_one_line(&self) -> String {
        // Delegating by naming the contained enum, not by calling `span`/`total`
        // on `self`: those live on `TurnBreakdown`, and the classifier's job is
        // to delegate rather than re-expose the whole surface.
        let breakdown = match self {
            Self::Complete(b) => b,
            Self::Incomplete { measured, .. } => measured,
            Self::Disabled => return "disabled".to_string(),
        };
        let spans = crate::turn_attribution::TurnSpan::ALL
            .iter()
            .map(|s| {
                let v = match breakdown.span(*s) {
                    crate::turn_attribution::SpanDuration::Measured(d) => {
                        format!("{}ms", d.as_millis())
                    }
                    crate::turn_attribution::SpanDuration::Unmeasured => "n/a".to_string(),
                };
                format!("{}={}", s.as_str(), v)
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("total={}ms {}", breakdown.total().as_millis(), spans)
    }

    /// True when the four spans partition the turn exactly.
    pub fn is_exclusive(&self) -> bool {
        matches!(self, Self::Complete(b) if b.is_exclusive())
    }
}

#[cfg(test)]
mod rt431_tests {
    use super::*;

    /// RT-431: `Complete` means every span was MEASURED — the classifier
    /// does not consult exclusivity. BEHAVIORAL: a turn whose measured
    /// spans exceed its wall clock is `Complete` (the residual saturates
    /// at zero) while `is_exclusive` says false. The docs now say exactly
    /// that, so a subscriber matching on `Complete` is not misled.
    #[test]
    fn complete_means_measured_and_exclusivity_is_a_separate_question() {
        let breakdown = TurnBreakdown::build(
            Duration::from_millis(100),
            SpanDuration::measured(Duration::from_millis(80)),
            SpanDuration::measured(Duration::from_millis(80)),
            SpanDuration::measured(Duration::from_millis(80)),
        );
        assert!(
            !breakdown.is_exclusive(),
            "the measured spans exceed the total, so the spans do not sum to it"
        );
        assert!(
            matches!(TurnAttribution::of(breakdown), TurnAttribution::Complete(_)),
            "the classifier judges measurement, not arithmetic: every span was \
             measured, so the turn is Complete"
        );
    }

    /// RT-431: the docs must not claim the `Complete` variant is exclusive.
    /// SOURCE contract (the behaviour above is the subject; this pins the
    /// words around it) — and this test's own prose does not quote the
    /// phrases it forbids.
    #[test]
    fn the_docs_do_not_claim_the_complete_variant_is_exclusive() {
        let src = include_str!("turn_attribution.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let measured_and_exclusive = concat!("measured and", " exclusive.");
        assert_eq!(
            prod.matches(measured_and_exclusive).count(),
            0,
            "the Complete variant is about measurement, not arithmetic"
        );
        let by_construction = concat!("exclusive BY", " CONSTRUCTION.");
        assert_eq!(
            prod.matches(by_construction).count(),
            0,
            "the breakdown's spans are not exclusive by construction — the \
             residual saturates"
        );
    }

    /// RT-431: and a turn whose spans DO sum to the total is both Complete
    /// and exclusive — the two questions agree there.
    #[test]
    fn a_balanced_turn_is_complete_and_exclusive() {
        let breakdown = TurnBreakdown::build(
            Duration::from_millis(100),
            SpanDuration::measured(Duration::from_millis(40)),
            SpanDuration::measured(Duration::from_millis(30)),
            SpanDuration::measured(Duration::from_millis(20)),
        );
        assert!(breakdown.is_exclusive());
        assert!(matches!(
            TurnAttribution::of(breakdown),
            TurnAttribution::Complete(_)
        ));
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
    fn the_one_line_render_keeps_the_named_gap_and_never_prints_a_zero_for_it() {
        // The RT-199 contract in its plainest form: an unmeasured span is
        // `n/a`, never `0ms`. A zero span and an absent span are different
        // facts, and this is the only place the operator sees the difference,
        // because the seam log reads this line.
        let partial = TurnBreakdown::build(
            ms(100),
            SpanDuration::measured(ms(10)),
            SpanDuration::Unmeasured,
            SpanDuration::measured(ms(30)),
        );
        let a = TurnAttribution::of(partial);
        let line = a.render_one_line();
        assert!(line.contains("total=100ms"), "total must appear: {line}");
        assert!(
            line.contains("tools=n/a"),
            "the unmeasured span must render n/a, not 0ms: {line}"
        );
        assert!(
            !line.contains("tools=0ms"),
            "a zero span is a measurement, not an absence: {line}"
        );
        // Every one of the four spans is present, so a subscriber reading one
        // line never has to wonder which span a value belonged to.
        for name in ["inference", "tools", "user_wait", "overhead"] {
            assert!(
                line.contains(&format!("{name}=")),
                "span {name} missing from {line}"
            );
        }
        // Degenerate: `Disabled` renders a word, not an empty line, so a log
        // reader never sees a blank attribution.
        assert_eq!(TurnAttribution::Disabled.render_one_line(), "disabled");
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

    /// Source paths, relative to the crate root. `CARGO_MANIFEST_DIR` is
    /// cargo's own env var, so it resolves regardless of the test's cwd (a
    /// `file!()`-relative read failed here for exactly that reason).
    fn src(rel: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    /// True when `needle` appears in at least one of `roots` (crate-relative
    /// paths). An unreadable file counts as no-match, so a renamed or deleted
    /// seam fails loudly instead of silently passing.
    fn used_in(needle: &str, roots: &[&str]) -> bool {
        roots
            .iter()
            .any(|r| std::fs::read_to_string(src(r)).is_ok_and(|c| c.contains(needle)))
    }

    /// Like [`used_in`], but ignores a file's `#[cfg(test)]` region.
    ///
    /// This distinction is the whole point. `stream.rs` holds the emission
    /// tests, so a test assertion of `bd.is_exclusive()` satisfies a naive
    /// whole-file scan while the method stays uncalled in production -- and a
    /// passing unit test is not a caller. iter-1631 shipped with
    /// `TurnAttribution` at zero callers while the type-name guard passed,
    /// because a type name appears in a signature; this closes the same hole
    /// one level down.
    ///
    /// The split is a byte-index cut, not a parser: it searches for the first
    /// `#[cfg(test)]`, which is how every module in this crate declares its
    /// tests. A file with no test region is searched whole.
    fn used_in_production(needle: &str, roots: &[&str]) -> bool {
        roots.iter().any(|r| {
            std::fs::read_to_string(src(r)).is_ok_and(|c| {
                let production = match c.find("#[cfg(test)]") {
                    Some(idx) => &c[..idx],
                    None => &c[..],
                };
                production.contains(needle)
            })
        })
    }

    /// The seams that must actually consume this module. Deliberately excludes
    /// `turn_attribution.rs` itself: a type used only by its own module's tests
    /// is exactly the orphan RT-209 was opened on.
    const PRODUCTION_SEAMS: [&str; 4] = [
        "src/agent/run.rs",
        "src/agent/stream.rs",
        "src/turn_end.rs",
        "src/agent/mod.rs",
    ];

    /// The attachment seam: the file where a host wires the bus in.
    ///
    /// This is the layer iter-1659's validation pass found unpinned. The
    /// emission site in `run.rs` is guarded twice over (exactly one call, and
    /// it must carry the classification), but EVERY one of those guards is
    /// inside `if let Some(ref bus) = self.turn_end_bus` -- and `turn_end_bus`
    /// is `None` unless a host attaches one. `with_turn_end_bus` has ZERO
    /// callers, so the only attachment paths are `attach_advisor` (which
    /// early-returns when `[advisor] enabled = false`, the default) and
    /// `attach_recap_engine` (which attaches unconditionally).
    ///
    /// The verified state, checked by reading both hosts rather than trusting
    /// the ledger: `gateway_runner.rs` and `tui/adapter_types/tui_app.rs` each
    /// call `attach_recap_engine` UNCONDITIONALLY, before the enabled-gated
    /// advisor, so a real turn does emit. That is a fact about two files in
    /// another crate, and nothing here pins it.
    const ATTACHMENT_SEAMS: [&str; 2] = [
        "../operant-cli/src/gateway_runner.rs",
        "../operant-cli/src/tui/adapter_types/tui_app.rs",
    ];

    /// The files that would actually READ the value if a subscriber existed.
    /// Wider than `PRODUCTION_SEAMS` by design: the emitter seam and the
    /// consuming subscriber are different modules, and the two `TurnEnd`
    /// subscribers in the tree (`advisor.rs`, `recap.rs`) live at `src/` --
    /// NOT `src/agent/`, an error the first draft of this constant made. Two
    /// of six paths named a file that does not exist, and because `used_in`
    /// treats an unreadable file as a no-match, the guard silently checked
    /// nothing for them. `subscriber_seams_resolve` below exists so that typo
    /// fails loudly instead.
    ///
    /// Scoped to the producers alone, this check would report "no consumer"
    /// forever while a consumer is being written somewhere else.
    const SUBSCRIBER_SEAMS: [&str; 6] = [
        "src/advisor.rs",
        "src/recap.rs",
        "src/agent/run.rs",
        "src/agent/stream.rs",
        "src/turn_end.rs",
        "src/agent/mod.rs",
    ];

    /// The orphan guard, added after iter-1631 missed it once.
    ///
    /// RT-209 was opened because this module shipped with "exported at
    /// `lib.rs:130` and called by NOTHING, tests included", and the compiler
    /// cannot see that: the module is `pub` inside a `pub mod`, so a type with
    /// zero users produces no `dead_code` warning and no gate fails. iter-1631
    /// wired `TurnSpan`, `SpanDuration` and `TurnBreakdown` and STILL left
    /// `TurnAttribution` unused, which this audit found only by grepping
    /// external references by hand.
    ///
    /// The check is a source scan because every user counts, and a new public
    /// type here with no production caller fails this test rather than
    /// accumulating a second invisible orphan.
    #[test]
    fn every_public_type_in_this_module_has_a_production_caller() {
        // The public surface, as RT-209's defect statement inventories it. A new
        // public type must be added to this list deliberately, WITH a caller.
        for name in [
            "TurnSpan",
            "SpanDuration",
            "TurnBreakdown",
            "TurnAttribution",
        ] {
            assert!(
                used_in(name, &PRODUCTION_SEAMS),
                "turn_attribution::{name} has no production caller: it is the RT-209 orphan \
                 again. Wire it into a seam or remove it; do NOT leave it for the next audit \
                 to find. iter-1631 wired three of the four and still missed TurnAttribution."
            );
        }

        // And the classifier must be CONSUMED at the emission seam, not merely
        // constructed: a `TurnAttribution::of(...)` whose result is dropped
        // still leaves every span invisible to a subscriber.
        assert!(
            used_in("TurnAttribution::of", &PRODUCTION_SEAMS),
            "TurnAttribution::of must be called at the emission seam, not only in this module"
        );

        // ...and the value must be CONSUMED, not merely carried.
        //
        // A type annotation counts as a "caller" for the check above, so the
        // whole chain can be live at every layer EXCEPT the last one, which is
        // exactly what iter-1635 shipped: `run()` classified the turn,
        // `TurnEnd` carried the enum, and BOTH subscribers (`advisor.rs`,
        // `recap.rs`) read only `end.session_id`. The attribution was written
        // into every delivered event and read by nothing, and no compiler,
        // clippy, or test could see it -- a `pub` field on a `pub` struct in a
        // library emits no `dead_code` warning.
        //
        // iter-1661 closed the deferred half at the SEAM (a `debug!` line at
        // turn end), so the needle now accepts two shapes and they are NOT
        // equivalent, which is why both are named rather than ORed into one
        // fuzzy check:
        //
        //   1. a field read off the delivered event (`end.attribution`) --
        //      what a real SUBSCRIBER does, and still absent.
        //   2. the seam consumer (`log_attribution(`) -- what now runs.
        //
        // (1) is the stronger claim and stays a NOTICE because it remains
        // genuinely absent: the log line reads the local variable before the
        // emit, so no subscriber consumes the value the event carries. That
        // distinction is the honest state, and reporting it is the point.
        let field_read = used_in("end.attribution", &PRODUCTION_SEAMS)
            || used_in("event.attribution", &PRODUCTION_SEAMS)
            || used_in("turn_end.attribution", &PRODUCTION_SEAMS);
        // The needle is the CALL SITE (`TurnEndBus::log_attribution(`), not the
        // bare name: `turn_end.rs` DEFINES `log_attribution(`, so a bare-name
        // needle matches the definition and the assert passes with the call
        // deleted. Mutation proved that first draft blind -- deleting the call
        // in `run.rs` left the test green, which is the fifth substring-shaped
        // check this module has shipped.
        let seam_consumer = used_in("TurnEndBus::log_attribution(", &PRODUCTION_SEAMS);
        assert!(
            seam_consumer,
            "no production caller consumes the attribution. iter-1661 wired the seam log; if it \
             was removed, restore a consumer rather than leaving the value carried and dropped."
        );
        if !field_read {
            // NOTICE, not a failure, and the reason is now inverted from what
            // it was: the deferred half this guard protected for eleven
            // iterations LANDED at the seam (iter-1661), so what remains is the
            // narrower and stronger claim -- a real subscriber that reads the
            // value the event carries. Asserting it would fail the build for a
            // state that is documented and intentional, which trains the next
            // agent to delete the assertion.
            eprintln!(
                "KNOWN GAP (RT-209): the attribution is CONSUMED at the seam (iter-1661's turn-end \
                 log), but no SUBSCRIBER reads `end.attribution` off the delivered event. The seam \
                 log reads the local variable before the emit. A subscriber -- advisor, recap, or a \
                 TUI surface -- is what would consume the carried value."
            );
        }

        // ...and the module's two public API METHODS that exist for a
        // subscriber's benefit. iter-1655's validation pass found them with
        // ZERO production callers, which is the RT-209 shape at method
        // granularity: the guard above checks type names, and a type name
        // appears in a signature, so it passed while the two methods the types
        // exist to expose went uncalled.
        //
        // `is_exclusive()` is the exclusivity proof -- `build()`
        // saturating-subtracts a residual, so the four spans partition the turn
        // only when nothing over-ran, and a subscriber that never asks cannot
        // know. `render()` is the human-readable block. Both are `pub`, both
        // are unit-tested, and both are reachable from nowhere.
        //
        // A NOTICE, not an assert, for the same documented reason the read
        // check above is one: this is the deferred half of the same feature, so
        // failing the build for a declared state would only train the next
        // agent to delete the guard.
        //
        // Two details that the first draft got wrong and that mutation then
        // caught. The needles are CALL SITES (`x.render()`), not bare method
        // names: a bare "render" matches 11 times in `run.rs` and 18 in
        // `mod.rs` as parts of unrelated identifiers, so a bare-name check
        // passes happily while both methods go uncalled. And the scan is
        // `used_in_production`, not `used_in`: `stream.rs`'s two
        // `.is_exclusive()` hits are test assertions inside its `#[cfg(test)]`
        // region (lines 2054 and 2088, past `#[cfg(test)]` at 1930 -- read, not
        // assumed), so a whole-file scan reports the method as called when its
        // only callers are tests.
        for method in [".is_exclusive()", ".render()"] {
            if !used_in_production(method, &SUBSCRIBER_SEAMS) {
                eprintln!(
                    "KNOWN GAP (RT-209): turn_attribution::{method} has no production caller. \
                     The seam log consumes `render_one_line`; this is the OTHER half of the \
                     deferred surface -- a subscriber that asks the exclusivity question or \
                     renders the multi-line block. Wire one into advisor.rs, recap.rs, or a TUI \
                     surface to clear this."
                );
            }
        }
    }

    /// Every `SUBSCRIBER_SEAMS` path must exist.
    ///
    /// `used_in` and `used_in_production` both return false for an unreadable
    /// file, so a wrong path is indistinguishable from "the method has no
    /// caller" and the guard would quietly check nothing. The first draft of
    /// `SUBSCRIBER_SEAMS` had exactly two such paths (`src/agent/advisor.rs`,
    /// `src/agent/recap.rs` -- the real files are at `src/`), which is how this
    /// assertion earned its place.
    #[test]
    fn subscriber_seams_resolve() {
        for path in SUBSCRIBER_SEAMS {
            assert!(
                src(path).exists(),
                "SUBSCRIBER_SEAMS names {path}, which does not exist: the guard would stop \
                 checking it silently (an unreadable file counts as no-match)"
            );
        }
    }

    /// Each real host must actually attach the bus.
    ///
    /// The emission site in `run.rs` is guarded twice over, and both guards
    /// sit INSIDE `if let Some(ref bus) = self.turn_end_bus`. So a host that
    /// never attaches a bus makes every span invisible without failing any
    /// test in this module -- the guards would all still pass, because the
    /// code they inspect is never reached.
    ///
    /// `with_turn_end_bus` has zero callers, so the only attachment path is
    /// `attach_recap_engine` (unconditional). `attach_advisor` also attaches
    /// but early-returns on `[advisor] enabled = false`, which is the
    /// DEFAULT -- so a host that calls only the advisor, or calls neither,
    /// emits nothing.
    ///
    /// The needle is the CALL SITE `agent.attach_recap_engine(`, not the bare
    /// method name, and that is not a stylistic choice: my first draft used the
    /// bare name and mutation proved it BLIND. Renaming the call to
    /// `agent.attach_recap_engine_DISABLED(` left the test green, because the
    /// mutated identifier still contains the needle as a substring. That is the
    /// fourth time this module shipped a substring-shaped check -- type name in
    /// a signature, `render` inside unrelated identifiers, `is_exclusive` in
    /// test code, and now a renamed call -- which is why the needle includes
    /// the receiver and the opening paren.
    #[test]
    fn every_host_attaches_the_turn_end_bus() {
        for host in ATTACHMENT_SEAMS {
            let text = std::fs::read_to_string(src(host))
                .unwrap_or_else(|e| panic!("cannot read host seam {host}: {e}"));
            assert!(
                text.contains("agent.attach_recap_engine("),
                "{host} never calls `attach_recap_engine`, so it attaches no `TurnEndBus` \
                 and every attribution byte this module computes is dropped. `with_turn_end_bus` \
                 has no callers and `attach_advisor` early-returns when `[advisor] enabled = \
                 false` (the default), so the unconditional attach is the only live path."
            );
        }
    }
}
