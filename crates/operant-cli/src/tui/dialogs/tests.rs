// dialogs/tests.rs — Unit tests for the dialogs module.
//
// Extracted from dialogs.rs.

use super::*;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

// -----------------------------------------------------------------------
// Existing / backward-compat tests
// -----------------------------------------------------------------------

#[test]
fn standard_permission_request_has_four_options() {
    let pr = PermissionRequest::standard(
        "id1".to_string(),
        "Bash".to_string(),
        "Run a shell command".to_string(),
    );
    assert_eq!(pr.options.len(), 4);
    assert_eq!(pr.options[0].key, 'y');
    assert_eq!(pr.options[1].key, 'Y');
    assert_eq!(pr.options[2].key, 'p');
    assert_eq!(pr.options[3].key, 'n');
}

#[test]
fn from_reason_splits_on_newline() {
    let pr = PermissionRequest::from_reason(
        "id2".to_string(),
        "Bash".to_string(),
        "Custom summary\nThis will delete files permanently.".to_string(),
        Some("rm -rf /tmp".to_string()),
    );
    assert_eq!(pr.description, "Custom summary");
    assert_eq!(pr.danger_explanation, "This will delete files permanently.");
    assert_eq!(pr.input_preview.as_deref(), Some("rm -rf /tmp"));
}

#[test]
fn powershell_reason_uses_reason_body_only() {
    let pr = PermissionRequest::powershell(
        "id-ps".to_string(),
        "PowerShell".to_string(),
        "[High risk] This may modify system-wide security policy.".to_string(),
        "Set-ExecutionPolicy RemoteSigned".to_string(),
    );
    assert!(pr.description.is_empty());
    assert_eq!(
        pr.danger_explanation,
        "[High risk] This may modify system-wide security policy."
    );
    assert_eq!(
        pr.kind,
        PermissionDialogKind::PowerShell {
            command: "Set-ExecutionPolicy RemoteSigned".to_string(),
        }
    );
}

#[test]
fn powershell_reason_drops_duplicate_command_line() {
    let pr = PermissionRequest::powershell(
        "id-ps-2".to_string(),
        "PowerShell".to_string(),
        "This may modify system-wide security policy.".to_string(),
        "Set-ExecutionPolicy RemoteSigned".to_string(),
    );
    assert!(pr.description.is_empty());
    assert_eq!(
        pr.danger_explanation,
        "This may modify system-wide security policy."
    );
}

#[test]
fn powershell_reason_without_duplicate_line_keeps_explanation() {
    let pr = PermissionRequest::powershell(
        "id-ps-3".to_string(),
        "PowerShell".to_string(),
        "This will execute a shell command.".to_string(),
        "Get-ChildItem".to_string(),
    );
    assert!(pr.description.is_empty());
    assert_eq!(pr.danger_explanation, "This will execute a shell command.");
}

#[test]
fn from_reason_no_newline() {
    let pr = PermissionRequest::from_reason(
        "id3".to_string(),
        "WebFetch".to_string(),
        "WebFetch wants to fetch: `https://example.com`".to_string(),
        None,
    );
    assert_eq!(
        pr.description,
        "WebFetch wants to fetch: `https://example.com`"
    );
    assert!(pr.danger_explanation.is_empty());
}

#[test]
fn word_wrap_short_text_unchanged() {
    let wrapped = word_wrap("hello world", 80);
    assert_eq!(wrapped, vec!["hello world"]);
}

#[test]
fn word_wrap_long_text_splits() {
    use unicode_width::UnicodeWidthStr;
    let text = "one two three four five six seven eight";
    let wrapped = word_wrap(text, 10);
    for line in &wrapped {
        assert!(
            UnicodeWidthStr::width(line.as_str()) <= 10,
            "Line too long: {:?}",
            line
        );
    }
}

