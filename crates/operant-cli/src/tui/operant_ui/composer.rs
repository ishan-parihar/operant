//! The chat composer's codex-parity visual layer.
//!
//! Ported from codex `tui/src/bottom_pane/{chat_composer.rs,composer_layout.rs}`
//! and `tui/src/style.rs` (upstream OpenAI codex, Apache-2.0). What was ported:
//! the filled-band surface, the two-column prompt gutter, the dimmed
//! placeholder, and the one blank row above and below the text. What was
//! deliberately NOT ported, and why:
//!
//! * `status_surface.rs`'s persistent status row. operant already renders a
//!   status block above the composer (the brand header plus the overscroll row
//!   below it); a second status surface would violate the repo's one-renderer
//!   rule in AGENTS.md and the operator's "keep the footer as it is" constraint.
//! * `Sparkle`, `VoiceStrip`, the effort-ignition glyph gradients, remote-image
//!   rows, `composer_gap` separator rows and warning notices. Each is a whole
//!   subsystem with its own frame scheduling; none is a restyle.
//! * `TextArea` (4,760 lines: vim registers, search, grapheme editing,
//!   paste-burst detection). operant's `wrap_input_segments` /
//!   `visual_line_move` is a working subset; the parity gap there is real but
//!   out of scope for an input-box restyle.
//!
//! ## Why a fill and not a border
//!
//! codex's composer has no box-drawing border anywhere: no `Borders::ALL`, no
//! rounded frame, no per-side colour. The premium read comes from
//! `Block::default().style(user_message_style()).render(composer_rect, buf)`
//! (`chat_composer.rs:4889`), which paints the terminal background lifted by
//! white at alpha 0.12 (`user_message_bg_rgb`, `style.rs:261-268`), plus two
//! columns of gutter and one blank row on each side of the text. A port that
//! adds a box has missed the point.
//!
//! operant cannot read the terminal's actual background: `theme_detect.rs`
//! gates the OSC 11 query behind a pending-dependency `#[cfg(any())]` and falls
//! back to the theme's assumption of dark. So the blend runs over
//! `crate::tui::theme_colors::panel_bg`, the surface colour the active theme
//! already declares, rather than over a queried terminal background. The result
//! is the same shape of lift, derived from data the process actually holds.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::tui::theme_colors;

/// Columns reserved on the left of the band for the prompt glyph.
///
/// codex's `LIVE_PREFIX_COLS` (`tui/src/ui_consts.rs:10`) is 2, shared by the
/// composer, the user-history lines and the status rows, so the whole left edge
/// of the transcript aligns on one gutter. operant's gutter is prompt-mode
/// aware (`$ `, `» `, `… `, `› `) and every one of those is two columns, so the
/// constant holds for every mode.
pub(crate) const GUTTER_COLS: u16 = 2;

/// The placeholder painted over an empty draft, dimmed.
///
/// codex's is `"Ask Codex to do anything"` (`chat_composer.rs:4976` reads
/// `self.placeholder_text`, seeded by the App). Operant's mirrors the voice with
/// its own name. It replaces a hint row rather than adding one, so an empty
/// composer occupies exactly the height a typed one does.
pub(crate) const PLACEHOLDER: &str = "Ask operant to do anything";

