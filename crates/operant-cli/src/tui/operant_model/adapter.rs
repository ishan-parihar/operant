//! The single deliberate adaptation point of the W1 seam: operant's
//! transcript state → jcode's [`DisplayMessage`] (plan §1, "one operant→
//! `DisplayMessage` adapter"). Everything else in `operant_model` is a verbatim
//! port; this file is operant's own code and the only place the two shapes
//! meet. W3 renderers consume only what this produces.
//!
//! ## Mapping
//!
//! | operant state | → `DisplayMessage` |
//! |---|---|
//! | `Message{Role::User, Text}` | `user(text)` |
//! | `Message{Role::Assistant, Text}` | `assistant(text)` |
//! | `Message{Role::System, Text}` | `system(text)` |
//! | `ContentBlock::Text` (in `Blocks`) | one row in the message's role; consecutive `Text` blocks join with `\n` |
//! | `ContentBlock::Thinking` | `reasoning(thinking)` |
//! | `ContentBlock::SystemAPIError` | `error(message)` |
//! | `ContentBlock::ToolUse` | `tool("", ToolCall{id, name, input})` — calls carry no output text in operant |
//! | `ContentBlock::ToolResult` | `tool_text(extracted text)` |
//! | `App::tool_use_blocks` entry (side registry) | `tool(output_preview.unwrap_or_default(), ToolCall{id, name, input: parse(input_json)})`, placed at its arrival anchor: after message `after_index - 1`, before message `after_index` (so a call that ran between two assistant messages renders between them) |
//! | `SystemAnnotation` | `system(text)`, interleaved at its `after_index` |
//! | `App::streaming_thinking` / `streaming_text` | trailing `reasoning` / `assistant` rows while non-empty |
//!
//! ## Named-unmapped (no `DisplayMessage` equivalent — named, never silently dropped)
//!
//! * `DisplayMessage::duration_secs`: operant keeps only a formatted string
//!   (`TurnMetadata.duration`, built by `format_elapsed_ms` at
//!   `app/agent_events.rs` — the raw seconds are discarded before storage).
//!   Rows therefore carry `None`.
//! * `TurnMetadata::{model_name, agent_mode, interrupted}`: jcode renders
//!   turn metadata in the chrome (W5), not in message rows. None of it
//!   enters the model.
//! * `ToolStatus` / `ToolUseBlock::status`: pending/failed state is app-side
//!   rendering state; jcode's `ui_tools` reads it from the app, not from the
//!   message model. `tool_output_looks_failed` (ported in `tool_display`)
//!   covers the settled-failure case from content alone.
//! * `ContentBlock::ToolResult::is_error`: same reasoning — W3 infers failure
//!   from content.
//! * `SystemMessageStyle`: jcode renders all system rows uniformly.
//! * `ContentBlock::{Image, Document, RedactedThinking, UserLocalCommandOutput,
//!   UserCommand, UserMemoryInput, CollapsedReadSearch, TaskAssignment}`:
//!   no row form exists in the ported model (images enter at prepare time in
//!   jcode; the rest are operant UI concepts). These variants are
//!   `#[allow(dead_code)]` and never constructed today; the arms below are
//!   explicit so the first live construction is a visible decision point.
//! * `DisplayMessage::tool_calls` (per-row call summaries): operant cannot
//!   attribute a tool call to a specific assistant row (`ToolUseBlock`
//!   carries only its arrival anchor), and the dedicated tool rows already carry
//!   full `tool_data`. Left empty.
//! * `DisplayMessage::title`: jcode's renderer derives titles from
//!   `tool_data.name`; the adapter does not pre-render.
//!
//! Ordering note (iter-668): tool rows splice at their arrival anchor —
//! after the message that preceded the call — so text↔tool interleaving
//! renders in true stream order. This walks
//! `App::messages` directly rather than reusing
//! `transcript_turn::build_transcript_turns`, because turns drop
//! `Role::System` messages and orphan tool blocks when no turn exists; this
//! walk drops nothing.
//!
//! Call this on a `transcript_version` bump, not per frame: it clones content
//! into owned rows by design (mirroring jcode's own
//! `display_messages_from_rendered_messages`).

use std::collections::HashMap;

use serde_json::Value;

use super::{DisplayMessage, ToolCall};
use crate::tui::adapter_types::types::{
    ContentBlock, Message, MessageContent, Role, ToolResultContent,
};
use crate::tui::app::{App, SystemAnnotation, ToolUseBlock};

