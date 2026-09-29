//! Turn-end heuristics — detect cut-off / truncated model responses and
//! decide whether the loop should request a continuation instead of
//! surfacing a partial answer as final.
//!
//! Ported from hermes-agent `run_agent.py` (`_has_natural_response_ending`,
//! `_has_content_after_think_block`, `_should_treat_stop_as_truncated`,
//! `_is_ollama_glm_backend`) and `agent/conversation_loop.py`
//! (`_get_continuation_prompt`).
//!
//! Also holds the deterministic tool-loop anomaly detectors
//! ([`Anomaly`], [`detect_anomalies`]) and the capped retry feedback
//! ([`anomaly_retry_feedback`]). The detectors are pure string analysis so
//! they are unit-testable in isolation; the *injection* reuses the existing
//! continuation shape in `agent/run.rs` (append a user `Message`, refund the
//! iteration, `continue`) — there is no second loop.

use std::fmt;
/// Opening markers of reasoning/thinking blocks (must stay in sync with
/// [`strip_think_blocks`] and [`thinking_exhausted`]).
const THINK_OPENERS: &[&str] = &["<think", "<thinking", "<reasoning", "<REASONING_SCRATCHPAD"];

/// Closing markers paired with [`THINK_OPENERS`].
const THINK_CLOSERS: &[&str] = &[
    "</think>",
    "</thinking>",
    "</reasoning>",
    "</REASONING_SCRATCHPAD>",
];

/// Remove all reasoning/thinking blocks from `content`, returning the visible
/// remainder. A block is dropped from its opening tag to the first closing
/// tag after it; an unclosed opening tag drops the rest of the content.
pub fn strip_think_blocks(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    loop {
        // Earliest opening tag in the remaining text.
        let mut open_idx: Option<usize> = None;
        for tag in THINK_OPENERS {
            if let Some(i) = rest.find(tag) {
                open_idx = Some(open_idx.map_or(i, |cur| cur.min(i)));
            }
        }
        let Some(open) = open_idx else {
            break;
        };
        let after_open = &rest[open..];
        // First closing tag after the opener.
        let mut close: Option<(usize, usize)> = None; // (index, tag len)
        for tag in THINK_CLOSERS {
            if let Some(i) = after_open.find(tag) {
                close = Some(match close {
                    Some((cur, len)) if cur <= i => (cur, len),
                    _ => (i, tag.len()),
                });
            }
        }
        out.push_str(&rest[..open]);
        match close {
            Some((i, len)) => rest = &after_open[i + len..],
            // No closing tag — drop the tail (unterminated think block).
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether `content` has meaningful (non-whitespace) text after any
/// reasoning/thinking blocks. Detects the case where the model only emitted
/// reasoning and no actual response — an incomplete generation.
pub fn has_content_after_think_block(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    !strip_think_blocks(content).trim().is_empty()
}

/// Heuristic: does visible assistant text look intentionally finished?
/// Mirrors hermes `_has_natural_response_ending` (punctuation, emoji,
/// code-fence, caret).
pub fn has_natural_response_ending(content: &str) -> bool {
    let stripped = content.trim_end();
    if stripped.is_empty() {
        return false;
    }
    if stripped.ends_with("```") || stripped.ends_with('^') {
        return true;
    }
    let Some(last) = stripped.chars().last() else {
        return false;
    };
    if ".!?:)\"']}。！？：）】」』》^".contains(last) {
        return true;
    }
    // Emoji / symbols ranges (Misc Symbols, Dingbats, Emoticons, Supplemental).
    let cp = last as u32;
    (0x1F300..=0x1FAFF).contains(&cp) || (0x2600..=0x27BF).contains(&cp)
}

/// Whether the response burned the entire output budget on reasoning with
/// nothing visible left — continuation retries are pointless, surface a
/// targeted error instead (hermes conversation_loop.py thinking-exhausted).
pub fn thinking_exhausted(content: &str) -> bool {
    let lower = content.to_lowercase();
    let has_think_tags = THINK_OPENERS
        .iter()
        .any(|t| lower.contains(&t.to_lowercase()));
    has_think_tags && !has_content_after_think_block(content)
}

/// Conservative stop→truncated misreport detection (hermes
/// `_should_treat_stop_as_truncated` + `_is_ollama_glm_backend`).
///
/// Ollama-hosted GLM models can misreport truncated output as
/// `finish_reason="stop"`. Hermes gates this on explicit Ollama signatures;
/// operant has no base_url at agent level, so we gate on the model name only
/// — the natural-ending check below is the real guard against false
/// positives on well-behaved proxies (LiteLLM/sglang/vLLM report
/// finish_reason correctly).
pub fn should_treat_stop_as_truncated(
    model: &str,
    finish_reason: Option<&str>,
    content: &str,
    has_tool_messages: bool,
    has_tool_calls: bool,
) -> bool {
    if finish_reason != Some("stop") {
        return false;
    }
    if !model.to_lowercase().contains("glm") {
        return false;
    }
    if !has_tool_messages || has_tool_calls {
        return false;
    }
    let visible = strip_think_blocks(content).trim().to_string();
    if visible.is_empty() {
        return false;
    }
    if visible.chars().count() < 20 || !visible.contains(char::is_whitespace) {
        return false;
    }
    !has_natural_response_ending(&visible)
}

/// Continuation prompt appended as a user message when a response was cut
/// off by the output length limit (hermes `_get_continuation_prompt`).
pub fn continuation_prompt() -> &'static str {
    "[System: Your previous response was truncated by the output \
     length limit. Continue exactly where you left off. Do not \
     restart or repeat prior text. Finish the answer directly.]"
}

// ── Deterministic tool-loop anomaly detectors ──────────────────
//
// A turn can "look" finished while resting on a tool result the model
// could not have used: an empty body, a payload that claims to be JSON
// but does not parse, or an error-class body. Each case is a
// deterministic string predicate — no model call, no heuristics that
// drift with phrasing. The caller injects a feedback message through
// the SAME continuation shape used for truncation, capped by
// `MAX_ANOMALY_RETRIES`.
//
// ponytail: the error marker lists are fixed sets. Extend them when a
// new tool reports failures in a shape they do not cover.

/// Cap on anomaly-driven retries within a single turn (mirrors the
/// `MAX_LENGTH_CONTINUE_RETRIES` cap on truncation continuations).
pub const MAX_ANOMALY_RETRIES: usize = 3;

/// Prefix every injected anomaly-retry message starts with. Used both as
/// the marker in [`anomaly_retry_prompt`] and as the key for counting
/// already-injected retries in the message list, so the turn needs no
/// extra loop state.
pub const ANOMALY_RETRY_PREFIX: &str = "[System: tool-result anomaly";

/// Leading markers of an error-class payload. Matched against the START of
/// the first non-empty line, lowercased — not a substring search, so prose
/// that merely discusses errors does not trip the detector.
const ERROR_PREFIXES: &[&str] = &[
    "error:",
    "fatal:",
    "panic:",
    "traceback",
    "exception:",
    "assertionerror",
    "nullpointerexception",
    "error ",
];

/// Error phrases a shell embeds mid-line (`bash: line 1: ls: command not
/// found`), so these are matched anywhere in the first non-empty line.
const ERROR_SUBSTRINGS: &[&str] = &["command not found", "no such file", "permission denied"];

/// A deterministic anomaly observed in a turn's tool batch or response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Anomaly {
    /// A tool result body was empty or whitespace only.
    EmptyToolResult,
    /// A tool result body looks like a JSON object/array but fails to parse.
    MalformedToolOutput,
    /// An error-class payload appeared in a tool result or in the response.
    ErrorSignal,
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::EmptyToolResult => "empty tool result",
            Self::MalformedToolOutput => "unparseable tool output",
            Self::ErrorSignal => "error-class signal",
        };
        f.write_str(s)
    }
}

