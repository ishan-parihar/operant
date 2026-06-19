use std::time::{Duration, Instant};

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub status: ToolStatus,
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolStatus {
    Running,
    Complete,
    Error,
}

impl ToolCall {
    pub fn new(name: String, arguments: String) -> Self {
        Self {
            name,
            arguments,
            start_time: Instant::now(),
            end_time: None,
            status: ToolStatus::Running,
            output: None,
        }
    }

    pub fn complete(&mut self, output: Option<String>) {
        self.end_time = Some(Instant::now());
        self.status = ToolStatus::Complete;
        self.output = output;
    }

    pub fn error(&mut self, error: String) {
        self.end_time = Some(Instant::now());
        self.status = ToolStatus::Error;
        self.output = Some(error);
    }

    pub fn elapsed(&self) -> Duration {
        match self.end_time {
            Some(end) => end.duration_since(self.start_time),
            None => self.start_time.elapsed(),
        }
    }

    pub fn format_tool_call(&self) -> String {
        let args = if self.arguments.len() > 50 {
            format!("{}...", &self.arguments[..47])
        } else {
            self.arguments.clone()
        };
        format!("{}({})", self.name, args)
    }
}

pub fn render_tool_trail(
    tools: &[ToolCall],
    accent: Color,
    muted: Color,
    error_color: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for tool in tools {
        let status_icon = match tool.status {
            ToolStatus::Running => "⏳",
            ToolStatus::Complete => "✓",
            ToolStatus::Error => "✗",
        };

        let status_color = match tool.status {
            ToolStatus::Running => accent,
            ToolStatus::Complete => Color::Green,
            ToolStatus::Error => error_color,
        };

        let elapsed = tool.elapsed();
        let elapsed_str = if elapsed.as_secs() > 0 {
            format!("{}s", elapsed.as_secs())
        } else {
            format!("{}ms", elapsed.as_millis())
        };

        let style = match tool.status {
            ToolStatus::Running => Style::default().fg(accent),
            ToolStatus::Complete => Style::default().fg(Color::Green),
            ToolStatus::Error => Style::default().fg(error_color),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", status_icon),
                Style::default().fg(status_color),
            ),
            Span::styled(tool.format_tool_call(), style),
            Span::styled(format!(" ({})", elapsed_str), Style::default().fg(muted)),
        ]));

        if let Some(output) = &tool.output {
            let preview = if output.len() > 100 {
                format!("{}...", &output[..97])
            } else {
                output.clone()
            };
            lines.push(Line::from(Span::styled(
                format!("  └─ {}", preview),
                Style::default().fg(muted),
            )));
        }
    }

    lines
}

pub fn render_tool_trail_summary(tools: &[ToolCall], accent: Color, muted: Color) -> Line<'static> {
    let running = tools
        .iter()
        .filter(|t| t.status == ToolStatus::Running)
        .count();
    let complete = tools
        .iter()
        .filter(|t| t.status == ToolStatus::Complete)
        .count();
    let errors = tools
        .iter()
        .filter(|t| t.status == ToolStatus::Error)
        .count();

    let mut spans = vec![Span::styled("Tools: ", Style::default().fg(muted))];

    if running > 0 {
        spans.push(Span::styled(
            format!("{} running", running),
            Style::default().fg(accent),
        ));
    }

    if complete > 0 {
        if !spans.is_empty() && running > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(Span::styled(
            format!("{} complete", complete),
            Style::default().fg(Color::Green),
        ));
    }

    if errors > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(", "));
        }
        spans.push(Span::styled(
            format!("{} errors", errors),
            Style::default().fg(Color::Red),
        ));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_new() {
        let tool = ToolCall::new(
            "read_file".to_string(),
            r#"{"path": "test.rs"}"#.to_string(),
        );
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.status, ToolStatus::Running);
        assert!(tool.end_time.is_none());
    }

    #[test]
    fn test_tool_call_complete() {
        let mut tool = ToolCall::new("test".to_string(), "{}".to_string());
        tool.complete(Some("result".to_string()));
        assert_eq!(tool.status, ToolStatus::Complete);
        assert!(tool.end_time.is_some());
    }

    #[test]
    fn test_tool_call_error() {
        let mut tool = ToolCall::new("test".to_string(), "{}".to_string());
        tool.error("failed".to_string());
        assert_eq!(tool.status, ToolStatus::Error);
    }

    #[test]
    fn test_format_tool_call() {
        let tool = ToolCall::new(
            "read_file".to_string(),
            r#"{"path": "test.rs"}"#.to_string(),
        );
        let formatted = tool.format_tool_call();
        assert!(formatted.starts_with("read_file("));
    }

    #[test]
    fn test_render_tool_trail() {
        let tools = vec![
            ToolCall::new("test1".to_string(), "{}".to_string()),
            ToolCall::new("test2".to_string(), "{}".to_string()),
        ];
        let lines = render_tool_trail(&tools, Color::Yellow, Color::Gray, Color::Red);
        assert_eq!(lines.len(), 2);
    }
}
