// render/tools.rs — Tool block rendering and system annotations.

use crate::tui::app::{SystemAnnotation, SystemMessageStyle, ToolStatus, ToolUseBlock};
use crate::tui::figures;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{ACCENT_PRIMARY, shimmer_spans};

pub(crate) fn build_tool_names(
    messages: &[crate::tui::adapter_types::types::Message],
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for msg in messages {
        for block in msg.content_blocks() {
            if let crate::tui::adapter_types::types::ContentBlock::ToolUse { id, name, .. } = block
            {
                map.insert(id.clone(), name.clone());
            }
        }
    }
    map
}

// â”€â”€ System annotation (compact boundary, info notices) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub(crate) fn render_system_annotation_lines(
    lines: &mut Vec<Line<'static>>,
    ann: &SystemAnnotation,
    width: usize,
) {
    // Compact boundary: show âœ» prefix with dimmed text
    if ann.style == SystemMessageStyle::Compact {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", figures::TEARDROP_ASTERISK),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                ann.text.clone(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
        lines.push(Line::from(""));
        return;
    }

    let (text_color, border_color) = match ann.style {
        SystemMessageStyle::Info => (Color::DarkGray, Color::DarkGray),
        SystemMessageStyle::Compact => (Color::DarkGray, Color::DarkGray),
    };

    // Centred, padded rule: "â”€â”€â”€ text â”€â”€â”€"
    let text = ann.text.as_str();
    let inner_width = width.saturating_sub(4);
    let text_len = text.len();
    let dashes = inner_width.saturating_sub(text_len + 2);
    let left = dashes / 2;
    let right = dashes - left;

    lines.push(Line::from(vec![
        Span::styled(
            format!("  {}", "\u{2500}".repeat(left)),
            Style::default().fg(border_color),
        ),
        Span::styled(
            format!("\u{2500} {} \u{2500}", text),
            Style::default().fg(text_color).add_modifier(Modifier::DIM),
        ),
        Span::styled("\u{2500}".repeat(right), Style::default().fg(border_color)),
    ]));
    lines.push(Line::from(""));
}

// â”€â”€ Tool use block â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub(crate) fn render_tool_block_lines(
    lines: &mut Vec<Line<'static>>,
    block: &crate::app::ToolUseBlock,
    frame_count: u64,
) {
    let ToolCallLabels {
        normalized,
        title,
        summary,
        input: input_val,
    } = tool_call_labels(block);
    // Queued = parsed but still waiting on a worker permit; it reads as
    // in-flight (present-progressive titles) but renders muted, not shimmering.
    let queued = block.status == ToolStatus::Queued;
    let in_flight = block.status.is_pending();

    let accent = if block.status == ToolStatus::Error {
        Color::Rgb(255, 140, 0)
    } else {
        ACCENT_PRIMARY
    };
    let mut header_spans = vec![Span::styled(
        "   ~ ".to_string(),
        Style::default().fg(accent),
    )];
    if queued {
        // Distinct from Running: dim/gray, plus an explicit `queued` label so
        // a tool waiting for a pool permit never reads as one already running.
        header_spans.push(Span::styled(
            title,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
        header_spans.push(Span::styled(
            "  queued".to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM | Modifier::ITALIC),
        ));
    } else if in_flight {
        header_spans.extend(shimmer_spans(&title, frame_count));
    } else {
        header_spans.push(Span::styled(
            title,
            Style::default()
                .fg(if block.status == ToolStatus::Error {
                    accent
                } else {
                    Color::White
                })
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header_spans));

    if !summary.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(summary, Style::default().fg(Color::DarkGray)),
        ]));
    }

    if normalized == "bash" || normalized == "powershell" {
        let command = input_val
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
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
                Span::styled("     $ ".to_string(), Style::default().fg(Color::Green)),
                Span::styled(display, Style::default().fg(Color::White)),
            ]));
        }
    }

    // Output preview (done/error state)
    if let Some(ref preview) = block.output_preview {
        let preview_style = match block.status {
            ToolStatus::Error => Style::default().fg(Color::Rgb(255, 140, 0)),
            _ => Style::default().fg(Color::DarkGray),
        };
        for line_text in preview.lines() {
            if line_text.starts_with('\u{2026}') {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(
                        line_text.to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(line_text.to_string(), preview_style),
                ]));
            }
        }
    }
}