/// Flatten operant's transcript state into the jcode message model.
pub fn display_messages(app: &App) -> Vec<DisplayMessage> {
    let last_index = app.messages.len().saturating_sub(1);
    let mut tool_rows_after: HashMap<usize, Vec<DisplayMessage>> = HashMap::new();
    let mut tool_rows_before: Vec<DisplayMessage> = Vec::new();
    let mut trailing_tool_rows: Vec<DisplayMessage> = Vec::new();
    for block in &app.tool_use_blocks {
        let row = display_message_from_tool_block(block);
        // Arrival anchoring (iter-668): a block created when `after_index`
        // messages existed renders after message `after_index - 1` — i.e.
        // exactly where the call happened in the stream. `after_index == 0`
        // renders before the first message; a block created after the last
        // message (running tool, streaming turn) trails it.
        if app.messages.is_empty() {
            trailing_tool_rows.push(row);
        } else if block.after_index == 0 {
            tool_rows_before.push(row);
        } else {
            tool_rows_after
                .entry(block.after_index.min(last_index + 1) - 1)
                .or_default()
                .push(row);
        }
    }

    let mut annotations_after: HashMap<usize, Vec<&SystemAnnotation>> = HashMap::new();
    for annotation in &app.system_annotations {
        annotations_after
            .entry(annotation.after_index)
            .or_default()
            .push(annotation);
    }

    let mut out = Vec::with_capacity(
        app.messages.len() + app.tool_use_blocks.len() + app.system_annotations.len(),
    );
    if let Some(annotations) = annotations_after.remove(&0) {
        out.extend(annotations.into_iter().map(system_annotation_row));
    }
    out.extend(tool_rows_before);
    for (index, message) in app.messages.iter().enumerate() {
        out.extend(expand_message(message));
        if let Some(rows) = tool_rows_after.remove(&index) {
            out.extend(rows);
        }
        if let Some(annotations) = annotations_after.remove(&(index + 1)) {
            out.extend(annotations.into_iter().map(system_annotation_row));
        }
    }
    if !app.streaming_thinking.is_empty() {
        out.push(DisplayMessage::reasoning(app.streaming_thinking.clone()));
    }
    if !app.streaming_text.is_empty() {
        out.push(DisplayMessage::assistant(app.streaming_text.clone()));
    }
    out.extend(trailing_tool_rows);
    out
}

fn system_annotation_row(annotation: &SystemAnnotation) -> DisplayMessage {
    DisplayMessage::system(annotation.text.clone())
}

fn display_message_from_tool_block(block: &ToolUseBlock) -> DisplayMessage {
    DisplayMessage::tool(
        block.output_preview.clone().unwrap_or_default(),
        ToolCall {
            id: block.id.clone(),
            name: block.name.clone(),
            input: parse_tool_input(&block.input_json),
            intent: None,
            thought_signature: None,
        },
    )
}

/// Operant stores tool input as a JSON *string*; jcode's model carries it
/// parsed. Unparseable input maps to `Value::Null` rather than panicking —
/// the cache hash and the renderers both treat it as ordinary data.
fn parse_tool_input(input_json: &str) -> Value {
    serde_json::from_str(input_json).unwrap_or(Value::Null)
}

/// One operant `Message` may carry several content blocks, and one block maps
/// to at most one row, so a message expands to a run of rows. Consecutive
/// `Text` blocks join into a single row in the message's own role; every
/// other block flushes that accumulator and emits its own row.
fn expand_message(message: &Message) -> Vec<DisplayMessage> {
    match &message.content {
        MessageContent::Text(text) => vec![role_row(&message.role, text.clone())],
        MessageContent::Blocks(blocks) => {
            let mut rows = Vec::new();
            let mut text_acc: Option<String> = None;
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => match &mut text_acc {
                        Some(acc) => {
                            acc.push('\n');
                            acc.push_str(text);
                        }
                        None => text_acc = Some(text.clone()),
                    },
                    other => {
                        flush_text_acc(&mut rows, &mut text_acc, &message.role);
                        match other {
                            ContentBlock::Thinking { thinking, .. } => {
                                rows.push(DisplayMessage::reasoning(thinking.clone()));
                            }
                            ContentBlock::SystemAPIError { message, .. } => {
                                rows.push(DisplayMessage::error(message.clone()));
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                rows.push(DisplayMessage::tool(
                                    String::new(),
                                    ToolCall {
                                        id: id.clone(),
                                        name: name.clone(),
                                        input: input.clone(),
                                        intent: None,
                                        thought_signature: None,
                                    },
                                ));
                            }
                            ContentBlock::ToolResult { content, .. } => {
                                rows.push(DisplayMessage::tool_text(tool_result_text(content)));
                            }
                            // Named-unmapped (see module docs): no row form in
                            // the jcode model. Never constructed today.
                            ContentBlock::Image { .. }
                            | ContentBlock::Document { .. }
                            | ContentBlock::RedactedThinking { .. }
                            | ContentBlock::UserLocalCommandOutput { .. }
                            | ContentBlock::UserCommand { .. }
                            | ContentBlock::UserMemoryInput { .. }
                            | ContentBlock::CollapsedReadSearch { .. }
                            | ContentBlock::TaskAssignment { .. } => {}
                            // Unreachable by construction: the outer arm owns
                            // Text. This arm exists only for exhaustiveness;
                            // if it ever ran, re-starting the accumulator
                            // (just flushed above) keeps the join semantics
                            // and drops nothing instead of panicking.
                            ContentBlock::Text { text } => text_acc = Some(text.clone()),
                        }
                    }
                }
            }
            flush_text_acc(&mut rows, &mut text_acc, &message.role);
            rows
        }
    }
}

