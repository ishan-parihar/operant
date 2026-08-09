// messages/transcript.rs — Transcript (assistant / user / reasoning) renderers.
//
// Extracted from messages/mod.rs. Renders assistant metadata, live
// streaming text, user messages with file/attachment segments, thinking
// blocks, and the tagged assistant message.

use super::*;
use crate::tui::adapter_types::types::{ContentBlock, Message, MessageContent, ToolResultContent};
use crate::tui::app::TurnMetadata;
use crate::tui::transcript_turn::reasoning_heading;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

pub fn render_transcript_assistant_meta(
    meta: Option<&TurnMetadata>,
    accent: Color,
) -> Option<Line<'static>> {
    let meta = meta?;

    // Only show interrupted status — mode, model, and duration are already
    // displayed in the status line above the prompt.
    if !meta.interrupted {
        return None;
    }

    let spans = vec![
        Span::styled(
            "   \u{25a3} ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("interrupted", Style::default().fg(TRANSCRIPT_MUTED)),
    ];

    Some(Line::from(spans))
}

pub fn render_transcript_live_text(text: &str, width: u16) -> Vec<Line<'static>> {
    // Strip \r carriage returns as a safety net before render_markdown.
    let text: std::borrow::Cow<str> = if text.contains('\r') {
        text.replace('\r', "").into()
    } else {
        text.into()
    };
    indent_lines(
        render_markdown(&text, width.saturating_sub(4)),
        "   ",
        Style::default(),
        TRANSCRIPT_TEXT,
    )
}

/// Segments of a potentially file-injected text block.
enum TextSegment {
    Plain(String),
    FileBlock(String), // path attribute value
}

/// Normalize `@token` references in user text when those files were already shown
/// as chips. Replaces `@long/absolute/path/file.rs` with just `@file.rs` so the
/// text stays readable ("Delete @file.rs" still makes sense) without showing the
/// full path noise.
fn normalize_at_tokens(text: &str, injected: &std::collections::HashSet<String>) -> String {
    let mut result = String::with_capacity(text.len());
    for word in text.split_inclusive(|c: char| c.is_whitespace()) {
        let trimmed = word.trim_end_matches(|c: char| c.is_whitespace());
        let trailing: &str = &word[trimmed.len()..];

        if trimmed.starts_with('@') && trimmed.len() > 1 {
            let mut path_part = trimmed[1..].to_string();
            // Strip trailing punctuation (same logic as parse_at_refs)
            while !path_part.is_empty()
                && path_part.ends_with(|c: char| c.is_ascii_punctuation())
                && !path_part.ends_with('/')
            {
                path_part.pop();
            }
            let punct_suffix = &trimmed[1 + path_part.len()..];

            let basename = std::path::Path::new(&path_part)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_part.clone());

            let matches = injected.iter().any(|p| {
                p == &path_part
                    || std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().as_ref() == path_part.as_str())
                        .unwrap_or(false)
                    || p.ends_with(&format!("/{}", path_part))
            });

            if matches && basename != path_part {
                // Shorten to just the filename
                result.push('@');
                result.push_str(&basename);
                result.push_str(punct_suffix);
                result.push_str(trailing);
                continue;
            }
        }
        result.push_str(word);
    }
    result
}

/// Split text that may contain `<file path="...">...</file>` injection blocks
/// into alternating Plain and FileBlock segments.
fn extract_file_segments(text: &str) -> Vec<TextSegment> {
    let mut result = Vec::new();
    let mut remaining = text;
    const OPEN: &str = "<file path=\"";
    const CLOSE: &str = "</file>";

    while let Some(start) = remaining.find(OPEN) {
        if start > 0 {
            result.push(TextSegment::Plain(remaining[..start].to_string()));
        }
        let after = &remaining[start + OPEN.len()..];
        if let Some(path_end) = after.find('"') {
            let path = after[..path_end].to_string();
            let after_open_tag = &remaining[start..];
            if let Some(close_pos) = after_open_tag.find(CLOSE) {
                let consumed = start + close_pos + CLOSE.len();
                // skip one trailing newline if present
                let consumed = if remaining[consumed..].starts_with('\n') {
                    consumed + 1
                } else {
                    consumed
                };
                result.push(TextSegment::FileBlock(path));
                remaining = &remaining[consumed..];
            } else {
                result.push(TextSegment::Plain(remaining[start..].to_string()));
                remaining = "";
                break;
            }
        } else {
            result.push(TextSegment::Plain(remaining[start..].to_string()));
            remaining = "";
            break;
        }
    }

    if !remaining.is_empty() {
        result.push(TextSegment::Plain(remaining.to_string()));
    }
    result
}

