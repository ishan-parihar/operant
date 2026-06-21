// Adapter types that bridge operant TUI expectations with operant backend types.
// This module provides stub types matching operant's exact API surface.

pub mod config {
    /// TUI-level config that matches operant's Config struct exactly.
    #[derive(Debug, Clone)]
    pub struct Config {
        pub model: Option<String>,
        pub provider: Option<String>,
        pub max_tokens: Option<u32>,
        pub permission_mode: PermissionMode,
        pub theme: Theme,
        pub output_style: Option<String>,
        pub output_format: OutputFormat,
        pub project_dir: Option<std::path::PathBuf>,
        pub mcp_servers: Vec<McpServerEntry>,
        pub additional_dirs: Vec<std::path::PathBuf>,
        pub compact_threshold: Option<f64>,
        pub append_system_prompt: Option<String>,
        pub file_autocomplete_limit: usize,
        pub file_autocomplete_show_hidden_files: bool,
        pub file_injection_max_size: usize,
        pub inner: InnerConfig,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                model: None,
                provider: None,
                max_tokens: None,
                permission_mode: PermissionMode::default(),
                theme: Theme::default(),
                output_style: None,
                output_format: OutputFormat::default(),
                project_dir: Some(std::env::current_dir().unwrap_or_default()),
                mcp_servers: vec![],
                additional_dirs: vec![],
                compact_threshold: None,
                append_system_prompt: None,
                file_autocomplete_limit: 25,
                file_autocomplete_show_hidden_files: false,
                file_injection_max_size: 10_000,
                inner: InnerConfig::default(),
            }
        }
    }

    impl Config {
        pub fn effective_model(&self) -> &str {
            self.model.as_deref().unwrap_or("claude-3-5-sonnet")
        }
        pub fn resolve_api_key(&self) -> Option<String> {
            std::env::var("ANTHROPIC_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .ok()
                .filter(|k| !k.is_empty())
        }
        pub fn api_key_for(&self, provider: &str) -> Option<String> {
            let env_var = match provider.to_lowercase().as_str() {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openai" => "OPENAI_API_KEY",
                "groq" => "GROQ_API_KEY",
                "google" | "gemini" => "GOOGLE_API_KEY",
                "mistral" => "MISTRAL_API_KEY",
                "cerebras" => "CEREBRAS_API_KEY",
                _ => return None,
            };
            std::env::var(env_var).ok().filter(|k| !k.is_empty())
        }
        pub fn set_model(&mut self, model: &str) {
            self.model = Some(model.to_string());
        }
        pub fn config_dir() -> std::path::PathBuf {
            dirs::home_dir().unwrap_or_default().join(".operant")
        }
        pub fn from_app_config(app: &operant_core::config::AppConfig) -> Self {
            Self {
                model: Some(app.agent.model.clone()),
                provider: None,
                max_tokens: None,
                permission_mode: PermissionMode::default(),
                theme: Theme::default(),
                output_style: None,
                output_format: OutputFormat::default(),
                project_dir: Some(std::env::current_dir().unwrap_or_default()),
                mcp_servers: vec![],
                additional_dirs: vec![],
                compact_threshold: None,
                append_system_prompt: None,
                file_autocomplete_limit: 25,
                file_autocomplete_show_hidden_files: false,
                file_injection_max_size: 10_000,
                inner: InnerConfig::default(),
            }
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct InnerConfig {
        pub verbose: bool,
        pub cursor_blink_enabled: bool,
        pub auto_commits: Option<bool>,
        pub disable_claude_mds: bool,
        pub file_injection_enabled: bool,
        pub file_autocomplete_limit: usize,
        pub file_autocomplete_show_hidden_files: bool,
        pub file_injection_max_size: usize,
        pub output_style: Option<String>,
        pub output_format: OutputFormat,
        pub compact_threshold: f64,
        pub model: Option<String>,
        pub provider: Option<String>,
        pub theme: Theme,
        pub max_tokens: Option<u32>,
    }

    #[derive(Debug, Clone)]
    pub struct McpServerEntry {
        pub name: String,
        pub command: Option<String>,
        pub args: Vec<String>,
        pub url: Option<String>,
        pub server_type: String,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Settings {
        pub provider: Option<String>,
        pub theme: Theme,
        pub permission_mode: PermissionMode,
        pub max_output_tokens: usize,
        pub model: Option<String>,
        pub output_style: Option<String>,
        pub reduce_motion: bool,
        pub show_cwd: bool,
        pub auto_compact: bool,
        pub auto_copy_on_highlight: bool,
        pub compact_threshold: Option<usize>,
        pub notifications: bool,
        pub show_turn_duration: bool,
        pub terminal_progress_bar: bool,
        pub show_git_branch: bool,
        pub providers: std::collections::HashMap<String, ProviderSettings>,
        pub has_completed_onboarding: bool,
        pub config: InnerConfig,
    }

    #[derive(Debug, Clone, Default)]
    pub struct ProviderSettings {
        pub api_key: Option<String>,
        pub base_url: Option<String>,
        pub api_base: Option<String>,
        pub enabled: bool,
    }

    impl Settings {
        pub fn save_sync(&self) -> anyhow::Result<()> {
            Ok(())
        }
        pub fn load_sync() -> anyhow::Result<Self> {
            Ok(Self::default())
        }
        pub fn effective_config(&self) -> Config {
            let ic = &self.config;
            Config {
                model: ic.model.clone(),
                provider: ic.provider.clone(),
                max_tokens: ic.max_tokens,
                permission_mode: PermissionMode::default(),
                theme: ic.theme.clone(),
                output_style: ic.output_style.clone(),
                output_format: ic.output_format.clone(),
                project_dir: None,
                mcp_servers: vec![],
                additional_dirs: vec![],
                compact_threshold: Some(ic.compact_threshold),
                append_system_prompt: None,
                file_autocomplete_limit: ic.file_autocomplete_limit,
                file_autocomplete_show_hidden_files: ic.file_autocomplete_show_hidden_files,
                file_injection_max_size: ic.file_injection_max_size,
                inner: ic.clone(),
            }
        }
        pub fn config_dir() -> std::path::PathBuf {
            dirs::home_dir().unwrap_or_default().join(".operant")
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Theme {
        Dark,
        Light,
        Default,
        Deuteranopia,
        Custom(String),
    }

    impl Default for Theme {
        fn default() -> Self {
            Self::Default
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum PermissionMode {
        AcceptEdits,
        Default,
        BypassPermissions,
        Plan,
    }

    impl Default for PermissionMode {
        fn default() -> Self {
            Self::Default
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum OutputFormat {
        Text,
        Json,
        StreamJson,
    }

    impl Default for OutputFormat {
        fn default() -> Self {
            Self::Text
        }
    }
}

pub mod constants {
    pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
    pub const DEFAULT_MAX_TOKENS: usize = 8192;
}

pub mod cost {
    #[derive(Debug, Clone, Default)]
    pub struct CostTracker {
        pub total_cost: f64,
        pub input_tokens: u32,
        pub output_tokens: u64,
    }

    impl CostTracker {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn record_usage(&mut self, input: u32, output: u32) {
            self.input_tokens += input;
            self.output_tokens += output as u64;
        }
        pub fn total_tokens(&self) -> u32 {
            self.input_tokens + self.output_tokens as u32
        }
        pub fn set_model(&self, _model: &str) {}
    }
}

pub mod file_history {
    #[derive(Debug, Clone)]
    pub struct FileSnapshot {
        pub path: String,
        pub binary: bool,
        pub before_text: Option<String>,
        pub after_text: Option<String>,
    }

    #[derive(Debug, Clone, Default)]
    pub struct FileHistory {
        entries: Vec<String>,
    }
    impl FileHistory {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn push(&mut self, entry: String) {
            self.entries.push(entry);
        }
        pub fn entries(&self) -> &[String] {
            &self.entries
        }
        pub fn snapshots_for_turn(&self, _turn: usize) -> Vec<FileSnapshot> {
            vec![]
        }
        pub fn latest_turn_index(&self) -> Option<usize> {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageSource {
    pub source_type: String,
    pub url: Option<String>,
    pub data: Option<String>,
    pub media_type: Option<String>,
}

pub mod keybindings {
    use std::collections::HashMap;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum KeyBinding {
        Copy,
        Paste,
        Interrupt,
        Exit,
        Clear,
        Redraw,
        Home,
        End,
        HistoryUp,
        HistoryDown,
        Tab,
        ShiftTab,
        Enter,
        Escape,
        Backspace,
        Delete,
        CtrlC,
        CtrlD,
        CtrlL,
        CtrlR,
        CtrlA,
        CtrlE,
        CtrlW,
        CtrlU,
        CtrlK,
        ShiftEnter,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum KeyContext {
        Global,
        Chat,
        Overlay,
        Settings,
        Confirmation,
        DiffDialog,
        Help,
        HistorySearch,
        Select,
        ThemePicker,
    }

    #[derive(Debug, Clone)]
    pub struct ParsedKeystroke {
        pub key: String,
        pub modifiers: Vec<String>,
        pub ctrl: bool,
        pub alt: bool,
        pub shift: bool,
        pub meta: bool,
    }

    #[derive(Debug, Clone)]
    pub enum KeybindingResult {
        Action(String),
        Pending,
        NoMatch,
        Unbound,
    }

    pub struct KeybindingResolver {
        bindings: HashMap<String, KeybindingResult>,
        has_pending: bool,
    }

    impl KeybindingResolver {
        pub fn new(_user_keybindings: &UserKeybindings) -> Self {
            Self {
                bindings: HashMap::new(),
                has_pending: false,
            }
        }
        pub fn resolve(&self, _keystroke: &ParsedKeystroke) -> Option<KeybindingResult> {
            None
        }
        pub fn process(&self, _key: &ParsedKeystroke, _context: &KeyContext) -> KeybindingResult {
            KeybindingResult::NoMatch
        }
        pub fn has_pending_chord(&self) -> bool {
            self.has_pending
        }
        pub fn cancel_chord(&mut self) {
            self.has_pending = false;
        }
    }

    pub struct UserKeybindings {
        pub bindings: HashMap<String, String>,
    }

    impl UserKeybindings {
        pub fn load(_config_dir: &std::path::Path) -> Self {
            Self {
                bindings: HashMap::new(),
            }
        }
    }
}

pub mod types {
    use super::cost::CostTracker;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Role {
        User,
        Assistant,
    }

    #[derive(Debug, Clone)]
    pub enum ContentBlock {
        Text {
            text: String,
        },
        Thinking {
            thinking: String,
            signature: Option<String>,
        },
        RedactedThinking {
            signature: Option<String>,
        },
        ToolUse {
            id: String,
            name: String,
            input: serde_json::Value,
        },
        ToolResult {
            tool_use_id: String,
            content: ToolResultContent,
            is_error: Option<bool>,
        },
        Image {
            source: super::ImageSource,
        },
        Document {
            title: Option<String>,
            context: Option<String>,
            source: super::ImageSource,
        },
        UserLocalCommandOutput {
            command: String,
            output: String,
        },
        UserCommand {
            name: String,
            args: String,
        },
        UserMemoryInput {
            key: String,
            value: String,
        },
        SystemAPIError {
            message: String,
            retry_secs: Option<u32>,
        },
        CollapsedReadSearch {
            tool_name: String,
            paths: Vec<String>,
            n_hidden: usize,
        },
        TaskAssignment {
            id: String,
            subject: String,
            description: String,
        },
    }

    #[derive(Debug, Clone)]
    pub enum ToolResultContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone)]
    pub enum MessageContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone)]
    pub struct CostInfo {
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub cache_creation_input_tokens: Option<u64>,
        pub cache_read_input_tokens: Option<u64>,
        pub cost_usd: f64,
    }

    #[derive(Debug, Clone)]
    pub struct Message {
        pub role: Role,
        pub content: MessageContent,
        pub uuid: String,
        pub cost: Option<CostInfo>,
    }

    impl Message {
        pub fn user(text: impl Into<String>) -> Self {
            Self {
                role: Role::User,
                content: MessageContent::Text(text.into()),
                uuid: format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    rand(),
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand()
                ),
                cost: None,
            }
        }

        pub fn user_blocks(blocks: Vec<ContentBlock>) -> Self {
            Self {
                role: Role::User,
                content: MessageContent::Blocks(blocks),
                uuid: format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    rand(),
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand()
                ),
                cost: None,
            }
        }

        pub fn assistant(text: impl Into<String>) -> Self {
            Self {
                role: Role::Assistant,
                content: MessageContent::Text(text.into()),
                uuid: format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    rand(),
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand()
                ),
                cost: None,
            }
        }

        pub fn assistant_blocks(blocks: Vec<ContentBlock>) -> Self {
            Self {
                role: Role::Assistant,
                content: MessageContent::Blocks(blocks),
                uuid: format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    rand(),
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand() & 0xFFFF,
                    rand()
                ),
                cost: None,
            }
        }

        pub fn content_blocks(&self) -> Vec<ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks.clone(),
                MessageContent::Text(t) => vec![ContentBlock::Text { text: t.clone() }],
            }
        }

        pub fn text_content(&self) -> String {
            match &self.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            }
        }

        pub fn get_all_text(&self) -> String {
            self.text_content()
        }
        pub fn total_tokens(&self) -> u32 {
            0
        }

        pub fn get_tool_use_blocks(&self) -> Vec<(&str, &str, &serde_json::Value)> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { id, name, input } => {
                            Some((id.as_str(), name.as_str(), input))
                        }
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            }
        }
    }

    fn rand() -> u32 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        t.as_nanos() as u32
    }

    #[derive(Debug, Clone)]
    pub struct ToolResult {
        pub tool_use_id: String,
        pub content: ToolResultContent,
        pub is_error: bool,
    }
}

