//! Message type renderers for the TUI.
//! Mirrors src/components/messages/ and src/components/Messages.tsx.
//!
//! Each message type has a dedicated render function.

use std::collections::HashMap;

use crate::tui::adapter_types::types::{ContentBlock, Message, MessageContent, ToolResultContent};
use crate::tui::app::TurnMetadata;
use crate::tui::transcript_turn::reasoning_heading;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

mod markdown;
pub use markdown::render_markdown;

mod markdown_enhanced;

/// Context passed to all renderers.
pub struct RenderContext {
    /// Current terminal width (for word-wrap decisions).
    pub width: u16,
    /// Whether to show thinking blocks.
    pub show_thinking: bool,
    /// Maps `tool_use_id` → `tool_name` so ToolResult blocks can dispatch to
    /// the correct specialized renderer (e.g. Bash output vs. generic result).
    pub tool_names: HashMap<String, String>,
    /// Set of thinking block content hashes that are expanded per-block.
    pub expanded_thinking: std::collections::HashSet<u64>,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            width: 80,
            show_thinking: false,
            tool_names: HashMap::new(),
            expanded_thinking: std::collections::HashSet::new(),
        }
    }
}

const MAX_USER_PROMPT_DISPLAY_CHARS: usize = 10_000;
const TRUNCATE_USER_PROMPT_HEAD_CHARS: usize = 2_500;
const TRUNCATE_USER_PROMPT_TAIL_CHARS: usize = 2_500;

/// Claude orange: Rgb(215, 119, 87)
const ACCENT_PRIMARY: Color = Color::Rgb(255, 191, 0);
const TRANSCRIPT_USER_BG: Color = Color::Rgb(23, 23, 31);
const TRANSCRIPT_CHIP_BG: Color = Color::Rgb(31, 31, 41);
const TRANSCRIPT_TEXT: Color = Color::Rgb(236, 236, 241);
const TRANSCRIPT_MUTED: Color = Color::Rgb(139, 139, 153);
const TRANSCRIPT_SUBTLE: Color = Color::Rgb(112, 112, 126);

const TOOL_RESULT_MAX_LINES: usize = 30;

/// Accent color for goal-event blocks (warm amber/gold).
const GOAL_ACCENT: Color = Color::Rgb(255, 170, 50);
/// Body text color for goal-event objective display.
const GOAL_BODY: Color = Color::Rgb(215, 180, 110);

/// Render a user text message body.
fn render_user_text_with_ctx(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let truncated = truncate_user_prompt_text(text);
    render_markdown(&truncated, ctx.width.saturating_sub(3))
}

fn indent_line(
    mut line: Line<'static>,
    prefix: &str,
    prefix_style: Style,
    default_fg: Color,
) -> Line<'static> {
    for span in &mut line.spans {
        if span.style.fg.is_none() {
            span.style = span.style.fg(default_fg);
        }
    }

    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::styled(prefix.to_string(), prefix_style));
    spans.extend(line.spans);
    Line::from(spans)
}

fn indent_lines(
    lines: Vec<Line<'static>>,
    prefix: &str,
    prefix_style: Style,
    default_fg: Color,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| indent_line(line, prefix, prefix_style, default_fg))
        .collect()
}

fn apply_block_style(mut line: Line<'static>, width: u16) -> Line<'static> {
    let bg = TRANSCRIPT_USER_BG;
    for span in &mut line.spans {
        if span.style.fg.is_none() {
            span.style = span.style.fg(TRANSCRIPT_TEXT);
        }
        span.style = span.style.bg(bg);
    }

    let mut spans = vec![
        Span::styled("▏", Style::default().fg(ACCENT_PRIMARY).bg(bg)),
        Span::styled(" ", Style::default().bg(bg)),
    ];
    spans.extend(line.spans);

    let used = spans.iter().map(|span| span.content.width()).sum::<usize>();
    if used < width as usize {
        spans.push(Span::styled(
            " ".repeat(width as usize - used),
            Style::default().bg(bg),
        ));
    }

    Line::from(spans)
}

fn empty_block_line(width: u16) -> Line<'static> {
    apply_block_style(Line::from(""), width)
}
fn render_attachment_chip(kind: &str, label: String) -> Line<'static> {
    render_attachment_chip_colored(kind, label, ACCENT_PRIMARY, Color::Black)
}

