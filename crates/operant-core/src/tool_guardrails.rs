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
use std::collections::HashSet;
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

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
    "env_probe",
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

// ── Wave-2 harvest (iter-668): patterns absorbed from the runtime's
// `loop_detector.rs` (dead since W1.10 deleted the Loop B engine — its only
// consumer). Thresholds verbatim from that module.
/// Recent tool names retained for ping-pong pattern analysis.
pub const PATTERN_WINDOW_SIZE: usize = 20;
/// Two distinct tools alternating for 4+ complete cycles earn a warning.
pub const PING_PONG_MIN_CYCLES: usize = 4;
/// At 5+ complete alternation cycles the next call of either tool is skipped.
pub const PING_PONG_SKIP_CYCLES: usize = 5;
/// Same tool returning the identical result 5+ times in a row (args may vary)
/// earns a warning.
pub const NO_PROGRESS_MIN_CALLS: usize = 5;
/// At 6+ identical results in a row the tool's next call is skipped
/// regardless of arguments — the varied-args backstop.
pub const NO_PROGRESS_SKIP_CALLS: usize = 6;

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

/// Which repetition pattern produced a guardrail decision.
#[derive(Debug, Clone, PartialEq)]
pub enum RepeatPattern {
    /// Same (tool, args) pair repeated `count` times.
    IdenticalArgs { count: usize },
    /// Two distinct tools alternating for `cycles` complete cycles.
    PingPong {
        tool_a: String,
        tool_b: String,
        cycles: usize,
    },
    /// One tool returning the identical result `calls` times in a row.
    NoProgress { tool: String, calls: usize },
}

/// Per-turn tracker of identical tool-call repeats.
///
/// Pure and side-effect free apart from its own counters — directly
/// unit-testable without an agent or network.
#[derive(Debug, Default)]
pub struct ToolGuardrailTracker {
    /// (tool_name, normalized-args) → observed count this turn.
    counts: HashMap<(String, String), usize>,
    /// Recent tool names (capped at [`PATTERN_WINDOW_SIZE`]) — ping-pong input.
    name_window: VecDeque<String>,
    /// tool → (last result hash, consecutive identical-result count this
    /// turn) — no-progress input. Fed by [`Self::observe_result`].
    result_streaks: HashMap<String, (Option<u64>, usize)>,
    /// Tools whose identical-result streak tripped [`NO_PROGRESS_SKIP_CALLS`]:
    /// their next call is skipped regardless of args.
    no_progress_armed: HashMap<String, usize>,
    /// `is_repeat_call_exempt` port (the W1.8c-dropped `tool_call_dedup_exempt`):
    /// tools exempt from every repeat/loop rung. Survives [`Self::reset`].
    exempt_tools: HashSet<String>,
    /// The pattern that produced the most recent non-Allow decision.
    last_pattern: Option<RepeatPattern>,
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
        // Exempt tools bypass every repeat/loop rung and never feed pattern
        // analysis (an exempt tool inside an alternation must not arm its
        // non-exempt partner).
        if self.exempt_tools.contains(tool_name) {
            self.last_pattern = None;
            return GuardrailDecision::Allow;
        }

        // No-progress backstop: a tool whose identical-result streak tripped
        // NO_PROGRESS_SKIP_CALLS skips regardless of arguments.
        if let Some(&calls) = self.no_progress_armed.get(tool_name) {
            self.last_pattern = Some(RepeatPattern::NoProgress {
                tool: tool_name.to_string(),
                calls,
            });
            return GuardrailDecision::Skip;
        }

        // Ping-pong: two distinct tools alternating (Wave-2 harvest; the
        // identical-args rung below never sees these — different tools and
        // different argument keys).
        self.name_window.push_back(tool_name.to_string());
        while self.name_window.len() > PATTERN_WINDOW_SIZE {
            self.name_window.pop_front();
        }
        if let Some((tool_a, tool_b, cycles)) = ping_pong_cycles(&self.name_window) {
            if cycles >= PING_PONG_SKIP_CYCLES {
                self.last_pattern = Some(RepeatPattern::PingPong {
                    tool_a,
                    tool_b,
                    cycles,
                });
                return GuardrailDecision::Skip;
            }
            if cycles >= PING_PONG_MIN_CYCLES {
                self.last_pattern = Some(RepeatPattern::PingPong {
                    tool_a,
                    tool_b,
                    cycles,
                });
                return GuardrailDecision::Warn;
            }
        }

