//! The restored transcript keeps the turn's ACTIVITY.
//!
//! Both rebuild paths used to carry the same lossy conversion inline: the
//! assistant's reasoning was dropped and every tool row became a bare system
//! line. A session restored after a restart (or switched to in the hub)
//! therefore showed a transcript with no tool calls and no thinking — as if
//! the agent had never run a tool.

use super::*;
use crate::tui::adapter_types::types::{ContentBlock, MessageContent};

/// The shape a tool-using turn leaves in the agent's conversation: the
/// operator's prompt, an assistant turn that reasoned and asked for a tool,
/// the tool's result, and the final answer.
fn tool_using_conversation() -> Vec<operant_core::client::Message> {
    let call = operant_core::client::ToolCall {
        id: "call_read".to_string(),
        function: operant_core::client::ToolCallFunction {
            name: "read_file".to_string(),
            arguments: r#"{"path":"src/main.rs","intent":"read the entrypoint"}"#.to_string(),
        },
    };
    vec![
        operant_core::client::Message::user("read the entrypoint"),
        operant_core::client::Message::assistant("Reading it now.")
            .with_reasoning("The operator wants the entrypoint, so read src/main.rs.")
            .with_tool_calls(vec![call]),
        operant_core::client::Message::tool("call_read", "fn main() {}\n"),
        operant_core::client::Message::assistant("The entrypoint is `fn main`."),
    ]
}

#[test]
fn a_restored_session_keeps_the_assistants_reasoning() {
    let mut app = super::tests::make_app();
    session_display::apply_conversation_to_display(&mut app, tool_using_conversation());

    let assistant = app
        .messages
        .iter()
        .find(|m| matches!(m.content, MessageContent::Blocks(_)))
        .expect("the reasoning turn must be a block message");
    match &assistant.content {
        MessageContent::Blocks(blocks) => {
            assert!(
                blocks
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Thinking { thinking, .. }
                        if thinking.contains("the operator wants the entrypoint".to_string().as_str())
                            || thinking.contains("entrypoint"))),
                "the assistant's reasoning must survive the rebuild, got {blocks:?}"
            );
            assert!(
                blocks
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Text { text } if text == "Reading it now.")),
                "the assistant's text must survive the rebuild, got {blocks:?}"
            );
        }
        other => panic!("expected blocks, got {other:?}"),
    }
}

#[test]
fn a_restored_session_keeps_the_tool_row_and_its_result() {
    let mut app = super::tests::make_app();
    session_display::apply_conversation_to_display(&mut app, tool_using_conversation());

    assert_eq!(
        app.tool_use_blocks.len(),
        1,
        "the tool call must become a tool row, not a bare system line"
    );
    let block = &app.tool_use_blocks[0];
    assert_eq!(block.name, "read_file");
    assert_eq!(block.id, "call_read");
    assert!(
        block.input_json.contains("src/main.rs"),
        "the row carries the arguments, got {}",
        block.input_json
    );
    assert!(
        block.intent.as_deref() == Some("read the entrypoint"),
        "the row shows what the call is doing, not only its arguments, got {:?}",
        block.intent
    );
    let output = block
        .output_preview
        .as_deref()
        .expect("the result must fill the row");
    assert!(
        output.contains("fn main"),
        "the tool result must land in the row's output, got {output}"
    );
    // The row is anchored after the assistant message that asked for it —
    // between the prompt and the final answer, not trailing the transcript.
    let assistant_index = app
        .messages
        .iter()
        .position(|m| matches!(m.content, MessageContent::Blocks(_)))
        .expect("the assistant turn is in the transcript");
    assert_eq!(
        block.after_index,
        assistant_index + 1,
        "the row anchors after the assistant turn that made the call"
    );
    // No tool row became a system line.
    let tool_text: Vec<&str> = app
        .messages
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !tool_text.iter().any(|t| t.contains("fn main() {}")),
        "the tool result must not be duplicated as a system line: {tool_text:?}"
    );
}

/// A tool result whose CALL is not in the transcript (the request was
/// evicted, or the row predates the tool call's persistence) keeps its
/// content visible rather than being dropped.
#[test]
fn an_orphan_tool_result_keeps_its_content_visible() {
    let mut app = super::tests::make_app();
    let conv = vec![
        operant_core::client::Message::user("run the build"),
        operant_core::client::Message::tool("call_missing", "build finished: 0 errors"),
    ];
    session_display::apply_conversation_to_display(&mut app, conv);

    assert!(
        app.tool_use_blocks.is_empty(),
        "an orphan result cannot become a tool row — there is no call to pair it with"
    );
    assert!(
        app.messages.iter().any(
            |m| matches!(&m.content, MessageContent::Text(text) if text.contains("build finished"))
        ),
        "the orphan result's content must stay visible, got {:?}",
        app.messages
    );
}

/// A pure tool-call turn (no text, no reasoning) commits NO assistant
/// message — the live flush does the same — so the row anchors after the
/// last real message instead of after an empty one.
#[test]
fn a_tool_only_turn_commits_no_empty_assistant_message() {
    let mut app = super::tests::make_app();
    let call = operant_core::client::ToolCall {
        id: "call_silent".to_string(),
        function: operant_core::client::ToolCallFunction {
            name: "bash".to_string(),
            arguments: "{}".to_string(),
        },
    };
    let conv = vec![
        operant_core::client::Message::user("run it"),
        operant_core::client::Message::assistant("").with_tool_calls(vec![call]),
    ];
    session_display::apply_conversation_to_display(&mut app, conv);

    assert_eq!(
        app.messages.len(),
        1,
        "the empty assistant turn must not commit a message, got {:?}",
        app.messages
    );
    assert_eq!(app.tool_use_blocks.len(), 1, "the call still becomes a row");
    assert_eq!(
        app.tool_use_blocks[0].after_index, 1,
        "the row anchors after the operator's prompt — the only message there is"
    );
}
