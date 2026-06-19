use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn render_markdown(text: &str, accent: Color, muted: Color) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in text.lines() {
        if line.starts_with("```") {
            if in_code_block {
                in_code_block = false;
                code_lang.clear();
            } else {
                in_code_block = true;
                code_lang = line[3..].trim().to_string();
            }
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(muted),
            )));
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 30)),
            )));
            continue;
        }

        let mut spans = Vec::new();
        let mut remaining = line;

        while !remaining.is_empty() {
            if remaining.starts_with("**") || remaining.starts_with("__") {
                let end = remaining[2..].find("**").or_else(|| remaining[2..].find("__"));
                if let Some(end) = end {
                    let bold_text = &remaining[2..end + 2];
                    spans.push(Span::styled(
                        bold_text.to_string(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                    remaining = &remaining[end + 4..];
                } else {
                    spans.push(Span::raw(remaining.to_string()));
                    remaining = "";
                }
            } else if remaining.starts_with('*') && !remaining.starts_with("**") {
                let end = remaining[1..].find('*');
                if let Some(end) = end {
                    let italic_text = &remaining[1..end + 1];
                    spans.push(Span::styled(
                        italic_text.to_string(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::ITALIC),
                    ));
                    remaining = &remaining[end + 2..];
                } else {
                    spans.push(Span::raw(remaining.to_string()));
                    remaining = "";
                }
            } else if remaining.starts_with('`') {
                let end = remaining[1..].find('`');
                if let Some(end) = end {
                    let code_text = &remaining[1..end + 1];
                    spans.push(Span::styled(
                        code_text.to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .bg(Color::Rgb(40, 40, 40)),
                    ));
                    remaining = &remaining[end + 2..];
                } else {
                    spans.push(Span::raw(remaining.to_string()));
                    remaining = "";
                }
            } else if remaining.starts_with("# ") {
                spans.push(Span::styled(
                    remaining[2..].to_string(),
                    Style::default()
                        .fg(accent)
                        .add_modifier(Modifier::BOLD),
                ));
                remaining = "";
            } else if remaining.starts_with("## ") {
                spans.push(Span::styled(
                    remaining[3..].to_string(),
                    Style::default()
                        .fg(accent)
                        .add_modifier(Modifier::BOLD),
                ));
                remaining = "";
            } else if remaining.starts_with("### ") {
                spans.push(Span::styled(
                    remaining[4..].to_string(),
                    Style::default()
                        .fg(accent)
                        .add_modifier(Modifier::BOLD),
                ));
                remaining = "";
            } else if remaining.starts_with("- ") || remaining.starts_with("* ") {
                spans.push(Span::styled("  • ", Style::default().fg(muted)));
                spans.push(Span::raw(remaining[2..].to_string()));
                remaining = "";
            } else if remaining.starts_with("> ") {
                spans.push(Span::styled(
                    "  │ ",
                    Style::default().fg(accent),
                ));
                spans.push(Span::styled(
                    remaining[2..].to_string(),
                    Style::default().fg(muted),
                ));
                remaining = "";
            } else {
                let next_special = remaining
                    .find("**")
                    .or_else(|| remaining.find("__"))
                    .or_else(|| remaining.find('*'))
                    .or_else(|| remaining.find('`'))
                    .or_else(|| remaining.find("# "))
                    .or_else(|| remaining.find("- "))
                    .or_else(|| remaining.find("* "))
                    .or_else(|| remaining.find("> "));

                if let Some(pos) = next_special {
                    spans.push(Span::raw(remaining[..pos].to_string()));
                    remaining = &remaining[pos..];
                } else {
                    spans.push(Span::raw(remaining.to_string()));
                    remaining = "";
                }
            }
        }

        if spans.is_empty() {
            lines.push(Line::from(Span::raw(line.to_string())));
        } else {
            lines.push(Line::from(spans));
        }
    }

    lines
}

pub fn strip_thinking_tags(text: &str) -> String {
    let mut result = text.to_string();
    let tags = [
        "<think>", "</think>",
        "<thinking>", "</thinking>",
        "<reasoning>", "</reasoning>",
        "<thought>", "</thought>",
        "<reasoning_scratchpad>", "</reasoning_scratchpad>",
    ];

    for tag in &tags {
        result = result.replace(tag, "");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_bold() {
        let lines = render_markdown("**bold text**", Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_render_italic() {
        let lines = render_markdown("*italic text*", Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::ITALIC));
    }

    #[test]
    fn test_render_code() {
        let lines = render_markdown("`code`", Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn test_render_heading() {
        let lines = render_markdown("# Heading", Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_render_list() {
        let lines = render_markdown("- item", Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains("•"));
    }

    #[test]
    fn test_render_blockquote() {
        let lines = render_markdown("> quote", Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains("│"));
    }

    #[test]
    fn test_strip_thinking_tags() {
        let text = "<think>reasoning</think>Hello";
        let stripped = strip_thinking_tags(text);
        assert_eq!(stripped, "reasoningHello");
    }

    #[test]
    fn test_render_code_block() {
        let text = "```\nfn main() {}\n```";
        let lines = render_markdown(text, Color::Yellow, Color::Gray);
        assert_eq!(lines.len(), 3);
    }
}
