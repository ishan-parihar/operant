//! Which glyph draws the boundary between two status-line segments
//! (plan 2026-11-15 U11).
//!
//! [`super::separator`] owns *where* the boundary goes; this owns *what it looks
//! like*. They are separate because the boundary's placement is a property of the
//! bar's structure (which spans are segments) and its glyph is a property of the
//! operator's taste.
//!
//! ## Restored, with the caps and the edge they need
//!
//! The style table is transcribed from OMP
//! (`packages/tui/src/status-line/separators.ts:4-41` + the `sep.*` glyph table in
//! `packages/tui/src/theme/symbols.ts:460-469`).
//!
//! An earlier version of this table also carried `left_cap` / `right_cap` and was
//! DELETED at `debe7541` for having no caller. They are restored here (U14) because
//! the edge they need now exists: `bar::fill_style` is applied to the whole status
//! `Rect`, so the bar's visual right edge is the rect's last column even though the
//! text inside it is short. A cap therefore sits at `area.right() - 1` without the
//! text having to be padded to the edge first.
//!
//! The caps are drawn ONLY for the two powerline styles, which is what OMP does:
//! the other six have no cap glyph in the reference table, and inventing one would
//! be a style the reference does not define.
//!
//! ## Why the default is `Dot` and not OMP's `powerline-thin`
//!
//! OMP falls back to `powerline-thin` for an unknown style
//! (`separators.ts:36`). This module deliberately does not, for a measured reason:
//! six ` · ` separators in `ui_input.rs` are **prose**, not boundaries (see
//! [`super::separator`]'s table), and they keep the literal. A powerline default
//! would draw `\u{e0b1}` at the 15 boundaries and ` · ` inside the six prose
//! strings, so the same bar would speak two separator languages at once and the
//! prose would read as corruption. `Dot` is byte-identical to what shipped before
//! this module, so adopting the key changes nothing until an operator picks a style.

use std::sync::RwLock;

/// How the boundary between two segments is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorStyle {
    /// The historical `" · "`: the default, and what every bar drew before U11.
    Dot,
    /// Hard powerline glyphs (U+E0B0), the classic look. Needs a patched font.
    Powerline,
    /// Thin powerline glyphs (U+E0B1). Needs a patched font.
    PowerlineThin,
    /// ` / ` between segments.
    Slash,
    /// ` │ ` between segments.
    Pipe,
    /// `█` between segments.
    Block,
    /// A single space: segments touch with no visible rule.
    None,
    /// ASCII `|`, for a terminal with no powerline glyphs.
    Ascii,
}

impl SeparatorStyle {
    /// Every style, in the order a picker should list them.
    pub const ALL: &'static [SeparatorStyle] = &[
        SeparatorStyle::Dot,
        SeparatorStyle::Powerline,
        SeparatorStyle::PowerlineThin,
        SeparatorStyle::Slash,
        SeparatorStyle::Pipe,
        SeparatorStyle::Block,
        SeparatorStyle::None,
        SeparatorStyle::Ascii,
    ];

    /// The config key, matching OMP's `StatusLineSeparatorStyle` spelling plus
    /// `dot` for the historical look.
    pub fn key(self) -> &'static str {
        match self {
            SeparatorStyle::Dot => "dot",
            SeparatorStyle::Powerline => "powerline",
            SeparatorStyle::PowerlineThin => "powerline-thin",
            SeparatorStyle::Slash => "slash",
            SeparatorStyle::Pipe => "pipe",
            SeparatorStyle::Block => "block",
            SeparatorStyle::None => "none",
            SeparatorStyle::Ascii => "ascii",
        }
    }

    /// Parse a config key, reporting whether it named a known style.
    ///
    /// An unknown key degrades to [`SeparatorStyle::Dot`] — the look that shipped
    /// before this key existed — rather than to a style that renders nothing. That
    /// is a deliberate divergence from OMP's `powerline-thin` fallback; see the
    /// module docs.
    ///
    /// The `bool` is the warning signal: a caller that silently accepted a typo
    /// would leave the operator believing they had chosen a style. There is no
    /// infallible variant on purpose, so a call site cannot forget the check.
    pub fn parse(key: &str) -> (SeparatorStyle, bool) {
        let normalized = key.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        match SeparatorStyle::ALL
            .iter()
            .copied()
            .find(|style| style.key() == normalized)
        {
            Some(style) => (style, true),
            None => (SeparatorStyle::Dot, false),
        }
    }

    /// The glyph capping the LEFT end of the bar, or `None` for a style with no
    /// caps (OMP gives caps only to the two powerline styles).
    ///
    /// A cap is drawn in the fill colour as its own ink, so it reads as the bar's
    /// edge rather than as content — see [`super::bar::cap_style`].
    pub fn left_cap(self) -> Option<&'static str> {
        match self {
            SeparatorStyle::Powerline | SeparatorStyle::PowerlineThin => Some("\u{e0b2}"),
            _ => None,
        }
    }

    /// The glyph capping the RIGHT end of the bar.
    pub fn right_cap(self) -> Option<&'static str> {
        match self {
            SeparatorStyle::Powerline | SeparatorStyle::PowerlineThin => Some("\u{e0b0}"),
            _ => None,
        }
    }

    /// The glyph drawn between two segments, with its surrounding spaces.
    ///
    /// The spaces are part of the value, not added by the caller: a boundary is
    /// `" · "` and not `·`, and every style here carries its own padding so a
    /// caller cannot forget it.
    pub fn glyph(self) -> &'static str {
        match self {
            SeparatorStyle::Dot => " · ",
            SeparatorStyle::Powerline => "\u{e0b0}",
            SeparatorStyle::PowerlineThin => "\u{e0b1}",
            SeparatorStyle::Slash => " / ",
            SeparatorStyle::Pipe => " │ ",
            SeparatorStyle::Block => "█",
            SeparatorStyle::None => " ",
            SeparatorStyle::Ascii => "|",
        }
    }
}

