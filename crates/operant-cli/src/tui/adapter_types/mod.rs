// adapter_types/mod.rs — All adapter/TUI types (decomposed from adapter_types.rs).
//
// Sub-modules:
//   config        — Settings, PermissionMode, OutputFormat, Theme
//   constants     — APP_VERSION
//   cost          — Model costs, usage tracking
//   types         — Message, ContentBlock, Role, ToolResult
//   output_styles — Output format helpers
//   voice         — Voice recorder logic
//   import_config — Data import pipelines
//   codex_oauth   — OAuth helpers
//   history       — Session history
//   tips          — Random tips for welcome screen
//   git_utils     — Git environment helpers
//   spinner       — Spinner definitions (currently empty)
//   tools         — TaskStatus, ToolUseBlock, TuiApp, ProviderId, LaunchMode
//   items         — AuthStore, ModelRegistry, FreeUpstream, AnthropicClient

use crate::commands::CommandResult;

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

// Re-exports for backward compatibility
pub use config::{PermissionMode, OutputFormat, Settings, Theme};
pub use constants::APP_VERSION;
pub use cost::{MODEL_COSTS, ModelCostEntry, track_usage, cost_for_model};
pub use types::{Message, ContentBlock, Role, ToolResult, ToolResultBlock};
pub use output_styles::{OutputStyle, OutputFormat as OutputStyleFormat};
pub use voice::is_available as voice_is_available;
pub use import_config::{detect as import_detect, build_import_preview, execute_import, ImportPreview, ImportSelection};
pub use history::{list_sessions, load_session, delete_session};
pub use tips::select_tip;
pub use git_utils::git_branch;
pub use items::{AuthStore, StoredCredential, FreeUpstream, ModelRegistry, RegistryModelEntry, ModelInfo, AnthropicClient};
pub use tools::{TaskStatus, ToolUseBlock, TuiApp, ProviderId, LaunchMode, context_window_for_model, sample_completion_verb, sample_spinner_verb};

// Top-level convenience functions
pub fn context_window_for_model(model: &str) -> usize {
    tools::context_window_for_model(model)
}

pub fn sample_completion_verb(seed: u64) -> &'static str {
    tools::sample_completion_verb(seed)
}

pub fn sample_spinner_verb(seed: u64) -> &'static str {
    tools::sample_spinner_verb(seed)
}