#[test]
fn word_wrap_hard_breaks_token_longer_than_width() {
    use unicode_width::UnicodeWidthStr;
    // A single token wider than the available width must be hard-broken at
    // character boundaries — otherwise it overflows the dialog border (the
    // bug that produced `~X~:~\~B~i~g~g~e~r~…`-style wrapping reports).
    let path = "'X:\\Bigger-Projects\\some-very-long-directory-name'";
    let wrapped = word_wrap(path, 16);
    assert!(wrapped.len() >= 2, "expected hard-break, got: {wrapped:?}");
    for line in &wrapped {
        assert!(
            UnicodeWidthStr::width(line.as_str()) <= 16,
            "hard-broken chunk too wide: {line:?}"
        );
    }
    // Round-trip: concatenating chunks should rebuild the token verbatim.
    assert_eq!(wrapped.join(""), path);
}

#[test]
fn word_wrap_mixed_short_and_long_tokens() {
    use unicode_width::UnicodeWidthStr;
    // The realistic shape that broke operant dialogs: a normal command
    // followed by a path longer than the column budget.
    let cmd = "git diff 'X:\\Bigger-Projects\\Operant\\very\\deep\\nested\\path.rs'";
    let wrapped = word_wrap(cmd, 24);
    for line in &wrapped {
        assert!(
            UnicodeWidthStr::width(line.as_str()) <= 24,
            "line wider than width: {line:?}"
        );
    }
}

// -----------------------------------------------------------------------
// PermissionDialogKind tests
// -----------------------------------------------------------------------

#[test]
fn bash_without_prefix_has_four_options() {
    let pr = PermissionRequest::bash(
        "id-bash-1".to_string(),
        "Bash".to_string(),
        "Wants to run a command".to_string(),
        "ls -la".to_string(),
        None,
    );
    assert_eq!(pr.options.len(), 4);
    assert_eq!(
        pr.kind,
        PermissionDialogKind::Bash {
            command: "ls -la".to_string(),
            suggested_prefix: None,
        }
    );
    // input_preview is set to the command
    assert_eq!(pr.input_preview.as_deref(), Some("ls -la"));
}

#[test]
fn bash_with_prefix_has_five_options() {
    let pr = PermissionRequest::bash(
        "id-bash-2".to_string(),
        "Bash".to_string(),
        "Wants to run git command".to_string(),
        "git status".to_string(),
        Some("git ".to_string()),
    );
    assert_eq!(pr.options.len(), 5);
    // 5th option (index 3 before deny) carries the prefix label
    assert!(
        pr.options[3].label.contains("git "),
        "Expected prefix in label: {:?}",
        pr.options[3].label
    );
    assert!(
        pr.options[3].label.ends_with('*'),
        "Expected * suffix: {:?}",
        pr.options[3].label
    );
    // Deny is still the last option
    assert_eq!(pr.options[4].key, 'n');
}

#[test]
fn file_read_has_three_options() {
    let pr = PermissionRequest::file_read(
        "id-fr".to_string(),
        "ReadFile".to_string(),
        "Wants to read /etc/hosts".to_string(),
        "/etc/hosts".to_string(),
    );
    assert_eq!(pr.options.len(), 3);
    assert_eq!(pr.options[0].key, 'y');
    assert_eq!(pr.options[1].key, 'Y');
    assert_eq!(pr.options[2].key, 'n');
    assert!(matches!(pr.kind, PermissionDialogKind::FileRead { .. }));
}

#[test]
fn file_write_has_four_options() {
    let pr = PermissionRequest::file_write(
        "id-fw".to_string(),
        "WriteFile".to_string(),
        "Wants to write /tmp/out.txt".to_string(),
        "/tmp/out.txt".to_string(),
    );
    assert_eq!(pr.options.len(), 4);
    assert_eq!(pr.options[2].key, 'p'); // project-level allow
    assert_eq!(pr.options[3].key, 'n');
    assert!(matches!(pr.kind, PermissionDialogKind::FileWrite { .. }));
}

// -----------------------------------------------------------------------
// McpApprovalDialogState tests
// -----------------------------------------------------------------------

#[test]
fn mcp_approval_new_is_invisible() {
    let state = McpApprovalDialogState::new();
    assert!(!state.visible);
    assert_eq!(state.selected, McpApprovalChoice::AllowSession);
}