/// The style an unconfigured cell resolves to.
///
/// Implemented rather than derived so the default cannot drift from
/// [`SeparatorStyle::Dot`] without a compile error at the one site that names it.
impl Default for SeparatorStyle {
    fn default() -> Self {
        SeparatorStyle::Dot
    }
}

/// The configured style.
///
/// A process-global cell with ONE writer, mirroring `theme_colors::ROLE_OVERRIDES`:
/// the TUI reads it per frame at the paint site, and there is no per-frame config
/// lookup to keep in step. The previous value is restored by the caller on drop
/// where a test changes it.
static STYLE: RwLock<Option<SeparatorStyle>> = RwLock::new(None);

/// Install the configured style. `None` restores the default.
pub fn set_style(style: Option<SeparatorStyle>) {
    if let Ok(mut slot) = STYLE.write() {
        *slot = style;
    }
}

/// The style in force: the configured one, else the default.
pub fn style() -> SeparatorStyle {
    STYLE
        .read()
        .ok()
        .and_then(|slot| *slot)
        .unwrap_or(SeparatorStyle::Dot)
}

/// A one-line sample of what `style` draws, for a command's reply.
///
/// (plan 2026-11-16 T3) WHY THIS EXISTS. `/bar <style>` applied correctly and
/// the operator still reported it "not working properly" — because on an idle
/// session the status row has ZERO separators. A separator only exists BETWEEN
/// two segments, and `draw_status` yields a single empty span when idle; the
/// composer hint adds one boundary, but only while a tip is showing, which is
/// 12 seconds of every 90 (`STATUS_TIP_SHOW_SECONDS` / `_PERIOD_SECONDS`).
///
/// So for most of the time the honest result of a style change is "nothing
/// moved on screen", and a reply that named only the key gave the operator no
/// way to tell a working change from a dead command. Showing the glyph makes
/// the command's own output the evidence that it took.
///
/// The sample is `a<glyph>b` rather than a bare glyph: three of the eight
/// styles (`slash`, `pipe`, `ascii`) are punctuation that reads as a typo
/// without a neighbour to sit between, and `none` is a single space, which
/// would render as an empty gap in the notice.
pub fn preview(style: SeparatorStyle) -> String {
    format!("a{}b", style.glyph())
}

/// Parse an operator-typed style name, or `None` if it names none.
///
/// (plan 2026-11-16 B1) The strict twin of [`SeparatorStyle::parse`], which
/// answers "the default, and you were wrong" so a config loader can degrade
/// silently. A command that TYPED a value must not degrade: an operator who
/// runs `/bar powerlin` and gets `dot` back has been told their input was fine
/// when it was not, and has no way to learn the spelling.
///
/// The name is normalised the same way the loader normalises it (trimmed,
/// lower-cased, `_` and space folded to `-`), so `/bar Powerline` and
/// `/bar powerline` agree with what the TOML accepts.
pub fn parse_strict(key: &str) -> Option<SeparatorStyle> {
    let normalized = key.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    SeparatorStyle::ALL
        .iter()
        .copied()
        .find(|style| style.key() == normalized)
}

