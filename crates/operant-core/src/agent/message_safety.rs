//! Message and tool-payload sanitization helpers.
//!
//! Pure functions extracted for the agent's message pipeline. These walk
//! OpenAI-format message lists and structured payloads, repairing or
//! stripping problematic characters that would otherwise crash JSON
//! serialization or be rejected by upstream APIs.
//!
//! Ported from `hermes-agent/agent/message_sanitization.py`.

use regex::Regex;
use std::sync::LazyLock;

use crate::client::{Message, Role};

/// Regex to strip trailing commas before `}` or `]` in JSON.
static TRAILING_COMMA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",\s*([}\]])").unwrap());

// ---------------------------------------------------------------------------
// Surrogate sanitization
// ---------------------------------------------------------------------------



// ---------------------------------------------------------------------------
// Tool call argument repair
// ---------------------------------------------------------------------------

/// Attempt to repair malformed tool_call argument JSON.
///
/// Models like GLM-5.1 via Ollama can produce truncated JSON, trailing
/// commas, Python `None`, etc. The API proxy rejects these with HTTP 400
/// "invalid tool call arguments". This function applies common repairs;
/// if all fail it returns `"{}"` so the request succeeds (better than
/// crashing the session).
pub fn repair_tool_call_arguments(raw_args: &str, tool_name: &str) -> String {
    let raw_stripped = raw_args.trim();

    // Fast-path: empty / whitespace-only -> empty object
    if raw_stripped.is_empty() {
        tracing::warn!("Sanitized empty tool_call arguments for {}", tool_name);
        return "{}".to_string();
    }

    // Python-literal None -> normalise to {}
    if raw_stripped == "None" {
        tracing::warn!(
            "Sanitized Python-None tool_call arguments for {}",
            tool_name
        );
        return "{}".to_string();
    }

    // Pass 0: try strict=False (accept control chars in strings)
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw_stripped) {
        // If it parsed, re-serialize to clean it up
        let reserialized = serde_json::to_string(&parsed).unwrap_or_default();
        if reserialized != raw_stripped {
            tracing::warn!(
                "Repaired tool_call arguments for {}: control chars cleaned",
                tool_name
            );
        }
        return reserialized;
    }

    // Attempt common JSON repairs
    let mut fixed = raw_stripped.to_string();

    // Combined pass: close unclosed strings and find trailing colon position
    // in a single O(n) traversal instead of two separate passes.
    //
    // - Tracks in_string/escape_next state to detect truncated string literals
    // - Tracks depth ({} only, not []) to find last comma at depth 1
    //   for trailing colon cleanup (object-level commas always come after
    //   array contents, so this works correctly)
    {
        let mut in_string = false;
        let mut escape_next = false;
        let mut depth = 0i32;
        let mut last_comma_at_depth1: Option<usize> = None;

        for (pos, ch) in fixed.char_indices() {
            if escape_next {
                escape_next = false;
                continue;
            }
            if ch == '\\' && in_string {
                escape_next = true;
                continue;
            }
            if ch == '"' {
                in_string = !in_string;
                continue;
            }
            if !in_string {
                match ch {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ',' if depth == 1 => last_comma_at_depth1 = Some(pos),
                    _ => {}
                }
            }
        }

        // Close unclosed string literal (streaming truncation)
        if in_string {
            fixed.push('"');
        }

        // Remove trailing incomplete key-value pairs (streaming truncation:
        // the output was cut after a colon, e.g. {"a": 1, "b": )
        if fixed.ends_with(':') {
            if let Some(comma_pos) = last_comma_at_depth1 {
                fixed.truncate(comma_pos);
            } else {
                // No comma at depth 1 — truncate to just after the opening brace
                if let Some(brace_pos) = fixed.find('{') {
                    fixed.truncate(brace_pos + 1);
                }
            }
        }
    }

    // 3. Strip trailing commas before } or ]
    fixed = TRAILING_COMMA_RE.replace_all(&fixed, "$1").to_string();

    // 4. Close unclosed structures
    let open_curly = fixed.matches('{').count() as i32 - fixed.matches('}').count() as i32;
    let open_bracket = fixed.matches('[').count() as i32 - fixed.matches(']').count() as i32;
    if open_curly > 0 {
        fixed.extend(std::iter::repeat_n('}', open_curly as usize));
    }
    if open_bracket > 0 {
        fixed.extend(std::iter::repeat_n(']', open_bracket as usize));
    }

    // 5. Remove excess closing braces/brackets (bounded to 50 iterations)
    for _ in 0..50 {
        if serde_json::from_str::<serde_json::Value>(&fixed).is_ok() {
            break;
        }
        let extra_close = if fixed.ends_with('}') {
            fixed.matches('}').count() > fixed.matches('{').count()
        } else if fixed.ends_with(']') {
            fixed.matches(']').count() > fixed.matches('[').count()
        } else {
            false
        };
        if extra_close {
            fixed.pop();
        } else {
            break;
        }
    }

    if serde_json::from_str::<serde_json::Value>(&fixed).is_ok() {
        tracing::warn!(
            "Repaired malformed tool_call arguments for {}: {:?} -> {:?}",
            tool_name,
            raw_stripped.chars().take(80).collect::<String>(),
            fixed.chars().take(80).collect::<String>()
        );
        return fixed;
    }

    // Last resort: replace with empty object so the API request doesn't
    // crash the entire session.
    tracing::warn!(
        "Unrepairable tool_call arguments for {} — replaced with empty object",
        tool_name
    );
    "{}".to_string()
}

