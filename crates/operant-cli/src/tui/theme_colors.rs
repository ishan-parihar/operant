// theme_colors.rs — Color palette management for accessibility-friendly themes.
//
// Provides color definitions for different themes, with special support for
// Deuteranopia (red-green color blindness) using blue, yellow, and gray palettes.

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
    fn default_theme() -> Self {
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
            border: Color::DarkGray,
        }
    }

    /// Dark theme
    fn dark() -> Self {
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
        }
    }

    /// Light theme
    fn light() -> Self {
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
        }
    }

    /// Solarized Dark theme
    fn solarized() -> Self {
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
        }
    }

    /// Nord theme
    fn nord() -> Self {
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
        }
    }

    /// Dracula theme
    fn dracula() -> Self {
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
        }
    }

    /// Monokai theme
    fn monokai() -> Self {
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
        }
    }

    /// Deuteranopia (red-green color blind) theme
    /// Uses blue, yellow, and gray to avoid red/green distinction
    fn deuteranopia() -> Self {
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
        }
    }
}

/// Get appropriate color for a given theme based on message type/role.
pub fn get_message_indicator_color(theme_name: &str, role: &str) -> Color {
    let palette = ColorPalette::for_theme(theme_name);
    match role {
        "user" => palette.accent,
        "assistant" => palette.secondary_accent,
        "system" => palette.disabled,
        "tool" => palette.action,
        _ => palette.text_light,
    }
}

/// Get error indicator color for given theme (always prominent, never red in deuteranopia).
pub fn get_error_color(theme_name: &str) -> Color {
    ColorPalette::for_theme(theme_name).error
}

/// Get success indicator color for given theme (blue instead of green in deuteranopia).
pub fn get_success_color(theme_name: &str) -> Color {
    ColorPalette::for_theme(theme_name).success
}

/// Get warning indicator color for given theme (yellow/gold instead of orange in deuteranopia).
pub fn get_warning_color(theme_name: &str) -> Color {
    ColorPalette::for_theme(theme_name).warning
}

/// Get the palette for a Theme enum from adapter_types::config
pub fn palette_for_theme(theme: &crate::tui::adapter_types::config::Theme) -> ColorPalette {
    let name = match theme {
        crate::tui::adapter_types::config::Theme::Dark => "dark",
        crate::tui::adapter_types::config::Theme::Light => "light",
        crate::tui::adapter_types::config::Theme::Default => "default",
        crate::tui::adapter_types::config::Theme::Deuteranopia => "deuteranopia",
        crate::tui::adapter_types::config::Theme::Custom(s) => s.as_str(),
    };
    ColorPalette::for_theme(name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn get_message_indicator_color_returns_valid_colors() {
        for role in ["user", "assistant", "system", "tool", "unknown"] {
            let color = get_message_indicator_color("default", role);
            assert!(matches!(
                color,
                Color::Rgb(_, _, _) | Color::Cyan | Color::White | Color::Black | Color::DarkGray
            ));
        }
    }

    #[test]
    fn get_error_color_returns_palette_error() {
        let color = get_error_color("deuteranopia");
        assert_eq!(color, Color::Rgb(255, 140, 0));
    }

    #[test]
    fn get_success_color_returns_palette_success() {
        let color = get_success_color("deuteranopia");
        assert_eq!(color, Color::Rgb(0, 150, 200));
    }

    #[test]
    fn get_warning_color_returns_palette_warning() {
        let color = get_warning_color("deuteranopia");
        assert_eq!(color, Color::Rgb(255, 180, 0));
    }
}
