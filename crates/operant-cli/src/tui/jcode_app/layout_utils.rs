// Vendored from jcode (crates/jcode-tui/src/tui/layout_utils.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole file
// (29 lines). Deltas: `jcode_tui_render::layout::` re-rooted to
// `crate::tui::jcode_render::layout::`; [port-excision] the `visual_debug`
// import + `rect_from_capture` (they reference the cut visual_debug module,
// unreferenced by the ported tree). See jcode_app/mod.rs for scope.
pub(crate) use crate::tui::jcode_render::layout::{parse_area_spec, point_in_rect, rect_contains};
use ratatui::layout::Rect;


