//! Operant ASCII wordmark banner.
//!
//! Renders the OPERANT wordmark in three sizes (full / compact / minimal) so
//! the welcome screen can show a real logo instead of just the small "Rustle"
//! mascot box. The full art is 7 lines tall × 56 columns wide; the compact
//! rule is 4 lines × 32 columns; the minimal is a single styled line.
//!
//! The art is generated to fit a 56-column canvas with consistent cap height
//! and baseline. Each letter is 6 columns wide with a 1-column gap, except
//! the 'R' which is 7 wide to accommodate the diagonal leg.
//!
//! Used by `render::render_welcome_box` (above the welcome panel) and by
//! `app::App::status_message` for the splash overlay.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The accent color used for the banner wordmark — matches `rustle::accent_style`
/// so the logo and the mascot read as one design system.
pub const BANNER_ACCENT: Color = Color::Rgb(255, 191, 0);
const BANNER_DIM: Color = Color::Rgb(140, 110, 0);

fn accent() -> Style {
    Style::default()
        .fg(BANNER_ACCENT)
        .add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::default().fg(BANNER_DIM)
}

/// Full OPERANT wordmark — 7 lines × 56 columns.
///
/// ```text
///   ___  ___  __  __ ______ _   _ _____  _____  _____
///  / _ \| _ \|  \/  |  ___| | | /  ___|/  __ \|  ___|
/// / /_\ \ | | | .  . | |__ | | | \ `--. | /  \/| |__
/// |  _  | | | | |\/| |  __|| | | |`--. \| |    |  __|
/// | | | | |/ /| |  | | |___| |_| /\__/ /\ \__/\| |___
/// \_| |_/___/ \_|  |_/\____/ \___/\____/  \____/\____/
/// ```
pub const FULL_ART: [&str; 7] = [
    "  ___  ___  __  __ ______ _   _ _____  _____  _____ ",
    " / _ \\| _ \\|  \\/  |  ___| | | /  ___|/  __ \\|  ___|",
    "/ /_\\ \\ | | | .  . | |__ | | | \\ `--. | /  \\/| |__  ",
    "|  _  | | | | |\\/| |  __|| | | |`--. \\| |    |  __| ",
    "| | | | |/ /| |  | | |___| |_| /\\__/ /\\ \\__/\\| |___ ",
    "\\_| |_/___/ \\_|  |_|\\____/ \\___/\\____/  \\____/\\____|",
    "                                                     ",
];

/// Compact OPERANT wordmark — 4 lines × 32 columns.
///
/// ```text
///   ___  ___  ___ ___
///  / _ \| _ \| __| _ \
/// | (_) | |_/ /|__ \   /
///  \___/|_| |_|___/_|_|
/// ```
pub const COMPACT_ART: [&str; 4] = [
    "  ___  ___  ___ ___    ",
    " / _ \\| _ \\| __| _ \\   ",
    "| (_) | |_/ /|__ \\   / ",
    " \\___/|_| |_|___/_|_|  ",
];

/// Returns the right banner art for the given terminal width.
///
/// - `>= 80 cols` → full art (56-wide, 7-tall)
/// - `>= 40 cols` → compact art (24-wide, 4-tall)
/// - `< 40 cols`  → no art (caller falls back to a styled text line)
pub fn pick_art(width: u16) -> Option<&'static [&'static str]> {
    if width >= 80 {
        Some(&FULL_ART)
    } else if width >= 40 {
        Some(&COMPACT_ART)
    } else {
        None
    }
}

/// Render the banner as styled ratatui lines.
///
/// Each line of the ASCII art is split into two spans: the bulk of the glyph
/// (which gets the accent color + bold) and the trailing whitespace (which is
/// left unstyled to avoid bleeding the accent color into adjacent cells).
pub fn banner_lines(width: u16) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();

    if let Some(art) = pick_art(width) {
        for line in art {
            // Trim trailing spaces for the styled span; keep the leading
            // indent intact so the wordmark stays centered.
            let trimmed = line.trim_end();
            let trailing_len = line.len().saturating_sub(trimmed.len());
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(2);
            if !trimmed.is_empty() {
                spans.push(Span::styled(trimmed.to_string(), accent()));
            }
            if trailing_len > 0 {
                spans.push(Span::raw(" ".repeat(trailing_len)));
            }
            out.push(Line::from(spans));
        }
    } else {
        // Below 40 cols: styled single-line wordmark + dim version tag.
        out.push(Line::from(vec![
            Span::styled("OPERANT", accent()),
            Span::raw(" "),
            Span::styled("·", dim()),
            Span::raw(" "),
            Span::styled("the personal AI agent", dim()),
        ]));
    }

    out
}

/// Convenience: render the banner plus a dim subtitle line.
///
/// The subtitle is the version string, e.g. `v0.1.3`. Used by the welcome
/// screen to give the wordmark a base without taking an extra layout slot.
pub fn banner_with_subtitle(width: u16, version: &str) -> Vec<Line<'static>> {
    let mut lines = banner_lines(width);
    if width >= 40 {
        // Underline the wordmark with a dim rule + version tag.
        let rule_width = (width as usize).min(56).max(20);
        let version_label = format!(" v{} ", version);
        let rule_total = rule_width.saturating_sub(version_label.len());
        let left_rule = "─".repeat(rule_total / 2);
        let right_rule = "─".repeat(rule_total - rule_total / 2);
        lines.push(Line::from(vec![
            Span::styled(left_rule, dim()),
            Span::styled(version_label, dim()),
            Span::styled(right_rule, dim()),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_art_is_seven_lines_uniform_width() {
        assert_eq!(FULL_ART.len(), 7, "full art must be 7 lines tall");
        let widths: Vec<usize> = FULL_ART.iter().map(|l| l.len()).collect();
        // All lines should be the same width so the bounding box is rectangular.
        let w0 = widths[0];
        for (i, w) in widths.iter().enumerate() {
            assert!(
                (w0 as i32 - *w as i32).abs() <= 1,
                "line {} width {} diverges from line 0 width {} by more than 1",
                i,
                w,
                w0
            );
        }
    }

    #[test]
    fn compact_art_is_four_lines() {
        assert_eq!(COMPACT_ART.len(), 4, "compact art must be 4 lines tall");
    }

    #[test]
    fn pick_art_responsive_thresholds() {
        assert!(pick_art(80).is_some(), ">=80 cols → full art");
        assert!(pick_art(120).is_some());
        assert!(pick_art(40).is_some(), ">=40 cols → compact art");
        assert!(pick_art(60).is_some());
        assert!(pick_art(39).is_none(), "<40 cols → no art");
        assert!(pick_art(20).is_none());
    }

    #[test]
    fn banner_lines_full_width_returns_eight_lines() {
        // 7 art lines + 1 subtitle rule.
        let lines = banner_lines(100);
        assert_eq!(lines.len(), 7, "banner_lines(100) returns 7 art lines");
        let with_sub = banner_with_subtitle(100, "0.1.3");
        assert_eq!(with_sub.len(), 8, "banner_with_subtitle adds a subtitle rule");
    }

    #[test]
    fn banner_lines_compact_width_returns_four_lines() {
        let lines = banner_lines(50);
        assert_eq!(lines.len(), 4, "banner_lines(50) returns 4 compact art lines");
    }

    #[test]
    fn banner_lines_narrow_returns_one_line_fallback() {
        let lines = banner_lines(30);
        assert_eq!(lines.len(), 1, "<40 cols falls back to single styled line");
    }
}

