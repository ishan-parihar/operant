// Vendored from jcode (crates/jcode-tui/src/tui/app/helpers/model_names.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole
// file (5 lines). Delta: `jcode_provider_core::model_names::` re-rooted to
// `crate::tui::jcode_app::provider::`. See jcode_app/mod.rs for scope.
//! Human-friendly model names live in `jcode-provider-core` so the TUI and
//! Jcode Desktop render identical labels.
pub(crate) use crate::tui::jcode_app::provider::{
    pretty_known_model_family, pretty_model_display_name,
};
