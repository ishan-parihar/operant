// render/messages.rs — Message pane rendering, turn items, live content.

use crate::tui::app::{App, SystemAnnotation, ToolStatus};
use crate::tui::messages::{
    RenderContext, render_thinking_live_content,
    render_transcript_assistant_message_tagged, render_transcript_assistant_meta,
    render_transcript_live_text, render_transcript_user_message,
};
use crate::tui::transcript_turn::{TranscriptTurn, build_transcript_turns};
use crate::tui::virtual_list::VirtualList;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::cache::*;
use super::{append_turn_items, build_tool_names, render_system_annotation_lines, render_tool_block_lines, shimmer_spans};
use super::{ACCENT_PRIMARY, RenderedLineItem};

fn render_messages(frame: &mut Frame, app: &App, area: Rect) {
    let content_area = area; // (iter-143: plugin_hints deleted — Vec was always empty)

    // Welcome block and banner removed — always use the full content area for messages.
    let msg_area = content_area;

    // Store the actual message pane bounds for mouse event handling (text selection, scrolling).
    app.last_msg_area.set(msg_area);

    let lines = render_message_items(app, msg_area.width);
    // Append live streaming content in the correct order:
    // completed messages → thinking → tool calls → streaming text.
    // (iter-118 — fixes causation chain ordering bug.)
    let lines = append_live_content(app, lines, msg_area.width);

    // Highlight search matches in transcript when global search is active
    let lines = if app.global_search.visible && !app.global_search.query.is_empty() {
        let query_lc = app.global_search.query.to_lowercase();
        lines
            .into_iter()
            .map(|mut item| {
                if item.search_text.to_lowercase().contains(query_lc.as_str()) {
                    // Re-render the line with yellow highlight on matching spans
                    let highlighted_spans: Vec<Span<'static>> = item
                        .line
                        .spans
                        .into_iter()
                        .map(|span| {
                            if span.content.to_lowercase().contains(query_lc.as_str()) {
                                Span::styled(
                                    span.content,
                                    span.style.bg(Color::Rgb(60, 50, 0)).fg(Color::Yellow),
                                )
                            } else {
                                span
                            }
                        })
                        .collect();
                    item.line = ratatui::text::Line::from(highlighted_spans);
                }
                item
            })
            .collect()
    } else {
        lines
    };

    // Compute total virtual height and apply scroll clamping.
    // When auto_scroll is on we always show the tail; otherwise we respect
    // the user's scroll_offset.
    let content_height = lines.len() as u16;
    let visible_height = msg_area.height; // no borders, full height available
    let max_scroll = content_height.saturating_sub(visible_height) as usize;
    // scroll_offset counts lines above the bottom (0 = at bottom).
    // ratatui scroll() takes an absolute top-row index, so convert:
    //   top_row = max_scroll - scroll_offset  (clamped to [0, max_scroll])
    let scroll = if app.auto_scroll {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let mut visible_rows: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
    let mut thinking_rows: std::collections::HashMap<u16, u64> = std::collections::HashMap::new();
    for (idx, item) in lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(msg_area.height as usize)
    {
        let screen_row = msg_area
            .y
            .saturating_add((idx.saturating_sub(scroll)) as u16);
        if let Some(message_index) = item.message_index {
            visible_rows.insert(screen_row, message_index);
        }
        if let Some(hash) = item.thinking_hash {
            thinking_rows.insert(screen_row, hash);
        }
    }
    *app.message_row_map.borrow_mut() = visible_rows;
    *app.thinking_row_map.borrow_mut() = thinking_rows;

    // No border — messages render directly into the area.
    let mut list = VirtualList::new();
    list.viewport_height = msg_area.height;
    list.sticky_bottom = app.auto_scroll;
    list.set_items(lines);
    list.scroll_offset = scroll as u16;

    // Track scroll offset for selection validation
    app.last_render_scroll_offset.set(scroll as u16);

    list.render(msg_area, frame.buffer_mut());

    // Scrollbar: thin vertical strip flush with the right edge — no arrow
    // caps, no visible track, muted thumb color. Mirrors Windows Terminal /
    // most modern terminal scrollbars rather than ratatui's chunky default.
    if content_height > visible_height {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

        // ratatui 0.29's Scrollbar maps `position` over `content_length - 1`,
        // not over a 0..=max_scroll range. Passing `content_height` directly
        // makes the thumb top out at `content / (content + viewport)` of the
        // track when fully scrolled — i.e. it never reaches the bottom.
        // Fix: tell ratatui the content length is the number of distinct
        // scroll positions (`max_scroll + 1`), keeping `viewport_content_length`
        // for the proportional thumb size.
        let content_len = max_scroll + 1;
        let mut scrollbar_state = ScrollbarState::new(content_len)
            .position(scroll.min(max_scroll))
            .viewport_content_length(visible_height as usize);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_symbol("\u{2590}") // ▐ right half block — thin vertical strip
            .thumb_style(Style::default().fg(Color::Rgb(110, 110, 130)));

        frame.render_stateful_widget(scrollbar, msg_area, &mut scrollbar_state);
    }

    // “â†” N new messages” indicator when scrolled up and new messages arrived.
    if app.new_messages_while_scrolled > 0 && msg_area.height > 4 && msg_area.width > 20 {
        let indicator = format!(
            " \u{2193} {} new message{} ",
            app.new_messages_while_scrolled,
            if app.new_messages_while_scrolled == 1 {
                ""
            } else {
                "s"
            }
        );
        let ind_len = unicode_width::UnicodeWidthStr::width(indicator.as_str()) as u16;
        let ind_x = msg_area
            .x
            .saturating_add(msg_area.width.saturating_sub(ind_len + 2));
        let ind_y = msg_area.y + msg_area.height.saturating_sub(1);
        let ind_area = Rect {
            x: ind_x,
            y: ind_y,
            width: ind_len.min(msg_area.width.saturating_sub(2)),
            height: 1,
        };
        let ind_line = Line::from(vec![Span::styled(
            indicator,
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )]);
        frame.render_widget(Paragraph::new(vec![ind_line]), ind_area);
    }
}

fn push_rendered_items(
    items: &mut Vec<RenderedLineItem>,
    lines: Vec<Line<'static>>,
    message_index: Option<usize>,
    mark_first_header: bool,
) {
    for (index, line) in lines.into_iter().enumerate() {
        items.push(RenderedLineItem {
            search_text: flatten_line_text(&line),
            is_header: mark_first_header && index == 0,
            message_index,
            thinking_hash: None,
            line,
        });
    }
}

/// Push tagged lines from `render_transcript_assistant_message_tagged`.
/// Lines with `Some(hash)` become clickable thinking headers.
fn push_rendered_items_tagged(
    items: &mut Vec<RenderedLineItem>,
    tagged: Vec<(Line<'static>, Option<u64>)>,
    message_index: Option<usize>,
) {
    for (line, thinking_hash) in tagged {
        items.push(RenderedLineItem {
            search_text: flatten_line_text(&line),
            is_header: false,
            message_index,
            thinking_hash,
            line,
        });
    }
}

fn push_blank_item(items: &mut Vec<RenderedLineItem>) {
    push_rendered_items(items, vec![Line::from("")], None, false);
}

#[allow(clippy::too_many_arguments)]
fn append_turn_items(
    items: &mut Vec<RenderedLineItem>,
    turn: &TranscriptTurn<'_>,
    width: u16,
    tool_names: &std::collections::HashMap<String, String>,
    expanded_thinking: &std::collections::HashSet<u64>,
    show_reasoning: bool,
    frame_count: u64,
    accent: Color,
) {
    push_rendered_items(
        items,
        render_transcript_user_message(turn.user_message, turn.metadata, width),
        Some(turn.user_index),
        true,
    );

    enum SectionContent {
        Plain(Vec<Line<'static>>),
        Tagged(Vec<(Line<'static>, Option<u64>)>),
    }

    let mut sections: Vec<(SectionContent, Option<usize>)> = Vec::new();

    // Interleave assistant messages and tool blocks in the correct causal
    // order. The agent loop produces: text → tool → text → tool → text.
    // Rendering all messages first then all tools breaks the causation chain.
    // (iter-260 — user-reported bug: tool call order was incorrect.)
    let msg_count = turn.assistant_messages.len();
    let tool_count = turn.tool_blocks.len();
    let max_len = msg_count.max(tool_count);

    for i in 0..max_len {
        // Render assistant message at this position (if it exists)
        if i < msg_count {
            let (message_index, message) = &turn.assistant_messages[i];
            let tagged = render_transcript_assistant_message_tagged(
                message,
                &RenderContext {
                    width,
                    show_thinking: show_reasoning,
                    tool_names: tool_names.clone(),
                    expanded_thinking: expanded_thinking.clone(),
                },
            );
            if !tagged.is_empty() {
                sections.push((SectionContent::Tagged(tagged), Some(*message_index)));
            }
        }

        // Render tool block at this position (if it exists)
        if i < tool_count {
            let block = turn.tool_blocks[i];
            let mut lines = Vec::new();
            render_tool_block_lines(&mut lines, block, frame_count);
            if !lines.is_empty() {
                sections.push((
                    SectionContent::Plain(lines),
                    Some(turn.primary_message_index()),
                ));
            }
        }
    }

    // NOTE: Live thinking, thinking shimmer, and live text are rendered by
    // append_live_content (not here) to avoid duplicating streaming content.
    // This was causing visible duplication/glitching during streaming.

    if !turn.active {
        if let Some(meta_line) = render_transcript_assistant_meta(turn.metadata, accent) {
            if turn.has_visible_assistant_content() {
                sections.push((
                    SectionContent::Plain(vec![meta_line]),
                    Some(turn.primary_message_index()),
                ));
            }
        }
    }

    if !sections.is_empty() {
        push_blank_item(items);
        let total_sections = sections.len();
        for (index, (content, message_index)) in sections.into_iter().enumerate() {
            match content {
                SectionContent::Plain(lines) => {
                    push_rendered_items(items, lines, message_index, false)
                }
                SectionContent::Tagged(tagged) => {
                    push_rendered_items_tagged(items, tagged, message_index)
                }
            }
            if index + 1 < total_sections {
                push_blank_item(items);
            }
        }
    }

    push_blank_item(items);
}

fn render_message_items(app: &App, width: u16) -> Vec<RenderedLineItem> {
    let streaming =
        app.is_streaming || !app.streaming_text.is_empty() || !app.streaming_thinking.is_empty();
    let has_running_tool_blocks = app
        .tool_use_blocks
        .iter()
        .any(|block| block.status == ToolStatus::Running);
    let cacheable = !streaming && !has_running_tool_blocks;

    // Fast path: nothing live — use the full-result cache (ptr-stable check).
    let full_key = MessageLinesCacheKey {
        width,
        transcript_version: app.transcript_version.get(),
        messages_ptr: app.messages.as_ptr() as usize,
        messages_len: app.messages.len(),
        annotations_ptr: app.system_annotations.as_ptr() as usize,
        annotations_len: app.system_annotations.len(),
        thinking_expanded_len: app.thinking_expanded.len(),
    };
    if cacheable {
        if let Some(lines) = MESSAGE_LINES_CACHE.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|c| c.key == full_key)
                .map(|c| c.lines.clone())
        }) {
            return lines;
        }
    }

    let completed_key = CompletedMsgCacheKey {
        width,
        transcript_version: app.transcript_version.get(),
        messages_len: app.messages.len(),
        annotations_len: app.system_annotations.len(),
        thinking_expanded_len: app.thinking_expanded.len(),
    };
    let build_items = || {
        let tool_names = build_tool_names(&app.messages);
        let turns = build_transcript_turns(app);
        let mut turn_map = std::collections::HashMap::new();
        for turn in &turns {
            turn_map.insert(turn.user_index, turn);
        }

        let mut items = Vec::new();
        let total = app.messages.len();
        let mut index = 0usize;
        while index <= total {
            for ann in app
                .system_annotations
                .iter()
                .filter(|ann| ann.after_index == index)
            {
                let mut lines = Vec::new();
                render_system_annotation_lines(&mut lines, ann, width as usize);
                push_rendered_items(&mut items, lines, None, false);
            }

            if index >= total {
                break;
            }

            let message = &app.messages[index];
            if message.role == Role::User {
                if let Some(&turn) = turn_map.get(&index) {
                    append_turn_items(
                        &mut items,
                        turn,
                        width,
                        &tool_names,
                        &app.thinking_expanded,
                        app.show_reasoning,
                        app.frame_count,
                        app.accent_color,
                    );
                    index = turn.end_message_index + 1;
                    continue;
                }
            }

            let tagged = render_transcript_assistant_message_tagged(
                message,
                &RenderContext {
                    width,
                    show_thinking: app.show_reasoning,
                    tool_names: tool_names.clone(),
                    expanded_thinking: app.thinking_expanded.clone(),
                },
            );
            push_rendered_items_tagged(&mut items, tagged, Some(index));
            push_blank_item(&mut items);
            index += 1;
        }

        if total == 0 && !app.tool_use_blocks.is_empty() {
            for block in &app.tool_use_blocks {
                let mut lines = Vec::new();
                render_tool_block_lines(&mut lines, block, app.frame_count);
                push_rendered_items(&mut items, lines, None, false);
                push_blank_item(&mut items);
            }
        }

        items
    };
    let completed_lines: Vec<RenderedLineItem> = if cacheable {
        if let Some(lines) = COMPLETED_MSG_CACHE.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|c| c.key == completed_key)
                .map(|c| c.lines.clone())
        }) {
            lines
        } else {
            let items = build_items();
            COMPLETED_MSG_CACHE.with(|cache| {
                *cache.borrow_mut() = Some(CompletedMsgCache {
                    key: completed_key,
                    lines: items.clone(),
                });
            });
            items
        }
    } else {
        build_items()
    };

    // If there is no live content, store in the full cache and return.
    if cacheable {
        MESSAGE_LINES_CACHE.with(|cache| {
            *cache.borrow_mut() = Some(MessageLinesCache {
                key: full_key,
                lines: completed_lines.clone(),
            });
        });
        return completed_lines;
    }

    completed_lines
}

