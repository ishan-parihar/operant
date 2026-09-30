// color_depth.rs — terminal colour-depth detection and palette quantization.
//!
//! The palettes in [`crate::tui::theme_colors`] are authored as
//! `Color::Rgb(…)` because that is the only lossless representation. Emitting
//! those triples to a terminal that can display 8 or 256 colours makes the
// emulator snap each one to the nearest palette entry it happens to hold, so a
// dark theme's carefully separated shades collapse into a handful of flat
// steps. This module answers "how much colour does this terminal actually
// have?" once, and quantizes the palette down to it.
//
// # Where quantization is applied
//
// [`quantize_palette`] runs inside
//! [`crate::tui::theme_colors::set_active_theme`], i.e. the single write path
//! for the process-wide palette. The 14 palette accessors therefore keep
//! reading a pre-quantized table: an accessor is a field read under a read
// lock, never a quantization pass. That is the whole point — the accessors are
// read once per styled span, thousands of times per frame, so the xterm search
// must not run there.
//
// # Override
//
// Set `OPERANT_COLOR_DEPTH` to one of `truecolor` / `24bit` / `256` /
// `256color` / `16` / `16color` / `none` / `mono` to skip detection. It is read
// once, by the first [`detect`] call (which is the first
//! `set_active_theme` call, at startup), so a wrong guess is fixable without
//! a rebuild. An unrecognised value is ignored and detection proceeds — a typo
//! must not cost the user their terminal.

use std::sync::OnceLock;

use ratatui::style::Color;

use crate::tui::theme_colors::ColorPalette;

/// Env var that forces the detected depth, overriding the environment probe.
///
/// Values: `truecolor` | `24bit` | `256` | `256color` | `16` | `16color` |
/// `none` | `mono`. An unknown value falls through to auto-detection.
pub const COLOR_DEPTH_OVERRIDE_VAR: &str = "OPERANT_COLOR_DEPTH";

/// How much colour the attached terminal can actually display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit colour: `Color::Rgb` is emitted untouched.
    Truecolor,
    /// 256 colours: the xterm 6x6x6 cube plus the 24-step grey ramp.
    Palette256,
    /// The 16 base ANSI colours.
    Palette16,
    /// `NO_COLOR` was set: emit no colour at all.
    None,
}

/// The xterm 6x6x6 colour-cube levels, and the base-16 SGR palette, in SGR
/// order 0..=15. Both tables are the standard xterm ones; the cube level
/// spacing is deliberately uneven (0/95/135/175/215/255), which is why
/// quantizing with `value * 5 / 255` is wrong.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

const BASE16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The depth this process renders for, detected at most once.
///
/// Seeded lazily by the first [`detect`] — the first
/// `set_active_theme`, which `App::new` runs before the first frame.
static DETECTED: OnceLock<ColorDepth> = OnceLock::new();

/// The terminal's colour depth, detected once and cached for the process.
pub fn detect() -> ColorDepth {
    *DETECTED.get_or_init(|| {
        let depth = detect_from(|key| std::env::var(key).ok());
        align_vendored_detector(depth);
        depth
    })
}

