// messages/tools.rs — Tool-use and tool-result renderers.
//
// Extracted from messages/mod.rs. Renders tool call summaries, file
// read/write results, generic success/error results, and bash I/O.

use super::*;
use crate::tui::render::{display_width, take_width};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

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

pub(crate) fn render_tool_use_inner(
    tool_name: &str,
    input: &serde_json::Value,
) -> Vec<Line<'static>> {
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
                    Span::styled(
                        "  ~ ".to_string(),
                        Style::default().fg(theme_colors::accent()),
                    ),
                    Span::styled(
                        subagent_title(input),
                        Style::default()
                            .fg(theme_colors::text())
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                if !summary.is_empty() {
                    task_lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(summary, Style::default().fg(theme::dim_color())),
                    ]));
                }
                task_lines
            };
        }
        _ => tool_name,
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  ~ ".to_string(),
            Style::default().fg(theme_colors::accent()),
        ),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme_colors::text())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    if !summary.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(summary, Style::default().fg(theme::dim_color())),
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
            // Budget in display cells, not chars: a CJK glyph is one char but
            // two columns, so a char budget would overflow the block.
            let display = take_width(cmd_line, 160);
            let display = if display_width(&display) < display_width(cmd_line) {
                format!("{display}\u{2026}")
            } else {
                display
            };
            lines.push(Line::from(vec![
                Span::styled(
                    "    $ ".to_string(),
                    Style::default()
                        .fg(theme_colors::success())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    display,
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    lines
}

/// Render a tool result (error variant).
///
/// The four generic result renderers that used to live here
/// (`render_file_read_result`, `render_file_op_result`,
/// `render_tool_result_success`, `render_tool_result_error`) were unreachable:
/// the live tool-block renderer in `render/tools.rs` paints every result the
/// transcript shows. They went in the W5 palette/dead-code sweep together
/// with `TOOL_RESULT_MAX_LINES`.

/// Render a bash command input line with a green `$ ` prefix.
#[allow(dead_code)] // Bash input line renderer
pub fn render_bash_input_line(command: &str) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled(
            "  $ ".to_string(),
            Style::default()
                .fg(theme_colors::success())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            command.to_string(),
            Style::default()
                .fg(theme_colors::text())
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
