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

// ── Wave-2 remainder (iter-681): per-tool failure ladder, hard-reject
// halt, and the successful-repeat recurrence ledger, ported from openhuman
// `no_progress/` (`mod.rs` ladder semantics + `successful_repeat.rs`).
/// Per-tool consecutive failures (any error class) before the model is
/// warned. The S2 timeout breaker masks after 3 timeouts; this is the
/// slower cross-class backstop beneath it.
pub const FAILURE_WARN_THRESHOLD: usize = 8;
/// Per-tool consecutive failures before the turn halts with a root-cause
/// summary (openhuman's any-failure backstop, per-tool at the 8/12
/// thresholds from the port plan).
pub const FAILURE_HALT_THRESHOLD: usize = 12;
/// Consecutive hard policy rejections before halting — a blocked call
/// re-issued unchanged can never succeed (openhuman
/// `HARD_REJECT_HALT_THRESHOLD`). Seat-policy denials are deliberately NOT
/// hard rejects: a standing grant minted between ticks can make them
/// succeed.
pub const HARD_REJECT_HALT: usize = 2;
/// Result-content prefix that marks a hard security/approval rejection.
/// stream.rs's blocked arm constructs its message from this constant so
/// the classifier and the message cannot drift apart.
pub const HARD_REJECT_PREFIX: &str = "Blocked by security policy";
/// Consecutive identical assistant outputs (narration + tool batch
/// signature) before the model is warned — openhuman
/// `DEFAULT_REPEAT_OUTPUT_THRESHOLD` (`record_output`).
pub const OUTPUT_REPEAT_WARN: usize = 4;
/// At one more identical output the next tool call is skipped regardless
/// of identity — the output-side analog of [`NO_PROGRESS_SKIP_CALLS`].
/// Deliberately warn/skip, never Halt: repeated *successes* are corrected,
/// not punished (the iter-682 deviation from openhuman's halt).
pub const OUTPUT_REPEAT_SKIP: usize = 5;

