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

/// A POKE is a synthetic continuation the harness wrote. It is persisted as
/// a user row (the transcript records what the model was told), so the
/// rebuild must NOT show it as something the operator typed.
#[test]
fn a_restored_session_does_not_show_a_poke_as_the_operators_own_message() {
    let mut app = super::tests::make_app();
    let conv = vec![
        operant_core::client::Message::user("do the work"),
        operant_core::client::Message::user(
            "You have 2 incomplete todos. Continue working, or update the todo tool.",
        ),
        operant_core::client::Message::assistant("Done — both checks pass."),
    ];
    session_display::apply_conversation_to_display(&mut app, conv);

    let user_messages: Vec<_> = app
        .messages
        .iter()
        .filter(|m| m.role == crate::tui::adapter_types::types::Role::User)
        .collect();
    assert_eq!(
        user_messages.len(),
        1,
        "only the operator's own message may be a user message, got {user_messages:?}"
    );
    assert!(
        app.messages.iter().any(|m| matches!(
            &m.content,
            MessageContent::Text(text) if text.contains("Auto-poke")
        )),
        "the poke must still be VISIBLE — as the harness's own continuation — \
         got {:?}",
        app.messages
    );
}

// ---------------------------------------------------------------------------
// RED-TEAM H-1 (cli-tui lane, iter-2628)
//
// A transcript rebuild replaces every row, but neither the selection nor copy
// mode was reset. Both are POSITIONAL / MODAL state over rows that no longer
// exist:
//
//   * the selection is stored in CONTENT-SPACE line indices `(col, line)`, so
//     after a `/clear` or `/new` a drag or `y` copies whatever now occupies
//     those row numbers -- text the operator never picked;
//   * copy mode is a thread-local only the keyboard gate can clear, so leaving
//     it armed swallows the composer's keys for a mode entered over a
//     transcript that is gone.
//
// `apply_conversation_to_display` is the single choke point: it has exactly two
// production callers (`rebuild_display_from_agent`, which serves /clear, /new,
// /undo and rewind, and the session-switch rebuild), so fixing it here fixes
// all five paths at once.
// ---------------------------------------------------------------------------

/// The defect: a rebuild leaves copy mode armed and the selection pointing at
/// content-space lines of the transcript that was just discarded.
#[test]
fn redteam_transcript_rebuild_resets_copy_mode_and_the_selection() {
    let mut app = super::tests::make_app();
    app.add_message(
        crate::tui::adapter_types::types::Role::User,
        "old session first line".to_string(),
    );
    app.add_message(
        crate::tui::adapter_types::types::Role::Assistant,
        "old session second line".to_string(),
    );

    // Arm the two pieces of positional/modal state over the CURRENT transcript.
    app.selection_anchor = Some((0, 0));
    app.selection_focus = Some((10, 1));
    *app.selection_text.borrow_mut() = "old session".to_string();
    app.enter_copy_mode();
    assert!(
        app.copy_mode_active(),
        "copy mode must be armed for this test"
    );

    // /clear, /new, /resume or a rewind: the transcript is replaced.
    session_display::apply_conversation_to_display(&mut app, Vec::new());

    assert!(
        !app.copy_mode_active(),
        "copy mode is still armed over a transcript that no longer exists, so \
         the composer's keys are still swallowed by it"
    );
    assert!(
        app.selection_anchor.is_none() && app.selection_focus.is_none(),
        "the selection still names content-space lines {:?}..{:?} of a cleared \
         transcript, so a drag or `y` copies whatever now occupies those rows",
        app.selection_anchor,
        app.selection_focus
    );
    assert!(
        app.selection_text.borrow().is_empty(),
        "the copied text outlived the transcript it was taken from: {:?}",
        app.selection_text.borrow()
    );
}

/// The control: copy mode entered and exited WITHOUT a rebuild still restores
/// its snapshot, so the assertions above are about the rebuild's missing
/// teardown and not about `exit_copy_mode` failing to clean up.
#[test]
fn redteam_copy_mode_exit_alone_still_restores_its_snapshot() {
    let mut app = super::tests::make_app();
    app.selection_anchor = Some((3, 7));
    app.selection_focus = Some((9, 11));
    *app.selection_text.borrow_mut() = "picked text".to_string();

    app.enter_copy_mode();
    // Move the selection while in copy mode; exit must put the SNAPSHOT back.
    app.selection_anchor = Some((100, 100));
    app.exit_copy_mode();

    assert!(!app.copy_mode_active());
    assert_eq!(
        app.selection_anchor,
        Some((3, 7)),
        "exit_copy_mode restores the anchor it snapshotted at entry"
    );
    assert_eq!(app.selection_focus, Some((9, 11)));
    assert_eq!(&*app.selection_text.borrow(), "picked text");
}
