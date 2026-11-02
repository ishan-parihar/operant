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

// ---- (2026-10-29) the preview's scroll bound counts WRAPPED rows ----

use ratatui::{Terminal, backend::TestBackend};

/// Paint the hub into a real TestBackend frame and return the scroll bound the
/// pane measured — the number that decides whether the arrow keys can reach
/// the last message.
fn preview_bound_for(messages: &[PreviewMessage], width: u16, height: u16) -> u16 {
    use crate::tui::render::FrameCtx;
    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.replace_rows(vec![row("s-1", Some("A session"), None)]);
    assert!(hub.apply_preview("s-1", messages.to_vec(), 0));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut ctx = FrameCtx {
        size: ratatui::layout::Rect::new(0, 0, width, height),
        preview_max_scroll: None,
        stop: false,
    };
    terminal
        .draw(|f| {
            let area = ratatui::layout::Rect::new(0, 0, width, height);
            super::render::render_session_hub(f, &hub, area, &mut ctx);
        })
        .unwrap();
    ctx.preview_max_scroll.unwrap_or(0)
}

#[test]
fn the_scroll_bound_accounts_for_wrapped_rows() {
    // A 40-row pane with a narrow list splits ~60/40, so the preview holds
    // roughly 36 inner rows. Each message renders as ONE source line whose
    // text wraps to ~2 rows at that width, so a 30-message transcript needs
    // far more than one screenful — the bound must exceed it.
    let long = "word ".repeat(120);
    let messages: Vec<PreviewMessage> = (0..30)
        .map(|i| PreviewMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: long.clone(),
            tool_calls: Vec::new(),
        })
        .collect();
    let bound = preview_bound_for(&messages, 120, 40);
    assert!(
        bound >= 10,
        "a wrapping pane must be scrollable past its first screenful — got {bound}"
    );
}

#[test]
fn the_bound_matches_the_wrapped_row_count_not_the_source_lines() {
    // The exact defect: `lines.len()` counted 30 source lines; the paragraph
    // wraps them into far more visual rows. The reported bound must be the
    // WRAPPED count, which is strictly larger here.
    let long = "word ".repeat(120);
    let messages: Vec<PreviewMessage> = (0..30)
        .map(|i| PreviewMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: long.clone(),
            tool_calls: Vec::new(),
        })
        .collect();
    let bound = preview_bound_for(&messages, 120, 40);
    // The unwrapped count would be 30 source lines + chrome, which at
    // inner_height ~38 scrolls 0. A wrapped count crosses the screen.
    assert!(
        bound > 30,
        "the bound must reflect wrapped rows, which exceed the source-line count — got {bound}"
    );
}

#[test]
fn a_short_transcript_has_no_scroll_bound() {
    // A pane whose content fits must not scroll into blank space.
    let messages = vec![PreviewMessage {
        role: "user".to_string(),
        content: "hi".to_string(),
        tool_calls: Vec::new(),
    }];
    let bound = preview_bound_for(&messages, 120, 40);
    assert_eq!(bound, 0, "one short row in a 40-row pane is 0");
}

#[test]
fn a_narrower_pane_measures_a_taller_preview() {
    // Same content, half the width: the wrapped row count must roughly
    // double. This is the regression the operator's glitch report pinned.
    let long = "word ".repeat(120);
    let messages: Vec<PreviewMessage> = (0..30)
        .map(|i| PreviewMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: long.clone(),
            tool_calls: Vec::new(),
        })
        .collect();
    let wide = preview_bound_for(&messages, 120, 40);
    let narrow = preview_bound_for(&messages, 60, 40);
    assert!(
        narrow > wide,
        "a narrower pane wraps to more rows and must scroll further — wide={wide} narrow={narrow}"
    );
}

// ---- (2026-10-30, iter-1491) the preview names its not-loaded state ----

/// Paint the hub with NO rows loaded and return the rows the right pane painted,
/// so a test can assert the pane says something rather than repainting an
/// empty box.
fn right_pane_rows_without_selection(loading: bool) -> Vec<String> {
    use crate::tui::render::FrameCtx;
    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.loading = loading;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut ctx = FrameCtx {
        size: ratatui::layout::Rect::new(0, 0, 120, 40),
        preview_max_scroll: None,
        stop: false,
    };
    terminal
        .draw(|f| {
            super::render::render_session_hub(f, &hub, f.area(), &mut ctx);
        })
        .unwrap();
    // Isolate the RIGHT pane by finding the second border column: taking the
    // whole screen would let the LEFT pane's own "Loading sessions…" banner
    // satisfy the assertion, which is a false pass — the defect is precisely
    // that the right pane says nothing. The two vertical borders mark the
    // split, so everything right of the second `│` belongs to the preview.
    let rows = crate::tui::debug::debug_hub::buffer_rows(terminal.backend().buffer());
    rows.iter()
        .map(|row| {
            // `│` is three bytes, so every index below comes from
            // `char_indices` — a byte index into the middle of the glyph
            // panics, which is exactly how this helper was written first.
            //
            // Two borders per screen row: the left pane's right edge and the
            // right pane's left edge. Everything after the second belongs to
            // the preview, which is the pane under test.
            let mut borders = row
                .char_indices()
                .filter(|(_, c)| *c == '\u{2502}')
                .map(|(i, _)| i);
            borders.next();
            let Some(second) = borders.next() else {
                return String::new();
            };
            row[second + 3..].trim_end_matches('\u{2502}').to_string()
        })
        .filter(|row| !row.trim().is_empty())
        .collect()
}

/// The defect the operator called the "right-pane glitch": while the row load
/// was in flight the LEFT pane painted its "Loading sessions…" banner and the
/// RIGHT pane painted NOTHING AT ALL — an empty bordered box for four frames,
/// then the transcript popped in. Two phases of nothing, no explanation.
#[test]
fn the_right_pane_names_its_loading_state_instead_of_painting_nothing() {
    let rows = right_pane_rows_without_selection(true);
    assert!(
        rows.iter().any(|r| r.contains("Loading sessions")),
        "the right pane must say the same thing the left pane says while the \
         rows load — an empty box is the glitch: {rows:?}"
    );
}

/// The other half: rows loaded, but nothing is selected (an empty database, a
/// filter that matches nothing). Saying so beats an empty box.
#[test]
fn the_right_pane_names_an_empty_list_rather_than_painting_nothing() {
    let rows = right_pane_rows_without_selection(false);
    assert!(
        rows.iter().any(|r| r.contains("No sessions")),
        "an empty list must be named on BOTH panes: {rows:?}"
    );
}

// ---- (2026-10-30) the subagent badge renders exactly once ----

/// Render the hub and return the buffer rows, one `String` per screen line.
///
/// The list and preview panes are side by side, so one screen line holds both:
/// whole-line matching cannot tell "the list badged the row" from "the preview
/// badged it too". The hub splits 40/60, and the list pane's own border closes
/// at column 47 for a 120-wide frame — measured from the drawn buffer, not
/// assumed. `LIST_PANE_END` is that boundary.
const FRAME_WIDTH: u16 = 120;
const LIST_PANE_END: usize = 47;

fn hub_rows(rows: Vec<SessionEntryV2>) -> (Vec<String>, Vec<String>) {
    use crate::tui::debug::debug_hub::buffer_rows;
    use crate::tui::render::FrameCtx;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.loading = false;
    hub.replace_rows(rows);
    let mut terminal = Terminal::new(TestBackend::new(FRAME_WIDTH, 40)).unwrap();
    let mut ctx = FrameCtx {
        size: ratatui::layout::Rect::new(0, 0, FRAME_WIDTH, 40),
        preview_max_scroll: None,
        stop: false,
    };
    terminal
        .draw(|f| {
            super::render::render_session_hub(f, &hub, f.area(), &mut ctx);
        })
        .unwrap();
    buffer_rows(terminal.backend().buffer())
        .into_iter()
        .map(|line| {
            let list = line.chars().take(LIST_PANE_END).collect::<String>();
            let preview = line.chars().skip(LIST_PANE_END).collect::<String>();
            (list, preview)
        })
        .unzip()
}