// ---------------------------------------------------------------------------
// Message sequence repair
// ---------------------------------------------------------------------------

/// Repair role-alternation violations in a message list.
///
/// Ported from hermes-agent's `repair_message_sequence`.
/// Fixes:
/// - `tool -> user` violations (insert synthetic assistant)
/// - `user -> user` violations (merge content)
/// - `assistant -> assistant` violations (merge tool_calls, keep newer content)
///
/// Returns the number of repairs made.
pub fn repair_message_sequence(messages: &mut Vec<Message>) -> usize {
    if messages.len() < 2 {
        return 0;
    }

    let mut repairs = 0;
    let mut i = 1;

    while i < messages.len() {
        let prev_role = messages[i - 1].role.clone();
        let curr_role = messages[i].role.clone();

        let violation = match (&prev_role, &curr_role) {
            // Tool followed by user — insert a synthetic assistant message
            (Role::Tool, Role::User) => {
                let synthetic = Message::assistant("[Continuing after tool result]");
                messages.insert(i, synthetic);
                repairs += 1;
                true
            }
            // User followed by user — merge into previous
            (Role::User, Role::User) => {
                let merged = format!("{}\n\n{}", messages[i - 1].content, messages[i].content);
                messages[i - 1].content = merged;
                messages.remove(i);
                repairs += 1;
                true
            }
            // Assistant followed by assistant — merge tool_calls, keep newer content
            (Role::Assistant, Role::Assistant) => {
                // Union tool_calls from both messages
                let prev_calls = messages[i - 1].tool_calls.take().unwrap_or_default();
                let new_calls = messages[i].tool_calls.take().unwrap_or_default();
                if !new_calls.is_empty() {
                    let mut merged_calls = prev_calls;
                    merged_calls.extend(new_calls);
                    messages[i - 1].tool_calls = Some(merged_calls);
                } else if messages[i - 1].tool_calls.is_none() {
                    messages[i - 1].tool_calls = Some(prev_calls);
                }
                // Keep the newer content
                if !messages[i].content.is_empty() {
                    messages[i - 1].content = messages[i].content.clone();
                }
                messages.remove(i);
                repairs += 1;
                true
            }
            _ => false,
        };

        if !violation {
            i += 1;
        }
    }

    repairs
}

// ---------------------------------------------------------------------------
// Close interrupted tool sequence
// ---------------------------------------------------------------------------

