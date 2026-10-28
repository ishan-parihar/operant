//! session_hub state tests (plan iter-B/C gates).

use super::*;

fn row(id: &str, title: Option<&str>, emp: Option<&str>) -> SessionEntryV2 {
    SessionEntryV2 {
        id: id.to_string(),
        parent_id: None,
        title: title.map(str::to_string),
        updated_at: chrono::Utc::now(),
        employee_id: emp.map(str::to_string),
        source: "local".to_string(),
        message_count: 3,
        cost_usd: 0.01,
        tokens: 1200,
        cwd: None,
        status: RowStatus::Idle,
        kind: RowKind::Session,
    }
}

fn subagent_row(id: &str, goal: &str, status: &'static str) -> SessionEntryV2 {
    SessionEntryV2 {
        id: id.to_string(),
        parent_id: None,
        title: None,
        updated_at: chrono::Utc::now(),
        employee_id: None,
        source: "subagent".to_string(),
        message_count: 0,
        cost_usd: 0.0,
        tokens: 0,
        cwd: None,
        status: if status == "pending" {
            RowStatus::Live
        } else {
            RowStatus::Idle
        },
        kind: RowKind::Subagent {
            goal: goal.to_string(),
            model: "test-model".to_string(),
            dispatch_time: 1,
            status,
        },
    }
}

#[test]
fn by_employee_filter_keeps_only_tagged_rows() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("s1", Some("tagged"), Some("premiere")),
        row("s2", Some("personal"), None),
    ]);
    hub.filter_mode = HubFilterMode::ByEmployee;
    hub.rebuild_visible();
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["s1"]);
}

#[test]
fn and_token_search_matches_all_tokens_across_fields() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("s1", Some("Refactor auth module"), Some("premiere")),
        row("s2", Some("Refactor parser"), None),
    ]);
    hub.search_query = "refactor premiere".to_string();
    hub.rebuild_visible();
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    // "premiere" only exists on s1 (via the employee field), so the AND
    // narrows to it even though both titles match "refactor".
    assert_eq!(ids, vec!["s1"]);
}

#[test]
fn active_filter_keeps_pending_subagents() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("s1", Some("idle"), None),
        subagent_row("dlg-1", "audit the tests", "pending"),
        subagent_row("dlg-2", "done work", "completed"),
    ]);
    hub.filter_mode = HubFilterMode::Active;
    hub.rebuild_visible();
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["dlg-1"]);
}

#[test]
fn enter_on_a_session_switches_and_on_a_subagent_focuses() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s1", Some("one"), None)]);
    assert!(matches!(
        hub.confirm_selection(),
        Some(HubSelection::SwitchSession(id)) if id == "s1"
    ));

    hub.replace_rows(vec![subagent_row("dlg-9", "goal", "pending")]);
    assert!(matches!(
        hub.confirm_selection(),
        Some(HubSelection::FocusSubagent(id)) if id == "dlg-9"
    ));
    assert_eq!(hub.focused_subagent.as_deref(), Some("dlg-9"));
}

#[test]
fn replace_rows_preserves_selection_by_id() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s1", None, None), row("s2", None, None)]);
    hub.select_next(); // s2
    assert_eq!(hub.selected_row().map(|r| r.id.clone()), Some("s2".into()));
    hub.replace_rows(vec![row("s0", None, None), row("s2", None, None)]);
    assert_eq!(
        hub.selected_row().map(|r| r.id.clone()),
        Some("s2".into()),
        "a refresh must not yank the operator off their row"
    );
}

#[test]
fn omp_close_tap_handoff_one_further_left_closes() {
    // omp agent-hub.ts:626-636: the opener seeds the close detector so the
    // ←← that opened also arms its close — per the ratified plan,
    // armCloseTap means ONE further Left dismisses.
    let mut hub = SessionHubState::new();
    hub.open(Some(Instant::now()));
    assert!(
        hub.left_tap_close(),
        "ONE further Left after the armed ←← open closes"
    );
    assert!(!hub.visible);
}

#[test]
fn close_tap_window_expires_after_500ms() {
    // A stale arm (the opening ←← was >500ms ago) must not close: the next
    // Left starts a fresh tap pair instead (omp LEFT_TAP_WINDOW_MS), and the
    // Left after that closes.
    let mut hub = SessionHubState::new();
    hub.open(Some(Instant::now() - std::time::Duration::from_millis(600)));
    assert!(hub.visible);
    assert!(
        !hub.left_tap_close(),
        "a stale arm must not close — it re-arms (omp LEFT_TAP_WINDOW_MS)"
    );
    assert!(hub.visible, "the hub stays open");
    assert!(
        hub.left_tap_close(),
        "the fresh re-arm means the very next Left closes"
    );
    assert!(!hub.visible);
}
