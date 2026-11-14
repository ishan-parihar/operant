// prompt_input/tests.rs — Unit tests for prompt input state + rendering.
//
// Extracted from the prompt_input/mod.rs monolith.

// vim-motion test names mirror key names (motion_W_B_basic, etc.)
#![allow(non_snake_case)]

use super::typeahead::{
    compute_file_suggestions, compute_slash_suggestions, file_ref_display_label, file_ref_value,
};
use super::vim::{
    VimOperator, motion_B, motion_E, motion_G, motion_W, motion_b, motion_e, motion_find_char,
    motion_first_nonblank, motion_gg, motion_w,
};
use super::*;
use crate::tui::slash_usage::UsageStore;

// ---- VimMode --------------------------------------------------------

#[test]
fn vim_mode_labels() {
    assert_eq!(VimMode::Insert.label(), "INSERT");
    assert_eq!(VimMode::Normal.label(), "NORMAL");
    assert_eq!(VimMode::Visual.label(), "VISUAL");
}

// ---- PromptInputState -----------------------------------------------

#[test]
fn insert_char_updates_cursor() {
    let mut s = PromptInputState::new();
    s.insert_char('h');
    s.insert_char('i');
    assert_eq!(s.text, "hi");
    assert_eq!(s.cursor, 2);
}

#[test]
fn insert_newline_works() {
    let mut s = PromptInputState::new();
    s.insert_char('a');
    s.insert_newline();
    s.insert_char('b');
    assert_eq!(s.text, "a\nb");
}

#[test]
fn backspace_removes_previous_char() {
    let mut s = PromptInputState::new();
    s.text = "hello".to_string();
    s.cursor = 5;
    s.backspace();
    assert_eq!(s.text, "hell");
    assert_eq!(s.cursor, 4);
}

#[test]
fn backspace_at_start_is_noop() {
    let mut s = PromptInputState::new();
    s.text = "hi".to_string();
    s.cursor = 0;
    s.backspace();
    assert_eq!(s.text, "hi");
}

#[test]
fn delete_removes_char_at_cursor() {
    let mut s = PromptInputState::new();
    s.text = "hello".to_string();
    s.cursor = 1;
    s.delete();
    assert_eq!(s.text, "hllo");
    assert_eq!(s.cursor, 1);
}

#[test]
fn move_left_right() {
    let mut s = PromptInputState::new();
    s.text = "abc".to_string();
    s.cursor = 1;
    s.move_right();
    assert_eq!(s.cursor, 2);
    s.move_left();
    assert_eq!(s.cursor, 1);
}

#[test]
fn cursor_visual_pos_counts_wide_characters() {
    let mut s = PromptInputState::new();
    s.text = "你a".to_string();
    s.cursor = "你".len();

    assert_eq!(s.cursor_visual_pos(10), (0, 2));
}

#[test]
fn readonly_blocks_insert() {
    let mut s = PromptInputState::new();
    s.mode = InputMode::Readonly;
    s.insert_char('x');
    assert!(s.text.is_empty());
}

#[test]
fn history_navigation_up_down() {
    let mut s = PromptInputState::new();
    s.history = vec!["first".to_string(), "second".to_string()];
    s.history_up();
    assert_eq!(s.text, "second");
    s.history_up();
    assert_eq!(s.text, "first");
    s.history_down();
    assert_eq!(s.text, "second");
    s.history_down();
    assert_eq!(s.text, "");
    assert!(s.history_pos.is_none());
}

#[test]
fn history_draft_restored() {
    let mut s = PromptInputState::new();
    s.text = "draft text".to_string();
    s.cursor = 10;
    s.history = vec!["old entry".to_string()];
    s.history_up();
    assert_eq!(s.text, "old entry");
    s.history_down();
    assert_eq!(s.text, "draft text");
}

#[test]
fn clear_resets_state() {
    let mut s = PromptInputState::new();
    s.text = "something".to_string();
    s.cursor = 5;
    s.token_estimate = 10;
    s.clear();
    assert!(s.text.is_empty());
    assert_eq!(s.cursor, 0);
    assert_eq!(s.token_estimate, 0);
}

#[test]
fn take_returns_and_clears() {
    let mut s = PromptInputState::new();
    s.text = "hello".to_string();
    s.cursor = 5;
    let taken = s.take();
    assert_eq!(taken, "hello");
    assert!(s.text.is_empty());
}

#[test]
fn is_empty_trims_whitespace() {
    let mut s = PromptInputState::new();
    s.text = "   \n  ".to_string();
    assert!(s.is_empty());
    s.text = "  x  ".to_string();
    assert!(!s.is_empty());
}

// ---- paste collapse / expand ----------------------------------------

/// The label a paste of `content` collapses to in an empty store.
fn placeholder_for(content: &str) -> String {
    match paste::PasteOutcome::of(content, &[]) {
        paste::PasteOutcome::Collapsed { placeholder, .. } => placeholder,
        paste::PasteOutcome::Inline => panic!("expected {content:?} to collapse"),
    }
}

#[test]
fn paste_small_content_inline() {
    assert_eq!(
        paste::PasteOutcome::of("short text", &[]),
        paste::PasteOutcome::Inline
    );
}

#[test]
fn paste_large_content_placeholder() {
    // >150 chars -> triggers a placeholder
    let big = "x".repeat(200);
    let placeholder = placeholder_for(&big);
    assert_eq!(placeholder, "[Pasted ~1 lines]");
}

#[test]
fn paste_large_multiline_placeholder() {
    // >=3 lines -> triggers a placeholder regardless of length
    let big = "line\n".repeat(300);
    let placeholder = placeholder_for(&big);
    assert!(
        placeholder.starts_with("[Pasted ~") && placeholder.contains("lines"),
        "expected a line-count placeholder, got: {placeholder}"
    );
}

#[test]
fn paste_three_lines_triggers_placeholder() {
    // Exactly 3 lines (the threshold) should use a placeholder.
    let three_lines = "a\nb\nc";
    assert_eq!(placeholder_for(three_lines), "[Pasted ~3 lines]");
}

#[test]
fn paste_two_lines_inline() {
    // 2 lines, <=150 chars -> inserted verbatim
    let two_lines = "hello\nworld";
    assert_eq!(
        paste::PasteOutcome::of(two_lines, &[]),
        paste::PasteOutcome::Inline
    );
}

#[test]
fn submitting_a_large_paste_sends_the_pasted_text_not_the_placeholder() {
    // 2026-10-13: the placeholder reached the model verbatim — the stored
    // paste was never read back at submit.
    let mut state = PromptInputState::new();
    state.paste("Look at this: ");
    let big = "line one\nline two\nline three\n".repeat(3);
    state.paste(&big);
    state.paste(" and this ");
    state.paste(&"x".repeat(200));
    assert!(
        state.text.contains("[Pasted ~"),
        "composer shows placeholders"
    );
    let sent = state.take();
    assert_eq!(
        sent,
        format!("Look at this: {big} and this {}", "x".repeat(200))
    );
    assert!(
        state.pending_pastes.is_empty(),
        "the store clears with the composer"
    );
}

// ---- RT-559: a deleted placeholder takes its payload with it ---------

#[test]
fn deleting_a_placeholder_drops_its_payload() {
    // The payload used to outlive the placeholder in the composer: `take()`
    // expanded it back into the model's prompt from text the operator had
    // already deleted, and undo restored a text with no expandable payload.
    let mut state = PromptInputState::new();
    let big = "a\nb\nc\nd\ne\n";
    state.paste(big);
    let placeholder = placeholder_for(big);
    assert!(state.text.contains(&placeholder));
    state.text.clear();
    state.drop_missing_paste_placeholders();
    assert!(
        state.pending_pastes.is_empty(),
        "a placeholder that left the composer takes its clipboard content with it"
    );
    assert_eq!(state.take(), "", "and it can no longer reach the model");
}

#[test]
fn a_numbered_placeholder_does_not_keep_the_bare_one_alive() {
    // The bare label is a PREFIX of the numbered one, so a plain `contains`
    // would keep the bare payload alive as long as the numbered one existed.
    let mut state = PromptInputState::new();
    let a = "a\nb\nc\n";
    let b = "x\ny\nz\nw\n";
    state.paste(a);
    state.paste(b);
    let bare = placeholder_for(a);
    let numbered = placeholder_for(b);
    assert_eq!(bare, "[Pasted ~3 lines]");
    assert_eq!(numbered, "[Pasted ~4 lines]");
    state.text = numbered.clone();
    state.drop_missing_paste_placeholders();
    assert_eq!(
        state.pending_pastes.len(),
        1,
        "the bare payload drops even though its label is a prefix of the numbered one"
    );
}

#[test]
fn keeping_a_placeholder_keeps_its_payload() {
    let mut state = PromptInputState::new();
    let big = "a\nb\nc\n";
    state.paste(big);
    state.text.insert_str(0, "look: ");
    state.drop_missing_paste_placeholders();
    assert_eq!(
        state.pending_pastes.len(),
        1,
        "an edit that leaves the placeholder alone must not drop the payload"
    );
    assert_eq!(
        state.take(),
        format!("look: {}", placeholder_for(big)).replace(&placeholder_for(big), big)
    );
}

// ---- RT-562: collapse is reversible ---------------------------------

#[test]
fn repasting_the_same_content_expands_the_placeholder() {
    // The collapse is no longer one-way: the payload is still in the store,
    // so pasting the same content again brings it back.
    let mut state = PromptInputState::new();
    let big = "a\nb\nc\nd\ne\n";
    state.paste(big);
    let placeholder = placeholder_for(big);
    assert!(state.text.contains(&placeholder), "collapsed first");
    state.paste(big);
    assert!(
        state.text.contains("a\nb\nc\nd\ne\n"),
        "the second paste of the same content expanded it"
    );
    assert!(
        !state.text.contains(&placeholder),
        "and the placeholder went with it"
    );
    assert!(state.pending_pastes.is_empty());
}

#[test]
fn expanding_reports_the_size_of_what_it_unwrapped() {
    let mut state = PromptInputState::new();
    let big = "a\nb\nc\nd\ne\n";
    state.paste(big);
    state.paste(big);
    let notice = state.paste_expanded.take().expect("expand reports");
    assert!(notice.contains("5 lines"), "got: {notice}");
}

#[test]
fn two_pastes_of_the_same_shape_are_not_confused() {
    // Same shape, different content: the numbers disambiguate, and a re-paste
    // of the first expands the FIRST placeholder.
    let mut state = PromptInputState::new();
    let one = "1\n2\n3\n";
    let two = "a\nb\nc\n";
    state.paste(one);
    state.paste(two);
    let labels: Vec<String> = state
        .pending_pastes
        .iter()
        .map(|(label, _)| label.clone())
        .collect();
    assert_ne!(
        labels[0], labels[1],
        "the two same-shape pastes get distinct labels"
    );
    state.paste(one);
    assert!(
        state.text.contains("1\n2\n3\n"),
        "the first paste's payload came back"
    );
    assert!(
        !state.text.contains("a\nb\nc\n"),
        "the second paste's payload stayed collapsed"
    );
    assert!(
        state.text.contains(labels[1].as_str()),
        "the second placeholder is still live"
    );
}

// ---- compute_typeahead ---------------------------------------------

// Helper constants for tests
const TEST_FILE_AUTOCOMPLETE_LIMIT: usize = 15;
const TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN: bool = false;

