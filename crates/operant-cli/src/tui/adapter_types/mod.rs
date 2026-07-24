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
pub use config::{Settings, PermissionMode, OutputFormat, Theme};
pub use constants::APP_VERSION;
pub use cost::CostTracker;
pub use types::{Message, ContentBlock, MessageContent, Role, ToolResultContent};
pub use output_styles::{StyleInfo, builtin_styles, find_style};
pub use voice::{VoiceEvent, VoiceRecorder, global_voice_recorder};
pub use import_config::{ImportPaths, ImportSelection, ImportResult, ImportPreview, build_import_preview, execute_import, summarize_import_result};
pub use history::{SessionRecord, list_sessions, load_session};
pub use tips::select_tip;
pub use items::{AuthStore, StoredCredential, FreeUpstream, ModelRegistry, RegistryModelEntry, ModelInfo, AnthropicClient, FREE_CATALOG, TuiApp, ProviderId, LaunchMode, context_window_for_model, sample_completion_verb, sample_spinner_verb};
pub use tools::TaskStatus;