/// Make the vendored design system's depth answer agree with ours.
///
/// The vendored jcode style layer runs a SECOND, independent depth detection:
/// [`crate::tui::vendor::style::color::color_capability`]. Unlike [`detect`],
/// it never consults [`COLOR_DEPTH_OVERRIDE_VAR`] — it reads `COLORTERM`,
/// `TERM` and terminal-name heuristics and stops there. Its answer reaches the
/// frame through [`crate::tui::vendor::style::color::rgb`], which every palette
/// substitution funnels through (`palette::remap_named_with` for named colours,
/// `configured_native_color` for concrete ones), so a single frame could carry
/// TWO encodings: the operant palette quantized to truecolor while one cell the
/// per-frame buffer pass rewrote came back `Color::Indexed`.
///
/// That is not hypothetical. With `OPERANT_COLOR_DEPTH=truecolor` on a
/// `TERM=xterm-256color` process, `[Esc to dismiss]` in the voice-mode notice
/// painted `Color::DarkGray` → `Role::Dim` → `muted` → `color::rgb(204,155,31)`
/// → `Color::Indexed(172)`, while every neighbouring cell stayed `rgb:`. Same
/// colour, two encodings, in one 120x40 grid — and which encoding you got
/// depended on the terminal rather than on the documented override.
///
/// Fixing it here — at the single place depth is resolved, so no draw path can
/// observe a different depth — needs no vendor edit:
/// [`crate::tui::vendor::style::color::pin_truecolor_for_tests`] is the
/// vendored module's own published seam for forcing its answer to TrueColor,
/// and it is the only disagreement that changes an ENCODING. At `Palette256`
/// and `Palette16` both pipelines already emit `Color::Indexed`, so the frame
/// is encoding-uniform without help.
///
/// Deliberately NOT done: editing the vendored file. Its own doc comment still
/// says production never calls the pin; that comment is now narrower than
/// reality and belongs upstream, not in a vendored copy we must not touch.
fn align_vendored_detector(depth: ColorDepth) {
    if depth == ColorDepth::Truecolor {
        crate::tui::vendor::style::color::pin_truecolor_for_tests();
    }
}

/// Decide the depth purely from an environment lookup.
///
/// `get` is a closure over the environment rather than a direct
/// `std::env::var` call so the decision table is testable without mutating
/// the real process environment (which is global, and racy under `cargo
/// test`). Decision order:
///
/// 1. `NO_COLOR` (non-empty) — emit no colour. Wins over everything.
/// 2. `OPERANT_COLOR_DEPTH` — an explicit user override.
/// 3. `TERM` is `screen*` / `tmux*` — cap at 256. A multiplexer only forwards
///    24-bit when its `Tc` capability was negotiated, which is not observable
///    from here, so 256 is the safe answer even when `COLORTERM` says
///    truecolor. Downgrading is safe; upgrading is not.
/// 4. `COLORTERM` is `truecolor` / `24bit`.
/// 5. `TERM` contains `direct` or `truecolor` (e.g. `xterm-direct`).
/// 6. `TERM` contains `256color` (covers `rxvt-unicode-256color`).
/// 7. `TERM` names a terminal that is truecolor but does not advertise it
///    (`kitty`, `ghostty`, `wezterm`, `alacritty`, `foot`, `contour`).
/// 8. `TERM` starts with `rxvt` — the plain 88-colour variant: 16.
/// 9. Anything else, including `TERM=dumb` and an unset `TERM`: 16.
pub fn detect_from(get: impl Fn(&str) -> Option<String>) -> ColorDepth {
    let var = |key: &str| get(key).map(|v| v.trim().to_ascii_lowercase());

    if let Some(no_color) = var("NO_COLOR")
        && !no_color.is_empty()
    {
        return ColorDepth::None;
    }

    if let Some(forced) = var(COLOR_DEPTH_OVERRIDE_VAR)
        && let Some(depth) = parse_depth(&forced)
    {
        return depth;
    }

    let term = var("TERM").unwrap_or_default();

    if is_multiplexer(&term) {
        return ColorDepth::Palette256;
    }

    if let Some(colorterm) = var("COLORTERM")
        && matches!(colorterm.as_str(), "truecolor" | "24bit")
    {
        return ColorDepth::Truecolor;
    }

    if term.contains("direct") || term.contains("truecolor") {
        return ColorDepth::Truecolor;
    }

    if term.contains("256color") {
        return ColorDepth::Palette256;
    }

    if [
        "kitty",
        "ghostty",
        "wezterm",
        "alacritty",
        "foot",
        "contour",
    ]
    .iter()
    .any(|name| term.contains(name))
    {
        return ColorDepth::Truecolor;
    }

    // `rxvt-unicode-256color` already matched `256color` above; the remaining
    // `rxvt*` values are the 88-colour ones, which quantize to 16.
    if term.starts_with("rxvt") {
        return ColorDepth::Palette16;
    }

    ColorDepth::Palette16
}

