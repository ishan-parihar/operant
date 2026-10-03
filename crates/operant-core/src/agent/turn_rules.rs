//! Plan 006: shared turn-behavior rules consumed by both `operant-core`
//! (OperantAgent in agent/run.rs) and `operant-runtime` (Agent in
//! agent/agent.rs). Before this module, the empty-response retry ladder,
//! the "is this an empty assistant message" decision, and the
//! `max_retries` cap were duplicated in both agents. Every hermes behavior
//! change had to be applied twice — and R23/R24 already had to port the
//! same fix to both. This module makes them share a single decision
//! surface, and a parity test (`agent_parity.rs` in operant-cli) proves
//! both agents reach the same conclusion for the same scripted input.
//!
//! The shared types are deliberately tiny: pure data + pure functions,
//! no agent handle, no provider, no I/O. Callers build the `TurnState`
//! from their own state and call `decide_*` to learn what to do.

/// Hermes parity: the cap on consecutive empty-content nudges before
/// the loop gives up. Both agents used to define this constant
/// separately (`self.config.max_retries` in core, hardcoded 3 in runtime)
/// — the same value, but two different sources = silent-divergence trap.
pub const EMPTY_RESPONSE_MAX_RETRIES: usize = 3;

/// Hermes parity: a finished assistant turn is "empty" when the visible
/// text is blank AND reasoning is absent or blank AND the model emitted
/// no tool calls. Empty turns are nudged (not accepted as the final
/// answer) up to [`EMPTY_RESPONSE_MAX_RETRIES`] times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssistantTurn<'a> {
    /// Visible assistant text (already trimmed in some callers, raw in
    /// others — `is_empty` is the source of truth, trimming is the
    /// caller's job).
    pub final_text: &'a str,
    /// Reasoning text from thinking-mode models, if any.
    pub reasoning: Option<&'a str>,
    /// Whether the turn emitted one or more tool calls.
    pub has_tool_calls: bool,
}

impl<'a> AssistantTurn<'a> {
    /// True iff the turn produced no text, no reasoning, and no tool
    /// calls. Both agents were already doing this same check inline.
    pub fn is_empty(&self) -> bool {
        self.final_text.trim().is_empty()
            && self.reasoning.is_none_or(|r| r.trim().is_empty())
            && !self.has_tool_calls
    }
}

/// Counter + decision for the empty-response retry ladder.
///
/// Constructed at the top of each turn; carried through the inner loop;
/// `decide()` is called after each assistant response. Same shape for
/// both agents (was named `empty_content_retries` in core, `empty_response_retries`
/// in runtime). Renamed to `EmptyResponseCounter` here and used uniformly.
#[derive(Debug, Clone, Copy)]
pub struct EmptyResponseCounter {
    pub count: usize,
    pub max: usize,
}

impl EmptyResponseCounter {
    pub fn new(max: usize) -> Self {
        Self {
            count: 0,
            max: max.min(EMPTY_RESPONSE_MAX_RETRIES),
        }
    }

    /// True when the turn should nudge-and-retry instead of returning.
    /// `turn` is the (possibly empty) assistant response. Caller is
    /// responsible for actually pushing the nudge + continuing the loop.
    pub fn should_retry(&mut self, turn: AssistantTurn<'_>) -> bool {
        if !turn.is_empty() {
            return false;
        }
        if self.count >= self.max {
            return false;
        }
        self.count += 1;
        true
    }

    pub fn remaining(&self) -> usize {
        self.max.saturating_sub(self.count)
    }
}

/// True for output that must not be surfaced as a turn's final answer:
/// empty per `AssistantTurn::is_empty`, or a short whitespace-free
/// artifact (the observed class: `]<]\u{200b}minimax[>[`, 13 chars —
/// reasoning-leak markers, not prose). Deliberate ceiling: a genuine
/// sub-80-char single-word summary is sacrificed; the operator gets the
/// named-reason stopped notice instead, which is strictly more honest.
pub fn is_degenerate_final_text(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || (t.len() < 80 && !t.chars().any(char::is_whitespace))
}

