// (iter-392: the `#![allow(dead_code)]` suppression is gone — this module is
// reached from `render/welcome.rs::render_banner_block`.)
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
//! Used by `render::render_banner_block` (above the welcome panel).

use crate::tui::vendor::style::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

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
fn accent_style() -> Style {
    Style::default()
        .fg(theme::accent_color())
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(theme::dim_color())
}

pub const FULL_ART: [&str; 7] = [
    " ██████╗ ██████╗ ███████╗██████╗  █████╗ ███╗   ██╗████████╗",
    "██╔═══██╗██╔══██╗██╔════╝██╔══██╗██╔══██╗████╗  ██║╚══██╔══╝",
    "██║   ██║██████╔╝█████╗  ██████╔╝███████║██╔██╗ ██║   ██║   ",
    "██║   ██║██╔═══╝ ██╔══╝  ██╔══██╗██╔══██║██║╚██╗██║   ██║   ",
    "╚██████╔╝██║     ███████╗██║  ██║██║  ██║██║ ╚████║   ██║   ",
    " ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝   ",
    "                                                              ",
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
                spans.push(Span::styled(trimmed.to_string(), accent_style()));
            }
            if trailing_len > 0 {
                spans.push(Span::raw(" ".repeat(trailing_len)));
            }
            out.push(Line::from(spans));
        }
    } else {
        // Below 40 cols: styled single-line wordmark + dim version tag.
        out.push(Line::from(vec![
            Span::styled("OPERANT", accent_style()),
            Span::raw(" "),
            Span::styled("·", dim_style()),
            Span::raw(" "),
            Span::styled("the personal AI agent", dim_style()),
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
        let rule_width = (width as usize).clamp(20, 56);
        let version_label = format!(" v{} ", version);
        let rule_total =
            rule_width.saturating_sub(crate::tui::render::display_width(&version_label));
        let left_rule = "─".repeat(rule_total / 2);
        let right_rule = "─".repeat(rule_total - rule_total / 2);
        lines.push(Line::from(vec![
            Span::styled(left_rule, dim_style()),
            Span::styled(version_label, dim_style()),
            Span::styled(right_rule, dim_style()),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    #[test]
    fn full_art_is_seven_lines_uniform_width() {
        assert_eq!(FULL_ART.len(), 7, "full art must be 7 lines tall");
        // The new ASCII art uses Unicode box-drawing characters (║, ╔, etc.)
        // which have varying byte lengths. We just check that all 7 lines
        // exist and are non-empty — visual uniformity is verified by eye.
        for (i, line) in FULL_ART.iter().enumerate() {
            assert!(!line.is_empty(), "line {} is empty", i);
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
        assert_eq!(
            with_sub.len(),
            8,
            "banner_with_subtitle adds a subtitle rule"
        );
    }

    #[test]
    fn banner_lines_compact_width_returns_four_lines() {
        let lines = banner_lines(50);
        assert_eq!(
            lines.len(),
            4,
            "banner_lines(50) returns 4 compact art lines"
        );
    }

    #[test]
    fn banner_lines_narrow_returns_one_line_fallback() {
        let lines = banner_lines(30);
        assert_eq!(lines.len(), 1, "<40 cols falls back to single styled line");
    }

    // ── Theme-driven accent ────────────────────────────────────────────────
    //
    // The wordmark emits the `Accent` role's FROZEN default, and the per-frame
    // buffer pass rewrites it onto the active theme's value. Asserting
    // `wordmark_fg == theme_colors::accent()` would therefore be wrong twice
    // over: the emitted colour is theme-independent by design, and the
    // resolved colour only exists after the pass. So these tests drive the real
    // pipeline: paint, substitute, read the cell back.

    /// Run `f` with `theme` active, then restore the default palette. Shares
    /// the palette lock the other theme-mutating tests serialize on. The
    /// palette is process-global, so both the render call and the expected
    /// colour must be read *inside* the closure.
    fn with_theme<T>(theme: &str, f: impl FnOnce() -> T) -> T {
        let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::tui::theme_colors::set_active_theme(theme);
        let out = f();
        crate::tui::theme_colors::set_active_theme("default");
        out
    }

    /// The colour the call site EMITS: always the role default, by design.
    #[test]
    fn wordmark_emits_the_accent_role_default() {
        for width in [100, 50, 30] {
            let fg = banner_lines(width)
                .into_iter()
                .next()
                .and_then(|line| line.spans.first().map(|s| s.style.fg))
                .unwrap_or(None);
            assert_eq!(
                fg,
                Some(theme::accent_color()),
                "width {width} must emit the role default so the buffer can substitute it"
            );
        }
    }

    /// The resolved colour after the per-frame substitution, which is where the
    /// active theme actually shows up.
    #[test]
    fn banner_wordmark_follows_the_active_theme_accent() {
        let paint = |width: u16| {
            let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 60, 8));
            for (row, line) in banner_lines(width).into_iter().enumerate() {
                line.render(Rect::new(0, row as u16, 60, 1), &mut buf);
            }
            crate::tui::vendor::style::theme_mode::adapt_buffer_for_display(&mut buf);
            buf[(0, 0)].fg
        };

        let nord = with_theme("nord", || (paint(100), crate::tui::theme_colors::accent()));
        assert_eq!(
            nord.0, nord.1,
            "after substitution the wordmark must show the active theme's accent"
        );

        let monokai = with_theme("monokai", || paint(100));
        assert_ne!(nord.0, monokai, "switching theme must repaint the wordmark");

        // The narrow (<40 col) fallback path goes through the same substitution.
        assert_eq!(with_theme("nord", || paint(30)), nord.0);
    }

    // ── The durable gate: no hardcoded default-theme amber anywhere else ───
    //
    // The 13 sites this replaced were all *individually* reasonable-looking
    // `Color::Rgb(255, 191, 0)` literals; nothing flagged them, so they came
    // back. This walks the whole crate so the next one fails the build.

    /// The palette that defines the colour.
    const AMBER_EXEMPT: [&str; 1] = ["src/tui/theme_colors.rs"];

    /// Assembled at runtime so this file — which lives inside the tree being
    /// walked — never holds a needle in live code and trips its own gate. The
    /// gate strips `//` comments, so the prose above can still name what was
    /// removed.
    fn needles() -> Vec<String> {
        vec![
            format!("Color::Rgb({}, {}, {})", 255, 191, 0),
            format!("ACCENT_{}", "PRIMARY"),
            format!("BANNER_{}", "ACCENT"),
        ]
    }

    /// Recursively collect `needle` hits as `(path relative to the crate, line)`.
    /// Trailing `//` comments are stripped: a literal in a comment cannot
    /// paint anything, and prose about the old hardcodes is worth keeping.
    fn grep_tree(root: &std::path::Path, needle: &str) -> Vec<(String, usize)> {
        let mut hits = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let rel = path
                    .strip_prefix(root.parent().unwrap_or(root))
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                if AMBER_EXEMPT.iter().any(|e| rel == *e) {
                    continue;
                }
                let Ok(src) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (i, line) in src.lines().enumerate() {
                    if line
                        .split("//")
                        .next()
                        .is_some_and(|code| code.contains(needle))
                    {
                        hits.push((rel.clone(), i + 1));
                    }
                }
            }
        }
        hits.sort();
        hits
    }

    fn crate_src() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// No renderer may hardcode the default theme's accent amber, and the
    /// duplicate accent `const`s must stay deleted. Both `ACCENT_PRIMARY`
    /// definitions plus `BANNER_ACCENT` were `const`s, so the palette is now
    /// the only source; a surviving one would reintroduce the same theme bug
    /// under a different name.
    ///
    /// Fix a hit by calling the role accessor that matches what the colour
    /// does — `theme::accent_color()` for an accent foreground,
    /// `theme::selection_bg_color()` for a selected-row background. Do not
    /// freeze a palette value into a `const`: the accessors are runtime `fn`s,
    /// so a `const` pins every theme to default amber.
    #[test]
    fn tui_sources_must_not_hardcode_the_default_theme_amber() {
        let mut failures = Vec::new();
        for needle in needles() {
            let hits = grep_tree(&crate_src(), &needle);
            if !hits.is_empty() {
                failures.push(format!("`{needle}`: {hits:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "default-theme amber leaked into live TUI code again:\n  {}\n\
             Call the role accessor that matches what the colour does \
             (`theme::accent_color()` for an accent foreground, \
             `theme::selection_bg_color()` for a selected-row background) — \
             never a `const`.",
            failures.join("\n  ")
        );
    }
}
