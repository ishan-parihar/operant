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

// ---- (audit R1 2026-10-28) the CurrentBranch filter that replaced the
// session_branching overlay: it listed parent + forks of one session, the
// hub filter shows exactly that set over the same rows.

fn branch_row(id: &str, parent: &str) -> SessionEntryV2 {
    let mut r = row(id, None, None);
    r.kind = RowKind::Branch;
    r.parent_id = Some(parent.to_string());
    r
}

#[test]
fn current_branch_shows_the_root_and_its_forks_only() {
    let mut hub = SessionHubState::new();
    hub.filter_mode = HubFilterMode::CurrentBranch;
    hub.branch_root = Some("s-1".to_string());
    hub.replace_rows(vec![
        row("s-1", Some("live work"), None),
        branch_row("s-1-fork-a", "s-1"),
        branch_row("s-1-fork-b", "s-1"),
        row("s-2", Some("unrelated"), None),
        branch_row("s-2-fork", "s-2"),
    ]);
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["s-1", "s-1-fork-a", "s-1-fork-b"], "{ids:?}");
}

#[test]
fn current_branch_without_an_anchor_shows_everything() {
    // An unanchored filter must not pretend an empty answer is right — the
    // hub is the surface that tells the operator what exists.
    let mut hub = SessionHubState::new();
    hub.filter_mode = HubFilterMode::CurrentBranch;
    hub.branch_root = None;
    hub.replace_rows(vec![row("s-1", None, None), row("s-2", None, None)]);
    assert_eq!(hub.visible_rows().len(), 2);
}

#[test]
fn a_live_row_auto_anchors_the_branch_root_when_the_load_lands() {
    // Ctrl+B opens before the background load answers, so the anchor is
    // taken from the live row — the only session that could be streaming.
    let mut hub = SessionHubState::new();
    hub.filter_mode = HubFilterMode::CurrentBranch;
    let mut live = row("s-live", Some("streaming"), None);
    live.status = RowStatus::Live;
    hub.replace_rows(vec![
        live,
        branch_row("s-live-fork", "s-live"),
        row("s-other", None, None),
    ]);
    assert_eq!(hub.branch_root.as_deref(), Some("s-live"));
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["s-live", "s-live-fork"], "{ids:?}");
}

#[test]
fn the_filter_cycle_includes_the_branch_view_between_active_and_archived() {
    let mut hub = SessionHubState::new();
    hub.filter_mode = HubFilterMode::Active;
    hub.cycle_filter();
    assert_eq!(hub.filter_mode, HubFilterMode::CurrentBranch);
    assert_eq!(hub.filter_mode.label(), "this branch's tree");
    hub.cycle_filter();
    assert_eq!(hub.filter_mode, HubFilterMode::Archived);
}

// ---- (audit R4/R5 2026-10-28) the pane's nav and wheel match jcode's ----

#[test]
fn home_and_end_jump_to_the_list_edges() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("s-1", None, None),
        row("s-2", None, None),
        row("s-3", None, None),
    ]);
    hub.select_last();
    assert_eq!(hub.selected_row().map(|r| r.id.as_str()), Some("s-3"));
    hub.select_first();
    assert_eq!(hub.selected_row().map(|r| r.id.as_str()), Some("s-1"));
}

#[test]
fn edge_jumps_are_inert_on_an_empty_list() {
    let mut hub = SessionHubState::new();
    hub.select_first();
    hub.select_last();
    assert_eq!(hub.selected, 0, "no panic, no jump on an empty pane");
}

// ---- (2026-10-29 preview fix) the pane's three-state contract ----

fn msg(role: &str, content: &str) -> PreviewMessage {
    PreviewMessage {
        role: role.to_string(),
        content: content.to_string(),
        tool_calls: Vec::new(),
    }
}

#[test]
fn window_messages_keeps_the_tail_and_counts_hidden_prompts() {
    let mut all = Vec::new();
    for i in 0..PREVIEW_MESSAGE_LIMIT + 2 {
        all.push(msg("user", &format!("prompt {i}")));
    }
    all.insert(10, msg("assistant", "noise"));
    let (windowed, hidden) = SessionHubState::window_messages(all);
    assert_eq!(windowed.len(), PREVIEW_MESSAGE_LIMIT);
    assert!(
        hidden > 0,
        "the prompts dropped off the head are counted, so the pane can number globally"
    );
}

#[test]
fn window_messages_is_identity_under_the_limit() {
    let msgs = vec![msg("user", "one"), msg("assistant", "two")];
    let (windowed, hidden) = SessionHubState::window_messages(msgs.clone());
    assert_eq!(windowed.len(), 2);
    assert_eq!(hidden, 0);
}

#[test]
fn tab_routes_the_step_keys_between_the_two_panes() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("s-1", None, None),
        row("s-2", None, None),
        row("s-3", None, None),
    ]);
    assert_eq!(hub.focus, PaneFocus::Sessions);
    // With focus in the list, Down moves the selection.
    hub.focus_down();
    assert_eq!(hub.selected_row().map(|r| r.id.as_str()), Some("s-2"));
    hub.toggle_focus();
    assert_eq!(hub.focus, PaneFocus::Preview);
    // With focus in the preview, the SAME key scrolls the pane instead —
    // this is what the operator reported dead.
    hub.set_preview_max_scroll(40);
    hub.focus_down();
    assert_eq!(
        hub.preview_scroll, PREVIEW_SCROLL_STEP as u16,
        "the preview scrolled"
    );
    assert_eq!(
        hub.selected_row().map(|r| r.id.as_str()),
        Some("s-2"),
        "the list selection did NOT move while the preview had focus"
    );
}

