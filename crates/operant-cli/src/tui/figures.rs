//! Figure/icon constants matching src/constants/figures.ts
//!
//! (iter-136b: pruned 11 dead constants — BULLET_OPERATOR, PLAY_ICON,
//! PAUSE_ICON, FORK_GLYPH, DIAMOND_OPEN, DIAMOND_FILLED, FLAG_ICON,
//! HEAVY_HORIZONTAL, THEREFORE, NEW_MESSAGES_DOWN, BRIDGE_READY,
//! BRIDGE_FAILED — all had zero callers per grep.)

// Platform-aware: on Windows use ● (U+25CF), elsewhere ⏺ (U+23FA)
pub fn black_circle() -> &'static str {
    if cfg!(target_os = "windows") { "●" } else { "⏺" }
}
pub const TEARDROP_ASTERISK: &str = "✻";     // U+273B - used for thinking/compact
pub const UP_ARROW: &str = "↑";              // U+2191
pub const DOWN_ARROW: &str = "↓";            // U+2193
pub const LIGHTNING_BOLT: &str = "↯";        // U+21AF - fast mode
pub const EFFORT_LOW: &str = "○";            // U+25CB
pub const EFFORT_MEDIUM: &str = "◐";         // U+25D0
pub const EFFORT_HIGH: &str = "●";           // U+25CF
pub const EFFORT_MAX: &str = "◉";            // U+25C9
pub const REFRESH_ARROW: &str = "↻";         // U+21BB
pub const REFERENCE_MARK: &str = "※";        // U+203B - away summary marker
pub const BLOCKQUOTE_BAR: &str = "▎";        // U+258E - blockquote left bar
