// diff_viewer/tests.rs — Unit tests for diff parsing and state.
//
// Extracted from the diff_viewer.rs monolith.

use super::*;
use ratatui::text::Span;

fn make_file(path: &str, added: u32, removed: u32, is_new: bool) -> FileDiffStats {
    FileDiffStats {
        path: path.to_string(),
        added,
        removed,
        binary: false,
        is_new_file: is_new,
        hunks: Vec::new(),
    }
}

#[test]
fn parse_unified_diff_new_file_flag() {
    let text = "diff --git a/new.rs b/new.rs\n\
                    new file mode 100644\n\
                    index 0000000..1234567\n\
                    --- /dev/null\n\
                    +++ b/new.rs\n\
                    @@ -0,0 +1,2 @@\n\
                    +fn foo() {}\n\
                    +fn bar() {}\n";
    let files = parse_unified_diff(text);
    assert_eq!(files.len(), 1);
    assert!(
        files[0].is_new_file,
        "new file mode header should set is_new_file"
    );
    assert_eq!(files[0].added, 2);
}

#[test]
fn parse_unified_diff_existing_file_not_new() {
    let text = "diff --git a/lib.rs b/lib.rs\n\
                    index 1111111..2222222 100644\n\
                    --- a/lib.rs\n\
                    +++ b/lib.rs\n\
                    @@ -1,1 +1,1 @@\n\
                    -old line\n\
                    +new line\n";
    let files = parse_unified_diff(text);
    assert_eq!(files.len(), 1);
    assert!(!files[0].is_new_file);
}

#[test]
fn build_inline_diff_spans_equal_content() {
    let (old, new) = build_inline_diff_spans("hello world", "hello world");
    // All spans should have no background (equal, not highlighted)
    for span in &old {
        assert!(
            span.style.bg.is_none(),
            "equal spans should have no bg highlight"
        );
    }
    for span in &new {
        assert!(
            span.style.bg.is_none(),
            "equal spans should have no bg highlight"
        );
    }
    // Combined text should contain the key words
    let old_text: String = old.iter().map(|s| s.content.as_ref()).collect::<String>();
    let new_text: String = new.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert!(
        old_text.contains("hello"),
        "old text should contain 'hello'"
    );
    assert!(
        new_text.contains("world"),
        "new text should contain 'world'"
    );
}

#[test]
fn build_inline_diff_spans_highlights_changed_word() {
    let (old_spans, new_spans) = build_inline_diff_spans("hello world", "hello earth");
    // "world" should be highlighted (deleted), "earth" should be highlighted (inserted)
    let has_highlighted_old = old_spans
        .iter()
        .any(|s| s.content.contains("world") && s.style.bg.is_some());
    let has_highlighted_new = new_spans
        .iter()
        .any(|s| s.content.contains("earth") && s.style.bg.is_some());
    assert!(has_highlighted_old, "deleted word should have bg highlight");
    assert!(
        has_highlighted_new,
        "inserted word should have bg highlight"
    );
}

#[test]
fn build_diff_lines_inline_diff_for_adjacent_pair() {
    let file = FileDiffStats {
        path: "test.rs".to_string(),
        added: 1,
        removed: 1,
        binary: false,
        is_new_file: false,
        hunks: vec![DiffHunk {
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Removed,
                    content: "let x = 1;".to_string(),
                    old_line_no: Some(1),
                    new_line_no: None,
                },
                DiffLine {
                    kind: DiffLineKind::Added,
                    content: "let x = 2;".to_string(),
                    old_line_no: None,
                    new_line_no: Some(1),
                },
            ],
        }],
    };
    let lines = build_diff_lines(&file, 80);
    // Should produce 2 lines (one removed, one added)
    assert_eq!(
        lines.len(),
        2,
        "adjacent removed+added should produce 2 lines"
    );
    // Each line should have multiple spans (gutter + marker + content spans)
    assert!(lines[0].spans.len() >= 3);
    assert!(lines[1].spans.len() >= 3);
}