        let key = (tool_name.to_string(), Self::normalize_args(args));
        let count = self.counts.entry(key).or_insert(0);
        *count += 1;
        let count = *count;

        if tool_may_have_side_effect(tool_name) {
            if count >= REPEAT_SKIP_THRESHOLD_EFFECT {
                self.last_pattern = Some(RepeatPattern::IdenticalArgs { count });
                return GuardrailDecision::Skip;
            }
        } else if count >= REPEAT_SKIP_THRESHOLD_NO_EFFECT {
            self.last_pattern = Some(RepeatPattern::IdenticalArgs { count });
            return GuardrailDecision::Skip;
        }

        if count >= REPEAT_WARN_THRESHOLD {
            self.last_pattern = Some(RepeatPattern::IdenticalArgs { count });
            GuardrailDecision::Warn
        } else {
            self.last_pattern = None;
            GuardrailDecision::Allow
        }
    }

    /// Feed one executed tool result to the no-progress rung: a tool that
    /// returns the identical result [`NO_PROGRESS_SKIP_CALLS`] times in a
    /// row (arguments may vary — the varied-args backstop) is armed so its
    /// NEXT call skips pre-execution. Call once per real executed result;
    /// never feed synthetic guardrail skips (identical by construction).
    pub fn observe_result(&mut self, tool_name: &str, result: &str) -> GuardrailDecision {
        if self.exempt_tools.contains(tool_name) {
            return GuardrailDecision::Allow;
        }
        let hash = hash_str(result);
        let entry = self
            .result_streaks
            .entry(tool_name.to_string())
            .or_insert((None, 0));
        if entry.0 == Some(hash) {
            entry.1 += 1;
        } else {
            *entry = (Some(hash), 1);
        }
        let calls = entry.1;

        if calls >= NO_PROGRESS_MIN_CALLS {
            self.last_pattern = Some(RepeatPattern::NoProgress {
                tool: tool_name.to_string(),
                calls,
            });
            if calls >= NO_PROGRESS_SKIP_CALLS {
                self.no_progress_armed.insert(tool_name.to_string(), calls);
            }
            GuardrailDecision::Warn
        } else {
            self.last_pattern = None;
            GuardrailDecision::Allow
        }
    }

    /// The pattern behind the most recent non-Allow decision (None on Allow).
    pub fn last_pattern(&self) -> Option<RepeatPattern> {
        self.last_pattern.clone()
    }

    /// Mark tools exempt from every repeat/loop rung (the
    /// `is_repeat_call_exempt` port). Exemptions survive [`Self::reset`].
    pub fn with_exempt_tools(
        mut self,
        exempt: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.exempt_tools.extend(exempt.into_iter().map(Into::into));
        self
    }

    /// Number of times this exact (tool, args) pair has been observed.
    pub fn count_of(&self, tool_name: &str, args: &str) -> usize {
        let key = (tool_name.to_string(), Self::normalize_args(args));
        self.counts.get(&key).copied().unwrap_or(0)
    }

    /// Reset per-turn state (call at the start of each user turn).
    /// Exemptions survive: they are configuration, not turn state.
    pub fn reset(&mut self) {
        self.counts.clear();
        self.name_window.clear();
        self.result_streaks.clear();
        self.no_progress_armed.clear();
        self.last_pattern = None;
    }
}