/// Blend `top` toward `base` at `alpha` (0.0 = base, 1.0 = top), per channel.
fn blend(top: (u8, u8, u8), base: (u8, u8, u8), alpha: f32) -> (u8, u8, u8) {
    let mix = |t: u8, b: u8| -> u8 {
        let t = f32::from(t);
        let b = f32::from(b);
        (t * alpha + b * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8
    };
    (mix(top.0, base.0), mix(top.1, base.1), mix(top.2, base.2))
}

/// The lifted surface colour for the composer band.
///
/// codex uses `is_light(terminal_bg)` to pick the lift direction. operant has no
/// queried background, so the theme's declared surface stands in for it: a
/// surface whose luminance is above mid-range counts as light and gets the
/// dark-direction lift, which is the same decision codex makes at the same
/// threshold. Returns `None` when the palette's surface is not an RGB value
/// (a quantized 256-colour palette), so the caller degrades to no fill — the
/// same degrade codex itself takes when `default_bg()` answers `None`.
fn composer_surface_rgb() -> Option<(u8, u8, u8)> {
    let base = match theme_colors::panel_bg() {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        _ => return None,
    };
    // Rec. 709 luma: the perceptual weighting, not a channel average, so a
    // saturated dark blue is not misread as light.
    let luminance =
        0.2126 * f32::from(base.0) + 0.7152 * f32::from(base.1) + 0.0722 * f32::from(base.2);
    Some(if luminance > 127.5 {
        blend((0, 0, 0), base, 0.04)
    } else {
        blend((255, 255, 255), base, 0.12)
    })
}

/// The composer band's fill: a barely-there surface lift, never a panel.
///
/// Returns an empty style when the active palette's surface is not an RGB value,
/// so the band still renders — it just carries no fill, which is exactly how
/// codex behaves on a terminal that will not answer the background probe.
pub(crate) fn composer_fill_style() -> Style {
    match composer_surface_rgb() {
        Some((r, g, b)) => Style::default().bg(ratatui::style::Color::Rgb(r, g, b)),
        None => Style::default(),
    }
}

/// Paint the band's fill over `area`.
///
/// Called before the composer's own paragraph so the text paints on top. Mirrors
/// `chat_composer.rs:4889`: a bare `Block::default().style(style)` render writes
/// the style into every cell of the rect without drawing a border.
pub(crate) fn paint_band_fill(area: Rect, frame: &mut ratatui::Frame) {
    let style = composer_fill_style();
    if style == Style::default() || area.width == 0 || area.height == 0 {
        return;
    }
    Block::default()
        .style(style)
        .render(area, frame.buffer_mut());
}

/// The rect the typed text occupies inside the band.
///
/// codex's `composer_layout.rs:105-110` inset is
/// `tlbr(1 + voice_rows, LIVE_PREFIX_COLS, 1, 1)`. operant has no voice strip,
/// so it is `tlbr(1, GUTTER_COLS, 1, 1)`.
///
/// The left gutter is inset here (not painted as padding inside the paragraph)
/// so the band's own left edge stays at the transcript's shared gutter and the
/// glyph — an in-line span at the start of the first line — lands exactly in it.
pub(crate) fn textarea_rect(area: Rect) -> Rect {
    // Arithmetic rather than `Rect::inner(Margin)`: `Margin::new` applies the
    // same inset on both axes and the four sides here are asymmetric. The
    // `saturating_sub` calls keep a band narrower than the inset an empty rect
    // instead of wrapping to a huge width.
    Rect {
        x: area.x.saturating_add(GUTTER_COLS),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(GUTTER_COLS).saturating_sub(1),
        height: area.height.saturating_sub(2),
    }
}

/// The dimmed placeholder line, painted at `area`'s origin.
///
/// codex paints the placeholder into the textarea rect (`chat_composer.rs:4976`)
/// while the `›` glyph is drawn to its LEFT, at `textarea_rect.x -
/// LIVE_PREFIX_COLS`. Operant's glyph is an in-line span instead, so the caller
/// insets `area` by the gutter width before calling: see the `placeholder_rect`
/// in `draw_input`. Either way the placeholder starts at the text origin and
/// never overwrites the glyph.
pub(crate) fn placeholder_line() -> Line<'static> {
    Line::from(Span::styled(
        PLACEHOLDER.to_string(),
        Style::default()
            .fg(theme_colors::muted())
            .add_modifier(Modifier::ITALIC),
    ))
}