pub use types::{ContentBlock, Message, MessageContent, Role, ToolResultContent};

pub mod output_styles {
    #[derive(Debug, Clone)]
    pub struct StyleInfo {
        pub name: String,
        pub description: String,
        pub label: String,
        pub accent: ratatui::style::Color,
        pub muted: ratatui::style::Color,
    }

    pub fn builtin_styles() -> Vec<StyleInfo> {
        vec![
            StyleInfo {
                name: "default".to_string(),
                description: "Default operant theme".to_string(),
                label: "Default".to_string(),
                accent: ratatui::style::Color::Rgb(232, 165, 54),
                muted: ratatui::style::Color::Rgb(134, 132, 126),
            },
            StyleInfo {
                name: "dark".to_string(),
                description: "Dark theme".to_string(),
                label: "Dark".to_string(),
                accent: ratatui::style::Color::Rgb(100, 149, 237),
                muted: ratatui::style::Color::Rgb(105, 105, 105),
            },
            StyleInfo {
                name: "light".to_string(),
                description: "Light theme".to_string(),
                label: "Light".to_string(),
                accent: ratatui::style::Color::Rgb(0, 102, 204),
                muted: ratatui::style::Color::Rgb(169, 169, 169),
            },
        ]
    }

    pub fn find_style(_all: &[StyleInfo], name: &str) -> Option<StyleInfo> {
        builtin_styles().into_iter().find(|s| s.name == name)
    }
}

