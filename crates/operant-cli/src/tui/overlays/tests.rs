// overlays/tests.rs — Unit tests for the overlays module.
//
// Extracted from the overlays.rs monolith.

use super::*;

// --- HelpOverlay ---------------------------------------------------

#[test]
fn help_overlay_toggle() {
    let mut h = HelpOverlay::new();
    assert!(!h.visible);
    h.toggle();
    assert!(h.visible);
    h.toggle();
    assert!(!h.visible);
}

#[test]
fn help_overlay_close_resets_state() {
    let mut h = HelpOverlay::new();
    h.visible = true;
    h.scroll_offset = 5;
    h.filter = "foo".to_string();
    h.close();
    assert!(!h.visible);
    assert_eq!(h.scroll_offset, 0);
    assert!(h.filter.is_empty());
}

#[test]
fn help_overlay_filter() {
    let mut h = HelpOverlay::new();
    h.push_filter_char('h');
    h.push_filter_char('e');
    assert_eq!(h.filter, "he");
    h.pop_filter_char();
    assert_eq!(h.filter, "h");
}

#[test]
fn modal_search_line_separates_leading_space_from_cursor() {
    let line = modal_search_line("", "Search", theme_colors::muted(), theme_colors::text());
    assert_eq!(line.spans.len(), 3);
    assert_eq!(line.spans[0].content.as_ref(), " ");
    assert_eq!(line.spans[1].content.as_ref(), "S");
    assert_eq!(line.spans[2].content.as_ref(), "earch");
}

// --- HistorySearchOverlay -----------------------------------------

#[test]
fn history_search_update_matches() {
    // All three entries contain 'g', so all three match.
    let history = vec![
        "git commit".to_string(),
        "cargo build".to_string(),
        "git push".to_string(),
    ];
    let mut hs = HistorySearchOverlay::open(&history);
    hs.push_char('g', &history);
    assert_eq!(hs.matches.len(), 3);

    // "gi": "cargo build" has 'g' at index 3 and 'i' in "build",
    // so it IS a subsequence match -- all three still match.
    hs.push_char('i', &history);
    assert_eq!(hs.matches.len(), 3);

    // Narrowing further to "git": "cargo build" has no 't' after g+i, so
    // only the two git entries match.
    hs.push_char('t', &history);
    assert_eq!(hs.matches.len(), 2);
    let idxs: Vec<usize> = hs.matches.iter().map(|m| m.snapshot_idx).collect();
    assert!(idxs.contains(&0));
    assert!(idxs.contains(&2));
}

#[test]
fn history_search_navigation() {
    let history = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let mut hs = HistorySearchOverlay::open(&history);
    assert_eq!(hs.selected_idx, 0);
    hs.select_prev();
    assert_eq!(hs.selected_idx, 2);
    hs.select_next();
    assert_eq!(hs.selected_idx, 0);
    hs.select_prev();
    assert_eq!(hs.selected_idx, 2);
}

#[test]
fn history_search_current_entry() {
    let history = vec!["first".to_string(), "second".to_string()];
    let hs = HistorySearchOverlay::open(&history);
    // With no query all entries match; index 0 is first.
    assert_eq!(hs.current_entry(&history), Some("first"));
}

// --- subsequence_score tests --------------------------------------

#[test]
fn subseq_score_none_for_non_subsequence() {
    // "xyz" cannot be a subsequence of "abcde"
    assert!(subsequence_score("xyz", "abcde").is_none());
    // letters out of order
    assert!(subsequence_score("ba", "abc").is_none());
}

#[test]
fn subseq_score_some_for_exact_subsequence() {
    // 'g','i','t' in order inside "git push"
    assert!(subsequence_score("git", "git push").is_some());
    // non-consecutive subsequence: 'g','t' in "get it together"
    assert!(subsequence_score("gt", "get it together").is_some());
}

#[test]
fn subseq_score_substring_beats_subsequence() {
    // "git" appears as a substring in "git push" and as a subsequence in
    // "go into town".  The substring match should score higher.
    let (score_sub, _) = subsequence_score("git", "git push").unwrap();
    let (score_seq, _) = subsequence_score("git", "go into town").unwrap();
    assert!(
        score_sub > score_seq,
        "substring score {score_sub} should beat subsequence score {score_seq}"
    );
}

