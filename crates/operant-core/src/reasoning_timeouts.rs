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
    // The 4.0 generation ships as a DATED id (`claude-sonnet-4-20250514`),
    // which no version-anchored needle could match — the live 2026-11-12
    // audit ran on exactly that id and got no floor at all.
    ("claude-opus-4", 240),
    ("claude-sonnet-4", 240),
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
    // dots-studio preview reasoning (emits reasoning_content — iter-779
    // salvage proves the thinking path is live; free-tier TTFB routinely
    // exceeds the 120s configured ceiling, killing cron seat runs at the
    // first LLM call — iter-784).
    ("dots-3-note-preview", 300),
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
            //
            // `:` and `+` count as separators because aggregator ids carry a
            // VARIANT suffix (`dots-studio/dots-3-note-preview:free`,
            // `openai/o3-mini:online`). Without them the floor never matched
            // the id the operator actually configured — the `dots-3-note-preview`
            // entry (added because "cron seat 120s kills") was dead for every
            // OpenRouter-style id, and `openai/o3-mini:free` matched the
            // shorter `o3` needle instead of `o3-mini`.
            let end = pos + needle.len();
            if end < slug.len() && !matches!(bytes[end], b'-' | b'.' | b'_' | b'/' | b':' | b'+') {
                return false;
            }
            true
        })
        .max_by_key(|(needle, _)| needle.len())
        .map(|(_, floor)| *floor)
}

/// The operator's explicit per-model request timeout, if one is configured.
///
/// Consulted BEFORE the built-in floor table: the floor table's own doc says
/// it applies "only when no explicit user-configured per-model timeout
/// exists", so an explicit value wins in BOTH directions — the operator may
/// raise a budget for a slow model or lower it for a cheap one.
///
/// Matching is the full model id first (case-insensitive), then the slug
/// after the last `/` — so `large-stack` also covers `openrouter/large-stack`.
/// This is the only lever that can cover a LOCAL ALIAS: the floor table
/// matches vendor model ids, and an alias (`large-stack`, `small-stack`) is
/// a name the table can never see.
pub fn explicit_model_timeout(
    overrides: &std::collections::HashMap<String, std::time::Duration>,
    model: &str,
) -> Option<std::time::Duration> {
    if overrides.is_empty() || model.is_empty() {
        return None;
    }
    let model_lower = model.to_ascii_lowercase();
    let slug = model_lower
        .rsplit('/')
        .next()
        .unwrap_or(&model_lower)
        .to_string();
    let mut slug_match: Option<std::time::Duration> = None;
    for (key, budget) in overrides {
        let key_lower = key.to_ascii_lowercase();
        if key_lower == model_lower {
            return Some(*budget);
        }
        if slug_match.is_none() && key_lower == slug {
            slug_match = Some(*budget);
        }
    }
    slug_match
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
         1. Set `[client] timeout_secs: 900` — or, better, a per-model budget \
         (`[client] model_request_timeouts = {{ \"your-model-id\" = 900 }}`) — in \
         `~/.operant/operant.toml`. Operant's built-in floor for known reasoning \
         models is per-model (180s-600s), so the default is already raised for \
         those ids; this error means the upstream killed the connection anyway, \
         or the id carries a local alias the floor table cannot match.\n\
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

    /// An aggregator id carries a VARIANT suffix (`...:free`). The matcher
    /// used to stop at the `:`, so the `dots-3-note-preview` entry (added
    /// because "cron seat 120s kills") never applied to the id the operator
    /// actually configured.
    #[test]
    fn an_aggregator_variant_suffix_still_matches_the_floor() {
        assert_eq!(
            get_reasoning_stale_timeout_floor("dots-studio/dots-3-note-preview:free"),
            Some(300)
        );
        assert_eq!(
            get_reasoning_stale_timeout_floor("deepseek/deepseek-r1:free"),
            Some(600)
        );
        // The LONGER needle wins: `o3-mini` (300s), not `o3` (600s).
        assert_eq!(
            get_reasoning_stale_timeout_floor("openai/o3-mini:free"),
            Some(300)
        );
    }

    /// The dated Claude 4 id the live logs show (`claude-sonnet-4-20250514`)
    /// had no floor at all: the table only carried 4.5/4.6/5.
    #[test]
    fn the_dated_claude_4_id_has_a_floor() {
        assert_eq!(
            get_reasoning_stale_timeout_floor("claude-sonnet-4-20250514"),
            Some(240)
        );
    }

    /// The operator's explicit per-model budget — the only lever that can
    /// cover a LOCAL ALIAS, which the vendor-slug floor table can never
    /// match.
    #[test]
    fn an_explicit_per_model_budget_is_found() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "large-stack".to_string(),
            std::time::Duration::from_secs(600),
        );
        assert_eq!(
            explicit_model_timeout(&overrides, "large-stack"),
            Some(std::time::Duration::from_secs(600))
        );
        // The slug form covers a provider-prefixed id.
        assert_eq!(
            explicit_model_timeout(&overrides, "openrouter/large-stack"),
            Some(std::time::Duration::from_secs(600))
        );
        assert_eq!(explicit_model_timeout(&overrides, "gpt-4o"), None);
        assert_eq!(
            explicit_model_timeout(&std::collections::HashMap::new(), "large-stack"),
            None
        );
    }
}
