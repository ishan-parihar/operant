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

/// The "give up on retries" sentinel: when an empty turn has been
/// nudged `max` times already, callers fall through to their normal
/// end-of-turn path. Returns a structured `Result` so the call site
/// can log + return a friendly error instead of silently emitting
/// nothing (R4-1 root cause).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyExhausted {
    pub attempts: usize,
    pub max: usize,
}

impl EmptyResponseCounter {
    /// Build an `EmptyExhausted` sentinel for the caller to surface.
    /// Pair with `should_retry` returning `false` — when both fire, the
    /// turn is empty + retries are gone = time to fail closed.
    pub fn exhausted(&self) -> Option<EmptyExhausted> {
        if self.count >= self.max {
            Some(EmptyExhausted {
                attempts: self.count,
                max: self.max,
            })
        } else {
            None
        }
    }
}

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
        assert!(c.exhausted().is_some());
    }

    #[test]
    fn cap_is_clamped_to_module_constant() {
        // Caller asked for 99 — should be capped to 3.
        let c = EmptyResponseCounter::new(99);
        assert_eq!(c.max, EMPTY_RESPONSE_MAX_RETRIES);
    }
}
