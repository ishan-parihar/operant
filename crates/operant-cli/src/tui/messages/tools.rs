// messages/tools.rs — Tool-use and tool-result renderers.
//
// Extracted from messages/mod.rs. Renders tool call summaries, file
// read/write results, generic success/error results, and bash I/O.

use super::*;
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
            };
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
pub(crate) fn render_file_read_result(output: &str) -> Vec<Line<'static>> {
    let n = output.lines().count();
    vec![Line::from(vec![Span::styled(
        format!("  Read {} line{}", n, if n == 1 { "" } else { "s" }),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    )])]
}

/// Render a file-edit/write tool result: `Updated file` or `Created file`.
pub(crate) fn render_file_op_result(is_create: bool) -> Vec<Line<'static>> {
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
#[allow(dead_code)] // Bash input line renderer
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
