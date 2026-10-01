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

use crate::tui::vendor::style::palette::{self, Palette, Role};
use crate::tui::vendor::style::theme_mode::{self, ThemeMode};

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
    let source = ColorPalette::for_theme(theme_name);
    let next =
        crate::tui::color_depth::quantize_palette(&source, crate::tui::color_depth::detect());
    match ACTIVE.write() {
        Ok(mut guard) => *guard = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
    sync_role_palette(&source);
}

// ---------------------------------------------------------------------------
// Bridge: this palette → the vendored 22-role palette
// ---------------------------------------------------------------------------
//
// The vendored design system (`crate::tui::vendor::style`) carries its own
// palette: 22 semantic roles, each with a FROZEN default RGB, plus a
// per-frame buffer pass (`theme_mode::adapt_buffer_for_display`) that rewrites
// any cell whose colour IS a role's default onto that role's configured value.
// That pass is the whole theming mechanism, and it is a no-op unless a role
// carries a configured value.
//
// So there are two palettes in the process — this file's 18-field
// `ColorPalette` and the role table — and they have to be bridged, not merged:
// deleting either would strand half the TUI. [`sync_role_palette`] seeds the
// role table from whichever `ColorPalette` is active, which is what makes
// `/theme` drive both from one action.
//
// The migration unit is the *call site*, not the draw: a widget becomes
// themable once it calls a `style::theme::*` accessor (those deliberately
// return the role's DEFAULT, so the buffer pass rewrites it). This seed only
// makes that possible; it does not by itself restyle anything.

/// Every [`ColorPalette`] field, in declaration order.
///
/// The role seed table ([`roles_for_field`]) and the collision test in this
/// module's `tests` both read this one list, so a new field cannot be added
/// without the seed table and its safety test seeing it.
fn palette_fields(p: &ColorPalette) -> [(&'static str, Color); 18] {
    [
        ("error", p.error),
        ("success", p.success),
        ("warning", p.warning),
        ("info", p.info),
        ("action", p.action),
        ("disabled", p.disabled),
        ("accent", p.accent),
        ("secondary_accent", p.secondary_accent),
        ("text_light", p.text_light),
        ("text_dark", p.text_dark),
        ("border", p.border),
        ("emphasis", p.emphasis),
        ("text", p.text),
        ("muted", p.muted),
        ("panel_bg", p.panel_bg),
        ("overlay_bg", p.overlay_bg),
        ("selection_bg", p.selection_bg),
        ("text_selection_bg", p.text_selection_bg),
    ]
}

/// Which jcode roles a [`ColorPalette`] field seeds.
///
/// Several roles share a field when operant has no closer counterpart:
/// `emphasis` is the accent role proper, `accent` doubles as the header icon,
/// and so on. `emphasis` — not `accent` — is [`Role::Accent`] because
/// `emphasis` is what this file's `accent()` accessor already returns and
/// therefore what the whole chrome is composed against today.
///
/// Four fields have no role counterpart and are deliberately left un-seeded:
/// `text_dark` (a *foreground* for use on top of a selection background, not a
/// shade), `overlay_bg` (a dimming wash, not a surface),
/// `text_selection_bg` (a mouse-drag highlight that is `Rgb(200,200,200)` in
/// all eight themes, so it carries no theme signal), and `secondary_accent`
/// (the jcode `Ai` role would be its home, but `success` claims it — see
/// below). They keep reading their own accessors, which `/theme` already
/// drives.
fn roles_for_field(field: &str) -> &'static [Role] {
    match field {
        // Primary/secondary accents: user turns + the header session icon.
        "accent" => &[Role::User, Role::HeaderIcon],
        // `Ai` and `Success` share `success`. The `Ai` half is forced, not
        // chosen: jcode's frozen `Ai` default (129,199,132) is byte-identical
        // to the `dark` theme's `success`, so if `success` seeded any other
        // role then every un-migrated `theme_colors::success()` cell in the
        // TUI would be repainted to that role's colour. See the collision note
        // on `sync_role_palette`.
        "success" => &[Role::Ai, Role::Success],
        // Same forced pairing for `Tool`: jcode's `Tool` default (120,120,120)
        // is the `dark` and `deuteranopia` themes' `muted`.
        "muted" => &[Role::Tool, Role::Dim],
        "info" => &[Role::FileLink, Role::Info],
        "action" => &[Role::System],
        "disabled" => &[Role::Pending],
        // The accent role.
        "emphasis" => &[Role::Accent],
        // `HeaderSession` is forced the same way as `Ai`/`Tool`: jcode's
        // `HeaderSession` default (255,255,255) is the `default` and `light`
        // themes' `text_light`.
        "text_light" => &[Role::UserText, Role::HeaderSession],
        "text" => &[Role::AiText, Role::HeaderName],
        "panel_bg" => &[Role::UserBg],
        "border" => &[Role::Border],
        "selection_bg" => &[Role::SelectionBg],
        // Lifecycle indicators borrow the status colours they already use:
        // a queued prompt is an attention-coloured prompt, ASAP is urgent.
        "warning" => &[Role::Queued, Role::Warning],
        "error" => &[Role::Asap, Role::Error],
        // No role counterpart — see the doc comment.
        "text_dark" | "overlay_bg" | "text_selection_bg" | "secondary_accent" => &[],
        // Unknown field name: seed nothing. `seed_covers_every_role` in the
        // tests below fails if this arm is ever reached by a real field.
        _ => &[],
    }
}

