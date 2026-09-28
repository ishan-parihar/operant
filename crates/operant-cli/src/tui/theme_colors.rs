// theme_colors.rs — Color palette management for accessibility-friendly themes.
//
// Provides color definitions for different themes, with special support for
// Deuteranopia (red-green color blindness) using blue, yellow, and gray palettes.
//
// The palette is the single source for the TUI's shared semantic colours: the
// renderers call the free accessors below (`theme_colors::text()`,
// `theme_colors::accent()`, …) instead of hardcoding `Color::Rgb(…)`. The
// active theme is a process-global set once at startup (from
// `config.tui.theme`) and again whenever `/theme` applies a new palette.

use std::sync::RwLock;

use ratatui::style::Color;

/// Color palette for a specific theme.
#[derive(Debug, Clone)]
pub struct ColorPalette {
    /// Error messages and alerts (normally red, but color-blind friendly)
    pub error: Color,
    /// Success indicators (normally green, but color-blind friendly)
    pub success: Color,
    /// Warning/caution messages
    pub warning: Color,
    /// Information messages
    pub info: Color,
    /// Action buttons and interactive elements
    pub action: Color,
    /// Disabled or dimmed states
    pub disabled: Color,
    /// Primary accent color
    pub accent: Color,
    /// Secondary accent
    pub secondary_accent: Color,
    /// Text on dark backgrounds
    pub text_light: Color,
    /// Text on light backgrounds
    pub text_dark: Color,
    /// Borders and dividers
    pub border: Color,
    // ---- Shared render semantics (the colours the TUI chrome actually uses).
    // `emphasis` is the amber accent bar / selection accent; the remaining
    // fields replace the former hardcoded `OPERANT_*` constants.
    /// Accent bar / selection accent (emphasis colour for the chrome).
    pub emphasis: Color,
    /// Primary body text on panel backgrounds.
    pub text: Color,
    /// Dimmed / secondary text.
    pub muted: Color,
    /// Panel (dialog) background.
    pub panel_bg: Color,
    /// Dimming overlay painted behind modals.
    pub overlay_bg: Color,
    /// Background of a selected list/menu row.
    pub selection_bg: Color,
    /// Background of a mouse-drag text selection.
    pub text_selection_bg: Color,
}

impl ColorPalette {
    /// Get the color palette for a given theme name.
    pub fn for_theme(theme_name: &str) -> Self {
        match theme_name {
            "deuteranopia" => Self::deuteranopia(),
            "dark" => Self::dark(),
            "light" => Self::light(),
            "solarized" => Self::solarized(),
            "nord" => Self::nord(),
            "dracula" => Self::dracula(),
            "monokai" => Self::monokai(),
            _ => Self::default_theme(),
        }
    }

