//! The meter primitive (plan 2026-11-14 U9).
//!
//! `█`/`░` was implemented three times, with three different rounding rules for
//! the same input:
//!
//! | site | filled | rounding |
//! |---|---|---|
//! | `context_viz.rs:96` | `ctx_pct * width` | truncate (`.min(width)`) |
//! | `usage_overlay.rs:367` | `percent/100 * width` | `.round()` |
//! | `markdown_wrap.rs:542` | `progress * width` | truncate |
//!
//! So the same fraction drew a different number of cells depending on which
//! surface you looked at, and at some widths the three disagreed by a cell.
//! That is the defect this closes: one implementation, one rounding rule.
//!
//! The rule chosen is round-half-up on the filled count, clamped to the width:
//! it is what the percentage readout beside each bar already implies, so the bar
//! and its label agree.

/// The glyphs a meter is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterStyle {
    /// The glyph for one filled cell.
    pub filled: char,
    /// The glyph for one empty cell.
    pub empty: char,
}

impl MeterStyle {
    /// The `█`/`░` pair all three call sites used.
    pub const BLOCK: MeterStyle = MeterStyle {
        filled: '\u{2588}',
        empty: '\u{2591}',
    };
}

/// Render `fraction` (0.0–1.0) as a meter `width` cells wide.
///
/// The fraction is clamped, so an out-of-range value degrades to an empty or
/// full meter rather than panicking on a `repeat` underflow — which is what the
/// three hand-written versions each had to remember to do separately.
pub fn meter(fraction: f32, width: usize, style: MeterStyle) -> String {
    if width == 0 {
        return String::new();
    }
    let clamped = fraction.clamp(0.0, 1.0);
    let filled = ((clamped * width as f32).round() as usize).min(width);
    let empty = width - filled;

    std::iter::repeat_n(style.filled, filled)
        .chain(std::iter::repeat_n(style.empty, empty))
        .collect()
}

/// [`meter`] for a percentage in 0–100, which is how two of the three call sites
/// held the value.
pub fn meter_percent(percent: f32, width: usize, style: MeterStyle) -> String {
    meter(percent / 100.0, width, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rounding rule, pinned. Two of the three sites truncated, so this is
    /// the assertion that catches a silent return to the old disagreement.
    #[test]
    fn the_filled_count_rounds_half_up() {
        // 0.5 of 5 cells is 2.5, which rounds to 3 (truncation gave 2).
        assert_eq!(meter(0.5, 5, MeterStyle::BLOCK).chars().count(), 5);
        assert_eq!(
            meter(0.5, 5, MeterStyle::BLOCK)
                .chars()
                .filter(|c| *c == MeterStyle::BLOCK.filled)
                .count(),
            3,
            "0.5 of 5 must round to 3 cells, not truncate to 2"
        );
    }

    /// The bar is always exactly `width` cells, whatever the fraction.
    #[test]
    fn the_meter_is_always_exactly_the_requested_width() {
        for width in [0usize, 1, 3, 7, 40] {
            for fraction in [-1.0f32, 0.0, 0.25, 0.5, 0.99, 1.0, 2.0] {
                assert_eq!(
                    meter(fraction, width, MeterStyle::BLOCK).chars().count(),
                    width,
                    "fraction {fraction} at width {width}"
                );
            }
        }
    }

    /// Out-of-range input degrades instead of panicking.
    #[test]
    fn out_of_range_fractions_clamp() {
        assert_eq!(meter(-5.0, 4, MeterStyle::BLOCK), "░░░░");
        assert_eq!(meter(9.0, 4, MeterStyle::BLOCK), "████");
    }

    /// The percentage wrapper agrees with the fraction form.
    #[test]
    fn the_percentage_form_matches_the_fraction_form() {
        for percent in [0.0f32, 12.5, 50.0, 99.9, 100.0] {
            assert_eq!(
                meter_percent(percent, 20, MeterStyle::BLOCK),
                meter(percent / 100.0, 20, MeterStyle::BLOCK)
            );
        }
    }

    /// A zero-width meter is empty, not a panic.
    #[test]
    fn a_zero_width_meter_is_empty() {
        assert_eq!(meter(0.5, 0, MeterStyle::BLOCK), "");
    }

    /// The style is honoured, not hardcoded to the block pair.
    #[test]
    fn the_style_chooses_the_glyphs() {
        let shade = MeterStyle {
            filled: '\u{2593}',
            empty: '\u{2591}',
        };
        assert_eq!(meter(0.5, 2, shade), "▓░");
    }
}
