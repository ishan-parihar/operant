//! The separator between two status-line segments (plan 2026-11-15 U10).
//!
//! `draw_status` and `build_notification_spans` both join their segments with a
//! literal `" · "` — 15 sites across the two functions, plus three more inside
//! `format!` strings that are *prose* rather than boundaries. Nothing could name
//! the separator, so nothing could restyle it: this module is the one definition.
//!
//! ## What is a boundary and what is not
//!
//! The distinction is load-bearing, because a powerline separator substituted
//! into a prose string reads as corruption. Measured at `ui_input.rs`:
//!
//! | Site | Kind |
//! |---|---|
//! | `:841` `append_transport_context` | boundary |
//! | `:887` `:894` `:901` `append_batch_progress_spans` | boundary |
//! | `:933` queued suffix | boundary |
//! | `:1133` `:1146` `:1151` `:1165` draw_status spans | boundary |
//! | `:1234` composer hint | boundary |
//! | `:1293` `running_tool_header_spans` | boundary |
//! | `:1047` `"{status} · {tps} tps"` | **within-segment** — tps qualifies the status |
//! | `:1051` `"{status} · ↑{in} ↓{out}"` | **within-segment** — token counts qualify it |
//! | `:1067` `"⚠ {miss} cache miss · {status}"` | **within-segment** |
//! | `:1083` `"network disconnected, … · {a} · {b}"` | **prose** — one span |
//! | `:1870` `"… · N chars · N lines · Enter/Y copy"` | **prose** — the hint text |
//!
//! A boundary is where two independently-styled spans meet. A within-segment
//! separator sits inside one span, between qualifiers on the same fact. Only
//! boundaries are the bar's shape; the rest are punctuation and must not move.
//!
//! ## Why the helper takes the whole span, not just the text
//!
//! [`push_separator`] is deliberately shaped like `build_notification_spans`'
//! own `push_sep` closure, which already had the right semantics: it emits
//! nothing for the first segment, so a caller can push unconditionally and let
//! the helper decide. Adopting that shape means the two functions converge on
//! one rule instead of forking a second.

use ratatui::prelude::*;

/// The separator drawn between two status-line segments.
///
/// One definition, so a style key can reach every boundary at once. U11 turns
/// this into a lookup; until then it is the literal both functions already used,
/// byte for byte.
pub fn separator() -> &'static str {
    " · "
}

/// The separator as a styled span, in the dim ink every boundary used.
pub fn separator_span() -> Span<'static> {
    Span::styled(separator(), Style::default().fg(dim()))
}

/// The dim ink the boundaries are drawn in.
///
/// `Role::Dim` rather than a literal, so a `/theme` switch reaches the
/// separator with everything else. Reads the role's frozen default, like every
/// accessor: substitution happens once per frame in
/// `theme_mode::adapt_buffer_for_display`.
fn dim() -> Color {
    crate::tui::vendor::style::theme::dim_color()
}

/// Push the separator before the next segment, unless this is the first one.
///
/// The first segment carries no leading separator, which is why the emptiness of
/// `spans` is the discriminator rather than a caller-supplied flag: a caller that
/// pushed unconditionally and then stripped a prefix would have to know the
/// prefix's width, and that knowledge is exactly what this module exists to hold.
pub fn push_separator(spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        spans.push(separator_span());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The separator must be the byte-identical literal both functions used, or
    /// U10 is not behaviour-preserving and every golden moves for no reason.
    #[test]
    fn the_separator_is_the_literal_both_functions_used() {
        assert_eq!(separator(), " · ");
    }

    /// Nothing precedes the first segment.
    #[test]
    fn no_separator_is_pushed_before_the_first_segment() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        push_separator(&mut spans);
        assert!(spans.is_empty(), "the first segment has no separator");
    }

    /// Every later segment gets exactly one.
    #[test]
    fn a_separator_is_pushed_before_every_later_segment() {
        let mut spans = vec![Span::raw("one")];
        push_separator(&mut spans);
        push_separator(&mut spans);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content.as_ref(), " · ");
        assert_eq!(spans[2].content.as_ref(), " · ");
    }

    /// The separator is dim, which is what makes it read as a rule between
    /// segments rather than as content.
    #[test]
    fn the_separator_is_dim() {
        assert_eq!(separator_span().style.fg, Some(dim()));
    }

    /// THE MIGRATION GUARD. Every boundary in `draw_status` and
    /// `build_notification_spans` goes through [`separator`], so a style key can
    /// reach the whole bar at once. A newly hand-written `" · "` literal is a new
    /// boundary nobody can restyle — the exact defect U10 closes.
    ///
    /// Scoped to the PRODUCTION half of `ui_input.rs`: it ends at the file's main
    /// `mod tests`, and the three further `#[cfg(test)]` blocks below that are all
    /// inside it. The search is over that whole half, not a line window, so a
    /// boundary added anywhere in the production code is caught. Test assertions
    /// are deliberately out of scope: they pin what the bar RENDERS, and a literal
    /// there is the expected value, not a second implementation.
    ///
    /// The PROSE sites are not needles this can match. Their `·` sits mid-string,
    /// so the `" · ` prefix below cannot see them — and forcing them through the
    /// helper is exactly the corruption the module doc warns about.
    #[test]
    fn every_bar_boundary_goes_through_the_separator_helper() {
        const UI_INPUT: &str = include_str!("../operant_ui/ui_input.rs");

        let production = match UI_INPUT.find("\nmod tests {") {
            Some(at) => &UI_INPUT[..at],
            // A renamed test module must fail loudly, not silently widen the scan
            // into assertions and report a defect that is not there.
            None => panic!("ui_input.rs has no `mod tests`; this guard's scope is undefined"),
        };

        // A boundary literal is `" · ` — a leading separator at the start of a
        // format string. Prose carries ` · ` mid-string and cannot match it.
        let hand_written: Vec<String> = production
            .lines()
            .enumerate()
            .filter(|(_, line)| line.split("//").next().unwrap_or(line).contains("\" · "))
            .map(|(i, line)| format!("{}: {}", i + 1, line.trim()))
            .collect();

        assert!(
            hand_written.is_empty(),
            "these lines hand-write a bar boundary instead of calling \
             status_line::separator::separator(); a boundary nobody can name is a \
             boundary a style key cannot reach:\n  {}",
            hand_written.join("\n  ")
        );
    }
}