/// Count complete alternation cycles at the tail of the name window:
/// `[..., a, b, a, b]` with two distinct names. One cycle = a→b→a.
/// Returns the two names and the cycle count when at least one full cycle is
/// present. Ported from the runtime `loop_detector::detect_ping_pong`
/// (MIN_CYCLES=4 → 8 alternating calls for the first warning).
fn ping_pong_cycles(window: &VecDeque<String>) -> Option<(String, String, usize)> {
    let n = window.len();
    if n < 3 {
        return None;
    }
    let newest = &window[n - 1];
    let prev = &window[n - 2];
    if newest == prev {
        return None;
    }
    let mut len = 2usize;
    let mut i = n - 2;
    while i > 0 {
        // Strict alternation: element i-1 must equal element i+1.
        if window[i - 1] == window[i + 1] {
            len += 1;
            i -= 1;
        } else {
            break;
        }
    }
    (len >= 3).then(|| (prev.clone(), newest.clone(), len / 2))
}

/// Prefix stamped on synthetic skip results. `run`'s repetition guard keys
/// on it so guardrail-skipped duplicates never advance the breaker streak.
/// EVERY guardrail skip variant must carry it.
pub const SKIP_MESSAGE_PREFIX: &str = "Guardrail: ";

/// Pattern-aware skip text for the synthetic ToolResult. `None` (defensive —
/// a Skip always carries a pattern) falls back to a generic copy.
pub fn build_pattern_skip_message(tool_name: &str, pattern: Option<&RepeatPattern>) -> String {
    match pattern {
        Some(RepeatPattern::IdenticalArgs { count }) => build_skip_message(tool_name, *count),
        Some(RepeatPattern::PingPong {
            tool_a,
            tool_b,
            cycles,
        }) => format!(
            "{SKIP_MESSAGE_PREFIX}tools '{tool_a}' and '{tool_b}' have alternated for \
             {cycles} complete cycles this turn with no progress. Continuing the \
             alternation cannot finish the task — break the pattern or stop."
        ),
        Some(RepeatPattern::NoProgress { tool, calls }) => format!(
            "{SKIP_MESSAGE_PREFIX}tool '{tool}' has returned the identical result \
             {calls} times in a row this turn regardless of arguments. Repeating it \
             cannot produce new information — stop calling it or change approach."
        ),
        None => format!(
            "{SKIP_MESSAGE_PREFIX}repetitive tool-call pattern detected — this call was skipped."
        ),
    }
}

/// Feed-visible warning copy for the guardrail Warn arm.
pub fn pattern_warning_message(tool_name: &str, pattern: Option<&RepeatPattern>) -> String {
    match pattern {
        Some(RepeatPattern::IdenticalArgs { count }) => format!(
            "⚠ Tool '{tool_name}' has been called with identical arguments {count} times this turn."
        ),
        Some(RepeatPattern::PingPong {
            tool_a,
            tool_b,
            cycles,
        }) => format!(
            "⚠ Tools '{tool_a}' and '{tool_b}' have been alternating for {cycles} \
             complete cycles this turn — this is a stuck pattern."
        ),
        Some(RepeatPattern::NoProgress { tool, calls }) => format!(
            "⚠ Tool '{tool}' has returned the identical result {calls} times in a row \
             this turn regardless of arguments."
        ),
        None => format!("⚠ Tool '{tool_name}' is repeating itself without progress."),
    }
}

/// Build the synthetic skip result text fed back to the model.
pub fn build_skip_message(tool_name: &str, count: usize) -> String {
    // W1.8c: no bracket prefix — a leading `[` makes the anomaly heuristic's
    // `is_malformed_tool_output` treat this as failed JSON and re-ask the
    // model, burning the budget (Loop B's skip text was bracketless too).
    format!(
        "{SKIP_MESSAGE_PREFIX}tool '{tool_name}' was already called with identical arguments \
         {count} times this turn. Skipping this duplicate call — do NOT repeat it. \
         If the previous results were insufficient, change the arguments or use a \
         different tool."
    )
}

// Fingerprint fns moved from operant-runtime/src/agent/loop_detector.rs @ 4ddbe62e (Wave 1 harvest; Wave 2 replaces normalize_args with hash_value).

