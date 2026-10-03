// Vendored from jcode (crates/jcode-tui-style), MIT License, Copyright (c) 2025 Jeremy Huang.
// Adapted for operant: dropped the `harmony` module and its re-exports (Oklab palette
// scoring, out of scope per docs/PLAN-TUI-OVERHAUL.md §2(d)); replaced
// `jcode_logging::warn` with operant-cli's `tracing::warn`.
//
// Upstream this file is `lib.rs` (it was a standalone crate root); it is `mod.rs`
// here so a plain `pub mod style;` in `tui/vendor/mod.rs` resolves to it.

// This is a library surface vendored into a *private* module of a binary crate.
// rustc cannot see that operant's TUI callers — which are still being migrated
// onto these roles — will reach the accessors, so every `pub` item reads as dead
// and the re-exports read as unused. Deleting the not-yet-wired accessors would
// defeat the port, so the lint is silenced for this module only. The four
// submodules inherit the level; the individual `pub use` re-exports below are
// annotated separately so real unused imports elsewhere in the module still warn.
#![allow(dead_code)]

// jcode-tui-style: the TUI design system's module root.
//
// Depend on it as `crate::tui::vendor::style` (or `super::style` from a sibling
// under `tui/vendor/`). The four public surfaces are:
//   * `palette` — the 22 semantic `Role`s, the frozen default table, and
//     `set_palette` for a user-configured palette.
//   * `theme`   — one accessor per role, plus the animation helpers.
//   * `theme_mode` — `adapt_buffer_for_display`, the per-frame substitution
//     choke point every widget's colors flow through.
//   * `color`   — plain RGB color construction and the indexed-colour
//     introspection `theme_mode` needs.

pub mod color;
pub mod palette;
pub mod theme;
pub mod theme_mode;

#[allow(unused_imports)]
pub use color::{clear_buf, indexed_to_rgb, rgb};
#[allow(unused_imports)]
pub use palette::{ALL_ROLES, Palette, Role, palette, role_color, set_palette};
#[allow(unused_imports)]
pub use theme_mode::{
    ThemeMode, adapt_buffer, adapt_buffer_for_display, adapt_buffer_for_theme,
    adapt_color_for_theme, adapt_foreground_for_display, adapt_foreground_for_theme,
    is_light_theme, set_theme_mode, theme_mode,
};

/// The active palette and theme mode are process-global. One lock serializes
/// every test that reads or mutates them, wherever the test lives (palette and
/// theme-mode tests used to carry separate locks and could interleave).
#[cfg(test)]
pub(crate) static STYLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restore the terminal, logging any failure instead of printing it.
///
/// `ratatui::restore()` reports failures with `eprintln!`. On a dead terminal
/// (closed window, dropped SSH) the restore fails with EIO and stderr is
/// equally dead, so that `eprintln!` itself panics inside
/// `std::io::stdio::print_to`. The panic hook can then record a live session as
/// crashed and clobber its snapshot.
///
/// Always prefer this over `ratatui::restore()` on cleanup and orphan-exit
/// paths. See issue #599 (and #129, the same class via another path).
pub fn restore_terminal_quietly() {
    if let Err(error) = ratatui::try_restore() {
        // `tracing` rather than `eprintln!`: operant-cli's subscriber writes to a
        // log file, `io::sink`, or stderr depending on config, and is a no-op when
        // no subscriber is installed. A direct stderr write could panic on the very
        // dead terminal this function exists to tolerate.
        tracing::warn!(%error, "failed to restore terminal");
    }
}
