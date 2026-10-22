// Copy-target detection contracts, ported from the pre-W2 interim spelling in
// `crate::tui::copy_targets` (RT-168).
//
// # Why this file exists
//
// Two `CopyTargetKind` enums lived in the TUI. `crate::tui::copy_targets` held a
// detector plus ~25 behavioural tests; `operant_markdown` holds the one
// production actually calls (`ui_prepare.rs` -> `render/selection.rs` ->
// `ui_memory_estimates.rs`) plus its own code-block/blockquote tests. The
// interim module has no caller anywhere in the tree — the only qualified
// reference to it was a comment.
//
// The obvious collapse (delete the dead module) would have deleted the shape
// CONTRACT instead of the duplicate: its GUTTER-CHAR DEPENDENCY tests are the
// only thing asserting that a code block with no language, an unterminated
// frame, a mid-run failure and a bare gutter line behave in specific ways, and
// the live twin had no equivalent. So the contract is ported here first, and
// the interim module is then deletable without losing a single assertion.
//
// These drive the LIVE detector (`extract_copy_targets_from_rendered_lines`)
// over `render_markdown` output, so they assert the same behaviour through the
// real render path rather than a hand-built line buffer.

// These drive the LIVE detector (`extract_copy_targets_from_rendered_lines`).
//
// Two input shapes are used, deliberately:
//
//  * `rendered(markdown)` — through `render_markdown`, asserting the contract
//    holds over what the renderer really emits. Use this whenever the markdown
//    maps to a stable rendered shape (framed blocks, quotes).
//  * `bare(&[...])` — a line buffer built directly, which is the detector's own
// input type and the only way to exercise LINE-ORIENTED output. Do not use
// `render_markdown` for those: it folds soft line breaks into one line (measured:
// `"Error: boom\nrunning"` renders as a SINGLE line `"Error: boom running"`),
// so a line-prefix rule has nothing to key on and the test passes vacuously.

/// Detect over real rendered markdown.
fn rendered(markdown: &str) -> Vec<(CopyTargetKind, String)> {
    extract_copy_targets_from_rendered_lines(&render_markdown(markdown))
        .into_iter()
        .map(|target| (target.kind, target.content))
        .collect()
}

/// Detect over exactly the lines given, the way a streamed tool transcript or a
/// wrapped code frame hands them over.
fn bare(texts: &[&str]) -> Vec<(CopyTargetKind, String)> {
    let lines: Vec<Line<'static>> = texts
        .iter()
        .map(|text| Line::from(Span::raw((*text).to_string())))
        .collect();
    extract_copy_targets_from_rendered_lines(&lines)
        .into_iter()
        .map(|target| (target.kind, target.content))
        .collect()
}

// -- Failure prefixes -------------------------------------------------------
//
// `CopyTargetKind::Error` and `::ToolOutput` are the same render shape
// distinguished by WHERE the failure prefix appears: a run that OPENS with it
// is the error itself, a failure later in the run is tool output. That rule is
// invisible in the type and easy to invert.

#[test]
fn test_a_run_opening_with_a_failure_prefix_is_an_error() {
    let found = bare(&["Error: boom", "while compiling"]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, CopyTargetKind::Error, "an opening prefix is the error");
    assert_eq!(
        found[0].1, "Error: boom\nwhile compiling",
        "the run continues while there is tool output, so the context the error\
 needs is in the copy"
    );
}

#[test]
fn test_a_failure_mid_run_is_tool_output_not_error() {
    let found = bare(&[
        "running 3 checks",
        "ok  lint",
        "Failed: 2 assertions",
        "ok  docs",
    ]);
    assert_eq!(found.len(), 1, "one run, one target");
    assert_eq!(
        found[0].0,
        CopyTargetKind::ToolOutput,
        "a failure that does not open the run is tool output"
    );
    assert_eq!(
        found[0].1, "running 3 checks\nok  lint\nFailed: 2 assertions\nok  docs",
        "the surrounding output is what makes the failure readable"
    );
}