// REMOVED: the `EmptyExhausted` sentinel and `EmptyResponseCounter::exhausted()`.
// It had zero production callers, and its doc promised a behaviour that already
// exists one layer up: `gateway_runner.rs` substitutes a user-facing message
// ("provider returned an empty response after retries ... Reply 'continue'")
// whenever the agent returns empty content. The exhausted terminal shape is
// `Ok(<assistant message with empty body>)`, reported as
// `reason=TextResponse ... response_len=0`. See BUGS.md Round 42 and the module
// header of `tests/loop_recovery_paths.rs`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_text_never_retries() {
        let mut c = EmptyResponseCounter::new(3);
        assert!(!c.should_retry(AssistantTurn {
            final_text: "hello",
            reasoning: None,
            has_tool_calls: false,
        }));
        assert_eq!(c.count, 0);
    }

    #[test]
    fn reasoning_only_counts_as_empty() {
        // Reasoning text alone (no visible text, no tool calls) → not empty
        // — a thinking-mode reply still represents an attempted response,
        // matching both agents' prior behavior.
        let mut c = EmptyResponseCounter::new(3);
        assert!(!c.should_retry(AssistantTurn {
            final_text: "",
            reasoning: Some("thinking aloud"),
            has_tool_calls: false,
        }));
        assert_eq!(c.count, 0);
        // Blank reasoning ("   ") IS empty.
        let mut c2 = EmptyResponseCounter::new(3);
        assert!(c2.should_retry(AssistantTurn {
            final_text: "",
            reasoning: Some("   "),
            has_tool_calls: false,
        }));
        assert_eq!(c2.count, 1);
    }

    #[test]
    fn tool_call_turn_is_never_empty() {
        let mut c = EmptyResponseCounter::new(3);
        assert!(!c.should_retry(AssistantTurn {
            final_text: "",
            reasoning: None,
            has_tool_calls: true,
        }));
    }

    #[test]
    fn caps_at_max_retries() {
        let mut c = EmptyResponseCounter::new(2);
        assert!(c.should_retry(AssistantTurn {
            final_text: "",
            reasoning: None,
            has_tool_calls: false,
        }));
        assert!(c.should_retry(AssistantTurn {
            final_text: "",
            reasoning: None,
            has_tool_calls: false,
        }));
        // Third nudge should be refused.
        assert!(!c.should_retry(AssistantTurn {
            final_text: "",
            reasoning: None,
            has_tool_calls: false,
        }));
        assert_eq!(c.count, 2);
    }

    #[test]
    fn cap_is_clamped_to_module_constant() {
        // Caller asked for 99 — should be capped to 3.
        let c = EmptyResponseCounter::new(99);
        assert_eq!(c.max, EMPTY_RESPONSE_MAX_RETRIES);
    }

    #[test]
    fn empty_text_is_degenerate() {
        assert!(is_degenerate_final_text(""));
        // Whitespace-only trims to empty — same degenerate class.
        assert!(is_degenerate_final_text("   \n\t "));
    }

    #[test]
    fn reasoning_leak_artifact_is_degenerate() {
        // The observed production class: 13 chars, no whitespace. U+200B
        // (zero-width space) is NOT whitespace and is not trimmed, so it
        // must not save the artifact from the gate.
        assert!(is_degenerate_final_text("]<]\u{200b}minimax[>["));
    }

    #[test]
    fn long_summary_survives_the_gate() {
        // A realistic 942-char report must pass — the gate is for
        // artifacts, and length alone clears the ceiling.
        let summary = format!("Progress report: {}.", "x".repeat(924));
        assert_eq!(summary.len(), 942);
        assert!(!is_degenerate_final_text(&summary));
    }

    #[test]
    fn short_multiword_answer_survives_the_gate() {
        // A genuine short answer with real prose structure: sub-80 chars,
        // but it contains whitespace, so it is NOT the artifact class.
        assert!(!is_degenerate_final_text("Done. Two files changed."));
    }
}