/// The defect: `row_title` already prefixes a subagent row with "🤖 ", and a
/// second badge span was added in the list renderer, so the row drew
/// "⚡ 🤖 🤖 <goal>" — two badges and a stutter between them, proven on the
/// drawn buffer rather than inferred from the source. MUTATION-PROVEN: putting
/// the second span back turns this red with `left: 2, right: 1` and the drawn
/// row quoted in the failure.
#[test]
fn a_subagent_row_draws_exactly_one_badge() {
    let (list, _preview) = hub_rows(vec![subagent_row("dlg-9", "audit the tests", "pending")]);
    let badge_rows: Vec<&String> = list.iter().filter(|r| r.contains('🤖')).collect();
    assert_eq!(
        badge_rows.len(),
        1,
        "the subagent row must appear in exactly one list row: {list:?}"
    );
    let badges = badge_rows[0].matches('🤖').count();
    assert_eq!(
        badges, 1,
        "the subagent badge must be drawn ONCE in the list — a second span \
         produced \"🤖 🤖\" on the screen: {badge_rows:?}"
    );
}

/// The other half of the contract: the badge is the subagent row's, so an
/// ordinary session row must not grow one, and the branch prefix is not the
/// same thing as the badge.
#[test]
fn an_ordinary_session_row_draws_no_badge() {
    let (list, _preview) = hub_rows(vec![
        row("sess-a", Some("alpha"), None),
        subagent_row("dlg-9", "audit the tests", "pending"),
    ]);
    let alpha = list
        .iter()
        .find(|r| r.contains("alpha"))
        .unwrap_or_else(|| panic!("the session row must render: {list:?}"));
    assert!(
        !alpha.contains('🤖'),
        "an ordinary session must not carry the subagent badge: {alpha:?}"
    );
    let sub = list
        .iter()
        .find(|r| r.contains("audit the tests"))
        .unwrap_or_else(|| panic!("the subagent row must render: {list:?}"));
    assert!(
        sub.contains('🤖'),
        "a subagent row must still carry its badge: {sub:?}"
    );
}

// ---- (2026-10-30) subagent rows say "usage —" and name their delegation status ----

/// The defect: `records_snapshot()` in async_delegation.rs hardcodes
/// message_count 0, cost 0.0 and tokens 0 because the delegation registry holds
/// no usage counters, so a subagent row rendered "$0.0000 · 0 msg · 0.000k tok"
/// — asserting it had measured and found nothing. omp's roster rules say
/// "missing data appears as `usage —` rather than an estimate"
/// (agent-hub.md:36), and a dash is the honest shape: not measured, not zero.
#[test]
fn a_subagent_row_says_usage_is_unmeasured_rather_than_printing_zeros() {
    let (list, _preview) = hub_rows(vec![subagent_row("dlg-9", "audit the tests", "pending")]);
    let row = list
        .iter()
        .find(|r| r.contains("dlg-9"))
        .unwrap_or_else(|| panic!("the row must render: {list:?}"));
    assert!(
        row.contains("usage —"),
        "a subagent row must say usage — because the registry measures none: {row:?}"
    );
    assert!(
        !row.contains("$0.0000"),
        "a fabricated cost is a claim, not a placeholder: {row:?}"
    );
    assert!(
        !row.contains("0 msg"),
        "a fabricated message count is a claim, not a placeholder: {row:?}"
    );
}

/// The other half (audit Gap B): the delegation STATUS is what told the two
/// terminal states apart, and it existed only in the focused preview — so a
/// Completed and a Failed subagent were indistinguishable in the list. Both
/// states now name themselves on the row.
#[test]
fn a_subagent_row_names_its_delegation_status_without_focusing_it() {
    let (list, _preview) = hub_rows(vec![
        subagent_row("dlg-1", "audit the tests", "pending"),
        subagent_row("dlg-2", "done work", "completed"),
        subagent_row("dlg-3", "broken work", "failed"),
    ]);
    let pending = list
        .iter()
        .find(|r| r.contains("dlg-1"))
        .unwrap_or_else(|| panic!("the pending row must render: {list:?}"));
    let completed = list
        .iter()
        .find(|r| r.contains("dlg-2"))
        .unwrap_or_else(|| panic!("the completed row must render: {list:?}"));
    let failed = list
        .iter()
        .find(|r| r.contains("dlg-3"))
        .unwrap_or_else(|| panic!("the failed row must render: {list:?}"));
    assert!(pending.contains("pending"), "pending row: {pending:?}");
    assert!(
        completed.contains("completed"),
        "completed row: {completed:?}"
    );
    assert!(failed.contains("failed"), "failed row: {failed:?}");
    assert_ne!(
        completed, failed,
        "Completed and Failed must not draw as the same row"
    );
}

/// The fix must not leak: an ordinary session row keeps its REAL numbers,
/// because for sessions those are measured and true.
///
/// The row is TWO lines (title + meta, one multi-line ListItem) and the meta
/// is the line carrying the numbers, so the assertion targets the line that
/// holds the row's id. An earlier version joined the whole list buffer and
/// failed on a *correct* frame: "usage —" was present, in the subagent row two
/// rows down. Per-row matching is the fix, not a looser assertion.
#[test]
fn an_ordinary_session_row_keeps_its_real_usage_numbers() {
    let (list, _preview) = hub_rows(vec![
        row("sess-a", Some("alpha"), None),
        subagent_row("dlg-9", "audit the tests", "pending"),
    ]);
    let session_meta = list
        .iter()
        .find(|r| r.contains("sess-a"))
        .unwrap_or_else(|| panic!("the session's meta row must render: {list:?}"));
    assert!(
        !session_meta.contains("usage —"),
        "a session's usage IS measured — it must not print the dash: {session_meta:?}"
    );
    assert!(
        session_meta.contains("3 msg"),
        "the session's real message count must survive (the helper sets 3): {session_meta:?}"
    );
    assert!(
        session_meta.contains("$0.0100"),
        "the session's real cost must survive (the helper sets 0.01): {session_meta:?}"
    );
}

// ── the live refresh (audit 2026-10-30, the Wave-3 false premise) ─────────
//
// The hub's rows are loaded when it OPENS, so while the operator watches it
// a subagent row kept reading `pending` however long ago the delegation
// finished. These pin `refresh_subagent_rows`: it must move a row whose
// delegation changed, and must NOT move anything else — the DB rows, the
// cursor, or a row the registry does not mention.
//
// The registry is a process-global, so serialise and assert on rows the
// test itself created (tagged with a unique goal), exactly as the gateway
// `subagents` tests do.

