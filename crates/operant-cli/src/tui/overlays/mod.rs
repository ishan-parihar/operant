// overlays/mod.rs — All full-screen and floating overlays:
//   - HelpOverlay (? / F1 / /help)
//   - HistorySearchOverlay (Ctrl+R)
//   - MessageSelectorOverlay (/rewind step 1)
//   - RewindFlowOverlay (/rewind full multi-step flow)
//   - GlobalSearchState (ripgrep search dialog)

mod global_search;
mod help;
mod history_search;
mod layout;
mod message_selector;
mod rewind_flow;

pub use global_search::*;
pub use help::*;
pub use history_search::*;
pub use layout::*;
pub use message_selector::*;
pub use rewind_flow::*;

#[cfg(test)]
mod tests;
