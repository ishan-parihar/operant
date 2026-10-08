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

/// Flatten a rendered markdown block to one string per line.
fn markdown_lines(text: &str, width: u16) -> Vec<String> {
    render_markdown(text, width).iter().map(line_text).collect()
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

#[test]
fn test_normalize_markdown_newlines_specific() {
    use super::markdown::normalize_markdown_newlines;
    let input = "Hello! 👋  Ho\nw\n can I help you today?";
    let output = normalize_markdown_newlines(input);
    assert_eq!(output, "Hello! 👋  How can I help you today?");
}

// ── Markdown renderer ─────────────────────────────────────────────────────

#[test]
fn markdown_should_render_nested_unordered_lists() {
    let lines = markdown_lines("- alpha\n- beta\n  - nested one\n  - nested two", 60);

    let top_level: Vec<&String> = lines.iter().filter(|l| l.contains("alpha")).collect();
    let nested: Vec<&String> = lines.iter().filter(|l| l.contains("nested one")).collect();
    assert_eq!(
        top_level.len(),
        1,
        "one line for the top-level item: {lines:?}"
    );
    assert_eq!(nested.len(), 1, "one line for the nested item: {lines:?}");

    // Real list markers, and no literal markdown dashes left behind.
    assert_eq!(top_level[0], "  \u{2022} alpha");
    assert_eq!(nested[0], "    \u{2022} nested one");
    let combined = lines.join("\n");
    assert!(
        !combined.contains("- alpha"),
        "the source dash must not leak"
    );
}

#[test]
fn markdown_should_render_ordered_list_with_aligned_markers() {
    let lines = markdown_lines("1. alpha\n2. beta\n3. gamma", 60);

    // Every marker occupies the same two columns, so the content column lines up.
    assert_eq!(lines, vec!["  1. alpha", "  2. beta", "  3. gamma"]);
}

#[test]
fn markdown_should_render_task_list_checkboxes() {
    let lines = render_markdown("- [ ] todo\n- [x] done", 60);
    let todo = lines
        .iter()
        .find(|l| line_text(l).contains("todo"))
        .expect("todo row");
    let done = lines
        .iter()
        .find(|l| line_text(l).contains("done"))
        .expect("done row");

    assert!(
        line_text(todo).contains('\u{2610}'),
        "unchecked box: {:?}",
        line_text(todo)
    );
    assert!(
        line_text(done).contains('\u{2611}'),
        "checked box: {:?}",
        line_text(done)
    );
    let combined = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
    assert!(!combined.contains("[ ]"), "the raw marker must not leak");
    assert!(!combined.contains("[x]"), "the raw marker must not leak");
}

#[test]
fn markdown_should_render_italic_and_strikethrough() {
    let rendered = render_markdown(
        "plain *italic* and ~~struck~~ and **bold** and ***both***",
        80,
    );

    assert!(
        rendered
            .iter()
            .any(|l| span_has_modifier(l, "italic", ratatui::style::Modifier::ITALIC)),
        "italic span missing"
    );
    assert!(
        rendered.iter().any(|l| span_has_modifier(
            l,
            "struck",
            ratatui::style::Modifier::CROSSED_OUT
        )),
        "strikethrough span missing"
    );
    assert!(
        rendered
            .iter()
            .any(|l| span_has_modifier(l, "bold", ratatui::style::Modifier::BOLD)),
        "bold span missing"
    );
    assert!(
        rendered.iter().any(|l| {
            span_has_modifier(l, "both", ratatui::style::Modifier::BOLD)
                && span_has_modifier(l, "both", ratatui::style::Modifier::ITALIC)
        }),
        "bold-italic span missing"
    );
    // The markers themselves must be consumed, not printed.
    let text = rendered
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains('*'), "asterisks leaked: {text}");
    assert!(!text.contains('~'), "tildes leaked: {text}");
}

#[test]
fn markdown_should_render_horizontal_rule() {
    let lines = markdown_lines("above\n\n---\n\nbelow", 40);

    let rule = lines
        .iter()
        .find(|l| l.contains('\u{2500}'))
        .unwrap_or_else(|| panic!("no horizontal rule in {lines:?}"));
    assert!(
        rule.trim_start().starts_with('\u{2500}'),
        "rule glyph: {rule:?}"
    );
    assert!(
        UnicodeWidthStr::width(rule.as_str()) <= 40,
        "rule exceeds wrap width: {rule:?}"
    );
    assert!(lines.iter().any(|l| l.contains("above")));
    assert!(lines.iter().any(|l| l.contains("below")));
}

#[test]
fn markdown_should_still_render_tables() {
    let markdown = "| Name | Qty |\n| --- | ---: |\n| alpha | 1 |\n| beta | 22 |";
    let lines = render_markdown(markdown, 60);
    let combined = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

    // Every cell survives.
    for needle in ["Name", "Qty", "alpha", "beta", "1", "22"] {
        assert!(
            combined.contains(needle),
            "missing {needle:?} in {combined}"
        );
    }
    // The box-drawing frame is still produced.
    for glyph in ['┌', '┐', '├', '┤', '└', '┘', '│'] {
        assert!(combined.contains(glyph), "missing {glyph} in {combined}");
    }
    // A wide table is squeezed to the wrap width rather than overflowing.
    let wide = render_markdown(
        "| a-very-long-header-name | another-long-header |\n| --- | --- |\n| value-one | value-two |",
        40,
    );
    for line in &wide {
        assert!(
            UnicodeWidthStr::width(line_text(line).as_str()) <= 40,
            "table row exceeds wrap width: {:?}",
            line_text(line)
        );
    }
}

// (iter-213: 18 broken test functions deleted — they referenced
// render functions that were deleted in prior iterations:
// render_agent_notification, render_attachment_message,
// render_advisor_message, render_tool_result_cancelled/rejected,
// render_shutdown_message, render_resource_update,
// render_rate_limit_*, render_plan_*. The functions were
// removed but the tests were never updated. YAGNI: delete
// the tests rather than re-add unused render functions.)