/// Append a synthetic assistant turn when an interrupted tail is a tool result.
///
/// A turn cut short by Ctrl-C can leave the transcript ending on a raw
/// `tool` message. Persisting that tail means the next user message lands
/// as `tool -> user` — a role-alternation violation that strict providers
/// (Gemini, Claude) reject.
///
/// Returns true if a closing turn was appended.
pub fn close_interrupted_tool_sequence(messages: &mut Vec<Message>, final_response: Option<&str>) -> bool {
    if messages.is_empty() {
        return false;
    }
    let last = messages.last().unwrap();
    if last.role != Role::Tool {
        return false;
    }
    let text = final_response.unwrap_or("");
    let content = if text.trim().is_empty() {
        "Operation interrupted.".to_string()
    } else {
        text.trim().to_string()
    };
    messages.push(Message::assistant(&content));
    true
}

// ---------------------------------------------------------------------------
// Thinking-only drop and user-merge (Anthropic-style cleanup)
// ---------------------------------------------------------------------------

/// Check if an assistant message is "thinking-only" — it has reasoning
/// content but empty main content and no tool calls.
///
/// Models like Claude emit thinking/reasoning blocks as separate messages.
/// These are useful for internal reasoning but strict providers reject them
/// if sent as standalone assistant messages with empty content.
fn is_thinking_only(msg: &Message) -> bool {
    msg.role == Role::Assistant
        && msg.content.trim().is_empty()
        && msg.tool_calls.is_none()
        && msg.reasoning.is_some()
}

/// Drop thinking-only assistant messages and merge consecutive user messages.
///
/// Ported from hermes-agent's `drop_thinking_only_and_merge_users`.
/// Operates on a copy to avoid mutating the original conversation.
///
/// This is needed because:
/// 1. Anthropic/Claude models emit reasoning as separate assistant messages
///    with empty content — these confuse strict providers (Gemini, OpenAI
///    strict mode) if forwarded as-is.
/// 2. After dropping those messages, consecutive user messages may appear,
///    which violates role-alternation invariants.
///
/// Returns the cleaned message list (original is not mutated).
pub fn drop_thinking_only_and_merge_users(messages: &[Message]) -> Vec<Message> {
    // Pass 1: Drop thinking-only assistant messages
    let mut cleaned: Vec<Message> = messages
        .iter()
        .filter(|m| !is_thinking_only(m))
        .cloned()
        .collect();

    let dropped = messages.len() - cleaned.len();

    // Pass 2: Merge consecutive user messages
    let mut merged: Vec<Message> = Vec::with_capacity(cleaned.len());
    for msg in cleaned.drain(..) {
        if msg.role == Role::User
            && !merged.is_empty()
            && merged.last().unwrap().role == Role::User
        {
            // Merge into the previous user message
            let prev = merged.last_mut().unwrap();
            if prev.content.is_empty() {
                prev.content = msg.content;
            } else if !msg.content.is_empty() {
                prev.content = format!("{}\n\n{}", prev.content, msg.content);
            }
        } else {
            merged.push(msg);
        }
    }

    let merge_count = messages.len() - dropped - merged.len();
    if dropped > 0 || merge_count > 0 {
        tracing::debug!(
            dropped,
            merged = merge_count,
            "drop_thinking_only_and_merge_users: cleaned message list"
        );
    }

    merged
}

// ---------------------------------------------------------------------------
// Tool call sanitization for strict providers
// ---------------------------------------------------------------------------