/// Resolve a palette colour to the RGB triple a role slot holds.
///
/// `Color::Rgb` and `Color::Indexed` carry their own value. A ratatui *named*
/// colour does not, so the handful operant's palettes actually use are mapped
/// to the xterm base-16 entry they render as. A name outside that table leaves
/// its role un-seeded — and therefore un-substituted — rather than guessing a
/// value that would repaint a surface to the wrong colour.
fn role_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(index) => Some(crate::tui::vendor::style::color::indexed_to_rgb(index)),
        Color::Black => Some((0, 0, 0)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::Blue => Some((0, 0, 255)),
        Color::Cyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        other => {
            tracing::debug!(
                ?other,
                "named colour has no role seed; role left un-substituted"
            );
            None
        }
    }
}

/// Push `source` into the vendored role palette and set the theme mode.
///
/// # The collision rule this function has to honour
///
/// The buffer pass attributes a colour to a role by EXACT match against that
/// role's frozen default, then replaces it with the role's configured value.
/// So if a theme's own value for field F happens to be some *other* role's
/// default D, then every un-migrated `theme_colors::F()` cell in the TUI —
/// hundreds of them — would be silently repainted to whatever `D`'s role was
/// seeded with.
///
/// The only way to prevent that is for `D`'s role to be seeded from the very
/// field that carries `D`, which makes the substitution a no-op. Three role
/// defaults collide with an operant value today (`Ai`/`Tool`/`HeaderSession`,
/// see [`roles_for_field`]); `seed_never_repaints_an_operant_value` in the
/// tests below fails the build if a fourth appears.
fn sync_role_palette(source: &ColorPalette) {
    palette::set_palette(role_palette_for(source));
    // jcode's `ThemeMode` exists because jcode ships ONE dark palette and
    // adapts it for light terminals at the buffer. operant ships eight
    // self-contained palettes — `light` already carries dark inks on a
    // near-white surface — so the seed above IS the light adaptation.
    // Staying in `Dark` mode disables the second, luminance-flipping pass,
    // which would otherwise double-adapt colours this palette already got
    // right: the `light` theme's `text` (Rgb(33,33,33)) would flip back to
    // near-white and disappear.
    theme_mode::set_theme_mode(ThemeMode::Dark);
}