/// Produce a deterministic hash for a JSON value by recursively sorting
/// object keys before serialisation.  This ensures `{"a":1,"b":2}` and
/// `{"b":2,"a":1}` hash identically.
pub fn hash_value(value: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    let canonical = serde_json::to_string(&canonicalise(value)).unwrap_or_default();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Return a clone of `value` with all object keys sorted recursively.
pub fn canonicalise(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let new_map: serde_json::Map<String, serde_json::Value> = sorted
                .into_iter()
                .map(|(k, v)| (k.clone(), canonicalise(v)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalise).collect())
        }
        other => other.clone(),
    }
}

pub fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert!(!tool_may_have_side_effect("env_probe"));
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

    // ── hash_value key-order independence ────────────────────────

    #[test]
    fn hash_value_is_key_order_independent() {
        let a = json!({"alpha": 1, "beta": 2});
        let b = json!({"beta": 2, "alpha": 1});
        assert_eq!(
            hash_value(&a),
            hash_value(&b),
            "hash_value must produce identical hashes regardless of JSON key order"
        );
    }

    #[test]
    fn hash_value_nested_key_order_independent() {
        let a = json!({"outer": {"x": 1, "y": 2}, "z": [1, 2]});
        let b = json!({"z": [1, 2], "outer": {"y": 2, "x": 1}});
        assert_eq!(
            hash_value(&a),
            hash_value(&b),
            "nested objects must also be key-order independent"
        );
    }
}

#[cfg(test)]
mod pattern_tests {
    //! Wave-2 harvest: the ping-pong / no-progress / exemption rungs ported
    //! from the runtime's deleted `loop_detector.rs` (dead since W1.10 — its
    //! only consumer was the deleted Loop B engine). Thresholds verbatim:
    //! MIN_CYCLES=4, MIN_CALLS=5, escalation at +1.

    use super::*;