/// Sanitize tool call names and arguments for strict API providers.
///
/// Ported from hermes-agent's `_sanitize_tool_calls_for_strict_api`.
/// Some providers (Gemini, Claude strict mode) enforce stricter validation:
/// - Tool names must match `^[a-zA-Z0-9_-]{1,64}$`
/// - Tool call arguments must be valid JSON objects (not arrays, strings, etc.)
///
/// Returns the number of sanitizations performed.
pub fn sanitize_tool_calls_for_strict_api(messages: &mut [Message]) -> usize {
    let mut sanitizations = 0;

    for msg in messages.iter_mut() {
        if msg.role != Role::Assistant {
            continue;
        }
        if let Some(ref mut tool_calls) = msg.tool_calls {
            for tc in tool_calls.iter_mut() {
                // 1. Sanitize tool name: strip invalid chars, truncate to 64
                let sanitized_name = sanitize_tool_name_for_strict(&tc.function.name);
                if sanitized_name != tc.function.name {
                    tracing::debug!(
                        original = %tc.function.name,
                        sanitized = %sanitized_name,
                        "Sanitized tool name for strict API"
                    );
                    tc.function.name = sanitized_name;
                    sanitizations += 1;
                }

                // 2. Ensure arguments are valid JSON object
                if !tc.function.arguments.is_empty() {
                    let trimmed = tc.function.arguments.trim();
                    // If it doesn't start with '{', wrap it
                    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
                        // Might be a bare string or other non-object
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if !parsed.is_object() {
                                // Wrap non-object values in {"input": ...}
                                tc.function.arguments =
                                    format!("{{\"input\":{}}}", trimmed);
                                sanitizations += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    sanitizations
}

/// Sanitize a single tool name to conform to strict API requirements.
///
/// Reuses the existing [`crate::schema::sanitize_tool_name`] which allows
/// alphanumeric, `_`, `.`, `:`, `-` and truncates to 128 chars.
fn sanitize_tool_name_for_strict(name: &str) -> String {
    crate::schema::sanitize_tool_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_repair_tool_call_arguments_empty() {
        assert_eq!(repair_tool_call_arguments("", "test_tool"), "{}");
        assert_eq!(repair_tool_call_arguments("  ", "test_tool"), "{}");
    }

    #[test]
    fn test_repair_tool_call_arguments_none() {
        assert_eq!(repair_tool_call_arguments("None", "test_tool"), "{}");
    }

    #[test]
    fn test_repair_tool_call_arguments_valid() {
        let args = r#"{"key": "value"}"#;
        assert_eq!(
            repair_tool_call_arguments(args, "test_tool"),
            r#"{"key":"value"}"#
        );
    }

    #[test]
    fn test_repair_tool_call_arguments_trailing_comma() {
        let args = r#"{"key": "value",}"#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn test_repair_tool_call_arguments_unclosed_brace() {
        let args = r#"{"key": "value""#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn test_repair_message_sequence_tool_then_user() {
        let mut messages = vec![
            Message::assistant("calling tool"),
            Message::tool("tc1", "result"),
            Message::user("next question"),
        ];
        let repairs = repair_message_sequence(&mut messages);
        assert!(repairs > 0);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2].role, Role::Assistant);
        assert_eq!(messages[3].role, Role::User);
    }

    #[test]
    fn test_repair_message_sequence_consecutive_users() {
        let mut messages = vec![
            Message::user("first question"),
            Message::user("second question"),
        ];
        let repairs = repair_message_sequence(&mut messages);
        assert!(repairs > 0);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("first question"));
        assert!(messages[0].content.contains("second question"));
    }

    #[test]
    fn test_repair_message_sequence_consecutive_assistants() {
        let mut messages = vec![
            Message::assistant("first response"),
            Message::assistant("second response"),
        ];
        let repairs = repair_message_sequence(&mut messages);
        assert!(repairs > 0);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "second response");
    }

    #[test]
    fn test_close_interrupted_tool_sequence() {
        let mut messages = vec![Message::tool("tc1", "result")];
        assert!(close_interrupted_tool_sequence(&mut messages, None));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "Operation interrupted.");
    }

    #[test]
    fn test_close_interrupted_tool_sequence_not_tool_tail() {
        let mut messages = vec![Message::user("hello")];
        assert!(!close_interrupted_tool_sequence(&mut messages, None));
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_repair_unclosed_string_literal() {
        let args = r#"{"key": "value""#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn test_repair_trailing_colon_incomplete_pair() {
        let args = r#"{"a": 1, "b": "#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], 1);
        assert!(parsed.get("b").is_none());
    }

    #[test]
    fn test_repair_trailing_colon_no_comma() {
        let args = r#"{"a": "#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed.is_object());
    }

    #[test]
    fn test_repair_string_with_escapes_unclosed() {
        let args = r#"{"path": "C:\\Users\\test""#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["path"].as_str().unwrap().contains("test"));
    }

    #[test]
    fn test_repair_trailing_colon_after_array() {
        let args = r#"{"a": [1, 2], "b": "#;
        let result = repair_tool_call_arguments(args, "test_tool");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], serde_json::json!([1, 2]));
        assert!(parsed.get("b").is_none());
    }

    #[test]
    fn test_drop_thinking_only_removes_empty_assistant_with_reasoning() {
        let messages = vec![
            Message::user("hello"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                reasoning: Some("I think...".to_string()),
                tool_calls: None,
                ..Default::default()
            },
            Message::assistant("Hi there!"),
        ];
        let cleaned = drop_thinking_only_and_merge_users(&messages);
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[0].role, Role::User);
        assert_eq!(cleaned[1].content, "Hi there!");
    }

    #[test]
    fn test_drop_thinking_only_keeps_assistant_with_content() {
        let messages = vec![
            Message::assistant("I have something to say"),
        ];
        let cleaned = drop_thinking_only_and_merge_users(&messages);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].content, "I have something to say");
    }

    #[test]
    fn test_drop_thinking_only_keeps_assistant_with_tool_calls() {
        use crate::client::{ToolCall, ToolCallFunction};
        let messages = vec![
            Message {
                role: Role::Assistant,
                content: String::new(),
                reasoning: Some("thinking".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "tc1".to_string(),
                    function: ToolCallFunction {
                        name: "read_file".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
                ..Default::default()
            },
        ];
        let cleaned = drop_thinking_only_and_merge_users(&messages);
        assert_eq!(cleaned.len(), 1);
    }

    #[test]
    fn test_drop_thinking_only_merges_consecutive_users() {
        let messages = vec![
            Message::user("first"),
            Message::user("second"),
            Message::assistant("response"),
        ];
        let cleaned = drop_thinking_only_and_merge_users(&messages);
        assert_eq!(cleaned.len(), 2);
        assert!(cleaned[0].content.contains("first"));
        assert!(cleaned[0].content.contains("second"));
    }

    #[test]
    fn test_drop_thinking_only_does_not_mutate_original() {
        let messages = vec![
            Message {
                role: Role::Assistant,
                content: String::new(),
                reasoning: Some("thinking".to_string()),
                tool_calls: None,
                ..Default::default()
            },
        ];
        let _cleaned = drop_thinking_only_and_merge_users(&messages);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_sanitize_tool_name_invalid_chars() {
        let name = sanitize_tool_name_for_strict("my tool.name!");
        assert_eq!(name, "my_tool.name_");
    }

    #[test]
    fn test_sanitize_tool_name_truncates_at_128() {
        let long_name = "a".repeat(200);
        let name = sanitize_tool_name_for_strict(&long_name);
        assert_eq!(name.len(), 128);
    }

    #[test]
    fn test_sanitize_tool_name_empty_becomes_underscore() {
        let name = sanitize_tool_name_for_strict("");
        assert_eq!(name, "_");
    }

    #[test]
    fn test_sanitize_tool_calls_for_strict_api_name_fix() {
        use crate::client::{ToolCall, ToolCallFunction};
        let mut messages = vec![Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "tc1".to_string(),
                function: ToolCallFunction {
                    name: "bad name!".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            ..Default::default()
        }];
        let count = sanitize_tool_calls_for_strict_api(&mut messages);
        assert!(count > 0);
        assert_eq!(messages[0].tool_calls.as_ref().unwrap()[0].function.name, "bad_name_");
    }

    #[test]
    fn test_sanitize_tool_calls_for_strict_api_wraps_non_object() {
        use crate::client::{ToolCall, ToolCallFunction};
        let mut messages = vec![Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "tc1".to_string(),
                function: ToolCallFunction {
                    name: "read_file".to_string(),
                    arguments: "\"just a string\"".to_string(),
                },
            }]),
            ..Default::default()
        }];
        let count = sanitize_tool_calls_for_strict_api(&mut messages);
        assert!(count > 0);
        let args = &messages[0].tool_calls.as_ref().unwrap()[0].function.arguments;
        let parsed: serde_json::Value = serde_json::from_str(args).unwrap();
        assert!(parsed.is_object());
    }
}