pub fn format_permission_reason(_kind: &str, _detail: &str) -> String {
    format!("{}: {}", _kind, _detail)
}
pub fn sample_completion_verb(_seed: usize) -> &'static str {
    "done"
}
pub fn sample_spinner_verb(_seed: usize) -> &'static str {
    "thinking"
}

pub mod voice {
    #[derive(Debug, Clone)]
    pub enum VoiceEvent {
        RecordingStarted,
        RecordingStopped,
        Transcription(String),
        TranscriptReady(String),
        Error(String),
    }

    pub struct VoiceRecorder;

    impl VoiceRecorder {
        pub fn new() -> Self {
            Self
        }
        pub async fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
        pub async fn stop(&mut self) -> Result<Vec<u8>, String> {
            Ok(vec![])
        }
        pub async fn start_recording(
            &mut self,
            _tx: tokio::sync::mpsc::Sender<VoiceEvent>,
        ) -> Result<(), String> {
            Ok(())
        }
        pub async fn stop_recording(&mut self) -> Result<Vec<u8>, String> {
            Ok(vec![])
        }
        pub fn set_enabled(&mut self, _enabled: bool) {}
    }

    pub fn global_voice_recorder() -> std::sync::Arc<std::sync::Mutex<VoiceRecorder>> {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<std::sync::Arc<std::sync::Mutex<VoiceRecorder>>> =
            OnceLock::new();
        INSTANCE
            .get_or_init(|| std::sync::Arc::new(std::sync::Mutex::new(VoiceRecorder::new())))
            .clone()
    }
}

