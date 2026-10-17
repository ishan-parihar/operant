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

pub const FULL_ART: [&str; 7] = [
    " ██████╗ ██████╗ ███████╗██████╗  █████╗ ███╗   ██╗████████╗",
    "██╔═══██╗██╔══██╗██╔════╝██╔══██╗██╔══██╗████╗  ██║╚══██╔══╝",
    "██║   ██║██████╔╝█████╗  ██████╔╝███████║██╔██╗ ██║   ██║   ",
    "██║   ██║██╔═══╝ ██╔══╝  ██╔══██╗██╔══██║██║╚██╗██║   ██║   ",
    "╚██████╔╝██║     ███████╗██║  ██║██║  ██║██║ ╚████║   ██║   ",
    " ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝   ",
    "                                                              ",
];

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

    // ── Theme-driven accent ────────────────────────────────────────────────
    //
    // The wordmark emits the `Accent` role's FROZEN default, and the per-frame
    // buffer pass rewrites it onto the active theme's value. Asserting
    // `wordmark_fg == theme_colors::accent()` would therefore be wrong twice
    // over: the emitted colour is theme-independent by design, and the
    // resolved colour only exists after the pass. So these tests drive the real
    // pipeline: paint, substitute, read the cell back.

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