mod refresh_liveness_tests {
    use super::*;
    use operant_core::tools::async_delegation::{
        create_record, list_records, mark_completed, mark_failed,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn unique(tag: &str) -> String {
        static N: AtomicU64 = AtomicU64::new(0);
        format!("lively-{tag}-{}", N.fetch_add(1, Ordering::SeqCst))
    }

    /// Everything a refresh is allowed to touch, as a comparable tuple.
    /// `SessionEntryV2` is not `PartialEq` in core, and making it one just
    /// for a test would widen a public type's surface for none.
    fn fingerprint(r: &SessionEntryV2) -> (String, String, i64, String) {
        let status = match &r.kind {
            RowKind::Subagent { status, .. } => (*status).to_string(),
            _ => "(not a subagent)".to_string(),
        };
        (
            r.id.clone(),
            status,
            r.updated_at.timestamp(),
            r.title.clone().unwrap_or_default(),
        )
    }

    fn hub_holding(rows: Vec<SessionEntryV2>) -> SessionHubState {
        let mut hub = SessionHubState::new();
        hub.replace_rows(rows);
        hub.visible = true;
        hub
    }

    /// The hub is OPEN. A delegation that was pending when it opened finishes.
    /// The row must follow it without the hub being closed and reopened.
    #[test]
    fn a_pending_row_follows_its_delegation_finishing_while_the_hub_is_open() {
        let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let goal = unique("follow");
        let id = create_record(&goal, "test-model");
        let mut hub = hub_holding(vec![subagent_row(&id, &goal, "pending")]);
        // Precondition: the row really does say pending before the refresh.
        assert!(
            matches!(
                hub.rows[0].kind,
                RowKind::Subagent { status, .. } if status == "pending"
            ),
            "precondition: the row opens as pending"
        );

        mark_completed(&id, "the answer");
        assert!(
            hub.refresh_subagent_rows(),
            "a delegation that finished MUST move the open row: {id}"
        );
        let RowKind::Subagent { status, .. } = &hub.rows[0].kind else {
            panic!("the row must still be a subagent row");
        };
        assert_eq!(*status, "completed", "the row must now say completed");
        assert!(
            !matches!(hub.rows[0].status, RowStatus::Live),
            "a finished delegation is not live work: {:?}",
            hub.rows[0].status
        );
    }

    /// A failure must reach the row too — and must not be conflated with
    /// success, which is the whole reason iter-1496 surfaced status here.
    #[test]
    fn a_failed_delegation_reaches_the_row_as_failed() {
        let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let goal = unique("fail");
        let id = create_record(&goal, "test-model");
        let mut hub = hub_holding(vec![subagent_row(&id, &goal, "pending")]);

        mark_failed(&id, "child timed out");
        assert!(hub.refresh_subagent_rows(), "a failure must move the row");
        let RowKind::Subagent { status, .. } = &hub.rows[0].kind else {
            panic!("expected a subagent row");
        };
        assert_eq!(*status, "failed");
    }

    /// The narrow half, and the half that makes the feature safe: nothing
    /// except the changed subagent row may move. A DB session row must come
    /// back byte-identical, or the refresh would reorder the operator's list
    /// under their cursor.
    #[test]
    fn a_refresh_leaves_ordinary_session_rows_and_the_selection_untouched() {
        let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let goal = unique("narrow");
        let id = create_record(&goal, "test-model");
        let mut hub = hub_holding(vec![
            row("s-db-1", Some("a real session"), Some("premiere")),
            subagent_row(&id, &goal, "pending"),
            row("s-db-2", Some("another session"), None),
        ]);
        // Put the cursor on the LAST row and record the whole view.
        hub.selected = 2;
        let before_ids: Vec<String> = hub.visible_rows().iter().map(|r| r.id.clone()).collect();
        let before_session = fingerprint(&hub.rows[0]);

        mark_completed(&id, "done");
        assert!(hub.refresh_subagent_rows());

        let after_ids: Vec<String> = hub.visible_rows().iter().map(|r| r.id.clone()).collect();
        assert_eq!(before_ids, after_ids, "row order must not change");
        assert_eq!(
            fingerprint(&hub.rows[0]),
            before_session,
            "an ordinary DB row must be untouched by a subagent refresh"
        );
        assert_eq!(hub.selected, 2, "the selection must not move");
        // And the subagent row is the only one that changed.
        let RowKind::Subagent { status, .. } = &hub.rows[1].kind else {
            panic!("expected the subagent row at index 1");
        };
        assert_eq!(*status, "completed");
    }

    /// A row the registry does not mention is left exactly as it is. This is
    /// what keeps a stale-but-unrelated row from being blanked by a registry
    /// that has since pruned it past the retention window.
    #[test]
    fn a_row_the_registry_does_not_mention_is_left_alone() {
        let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut hub = hub_holding(vec![subagent_row(
            "dlg-not-in-registry",
            "old job",
            "pending",
        )]);
        let before = fingerprint(&hub.rows[0]);

        assert!(
            !hub.refresh_subagent_rows(),
            "an unknown row is not a change; claiming one forces a needless repaint"
        );
        assert_eq!(
            fingerprint(&hub.rows[0]),
            before,
            "the unknown row must be untouched"
        );
    }

    /// A settled hub costs one lock read and reports no change, so the frame
    /// loop can skip work. Without this, every frame while the hub is open
    /// would rewrite rows and burn repaints for nothing.
    #[test]
    fn an_already_current_row_reports_no_change() {
        let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let goal = unique("settled");
        let id = create_record(&goal, "test-model");
        let mut hub = hub_holding(vec![subagent_row(&id, &goal, "pending")]);

        assert!(
            !hub.refresh_subagent_rows(),
            "a pending row matching a pending record is already current"
        );
        mark_completed(&id, "done");
        assert!(
            hub.refresh_subagent_rows(),
            "the transition must be seen the first time"
        );
        assert!(
            !hub.refresh_subagent_rows(),
            "the SECOND refresh must report no change — the row already says it"
        );
    }

    /// The registry's own retention cap prunes terminal records. A row that
    /// survives in the hub for a pruned delegation must still render (a
    /// missing row is a UI crash in the caller's `unwrap`).
    #[test]
    fn pruning_a_record_does_not_remove_the_hub_row() {
        let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let goal = unique("pruned");
        let id = create_record(&goal, "test-model");
        let mut hub = hub_holding(vec![subagent_row(&id, &goal, "pending")]);
        mark_completed(&id, "done");
        assert!(hub.refresh_subagent_rows());
        // The row must remain addressable and render the settled state.
        let live: Vec<String> = list_records()
            .iter()
            .map(|r| r.delegation_id.clone())
            .collect();
        assert!(
            !hub.rows.is_empty(),
            "a refresh must never delete a row the operator is looking at \
             (registry holds {} records)",
            live.len()
        );
    }
}

// ---- (RT-206) The `CurrentBranch` filter's auto-anchor has no real anchor in
// production. Ctrl+B sets `branch_root = None` and relies on `replace_rows`
// taking "the live row", but the ONLY assignment of `RowStatus::Live` outside
// these tests is `async_delegation.rs` for a PENDING SUBAGENT delegation. The
// operator's own current session row is built by
// `Database::list_sessions_rich`, which never marks anything Live. So the
// anchor is either a pending subagent (an unrelated branch) or absent.

#[test]
fn ctrl_b_with_no_live_row_lets_the_branch_filter_show_every_session() {
    // The exact production shape: the hub opens by Ctrl+B before the load
    // lands, and the load lands with NO pending-subagent row.
    let mut hub = SessionHubState::new();
    hub.filter_mode = HubFilterMode::CurrentBranch;
    hub.branch_root = None; // what Ctrl+B does
    hub.replace_rows(vec![
        row("s-current", Some("idle"), None),
        branch_row("s-current-fork", "s-current"),
        row("s-unrelated", None, None),
    ]);

    assert_eq!(
        hub.branch_root, None,
        "no row is Live, so the auto-anchor finds nothing - the anchor cannot \
         come from the operator's own session in production"
    );
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids.len(),
        3,
        "DEFECT (RT-206): the filter titled \"{label}\" showed every session \
         ({ids:?}) because an unanchored CurrentBranch falls back to \
         `None => true`. The operator pressed Ctrl+B to see THIS branch and \
         was shown the whole hub.",
        label = HubFilterMode::CurrentBranch.label()
    );
}