fn flush_text_acc(rows: &mut Vec<DisplayMessage>, text_acc: &mut Option<String>, role: &Role) {
    if let Some(text) = text_acc.take() {
        rows.push(role_row(role, text));
    }
}

fn role_row(role: &Role, text: String) -> DisplayMessage {
    match role {
        Role::User => DisplayMessage::user(text),
        Role::Assistant => DisplayMessage::assistant(text),
        Role::System => DisplayMessage::system(text),
    }
}

/// Text of a tool result. Non-text parts (images) have no `DisplayMessage`
/// form and contribute nothing (named-unmapped, see module docs).
fn tool_result_text(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(text) => text.clone(),
        ToolResultContent::Image { .. } => String::new(),
        ToolResultContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::adapter_types::types::ToolResultContent;
    use crate::tui::app::tests::make_app;
    use crate::tui::app::{SystemMessageStyle, ToolStatus};
    use serde_json::json;

    fn tool_block(id: &str, name: &str, after_index: usize) -> ToolUseBlock {
        ToolUseBlock {
            id: id.to_string(),
            name: name.to_string(),
            after_index,
            status: ToolStatus::Done,
            output_preview: Some("done".to_string()),
            input_json: r#"{"file_path":"a.rs"}"#.to_string(),
        }
    }

    #[test]
    fn user_and_assistant_text_map_to_roles() {
        let mut app = make_app();
        app.messages.push(Message::user("hi".to_string()));
        app.messages.push(Message::assistant("hello".to_string()));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].role, "user");
        assert_eq!(rows[0].content, "hi");
        assert_eq!(rows[1].role, "assistant");
        assert_eq!(rows[1].content, "hello");
    }

    #[test]
    fn thinking_block_becomes_reasoning_row() {
        let mut app = make_app();
        app.messages.push(Message::assistant_blocks(vec![
            ContentBlock::Thinking {
                thinking: "let me think".to_string(),
                signature: String::new(),
            },
            ContentBlock::Text {
                text: "answer".to_string(),
            },
        ]));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].role, "reasoning");
        assert_eq!(rows[0].content, "let me think");
        assert_eq!(rows[1].role, "assistant");
        assert_eq!(rows[1].content, "answer");
    }

    #[test]
    fn api_error_block_becomes_error_row() {
        let mut app = make_app();
        app.messages.push(Message::assistant_blocks(vec![
            ContentBlock::SystemAPIError {
                message: "rate limited".to_string(),
                retry_secs: None,
            },
        ]));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, "error");
        assert_eq!(rows[0].content, "rate limited");
    }

    #[test]
    fn tool_registry_block_becomes_tool_row_with_parsed_input() {
        let mut app = make_app();
        app.messages.push(Message::user("run it".to_string()));
        app.messages.push(Message::assistant("working".to_string()));
        app.tool_use_blocks.push(tool_block("t1", "read", 2));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 3);
        let tool_row = &rows[2];
        assert_eq!(tool_row.role, "tool");
        assert_eq!(tool_row.content, "done");
        let tool_data = tool_row.tool_data.as_ref().expect("tool_data set");
        assert_eq!(tool_data.id, "t1");
        assert_eq!(tool_data.name, "read");
        assert_eq!(tool_data.input, json!({"file_path": "a.rs"}));
    }

    #[test]
    fn invalid_tool_input_json_maps_to_null_not_panic() {
        let mut app = make_app();
        let mut block = tool_block("t1", "read", 0);
        block.input_json = "not json".to_string();
        app.tool_use_blocks.push(block);

        let rows = display_messages(&app);
        assert_eq!(rows[0].tool_data.as_ref().unwrap().input, Value::Null);
    }

    #[test]
    fn tool_row_stays_between_the_messages_it_ran_between() {
        let mut app = make_app();
        app.messages.push(Message::user("go".to_string()));
        app.messages
            .push(Message::assistant("let me look".to_string()));
        // The tool started when two messages existed (after "let me look",
        // before the final answer was streamed) …
        app.tool_use_blocks.push(tool_block("t1", "read", 2));
        // … and the turn then finished with a final assistant message.
        app.messages
            .push(Message::assistant("the answer".to_string()));

        let rows = display_messages(&app);
        let roles: Vec<&str> = rows.iter().map(|r| r.role.as_str()).collect();
        // iter-668 regression: the tool row renders BETWEEN the two assistant
        // messages, not after the turn's last one.
        assert_eq!(roles, ["user", "assistant", "tool", "assistant"]);
        assert_eq!(rows[2].tool_data.as_ref().unwrap().id, "t1");
    }

    #[test]
    fn tool_row_from_a_older_turn_stays_in_place_across_later_turns() {
        let mut app = make_app();
        app.messages.push(Message::user("first".to_string()));
        app.messages.push(Message::assistant("one".to_string()));
        // Tool anchored in turn 0 (arrived after "one").
        app.tool_use_blocks.push(tool_block("t1", "read", 2));
        app.messages.push(Message::user("second".to_string()));
        app.messages.push(Message::assistant("two".to_string()));

        let rows = display_messages(&app);
        let roles: Vec<&str> = rows.iter().map(|r| r.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "user", "assistant"]);
    }

    #[test]
    fn tool_blocks_with_no_messages_still_emit() {
        let mut app = make_app();
        app.tool_use_blocks.push(tool_block("t1", "read", 0));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, "tool");
    }

    #[test]
    fn stale_after_index_beyond_the_end_clamps_to_the_last_message() {
        let mut app = make_app();
        app.messages.push(Message::user("hi".to_string()));
        app.messages.push(Message::assistant("hello".to_string()));
        // Session-clear style staleness: the anchor points past every message.
        app.tool_use_blocks.push(tool_block("t9", "read", 42));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].role, "tool");
    }

    #[test]
    fn system_annotations_interleave_by_after_index() {
        let mut app = make_app();
        app.messages.push(Message::user("hi".to_string()));
        app.messages.push(Message::assistant("hello".to_string()));
        app.push_system_message("before all".to_string(), SystemMessageStyle::Info);
        app.system_annotations[0].after_index = 0;
        app.push_system_message("mid".to_string(), SystemMessageStyle::Info);
        app.system_annotations[1].after_index = 1;

        let rows = display_messages(&app);
        let contents: Vec<&str> = rows.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(contents, ["before all", "hi", "mid", "hello"]);
    }

    #[test]
    fn system_role_message_is_not_dropped() {
        let mut app = make_app();
        app.messages.push(Message {
            role: Role::System,
            content: MessageContent::Text("plain system row".to_string()),
        });

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, "system");
        assert_eq!(rows[0].content, "plain system row");
    }

    #[test]
    fn streaming_partials_emit_trailing_rows() {
        let mut app = make_app();
        app.messages.push(Message::user("hi".to_string()));
        app.streaming_thinking = "pondering".to_string();
        app.streaming_text = "so far".to_string();

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].role, "reasoning");
        assert_eq!(rows[1].content, "pondering");
        assert_eq!(rows[2].role, "assistant");
        assert_eq!(rows[2].content, "so far");
    }

    #[test]
    fn tool_result_block_maps_to_tool_text_row() {
        let mut app = make_app();
        app.messages
            .push(Message::assistant_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                content: ToolResultContent::Text("result body".to_string()),
                is_error: false,
            }]));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, "tool");
        assert_eq!(rows[0].content, "result body");
        assert!(rows[0].tool_data.is_none());
    }

    #[test]
    fn consecutive_text_blocks_join_into_one_row() {
        let mut app = make_app();
        app.messages.push(Message::assistant_blocks(vec![
            ContentBlock::Text {
                text: "part one".to_string(),
            },
            ContentBlock::Text {
                text: "part two".to_string(),
            },
        ]));

        let rows = display_messages(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "part one\npart two");
    }

    #[test]
    fn unmapped_variants_emit_no_rows() {
        let mut app = make_app();
        app.messages.push(Message::assistant_blocks(vec![
            ContentBlock::Text {
                text: "visible".to_string(),
            },
            ContentBlock::UserCommand {
                name: "init".to_string(),
                args: String::new(),
            },
        ]));

        let rows = display_messages(&app);
        // UserCommand has no DisplayMessage form (named-unmapped); the text
        // block beside it still maps.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "visible");
    }
}