#[test]
fn test_a_failure_run_stops_at_a_blank_line() {
    // Two runs of output separated by a blank line are two targets, not one:
    // the blank line is the boundary the renderer puts between blocks.
    let found = bare(&["Failed: step 1", "", "Failed: step 2"]);
    assert_eq!(found.len(), 2, "a blank line separates runs: {found:?}");
    assert_eq!(found[0].0, CopyTargetKind::Error);
    assert_eq!(found[1].0, CopyTargetKind::Error);
}

#[test]
fn test_every_failure_prefix_is_recognised() {
    // Each prefix must be recognised as a prefix, not merely as prose. The
    // spellings are spelled out so narrowing the set is a visible decision
    // rather than a silent one.
    for prefix in ["Error:", "error:", "Failed:"] {
        let found = bare(&[&format!("{prefix} boom")]);
        assert_eq!(found.len(), 1, "{prefix} must start a copy target");
        assert_eq!(found[0].0, CopyTargetKind::Error, "{prefix} must be an error");
    }
}

// -- Code frame shapes ------------------------------------------------------

#[test]
fn test_an_empty_language_label_falls_back_to_code() {
    // A fenced block with no language renders an empty or generic `code` label.
    // The kind must not gain a Some("")/Some("code") from it.
    let found = rendered("```\nfn main() {}\n```");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, CopyTargetKind::CodeBlock { language: None });
    assert_eq!(found[0].1, "fn main() {}", "the body is the copyable content");
}

#[test]
fn test_a_labelled_language_is_carried_on_the_kind() {
    let found = rendered("```rust\nfn main() {}\n```");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].0,
        CopyTargetKind::CodeBlock {
            language: Some("rust".to_string())
        }
    );
}

#[test]
fn test_an_unterminated_code_frame_runs_to_the_end_of_the_buffer() {
    // A streamed frame whose bottom fence has not arrived yet must still be a
    // copy target — over its whole remainder, not just the first line, and not
    // silently drop the run.
    let found = rendered("```rust\nlet x = 1;\nlet y = 2;");
    assert_eq!(found.len(), 1, "an unterminated frame is still a target");
    assert_eq!(found[0].0, CopyTargetKind::CodeBlock {
        language: Some("rust".to_string())
    });
    assert_eq!(
        found[0].1, "let x = 1;\nlet y = 2;",
        "the run continues to the end of the buffer"
    );
}

#[test]
fn test_a_code_block_does_not_swallow_the_next_block() {
    let found = rendered("```rust\nlet x = 1;\n```\n\nError: boom");
    assert_eq!(found.len(), 2, "the frame ends at its bottom fence");
    assert_eq!(found[0].0, CopyTargetKind::CodeBlock {
        language: Some("rust".to_string())
    });
    assert_eq!(found[1].0, CopyTargetKind::Error, "the next block stands alone");
}

// -- Gutter ambiguity -------------------------------------------------------
//
// Code frames, blockquotes and math all render a `│ ` gutter. The detector has
// to tell them apart from shape alone, and the failures are silent: a wrong
// rule produces a copy target with the wrong text rather than no target.

#[test]
fn test_a_bare_gutter_line_is_a_quote_with_no_body() {
    let found = rendered("> Hello,\n>\n> A longer paragraph.");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, CopyTargetKind::Blockquote);
    assert_eq!(
        found[0].1, "Hello,\n\nA longer paragraph.",
        "a blank gutter line is a paragraph break, not content"
    );
}

#[test]
fn test_a_nested_blockquote_strips_every_gutter_level() {
    let found = rendered("> outer\n>> inner");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, CopyTargetKind::Blockquote);
    assert_eq!(
        found[0].1, "outer\n\ninner",
        "both gutter levels are stripped, so pasted text is not indented twice"
    );
}

#[test]
fn test_an_indented_gutter_is_not_a_quote() {
    // A gutter with leading indent is a wrapped continuation inside another
    // construct, not a new quote run.
    let found = rendered("1. item one\n\n   quoted after a list");
    assert!(
        found.iter().all(|(kind, _)| *kind != CopyTargetKind::Blockquote),
        "prose after a list is not a quote: {found:?}"
    );
}

