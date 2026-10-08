// messages/helpers.rs — Shared rendering primitives for message renderers.
//
// Extracted from messages/mod.rs. Low-level helpers used by the
// transcript, tool, and command renderers.

use crate::tui::vendor::style::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

// ---------------------------------------------------------------------------
// Overflow labelling
// ---------------------------------------------------------------------------
//
// A pane that shows a window over a longer list used to stop at the last row
// that fit, silently: a short list and a clipped one rendered identically. The
// two markers below say which happened, in `dim` so they read as chrome rather
// than as content. `fold_window` is the single place the arithmetic lives, so a
// marker can never disagree with the rows actually painted.

/// A list window inside a pane, plus the counts needed to label the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FoldWindow {
    /// Index of the first entry the pane paints.
    pub start: usize,
    /// How many entries the pane paints. Always the last `shown` of
    /// `start..=start + shown`, so callers can slice `items[start..][..shown]`.
    pub shown: usize,
    /// Entries above the fold. `0` when the window starts at the top.
    pub hidden_above: usize,
    /// Entries below the fold. `0` when the window reaches the end.
    pub hidden_below: usize,
}

/// Pick the window a pane shows for `total` entries, keeping `selected`
/// visible, and report how much of the list falls outside it.
///
/// When the list is taller than the pane, one row is reserved for each marker
/// *whether or not* that marker ends up non-empty. Reserving both up front
/// keeps this single-pass and the arithmetic exact; an empty reservation just
/// leaves a blank row, which is cheaper than a marker whose count could be
/// off by one.
///
/// Below three rows the pane is too short to spend a row on chrome, so the
/// counts are reported as `0`: nothing is claimed that cannot be shown. The
/// `pane < 3` floor is why the tool list is not given markers on a stub
/// terminal.
pub(crate) fn fold_window(total: usize, selected: usize, pane_rows: u16) -> FoldWindow {
    let pane = pane_rows as usize;
    if total <= pane || pane < 3 {
        return FoldWindow {
            start: 0,
            shown: total.min(pane),
            hidden_above: 0,
            hidden_below: 0,
        };
    }
    let last = total - 1;
    let body = pane - 2;
    let mut start = selected.min(last).saturating_sub(body / 2);
    // Pull the window back so it ends at the end of the list rather than
    // running off it, which is what keeps `hidden_below` at 0 when the
    // selection is near the tail.
    if start + body > total {
        start = total - body;
    }
    FoldWindow {
        start,
        shown: body,
        hidden_above: start,
        hidden_below: total - start - body,
    }
}

/// `↑N` — how many entries sit above the fold. `dim`, like every other piece of
/// chrome here: the arrow says "there is more", the count says how much.
pub(crate) fn fold_above_label(hidden: usize) -> Span<'static> {
    Span::styled(
        format!("↑{hidden}"),
        Style::default()
            .fg(theme::dim_color())
            .add_modifier(Modifier::DIM),
    )
}

/// `+N more` — how many entries sit below the fold.
pub(crate) fn fold_below_label(hidden: usize) -> Span<'static> {
    Span::styled(
        format!("+{hidden} more"),
        Style::default()
            .fg(theme::dim_color())
            .add_modifier(Modifier::DIM),
    )
}

#[cfg(test)]
mod fold_tests {
    use super::*;

    /// The slice a caller is about to take must always be in range — this is
    /// the invariant that keeps the render loops from panicking on a short pane.
    #[test]
    fn the_window_slice_is_always_in_range() {
        for pane in 0u16..14 {
            for total in 0usize..40 {
                for selected in 0usize..40 {
                    let f = fold_window(total, selected, pane);
                    assert!(
                        f.start + f.shown <= total,
                        "pane={pane} total={total} selected={selected} -> {f:?}"
                    );
                    assert!(
                        total.saturating_sub(1) == 0 || f.start < total,
                        "pane={pane} total={total} selected={selected} -> {f:?}"
                    );
                }
            }
        }
    }

    /// `↑N` + shown + `+N more` must account for every entry exactly once: a
    /// marker that under- or over-counts is the whole bug this exists to stop.
    #[test]
    fn the_counts_account_for_every_entry_exactly_once() {
        for pane in 3u16..14 {
            for total in 1usize..40 {
                for selected in 0usize..40 {
                    let f = fold_window(total, selected, pane);
                    assert_eq!(
                        f.hidden_above + f.shown + f.hidden_below,
                        total,
                        "pane={pane} total={total} selected={selected} -> {f:?}"
                    );
                    assert_eq!(f.hidden_above, f.start, "pane={pane} total={total}");
                }
            }
        }
    }

    #[test]
    fn a_list_that_fits_labelled_nothing() {
        let f = fold_window(4, 3, 10);
        assert_eq!(
            f,
            FoldWindow {
                start: 0,
                shown: 4,
                hidden_above: 0,
                hidden_below: 0
            }
        );
    }

    #[test]
    fn a_list_too_tall_for_its_claims_nothing() {
        // Two rows cannot pay for a marker row, so no fold is claimed at all.
        for selected in 0..9 {
            let f = fold_window(9, selected, 2);
            assert_eq!(f.hidden_above, 0, "selected={selected}");
            assert_eq!(f.hidden_below, 0, "selected={selected}");
        }
    }

    #[test]
    fn an_overflowing_list_labelled_both_ends_of_the_window() {
        // 20 entries in 8 rows: 6 body rows, 2 marker rows.
        let f = fold_window(20, 0, 8);
        assert_eq!(f.hidden_above, 0);
        assert_eq!(f.shown, 6);
        assert_eq!(f.hidden_below, 14);

        let f = fold_window(20, 10, 8);
        assert_eq!(f.hidden_above, 7);
        assert_eq!(f.shown, 6);
        assert_eq!(f.hidden_below, 7);

        // Selection near the tail pulls the window back so it ends at the end.
        let f = fold_window(20, 19, 8);
        assert_eq!(f.hidden_below, 0);
        assert_eq!(f.start, 14);
    }

    #[test]
    fn the_labels_read_as_chrome() {
        assert_eq!(fold_above_label(15).content, "↑15");
        assert_eq!(fold_below_label(33).content, "+33 more");
        for label in [fold_above_label(15), fold_below_label(33)] {
            let style = label.style;
            assert_eq!(style.fg, Some(theme::dim_color()), "labels must be `dim`");
            assert!(style.add_modifier.contains(Modifier::DIM));
        }
    }
}
