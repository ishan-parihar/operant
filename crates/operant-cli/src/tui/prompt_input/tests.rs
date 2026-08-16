// prompt_input/tests.rs — Unit tests for prompt input state + rendering.
//
// Extracted from the prompt_input/mod.rs monolith.

// vim-motion test names mirror key names (motion_W_B_basic, etc.)
#![allow(non_snake_case)]

use super::typeahead::{compute_file_suggestions, compute_slash_suggestions};
use super::vim::{
    VimOperator, motion_B, motion_E, motion_G, motion_W, motion_b, motion_e, motion_find_char,
    motion_first_nonblank, motion_gg, motion_w,
};
use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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
fn render_cursor_after_wide_character() {
    let mut s = PromptInputState::new();
    s.text = "你a".to_string();
    s.cursor = "你".len();

    let area = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 4,
    };
    let mut buf = Buffer::empty(area);
    render_prompt_input(
        &s,
        area,
        &mut buf,
        true,
        InputMode::Default,
        Color::Blue,
        false,
    );

    // iter-121: cursor now uses reverse video instead of solid block.
    // The character at the cursor position ('a') should still be visible
    // (not replaced by █). We check that the cell contains 'a' and has
    // reverse-video styling (black fg, white bg).
    let cell = &buf[(4, 1)];
    assert_eq!(cell.symbol(), "a");
    assert_eq!(cell.fg, Color::Black);
    assert_eq!(cell.bg, Color::White);
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

// ---- handle_paste ---------------------------------------------------

#[test]
fn paste_small_content_inline() {
    let mut counter = 0u32;
    let (result, stored) = handle_paste("short text", &mut counter);
    assert_eq!(result, "short text");
    assert!(stored.is_none());
    assert_eq!(counter, 0);
}

#[test]
fn paste_large_content_placeholder() {
    let mut counter = 0u32;
    // >150 chars → triggers placeholder
    let big = "x".repeat(200);
    let (result, stored) = handle_paste(&big, &mut counter);
    assert!(
        result.starts_with("[Pasted ~"),
        "expected placeholder, got: {result}"
    );
    assert!(
        result.contains("#1"),
        "expected counter in placeholder, got: {result}"
    );
    assert!(stored.is_some());
    assert_eq!(counter, 1);
}

#[test]
fn paste_large_multiline_placeholder() {
    let mut counter = 0u32;
    // ≥3 lines → triggers placeholder regardless of length
    let big = "line\n".repeat(300);
    let (result, stored) = handle_paste(&big, &mut counter);
    assert!(
        result.starts_with("[Pasted ~"),
        "expected placeholder, got: {result}"
    );
    assert!(
        result.contains("lines"),
        "expected line count in placeholder, got: {result}"
    );
    assert!(stored.is_some());
}

#[test]
fn paste_three_lines_triggers_placeholder() {
    let mut counter = 0u32;
    // Exactly 3 lines (the threshold) should use a placeholder.
    let three_lines = "a\nb\nc";
    let (result, stored) = handle_paste(three_lines, &mut counter);
    assert!(
        result.starts_with("[Pasted ~"),
        "3-line paste should be placeholder, got: {result}"
    );
    assert!(stored.is_some());
}

#[test]
fn paste_two_lines_inline() {
    let mut counter = 0u32;
    // 2 lines, ≤150 chars → inserted verbatim
    let two_lines = "hello\nworld";
    let (result, stored) = handle_paste(two_lines, &mut counter);
    assert_eq!(result, two_lines);
    assert!(stored.is_none());
}

#[test]
fn paste_counter_increments() {
    let mut counter = 0u32;
    let big = "x".repeat(2000);
    handle_paste(&big, &mut counter);
    handle_paste(&big, &mut counter);
    assert_eq!(counter, 2);
}

// ---- compute_typeahead ---------------------------------------------

// Helper constants for tests
const TEST_FILE_AUTOCOMPLETE_LIMIT: usize = 15;
const TEST_FILE_AUTOCOMPLETE_SHOW_HIDDEN: bool = false;

#[test]
fn typeahead_slash_prefix_matches() {
    let cmds = [
        ("help", "Show help"),
        ("history", "Show history"),
        ("compact", "Compact"),
    ];
    let suggestions = compute_slash_suggestions("/h", &cmds);
    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0].text, "/help");
    assert_eq!(suggestions[1].text, "/history");
}

#[test]
fn typeahead_full_match() {
    let cmds = [("compact", "Compact conversation")];
    let suggestions = compute_slash_suggestions("/compact", &cmds);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].text, "/compact");
    assert_eq!(suggestions[0].description, "Compact conversation");
}

#[test]
fn typeahead_case_insensitive() {
    let cmds = [("Help", "Show help")];
    let suggestions = compute_slash_suggestions("/H", &cmds);
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
    let s = compute_typeahead("/skill git", &cmds, 15, false);
    assert!(s.iter().any(|x| x.text == "/skill gitcrawl"));

    // /bundle <prefix> completes bundle names too.
    let s = compute_typeahead("/bundle rese", &cmds, 15, false);
    assert!(s.iter().any(|x| x.text == "/bundle research-pack"));

    // No match → empty (falls through to normal slash suggestions).
    let s = compute_typeahead("/skill zzz", &cmds, 15, false);
    assert!(s.is_empty());

    // Regular slash commands still work after the skill prefix handling.
    let cmds2 = [("help", "Show help")];
    let s = compute_typeahead("/he", &cmds2, 15, false);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].text, "/help");
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
    s.update_suggestions(&cmds, 15, false);
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
    s.update_suggestions(&cmds, 15, false);
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
    let suggestions = compute_slash_suggestions("/he", &cmds);
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

// ---- accept_suggestion FileRef tests ------------------------------------

#[test]
fn accept_suggestion_file_ref_at_start() {
    let mut s = PromptInputState::new();
    s.text = "@src/ma".to_string();
    s.cursor = s.text.len();
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@src/main.rs".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "@src/main.rs");
    assert_eq!(s.cursor, "@src/main.rs".len());
    assert!(s.suggestions.is_empty());
}

#[test]
fn accept_suggestion_file_ref_after_text_preserves_prefix() {
    let mut s = PromptInputState::new();
    s.text = "some text @src/ma".to_string();
    s.cursor = s.text.len();
    s.suggestions = vec![TypeaheadSuggestion {
        text: "@src/main.rs".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "some text @src/main.rs");
    assert_eq!(s.cursor, "some text @src/main.rs".len());
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
        text: "@src/main.rs".to_string(),
        description: "file".to_string(),
        source: TypeaheadSource::FileRef,
    }];
    s.suggestion_index = Some(0);
    s.accept_suggestion();
    assert_eq!(s.text, "@src/main.rs more text");
    assert_eq!(s.cursor, "@src/main.rs".len());
}
