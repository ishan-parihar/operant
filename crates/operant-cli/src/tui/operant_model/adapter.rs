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
//! | `App::streaming_thinking` / `streaming_text` | none — rendered once, in the mutable Streaming section (iter-781) |
//! | `ToolUseBlock` reserved for live tails | none while the tails stream — see [`live_tool_rows`] |
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
//! renders in true stream order. This walks `App::messages` directly and
//! drops nothing (no `Role::System` message, no orphan tool block).
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
    let live = tails_live(app);
    for block in &app.tool_use_blocks {
        // Reserved rows belong to the live tail while it streams — they
        // render in the mutable Streaming section, below the thinking/text
        // that preceded them (see `live_tool_rows`).
        if live && is_reserved(app, block) {
            continue;
        }
        let row = display_message_from_tool_block(app, block);
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
    // Reasoning visibility (jcode ReasoningDisplayMode): Full shows every
    // committed trace; Current keeps the current turn's traces anchored where
    // they streamed and drops them once the next user prompt lands (jcode
    // `clear_turn_reasoning_traces`) — render-time, so no message is removed
    // and no anchor index shifts; Off shows none.
    let mode = crate::tui::operant_app::config_shim::config()
        .display
        .reasoning_display();
    let last_user = app.messages.iter().rposition(|m| m.role == Role::User);
    for (index, message) in app.messages.iter().enumerate() {
        use crate::tui::operant_app::config_shim::ReasoningDisplayMode;
        let show_thinking = match mode {
            ReasoningDisplayMode::Full => true,
            ReasoningDisplayMode::Current => last_user.is_none_or(|user| index > user),
            ReasoningDisplayMode::Off => false,
        };
        out.extend(expand_message(message, show_thinking));
        if let Some(rows) = tool_rows_after.remove(&index) {
            out.extend(rows);
        }
        if let Some(annotations) = annotations_after.remove(&(index + 1)) {
            out.extend(annotations.into_iter().map(system_annotation_row));
        }
    }
    // (iter-781) Live thinking/text never appear here: this vec feeds the
    // settled Body section. They render once, in the mutable Streaming
    // section (prepare_streaming_cached), followed by `live_tool_rows`.
    out.extend(trailing_tool_rows);
    out
}

/// Tool rows whose ToolStart reserved a slot for the still-live tails
/// (after_index counted them). While the tails stream, these render in the
/// Streaming section directly below the tails — one renderer per row, and
/// thinking that preceded a call still renders above the call (iter-785).
/// Once the boundary flush commits the tails, `after_index <=
/// messages.len()` and the rows return to the Body at their anchor.
pub fn live_tool_rows(app: &App) -> Vec<DisplayMessage> {
    if !tails_live(app) {
        return Vec::new();
    }
    app.tool_use_blocks
        .iter()
        .filter(|b| is_reserved(app, b))
        .map(|b| display_message_from_tool_block(app, b))
        .collect()
}

/// Same predicate as the Streaming section's `has_streaming` gate in
/// `ui_prepare`, so a row excluded from the Body is always rendered there.
fn tails_live(app: &App) -> bool {
    app.is_streaming && (!app.streaming_thinking.is_empty() || !app.streaming_text.is_empty())
}

fn is_reserved(app: &App, block: &ToolUseBlock) -> bool {
    block.after_index > app.messages.len()
}

fn system_annotation_row(annotation: &SystemAnnotation) -> DisplayMessage {
    DisplayMessage::system(annotation.text.clone())
}

/// Content of a tool row whose call has not produced output yet; the
/// renderer shows a pending icon for it instead of `✓`.
pub const TOOL_PENDING_PREVIEW: &str = "\u{2026}";

/// Lines an expanded (Ctrl+O) tool row shows before eliding the rest.
const EXPANDED_TOOL_MAX_LINES: usize = 400;

