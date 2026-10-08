// Vendored from jcode (crates/operant-tui/src/tui/app/helpers/model_names.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole
// file (5 lines). Delta: `operant_provider_core::model_names::` re-rooted to
// `crate::tui::operant_app::provider::`. See operant_app/mod.rs for scope.
//! Human-friendly model names live in `operant-provider-core` so the TUI and
//! Jcode Desktop render identical labels.
pub(crate) use crate::tui::operant_app::provider::{
    pretty_known_model_family, pretty_model_display_name,
};
