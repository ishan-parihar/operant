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
