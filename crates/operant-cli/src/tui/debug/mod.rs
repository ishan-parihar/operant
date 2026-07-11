//! TUI debugging infrastructure — event bus, debug hub, F12 overlay.
//!
//! This module provides the observability layer that the operant TUI was
//! missing (per audit-report §4.5). It is:
//!
//! - **Non-destructive**: purely additive. Existing behavior is unchanged
//!   when the infrastructure is disabled (default).
//! - **Toggleable**: gated by `OPERANT_TUI_DEBUG=1` env var or F12 key.
//!   Zero overhead when off (all hot paths use `AtomicBool::load(Relaxed)`).
//! - **Serializable**: all debug state is JSON-serializable for diffing.
//!
//! ## Components
//!
//! | Component        | File           | Purpose                                  |
//! |------------------|----------------|------------------------------------------|
//! | `TuiEventBus`    | `event_bus.rs` | Typed event ring buffer (last 1000)     |
//! | `TuiDebugHub`    | `debug_hub.rs` | Centralized debug state (frame count, etc)|
//! | `TuiDebugOverlay`| `overlay.rs`   | F12 in-TUI panel showing live debug info |
//!
//! ## Usage
//!
//! In `App::new`:
//! ```ignore
//! let debug_hub = TuiDebugHub::new_from_env();
//! // store in self.debug_hub
//! ```
//!
//! In `App::run` loop, after `terminal.draw`:
//! ```ignore
//! self.debug_hub.record_frame(render_ms);
//! ```
//!
//! In `App::handle_key_event`, at the top:
//! ```ignore
//! if key.code == KeyCode::F12 {
//!     self.debug_hub.toggle_overlay();
//!     return true;
//! }
//! ```
//!
//! In `render::render_app`, after the main render:
//! ```ignore
//! if app.debug_hub.overlay_visible() {
//!     debug::overlay::render_debug_overlay(f, &app.debug_hub, area);
//! }
//! ```

pub mod debug_hub;
pub mod event_bus;
pub mod overlay;

pub use debug_hub::TuiDebugHub;
pub use event_bus::TuiEvent;
