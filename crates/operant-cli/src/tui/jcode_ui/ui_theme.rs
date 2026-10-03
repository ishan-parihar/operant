// Vendored from jcode (crates/jcode-tui/src/tui/ui_theme.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; jcode_tui_render
// re-rooted to crate::tui::jcode_render, jcode_tui_style::theme to
// crate::tui::vendor::style::theme. See jcode_ui/mod.rs for scope.

pub(super) use crate::tui::vendor::style::theme::{
    accent_color, ai_color, ai_text, asap_color, blend_color, dim_color, file_link_color,
    header_icon_color, header_name_color, header_session_color, pending_color,
    prompt_entry_bg_color, prompt_entry_color, prompt_entry_shimmer_color, queued_color,
    rainbow_prompt_color, system_message_color, tool_color, user_bg, user_color, user_text,
};

pub(super) fn activity_indicator_frame_index(elapsed: f32, fps: f32) -> usize {
    crate::tui::vendor::style::theme::activity_indicator_frame_index(
        elapsed,
        fps,
        crate::tui::jcode_app::perf::tui_policy().enable_decorative_animations,
    )
}

pub(super) fn activity_indicator(elapsed: f32, fps: f32) -> &'static str {
    crate::tui::vendor::style::theme::activity_indicator(
        elapsed,
        fps,
        crate::tui::jcode_app::perf::tui_policy().enable_decorative_animations,
    )
}