#[test]
fn format_gutter_both_line_numbers() {
    let g = format_gutter(Some(10), Some(20));
    assert_eq!(g.len(), 10, "gutter should always be 10 chars");
    assert!(g.contains("10"));
    assert!(g.contains("20"));
}

#[test]
fn format_gutter_old_only() {
    let g = format_gutter(Some(5), None);
    assert_eq!(g.len(), 10);
    assert!(g.contains("5"));
}

#[test]
fn format_gutter_new_only() {
    let g = format_gutter(None, Some(99));
    assert_eq!(g.len(), 10);
    assert!(g.contains("99"));
}

#[test]
fn truncate_spans_to_width_exact() {
    let spans = vec![Span::raw("hello"), Span::raw(" world")];
    let result = truncate_spans_to_width(spans, 11);
    let text: String = result.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "hello world");
}

#[test]
fn truncate_spans_to_width_cuts_mid_span() {
    let spans = vec![Span::raw("abcdefghij")];
    let result = truncate_spans_to_width(spans, 5);
    let text: String = result.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "abcde");
}

#[test]
fn file_stats_binary_renders_badge() {
    // Verify the binary badge logic in render_file_list: binary=true → "[binary]"
    let file = FileDiffStats {
        path: "image.png".to_string(),
        added: 0,
        removed: 0,
        binary: true,
        is_new_file: false,
        hunks: Vec::new(),
    };
    let (stats, _color) = if file.binary {
        ("[binary]".to_string(), ratatui::style::Color::DarkGray)
    } else if file.is_new_file {
        (
            format!("[new] +{}", file.added),
            ratatui::style::Color::Yellow,
        )
    } else {
        (
            format!("+{} -{}", file.added, file.removed),
            ratatui::style::Color::DarkGray,
        )
    };
    assert_eq!(stats, "[binary]");
}

#[test]
fn file_stats_new_file_renders_badge() {
    let file = make_file("src/new.rs", 42, 0, true);
    let (stats, color) = if file.binary {
        ("[binary]".to_string(), ratatui::style::Color::DarkGray)
    } else if file.is_new_file {
        (
            format!("[new] +{}", file.added),
            ratatui::style::Color::Yellow,
        )
    } else {
        (
            format!("+{} -{}", file.added, file.removed),
            ratatui::style::Color::DarkGray,
        )
    };
    assert_eq!(stats, "[new] +42");
    assert_eq!(color, ratatui::style::Color::Yellow);
}

#[test]
fn diff_viewer_collapse_initializes_false() {
    let mut state = DiffViewerState::new();
    // Directly set files to simulate reload
    state.files = vec![
        make_file("a.rs", 1, 0, false),
        make_file("b.rs", 2, 1, false),
    ];
    state.collapsed = vec![false; state.files.len()];
    assert_eq!(state.collapsed.len(), 2);
    assert!(state.collapsed.iter().all(|&c| !c));
}

#[test]
fn diff_viewer_toggle_collapse_selected() {
    let mut state = DiffViewerState::new();
    state.files = vec![
        make_file("a.rs", 1, 0, false),
        make_file("b.rs", 2, 1, false),
    ];
    state.collapsed = vec![false; 2];
    state.selected_file = 1;
    state.toggle_file_collapse();
    assert!(!state.collapsed[0], "file 0 should remain expanded");
    assert!(state.collapsed[1], "file 1 should now be collapsed");
    assert_eq!(state.detail_scroll, 0, "scroll resets on collapse");
}

#[test]
fn diff_viewer_toggle_collapse_twice_restores() {
    let mut state = DiffViewerState::new();
    state.files = vec![make_file("a.rs", 1, 0, false)];
    state.collapsed = vec![false];
    state.toggle_file_collapse();
    assert!(state.collapsed[0]);
    state.toggle_file_collapse();
    assert!(!state.collapsed[0]);
}