pub fn render_transcript_user_message(
    msg: &Message,
    meta: Option<&TurnMetadata>,
    width: u16,
) -> Vec<Line<'static>> {
    // Goal-event messages injected by the /goal machinery render as a compact
    // event block, not as a user input bubble. The same applies to the user's
    // own `/goal <objective>` typing — replace it with the yellow GOAL ACTIVE
    // badge so the raw slash command doesn't sit next to the `[Goal started]`
    // event the machinery injects right after.
    // Check for /goal slash command in both MessageContent::Text and Blocks variants.
    let goal_text = match &msg.content {
        MessageContent::Text(text) => Some(text.as_str()),
        MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        }),
    };
    if let Some(text) = goal_text
        && let Some(objective) = extract_goal_slash_objective(text)
    {
        return render_goal_active_block(&objective);
    }

    let inner_width = width.saturating_sub(4).max(10);
    let mut lines = Vec::new();
    let mut pending_text = String::new();

    // Handle MessageContent::Text (simple text messages) — these don't have
    // content_blocks(), so we extract the text directly.  This fixes the bug
    // where user input text was invisible because content_blocks() returned
    // empty for MessageContent::Text messages.
    if let MessageContent::Text(ref text) = msg.content
        && !text.is_empty()
    {
        pending_text.push_str(text);
    }

    // Collect the absolute paths of every injected file so we can strip the
    // corresponding @token references from the user's original text block.
    let injected_paths: std::collections::HashSet<String> = msg
        .content_blocks()
        .iter()
        .filter_map(|b| {
            if let ContentBlock::Text { text } = b {
                if text.contains("<file path=\"") {
                    Some(text)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .flat_map(|text| {
            extract_file_segments(text).into_iter().filter_map(|s| {
                if let TextSegment::FileBlock(p) = s {
                    Some(p)
                } else {
                    None
                }
            })
        })
        .collect();

    let flush_text = |buffer: &mut String, target: &mut Vec<Line<'static>>| {
        if buffer.is_empty() {
            return;
        }
        target.extend(render_user_text_with_ctx(
            buffer,
            &RenderContext {
                width: inner_width,
                ..RenderContext::default()
            },
        ));
        buffer.clear();
    };

    for block in msg.content_blocks() {
        match block {
            ContentBlock::Text { text } => {
                if text.contains("<file path=\"") {
                    flush_text(&mut pending_text, &mut lines);
                    for segment in extract_file_segments(text) {
                        match segment {
                            TextSegment::FileBlock(path) => {
                                let label = std::path::Path::new(&path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or(path);
                                lines.push(render_file_chip(label));
                            }
                            TextSegment::Plain(t) => {
                                if !t.trim().is_empty() {
                                    if !pending_text.is_empty() {
                                        pending_text.push('\n');
                                    }
                                    pending_text.push_str(&t);
                                }
                            }
                        }
                    }
                } else if !injected_paths.is_empty() {
                    // Shorten @long/path/file.rs → @file.rs since the chips already
                    // show the full path context.
                    let cleaned = normalize_at_tokens(text, &injected_paths);
                    let trimmed = cleaned.trim();
                    if !trimmed.is_empty() {
                        if !pending_text.is_empty() {
                            pending_text.push('\n');
                        }
                        pending_text.push_str(trimmed);
                    }
                } else {
                    if !pending_text.is_empty() {
                        pending_text.push('\n');
                    }
                    pending_text.push_str(text);
                }
            }
            ContentBlock::Image {
                source,
                data: _,
                media_type,
            } => {
                flush_text(&mut pending_text, &mut lines);
                let label = if !media_type.is_empty() {
                    media_type.clone()
                } else if !source.is_empty() {
                    source.clone()
                } else {
                    "pasted image".to_string()
                };
                lines.push(render_attachment_chip("img", label));
            }
            ContentBlock::Document {
                title,
                context,
                source,
                ..
            } => {
                flush_text(&mut pending_text, &mut lines);
                let label = [title.as_str(), context.as_str(), source.as_str()]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "attached document".to_string());
                lines.push(render_attachment_chip("doc", label));
            }
            ContentBlock::UserLocalCommandOutput { command, output } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_user_local_command_output(command, output, 30));
            }
            ContentBlock::UserCommand { name, args } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_user_command(name, args));
            }
            ContentBlock::UserMemoryInput { key, value } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_user_memory_input(key, value));
            }
            ContentBlock::SystemAPIError {
                message,
                retry_secs,
            } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_system_api_error(message, *retry_secs));
            }
            ContentBlock::CollapsedReadSearch {
                tool_name,
                paths,
                n_hidden,
            } => {
                flush_text(&mut pending_text, &mut lines);
                let path_refs: Vec<&str> = paths.iter().map(|path| path.as_str()).collect();
                lines.extend(render_collapsed_read_search(
                    tool_name, &path_refs, *n_hidden,
                ));
            }
            ContentBlock::TaskAssignment {
                id,
                subject,
                description,
            } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_task_assignment(id, subject, description));
            }
            ContentBlock::ToolUse { name, input, .. } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_tool_use_inner(name, input));
            }
            ContentBlock::ToolResult {
                tool_use_id: _,
                content,
                is_error,
            } => {
                flush_text(&mut pending_text, &mut lines);
                let text = tool_result_text(content);
                let rendered = if *is_error {
                    render_tool_result_error(&text)
                } else {
                    render_tool_result_success(&text, false)
                };
                lines.extend(rendered);
            }
            ContentBlock::Thinking { thinking, .. } => {
                flush_text(&mut pending_text, &mut lines);
                lines.extend(render_transcript_reasoning_block(
                    thinking,
                    false,
                    inner_width,
                ));
            }
            ContentBlock::RedactedThinking { .. } => {
                flush_text(&mut pending_text, &mut lines);
                lines.push(Line::from(vec![Span::styled(
                    "Thinking hidden".to_string(),
                    Style::default()
                        .fg(TRANSCRIPT_MUTED)
                        .add_modifier(Modifier::ITALIC),
                )]));
            }
        }
    }
    flush_text(&mut pending_text, &mut lines);

    if let Some(meta_line) = user_metadata_line(meta) {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(meta_line);
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    let mut wrapped = Vec::with_capacity(lines.len() + 2);
    wrapped.push(empty_block_line(width));
    wrapped.extend(lines.into_iter().map(|line| apply_block_style(line, width)));
    wrapped.push(empty_block_line(width));
    wrapped
}

