//! Char-boundary-safe text slicing.
//!
//! # Why this module exists
//!
//! Cutting a `String`/`&str` with a BYTE offset panics when the offset lands
//! inside a multi-byte char ("byte index N is not a char boundary"). Free text
//! is everywhere in this system -- session titles, cron names, task titles,
//! gateway payloads, model output -- so an operator with an accented or CJK
//! name could panic a CLI listing or the TUI draw path.
//!
//! This is the RT-301 class (`docs/audit/2026-06-19-BUGS.md`), which has now
//! recurred enough times that the fix belongs in one place rather than being
//! re-derived at each call site. Prefer these helpers over any `&s[..n]`.
//!
//! Two complementary helpers, because truncation has two shapes:
//!
//! - [`char_prefix`] / [`char_suffix`] -- keep the first/last `n` CHARS.
//! - [`take_chars`] -- truncate to `n` CHARS and append an ellipsis when cut.
//!
//! The TUI's own `tui::render::utils::take_width` is grapheme- and cell-aware and
//! remains the right helper for DRAWING, where the budget is terminal columns.
//! These are char-based so this crate needs no Unicode dependency.

/// The longest prefix of `s` with at most `n` chars.
///
/// Returns `s` unchanged when it is already short enough. Never panics, whatever
/// the multi-byte content.
pub fn char_prefix(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect()
}

/// The longest suffix of `s` with at most `n` chars. See [`char_prefix`].
pub fn char_suffix(s: &str, n: usize) -> String {
    let total = s.chars().count();
    if total <= n {
        return s.to_string();
    }
    s.chars().skip(total - n).collect()
}

/// Truncate `s` to at most `max` CHARS, appending `ellipsis` when cut.
///
/// Char-based, not display-cell-based: this crate does not depend on
/// `unicode-segmentation`, and a combining sequence split at a char boundary is
/// still valid UTF-8 (unlike a byte split, which panics). The TUI keeps its own
/// grapheme- and cell-aware `tui::render::utils::take_width` for drawing, where
/// the budget really is terminal columns; this one is for CLI text.
pub fn take_chars(s: &str, max: usize, ellipsis: &str) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(ellipsis);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The RT-301 class: a byte slice at these offsets would panic.
    #[test]
    fn char_prefix_never_splits_a_multibyte_char() {
        let s = format!("{}é rest", "a".repeat(38));
        assert!(s.len() > 39);
        // Byte 39 is inside the é; a byte slice would panic here.
        let _ = char_prefix(&s, 39);
    }

    #[test]
    fn char_suffix_never_splits_a_multibyte_char() {
        let s = format!("head {}é", "b".repeat(20));
        let _ = char_suffix(&s, 10);
    }

    #[test]
    fn short_input_is_returned_unchanged() {
        assert_eq!(char_prefix("hello", 40), "hello");
        assert_eq!(char_suffix("hello", 40), "hello");
    }

    #[test]
    fn cjk_input_is_counted_in_chars_not_bytes() {
        let s = "日本語のテキストです";
        assert_eq!(char_prefix(s, 2), "日本");
        assert_eq!(char_suffix(s, 2), "です");
    }

    // Budgets in CHARS, so a 2-char cut of CJK keeps both ideographs; a byte
    // cut at 2 would have panicked inside the first one.
    #[test]
    fn take_chars_budgets_chars_not_bytes() {
        let s = "日本語";
        // 2 CHARS, not 2 bytes: a byte slice at 2 would panic inside 日.
        assert_eq!(take_chars(s, 2, "…"), "日本…");
    }

    #[test]
    fn take_chars_leaves_short_input_alone_without_an_ellipsis() {
        assert_eq!(take_chars("abc", 10, "…"), "abc");
    }

    #[test]
    fn take_chars_does_not_split_a_multibyte_char() {
        let s = "日本語のとても長いテキスト";
        let out = take_chars(s, 5, "…");
        assert!(out.starts_with('日') || out.starts_with('本'));
        assert!(out.ends_with('…'));
    }
}
