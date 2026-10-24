//! Case-insensitive search that never panics on fold-length drift.
//!
//! BUGS.md R48-3: `str::to_lowercase()` is not byte-length-preserving —
//! U+212A KELVIN SIGN folds 3B→1B, U+212B ANGSTROM SIGN 3B→2B, U+1E9E
//! CAPITAL SHARP S 3B→2B, U+0130 LATIN CAPITAL I WITH DOT 2B→3B. A byte
//! offset found in a lowercased copy must never be applied to the original
//! string: the global-search highlighter did exactly that and panicked
//! `text_trimmed[..pos]` every frame on a repo line like `K ok` (query
//! `k`). These helpers fold per char and map every match offset back to an
//! ORIGINAL char boundary.

/// The lowercased text plus, per ORIGINAL char, its byte span inside the
/// folded text and its byte start in the original:
/// `(folded, [(fold_start, fold_end, orig_start)])`.
///
/// `pub(crate)` so both search overlays (and any future highlighter) share
/// one implementation instead of growing a third copy of the drift bug.
pub(crate) fn folded_spans(haystack: &str) -> (String, Vec<(usize, usize, usize)>) {
    let mut folded = String::with_capacity(haystack.len());
    let mut spans = Vec::with_capacity(haystack.len());
    for (b, ch) in haystack.char_indices() {
        let start = folded.len();
        folded.extend(ch.to_lowercase());
        spans.push((start, folded.len(), b));
    }
    (folded, spans)
}

/// The original byte start of the char whose folded span contains `offset`.
fn span_orig_byte(spans: &[(usize, usize, usize)], offset: usize) -> Option<usize> {
    spans
        .iter()
        .find(|&&(fs, fe, _)| fs <= offset && offset < fe)
        .map(|&(_, _, b)| b)
}

/// Find `needle` in `haystack` case-insensitively; returns `(start, end)`
/// byte offsets into the ORIGINAL haystack, always at char boundaries.
///
/// `None` when there is no match. An empty needle matches at `(0, 0)`.
pub(crate) fn folded_find(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let needle_lc = needle.to_lowercase();
    if needle_lc.is_empty() {
        return Some((0, 0));
    }
    let (folded, spans) = folded_spans(haystack);
    let pos = folded.find(&needle_lc)?;
    Some(map_folded_range(haystack, &spans, pos, needle_lc.len()))
}

/// Find the LAST occurrence of `needle` in `haystack` case-insensitively,
/// with the same original-coordinate contract as [`folded_find`].
pub(crate) fn folded_find_last(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let needle_lc = needle.to_lowercase();
    if needle_lc.is_empty() {
        return None;
    }
    let (folded, spans) = folded_spans(haystack);
    let pos = folded.rfind(&needle_lc)?;
    Some(map_folded_range(haystack, &spans, pos, needle_lc.len()))
}

/// Map a matched byte range in the FOLDED text back to original-char
/// boundaries: the start is the original byte of the char whose folded span
/// contains `pos`; the end closes the char whose folded span contains the
/// match's last byte.
fn map_folded_range(
    haystack: &str,
    spans: &[(usize, usize, usize)],
    pos: usize,
    folded_len: usize,
) -> (usize, usize) {
    let end = pos + folded_len;
    let start_byte = span_orig_byte(spans, pos).unwrap_or(0);
    let end_byte = span_orig_byte(spans, end - 1)
        .and_then(|b| haystack[b..].chars().next().map(|c| b + c.len_utf8()))
        .unwrap_or(haystack.len());
    (start_byte, end_byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R48-3: the drift class that panicked the old highlighter — a query
    /// matching only after folding, on a char whose fold changes length.
    #[test]
    fn kelvin_sign_match_maps_to_original_boundaries() {
        let hay = "\u{212A} ok"; // KELVIN SIGN + " ok"
        let (s, e) = folded_find(hay, "k").expect("k folds to match K");
        assert_eq!((s, e), (0, 3));
        assert_eq!(&hay[s..e], "\u{212A}");
    }

    /// İ folds to TWO chars (i + combining dot); matching the visible char
    /// must highlight the whole original char, and offsets stay boundaries.
    #[test]
    fn multi_char_fold_highlights_the_whole_char() {
        let hay = "İx";
        let (s, e) = folded_find(hay, "i").expect("i matches the folded İ");
        assert_eq!((s, e), (0, 2));
        assert_eq!(&hay[s..e], "\u{0130}");
    }

    /// Sharp-s case pair: ẞ (3B) folds to ß (2B).
    #[test]
    fn capital_sharp_s_matches_lower() {
        let hay = "aẞb";
        let (s, e) = folded_find(hay, "ß").expect("ß matches ẞ");
        assert_eq!(&hay[s..e], "\u{1E9E}");
    }

    /// Boundaries hold for every result — the property the old code lacked.
    #[test]
    fn every_match_is_at_char_boundaries() {
        for hay in [
            "\u{212A} ok",
            "\u{0130}x",
            "a\u{1E9E}b",
            "plain ascii",
            "\u{e9}\u{c0}\u{fc}\u{4e2d}",
            "",
        ] {
            let (folded, spans) = folded_spans(hay);
            for (fs, fe, ob) in &spans {
                assert!(hay.is_char_boundary(*ob));
                assert!(fs <= fe && *fe <= folded.len());
            }
            if let Some((s, e)) = folded_find(hay, "o") {
                assert!(hay.is_char_boundary(s) && hay.is_char_boundary(e));
            }
        }
    }

    #[test]
    fn plain_cases_match_str_find_semantics() {
        assert_eq!(folded_find("hello world", "wor"), Some((6, 9)));
        assert_eq!(folded_find("hello", "xyz"), None);
        assert_eq!(folded_find("hello", ""), Some((0, 0)));
        assert_eq!(folded_find("", "x"), None);
        assert_eq!(folded_find("MIXED Case", "mixed case"), Some((0, 10)));
    }
}