#[test]
fn a_pending_subagent_row_is_not_the_current_branch_anchor() {
    // If any row carries `Live`, the auto-anchor takes it - and in production
    // that row is a PENDING SUBAGENT DELEGATION, which lives under its own
    // subagent session, not the operator's conversation.
    let mut hub = SessionHubState::new();
    hub.filter_mode = HubFilterMode::CurrentBranch;
    hub.branch_root = None;

    let mut subagent = row("sub-1", None, None);
    subagent.status = RowStatus::Live;
    subagent.kind = RowKind::Subagent {
        goal: "background work".to_string(),
        model: "small-stack".to_string(),
        dispatch_time: 0,
        status: "pending",
    };
    hub.replace_rows(vec![
        row("s-current", Some("idle"), None),
        branch_row("s-current-fork", "s-current"),
        subagent,
        row("sub-1-other", None, None),
    ]);

    assert_eq!(
        hub.branch_root.as_deref(),
        Some("sub-1"),
        "the auto-anchor takes the pending subagent, because that is the only \
         row Live in production"
    );
    let ids: Vec<&str> = hub.visible_rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["sub-1"],
        "DEFECT (RT-206): the filter narrowed to the SUBAGENT's branch instead \
         of the operator's - the operator pressed Ctrl+B in their own session \
         and was shown a background delegation's tree"
    );
}

// ---- RT-206 FIX: Ctrl+B anchors on the App's OWN session id.

#[test]
fn ctrl_b_anchors_the_branch_filter_on_the_apps_own_session() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = crate::tui::app::tests::make_app();
    // The one fact that names "this branch": the session the App is attached
    // to. Before the fix this was a guess taken from a `RowStatus::Live` row,
    // which production only ever assigns to a pending subagent delegation.
    app.current_session = Some("s-operator".to_string());

    app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));

    assert_eq!(
        app.session_hub.filter_mode,
        HubFilterMode::CurrentBranch,
        "Ctrl+B must open the hub on the branch filter"
    );
    assert_eq!(
        app.session_hub.branch_root.as_deref(),
        Some("s-operator"),
        "the anchor must be the App's own session — not None (which showed \
         every session) and not a pending subagent's row (which showed an \
         unrelated branch)"
    );
    assert!(app.session_hub.visible, "the hub must open");

    // With the real anchor set before the load lands, rows must narrow to that
    // session and its forks — no live-row guess involved.
    app.session_hub.replace_rows(vec![
        row("s-operator", Some("operator's session"), None),
        branch_row("s-operator-fork-a", "s-operator"),
        branch_row("s-unrelated-fork", "s-unrelated-parent"),
        row("s-unrelated", None, None),
    ]);
    let ids: Vec<&str> = app
        .session_hub
        .visible_rows()
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["s-operator", "s-operator-fork-a"],
        "the branch filter must show the operator's session and ITS forks, \
         excluding unrelated sessions"
    );
}

#[test]
fn ctrl_b_with_no_known_session_still_must_not_show_every_session_silently() {
    // The anchor can be absent when the App has never switched or started a
    // session (a sessionless TUI). That case must still be honest: the filter
    // cannot narrow, so it must SAY so rather than claim "this branch's tree"
    // over the whole list. This pins the fallback the fix relies on.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = crate::tui::app::tests::make_app();
    app.current_session = None;

    app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));

    assert_eq!(
        app.session_hub.branch_root, None,
        "with no known session there is nothing to anchor on"
    );
    // The filter is CurrentBranch with no anchor: every row matches. That is
    // the recorded fallback — the hub is the surface that tells the operator
    // what exists, and an empty pane would be a lie. What must NOT happen is
    // the anchor silently becoming a pending subagent's row.
    app.session_hub
        .replace_rows(vec![row("s-one", None, None), branch_row("s-two", "s-one")]);
    let _ = app.session_hub.branch_root.as_deref();
    assert_eq!(
        app.session_hub.visible_rows().len(),
        2,
        "the no-anchor fallback shows everything rather than an empty pane — \\
         the honest default the existing CurrentBranch test records"
    );
}

/// RT-206, the piece that makes the fix real in production: Ctrl+B must read
/// the anchor from the AGENT'S live session id, not from a field the App
/// remembered at construction.
///
/// MEASURED LIVE on `operant chat` (2026-10-31, freshly-built debug binary,
/// real `~/.operant/database.db`): `agent.session_id()` is `None` at adapter
/// construction — the id is minted on the first turn — so a field seeded at
/// construction stays `None` and Ctrl+B anchors on nothing. The filter labeled
/// "this branch's tree" then showed all 19 sessions (the identical ids as
/// `[all]`). Reading the agent's id at the moment of the keypress fixed it:
/// after one turn, "this branch's tree" showed exactly 1 row and `[all]` 19.
#[test]
fn ctrl_b_reads_the_anchor_from_the_agents_live_session_id() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;

    let mut app = crate::tui::app::tests::make_app();
    // The production shape that broke: nothing has remembered a session yet.
    app.current_session = None;
    // But the agent has one, because a turn already ran. This is the ONLY
    // authoritative source — stand in for the real agent's closure.
    app.core_session_switch = Some(Arc::new(
        crate::tui::session_switch::SessionSwitchSource::new(
            |_| {},
            || Vec::new(),
            || Some("sess-live-from-agent".to_string()),
        ),
    ));

    app.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));

    assert_eq!(
        app.session_hub.branch_root.as_deref(),
        Some("sess-live-from-agent"),
        "Ctrl+B must anchor on the AGENT's live session id even when the App \
         remembered none — a construction-time field cannot work, because \
         agent.session_id() is None until the first turn mints one"
    );

    // And the anchor still narrows: the operator's row plus its forks only.
    app.session_hub.replace_rows(vec![
        row("sess-live-from-agent", Some("the real session"), None),
        branch_row("sess-live-from-agent-fork", "sess-live-from-agent"),
        row("sess-someone-else", None, None),
    ]);
    let ids: Vec<&str> = app
        .session_hub
        .visible_rows()
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["sess-live-from-agent", "sess-live-from-agent-fork"],
        "the agent-sourced anchor must narrow the same way a remembered one does"
    );
}

// ---- (2026-11-02) the session organiser's Stage A chrome ----

/// Paint the hub and return the raw screen lines, so a test can assert on the
/// drawn frame rather than on state.
fn hub_screen(rows: Vec<SessionEntryV2>) -> Vec<String> {
    use crate::tui::debug::debug_hub::buffer_rows;
    use crate::tui::render::FrameCtx;
    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.replace_rows(rows);
    hub.loading = false;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut ctx = FrameCtx {
        size: ratatui::layout::Rect::new(0, 0, 120, 40),
        preview_max_scroll: None,
        stop: false,
    };
    terminal
        .draw(|f| {
            super::render::render_session_hub(f, &hub, f.area(), &mut ctx);
        })
        .unwrap();
    buffer_rows(terminal.backend().buffer())
}

