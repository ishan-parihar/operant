// messages/tests.rs — Unit tests for the messages module.
//
// Extracted from messages/mod.rs.

use super::*;
use ratatui::text::Line;

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|s| s.content.to_string())
        .collect::<String>()
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

#[test]
fn test_render_tool_result_success_uses_30_lines() {
    let output = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let result = render_tool_result_success(&output, false);
    // 30 content lines + 1 overflow indicator = 31 (no separate header line)
    assert_eq!(result.len(), 31);
    let overflow_text = line_text(result.last().unwrap());
    assert!(overflow_text.contains("more lines"));
    assert!(!overflow_text.contains("ctrl+o"));
}

// ── New function tests ────────────────────────────────────────────────────

#[test]
fn test_render_system_api_error_short_message() {
    let result = render_system_api_error("Connection refused", None);
    assert!(!result.is_empty());
    let combined = result
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(combined.contains("API Error"));
    assert!(combined.contains("Connection refused"));
    // No retry line
    assert!(!combined.contains("Retrying"));
}

#[test]
fn test_render_system_api_error_with_retry() {
    let result = render_system_api_error("Timeout", Some(30));
    let combined = result
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(combined.contains("API Error"));
    assert!(combined.contains("Timeout"));
    assert!(combined.contains("Retrying in 30s"));
}

#[test]
fn test_render_system_api_error_long_message_shows_expand_hint() {
    let msg = (0..10)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let result = render_system_api_error(&msg, None);
    let combined = result
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("[expand]"),
        "should show [expand] hint when more than 5 lines"
    );
    assert!(combined.contains("5 more lines"));
}

#[test]
fn test_render_user_command() {
    let result = render_user_command("doctor", "--verbose");
    assert!(!result.is_empty());
    let text = line_text(&result[0]);
    assert!(text.contains('\u{25b8}'), "should have ▸ prefix");
    assert!(text.contains("doctor"));
    assert!(text.contains("--verbose"));
}

#[test]
fn goal_objective_renders_goal_active_block_not_user_command() {
    let result = render_user_command("goal", "Migrate to React");
    let header = line_text(&result[0]);
    let body = line_text(&result[1]);
    assert!(header.contains("GOAL ACTIVE"));
    assert!(
        !header.contains('\u{25b8}'),
        "should not show ▸ user-command prefix"
    );
    assert!(body.contains("Objective:"));
    assert!(body.contains("Migrate to React"));
}

#[test]
fn goal_subcommands_render_as_normal_user_command() {
    for sub in ["status", "pause", "resume", "clear", "complete"] {
        let result = render_user_command("goal", sub);
        let text = line_text(&result[0]);
        assert!(
            text.contains('\u{25b8}'),
            "/goal {sub} should keep ▸ prefix"
        );
        assert!(text.contains(sub));
    }
}

#[test]
fn goal_with_tokens_flag_strips_flag_from_objective() {
    let result = render_user_command("goal", "--tokens 250K Migrate to React");
    let body = line_text(&result[1]);
    assert!(body.contains("Migrate to React"));
    assert!(
        !body.contains("--tokens"),
        "flag should not appear in displayed objective"
    );
    assert!(!body.contains("250K"));
}

#[test]
fn extract_goal_objective_returns_none_for_subcommands_and_empty() {
    assert!(extract_goal_objective_from_args("").is_none());
    assert!(extract_goal_objective_from_args("   ").is_none());
    assert!(extract_goal_objective_from_args("status").is_none());
    assert!(extract_goal_objective_from_args("pause now").is_none()); // first token is subcommand
    assert_eq!(
        extract_goal_objective_from_args("Migrate to React").as_deref(),
        Some("Migrate to React"),
    );
}

#[test]
fn extract_goal_slash_objective_handles_typed_user_message() {
    assert_eq!(
        extract_goal_slash_objective("/goal build GPT 6 make no mistakes").as_deref(),
        Some("build GPT 6 make no mistakes"),
    );
    assert_eq!(
        extract_goal_slash_objective("/goal --tokens 250K Migrate to React").as_deref(),
        Some("Migrate to React"),
    );
    // Subcommands fall through.
    assert!(extract_goal_slash_objective("/goal status").is_none());
    assert!(extract_goal_slash_objective("/goal").is_none());
    // Not a /goal message.
    assert!(extract_goal_slash_objective("just a normal message").is_none());
    assert!(extract_goal_slash_objective("/goalbuild").is_none());
}