/// First non-empty line of `body`, lowercased. Returns `""` when the body
/// has no content.
fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

/// True when `body` carries an error-class payload: its first non-empty
/// line starts with one of [`ERROR_PREFIXES`] or contains one of
/// [`ERROR_SUBSTRINGS`].
pub fn has_error_signal(body: &str) -> bool {
    let line = first_line(body);
    if line.is_empty() {
        return false;
    }
    ERROR_PREFIXES.iter().any(|p| line.starts_with(p))
        || ERROR_SUBSTRINGS.iter().any(|s| line.contains(s))
}

/// True when `body` is empty or whitespace only.
pub fn is_empty_tool_result(body: &str) -> bool {
    body.trim().is_empty()
}

/// True when `body` is shaped like JSON (starts with `{` or `[`) but does
/// not parse. A tool that answers with malformed JSON has failed, and the
/// model cannot build an answer on it — the correctness check the
/// prior art calls a "tool-output schema check".
pub fn is_malformed_tool_output(body: &str) -> bool {
    let trimmed = body.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(trimmed).is_err()
}

/// Run every detector over the trailing tool batch of a turn plus the
/// assistant response. Returns anomalies in a stable, deduplicated order
/// (enum declaration order), so the retry prompt is deterministic.
pub fn detect_anomalies(tool_bodies: &[&str], response_text: &str) -> Vec<Anomaly> {
    let mut found = Vec::new();
    let mut push = |a: Anomaly| {
        if !found.contains(&a) {
            found.push(a);
        }
    };

    for body in tool_bodies {
        if is_empty_tool_result(body) {
            push(Anomaly::EmptyToolResult);
        }
        if is_malformed_tool_output(body) {
            push(Anomaly::MalformedToolOutput);
        }
        if has_error_signal(body) {
            push(Anomaly::ErrorSignal);
        }
    }
    if has_error_signal(response_text) {
        push(Anomaly::ErrorSignal);
    }
    found
}