/// Parse an override value. `None` for an unrecognised one, so a typo falls
/// through to detection instead of blanking the UI.
fn parse_depth(value: &str) -> Option<ColorDepth> {
    match value {
        "truecolor" | "24bit" => Some(ColorDepth::Truecolor),
        "256" | "256color" => Some(ColorDepth::Palette256),
        "16" | "16color" => Some(ColorDepth::Palette16),
        "none" | "mono" => Some(ColorDepth::None),
        _ => None,
    }
}

fn is_multiplexer(term: &str) -> bool {
    term.starts_with("screen") || term.starts_with("tmux")
}

/// Perceptual-ish distance between two RGB triples ("redmean").
///
/// A cheap approximation that behaves far better than plain RGB distance on
/// the blue/cyan and skin-tone ranges a dark UI theme leans on: it weights the
/// red channel by how bright the pair already is, because the eye's red
/// sensitivity falls off in bright regions.
fn redmean(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> u32 {
    let (r1, g1, b1, r2, g2, b2) = (
        i32::from(r1),
        i32::from(g1),
        i32::from(b1),
        i32::from(r2),
        i32::from(g2),
        i32::from(b2),
    );
    let mean_r = (r1 + r2) / 2;
    let dr = r1 - r2;
    let dg = g1 - g2;
    let db = b1 - b2;
    ((((512 + mean_r) * dr * dr) >> 8) + 4 * dg * dg + (((767 - mean_r) * db * db) >> 8)) as u32
}

/// Index into [`CUBE`] for one channel. Squared error is separable per
/// channel, so per-channel nearest is the optimal cube entry for the triple —
/// no search over the 216 cube cells is needed.
fn cube_channel(value: u8) -> usize {
    CUBE.iter()
        .enumerate()
        .min_by_key(|(_, level)| level.abs_diff(value))
        .map_or(0, |(index, _)| index)
}

/// Nearest entry on the 24-step grey ramp (xterm indices 232..=255, whose
/// values are `8 + 10 * i`).
fn gray_index(value: u8) -> u8 {
    let step = ((i32::from(value) + 5 - 8) / 10).clamp(0, 23) as u8;
    232 + step
}

/// The xterm-256 index that best represents `(r, g, b)`.
///
/// The answer is the perceptually closer of two candidates: the best cube
/// cell (per-channel nearest level) and the best grey ramp step. That is the
/// optimum over all 240 entries, because squared error is separable per cube
/// channel — so the two candidates bracket every other cell.
fn xterm256_index(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (cube_channel(r), cube_channel(g), cube_channel(b));
    let (cr, cg, cb) = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let cube = 16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8;

    let gray = gray_index(((u32::from(r) + u32::from(g) + u32::from(b) + 1) / 3) as u8);
    let gray_value = 8 + 10 * (gray - 232);

    if redmean(r, g, b, gray_value, gray_value, gray_value) < redmean(r, g, b, cr, cg, cb) {
        gray
    } else {
        cube
    }
}

/// The base-16 SGR index that best represents `(r, g, b)`.
///
/// A plain nearest-entry search against [`BASE16`], so an accent stays an
/// accent instead of collapsing onto one arbitrary entry of the eight.
fn xterm16_index(r: u8, g: u8, b: u8) -> u8 {
    let mut best = (u32::MAX, 7u8);
    for (index, &(cr, cg, cb)) in BASE16.iter().enumerate() {
        let distance = redmean(r, g, b, cr, cg, cb);
        if distance < best.0 {
            best = (distance, index as u8);
        }
    }
    best.1
}

/// Reduce one colour to what `depth` can display.
pub fn quantize(color: Color, depth: ColorDepth) -> Color {
    match (color, depth) {
        (Color::Rgb(r, g, b), ColorDepth::Truecolor) => Color::Rgb(r, g, b),
        (Color::Rgb(r, g, b), ColorDepth::Palette256) => Color::Indexed(xterm256_index(r, g, b)),
        (Color::Rgb(r, g, b), ColorDepth::Palette16) => Color::Indexed(xterm16_index(r, g, b)),
        // `NO_COLOR`: let the terminal's own default show through.
        (_, ColorDepth::None) => Color::Reset,
        // A named colour is already drawn from the 16-entry ANSI set, so it
        // survives every depth unchanged.
        (other, _) => other,
    }
}

/// Counts [`quantize_palette`] calls, so a test can prove the accessors do not
/// re-run quantization.
#[cfg(test)]
static QUANTIZE_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn note_quantize_call() {
    use std::sync::atomic::Ordering;
    QUANTIZE_CALLS.fetch_add(1, Ordering::Relaxed);
}

/// How many times a palette has been quantized. Test-only.
#[cfg(test)]
pub(crate) fn quantize_calls() -> usize {
    use std::sync::atomic::Ordering;
    QUANTIZE_CALLS.load(Ordering::Relaxed)
}

/// Quantize a whole palette to `depth`.
///
/// Called from `set_active_theme`, once per theme change, so that every read
/// through the palette accessors is a table lookup. Written as a struct
/// literal rather than a `clone()`-then-assign loop so that adding a palette
/// role fails to compile here instead of silently bypassing quantization.
pub fn quantize_palette(palette: &ColorPalette, depth: ColorDepth) -> ColorPalette {
    #[cfg(test)]
    note_quantize_call();
    let q = |color: Color| quantize(color, depth);
    ColorPalette {
        error: q(palette.error),
        success: q(palette.success),
        warning: q(palette.warning),
        info: q(palette.info),
        action: q(palette.action),
        disabled: q(palette.disabled),
        accent: q(palette.accent),
        secondary_accent: q(palette.secondary_accent),
        text_light: q(palette.text_light),
        text_dark: q(palette.text_dark),
        border: q(palette.border),
        emphasis: q(palette.emphasis),
        text: q(palette.text),
        muted: q(palette.muted),
        panel_bg: q(palette.panel_bg),
        overlay_bg: q(palette.overlay_bg),
        selection_bg: q(palette.selection_bg),
        text_selection_bg: q(palette.text_selection_bg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_colors;

    /// Build an environment lookup from a fixed map, so each case is a pure
    /// function call over the decision table.
    fn env(pairs: &[(&'static str, &str)]) -> impl Fn(&str) -> Option<String> {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn color_depth_should_detect_truecolor_from_colorterm() {
        // COLORTERM alone is enough.
        assert_eq!(
            detect_from(env(&[
                ("COLORTERM", "truecolor"),
                ("TERM", "xterm-256color")
            ])),
            ColorDepth::Truecolor
        );
        assert_eq!(
            detect_from(env(&[("COLORTERM", "24bit"), ("TERM", "xterm-256color")])),
            ColorDepth::Truecolor
        );
        // Case and surrounding whitespace are not significant.
        assert_eq!(
            detect_from(env(&[("COLORTERM", " TrueColor ")])),
            ColorDepth::Truecolor
        );
        // TERM markers, with no COLORTERM at all.
        assert_eq!(
            detect_from(env(&[("TERM", "xterm-direct")])),
            ColorDepth::Truecolor
        );
        assert_eq!(
            detect_from(env(&[("TERM", "xterm-truecolor")])),
            ColorDepth::Truecolor
        );
        // Truecolor terminals that do not advertise it in TERM.
        assert_eq!(
            detect_from(env(&[("TERM", "xterm-kitty")])),
            ColorDepth::Truecolor
        );
        // The override beats every environment signal.
        assert_eq!(
            detect_from(env(&[
                ("COLORTERM", "truecolor"),
                ("TERM", "xterm-256color"),
                (COLOR_DEPTH_OVERRIDE_VAR, "16"),
            ])),
            ColorDepth::Palette16
        );
        // A mistyped override is ignored rather than blanking the UI.
        assert_eq!(
            detect_from(env(&[
                ("TERM", "xterm-256color"),
                (COLOR_DEPTH_OVERRIDE_VAR, "sixteen-ish"),
            ])),
            ColorDepth::Palette256
        );
    }

    #[test]
    fn color_depth_should_detect_256_and_16_and_plain() {
        // 256: the TERM marker, and both multiplexers.
        assert_eq!(
            detect_from(env(&[("TERM", "xterm-256color")])),
            ColorDepth::Palette256
        );
        assert_eq!(
            detect_from(env(&[("TERM", "screen-256color")])),
            ColorDepth::Palette256
        );
        assert_eq!(
            detect_from(env(&[("TERM", "tmux-256color")])),
            ColorDepth::Palette256
        );
        // A multiplexer caps truecolor at 256: `Tc` negotiation is not
        // observable from here, and overstating depth is the failure mode
        // this whole module exists to prevent.
        assert_eq!(
            detect_from(env(&[
                ("TERM", "screen.xterm-256color"),
                ("COLORTERM", "truecolor"),
            ])),
            ColorDepth::Palette256
        );

        // 16: the plain rxvt variant, and every unhelpful TERM value.
        assert_eq!(detect_from(env(&[("TERM", "rxvt")])), ColorDepth::Palette16);
        assert_eq!(
            detect_from(env(&[("TERM", "xterm")])),
            ColorDepth::Palette16
        );
        assert_eq!(detect_from(env(&[("TERM", "dumb")])), ColorDepth::Palette16);
        assert_eq!(detect_from(env(&[])), ColorDepth::Palette16);
    }

    #[test]
    fn color_depth_should_honor_no_color() {
        // NO_COLOR wins over an explicit truecolor environment...
        assert_eq!(
            detect_from(env(&[
                ("NO_COLOR", "1"),
                ("COLORTERM", "truecolor"),
                ("TERM", "xterm-256color"),
            ])),
            ColorDepth::None
        );
        // ...and over the override, so a stale export cannot force colour back
        // on for someone who asked for none.
        assert_eq!(
            detect_from(env(&[
                ("NO_COLOR", "yes"),
                (COLOR_DEPTH_OVERRIDE_VAR, "truecolor"),
            ])),
            ColorDepth::None
        );
        // An empty NO_COLOR is not a request; treat it as unset.
        assert_eq!(
            detect_from(env(&[("NO_COLOR", ""), ("TERM", "xterm-256color")])),
            ColorDepth::Palette256
        );
        // And quantization actually drops the colour.
        assert_eq!(
            quantize(Color::Rgb(1, 2, 3), ColorDepth::None),
            Color::Reset
        );
    }

    #[test]
    fn quantize_should_map_primaries_to_expected_xterm_indices() {
        // Pins the xterm-256 layout: cube cells are 16 + 36*r + 6*g + b, with
        // cube level 5 == 255. Getting the layout or the level table wrong
        // sends every primary to the wrong cell, so these are the canonical
        // known-good indices.
        assert_eq!(
            quantize(Color::Rgb(255, 0, 0), ColorDepth::Palette256),
            Color::Indexed(196)
        );
        assert_eq!(
            quantize(Color::Rgb(0, 255, 0), ColorDepth::Palette256),
            Color::Indexed(46)
        );
        assert_eq!(
            quantize(Color::Rgb(0, 0, 255), ColorDepth::Palette256),
            Color::Indexed(21)
        );
        assert_eq!(
            quantize(Color::Rgb(255, 255, 255), ColorDepth::Palette256),
            Color::Indexed(231)
        );
        assert_eq!(
            quantize(Color::Rgb(0, 0, 0), ColorDepth::Palette256),
            Color::Indexed(16)
        );

        // The 16-colour path maps the same primaries onto the base ANSI set:
        // dark red / dark green / dark blue / bright white / black.
        assert_eq!(
            quantize(Color::Rgb(255, 0, 0), ColorDepth::Palette16),
            Color::Indexed(9)
        );
        assert_eq!(
            quantize(Color::Rgb(0, 255, 0), ColorDepth::Palette16),
            Color::Indexed(10)
        );
        assert_eq!(
            quantize(Color::Rgb(0, 0, 255), ColorDepth::Palette16),
            Color::Indexed(12)
        );
        assert_eq!(
            quantize(Color::Rgb(255, 255, 255), ColorDepth::Palette16),
            Color::Indexed(15)
        );
        assert_eq!(
            quantize(Color::Rgb(0, 0, 0), ColorDepth::Palette16),
            Color::Indexed(0)
        );

        // Truecolor passes triples through, and named colours are already
        // 16-colour-safe so they survive every depth.
        assert_eq!(
            quantize(Color::Rgb(1, 2, 3), ColorDepth::Truecolor),
            Color::Rgb(1, 2, 3)
        );
        assert_eq!(quantize(Color::Cyan, ColorDepth::Palette256), Color::Cyan);
        assert_eq!(quantize(Color::Cyan, ColorDepth::Palette16), Color::Cyan);
    }

    #[test]
    fn quantize_should_prefer_gray_ramp_for_neutrals() {
        // The point of the grey ramp: a dark theme's neutrals must land on the
        // 24 grey steps, not on the cube's six grey levels (0/95/135/175/215/255),
        // which is what crushes them.
        for (rgb, expected) in [
            ((0u8, 0u8, 0u8), 16u8), // exact cube corner stays put
            ((8, 8, 8), 232),        // first ramp step
            ((18, 18, 18), 233),
            ((120, 120, 120), 243), // ramp value 118
            ((198, 198, 198), 251), // ramp value 198, exact
            ((229, 229, 229), 254), // ramp value 228 (distance 1), nord/dracula `text`
        ] {
            let got = quantize(Color::Rgb(rgb.0, rgb.1, rgb.2), ColorDepth::Palette256);
            assert_eq!(got, Color::Indexed(expected), "neutral {rgb:?}");
        }

        // A neutral that the cube happens to hit exactly still resolves to a
        // real cell: the Operant default's `border` is (72, 72, 80), which is
        // near-neutral and must not land on a saturated cube colour.
        // The whole 24-step ramp (232..=255) is the assertion, not a subrange:
        // for (72, 72, 80) the mean is 75, which lands on ramp step 7 -> index
        // 239 -> grey value 78. That is the nearest ramp entry (the next step up
        // is 88), and 239 is inside the ramp. An earlier version of this
        // assertion read `240..=255` and failed against the correct answer by
        // excluding exactly the step the quantiser picks.
        let border = quantize(Color::Rgb(72, 72, 80), ColorDepth::Palette256);
        assert!(matches!(border, Color::Indexed(232..=255)), "{border:?}");

        // Saturated colours must still prefer the cube over the ramp, so the
        // previous case is not just "always grey".
        for rgb in [(255, 0, 0), (0, 200, 255), (200, 0, 255), (255, 200, 0)] {
            let got = quantize(Color::Rgb(rgb.0, rgb.1, rgb.2), ColorDepth::Palette256);
            assert!(
                matches!(got, Color::Indexed(16..=231)),
                "{rgb:?} should stay in the cube, got {got:?}"
            );
        }
    }

    #[test]
    fn align_vendored_detector_makes_the_two_pipelines_emit_one_encoding() {
        // Serialize against the vendored style tests, which mutate the same
        // process-global capability.
        let _lock = crate::tui::vendor::style::STYLE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // `muted` in the default theme — the colour whose DarkGray->Dim rewrite
        // produced the mixed-encoding frame.
        let (r, g, b) = (204u8, 155, 31);

        // The defect: with OPERANT_COLOR_DEPTH=truecolor on a 256-colour TERM,
        // `detect()` returned Truecolor (palette stayed rgb:) while the
        // vendored pipeline independently chose Color256 and rewrote one cell to
        // `Color::Indexed(172)`.
        align_vendored_detector(ColorDepth::Truecolor);

        // Post-condition: once we have resolved Truecolor, every substitution
        // must come back as a triple, so a frame cannot mix `rgb:` and
        // `indexed:` encodings. NOTE: on a host that already reports truecolor
        // this passes whether or not the call above happens, so it only fails on
        // a 256-colour host — which is the case the corpus pins and the case
        // that produced the bad golden.
        assert_eq!(
            crate::tui::vendor::style::color::rgb(r, g, b),
            Color::Rgb(r, g, b),
            "the vendored quantiser must agree with the resolved depth, or one \
             frame carries two colour encodings"
        );

        // The other depths need no alignment: both pipelines already emit
        // `Color::Indexed` there, so alignment is a deliberate no-op rather
        // than a panic.
        for depth in [
            ColorDepth::Palette256,
            ColorDepth::Palette16,
            ColorDepth::None,
        ] {
            align_vendored_detector(depth);
        }
    }

    #[test]
    fn quantized_palette_should_be_precomputed_not_per_access() {
        // Serialize against the theme_colors tests, which mutate the shared
        // process-global palette, so the call counts below are exact.
        let _guard = theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Setting the theme quantizes the palette once, eagerly.
        let before = quantize_calls();
        theme_colors::set_active_theme("nord");
        let after_set = quantize_calls();
        assert!(
            after_set > before,
            "set_active_theme must quantize the whole palette once, up front"
        );

        // 1000 reads of all 14 accessors — 13k styled spans — must not run a
        // single quantization pass: the stored palette is already quantized and
        // an accessor is a field read under a read lock.
        for _ in 0..1000 {
            let _ = (
                theme_colors::accent(),
                theme_colors::text(),
                theme_colors::muted(),
                theme_colors::border(),
                theme_colors::panel_bg(),
                theme_colors::overlay_bg(),
                theme_colors::selection_bg(),
                theme_colors::text_selection_bg(),
                theme_colors::error(),
                theme_colors::success(),
                theme_colors::warning(),
                theme_colors::disabled(),
                theme_colors::on_selection(),
            );
        }
        assert_eq!(
            quantize_calls(),
            after_set,
            "palette accessors must be table lookups, not per-call quantization"
        );

        // And the values the accessors hand out are the quantized ones: with a
        // 256-colour terminal, `nord`'s border (76, 86, 106) becomes an indexed
        // colour rather than a triple the terminal would have to crush.
        let quantized = quantize_palette(&ColorPalette::for_theme("nord"), ColorDepth::Palette256);
        assert!(matches!(quantized.border, Color::Indexed(_)));
        assert!(matches!(quantized.emphasis, Color::Indexed(_)));
        assert!(matches!(quantized.panel_bg, Color::Indexed(_)));
        // No field was left as a raw triple.
        for color in [
            quantized.error,
            quantized.success,
            quantized.warning,
            quantized.info,
            quantized.action,
            quantized.disabled,
            quantized.accent,
            quantized.secondary_accent,
            quantized.text_light,
            quantized.text_dark,
            quantized.border,
            quantized.emphasis,
            quantized.text,
            quantized.muted,
            quantized.panel_bg,
            quantized.overlay_bg,
            quantized.selection_bg,
            quantized.text_selection_bg,
        ] {
            assert!(
                !matches!(color, Color::Rgb(..)),
                "{color:?} left unquantized"
            );
        }

        theme_colors::set_active_theme("default");
    }
}
