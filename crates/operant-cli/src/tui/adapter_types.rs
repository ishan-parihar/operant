pub mod config {
    pub use operant_core::config::AppConfig as Config;

    impl Config {
        pub fn effective_model(&self) -> &str {
            &self.agent.model
        }
        pub fn resolve_api_key(&self, _provider: &str) -> Option<String> {
            self.client.api_key.clone()
        }
        pub fn api_key_for(&self, _provider: &str) -> Option<String> {
            self.client.api_key.clone()
        }
        pub fn set_model(&mut self, model: &str) {
            self.agent.model = model.to_string();
        }
        pub fn config_dir() -> std::path::PathBuf {
            dirs::home_dir().unwrap_or_default().join(".operant")
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct Settings {
        pub theme: Theme,
        pub permission_mode: PermissionMode,
        pub max_output_tokens: usize,
        pub model: Option<String>,
        pub provider: Option<String>,
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
        pub config: InnerConfig,
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
    }

    impl Settings {
        pub fn save_sync(&self) -> Result<(), String> { Ok(()) }
        pub fn load_sync() -> Result<Self, String> { Ok(Self::default()) }
        pub fn config_dir() -> std::path::PathBuf {
            dirs::home_dir().unwrap_or_default().join(".operant")
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct Theme {
        pub name: String,
    }

    impl Theme {
        pub fn dark() -> Self { Self { name: "dark".to_string() } }
        pub fn light() -> Self { Self { name: "light".to_string() } }
        pub fn deuteranopia() -> Self { Self { name: "deuteranopia".to_string() } }
        pub fn custom(name: &str) -> Self { Self { name: name.to_string() } }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum PermissionMode {
        AcceptEdits,
        Default,
        BypassPermissions,
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
        pub output_tokens: u32,
    }

    impl CostTracker {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn record_usage(&mut self, input: u32, output: u32) {
            self.input_tokens += input;
            self.output_tokens += output;
        }
    }
}

pub mod file_history {
    #[derive(Debug, Clone, Default)]
    pub struct FileHistory {
        entries: Vec<String>,
    }

    impl FileHistory {
        pub fn new() -> Self { Self::default() }
        pub fn push(&mut self, entry: String) { self.entries.push(entry); }
        pub fn entries(&self) -> &[String] { &self.entries }
        pub fn snapshots_for_turn(&self, _turn: usize) -> Vec<String> { vec![] }
        pub fn latest_turn_index(&self) -> Option<usize> { None }
    }
}

#[derive(Debug, Clone)]
pub enum ImageSource {
    Clipboard,
    File(String),
    Url(String),
    Paste {
        source_type: String,
        url: Option<String>,
        data: Option<Vec<u8>>,
        media_type: String,
    },
}

pub mod keybindings {
    use std::collections::HashMap;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum KeyBinding {
        Copy, Paste, Interrupt, Exit, Clear, Redraw,
        Home, End, HistoryUp, HistoryDown,
        Tab, ShiftTab, Enter, Escape, Backspace, Delete,
        CtrlC, CtrlD, CtrlL, CtrlR, CtrlA, CtrlE, CtrlW, CtrlU, CtrlK,
        ShiftEnter,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum KeyContext {
        Global,
        Chat,
        Overlay,
        Settings,
    }

    #[derive(Debug, Clone)]
    pub struct ParsedKeystroke {
        pub key: String,
        pub modifiers: Vec<String>,
    }

    #[derive(Debug, Clone)]
    pub struct KeybindingResult {
        pub action: String,
        pub context: KeyContext,
    }

    pub struct KeybindingResolver {
        bindings: HashMap<String, KeybindingResult>,
    }

    impl KeybindingResolver {
        pub fn new() -> Self {
            Self { bindings: HashMap::new() }
        }
        pub fn resolve(&self, _keystroke: &ParsedKeystroke) -> Option<KeybindingResult> {
            None
        }
    }

    pub struct UserKeybindings {
        pub bindings: HashMap<String, String>,
    }

    impl UserKeybindings {
        pub fn load() -> Self {
            Self { bindings: HashMap::new() }
        }
    }
}

pub mod types {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Role { User, Assistant, System }

    #[derive(Debug, Clone)]
    pub enum ContentBlock {
        Text { text: String },
        Thinking { thinking: String, signature: Option<String> },
        ToolUse { id: String, name: String, input: serde_json::Value },
        ToolResult { tool_use_id: String, content: ToolResultContent, is_error: bool },
        Image { source: String, data: String, media_type: String },
        Document { title: String, context: String, source: String },
        UserLocalCommandOutput { command: String, output: String },
        UserCommand { name: String, args: String },
        UserMemoryInput { key: String, value: String },
        SystemAPIError { message: String, retry_secs: Option<u64> },
        CollapsedReadSearch { tool_name: String, paths: Vec<String>, n_hidden: usize },
        TaskAssignment { id: String, subject: String, description: String },
    }

    #[derive(Debug, Clone)]
    pub enum ToolResultContent {
        Text(String),
        Image { data: String, media_type: String },
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone)]
    pub enum MessageContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone)]
    pub struct Message {
        pub role: Role,
        pub content: MessageContent,
    }

    impl Message {
        pub fn content_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks.iter().collect(),
                MessageContent::Text(_) => vec![],
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
        pub fn total_tokens(&self) -> u32 { 0 }
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
    pub struct StyleInfo {
        pub name: String,
        pub accent: ratatui::style::Color,
        pub muted: ratatui::style::Color,
    }

    pub fn builtin_styles() -> Vec<StyleInfo> {
        vec![StyleInfo {
            name: "default".to_string(),
            accent: ratatui::style::Color::Rgb(232, 165, 54),
            muted: ratatui::style::Color::Rgb(134, 132, 126),
        }]
    }

    pub fn find_style(_name: &str) -> Option<StyleInfo> {
        builtin_styles().into_iter().next()
    }
}

pub fn format_permission_reason(_kind: &str, _detail: &str) -> String {
    format!("{}: {}", _kind, _detail)
}

pub fn sample_completion_verb() -> &'static str { "done" }
pub fn sample_spinner_verb() -> &'static str { "thinking" }

pub mod voice {
    #[derive(Debug, Clone)]
    pub enum VoiceEvent {
        RecordingStarted,
        RecordingStopped,
        Transcription(String),
        Error(String),
    }

    pub struct VoiceRecorder;

    impl VoiceRecorder {
        pub fn new() -> Self { Self }
        pub async fn start(&mut self) -> std::result::Result<(), String> { Ok(()) }
        pub async fn stop(&mut self) -> std::result::Result<Vec<u8>, String> { Ok(vec![]) }
    }

    pub fn global_voice_recorder() -> std::sync::Arc<tokio::sync::Mutex<VoiceRecorder>> {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<std::sync::Arc<tokio::sync::Mutex<VoiceRecorder>>> = OnceLock::new();
        INSTANCE.get_or_init(|| std::sync::Arc::new(tokio::sync::Mutex::new(VoiceRecorder::new()))).clone()
    }
}

pub mod tools {
    use std::sync::OnceLock;
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq)]
    pub enum TaskStatus { Pending, Running, Completed, Failed }

    #[derive(Debug, Clone)]
    pub struct Task {
        pub id: String,
        pub status: TaskStatus,
        pub description: String,
    }

    pub struct TaskStore {
        tasks: HashMap<String, Task>,
    }

    impl TaskStore {
        pub fn new() -> Self { Self { tasks: HashMap::new() } }
    }

    pub fn task_store() -> &'static std::sync::Mutex<TaskStore> {
        static INSTANCE: OnceLock<std::sync::Mutex<TaskStore>> = OnceLock::new();
        INSTANCE.get_or_init(|| std::sync::Mutex::new(TaskStore::new()))
    }

    pub static TASK_STORE: once_cell::sync::Lazy<std::sync::Mutex<TaskStore>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(TaskStore::new()));

    #[derive(Debug, Clone)]
    pub struct UserQuestionEvent {
        pub question: String,
        pub options: Vec<String>,
    }
}

