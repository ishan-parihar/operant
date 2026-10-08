// Vendored from jcode (crates/operant-core/src/output_style.rs + the adapter
// in crates/operant-tui/src/tui/ui/output_style.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported @ 0a9dc7805 (batch-4 sweep tail).
//
// [port-decision] unicode_properties (the exact Unicode emoji tables) is a
// pending dependency decision, like ratatui_image. Until it is approved, the
// classifier predicates below use fixed codepoint ranges instead of the
// tables: presentation selectors, regional indicators, ZWJ and the keycap
// combiner are exact by definition; `is_emoji_char`/`emoji_status` use the
// coarse Emoji-Presentation blocks. Ceiling: exotic emoji outside the common
// presentation blocks classify as text (left verbatim, not stripped) — the
// upstream tests' contract holds. Re-point the five predicates at
// unicode_properties::emoji when the dep is approved.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_segmentation::UnicodeSegmentation;

static EMOJI_ENABLED: AtomicBool = AtomicBool::new(true);

/// Set whether terminal-facing output may contain emoji.
pub fn set_emoji_enabled(enabled: bool) {
    EMOJI_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Return whether terminal-facing output may contain emoji.
pub fn emoji_enabled() -> bool {
    EMOJI_ENABLED.load(Ordering::Relaxed)
}

/// Adapt terminal-facing text to the configured emoji preference.
pub fn terminal_text(text: &str) -> Cow<'_, str> {
    terminal_text_with_emoji(text, emoji_enabled())
}

/// Adapt terminal-facing text using an explicit emoji preference.
pub fn terminal_text_with_emoji(text: &str, enabled: bool) -> Cow<'_, str> {
    if enabled || text.is_ascii() || !contains_emoji(text) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(replace_emoji_with_ascii(text))
    }
}

/// Replace emoji grapheme clusters with compact ASCII markers.
pub fn replace_emoji_with_ascii(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for grapheme in text.graphemes(true) {
        if grapheme_is_emoji(grapheme) {
            output.push_str(emoji_ascii_fallback(grapheme));
        } else {
            output.push_str(grapheme);
        }
    }
    output
}

fn contains_emoji(text: &str) -> bool {
    text.graphemes(true).any(grapheme_is_emoji)
}

fn emoji_ascii_fallback(grapheme: &str) -> &'static str {
    if grapheme.chars().any(|ch| matches!(ch, '✓' | '✔' | '✅')) {
        "+"
    } else if grapheme
        .chars()
        .any(|ch| matches!(ch, '✕' | '✗' | '❌' | '❎'))
    {
        "x"
    } else if grapheme.chars().any(|ch| matches!(ch, '⚠' | '🚨')) {
        "!"
    } else if grapheme
        .chars()
        .any(|ch| matches!(ch, '➡' | '👉' | '➜' | '➤'))
    {
        "->"
    } else if grapheme.chars().any(|ch| matches!(ch, '⬅' | '👈')) {
        "<-"
    } else {
        "*"
    }
}

// --- degraded classifier (see header [port-decision]) ---

fn is_text_presentation_selector(ch: char) -> bool {
    ch == '\u{FE0E}'
}

fn is_emoji_presentation_selector(ch: char) -> bool {
    ch == '\u{FE0F}'
}

fn is_regional_indicator(ch: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch)
}

fn is_zwj(ch: char) -> bool {
    ch == '\u{200D}'
}

fn is_emoji_char(ch: char) -> bool {
    matches!(ch as u32,
        0x1F300..=0x1F5FF   // misc symbols & pictographs
        | 0x1F600..=0x1F64F // emoticons
        | 0x1F680..=0x1F6FF // transport & map
        | 0x1F900..=0x1F9FF // supplemental symbols
        | 0x1FA70..=0x1FAFF // symbols & pictographs extended-A
    )
}

fn emoji_presentation(ch: char) -> bool {
    matches!(
        ch,
        '\u{1F300}'
            ..='\u{1FAFF}' // emoji presentation by default in these blocks
        | '✅' | '❌' | '❎' | '➡' | '➜' | '➤' | '⬅'
    )
}

fn grapheme_is_emoji(grapheme: &str) -> bool {
    let has_text_selector = grapheme.chars().any(is_text_presentation_selector);
    let has_emoji_selector = grapheme.chars().any(is_emoji_presentation_selector);
    if has_text_selector && !has_emoji_selector {
        return false;
    }

    let regional_indicators = grapheme
        .chars()
        .filter(|ch| is_regional_indicator(*ch))
        .count();
    has_emoji_selector
        || grapheme.contains('\u{20E3}')
        || regional_indicators >= 2
        || (grapheme.chars().any(is_emoji_char) && grapheme.chars().any(is_zwj))
        || grapheme.chars().any(emoji_presentation)
}

// --- TUI frame adapter (upstream: operant-tui/src/tui/ui/output_style.rs) ---

pub fn adapt_buffer_for_emoji_preference(buffer: &mut ratatui::buffer::Buffer) {
    adapt_buffer_for_emoji_enabled(buffer, emoji_enabled());
}

fn adapt_buffer_for_emoji_enabled(buffer: &mut ratatui::buffer::Buffer, enabled: bool) {
    if enabled {
        return;
    }
    for cell in &mut buffer.content {
        if let Cow::Owned(symbol) = terminal_text_with_emoji(cell.symbol(), enabled) {
            cell.set_symbol(&symbol);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_clusters_use_readable_ascii_fallbacks() {
        assert_eq!(
            replace_emoji_with_ascii(
                "🐝 ready ✅ warning ⚠️ failed ❌ family 👨‍👩‍👧‍👦 tone 👋🏽 flag 🇺🇸 key 1️⃣"
            ),
            "* ready + warning ! failed x family * tone * flag * key *"
        );
    }

    #[test]
    fn non_emoji_unicode_is_preserved() {
        assert_eq!(
            replace_emoji_with_ascii("box ─│ arrows →←↔ CJK 中文 math α © ® ✓ ✗ ⚠"),
            "box ─│ arrows →←↔ CJK 中文 math α © ® ✓ ✗ ⚠"
        );
        assert_eq!(replace_emoji_with_ascii("text heart ♥︎"), "text heart ♥︎");
    }

    #[test]
    fn no_emoji_mode_rewrites_completed_frame_cells_to_ascii() {
        let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 24, 1));
        buffer.set_string(0, 0, "🐝 ready ✅ box ─", ratatui::style::Style::default());
        adapt_buffer_for_emoji_enabled(&mut buffer, false);
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert_eq!(
            rendered.split_whitespace().collect::<Vec<_>>(),
            vec!["*", "ready", "+", "box", "─"]
        );
        assert!(!rendered.contains(['🐝', '✅']));
    }
}