    /// Default Operant theme
    const fn default_theme() -> Self {
        Self {
            error: Color::Rgb(255, 87, 51),   // Bright red-orange
            success: Color::Rgb(76, 175, 80), // Green
            warning: Color::Rgb(255, 152, 0), // Orange
            info: Color::Cyan,
            action: Color::Cyan,
            disabled: Color::DarkGray,
            accent: Color::Cyan,
            secondary_accent: Color::Rgb(233, 30, 99), // Magenta
            text_light: Color::White,
            text_dark: Color::Black,
            border: Color::Rgb(72, 72, 80),
            emphasis: Color::Rgb(255, 191, 0), // Amber accent bar
            text: Color::Rgb(255, 248, 220),   // Warm cream
            muted: Color::Rgb(204, 155, 31),   // Dim amber
            panel_bg: Color::Rgb(26, 26, 46),
            overlay_bg: Color::Rgb(10, 10, 14),
            selection_bg: Color::Rgb(255, 191, 0),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Dark theme
    const fn dark() -> Self {
        Self {
            error: Color::Rgb(239, 83, 80),     // Light red
            success: Color::Rgb(129, 199, 132), // Light green
            warning: Color::Rgb(255, 171, 64),  // Light orange
            info: Color::Rgb(100, 181, 246),    // Light blue
            action: Color::Rgb(100, 181, 246),
            disabled: Color::Rgb(97, 97, 97),
            accent: Color::Rgb(100, 181, 246),
            secondary_accent: Color::Rgb(229, 57, 53),
            text_light: Color::Rgb(229, 229, 229),
            text_dark: Color::Rgb(33, 33, 33),
            border: Color::Rgb(66, 66, 66),
            emphasis: Color::Rgb(100, 181, 246),
            text: Color::Rgb(229, 229, 229),
            muted: Color::Rgb(120, 120, 120),
            panel_bg: Color::Rgb(24, 24, 24),
            overlay_bg: Color::Rgb(10, 10, 10),
            selection_bg: Color::Rgb(100, 181, 246),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Light theme
    const fn light() -> Self {
        Self {
            error: Color::Rgb(211, 47, 47),    // Dark red
            success: Color::Rgb(27, 94, 32),   // Dark green
            warning: Color::Rgb(230, 124, 13), // Dark orange
            info: Color::Rgb(13, 71, 161),     // Dark blue
            action: Color::Blue,
            disabled: Color::Rgb(189, 189, 189),
            accent: Color::Blue,
            secondary_accent: Color::Rgb(194, 24, 91),
            text_light: Color::White,
            text_dark: Color::Black,
            border: Color::Rgb(189, 189, 189),
            emphasis: Color::Blue,
            text: Color::Rgb(33, 33, 33),
            muted: Color::Rgb(110, 110, 110),
            panel_bg: Color::Rgb(248, 248, 248),
            overlay_bg: Color::Rgb(90, 90, 90),
            selection_bg: Color::Blue,
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Solarized Dark theme
    const fn solarized() -> Self {
        Self {
            error: Color::Rgb(220, 50, 47),   // Solarized red
            success: Color::Rgb(133, 153, 0), // Solarized green
            warning: Color::Rgb(181, 137, 0), // Solarized yellow
            info: Color::Rgb(38, 139, 210),   // Solarized blue
            action: Color::Rgb(38, 139, 210),
            disabled: Color::Rgb(88, 110, 117),
            accent: Color::Rgb(38, 139, 210),
            secondary_accent: Color::Rgb(108, 113, 196),
            text_light: Color::Rgb(131, 148, 150),
            text_dark: Color::Rgb(0, 43, 54),
            border: Color::Rgb(7, 54, 66),
            emphasis: Color::Rgb(38, 139, 210),
            text: Color::Rgb(147, 161, 161),
            muted: Color::Rgb(88, 110, 117),
            panel_bg: Color::Rgb(0, 43, 54),
            overlay_bg: Color::Rgb(7, 54, 66),
            selection_bg: Color::Rgb(38, 139, 210),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Nord theme
    const fn nord() -> Self {
        Self {
            error: Color::Rgb(191, 97, 106),    // Nord red
            success: Color::Rgb(163, 190, 140), // Nord green
            warning: Color::Rgb(235, 203, 139), // Nord yellow
            info: Color::Rgb(136, 192, 208),    // Nord blue
            action: Color::Rgb(136, 192, 208),
            disabled: Color::Rgb(76, 86, 106),
            accent: Color::Rgb(136, 192, 208),
            secondary_accent: Color::Rgb(191, 97, 106),
            text_light: Color::Rgb(236, 239, 244),
            text_dark: Color::Rgb(46, 52, 64),
            border: Color::Rgb(67, 76, 94),
            emphasis: Color::Rgb(136, 192, 208),
            text: Color::Rgb(216, 222, 233),
            muted: Color::Rgb(76, 86, 106),
            panel_bg: Color::Rgb(46, 52, 64),
            overlay_bg: Color::Rgb(36, 41, 51),
            selection_bg: Color::Rgb(136, 192, 208),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Dracula theme
    const fn dracula() -> Self {
        Self {
            error: Color::Rgb(255, 85, 85),     // Dracula red
            success: Color::Rgb(80, 250, 123),  // Dracula green
            warning: Color::Rgb(241, 250, 140), // Dracula yellow
            info: Color::Rgb(139, 233, 253),    // Dracula blue
            action: Color::Rgb(139, 233, 253),
            disabled: Color::Rgb(98, 114, 164),
            accent: Color::Rgb(139, 233, 253),
            secondary_accent: Color::Rgb(189, 147, 249),
            text_light: Color::Rgb(248, 248, 242),
            text_dark: Color::Rgb(40, 42, 54),
            border: Color::Rgb(68, 71, 90),
            emphasis: Color::Rgb(189, 147, 249),
            text: Color::Rgb(248, 248, 242),
            muted: Color::Rgb(98, 114, 164),
            panel_bg: Color::Rgb(40, 42, 54),
            overlay_bg: Color::Rgb(20, 21, 30),
            selection_bg: Color::Rgb(189, 147, 249),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Monokai theme
    const fn monokai() -> Self {
        Self {
            error: Color::Rgb(249, 38, 114), // Monokai magenta (used for errors)
            success: Color::Rgb(166, 226, 46), // Monokai green
            warning: Color::Rgb(253, 151, 31), // Monokai orange
            info: Color::Rgb(102, 217, 239), // Monokai cyan
            action: Color::Rgb(102, 217, 239),
            disabled: Color::Rgb(117, 113, 94),
            accent: Color::Rgb(102, 217, 239),
            secondary_accent: Color::Rgb(249, 38, 114),
            text_light: Color::Rgb(248, 248, 242),
            text_dark: Color::Rgb(39, 40, 34),
            border: Color::Rgb(75, 75, 75),
            emphasis: Color::Rgb(102, 217, 239),
            text: Color::Rgb(248, 248, 242),
            muted: Color::Rgb(117, 113, 94),
            panel_bg: Color::Rgb(39, 40, 34),
            overlay_bg: Color::Rgb(20, 20, 18),
            selection_bg: Color::Rgb(102, 217, 239),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }

    /// Deuteranopia (red-green color blind) theme
    /// Uses blue, yellow, and gray to avoid red/green distinction
    const fn deuteranopia() -> Self {
        Self {
            error: Color::Rgb(255, 140, 0),   // Orange (not red)
            success: Color::Rgb(0, 150, 200), // Blue (not green)
            warning: Color::Rgb(255, 180, 0), // Gold/Yellow
            info: Color::Cyan,
            action: Color::Rgb(0, 150, 200), // Blue action buttons
            disabled: Color::Rgb(120, 120, 120), // Neutral gray
            accent: Color::Rgb(0, 150, 200), // Blue accent
            secondary_accent: Color::Rgb(180, 140, 255), // Purple accent
            text_light: Color::Rgb(220, 220, 220),
            text_dark: Color::Rgb(40, 40, 40),
            border: Color::Rgb(100, 100, 100),
            emphasis: Color::Rgb(0, 150, 200), // Blue, not amber/red
            text: Color::Rgb(220, 220, 220),
            muted: Color::Rgb(120, 120, 120),
            panel_bg: Color::Rgb(18, 18, 18),
            overlay_bg: Color::Rgb(8, 8, 8),
            selection_bg: Color::Rgb(0, 150, 200),
            text_selection_bg: Color::Rgb(200, 200, 200),
        }
    }
}

/// The Operant default palette, used as the process-global starting point.
pub const DEFAULT_PALETTE: ColorPalette = ColorPalette::default_theme();

/// The palette every renderer reads through the accessors below.
///
/// Set once at startup from the persisted theme name and again on every
/// `/theme` selection. Held behind an `RwLock` so the render hot path only
/// ever copies a single `Color` out — never the whole palette.
///
/// What is stored here is the palette *already quantized* to the terminal's
/// colour depth (see [`crate::tui::color_depth`]): the accessors are read once
/// per styled span, so quantization has to happen on the write path in
/// [`set_active_theme`], not per read. `DEFAULT_PALETTE` is the one exception
/// — it is a `const` static initializer, so the colours read before the first
/// `set_active_theme` (which `App::new` runs before the first frame) are the
/// authored triples.
static ACTIVE: RwLock<ColorPalette> = RwLock::new(DEFAULT_PALETTE);

/// Read one colour out of the active palette without cloning the palette.
///
/// A poisoned lock still holds a valid palette (the accessors cannot panic),
/// so recover the guard instead of propagating the poisoning.
fn with_active(f: impl FnOnce(&ColorPalette) -> Color) -> Color {
    match ACTIVE.read() {
        Ok(guard) => f(&guard),
        Err(poisoned) => f(&poisoned.into_inner()),
    }
}

/// Apply a theme by name, making it the palette every renderer reads.
///
/// Unknown names fall back to [`DEFAULT_PALETTE`], matching
/// [`ColorPalette::for_theme`].
///
/// This is also where the palette is quantized to the terminal's colour depth:
/// it is the only write path for [`ACTIVE`], so quantizing here means all 14
/// accessors — and every renderer reading through them — get colours the
/// terminal can actually display, at the cost of one pass per theme change
/// rather than one per styled span. [`crate::tui::color_depth::detect`] is
/// itself cached, so the detection cost is paid once per process, here.
pub fn set_active_theme(theme_name: &str) {
    let next = crate::tui::color_depth::quantize_palette(
        &ColorPalette::for_theme(theme_name),
        crate::tui::color_depth::detect(),
    );
    match ACTIVE.write() {
        Ok(mut guard) => *guard = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

/// Accent bar / selection accent.
pub fn accent() -> Color {
    with_active(|p| p.emphasis)
}

/// Primary body text on panel backgrounds.
pub fn text() -> Color {
    with_active(|p| p.text)
}

/// Dimmed / secondary text.
pub fn muted() -> Color {
    with_active(|p| p.muted)
}

/// Borders and dividers.
pub fn border() -> Color {
    with_active(|p| p.border)
}

/// Panel (dialog) background.
pub fn panel_bg() -> Color {
    with_active(|p| p.panel_bg)
}

/// Dimming overlay painted behind modals.
pub fn overlay_bg() -> Color {
    with_active(|p| p.overlay_bg)
}

/// Background of a selected list/menu row.
pub fn selection_bg() -> Color {
    with_active(|p| p.selection_bg)
}

/// Background of a mouse-drag text selection.
pub fn text_selection_bg() -> Color {
    with_active(|p| p.text_selection_bg)
}

/// Error / alert colour (orange, never red, under deuteranopia).
pub fn error() -> Color {
    with_active(|p| p.error)
}

/// Success colour (blue, never green, under deuteranopia).
pub fn success() -> Color {
    with_active(|p| p.success)
}

/// Warning / caution colour.
pub fn warning() -> Color {
    with_active(|p| p.warning)
}

/// Colour for disabled/dimmed chrome.
pub fn disabled() -> Color {
    with_active(|p| p.disabled)
}

// ---------------------------------------------------------------------------
// Fixed neutral greys — deliberately NOT palette roles
// ---------------------------------------------------------------------------
//
// These three are the same colour repeated across the UI: six dialogs each
// declared `let dim = Color::Rgb(90, 90, 90)`, two declared
// `let muted = Color::Rgb(180, 180, 180)`, and the footer declared its own
// `let dim = Color::Rgb(110, 110, 124)`.
//
// They are constants, not accessors, on purpose. Routing them through the
// palette would CHANGE their appearance: this palette's `muted()` is
// `Rgb(204, 155, 31)` — a dim amber — under the default theme, and
// `disabled()` is `Rgb(189, 189, 189)` under the light theme, which is nearly
// invisible on that theme's `Rgb(248, 248, 248)` background. These greys are
// the neutral, theme-invariant tier the dialogs actually want. Making them
// themed is a per-theme design decision, not a refactor, and it is deliberately
// not made here.
//
// The two greys that both read as "dim" are kept apart on purpose: the footer's
// is a different value, and a single `dim` name across seven files is a trap
// for whoever eventually does that design work.

/// The dimmest text tier used by the dialogs. Was `Rgb(90, 90, 90)`.
pub const DIALOG_DIM: Color = Color::Rgb(90, 90, 90);

/// The mid-strength text tier used by the dialogs. Was `Rgb(180, 180, 180)`.
pub const DIALOG_MUTED: Color = Color::Rgb(180, 180, 180);

/// The footer's own dim tier. Was `Rgb(110, 110, 124)`, and was named `dim`
/// like the dialogs' `Rgb(90, 90, 90)` despite being a different colour.
pub const FOOTER_DIM: Color = Color::Rgb(110, 110, 124);

/// The brightest of the dialog text tiers: a SELECTED item's description line,
/// and the "press Enter to use custom model" hint.
///
/// DO NOT confuse this with [`text_selection_bg`], which has the IDENTICAL value
/// in all eight themes. That equality makes it look like a provably safe palette
/// substitution, and it is not: every use of this constant is a FOREGROUND,
/// while `text_selection_bg` is a BACKGROUND role. Swapping them would be
/// value-preserving today and wrong the moment someone changes
/// `text_selection_bg` for an unrelated reason — selected text would silently
/// move with it. Equal value, different role.
///
/// It is brighter than [`DIALOG_MUTED`] (`Rgb(180, 180, 180)`), which is the
/// point: a selected description should stand out from the unselected `dim`
/// fallback beside it.
pub const DIALOG_TEXT_BRIGHT: Color = Color::Rgb(200, 200, 200);

/// Foreground to use on top of [`selection_bg`] / [`text_selection_bg`].
pub fn on_selection() -> Color {
    with_active(|p| p.text_dark)
}

/// Apply the palette for a `Theme` enum value from `adapter_types::config`.
pub fn set_active_theme_enum(theme: &crate::tui::adapter_types::config::Theme) {
    set_active_theme(theme.as_str());
}

// `pub(crate)` so `tui::color_depth`'s precomputation test can reach the
// shared-palette lock below and serialize against these mutations. The module
// only exists under `#[cfg(test)]`, so this exposes nothing in a real build.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The process-global palette is shared, so tests that switch it must not
    /// interleave. Every mutating test takes this lock and restores the default
    /// palette before releasing it. `pub(crate)` so `tui::color_depth`'s
    /// precomputation test can serialize against these mutations too.
    pub(crate) static ACTIVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `f` with `theme` active, then restore the default palette.
    fn with_theme<T>(theme: &str, f: impl FnOnce() -> T) -> T {
        let _guard = ACTIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_active_theme(theme);
        let out = f();
        set_active_theme("default");
        out
    }

    #[test]
    fn default_theme_has_correct_colors() {
        let palette = ColorPalette::for_theme("default");
        assert_eq!(palette.error, Color::Rgb(255, 87, 51));
        assert_eq!(palette.success, Color::Rgb(76, 175, 80));
    }

    #[test]
    fn deuteranopia_uses_blue_for_success() {
        let palette = ColorPalette::for_theme("deuteranopia");
        // Success should be blue, not green
        assert_eq!(palette.success, Color::Rgb(0, 150, 200));
        // Error should be orange, not red
        assert_eq!(palette.error, Color::Rgb(255, 140, 0));
    }

    #[test]
    fn dark_theme_is_different_from_default() {
        let default = ColorPalette::for_theme("default");
        let dark = ColorPalette::for_theme("dark");
        assert_ne!(default.error, dark.error);
        assert_ne!(default.border, dark.border);
    }

    #[test]
    fn all_themes_return_palette() {
        for theme in [
            "default",
            "dark",
            "light",
            "solarized",
            "nord",
            "dracula",
            "monokai",
            "deuteranopia",
            "unknown",
        ] {
            let palette = ColorPalette::for_theme(theme);
            // Just verify it doesn't panic and returns valid colors
            assert!(matches!(
                palette.error,
                Color::Rgb(_, _, _) | Color::Red | Color::White | Color::Black
            ));
        }
    }

    #[test]
    fn error_accessor_follows_the_active_palette() {
        with_theme("deuteranopia", || {
            assert_eq!(error(), Color::Rgb(255, 140, 0));
        });
    }

    #[test]
    fn success_accessor_follows_the_active_palette() {
        with_theme("deuteranopia", || {
            assert_eq!(success(), Color::Rgb(0, 150, 200));
        });
    }

    #[test]
    fn warning_accessor_follows_the_active_palette() {
        with_theme("deuteranopia", || {
            assert_eq!(warning(), Color::Rgb(255, 180, 0));
        });
    }

    /// The whole point of the palette: switching theme changes the shared
    /// render colours, including the accessibility palette. Before this was
    /// wired every renderer hardcoded its own `Color::Rgb(…)`, so `/theme`
    /// persisted a name nothing read.
    #[test]
    fn theme_palette_should_change_shared_render_colors() {
        // Deuteranopia vs the Operant default: every shared semantic colour the
        // renderers read through the accessors must differ.
        with_theme("deuteranopia", || {
            let deuteranopia = (
                accent(),
                text(),
                muted(),
                border(),
                panel_bg(),
                overlay_bg(),
                selection_bg(),
                text_selection_bg(),
                error(),
                success(),
            );
            set_active_theme("default");
            let default = (
                accent(),
                text(),
                muted(),
                border(),
                panel_bg(),
                overlay_bg(),
                selection_bg(),
                text_selection_bg(),
                error(),
                success(),
            );
            assert_ne!(deuteranopia, default);
        });

        // A second accessibility-relevant pair, to prove it is not a one-off.
        with_theme("nord", || {
            let nord = (accent(), text(), muted(), border(), panel_bg());
            set_active_theme("solarized");
            let solarized = (accent(), text(), muted(), border(), panel_bg());
            assert_ne!(nord, solarized);
        });

        // And that the accessors actually follow the global, not a snapshot.
        with_theme("dracula", || {
            assert_eq!(accent(), ColorPalette::for_theme("dracula").emphasis);
            set_active_theme("monokai");
            assert_eq!(accent(), ColorPalette::for_theme("monokai").emphasis);
        });
    }
}