/// The accepted style names for a message, in catalogue order.
pub fn vocabulary() -> String {
    SeparatorStyle::ALL
        .iter()
        .map(|s| s.key())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores the previous style so one test cannot poison the next — the
    /// process-global rule this module's `STYLE` cell lives under.
    struct Restore(Option<SeparatorStyle>);

    impl Drop for Restore {
        fn drop(&mut self) {
            set_style(self.0);
        }
    }

    fn with_style(next: Option<SeparatorStyle>) -> Restore {
        let previous = style();
        set_style(next);
        // `style()` resolves the cell to a concrete value, so the cell's own
        // previous state (which may have been "unset") is what a test must
        // restore to keep the default reachable for the next one.
        Restore(Some(previous))
    }

    /// The table is a transcription, so pin the glyphs: a silent edit here
    /// changes every bar in the UI and nothing else would notice.
    #[test]
    fn separator_glyphs_match_the_reference_table() {
        assert_eq!(SeparatorStyle::Dot.glyph(), " · ");
        assert_eq!(SeparatorStyle::Powerline.glyph(), "\u{e0b0}");
        assert_eq!(SeparatorStyle::PowerlineThin.glyph(), "\u{e0b1}");
        assert_eq!(SeparatorStyle::Slash.glyph(), " / ");
        assert_eq!(SeparatorStyle::Pipe.glyph(), " │ ");
        assert_eq!(SeparatorStyle::Block.glyph(), "█");
        assert_eq!(SeparatorStyle::None.glyph(), " ");
        assert_eq!(SeparatorStyle::Ascii.glyph(), "|");
    }

    /// The default must be the historical look, byte for byte, or merely
    /// installing this key would move every golden in the corpus.
    #[test]
    fn the_default_is_the_historical_dot() {
        assert_eq!(SeparatorStyle::Dot.glyph(), " · ");
        assert_eq!(
            SeparatorStyle::parse("no-such-style").0,
            SeparatorStyle::Dot
        );
        assert_eq!(SeparatorStyle::default(), SeparatorStyle::Dot);
    }

    /// Every key must round-trip, or a picker built from `ALL` could not be
    /// persisted.
    #[test]
    fn every_key_round_trips() {
        for style in SeparatorStyle::ALL {
            assert_eq!(
                SeparatorStyle::parse(style.key()).0,
                *style,
                "{style:?} does not survive key -> style"
            );
        }
    }

    /// The key is separator- and case-insensitive, like every role key.
    #[test]
    fn the_key_is_normalized_like_every_role_key() {
        assert_eq!(
            SeparatorStyle::parse("powerline_thin").0,
            SeparatorStyle::PowerlineThin
        );
        assert_eq!(SeparatorStyle::parse(" ASCII ").0, SeparatorStyle::Ascii);
        assert_eq!(
            SeparatorStyle::parse("Powerline").0,
            SeparatorStyle::Powerline
        );
    }

    /// Only the two powerline styles have caps, and they point outward: `\u{e0b2}`
    /// on the left, `\u{e0b0}` on the right. A style that grew caps here would be
    /// drawing a glyph the reference does not define for it.
    #[test]
    fn only_powerline_styles_have_caps() {
        for style in SeparatorStyle::ALL {
            let has = style.left_cap().is_some();
            assert_eq!(
                has,
                matches!(
                    style,
                    SeparatorStyle::Powerline | SeparatorStyle::PowerlineThin
                ),
                "{style:?} cap presence is wrong"
            );
            assert_eq!(style.right_cap().is_some(), has);
        }
        assert_eq!(SeparatorStyle::Powerline.left_cap(), Some("\u{e0b2}"));
        assert_eq!(SeparatorStyle::Powerline.right_cap(), Some("\u{e0b0}"));
    }

    /// No style renders an empty boundary: an empty glyph would weld two
    /// segments into one word with no way to see where the split is.
    #[test]
    fn no_style_renders_an_empty_boundary() {
        for style in SeparatorStyle::ALL {
            assert!(!style.glyph().is_empty(), "{style:?} renders nothing");
        }
    }

    /// A style that was never installed resolves to the default.
    ///
    /// This is the whole unconfigured-cell claim, expressed as the resolution the
    /// paint site reads rather than as a read of the cell: the cell is
    /// process-global and these tests run in parallel, so an assertion that some
    /// OTHER test has not written it is a race, not a contract. `set_style(None)`
    /// is the documented "restore the default" path and is what this exercises.
    #[test]
    fn an_unset_cell_resolves_to_the_default() {
        let _guard = with_style(None);
        set_style(None);
        assert_eq!(style(), SeparatorStyle::Dot);
    }
}