pub mod query {
    #[derive(Debug)]
    pub enum QueryEvent {
        Stream(StreamEvent),
        ToolStart {
            tool_name: String,
            tool_id: String,
            input_json: String,
        },
        ToolEnd {
            tool_id: String,
            tool_name: String,
            result: String,
            is_error: bool,
        },
        ToolPermissionRequest {
            tool_name: String,
            tool_id: String,
            description: String,
            danger_explanation: String,
            input_preview: Option<String>,
            response_tx: tokio::sync::oneshot::Sender<operant_core::agent::ToolPermissionResponse>,
        },
        TurnComplete {
            turn: usize,
            stop_reason: String,
            usage: Option<UsageInfo>,
        },
        Error(String),
        TokenWarning {
            state: TokenWarningState,
            pct_used: f64,
        },
        Status(String),
    }

    // StreamEvent is an alias for AnthropicStreamEvent so operant match arms work.
    pub type StreamEvent = super::streaming::AnthropicStreamEvent;

    #[derive(Debug, Clone)]
    pub enum ContentDelta {
        TextDelta { text: String },
        ThinkingDelta { thinking: String },
    }

    #[derive(Debug, Clone, Default)]
    pub struct UsageInfo {
        pub input_tokens: u64,
        pub output_tokens: u64,
        pub total_cost: f64,
        pub cache_creation_input_tokens: u64,
        pub cache_read_input_tokens: u64,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum TokenWarningState {
        Normal,
        Warning,
        Critical,
        Ok,
    }

    pub fn context_window_for_model(_model: &str) -> usize {
        128000
    }

    pub mod compact {
        pub use super::TokenWarningState;
    }
}

pub mod types_query {
    pub use super::query::{QueryEvent, StreamEvent, TokenWarningState, UsageInfo};
}

pub mod import_config {
    #[derive(Debug, Clone)]
    pub struct ImportPaths {
        pub settings_json: Option<std::path::PathBuf>,
        pub claude_md: Option<std::path::PathBuf>,
    }