/// Append live streaming content (thinking, text, running tool blocks) to the
/// completed message items. This ensures the correct causation order:
/// completed messages → live thinking → live tool calls → live text.
/// (iter-118 — user-reported bug: thinking was always at the bottom while
/// tool calls piled up above it, breaking the causation chain order.)
fn append_live_content(
    app: &App,
    mut items: Vec<RenderedLineItem>,
    width: u16,
) -> Vec<RenderedLineItem> {
    // 1. Live thinking (appears FIRST — model thinks before acting).
    //    Includes a "▼ Thinking" header with shimmer so the user sees visual
    //    feedback that the model is working.
    if !app.streaming_thinking.is_empty() {
        let mut header_spans = vec![Span::raw("  ▼ ")];
        header_spans.extend(shimmer_spans("Thinking", app.frame_count));
        let mut thinking_lines = vec![Line::from(header_spans)];
        thinking_lines.extend(render_thinking_live_content(&app.streaming_thinking, width));
        push_rendered_items(&mut items, thinking_lines, None, false);
        push_blank_item(&mut items);
    }

    // Tool blocks are rendered by append_turn_items (not here) — they belong
    // within their respective turns for correct per-turn ordering.

    // 3. "Thinking" shimmer when the turn is active but no text or
    //    thinking content has arrived yet — gives visual feedback that the
    //    model is working (especially for providers without thinking support).
    if app.is_streaming
        && app.streaming_text.is_empty()
        && app.streaming_thinking.is_empty()
        && app
            .tool_use_blocks
            .iter()
            .all(|b| b.status != ToolStatus::Running)
    {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(shimmer_spans("Thinking", app.frame_count));
        push_rendered_items(&mut items, vec![Line::from(spans)], None, false);
    }

    // 4. Live streaming text (the model's final response — appears LAST).
    //    Uses the same rendering path as committed text
    //    (render_transcript_live_text) so the text width and indent are
    //    consistent between streaming and committed states. Without this,
    //    text jumps horizontally when streaming ends. (iter-260)
    //    Reuses cached render when unchanged to avoid re-running syntect
    //    every frame. (C1)
    if !app.streaming_text.is_empty() {
        let text_lines = STREAMING_TEXT_CACHE.with(|cache| {
            let mut slot = cache.borrow_mut();
            if let Some(cached) = slot.as_ref() {
                if cached.width == width && cached.text == app.streaming_text {
                    return cached.lines.clone();
                }
            }
            let lines = render_transcript_live_text(&app.streaming_text, width);
            *slot = Some(StreamingTextCache {
                width,
                text: app.streaming_text.clone(),
                lines: lines.clone(),
            });
            lines
        });
        push_rendered_items(&mut items, text_lines, None, false);
    }

    items
}

// â”€â”€ Welcome / startup screen â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Render the OPERANT ASCII wordmark banner above the welcome box.
///
/// The banner is responsive: full 7-line art + dim version rule at >=80 cols,
/// compact 4-line art + version rule at >=40 cols, nothing below 40 cols (the
/// welcome box itself shows a styled text fallback). The art is centered
/// horizontally within `area`.
