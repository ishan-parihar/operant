//! The bar fill (plan 2026-11-14 U7).
//!
//! OMP's whole status line is built on one key: `theme.getBgAnsi("statusLineBg")`
//! is the fill behind every segment, and the powerline caps reuse that same
//! colour as their ink. Operant's status row had no fill, so there was nothing
//! for a separator to be drawn against.
//!
//! A ratatui `Span` carries foreground and background independently, so the
//! `useBgAsFg` flag OMP needs does not exist here: a cap simply gets the fill as
//! its foreground. See [`cap_style`].

use ratatui::prelude::*;

/// The configured bar fill, resolved from `Role::StatusLineBg`.
///
/// Reads the role's default rather than the configured value, for the same
/// reason every other accessor does: substitution happens once per frame in
/// `theme_mode::adapt_buffer_for_display`, and doing it in the accessor too
/// would remap the cell twice.
pub fn fill() -> Color {
    crate::tui::vendor::style::theme::status_line_bg()
}

/// The style the whole bar row takes: the fill, and nothing else.
///
/// Applied to the `Paragraph`, not to each span, so a span carrying its own
/// background still wins — which is what lets a warning chip stand out on the
/// fill instead of being flattened into it.
pub fn fill_style() -> Style {
    Style::default().bg(fill())
}

/// The style a powerline cap takes.
///
/// OMP marks its caps `useBgAsFg: true` and rewrites `\x1b[48;` to `\x1b[38;` at
/// render time (`status-line/component.ts:3333`). In ratatui the two colours are
/// independent fields, so the cap is simply the fill as its own foreground — no
/// escape rewriting. U8 draws the caps; the translation is pinned here because
/// getting it wrong is what makes a cap invisible against the fill.
#[allow(dead_code)] // U8 draws the caps; the translation is pinned here now.
pub fn cap_style() -> Style {
    let fill = fill();
    Style::default().fg(fill).bg(fill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::vendor::style::palette::Role;

    /// The fill must be the role's own value, not a constant, or `/theme`
    /// cannot reach the bar.
    #[test]
    fn fill_reads_the_status_line_bg_role() {
        let (r, g, b) = Role::StatusLineBg.default_rgb();
        assert_eq!(fill(), Color::Rgb(r, g, b));
    }

    /// A cap paints the fill as its own ink. This is the one assertion that
    /// pins the ratatui translation of OMP's `useBgAsFg` flag: if the two ever
    /// diverge, the cap becomes invisible against the fill.
    #[test]
    fn a_cap_reuses_the_fill_as_its_ink() {
        let cap = cap_style();
        assert_eq!(cap.fg, Some(fill()), "cap ink must be the fill colour");
        assert_eq!(cap.bg, Some(fill()), "cap background must be the fill");
    }

    /// The row style carries the fill and nothing else, so a span with its own
    /// background still wins.
    #[test]
    fn the_row_style_carries_only_the_fill() {
        let style = fill_style();
        assert_eq!(style.bg, Some(fill()));
        assert_eq!(
            style.fg, None,
            "the row must not impose an ink on every span"
        );
    }
}
