// Vendored from jcode (crates/operant-tui-markdown/src/markdown_tests/mod.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805,
// no changes. See operant_markdown/mod.rs for scope.

use super::*;

fn line_to_string(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn leading_spaces(text: &str) -> usize {
    text.chars().take_while(|c| *c == ' ').count()
}

fn render_markdown_with_mode(text: &str, mode: MarkdownSpacingMode) -> Vec<Line<'static>> {
    with_markdown_spacing_mode_override(Some(mode), || render_markdown(text))
}

fn render_markdown_with_width_and_mode(
    text: &str,
    width: usize,
    mode: MarkdownSpacingMode,
) -> Vec<Line<'static>> {
    with_markdown_spacing_mode_override(Some(mode), || {
        render_markdown_with_width(text, Some(width))
    })
}

fn lines_to_string(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(line_to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[path = "cases.rs"]
mod cases;

// ---------------------------------------------------------------------------
// Text-layer wiring (plan 2026-11-14 U4)
// ---------------------------------------------------------------------------

/// The three rewired helpers must still return the literal they replaced.
///
/// The role's frozen default IS that literal, so this is what makes U4 a
/// zero-pixel change: if a role default is ever edited, this fails and says the
/// markdown surface moved. Deliberately asserted against the LITERAL, not
/// against the role, so it also catches the role being re-pointed at a
/// different slot.
#[test]
fn the_rewired_markdown_helpers_still_return_their_historical_literals() {
    assert_eq!(code_fg(), Color::Rgb(180, 180, 180));
    assert_eq!(link_fg(), Color::Rgb(120, 180, 240));
    assert_eq!(heading_color(), Color::Rgb(200, 155, 75));
}

/// And the roles the helpers now read must be CONFIGURABLE — U4's whole point.
///
/// Note what this does NOT assert: `code_fg()` returning the configured colour.
/// `role_color` deliberately returns the role's frozen DEFAULT, because the
/// per-frame buffer pass is what substitutes the configured value, and doing it
/// in the accessor too would remap the same cell twice. So the contract is
/// asserted where it lives: the configured palette must carry the override for
/// the exact role the helper reads.
#[test]
fn the_roles_the_markdown_helpers_read_are_configurable() {
    let guard = crate::tui::vendor::style::STYLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut pairs = std::collections::BTreeMap::new();
    pairs.insert("md_code".to_string(), "#010203".to_string());
    crate::tui::theme_colors::set_role_overrides(&pairs);
    crate::tui::theme_colors::set_active_theme("dark");

    let live = crate::tui::vendor::style::palette::configured_palette()
        .expect("a configured role must install a palette");
    assert_eq!(
        live.rgb(crate::tui::vendor::style::palette::Role::MdCode),
        (1, 2, 3),
        "the role code_fg() reads must take the operator's colour, or U4 wired nothing"
    );
    // The helper still resolves (to the default the buffer pass rewrites).
    assert_eq!(code_fg(), Color::Rgb(180, 180, 180));

    crate::tui::theme_colors::set_role_overrides(&std::collections::BTreeMap::new());
    crate::tui::theme_colors::set_active_theme("default");
    drop(guard);
}
