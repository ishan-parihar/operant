// messages/tests.rs — Unit tests for the messages module.
//
// Extracted from messages/mod.rs.

use super::*;
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|s| s.content.to_string())
        .collect::<String>()
}

/// True when some span holding `needle` carries `modifier`.
fn span_has_modifier(line: &Line<'_>, needle: &str, modifier: ratatui::style::Modifier) -> bool {
    line.spans
        .iter()
        .any(|s| s.content.contains(needle) && s.style.add_modifier.contains(modifier))
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

// ── New function tests ────────────────────────────────────────────────────