    impl ImportPaths {
        pub fn detect() -> Self {
            Self {
                settings_json: None,
                claude_md: None,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum ImportSelection {
        Both,
        Settings,
        ClaudeMd,
    }

    #[derive(Debug, Clone)]
    pub struct FilePlan {
        pub source_path: std::path::PathBuf,
        pub target_path: std::path::PathBuf,
        pub target_exists: bool,
    }

    #[derive(Debug, Clone)]
    pub struct ClaudeMdPreview {
        pub plan: FilePlan,
        pub line_count: usize,
        pub char_count: usize,
        pub excerpt: String,
    }

    #[derive(Debug, Clone)]
    pub struct SettingsField {
        pub key: String,
        pub name: String,
        pub action: PreviewAction,
        pub reason: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub struct SettingsPreview {
        pub plan: FilePlan,
        pub imported_count: usize,
        pub replaced_count: usize,
        pub kept_count: usize,
        pub skipped_count: usize,
        pub fields: Vec<SettingsField>,
    }

    #[derive(Debug, Clone)]
    pub struct ImportPreview {
        pub settings: Option<SettingsPreview>,
        pub claude_md: Option<ClaudeMdPreview>,
        pub auth: bool,
        pub selection: ImportSelection,
    }

    #[derive(Debug, Clone)]
    pub struct ImportResult {
        pub imported_fields: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum PreviewAction {
        Apply,
        Skip,
        Cancel,
        Import,
        Keep,
        Replace,
    }

    impl PreviewAction {
        pub fn label(&self) -> &'static str {
            match self {
                Self::Apply => "apply",
                Self::Skip => "skip",
                Self::Cancel => "cancel",
                Self::Import => "import",
                Self::Keep => "keep",
                Self::Replace => "replace",
            }
        }
    }

    pub fn build_import_preview(sel: ImportSelection) -> anyhow::Result<ImportPreview> {
        Ok(ImportPreview {
            settings: None,
            claude_md: None,
            auth: false,
            selection: sel,
        })
    }

    pub fn execute_import(_sel: ImportSelection) -> anyhow::Result<ImportResult> {
        Ok(ImportResult {
            imported_fields: vec![],
        })
    }

    pub fn summarize_import_result(_result: &ImportResult, _paths: &ImportPaths) -> String {
        "Import complete".to_string()
    }
}

pub use import_config::{
    build_import_preview, execute_import, summarize_import_result, ClaudeMdPreview, FilePlan,
    ImportPaths, ImportPreview, ImportResult, ImportSelection, PreviewAction, SettingsField,
    SettingsPreview,
};

pub mod codex_oauth {
    pub const CODEX_MODELS: &[(&str, &str)] =
        &[("codex-mini", "Codex Mini"), ("o3-mini", "O3 Mini")];
}

pub mod history {
    pub struct SessionInfo {
        pub id: String,
        pub title: Option<String>,
        pub updated_at: chrono::DateTime<chrono::Utc>,
        pub messages: Vec<()>,
        pub total_cost: f64,
    }

    pub async fn list_sessions() -> Vec<SessionInfo> {
        // Placeholder so the session browser isn't completely empty.
        vec![SessionInfo {
            id: "current".to_string(),
            title: Some("Current session".to_string()),
            updated_at: chrono::Utc::now(),
            messages: vec![],
            total_cost: 0.0,
        }]
    }
}

pub mod tips {
    pub struct Tip {
        pub content: String,
    }
    pub fn select_tip(_seed: u64) -> Option<Tip> {
        None
    }
}

pub mod git_utils {
    pub fn get_current_branch(repo_root: &std::path::Path) -> Option<String> {
        std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_root)
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout)
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                } else {
                    None
                }
            })
    }

    pub fn get_repo_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
        std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(start)
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout)
                        .ok()
                        .map(|s| std::path::PathBuf::from(s.trim()))
                } else {
                    None
                }
            })
    }
}

pub mod spinner {
    pub fn random_face() -> &'static str {
        "●"
    }
}

pub mod compact {
    pub use super::query::TokenWarningState;
}

#[derive(Debug, Clone)]
pub struct AuthStore;

impl AuthStore {
    pub fn load() -> Self {
        Self
    }
    pub fn api_key_for(&self, _provider: impl Into<ProviderId>) -> Option<String> {
        None
    }
    pub fn set(&mut self, _provider: &str, _credential: StoredCredential) {}
}

#[derive(Debug, Clone)]
pub enum StoredCredential {
    OAuthToken {
        access: String,
        refresh: String,
        expires: i64,
    },
    ApiKey {
        key: String,
    },
}

pub mod file_injection {
    #[derive(Debug, Clone)]
    pub struct AtFileRef {
        pub path: String,
        pub line_start: Option<usize>,
        pub line_end: Option<usize>,
    }

    #[derive(Debug, Clone)]
    pub enum AtFileIssue {
        Binary,
        IsDirectory,
        NoMatch(String),
        TooLarge(usize),
        Unreadable(String),
    }

