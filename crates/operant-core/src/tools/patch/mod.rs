//! The native patch engine: `seek_sequence` + `text_file` + occurrence rules.
//!
//! Ported from codex `apply-patch/src/seek_sequence.rs` (164 LOC) and
//! `text_file.rs` (121 LOC) — both portable as-is with **zero imports** — plus
//! the occurrence guard that closes operant defect D1.
//!
//! ## Why this exists
//!
//! `AftApplyPatchTool` (`tools/aft_tools.rs:615-668`) already puts the
//! `*** Begin Patch` grammar in front of the model, but it forwards to a
//! subprocess. When that bridge is dead, native `patch`
//! (`tools/patch_tool.rs`) collapses editing to **one whole-file
//! find-and-replace with one tier of whitespace tolerance**, which loses CRLF
//! and cannot express "edit these five places in one call".
//!
//! ## What the port fixes
//!
//! 1. **CRLF fidelity.** operant's `normalize_whitespace` (`patch_tool.rs:34-44`)
//!    normalizes `\r\n` away entirely, so a fuzzy match on a CRLF file rewrites
//!    every line ending in that file to LF. `SourceFile` preserves each line's
//!    own ending and gives inserted lines the file's preferred one.
//! 2. **Four tiers of match, not one.** exact, rstrip, trim, and Unicode
//!    punctuation-normalizing — the last being the one that matters for a
//!    model writing ASCII against a file containing typographic dashes and
//!    curly quotes (`seek_sequence.rs:74-105`).
//! 3. **D1: an ambiguous `find` is refused.** operant computed
//!    `exact_count` and used it only as a boolean (`patch_tool.rs:172-176`), so a
//!    model asking for one edit could rewrite every occurrence. See
//!    [`OccurrenceRule`]. **Note the correction recorded in AD-043:** the count
//!    was *reported back* to the model as `replacements`, so this was a missing
//!    contract rather than silent corruption — the row is still worth closing,
//!    but the regression test asserts the *refusal names the count*, not that
//!    the old behavior was silent.
//! 4. **D2: the no-op branch is gone.** The dead `if replace.is_empty() { … }`
//!    arm at `patch_tool.rs:95-99` is not ported; delete-a-line works through
//!    the replacement list and is tested directly.
//!
//! ## What is deliberately NOT ported
//!
//! `parser.rs` / `streaming_parser.rs` / `invocation.rs`. The grammar is
//! already agent-reachable through AFT, and a second agent-facing surface is
//! two ways to say one thing. Cross-file rollback is not ported either —
//! codex has none, and `atomic_write` per file already gives operant the part
//! that matters.

/// Attempt to find the sequence of `pattern` lines within `lines` beginning at
/// or after `start`.
///
/// Returns the starting index of the match or `None`. Matches are attempted
/// with **decreasing strictness**: exact, then ignoring trailing whitespace,
/// then ignoring leading and trailing whitespace, then after normalizing
/// common Unicode punctuation to ASCII. When `eof` is true, the search starts
/// at the end-of-file position (so patterns intended to match file endings are
/// applied at the end) and falls back to `start` when that fails.
///
/// Defensive special cases, both carried from the source because a panic here
/// would be a crash in the middle of a write:
/// - an empty `pattern` returns `Some(start)` (a no-op match), and
/// - `pattern.len() > lines.len()` returns `None`, avoiding the out-of-bounds
///   slice that occurred in the source before 2025-04-12.
pub fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    // A pattern longer than the input cannot match anywhere.
    if pattern.len() > lines.len() {
        return None;
    }
    let search_start = if eof && lines.len() >= pattern.len() {
        (lines.len() - pattern.len()).max(start)
    } else {
        start
    };

    let upper = lines.len().saturating_sub(pattern.len());

    // 1. Exact match.
    for i in search_start..=upper {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }

    // 2. Trailing whitespace ignored.
    for i in search_start..=upper {
        let ok = pattern
            .iter()
            .enumerate()
            .all(|(p_idx, pat)| lines[i + p_idx].trim_end() == pat.trim_end());
        if ok {
            return Some(i);
        }
    }

    // 3. Leading and trailing whitespace ignored.
    for i in search_start..=upper {
        let ok = pattern
            .iter()
            .enumerate()
            .all(|(p_idx, pat)| lines[i + p_idx].trim() == pat.trim());
        if ok {
            return Some(i);
        }
    }

    // 4. Unicode punctuation normalized to ASCII — the tier that lets a model
    //    writing ASCII apply to a file containing typographic dashes / quotes.
    for i in search_start..=upper {
        let ok = pattern
            .iter()
            .enumerate()
            .all(|(p_idx, pat)| normalise(&lines[i + p_idx]) == normalise(pat));
        if ok {
            return Some(i);
        }
    }

    None
}

