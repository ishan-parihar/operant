// messages/commands.rs — System, command, and goal-event renderers.
//
// Extracted from messages/mod.rs. Renders API errors, slash-command
// echoes, memory inputs, local command output, collapsed read/search,
// task assignments, and goal blocks.

use super::*;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

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
    if name == "goal"
        && let Some(objective) = extract_goal_objective_from_args(args)
    {
        return render_goal_active_block(&objective);
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
pub(crate) fn extract_goal_slash_objective(text: &str) -> Option<String> {
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
pub(crate) fn extract_goal_objective_from_args(args: &str) -> Option<String> {
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
pub(crate) fn render_goal_active_block(objective: &str) -> Vec<Line<'static>> {
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
