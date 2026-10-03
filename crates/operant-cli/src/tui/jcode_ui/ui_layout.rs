// Vendored from jcode (crates/jcode-tui/src/tui/ui_layout.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; jcode_tui_render
// re-rooted to crate::tui::jcode_render, jcode_tui_style::theme to
// crate::tui::vendor::style::theme. See jcode_ui/mod.rs for scope.

use ratatui::prelude::*;

pub(crate) use crate::tui::jcode_render::chrome::{
    align_if_unset, centered_content_block_width, left_aligned_content_inset,
    left_pad_lines_to_block_width,
};
pub(super) use crate::tui::jcode_render::chrome::{clear_area, draw_right_rail_chrome};

pub(super) fn right_rail_border_style(focused: bool, focus_color: Color) -> Style {
    crate::tui::jcode_render::chrome::right_rail_border_style(
        focused,
        focus_color,
        super::theme_support::dim_color(),
    )
}
