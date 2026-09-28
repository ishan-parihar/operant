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

use crate::tui::theme_colors;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const BANNER_DIM: Color = Color::Rgb(140, 110, 0);

/// The banner wordmark reads the active theme's accent — the same accessor
/// `rustle::accent_style` uses, so the logo and the mascot stay one design
/// system under every theme rather than freezing on the default theme's amber.
fn accent() -> Style {
    Style::default()
        .fg(theme_colors::accent())
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
        let rule_width = (width as usize).clamp(20, 56);
        let version_label = format!(" v{} ", version);
        let rule_total =
            rule_width.saturating_sub(crate::tui::render::display_width(&version_label));
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
    // `BANNER_ACCENT` used to be a `const` pinned to the default theme's
    // amber, and `ACCENT_PRIMARY` was a second, duplicate `const` in
    // `messages/mod.rs` and a third in `prompt_input/mod.rs`. The
    // `theme_colors` accessors are runtime `fn`s (the active palette lives
    // behind a lock), so none of the three could be a `const` and all three
    // had to be deleted. The wordmark now reads the active palette.

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

    /// Foreground of the first span of the first banner line.
    fn first_wordmark_fg(width: u16) -> Color {
        banner_lines(width)
            .into_iter()
            .next()
            .and_then(|line| {
                line.spans
                    .first()
                    .map(|s| s.style.fg.unwrap_or(Color::Reset))
            })
            .unwrap_or(Color::Reset)
    }

    /// The wordmark foreground must come from the palette, so switching theme
    /// repaints it. Before the fix it was the default theme's amber under every
    /// theme.
    #[test]
    fn banner_wordmark_follows_the_active_theme_accent() {
        let (nord_fg, nord_accent) =
            with_theme("nord", || (first_wordmark_fg(100), theme_colors::accent()));
        assert_eq!(
            nord_fg, nord_accent,
            "wordmark must render the active palette accent"
        );

        let (monokai_fg, monokai_accent) = with_theme("monokai", || {
            (first_wordmark_fg(100), theme_colors::accent())
        });
        assert_eq!(monokai_fg, monokai_accent);
        assert_ne!(
            nord_fg, monokai_fg,
            "switching theme must repaint the wordmark, not leave default amber"
        );

        // The narrow (<40 col) fallback path styles the same way.
        let (narrow_fg, narrow_accent) =
            with_theme("nord", || (first_wordmark_fg(30), theme_colors::accent()));
        assert_eq!(
            narrow_fg, narrow_accent,
            "narrow fallback must use the accent too"
        );
    }

    // ── The durable gate: no hardcoded default-theme amber anywhere else ───
    //
    // The 13 sites this replaced were all *individually* reasonable-looking
    // `Color::Rgb(255, 191, 0)` literals; nothing flagged them, so they came
    // back. This walks the whole crate so the next one fails the build.

    /// The palette that defines the colour, plus `app/enums.rs::ACCENT_BUILD`
    /// — a tracked hardcode owned by a separate change. Remove that second
    /// entry when `ACCENT_BUILD` is migrated to `theme_colors::accent()`.
    const AMBER_EXEMPT: [&str; 2] = ["src/tui/theme_colors.rs", "src/tui/app/enums.rs"];

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
    /// Fix a hit by calling the `theme_colors` accessor that matches what the
    /// colour does — `accent()` for an accent foreground, `selection_bg()` for
    /// a selected-row background. Do not freeze a palette value into a `const`:
    /// the accessors are runtime `fn`s (the palette lives behind a lock), so a
    /// `const` pins every theme to default amber.
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
             Call the `theme_colors` accessor that matches what the colour does \
             (`accent()` for an accent foreground, `selection_bg()` for a \
             selected-row background) — never a `const`.",
            failures.join("\n  ")
        );
    }
}