/// Guardrail decision for one observed tool call or result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailDecision {
    /// Proceed with execution.
    Allow,
    /// Warn the model (surfaced in the feed) but still execute — used for
    /// repeated no-effect calls below the skip threshold.
    Warn,
    /// Skip execution and return a synthetic result telling the model to
    /// stop repeating this exact call.
    Skip,
    /// Halt the whole turn and surface this root-cause summary
    /// (`failure_copy`) as the final content. Only the post-result rungs
    /// (per-tool failure ladder, hard-reject) produce it.
    Halt(String),
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
    /// One tool returning the identical result `calls` times this turn
    /// (consecutive streak or run-wide recurrence ledger).
    NoProgress { tool: String, calls: usize },
    /// One tool failing `count` times in a row this turn, any error class.
    Failures { tool: String, count: usize },
    /// The identical assistant output (narration + tool batch) repeated
    /// `count` consecutive iterations this turn — the output-side
    /// successful-repeat rung (openhuman `record_output`).
    IdenticalOutput { count: usize },
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
    /// Per-tool consecutive failures (any error class), reset by that
    /// tool's next success. Warns at [`FAILURE_WARN_THRESHOLD`], halts at
    /// [`FAILURE_HALT_THRESHOLD`].
    failure_streaks: HashMap<String, usize>,
    /// Per-tool consecutive hard policy rejections (content prefix
    /// [`HARD_REJECT_PREFIX`]), reset by any other result. Halts at
    /// [`HARD_REJECT_HALT`].
    hard_reject_streaks: HashMap<String, usize>,
    /// Run-wide `(tool, result-hash)` → times that identical successful
    /// result was observed this turn (openhuman `recurrences`): catches
    /// A, B(reset), A cycles the consecutive streak rung can never see.
    recurrences: HashMap<(String, u64), usize>,
    /// (last iteration-signature hash, consecutive identical count) — the
    /// output-side successful-repeat streak (openhuman
    /// `SuccessfulRepeatTracker::record_output`). Fed by
    /// [`Self::observe_output`] once per completed iteration.
    output_streak: (Option<u64>, usize),
    /// True when the output streak tripped [`OUTPUT_REPEAT_SKIP`]: the
    /// next non-exempt tool call is skipped regardless of identity.
    output_armed: bool,
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

        // Output-side backstop (iter-688): the iteration-level repeat
        // streak tripped OUTPUT_REPEAT_SKIP — skip the next call whatever
        // it is. The model already narrated the identical output ≥5 times;
        // feeding it another result cannot produce new information.
        if self.output_armed {
            self.last_pattern = Some(RepeatPattern::IdenticalOutput {
                count: self.output_streak.1,
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

    /// Feed one executed tool result to the post-execution rungs: the
    /// no-progress identical-result ladder (streak + run-wide recurrence
    /// ledger), the per-tool failure ladder (warn [`FAILURE_WARN_THRESHOLD`],
    /// halt [`FAILURE_HALT_THRESHOLD`]), and the hard-reject halt
    /// ([`HARD_REJECT_HALT`]). Call once per real executed result, with its
    /// success flag; never feed synthetic guardrail skips (identical by
    /// construction, excluded upstream).
    pub fn observe_result(
        &mut self,
        tool_name: &str,
        result: &str,
        success: bool,
    ) -> GuardrailDecision {
        if self.exempt_tools.contains(tool_name) {
            return GuardrailDecision::Allow;
        }

        // ── Hard policy rejection: a blocked call re-issued unchanged can
        // never succeed — halt at the second one (openhuman
        // HARD_REJECT_HALT_THRESHOLD).
        if !success && result.starts_with(HARD_REJECT_PREFIX) {
            let streak = self
                .hard_reject_streaks
                .entry(tool_name.to_string())
                .or_insert(0);
            *streak += 1;
            if *streak >= HARD_REJECT_HALT {
                // Self-reset so a resumed turn does not insta-trip.
                self.hard_reject_streaks.remove(tool_name);
                self.last_pattern = None;
                return GuardrailDecision::Halt(hard_reject_halt_summary(
                    tool_name,
                    result,
                ));
            }
            self.last_pattern = None;
            return GuardrailDecision::Allow;
        }

        // ── Per-tool failure ladder (any error class). A success on the
        // same tool resets the streak below.
        if !success {
            let streak = self
                .failure_streaks
                .entry(tool_name.to_string())
                .or_insert(0);
            *streak += 1;
            let count = *streak;
            if count >= FAILURE_HALT_THRESHOLD {
                self.failure_streaks.remove(tool_name);
                self.last_pattern = None;
                return GuardrailDecision::Halt(failure_halt_summary(tool_name, count));
            }
            if count >= FAILURE_WARN_THRESHOLD {
                self.last_pattern = Some(RepeatPattern::Failures {
                    tool: tool_name.to_string(),
                    count,
                });
                return GuardrailDecision::Warn;
            }
            self.last_pattern = None;
            return GuardrailDecision::Allow;
        }

        // ── Success: the failure ladders reset for this tool…
        self.failure_streaks.remove(tool_name);
        self.hard_reject_streaks.remove(tool_name);

        // ── …and the result feeds the no-progress rungs. The consecutive
        // streak (`result_streaks`) plus the run-wide recurrence ledger
        // (`recurrences`): a repeat after a streak-resetting different
        // result still counts, so A, B(reset), A cycles trip at the same
        // 5/6 thresholds as back-to-back repeats.
        let hash = hash_str(result);
        let ledger = {
            let rec = self
                .recurrences
                .entry((tool_name.to_string(), hash))
                .or_insert(0);
            *rec += 1;
            *rec
        };
        let entry = self
            .result_streaks
            .entry(tool_name.to_string())
            .or_insert((None, 0));
        if entry.0 == Some(hash) {
            entry.1 += 1;
        } else {
            *entry = (Some(hash), 1);
        }
        let calls = entry.1.max(ledger);

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

    /// Adds exempt tools to a live tracker — the mutable complement to
    /// [`Self::with_exempt_tools`], used at construction time when
    /// `AgentConfig::guardrail_exempt_tools` is non-empty (iter-683).
    pub fn add_exempt_tools(
        &mut self,
        exempt: impl IntoIterator<Item = impl Into<String>>,
    ) {
        self.exempt_tools.extend(exempt.into_iter().map(Into::into));
    }

    /// `true` when `tool_name` is exempt from every repeat/loop rung.
    pub fn is_exempt(&self, tool_name: &str) -> bool {
        self.exempt_tools.contains(tool_name)
    }

    /// Feed one completed iteration's canonical output signature (assistant
    /// narration + ordered tool batch) to the output-side successful-repeat
    /// rung — the openhuman `SuccessfulRepeatTracker::record_output` port.
    /// Warn at [`OUTPUT_REPEAT_WARN`] identical consecutive outputs; arm the
    /// skip-everything backstop at [`OUTPUT_REPEAT_SKIP`]. Per the iter-682
    /// deviation this rung never Halts: repeated successes get a warning
    /// and a skip, not a turn abort.
    ///
    /// `batch_successful` = the batch produced at least one real executed
    /// result and none failed; `batch_exempt` = every call in the batch was
    /// exempt (legitimate polling). A failed or all-exempt batch resets the
    /// streak so its output cannot leak into the next progress-eligible
    /// iteration (openhuman `record_call_batch` gate semantics).
    pub fn observe_output(
        &mut self,
        signature: u64,
        batch_successful: bool,
        batch_exempt: bool,
    ) -> GuardrailDecision {
        if batch_exempt || !batch_successful {
            self.output_streak = (None, 0);
            self.output_armed = false;
            self.last_pattern = None;
            return GuardrailDecision::Allow;
        }
        if self.output_streak.0 == Some(signature) {
            self.output_streak.1 += 1;
        } else {
            self.output_streak = (Some(signature), 1);
        }
        let count = self.output_streak.1;
        if count >= OUTPUT_REPEAT_WARN {
            self.last_pattern = Some(RepeatPattern::IdenticalOutput { count });
            if count >= OUTPUT_REPEAT_SKIP {
                self.output_armed = true;
            }
            return GuardrailDecision::Warn;
        }
        self.last_pattern = None;
        GuardrailDecision::Allow
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
        self.failure_streaks.clear();
        self.hard_reject_streaks.clear();
        self.recurrences.clear();
        self.output_streak = (None, 0);
        self.output_armed = false;
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

/// Halt copy for the second consecutive hard policy rejection (the
/// `failure_copy` port — root-cause summaries instead of a generic cap
/// error). `last` is the blocked result the classifier matched on.
fn hard_reject_halt_summary(tool_name: &str, last: &str) -> String {
    format!(
        "Stopping: the `{tool_name}` call is blocked by the security policy and was re-issued \
         unchanged — it can never succeed this way. Reason:\n{last}\n\nDo not repeat this \
         call; use an allowed alternative or report that it can't be done here."
    )
}

/// Halt copy for the per-tool failure ladder exhausting its budget.
fn failure_halt_summary(tool_name: &str, count: usize) -> String {
    format!(
        "Stopping: `{tool_name}` has failed {count} times in a row this turn with no progress. \
         Repeating variations of the same approach cannot finish the task — report this back \
         instead of retrying."
    )
}

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
             {calls} times this turn regardless of arguments. Repeating it \
             cannot produce new information — stop calling it or change approach."
        ),
        Some(RepeatPattern::Failures { tool, count }) => format!(
            "{SKIP_MESSAGE_PREFIX}tool '{tool}' has failed {count} times in a row this turn \
             without progress — a different approach is required."
        ),
        Some(RepeatPattern::IdenticalOutput { count }) => format!(
            "{SKIP_MESSAGE_PREFIX}the last {count} iterations produced the identical \
             response and tool call. Repeating the same step cannot finish the task \
             — this call was skipped. Change your approach or finish with what you \
             already have."
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
            "⚠ Tool '{tool}' has returned the identical result {calls} times \
             this turn regardless of arguments."
        ),
        Some(RepeatPattern::Failures { tool, count }) => format!(
            "⚠ Tool '{tool}' has failed {count} times in a row this turn without \
             making progress — change approach before it exhausts the turn."
        ),
        Some(RepeatPattern::IdenticalOutput { count }) => format!(
            "⚠ The last {count} iterations produced the identical response and tool \
             call with no change. The run is stuck repeating the same step — \
             change your approach or finish with what you have."
        ),
        None => format!("⚠ Tool '{tool_name}' is repeating itself without progress."),
    }
}

/// Feed-visible warning copy for the output-side repeat rung (the Warn
/// verdict from [`ToolGuardrailTracker::observe_output`] — no tool name
/// to anchor it, unlike the per-tool patterns).
pub fn output_repeat_warning_message(count: usize) -> String {
    format!(
        "⚠ The last {count} iterations produced the identical response and tool \
         call with no change. The run appears stuck repeating the same step — \
         change your approach or finish with what you already have."
    )
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
                t.observe_result("tool_x", "identical", true),
                GuardrailDecision::Allow,
                "call {i}"
            );
        }
        assert_eq!(
            t.observe_result("tool_x", "identical", true),
            GuardrailDecision::Warn
        );
    }

    #[test]
    fn no_progress_arms_skip_on_sixth_regardless_of_args() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..6 {
            t.observe_result("tool_x", "identical", true);
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
            t.observe_result("tool_x", "same", true);
        }
        assert_eq!(
            t.observe_result("tool_x", "different", true),
            GuardrailDecision::Allow
        );
        // The consecutive streak restarted, but the run-wide recurrence
        // ledger still holds the four earlier identical results: the next
        // "same" is the fifth recurrence and warns (iter-681).
        assert_eq!(
            t.observe_result("tool_x", "same", true),
            GuardrailDecision::Warn
        );
    }

    #[test]
    fn no_progress_tracks_tools_independently() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..5 {
            t.observe_result("tool_x", "same", true);
            t.observe_result("tool_y", "same", true);
        }
        assert_eq!(t.observe_result("tool_x", "same", true), GuardrailDecision::Warn);
        assert_eq!(t.observe_result("tool_y", "same", true), GuardrailDecision::Warn);
    }

    #[test]
    fn exempt_tools_bypass_every_rung() {
        let mut t = ToolGuardrailTracker::new().with_exempt_tools(["terminal", "tool_a", "tool_b"]);        for _ in 0..8 {
            assert_eq!(t.observe("terminal", "{}"), GuardrailDecision::Allow);
            assert_eq!(
                t.observe_result("terminal", "same", true),
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

#[cfg(test)]
mod ladder_tests {
    //! Wave-2 remainder (iter-681): adversarial ports of openhuman's
    //! `no_progress/mod_tests.rs` suite, adapted to operant's decision
    //! vocabulary (Warn/Skip/Halt instead of Nudge/Halt): the per-tool
    //! failure ladder, the hard-reject halt, and the successful-repeat
    //! recurrence ledger.

    use super::*;

    /// Distinct tool names for the cycle-length test.
    const CYCLE_TOOLS: [&str; 4] = ["tool_a", "tool_b", "tool_c", "tool_d"];

    fn fail(t: &mut ToolGuardrailTracker, tool: &str, err: &str) -> GuardrailDecision {
        t.observe_result(tool, err, false)
    }

    fn ok(t: &mut ToolGuardrailTracker, tool: &str, result: &str) -> GuardrailDecision {
        t.observe_result(tool, result, true)
    }

    #[test]
    fn a_hard_rejection_halts_on_the_second_consecutive_one() {
        let mut t = ToolGuardrailTracker::new();
        assert_eq!(
            fail(&mut t, "send_email", "Blocked by security policy: no email tool"),
            GuardrailDecision::Allow
        );
        match fail(&mut t, "send_email", "Blocked by security policy: no email tool") {
            GuardrailDecision::Halt(msg) => {
                assert!(msg.contains("security policy"), "{msg}");
                assert!(msg.contains("send_email"), "{msg}");
                assert!(msg.contains("can never succeed"), "{msg}");
            }
            other => panic!("expected a halt on the second hard reject, got {other:?}"),
        }
    }

    #[test]
    fn hard_reject_streak_resets_on_a_non_blocked_result() {
        let mut t = ToolGuardrailTracker::new();
        fail(&mut t, "shell", "Blocked by security policy: dangerous");
        // A success in between means the next blocked call is a first again.
        assert_eq!(ok(&mut t, "shell", "done"), GuardrailDecision::Allow);
        assert_eq!(
            fail(&mut t, "shell", "Blocked by security policy: dangerous"),
            GuardrailDecision::Allow
        );
    }

    #[test]
    fn only_the_security_policy_prefix_is_a_hard_reject() {
        let mut t = ToolGuardrailTracker::new();
        // Seat-policy denials and ordinary errors must feed the (slower)
        // failure ladder, never the hard-reject halt.
        for i in 0..3 {
            assert_eq!(
                fail(&mut t, "write_file", &format!("declined by seat policy ({i})")),
                GuardrailDecision::Allow
            );
        }
        assert_ne!(
            fail(&mut t, "write_file", "declined by seat policy (4)"),
            GuardrailDecision::Halt("x".into())
        );
    }

    #[test]
    fn varied_failures_warn_at_eight_and_halt_at_twelve() {
        let mut t = ToolGuardrailTracker::new();
        for i in 1..=7 {
            assert_eq!(
                fail(&mut t, "terminal", &format!("error variant {i}")),
                GuardrailDecision::Allow,
                "call {i}"
            );
        }
        assert_eq!(
            fail(&mut t, "terminal", "error variant 8"),
            GuardrailDecision::Warn
        );
        for i in 9..=11 {
            assert_eq!(
                fail(&mut t, "terminal", &format!("error variant {i}")),
                GuardrailDecision::Warn,
                "call {i}"
            );
        }
        match fail(&mut t, "terminal", "error variant 12") {
            GuardrailDecision::Halt(msg) => {
                assert!(msg.contains("12 times"), "{msg}");
                assert!(msg.contains("terminal"), "{msg}");
                assert!(msg.contains("report this back"), "{msg}");
            }
            other => panic!("expected a halt at the twelfth failure, got {other:?}"),
        }
    }

    #[test]
    fn a_success_resets_the_failure_ladder() {
        let mut t = ToolGuardrailTracker::new();
        for i in 1..=11 {
            let _ = fail(&mut t, "search", &format!("boom {i}"));
        }
        assert_eq!(ok(&mut t, "search", "found it"), GuardrailDecision::Allow);
        // The ladder restarted: seven more failures stay quiet, and the
        // eighth warns instead of halting.
        for i in 1..=7 {
            assert_eq!(
                fail(&mut t, "search", &format!("boom again {i}")),
                GuardrailDecision::Allow,
                "call {i}"
            );
        }
        assert_eq!(
            fail(&mut t, "search", "boom again 8"),
            GuardrailDecision::Warn
        );
    }

    #[test]
    fn failure_ladders_track_tools_independently() {
        let mut t = ToolGuardrailTracker::new();
        for i in 0..7 {
            assert_eq!(
                fail(&mut t, "tool_a", &format!("a{i}")),
                GuardrailDecision::Allow
            );
            assert_eq!(
                fail(&mut t, "tool_b", &format!("b{i}")),
                GuardrailDecision::Allow
            );
        }
        assert_eq!(fail(&mut t, "tool_a", "a8"), GuardrailDecision::Warn);
        assert_eq!(fail(&mut t, "tool_b", "b8"), GuardrailDecision::Warn);
    }

    #[test]
    fn failure_halts_self_reset_so_a_resumed_turn_does_not_insta_trip() {
        let mut t = ToolGuardrailTracker::new();
        // Calls 1-7 stay quiet, 8-11 warn, 12 halts.
        for i in 1..=7 {
            assert_eq!(
                fail(&mut t, "lookup", &format!("err{i}")),
                GuardrailDecision::Allow
            );
        }
        for i in 8..=11 {
            assert_eq!(
                fail(&mut t, "lookup", &format!("err{i}")),
                GuardrailDecision::Warn
            );
        }
        assert!(matches!(
            fail(&mut t, "lookup", "err12"),
            GuardrailDecision::Halt(_)
        ));
        // The halt cleared the streak: the next failure is a first again.
        assert_eq!(fail(&mut t, "lookup", "err13"), GuardrailDecision::Allow);
    }

    #[test]
    fn recurrence_ledger_catches_cycles_for_any_cycle_length() {
        // openhuman `cycling_identical_calls_halt_for_any_cycle_length`:
        // the streak rung never sees two identical results in a row inside
        // a cycle, but the run-wide ledger does — adapted to operant's
        // warn-at-5 / arm-at-6 escalation.
        for cycle_len in 2..=4 {
            let mut t = ToolGuardrailTracker::new();
            let tools: Vec<&str> = (0..cycle_len).map(|i| CYCLE_TOOLS[i]).collect();
            let mut warned_at = None;
            for step in 0..cycle_len * 7 {
                let tool = tools[step % cycle_len];
                if ok(&mut t, tool, "same result") == GuardrailDecision::Warn {
                    warned_at = Some(step);
                    break;
                }
            }
            assert_eq!(
                warned_at,
                Some(cycle_len * 4),
                "a {cycle_len}-tool cycle with identical results must warn on the first tool's fifth recurrence"
            );
        }
    }

    #[test]
    fn a_changed_result_is_not_a_recurrence() {
        let mut t = ToolGuardrailTracker::new();
        for i in 0..10 {
            assert_eq!(
                ok(&mut t, "read_status", &format!("status-{i}")),
                GuardrailDecision::Allow,
                "a re-read whose result changed is progress, not a repeat"
            );
        }
    }

    #[test]
    fn failures_do_not_feed_the_recurrence_ledger() {
        let mut t = ToolGuardrailTracker::new();
        ok(&mut t, "read_doc", "doc");
        // Failing sibling calls between the identical successes must not
        // hide the recurrence (openhuman: failed batches don't clear the
        // ledger — here they also don't feed it).
        let _ = fail(&mut t, "read_doc", "boom");
        ok(&mut t, "read_doc", "doc");
        let _ = fail(&mut t, "read_doc", "boom again");
        assert_eq!(
            ok(&mut t, "read_doc", "doc"),
            GuardrailDecision::Allow,
            "two successes + two failures is only the third recurrence"
        );
        assert_eq!(
            ok(&mut t, "read_doc", "doc"),
            GuardrailDecision::Allow,
            "fourth recurrence: still below the warn threshold"
        );
        assert_eq!(
            ok(&mut t, "read_doc", "doc"),
            GuardrailDecision::Warn,
            "fifth recurrence of (read_doc, doc) warns even though the streak kept resetting"
        );
    }

    #[test]
    fn duplicate_progress_spam_survives_the_output_batch_reset() {
        // The duplicate-progress-spam fault (openhuman no_progress
        // remainder): the loop is padded with failed batches so the
        // output-side streak keeps resetting, while the same successful
        // result recurs underneath. `observe_output`'s reset arm clears
        // the OUTPUT streak only — the recurrence ledger is run-wide
        // state, not batch state, so the spam still escalates on the
        // same 5/6 rungs as back-to-back repeats.
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..3 {
            let _ = ok(&mut t, "poll", "same");
            // A failed batch resets the output streak between every pair.
            let _ = t.observe_output(7, false, false);
        }
        assert_eq!(
            ok(&mut t, "poll", "same"),
            GuardrailDecision::Allow,
            "fourth recurrence: still below the warn threshold"
        );
        assert_eq!(
            ok(&mut t, "poll", "same"),
            GuardrailDecision::Warn,
            "fifth recurrence warns even though every interleaved batch reset the output streak"
        );
        let _ = ok(&mut t, "poll", "same");
        assert_eq!(
            t.observe("poll", "{\"args\":\"varied to dodge the identical-call rung\"}"),
            GuardrailDecision::Skip,
            "the sixth recurrence arms the skip across resets"
        );
    }

    #[test]
    fn recurrence_ledger_arms_the_varied_args_backstop() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..6 {
            let _ = ok(&mut t, "poll", "same");
        }
        assert_eq!(
            t.observe("poll", "{\"fresh_args\":true}"),
            GuardrailDecision::Skip,
            "six run-wide identical results arm the next-call skip"
        );
    }

    #[test]
    fn reset_clears_the_failure_ladders_and_ledger() {
        let mut t = ToolGuardrailTracker::new();
        for i in 0..11 {
            let _ = fail(&mut t, "lookup", &format!("e{i}"));
        }
        ok(&mut t, "other", "same");
        ok(&mut t, "other", "same");
        t.reset();
        assert_eq!(fail(&mut t, "lookup", "e12"), GuardrailDecision::Allow);
        assert_eq!(ok(&mut t, "other", "same"), GuardrailDecision::Allow);
    }

    #[test]
    fn exempt_tools_bypass_the_failure_and_hard_reject_rungs() {
        let mut t = ToolGuardrailTracker::new().with_exempt_tools(["terminal"]);
        for i in 0..15 {
            assert_eq!(
                fail(&mut t, "terminal", &format!("boom {i}")),
                GuardrailDecision::Allow
            );
            assert_eq!(
                fail(&mut t, "terminal", "Blocked by security policy: denied"),
                GuardrailDecision::Allow
            );
        }
    }

    #[test]
    fn add_exempt_tools_extends_a_live_tracker() {
        // iter-683: the construction-time path — a tracker built plain, then
        // extended from AgentConfig::guardrail_exempt_tools.
        let mut t = ToolGuardrailTracker::new();
        assert!(!t.is_exempt("poll_status"));
        t.add_exempt_tools(["poll_status"]);
        assert!(t.is_exempt("poll_status"));
        // The full rung surface stays bypassed, identical to
        // `with_exempt_tools`.
        for _ in 0..10 {
            assert_eq!(t.observe("poll_status", "{}"), GuardrailDecision::Allow);
            assert_eq!(ok(&mut t, "poll_status", "same"), GuardrailDecision::Allow);
        }
    }

    // ── iter-688: output-side successful-repeat rung (`record_output`) ──

    #[test]
    fn identical_outputs_warn_at_four_and_arm_skip_at_five() {
        let mut t = ToolGuardrailTracker::new();
        let sig = 42u64;
        for i in 1..=3 {
            assert_eq!(
                t.observe_output(sig, true, false),
                GuardrailDecision::Allow,
                "iterations 1-3 stay quiet (openhuman: no verdict before the threshold)"
            );
            let _ = i;
        }
        assert_eq!(
            t.observe_output(sig, true, false),
            GuardrailDecision::Warn,
            "4th identical output warns (DEFAULT_REPEAT_OUTPUT_THRESHOLD)"
        );
        assert_eq!(
            t.observe_output(sig, true, false),
            GuardrailDecision::Warn,
            "5th warns again and arms the skip backstop"
        );
        assert_eq!(
            t.observe("anything", "{\"different\":1}"),
            GuardrailDecision::Skip,
            "an armed output streak skips the next call regardless of identity"
        );
    }

    #[test]
    fn changed_output_resets_the_streak() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..3 {
            let _ = t.observe_output(1, true, false);
        }
        let _ = t.observe_output(2, true, false);
        for _ in 0..2 {
            assert_eq!(
                t.observe_output(2, true, false),
                GuardrailDecision::Allow,
                "a changed output starts a fresh streak (3 total, below the threshold)"
            );
        }
    }

    #[test]
    fn failed_or_exempt_batches_reset_the_output_streak() {
        // openhuman `record_call_batch`: output is observed before the
        // completed batch can be classified, so a failed or exempt batch
        // resets the streak — its output cannot leak into the next
        // progress-eligible iteration.
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..3 {
            let _ = t.observe_output(1, true, false);
        }
        let _ = t.observe_output(1, false, false);
        for _ in 0..3 {
            assert_eq!(
                t.observe_output(1, true, false),
                GuardrailDecision::Allow,
                "a failed prior batch must not count toward a successful output loop"
            );
        }
        let _ = t.observe_output(1, true, true);
        for _ in 0..3 {
            assert_eq!(
                t.observe_output(1, true, false),
                GuardrailDecision::Allow,
                "an exempt (polling) batch must not count either"
            );
        }
    }

    #[test]
    fn exempt_tools_still_bypass_an_armed_output_streak() {
        let mut t = ToolGuardrailTracker::new().with_exempt_tools(["wait_poll"]);
        for _ in 0..5 {
            let _ = t.observe_output(1, true, false);
        }
        assert!(t.output_armed, "test helper: streak must be armed");
        assert_eq!(
            t.observe("wait_poll", "{}"),
            GuardrailDecision::Allow,
            "exempt tools bypass every rung, including the output backstop"
        );
        assert_eq!(
            t.observe("non_exempt", "{}"),
            GuardrailDecision::Skip,
            "non-exempt tools are still skipped"
        );
    }

    #[test]
    fn armed_output_streak_disarms_on_a_failed_or_exempt_batch() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..5 {
            let _ = t.observe_output(1, true, false);
        }
        assert!(t.output_armed);
        let _ = t.observe_output(1, false, false);
        assert!(!t.output_armed, "a failed batch clears the backstop");
        assert_eq!(
            t.observe("anything", "{}"),
            GuardrailDecision::Allow,
            "and the next call proceeds"
        );
    }

    #[test]
    fn reset_clears_the_output_streak_and_backstop() {
        let mut t = ToolGuardrailTracker::new();
        for _ in 0..5 {
            let _ = t.observe_output(1, true, false);
        }
        t.reset();
        assert_eq!(
            t.observe("anything", "{}"),
            GuardrailDecision::Allow,
            "reset disarms the output backstop"
        );
        assert_eq!(
            t.observe_output(1, true, false),
            GuardrailDecision::Allow,
            "reset clears the streak count"
        );
    }
}
