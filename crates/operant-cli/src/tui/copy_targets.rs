//! Copy-target detection over RENDERED lines.
//!
//! Ported from jcode `crates/jcode-tui-markdown/src/markdown_render_support.rs:10-129`
//! and `crates/jcode-tui-markdown/src/markdown_types.rs:27-33`.
//!
//! # Why rendered shape, not source
//!
//! jcode runs this over the `Vec<Line>` a markdown block already rendered to.
//! That is the only place the frames, gutters and labels actually exist as
//! text, so detection needs no second parser and cannot disagree with what the
//! user sees. The corollary is that detection is *coupled to the renderer*:
//!
//! > **GUTTER-CHAR DEPENDENCY.** [FRAME_TOP], [FRAME_BOTTOM], [GUTTER] and
//! > [GUTTER_CHAR] below are literal bytes the detectors match on. If the
//! > markdown renderer changes its code-frame corners, its quote gutter, or
//! > indents the gutter even one column, every code block / blockquote silently
//! > stops being a copy target. There is no compile error and no test failure
//! > unless the renderer change lands in the same commit as the change to these
//! > constants. Change them together, and keep the shape tests at the bottom of
//! > this file in that commit.
//!
//! # Scope of this iteration
//!
//! The detection API and its shape tests are landed here; nothing renders a
//! copy badge yet, so [extract_copy_targets] has no production caller. See the
//! `#[expect(dead_code)]` on the module declaration in `tui/mod.rs`.
//!
//! # What is deliberately not a target
//!
//! Ordinary assistant prose. Whole-message copying already exists via the
//! context menu; a copy *badge* is for things you would otherwise retype, and
//! a badge on every paragraph would be noise. Only framed sub-blocks and
//! failure runs qualify.

use ratatui::text::Line;

/// Mnemonic keys handed to copy badges, in assignment order.
///
/// Ported verbatim from jcode `crates/jcode-tui/src/tui/ui.rs:557-559`. The
/// pool already excludes `h`/`j`/`k`/`l` so a badge can never shadow a
/// vi-style motion key; keep it that way if the pool is ever extended.
pub const COPY_BADGE_KEYS: [char; 12] =
    ['s', 'd', 'f', 'g', 'w', 'e', 'r', 't', 'x', 'c', 'v', 'b'];

/// Top-left corner of a rendered code frame, plus the space before the label.
const FRAME_TOP: &str = "┌─ ";
/// Bottom-left corner of a rendered code frame.
const FRAME_BOTTOM: &str = "└─";
/// One level of the blockquote gutter (nested quotes repeat it).
const GUTTER: &str = "│ ";
/// The gutter on its own - a quote line with no body text.
const GUTTER_CHAR: &str = "│";
/// The dim label a successfully rasterised formula renders above its image.
const MATH_LABEL: &str = "math";
/// Opening of the inline image placeholder a rasterised formula renders as.
///
/// jcode recovers the real placeholder through
/// `mermaid::parse_inline_image_placeholder`; operant has no such parser, so the
/// prefix is matched directly. It is the shape operant's own emitters already
/// produce - see `image_paste.rs`. The W2 markdown port of `jcode-tui-markdown`
/// (docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md §2) is the future renderer
/// whose rasterised-formula output must keep this `[image` prefix for this
/// constant to keep matching.
const IMAGE_PLACEHOLDER: &str = "[image";
/// The label a code frame carries when the fence declared no language. jcode
/// maps it to "no language" rather than to a language literally named `code`.
const UNLABELLED_CODE: &str = "code";

/// A line reads as a failure when it opens with one of these.
///
/// Ported from jcode `ui_prepare.rs:199-201` (`is_error_copy_content`).
const FAILURE_PREFIXES: [&str; 3] = ["Error:", "error:", "Failed:"];

/// What a copy target is, and the only thing that varies per kind: a code block
/// carries its language.
///
/// Ported from jcode `markdown_types.rs:27-33`. jcode's `Math` variant has a
/// `display: bool` recording whether the image actually rasterised; that flag
/// comes from a LaTeX hash registry operant does not have, so it is dropped
/// rather than carried as a field nothing can set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyTargetKind {
    /// A framed code block. `language` is `None` when the fence was unlabelled.
    CodeBlock { language: Option<String> },
    /// A blockquote run, with its gutter stripped.
    Blockquote,
    /// A `math` label plus the image placeholder beneath it.
    Math,
    /// A run that *opens* with a failure prefix.
    Error,
    /// A run that *contains* a failure prefix without opening with one - a tool
    /// result that failed partway, where the surrounding output is worth keeping.
    ToolOutput,
}