fn render_file_chip(label: String) -> Line<'static> {
    // Use a steel-blue badge with white text for file injections — distinct from
    // the orange img/doc chips and readable on dark terminal backgrounds.
    render_attachment_chip_colored("file", label, Color::Rgb(51, 102, 170), Color::White)
}

fn render_attachment_chip_colored(
    kind: &str,
    label: String,
    badge_bg: Color,
    badge_fg: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {} ", kind),
            Style::default()
                .fg(badge_fg)
                .bg(badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", label),
            Style::default().fg(TRANSCRIPT_MUTED).bg(TRANSCRIPT_CHIP_BG),
        ),
    ])
}

fn user_metadata_line(_meta: Option<&TurnMetadata>) -> Option<Line<'static>> {
    // User prompt line has no metadata — mode/model/duration are shown on the
    // assistant footer instead (matching OpenCode's layout).
    None
}

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
    if let Some(text) = goal_text {
        if let Some(objective) = extract_goal_slash_objective(text) {
            return render_goal_active_block(&objective);
        }
    }

    let inner_width = width.saturating_sub(4).max(10);
    let mut lines = Vec::new();
    let mut pending_text = String::new();

    // Handle MessageContent::Text (simple text messages) — these don't have
    // content_blocks(), so we extract the text directly.  This fixes the bug
    // where user input text was invisible because content_blocks() returned
    // empty for MessageContent::Text messages.
    if let MessageContent::Text(ref text) = msg.content {
        if !text.is_empty() {
            pending_text.push_str(text);
        }
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
    if let MessageContent::Text(ref text) = msg.content {
        if !text.is_empty() {
            pending_text.push_str(text);
        }
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

/// Extract a short one-line summary of a tool call's arguments.
/// Used by both the transcript renderer and live tool block renderer in render.rs.
fn title_case_word(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

pub fn extract_tool_summary(tool_name: &str, input: &serde_json::Value) -> String {
    fn str_field<'a>(input: &'a serde_json::Value, key: &str) -> &'a str {
        input.get(key).and_then(|v| v.as_str()).unwrap_or("")
    }
    fn truncate(s: &str, n: usize) -> String {
        let s = s.trim();
        let chars: Vec<char> = s.chars().collect();
        if chars.len() > n {
            format!("{}\u{2026}", chars[..n].iter().collect::<String>())
        } else {
            s.to_string()
        }
    }
    match tool_name.to_ascii_lowercase().as_str() {
        "bash" | "powershell" => {
            let cmd = str_field(input, "command");
            truncate(cmd.lines().next().unwrap_or(""), 60)
        }
        "read" => truncate(str_field(input, "file_path"), 60),
        "edit" => truncate(str_field(input, "file_path"), 60),
        "write" => truncate(str_field(input, "file_path"), 60),
        "glob" => truncate(str_field(input, "pattern"), 60),
        "grep" => truncate(str_field(input, "pattern"), 60),
        "webfetch" => truncate(str_field(input, "url"), 60),
        "websearch" => truncate(str_field(input, "query"), 60),
        "task" | "agent" => {
            let task = str_field(input, "task");
            let task = if task.is_empty() {
                str_field(input, "description")
            } else {
                task
            };
            truncate(task.lines().next().unwrap_or(""), 60)
        }
        _ => {
            // First string value from the input object
            if let Some(obj) = input.as_object() {
                for v in obj.values() {
                    if let Some(s) = v.as_str() {
                        return truncate(s, 60);
                    }
                }
            }
            String::new()
        }
    }
}

pub fn subagent_title(input: &serde_json::Value) -> String {
    let label = input
        .get("subagent_type")
        .and_then(|value| value.as_str())
        .map(title_case_word)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "General".to_string());
    format!("{label} agent")
}