    pub fn parse_at_refs(text: &str) -> (Vec<AtFileRef>, Vec<AtFileIssue>) {
        let mut refs = Vec::new();
        let mut issues = Vec::new();
        for word in text.split_whitespace() {
            if word.starts_with('@') && word.len() > 1 {
                refs.push(AtFileRef {
                    path: word[1..].to_string(),
                    line_start: None,
                    line_end: None,
                });
            }
        }
        (refs, issues)
    }

    pub fn build_file_blocks(_refs: &[AtFileRef]) -> Vec<String> {
        vec![]
    }
}

#[derive(Debug, Clone)]
pub struct FreeUpstream {
    pub id: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub api_key_env: &'static str,
    pub default_model: &'static str,
    pub note: &'static str,
    pub key_url: &'static str,
}

pub const FREE_CATALOG: &[FreeUpstream] = &[
    FreeUpstream {
        id: "groq",
        name: "Groq",
        title: "Groq",
        api_key_env: "GROQ_API_KEY",
        default_model: "llama-3.3-70b-versatile",
        note: "fast — Llama 3.3, GPT-OSS, Qwen3",
        key_url: "console.groq.com/keys",
    },
    FreeUpstream {
        id: "cerebras",
        name: "Cerebras",
        title: "Cerebras",
        api_key_env: "CEREBRAS_API_KEY",
        default_model: "qwen-3-235b-a22b-instruct-2507",
        note: "wafer-scale — Qwen3 235B",
        key_url: "cloud.cerebras.ai",
    },
    FreeUpstream {
        id: "google",
        name: "Google",
        title: "Google Gemini",
        api_key_env: "GOOGLE_API_KEY",
        default_model: "gemini-2.5-flash",
        note: "Gemini 2.5 Flash",
        key_url: "aistudio.google.com/app/apikey",
    },
    FreeUpstream {
        id: "mistral",
        name: "Mistral",
        title: "Mistral",
        api_key_env: "MISTRAL_API_KEY",
        default_model: "mistral-large-latest",
        note: "Large · Medium · Codestral · Devstral",
        key_url: "console.mistral.ai/api-keys",
    },
    FreeUpstream {
        id: "anthropic",
        name: "Anthropic",
        title: "Anthropic",
        api_key_env: "ANTHROPIC_API_KEY",
        default_model: "claude-3-5-sonnet",
        note: "Fast, capable",
        key_url: "",
    },
];

#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub cost_input: Option<f64>,
    pub cost_output: Option<f64>,
    pub release_date: Option<String>,
    pub info: ModelInfo,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_window: u32,
}

pub struct ModelRegistry {
    models: Vec<ModelEntry>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::bundled()
    }
    pub fn bundled() -> Self {
        let mut models = Vec::new();
        for (id, name, ctx) in [
            ("claude-sonnet-4-20250514", "Claude Sonnet 4", 200000u32),
            ("claude-3-5-sonnet-20241022", "Claude 3.5 Sonnet", 200000),
            ("claude-3-5-haiku-20241022", "Claude 3.5 Haiku", 200000),
            ("claude-3-opus-20240229", "Claude 3 Opus", 200000),
        ] {
            models.push(ModelEntry {
                id: id.to_string(),
                provider: "anthropic".to_string(),
                name: name.to_string(),
                cost_input: Some(3.0),
                cost_output: Some(15.0),
                release_date: Some("2025-05-14".to_string()),
                info: ModelInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    context_window: ctx,
                },
            });
        }
        for (id, name, ctx) in [
            ("gpt-4o", "GPT-4o", 128000u32),
            ("gpt-4o-mini", "GPT-4o Mini", 128000),
            ("o3-mini", "o3-mini", 128000),
            ("o4-mini", "o4-mini", 128000),
        ] {
            models.push(ModelEntry {
                id: id.to_string(),
                provider: "openai".to_string(),
                name: name.to_string(),
                cost_input: Some(2.5),
                cost_output: Some(10.0),
                release_date: Some("2024-08-06".to_string()),
                info: ModelInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    context_window: ctx,
                },
            });
        }
        Self { models }
    }
    pub fn list_visible_by_provider(&self, provider: &str) -> Vec<ModelEntry> {
        self.models
            .iter()
            .filter(|m| m.provider == provider)
            .cloned()
            .collect()
    }
    pub fn list_by_provider(&self, provider: &str) -> Vec<ModelEntry> {
        self.list_visible_by_provider(provider)
    }
    pub fn best_model_for_provider(&self, provider: &str) -> Option<String> {
        self.list_visible_by_provider(provider)
            .first()
            .map(|m| m.id.clone())
    }
    pub fn get(&self, provider: &str, model_id: &str) -> Option<ModelEntry> {
        self.models
            .iter()
            .find(|m| m.id == model_id && (m.id.starts_with(provider) || provider.is_empty()))
            .cloned()
    }
    pub fn load_cache(&mut self, _path: &std::path::Path) {}
}

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn new() -> Self {
        Self
    }
}

