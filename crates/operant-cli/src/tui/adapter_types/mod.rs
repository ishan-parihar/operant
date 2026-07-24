// adapter_types/mod.rs — All adapter/TUI types (decomposed from adapter_types.rs).

pub mod config;
pub mod constants;
pub mod cost;
pub mod types;
pub mod output_styles;
pub mod voice;
pub mod import_config;
pub mod codex_oauth;
pub mod history;
pub mod tips;
pub mod git_utils;
pub mod spinner;
pub mod tools;
pub mod items;

// Flat re-exports for backward compatibility.
pub use config::Settings;
pub use import_config::{ImportPaths, ImportSelection, build_import_preview, execute_import, summarize_import_result};
pub use items::{AuthStore, StoredCredential, FreeUpstream, ModelRegistry, FREE_CATALOG, TuiApp, ProviderId, LaunchMode, context_window_for_model, sample_completion_verb, sample_spinner_verb};
