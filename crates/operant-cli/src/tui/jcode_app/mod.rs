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
pub mod ambient;
pub mod app;
pub mod auth;
pub mod build;
pub mod bus;
pub mod color_support;
pub mod config_shim;
pub mod core;
pub mod fuzzy;
pub mod fuzzy_engine;
pub mod info_widget;
pub mod keybind;
pub mod layout_utils;
pub mod logging;
pub mod markdown;
pub mod memory;
pub mod mermaid;
pub mod mermaid_inline;
pub mod message;
pub mod overnight;
pub mod perf;
pub mod process_memory;
pub mod prompt;
pub mod protocol;
pub mod provider;
pub mod session;
pub mod session_facts;
pub mod storage;
pub mod tui_fns;
// [port-decision] the TuiState presentation trait ports beside the tui-level
// fns: sed'd consumers reference jcode_app::tui_fns::TuiState, so tui_fns
// re-exports it from tui_state (the full 114-method cutover contract).
pub mod tui_state;
pub mod side_panel;
pub mod todo;
pub mod usage;
pub mod util;

pub mod helpers {
    // Renderer-facing slice of jcode-tui/src/tui/app/helpers/ (model_names).
    pub mod model_names;
}