impl CopyTargetKind {
    /// The short noun a badge shows. Ported from jcode `markdown_types.rs:36-48`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::CodeBlock { language } => language
                .as_deref()
                .filter(|lang| !lang.is_empty())
                .unwrap_or("code")
                .to_string(),
            Self::Blockquote => "quote".to_string(),
            Self::Math => "math".to_string(),
            Self::Error => "error".to_string(),
            Self::ToolOutput => "output".to_string(),
        }
    }

    /// The confirmation a copy shows. Ported from jcode `markdown_types.rs:50-63`.
    #[must_use]
    pub fn copied_notice(&self) -> String {
        match self {
            Self::CodeBlock { language } => {
                let label = language
                    .as_deref()
                    .filter(|lang| !lang.is_empty())
                    .unwrap_or("code block");
                format!("Copied {label}")
            }
            Self::Blockquote => "Copied quote".to_string(),
            Self::Math => "Copied math".to_string(),
            Self::Error => "Copied error".to_string(),
            Self::ToolOutput => "Copied output".to_string(),
        }
    }
}

/// One copyable region, as spans of `lines` plus the text a copy yields.
///
/// `end_line` is exclusive, so `[start_line, end_line)` is the region. The
/// badge anchors to `badge_line` rather than `start_line` because a multi-line
/// code frame should hang its badge on the frame header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyTarget {
    /// What kind of block this is.
    pub kind: CopyTargetKind,
    /// The text a copy puts on the clipboard.
    pub content: String,
    /// First line of the region (inclusive).
    pub start_line: usize,
    /// Line after the region (exclusive).
    pub end_line: usize,
    /// Line the copy badge anchors to.
    pub badge_line: usize,
}