#[test]
fn unpreviewed_rows_say_so_instead_of_claiming_to_load() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None)]);
    // No load in flight, nothing cached: this is a MISS, not a spinner.
    assert!(!hub.preview_loading);
    assert!(hub.preview.is_none());
    assert!(
        !hub.has_preview_content(),
        "an empty preview is not content"
    );
    // and the frame loop re-asks, so the miss does not persist silently.
    assert!(hub.preview_needs_load());
}

#[test]
fn a_cached_preview_satisfies_needs_load() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None)]);
    assert!(hub.apply_preview("s-1", vec![msg("user", "hi")], 0));
    assert!(!hub.preview_needs_load());
    assert!(hub.has_preview_content());
}

#[test]
fn the_load_key_forces_a_reload_of_the_selected_row() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None)]);
    assert!(hub.apply_preview("s-1", vec![msg("user", "hi")], 0));
    hub.set_preview_max_scroll(20);
    hub.preview_scroll = 12;
    hub.request_preview_reload();
    assert!(hub.preview.is_none(), "the cache is dropped for this row");
    assert_eq!(hub.preview_scroll, 0, "the stale scroll offset goes too");
    assert!(
        hub.preview_needs_load(),
        "the next frame re-asks for the load"
    );
}

#[test]
fn reload_clears_a_recorded_failure_for_the_selected_row() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None), row("s-2", None, None)]);
    hub.note_preview_failure("s-1");
    assert!(
        !hub.preview_needs_load(),
        "a failed row is not retried every frame"
    );
    hub.request_preview_reload();
    assert!(
        hub.preview_needs_load(),
        "r retries a row that previously failed"
    );
}

#[test]
fn preview_scroll_is_clamped_to_the_measured_bound() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None)]);
    hub.apply_preview("s-1", vec![msg("user", "hi")], 0);
    // The stale-offset order the frame loop actually runs in: an old offset
    // from a longer preview, then a fresh (smaller) measured bound.
    hub.preview_scroll = 99;
    hub.set_preview_max_scroll(6);
    assert_eq!(
        hub.preview_scroll, 6,
        "the measured bound clamps a stale offset down"
    );
}

#[test]
fn preview_steps_never_pass_the_measured_bound() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None)]);
    hub.set_preview_max_scroll(4);
    // jcode's step is 3 rows per j/Down, held down to grow the offset; the
    // clamp is the only thing keeping it inside the rendered rows.
    for _ in 0..20 {
        hub.scroll_preview(PREVIEW_SCROLL_STEP);
    }
    assert_eq!(hub.preview_scroll, 4);
}

#[test]
fn moving_the_selection_drops_the_previous_rows_preview() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None), row("s-2", None, None)]);
    assert!(hub.apply_preview("s-1", vec![msg("user", "hi")], 0));
    hub.focus_down();
    assert!(
        hub.preview.is_none(),
        "s-2's preview must not show s-1's transcript"
    );
    assert!(hub.preview_needs_load());
}

#[test]
fn a_load_that_lands_for_a_deselected_row_is_dropped() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("s-1", None, None), row("s-2", None, None)]);
    // s-1's load was in flight; the operator moved to s-2 before it landed.
    hub.focus_down();
    assert_ne!(hub.selected_row().map(|r| r.id.as_str()), Some("s-1"));
    assert!(!hub.apply_preview("s-1", vec![msg("user", "hi")], 0));
    assert!(hub.preview.is_none());
}

#[test]
fn a_row_with_no_storable_content_is_not_a_preview_row() {
    // A tool-only message whose JSON body and an empty content block: the
    // preview collapses it rather than painting a blank line.
    let blank = operant_core::database::MessageData {
        id: 1,
        session_id: "s".to_string(),
        role: "assistant".to_string(),
        content: Some(String::new()),
        tool_call_id: None,
        tool_calls: None,
        tool_name: None,
        timestamp: "now".to_string(),
        ..Default::default()
    };
    assert!(preview_message_from_row(blank).is_none());
}

#[test]
fn tool_call_json_is_summarized_not_printed() {
    let row = operant_core::database::MessageData {
        id: 2,
        session_id: "s".to_string(),
        role: "assistant".to_string(),
        content: Some(String::new()),
        tool_call_id: None,
        tool_calls: Some(
            r#"[{"name":"bash","arguments":{"command":"ls"}},{"name":"read_file"}]"#.to_string(),
        ),
        tool_name: None,
        timestamp: "now".to_string(),
        ..Default::default()
    };
    let pv = preview_message_from_row(row).expect("tool calls alone still render a row");
    assert_eq!(pv.tool_calls, vec!["bash", "read_file"]);
    assert!(pv.content.is_empty());
}

#[test]
fn a_text_message_with_tool_calls_keeps_both() {
    let row = operant_core::database::MessageData {
        id: 3,
        session_id: "s".to_string(),
        role: "assistant".to_string(),
        content: Some("Reading the tree".to_string()),
        tool_call_id: None,
        tool_calls: Some(r#"[{"name":"bash"}]"#.to_string()),
        tool_name: None,
        timestamp: "now".to_string(),
        ..Default::default()
    };
    let pv = preview_message_from_row(row).expect("renders");
    assert_eq!(pv.content, "Reading the tree");
    assert_eq!(pv.tool_calls, vec!["bash"]);
}

#[test]
fn a_no_selection_selection_is_not_an_illegal_panic() {
    // `request_preview_reload` and the frame-loop spawn both index
    // `selected_row()`; with no rows they must no-op.
    let mut hub = SessionHubState::new();
    hub.replace_rows(Vec::new());
    hub.request_preview_reload();
    assert!(hub.preview.is_none());
    assert!(!hub.preview_needs_load());
}