/// (plan 04) The TUI's real command list comes from the ONE table —
/// REGISTERED_COMMANDS via tui_slash_command_data. Typing a TUI-only
/// command's prefix must suggest it: pre-plan-04 the typeahead read the
/// GATEWAY's COMMAND_REGISTRY, so /rewind, /permissions and friends never
/// appeared.
#[test]
fn typeahead_suggests_the_registered_tui_commands() {
    let cmds = crate::tui::app::helpers::tui_slash_command_data(&[]);
    let cmds: Vec<(&str, &str)> = cmds
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect();
    for needle in ["/rew", "/perm", "/goal", "/undo"] {
        let suggestions = compute_slash_suggestions(needle, &cmds, &UsageStore::default());
        assert!(
            suggestions.iter().any(|s| s.text.starts_with(needle)),
            "typeahead must suggest a registered command for {needle}"
        );
    }
    // The lying suggestions are gone: nothing suggests the deleted
    // commands. Every suggestion's dispatch is pinned by the sweep; this
    // pins that the SUGGESTIONS come from the one table.
    for (name, _) in &cmds {
        assert!(
            !matches!(
                *name,
                "history"
                    | "handoff"
                    | "snapshot"
                    | "env"
                    | "toolsets"
                    | "blueprint"
                    | "gquota"
                    | "billing"
                    | "credits"
                    | "pet"
                    | "heapdump"
                    | "replay"
                    | "busy"
                    | "browser"
                    | "indicator"
            ),
            "{name} must not be suggested — it was deleted or made honest (plan 04)"
        );
    }
}

#[test]
fn typeahead_slash_prefix_matches() {
    let cmds = [
        ("help", "Show help"),
        ("history", "Show history"),
        ("compact", "Compact"),
    ];
    let suggestions = compute_slash_suggestions("/h", &cmds, &UsageStore::default());
    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0].text, "/help");
    assert_eq!(suggestions[1].text, "/history");
}

#[test]
fn typeahead_full_match() {
    let cmds = [("compact", "Compact conversation")];
    let suggestions = compute_slash_suggestions("/compact", &cmds, &UsageStore::default());
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].text, "/compact");
    assert_eq!(suggestions[0].description, "Compact conversation");
}

#[test]
fn typeahead_case_insensitive() {
    let cmds = [("Help", "Show help")];
    let suggestions = compute_slash_suggestions("/H", &cmds, &UsageStore::default());
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].text, "/Help");
}

// ---- skill / bundle name typeahead (iter-320) ----------------------

#[test]
fn typeahead_skill_names_complete() {
    // Hold the snapshot-writer lock for the whole test: parallel tests that
    // construct `App` re-register the real installed skills into the
    // process-wide snapshot, which would otherwise be replaced between our
    // registration and assertions (flaky under `cargo test --workspace`).
    let _guard = super::typeahead::SKILL_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    super::typeahead::set_typeahead_names(
        vec!["gitcrawl".to_string(), "web-research".to_string()],
        vec!["research-pack".to_string()],
    );
    let cmds: [(&str, &str); 0] = [];

    // /skill <prefix> completes installed skill names. (The snapshot is
    // process-wide and shared with parallel tests that construct App and
    // register the real installed skills, so assert membership, not length.)
    let s = compute_typeahead("/skill git", &cmds, &UsageStore::default(), 15, false);
    assert!(s.iter().any(|x| x.text == "/skill gitcrawl"));

    // /bundle <prefix> completes bundle names too.
    let s = compute_typeahead("/bundle rese", &cmds, &UsageStore::default(), 15, false);
    assert!(s.iter().any(|x| x.text == "/bundle research-pack"));

    // No match → empty (falls through to normal slash suggestions).
    let s = compute_typeahead("/skill zzz", &cmds, &UsageStore::default(), 15, false);
    assert!(s.is_empty());

    // Regular slash commands still work after the skill prefix handling.
    let cmds2 = [("help", "Show help")];
    let s = compute_typeahead("/he", &cmds2, &UsageStore::default(), 15, false);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].text, "/help");
}

// RT-30: slash usage was recorded and never ranked the typeahead. The
// suggestions now order by the usage store the caller passes in.
#[test]
fn slash_suggestions_rank_by_the_usage_the_caller_passes() {
    let mut usage = UsageStore::default();
    usage.record("history");
    let cmds = [("help", "Show help"), ("history", "History")];

    let s = compute_slash_suggestions("/", &cmds, &usage);
    assert_eq!(s[0].text, "/history", "the recently used command leads");
    assert_eq!(s[1].text, "/help");

    let s = compute_slash_suggestions("/", &cmds, &UsageStore::default());
    assert_eq!(s[0].text, "/help", "with no usage, declaration order holds");
}

// ---- suggestion navigation -----------------------------------------

#[test]
fn suggestion_next_cycles() {
    let mut s = PromptInputState::new();
    let cmds = [
        ("help", "Help"),
        ("history", "History"),
        ("compact", "Compact"),
    ];
    s.text = "/h".to_string();
    s.cursor = s.text.len();
    s.update_suggestions(&cmds, &UsageStore::default(), 15, false);
    assert_eq!(s.suggestions.len(), 2);
    assert_eq!(s.suggestion_index, Some(0));
    s.suggestion_next();
    assert_eq!(s.suggestion_index, Some(1));
    s.suggestion_next();
    assert_eq!(s.suggestion_index, Some(0)); // wraps
}

#[test]
fn accept_suggestion_fills_text() {
    let mut s = PromptInputState::new();
    let cmds = [("help", "Show help")];
    s.text = "/he".to_string();
    s.cursor = s.text.len();
    s.update_suggestions(&cmds, &UsageStore::default(), 15, false);
    s.suggestion_next();
    s.accept_suggestion();
    assert_eq!(s.text, "/help");
    assert_eq!(s.cursor, 5);
    assert!(s.suggestions.is_empty());
}

// ---- token estimate -------------------------------------------------

#[test]
fn token_estimate_rough() {
    let mut s = PromptInputState::new();
    for _ in 0..40 {
        s.insert_char('a');
    }
    // 40 chars / 4 = 10 tokens
    assert_eq!(s.token_estimate, 10);
}

// ---- motion_w / motion_b -----------------------------------------------

#[test]
fn motion_w_basic() {
    assert_eq!(motion_w("hello world", 0), 6);
    assert_eq!(motion_w("hello world", 6), 11); // at start of 'world', moves to end
    assert_eq!(motion_w("  foo", 0), 2); // skip leading spaces
}

#[test]
fn motion_b_basic() {
    assert_eq!(motion_b("hello world", 6), 0); // 'w' → start of 'hello'
    assert_eq!(motion_b("hello world", 0), 0); // already at start
}

#[test]
fn motion_e_basic() {
    assert_eq!(motion_e("hello world", 0), 4); // cursor on 'h', end at 'o'
    assert_eq!(motion_e("hello world", 4), 10); // at 'o' (end), jump to 'd'
}

#[test]
fn motion_W_B_basic() {
    // "foo.bar baz"  W from 0 → 8 ('b' of 'baz')
    assert_eq!(motion_W("foo.bar baz", 0), 8);
    assert_eq!(motion_B("foo.bar baz", 8), 0);
}

#[test]
fn motion_E_basic() {
    assert_eq!(motion_E("foo.bar baz", 0), 6); // end of 'foo.bar' WORD
}

#[test]
fn motion_first_nonblank_basic() {
    assert_eq!(motion_first_nonblank("  hello", 0), 2);
    assert_eq!(motion_first_nonblank("hello", 0), 0);
}

#[test]
fn motion_G_basic() {
    assert_eq!(motion_G("foo\nbar"), 4);
    assert_eq!(motion_G("single line"), 0);
}

#[test]
fn motion_gg_basic() {
    assert_eq!(motion_gg("foo\nbar\nbaz", 1), 0);
    assert_eq!(motion_gg("foo\nbar\nbaz", 2), 4);
    assert_eq!(motion_gg("foo\nbar\nbaz", 3), 8);
}

#[test]
fn motion_find_char_f() {
    // f: cursor lands on 'o', count=1
    assert_eq!(
        motion_find_char("hello", 0, 'o', VimFindKind::F, 1),
        Some(4)
    );
    // f: not found
    assert_eq!(motion_find_char("hello", 0, 'z', VimFindKind::F, 1), None);
}

#[test]
fn motion_find_char_t() {
    // t: cursor stops before 'o'
    assert_eq!(
        motion_find_char("hello", 0, 'o', VimFindKind::T, 1),
        Some(3)
    );
}

#[test]
fn motion_find_char_bigF() {
    // F: search backward
    assert_eq!(
        motion_find_char("hello", 4, 'h', VimFindKind::BigF, 1),
        Some(0)
    );
}

// ---- apply_vim_key new commands ----------------------------------------

#[test]
fn vim_key_e_motion() {
    let mut mode = VimMode::Normal;
    let mut text = "hello world".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "e",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 4); // end of 'hello'
}

#[test]
fn vim_key_j_and_k_move_between_lines() {
    let mut mode = VimMode::Normal;
    let mut text = "one\ntwo\nthree".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;

    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "j",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(
        cursor, 4,
        "j must move down one line, to the start of `two`"
    );

    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "k",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 0, "k must move back up one line");
}

#[test]
fn vim_key_W_motion() {
    let mut mode = VimMode::Normal;
    let mut text = "foo.bar baz".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "W",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 8); // 'baz'
}

#[test]
fn vim_key_G_last_line() {
    let mut mode = VimMode::Normal;
    let mut text = "first\nsecond\nthird".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "G",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 13); // start of 'third'
}

#[test]
fn vim_key_gg_first_line() {
    let mut mode = VimMode::Normal;
    let mut text = "first\nsecond".to_string();
    let mut cursor = 6usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    // 'g' sets pending G
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "g",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert!(matches!(pending, VimPendingState::G { .. }));
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "g",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 0);
}

#[test]
fn vim_key_count_motion() {
    let mut mode = VimMode::Normal;
    let mut text = "a b c d e".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    // 3w — advance 3 words
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "3",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert!(matches!(pending, VimPendingState::Count { .. }));
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "w",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 6); // 3 words forward: a→b→c→d start = pos 6
}

#[test]
fn vim_key_dw_delete_word() {
    let mut mode = VimMode::Normal;
    let mut text = "hello world".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "d",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert!(matches!(
        pending,
        VimPendingState::Operator {
            op: VimOperator::Delete,
            ..
        }
    ));
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "w",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(text, "world");
    assert_eq!(yank, "hello ");
}

#[test]
fn vim_key_cw_change_word_enters_insert() {
    let mut mode = VimMode::Normal;
    let mut text = "hello world".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "c",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "w",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(mode, VimMode::Insert);
    assert_eq!(text, "world");
}

#[test]
fn vim_key_dd_deletes_line() {
    let mut mode = VimMode::Normal;
    let mut text = "first\nsecond".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "d",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "d",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(text, "second");
    assert_eq!(yank, "first\n");
}

#[test]
fn vim_key_r_replace_char() {
    let mut mode = VimMode::Normal;
    let mut text = "hello".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "r",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert!(matches!(pending, VimPendingState::Replace { .. }));
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "H",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(text, "Hello");
    assert_eq!(mode, VimMode::Normal); // stays in Normal after replace
}

#[test]
fn vim_key_find_f() {
    let mut mode = VimMode::Normal;
    let mut text = "hello world".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "f",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "o",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 4); // first 'o' in 'hello'
    assert_eq!(last_find, Some((VimFindKind::F, 'o')));
}

#[test]
fn vim_key_semicolon_repeat_find() {
    let mut mode = VimMode::Normal;
    let mut text = "a.b.c".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "f",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        ".",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 1);
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        ";",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(cursor, 3); // repeated find → next '.'
}

#[test]
fn vim_key_X_delete_before_cursor() {
    let mut mode = VimMode::Normal;
    let mut text = "hello".to_string();
    let mut cursor = 4usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "X",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(text, "helo");
    assert_eq!(cursor, 3);
}

#[test]
fn vim_key_tilde_toggle_case() {
    let mut mode = VimMode::Normal;
    let mut text = "hello".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "~",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(text, "Hello");
}

#[test]
fn vim_key_o_open_line_below() {
    let mut mode = VimMode::Normal;
    let mut text = "first\nthird".to_string();
    let mut cursor = 0usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "o",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(mode, VimMode::Insert);
    assert!(text.contains('\n'));
    assert_eq!(cursor, 6); // after first newline
}

