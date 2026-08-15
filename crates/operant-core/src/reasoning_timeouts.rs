//! Reasoning-model stale-timeout floors + thinking-timeout guidance — hermes
//! `agent/reasoning_timeouts.py` + `agent/thinking_timeout_guidance.py` parity (R2).
//!
//! Reasoning models (those that emit extended thinking blocks before their
//! first content token) routinely exceed the default chat-model timeouts:
//! a hosted reasoning model that thinks 120s+ before the first token gets
//! idle-killed by upstream proxies / load-balancers (NVIDIA NIM, OpenAI,
//! Anthropic, DeepSeek — all reproduce this), surfacing as a transport error
//! with no actionable guidance.
//!
//! Two pieces:
//!
//! 1. [`get_reasoning_stale_timeout_floor`] — a per-model FLOOR for the
//!    request/stale timeout. Callers apply it as `max(default, floor)` and
//!    only when no explicit user-configured per-model timeout exists, so it
//!    never overrides user config.
//! 2. [`is_thinking_timeout`] + [`build_thinking_timeout_guidance`] — detect
//!    the "killed mid-think" failure mode (transport error before the first
//!    content token on a known reasoning model) and give the user distinct,
//!    copy-pasteable guidance instead of the generic stream-drop message.

/// Reasoning-model stale-timeout floors: (slug-substring, floor-seconds).
/// Slugs are matched with word boundaries so `o1` doesn't match `olmo-1`.
/// Ported verbatim from hermes `_REASONING_STALE_TIMEOUT_FLOORS`.
const REASONING_FLOORS: &[(&str, u64)] = &[
    // NVIDIA Nemotron — 60-180s upstream idle kill documented (120s measured).
    ("nemotron-3-ultra", 600),
    ("nemotron-3-super", 600),
    ("nemotron-3-nano", 300),
    // DeepSeek R1 + V4 reasoning (V4 emits reasoning_content separately).
    ("deepseek-r1", 600),
    ("deepseek-reasoner", 600),
    ("deepseek-v4-flash", 600),
    ("deepseek-v4-pro", 600),
    // Qwen QwQ + Qwen3 thinking variants.
    ("qwq-32b", 300),
    ("qwen3", 180),
    // OpenAI o-series — multi-minute TTFB. Explicit variants only so bare
    // `o1` doesn't over-match `olmo-1`.
    ("o1", 600),
    ("o1-mini", 600),
    ("o1-pro", 600),
    ("o1-preview", 600),
    ("o3", 600),
    ("o3-pro", 600),
    ("o3-mini", 300),
    ("o4-mini", 300),
    // Anthropic Claude 4.x thinking variants (anchored at claude-opus-4).
    ("claude-opus-4", 240),
    ("claude-opus-5", 240),
    ("claude-sonnet-5", 180),
    ("claude-sonnet-4.5", 180),
    ("claude-sonnet-4.6", 180),
    // Anthropic Mythos-class named reasoning models.
    ("claude-fable", 600),
    // xAI Grok reasoning variants.
    ("grok-4-fast-reasoning", 300),
    ("grok-4.20-reasoning", 300),
    ("grok-4.5", 300),
    ("grok-4-fast-non-reasoning", 180),
];

/// Return the stale-timeout floor (seconds) for a known reasoning model.
///
/// Returns `None` when the model is not in the allowlist or the argument is
/// empty. Matching is word-boundary anchored on the lowercased model name
/// against the part after the last `/` (aggregator prefix preserved through
/// matching — the `/` is itself a boundary). This is a FLOOR: callers apply
/// it as `max(default, floor)`.
pub fn get_reasoning_stale_timeout_floor(model: &str) -> Option<u64> {
    let model = model.to_ascii_lowercase();
    if model.is_empty() {
        return None;
    }
    let slug = model.rsplit('/').next().unwrap_or(&model);
    let bytes = slug.as_bytes();
    let is_boundary = |i: usize| {
        if i == 0 {
            return true;
        }
        matches!(bytes[i - 1], b'-' | b'.' | b'_' | b'/')
    };
    // Match the LONGEST needle first (hermes sorts by descending slug
    // length), so `o3-mini` matches the 300s entry rather than the bare
    // `o3` 600s entry, and `grok-4-fast-reasoning` beats `grok-4.5`.
    REASONING_FLOORS
        .iter()
        .filter(|(needle, _)| {
            let Some(pos) = slug.find(needle) else {
                return false;
            };
            // Left boundary: at start, or preceded by a separator.
            if !is_boundary(pos) {
                return false;
            }
            // Right boundary: at end, or followed by a separator.
            let end = pos + needle.len();
            if end < slug.len() && !matches!(bytes[end], b'-' | b'.' | b'_' | b'/') {
                return false;
            }
            true
        })
        .max_by_key(|(needle, _)| needle.len())
        .map(|(_, floor)| *floor)
}