pub struct AnthropicClient;

impl AnthropicClient {
    pub fn new() -> Self {
        Self
    }
    pub async fn fetch_available_models(&self) -> anyhow::Result<Vec<AvailableModel>> {
        Ok(vec![])
    }
}

pub struct AvailableModel {
    pub id: String,
    pub display_name: Option<String>,
    pub created_at: Option<i64>,
}

pub struct LoadedPlugin {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderId {
    OPENCODE_GO,
    OPENCODE_ZEN,
    Other(String),
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OPENCODE_GO => write!(f, "opencode-go"),
            Self::OPENCODE_ZEN => write!(f, "opencode-zen"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

impl From<&str> for ProviderId {
    fn from(s: &str) -> Self {
        Self::Other(s.to_string())
    }
}

pub mod streaming {
    pub use super::query::ContentDelta;
    #[derive(Debug, Clone)]
    pub enum AnthropicStreamEvent {
        ContentBlockStart {
            index: usize,
        },
        ContentBlockDelta {
            index: usize,
            delta: super::query::ContentDelta,
        },
        ContentBlockStop {
            index: usize,
        },
        MessageStart,
        MessageStop,
        Ping,
    }
}

pub mod mcp {
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    pub struct McpToolDefinition {
        pub name: String,
        pub description: String,
        pub input_schema: serde_json::Value,
    }

    pub struct McpManager;

    impl McpManager {
        pub fn new() -> Arc<Self> {
            Arc::new(Self)
        }
        pub fn all_tool_definitions(&self) -> Vec<(String, McpToolDefinition)> {
            vec![]
        }
        pub fn server_catalog(&self, _name: &str) -> Option<McpCatalogEntry> {
            None
        }
        pub fn server_status(&self, _name: &str) -> McpServerStatus {
            McpServerStatus::Disconnected { last_error: None }
        }
    }

    #[derive(Debug, Clone)]
    pub struct McpCatalogEntry {
        pub tool_count: usize,
        pub resource_count: usize,
        pub prompt_count: usize,
        pub resources: Vec<String>,
        pub prompts: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum McpServerStatus {
        Connected { name: String },
        Connecting,
        Disconnected { last_error: Option<String> },
        Failed { error: String },
    }
}

pub mod tools {
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq)]
    pub enum TaskStatus {
        Pending,
        Running,
        Completed,
        Failed,
        InProgress,
        Deleted,
    }

    impl std::fmt::Display for TaskStatus {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                TaskStatus::Pending => write!(f, "Pending"),
                TaskStatus::Running => write!(f, "Running"),
                TaskStatus::Completed => write!(f, "Completed"),
                TaskStatus::Failed => write!(f, "Failed"),
                TaskStatus::InProgress => write!(f, "In Progress"),
                TaskStatus::Deleted => write!(f, "Deleted"),
            }
        }
    }

    impl TaskStatus {
        pub fn emoji(&self) -> &'static str {
            match self {
                TaskStatus::Pending => "\u{23f3}",
                TaskStatus::Running => "\u{1f504}",
                TaskStatus::Completed => "\u{2705}",
                TaskStatus::Failed => "\u{274c}",
                TaskStatus::InProgress => "\u{1f7e1}",
                TaskStatus::Deleted => "\u{1f5d1}",
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct Task {
        pub id: String,
        pub status: TaskStatus,
        pub description: String,
        pub subject: String,
        pub updated_at: chrono::DateTime<chrono::Utc>,
    }

    pub struct TaskStore {
        tasks: HashMap<String, Task>,
    }

    impl TaskStore {
        pub fn new() -> Self {
            Self {
                tasks: HashMap::new(),
            }
        }
        pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
            self.tasks.get_mut(id)
        }
    }

    pub static TASK_STORE: once_cell::sync::Lazy<std::sync::Mutex<TaskStore>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(TaskStore::new()));

    #[derive(Debug, Clone)]
    pub struct UserQuestionEvent {
        pub question: String,
        pub options: Vec<String>,
    }
}

pub struct TuiApp {
    pub debug_state: Option<super::tui_debug::TuiDebugState>,
    config: operant_core::config::AppConfig,
    turn_counter: usize,
}

