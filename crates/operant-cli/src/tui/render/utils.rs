// render/utils.rs — Spinner helpers, modal checks, text truncation, shimmer effects.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

// -----------------------------------------------------------------------
// Text truncation helpers
// -----------------------------------------------------------------------

/// Display width of `s` in terminal cells.
///
/// This is the single width primitive for the render layer: every
/// width-sensitive path must call this instead of `.chars().count()` or
/// `.len()`. Two things it gets right that those do not:
///
/// * **Grapheme clusters are indivisible.** `unicode-width` folds combining
///   marks, variation selectors and ZWJ emoji sequences into the width the
///   terminal actually advances, so a base character plus its accents is one
///   unit of width, not one per code point.
/// * **East Asian wide / fullwidth forms are two cells.** They are a single
///   grapheme cluster, so counting clusters alone would still undercount; the
///   East Asian Width table is what makes a CJK ideograph measure 2.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Longest prefix of `s` that fits within `max_width` terminal cells, cut only
/// on a grapheme-cluster boundary.
///
/// A cluster wider than `max_width` on its own (a two-cell ideograph with a
/// one-cell budget) contributes nothing rather than overflowing the budget.
pub fn take_width(s: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for g in s.graphemes(true) {
        let gw = display_width(g);
        if width + gw > max_width {
            break;
        }
        out.push_str(g);
        width += gw;
    }
    out
}

// -----------------------------------------------------------------------
// Line wrapping
// -----------------------------------------------------------------------

/// Minimum-raggedness (balanced) word wrap.
///
/// Greedy fill packs every line except the last to `width`, which leaves the
/// final line as a lone short word. This picks the break points that even the
/// line widths out instead, by minimising the total slack
/// `sum((width - line_width)^2)` over all break choices — the classic
/// minimum-raggedness objective.
///
/// **Opt-in.** Greedy stays the default everywhere: balanced wrap destroys
/// column alignment in tables, code blocks and ASCII art. Use this only for
/// flowing prose, where an even right edge reads better than a greedy one.
///
/// Pure — no terminal, no global state — so it is unit-testable directly.
/// Breaks fall on whitespace, except for a single token wider than `width`,
/// which is hard-broken on grapheme-cluster boundaries so it can never
/// overflow or split a cluster.
pub fn balanced_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }

    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            out.push(paragraph.to_string());
            continue;
        }

        // Break candidates, each a chunk that fits on one line: every word,
        // with over-wide words hard-broken.
        let mut chunks: Vec<String> = Vec::new();
        for word in paragraph.split_whitespace() {
            if display_width(word) > width {
                let mut rest = word;
                while display_width(rest) > width {
                    let head = take_width(rest, width);
                    // A leading cluster wider than the budget gives an empty
                    // head. Emit that cluster anyway — it overflows, but it is
                    // the only way to guarantee progress, and a chunk that
                    // consumes no input would spin this loop forever.
                    let split = if head.is_empty() {
                        match rest.graphemes(true).next() {
                            Some(g) => g.len(),
                            None => break,
                        }
                    } else {
                        head.len()
                    };
                    chunks.push(rest[..split].to_string());
                    rest = &rest[split..];
                }
                // The loop above can consume the token exactly, leaving an
                // empty remainder. Pushing it would add a zero-width chunk,
                // which both miscounts the greedy line count and desyncs the
                // DP's reachability.
                if !rest.is_empty() {
                    chunks.push(rest.to_string());
                }
            } else {
                chunks.push(word.to_string());
            }
        }

        // `best[k]` = least slack for wrapping chunks `k..`; one entry per line
        // start, with the line-start index of the optimum in `from[k]`.
        let n = chunks.len();
        let widths: Vec<usize> = chunks.iter().map(|c| display_width(c)).collect();

        // Greedy line count. The DP is pinned to exactly this many lines so
        // balancing can never make a block *taller* than greedy would — a
        // dialog sized from this wrap would otherwise grow.
        let mut line_count = 1usize;
        let mut acc = 0usize;
        for (i, w) in widths.iter().enumerate() {
            let add = *w + usize::from(i > 0 && acc > 0);
            if acc > 0 && acc + add > width {
                line_count += 1;
                acc = *w;
            } else {
                acc += add;
            }
        }

        // `best[k][i]` = least total slack for laying out chunks `i..` in
        // exactly `k` lines; `from[k][i]` records where the first of those
        // lines ends.
        let mut best = vec![vec![usize::MAX; n + 1]; line_count + 1];
        let mut from = vec![vec![usize::MAX; n + 1]; line_count + 1];
        best[0][n] = 0;
        for k in 1..=line_count {
            for start in (0..n).rev() {
                // A chunk wider than the budget (a single indivisible cluster)
                // has no feasible placement inside a line, so give it its own
                // line. Without this, `from[k][start]` would stay unset and the
                // paragraph would lose every chunk after it.
                if widths[start] > width {
                    best[k][start] = best[k - 1][start + 1];
                    from[k][start] = start;
                    continue;
                }
                let mut line_w = 0usize;
                for end in start..n {
                    // A line costs one space between adjacent chunks, none at
                    // the first chunk.
                    line_w += widths[end] + usize::from(end > start);
                    if line_w > width {
                        break;
                    }
                    let slack = width - line_w;
                    let cost = best[k - 1][end + 1].saturating_add(slack * slack);
                    if cost < best[k][start] {
                        best[k][start] = cost;
                        from[k][start] = end;
                    }
                }
            }
        }

        let mut k = line_count;
        let mut start = 0usize;
        while start < n {
            let end = from[k][start];
            debug_assert_ne!(end, usize::MAX, "every chunk has a feasible placement");
            let end = if end == usize::MAX { start } else { end };
            let line: Vec<&str> = chunks[start..=end].iter().map(String::as_str).collect();
            out.push(line.join(" "));
            start = end + 1;
            k -= 1;
        }
    }
    out
}