#[test]
fn subseq_score_returns_correct_positions_for_substring() {
    // "git" at position 0 in "git commit" → positions 0,1,2
    let (_, positions) = subsequence_score("git", "git commit").unwrap();
    assert_eq!(positions, vec![0, 1, 2]);
}

#[test]
fn subseq_score_sorts_correctly_in_overlay() {
    // "git commit" and "get items together" both match query "git".
    // "git commit" is a substring match → higher score → appears first.
    let history = vec!["get items together".to_string(), "git commit".to_string()];
    let mut hs = HistorySearchOverlay::open(&history);
    hs.push_char('g', &history);
    hs.push_char('i', &history);
    hs.push_char('t', &history);
    // First match should be "git commit" (higher score); the snapshot is
    // newest-first, so the history's last entry is snapshot index 0.
    assert_eq!(hs.snapshot[hs.matches[0].snapshot_idx].text, "git commit");
}

#[test]
fn history_search_is_newest_first_and_unique() {
    // Ctrl+R preselects the most recent prompt, and a prompt sent twice
    // appears once (at its latest position).
    let history: Vec<String> = ["old", "middle", "old", "newest"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let hs = HistorySearchOverlay::open(&history);
    let order: Vec<&str> = hs
        .matches
        .iter()
        .map(|m| hs.snapshot[m.snapshot_idx].text.as_str())
        .collect();
    assert_eq!(order, ["newest", "old", "middle"]);
}

// --- MessageSelectorOverlay ---------------------------------------

#[test]
fn message_selector_open_selects_last() {
    let msgs = vec![
        SelectorMessage {
            idx: 0,
            role: "user".to_string(),
            preview: "hi".to_string(),
            has_tool_use: false,
        },
        SelectorMessage {
            idx: 1,
            role: "assistant".to_string(),
            preview: "hello".to_string(),
            has_tool_use: false,
        },
    ];
    let sel = MessageSelectorOverlay::open(msgs);
    assert_eq!(sel.selected_idx, 1);
}

#[test]
fn message_selector_navigate() {
    let msgs = vec![
        SelectorMessage {
            idx: 0,
            role: "user".to_string(),
            preview: "a".to_string(),
            has_tool_use: false,
        },
        SelectorMessage {
            idx: 1,
            role: "assistant".to_string(),
            preview: "b".to_string(),
            has_tool_use: false,
        },
        SelectorMessage {
            idx: 2,
            role: "user".to_string(),
            preview: "c".to_string(),
            has_tool_use: false,
        },
    ];
    let mut sel = MessageSelectorOverlay::open(msgs);
    // starts at last
    assert_eq!(sel.selected_idx, 2);
    sel.select_prev();
    assert_eq!(sel.selected_idx, 1);
    sel.select_next();
    assert_eq!(sel.selected_idx, 2);
    sel.select_next();
    assert_eq!(sel.selected_idx, 0);
}

// --- RewindFlowOverlay -------------------------------------------

#[test]
fn rewind_flow_confirm_advances_step() {
    let msgs = vec![SelectorMessage {
        idx: 0,
        role: "user".to_string(),
        preview: "hi".to_string(),
        has_tool_use: false,
    }];
    let mut flow = RewindFlowOverlay::new();
    flow.open(msgs);
    let idx = flow.confirm_selection().unwrap();
    assert_eq!(idx, 0);
    assert!(matches!(
        flow.step,
        RewindStep::Confirming { message_idx: 0 }
    ));
}

#[test]
fn rewind_flow_accept_closes() {
    let msgs = vec![SelectorMessage {
        idx: 3,
        role: "user".to_string(),
        preview: "test".to_string(),
        has_tool_use: false,
    }];
    let mut flow = RewindFlowOverlay::new();
    flow.open(msgs);
    flow.confirm_selection();
    let result = flow.accept_confirm().unwrap();
    assert_eq!(result, 3);
    assert!(!flow.visible);
}

#[test]
fn rewind_flow_reject_returns_to_selector() {
    let msgs = vec![SelectorMessage {
        idx: 0,
        role: "user".to_string(),
        preview: "x".to_string(),
        has_tool_use: false,
    }];
    let mut flow = RewindFlowOverlay::new();
    flow.open(msgs);
    flow.confirm_selection();
    assert!(matches!(flow.step, RewindStep::Confirming { .. }));
    flow.reject_confirm();
    assert_eq!(flow.step, RewindStep::Selecting);
    assert!(flow.visible);
}