#[test]
fn vim_key_D_delete_to_eol() {
    let mut mode = VimMode::Normal;
    let mut text = "hello world".to_string();
    let mut cursor = 6usize;
    let mut yank = String::new();
    let mut pending = VimPendingState::None;
    let mut last_find = None;
    apply_vim_key(
        &mut mode,
        &mut text,
        &mut cursor,
        "D",
        &mut yank,
        &mut pending,
        &mut last_find,
    );
    assert_eq!(text, "hello ");
    assert_eq!(yank, "world");
}

// ---- PromptInputState undo ---------------------------------------------

#[test]
fn prompt_input_undo_restores_text() {
    let mut s = PromptInputState::new();
    s.vim_enabled = true;
    s.vim_mode = VimMode::Normal;
    s.text = "hello".to_string();
    s.cursor = 5;
    s.vim_command("x"); // deletes 'o' (but cursor at 5 = past end)
    // let's set cursor to 4 and delete
    s.cursor = 4;
    s.vim_command("x");
    assert_eq!(s.text, "hell");
    s.vim_command("u");
    assert_eq!(s.text, "hello");
}

#[test]
fn prompt_input_visual_yank() {
    let mut s = PromptInputState::new();
    s.vim_enabled = true;
    s.vim_mode = VimMode::Normal;
    s.text = "hello world".to_string();
    s.cursor = 0;
    s.vim_command("v");
    assert_eq!(s.vim_mode, VimMode::Visual);
    // Move to end of word
    s.vim_command("e");
    s.vim_command("y"); // yank selection
    assert_eq!(s.yank_buf, "hello");
    assert_eq!(s.vim_mode, VimMode::Normal);
}

// ---- Named registers ------------------------------------------------

#[test]
fn register_yank_and_paste() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello world".to_string();
    s.cursor = 0;
    // `"ay` — yank line to register 'a'
    s.vim_command("\"");
    s.vim_command("a");
    s.vim_command("y");
    assert_eq!(
        s.vim_registers.get(&'a').map(|s| s.as_str()),
        Some("hello world")
    );
    // `"ap` — paste from register 'a' after cursor
    s.cursor = 0;
    s.vim_command("\"");
    s.vim_command("a");
    s.vim_command("p");
    assert!(s.text.contains("hello world"));
}

#[test]
fn register_yank_method() {
    let mut s = PromptInputState::new();
    s.yank_to_register('b', "some text");
    assert_eq!(s.paste_from_register('b'), Some("some text".to_string()));
    assert_eq!(s.paste_from_register('z'), None);
}

#[test]
fn register_delete_to_named() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello\nworld".to_string();
    s.cursor = 0;
    // `"ad` — delete line to register 'a'
    s.vim_command("\"");
    s.vim_command("a");
    s.vim_command("d");
    assert_eq!(
        s.vim_registers.get(&'a').map(|s| s.as_str()),
        Some("hello\n")
    );
    assert_eq!(s.text, "world");
}

// ---- Marks ----------------------------------------------------------

#[test]
fn mark_set_and_jump() {
    let mut s = PromptInputState::new();
    s.text = "hello world".to_string();
    s.cursor = 6; // at 'w'
    s.set_mark('a');
    s.cursor = 0;
    s.jump_to_mark('a');
    assert_eq!(s.cursor, 6);
}

#[test]
fn mark_jump_nonexistent_is_noop() {
    let mut s = PromptInputState::new();
    s.text = "hello".to_string();
    s.cursor = 3;
    s.jump_to_mark('z'); // no mark 'z' set
    assert_eq!(s.cursor, 3);
}

#[test]
fn mark_via_vim_command() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello world".to_string();
    s.cursor = 6;
    // `ma` — set mark 'a'
    s.vim_command("m");
    s.vim_command("a");
    assert!(s.vim_marks.contains_key(&'a'));
    // Move cursor and jump back with `'a`
    s.cursor = 0;
    s.vim_command("'");
    s.vim_command("a");
    assert_eq!(s.cursor, 6);
}

#[test]
fn mark_clamped_when_text_shortened() {
    let mut s = PromptInputState::new();
    s.text = "hello world".to_string();
    s.cursor = 10;
    s.set_mark('x');
    // Shorten the text
    s.text = "hi".to_string();
    s.cursor = 0;
    s.jump_to_mark('x');
    // Should clamp to text length
    assert!(s.cursor <= s.text.len());
    assert!(s.text.is_char_boundary(s.cursor));
}

// ---- Macro recording ------------------------------------------------

#[test]
fn macro_record_and_replay() {
    let mut s = PromptInputState::new();
    // Start recording into register 'q'
    s.start_macro_recording('q');
    assert_eq!(s.vim_macro_recording, Some('q'));
    // Simulate accumulating keys
    s.vim_macro_content
        .get_mut(&'q')
        .unwrap()
        .push("w".to_string());
    s.vim_macro_content
        .get_mut(&'q')
        .unwrap()
        .push("e".to_string());
    // Stop recording
    let reg = s.stop_macro_recording();
    assert_eq!(reg, Some('q'));
    assert_eq!(s.vim_macro_recording, None);
    // Replay
    let keys = s.replay_macro('q');
    assert_eq!(keys, vec!["w".to_string(), "e".to_string()]);
}

#[test]
fn macro_replay_empty_register() {
    let s = PromptInputState::new();
    let keys = s.replay_macro('z');
    assert!(keys.is_empty());
}

#[test]
fn macro_via_vim_command() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "abc".to_string();
    s.cursor = 0;
    // `qq` — start recording into 'q'
    s.vim_command("q");
    assert!(matches!(s.vim_pending, VimPendingState::MacroRecord));
    s.vim_command("q"); // register name = 'q'
    assert_eq!(s.vim_macro_recording, Some('q'));
    // Record some keys: move right twice
    s.vim_command("l");
    s.vim_command("l");
    // Stop recording with `q`
    s.vim_command("q");
    assert_eq!(s.vim_macro_recording, None);
    // The recorded content should have 'l', 'l'
    let keys = s.replay_macro('q');
    assert_eq!(keys, vec!["l".to_string(), "l".to_string()]);
}

#[test]
fn macro_replay_via_at() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "abcdef".to_string();
    s.cursor = 0;
    // Manually record a macro: move 2 chars right
    s.vim_macro_content
        .insert('q', vec!["l".to_string(), "l".to_string()]);
    // `@q` — replay macro 'q'
    s.vim_command("@");
    assert!(matches!(s.vim_pending, VimPendingState::MacroReplay));
    s.vim_command("q");
    // cursor should have moved right by 2
    assert_eq!(s.cursor, 2);
}

// ---- Dot-repeat -----------------------------------------------------

#[test]
fn dot_repeat_delete_char() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello".to_string();
    s.cursor = 0;
    // Delete char at cursor with `x`
    s.vim_command("x");
    assert_eq!(s.text, "ello");
    // Dot-repeat should delete again
    s.vim_command(".");
    assert_eq!(s.text, "llo");
}

#[test]
fn dot_repeat_replace_char() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello".to_string();
    s.cursor = 0;
    // Replace 'h' with 'H' using `r`
    s.vim_command("r");
    s.vim_command("H");
    assert_eq!(s.text, "Hello");
    // Move and dot-repeat: should replace 'e' with 'H'
    s.vim_command("l");
    s.vim_command(".");
    assert_eq!(s.text, "HHllo");
}

#[test]
fn dot_repeat_noop_when_no_action() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello".to_string();
    s.cursor = 0;
    // `.` with no prior modifying action should be a no-op
    s.vim_command(".");
    assert_eq!(s.text, "hello");
    assert_eq!(s.cursor, 0);
}

#[test]
fn dot_repeat_after_visual_delete() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello world".to_string();
    s.cursor = 0;
    // Enter visual, select 'hel', then delete
    s.vim_command("v");
    s.vim_command("l");
    s.vim_command("l");
    s.vim_command("d");
    assert_eq!(s.text, "lo world");
    // Dot-repeat should delete chars again
    s.vim_command(".");
    // The text should be shorter
    assert!(s.text.len() < "lo world".len());
}

// ---- Visual line mode (V) -------------------------------------------

#[test]
fn visual_line_mode_enter() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "line one\nline two".to_string();
    s.cursor = 0;
    s.vim_command("V");
    assert_eq!(s.vim_mode, VimMode::VisualLine);
    assert!(s.visual_anchor.is_some());
}

#[test]
fn visual_line_yank() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "line one\nline two".to_string();
    s.cursor = 0;
    s.vim_command("V");
    s.vim_command("y");
    assert_eq!(s.vim_mode, VimMode::Normal);
    assert_eq!(s.yank_buf, "line one\n");
}

#[test]
fn visual_line_delete() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "line one\nline two".to_string();
    s.cursor = 0;
    s.vim_command("V");
    s.vim_command("d");
    assert_eq!(s.vim_mode, VimMode::Normal);
    assert_eq!(s.text, "line two");
}

#[test]
fn visual_line_escape_returns_normal() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello".to_string();
    s.vim_command("V");
    assert_eq!(s.vim_mode, VimMode::VisualLine);
    s.vim_command("Escape");
    assert_eq!(s.vim_mode, VimMode::Normal);
}

// ---- Command-line mode (:) ------------------------------------------

#[test]
fn command_line_mode_enter() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    assert_eq!(s.vim_mode, VimMode::Command);
    assert!(s.vim_command_buf.is_empty());
}

#[test]
fn command_line_accumulates_chars() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    s.vim_command("q");
    assert_eq!(s.vim_command_buf, "q");
    s.vim_command("!");
    assert_eq!(s.vim_command_buf, "q!");
}

#[test]
fn command_line_backspace_pops() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    s.vim_command("q");
    s.vim_command("w");
    s.vim_command("Backspace");
    assert_eq!(s.vim_command_buf, "q");
}

#[test]
fn command_line_empty_backspace_cancels() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    s.vim_command("Backspace");
    assert_eq!(s.vim_mode, VimMode::Normal);
}

#[test]
fn command_q_sets_quit_flag() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    s.vim_command("q");
    s.vim_command("Enter");
    assert!(s.vim_quit_requested);
    assert_eq!(s.vim_mode, VimMode::Normal);
}

#[test]
fn command_noh_clears_search() {
    let mut s = PromptInputState::new();
    s.vim_search_last = Some("foo".to_string());
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    for c in "noh".chars() {
        s.vim_command(&c.to_string());
    }
    s.vim_command("Enter");
    assert!(s.vim_search_last.is_none());
}

#[test]
fn command_escape_cancels() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command(":");
    s.vim_command("q");
    s.vim_command("Escape");
    assert_eq!(s.vim_mode, VimMode::Normal);
}

// ---- In-prompt search (/) -------------------------------------------

#[test]
fn search_mode_enter() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command("/");
    assert_eq!(s.vim_mode, VimMode::Search);
    assert!(s.vim_search_buf.is_empty());
}

#[test]
fn search_finds_match_and_moves_cursor() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "hello world hello".to_string();
    s.cursor = 0;
    s.vim_command("/");
    for c in "world".chars() {
        s.vim_command(&c.to_string());
    }
    s.vim_command("Enter");
    assert_eq!(s.vim_mode, VimMode::Normal);
    assert_eq!(s.cursor, 6); // "world" starts at byte 6
    assert_eq!(s.vim_search_last.as_deref(), Some("world"));
}

#[test]
fn search_n_finds_next() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "aa bb aa".to_string();
    s.cursor = 0;
    s.vim_command("/");
    s.vim_command("a");
    s.vim_command("a");
    s.vim_command("Enter");
    assert_eq!(s.cursor, 0); // first 'aa'
    s.vim_command("n");
    assert_eq!(s.cursor, 6); // second 'aa'
}

#[test]
fn search_N_finds_prev() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.text = "aa bb aa".to_string();
    s.cursor = 7; // at second 'aa'
    s.vim_search_last = Some("aa".to_string());
    s.vim_command("N");
    assert_eq!(s.cursor, 0); // wraps to first 'aa'
}