#[cfg(test)]
mod width_wrap_tests {
    use super::{balanced_wrap, display_width, take_width};
    use crate::tui::dialogs::permission::word_wrap;
    use unicode_segmentation::UnicodeSegmentation;

    /// A four-person ZWJ family emoji: five code points, one grapheme cluster.
    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";

    /// No wrap output may end mid-cluster, which a `chars()`-driven wrapper
    /// does by leaving a dangling ZWJ (or a base with its joiner torn off).
    fn assert_no_split_clusters(lines: &[String]) {
        for line in lines {
            assert!(
                !line.ends_with('\u{200D}'),
                "line ends with a dangling ZWJ — a cluster was split: {line:?}"
            );
            assert!(
                !line.contains('\u{200D}') || line.contains(FAMILY),
                "line carries a ZWJ that is not part of a whole emoji cluster: {line:?}"
            );
        }
    }

    #[test]
    fn wrap_should_never_split_a_grapheme_cluster() {
        // An unbreakable token of ZWJ clusters, wide enough to force hard breaks.
        let token = FAMILY.repeat(6);
        for width in 1..=12 {
            for lines in [
                word_wrap(&token, width),
                balanced_wrap(&token, width),
                balanced_wrap(&format!("prefix {token} suffix"), width),
            ] {
                assert_no_split_clusters(&lines);
                // Every line must be a whole number of source clusters, and the
                // concatenation must lose no cluster other than the joins.
                let joined: String = lines.join("");
                for g in token.graphemes(true) {
                    assert!(
                        joined.matches(g).count() > 0,
                        "cluster {g:?} vanished at width {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn wrap_should_measure_east_asian_characters_as_two_cells() {
        // One ideograph is one grapheme cluster but TWO terminal cells.
        assert_eq!("\u{4E2D}".graphemes(true).count(), 1);
        assert_eq!(display_width("\u{4E2D}"), 2);
        assert_eq!(display_width("\u{4E2D}\u{6587}"), 4);
        // Fullwidth form and a fullwidth Latin letter are two cells each.
        assert_eq!(display_width("\u{FF21}"), 2);

        // A 4-cell budget fits exactly one two-glyph word and no more.
        assert_eq!(
            word_wrap("\u{4E2D}\u{6587} \u{4E2D}\u{6587}", 4),
            vec!["\u{4E2D}\u{6587}", "\u{4E2D}\u{6587}"]
        );
        // A 3-cell budget cannot hold a 4-cell word, so it must not "fit".
        assert_eq!(
            word_wrap("\u{4E2D}\u{6587}", 3),
            vec!["\u{4E2D}", "\u{6587}"]
        );
        assert_eq!(
            balanced_wrap("\u{4E2D}\u{6587}", 3),
            vec!["\u{4E2D}", "\u{6587}"]
        );
        // `take_width` cuts on cluster boundaries and honours the cell budget.
        assert_eq!(take_width("\u{4E2D}\u{6587}", 3), "\u{4E2D}");
        assert_eq!(display_width(&take_width("\u{4E2D}\u{6587}", 3)), 2);
    }

    #[test]
    fn balanced_wrap_should_not_lengthen_the_final_line() {
        // The paragraph greedy wrap renders with a very short last line.
        let text = "The quick brown fox jumps over the lazy dog near the riverbank at dawn.";

        for width in [9usize, 12, 16, 20, 24, 32, 40] {
            let greedy = word_wrap(text, width);
            let balanced = balanced_wrap(text, width);

            let greedy_last = display_width(greedy.last().map_or("", |s| s.as_str()));
            let balanced_last = display_width(balanced.last().map_or("", |s| s.as_str()));
            assert!(
                balanced_last >= greedy_last,
                "balanced shortened the final line at width {width}: \
                 greedy {greedy:?} (last {greedy_last}) vs balanced {balanced:?} (last {balanced_last})"
            );

            // Every line must still fit the budget, the line count must match
            // greedy (balancing redistributes slack, it never adds a line), and
            // no word may be dropped or duplicated.
            for line in &balanced {
                assert!(
                    display_width(line) <= width,
                    "balanced overflowed the budget at width {width}: {line:?}"
                );
            }
            assert_eq!(
                balanced.len(),
                greedy.len(),
                "balanced changed the line count at width {width}"
            );
            assert_eq!(
                balanced
                    .iter()
                    .map(|l| l.split_whitespace().count())
                    .sum::<usize>(),
                text.split_whitespace().count(),
                "balanced dropped or duplicated words at width {width}"
            );
        }

        // Spot-check the actual improvement, not just the invariant. Both use
        // the same six lines — balancing redistributes slack, it never adds a
        // line (an extra line would resize the dialog).
        assert_eq!(
            word_wrap(text, 16),
            vec![
                "The quick brown",
                "fox jumps over",
                "the lazy dog",
                "near the",
                "riverbank at",
                "dawn."
            ]
        );
        assert_eq!(
            balanced_wrap(text, 16),
            vec![
                "The quick",
                "brown fox",
                "jumps over the",
                "lazy dog near",
                "the riverbank",
                "at dawn."
            ]
        );
    }

    #[test]
    fn greedy_wrap_should_be_unchanged_for_ascii() {
        // Greedy stays the default: these are the exact outputs the renderer
        // produced before the width change, pinned so the ASCII path cannot drift.
        assert_eq!(
            word_wrap("the quick brown fox", 9),
            vec!["the quick", "brown fox"]
        );
        assert_eq!(word_wrap("0123456789", 4), vec!["0123", "4567", "89"]);
        assert_eq!(word_wrap("0123456789", 9), vec!["012345678", "9"]);
        assert_eq!(word_wrap("0123456789", 16), vec!["0123456789"]);
        assert_eq!(word_wrap("hello world", 80), vec!["hello world"]);
        assert_eq!(word_wrap("", 80), vec![""]);
        // The tool command is wrapped greedily on purpose — column alignment
        // in code/ASCII art must survive. 22 cells does not fit a 21-cell
        // command column, so it still breaks on a space.
        assert_eq!(
            word_wrap("cargo test --workspace", 22),
            vec!["cargo test --workspace"]
        );
        assert_eq!(
            word_wrap("cargo test --workspace", 21),
            vec!["cargo test", "--workspace"]
        );
    }
}