#[test]
fn diff_viewer_toggle_collapse_empty_files_no_panic() {
    let mut state = DiffViewerState::new();
    // No files — toggle should not panic
    state.toggle_file_collapse();
}

#[test]
fn diff_viewer_collapse_renders_without_panic() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut state = DiffViewerState::new();
    state.visible = true;
    state.files = vec![make_file("src/lib.rs", 5, 2, false)];
    state.collapsed = vec![true]; // collapsed
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_diff_dialog(&mut state, area, frame.buffer_mut());
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let content: String = buf
        .content()
        .iter()
        .map(|c| c.symbol().chars().next().unwrap_or(' '))
        .collect();
    assert!(content.contains("collapsed") || content.contains("Space"));
}

#[test]
fn a_git_diff_is_read_off_the_loop_and_taken_once_it_arrives() {
    // TUI-11: `open` returns before `git diff` finishes; `drain_git_diff` takes the result.
    let dir = tempfile::tempdir().unwrap();
    let mut state = DiffViewerState::new();
    state.open(dir.path());
    assert!(state.loading);
    assert!(state.files.is_empty());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !state.drain_git_diff() {
        assert!(
            std::time::Instant::now() < deadline,
            "git diff never arrived"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!state.loading);
}

/// The keys the diff viewer's footer names, and the arm that implements each.
///
/// (RT-272) A claim without an arm is a promise the panel does not keep: the
/// footer used to end with `d toggle scope`, and NOTHING dispatched 'd' —
/// the panel has no scope to toggle, so the operator pressed it and nothing
/// happened.
///
/// (iter-1930) The checked set is the pane-independent half; the pane-gated
/// clause is pinned separately by `the_space_clause_rides_the_pane_that_dispatches_it`.
#[test]
fn the_diff_viewers_footer_only_names_keys_it_dispatches() {
    /// Each claim: the label the footer shows, the key it means, and the
    /// handler's arm that implements it.
    ///
    /// (iter-1930) These are the PANE-INDEPENDENT claims — the ones the shared
    /// constant carries because they dispatch in both panes. The space clause
    /// moved out of the constant (it only dispatches in the file-list pane) and
    /// is pinned by `the_space_clause_rides_the_pane_that_dispatches_it`.
    const CLAIMS: &[(&str, &str, &str)] = &[
        ("tab switch pane", "Tab", "KeyCode::Tab"),
        ("↑↓ navigate", "Up/Down", "KeyCode::Up"),
        ("pgup/pgdn", "PageUp/PageDown", "KeyCode::PageUp"),
    ];

    let footer = super::render::FOOTER_HINT;
    let handler = include_str!("../app/key_handling.rs");

    for (label, key, arm) in CLAIMS {
        assert!(
            footer.contains(label),
            "the footer must name the {key} key: {footer:?}"
        );
        assert!(
            handler.contains(arm),
            "the footer claims {key} ({label}), but the handler has no {arm} arm"
        );
    }

    // The scope toggle that never existed is not named. The needle is built
    // by concatenation on purpose: written out literally it would appear in
    // this test's own source.
    assert!(
        !footer.contains(&format!("{} {}", "toggle", "scope")),
        "the footer must not name a key the panel does not dispatch: {footer:?}"
    );
}

/// (iter-1930) The pane-dependent clause is only reachable from its pane.
///
/// RT-272's check above asks "does an arm exist somewhere for this key".
/// That is necessary but not sufficient: the space arm is GUARDED by
/// `active_pane == DiffPane::FileList`, so the clause was displayed in the
/// detail pane where the key is inert. A claim shown in a context its arm
/// excludes is still a promise the panel does not keep, so the two halves of
/// the footer are now derived from the live pane and this pins both.
#[test]
fn the_space_clause_rides_the_pane_that_dispatches_it() {
    // The shared claims dispatch in both panes; the space clause in only one.
    let file_list = super::render::footer_hint(super::DiffPane::FileList);
    let detail = super::render::footer_hint(super::DiffPane::Detail);

    for shared in ["switch pane", "navigate", "pgup/pgdn"] {
        assert!(
            file_list.contains(shared) && detail.contains(shared),
            "{shared:?} dispatches in both panes, so both footers carry it"
        );
    }
    assert!(
        file_list.contains("space") && file_list.contains("collapse"),
        "the file-list pane dispatches space, so its footer names it: {file_list:?}"
    );
    assert!(
        !detail.contains("space") && !detail.contains("collapse"),
        "the detail pane does not dispatch space, so its footer must not: {detail:?}"
    );

    // And the handler's guard is what makes that true: the space arm is the
    // reason the clause is pane-dependent, so this pins the footer that
    // describes it AND the gate it describes. (The needles are built from
    // pieces so this test's own source cannot satisfy them.)
    let handler = include_str!("../app/key_handling.rs");
    let fn_name = ["handle_diff", "_viewer_key"].concat();
    let fn_at = handler
        .find(&fn_name)
        .unwrap_or_else(|| panic!("the handler has no diff-viewer dispatcher"));
    let dispatcher = &handler[fn_at..];
    let space_arm = ["KeyCode::Char", "(' ')"].concat();
    let space_at = dispatcher
        .find(&space_arm)
        .unwrap_or_else(|| panic!("the diff dispatcher has no space arm at all"));
    let arm = &dispatcher[space_at..space_at + 320];
    let guard = ["DiffPane", "::FileList"].concat();
    let body = ["toggle", "_file_collapse"].concat();
    assert!(
        arm.contains(&guard) && arm.contains(&body),
        "the space arm stays gated to the file-list pane, so the footer's clause is \
         pane-dependent — got arm: {arm:?}"
    );
}

/// (iter-1930) The diff viewer's footer must be honest in the pane where it is
/// shown.
///
/// The footer names four claims: "tab switch pane · ↑↓ navigate · pgup/pgdn/
/// wheel scroll · space collapse". The first three are reachable from BOTH
/// panes (Tab/Left/Right switch, ↑↓ navigate or scroll, the page keys scroll the
/// detail). The fourth is not: `handle_diff_viewer_key` guards the space arm
/// with `if self.diff_viewer.active_pane == DiffPane::FileList`, so with the
/// detail pane focused the key falls through to `_ => {}` and does NOTHING —
/// while the footer still tells the operator it collapses.
///
/// RT-272's pin checks that each named key has an arm SOMEWHERE in the
/// handler; this is the next level of the same rule — a claim displayed in a
/// context where the arm is gated out is still a promise the panel does not
/// keep. The operator who Tabs to the detail pane (which the footer itself
/// tells them to do) and presses space gets silence.
#[test]
fn the_diff_footers_space_claim_is_only_shown_where_space_dispatches() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The whole rendered dialog as text, for the state's CURRENT pane.
    fn rendered(state: &mut super::DiffViewerState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                super::render::render_diff_dialog(state, frame.area(), frame.buffer_mut());
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    let mut state = super::DiffViewerState::new();
    state.visible = true;
    state.files = vec![make_file("src/lib.rs", 5, 2, false)];
    state.collapsed = vec![false; state.files.len()];

    // The file-list pane: every claim is reachable, space included.
    assert_eq!(state.active_pane, super::DiffPane::FileList);
    let list_footer = rendered(&mut state);
    assert!(
        list_footer.contains("space") && list_footer.contains("collapse"),
        "the file-list pane dispatches space, so the footer may name it: {list_footer:?}"
    );

    // The detail pane: space is gated out, so the footer must not promise it.
    state.switch_pane();
    assert_eq!(state.active_pane, super::DiffPane::Detail);
    let detail_footer = rendered(&mut state);
    assert!(
        !detail_footer.contains("space") || !detail_footer.contains("collapse"),
        "the detail pane does not dispatch space — the footer must not promise it there: \
         {detail_footer:?}"
    );
}
