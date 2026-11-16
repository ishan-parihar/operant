//! Status-line message machinery (plan 2026-11-16, the `/bar` retraction).
//!
//! The status row is assembled imperatively in `operant_ui::ui_input::draw_status`
//! using the `· ` boundary literal (`SEGMENT_SEPARATOR` in that module). The
//! half-port that added a `StatusLineBg` grey row, a `[tui] bar_style` config
//! key, powerline caps and a `/bar` picker was removed entirely; what remains is
//! the shared meter the markdown lexer and the trace cards read.

pub mod meter;
