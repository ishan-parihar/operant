// Bar separator vocabulary, transcribed from oh-my-pi
// `packages/tui/src/status-line/separators.ts` + the `sep.*` glyph table in
// `packages/tui/src/theme/symbols.ts` (plan 2026-11-14 U7).
//
// Operant's status row had no separator vocabulary at all: every boundary was
// an ad-hoc `format!(" · {label}")` inside `draw_status`, so the separator
// could not be changed in one place and a powerline style was impossible.
//
// The port is deliberately small. OMP's `SeparatorDef` carries `endCaps` with a
// `useBgAsFg` flag because its renderer paints caps by swapping background for
// foreground. A ratatui `Span` takes both colours directly, so a cap here is
// just a glyph plus the two colours it needs — the flag disappears rather than
// being emulated.

/// How segments are joined, and what caps the ends of the bar.
///
/// Transcribed and tested, but not yet DRAWN: `draw_status` still joins its
/// boundaries with 51 hand-written ` · ` literals, and converting them is U8's
/// segment model. The table is complete now so U8 is only a rewiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorStyle {
    /// Hard powerline glyphs (U+E0B0 / U+E0B2), the classic look.
    Powerline,
    /// Thin powerline glyphs (U+E0B1 / U+E0B3).
    PowerlineThin,
    /// ` / ` between segments.
    Slash,
    /// ` │ ` between segments.
    Pipe,
    /// `█` between segments.
    Block,
    /// A single space: segments touch with no visible rule.
    None,
    /// ASCII `>` / `<`, for a terminal with no powerline glyphs.
    Ascii,
}

#[allow(dead_code)] // read by U8's segment assembly; tested here from day one
impl SeparatorStyle {
    /// Every style, in the order a picker should list them.
    pub const ALL: &'static [SeparatorStyle] = &[
        SeparatorStyle::Powerline,
        SeparatorStyle::PowerlineThin,
        SeparatorStyle::Slash,
        SeparatorStyle::Pipe,
        SeparatorStyle::Block,
        SeparatorStyle::None,
        SeparatorStyle::Ascii,
    ];

    /// The config key, matching OMP's `StatusLineSeparatorStyle` spelling.
    pub fn key(self) -> &'static str {
        match self {
            SeparatorStyle::Powerline => "powerline",
            SeparatorStyle::PowerlineThin => "powerline-thin",
            SeparatorStyle::Slash => "slash",
            SeparatorStyle::Pipe => "pipe",
            SeparatorStyle::Block => "block",
            SeparatorStyle::None => "none",
            SeparatorStyle::Ascii => "ascii",
        }
    }

    /// Parse a config key. Unknown values fall back to `PowerlineThin`, which
    /// is OMP's documented fallback for an unknown style too.
    pub fn from_key(key: &str) -> SeparatorStyle {
        let normalized = key.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        SeparatorStyle::ALL
            .iter()
            .copied()
            .find(|style| style.key() == normalized)
            .unwrap_or(SeparatorStyle::PowerlineThin)
    }

    /// The glyph drawn between two segments.
    ///
    /// `powerline` and `powerline-thin` use the same glyph in both directions:
    /// the direction is carried by which side of the segment it is drawn on,
    /// and by the cap glyphs at the ends of the bar.
    pub fn separator(self) -> &'static str {
        match self {
            SeparatorStyle::Powerline => "\u{e0b0}",
            SeparatorStyle::PowerlineThin => "\u{e0b1}",
            SeparatorStyle::Slash => " / ",
            SeparatorStyle::Pipe => " │ ",
            SeparatorStyle::Block => "█",
            SeparatorStyle::None => " ",
            SeparatorStyle::Ascii => "|",
        }
    }

    /// The glyph capping the LEFT end of the bar, or `None` for a style with
    /// no caps (OMP gives caps only to the two powerline styles).
    pub fn left_cap(self) -> Option<&'static str> {
        match self {
            SeparatorStyle::Powerline => Some("\u{e0b2}"),
            SeparatorStyle::PowerlineThin => Some("\u{e0b2}"),
            _ => None,
        }
    }

    /// The glyph capping the RIGHT end of the bar.
    pub fn right_cap(self) -> Option<&'static str> {
        match self {
            SeparatorStyle::Powerline => Some("\u{e0b0}"),
            SeparatorStyle::PowerlineThin => Some("\u{e0b0}"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SeparatorStyle;

    /// The table is a transcription, so pin the glyphs: a silent edit here
    /// changes every bar in the UI and nothing else would notice.
    #[test]
    fn separator_glyphs_match_the_reference_table() {
        assert_eq!(SeparatorStyle::Powerline.separator(), "\u{e0b0}");
        assert_eq!(SeparatorStyle::PowerlineThin.separator(), "\u{e0b1}");
        assert_eq!(SeparatorStyle::Slash.separator(), " / ");
        assert_eq!(SeparatorStyle::Pipe.separator(), " │ ");
        assert_eq!(SeparatorStyle::Block.separator(), "█");
        assert_eq!(SeparatorStyle::None.separator(), " ");
        assert_eq!(SeparatorStyle::Ascii.separator(), "|");
    }

    /// Only the two powerline styles have caps, and they point outward.
    #[test]
    fn only_powerline_styles_have_caps() {
        for style in SeparatorStyle::ALL {
            let has_caps = style.left_cap().is_some();
            assert_eq!(
                has_caps,
                matches!(
                    style,
                    SeparatorStyle::Powerline | SeparatorStyle::PowerlineThin
                ),
                "{style:?} cap presence is wrong"
            );
            assert_eq!(style.right_cap().is_some(), has_caps);
        }
    }

    /// An unknown key degrades to the documented fallback rather than to a
    /// style that renders nothing.
    #[test]
    fn an_unknown_key_falls_back_to_powerline_thin() {
        assert_eq!(
            SeparatorStyle::from_key("no-such-style"),
            SeparatorStyle::PowerlineThin
        );
        assert_eq!(
            SeparatorStyle::from_key("powerline"),
            SeparatorStyle::Powerline
        );
        assert_eq!(
            SeparatorStyle::from_key("powerline_thin"),
            SeparatorStyle::PowerlineThin,
            "the key is separator-insensitive, like every other role key"
        );
        assert_eq!(SeparatorStyle::from_key("ASCII"), SeparatorStyle::Ascii);
    }

    /// Every key must round-trip, or a picker built from `ALL` could not be
    /// persisted.
    #[test]
    fn every_key_round_trips() {
        for style in SeparatorStyle::ALL {
            assert_eq!(
                SeparatorStyle::from_key(style.key()),
                *style,
                "{style:?} does not survive key -> style"
            );
        }
    }
}
