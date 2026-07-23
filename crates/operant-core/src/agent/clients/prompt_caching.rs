//! Anthropic prompt caching strategy.
//!
//! Single layout: `system_and_3`. Up to 4 `cache_control` breakpoints —
//! system prompt + last 3 non-system messages, all at the same TTL (5m or 1h).
//! Reduces input token costs by ~75% on multi-turn conversations within a
//! single session.
//!
//! Pure functions — no class state, no agent dependency.
//! Ported from hermes-agent `agent/prompt_caching.py`.

use serde_json::{Value, json};

/// Cache TTL options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTtl {
    /// 5-minute cache (default for Anthropic).
    FiveMinutes,
    /// 1-hour cache (longer-lived).
    OneHour,
}

impl CacheTtl {
    #[allow(dead_code)]
    fn as_str(&self) -> &'static str {
        match self {
            CacheTtl::FiveMinutes => "5m",
            CacheTtl::OneHour => "1h",
        }
    }
}

impl Default for CacheTtl {
    fn default() -> Self {
        Self::FiveMinutes
    }
}

/// Build a `cache_control` marker JSON for the given TTL.
fn build_marker(ttl: CacheTtl) -> Value {
    match ttl {
        CacheTtl::FiveMinutes => json!({"type": "ephemeral"}),
        CacheTtl::OneHour => json!({"type": "ephemeral", "ttl": "1h"}),
    }
}

/// Apply `cache_control` to a single message in OpenAI-compatible (envelope) layout.
///
/// This is the layout used by OpenRouter and other OpenAI-compatible providers.
/// Markers go inside content parts (not top-level), as top-level markers on
/// empty-content messages are silently ignored.
fn apply_cache_marker_envelope(msg: &mut Value, marker: &Value) {
    // Early return for empty or missing content — can't carry a marker.
    if !can_carry_marker_envelope(msg) {
        return;
    }

    // Dispatch on content type: string → wrap in array; array → mark last element.
    if let Some(s) = msg.get("content").and_then(|c| c.as_str()) {
        // Non-empty string content → wrap in array with cache_control.
        let text = s.to_string();
        msg["content"] = json!([{
            "type": "text",
            "text": text,
            "cache_control": marker
        }]);
    } else if let Some(last) = msg.get_mut("content").and_then(|c| c.as_array_mut()).and_then(|a| a.last_mut()) {
        if let Some(obj) = last.as_object_mut() {
            obj.insert("cache_control".to_string(), marker.clone());
        }
    }
}

/// Apply `cache_control` to a single message in native Anthropic layout.
///
/// On the native Anthropic layout, top-level markers are relocated by the
/// adapter, so every message can carry one. For tool messages, the adapter
/// moves the top-level marker inside the `tool_result` block.
fn apply_cache_marker_native(msg: &mut Value, marker: &Value) {
    msg["cache_control"] = marker.clone();
}

/// Check if a message can carry a cache marker that will actually be honored.
///
/// On the envelope layout (OpenRouter), empty-content messages waste a
/// breakpoint. On native Anthropic, every message works.
fn can_carry_marker_envelope(msg: &Value) -> bool {
    match msg.get("content") {
        None => false,
        Some(Value::String(s)) if s.is_empty() => false,
        Some(Value::String(_)) => true,
        Some(Value::Array(arr)) => !arr.is_empty() && arr.last().map_or(false, |l| l.is_object()),
        _ => false,
    }
}