// ─── Batched tool calls ────────────────────────────────────────────────────

/// Everything a tool block derives from its JSON input: the lowercased name
/// (which picks the title dialect), the present/past-tense title, the human
/// summary, and the parsed input. Derived once so a call describes itself
/// identically whether it renders standalone or as a group member.
struct ToolCallLabels {
    normalized: String,
    title: String,
    summary: String,
    input: serde_json::Value,
}

fn tool_call_labels(block: &ToolUseBlock) -> ToolCallLabels {
    let input: serde_json::Value =
        serde_json::from_str(&block.input_json).unwrap_or(serde_json::Value::Null);
    let normalized = block.name.to_ascii_lowercase();
    let in_flight = block.status.is_pending();
    let mut summary = crate::messages::extract_tool_summary(&block.name, &input);
    let title = if normalized == "task" || normalized == "agent" {
        if let Some(description) = input.get("description").and_then(|value| value.as_str()) {
            summary = description.to_string();
        }
        crate::messages::subagent_title(&input)
    } else {
        match (normalized.as_str(), in_flight) {
            ("bash" | "powershell", true) => "Running command".to_string(),
            ("bash" | "powershell", false) => "Ran command".to_string(),
            ("read", true) => "Reading file".to_string(),
            ("read", false) => "Read file".to_string(),
            ("write" | "apply_patch", true) => "Writing file".to_string(),
            ("write" | "apply_patch", false) => "Wrote file".to_string(),
            ("edit", true) => "Editing file".to_string(),
            ("edit", false) => "Edited file".to_string(),
            ("glob" | "list", true) => "Listing files".to_string(),
            ("glob" | "list", false) => "Listed files".to_string(),
            ("grep" | "codesearch", true) => "Searching code".to_string(),
            ("grep" | "codesearch", false) => "Searched code".to_string(),
            ("webfetch", true) => "Fetching page".to_string(),
            ("webfetch", false) => "Fetched page".to_string(),
            ("websearch", true) => "Searching web".to_string(),
            ("websearch", false) => "Searched web".to_string(),
            _ => block.name.clone(),
        }
    };
    ToolCallLabels {
        normalized,
        title,
        summary,
        input,
    }
}

/// The single line that identifies one call inside a group: its summary when
/// the tool produced one, otherwise the derived title.
fn tool_block_label(block: &ToolUseBlock) -> String {
    let labels = tool_call_labels(block);
    if labels.summary.is_empty() {
        labels.title
    } else {
        labels.summary
    }
}

/// A maximal run of adjacent same-name tool blocks, as a half-open range into a
/// turn's `tool_blocks`.
pub(crate) type ToolGroup = std::ops::Range<usize>;

/// Group adjacent same-name tool blocks so a concurrent batch renders as one
/// block instead of N undifferentiated siblings.
///
/// `assistant_count` is how many assistant messages the turn renders before its
/// tool blocks are interleaved: when `i <= assistant_count` the slot just above
/// block `i` held an assistant message, so `i` is causally *after* that text and
/// must not be folded into the group above it.
///
/// Callers pass a single turn's blocks — `build_transcript_turns` already
/// partitions `tool_use_blocks` per user message — so a group can never span a
/// user-message boundary or a turn, and the append-only order is preserved.
pub(crate) fn tool_group_ranges(
    tool_blocks: &[&ToolUseBlock],
    assistant_count: usize,
) -> Vec<ToolGroup> {
    let mut groups: Vec<ToolGroup> = Vec::new();
    for (i, block) in tool_blocks.iter().enumerate() {
        match groups.last_mut() {
            Some(last)
                if i > assistant_count
                    && last.end == i
                    && tool_blocks[i - 1].name == block.name =>
            {
                last.end = i + 1;
            }
            _ => groups.push(i..i + 1),
        }
    }
    groups
}

/// Render one run of tool calls. A lone call keeps the full standalone
/// treatment (command preview, output tail); an actual batch gets the grouped
/// progress meter. Splitting here rather than at each call site keeps the
/// "when is a batch" decision in exactly one place.
pub(crate) fn render_tool_items_lines(
    lines: &mut Vec<Line<'static>>,
    blocks: &[&ToolUseBlock],
    frame_count: u64,
) {
    match blocks {
        [single] => render_tool_block_lines(lines, single, frame_count),
        _ => render_tool_group_lines(lines, blocks, frame_count),
    }
}