fn display_message_from_tool_block(app: &App, block: &ToolUseBlock) -> DisplayMessage {
    use crate::tui::app::ToolStatus;
    let expanded = app
        .expanded_tool_id
        .as_deref()
        .filter(|id| *id == block.id)
        .and_then(|id| app.tool_full_outputs.get(id));
    let preview = match expanded {
        Some(full) => {
            let lines: Vec<&str> = full.lines().collect();
            let mut text = lines[..lines.len().min(EXPANDED_TOOL_MAX_LINES)].join("\n");
            if lines.len() > EXPANDED_TOOL_MAX_LINES {
                text.push_str(&format!(
                    "\n\u{2026} {} more lines",
                    lines.len() - EXPANDED_TOOL_MAX_LINES
                ));
            }
            text
        }
        None => block.output_preview.clone().unwrap_or_default(),
    };
    // The renderer infers ✓/✗ from the content, so the app-side status has
    // to be legible there: a failed call whose output does not read as a
    // failure gets the `✗` marker the inference keys on (2026-10-13: every
    // failed `process` call rendered ✓).
    let content = match block.status {
        ToolStatus::Error
            if !crate::tui::operant_model::tool_display::tool_output_looks_failed(&preview) =>
        {
            if preview.trim().is_empty() {
                "✗ failed".to_string()
            } else {
                format!("✗ {preview}")
            }
        }
        status if status.is_pending() && preview.trim().is_empty() => {
            TOOL_PENDING_PREVIEW.to_string()
        }
        _ => preview,
    };
    DisplayMessage::tool(
        content,
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
fn expand_message(message: &Message, show_thinking: bool) -> Vec<DisplayMessage> {
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
                            ContentBlock::Thinking { .. } if !show_thinking => {}
                            ContentBlock::Thinking { thinking, .. } => {
                                // Committed thinking must render dim+italic like
                                // the live stream: wrap through the shared
                                // reasoning markup (sentinel-wrapped lines) —
                                // the markdown renderer keys dim+italic off the
                                // sentinel. Raw text renders as plain body text
                                // (glitch 3, 2026-10-12).
                                rows.push(DisplayMessage::reasoning(
                                    crate::tui::operant_app::session::reasoning_markup(thinking),
                                ));
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
    fn reasoning_mode_contract_current_and_off() {
        let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
        // One test, sequential sections: the display-mode override is a
        // process-global cell and parallel test threads would race on it.
        // Section 1 — Current (jcode anchor_current_reasoning_block +
        // clear_turn_reasoning_traces): the trace commits where it streamed
        // and renders for its own turn; the next user prompt hides it.
        crate::tui::operant_app::config_shim::set_reasoning_display_override(Some(
            crate::tui::operant_app::config_shim::ReasoningDisplayMode::Current,
        ));
        let mut app = crate::tui::app::tests::make_app();
        app.messages.push(Message::user("what is 2+2".to_string()));
        app.streaming_thinking = "draft answer is 4".to_string();
        app.streaming_text = "2 + 2 = 4.".to_string();
        app.flush_streamed_assistant_message();
        let roles = |app: &App| -> Vec<String> {
            display_messages(app).into_iter().map(|r| r.role).collect()
        };
        assert_eq!(
            roles(&app),
            ["user", "reasoning", "assistant"],
            "Current shows the current turn's trace"
        );
        app.messages.push(Message::user("thanks".to_string()));
        assert_eq!(
            roles(&app),
            ["user", "assistant", "user"],
            "the next prompt hides the previous turn's trace"
        );

        // Section 2 — Off never shows reasoning anywhere: the handler
        // must not even accumulate it into the live buffer.
        crate::tui::operant_app::config_shim::set_reasoning_display_override(Some(
            crate::tui::operant_app::config_shim::ReasoningDisplayMode::Off,
        ));
        let mut app = crate::tui::app::tests::make_app();
        app.handle_agent_event(operant_core::agent::AgentEvent::Thinking {
            content: "secret thought".to_string(),
        });
        assert!(
            app.streaming_thinking.is_empty(),
            "Off mode must not accumulate reasoning into the live tail"
        );

        // Section 3 — Full keeps the classic behavior (guard against
        // overshoot): the flush persists the thinking block.
        crate::tui::operant_app::config_shim::set_reasoning_display_override(Some(
            crate::tui::operant_app::config_shim::ReasoningDisplayMode::Full,
        ));
        let mut app = crate::tui::app::tests::make_app();
        app.streaming_thinking = "kept trace".to_string();
        app.streaming_text = "answer".to_string();
        app.flush_streamed_assistant_message();
        let blocks = app.messages[0].content_blocks();
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Thinking { .. })),
            "Full mode must persist the thinking block (classic behavior)"
        );

        // Restore the snapshot default so later tests are not poisoned.
        crate::tui::operant_app::config_shim::set_reasoning_display_override(None);
    }

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
        // Committed thinking is stored sentinel-wrapped (glitch 3,
        // 2026-10-12): the markdown renderer keys dim+italic off
        // REASONING_SENTINEL; raw text rendered as plain body text.
        assert_eq!(
            rows[0].content,
            format!(
                "*{}let me think{}*  \n\n",
                crate::tui::operant_render_core::REASONING_SENTINEL,
                crate::tui::operant_render_core::REASONING_SENTINEL
            )
        );
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
    fn reserved_tool_row_moves_to_the_live_tail_while_tails_stream() {
        let _guard = crate::tui::operant_app::config_shim::reasoning_override_test_guard();
        // (iter-785 order, 2026-10-13 one-renderer fix) A tool row whose
        // ToolStart reserved a slot for the live tails is NOT a Body row
        // while the tails stream — it renders in the Streaming section,
        // below them. Rendering it (and the tails) in both places was the
        // 2026-10-13 duplication.
        let mut app = make_app();
        app.messages.push(Message::user("read it".to_string()));
        // ToolStart reserved a slot for the live tails: after_index counted
        // them (messages.len() + 1).
        app.tool_use_blocks.push(tool_block("t1", "read", 2));
        app.is_streaming = true;
        app.streaming_thinking = "The user wants me to read it".to_string();

        let roles = |rows: Vec<DisplayMessage>| -> Vec<String> {
            rows.into_iter().map(|r| r.role).collect()
        };
        assert_eq!(roles(display_messages(&app)), ["user"]);
        assert_eq!(roles(live_tool_rows(&app)), ["tool"]);

        // Once the boundary flush commits the thinking, the reservation no
        // longer matches (after_index == messages.len()) and the row returns
        // to the Body after the committed message.
        app.streaming_thinking.clear();
        app.messages
            .push(Message::assistant_blocks(vec![ContentBlock::Thinking {
                thinking: "The user wants me to read it".to_string(),
                signature: String::new(),
            }]));
        assert_eq!(roles(display_messages(&app)), ["user", "reasoning", "tool"]);
        assert!(live_tool_rows(&app).is_empty());
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
    fn streaming_partials_do_not_pollute_display_messages() {
        // (iter-781) Live streaming rows must NOT appear in display_messages:
        // that vec feeds the settled Body section, and the scrollback emitter
        // freezes every Body row into native history. Live thinking/text
        // partials belong to the mutable Streaming section only
        // (prepare_streaming_cached reads them via the TuiState accessors).
        let mut app = make_app();
        app.messages.push(Message::user("hi".to_string()));
        app.streaming_thinking = "pondering".to_string();
        app.streaming_text = "so far".to_string();

        let rows = display_messages(&app);
        assert_eq!(
            rows.len(),
            1,
            "live tails must not render as settled Body rows"
        );
        assert_eq!(rows[0].role, "user");
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