#[test]
fn search_escape_cancels() {
    let mut s = PromptInputState::new();
    s.vim_mode = VimMode::Normal;
    s.vim_command("/");
    s.vim_command("f");
    s.vim_command("Escape");
    assert_eq!(s.vim_mode, VimMode::Normal);
}

// ---- VimMode labels -------------------------------------------------

#[test]
fn vim_mode_new_labels() {
    assert_eq!(VimMode::VisualLine.label(), "VISUAL LINE");
    assert_eq!(VimMode::Command.label(), "COMMAND");
    assert_eq!(VimMode::Search.label(), "SEARCH");
}

// ---- File reference (@) autocomplete tests ----

#[test]
fn file_autocomplete_slash_commands_still_work() {
    let cmds = vec![("help", "Show help"), ("clear", "Clear messages")];
    let suggestions = compute_slash_suggestions("/he", &cmds, &UsageStore::default());
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].text, "/help");
}

#[test]
fn file_autocomplete_at_requires_word_boundary() {
    // @ at word boundary: should suggest files (or be empty if cwd has no files)
    let suggestions_at_boundary = compute_file_suggestions(
        "@",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    let suggestions_at_boundary_with_space = compute_file_suggestions(
        "hello @",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );

    // @ not at word boundary: should never suggest files
    let suggestions_no_boundary = compute_file_suggestions(
        "test@",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert!(
        suggestions_no_boundary.is_empty(),
        "@ without word boundary should never suggest files"
    );

    // At least one of the boundary cases should work if cwd has files
    // but more importantly, the non-boundary case should always be empty
    for suggestion in suggestions_at_boundary
        .iter()
        .chain(suggestions_at_boundary_with_space.iter())
    {
        assert_eq!(suggestion.source, TypeaheadSource::FileRef);
    }
}

#[test]
fn file_autocomplete_returns_fileref_source() {
    let suggestions = compute_file_suggestions(
        "@",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );

    for suggestion in suggestions {
        assert_eq!(suggestion.source, TypeaheadSource::FileRef);
    }
}

#[test]
fn file_autocomplete_format_filenames() {
    let suggestions = compute_file_suggestions(
        "@",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );

    // All suggestions should start with @
    for suggestion in suggestions {
        assert!(suggestion.text.starts_with('@'));
    }
}

#[test]
fn file_autocomplete_with_whitespace_prefix() {
    // @ after whitespace: should suggest files
    let suggestions = compute_file_suggestions(
        "hello @",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );

    // Check they all start with @ and are FileRef source
    for suggestion in suggestions {
        assert!(suggestion.text.starts_with('@'));
        assert_eq!(suggestion.source, TypeaheadSource::FileRef);
    }
}

/// (iter-1939) The consumer half of the two @-autocomplete SETTINGS.
///
/// The settings screen's save path is already pinned ("every entry MUST be
/// handled by `apply_and_save`", iter-1176, plus the autocomplete-limit
/// round-trip), but NOTHING pinned that the saved value reaches the popup — the
/// half that decides whether the row is real. Every earlier test here passed a
/// fixed `15` and listed the REPO's own working directory, whose file count
/// nobody knows, so a limit the popup ignored would still have been green.
///
/// This drives the real handler through a tempdir whose file count is known, so
/// the limit and the hidden policy are observable:
/// - limit 3 over 9 candidates yields 3, and limit 1 yields 1;
/// - show_hidden=false hides the dotfile, true shows it.
#[test]
fn the_file_autocomplete_settings_reach_the_popup_that_renders_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..8 {
        std::fs::write(dir.path().join(format!("file{i}.rs")), b"x").expect("write a visible file");
    }
    std::fs::write(dir.path().join(".hidden.rs"), b"x").expect("write a hidden file");

    // An absolute prefix lists that directory — no dependence on the cwd, so
    // this is deterministic under the suite's parallel test threads.
    let prefix = format!("@{}/", dir.path().to_string_lossy());

    let at = |limit: usize, hidden: bool| compute_file_suggestions(&prefix, limit, hidden);

    // --- the limit: 9 candidates, 3 asked for -----------------------------
    let capped = at(3, false);
    assert_eq!(
        capped.len(),
        3,
        "the @-autocomplete limit must bound the popup the operator sees — \
         3 asked for, 9 available: {:?}",
        capped.iter().map(|s| s.text.clone()).collect::<Vec<_>>()
    );
    let tighter = at(1, false);
    assert_eq!(
        tighter.len(),
        1,
        "a tighter limit must narrow the same popup — the setting is read live, \
         not once at startup"
    );

    // --- the hidden policy: the dotfile is the discriminator --------------
    let without_hidden = at(20, false);
    assert!(
        without_hidden.iter().all(|s| !s.text.contains(".hidden")),
        "with hidden files off the popup must not offer .hidden.rs: {:?}",
        without_hidden
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
    );
    let with_hidden = at(20, true);
    assert!(
        with_hidden.iter().any(|s| s.text.contains(".hidden")),
        "with hidden files on the popup must offer .hidden.rs — the row is \
         advertised in the settings screen as controlling this"
    );
}

#[test]
fn file_autocomplete_detects_symlinks() {
    // This test verifies that symlinks/junction links are properly detected.
    // On systems with symlinks/junctions, suggestions will include descriptions
    // like "file link" or "directory link".
    let suggestions = compute_file_suggestions(
        "@",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );

    // All suggestions should have a description (file, directory, file link, or directory link)
    for suggestion in suggestions {
        assert!(!suggestion.description.is_empty());
        assert!(
            suggestion.description.contains("file") || suggestion.description.contains("directory"),
            "Unexpected description: {}",
            suggestion.description
        );
    }
}

// ---- has_active_file_ref tests ----------------------------------------

#[test]
fn has_active_file_ref_at_start() {
    let mut s = PromptInputState::new();
    s.text = "@src/".to_string();
    s.cursor = s.text.len();
    assert!(s.has_active_file_ref());
}

#[test]
fn has_active_file_ref_after_space() {
    let mut s = PromptInputState::new();
    s.text = "hello @".to_string();
    s.cursor = s.text.len();
    assert!(s.has_active_file_ref());
}

#[test]
fn has_active_file_ref_email_not_boundary() {
    let mut s = PromptInputState::new();
    s.text = "email@host".to_string();
    s.cursor = s.text.len();
    assert!(!s.has_active_file_ref());
}

#[test]
fn has_active_file_ref_no_at() {
    let mut s = PromptInputState::new();
    s.text = "no at sign here".to_string();
    s.cursor = s.text.len();
    assert!(!s.has_active_file_ref());
}

// ---- the caret and the painter must agree on the row -------------------

/// The caret's row and the painter's row are ONE model now (2026-11-12).
/// The caret used to resolve its row as `display_col / width`, which put a
/// caret sitting at the end of a full-width row on the NEXT row — and Up/Down
/// then walked a row the operator cannot see.
#[test]
fn the_caret_row_is_the_row_the_painter_draws() {
    let mut s = PromptInputState::new();
    s.text = "abcdefghij".to_string(); // exactly one full row of ten cells
    s.cursor = s.text.len();
    let rows = super::visual::wrap_input_rows(&s.text, 10);
    assert_eq!(rows.len(), 1, "the line fits in one painted row");
    assert_eq!(
        s.cursor_visual_pos(10),
        (0, 10),
        "the caret stays on the row the painter drew"
    );
}

/// A CJK row is ten cells wide but only five characters: the caret must still
/// land on the painted row, and the column must be a DISPLAY column.
#[test]
fn the_caret_column_is_a_display_column_on_a_cjk_row() {
    let mut s = PromptInputState::new();
    s.text = "你你你你你".to_string();
    s.cursor = s.text.len();
    let rows = super::visual::wrap_input_rows(&s.text, 10);
    assert_eq!(rows.len(), 1);
    assert_eq!(s.cursor_visual_pos(10), (0, 10));
}

/// Up from the second painted row lands on the first painted row's end — the
/// old model computed the target as "row index × width CHARACTERS", which is
/// a byte offset past the end of a CJK row.
#[test]
fn moving_up_lands_on_the_painted_row() {
    let mut s = PromptInputState::new();
    s.text = "你你你你你你你你你你".to_string(); // ten wide chars = two rows
    s.cursor = s.text.len();
    assert!(s.move_visual_up(10), "there is a row above");
    assert_eq!(
        s.cursor,
        "你你你你你".len(),
        "the caret lands at the end of the first painted row"
    );
}

// ---- accept_suggestion FileRef tests ------------------------------------

#[test]
fn accept_suggestion_file_ref_at_start() {
    let mut s = PromptInputState::new();
    s.text = "@src/ma".to_string();
    s.cursor = s.text.len();
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@file:src/main.rs".to_string(),
        display: "@file:src/main.rs".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    // M4: values use the @file: grammar and file accepts append a space.
    assert_eq!(s.text, "@file:src/main.rs ");
    assert_eq!(s.cursor, "@file:src/main.rs ".len());
    assert!(s.suggestions.is_empty());
}

#[test]
fn accept_suggestion_file_ref_after_text_preserves_prefix() {
    let mut s = PromptInputState::new();
    s.text = "some text @src/ma".to_string();
    s.cursor = s.text.len();
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@file:src/main.rs".to_string(),
        display: "@file:src/main.rs".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "some text @file:src/main.rs ");
    assert_eq!(s.cursor, "some text @file:src/main.rs ".len());
}

#[test]
fn accept_suggestion_file_ref_preserves_tail() {
    let mut s = PromptInputState::new();
    // Cursor is mid-string; tail after cursor is preserved
    let prefix = "@src/ma";
    let tail = " more text";
    s.text = format!("{}{}", prefix, tail);
    s.cursor = prefix.len();
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@file:src/main.rs".to_string(),
        display: "@file:src/main.rs".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    // The tail already starts with a space: no second one is inserted.
    assert_eq!(s.text, "@file:src/main.rs more text");
    assert_eq!(s.cursor, "@file:src/main.rs".len());
}

// ---- slash-command completion: the TOKEN is spliced, never the buffer ---

