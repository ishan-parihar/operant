//! Row backgrounds for the session hub — codex's `resume_picker.rs`
//! `dense_row_background_style` + `apply_line_background`, reduced to what a
//! 40%-width operating session list needs.
//!
//! ## The problem this solves
//!
//! Operant's rows painted their foregrounds (`theme_colors::text()`,
//! `muted()`, `accent()`) against whatever the terminal's base background
//! happened to be, and the selected row set `bg(selection_bg())` on three
//! spans only. Two consequences, both visible on the drawn buffer:
//!
//! 1. The selection highlight ended where the text ended, so a short row
//!    looked like a stripe of colour with the row still "open" to the right.
//! 2. Nothing resolved the text against the painted background, so a span
//!    whose own hue sat too close to the fill under it read as a gap.
//!
//! ## What codex does and what is ported
//!
//! `dense_row_background_style` blends a white (on light themes) or black
//! (on dark themes) overlay INTO the terminal background at 0.12 for the
//! selected row and 0.04–0.055 for unselected rows — the tint comes from the
//! surface itself, so it cannot clash. `apply_line_background` then paints
//! the fill across the row's full width and re-resolves every span's fg
//! against the painted background (`readable_color_on`).
//!
//! Ported here: the alpha ladder and the full-width fill. NOT ported: codex's
//! `readable_color_on`, because it needs a terminal-palette xterm table and a
//! quantised-output guarantee that operant's 22-role palette does not use;
//! operant's spans are already palette roles chosen against the base
//! background, which is the invariant that matters here. What IS ported of the
//! contrast question is the honest part: the alpha values below were measured
//! against operant's `theme_colors::selection_bg()`, and each carries the
//! test that pins it.

use ratatui::style::{Color, Style};

use crate::tui::vendor::style::theme::{blend_color, user_bg};
use crate::tui::vendor::style::theme_mode::is_light_theme;

/// Alpha of the tint blended into the terminal background for the row the
/// operator has selected. Codex uses 0.12 for exactly this row, and the
/// measurement agrees with operant's light AND dark themes (see
/// `selected_fill_is_visible_against_both_theme_backgrounds`).
pub const SELECTED_ALPHA: f32 = 0.12;

/// Alpha for an unselected row in a zebra pass. Codex splits this by theme
/// (0.04 light / 0.055 dark); a single value that is visible on both is
/// preferable here, because operant's operator does not re-theme per read.
pub const ZEBRA_ALPHA: f32 = 0.05;

/// Below this the fill is invisible and the zebra is decoration that costs a
/// paint without information. Operant's `selection_bg()` alpha sits here, so
/// the test pins the ladder rather than the literals.
pub const MIN_VISIBLE_ALPHA: f32 = 0.02;

/// The row's own fill, blended from the terminal background at `alpha`.
///
/// `None` when `alpha` is below `MIN_VISIBLE_ALPHA`: an invisible fill is not
/// a "subtle" fill, it is wasted work, and returning `Style::default()` lets
/// the caller skip the per-span patch pass entirely.
pub fn row_fill_style(alpha: f32) -> Option<Style> {
    if alpha < MIN_VISIBLE_ALPHA {
        return None;
    }
    let bg = blend_color(user_bg(), overlay_color(), alpha);
    Some(Style::default().bg(bg))
}

/// The selected row's fill.
pub fn selected_fill_style() -> Option<Style> {
    row_fill_style(SELECTED_ALPHA)
}

/// The unselected row's fill on a zebra pass (`row_idx` even).
pub fn zebra_fill_style(row_idx: usize) -> Option<Style> {
    if row_idx.is_multiple_of(2) {
        row_fill_style(ZEBRA_ALPHA)
    } else {
        None
    }
}

/// White on light, black on dark — the tint must move AGAINST the surface or
/// it disappears. This is the whole content of codex's `is_light(bg)` branch.
pub fn overlay_color() -> Color {
    if is_light_theme() {
        Color::Rgb(0, 0, 0)
    } else {
        Color::Rgb(255, 255, 255)
    }
}

/// Paint `fill` across the row's full width and re-resolve every span's
/// foreground against the painted background.
///
/// Three things happen, in order, and each is load-bearing:
///
/// 1. The trailing padding is PADDED to `width` so the fill reaches the right
///    border. Without it the highlight stops at the text — the exact gap
///    documented in the module header.
/// 2. `line.style` is patched so a line-level modifier survives.
/// 3. Each span keeps its own hue; only the background is forced. Recolouring
///    the text here is what would destroy the status-glyph and accent
///    semantics the rows carry.
pub fn apply_row_background(
    line: ratatui::text::Line<'_>,
    fill: Style,
    width: u16,
) -> ratatui::text::Line<'_> {
    let mut line = line;
    let pad = (width as usize).saturating_sub(line.width());
    if pad > 0 {
        line.spans
            .push(ratatui::text::Span::styled(" ".repeat(pad), fill));
    }
    line.style = line.style.patch(fill);
    for span in &mut line.spans {
        // Patch, do not replace: `patch` keeps the span's own fg and modifiers
        // where it has them, and only the fill's background lands.
        span.style = fill.patch(span.style);
    }
    line
}