/// Count anomaly-retry messages already present in the turn's message
/// list. The injected messages are the state — no parallel counter to
/// keep in sync with the conversation.
pub fn count_injected_retries<'a>(contents: impl IntoIterator<Item = &'a str>) -> usize {
    contents
        .into_iter()
        .filter(|c| c.trim_start().starts_with(ANOMALY_RETRY_PREFIX))
        .count()
}

/// The feedback message for a detected anomaly batch, or `None` when the
/// turn has already spent its budget (`injected >= MAX_ANOMALY_RETRIES`).
/// `injected` is the value [`count_injected_retries`] returned for the
/// current message list.
pub fn anomaly_retry_prompt(anomalies: &[Anomaly], injected: usize) -> Option<String> {
    if anomalies.is_empty() || injected >= MAX_ANOMALY_RETRIES {
        return None;
    }
    let listed: Vec<String> = anomalies.iter().map(ToString::to_string).collect();
    Some(format!(
        "{ANOMALY_RETRY_PREFIX} (attempt {}/{}) — {}. \
         The tool results you were given are not usable as-is. \
         Re-run the affected tool with corrected arguments or a different \
         tool; if it still fails, answer with what you can verify and state \
         plainly what you could not determine. Do not repeat the same call \
         with the same arguments unchanged.",
        injected + 1,
        MAX_ANOMALY_RETRIES,
        listed.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_think_blocks_removes_single_block() {
        let out = strip_think_blocks("Before <think>hidden</think> after");
        assert_eq!(out, "Before  after");
    }

    #[test]
    fn strip_think_blocks_handles_variants() {
        let out = strip_think_blocks("<thinking>a</thinking><reasoning>b</reasoning>visible");
        assert_eq!(out, "visible");
    }

    #[test]
    fn strip_think_blocks_unclosed_drops_tail() {
        let out = strip_think_blocks("head <think>never closed");
        assert_eq!(out, "head ");
    }

    #[test]
    fn strip_think_blocks_no_tags_passthrough() {
        let out = strip_think_blocks("plain text");
        assert_eq!(out, "plain text");
    }

    #[test]
    fn has_content_after_think_block_false_when_reasoning_only() {
        assert!(!has_content_after_think_block(
            "<think>all thinking</think>"
        ));
        assert!(!has_content_after_think_block("<thinking>  </thinking>   "));
    }

    #[test]
    fn has_content_after_think_block_true_with_visible_text() {
        assert!(has_content_after_think_block("<think>t</think>the answer"));
    }

    #[test]
    fn natural_ending_punctuation() {
        assert!(has_natural_response_ending("the answer."));
        assert!(has_natural_response_ending("Done!"));
        assert!(has_natural_response_ending("closing ```"));
    }

    #[test]
    fn natural_ending_mid_sentence_is_not_natural() {
        assert!(!has_natural_response_ending("the answer"));
        assert!(!has_natural_response_ending("implementing the"));
    }

    #[test]
    fn thinking_exhausted_detects_reasoning_only() {
        assert!(thinking_exhausted(
            "<thinking>long reasoning block</thinking>"
        ));
        assert!(!thinking_exhausted("<thinking>r</thinking>visible answer"));
        assert!(!thinking_exhausted("no tags at all"));
    }

    #[test]
    fn stop_truncated_requires_glm_and_no_natural_ending() {
        let content = "The refactor is incomplete because the trait bounds are still";
        assert!(should_treat_stop_as_truncated(
            "glm-4.7",
            Some("stop"),
            content,
            true,
            false
        ));
        // Not a GLM model → never treat stop as truncated.
        assert!(!should_treat_stop_as_truncated(
            "gpt-4",
            Some("stop"),
            content,
            true,
            false
        ));
        // Natural ending → not truncated.
        assert!(!should_treat_stop_as_truncated(
            "glm-4.7",
            Some("stop"),
            "The refactor is incomplete because of trait bounds.",
            true,
            false
        ));
        // finish_reason=length is handled by the caller, not this check.
        assert!(!should_treat_stop_as_truncated(
            "glm-4.7",
            Some("length"),
            content,
            true,
            false
        ));
        // Tool calls present → not a truncation.
        assert!(!should_treat_stop_as_truncated(
            "glm-4.7",
            Some("stop"),
            content,
            true,
            true
        ));
    }

    #[test]
    fn continuation_prompt_guides_continuation() {
        let p = continuation_prompt();
        assert!(p.contains("truncated"));
        assert!(p.contains("Continue exactly where you left off"));
    }

    // ── Anomaly detectors ───────────────────────────────────────

    #[test]
    fn empty_tool_result_detector() {
        assert!(is_empty_tool_result(""));
        assert!(is_empty_tool_result("   \n\t "));
        assert!(!is_empty_tool_result("ok"));
        assert!(!is_empty_tool_result("{}"));
    }

    #[test]
    fn malformed_tool_output_detector() {
        // JSON-shaped but unparseable → anomaly.
        assert!(is_malformed_tool_output(r#"{"a": 1"#));
        assert!(is_malformed_tool_output("  [1, 2,"));
        // Valid JSON → clean.
        assert!(!is_malformed_tool_output(r#"{"a": 1}"#));
        assert!(!is_malformed_tool_output("[1, 2, 3]"));
        // Free-form text that is not claiming to be JSON → not this detector.
        assert!(!is_malformed_tool_output("plain output"));
        assert!(!is_malformed_tool_output("key: value"));
    }

    #[test]
    fn error_signal_detector() {
        assert!(has_error_signal("Error: file not found"));
        assert!(has_error_signal("\n\n  FATAL: out of memory"));
        assert!(has_error_signal("Traceback (most recent call last):"));
        assert!(has_error_signal("bash: command not found"));
        // Prose that merely mentions an error must NOT trip it.
        assert!(!has_error_signal("The error handling is done in step 2."));
        assert!(!has_error_signal("all good"));
        assert!(!has_error_signal(""));
    }

    #[test]
    fn detect_anomalies_fires_per_class() {
        assert_eq!(
            detect_anomalies(&[""], "done"),
            vec![Anomaly::EmptyToolResult]
        );
        assert_eq!(
            detect_anomalies(&["{\"a\":"], "done"),
            vec![Anomaly::MalformedToolOutput]
        );
        assert_eq!(
            detect_anomalies(&["Error: nope"], "done"),
            vec![Anomaly::ErrorSignal]
        );
        // Error signal in the RESPONSE, not the tool body.
        assert_eq!(
            detect_anomalies(&["ok"], "Error: I could not read the file"),
            vec![Anomaly::ErrorSignal]
        );
        // Multiple classes → stable declaration order, deduplicated.
        assert_eq!(
            detect_anomalies(&["", "Error: x", "Error: y"], "ok"),
            vec![Anomaly::EmptyToolResult, Anomaly::ErrorSignal]
        );
    }

    #[test]
    fn detect_anomalies_silent_on_clean_turn() {
        let clean = [
            "42",
            r#"{"rows": 3}"#,
            "the file contains three lines of text",
        ];
        assert!(detect_anomalies(&clean, "Here is the answer.").is_empty());
        // No tool calls at all — nothing to judge.
        assert!(detect_anomalies(&[], "Here is the answer.").is_empty());
    }

    #[test]
    fn anomaly_retry_cap_is_pinned() {
        assert_eq!(MAX_ANOMALY_RETRIES, 3);
    }

    #[test]
    fn anomaly_retry_prompt_stops_at_cap() {
        let anomalies = [Anomaly::EmptyToolResult];
        for injected in 0..MAX_ANOMALY_RETRIES {
            let p = anomaly_retry_prompt(&anomalies, injected).expect("under cap → inject");
            assert!(p.starts_with(ANOMALY_RETRY_PREFIX));
            assert!(p.contains(&format!("attempt {}/", injected + 1)));
        }
        // Cap reached → no message, so the loop cannot spin forever.
        assert!(anomaly_retry_prompt(&anomalies, MAX_ANOMALY_RETRIES).is_none());
        assert!(anomaly_retry_prompt(&anomalies, MAX_ANOMALY_RETRIES + 5).is_none());
        // Nothing detected → no message.
        assert!(anomaly_retry_prompt(&[], 0).is_none());
    }

    #[test]
    fn count_injected_retries_reads_message_contents() {
        let first = anomaly_retry_prompt(&[Anomaly::EmptyToolResult], 0).unwrap();
        let second = anomaly_retry_prompt(&[Anomaly::ErrorSignal], 1).unwrap();
        let messages = vec![
            "an ordinary user message",
            first.as_str(),
            "another ordinary message",
            second.as_str(),
        ];
        assert_eq!(count_injected_retries(messages), 2);
        assert_eq!(count_injected_retries(["nothing injected"]), 0);
    }

    #[test]
    fn anomaly_retry_prompt_lists_every_anomaly() {
        let p = anomaly_retry_prompt(
            &[
                Anomaly::EmptyToolResult,
                Anomaly::MalformedToolOutput,
                Anomaly::ErrorSignal,
            ],
            0,
        )
        .unwrap();
        assert!(p.contains("empty tool result"));
        assert!(p.contains("unparseable tool output"));
        assert!(p.contains("error-class signal"));
    }
}
