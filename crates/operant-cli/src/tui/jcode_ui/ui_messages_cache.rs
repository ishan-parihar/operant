// Vendored from jcode (crates/jcode-tui/src/tui/ui_messages_cache.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-3 map. See jcode_ui/mod.rs for scope.

use super::*;

pub(super) use crate::tui::jcode_model::{centered_wrap_width, left_pad_lines_for_centered_mode};

pub(crate) fn get_cached_message_lines<F>(
    msg: &DisplayMessage,
    width: u16,
    diff_mode: crate::tui::jcode_app::config_shim::DiffDisplayMode,
    render: F,
) -> Vec<Line<'static>>
where
    F: FnOnce(&DisplayMessage, u16, crate::tui::jcode_app::config_shim::DiffDisplayMode) -> Vec<Line<'static>>,
{
    crate::tui::jcode_model::get_cached_message_lines(
        msg,
        width,
        diff_mode,
        crate::tui::jcode_model::MessageCacheContext {
            diagram_mode: crate::tui::jcode_app::config_shim::config().display.diagram_mode,
            centered: markdown::center_code_blocks(),
            // Message lines contain Mermaid placeholder rows. Size clicks must
            // invalidate this cache just like a completed deferred render does.
            mermaid_epoch: crate::tui::jcode_app::mermaid::deferred_render_epoch()
                .wrapping_add(crate::tui::jcode_app::mermaid::mermaid_inline_expand_epoch()),
            mermaid_aspect_bucket: crate::tui::jcode_app::mermaid::current_preferred_aspect_ratio_bucket(),
            show_agentgrep_output: crate::tui::jcode_app::config_shim::config().display.show_agentgrep_output,
            show_bash_output: crate::tui::jcode_app::config_shim::config().display.show_bash_output,
            tool_call_details: crate::tui::jcode_app::config_shim::config().display.tool_call_details,
        },
        render,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_expand_transition_invalidates_cached_message_lines() {
        let hash = 0xf4ce_a123_u64;
        let msg = DisplayMessage::assistant(format!(
            "```mermaid\nflowchart LR\nA[cache-{hash}] --> B\n```"
        ));

        crate::tui::jcode_app::mermaid::set_mermaid_inline_expand_level(hash, 0);
        let first =
            get_cached_message_lines(&msg, 80, crate::tui::jcode_app::config_shim::DiffDisplayMode::Off, |_, _, _| {
                vec![Line::from("fit")]
            });
        assert_eq!(first.len(), 1);

        crate::tui::jcode_app::mermaid::set_mermaid_inline_expand_level(hash, 1);
        let second =
            get_cached_message_lines(&msg, 80, crate::tui::jcode_app::config_shim::DiffDisplayMode::Off, |_, _, _| {
                vec![Line::from("large-1"), Line::from("large-2")]
            });
        crate::tui::jcode_app::mermaid::set_mermaid_inline_expand_level(hash, 0);

        assert_eq!(
            second.len(),
            2,
            "the message cache must rerender after a Mermaid size transition"
        );
    }
}