/// The plain text of one rendered line, ignoring styling.
///
/// Ported from jcode `markdown_render_support.rs:3-8` (`line_plain_text`).
#[must_use]
pub fn line_plain_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Every copy target in `lines`, in order.
///
/// Ported from jcode `markdown_render_support.rs:10-121`. Framed blocks are
/// tested before the failure run so a code block or quote is never swallowed by
/// an adjacent error message, and so the gutter bodies of a frame are not
/// mistaken for a blockquote gutter.
#[must_use]
pub fn extract_copy_targets(lines: &[Line<'_>]) -> Vec<CopyTarget> {
    let mut targets = Vec::new();
    let mut idx = 0usize;
    while idx < lines.len() {
        let found = math_target(lines, idx)
            .or_else(|| code_block_target(lines, idx))
            .or_else(|| blockquote_target(lines, idx))
            .or_else(|| failure_target(lines, idx));
        match found {
            Some((target, next)) => {
                targets.push(target);
                idx = next;
            }
            None => idx += 1,
        }
    }
    targets
}

// ---------------------------------------------------------------------------
// Detectors
// ---------------------------------------------------------------------------

/// A `math` label immediately followed by an inline image placeholder.
///
/// Ported from jcode `markdown_render_support.rs:21-41`.
fn math_target(lines: &[Line<'_>], idx: usize) -> Option<(CopyTarget, usize)> {
    if plain_at(lines, idx).trim() != MATH_LABEL {
        return None;
    }
    let placeholder = plain_at(lines, idx + 1);
    if !placeholder.trim_start().starts_with(IMAGE_PLACEHOLDER) {
        return None;
    }
    // ponytail: jcode recovers the LaTeX source - and the placeholder's true row
    // count - from a hash registry keyed by the marker, which is what lets its
    // `content` be the original markup rather than the on-screen image. Operant
    // has no such registry, so the target is the label plus the single
    // placeholder line and the copy is what is on screen. Upgrade path: add the
    // registry, then only `content` and the span change.
    let content = format!("{MATH_LABEL}\n{placeholder}");
    let end = idx + 2;
    Some((
        CopyTarget {
            kind: CopyTargetKind::Math,
            content,
            start_line: idx,
            end_line: end,
            badge_line: idx,
        },
        end,
    ))
}

/// A code-frame header, its gutter bodies, and the bottom corner that closes it.
///
/// Ported from jcode `markdown_render_support.rs:42-73`. An unterminated frame
/// runs to the end of the buffer, matching jcode: a truncated render should
/// still offer whatever body text survived.
fn code_block_target(lines: &[Line<'_>], idx: usize) -> Option<(CopyTarget, usize)> {
    let header = plain_at(lines, idx);
    let rest = header.trim_start().strip_prefix(FRAME_TOP)?;
    let label = rest.trim();
    let language = if label.is_empty() || label == UNLABELLED_CODE {
        None
    } else {
        Some(label.to_string())
    };

    let mut cursor = idx + 1;
    let mut content_lines: Vec<String> = Vec::new();
    while cursor < lines.len() {
        let line = plain_at(lines, cursor);
        let line = line.trim_start();
        if line.starts_with(FRAME_BOTTOM) {
            cursor += 1;
            break;
        }
        if let Some(code) = line.strip_prefix(GUTTER) {
            content_lines.push(code.to_string());
        }
        cursor += 1;
    }

    Some((
        CopyTarget {
            kind: CopyTargetKind::CodeBlock { language },
            content: content_lines.join("\n"),
            start_line: idx,
            end_line: cursor,
            badge_line: idx,
        },
        cursor,
    ))
}

/// A flush-left gutter run, bridged across the blank lines nested quotes emit.
///
/// Ported from jcode `markdown_render_support.rs:74-117`. The framing branches
/// above already consumed every gutter line that belongs to a code or math
/// frame, so any gutter line reached here belongs to a quote.
fn blockquote_target(lines: &[Line<'_>], idx: usize) -> Option<(CopyTarget, usize)> {
    if !is_blockquote_gutter_line(&plain_at(lines, idx)) {
        return None;
    }
    let mut cursor = idx;
    let mut content_lines: Vec<String> = Vec::new();
    while cursor < lines.len() {
        let text = plain_at(lines, cursor);
        if is_blockquote_gutter_line(&text) {
            content_lines.push(strip_blockquote_gutter(&text).to_string());
            cursor += 1;
            continue;
        }
        // A nested quote (or a multi-paragraph quote) renders a blank separator
        // between gutter runs. Bridge it when the run resumes, so one quote gets
        // one badge instead of one badge per paragraph.
        if text.trim().is_empty() {
            let mut probe = cursor + 1;
            while probe < lines.len() && plain_at(lines, probe).trim().is_empty() {
                probe += 1;
            }
            if probe < lines.len() && is_blockquote_gutter_line(&plain_at(lines, probe)) {
                for _ in cursor..probe {
                    content_lines.push(String::new());
                }
                cursor = probe;
                continue;
            }
        }
        break;
    }

    Some((
        CopyTarget {
            kind: CopyTargetKind::Blockquote,
            content: content_lines.join("\n"),
            start_line: idx,
            end_line: cursor,
            badge_line: idx,
        },
        cursor,
    ))
}

/// A run of non-blank lines containing at least one failure-prefixed line.
///
/// This is the one detector jcode does not have. jcode picks Error vs
/// ToolOutput from the *message role*, which survives markdown rendering
/// because it lives in the transcript, not in the `Vec<Line>`. Scanning
/// rendered lines throws that away, so the failure marker is the only evidence
/// left, and it is what this keys on:
///
/// - the run **opens** with a failure prefix -> `CopyTargetKind::Error`, a
///   standalone error message;
/// - a failure prefix appears **later** in the run -> `CopyTargetKind::ToolOutput`,
///   a tool result that failed partway, where the context around the failure is
///   what you want on the clipboard.
fn failure_target(lines: &[Line<'_>], idx: usize) -> Option<(CopyTarget, usize)> {
    let end = failure_run_end(lines, idx);
    if (idx..end).all(|i| !is_failure_line(&plain_at(lines, i))) {
        return None;
    }
    let kind = if is_failure_line(&plain_at(lines, idx)) {
        CopyTargetKind::Error
    } else {
        CopyTargetKind::ToolOutput
    };
    let content = (idx..end)
        .map(|i| plain_at(lines, i))
        .collect::<Vec<_>>()
        .join("\n");
    Some((
        CopyTarget {
            kind,
            content,
            start_line: idx,
            end_line: end,
            badge_line: idx,
        },
        end,
    ))
}

/// End of the contiguous non-blank run starting at `idx`.
///
/// Stops before a framed block so a failure message never grows to swallow the
/// code block or quote rendered after it. The `end > idx` guard means the
/// opening line is never rejected for being framed - the three framed detectors
/// have already declined it by the time this runs.
fn failure_run_end(lines: &[Line<'_>], idx: usize) -> usize {
    let mut end = idx;
    while end < lines.len() {
        let text = plain_at(lines, end);
        if text.trim().is_empty() {
            break;
        }
        if end > idx && (is_frame_line(&text) || is_blockquote_gutter_line(&text)) {
            break;
        }
        end += 1;
    }
    end
}

/// A rendered blockquote line: flush-left `GUTTER`, or a bare `GUTTER_CHAR`
/// with no body.
///
/// Ported from jcode `markdown_render_support.rs:124-130`. Requiring the gutter
/// to be flush-left is what keeps table rows out: a table pads its first cell,
/// so its separator always sits at least one column in, and its leading cell is
/// a space or a glyph rather than the gutter.
fn is_blockquote_gutter_line(text: &str) -> bool {
    // GUTTER-CHAR DEPENDENCY - see the module warning. Indenting this gutter by
    // one column silently turns every blockquote into an un-copyable paragraph.
    text.starts_with(GUTTER) || text.trim_end() == GUTTER_CHAR
}

/// Strip every leading gutter level from a rendered blockquote line.
///
/// Ported from jcode `markdown_render_support.rs:132-145`.
fn strip_blockquote_gutter(text: &str) -> &str {
    let mut rest = text;
    loop {
        if let Some(next) = rest.strip_prefix(GUTTER) {
            rest = next;
        } else if rest.trim_end() == GUTTER_CHAR {
            // GUTTER-CHAR DEPENDENCY - see the module warning.
            return "";
        } else {
            return rest;
        }
    }
}

/// Whether a rendered line opens or closes a code frame.
fn is_frame_line(text: &str) -> bool {
    let text = text.trim_start();
    // GUTTER-CHAR DEPENDENCY - see the module warning.
    text.starts_with(FRAME_TOP) || text.starts_with(FRAME_BOTTOM)
}

/// Whether a rendered line opens with a failure prefix.
fn is_failure_line(text: &str) -> bool {
    let trimmed = text.trim();
    FAILURE_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// Plain text of `lines[idx]`, or empty when out of range.
///
/// The detectors index one past the end on purpose - a truncated buffer is a
/// normal render state, not a bug to report.
fn plain_at(lines: &[Line<'_>], idx: usize) -> String {
    lines.get(idx).map_or_else(String::new, line_plain_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;
    use ratatui::text::Span;

    /// Render `texts` as bare lines, the way a markdown block hands them over.
    fn lines(texts: &[&str]) -> Vec<Line<'static>> {
        texts
            .iter()
            .map(|text| Line::from(Span::raw((*text).to_string())))
            .collect()
    }

    /// The (kind, content) of each target, for compact assertions.
    fn found(texts: &[&str]) -> Vec<(CopyTargetKind, String)> {
        extract_copy_targets(&lines(texts))
            .into_iter()
            .map(|target| (target.kind, target.content))
            .collect()
    }

    // -- CodeBlock ---------------------------------------------------------

    #[test]
    fn code_block_collects_gutter_body_until_the_bottom_fence() {
        let targets = found(&[
            "here is some code:",
            "┌─ rust",
            "│ fn main() {",
            "│     println!(\"hi\");",
            "│ }",
            "└─",
            "and that is all.",
        ]);
        assert_eq!(targets.len(), 1, "one code frame, one target");
        let (kind, content) = &targets[0];
        assert_eq!(
            *kind,
            CopyTargetKind::CodeBlock {
                language: Some("rust".to_string())
            }
        );
        assert_eq!(content, "fn main() {\n    println!(\"hi\");\n}");
    }

    #[test]
    fn code_block_spans_the_frame_and_badges_on_its_header() {
        let rendered = lines(&["┌─ rust", "│ x", "└─"]);
        let targets = extract_copy_targets(&rendered);
        assert_eq!(targets.len(), 1);
        let target = &targets[0];
        assert_eq!(target.start_line, 0);
        assert_eq!(target.end_line, 3, "end_line is exclusive, past the close");
        assert_eq!(target.badge_line, 0, "badge hangs on the frame header");
    }

    #[test]
    fn code_block_without_a_language_reports_none() {
        // jcode maps a bare `code` label to "no language", not to a language
        // literally called `code`.
        for header in ["┌─ code", "┌─ "] {
            let rendered = lines(&[header, "│ x", "└─"]);
            let targets = extract_copy_targets(&rendered);
            assert_eq!(
                targets[0].kind,
                CopyTargetKind::CodeBlock { language: None },
                "header {header:?} must not invent a language"
            );
        }
    }

    #[test]
    fn unterminated_code_frame_runs_to_the_end_of_the_buffer() {
        let targets = found(&["┌─ rust", "│ a", "│ b"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, "a\nb", "body survives a truncated render");
    }

    #[test]
    fn code_block_does_not_swallow_the_next_block() {
        let targets = found(&["┌─ rust", "│ a", "└─", "┌─ python", "│ b", "└─"]);
        assert_eq!(targets.len(), 2, "two frames, two targets");
        assert_eq!(targets[0].1, "a");
        assert_eq!(targets[1].1, "b");
    }

    // -- Blockquote ------------------------------------------------------

    #[test]
    fn blockquote_gutter_is_stripped() {
        let targets = found(&["│ quoted line", "plain prose after"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, CopyTargetKind::Blockquote);
        assert_eq!(targets[0].1, "quoted line");
    }

    #[test]
    fn nested_blockquote_strips_every_gutter_level() {
        let targets = found(&["│ │ deep"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, "deep", "both gutter levels come off");
    }

    #[test]
    fn blockquote_bridges_a_blank_line_between_gutter_runs() {
        let targets = found(&["│ first para", "", "│ second para", "prose"]);
        assert_eq!(targets.len(), 1, "one quote, one badge");
        assert_eq!(targets[0].1, "first para\n\nsecond para");
    }

    #[test]
    fn blockquote_ends_at_a_blank_line_the_run_does_not_resume_after() {
        let targets = found(&["│ quoted", "", "prose"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, "quoted", "trailing blanks are not bridged");
    }

    #[test]
    fn a_bare_gutter_line_is_a_quote_with_no_body() {
        let targets = found(&["│", "prose"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].1, "", "a bare gutter copies as empty");
    }

    #[test]
    fn table_row_with_a_mid_line_separator_is_not_a_blockquote() {
        // A table pads its first cell, so its separator is never flush-left.
        // This is the exclusion that keeps tables out.
        let targets = found(&[" name │ age", "  alice  │  30", "────────┼─────"]);
        assert!(
            targets.is_empty(),
            "a table must not become a blockquote, got {targets:?}"
        );
    }

    #[test]
    fn an_indented_gutter_is_not_a_quote() {
        // A frame body's gutter is indented by the frame. If the flush-left rule
        // were ever relaxed, this is the line that would start misfiring.
        let targets = found(&["  │ framed body"]);
        assert!(targets.is_empty(), "indented gutter, got {targets:?}");
    }

    // -- Math -----------------------------------------------------------

    #[test]
    fn math_label_plus_image_placeholder_is_one_target() {
        let targets = found(&["math", "[image: eq-1 120x40]", "prose"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, CopyTargetKind::Math);
        assert_eq!(targets[0].1, "math\n[image: eq-1 120x40]");
    }

    #[test]
    fn a_lone_math_label_is_not_a_target() {
        // No placeholder beneath it means the raster never happened; there is
        // nothing to copy back to the source.
        let targets = found(&["math", "prose"]);
        assert!(targets.is_empty(), "got {targets:?}");
    }

    // -- Error / ToolOutput ----------------------------------------------

    #[test]
    fn a_run_opening_with_a_failure_prefix_is_an_error() {
        let targets = found(&["Error: no such file", "  at src/main.rs:4", "", "prose"]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, CopyTargetKind::Error);
        assert_eq!(
            targets[0].1, "Error: no such file\n  at src/main.rs:4",
            "the whole stack trace copies, not just the first line"
        );
    }

    #[test]
    fn every_failure_prefix_is_recognised() {
        for prefix in FAILURE_PREFIXES {
            let rendered = lines(&[&format!("{prefix} boom")]);
            let targets = extract_copy_targets(&rendered);
            assert_eq!(
                targets[0].kind,
                CopyTargetKind::Error,
                "prefix {prefix:?} must be recognised"
            );
        }
    }

    #[test]
    fn a_failure_mid_run_is_tool_output_not_error() {
        let targets = found(&[
            "running 3 checks",
            "ok  lint",
            "Failed: 2 assertions",
            "ok  docs",
        ]);
        assert_eq!(targets.len(), 1, "one run, one target");
        assert_eq!(
            targets[0].0,
            CopyTargetKind::ToolOutput,
            "a run that does not open with the failure is tool output"
        );
        assert_eq!(
            targets[0].1, "running 3 checks\nok  lint\nFailed: 2 assertions\nok  docs",
            "the surrounding output is what makes the failure readable"
        );
    }

    #[test]
    fn a_failure_run_stops_before_the_next_framed_block() {
        let targets = found(&["Error: boom", "┌─ rust", "│ let x = 1;", "└─"]);
        assert_eq!(targets.len(), 2, "error and code block stay separate");
        assert_eq!(targets[0].0, CopyTargetKind::Error);
        assert_eq!(targets[0].1, "Error: boom", "the error stops at the frame");
        assert_eq!(targets[1].1, "let x = 1;");
    }

    #[test]
    fn a_failure_run_stops_before_the_next_blockquote() {
        let targets = found(&["Failed: step 2", "│ quoted"]);
        assert_eq!(targets.len(), 2, "error and quote stay separate");
        assert_eq!(targets[0].1, "Failed: step 2");
    }

    // -- Prose is not a target -------------------------------------------

    #[test]
    fn assistant_prose_is_never_a_copy_target() {
        let targets = found(&[
            "I looked at the renderer and the frame corners are wrong.",
            "Here is the fix.",
            "The second paragraph is ordinary prose too.",
        ]);
        assert!(
            targets.is_empty(),
            "whole assistant prose must not be a copy target, got {targets:?}"
        );
    }

    #[test]
    fn a_lone_math_word_in_prose_does_not_open_a_target() {
        // `math` as a whole trimmed line is the label shape; the word
        // mid-sentence is not.
        let targets = found(&["the math here is fine"]);
        assert!(targets.is_empty(), "got {targets:?}");
    }

    // -- Kind labels / notices -------------------------------------------

    #[test]
    fn labels_and_notices_match_jcode() {
        let cases = [
            (
                CopyTargetKind::CodeBlock {
                    language: Some("rust".to_string()),
                },
                "rust",
                "Copied rust",
            ),
            (
                CopyTargetKind::CodeBlock { language: None },
                "code",
                "Copied code block",
            ),
            (CopyTargetKind::Blockquote, "quote", "Copied quote"),
            (CopyTargetKind::Math, "math", "Copied math"),
            (CopyTargetKind::Error, "error", "Copied error"),
            (CopyTargetKind::ToolOutput, "output", "Copied output"),
        ];
        for (kind, label, notice) in cases {
            assert_eq!(kind.label(), label, "label for {kind:?}");
            assert_eq!(kind.copied_notice(), notice, "notice for {kind:?}");
        }
    }

    #[test]
    fn an_empty_language_label_falls_back_to_code() {
        let kind = CopyTargetKind::CodeBlock {
            language: Some(String::new()),
        };
        assert_eq!(kind.label(), "code");
        assert_eq!(kind.copied_notice(), "Copied code block");
    }

    // -- Badge mnemonics -------------------------------------------------

    #[test]
    fn badge_mnemonics_avoid_vi_motion_keys() {
        for banned in ['h', 'j', 'k', 'l'] {
            assert!(
                !COPY_BADGE_KEYS.contains(&banned),
                "a copy badge must never take {banned:?} - it would shadow vi motion"
            );
        }
    }

    #[test]
    fn badge_mnemonics_are_distinct_so_one_frame_per_key() {
        let mut seen = COPY_BADGE_KEYS.to_vec();
        seen.sort_unstable();
        let unique = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), unique, "duplicate mnemonic in the pool");
    }

    // -- Line text -------------------------------------------------------

    #[test]
    fn line_plain_text_concatenates_spans() {
        let line = Line::from(vec![
            Span::raw("┌─ "),
            Span::styled("rust", Style::default()),
        ]);
        assert_eq!(line_plain_text(&line), "┌─ rust");
    }

    #[test]
    fn an_empty_buffer_yields_no_targets() {
        assert!(extract_copy_targets(&[]).is_empty());
    }
}