#[test]
fn extract_goal_slash_objective_folds_trailing_lines_into_objective() {
    let text = "/goal Migrate to React\nwith strict typing\nand tests passing";
    let extracted = extract_goal_slash_objective(text).unwrap();
    assert!(extracted.starts_with("Migrate to React"));
    assert!(extracted.contains("strict typing"));
    assert!(extracted.contains("tests passing"));
}

#[test]
fn test_render_user_memory_input() {
    let result = render_user_memory_input("project", "Operant");
    assert_eq!(result.len(), 2);
    let first = line_text(&result[0]);
    assert!(first.contains("# project: Operant"));
    let second = line_text(&result[1]);
    assert!(second.contains("Got it."));
}

#[test]
fn test_render_user_local_command_output_with_overflow() {
    let output = (0..20)
        .map(|i| format!("out {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let result = render_user_local_command_output("ls", &output, 5);
    // 1 header + 5 body + 1 overflow = 7
    assert_eq!(result.len(), 7);
    let header = line_text(&result[0]);
    assert!(header.contains("!ls"));
    let overflow = line_text(result.last().unwrap());
    assert!(overflow.contains("15 more lines"));
}

#[test]
fn test_render_user_local_command_output_no_overflow() {
    let output = "line1\nline2";
    let result = render_user_local_command_output("echo", output, 10);
    // 1 header + 2 body = 3
    assert_eq!(result.len(), 3);
    let header = line_text(&result[0]);
    assert!(header.contains("!echo"));
}

#[test]
fn test_render_collapsed_read_search_no_hidden() {
    let paths = vec!["src/lib.rs", "src/main.rs"];
    let result = render_collapsed_read_search("Read", &paths, 0);
    assert!(!result.is_empty());
    let text = line_text(&result[0]);
    assert!(text.contains('\u{25b8}'), "should have ▸ prefix");
    assert!(text.contains("Read"));
    assert!(text.contains("src/lib.rs"));
    assert!(
        !text.contains("more"),
        "should not show 'more' when n_hidden is 0"
    );
}

#[test]
fn test_render_collapsed_read_search_with_hidden() {
    let paths = vec!["a.rs", "b.rs"];
    let result = render_collapsed_read_search("Glob", &paths, 3);
    assert!(!result.is_empty());
    let text = line_text(&result[0]);
    assert!(text.contains("(+ 3 more)"));
}

#[test]
fn test_render_task_assignment() {
    let result = render_task_assignment(
        "42",
        "Implement feature X",
        "Add the new widget system\nWith multi-line support",
    );
    assert!(!result.is_empty());
    let combined = result
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(combined.contains("Implement feature X"));
    assert!(combined.contains("task #42"));
    assert!(combined.contains("Add the new widget system"));
}

#[test]
fn test_render_task_assignment_truncates_desc_at_5_lines() {
    let desc = (0..10)
        .map(|i| format!("desc line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let result = render_task_assignment("1", "Subject", &desc);
    let combined = result
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n");
    // Only first 5 desc lines should appear
    assert!(combined.contains("desc line 4"));
    assert!(
        !combined.contains("desc line 5"),
        "should truncate desc at 5 lines"
    );
}

#[test]
fn test_tui_render_bug_reproduce() {
    let text = "Hello!\n\n👋  I'm Operant, your\n\n AI assistant. How\n can I help you today?";
    let result = render_transcript_live_text(text, 24);
    assert_eq!(result.len(), 8);
    assert_eq!(line_text(&result[0]), "     Hello!");
    assert_eq!(line_text(&result[1]), "     ");
    assert_eq!(line_text(&result[2]), "     👋 I'm Operant,");
    assert_eq!(line_text(&result[3]), "     your");
    assert_eq!(line_text(&result[4]), "     ");
    assert_eq!(line_text(&result[5]), "     AI assistant.");
    assert_eq!(line_text(&result[6]), "     How can I help");
    assert_eq!(line_text(&result[7]), "     you today?");
}

#[test]
fn test_normalize_markdown_newlines_specific() {
    use super::markdown::normalize_markdown_newlines;
    let input = "Hello! 👋  Ho\nw\n can I help you today?";
    let output = normalize_markdown_newlines(input);
    assert_eq!(output, "Hello! 👋  How can I help you today?");
}

// (iter-213: 18 broken test functions deleted — they referenced
// render functions that were deleted in prior iterations:
// render_agent_notification, render_attachment_message,
// render_advisor_message, render_tool_result_cancelled/rejected,
// render_shutdown_message, render_resource_update,
// render_rate_limit_*, render_plan_*. The functions were
// removed but the tests were never updated. YAGNI: delete
// the tests rather than re-add unused render functions.)