/// The operator's contract (2026-11-12): "when using tab with slash
/// command, all text in front of it must not be deleted". The accept
/// replaces the `/command` TOKEN and leaves every other character of the
/// draft alone.
#[test]
fn accept_suggestion_slash_command_preserves_the_text_in_front() {
    let mut s = PromptInputState::new();
    s.text = "look at /rev".to_string();
    s.cursor = s.text.len();
    s.suggestions = vec![TypeaheadSuggestion {
        text: "/review".to_string(),
        display: "/review".to_string(),
        description: "review".to_string(),
        source: TypeaheadSource::SlashCommand,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "look at /review");
    assert_eq!(s.cursor, "look at /review".len());
}

/// The same contract for the DRAFT TAIL: a multi-line prompt whose first
/// line is a command must survive the accept. The whole-buffer rewrite
/// this replaces deleted the tail (and every line below it).
#[test]
fn accept_suggestion_slash_command_preserves_the_draft_tail() {
    let mut s = PromptInputState::new();
    s.text = "/rev\nand some notes".to_string();
    s.cursor = 4;
    s.suggestions = vec![TypeaheadSuggestion {
        text: "/review".to_string(),
        display: "/review".to_string(),
        description: "review".to_string(),
        source: TypeaheadSource::SlashCommand,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "/review\nand some notes");
    assert_eq!(s.cursor, "/review".len());
}

/// An accept with no live token under the caret must not touch the buffer.
#[test]
fn accepting_a_slash_command_with_no_token_never_clobbers_the_buffer() {
    let mut s = PromptInputState::new();
    s.text = "look at /rev".to_string();
    s.cursor = 0;
    s.suggestions = vec![TypeaheadSuggestion {
        text: "/review".to_string(),
        display: "/review".to_string(),
        description: "review".to_string(),
        source: TypeaheadSource::SlashCommand,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "look at /rev");
}

/// The popup opens on the `/command` TOKEN under the caret, not only when
/// the text starts with `/`.
#[test]
fn a_slash_command_after_prose_opens_the_popup() {
    let cmds = [("review", "Review")];
    let suggestions = compute_typeahead("look at /rev", &cmds, &UsageStore::default(), 15, false);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].text, "/review");
}

/// A PATH is not a command: a token with a second `/` never opens the
/// command popup.
#[test]
fn a_path_token_never_opens_the_command_popup() {
    let cmds = [("review", "Review")];
    for input in ["/usr/bin", "/usr/b", "see /usr/bin"] {
        assert!(
            compute_typeahead(input, &cmds, &UsageStore::default(), 15, false).is_empty(),
            "{input} must not offer slash commands"
        );
    }
}

/// The command name ends at the space: the popup closes once the caret
/// leaves the name.
#[test]
fn the_command_popup_closes_after_the_command_name() {
    let cmds = [("review", "Review")];
    assert!(
        compute_typeahead("/review ", &cmds, &UsageStore::default(), 15, false).is_empty(),
        "a completed command name closes the popup"
    );
}

// ---- history-nav fix (2026-10-25): recall must not open the popup ----

/// (2026-10-25 history-nav fix) THE operator defect: recalling a `/…`
/// entry re-opened the typeahead popup (update_suggestions saw the leading
/// slash), and the NEXT Up/Down then fed the popup instead of stepping
/// history — browsing was stranded in slash-completions. OMP's editor only
/// auto-triggers autocomplete on a TYPED trigger character; recall is
/// programmatic and must not. Mutation: delete the `suppress_suggestions_once`
/// arm in history_up and this test fails (the popup reopens).
#[test]
fn recalling_a_slash_entry_must_not_reopen_the_popup_and_browsing_continues() {
    let mut s = PromptInputState::new();
    // oldest-first: the first Up recalls the NEWEST ("real prompt"), the
    // second Up reaches the slash entry — THE defect case.
    s.history = vec!["/help".to_string(), "real prompt".to_string()];
    let cmds = [("help", "Show help")];
    let usage = UsageStore::default();

    s.history_up();
    assert_eq!(s.text, "real prompt");
    s.update_suggestions(
        &cmds,
        &usage,
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert!(s.suggestions.is_empty());

    // Second Up recalls "/help": the leading slash must not re-open the
    // typeahead popup, and browsing must keep stepping history.
    s.history_up();
    assert_eq!(s.text, "/help");
    s.update_suggestions(
        &cmds,
        &usage,
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert!(
        s.suggestions.is_empty(),
        "a recalled /help must not re-open the popup — that is what stranded browsing"
    );
    assert!(!s.suppress_suggestions_once, "the one-shot is consumed");
    assert_eq!(s.history_pos, Some(0), "still browsing — Up keeps control");

    // And the next genuinely typed character re-arms normal behaviour.
    s.clear();
    s.insert_char('/');
    s.update_suggestions(
        &cmds,
        &usage,
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert!(
        !s.suggestions.is_empty(),
        "typing / again must reopen the popup"
    );
}

/// history_down's exit-to-draft path also arms the suppress one-shot: Down
/// past the newest entry restores the draft, and if that draft begins with
/// '/' the popup must still not hijack the next key.
#[test]
fn exiting_history_to_a_slash_draft_must_not_reopen_the_popup() {
    let mut s = PromptInputState::new();
    s.history = vec!["a real prompt".to_string()];
    s.text = "/he".to_string();
    s.cursor = 3;
    let cmds = [("help", "Show help")];
    let usage = UsageStore::default();

    s.history_up();
    assert_eq!(s.text, "a real prompt");
    s.history_down();
    assert_eq!(s.text, "/he", "the draft is restored");
    s.update_suggestions(
        &cmds,
        &usage,
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert!(
        s.suggestions.is_empty(),
        "returning to a / draft via Down must not open the popup either"
    );
}

// ---- word_nav (plan 2026-10-24 M1: OMP word classes) ------------------

/// Helper: set text + cursor in one call.
fn nav_state(text: &str, cursor: usize) -> PromptInputState {
    let mut s = PromptInputState::new();
    s.text = text.to_string();
    s.cursor = cursor;
    s
}

#[test]
fn word_left_joins_apostrophes_and_hyphens() {
    // `don't stop` — Ctrl+Left from after `stop` stops before `stop`,
    // again before `don't` as ONE word (joiner binds), not before `don`.
    let mut s = nav_state("don't stop", 10);
    s.move_word_backward();
    assert_eq!(s.cursor, 6);
    s.move_word_backward();
    assert_eq!(s.cursor, 0, "apostrophe must keep don't one word");
}

#[test]
fn word_right_joins_hyphenated_words() {
    // well-known is one stop: Ctrl+Right from 0 consumes the joined word
    // AND the following space (readline-style), landing on `code`.
    let mut s = nav_state("well-known code", 0);
    s.move_word_forward();
    assert_eq!(s.cursor, 11, "well-known must move as one word");
    s.move_word_forward();
    assert_eq!(s.cursor, 15);
}

#[test]
fn word_left_treats_punctuation_runs_as_one_stop() {
    // `a -> b`: from after `b` the stops are before `b`, then before the
    // whole `->` run (one stop), then before `a`.
    let mut s = nav_state("a -> b", 6);
    s.move_word_backward();
    assert_eq!(s.cursor, 5);
    s.move_word_backward();
    assert_eq!(s.cursor, 2, "delimiter run -> is one stop");
    s.move_word_backward();
    assert_eq!(s.cursor, 0);
}

#[test]
fn word_left_over_cjk_run_is_one_stop() {
    // A run of Han characters is a single stop (OMP treats a same-script
    // run as one unit).
    let mut s = nav_state("fix 你好世界 now", "fix 你好世界 ".len());
    s.move_word_backward();
    assert_eq!(s.cursor, 4, "the whole Han run is one stop");
}

#[test]
fn word_left_skips_leading_joiner_without_left_word() {
    // A hyphen with no word char to its LEFT does not bind: `-x` from
    // after `x` stops before `x`, and the delimiter run `-` is its own
    // stop before that.
    let mut s = nav_state("-x", 2);
    s.move_word_backward();
    assert_eq!(s.cursor, 1);
    s.move_word_backward();
    assert_eq!(s.cursor, 0);
}

#[test]
fn word_left_skips_whitespace_before_stopping() {
    let mut s = nav_state("one two  ", 9);
    s.move_word_backward();
    assert_eq!(s.cursor, 4, "trailing spaces are skipped, stop before two");
}

#[test]
fn kill_word_backward_takes_joined_word_whole() {
    let mut s = nav_state("keep well-known", 15);
    s.kill_word_backward();
    assert_eq!(s.text, "keep ");
    assert_eq!(s.cursor, 5);
    assert_eq!(s.kill_ring.get_current(), Some("well-known"));
}

#[test]
fn delete_word_forward_stops_at_word_end_no_trailing_space() {
    let mut s = nav_state("one two", 0);
    s.delete_word_at_cursor();
    assert_eq!(s.text, " two", "word deleted, the following space stays");
}

#[test]
fn delete_word_forward_consumes_delimiter_run() {
    let mut s = nav_state("a :: b", 1);
    s.delete_word_at_cursor();
    assert_eq!(s.text, "a b", ":: and one space eaten as run + skip");
}

#[test]
fn move_word_forward_lands_on_next_word_start() {
    let mut s = nav_state("one  two", 0);
    s.move_word_forward();
    assert_eq!(
        s.cursor, 5,
        "readline-style: whitespace after word consumed"
    );
}

// ---- line-scoped Home/End (plan 2026-10-24 M1) ------------------------

#[test]
fn home_moves_to_line_start_not_buffer_start() {
    // Cursor mid-third-line: Home goes to the start of THAT line, not of
    // the buffer.
    let mut s = nav_state(
        "first line\nsecond line\nthird",
        "first line\nsecond line\nth".len(),
    );
    s.move_to_start();
    assert_eq!(
        s.cursor,
        "first line\nsecond line\n".len(),
        "Home is line-scoped on multi-line drafts"
    );
}

#[test]
fn home_on_first_line_goes_to_zero() {
    let mut s = nav_state("first line\nsecond", 4);
    s.move_to_start();
    assert_eq!(s.cursor, 0);
}

#[test]
fn end_moves_to_line_end_not_buffer_end() {
    let mut s = nav_state("first\nsecond line\nthird", "first\n".len() + 3);
    s.move_to_end();
    assert_eq!(
        s.cursor,
        "first\nsecond line".len(),
        "End is line-scoped on multi-line drafts"
    );
}

// ---- async fuzzy tier (plan 2026-10-24 M3) ----------------------------

#[test]
fn active_file_query_extracts_token_and_tail() {
    let q = active_file_query("check @src/ma").expect("query");
    assert_eq!(q.token, "@src/ma");
    assert_eq!(q.query, "ma", "the directory part rides the result paths");
    let flat = active_file_query("see @main").expect("flat");
    assert_eq!(flat.query, "main");
}

#[test]
fn active_file_query_listing_modes_return_none() {
    assert!(
        active_file_query("@").is_none(),
        "bare @: interim listing only"
    );
    assert!(
        active_file_query("@src/").is_none(),
        "trailing slash: listing"
    );
}

#[test]
fn active_file_query_boundary_and_slash_rules() {
    assert!(active_file_query("email@example").is_none(), "mid-word @");
    assert!(
        active_file_query("/help @x").is_none(),
        "slash-command input owns the popup"
    );
    assert!(active_file_query("one @two").is_some(), "space-separated @");
    assert!(active_file_query("@x").is_some(), "buffer-start @");
}

#[test]
fn update_suggestions_stages_the_fuzzy_request() {
    let mut s = nav_state("open @crates/ope", "open @crates/ope".len());
    let usage = UsageStore::default();
    s.update_suggestions(&[("help", "h")], &usage, 20, false);
    let pending = s.take_pending_file_query().expect("staged");
    assert_eq!(pending.token, "@crates/ope");
    assert_eq!(pending.query, "ope");
    assert!(s.take_pending_file_query().is_none(), "take is one-shot");
}

#[test]
fn replace_file_suggestions_keeps_interim_on_empty() {
    let mut s = nav_state("@src", 4);
    let usage = UsageStore::default();
    s.update_suggestions(&[], &usage, 20, false);
    let before = s.suggestions.clone();
    s.replace_file_suggestions(Vec::new());
    assert_eq!(
        s.suggestions, before,
        "empty fuzzy result must not blank the popup"
    );
}

#[test]
fn replace_file_suggestions_swaps_and_clamps_index() {
    let mut s = nav_state("@src", 4);
    s.suggestions = vec![
        TypeaheadSuggestion {
            text: "@a".into(),
            display: "@a".into(),
            description: String::new(),
            source: TypeaheadSource::FileRef,
        },
        TypeaheadSuggestion {
            text: "@b".into(),
            display: "@b".into(),
            description: String::new(),
            source: TypeaheadSource::FileRef,
        },
    ];
    s.suggestion_index = Some(1);
    s.replace_file_suggestions(vec![TypeaheadSuggestion {
        text: "@nested/deep/one.rs".into(),
        display: "@nested/deep/one.rs".into(),
        description: "file".into(),
        source: TypeaheadSource::FileRef,
    }]);
    assert_eq!(s.suggestions.len(), 1);
    assert_eq!(s.suggestion_index, Some(0), "index clamped to the new list");
}

// ---- @file:/@folder: grammar (plan 2026-10-24 M4) ----------------------

#[test]
fn file_ref_value_grammar_shapes() {
    assert_eq!(file_ref_value("src/main.rs", false), "@file:src/main.rs");
    assert_eq!(file_ref_value("crates", true), "@folder:crates/");
    // Paths with spaces quote; a directory quote stays OPEN (the drill-down
    // continues inside it) and a file quote closes — OMP's shape.
    assert_eq!(
        file_ref_value("my dir/a b.txt", false),
        "@file:\"my dir/a b.txt\""
    );
    assert_eq!(file_ref_value("my dir", true), "@folder:\"my dir/");
}

/// (2026-10-27, operator report) The interim directory listing builds its
/// rows as `name/` — `file_ref_value` used to append ANOTHER separator,
/// producing `@folder:crates//` on screen AND in the accepted token (which
/// then failed to round-trip as a path). The separator normalizes to
/// exactly one, OMP's `isDirectoryCompletionValue` semantics.
#[test]
fn file_ref_value_never_doubles_the_separator() {
    assert_eq!(
        file_ref_value("crates/", true),
        "@folder:crates/",
        "a directory already carrying its listing separator must not gain a second one"
    );
    assert_eq!(
        file_ref_value("my dir/", true),
        "@folder:\"my dir/",
        "the quoted form keeps exactly one separator inside the open quote"
    );
    assert_eq!(
        file_ref_value("docs///", true),
        "@folder:docs/",
        "pathological multi-separator input normalizes to one"
    );
}

/// (2026-10-27, operator report: "it shows a suffix: folder:, or file: which
/// takes up reading space") The popup row is the DISPLAY label — the bare
/// path, OMP `AutocompleteItem.label` — never the `@file:`/`@folder:`
/// insertion grammar. The grammar stays on `text`, which accept inserts.
#[test]
fn file_ref_display_label_is_the_bare_path() {
    assert_eq!(file_ref_display_label("crates", true), "crates/");
    assert_eq!(
        file_ref_display_label("crates/", true),
        "crates/",
        "exactly one trailing separator, same normalization as the value"
    );
    assert_eq!(file_ref_display_label("src/main.rs", false), "src/main.rs");
    assert_eq!(file_ref_display_label("docs///", true), "docs/");
    // The grammar NEVER appears in the display label.
    assert!(!file_ref_display_label("crates", true).contains('@'));
    assert!(!file_ref_display_label("crates", true).contains("folder:"));
    assert!(!file_ref_display_label("src/main.rs", false).contains("file:"));
}

#[test]
fn parse_file_ref_token_forms() {
    let bare = parse_file_ref_token("see @src/ma").expect("bare");
    assert_eq!((bare.raw, bare.path), ("@src/ma", "src/ma"));
    let file = parse_file_ref_token("see @file:src/ma").expect("file form");
    assert_eq!((file.raw, file.path), ("@file:src/ma", "src/ma"));
    let folder = parse_file_ref_token("see @folder:src/").expect("folder form");
    assert_eq!((folder.raw, folder.path), ("@folder:src/", "src/"));
    let quoted = parse_file_ref_token("see @file:\"a b").expect("quoted");
    assert_eq!(quoted.path, "a b");
    assert!(
        parse_file_ref_token("a@b").is_none(),
        "mid-word @ never triggers"
    );
}

/// The token terminates where the expander's grammar does —
/// `@kind:(quoted|\S+)` — and a popup token must END at the cursor: a
/// completed mention followed by prose leaves no live token. The old
/// unbounded token ran from the `@` to end-of-input, which kept the ghost
/// gate suppressed for the rest of any message that contained an `@`,
/// spawned a fuzzy walk on every later keystroke, and let a space-free
/// tail reopen the popup mid-prose where Enter accepts a file instead of
/// submitting.
///
/// MUTATION-PROOF: reverting parse_file_ref_token to the unbounded tail
/// (`raw = &input[at_idx..]`, no termination) fails all three `is_none`
/// assertions.
#[test]
fn parse_file_ref_token_terminates_at_whitespace_and_requires_a_live_token() {
    // The mention is complete and the cursor is in prose.
    assert!(parse_file_ref_token("use @file:src/main.rs and summar").is_none());
    assert!(parse_file_ref_token("see @src/ma and then prose").is_none());
    // A closed quoted value ends the token too.
    assert!(parse_file_ref_token("see @file:\"a b\" done").is_none());
    // Still live while the user is inside the mention.
    assert!(parse_file_ref_token("use @file:src/main.rs").is_some());
    assert!(parse_file_ref_token("see @src/ma").is_some());
    // An OPEN quote runs to the end of the input while it stays open —
    // that is what keeps a directory whose name contains a space drillable.
    let open = parse_file_ref_token("see @folder:\"my dir/").expect("open quote");
    assert_eq!(open.path, "my dir/");
    let closed = parse_file_ref_token("see @folder:\"my dir\"").expect("closed quote");
    assert_eq!(closed.raw, "@folder:\"my dir\"");
    assert_eq!(closed.path, "my dir");
}

/// An accepted directory chains as `@folder:<dir>/`; the immediate tier
/// must list THAT directory's children. The old immediate tier fed the raw
/// token to suggest_files verbatim — it statted a literal path named
/// `folder:…`, the popup blanked, and the fuzzy tier could not rescue it
/// because an empty tail after the last `/` disables the walk: the chained
/// browse flow was dead since the grammar shipped.
///
/// MUTATION-PROOF: reverting compute_file_suggestions to the raw-token
/// feed fails the non-empty assertion.
#[test]
fn compute_file_suggestions_relists_the_chained_directory() {
    let chained = compute_file_suggestions(
        "@folder:src/",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert!(
        !chained.is_empty(),
        "the chained directory must re-list its children"
    );
    for s in &chained {
        assert_eq!(s.source, TypeaheadSource::FileRef);
    }
    // The grammar is not part of the path: a bare typed directory lists
    // the same children.
    let bare = compute_file_suggestions(
        "@src/",
        TEST_FILE_AUTOCOMPLETE_LIMIT,
        TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN,
    );
    assert_eq!(
        chained.iter().map(|s| &s.text).collect::<Vec<_>>(),
        bare.iter().map(|s| &s.text).collect::<Vec<_>>(),
        "the kind prefix must not change what the immediate tier lists"
    );
}

#[test]
fn active_file_query_strips_kind_prefix() {
    let q = active_file_query("read @file:crates/ope").expect("query");
    assert_eq!(q.token, "@file:crates/ope");
    assert_eq!(
        q.query, "ope",
        "kind prefix must not leak into the fuzzy query"
    );
}

#[test]
fn accept_file_ref_appends_space_and_uses_grammar() {
    let mut s = nav_state("read @file:crates/ope", "read @file:crates/ope".len());
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@file:crates/operant/README.md".to_string(),
        display: "@file:crates/operant/README.md".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "read @file:crates/operant/README.md ");
    assert_eq!(s.cursor, s.text.len());
}

#[test]
fn accept_directory_chains_without_space() {
    let mut s = nav_state("read @crates/ope", "read @crates/ope".len());
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@folder:crates/operant/".to_string(),
        display: "@folder:crates/operant/".to_string(),
        description: "directory".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "read @folder:crates/operant/");
    assert_eq!(
        s.cursor,
        s.text.len(),
        "cursor before the drill-down listing"
    );
    // The chained token still parses — the popup re-lists the directory.
    assert!(s.has_active_file_ref());
}

#[test]
fn accept_preserves_text_after_the_cursor() {
    let mut s = nav_state("@crates tail", 7);
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@folder:crates/".to_string(),
        display: "@folder:crates/".to_string(),
        description: "directory".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "@folder:crates/ tail");
}

/// The ask-#2 acceptance gate: a token the popup inserts must be INJECTED
/// by the agent-side expander — `@file:<path>` inlines the file's content
/// into the effective message.
#[tokio::test]
async fn accepted_file_ref_token_injects_via_context_references() {
    let dir = std::env::temp_dir().join(format!(
        "operant-m4-inject-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("src")).expect("fixture");
    std::fs::write(dir.join("src/notes.txt"), "marker_line_for_injection").expect("fixture");
    let ctx = operant_core::context_references::preprocess_context_references(
        "read @file:src/notes.txt please",
        &dir,
        100_000,
    )
    .await;
    assert!(!ctx.blocked, "injection refused: {:?}", ctx.warnings);
    assert!(
        ctx.message.contains("marker_line_for_injection"),
        "the @file: token must inline the file content; got: {}",
        ctx.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- ghost text (plan 2026-10-24 M6) -----------------------------------

#[test]
fn ghost_query_gates() {
    let s = nav_state("read the par", 12);
    let q = s.ghost_query().expect("prose word at caret");
    assert_eq!(q, ("read the ".to_string(), "par".to_string()));

    // Mid-word: no ghost.
    let mid = nav_state("read the parser", 14);
    assert!(mid.ghost_query().is_none(), "caret mid-word");

    // Popup open: no ghost.
    let mut popup = nav_state("read the par", 12);
    popup.suggestions = vec![TypeaheadSuggestion {
        text: "@file:x".into(),
        display: "@file:x".into(),
        description: String::new(),
        source: TypeaheadSource::FileRef,
    }];
    assert!(popup.ghost_query().is_none(), "popup owns attention");

    // Inside an @ reference: no ghost.
    let atref = nav_state("read @file:src/ma", 16);
    assert!(atref.ghost_query().is_none(), "@ reference");

    // Slash command: no ghost.
    let slash = nav_state("/he", 3);
    assert!(slash.ghost_query().is_none(), "slash input");

    // Backtick span: no ghost.
    let code = nav_state("see `let x = par", 16);
    assert!(code.ghost_query().is_none(), "code span");

    // Path-ish token (non-alphabetic head): no ghost.
    let path = nav_state("open src/ma", 11);
    // 'ma' is alphabetic and preceded by '/'; v1 gate allows it only when
    // the run starts at a boundary — the leading '/' means the word is
    // 'ma' preceded by '/', which the @/slash gates above do not catch;
    // the engine's own prefix scoring handles it. This asserts the word
    // IS asked (documented v1 behavior).
    assert!(path.ghost_query().is_some());
}

#[test]
fn ghost_projection_keeps_the_word() {
    let mut s = nav_state("read the par", 12);
    s.show_ghost("par".into(), "ser".into());
    assert!(s.project_ghost("pars"), "still extends");
    let g = s.ghost.as_ref().expect("projected");
    assert_eq!((g.prefix.as_str(), g.suffix.as_str()), ("pars", "er"));
    assert!(!s.project_ghost("parx"), "divergence breaks it");
    assert!(s.ghost.is_none(), "divergent ghost dropped");
}

#[test]
fn ghost_accept_with_and_without_space() {
    let mut with = nav_state("read the par", 12);
    with.show_ghost("par".into(), "ser".into());
    assert!(with.accept_ghost(true));
    assert_eq!(with.text, "read the parser ");
    assert_eq!(with.cursor, with.text.len());

    let mut without = nav_state("read the par", 12);
    without.show_ghost("par".into(), "ser".into());
    assert!(without.accept_ghost(false));
    assert_eq!(without.text, "read the parser");
}

#[test]
fn ghost_accept_noop_without_ghost() {
    let mut s = nav_state("plain", 5);
    assert!(!s.accept_ghost(true));
    assert_eq!(s.text, "plain");
}

// ---- R48-4/R48-5: fold-length-drift and slice-order guards ----------------

/// `/k` in a buffer holding a KELVIN SIGN must land the cursor on an
/// original-text char boundary — the old search wrote lowercased-string
/// offsets into `self.cursor` and the next keypress's slice panicked.
#[test]
fn vim_search_forward_lands_on_an_original_boundary_after_folding() {
    let mut s = PromptInputState::new();
    s.text = "\u{212A}x".to_string(); // KELVIN SIGN (3B) + x
    s.cursor = s.text.len();
    s.vim_search_forward("k", 0);
    assert_eq!(s.cursor, 0, "the match is the KELVIN SIGN at the start");
    // the very next slice (ghost text, edits) must not panic
    let _ = &s.text[..s.cursor];
}

/// Backward search picks the last occurrence BEFORE the cursor, in original
/// coordinates.
#[test]
fn vim_search_backward_lands_on_an_original_boundary_after_folding() {
    let mut s = PromptInputState::new();
    s.text = "ab\u{212A}cd".to_string();
    s.cursor = s.text.len();
    s.vim_search_backward("k");
    assert_eq!(s.cursor, 2);
    let _ = &s.text[..s.cursor];
}

/// Escape's dot-repeat extraction must not slice backwards (start > end)
/// when in-insert edits moved the cursor; it records an empty repeat.
#[test]
fn vim_escape_dot_repeat_survives_a_backwards_cursor() {
    let mut s = PromptInputState::new();
    s.insert_char('h');
    s.insert_char('e');
    s.insert_char('l');
    s.insert_char('l');
    s.insert_char('o');
    // simulate: entered insert with "hello", then arrows+edits left the
    // cursor BEFORE the prefix length
    s.vim_insert_text_before = Some("hello".to_string());
    s.cursor = 1;
    s.vim_command("Escape"); // must not panic
    assert!(
        matches!(
            s.vim_dot_action,
            None | Some(DotRepeatAction::Insert { .. })
        ),
        "no panic, and any recorded action is a valid Insert"
    );
}

/// A multi-byte char under the extraction start must not panic either.
#[test]
fn vim_escape_dot_repeat_survives_a_mid_char_start() {
    let mut s = PromptInputState::new();
    s.insert_char('a');
    s.insert_char('b');
    s.vim_insert_text_before = Some("ab".to_string()); // 2 bytes
    s.cursor = 1; // inside/behind a multi-byte replacement's boundary
    s.vim_command("Escape"); // must not panic
}

/// The ghost is armed from refresh_prompt_input only, so cursor moves (which
/// run sync_legacy_prompt_fields without a refresh) leave it armed-but-
/// invisible; an accept then inserted the stale suffix at the moved caret —
/// 'read the par' + Left + Right produced 'read the paserr', and Home + TAB
/// injected the suffix at line start. The accept now validates that the
/// caret still sits at the ghost's prefix end and invalidates instead of
/// inserting; Right's existing move_right fallback keeps the key meaningful.
///
/// MUTATION-PROOF: deleting the ends_with validation in accept_ghost fails
/// both assertions (the stale suffix reappears in the text).
#[test]
fn a_stale_ghost_is_never_accepted_at_the_wrong_position() {
    let mut s = nav_state("read the par", "read the par".len());
    s.show_ghost("par".to_string(), "ser".to_string());
    s.move_left();
    assert_eq!(s.cursor, "read the par".len() - 1);
    // Right: accept must fail and fall back to the plain move.
    assert!(!s.accept_ghost(false), "a stale ghost refuses to accept");
    assert_eq!(
        s.text, "read the par",
        "no stale suffix may be inserted at the moved caret"
    );
    // Right's move_right fallback for the refused accept lives in the key
    // handler (key_handling.rs: `if !accept_ghost(false) { move_right() }`)
    // and is untouched here — the refusal is what this pins.
}

/// A suggestion accepted with no anchor `@` under the caret — reachable via
/// Home with a stale popup, which had no Esc dismissal either — used to
/// REPLACE THE WHOLE BUFFER with the bare suggestion value, destroying the
/// draft. It is nothing to apply: the text stays, the popup clears.
///
/// MUTATION-PROOF: restoring the old fallback (rewrite the buffer with
/// s.text + tail) fails the equality by name.
#[test]
fn accepting_a_suggestion_with_no_anchor_never_clobbers_the_buffer() {
    let mut s = nav_state("look at @src/ma", 15);
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@folder:src/".to_string(),
        display: "@folder:src/".to_string(),
        description: "directory".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.cursor = 0; // moved before the @ — no anchor under the caret
    s.accept_suggestion();
    assert_eq!(
        s.text, "look at @src/ma",
        "the draft must survive an anchor-less accept"
    );
    assert!(s.suggestions.is_empty(), "the popup still dismisses");
}

// ---- `$skill` mention typeahead (the dollar-sigil invocation) ------------

/// Register the mention snapshot for a test, holding the writer lock so a
/// parallel test that constructs `App` cannot replace it mid-assertion.
fn register_mention_skills(entries: &[(&str, &str)]) -> std::sync::MutexGuard<'static, ()> {
    let guard = super::typeahead::SKILL_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    super::typeahead::set_skill_mentions(
        entries
            .iter()
            .map(|(n, d)| (n.to_string(), d.to_string()))
            .collect(),
    );
    guard
}

/// A skill is findable by what it DOES, not only by its name.
///
/// The description is displayed beside every row already, so matching on name
/// alone was backwards: a caller who remembered "the one about PDF forms" had
/// to guess the name to find it. Measured before this fix: `$pdf` offered
/// nothing for a skill described as "fill and flatten PDF forms".
///
/// MUTATION-PROOF: removing the description arm from the filter fails the
/// first assertion.
#[test]
fn a_skill_mention_is_found_by_its_description() {
    let _guard = register_mention_skills(&[
        ("ntn-forms", "Fill and flatten PDF forms"),
        ("axi", "Agent CLI standards"),
    ]);
    let cmds: [(&str, &str); 0] = [];

    // The capability is in the description; the name shares no substring with
    // the query, so only a description match can find it.
    let s = compute_typeahead("apply $pdf", &cmds, &UsageStore::default(), 15, false);
    assert!(
        s.iter().any(|x| x.text == "$ntn-forms"),
        "a skill must be findable by what it does, not only by its name: {s:?}"
    );
    assert!(
        !s.iter().any(|x| x.text == "$axi"),
        "an unrelated skill is not offered: {s:?}"
    );

    // A NAME hit still sorts ahead of a description-only hit -- see
    // `a_skill_mention_name_hit_sorts_ahead_of_a_description_hit`, a separate
    // test because the snapshot lock is a plain `Mutex` and a second
    // `register_mention_skills` on this thread would self-deadlock.
}

/// A NAME hit sorts ahead of a description-only hit, so an exact name is never
/// buried under a description coincidence.
#[test]
fn a_skill_mention_name_hit_sorts_ahead_of_a_description_hit() {
    let _guard = register_mention_skills(&[
        ("pdf-fill", "Fill and flatten PDF forms"),
        ("ntn-forms", "Fill and flatten PDF forms"),
    ]);
    let cmds: [(&str, &str); 0] = [];
    let s = compute_typeahead("apply $pdf", &cmds, &UsageStore::default(), 15, false);
    let order: Vec<&str> = s.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(
        order,
        vec!["$pdf-fill", "$ntn-forms"],
        "a name hit must sort ahead of a description-only hit: {order:?}"
    );
}

#[test]
fn typeahead_skill_mentions_complete_mid_sentence() {
    let _guard = register_mention_skills(&[
        ("ponytail", "Lazy senior dev mode"),
        ("ponytail-audit", "Audit a repo"),
        ("axi", "Agent CLI standards"),
    ]);
    let cmds: [(&str, &str); 0] = [];

    // The composer asks with the text UP TO THE CURSOR, so the token is the
    // live tail — mid-sentence, with the instruction already typed after it.
    let s = compute_typeahead("apply $pony", &cmds, &UsageStore::default(), 15, false);
    assert!(s.iter().any(|x| x.text == "$ponytail"), "{s:?}");
    assert!(s.iter().any(|x| x.text == "$ponytail-audit"), "{s:?}");
    // A non-matching skill is not offered.
    assert!(!s.iter().any(|x| x.text == "$axi"), "{s:?}");
    // The row carries the skill's description.
    let row = s.iter().find(|x| x.text == "$ponytail").expect("the row");
    assert_eq!(row.description, "Lazy senior dev mode");

    // The popup follows the CURSOR: once the caret has moved past the token,
    // the mention is no longer being typed and nothing is offered.
    let s = compute_typeahead(
        "apply $pony to this",
        &cmds,
        &UsageStore::default(),
        15,
        false,
    );
    assert!(
        !s.iter().any(|x| x.text == "$ponytail"),
        "a mention the caret has left must not keep the popup open: {s:?}"
    );
}

#[test]
fn typeahead_never_offers_a_mention_for_shell_syntax() {
    let _guard = register_mention_skills(&[("home", "A skill that happens to be named home")]);
    let cmds: [(&str, &str); 0] = [];
    for input in ["echo $HOME", "costs $5", "user$name", "in ${VAR}"] {
        let s = compute_typeahead(input, &cmds, &UsageStore::default(), 15, false);
        assert!(s.is_empty(), "{input} offered {s:?}");
    }
}

#[test]
fn accepting_a_skill_mention_replaces_the_token_in_place() {
    let _guard = register_mention_skills(&[("ponytail", "Lazy senior dev mode")]);
    let mut s = PromptInputState::new();
    s.text = "apply $pony to this".to_string();
    s.cursor = "apply $pony".len();
    s.update_suggestions(&[], &UsageStore::default(), 15, false);
    assert_eq!(s.suggestions.len(), 1);
    s.accept_suggestion();
    // The text before the token and the tail after the cursor both survive.
    assert_eq!(s.text, "apply $ponytail to this");
    assert_eq!(s.cursor, "apply $ponytail".len());
}

#[test]
fn accepting_a_skill_mention_keeps_editing_rather_than_submitting() {
    let _guard = register_mention_skills(&[("ponytail", "Lazy senior dev mode")]);
    let mut s = PromptInputState::new();
    s.text = "$pony".to_string();
    s.cursor = s.text.len();
    s.update_suggestions(&[], &UsageStore::default(), 15, false);
    assert_eq!(
        s.accept_suggestion_for_submit(),
        AcceptForSubmitOutcome::ExtendInput
    );
    assert_eq!(s.text, "$ponytail ");
}

#[test]
fn the_name_tiers_offer_the_vendor_roots_too() {
    // Two pools: the operator's own, and a vendor (integration) root — the
    // shape `~/.operant/skills` + `~/.operant/integrations/ponytail-v*/skills`
    // has on this host. A vendor skill typed in full always resolved; the
    // popup used to omit it, which read as "not installed".
    let root = std::env::temp_dir().join(format!("operant-mention-pools-{}", std::process::id()));
    let pool = root.join("skills");
    let vendor = root.join("vendor");
    for (dir, name) in [(&pool, "alpha-skill"), (&vendor, "ponytail-review")] {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).expect("pool");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: The {name} skill.\n---\n\n# {name}\n"),
        )
        .expect("SKILL.md");
    }
    let guard = super::typeahead::SKILL_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    super::typeahead::set_invocable_skills(&pool, std::slice::from_ref(&vendor), Vec::new());

    // The `$` popup offers the vendor skill.
    let s = compute_typeahead("$pony", &[], &UsageStore::default(), 15, false);
    assert!(
        s.iter().any(|x| x.text == "$ponytail-review"),
        "the popup must offer the vendor skill, got: {s:?}"
    );
    // `/skill <Tab>` completes it too — ONE pool, two tiers.
    let s = compute_typeahead("/skill pony", &[], &UsageStore::default(), 15, false);
    assert!(
        s.iter().any(|x| x.text == "/skill ponytail-review"),
        "the slash tier must complete the vendor skill, got: {s:?}"
    );
    // The operator's own skill is there too.
    let s = compute_typeahead("$alpha", &[], &UsageStore::default(), 15, false);
    assert!(
        s.iter().any(|x| x.text == "$alpha-skill"),
        "the operator pool must still be offered, got: {s:?}"
    );

    drop(guard);
    std::fs::remove_dir_all(&root).ok();
}

// ---- RT-560: the ladder answers only while the composer holds the recall ----

/// (RT-560) An edit after a recall used to keep `history_pos` set, so the
/// NEXT Up/Down replaced the edit with the next history entry instead of
/// editing or moving the caret. Codex gates browsing on the recalled text
/// (`should_handle_navigation`); this is that rule.
#[test]
fn editing_a_recalled_entry_ends_browsing() {
    let mut s = PromptInputState::new();
    s.history = vec!["first prompt".to_string(), "second prompt".to_string()];

    s.history_up();
    assert_eq!(s.text, "second prompt");
    assert!(s.history_is_live(), "the recall is live");

    // The operator edits what they recalled: they are no longer browsing.
    s.text.push_str(" and a fix");
    s.cursor = s.text.len();
    assert!(
        !s.history_is_live(),
        "an edited recall is not browsing — the ladder must not own the next key"
    );

    // The edited recall is the draft the operator gets back when they walk
    // back out of history: the ladder's next call starts fresh from it.
    s.history_up();
    assert_eq!(
        s.text, "second prompt",
        "a dead ladder restarts from the newest entry"
    );
    assert_eq!(
        s.history_draft, "second prompt and a fix",
        "the edit is captured as the draft"
    );
    s.history_down();
    assert_eq!(
        s.text, "second prompt and a fix",
        "Down past the newest entry restores the edit"
    );
}

/// (RT-560) The mutation that matters: with the edit in the buffer, `Down` must
/// not silently swallow it.
#[test]
fn an_edited_recall_is_not_replaced_by_the_ladder() {
    let mut s = PromptInputState::new();
    s.history = vec!["one".to_string(), "two".to_string()];
    s.history_up();
    assert_eq!(s.text, "two");
    s.text = "two but different".to_string();
    s.cursor = s.text.len();
    assert!(!s.history_is_live());
    // Down past the end of the ladder restores the draft, and with no live
    // browse there is nothing to restore — the edit stays exactly as typed.
    s.history_down();
    assert_eq!(
        s.text, "two but different",
        "an edit after a recall is not swallowed by the history ladder"
    );
}

// ---- RT-561: the vertical move remembers its column ----

/// (RT-561) Walking up over a SHORT row used to clamp the caret to that row's
/// end and then keep it there: the column was recomputed from the caret every
/// move, so the memory of "I was at column 20" never survived the first
/// short row.
#[test]
fn a_vertical_walk_keeps_the_column_it_started_from() {
    // 3 rows of width 10: row 1 is short, so a recomputed column would drift.
    let text = "aaaaaaaaaa\nbb\ncccccccccc";
    let mut s = PromptInputState::new();
    s.text = text.to_string();
    // Put the caret at column 8 on the LAST row (index 2).
    let width = 10;
    s.cursor = "aaaaaaaaaa\nbb\n".len() + 8;
    let (_, col) = s.cursor_visual_pos(width);
    assert_eq!(col, 8, "the caret starts at display column 8");

    s.move_visual_up(width);
    let (_, middle_col) = s.cursor_visual_pos(width);
    assert_eq!(
        middle_col, 2,
        "the short row clamps to its own end (2 cells), it cannot invent columns"
    );

    s.move_visual_up(width);
    let (_, top_col) = s.cursor_visual_pos(width);
    assert_eq!(
        top_col, 8,
        "but the run remembers the column it started from and returns to it"
    );
}

/// (RT-561) A horizontal move, an insert, or Home/End ends the run, so the next
/// vertical move starts from the caret's NEW column.
#[test]
fn a_horizontal_change_restarts_the_saved_column() {
    let text = "aaaaaaaaaa\nbb\ncccccccccc";
    let mut s = PromptInputState::new();
    s.text = text.to_string();
    s.cursor = "aaaaaaaaaa\nbb\n".len() + 8;
    let width = 10;

    s.move_visual_up(width);
    s.move_visual_up(width);
    let (_, col) = s.cursor_visual_pos(width);
    assert_eq!(col, 8, "the run holds the column");

    // Any horizontal change drops it: the next vertical move starts fresh.
    s.move_to_start();
    assert_eq!(s.preferred_col, None, "Home ends the preferred-column run");
    s.move_visual_down(width);
    let (_, col) = s.cursor_visual_pos(width);
    assert_eq!(col, 0, "the next move starts from the caret's own column");
}

/// PROBE: does take() hand the MODEL a raw control byte? The composer never
/// holds one (sanitize), but take() expands the payload from the store, which
/// holds whatever the clipboard gave us.
#[test]
fn probe_take_can_hand_the_model_a_raw_control_byte() {
    let mut s = PromptInputState::new();
    s.paste("a\u{1b}b\nc\nd");
    let sent = s.take();
    eprintln!("PROBE sent={:?}", sent);
    assert!(
        !sent.contains('\u{1b}'),
        "the model must never receive a raw ESC, got {sent:?}"
    );
}

/// PROBE: the expand splice inserts RAW text, so the composer can hold a
/// control byte after an expand (the composer's own invariant is broken).
#[test]
fn probe_expand_breaks_the_composer_invariant() {
    let mut s = PromptInputState::new();
    s.paste("a\u{1b}b\nc\nd");
    s.paste("a\u{1b}b\nc\nd"); // expand
    eprintln!("PROBE text={:?}", s.text);
    assert!(
        !s.text.contains('\u{1b}'),
        "the composer must never hold a control byte"
    );
}

// ---- red team of iter-2266 (audit 2026-11-14) ------------------------
// Every test below was written as a probe against the shipped slice, failed,
// and now pins the fix. See
// docs/audit/2026-11-14-redteam-paste-collapse-expand-slice.md.

/// (audit finding 1) The expand must not reintroduce a byte the paste path
/// dropped. The store holds sanitised payloads, so a payload spliced back into
/// the buffer cannot put a raw ESC into a cell — the RT-539 defect class.
#[test]
fn expanding_a_paste_never_puts_a_control_byte_in_the_composer() {
    let mut s = PromptInputState::new();
    s.paste("a\u{1b}[31mRED\u{1b}[0m\nb\nc");
    let placeholder = s.pending_pastes[0].0.clone();
    assert!(s.text.contains(&placeholder), "collapsed");
    assert!(!s.text.contains('\u{1b}'), "the collapsed buffer is clean");
    // The store's payload is what the buffer would hold, which is the contract
    // the expand relies on. A payload that still carries the raw bytes is a
    // payload that can put them back.
    assert!(
        !s.pending_pastes[0].1.contains('\u{1b}'),
        "the stored payload must be sanitised, got {:?}",
        s.pending_pastes[0].1
    );

    // A store written by something that did NOT sanitise (a future writer, a
    // restore path) still cannot corrupt the buffer: the splice is guarded.
    s.pending_pastes[0].1 = "x\u{1b}[31mY\u{1b}[0m\nz\nw".to_string();
    s.expand_paste_placeholder(&placeholder);
    assert!(
        !s.text.contains('\u{1b}'),
        "the expand must splice clean text even from a dirty store, got {:?}",
        s.text
    );
    assert!(s.text.contains('Y'));
}

/// (audit finding 4) The store holds what the BUFFER holds: a tab is widened
/// on the way in, so it is widened in the store too. Otherwise the store and
/// the buffer disagree about the same fact.
#[test]
fn the_store_holds_the_sanitised_text_not_the_raw_clipboard() {
    let mut s = PromptInputState::new();
    s.paste("a\tb\nc\nd");
    let (_, stored) = &s.pending_pastes[0];
    assert_eq!(stored, "a    b\nc\nd", "the store carries the widened tab");
    assert_eq!(s.text, s.pending_pastes[0].0, "buffer holds only the token");
}

/// (audit finding 5) The outgoing prompt never carries a control byte, even
/// though the store could hold one if a future writer forgets to sanitise.
#[test]
fn the_prompt_the_model_receives_is_sanitised() {
    let mut s = PromptInputState::new();
    s.paste("a\u{1b}b\nc\nd");
    let sent = s.take();
    assert!(!sent.contains('\u{1b}'), "got {sent:?}");
    assert!(sent.contains("a"), "the text itself survives");
}

/// (audit finding 2) Ctrl+S twice must bring back a composer whose tokens
/// still have payloads behind them. This is the defect that sent the model a
/// literal `[Pasted ~5 lines]`.
#[test]
fn stashing_and_restoring_keeps_the_payloads() {
    let mut s = PromptInputState::new();
    let big = "1\n2\n3\n4\n5\n";
    s.paste(big);
    s.cursor = s.text.len();
    s.stash_input();
    assert!(s.pending_pastes.is_empty(), "parking empties the composer");
    assert!(s.restore_stash());
    assert_eq!(
        s.pending_pastes.len(),
        1,
        "the payload came back with the text"
    );
    let sent = s.take();
    assert_eq!(sent, big, "the model must get the paste, not the token");
}

/// (audit finding 3) Undo restores text and store together: an undo back
/// across a paste leaves no orphan payload behind.
#[test]
fn undoing_across_a_paste_leaves_no_orphan_payload() {
    let mut s = PromptInputState::new();
    s.text = "draft".to_string();
    s.cursor = 5;
    for _ in 0..8 {
        s.record_edit();
    }
    s.paste("1\n2\n3\n4\n5\n");
    // After EVERY undo step the store must hold only payloads whose token is in
    // the text. That is the invariant; which draft the undo lands on is a
    // separate question from whether the store lies.
    let mut steps = 0;
    while s.undo_burst() {
        steps += 1;
        for (placeholder, _) in &s.pending_pastes {
            assert!(
                s.text.contains(placeholder.as_str()),
                "after undo #{steps} the text {:?} does not hold its own placeholder {:?}",
                s.text,
                placeholder
            );
        }
        if steps > 8 {
            break;
        }
    }
    assert!(steps > 0, "undo must do something");
}

/// (audit finding 7) A clear ends every paste-scoped one-shot, so the next
/// paste's payload is not dropped by a stale suppress flag.
#[test]
fn clearing_the_composer_disarms_the_expand_one_shot() {
    let mut s = PromptInputState::new();
    s.paste("1\n2\n3\n4\n5\n");
    s.paste("1\n2\n3\n4\n5\n"); // expand: arms the one-shot
    assert!(s.suppress_paste_expand);
    s.clear();
    assert!(!s.suppress_paste_expand, "clear must disarm the one-shot");
    assert!(
        s.paste_expanded.is_none(),
        "and drop a pending expand notice"
    );
    assert!(s.paste_collapsed.is_none(), "and a pending collapse notice");
}

/// (audit finding 8) A paste is a non-vertical edit, so it ends the saved
/// column the way every other non-vertical primitive does.
#[test]
fn a_paste_clears_the_saved_column() {
    let text = "aaaaaaaaaa\nbb\ncccccccccc";
    let mut s = PromptInputState::new();
    s.text = text.to_string();
    s.cursor = "aaaaaaaaaa\nbb\n".len() + 8;
    let width = 10;
    s.move_visual_up(width);
    s.move_visual_up(width);
    assert_eq!(s.preferred_col, Some(8));
    s.paste("XY");
    assert_eq!(s.preferred_col, None, "a paste ends the column run");
}

/// (audit finding 9) One boundary rule for both the drop and the expand. A
/// hand-typed ` #2` after a bare token must read the same way to both.
#[test]
fn the_drop_and_expand_rules_agree_on_a_hand_typed_suffix() {
    // The expander sees the bare token in "[Pasted ~3 lines] #2" as live text
    // (the numbered token is a different, longer label); the drop rule must
    // agree that the STORE's bare token is still live, because the text really
    // does contain it as a token of its own.
    let text = "[Pasted ~3 lines] #2";
    assert!(
        paste::is_live_placeholder(text, "[Pasted ~3 lines]"),
        "the bare token IS present in that text"
    );
    assert!(
        !paste::is_live_placeholder(text, "[Pasted ~3 lines] #2 trailing"),
        "a token with trailing text is not a token"
    );
    // and the two entry points use the same predicate
    let mut s = PromptInputState::new();
    s.pending_pastes
        .push(("[Pasted ~3 lines]".to_string(), "a\nb\nc".to_string()));
    s.text = text.to_string();
    s.drop_missing_paste_placeholders();
    assert_eq!(
        s.pending_pastes.len(),
        1,
        "the drop rule kept it, as the expand rule would"
    );
}

/// (audit finding 6) The COLLAPSE announces itself, and says how to undo it.
#[test]
fn a_collapse_tells_the_operator_what_happened_and_how_to_expand() {
    let mut s = PromptInputState::new();
    s.paste("a\nb\nc\nd\ne\n");
    let notice = s.paste_collapsed.take().expect("a collapse must report");
    assert!(notice.contains("5 lines"), "got: {notice}");
    assert!(
        notice.contains("expand"),
        "the notice must say how to get the text back, got: {notice}"
    );
}
