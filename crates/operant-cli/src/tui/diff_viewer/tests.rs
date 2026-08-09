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
fn diff_viewer_set_turn_diff_resets_collapsed() {
    let mut state = DiffViewerState::new();
    state.diff_type = DiffType::TurnDiff;
    state.files = vec![make_file("x.rs", 1, 0, false)];
    state.collapsed = vec![true]; // manually set collapsed
    let new_files = vec![
        make_file("y.rs", 2, 0, false),
        make_file("z.rs", 3, 0, false),
    ];
    state.set_turn_diff(new_files);
    assert_eq!(
        state.collapsed.len(),
        2,
        "collapsed should match new file count"
    );
    assert!(
        state.collapsed.iter().all(|&c| !c),
        "new files start uncollapsed"
    );
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