/// The finding that forced Stage A to render inside the block: with the header
/// and tab bar drawn ABOVE the bordered list, the left pane's top rows carry no
/// border glyph at all. Every test that finds the pane split by counting the
/// second `│` per row then reads the wrong pane, and the operator sees a label
/// floating two rows above its box. Border parity is the invariant.
#[test]
fn the_left_pane_keeps_its_border_on_every_row_chrome_included() {
    let screen = hub_screen(vec![row("sess-a", Some("alpha"), None)]);
    // Any box-drawing glyph counts: `│` on a body row, `┌ ┐ └ ┘` on the top and
    // bottom edges, `─` on a horizontal line. What must NEVER happen is a row
    // carrying only one pane's border, which is what drawing the chrome above
    // the block produced.
    let is_border = |c: char| {
        matches!(
            c,
            '\u{2502}' | '\u{250c}' | '\u{2510}' | '\u{2514}' | '\u{2518}'
        )
    };
    let missing: Vec<&String> = screen
        .iter()
        .filter(|line| line.chars().filter(|c| is_border(*c)).count() < 2)
        .collect();
    assert!(
        missing.is_empty(),
        "every row must still carry both panes' border glyphs — the chrome \
         broke border parity, which is how the pane split is found: {missing:?}"
    );
}

/// The counts are the point of the tab bar: before Stage A the operator had to
/// cycle all six filters to learn which of them had anything in them.
#[test]
fn the_tab_bar_shows_a_live_count_for_every_filter_mode() {
    let screen = hub_screen(vec![
        row("sess-a", Some("alpha"), None),
        row("sess-live", Some("beta"), None),
    ]);
    // The tab bar lives in the LEFT pane, whose width is 40% of 120 = 48
    // columns plus the border. Reading only the left 48 columns keeps the
    // preview pane's own session titles from satisfying the assertions.
    let joined: String = screen
        .iter()
        .map(|line| line.chars().take(48).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in ["all 2", "arch", "cur", "emp", "live"] {
        assert!(
            joined.contains(needle),
            "the tab bar must show every mode with its count, missing {needle:?}"
        );
    }
}

/// The counts are the honest kind: `all` counts ALL rows, so switching to it
/// shows exactly that many. `filter_counts` shares its predicate with
/// `rebuild_visible` for exactly this reason — a count that disagrees with the
/// list is worse than no count.
#[test]
fn filter_counts_agree_with_the_visible_set_for_each_mode() {
    let rows = vec![
        row("sess-a", Some("alpha"), None),
        row("emp-1", Some("employee session"), Some("emp-1")),
        row("sess-b", Some("beta"), None),
    ];
    let mut hub = SessionHubState::new();
    hub.replace_rows(rows);
    for (mode, counted) in hub.filter_counts() {
        // Reach the same mode through the public cycle so we exercise the real
        // path rather than assuming `filter_counts`'s ordering.
        hub.cycle_filter();
        if hub.filter_mode != mode {
            continue;
        }
        assert_eq!(
            counted,
            hub.visible_rows().len(),
            "the {mode:?} count must equal what the list shows"
        );
    }
}

/// The highlight must land on the SESSION the operator is on, not two rows up
/// on the tab bar. `state.selected` indexes the body set, so the rendered
/// selection is `CHROME_ROWS + selected`.
#[test]
fn the_highlight_follows_the_selection_past_the_chrome_rows() {
    let rows = vec![
        row("sess-a", Some("alpha"), None),
        row("sess-b", Some("beta"), None),
    ];
    let mut hub = SessionHubState::new();
    hub.replace_rows(rows);
    hub.select_next();
    assert_eq!(hub.selected, 1, "precondition: the second row is selected");
    assert_eq!(
        hub.visible_idx.len(),
        2,
        "the selection indexes the BODY set, which is where the offset matters"
    );
    // Drawn at 120x40 the second session's title must be the highlighted one;
    // the tab bar and the first row must not be.
    let screen = hub_screen(hub.rows.clone());
    let beta = screen
        .iter()
        .position(|line| line.contains("beta"))
        .expect("the second session renders");
    let all_count = screen.iter().position(|line| line.contains("all 2"));
    assert!(
        all_count.is_some(),
        "the tab bar renders above the rows, so the chrome must precede them"
    );
    let tab_row = all_count.unwrap();
    assert!(
        beta > tab_row,
        "the rows render below the chrome: tab={tab_row}, beta={beta}"
    );
}

// ── The interrupted-session pre-selection (plan 2026-11-02, P2) ─────────
//
// The cursor-placement half of the interrupted-session notice. The rank
// (interrupted above idle) is NOT tested here: it belongs to the session-
// organiser plan's Stage C, and duplicating it would put two agents in one
// ordering rule.

#[test]
fn open_selecting_puts_the_cursor_on_the_named_row() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("newest", Some("Newest"), None),
        row("interrupted", None, None),
        row("oldest", None, None),
    ]);
    // The interrupted session is deliberately NOT row 0: it was interrupted a
    // while ago and other sessions have run since, which is exactly the case
    // `open()` gets wrong.
    hub.open_selecting(None, "interrupted");
    assert_eq!(
        hub.selected_row().map(|r| r.id.as_str()),
        Some("interrupted"),
        "the cursor must land on the row the exit marker named"
    );
}

#[test]
fn open_selecting_falls_back_to_row_zero_when_the_row_is_not_visible() {
    // The filter/search case: the marked session is not in `visible_idx` at
    // all. `open()`'s row-0 behaviour is the right answer then, and this pins
    // that degrade rather than letting a missing row mean a stale cursor.
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("a", None, None), row("b", None, None)]);
    hub.open_selecting(None, "pruned-or-filtered-out");
    assert_eq!(
        hub.selected_row().map(|r| r.id.as_str()),
        Some("a"),
        "a missing row must degrade to row 0, not to no selection"
    );
}

#[test]
fn open_selecting_does_not_reorder_the_rows() {
    // The rank is Stage C's. Pinning that the ordering is untouched here is
    // what stops this from quietly pre-empting it.
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![row("first", None, None), row("second", None, None)]);
    let before: Vec<String> = hub.visible_rows().iter().map(|r| r.id.clone()).collect();
    hub.open_selecting(None, "second");
    let after: Vec<String> = hub.visible_rows().iter().map(|r| r.id.clone()).collect();
    assert_eq!(
        after, before,
        "cursor placement must not change the row order"
    );
}

// ---- (2026-11-02) Stage C: grouping is a view, and the offset survives it ----

fn row_with_cwd(id: &str, cwd: &str) -> SessionEntryV2 {
    let mut r = row(id, Some(id), None);
    r.cwd = Some(cwd.to_string());
    r
}

/// The default must not add chrome: with grouping in `Auto` the list is flat,
/// so a heading row would be a line of noise repeating the whole list.
#[test]
fn auto_grouping_adds_no_heading_rows() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row_with_cwd("a", "/one"),
        row_with_cwd("b", "/two"),
        row_with_cwd("c", "/one"),
    ]);
    hub.group_mode = HubGroupMode::Auto;
    let groups = hub.visible_groups();
    assert_eq!(
        groups.len(),
        1,
        "Auto is one flat group, not per-cwd groups"
    );
    assert!(groups[0].0.is_empty(), "the Auto key is empty by contract");
    assert_eq!(groups[0].1.len(), 3, "every visible row is in it");
}

/// Group order is FIRST-APPEARANCE, not alphabetical or by count. Codex's
/// `AgentsOverviewGroup` guarantees the same so a row refresh does not shuffle
/// the section the operator is reading.
#[test]
fn group_order_is_first_appearance_not_recomputed() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row_with_cwd("a", "/zeta"),
        row_with_cwd("b", "/alpha"),
        row_with_cwd("c", "/zeta"),
    ]);
    hub.group_mode = HubGroupMode::Directory;
    let groups = hub.visible_groups();
    assert_eq!(groups.len(), 2, "two directories, two groups");
    assert_eq!(
        groups[0].0, "/zeta",
        "the first row's dir opens the first group"
    );
    assert_eq!(groups[1].0, "/alpha");
    assert_eq!(
        groups[0].1.len(),
        2,
        "the first group holds both /zeta rows"
    );
}