fn render_tool_use_inner(tool_name: &str, input: &serde_json::Value) -> Vec<Line<'static>> {
    let summary = extract_tool_summary(tool_name, input);
    let mut lines = Vec::new();
    let title = match tool_name.to_ascii_lowercase().as_str() {
        "bash" | "powershell" => "Running command",
        "read" => "Reading file",
        "write" => "Writing file",
        "edit" => "Editing file",
        "glob" | "list" => "Listing files",
        "grep" => "Searching code",
        "webfetch" => "Fetching page",
        "websearch" => "Searching web",
        "task" | "agent" => {
            return {
                let mut task_lines = Vec::new();
                task_lines.push(Line::from(vec![
                    Span::styled("  ~ ".to_string(), Style::default().fg(ACCENT_PRIMARY)),
                    Span::styled(
                        subagent_title(input),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                if !summary.is_empty() {
                    task_lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(summary, Style::default().fg(TRANSCRIPT_MUTED)),
                    ]));
                }
                task_lines
            }
        }
        _ => tool_name,
    };

    lines.push(Line::from(vec![
        Span::styled("  ~ ".to_string(), Style::default().fg(ACCENT_PRIMARY)),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    if !summary.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(summary, Style::default().fg(TRANSCRIPT_MUTED)),
        ]));
    }

    if matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "bash" | "powershell"
    ) {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        for (i, cmd_line) in command.lines().enumerate() {
            if i >= 2 {
                break;
            }
            let display: String = cmd_line.chars().take(160).collect();
            let display = if cmd_line.chars().count() > 160 {
                format!("{}\u{2026}", display)
            } else {
                display
            };
            lines.push(Line::from(vec![
                Span::styled(
                    "    $ ".to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    display,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    lines
}

/// Render a file-read tool result: `Read N lines` summary.
fn render_file_read_result(output: &str) -> Vec<Line<'static>> {
    let n = output.lines().count();
    vec![Line::from(vec![Span::styled(
        format!("  Read {} line{}", n, if n == 1 { "" } else { "s" }),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    )])]
}

/// Render a file-edit/write tool result: `Updated file` or `Created file`.
fn render_file_op_result(is_create: bool) -> Vec<Line<'static>> {
    let action = if is_create { "Created" } else { "Updated" };
    vec![Line::from(vec![Span::styled(
        format!("  {} file", action),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    )])]
}

/// Render a tool result (success variant) — generic fallback.
pub fn render_tool_result_success(output: &str, truncated: bool) -> Vec<Line<'static>> {
    let total_lines = output.lines().count();
    // Use explicit Gray (brighter than terminal default DarkGray) so tool
    // output stays legible on themes where the default fg gets dimmed by
    // surrounding styles. Issue #149: tool result text contrast was too low.
    let body_style = Style::default().fg(Color::Gray);
    let mut lines: Vec<Line<'static>> = output
        .lines()
        .enumerate()
        .take_while(|(i, _)| *i < TOOL_RESULT_MAX_LINES)
        .map(|(_, l)| {
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(l.to_string(), body_style),
            ])
        })
        .collect();
    if total_lines > TOOL_RESULT_MAX_LINES {
        let remaining = total_lines - TOOL_RESULT_MAX_LINES;
        lines.push(Line::from(vec![Span::styled(
            format!("  ... {} more lines", remaining),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )]));
    }
    if truncated {
        lines.push(Line::from(vec![Span::styled(
            "  ... output truncated".to_string(),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines
}

/// Render a tool result (error variant).
pub fn render_tool_result_error(error: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Use orange instead of red for color-blind accessibility
    let error_color = Color::Rgb(255, 140, 0); // Orange
    lines.push(Line::from(vec![Span::styled(
        "  Error",
        Style::default()
            .fg(error_color)
            .add_modifier(Modifier::BOLD),
    )]));
    for line in error.lines().take(10) {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(line.to_string(), Style::default().fg(error_color)),
        ]));
    }
    lines
}

/// Render a bash command input line with a green `$ ` prefix.
pub fn render_bash_input_line(command: &str) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled(
            "  $ ".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            command.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])]
}