    fn seq(t: &mut ToolGuardrailTracker, names: &[&str]) -> Vec<GuardrailDecision> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| t.observe(n, &format!("{{\"i\":{i}}}")))
            .collect()
    }

    #[test]
    fn ping_pong_warns_on_four_complete_cycles() {
        let mut t = ToolGuardrailTracker::new();
        let names: Vec<&str> = (0..4).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &names);
        assert_eq!(d[7], GuardrailDecision::Warn);
        assert!(matches!(
            t.last_pattern(),
            Some(RepeatPattern::PingPong { cycles: 4, .. })
        ));
    }

    #[test]
    fn ping_pong_skips_on_fifth_cycle() {
        let mut t = ToolGuardrailTracker::new();
        let names: Vec<&str> = (0..5).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &names);
        assert_eq!(d[7], GuardrailDecision::Warn);
        assert_eq!(d[9], GuardrailDecision::Skip);
    }

    #[test]
    fn ping_pong_requires_two_distinct_tools() {
        let mut t = ToolGuardrailTracker::new();
        // Same tool, varied args: neither the ping-pong rung (needs two
        // distinct names) nor the identical-args rung (args differ) may trip.
        let all_allow = (0..8)
            .map(|i| t.observe("tool_a", &format!("{{\"i\":{i}}}")))
            .all(|d| d == GuardrailDecision::Allow);
        assert!(all_allow);
    }

    #[test]
    fn ping_pong_broken_by_third_tool() {
        let mut t = ToolGuardrailTracker::new();
        let names = [
            "tool_a", "tool_b", "tool_a", "tool_b", // 2 cycles
            "tool_c", // break
            "tool_a", "tool_b", "tool_a", "tool_b", // 2 cycles again
        ];
        let d = seq(&mut t, &names);
        assert!(d.iter().all(|d| *d == GuardrailDecision::Allow));
    }

    #[test]
    fn ping_pong_window_expires_old_patterns() {
        let mut t = ToolGuardrailTracker::new();
        let warm: Vec<&str> = (0..4).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &warm);
        assert_eq!(d[7], GuardrailDecision::Warn);
        for i in 0..PATTERN_WINDOW_SIZE {
            t.observe("tool_c", &format!("{{\"i\":{i}}}"));
        }
        let fresh: Vec<&str> = (0..2).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &fresh);
        assert!(
            d.iter().all(|d| *d == GuardrailDecision::Allow),
            "a 2-cycle probe after the old alternation aged out must stay quiet: {d:?}"
        );
    }

    #[test]
    fn no_progress_warns_on_fifth_identical_result() {
        let mut t = ToolGuardrailTracker::new();
        for i in 0..4 {
            assert_eq!(
                t.observe_result("tool_x", "identical"),
                GuardrailDecision::Allow,
                "call {i}"
            );
        }
        assert_eq!(
            t.observe_result("tool_x", "identical"),
            GuardrailDecision::Warn
        );
    }

    #[test]
    fn no_progress_arms_skip_on_sixth_regardless_of_args() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..6 {
            t.observe_result("tool_x", "identical");
        }
        // Varied-args backstop: the NEXT call of the armed tool skips even
        // with fresh arguments — the rung the identical-args guard could
        // never catch.
        assert_eq!(
            t.observe("tool_x", "{\"never_seen\":true}"),
            GuardrailDecision::Skip
        );
        assert!(matches!(
            t.last_pattern(),
            Some(RepeatPattern::NoProgress { calls: 6, .. })
        ));
    }

    #[test]
    fn no_progress_streak_resets_on_different_result() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..4 {
            t.observe_result("tool_x", "same");
        }
        assert_eq!(
            t.observe_result("tool_x", "different"),
            GuardrailDecision::Allow
        );
        assert_eq!(t.observe_result("tool_x", "same"), GuardrailDecision::Allow);
        assert_eq!(t.observe_result("tool_x", "same"), GuardrailDecision::Allow);
    }

    #[test]
    fn no_progress_tracks_tools_independently() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..5 {
            t.observe_result("tool_x", "same");
            t.observe_result("tool_y", "same");
        }
        assert_eq!(t.observe_result("tool_x", "same"), GuardrailDecision::Warn);
        assert_eq!(t.observe_result("tool_y", "same"), GuardrailDecision::Warn);
    }

    #[test]
    fn exempt_tools_bypass_every_rung() {
        let mut t = ToolGuardrailTracker::new().with_exempt_tools(["terminal", "tool_a", "tool_b"]);
        for _ in 0..8 {
            assert_eq!(t.observe("terminal", "{}"), GuardrailDecision::Allow);
            assert_eq!(
                t.observe_result("terminal", "same"),
                GuardrailDecision::Allow
            );
        }
        let names: Vec<&str> = (0..6).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &names);
        assert!(d.iter().all(|d| *d == GuardrailDecision::Allow));
    }

    #[test]
    fn reset_clears_patterns_but_keeps_exemptions() {
        let mut t = ToolGuardrailTracker::new().with_exempt_tools(["terminal"]);
        let tripped: Vec<&str> = (0..5).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &tripped);
        assert_eq!(d[9], GuardrailDecision::Skip);
        t.reset();
        let fresh: Vec<&str> = (0..4).flat_map(|_| ["tool_a", "tool_b"]).collect();
        let d = seq(&mut t, &fresh);
        assert_eq!(d[7], GuardrailDecision::Warn, "fresh window after reset");
        assert_eq!(
            t.observe("terminal", "{}"),
            GuardrailDecision::Allow,
            "exemptions survive reset"
        );
    }

    #[test]
    fn every_skip_variant_carries_the_prefix() {
        assert!(
            build_pattern_skip_message("t", Some(&RepeatPattern::IdenticalArgs { count: 3 }))
                .starts_with(SKIP_MESSAGE_PREFIX)
        );
        assert!(
            build_pattern_skip_message(
                "t",
                Some(&RepeatPattern::PingPong {
                    tool_a: "a".into(),
                    tool_b: "b".into(),
                    cycles: 5
                })
            )
            .starts_with(SKIP_MESSAGE_PREFIX)
        );
        assert!(
            build_pattern_skip_message(
                "t",
                Some(&RepeatPattern::NoProgress {
                    tool: "x".into(),
                    calls: 6
                })
            )
            .starts_with(SKIP_MESSAGE_PREFIX)
        );
        assert!(build_pattern_skip_message("t", None).starts_with(SKIP_MESSAGE_PREFIX));
    }
}
