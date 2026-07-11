//! Figure/icon constants matching src/constants/figures.ts
//!
//! (iter-136b: pruned 11 dead constants — BULLET_OPERATOR, PLAY_ICON,
//! PAUSE_ICON, FORK_GLYPH, DIAMOND_OPEN, DIAMOND_FILLED, FLAG_ICON,
//! HEAVY_HORIZONTAL, THEREFORE, NEW_MESSAGES_DOWN, BRIDGE_READY,
//! BRIDGE_FAILED — all had zero callers per grep.)

// Platform-aware: on Windows use ● (U+25CF), elsewhere ⏺ (U+23FA)
pub fn black_circle() -> &'static str {
    if cfg!(target_os = "windows") {
        "●"
    } else {
        "⏺"
    }
}
pub const TEARDROP_ASTERISK: &str = "✻"; // U+273B - used for thinking/compact
pub const BLOCKQUOTE_BAR: &str = "▎"; // U+258E - blockquote left bar
