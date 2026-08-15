//! Tool-call loop guardrails — hermes `agent/tool_guardrails.py` +
//! `agent/tool_result_classification.py` parity (R4).
//!
//! The model can degenerate into calling the same tool with the same
//! arguments repeatedly within one turn (a retry storm). Each repeat costs a
//! full LLM round-trip plus the tool's execution; a side-effecting tool
//! (terminal, write_file) can also mutate state repeatedly.
//!
//! This module provides a **pure per-turn controller** — it tracks
//! (tool_name, normalized-args) observations and returns a decision. The
//! runtime (`OperantAgent`) converts decisions into either a warning
//! surfaced to the model feed or a synthetic skip result.
//!
//! Also ports `tool_result_classification.py`'s no-effect/side-effect
//! vocabulary so interruption and repetition policy can treat read-only
//! tools (cheap, safe to re-run) differently from mutating ones.

use std::collections::HashMap;

/// Tools that cannot mutate external state or session state — safe to
/// re-run and safe to discard if interrupted. Mirrors hermes
/// `NO_EFFECT_TOOL_NAMES` (adapted to operant's tool vocabulary).
pub const NO_EFFECT_TOOL_NAMES: &[&str] = &[
    "file_search",
    "file_read",
    "session_search",
    "skill_view",
    "skills_list",
    "web_search",
    "web_fetch",
    "web_extract",
    "vision_analyze",
    "browser_snapshot",
    "browser_get_images",
    "browser_console",
    "datetime",
];

/// True when a tool may mutate external or session state.
pub fn tool_may_have_side_effect(tool_name: &str) -> bool {
    !NO_EFFECT_TOOL_NAMES.contains(&tool_name)
}

/// Repeated identical calls that trip the guardrail per turn.
pub const REPEAT_WARN_THRESHOLD: usize = 3;
/// Side-effecting tools are skipped one repeat earlier (2nd identical call).
pub const REPEAT_SKIP_THRESHOLD_EFFECT: usize = 3;
/// No-effect tools get a warning first, then skip on the 4th identical call.
pub const REPEAT_SKIP_THRESHOLD_NO_EFFECT: usize = 4;

/// Guardrail decision for one observed tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailDecision {
    /// Proceed with execution.
    Allow,
    /// Warn the model (surfaced in the feed) but still execute — used for
    /// repeated no-effect calls below the skip threshold.
    Warn,
    /// Skip execution and return a synthetic result telling the model to
    /// stop repeating this exact call.
    Skip,
}

/// Per-turn tracker of identical tool-call repeats.
///
/// Pure and side-effect free apart from its own counters — directly
/// unit-testable without an agent or network.
#[derive(Debug, Default)]
pub struct ToolGuardrailTracker {
    /// (tool_name, normalized-args) → observed count this turn.
    counts: HashMap<(String, String), usize>,
}

impl ToolGuardrailTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Normalize arguments for identity comparison: all whitespace removed.
    /// Two calls differing only in whitespace (pretty-printed vs compact
    /// JSON) are the same call.
    fn normalize_args(args: &str) -> String {
        args.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Record a tool call and return the guardrail decision.
    pub fn observe(&mut self, tool_name: &str, args: &str) -> GuardrailDecision {
        let key = (tool_name.to_string(), Self::normalize_args(args));
        let count = self.counts.entry(key).or_insert(0);
        *count += 1;
        let count = *count;

        if tool_may_have_side_effect(tool_name) {
            if count >= REPEAT_SKIP_THRESHOLD_EFFECT {
                return GuardrailDecision::Skip;
            }
        } else if count >= REPEAT_SKIP_THRESHOLD_NO_EFFECT {
            return GuardrailDecision::Skip;
        }

        if count >= REPEAT_WARN_THRESHOLD {
            GuardrailDecision::Warn
        } else {
            GuardrailDecision::Allow
        }
    }

    /// Number of times this exact (tool, args) pair has been observed.
    pub fn count_of(&self, tool_name: &str, args: &str) -> usize {
        let key = (tool_name.to_string(), Self::normalize_args(args));
        self.counts.get(&key).copied().unwrap_or(0)
    }

    /// Reset per-turn state (call at the start of each user turn).
    pub fn reset(&mut self) {
        self.counts.clear();
    }
}

/// Build the synthetic skip result text fed back to the model.
pub fn build_skip_message(tool_name: &str, count: usize) -> String {
    format!(
        "[GUARDRAIL] Tool '{tool_name}' was already called with identical arguments \
         {count} times this turn. Skipping this duplicate call — do NOT repeat it. \
         If the previous results were insufficient, change the arguments or use a \
         different tool."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_side_effect_call_skipped_on_third() {
        let mut t = ToolGuardrailTracker::new();
        assert_eq!(
            t.observe("terminal", r#"{"command": "rm -rf x"}"#),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("terminal", r#"{"command": "rm -rf x"}"#),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("terminal", r#"{"command": "rm -rf x"}"#),
            GuardrailDecision::Skip
        );
    }

    #[test]
    fn whitespace_variants_count_as_identical() {
        let mut t = ToolGuardrailTracker::new();
        assert_eq!(
            t.observe("terminal", r#"{"command":  "ls -la" }"#),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("terminal", r#"{"command":"ls -la"}"#),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("terminal", r#"{"command": "ls -la"}"#),
            GuardrailDecision::Skip
        );
    }

    #[test]
    fn different_args_do_not_trip() {
        let mut t = ToolGuardrailTracker::new();
        assert_eq!(
            t.observe("terminal", r#"{"command": "ls"}"#),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("terminal", r#"{"command": "pwd"}"#),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("terminal", r#"{"command": "date"}"#),
            GuardrailDecision::Allow
        );
        assert_eq!(t.count_of("terminal", r#"{"command": "ls"}"#), 1);
    }

    #[test]
    fn no_effect_tool_warns_then_skips() {
        let mut t = ToolGuardrailTracker::new();
        assert_eq!(
            t.observe("file_search", "query=foo"),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("file_search", "query=foo"),
            GuardrailDecision::Allow
        );
        assert_eq!(
            t.observe("file_search", "query=foo"),
            GuardrailDecision::Warn
        );
        assert_eq!(
            t.observe("file_search", "query=foo"),
            GuardrailDecision::Skip
        );
    }

    #[test]
    fn reset_clears_per_turn_state() {
        let mut t = ToolGuardrailTracker::new();
        t.observe("terminal", "x");
        t.observe("terminal", "x");
        t.observe("terminal", "x");
        assert_eq!(t.count_of("terminal", "x"), 3);
        t.reset();
        assert_eq!(t.count_of("terminal", "x"), 0);
        assert_eq!(t.observe("terminal", "x"), GuardrailDecision::Allow);
    }

    #[test]
    fn side_effect_classification() {
        assert!(tool_may_have_side_effect("terminal"));
        assert!(tool_may_have_side_effect("write_file"));
        assert!(tool_may_have_side_effect("patch"));
        assert!(tool_may_have_side_effect("unknown_tool")); // default effect-capable
        assert!(!tool_may_have_side_effect("file_read"));
        assert!(!tool_may_have_side_effect("web_search"));
        assert!(!tool_may_have_side_effect("datetime"));
        assert!(!tool_may_have_side_effect("skill_view"));
    }

    #[test]
    fn skip_message_mentions_tool_and_count() {
        let m = build_skip_message("terminal", 3);
        assert!(m.contains("terminal"));
        assert!(m.contains("3 times"));
        assert!(m.contains("do NOT repeat"));
    }
}
