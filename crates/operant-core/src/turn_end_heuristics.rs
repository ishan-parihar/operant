//! Turn-end heuristics — detect cut-off / truncated model responses and
//! decide whether the loop should request a continuation instead of
//! surfacing a partial answer as final.
//!
//! Ported from hermes-agent `run_agent.py` (`_has_natural_response_ending`,
//! `_has_content_after_think_block`, `_should_treat_stop_as_truncated`,
//! `_is_ollama_glm_backend`) and `agent/conversation_loop.py`
//! (`_get_continuation_prompt`).

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
}
