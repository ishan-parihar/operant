// Vendored from jcode (crates/operant-base/src/provider_catalog.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial:
// only the two fns the header renderer's auth-kind badge calls
// (ui_header.rs "openrouter" | "openai-compatible" arm):
//! resolve_openai_compatible_profile_selection (:247),
//! openai_compatible_profile_id_for_display_name (:328).
// The provider_metadata dependency is re-rooted from
// `operant_provider_metadata::` to
// `crate::tui::operant_app::provider_metadata::` (the verbatim port of
// crates/operant-provider-metadata carrying the catalog tables).
// [port-excision] the rest of upstream provider_catalog.rs (profile env
// resolution, static model tables, context limits, minimax overrides) is
// unreferenced by the ported tree; re-activate at provider-subsystem cutover.
use crate::tui::operant_app::provider_metadata::{
    LoginProviderTarget, OpenAiCompatibleProfile, resolve_login_provider,
};

pub fn resolve_openai_compatible_profile_selection(input: &str) -> Option<OpenAiCompatibleProfile> {
    let provider = resolve_login_provider(input)?;
    match provider.target {
        LoginProviderTarget::OpenAiCompatible(profile) => Some(profile),
        _ => None,
    }
}

pub fn openai_compatible_profile_id_for_display_name(display_name: &str) -> Option<&'static str> {
    let normalized = display_name.trim().to_ascii_lowercase();
    crate::tui::operant_app::provider_metadata::openai_compatible_profiles()
        .iter()
        .copied()
        .find(|profile| {
            profile.id == normalized
                || profile
                    .display_name
                    .eq_ignore_ascii_case(display_name.trim())
        })
        .map(|profile| profile.id)
}