/// The role palette `source` seeds. Split out from [`sync_role_palette`] so
/// the collision test below can inspect it without touching the process-global.
fn role_palette_for(source: &ColorPalette) -> Palette {
    let mut next = Palette::default();
    for (field, color) in palette_fields(source) {
        let Some(rgb) = role_rgb(color) else { continue };
        for role in roles_for_field(field) {
            next.set(*role, rgb);
        }
    }
    next
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
// These two are the same colour repeated across the UI: six dialogs each
// declared `let dim = Color::Rgb(90, 90, 90)` and two declared
// `let muted = Color::Rgb(180, 180, 180)`.
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
// The footer's own `Rgb(110, 110, 124)` and the banner's `Rgb(140, 110, 0)`
// used to live here as `FOOTER_DIM` / `BANNER_DIM` for the same reason. The
// chrome migration moved them to the `Dim` role instead, because the base
// chrome is the one surface whose colour `/theme` must visibly repaint.

/// The dimmest text tier used by the dialogs. Was `Rgb(90, 90, 90)`.
///
/// `DIALOG_MUTED` and `DIALOG_TEXT_BRIGHT` used to sit beside this as fixed
/// neutral greys outside the palette. They had no role, so `/theme` could not
/// repaint them — the exact defect that gap inventory item 2 described. The
/// Wave 3 migration replaced every call site with the matching
/// `vendor::style::theme` role and removed both constants; a fixed grey with no
/// role is a palette leak, not a design token.
pub const DIALOG_DIM: Color = Color::Rgb(90, 90, 90);

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

/// A tool call that failed: the error glyph, its label, the standalone
/// block's preview, and the group header's error count.
///
/// This is deliberately a constant rather than a `palette` lookup, and the
/// reason is accessibility rather than convenience. `Rgb(255, 140, 0)` is the
/// `error` value of the **deuteranopia** palette, which is not a coincidence:
/// a red/green-safe palette has to use orange, because red is exactly the
/// colour a deuteranope cannot reliably tell from green. Hardcoding it means
/// tool errors are colourblind-safe on every theme, and the price is that a
/// user on any other theme also gets orange.
///
/// Do NOT "fix" this to `palette.error()`. That is the obvious theme
/// substitution and it is wrong: `error` ranges from `Rgb(191, 97, 106)`
/// (nord) to `Rgb(255, 140, 0)` (deuteranopia), so routing through the palette
/// changes the rendered colour on seven of eight themes. Whether tool errors
/// *should* follow the theme is a genuine per-theme appearance decision
/// (BUGS.md R40-30), and it is not this constant's job to make it silently.
/// The value here is identical to the previous literal at all six sites, so
/// nothing about the rendering changes.
pub const TOOL_ERROR: Color = Color::Rgb(255, 140, 0);

/// The highlighted background behind a search-match substring, used by both the
/// inline transcript search and the global-search overlay.
///
/// The foreground paired with it is deliberately NOT unified: the overlay uses
/// `warning()` while the transcript search uses `Color::Yellow`. That split was
/// an explicit skip at iter-414, recorded as "the search highlight whose fg is
/// coupled to a hardcoded bg", so unifying the two foregrounds is its own
/// appearance decision rather than a leftover.
pub const SEARCH_MATCH_BG: Color = Color::Rgb(60, 50, 0);

/// Foreground to use on top of [`selection_bg`] / [`text_selection_bg`], and on
/// top of `accent()` when a list row is selected — `accent()` returns
/// `p.emphasis`, which carries the same value as `selection_bg` on every theme
/// whose accent is a literal, so this one role covers both cases.
///
/// **Do NOT put a light foreground on a selected row's background.** That was the
/// defect fixed at iter-468: the four list-row renderers used `theme_colors::text()`
/// for the selected title and a pale pink for the secondary line, measuring
/// 1.29:1 to 2.88:1 on the six themes whose `accent()` is a literal. `text_dark`
/// reads 4.08:1 to 9.01:1 on those same six. On `default` and `light`,
/// `accent()` is a named ANSI slot (Cyan / Blue), so the ratio is
/// terminal-dependent and not computable from source; `text_dark` is
/// `Color::Black` on both, so every foreground option coincides there.
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

    /// Pins [`TOOL_ERROR`] to the palette value that justifies it.
    ///
    /// The six tool-error sites deliberately bypass `palette.error()`, which is
    /// only defensible while the constant IS the deuteranopia palette's error
    /// colour: a red/green-safe palette uses orange, which is precisely what
    /// makes tool errors colourblind-safe on every theme. Editing the constant
    /// to any other value silently drops that property while leaving the
    /// rationale in its doc comment intact, so tie the two together.
    #[test]
    fn tool_error_constant_tracks_the_deuteranopia_error_value() {
        assert_eq!(TOOL_ERROR, ColorPalette::for_theme("deuteranopia").error);
    }

    #[test]
    fn dark_theme_is_different_from_default() {
        let default = ColorPalette::for_theme("default");
        let dark = ColorPalette::for_theme("dark");
        assert_ne!(default.error, dark.error);
        assert_ne!(default.border, dark.border);
    }

    /// The bridge's whole point: one `/theme` action moves BOTH palettes.
    ///
    /// `set_active_theme` is the only write path for operant's palette, so
    /// this is the single point at which the role palette has to follow. If it
    /// ever stops doing so, every migrated surface freezes on whatever
    /// jcode's frozen role defaults are.
    #[test]
    fn switching_theme_rebuilds_the_role_palette() {
        let _guard = ACTIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_active_theme("dark");
        let dark = palette::palette();
        set_active_theme("light");
        let light = palette::palette();
        set_active_theme("default");

        assert_ne!(dark, light, "the role palette must follow /theme");
        for role in [Role::Accent, Role::Border, Role::AiText, Role::UserBg] {
            assert_ne!(dark.rgb(role), light.rgb(role), "{role:?} must repaint");
        }
    }

    /// Every role must be overridden, or the per-frame substitution is a no-op
    /// for it and no surface that migrated to it can ever be themed.
    #[test]
    fn every_role_is_seeded() {
        let seeded = role_palette_for(&ColorPalette::for_theme("nord"));
        for role in palette::ALL_ROLES {
            assert!(
                seeded.is_overridden(*role),
                "{role:?} is never seeded, so no surface using it can ever be themed"
            );
        }
    }

    /// The safety property the whole seed table exists to hold: whenever a
    /// theme's own value for a field IS some role's frozen default, that role
    /// must be seeded from the very field carrying it.
    ///
    /// Otherwise the per-frame substitution would attribute those cells to
    /// the other role and repaint hundreds of un-migrated surfaces to an
    /// unrelated colour. Three pairs collide today (`Ai`/`dark.success`,
    /// `Tool`/`dark.muted`, `HeaderSession`/`default.text_light`); this fails
    /// if a theme edit adds a fourth without fixing the seed table.
    #[test]
    fn seed_never_repaints_an_operant_value() {
        for name in [
            "default",
            "dark",
            "light",
            "solarized",
            "nord",
            "dracula",
            "monokai",
            "deuteranopia",
        ] {
            let source = ColorPalette::for_theme(name);
            let seeded = role_palette_for(&source);
            for (field, color) in palette_fields(&source) {
                let Some(value) = role_rgb(color) else {
                    continue;
                };
                let Some(role) = palette::ALL_ROLES
                    .iter()
                    .copied()
                    .find(|role| role.default_rgb() == value)
                else {
                    continue;
                };
                assert_eq!(
                    seeded.rgb(role),
                    value,
                    "{name}.{field} = {value:?}, which is the frozen default of {role:?}; \
                     {role:?} is seeded with {:?} instead, so every {field}() cell in the TUI \
                     would be silently repainted. Seed {role:?} from a field carrying {value:?}.",
                    seeded.rgb(role)
                );
            }
        }
    }

    /// A typo in the seed table's field names would silently un-seed a role
    /// (and could open a collision), so pin both directions: every role is
    /// reached, and the only fields with no role are the documented ones.
    #[test]
    fn seed_covers_every_role() {
        let source = ColorPalette::for_theme("default");
        let mut covered: Vec<Role> = palette_fields(&source)
            .iter()
            .flat_map(|(field, _)| roles_for_field(field).iter().copied())
            .collect();
        covered.sort();
        covered.dedup();
        let mut expected = palette::ALL_ROLES.to_vec();
        expected.sort();
        assert_eq!(covered, expected, "seed table does not cover every role");

        let mut unmapped: Vec<&str> = palette_fields(&source)
            .iter()
            .filter(|(field, _)| roles_for_field(field).is_empty())
            .map(|(field, _)| *field)
            .collect();
        unmapped.sort_unstable();
        assert_eq!(
            unmapped,
            [
                "overlay_bg",
                "secondary_accent",
                "text_dark",
                "text_selection_bg"
            ]
        );
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