/// Render bash output lines truncated to `max_lines` with an overflow indicator.
pub fn render_bash_output_block(output: &str, max_lines: usize) -> Vec<Line<'static>> {
    let total = output.lines().count();
    let mut lines: Vec<Line<'static>> = output
        .lines()
        .take(max_lines)
        .map(|l| {
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(l.to_string(), Style::default().fg(Color::Gray)),
            ])
        })
        .collect();
    if total > max_lines {
        let remaining = total - max_lines;
        lines.push(Line::from(vec![Span::styled(
            format!("  ... {} more lines", remaining),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines
}

fn truncate_user_prompt_text(text: &str) -> String {
    if text.len() <= MAX_USER_PROMPT_DISPLAY_CHARS {
        return text.to_string();
    }

    let head = &text[..TRUNCATE_USER_PROMPT_HEAD_CHARS.min(text.len())];
    let tail_start = text.len().saturating_sub(TRUNCATE_USER_PROMPT_TAIL_CHARS);
    let tail = &text[tail_start..];
    let hidden_lines = text
        .chars()
        .take(TRUNCATE_USER_PROMPT_HEAD_CHARS)
        .filter(|c| *c == '\n')
        .count()
        .saturating_sub(tail.chars().filter(|c| *c == '\n').count());

    format!("{head}\n… +{hidden_lines} lines …\n{tail}")
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

/// Render a system API error block (red-bordered, first 5 lines with [expand] hint,
/// optional retry countdown).
pub fn render_system_api_error(msg: &str, retry_secs: Option<u64>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        "\u{250c}\u{2500} API Error ",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )]));
    let all_lines: Vec<&str> = msg.lines().collect();
    let total = all_lines.len();
    for line in all_lines.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled("\u{2502} ", Style::default().fg(Color::Red)),
            Span::styled(line.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    if total > 5 {
        lines.push(Line::from(vec![Span::styled(
            format!("\u{2502} ... {} more lines [expand]", total - 5),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines.push(Line::from(vec![Span::styled(
        "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        Style::default().fg(Color::Red),
    )]));
    if let Some(n) = retry_secs {
        lines.push(Line::from(vec![Span::styled(
            format!("  \u{21bb} Retrying in {}s...", n),
            Style::default().fg(Color::Yellow),
        )]));
    }
    lines
}

/// Render a user command invocation (skill invocation display).
/// Shows: `▸ ` in cyan bold + command name in cyan bold + " " + args in white.
///
/// Special case: `/goal <objective>` is replaced with a yellow `GOAL ACTIVE /
/// Objective: <obj>` badge so the raw slash command doesn't sit next to the
/// `[Goal started]` event the machinery injects right after it. Subcommands
/// (`/goal status`, `pause`, `resume`, `clear`, `complete`) keep the normal
/// rendering.
pub fn render_user_command(name: &str, args: &str) -> Vec<Line<'static>> {
    if name == "goal" {
        if let Some(objective) = extract_goal_objective_from_args(args) {
            return render_goal_active_block(&objective);
        }
    }
    vec![Line::from(vec![
        Span::styled(
            "\u{25b8} ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(args.to_string(), Style::default().fg(Color::White)),
    ])]
}

/// Recognizes a raw `/goal <objective>` user message. Returns the objective
/// string when the first line is `/goal …` with actual objective text;
/// returns `None` for subcommand forms, no-args, or anything that isn't a
/// `/goal` slash command (including the case where the user pastes a
/// multi-line message with `/goal …` somewhere in the middle).
fn extract_goal_slash_objective(text: &str) -> Option<String> {
    let first_line = text.lines().next()?;
    let rest = first_line
        .trim_start()
        .strip_prefix("/goal")?
        .strip_prefix(|c: char| c.is_whitespace())
        .unwrap_or("");
    let objective = extract_goal_objective_from_args(rest)?;
    // Reject bare `/goal` (no following body) — strip_prefix above returned
    // empty `rest`, which extract_goal_objective_from_args already handles.
    if text.lines().count() > 1 {
        // If the user typed more than just `/goal …`, fold the rest of the
        // message into the objective so nothing is silently dropped.
        let trailing: String = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        let trailing = trailing.trim();
        if !trailing.is_empty() {
            return Some(format!("{}\n{}", objective, trailing));
        }
    }
    Some(objective)
}

/// Pulls the objective text out of the `args` portion of a `/goal …` slash
/// command. Returns `None` for empty args or for the subcommand forms
/// (`status`, `pause`, `resume`, `clear`, `complete`).
fn extract_goal_objective_from_args(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Strip an optional `--tokens <budget>` prefix so the objective shown
    // doesn't include the budget flag.
    let rest = if let Some(after_flag) = trimmed.strip_prefix("--tokens") {
        let after_flag = after_flag.trim_start();
        after_flag
            .split_once(char::is_whitespace)
            .map(|x| x.1)
            .unwrap_or("")
            .trim()
    } else {
        trimmed
    };
    if rest.is_empty() {
        return None;
    }
    let first = rest
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        first.as_str(),
        "status" | "pause" | "resume" | "clear" | "complete"
    ) {
        return None;
    }
    Some(rest.to_string())
}

/// Render the yellow `GOAL ACTIVE / Objective: …` badge that replaces the
/// `/goal <objective>` user-input line in the transcript.
fn render_goal_active_block(objective: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled(
            "  GOAL ACTIVE".to_string(),
            Style::default()
                .fg(GOAL_ACCENT)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(
                "  Objective: ".to_string(),
                Style::default()
                    .fg(GOAL_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(objective.to_string(), Style::default().fg(GOAL_BODY)),
        ]),
    ]
}

/// Render a user memory input line.
/// Shows: `# {key}: {value}` in cyan, with an optional `  Got it.` line in dark gray italic.
pub fn render_user_memory_input(key: &str, value: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled(
            format!("# {}: {}", key, value),
            Style::default().fg(Color::Cyan),
        )]),
        Line::from(vec![Span::styled(
            "  Got it.",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )]),
    ]
}

/// Render a user local command output block.
/// Header: `  !{command}` in dark gray bold, body up to max_lines in gray,
/// overflow indicator: `  ... N more lines` in dark gray.
pub fn render_user_local_command_output(
    command: &str,
    output: &str,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        format!("  !{}", command),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));
    let total = output.lines().count();
    for line in output.lines().take(max_lines) {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(line.to_string(), Style::default().fg(Color::Gray)),
        ]));
    }
    if total > max_lines {
        lines.push(Line::from(vec![Span::styled(
            format!("  ... {} more lines", total - max_lines),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    lines
}

/// Render a collapsed read/search tool use summary.
/// Shows: `▸ ` in yellow + `{tool_name} ` in yellow bold + first few paths comma-joined,
/// followed by `(+ {n_hidden} more)` in dark gray if n_hidden > 0.
pub fn render_collapsed_read_search(
    tool_name: &str,
    paths: &[&str],
    n_hidden: usize,
) -> Vec<Line<'static>> {
    let paths_str = paths.join(", ");
    let mut spans = vec![
        Span::styled("\u{25b8} ", Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{} ", tool_name),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(paths_str, Style::default().fg(Color::White)),
    ];
    if n_hidden > 0 {
        spans.push(Span::styled(
            format!(" (+ {} more)", n_hidden),
            Style::default().fg(Color::DarkGray),
        ));
    }
    vec![Line::from(spans)]
}

/// Render a transcript task assignment row using the same structured title/subtitle language.
pub fn render_task_assignment(id: &str, subject: &str, desc: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let title = if subject.trim().is_empty() {
        "Assigned task"
    } else {
        subject.trim()
    };
    lines.push(Line::from(vec![
        Span::styled("  ~ ", Style::default().fg(ACCENT_PRIMARY)),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · task #{}", id),
            Style::default().fg(TRANSCRIPT_MUTED),
        ),
    ]));
    for line in desc.lines().take(5) {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(line.to_string(), Style::default().fg(TRANSCRIPT_MUTED)),
        ]));
    }
    lines
}

