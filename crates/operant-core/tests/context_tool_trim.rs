//! The aggregate tool-result trim: the rule that keeps the bloat from riding
//! every turn.
//!
//! MEASURED on the operator's live long-horizon session (2026-11-12): 988
//! messages, 540 tool results, 1 106 028 characters of tool output against
//! 11 057 characters of actual conversation — 99 % of the stored context was
//! tool output — and the session sat at 41 % of a 1 M-token window, BELOW the
//! preflight threshold, so the reactive ladder never fired and the bloat rode
//! every subsequent turn.
//!
//! The engine's per-result guard (`guard_tool_output_spend`) caps ONE result
//! at 5 % of the window; it has no aggregate half, and none of those 540
//! results was individually large enough to trip it.
//!
//! The trim's defining property: it rewrites the RESULT and leaves the CALL
//! alone. The engine's eviction unit is the tool GROUP — because a `tool_use`
//! without its `tool_result` is a malformed request — so an evicted result
//! always takes its call description with it. The description is the cheap
//! part and the output is the expensive part; these tests pin that the trim
//! gets that the wrong way round.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::client::{Message, Role, ToolCall, ToolCallFunction};
use operant_core::context_management::{
    TOOL_TRIM_MARKER, TOOL_TRIM_PROTECT_LAST, trim_tool_results_to_share,
};

/// An assistant turn that asked for one tool call, and the tool's result.
fn exchange(id: &str, args: &str, result_chars: usize) -> (Message, Message) {
    let call = ToolCall {
        id: id.to_string(),
        function: ToolCallFunction {
            name: "read_file".to_string(),
            arguments: args.to_string(),
        },
    };
    let mut assistant = Message::new(Role::Assistant, "");
    assistant.tool_calls = Some(vec![call]);
    let mut result = Message::new(Role::Tool, "x".repeat(result_chars));
    result.tool_call_id = Some(id.to_string());
    result.name = Some("read_file".to_string());
    (assistant, result)
}

#[test]
fn the_trim_keeps_the_call_and_replaces_the_result() {
    let (assistant, result) = exchange("call_a", "{\"path\":\"src/lib.rs\"}", 20_000);
    let original = result.content.clone();
    let mut messages = vec![
        Message::new(Role::System, "you are operant"),
        Message::new(Role::User, "read the file"),
        assistant,
        result,
    ];
    // A budget that leaves room for only a small share of tool output.
    let report = trim_tool_results_to_share(&mut messages, 10_000, 0.10, 0, 400);

    assert_eq!(report.trimmed, 1, "the oversized result was trimmed");
    assert!(report.saved() > 0, "the pass saved tokens: {report:?}");
    assert!(
        messages[2].tool_calls.is_some(),
        "the CALL survives — the description is the cheap part"
    );
    let args = &messages[2].tool_calls.as_ref().unwrap()[0]
        .function
        .arguments;
    assert_eq!(
        args, "{\"path\":\"src/lib.rs\"}",
        "the arguments are untouched"
    );
    assert!(
        messages[3].content.starts_with(TOOL_TRIM_MARKER),
        "the result carries the marker: {}",
        messages[3].content
    );
    assert!(
        messages[3].content.contains("20000 chars returned"),
        "the marker states the original size: {}",
        messages[3].content
    );
    // The marker's promise must be true: "the first 400 are kept" means the
    // first 400 characters of the ORIGINAL are in the content the model reads.
    // Without this assertion the trim could replace every character with a
    // marker and still pass — which is exactly what it did.
    let first_400: String = original.chars().take(400).collect();
    assert!(
        messages[3].content.contains(&first_400),
        "the first 400 characters of the original survive the trim"
    );
    assert_eq!(
        messages[3].tool_call_id.as_deref(),
        Some("call_a"),
        "the pair stays well-formed: the result still answers the call"
    );
}

#[test]
fn the_newest_results_are_never_trimmed() {
    let (a1, r1) = exchange("call_1", "{}", 20_000);
    let (a2, r2) = exchange("call_2", "{}", 20_000);
    let mut messages = vec![a1, r1, a2, r2];
    // protect_last = 1: the newest result is the current turn's evidence.
    let report = trim_tool_results_to_share(&mut messages, 10_000, 0.10, 1, 400);
    assert_eq!(report.trimmed, 1, "only the older result was trimmed");
    assert!(
        !messages[3].content.starts_with(TOOL_TRIM_MARKER),
        "the newest result is untouched"
    );
    assert!(messages[1].content.starts_with(TOOL_TRIM_MARKER));
}

#[test]
fn the_trim_is_a_no_op_when_the_share_is_already_met() {
    let (assistant, result) = exchange("call_a", "{}", 100);
    let mut messages = vec![assistant, result];
    let before = messages[1].content.clone();
    let report =
        trim_tool_results_to_share(&mut messages, 100_000, 0.40, TOOL_TRIM_PROTECT_LAST, 400);
    assert_eq!(report.trimmed, 0);
    assert_eq!(report.saved(), 0);
    assert_eq!(messages[1].content, before, "the result is untouched");
}

#[test]
fn a_second_pass_over_the_same_context_changes_nothing() {
    // The trim rewrites the BUILT list, so every byte it changes busts the
    // provider's cached prefix. The property that keeps that cost to ONE
    // event is IDEMPOTENCE: the second pass produces the same bytes, so the
    // cache survives every turn after the first.
    //
    // Without this test, a future change that re-trims an already-trimmed
    // result would bust the cache on EVERY request — a cost that shows up as
    // "the provider is slower" and nowhere else.
    let (assistant, result) = exchange("call_a", "{}", 20_000);
    let mut messages = vec![assistant, result];
    let first = trim_tool_results_to_share(&mut messages, 10_000, 0.10, 0, 400);
    assert_eq!(first.trimmed, 1);
    let after_first: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();

    let second = trim_tool_results_to_share(&mut messages, 10_000, 0.10, 0, 400);
    assert_eq!(second.trimmed, 0, "a trimmed result is not trimmed again");
    assert_eq!(second.saved(), 0);
    let after_second: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(
        after_first, after_second,
        "the built list is byte-identical, so the provider's cache survives"
    );
}

#[test]
fn the_trim_never_touches_the_users_own_words() {
    let (assistant, result) = exchange("call_a", "{}", 20_000);
    let user = Message::new(Role::User, "the exact wording that matters");
    let mut messages = vec![user.clone(), assistant, result];
    trim_tool_results_to_share(&mut messages, 10_000, 0.10, 0, 400);
    assert_eq!(
        messages[0].content, user.content,
        "the user's words are never the trim's target"
    );
}

#[test]
fn a_disabled_share_disables_the_trim() {
    let (assistant, result) = exchange("call_a", "{}", 20_000);
    let mut messages = vec![assistant, result];
    let report = trim_tool_results_to_share(&mut messages, 1_000, 0.0, 0, 400);
    assert_eq!(report.trimmed, 0, "0 disables the rule");
    assert!(!messages[1].content.starts_with(TOOL_TRIM_MARKER));
}
