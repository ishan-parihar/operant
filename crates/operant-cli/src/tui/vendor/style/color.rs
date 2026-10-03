// Vendored from jcode (crates/jcode-tui-style), MIT License, Copyright (c) 2025 Jeremy Huang.
// Adapted for operant: rewrote two doc comments that named the jcode app-core
// module (`jcode_app_core::perf`) to describe the TUI app layer instead.
// Wave 0 (docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md item 3): deleted the
// vendored color-capability detector (`ColorCapability`, `color_capability`,
// `detect_color_capability`, `detect_raw_color_capability`,
// `fragile_glyph_cache_terminal`, `has_truecolor`, `pin_truecolor_for_tests`)
// and the `rgb()` quantization branch, which together ran a SECOND depth
// probe (`COLORTERM`/`TERM`/terminal-name heuristics) alongside operant's
// `tui::color_depth::detect`. jcode's model is one palette with no depth
// negotiation, so `rgb()` now always returns `Color::Rgb`; terminals with
// less colour get their quantization from `color_depth::quantize_palette`
// on the operant theme (`set_active_theme`), never here. This also removes
// the `JCODE_GLYPH_SAFE_MODE` env var (upstream issue #330's glyph-atlas
// downgrade), which is jcode residue: nothing in operant set or read it.
// The xterm-256 quantizer helpers went with it; `indexed_to_rgb` survives
// because `theme_mode::color_rgb` maps indexed and named terminal colours
// through it.

use ratatui::style::Color;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

pub fn clear_buf(area: Rect, buf: &mut Buffer) {
    for x in area.left()..area.right() {
        for y in area.top()..area.bottom() {
            buf[(x, y)].reset();
        }
    }
}

/// Build a renderable color from an RGB literal.
///
/// One palette, no depth negotiation: the returned color is always the
/// `Color::Rgb` triple. Operant's own depth handling lives in
/// `crate::tui::color_depth` and quantizes the *operant* theme palette at
/// `set_active_theme`; this vendored layer carries no second opinion about
/// the terminal's depth, so a single frame can never mix `rgb:` and
/// `indexed:` encodings of the same colour.
///
/// User color configuration is *not* applied here. It is applied once per
/// frame at the buffer level (`theme_mode::adapt_buffer_for_display`) so a color
/// can never be remapped twice. See `palette` for why that choke point is the
/// single place colors are substituted.
#[inline]
pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the Wave-0 invariant. The deleted second detector used to
    /// rewrite `rgb()` to `Color::Indexed` on a 256-colour terminal, which
    /// let one frame carry two encodings of the same colour (the exact
    /// defect `align_vendored_detector` used to paper over). If this test
    /// fails, someone reintroduced a depth negotiation into the vendored
    /// layer - fix that, not the assertion.
    ///
    /// No env manipulation: `rgb` is a one-line function that reads no
    /// environment at all, and mutating process-global env from a test that
    /// shares a process with other style tests would be contamination, not
    /// proof. The literals below are the checked values.
    #[test]
    fn rgb_is_never_quantized_regardless_of_terminal() {
        // Pinned to literals, not derived from the code under test.
        assert_eq!(rgb(204, 155, 31), Color::Rgb(204, 155, 31));
        assert_eq!(rgb(0, 0, 0), Color::Rgb(0, 0, 0));
        assert_eq!(rgb(255, 87, 51), Color::Rgb(255, 87, 51));
    }
}

pub fn indexed_to_rgb(idx: u8) -> (u8, u8, u8) {
    if idx >= 232 {
        let v = 8 + (idx - 232) * 10;
        (v, v, v)
    } else if idx >= 16 {
        // The xterm-256 color cube: indices 16-231 map to a 6x6x6 RGB cube.
        // Each axis uses values: 0, 95, 135, 175, 215, 255 (indices 0-5).
        const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let idx = idx - 16;
        (
            CUBE_VALUES[(idx / 36) as usize],
            CUBE_VALUES[((idx / 6) % 6) as usize],
            CUBE_VALUES[(idx % 6) as usize],
        )
    } else {
        match idx {
            0 => (0, 0, 0),
            1 => (128, 0, 0),
            2 => (0, 128, 0),
            3 => (128, 128, 0),
            4 => (0, 0, 128),
            5 => (128, 0, 128),
            6 => (0, 128, 128),
            7 => (192, 192, 192),
            8 => (128, 128, 128),
            9 => (255, 0, 0),
            10 => (0, 255, 0),
            11 => (255, 255, 0),
            12 => (0, 0, 255),
            13 => (255, 0, 255),
            14 => (255, 255, 255),
            _ => (255, 255, 255),
        }
    }
}
