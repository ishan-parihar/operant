// Vendored from jcode (crates/jcode-tui/src/tui/fuzzy.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole file
// (25 lines). Only delta: `jcode_fuzzy::` re-rooted to
// `crate::tui::jcode_app::fuzzy_engine::`. See jcode_app/mod.rs for scope.
//! Shared typo-resistant fuzzy matching adapters for TUI slash commands.

pub(crate) fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    crate::tui::jcode_app::fuzzy_engine::command_fuzzy_score(needle, haystack)
}

pub(crate) fn fuzzy_match_positions(needle: &str, haystack: &str) -> Vec<usize> {
    crate::tui::jcode_app::fuzzy_engine::command_fuzzy_match_positions(needle, haystack)
}

#[cfg(test)]
mod tests {
    #[test]
    fn slash_commands_tolerate_interior_typos() {
        assert!(
            crate::tui::jcode_app::fuzzy_engine::command_fuzzy_match("/conifg", "/config")
                .is_some()
        );
        assert!(
            crate::tui::jcode_app::fuzzy_engine::command_fuzzy_match("/comapct", "/compact")
                .is_some()
        );
        assert!(
            crate::tui::jcode_app::fuzzy_engine::command_fuzzy_match("/memroy", "/memory")
                .is_some()
        );
    }

    #[test]
    fn slash_commands_remain_anchored() {
        assert!(
            crate::tui::jcode_app::fuzzy_engine::command_fuzzy_match("/g", "/config").is_none()
        );
        assert!(crate::tui::jcode_app::fuzzy_engine::command_fuzzy_match("/g", "/goals").is_some());
    }
}