/// Render a batch of same-name tool calls as one block: a progress-meter
/// header (`N/M done · running: … · last done: …`) plus one status row per
/// sub-call. Reuses the standalone block's own title/summary derivation and
/// status vocabulary — no new colour scheme.
pub(crate) fn render_tool_group_lines(
    lines: &mut Vec<Line<'static>>,
    blocks: &[&ToolUseBlock],
    frame_count: u64,
) {
    let Some(first) = blocks.first() else {
        return;
    };
    let total = blocks.len();
    let done = blocks
        .iter()
        .filter(|block| block.status == ToolStatus::Done)
        .count();
    let errored = blocks
        .iter()
        .filter(|block| block.status == ToolStatus::Error)
        .count();
    let running: Vec<&ToolUseBlock> = blocks
        .iter()
        .filter(|block| block.status == ToolStatus::Running)
        .copied()
        .collect();
    let queued: Vec<&ToolUseBlock> = blocks
        .iter()
        .filter(|block| block.status == ToolStatus::Queued)
        .copied()
        .collect();
    // The transcript model carries no completion timestamps, so "last done" is
    // the most recently settled call in append order. Blocks are appended on
    // ToolStart and a queued call is always settled after the ones queued
    // before it, so the tail of the settled set is the freshest signal the
    // model has — the render layer must not invent an ordering it can't see.
    let last_settled = blocks
        .iter()
        .rev()
        .find(|block| !block.status.is_pending())
        .copied();

    // Errors tint the whole group, matching the standalone block's accent.
    let accent = if errored > 0 {
        Color::Rgb(255, 140, 0)
    } else {
        ACCENT_PRIMARY
    };
    let mut header = vec![Span::styled(
        "   ~ ".to_string(),
        Style::default().fg(accent),
    )];
    header.push(Span::styled(
        first.name.clone(),
        Style::default()
            .fg(if errored > 0 { accent } else { Color::White })
            .add_modifier(Modifier::BOLD),
    ));
    header.push(Span::styled(
        format!("  {done}/{total} done"),
        Style::default().fg(Color::DarkGray),
    ));
    if errored > 0 {
        header.push(Span::styled(
            format!("  · {errored} error"),
            Style::default()
                .fg(Color::Rgb(255, 140, 0))
                .add_modifier(Modifier::DIM),
        ));
    }
    // Exactly one "what is moving now" clause: a permit-held call wins,
    // otherwise fall back to the queue head so a pool that has not drained yet
    // still says so instead of reading as idle.
    let in_flight = running
        .first()
        .map(|head| ("running", *head))
        .or_else(|| queued.first().map(|head| ("queued", *head)));
    if let Some((label, block)) = in_flight {
        header.push(Span::styled(
            format!("  · {label}: {}", tool_block_label(block)),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
        let in_flight_total = running.len() + queued.len();
        if in_flight_total > 1 {
            header.push(Span::styled(
                format!(" +{} more", in_flight_total - 1),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM | Modifier::ITALIC),
            ));
        }
    }
    if let Some(block) = last_settled {
        header.push(Span::styled(
            format!("  · last done: {}", tool_block_label(block)),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }
    lines.push(Line::from(header));

    for block in blocks {
        let labels = tool_call_labels(block);
        let label = if labels.summary.is_empty() {
            labels.title
        } else {
            labels.summary
        };
        let mut spans = vec![Span::raw("     ")];
        match block.status {
            // Queued keeps the standalone block's muted, explicitly-labelled
            // treatment, so waiting on a permit never reads as running.
            ToolStatus::Queued => {
                spans.push(Span::styled(
                    "\u{2026} ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ));
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ));
                spans.push(Span::styled(
                    "  queued".to_string(),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                ));
            }
            ToolStatus::Running => {
                spans.push(Span::styled(
                    "\u{2026} ".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
                spans.extend(shimmer_spans(&label, frame_count));
            }
            ToolStatus::Done => {
                spans.push(Span::styled(
                    format!("{} ", figures::black_circle()),
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));
            }
            ToolStatus::Error => {
                spans.push(Span::styled(
                    "\u{26a0} ".to_string(),
                    Style::default().fg(Color::Rgb(255, 140, 0)),
                ));
                spans.push(Span::styled(
                    label,
                    Style::default().fg(Color::Rgb(255, 140, 0)),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
}
