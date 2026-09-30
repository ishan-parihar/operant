// tui/space.rs — The TUI's spacing scale, in terminal cells.
//
// One doubling ladder and nothing else: no type scale, no token struct, no
// builder. `XXS * 2 == XS`, `XS * 2 == S`, and so on up to `XL`.
//
// Deliberately a *local* addition. `jcode-tui` ships no spacing scale at all
// (zero `Margin::`/`Padding::`/`theme::space` across the crate), so this is not
// a port of anything — it exists because every modal-sizing call site re-typed
// the same four numbers as `-2`, `-4`, or nothing at all. One ladder is the fix;
// a token hierarchy would be the same fix wearing a costume.
//
// The ladder is pinned by `overlays::layout::tests::spacing_scale_is_a_doubling_ladder`.

use ratatui::layout::Margin;

/// No inset.
#[allow(dead_code)] // unread until the call-site migration reaches for it
pub const XXS: u16 = 0;
/// The border inset, and a modal title's leading space.
pub const XS: u16 = 1;
/// A dialog's content gutter.
#[allow(dead_code)] // unread until the call-site migration reaches for it
pub const S: u16 = 2;
/// A modal's margin from the screen edge.
pub const M: u16 = 4;
/// The floor for a modal's width.
pub const L: u16 = 8;
/// The ceiling step: room for a short list without scrolling.
#[allow(dead_code)] // unread until the call-site migration reaches for it
pub const XL: u16 = 16;

/// Vertical-only inset. Horizontal stays 0.
pub const fn pad_v(vertical: u16) -> Margin {
    Margin::new(0, vertical)
}

/// Horizontal-only inset. Vertical stays 0.
pub const fn pad_h(horizontal: u16) -> Margin {
    Margin::new(horizontal, 0)
}