impl TuiApp {
    pub async fn enter(
        config: operant_core::config::AppConfig,
        _system: Option<String>,
        _mode: LaunchMode,
    ) -> anyhow::Result<Self> {
        use tracing::debug;

        debug!(target: "tui_wiring", "TuiApp::enter called with model={}, base_url={}",
            config.agent.model, config.client.base_url);

        let adapter_config = config::Config::from_app_config(&config);
        debug!(target: "tui_wiring", "Config converted: model={}",
            adapter_config.effective_model());

        let has_key = adapter_config.resolve_api_key().is_some();
        debug!(target: "tui_wiring", "API key resolved: {}", has_key);

        let mut debug_state = super::tui_debug::TuiDebugState::check_config(&config);
        debug_state.run_all_checks(&config);
        debug_state.tui_app_entered = true;

        debug!(target: "tui_wiring", "Debug state: {}", debug_state.summary());

        Ok(Self {
            debug_state: Some(debug_state),
            config,
            turn_counter: 0,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        use tracing::{debug, warn};

        debug!(target: "tui_wiring", "TuiApp::run starting — creating terminal");

        let stdout = std::io::stdout();
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = ratatui::Terminal::new(backend)?;
        debug!(target: "tui_wiring", "Terminal created");

        let adapter_config = config::Config::from_app_config(&self.config);
        let cost_tracker = std::sync::Arc::new(super::adapter_types::cost::CostTracker::new());
        let mut app = super::app::App::new(adapter_config.clone(), cost_tracker);
        debug!(target: "tui_wiring", "App instance created");

        let has_key = adapter_config.resolve_api_key().is_some();
        if has_key {
            app.has_credentials = true;
        } else {
            warn!(target: "tui_wiring", "No API key configured — agent calls will fail");
            app.has_credentials = false;
        }

        if let Some(ref ds) = self.debug_state {
            app.tui_debug_state = Some(ds.clone());
        }

        let (bridge, bridge_rx) = super::bridge::create_bridge(256, self.turn_counter);
        let super::bridge::BridgeReceivers { query_rx, agent_event_rx, permission_rx } = bridge_rx;
        app.query_rx = Some(query_rx);

        let mcp_manager = std::sync::Arc::new(operant_core::mcp::McpManager::new());
        let skills_dir = self.config.skills.root_dir.clone();
        let agent_config = self.config.agent.clone();
        let full_config = self.config.clone();

        let agent_handle = if has_key {
            match crate::create_runtime_agent(
                &full_config,
                &agent_config,
                None,
                bridge.agent_event_tx.clone(),
                &mcp_manager,
                &skills_dir,
            )
            .await
            {
                Ok(agent) => {
                    let session_id = {
                        use std::time::{SystemTime, UNIX_EPOCH};
                        let nanos = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos();
                        format!("tui_sess_{:016x}", nanos)
                    };
                    debug!(target: "tui_wiring", session_id, "Created persistent TUI session");
                    let agent = agent
                        .with_permissions(bridge.permission_tx)
                        .with_persistent_session(session_id);
                    Some(std::sync::Arc::new(agent))
                }
                Err(e) => {
                    warn!(target: "tui_wiring", error = %e, "Failed to create agent");
                    None
                }
            }
        } else {
            None
        };

        super::bridge::spawn_bridge_tasks(
            super::bridge::BridgeReceivers { query_rx, agent_event_rx, permission_rx },
            bridge.query_tx.clone(),
            bridge.turn_counter.clone(),
        );

        debug!(target: "tui_wiring", "Entering main event loop");

        loop {
            let result = app.run(&mut terminal)?;

            match result {
                Some(input) => {
                    if input.trim().is_empty() {
                        continue;
                    }

                    debug!(target: "tui_wiring", input_len = input.len(), "User submitted input");

                    let user_msg = super::adapter_types::types::Message::user(&input);
                    app.messages.push(user_msg);
                    app.invalidate_transcript();
                    app.on_new_message();
                    app.streaming.is_streaming = true;
                    app.streaming.streaming_text.clear();
                    app.streaming.streaming_thinking.clear();
                    app.streaming.turn_start = Some(std::time::Instant::now());
                    bridge.turn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if let Some(ref agent) = agent_handle {
                        let agent = std::sync::Arc::clone(agent);
                        let tx = bridge.query_tx.clone();
                        let query = input.clone();
                        tokio::spawn(async move {
                            // The bridge task (above) already forwards AgentEvent::Content
                            // as Stream(TextDelta) and AgentEvent::Done as TurnComplete.
                            // We only need to handle the case where agent.run() itself fails.
                            if let Err(e) = agent.run(query).await {
                                warn!(target: "tui_wiring", error = %e, "Agent run failed");
                                let _ = tx
                                    .send(super::adapter_types::query::QueryEvent::Error(
                                        e.to_string(),
                                    ))
                                    .await;
                            }
                        });
                    } else {
                        let _ = bridge.query_tx
                            .send(super::adapter_types::query::QueryEvent::Error(
                                "No API key configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY."
                                    .to_string(),
                            ))
                            .await;
                    }
                }
                None => {
                    debug!(target: "tui_wiring", "User exited TUI");
                    break;
                }
            }
        }

        debug!(target: "tui_wiring", "TUI event loop ended, restoring terminal");
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum LaunchMode {
    Landing,
    Query(String),
}
