// overlays/mod.rs — All full-screen and floating overlays:
//   - HelpOverlay (? / F1 / /help)
//   - HistorySearchOverlay (Ctrl+R)
//   - GlobalSearchState (ripgrep search dialog)

mod global_search;
pub mod help;
mod history_search;
mod layout;

// The shared semantic colours used to be `OPERANT_*` constants in `layout`.
// They now resolve through the active theme palette; re-export the module so
// the `use super::*` sub-modules pick it up alongside the layout helpers.
use crate::tui::theme_colors;

pub use global_search::*;
pub use help::*;
pub use history_search::*;
pub use layout::*;

#[cfg(test)]
mod tests;