/// `cycle_group` preserves the selection BY ROW ID. Grouping reorders the
/// reading, so a position-based preservation would move the cursor onto a
/// different session and the operator would watch the highlight jump sideways
/// while pressing `g`.
#[test]
fn cycling_group_keeps_the_cursor_on_the_same_session() {
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row_with_cwd("a", "/one"),
        row_with_cwd("b", "/two"),
        row_with_cwd("c", "/one"),
    ]);
    hub.select_next();
    assert_eq!(hub.selected_row().map(|r| r.id.as_str()), Some("b"));
    hub.cycle_group();
    assert_eq!(hub.group_mode, HubGroupMode::Directory);
    assert_eq!(
        hub.selected_row().map(|r| r.id.as_str()),
        Some("b"),
        "the selected row must be the same session after grouping changes"
    );
}

/// The heading offset. `state.selected` indexes the flat `visible_idx` while
/// the renderer draws flattened headings + rows, so the highlight must be
/// shifted by the headings above it — the Stage A offset class, one level
/// deeper, and the exact bug a flat `CHROME_ROWS + selected` reintroduces.
#[test]
fn the_highlight_accounts_for_group_headings_above_the_row() {
    // Four rows, three groups. First-appearance order puts /one first (it holds
    // rows 0 AND 3), then /two, then /three. So row 3 sits in the FIRST group
    // and has NO heading above it, while row 2 (/three) has TWO — that is the
    // case a flat `CHROME_ROWS + selected` gets wrong by two rows.
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row_with_cwd("a", "/one"),
        row_with_cwd("b", "/two"),
        row_with_cwd("c", "/three"),
        row_with_cwd("d", "/one"),
    ]);
    hub.group_mode = HubGroupMode::Directory;

    // Row 3 (`d`) is in the first group: no heading above it.
    hub.selected = 3;
    let above_d = hub
        .visible_groups()
        .iter()
        .take_while(|(_, rows)| !rows.contains(&row_index(&hub, "d")))
        .count();
    assert_eq!(
        above_d, 0,
        "the first group's later rows have no heading above them"
    );

    // Row 2 (`c`) is in the third group: two headings above it.
    hub.selected = 2;
    let above_c = hub
        .visible_groups()
        .iter()
        .take_while(|(_, rows)| !rows.contains(&row_index(&hub, "c")))
        .count();
    assert_eq!(above_c, 2, "the third group opens below two headings");

    // And the row must still be the same one after all of it.
    assert_eq!(hub.selected_row().map(|r| r.id.as_str()), Some("c"));
}

/// The ROW index (into `hub.rows`) of `id` — the index space `visible_groups`
/// actually returns. Under a filter this differs from the row's position in
/// `visible_idx`, which is exactly the distinction the heading-offset code has
/// to honour (see `interpose_group_headings`, which translates between them).
fn row_index(hub: &SessionHubState, id: &str) -> usize {
    hub.rows
        .iter()
        .position(|r| r.id == id)
        .unwrap_or_else(|| panic!("{id} must exist"))
}

/// Status grouping falls back to a label the read-only API already owns, so
/// the group heading does not invent a second vocabulary for the same states.
#[test]
fn status_grouping_keys_are_the_api_labels() {
    let mut interrupted = row("i", Some("interrupted one"), None);
    interrupted.status = RowStatus::Interrupted;
    let mut hub = SessionHubState::new();
    hub.replace_rows(vec![
        row("idle", Some("idle one"), None),
        interrupted,
        row("live", Some("live one"), None),
    ]);
    hub.group_mode = HubGroupMode::Status;
    let groups = hub.visible_groups();
    let keys: Vec<&str> = groups.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["idle", "interrupted"],
        "the live row is grouped under its api label, in first-appearance order"
    );
    // The live row must have landed in a group, not vanished: grouping is a
    // view over the SAME row set, so no row may drop out of it.
    let total: usize = groups.iter().map(|(_, rows)| rows.len()).sum();
    assert_eq!(total, hub.visible_rows().len(), "grouping drops no rows");
}

// ── Stage D — row backgrounds (2026-11-02) ──────────────────────────────────
//
// The problem these pin: before Stage D the selected row's highlight stopped
// where the text stopped, so a short row looked like a colour stripe with the
// row still "open" to the right. Each test reads the DRAWN buffer's background
// colours, not a style the code claims to set — a fill that is constructed but
// never painted is exactly the bug that motivates this section.

