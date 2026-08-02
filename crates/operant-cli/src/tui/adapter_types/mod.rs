// adapter_types/mod.rs — All adapter/TUI types (decomposed from adapter_types.rs).

pub mod anthropic_client;
pub mod auth;
pub mod codex_oauth;
pub mod config;
pub mod constants;
pub mod cost;
pub mod free_catalog;
pub mod git_utils;
pub mod helpers;
pub mod history;
pub mod import_config;
pub mod items;
pub mod model_registry;
pub mod output_styles;
pub mod provider_id;
pub mod spinner;
pub mod tips;
pub mod tools;
pub mod tui_app;
pub mod types;
pub mod voice;

// Flat re-exports for backward compatibility.
pub use config::Settings;
pub use import_config::{
    ImportPaths, ImportSelection, build_import_preview, execute_import, summarize_import_result,
};
pub use items::{
    AuthStore, FREE_CATALOG, FreeUpstream, LaunchMode, ModelRegistry, ProviderId, StoredCredential,
    TuiApp, context_window_for_model, sample_completion_verb, sample_spinner_verb,
};