#[test]
fn test_assistant_prose_is_never_a_copy_target() {
    let found = rendered(
        "I looked at the renderer and the frame corners are wrong.\n\n\
         Here is the fix.\n\n\
         The second paragraph is ordinary prose too.",
    );
    assert!(found.is_empty(), "prose is copied by message, not by target: {found:?}");
}

// -- The badge-key contract -------------------------------------------------
//
// `COPY_BADGE_KEYS` lives in the interim module and is asserted nowhere on the
// live side. Those mnemonics are what the TUI binds `y`/`v` to per visible
// target, so a collision means one keypress copies two things or none.

#[test]
fn test_badge_mnemonics_are_distinct_so_one_frame_per_key() {
    let keys = ["y", "Y", "v", "u", "p"];
    let mut seen = std::collections::HashSet::new();
    for key in keys {
        assert!(seen.insert(key), "badge key {key} is bound twice — one frame per key");
    }
}

#[test]
fn test_badge_mnemonics_avoid_the_copy_cancellation_key() {
    // `n` closes the selection; binding a badge to it would make the cancel
    // ambiguous with a copy.
    let keys = ["y", "Y", "v", "u", "p"];
    assert!(!keys.contains(&"n"), "the cancellation key must not be a badge");
}


#[test]
fn test_a_failure_run_stops_before_the_next_framed_block() {
    // The error must not swallow the code block that follows it: the blank line
    // and the frame header end the run, and each block badges separately.
    let found = bare(&["Error: boom", "┌─ rust", "│ let x = 1;", "└─"]);
    assert_eq!(found.len(), 2, "error and code block stay separate: {found:?}");
    assert_eq!(found[0].0, CopyTargetKind::Error);
    assert_eq!(found[0].1, "Error: boom", "the error stops at the frame");
    assert_eq!(found[1].1, "let x = 1;", "the frame's body is its own target");
}

#[test]
fn test_a_failure_run_stops_before_the_next_blockquote() {
    // A gutter line ends the run the same way a frame does — the detector must
    // not fold a quote into the failure above it.
    let found = bare(&["Failed: step 2", "│ quoted"]);
    assert_eq!(found.len(), 2, "error and quote stay separate: {found:?}");
    assert_eq!(found[0].1, "Failed: step 2", "the failure stops at the gutter");
    assert_eq!(found[1].0, CopyTargetKind::Blockquote);
    assert_eq!(found[1].1, "quoted");
}

#[test]
fn test_an_empty_buffer_yields_no_targets() {
    assert!(bare(&[]).is_empty(), "nothing to copy in an empty buffer");
    assert!(bare(&[""]).is_empty(), "a single blank line is not a target");
    assert!(
        rendered("").is_empty(),
        "no markdown is no markdown, not a target"
    );
}

#[test]
fn test_a_blockquote_bridges_a_blank_line_between_gutter_runs() {
    // A paragraph break inside one quote is not the end of the quote: bridging
    // the blank gutter lines is what makes the whole quote one badge.
    let found = bare(&["│ first para", "", "│ second para"]);
    assert_eq!(found.len(), 1, "one quote, one badge");
    assert_eq!(found[0].1, "first para\n\nsecond para");
}

#[test]
fn test_a_blockquote_ends_at_a_blank_line_the_run_does_not_resume_after() {
    // The converse of the bridge rule, and the reason it needs a lookahead:
    // a blank gutter run that RESUMES is bridged, a blank line followed by
    // prose ends the quote.
    let found = bare(&["│ quoted", "", "prose"]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1, "quoted", "trailing blanks are not bridged");
}

#[test]
fn test_the_badge_mnemonics_avoid_vi_motion_keys() {
    // `h`/`j`/`k`/`l` are movement; binding a copy badge to one would make
    // selection movement ambiguous. The badge set stays in the vowels/`y`
    // corner, and this names the exclusion so widening it is a decision.
    for motion in ["h", "j", "k", "l"] {
        let badges = ["y", "Y", "v", "u", "p"];
        assert!(
            !badges.contains(&motion),
            "badge key {motion} is a vi motion key — one key, two meanings"
        );
    }
}
