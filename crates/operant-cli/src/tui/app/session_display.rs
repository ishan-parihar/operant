//! ONE converter: a core conversation → the TUI's display state.
//!
//! Two rebuild paths used to carry the SAME lossy conversion inline: the
//! outer drain's `TuiApp::rebuild_display_from_agent` and the frame loop's
//! `App::apply_session_conversation`. Both mapped a tool row to a bare
//! system line and dropped the assistant's reasoning, so a RESTORED session
//! showed none of the turn's activity — no tool rows, no thinking. The
//! transcript looked like the agent had never run a tool.
//!
//! One converter, both callers: the display and the context cannot drift
//! apart, and a fix to one is a fix to the other.

use std::collections::HashMap;

use super::App;
use crate::tui::adapter_types::types::{ContentBlock, Message, MessageContent, Role};
use crate::tui::app::{ToolStatus, ToolUseBlock};
use operant_core::client::{Message as CoreMessage, Role as CoreRole};

/// Rebuild the transcript from a core conversation.
///
/// The shape mirrors what the LIVE path produces, so a restored session
/// looks like the session that was running:
/// - an assistant turn's reasoning is a `Thinking` block, its text a `Text`
///   block (the live flush commits exactly this pair);
/// - a tool CALL is a tool row anchored after the assistant message that
///   asked for it, and the matching tool RESULT fills that row's output
///   (the live `ToolStart`/`ToolComplete` pair produces the same row).
pub(crate) fn apply_conversation_to_display(app: &mut App, conv: Vec<CoreMessage>) {
    app.messages.clear();
    app.clear_tool_blocks();

    // The tool RESULT is a separate core message; the CALL lives on the
    // assistant message. This map pairs them by call id.
    let mut block_by_call: HashMap<String, usize> = HashMap::new();

    for m in conv {
        match m.role {
            CoreRole::User => app.messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(m.content),
            }),
            CoreRole::System => app.messages.push(Message {
                role: Role::System,
                content: MessageContent::Text(m.content),
            }),
            CoreRole::Assistant => {
                let mut blocks = Vec::new();
                if let Some(reasoning) = m.reasoning.filter(|r| !r.trim().is_empty()) {
                    blocks.push(ContentBlock::Thinking {
                        thinking: reasoning,
                        signature: String::new(),
                    });
                }
                if !m.content.is_empty() {
                    blocks.push(ContentBlock::Text {
                        text: m.content.clone(),
                    });
                }
                // An assistant turn with NEITHER text nor reasoning (a pure
                // tool call) commits no message — the live flush does the
                // same, and the tool row anchors after the last message.
                if !blocks.is_empty() {
                    app.messages.push(assistant_message(blocks));
                }
                let anchor = app.messages.len();
                for call in m.tool_calls.unwrap_or_default() {
                    let intent =
                        crate::tui::operant_model::vendor_types::ToolCall::intent_from_input(
                            &serde_json::from_str(&call.function.arguments)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    app.tool_use_blocks.push(ToolUseBlock {
                        id: call.id.clone(),
                        name: call.function.name.clone(),
                        after_index: anchor,
                        status: ToolStatus::Done,
                        output_preview: None,
                        input_json: call.function.arguments.clone(),
                        intent,
                    });
                    block_by_call.insert(call.id.clone(), app.tool_use_blocks.len() - 1);
                }
            }
            CoreRole::Tool => {
                let id = m.tool_call_id.clone().unwrap_or_default();
                match block_by_call.get(&id).copied() {
                    Some(index) => {
                        let failed =
                            crate::tui::operant_model::tool_display::tool_output_looks_failed(
                                &m.content,
                            );
                        let preview = super::agent_events::tool_output_preview(&m.content, failed);
                        app.tool_use_blocks[index].status = if failed {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Done
                        };
                        app.tool_use_blocks[index].output_preview = Some(preview);
                        app.remember_tool_output(&id, &m.content);
                    }
                    None => {
                        // A result whose CALL is not in the transcript (the
                        // request was evicted, or the row predates the tool
                        // call's persistence): keep the CONTENT visible as a
                        // system line rather than dropping it.
                        app.messages.push(Message {
                            role: Role::System,
                            content: MessageContent::Text(m.content),
                        });
                    }
                }
            }
        }
    }
}

/// The assistant message shape the live flush commits: one text block is a
/// plain text message, a thinking+text pair is a block list.
fn assistant_message(blocks: Vec<ContentBlock>) -> Message {
    match blocks.len() {
        0 => Message::assistant(String::new()),
        1 => match blocks.into_iter().next() {
            Some(ContentBlock::Text { text }) => Message::assistant(text),
            Some(block) => Message::assistant_blocks(vec![block]),
            None => Message::assistant(String::new()),
        },
        _ => Message::assistant_blocks(blocks),
    }
}
