// render/tools.rs — Tool block rendering and system annotations.

use crate::tui::app::{App, SystemAnnotation, SystemMessageStyle, ToolStatus};
use crate::tui::figures;
use crate::tui::messages::RenderContext;
use crate::app::ToolUseBlock;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{shimmer_spans, truncate_text, ACCENT_PRIMARY};

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
    let input_val: serde_json::Value =
        serde_json::from_str(&block.input_json).unwrap_or(serde_json::Value::Null);
    let normalized = block.name.to_ascii_lowercase();
    let running = block.status == ToolStatus::Running;
    let mut summary = crate::messages::extract_tool_summary(&block.name, &input_val);
    let title = if normalized == "task" || normalized == "agent" {
        if let Some(description) = input_val
            .get("description")
            .and_then(|value| value.as_str())
        {
            summary = description.to_string();
        }
        crate::messages::subagent_title(&input_val)
    } else {
        match (normalized.as_str(), running) {
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

    let accent = if block.status == ToolStatus::Error {
        Color::Rgb(255, 140, 0)
    } else {
        ACCENT_PRIMARY
    };
    let mut header_spans = vec![Span::styled(
        "   ~ ".to_string(),
        Style::default().fg(accent),
    )];
    if running {
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