pub mod query {
    #[derive(Debug, Clone)]
    pub enum QueryEvent {
        Stream(StreamEvent),
        ToolStart { tool_name: String, tool_id: String, input_json: String },
        ToolEnd { tool_id: String, result: String, is_error: bool },
        TurnComplete { stop_reason: String, usage: Option<UsageInfo> },
        Error(String),
        TokenWarning { state: TokenWarningState, pct_used: f64 },
    }

    #[derive(Debug, Clone)]
    pub enum StreamEvent {
        ContentBlockDelta { delta: String },
        ContentBlockStart,
        ContentBlockStop,
        MessageStart,
        MessageStop,
    }

    #[derive(Debug, Clone, Default)]
    pub struct UsageInfo {
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub total_cost: f64,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum TokenWarningState { Normal, Warning, Critical }

    pub async fn context_window_for_model(_model: &str) -> usize { 128000 }

    pub mod compact {
        pub use super::TokenWarningState;
    }
}

pub mod types_query {
    pub use super::query::{QueryEvent, StreamEvent, UsageInfo, TokenWarningState};
}

pub use query::{QueryEvent, StreamEvent, UsageInfo, TokenWarningState};

pub mod import_config {
    #[derive(Debug, Clone)]
    pub struct ImportPaths {
        pub settings_json: Option<std::path::PathBuf>,
        pub claude_md: Option<std::path::PathBuf>,
    }