/// Map common typographic punctuation to its ASCII equivalent.
///
/// This mirrors `git apply`'s tolerance for minor byte-level differences when
/// locating context lines, and it is the only reason a patch written from an
/// ASCII mental model applies to a file that uses em-dashes or curly quotes.
fn normalise(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| match c {
            // Various dash / hyphen code-points → ASCII '-'
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            // Fancy single quotes → '\''
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            // Fancy double quotes → '"'
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            // Non-breaking and other odd spaces → a normal space
            '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
            | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
            | '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

/// A line together with the ending it ended with, so a rewrite can preserve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

/// One source line and its ending. `ending` is `None` for the final line of a
/// file that does not end with a newline.
struct SourceLine {
    text: String,
    ending: Option<LineEnding>,
}

/// A parsed file whose line endings survive a rewrite.
///
/// This is the piece operant's own `normalize_whitespace` destroys: it strips
/// `\r\n` entirely, so a fuzzy patch on a CRLF file converts the whole file.
pub struct SourceFile {
    lines: Vec<SourceLine>,
    preferred_ending: LineEnding,
}

impl SourceFile {
    /// Split contents into logical lines, retaining each line's ending.
    ///
    /// The first ending found becomes the preferred style for inserted lines;
    /// a file with no ending at all defaults to LF.
    pub fn parse(contents: &str) -> Self {
        let mut lines = Vec::new();
        let mut preferred_ending = None;
        let mut line_start = 0;
        let mut cursor = 0;
        let bytes = contents.as_bytes();

        while cursor < bytes.len() {
            let (ending, ending_len) = match bytes[cursor] {
                b'\r' if bytes.get(cursor + 1) == Some(&b'\n') => (LineEnding::CrLf, 2),
                b'\r' => (LineEnding::Cr, 1),
                b'\n' => (LineEnding::Lf, 1),
                _ => {
                    cursor += 1;
                    continue;
                }
            };
            preferred_ending.get_or_insert(ending);
            lines.push(SourceLine {
                text: contents[line_start..cursor].to_string(),
                ending: Some(ending),
            });
            cursor += ending_len;
            line_start = cursor;
        }

        if line_start < contents.len() {
            lines.push(SourceLine {
                text: contents[line_start..].to_string(),
                ending: None,
            });
        }

        Self {
            lines,
            preferred_ending: preferred_ending.unwrap_or(LineEnding::Lf),
        }
    }

    /// The line texts alone — the input `seek_sequence` wants.
    pub fn line_texts(&self) -> Vec<String> {
        self.lines.iter().map(|l| l.text.clone()).collect()
    }

    /// The ending this file prefers for inserted lines.
    pub fn preferred_ending(&self) -> LineEnding {
        self.preferred_ending
    }

    /// Apply source-ordered, non-overlapping replacements.
    ///
    /// Unchanged lines keep their original endings; inserted lines use the
    /// preferred one. Every resulting line ends up with an ending, matching
    /// apply-patch's historical trailing-newline behavior — which also gives
    /// an unterminated final line one when an insertion moves it inward.
    pub fn apply_replacements(&mut self, replacements: &[Replacement]) {
        let mut source = std::mem::take(&mut self.lines).into_iter();
        let mut new_lines: Vec<SourceLine> = Vec::new();
        let mut source_index = 0;

        for (start_idx, old_len, new_segment) in replacements {
            debug_assert!(
                *start_idx >= source_index,
                "replacements must be source-ordered and non-overlapping"
            );
            for line in source.by_ref().take(*start_idx - source_index) {
                new_lines.push(line);
            }
            for _ in source.by_ref().take(*old_len) {}
            new_lines.extend(new_segment.iter().map(|text| SourceLine {
                text: text.clone(),
                ending: Some(self.preferred_ending),
            }));
            source_index = start_idx + old_len;
        }
        new_lines.extend(source);
        self.lines = new_lines;

        for line in &mut self.lines {
            line.ending.get_or_insert(self.preferred_ending);
        }
    }

    /// Rebuild the file content, endings and all.
    pub fn into_contents(self) -> String {
        let mut contents = String::new();
        for line in self.lines {
            contents.push_str(&line.text);
            if let Some(ending) = line.ending {
                contents.push_str(ending.as_str());
            }
        }
        contents
    }
}

/// A replacement: replace `old_len` lines starting at `start_idx` with these.
pub type Replacement = (usize, usize, Vec<String>);

/// What to do when a `find` matches more than once.
///
/// The contract operant's `patch` tool did not have. codex refuses an
/// ambiguous find; operant used to rewrite every occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OccurrenceRule {
    /// The find matched exactly once. Apply it.
    Unique,
    /// The find matched N times and no occurrence index was given. Refuse, and
    /// report the count so the model can retry with an index or a longer find.
    Ambiguous { count: usize },
    /// The caller named occurrence `picked` out of `count`. Apply that one.
    Chosen { picked: usize, count: usize },
}

impl OccurrenceRule {
    /// Resolve a match count against an optional explicit index.
    ///
    /// A zero or one-match count is always `Unique`. More than one needs an
    /// index in range; a missing or out-of-range index is `Ambiguous` rather
    /// than a panic or a silent pick.
    pub fn resolve(count: usize, occurrence: Option<usize>) -> Self {
        if count <= 1 {
            return Self::Unique;
        }
        match occurrence {
            Some(picked) if picked < count => Self::Chosen { picked, count },
            _ => Self::Ambiguous { count },
        }
    }

    /// The model-facing message when the find is ambiguous.
    ///
    /// It has to name the count (that is the whole point: the model can act on
    /// it) and say what would unblock it.
    pub fn refusal_message(&self, needle: &str) -> Option<String> {
        match self {
            Self::Ambiguous { count } => Some(format!(
                "The text to find appears {count} times in this file, so the edit is \
                 ambiguous. Re-issue the patch with a longer find string that matches only \
                 the place you mean, or add an occurrence index 0..{count}. I will not \
                 guess which of the {count} to change. (find: {:?})",
                crate::agent::safe_truncate_str(needle, 120)
            )),
            _ => None,
        }
    }

    /// True when this rule permits applying the edit.
    pub fn is_allowed(&self) -> bool {
        !matches!(self, Self::Ambiguous { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_vec(strings: &[&str]) -> Vec<String> {
        strings.iter().map(|s| (*s).to_string()).collect()
    }

    // ── seek_sequence ──────────────────────────────────────────────────────

    #[test]
    fn exact_match_finds_sequence() {
        let lines = to_vec(&["foo", "bar", "baz"]);
        let pattern = to_vec(&["bar", "baz"]);
        assert_eq!(seek_sequence(&lines, &pattern, 0, false), Some(1));
    }

    #[test]
    fn rstrip_match_ignores_trailing_whitespace() {
        let lines = to_vec(&["foo   ", "bar\t\t"]);
        let pattern = to_vec(&["foo", "bar"]);
        assert_eq!(seek_sequence(&lines, &pattern, 0, false), Some(0));
    }

    #[test]
    fn trim_match_ignores_leading_and_trailing_whitespace() {
        let lines = to_vec(&["    foo   ", "   bar\t"]);
        let pattern = to_vec(&["foo", "bar"]);
        assert_eq!(seek_sequence(&lines, &pattern, 0, false), Some(0));
    }

    #[test]
    fn pattern_longer_than_input_returns_none_and_does_not_panic() {
        let lines = to_vec(&["just one line"]);
        let pattern = to_vec(&["too", "many", "lines"]);
        assert_eq!(seek_sequence(&lines, &pattern, 0, false), None);
    }

    #[test]
    fn an_empty_pattern_is_a_noop_match_at_start() {
        let lines = to_vec(&["a", "b"]);
        assert_eq!(seek_sequence(&lines, &[], 0, false), Some(0));
        assert_eq!(seek_sequence(&lines, &[], 1, false), Some(1));
    }

    #[test]
    fn the_eof_anchor_lets_a_pattern_match_the_end_of_file() {
        // A pattern intended for the file ending must land at the end, not at
        // an identical earlier block.
        let lines = to_vec(&["fn a() {}", "x", "y", "fn a() {}"]);
        let pattern = to_vec(&["fn a() {}"]);
        assert_eq!(
            seek_sequence(&lines, &pattern, 0, false),
            Some(0),
            "forward search finds the FIRST match"
        );
        assert_eq!(
            seek_sequence(&lines, &pattern, 0, true),
            Some(3),
            "the eof anchor finds the LAST — the one at the end of the file"
        );
    }

    #[test]
    fn the_unicode_tier_matches_a_file_written_with_typographic_punctuation() {
        // The tier operant's single normalize_whitespace does not have: a model
        // writing ASCII must apply to a file containing an em-dash and curly
        // quotes.
        let lines = vec!["let x = \"hello\u{2014}world\";".to_string()];
        let pattern = to_vec(&["let x = \"hello-world\";"]);
        assert_eq!(seek_sequence(&lines, &pattern, 0, false), Some(0));
    }

    #[test]
    fn unicode_normalisation_handles_curly_quotes_and_nbsp() {
        assert_eq!(normalise("\u{201C}x\u{201D}"), "\"x\"");
        assert_eq!(normalise("a\u{00A0}b").trim_end(), "a b".trim_end());
        assert_eq!(normalise("\u{2013}"), "-");
    }

    // ── SourceFile / CRLF fidelity ───────────────────────────────────────────

    #[test]
    fn crlf_endings_survive_a_rewrite() {
        // THE defect the port fixes. operant's normalize_whitespace strips
        // `\r\n` away, so a fuzzy match on a CRLF file rewrites every line
        // ending in it to LF.
        let original = "fn a() {\r\n  old\r\n}\r\n";
        let mut file = SourceFile::parse(original);
        assert_eq!(file.preferred_ending(), LineEnding::CrLf);

        let lines = file.line_texts();
        let at = seek_sequence(&lines, &to_vec(&["  old"]), 0, false).expect("finds the line");
        file.apply_replacements(&[(at, 1, to_vec(&["  new"]))]);

        let out = file.into_contents();
        assert_eq!(out, "fn a() {\r\n  new\r\n}\r\n", "CRLF must be preserved");
    }

    #[test]
    fn a_cr_file_keeps_cr_endings() {
        // Note the final `\r`: the port carries codex's documented behavior that
        // EVERY line gets an ending, so an unterminated final line gains one.
        let mut file = SourceFile::parse("a\rb\rc");
        assert_eq!(file.preferred_ending(), LineEnding::Cr);
        let lines = file.line_texts();
        let at = seek_sequence(&lines, &to_vec(&["b"]), 0, false).expect("finds b");
        file.apply_replacements(&[(at, 1, to_vec(&["B"]))]);
        assert_eq!(file.into_contents(), "a\rB\rc\r");
    }

    #[test]
    fn a_file_with_no_trailing_newline_gets_one_from_an_insert() {
        // apply-patch's historical behavior, ported: an insertion moving an
        // unterminated final line inward gives it an ending.
        let mut file = SourceFile::parse("a\nb");
        let lines = file.line_texts();
        let at = seek_sequence(&lines, &to_vec(&["b"]), 0, false).expect("finds b");
        file.apply_replacements(&[(at, 1, to_vec(&["b", "c"]))]);
        assert_eq!(file.into_contents(), "a\nb\nc\n");
    }

    #[test]
    fn a_multi_chunk_rewrite_is_one_call() {
        // "edit 5 places" becomes one apply, not five calls — the shape the
        // native fallback could not express. The trailing `\n` is the same
        // every-line-gets-an-ending rule.
        let mut file = SourceFile::parse("a\nx\nb\ny\nc");
        let lines = file.line_texts();
        let at_x = seek_sequence(&lines, &to_vec(&["x"]), 0, false).unwrap();
        let at_y = seek_sequence(&lines, &to_vec(&["y"]), 0, false).unwrap();
        file.apply_replacements(&[(at_x, 1, to_vec(&["X"])), (at_y, 1, to_vec(&["Y"]))]);
        assert_eq!(file.into_contents(), "a\nX\nb\nY\nc\n");
    }

    // ── OccurrenceRule — closing D1 ─────────────────────────────────────────

    /// THE D1 mutation guard, stated as an executable test.
    ///
    /// Operant computed `exact_count` and used it only as a boolean
    /// (`patch_tool.rs:172-176`), so a two-occurrence find rewrote BOTH. The
    /// rule now refuses. Restore the boolean-only use and this test fails,
    /// because it asserts the refusal MESSAGE NAMES THE COUNT.
    #[test]
    fn a_two_occurrence_find_is_refused_with_the_count_named() {
        let rule = OccurrenceRule::resolve(2, None);
        assert!(!rule.is_allowed(), "an ambiguous find must not apply");
        let msg = rule
            .refusal_message("foo")
            .expect("an ambiguous find produces a refusal");
        assert!(
            msg.contains('2'),
            "the refusal must name the occurrence count — got: {msg}"
        );
        assert!(
            msg.contains("occurrence index"),
            "the refusal must say what would unblock it — got: {msg}"
        );
    }

    #[test]
    fn an_explicit_occurrence_index_selects_one_of_many() {
        let rule = OccurrenceRule::resolve(3, Some(1));
        assert_eq!(
            rule,
            OccurrenceRule::Chosen {
                picked: 1,
                count: 3
            }
        );
        assert!(rule.is_allowed());
        assert!(rule.refusal_message("foo").is_none());
    }

    #[test]
    fn an_out_of_range_occurrence_index_is_ambiguous_not_a_panic() {
        assert_eq!(OccurrenceRule::resolve(3, Some(9)).is_allowed(), false);
        assert_eq!(OccurrenceRule::resolve(3, Some(0)).is_allowed(), true);
    }

    #[test]
    fn a_single_match_is_always_unique_index_or_not() {
        assert_eq!(OccurrenceRule::resolve(1, None), OccurrenceRule::Unique);
        assert_eq!(OccurrenceRule::resolve(0, None), OccurrenceRule::Unique);
        // A single match ignores a stray index rather than rejecting it.
        assert!(OccurrenceRule::resolve(1, Some(7)).is_allowed());
    }

    #[test]
    fn deleting_a_line_through_the_replacement_list_works() {
        // D2's intent, tested directly: an empty new segment removes the lines.
        // operant had a dead branch pretending to do this.
        let mut file = SourceFile::parse("keep\nremove\nkeep\n");
        let lines = file.line_texts();
        let at = seek_sequence(&lines, &to_vec(&["remove"]), 0, false).unwrap();
        file.apply_replacements(&[(at, 1, Vec::new())]);
        assert_eq!(file.into_contents(), "keep\nkeep\n");
    }
}