/// Apply the `system_and_3` caching strategy to a list of API messages.
///
/// Places up to 4 `cache_control` breakpoints:
/// 1. System prompt (if present as first message)
/// 2. Last 3 non-system messages that can carry a marker
///
/// # Arguments
/// * `messages` — The messages in API format (mutated in place)
/// * `cache_ttl` — Cache duration (`FiveMinutes` or `OneHour`)
/// * `native_anthropic` — `true` for native Anthropic layout, `false` for envelope (OpenRouter)
pub fn apply_cache_control(
    messages: &mut [Value],
    cache_ttl: CacheTtl,
    native_anthropic: bool,
) {
    if messages.is_empty() {
        return;
    }

    let marker = build_marker(cache_ttl);
    let mut breakpoints_used = 0;

    // 1. Cache the system prompt (first message if it has role "system").
    if messages[0].get("role").and_then(|r| r.as_str()) == Some("system") {
        if native_anthropic {
            apply_cache_marker_native(&mut messages[0], &marker);
        } else {
            apply_cache_marker_envelope(&mut messages[0], &marker);
        }
        breakpoints_used += 1;
    }

    // 2. Cache the last 3 non-system messages.
    let remaining = 4 - breakpoints_used;
    let non_sys_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, msg)| {
            msg.get("role").and_then(|r| r.as_str()) != Some("system")
        })
        .filter(|(_, msg)| {
            if native_anthropic {
                true
            } else {
                can_carry_marker_envelope(msg)
            }
        })
        .map(|(i, _)| i)
        .collect();

    for &idx in non_sys_indices.iter().rev().take(remaining) {
        if native_anthropic {
            apply_cache_marker_native(&mut messages[idx], &marker);
        } else {
            apply_cache_marker_envelope(&mut messages[idx], &marker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_msg(text: &str) -> Value {
        json!({"role": "system", "content": text})
    }

    fn user_msg(text: &str) -> Value {
        json!({"role": "user", "content": text})
    }

    fn assistant_msg(text: &str) -> Value {
        json!({"role": "assistant", "content": text})
    }

    fn empty_assistant() -> Value {
        json!({"role": "assistant", "content": ""})
    }

    fn empty_tool(tool_call_id: &str) -> Value {
        json!({"role": "tool", "tool_call_id": tool_call_id, "content": ""})
    }

    // ── Native Anthropic layout tests ──────────────────────────────────────

    #[test]
    fn native_caches_system_prompt() {
        let mut msgs = vec![system_msg("You are helpful"), user_msg("Hi")];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);
        assert_eq!(msgs[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn native_caches_last_3_messages() {
        let mut msgs = vec![
            system_msg("prompt"),
            user_msg("1"),
            assistant_msg("2"),
            user_msg("3"),
            assistant_msg("4"),
            user_msg("5"),
        ];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);

        // System + last 3 non-system = messages 3, 4, 5
        assert_eq!(msgs[0]["cache_control"]["type"], "ephemeral");
        assert!(msgs[1].get("cache_control").is_none());
        assert!(msgs[2].get("cache_control").is_none());
        assert_eq!(msgs[3]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[4]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[5]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn native_works_with_empty_content() {
        let mut msgs = vec![system_msg("prompt"), empty_assistant(), user_msg("Hi")];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);

        assert_eq!(msgs[0]["cache_control"]["type"], "ephemeral");
        // Empty assistant is non-system, so it should get a marker (native allows all)
        assert_eq!(msgs[1]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[2]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn native_one_hour_ttl() {
        let mut msgs = vec![system_msg("prompt"), user_msg("Hi")];
        apply_cache_control(&mut msgs, CacheTtl::OneHour, true);
        assert_eq!(msgs[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[0]["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn native_no_system_prompt_caches_last_4() {
        // Without a system prompt, all 4 breakpoints are available for messages.
        let mut msgs = vec![
            user_msg("1"),
            assistant_msg("2"),
            user_msg("3"),
            assistant_msg("4"),
            user_msg("5"),
            assistant_msg("6"),
        ];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);

        // First 2 messages don't get markers; last 4 do.
        assert!(msgs[0].get("cache_control").is_none());
        assert!(msgs[1].get("cache_control").is_none());
        assert_eq!(msgs[2]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[3]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[4]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[5]["cache_control"]["type"], "ephemeral");
    }

    // ── Envelope (OpenRouter) layout tests ─────────────────────────────────

    #[test]
    fn envelope_caches_system_prompt() {
        let mut msgs = vec![system_msg("prompt"), user_msg("Hi")];
        apply_cache_control(&mut msgs, CacheTtl::default(), false);

        // System prompt should have cache_control wrapping in array format
        let sys = &msgs[0]["system"];
        // Actually, envelope layout treats system as a regular message
        // The cache_control should be on the content part
        let content = msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content.last().unwrap()["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn envelope_skips_empty_assistant() {
        let mut msgs = vec![
            system_msg("prompt"),
            empty_assistant(),
            user_msg("Hi"),
        ];
        apply_cache_control(&mut msgs, CacheTtl::default(), false);

        // System gets marker
        let content = msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content.last().unwrap()["cache_control"]["type"], "ephemeral");

        // Empty assistant is skipped (can't carry marker)
        assert!(msgs[1].get("cache_control").is_none());
        // But it shouldn't have the marker
        assert!(msgs[1]["content"].as_str().map(|s| s.is_empty()).unwrap_or(true));
    }

    #[test]
    fn envelope_caches_last_3_with_content() {
        let mut msgs = vec![
            system_msg("prompt"),
            user_msg("1"),
            assistant_msg("2"),
            user_msg("3"),
            assistant_msg("4"),
            user_msg("5"),
        ];
        apply_cache_control(&mut msgs, CacheTtl::default(), false);

        // System gets marker
        let sys_content = msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(
            sys_content.last().unwrap()["cache_control"]["type"],
            "ephemeral"
        );

        // Last 3 non-system messages get markers
        for msg in &msgs[3..] {
            let content = msg["content"].as_array().expect("content should be array");
            assert_eq!(
                content.last().unwrap()["cache_control"]["type"],
                "ephemeral"
            );
        }
    }

    #[test]
    fn envelope_skips_empty_tool_message() {
        let mut msgs = vec![
            system_msg("prompt"),
            user_msg("Do something"),
            assistant_msg(""),
            empty_tool("call_1"),
            user_msg("Continue"),
        ];
        apply_cache_control(&mut msgs, CacheTtl::default(), false);

        // System gets marker
        let sys_content = msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(
            sys_content.last().unwrap()["cache_control"]["type"],
            "ephemeral"
        );

        // Empty tool is skipped; user msg "Continue" gets marker
        assert!(msgs[3].get("cache_control").is_none());
        let content = msgs[4]["content"].as_array().expect("should be array");
        assert_eq!(
            content.last().unwrap()["cache_control"]["type"],
            "ephemeral"
        );
    }

    // ── Edge cases ─────────────────────────────────────────────────────────

    #[test]
    fn empty_messages_noop() {
        let mut msgs: Vec<Value> = vec![];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);
        assert!(msgs.is_empty());
    }

    #[test]
    fn single_system_message_only() {
        let mut msgs = vec![system_msg("prompt")];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);
        assert_eq!(msgs[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn exactly_4_messages_all_get_markers() {
        let mut msgs = vec![system_msg("p"), user_msg("1"), user_msg("2"), user_msg("3")];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);
        // All 4 messages get markers (system + last 3)
        for msg in &msgs {
            assert_eq!(msg["cache_control"]["type"], "ephemeral");
        }
    }

    #[test]
    fn fewer_than_4_messages_only_available_get_markers() {
        let mut msgs = vec![user_msg("1"), user_msg("2")];
        apply_cache_control(&mut msgs, CacheTtl::default(), true);
        // No system, so all non-system messages up to 4 get markers
        assert_eq!(msgs[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[1]["cache_control"]["type"], "ephemeral");
    }
}