pub fn render_transcript_reasoning_block(
    text: &str,
    expanded: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let heading = reasoning_heading(text).unwrap_or_else(|| "Thinking".to_string());
    let chevron = if expanded { "▼" } else { "▶" };
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {} Thinking: ", chevron),
            Style::default()
                .fg(TRANSCRIPT_MUTED)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(
            heading,
            Style::default()
                .fg(TRANSCRIPT_SUBTLE)
                .add_modifier(Modifier::ITALIC),
        ),
    ]));

    if expanded {
        let rendered = render_markdown(text, width.saturating_sub(6));
        lines.extend(indent_lines(
            rendered,
            "    ",
            Style::default(),
            TRANSCRIPT_MUTED,
        ));
    }

    lines
}

/// Render the thinking content body (without header) for live streaming display.
pub fn render_thinking_live_content(text: &str, width: u16) -> Vec<Line<'static>> {
    let rendered = render_markdown(text, width.saturating_sub(6));
    indent_lines(rendered, "    ", Style::default(), TRANSCRIPT_MUTED)
}

/// Returns lines for each content block with an optional thinking hash tag.
/// The hash is `Some(hash)` only for the header line of a Thinking block,
/// enabling click-to-expand in the TUI.
pub fn render_transcript_assistant_message_tagged(
    msg: &Message,
    ctx: &RenderContext,
) -> Vec<(Line<'static>, Option<u64>)> {
    let mut out: Vec<(Line<'static>, Option<u64>)> = Vec::new();
    let mut pending_text = String::new();

    // Handle MessageContent::Text (simple text messages) — these don't have
    // content_blocks(), so we extract the text directly.  This fixes the bug
    // where committed assistant messages were invisible because content_blocks()
    // returned empty for MessageContent::Text messages created by
    // flush_streamed_assistant_message.
    if let MessageContent::Text(ref text) = msg.content
        && !text.is_empty()
    {
        pending_text.push_str(text);
    }

    let flush_text =
        |buffer: &mut String, target: &mut Vec<(Line<'static>, Option<u64>)>, width: u16| {
            if buffer.is_empty() {
                return;
            }
            for line in render_transcript_live_text(buffer, width) {
                target.push((line, None));
            }
            buffer.clear();
        };

    for block in msg.content_blocks() {
        match block {
            ContentBlock::Text { text } => {
                if !pending_text.is_empty() {
                    pending_text.push('\n');
                }
                pending_text.push_str(text);
            }
            ContentBlock::Thinking { thinking, .. } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                let thinking_hash = {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut h = DefaultHasher::new();
                    thinking.hash(&mut h);
                    h.finish()
                };
                let expanded = ctx.show_thinking || ctx.expanded_thinking.contains(&thinking_hash);
                let block_lines = render_transcript_reasoning_block(thinking, expanded, ctx.width);
                for (i, line) in block_lines.into_iter().enumerate() {
                    // Tag only the header line (index 0) with the hash so it's clickable.
                    out.push((line, if i == 0 { Some(thinking_hash) } else { None }));
                }
            }
            ContentBlock::RedactedThinking { .. } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                out.push((
                    Line::from(vec![Span::styled(
                        "  Thinking hidden".to_string(),
                        Style::default()
                            .fg(TRANSCRIPT_MUTED)
                            .add_modifier(Modifier::ITALIC),
                    )]),
                    None,
                ));
            }
            ContentBlock::ToolUse { name, input, .. } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                for line in indent_lines(
                    render_tool_use_inner(name, input),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                let text = tool_result_text(content);
                let tool_name = ctx.tool_names.get(tool_use_id).map(|name| name.as_str());
                let rendered = if *is_error {
                    render_tool_result_error(&text)
                } else {
                    match tool_name {
                        Some("Bash") | Some("PowerShell") => {
                            render_bash_output_block(&text, TOOL_RESULT_MAX_LINES)
                        }
                        Some("Read") => render_file_read_result(&text),
                        Some("Edit") => render_file_op_result(false),
                        Some("Write") => render_file_op_result(true),
                        _ => render_tool_result_success(&text, false),
                    }
                };
                for line in indent_lines(rendered, "   ", Style::default(), TRANSCRIPT_TEXT) {
                    out.push((line, None));
                }
            }
            ContentBlock::Image {
                source,
                data: _,
                media_type,
            } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                let label = if !media_type.is_empty() {
                    media_type.clone()
                } else if !source.is_empty() {
                    source.clone()
                } else {
                    "assistant image".to_string()
                };
                for line in indent_lines(
                    vec![render_attachment_chip("img", label)],
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::Document {
                title,
                context,
                source,
                ..
            } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                let label = [title.as_str(), context.as_str(), source.as_str()]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "attached document".to_string());
                for line in indent_lines(
                    vec![render_attachment_chip("doc", label)],
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::UserLocalCommandOutput { command, output } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                for line in indent_lines(
                    render_user_local_command_output(command, output, 30),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::UserCommand { name, args } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                for line in indent_lines(
                    render_user_command(name, args),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::UserMemoryInput { key, value } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                for line in indent_lines(
                    render_user_memory_input(key, value),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::SystemAPIError {
                message,
                retry_secs,
            } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                for line in indent_lines(
                    render_system_api_error(message, *retry_secs),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::CollapsedReadSearch {
                tool_name,
                paths,
                n_hidden,
            } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                let path_refs: Vec<&str> = paths.iter().map(|path| path.as_str()).collect();
                for line in indent_lines(
                    render_collapsed_read_search(tool_name, &path_refs, *n_hidden),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
            ContentBlock::TaskAssignment {
                id,
                subject,
                description,
            } => {
                flush_text(&mut pending_text, &mut out, ctx.width);
                for line in indent_lines(
                    render_task_assignment(id, subject, description),
                    "   ",
                    Style::default(),
                    TRANSCRIPT_TEXT,
                ) {
                    out.push((line, None));
                }
            }
        }
    }

    flush_text(&mut pending_text, &mut out, ctx.width);
    out
}

fn tool_result_text(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(text) => text.clone(),
        ToolResultContent::Image {
            data: _,
            media_type,
        } => format!("[image: {}]", media_type),
        ToolResultContent::Blocks(blocks) => {
            let joined = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                    ContentBlock::RedactedThinking { .. } => Some("[redacted thinking]"),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                "[structured tool result]".to_string()
            } else {
                joined
            }
        }
    }
}