/// Transport-error substrings that indicate a mid-think upstream kill.
const THINKING_TIMEOUT_SUBSTRINGS: &[&str] = &[
    "broken pipe",
    "connection reset",
    "connection closed",
    "remote protocol error",
    "error decoding response body",
    "stream dropped",
    "unexpected eof",
    "connection refused",
    "reset by peer",
    "read timeout",
    "timed out",
];

/// Detect the thinking-timeout failure mode: a transport error on a known
/// reasoning model. Callers should only consult this when the failure
/// happened BEFORE the first content token arrived (the model was still in
/// its thinking phase) — that's what makes it a thinking-timeout rather than
/// a plain stream drop.
pub fn is_thinking_timeout(model: &str, error_msg: &str) -> bool {
    if get_reasoning_stale_timeout_floor(model).is_none() {
        return false;
    }
    let msg = error_msg.to_ascii_lowercase();
    THINKING_TIMEOUT_SUBSTRINGS
        .iter()
        .any(|needle| msg.contains(needle))
}

/// User-facing guidance for the thinking-timeout failure mode. Mirrors
/// hermes `build_thinking_timeout_guidance`, adapted to operant's config
/// layout (`~/.operant/operant.toml`).
pub fn build_thinking_timeout_guidance(model: &str) -> String {
    let label = model.rsplit('/').next().unwrap_or(model);
    format!(
        "\n\nThe model's thinking phase exceeded the upstream proxy's idle timeout \
         before the first content token arrived. This is a known issue with reasoning \
         models (like {label}) behind cloud gateways (NVIDIA NIM, OpenAI, Anthropic, \
         DeepSeek). Workarounds in priority order:\n\
         1. Set `provider.timeout_secs: 900` (or a per-model request timeout) in \
         `~/.operant/operant.toml` to extend the per-call timeout. Operant's built-in \
         floor for known reasoning models is 600s, so the default is already raised — \
         this error means the upstream killed the connection anyway.\n\
         2. Lower `reasoning_effort` to `medium`/`low` (or set `max_tokens` lower) to \
         shorten the thinking phase.\n\
         3. Use a smaller / faster reasoning model."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_reasoning_models_get_floors() {
        assert_eq!(
            get_reasoning_stale_timeout_floor("nvidia/nemotron-3-ultra-550b-a55b"),
            Some(600)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("openai/o3-mini"),
            Some(300)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("deepseek/deepseek-r1"),
            Some(600)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("deepseek/deepseek-v4-flash"),
            Some(600)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("qwen/qwen3-235b-a22b-thinking"),
            Some(180)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("x-ai/grok-4-fast-reasoning"),
            Some(300)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("anthropic/claude-opus-4-6"),
            Some(240)
        );
    }

    #[test]
    fn non_reasoning_models_get_no_floor() {
        assert_eq!(get_reasoning_stale_timeout_floor("gpt-4o"), None);
        assert_eq!(get_reasoning_stale_timeout_floor("olmo-1"), None); // o1 substring must NOT match
        assert_eq!(get_reasoning_stale_timeout_floor(""), None);
        assert_eq!(get_reasoning_stale_timeout_floor("grok-4"), None); // bare grok-4 doesn't match
        assert_eq!(
            get_reasoning_stale_timeout_floor("deepseek/deepseek-chat"),
            None
        );
    }

    #[test]
    fn thinking_timeout_detection() {
        assert!(is_thinking_timeout(
            "deepseek/deepseek-v4-flash",
            "error decoding response body: connection reset by peer"
        ));
        assert!(!is_thinking_timeout(
            "gpt-4o",
            "error decoding response body"
        ));
        assert!(!is_thinking_timeout(
            "deepseek/deepseek-v4-flash",
            "context length exceeded"
        ));
    }

    #[test]
    fn guidance_mentions_config_path_and_model() {
        let g = build_thinking_timeout_guidance("nvidia/nemotron-3-ultra-550b-a55b");
        assert!(g.contains("nemotron-3-ultra-550b-a55b"));
        assert!(g.contains("~/.operant/operant.toml"));
        assert!(g.contains("600s"));
    }
}
