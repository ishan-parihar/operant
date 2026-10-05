// Vendored from jcode, MIT License, Copyright (c) 2025 Jeremy Huang.
// This module is the renderer-facing app-core leaf surface for the jcode TUI
// port (plan: docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md): the `crate::*`
// modules that jcode's ported renderers reference, re-rooted to
// `crate::tui::jcode_app::<module>`. Each submodule is a verbatim port at
// jcode @ 0a9dc7805 carrying only the referenced symbols (see each file's
// header for its citation list). Ported on demand: a later wave that needs a
// new symbol extends the same submodule rather than adding a new module.
// The App struct itself and every App-bound fn deliberately never port — the
// cutover adapts operant's own App state onto these types.
#![cfg_attr(test, allow(dead_code))]

pub mod ambient;
pub mod app;
pub mod auth;
pub mod build;
pub mod build_meta;
pub mod bus;
pub mod catalog_scheduler;
pub mod color_support;
pub mod config_shim;
pub mod core;
pub mod env;
pub mod fuzzy;
pub mod fuzzy_engine;
pub mod id;
pub mod info_widget;
pub mod keybind;
pub mod layout_utils;
pub mod live_tests;
pub mod logging;
pub mod markdown;
pub mod memory;
pub mod mermaid;
pub mod mermaid_inline;
// [port-decision] mermaid_inline re-declared at integration (reviving the earlier
// un-declare): the ui_inline_image content surface calls into it, and most of it
// is on disk already. The batch-4 remaining piece is the ratatui_image dep
// (pending a user call) - statements requiring it are gated with markers in place.
pub mod message;
pub mod overnight;
pub mod perf;
pub mod process_memory;
pub mod prompt;
pub mod protocol;
pub mod provider;
pub mod provider_catalog;
pub mod provider_metadata;
pub mod session;
pub mod session_facts;
pub mod storage;
pub mod tui_fns;
// [port-decision] the TuiState presentation trait ports beside the tui-level
// fns: sed'd consumers reference jcode_app::tui_fns::TuiState, so tui_fns
// re-exports it from tui_state (the full 114-method cutover contract).
pub mod side_panel;
pub mod todo;
pub mod tui_state;
pub mod usage;
pub mod util;
pub mod visual_debug;

pub mod helpers {
    // Renderer-facing slice of jcode-tui/src/tui/app/helpers/ (model_names).
    pub mod model_names;
}