// ---------------------------------------------------------------------------
// Goal event rendering
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>()
    }

    #[test]
    fn test_render_bash_input_line() {
        let result = render_bash_input_line("ls -la");
        assert!(!result.is_empty());
        let text = line_text(&result[0]);
        assert!(text.contains("$"));
        assert!(text.contains("ls -la"));
    }

    #[test]
    fn test_render_bash_output_block() {
        let output = (0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = render_bash_output_block(&output, 10);
        assert!(!result.is_empty());
        // 10 content lines + 1 overflow indicator
        assert_eq!(result.len(), 11);
        let last = line_text(result.last().unwrap());
        assert!(last.contains("more lines"));
    }

    #[test]
    fn test_render_bash_output_block_no_overflow() {
        let output = "line 1\nline 2\nline 3";
        let result = render_bash_output_block(output, 10);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_render_tool_result_success_uses_30_lines() {
        let output = (0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = render_tool_result_success(&output, false);
        // 30 content lines + 1 overflow indicator = 31 (no separate header line)
        assert_eq!(result.len(), 31);
        let overflow_text = line_text(result.last().unwrap());
        assert!(overflow_text.contains("more lines"));
        assert!(!overflow_text.contains("ctrl+o"));
    }

    // ── New function tests ────────────────────────────────────────────────────

    #[test]
    fn test_render_system_api_error_short_message() {
        let result = render_system_api_error("Connection refused", None);
        assert!(!result.is_empty());
        let combined = result
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(combined.contains("API Error"));
        assert!(combined.contains("Connection refused"));
        // No retry line
        assert!(!combined.contains("Retrying"));
    }

    #[test]
    fn test_render_system_api_error_with_retry() {
        let result = render_system_api_error("Timeout", Some(30));
        let combined = result
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(combined.contains("API Error"));
        assert!(combined.contains("Timeout"));
        assert!(combined.contains("Retrying in 30s"));
    }

    #[test]
    fn test_render_system_api_error_long_message_shows_expand_hint() {
        let msg = (0..10)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = render_system_api_error(&msg, None);
        let combined = result
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("[expand]"),
            "should show [expand] hint when more than 5 lines"
        );
        assert!(combined.contains("5 more lines"));
    }

    #[test]
    fn test_render_user_command() {
        let result = render_user_command("doctor", "--verbose");
        assert!(!result.is_empty());
        let text = line_text(&result[0]);
        assert!(text.contains('\u{25b8}'), "should have ▸ prefix");
        assert!(text.contains("doctor"));
        assert!(text.contains("--verbose"));
    }

    #[test]
    fn goal_objective_renders_goal_active_block_not_user_command() {
        let result = render_user_command("goal", "Migrate to React");
        let header = line_text(&result[0]);
        let body = line_text(&result[1]);
        assert!(header.contains("GOAL ACTIVE"));
        assert!(
            !header.contains('\u{25b8}'),
            "should not show ▸ user-command prefix"
        );
        assert!(body.contains("Objective:"));
        assert!(body.contains("Migrate to React"));
    }

    #[test]
    fn goal_subcommands_render_as_normal_user_command() {
        for sub in ["status", "pause", "resume", "clear", "complete"] {
            let result = render_user_command("goal", sub);
            let text = line_text(&result[0]);
            assert!(
                text.contains('\u{25b8}'),
                "/goal {sub} should keep ▸ prefix"
            );
            assert!(text.contains(sub));
        }
    }

    #[test]
    fn goal_with_tokens_flag_strips_flag_from_objective() {
        let result = render_user_command("goal", "--tokens 250K Migrate to React");
        let body = line_text(&result[1]);
        assert!(body.contains("Migrate to React"));
        assert!(
            !body.contains("--tokens"),
            "flag should not appear in displayed objective"
        );
        assert!(!body.contains("250K"));
    }

    #[test]
    fn extract_goal_objective_returns_none_for_subcommands_and_empty() {
        assert!(extract_goal_objective_from_args("").is_none());
        assert!(extract_goal_objective_from_args("   ").is_none());
        assert!(extract_goal_objective_from_args("status").is_none());
        assert!(extract_goal_objective_from_args("pause now").is_none()); // first token is subcommand
        assert_eq!(
            extract_goal_objective_from_args("Migrate to React").as_deref(),
            Some("Migrate to React"),
        );
    }

    #[test]
    fn extract_goal_slash_objective_handles_typed_user_message() {
        assert_eq!(
            extract_goal_slash_objective("/goal build GPT 6 make no mistakes").as_deref(),
            Some("build GPT 6 make no mistakes"),
        );
        assert_eq!(
            extract_goal_slash_objective("/goal --tokens 250K Migrate to React").as_deref(),
            Some("Migrate to React"),
        );
        // Subcommands fall through.
        assert!(extract_goal_slash_objective("/goal status").is_none());
        assert!(extract_goal_slash_objective("/goal").is_none());
        // Not a /goal message.
        assert!(extract_goal_slash_objective("just a normal message").is_none());
        assert!(extract_goal_slash_objective("/goalbuild").is_none());
    }

    #[test]
    fn extract_goal_slash_objective_folds_trailing_lines_into_objective() {
        let text = "/goal Migrate to React\nwith strict typing\nand tests passing";
        let extracted = extract_goal_slash_objective(text).unwrap();
        assert!(extracted.starts_with("Migrate to React"));
        assert!(extracted.contains("strict typing"));
        assert!(extracted.contains("tests passing"));
    }

    #[test]
    fn test_render_user_memory_input() {
        let result = render_user_memory_input("project", "Operant");
        assert_eq!(result.len(), 2);
        let first = line_text(&result[0]);
        assert!(first.contains("# project: Operant"));
        let second = line_text(&result[1]);
        assert!(second.contains("Got it."));
    }

    #[test]
    fn test_render_user_local_command_output_with_overflow() {
        let output = (0..20)
            .map(|i| format!("out {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = render_user_local_command_output("ls", &output, 5);
        // 1 header + 5 body + 1 overflow = 7
        assert_eq!(result.len(), 7);
        let header = line_text(&result[0]);
        assert!(header.contains("!ls"));
        let overflow = line_text(result.last().unwrap());
        assert!(overflow.contains("15 more lines"));
    }

    #[test]
    fn test_render_user_local_command_output_no_overflow() {
        let output = "line1\nline2";
        let result = render_user_local_command_output("echo", output, 10);
        // 1 header + 2 body = 3
        assert_eq!(result.len(), 3);
        let header = line_text(&result[0]);
        assert!(header.contains("!echo"));
    }

    #[test]
    fn test_render_collapsed_read_search_no_hidden() {
        let paths = vec!["src/lib.rs", "src/main.rs"];
        let result = render_collapsed_read_search("Read", &paths, 0);
        assert!(!result.is_empty());
        let text = line_text(&result[0]);
        assert!(text.contains('\u{25b8}'), "should have ▸ prefix");
        assert!(text.contains("Read"));
        assert!(text.contains("src/lib.rs"));
        assert!(
            !text.contains("more"),
            "should not show 'more' when n_hidden is 0"
        );
    }

    #[test]
    fn test_render_collapsed_read_search_with_hidden() {
        let paths = vec!["a.rs", "b.rs"];
        let result = render_collapsed_read_search("Glob", &paths, 3);
        assert!(!result.is_empty());
        let text = line_text(&result[0]);
        assert!(text.contains("(+ 3 more)"));
    }

    #[test]
    fn test_render_task_assignment() {
        let result = render_task_assignment(
            "42",
            "Implement feature X",
            "Add the new widget system\nWith multi-line support",
        );
        assert!(!result.is_empty());
        let combined = result
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(combined.contains("Implement feature X"));
        assert!(combined.contains("task #42"));
        assert!(combined.contains("Add the new widget system"));
    }

    #[test]
    fn test_render_task_assignment_truncates_desc_at_5_lines() {
        let desc = (0..10)
            .map(|i| format!("desc line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = render_task_assignment("1", "Subject", &desc);
        let combined = result
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        // Only first 5 desc lines should appear
        assert!(combined.contains("desc line 4"));
        assert!(
            !combined.contains("desc line 5"),
            "should truncate desc at 5 lines"
        );
    }

    #[test]
    fn test_tui_render_bug_reproduce() {
        let text = "Hello!\n\n👋  I'm Operant, your\n\n AI assistant. How\n can I help you today?";
        let result = render_transcript_live_text(text, 24);
        assert_eq!(result.len(), 8);
        assert_eq!(line_text(&result[0]), "     Hello!");
        assert_eq!(line_text(&result[1]), "     ");
        assert_eq!(line_text(&result[2]), "     👋 I'm Operant,");
        assert_eq!(line_text(&result[3]), "     your");
        assert_eq!(line_text(&result[4]), "     ");
        assert_eq!(line_text(&result[5]), "     AI assistant.");
        assert_eq!(line_text(&result[6]), "     How can I help");
        assert_eq!(line_text(&result[7]), "     you today?");
    }

    #[test]
    fn test_normalize_markdown_newlines_specific() {
        use super::markdown::normalize_markdown_newlines;
        let input = "Hello! 👋  Ho\nw\n can I help you today?";
        let output = normalize_markdown_newlines(input);
        assert_eq!(output, "Hello! 👋  How can I help you today?");
    }

    // (iter-213: 18 broken test functions deleted — they referenced
    // render functions that were deleted in prior iterations:
    // render_agent_notification, render_attachment_message,
    // render_advisor_message, render_tool_result_cancelled/rejected,
    // render_shutdown_message, render_resource_update,
    // render_rate_limit_*, render_plan_*. The functions were
    // removed but the tests were never updated. YAGNI: delete
    // the tests rather than re-add unused render functions.)
}
