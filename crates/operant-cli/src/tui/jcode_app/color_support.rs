// Vendored from jcode (crates/jcode-tui/src/tui/color_support.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole file
// (1 line). Delta: `jcode_tui_style::color::*` re-rooted to
// `crate::tui::vendor::style::color::*`. See jcode_app/mod.rs for scope.
#[allow(unused_imports)] // re-export: the sed'd renderers reach rgb through this path
pub use crate::tui::vendor::style::color::*;