    impl ImportPaths {
        pub fn detect() -> Self {
            Self { settings_json: None, claude_md: None }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum ImportSelection { Both, Settings, ClaudeMd }

    #[derive(Debug, Clone)]
    pub struct AuthStore;

    impl AuthStore {
        pub fn load() -> Option<Self> { None }
    }

    #[derive(Debug, Clone)]
    pub struct StoredCredential {
        pub provider: String,
        pub kind: CredentialKind,
    }

    #[derive(Debug, Clone)]
    pub enum CredentialKind { ApiKey(String), OAuthToken(String) }

    pub fn build_import_preview(_paths: &ImportPaths, _sel: ImportSelection) -> String {
        "Import preview".to_string()
    }

    pub fn execute_import(_paths: &ImportPaths, _sel: ImportSelection) -> String {
        "Import executed".to_string()
    }

    pub fn summarize_import_result(_result: &str) -> String {
        _result.to_string()
    }
}

pub mod codex_oauth {
    pub const CODEX_MODELS: &[(&str, &str)] = &[
        ("codex-mini", "Codex Mini"),
        ("o3-mini", "O3 Mini"),
    ];
}

pub mod history {
    pub fn list_sessions() -> Vec<String> { vec![] }
}

pub mod tips {
    pub fn select_tip() -> Option<String> { None }
}

pub mod git_utils {
    pub fn get_current_branch() -> Option<String> { None }
    pub fn get_repo_root() -> Option<std::path::PathBuf> { None }
}

pub mod spinner {
    pub fn random_face() -> &'static str { "●" }
}

pub mod compact {
    pub use super::query::TokenWarningState;
}

pub struct AuthStore;

impl AuthStore {
    pub fn load() -> Option<Self> { None }
}

#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub provider: String,
}

pub fn build_import_preview(_paths: &import_config::ImportPaths, _sel: import_config::ImportSelection) -> String {
    String::new()
}

pub fn execute_import(_paths: &import_config::ImportPaths, _sel: import_config::ImportSelection) -> String {
    String::new()
}

pub fn summarize_import_result(_result: &str) -> String {
    _result.to_string()
}

pub use import_config::{ImportPaths, ImportSelection};

#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub settings: bool,
    pub claude_md: bool,
    pub auth: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreviewAction {
    Apply,
    Skip,
    Cancel,
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
    FreeUpstream { id: "anthropic", name: "Anthropic", title: "Claude", api_key_env: "ANTHROPIC_API_KEY", default_model: "claude-3-5-sonnet", note: "Fast, capable", key_url: "" },
];

pub struct ModelRegistry;

impl ModelRegistry {
    pub fn new() -> Self { Self }
    pub fn list_visible_by_provider(&self, _provider: &str) -> Vec<crate::tui::model_picker::ModelEntry> { vec![] }
    pub fn list_by_provider(&self, _provider: &str) -> Vec<crate::tui::model_picker::ModelEntry> { vec![] }
    pub fn best_model_for_provider(&self, _provider: &str) -> Option<String> { None }
}

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn new() -> Self { Self }
}

pub struct AnthropicClient;

pub struct LoadedPlugin {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderId {
    OPENCODE_GO,
    OPENCODE_ZEN,
    Other(String),
}

pub mod streaming {
    #[derive(Debug, Clone)]
    pub enum AnthropicStreamEvent {
        ContentBlockStart { index: usize },
        ContentBlockDelta { index: usize, delta: String },
        ContentBlockStop { index: usize },
        MessageStart,
        MessageStop,
        Ping,
    }

    #[derive(Debug, Clone)]
    pub struct ContentDelta {
        pub text: Option<String>,
        pub thinking: Option<String>,
    }
}

pub mod mcp {
    use std::sync::Arc;

    pub struct McpManager;

    impl McpManager {
        pub fn new() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum McpServerStatus {
        Connected { name: String },
        Connecting,
        Disconnected { last_error: Option<String> },
        Failed { error: String, .. },
    }
}

pub mod tools {
    use std::sync::OnceLock;
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq)]
    pub enum TaskStatus { Pending, Running, Completed, Failed, InProgress, Deleted }

    impl TaskStatus {
        pub fn emoji(&self) -> &'static str {
            match self {
                TaskStatus::Pending => "⏳",
                TaskStatus::Running => "🔄",
                TaskStatus::Completed => "✅",
                TaskStatus::Failed => "❌",
                TaskStatus::InProgress => "🟡",
                TaskStatus::Deleted => "🗑",
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct Task {
        pub id: String,
        pub status: TaskStatus,
        pub description: String,
        pub subject: String,
    }

    pub struct TaskStore {
        tasks: HashMap<String, Task>,
    }

    impl TaskStore {
        pub fn new() -> Self { Self { tasks: HashMap::new() } }
    }

    pub static TASK_STORE: once_cell::sync::Lazy<std::sync::Mutex<TaskStore>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(TaskStore::new()));

    #[derive(Debug, Clone)]
    pub struct UserQuestionEvent {
        pub question: String,
        pub options: Vec<String>,
    }
}

pub struct TuiApp;

impl TuiApp {
    pub async fn enter(
        _config: operant_core::config::AppConfig,
        _system: Option<String>,
        _mode: LaunchMode,
    ) -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub async fn run(self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum LaunchMode {
    Landing,
    Query(String),
}