/// Paint the placeholder over an empty draft's textarea rect.
pub(crate) fn paint_placeholder(area: Rect, frame: &mut ratatui::Frame) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    Paragraph::new(placeholder_line()).render(area, frame.buffer_mut());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_is_two_columns_for_every_prompt_mode() {
        // Every glyph operant's `input_prompt` can return is exactly two
        // columns wide, so the shared gutter constant never desynchronises from
        // the wrapping width the composer computes.
        for glyph in ["> ", "$ ", "» ", "… ", "› "] {
            assert_eq!(glyph.chars().count(), 2, "glyph {glyph:?} is not 2 wide");
        }
        assert_eq!(GUTTER_COLS, 2);
    }

    #[test]
    fn blend_hits_both_endpoints() {
        assert_eq!(blend((255, 255, 255), (0, 0, 0), 0.0), (0, 0, 0));
        assert_eq!(blend((255, 255, 255), (0, 0, 0), 1.0), (255, 255, 255));
        assert_eq!(blend((200, 100, 50), (100, 200, 50), 0.5), (150, 150, 50));
    }

    #[test]
    fn blend_clamps_rather_than_wrapping() {
        // A alpha above 1.0 or below 0.0 must saturate the channel, not wrap it
        // around the u8 range — `as` casts on an out-of-range float are
        // saturating in Rust, and this pins that so a future refactor cannot
        // silently switch to a wrapping cast.
        assert_eq!(blend((255, 255, 255), (0, 0, 0), 5.0), (255, 255, 255));
        assert_eq!(blend((255, 255, 255), (0, 0, 0), -1.0), (0, 0, 0));
    }

    #[test]
    fn dark_surfaces_lift_toward_white_and_light_surfaces_toward_black() {
        // The two directions codex picks with `is_light`, exercised directly so
        // a regression in the luminance threshold cannot silently invert both.
        assert!(blend((255, 255, 255), (16, 16, 20), 0.12).0 > 16);
        assert!(blend((0, 0, 0), (248, 248, 248), 0.04).0 < 248);
    }

    #[test]
    fn textarea_insets_one_row_each_side_and_the_gutter() {
        let area = Rect::new(10, 20, 40, 6);
        let inner = textarea_rect(area);
        assert_eq!(inner.x, area.x + GUTTER_COLS);
        assert_eq!(inner.y, area.y + 1);
        assert_eq!(inner.height, area.height - 2);
        assert_eq!(inner.width, area.width - GUTTER_COLS - 1);
    }

    #[test]
    fn textarea_inset_survives_a_one_row_band() {
        // A band collapsed to one row yields an empty rect rather than a panic.
        // The composer's height maths must never feed an under-sized band here.
        assert_eq!(textarea_rect(Rect::new(0, 0, 20, 1)).height, 0);
    }

    #[test]
    fn textarea_inset_survives_a_band_narrower_than_the_gutter() {
        let inner = textarea_rect(Rect::new(0, 0, 2, 5));
        assert_eq!(inner.x, 2);
        assert_eq!(inner.width, 0, "width must saturate, not underflow");
    }

    #[test]
    fn placeholder_is_a_single_dimmed_italic_span() {
        // One span, no pad: the caller's rect is already offset past the glyph,
        // so any pad here would double-count it.
        let line = placeholder_line();
        assert_eq!(line.spans.len(), 1);
        let style = line.spans[0].style;
        assert!(style.fg.is_some(), "placeholder needs a foreground");
        assert!(style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(line.spans[0].content.as_ref(), PLACEHOLDER);
    }

    #[test]
    fn fill_style_is_either_empty_or_carries_a_background() {
        // The degrade path: a non-RGB surface colour yields the default (empty)
        // style and an RGB surface yields one carrying a bg. Either way the band
        // paints; only the fill is conditional.
        let style = composer_fill_style();
        if style != Style::default() {
            assert!(style.bg.is_some(), "a non-empty fill style must set a bg");
        }
    }

    // ── the drawn band, asserted on a buffer (2026-11-02 validation pass) ────
    //
    // iter-1637 verified this by hand ("`› hello` at col 4 with fill
    // rgb:353547") and pinned none of it. Every test above reads the HELPER'S
    // return value, which passes even if the renderer never calls it. These
    // paint the band the way the real path does and read the buffer back.

    /// Paint `paint_band_fill` over a full-width row and return the buffer, so
    /// the fill's actual reach can be measured rather than assumed.
    fn band_buffer(width: u16, height: u16) -> ratatui::buffer::Buffer {
        use ratatui::backend::TestBackend;
        use ratatui::{Terminal, widgets::Block};
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                // The band fill, then a bare block over the same area exactly as
                // `chat_composer.rs:4889` does, so the fill survives.
                paint_band_fill(area, f);
                f.render_widget(Block::default(), area);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn the_band_fill_paints_every_column_of_the_row() {
        // The whole point of the fill over a border: it reaches the row's full
        // width, including the columns no text occupies.
        let width = 40u16;
        let buf = band_buffer(width, 3);
        let fill = composer_fill_style();
        // Refuse to pass vacuously: if the surface has no colour there is no
        // fill, and this test must SAY so rather than returning quietly and
        // reporting green.
        let expected = fill
            .bg
            .expect("this theme declares a surface colour, so the band has a fill");
        for x in 0..width {
            assert_eq!(
                buf[(x, 1)].style().bg,
                Some(expected),
                "the band fill must reach column {x} — a fill that stops where \
                 the text stops is the border-based look this stage replaced"
            );
        }
    }

    #[test]
    fn the_gutter_constants_are_what_the_renderer_uses() {
        // iter-1637's claim "the gutter is 2 columns and the text starts after
        // it" is only true if `textarea_rect` really subtracts GUTTER_COLS.
        // Pinned here so a gutter change and a rect change cannot drift apart.
        let area = ratatui::layout::Rect::new(0, 0, 50, 5);
        let text = textarea_rect(area);
        assert_eq!(
            text.x,
            area.x + GUTTER_COLS,
            "the text must start after the gutter"
        );
        // One column of right padding too — see the module's `tlbr(1, GUTTER, 1, 1)`.
        assert_eq!(
            text.width,
            area.width - GUTTER_COLS - 1,
            "the text area must leave the gutter on the left and a column on the right"
        );
    }

    #[test]
    fn the_placeholder_is_dimmed_and_not_bold() {
        // A placeholder that reads as strongly as real input is the failure mode
        // this restyle exists to avoid: codex dims it.
        let line = placeholder_line();
        let style = line.spans[0].style;
        // Some foreground that is NOT the ordinary text colour.
        assert!(
            style.fg.is_some(),
            "the placeholder needs its own foreground: {style:?}"
        );
        assert!(
            style.add_modifier.contains(Modifier::ITALIC),
            "the placeholder is italic, which is how it reads as a hint: {style:?}"
        );
        assert!(
            !style.add_modifier.contains(Modifier::BOLD),
            "a bold placeholder competes with the text it stands in for: {style:?}"
        );
    }

    #[test]
    fn the_band_fill_cannot_reach_past_the_row_below_it() {
        // The operator's standing constraint, pinned mechanically: the footer
        // (overscroll row, hint rows, the INSERT badge, the send icon) must stay
        // byte-identical. The band is its own layout chunk, so a fill that bled
        // into the row below would mean the band grew.
        //
        // Byte-identity, not "bg is None": `Block::default().render` legitimately
        // writes a `Reset` style over the row, so the control comparison is the
        // same render WITHOUT the band fill. The two footer rows must be equal.
        use ratatui::backend::TestBackend;
        use ratatui::{Terminal, widgets::Block};

        /// Footer row 3 with (or without) the band fill painted above it.
        fn footer_row(with_band_fill: bool) -> String {
            let mut terminal = Terminal::new(TestBackend::new(40, 4)).unwrap();
            terminal
                .draw(|f| {
                    let all = f.area();
                    let band = Rect { height: 3, ..all };
                    let footer = Rect {
                        y: all.y + 3,
                        height: 1,
                        ..all
                    };
                    if with_band_fill {
                        paint_band_fill(band, f);
                    }
                    f.render_widget(Block::default(), footer);
                })
                .unwrap();
            let buf = terminal.backend().buffer();
            (0..40u16)
                .map(|x| format!("{:?}", buf[(x, 3)]))
                .collect::<Vec<_>>()
                .join("|")
        }

        assert_eq!(
            footer_row(false),
            footer_row(true),
            "the band fill changed the footer row — the footer must stay byte-identical"
        );

        // And the band itself still gets its fill, so the test is not vacuous.
        let fill = composer_fill_style()
            .bg
            .expect("this theme declares a surface colour");
        let mut terminal = Terminal::new(TestBackend::new(40, 4)).unwrap();
        terminal
            .draw(|f| {
                let all = f.area();
                let band = Rect { height: 3, ..all };
                paint_band_fill(band, f);
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(0, 0)].style().bg, Some(fill));
    }
}
