pub mod config {
    pub use operant_core::config::AppConfig as Config;

    #[derive(Debug, Clone, Default)]
    pub struct Settings {
        pub theme: Theme,
        pub permission_mode: PermissionMode,
        pub max_output_tokens: usize,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Theme {
        pub name: String,
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

    #[derive(Debug, Clone, Default)]
    pub struct OutputFormat {
        pub format: String,
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
    }
}

#[derive(Debug, Clone)]
pub enum ImageSource {
    Clipboard,
    File(String),
    Url(String),
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
        Image { data: String, media_type: String },
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

pub struct FreeUpstream {
    pub id: &'static str,
    pub name: &'static str,
    pub api_key_env: &'static str,
}

pub const FREE_CATALOG: &[FreeUpstream] = &[
    FreeUpstream { id: "anthropic", name: "Anthropic", api_key_env: "ANTHROPIC_API_KEY" },
];

pub struct ModelRegistry;

impl ModelRegistry {
    pub fn new() -> Self { Self }
}

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn new() -> Self { Self }
}

pub struct AnthropicClient;

pub struct LoadedPlugin {
    pub name: String,
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