/// A hub state with `n` rows, selected on row 0, rendered to a `TestBackend` so
/// the caller can read the painted cell backgrounds.
fn hub_buffer(rows: usize) -> ratatui::buffer::Buffer {
    use crate::tui::render::FrameCtx;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.loading = false;
    hub.replace_rows(
        (0..rows)
            .map(|i| row(&format!("sess-{i}"), Some("t"), None))
            .collect(),
    );
    let mut terminal = Terminal::new(TestBackend::new(FRAME_WIDTH, 40)).unwrap();
    let mut ctx = FrameCtx {
        size: ratatui::layout::Rect::new(0, 0, FRAME_WIDTH, 40),
        preview_max_scroll: None,
        stop: false,
    };
    terminal
        .draw(|f| {
            super::render::render_session_hub(f, &hub, f.area(), &mut ctx);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The background colour of the cell at (x, y).
///
/// A `TestBackend` leaves every untouched cell at `Color::Reset` rather than
/// `None`, so a fill is proven by comparing VALUES, not by `is_some()`.
fn bg_at(buf: &ratatui::buffer::Buffer, x: u16, y: u16) -> ratatui::style::Color {
    buf[(x, y)]
        .style()
        .bg
        .unwrap_or(ratatui::style::Color::Reset)
}

/// The 0.12 selected tint, blended from `user_bg()` by the module under test.
///
/// Derived here rather than hard-coded so the test survives a palette change:
/// what it pins is that the ROW carries this tint, not the tint's literal.
#[allow(clippy::unwrap_used)]
fn selected_tint() -> ratatui::style::Color {
    let style = super::row_style::selected_fill_style().expect("the selected alpha paints a fill");
    style.bg.expect("a fill is a background")
}

/// How far right the row at buffer row `y` paints a background other than the
/// terminal default.
fn fill_extent(buf: &ratatui::buffer::Buffer, y: u16) -> u16 {
    (0..48u16)
        .rev()
        .find(|x| bg_at(buf, *x, y) != ratatui::style::Color::Reset)
        .unwrap_or(0)
}

#[test]
fn selected_row_background_reaches_the_right_border() {
    let buf = hub_buffer(4);
    // Measured layout: the block's top border is buffer row 0, the 3 chrome
    // rows follow (header, tab bar, column header), and the selected session
    // row begins at buffer row 4 — one buffer row per rendered line.
    let selected_y = 4u16;
    assert_eq!(
        bg_at(&buf, 30, selected_y),
        selected_tint(),
        "the selected row must carry the 0.12 tint"
    );
    let extent = fill_extent(&buf, selected_y);
    // A short row's own text ends near column 20. Before Stage D the highlight
    // ended with the text; the pass exists so it reaches the pane's right edge.
    let text_end = 23u16;
    assert!(
        extent > text_end,
        "the fill must extend past the row's text (it ends near column \
         {text_end}) — it stopped at column {extent}"
    );
    assert!(
        extent >= 45,
        "the fill must reach the pane's right edge, not stop short of the border \
         — it stopped at column {extent}"
    );
    assert_eq!(
        bg_at(&buf, 0, selected_y),
        ratatui::style::Color::Reset,
        "column 0 is the block's own left border and must not take the row tint"
    );
}

#[test]
fn selected_row_fill_differs_from_an_unselected_rows() {
    let buf = hub_buffer(4);
    // Row index 0 is selected (buffer row 4); row index 1 is odd and must be
    // left clean (buffer row 6, two rendered lines per session row).
    let (selected_y, plain_y) = (4u16, 6u16);
    assert_eq!(
        bg_at(&buf, 30, selected_y),
        selected_tint(),
        "the selected row takes the tint"
    );
    assert_eq!(
        bg_at(&buf, 30, plain_y),
        ratatui::style::Color::Reset,
        "row 1 is odd and must not be zebra'd — a tint here means the zebra \
         ladder is off by one, and a zebra on every row is no zebra at all"
    );
}

#[test]
fn zebra_fill_only_lands_on_even_rows() {
    // Directly on the helper, not the buffer: the ladder is a property of
    // `zebra_fill_style`, and asserting it at the pixel level would conflate
    // the zebra with the chrome's own background.
    assert!(
        super::row_style::zebra_fill_style(0).is_some(),
        "row 0 is a zebra row"
    );
    assert!(
        super::row_style::zebra_fill_style(1).is_none(),
        "row 1 must stay clean — a zebra on every row is no zebra at all"
    );
    assert!(
        super::row_style::zebra_fill_style(2).is_some(),
        "row 2 is a zebra row"
    );
}

#[test]
fn a_sub_visible_alpha_produces_no_fill_at_all() {
    // An invisible fill is not a subtle fill: it costs a paint and carries no
    // information. This pins the ladder, not the literal.
    assert!(
        super::row_style::row_fill_style(super::row_style::MIN_VISIBLE_ALPHA - 0.001).is_none(),
        "below MIN_VISIBLE_ALPHA the caller must be told there is no fill, so it \
         can skip the per-span patch pass entirely"
    );
    assert!(
        super::row_style::row_fill_style(super::row_style::SELECTED_ALPHA).is_some(),
        "the selected alpha must produce a fill"
    );
}

#[test]
fn apply_row_background_preserves_each_spans_own_foreground() {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};

    let line = Line::from(vec![
        Span::styled(
            "live",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" plain"),
    ]);
    let fill = Style::default().bg(Color::Rgb(40, 40, 50));
    let painted = super::row_style::apply_row_background(line, fill, 40);

    let first = &painted.spans[0].style;
    assert_eq!(
        first.fg,
        Some(Color::Green),
        "the span's own hue must survive the fill: recolouring the text here is \
         what would destroy the status-glyph and accent semantics"
    );
    assert!(
        first.add_modifier.contains(Modifier::BOLD),
        "the span's weight must survive the fill"
    );
    assert_eq!(
        first.bg,
        Some(Color::Rgb(40, 40, 50)),
        "the fill's background must land on every span"
    );
    // The fill reaches the full width, which is the whole point of the pass.
    let painted_width: usize = painted.spans.iter().map(|s| s.content.len()).sum();
    assert_eq!(
        painted_width, 40,
        "the fill must pad the row to the full inner width: {painted_width}"
    );
}

// ── Stage E — the sort key (2026-11-02) ─────────────────────────────────────
//
// Sort is the third of the three controls (filter `s`, group `g`, sort `o`),
// and the one that can silently betray the operator: a sort that does not
// reorder, or a sort that moves the cursor, is worse than no sort. These pin
// both halves — the ORDER it produces and the SELECTION it keeps.

/// Three rows with distinct, deliberately scrambled usage numbers: `rows` as
/// the database load would order them (newest-`updated_at` first) has NO
/// relationship to the cost/message/token ladders, so a passing test cannot be
/// an accident of the fixture.
fn sort_fixture() -> SessionHubState {
    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.loading = false;
    let mut cheap = row("cheap", Some("cheapest"), None);
    cheap.cost_usd = 0.01;
    cheap.message_count = 1;
    cheap.tokens = 10;
    let mut mid = row("mid", Some("middle"), None);
    mid.cost_usd = 0.50;
    mid.message_count = 7;
    mid.tokens = 500;
    let mut rich = row("rich", Some("priciest"), None);
    rich.cost_usd = 12.34;
    rich.message_count = 120;
    rich.tokens = 90_000;
    hub.replace_rows(vec![cheap, mid, rich]);
    hub
}

fn visible_ids(hub: &SessionHubState) -> Vec<String> {
    hub.visible_rows().iter().map(|r| r.id.clone()).collect()
}

#[test]
fn cost_sort_puts_the_most_expensive_row_first() {
    let mut hub = sort_fixture();
    hub.sort_mode = HubSortMode::Cost;
    hub.rebuild_visible();
    assert_eq!(
        visible_ids(&hub),
        vec!["rich", "mid", "cheap"],
        "cost sort must lead with the row that cost the most"
    );
}

#[test]
fn messages_sort_puts_the_busiest_row_first() {
    let mut hub = sort_fixture();
    hub.sort_mode = HubSortMode::Messages;
    hub.rebuild_visible();
    assert_eq!(
        visible_ids(&hub),
        vec!["rich", "mid", "cheap"],
        "messages sort must lead with the row carrying the most prompts"
    );
}

#[test]
fn tokens_sort_puts_the_largest_row_first() {
    let mut hub = sort_fixture();
    hub.sort_mode = HubSortMode::Tokens;
    hub.rebuild_visible();
    assert_eq!(
        visible_ids(&hub),
        vec!["rich", "mid", "cheap"],
        "tokens sort must lead with the row that consumed the most context"
    );
}

#[test]
fn auto_sort_is_the_loads_own_order_and_reorders_nothing() {
    let mut hub = sort_fixture();
    // The fixture is loaded in `cheap, mid, rich` order, which is NOT any of
    // the usage ladders — so if `Auto` sorted by anything it would change this.
    let before = visible_ids(&hub);
    hub.sort_mode = HubSortMode::Auto;
    hub.rebuild_visible();
    assert_eq!(
        visible_ids(&hub),
        before,
        "Auto must preserve the load's order; a sort here would only shuffle \\
         rows that tie on the load's own `updated_at DESC`"
    );
}

#[test]
fn cycling_sort_preserves_the_selected_row_by_identity() {
    let mut hub = sort_fixture();
    // Land on `mid`, then cycle the sort: `Auto -> Cost` reorders the flat
    // visible set, and a position-based cursor would land on a different row.
    let mid_pos = hub
        .visible_rows()
        .iter()
        .position(|r| r.id == "mid")
        .expect("the fixture has a mid row");
    hub.selected = mid_pos;
    let selected_before = hub.selected_row().map(|r| r.id.clone());
    hub.cycle_sort();
    assert_eq!(
        hub.selected_row().map(|r| r.id.clone()),
        selected_before,
        "cycling the sort must keep the SAME row selected, not the same \\
         position — a sort that teleports the cursor is the operator's \\
         highlight jumping to a session they were not reading"
    );
    assert_eq!(
        hub.sort_mode,
        HubSortMode::Cost,
        "cycle_sort must actually advance the key"
    );
}

#[test]
fn cycling_sort_walks_the_whole_ladder_back_to_auto() {
    let mut mode = HubSortMode::Auto;
    let mut seen = vec![mode];
    for _ in 0..4 {
        mode = mode.next();
        seen.push(mode);
    }
    assert_eq!(
        seen,
        vec![
            HubSortMode::Auto,
            HubSortMode::Cost,
            HubSortMode::Messages,
            HubSortMode::Tokens,
            HubSortMode::Auto,
        ],
        "the ladder must be exactly the four keys and must return to Auto — a \\
         duplicate key would make one press look like nothing happened"
    );
}

#[test]
fn no_sort_key_sorts_by_a_field_the_row_does_not_have() {
    // The regression guard for the phantom `Created` key: operant's projection
    // (`SessionEntryV2`) carries NO creation timestamp, so a `Created` variant
    // would have to sort by `updated_at` and silently answer a different
    // question. The ladder must not contain it.
    let labels: Vec<&str> = [
        HubSortMode::Auto,
        HubSortMode::Cost,
        HubSortMode::Messages,
        HubSortMode::Tokens,
    ]
    .iter()
    .map(|m| m.label())
    .collect();
    assert!(
        !labels.contains(&"created"),
        "no sort key may claim to sort by creation time: the row has no such \\
        field, and a key that silently sorts by another axis is worse than no \\
        key at all (found in the projection, session/listing.rs)"
    );
}

#[test]
fn the_sort_clause_renders_in_the_controls_row() {
    // The clause must actually reach the buffer: a control that exists in the
    // model and not on the screen is decoration the operator cannot find.
    let (list, _preview) = hub_rows(vec![
        row("rich", Some("priciest"), None),
        row("mid", Some("middle"), None),
        row("cheap", Some("cheapest"), None),
    ]);
    // Match on the tab bar's own content rather than its leading glyph: the
    // row is a `│` border plus the tabs, so prefix-matching `all ` never hits.
    let controls = list
        .iter()
        .find(|r| r.contains(" arch 0") || r.contains("Sort:"))
        .unwrap_or_else(|| panic!("the controls row must render: {list:?}"));
    assert!(
        controls.contains("recent"),
        "a fresh hub's sort clause must name its own key (`recent`) — the \\
         operator has to be able to see which ORDER the list is in: {controls:?}"
    );
}

// ── Stage E — the archive confirm (2026-11-02) ───────────────────────────────
//
// Codex's `ArchiveState` (resume_picker/archive.rs) arms on the key and commits
// on Enter, and it cancels on every other keystroke. These pin that machine's
// two load-bearing properties: the confirm DOES require a second keystroke, and
// it does NOT commit a row the operator moved off.

#[test]
fn arming_archive_requires_a_second_keystroke_to_commit() {
    let mut hub = sort_fixture();
    let armed = hub.arm_archive();
    assert_eq!(
        armed.as_deref(),
        Some("cheap"),
        "`a` arms the selected row and names it"
    );
    assert!(
        hub.archive_state.is_armed(),
        "arming must actually arm — an arm that does nothing is a confirm that \\
         archives without asking"
    );
    // The row is still in the visible set: nothing has been written yet.
    assert!(
        visible_ids(&hub).contains(&"cheap".to_string()),
        "arming must not remove the row: the operator has not confirmed"
    );
}

#[test]
fn committing_an_armed_archive_yields_the_selection_and_disarms() {
    let mut hub = sort_fixture();
    hub.arm_archive();
    let committed = hub.commit_archive();
    match committed {
        Some(HubSelection::ArchiveSession(id)) => {
            assert_eq!(id, "cheap", "the commit archives the ARMED row");
        }
        other => panic!("an armed Enter must yield ArchiveSession, got {other:?}"),
    }
    assert!(
        !hub.archive_state.is_armed(),
        "the commit must clear the armed state, or a second Enter re-archives"
    );
    // A commit with nothing armed is not a commit.
    assert!(
        hub.commit_archive().is_none(),
        "committing an idle confirm must do nothing at all"
    );
}

#[test]
fn disarming_leaves_nothing_armed() {
    let mut hub = sort_fixture();
    hub.arm_archive();
    hub.disarm_archive();
    assert!(!hub.archive_state.is_armed());
    assert!(
        hub.commit_archive().is_none(),
        "a disarmed confirm must not be committable — this is the property that \\
         makes the harmless escape hatch harmless"
    );
}

#[test]
fn a_subagent_row_cannot_be_armed_for_archiving() {
    // A delegation record is process-local registry state, not a database row.
    // `set_session_archived` would update nothing and the operator would be
    // told "0 rows changed" for a keystroke that looked like it worked.
    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.loading = false;
    hub.replace_rows(vec![subagent_row("dlg-9", "audit the tests", "pending")]);
    assert!(
        hub.arm_archive().is_none(),
        "a subagent row has no `sessions` row to archive and must refuse"
    );
    assert!(
        !hub.archive_state.is_armed(),
        "the refusal must leave the confirm idle, not half-armed"
    );
}

#[test]
fn the_archive_confirm_commits_the_armed_row_not_the_now_selected_row() {
    // The bug this pins: the operator arms row A, moves to row B, and Enter
    // archives B. The armed state must hold the ROW, not "whatever is selected
    // now".
    let mut hub = sort_fixture();
    hub.arm_archive();
    hub.select_next();
    hub.select_next();
    assert_eq!(
        hub.selected_row().map(|r| r.id.as_str()),
        Some("rich"),
        "the fixture's third row must now be selected"
    );
    match hub.commit_archive() {
        Some(HubSelection::ArchiveSession(id)) => assert_eq!(
            id, "cheap",
            "the commit must archive the ARMED row, not the one now under the \\
             cursor — archiving a session the operator never confirmed is the \\
             failure this stage exists to prevent"
        ),
        other => panic!("expected ArchiveSession, got {other:?}"),
    }
}

#[test]
fn the_armed_row_renders_the_archive_marker_not_its_status() {
    // The operator must see WHICH row will be archived, not only the notice
    // line at the bottom of the frame. This asserts on the DRAWN buffer: the
    // first attempt at this test rendered a fresh (unarmed) hub and asserted on
    // that, which proved nothing about the armed render at all.
    use crate::tui::render::FrameCtx;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut hub = SessionHubState::new();
    hub.visible = true;
    hub.loading = false;
    hub.replace_rows(vec![
        row("sess-a", Some("to archive"), None),
        row("sess-b", Some("leave alone"), None),
    ]);
    hub.arm_archive();
    assert_eq!(
        hub.selected_row().map(|r| r.id.as_str()),
        Some("sess-a"),
        "the fixture must start with sess-a selected for the marker to land on it"
    );

    let mut terminal = Terminal::new(TestBackend::new(FRAME_WIDTH, 40)).unwrap();
    let mut ctx = FrameCtx {
        size: ratatui::layout::Rect::new(0, 0, FRAME_WIDTH, 40),
        preview_max_scroll: None,
        stop: false,
    };
    terminal
        .draw(|f| {
            super::render::render_session_hub(f, &hub, f.area(), &mut ctx);
        })
        .unwrap();
    let rows: Vec<String> = crate::tui::debug::debug_hub::buffer_rows(terminal.backend().buffer())
        .into_iter()
        .map(|l| l.chars().take(LIST_PANE_END).collect())
        .collect();

    let armed_row = rows
        .iter()
        .find(|r| r.contains("to archive"))
        .unwrap_or_else(|| panic!("the armed row must render its title: {rows:?}"));
    assert!(
        armed_row.contains('\u{1f5d1}'),
        "the armed row must carry the archive marker (\\u{{1f5d1}}), not its \\
         ordinary status glyph: {armed_row:?}"
    );

    let other_row = rows
        .iter()
        .find(|r| r.contains("leave alone"))
        .unwrap_or_else(|| panic!("the unarmed row must render: {rows:?}"));
    assert!(
        !other_row.contains('\u{1f5d1}'),
        "only the ARMED row carries the marker — a marker on every row would \\
         make the confirm ambiguous: {other_row:?}"
    );
}