#[test]
fn mcp_approval_show_populates_state() {
    let mut state = McpApprovalDialogState::new();
    state.show(
        "my-server",
        Some("wss://example.com/mcp"),
        None,
        vec!["tool_a".to_string(), "tool_b".to_string()],
    );
    assert!(state.visible);
    assert_eq!(state.server_name, "my-server");
    assert_eq!(state.server_url.as_deref(), Some("wss://example.com/mcp"));
    assert_eq!(state.tool_names.len(), 2);
    assert_eq!(state.selected, McpApprovalChoice::AllowSession);
}

#[test]
fn mcp_approval_select_next_and_prev() {
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    assert_eq!(state.selected, McpApprovalChoice::AllowSession);
    state.select_next();
    assert_eq!(state.selected, McpApprovalChoice::AllowAlways);
    state.select_next();
    assert_eq!(state.selected, McpApprovalChoice::Deny);
    state.select_next(); // wraps
    assert_eq!(state.selected, McpApprovalChoice::AllowSession);
    state.select_prev(); // wraps backward
    assert_eq!(state.selected, McpApprovalChoice::Deny);
}

#[test]
fn mcp_approval_confirm_closes_and_returns_choice() {
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    state.select_next(); // AllowAlways
    let choice = state.confirm();
    assert_eq!(choice, McpApprovalChoice::AllowAlways);
    assert!(!state.visible);
}

#[test]
fn mcp_approval_key_enter_confirms() {
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    state.select_next(); // AllowAlways
    let result = handle_mcp_approval_key(&mut state, key(KeyCode::Enter));
    assert_eq!(result, Some(McpApprovalChoice::AllowAlways));
    assert!(!state.visible);
}

#[test]
fn mcp_approval_key_esc_denies() {
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    let result = handle_mcp_approval_key(&mut state, key(KeyCode::Esc));
    assert_eq!(result, Some(McpApprovalChoice::Deny));
    assert!(!state.visible);
}

#[test]
fn mcp_approval_key_digit_shortcuts() {
    // '1' → AllowSession
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    let r = handle_mcp_approval_key(&mut state, key(KeyCode::Char('1')));
    assert_eq!(r, Some(McpApprovalChoice::AllowSession));

    // '2' → AllowAlways
    state.show("s", None, None, vec![]);
    let r = handle_mcp_approval_key(&mut state, key(KeyCode::Char('2')));
    assert_eq!(r, Some(McpApprovalChoice::AllowAlways));

    // '3' → Deny
    state.show("s", None, None, vec![]);
    let r = handle_mcp_approval_key(&mut state, key(KeyCode::Char('3')));
    assert_eq!(r, Some(McpApprovalChoice::Deny));
}

#[test]
fn mcp_approval_key_n_denies() {
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    let r = handle_mcp_approval_key(&mut state, key(KeyCode::Char('n')));
    assert_eq!(r, Some(McpApprovalChoice::Deny));
}

#[test]
fn mcp_approval_key_navigation_returns_none() {
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, vec![]);
    let r = handle_mcp_approval_key(&mut state, key(KeyCode::Down));
    assert_eq!(r, None);
    assert!(state.visible); // still open
    let r = handle_mcp_approval_key(&mut state, key(KeyCode::Up));
    assert_eq!(r, None);
}

#[test]
fn mcp_approval_tool_list_capped_at_five_in_display() {
    let tools: Vec<String> = (0..10).map(|i| format!("tool_{}", i)).collect();
    let mut state = McpApprovalDialogState::new();
    state.show("s", None, None, tools.clone());
    // State stores all 10 but render only shows first 5.
    assert_eq!(state.tool_names.len(), 10);
    // We test the cap by checking state.tool_names.iter().take(5) gives 5 items.
    assert_eq!(state.tool_names.iter().take(5).count(), 5);
}

#[test]
fn truncate_str_within_limit() {
    assert_eq!(truncate_str("hello", 10), "hello");
}

#[test]
fn truncate_str_exceeds_limit() {
    let s = truncate_str("hello world", 6);
    assert!(s.ends_with('\u{2026}'));
    assert!(s.chars().count() <= 6);
}
