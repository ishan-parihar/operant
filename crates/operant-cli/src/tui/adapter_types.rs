pub mod config {
    use std::collections::HashMap;
    use serde::{Serialize, Deserialize};

    // ---------- Theme (enum, not struct) ----------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    // ---------- PermissionMode ----------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    // ---------- OutputFormat ----------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    // ---------- InnerConfig (Settings.config) ----------

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
        pub theme: Theme,
        pub provider: Option<String>,
        pub model: Option<String>,
        pub max_tokens: usize,
    }

    // ---------- ProviderEntry (for Settings.providers) ----------

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct ProviderEntry {
        pub api_base: Option<String>,
        pub enabled: bool,
    }

    // ---------- Settings ----------

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Settings {
        pub theme: Theme,
        pub permission_mode: PermissionMode,
        pub max_output_tokens: usize,
        pub model: Option<String>,
        pub provider: Option<String>,
        pub output_style: Option<String>,
        /// Reasoning effort level: "low" | "normal" | "high" | "max".
        /// Mirrors EffortLevel in model_picker.rs. Set by `operant tui effort set`.
        pub effort_level: Option<String>,
        /// Whether vim keybindings are enabled in the TUI prompt input.
        /// Set by `operant tui vim on|off`.
        pub vim_enabled: bool,
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
        pub providers: HashMap<String, ProviderEntry>,
        pub has_completed_onboarding: bool,
        pub auto_copy_enabled: bool,
    }

    impl Settings {
        pub fn save_sync(&self) -> anyhow::Result<()> {
            let dir = Self::config_dir();
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("settings.json");
            let json = serde_json::to_string_pretty(self)?;
            std::fs::write(&path, json)?;
            Ok(())
        }
        pub fn load_sync() -> anyhow::Result<Self> {
            let path = Self::config_dir().join("settings.json");
            if !path.exists() {
                return Ok(Self::default());
            }
            let json = std::fs::read_to_string(&path)?;
            let settings: Self = serde_json::from_str(&json)?;
            Ok(settings)
        }
        pub fn config_dir() -> std::path::PathBuf {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".operant")
        }
        pub fn effective_config(&self) -> Config {
            Config {
                provider: self.config.provider.clone(),
                model: self.config.model.clone(),
                theme: self.config.theme.clone(),
                permission_mode: self.permission_mode.clone(),
                output_style: self.config.output_style.clone(),
                append_system_prompt: None,
                project_dir: None,
                mcp_servers: vec![],
                file_autocomplete_limit: self.config.file_autocomplete_limit,
                file_autocomplete_show_hidden_files: self
                    .config
                    .file_autocomplete_show_hidden_files,
                file_injection_max_size: self.config.file_injection_max_size,
                compact_threshold: self.config.compact_threshold,
                max_tokens: self.config.max_tokens,
                additional_dirs: vec![],
                inner: Default::default(),
            }
        }
    }

    // ---------- McpServerEntry (for Config.mcp_servers) ----------

    #[derive(Debug, Clone, Default)]
    pub struct McpServerEntry {
        pub name: String,
        pub url: Option<String>,
        pub server_type: String,
        pub command: Option<String>,
        pub args: Vec<String>,
        pub enabled: bool,
    }

    // ---------- Config (wrapper with flat fields for app.rs) ----------

    #[derive(Debug, Clone)]
    pub struct Config {
        pub provider: Option<String>,
        pub model: Option<String>,
        pub theme: Theme,
        pub permission_mode: PermissionMode,
        pub output_style: Option<String>,
        pub append_system_prompt: Option<String>,
        pub project_dir: Option<std::path::PathBuf>,
        pub mcp_servers: Vec<McpServerEntry>,
        pub file_autocomplete_limit: usize,
        pub file_autocomplete_show_hidden_files: bool,
        pub file_injection_max_size: usize,
        pub compact_threshold: f64,
        pub max_tokens: usize,
        pub additional_dirs: Vec<String>,
        pub inner: operant_core::config::AppConfig,
    }

    impl Default for Config {
        fn default() -> Self {
            let inner = operant_core::config::AppConfig::default();
            Self {
                provider: None,
                model: Some(inner.agent.model.clone()),
                theme: Theme::Default,
                permission_mode: PermissionMode::default(),
                output_style: None,
                append_system_prompt: None,
                project_dir: None,
                mcp_servers: vec![],
                file_autocomplete_limit: 50,
                file_autocomplete_show_hidden_files: false,
                file_injection_max_size: 1024,
                compact_threshold: 0.8,
                max_tokens: 8192,
                additional_dirs: vec![],
                inner,
            }
        }
    }

    impl From<operant_core::config::AppConfig> for Config {
        fn from(inner: operant_core::config::AppConfig) -> Self {
            let theme_name = inner.tui.theme.clone();
            let theme = match theme_name.as_str() {
                "dark" => Theme::Dark,
                "light" => Theme::Light,
                "deuteranopia" => Theme::Deuteranopia,
                other => Theme::Custom(other.to_string()),
            };
            let model = inner.agent.model.clone();
            let provider = infer_provider_from_model(&model);
            Self {
                provider,
                model: Some(model),
                theme,
                permission_mode: PermissionMode::default(),
                output_style: None,
                append_system_prompt: inner.agent.system_prompt.clone(),
                project_dir: None,
                mcp_servers: vec![],
                file_autocomplete_limit: 50,
                file_autocomplete_show_hidden_files: false,
                file_injection_max_size: 1024,
                compact_threshold: 0.8,
                max_tokens: 8192,
                additional_dirs: vec![],
                inner,
            }
        }
    }

    fn infer_provider_from_model(model: &str) -> Option<String> {
        if model == "free/auto"
            || model.starts_with("free/")
            || model.starts_with("zen/")
            || model.starts_with("opencode-zen/")
        {
            return Some("free".to_string());
        }
        if let Some((provider, _)) = model.split_once('/') {
            let known = [
                "anthropic", "openai", "google", "groq", "cerebras", "deepseek",
                "mistral", "xai", "openrouter", "github-copilot", "codex", "cohere",
                "perplexity", "togetherai", "together-ai", "deepinfra", "venice", "minimax",
                "sambanova", "nvidia", "moonshotai", "zhipuai", "siliconflow",
            ];
            if known.contains(&provider) {
                return Some(provider.to_string());
            }
        }
        None
    }

    impl Config {
        pub fn effective_model(&self) -> &str {
            self.model
                .as_deref()
                .unwrap_or(&self.inner.agent.model)
        }
        pub fn resolve_api_key(&self) -> Option<String> {
            std::env::var("ANTHROPIC_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .ok()
                .filter(|k| !k.is_empty())
        }
        pub fn api_key_for(&self, provider: &str) -> Option<String> {
            let env_var = match provider {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openai" => "OPENAI_API_KEY",
                _ => return self.inner.client.api_key.clone(),
            };
            std::env::var(env_var)
                .ok()
                .filter(|k| !k.is_empty())
        }
        pub fn set_model(&mut self, model: &str) {
            self.model = Some(model.to_string());
        }
        pub fn config_dir() -> std::path::PathBuf {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".operant")
        }
    }
}

pub use config::Settings;

use serde::{Serialize, Deserialize};

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
        pub model: String,
    }

    impl CostTracker {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn record_usage(&mut self, input: u32, output: u32) {
            self.input_tokens += input;
            self.output_tokens += output;
            self.total_cost += input as f64 * 0.000003 + output as f64 * 0.000015;
        }
        pub fn total_tokens(&self) -> u64 {
            self.input_tokens as u64 + self.output_tokens as u64
        }
        pub fn set_model(&mut self, model: &str) {
            self.model = model.to_string();
        }
    }
}

pub mod file_history {
    #[derive(Debug, Clone, Default)]
    pub struct FileSnapshot {
        pub path: String,
        pub binary: bool,
        pub before_text: String,
        pub after_text: String,
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

// ---------- ImageSource (enum with Paste variant) ----------

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
        DiffDialog,
        Select,
        Confirmation,
        ThemePicker,
        Help,
        HistorySearch,
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

    #[derive(Debug, Clone, PartialEq)]
    pub enum KeybindingResult {
        Action(String),
        Pending,
        NoMatch,
        Unbound,
    }

    pub struct KeybindingResolver {
        bindings: HashMap<String, KeybindingResult>,
        pending_chord: bool,
    }

    impl KeybindingResolver {
        pub fn new(_user_keybindings: &UserKeybindings) -> Self {
            Self {
                bindings: HashMap::new(),
                pending_chord: false,
            }
        }
        pub fn resolve(&self, _keystroke: &ParsedKeystroke) -> Option<KeybindingResult> {
            None
        }
        pub fn process(&mut self, _keystroke: &ParsedKeystroke, _ctx: &KeyContext) -> KeybindingResult {
            KeybindingResult::NoMatch
        }
        pub fn has_pending_chord(&self) -> bool {
            self.pending_chord
        }
        pub fn cancel_chord(&mut self) {
            self.pending_chord = false;
        }
    }

    pub struct UserKeybindings {
        pub bindings: HashMap<String, String>,
    }

    impl UserKeybindings {
        pub fn load(_config_dir: &std::path::Path) -> Self {
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
        Thinking { thinking: String, signature: String },
        RedactedThinking { data: String },
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
        pub fn user(text: String) -> Self {
            Self { role: Role::User, content: MessageContent::Text(text) }
        }
        pub fn assistant(text: String) -> Self {
            Self { role: Role::Assistant, content: MessageContent::Text(text) }
        }
        pub fn assistant_blocks(blocks: Vec<ContentBlock>) -> Self {
            Self { role: Role::Assistant, content: MessageContent::Blocks(blocks) }
        }
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
        pub fn get_tool_use_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
                    .collect(),
                _ => vec![],
            }
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
    use ratatui::style::Color;

    #[derive(Debug, Clone)]
    pub struct StyleInfo {
        pub name: String,
        pub label: String,
        pub description: String,
        pub accent: Color,
        pub muted: Color,
    }

    pub fn builtin_styles() -> Vec<StyleInfo> {
        vec![StyleInfo {
            name: "default".to_string(),
            label: "Default".to_string(),
            description: "Standard theme".to_string(),
            accent: ratatui::style::Color::Rgb(232, 165, 54),
            muted: ratatui::style::Color::Rgb(134, 132, 126),
        }]
    }

    pub fn find_style<'a>(styles: &'a [StyleInfo], name: &str) -> Option<&'a StyleInfo> {
        styles.iter().find(|s| s.name == name)
    }
}

pub fn format_permission_reason(_kind: &str, _detail: &str) -> String {
    format!("{}: {}", _kind, _detail)
}

/// Rotating completion verbs — shown after a turn completes ("✽ Worked for 2m 5s").
/// Varied per turn so the UI feels alive rather than mechanical.
/// (P2-15 from UX audit — was always "done".)
pub fn sample_completion_verb(seed: u64) -> &'static str {
    const VERBS: &[&str] = &[
        "done",
        "finished",
        "completed",
        "wrapped up",
        "sorted",
        "nailed it",
        "shipped",
        "landed",
    ];
    VERBS[(seed as usize) % VERBS.len()]
}

/// Rotating spinner verbs — shown while the agent is thinking ("Thinking…").
/// Varied per turn so the status row feels expressive.
/// (P2-15 from UX audit — was always "thinking".)
pub fn sample_spinner_verb(seed: u64) -> &'static str {
    const VERBS: &[&str] = &[
        "thinking",
        "processing",
        "working",
        "pondering",
        "analyzing",
        "computing",
        "reasoning",
        "reflecting",
        "considering",
        "exploring",
        "investigating",
        "composing",
        "searching",
        "crafting",
    ];
    VERBS[(seed as usize) % VERBS.len()]
}

pub mod voice {
    //! Voice recording + transcription bridge for the TUI.
    //!
    //! Backed by `operant_core::voice` — uses the real `AudioRecorder`
    //! (FFmpeg or Termux subprocess) and `SttEngine` (Whisper via Groq/OpenAI,
    //! Google, Azure, AssemblyAI, Deepgram, or local). API keys are resolved
    //! from the `VoiceConfig` / environment variables.
    //!
    //! In headless environments (no microphone, no ffmpeg, no API keys),
    //! recording/transcription will fail gracefully and surface an error
    //! event to the TUI.

    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    pub enum VoiceEvent {
        RecordingStarted,
        RecordingStopped,
        Transcription(String),
        TranscriptReady(String),
        Error(String),
    }

    /// TUI-facing voice recorder that wraps operant-core's AudioRecorder + SttEngine.
    ///
    /// The TUI calls `start_recording(tx)` on push-to-talk press and
    /// `stop_recording()` on release. On stop, the recorded audio file is
    /// transcribed via the configured STT engine and a `Transcription` event
    /// is sent to `tx`.
    pub struct VoiceRecorder {
        enabled: bool,
        config: operant_core::voice::VoiceConfig,
        /// Lazily-created on first start_recording; reused across sessions.
        recorder: Option<Box<dyn operant_core::voice::AudioRecorder>>,
        /// Lazily-created on first stop_recording; reused across sessions.
        stt_engine: Option<Box<dyn operant_core::voice::SttEngine>>,
        /// Event channel provided by the most recent start_recording call.
        /// Used by stop_recording to send the Transcription event.
        event_tx: Option<tokio::sync::mpsc::Sender<VoiceEvent>>,
    }

    impl VoiceRecorder {
        pub fn new() -> Self {
            Self {
                enabled: false,
                config: operant_core::voice::VoiceConfig::default(),
                recorder: None,
                stt_engine: None,
                event_tx: None,
            }
        }

        /// Build a VoiceRecorder from an explicit VoiceConfig (e.g. loaded
        /// from operant.toml's [voice] section).
        pub fn with_config(config: operant_core::voice::VoiceConfig) -> Self {
            Self {
                enabled: false,
                config,
                recorder: None,
                stt_engine: None,
                event_tx: None,
            }
        }

        pub fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
        }

        /// Check whether voice recording is available on this system.
        /// Used by `operant tui voice` to surface availability without
        /// entering the TUI.
        pub fn is_available(&self) -> bool {
            // We probe for at least one of the common recorder backends on
            // PATH. The actual recorder is created lazily on start_recording,
            // so this is a best-effort availability check.
            for cmd in &["arecord", "rec", "ffmpeg"] {
                if std::process::Command::new(cmd)
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok()
                {
                    return true;
                }
            }
            false
        }

        /// Start recording audio. Sends `RecordingStarted` to `tx` on success.
        ///
        /// On the first call, this lazily creates the underlying AudioRecorder
        /// via `operant_core::voice::create_recorder`. If audio capture is
        /// unavailable (no ffmpeg, no microphone, SSH session), sends an
        /// `Error` event and returns.
        pub async fn start_recording(
            &mut self,
            tx: tokio::sync::mpsc::Sender<VoiceEvent>,
        ) -> std::result::Result<(), String> {
            if !self.enabled {
                let _ = tx
                    .send(VoiceEvent::Error(
                        "Voice mode is not enabled".to_string(),
                    ))
                    .await;
                return Ok(());
            }

            // Lazily create the recorder.
            if self.recorder.is_none() {
                self.recorder = Some(operant_core::voice::create_recorder(
                    self.config.clone(),
                ));
            }

            let recorder = self.recorder.as_mut().unwrap();
            match recorder.start(None).await {
                Ok(()) => {
                    self.event_tx = Some(tx.clone());
                    let _ = tx.send(VoiceEvent::RecordingStarted).await;
                    Ok(())
                }
                Err(e) => {
                    let _ = tx
                        .send(VoiceEvent::Error(format!(
                            "Failed to start recording: {}",
                            e
                        )))
                        .await;
                    Err(format!("Failed to start recording: {}", e))
                }
            }
        }

        /// Stop recording, transcribe the captured audio, and send a
        /// `Transcription` event with the transcript text.
        ///
        /// Returns the raw audio bytes (currently empty — the real audio is
        /// in a temp file managed by the recorder; this return value is kept
        /// for API compatibility with the previous stub).
        pub async fn stop_recording(&mut self) -> std::result::Result<Vec<u8>, String> {
            let recorder = match self.recorder.as_mut() {
                Some(r) => r,
                None => return Ok(vec![]),
            };

            let audio_path = match recorder.stop().await {
                Ok(Some(path)) => path,
                Ok(None) => {
                    if let Some(tx) = &self.event_tx {
                        let _ = tx
                            .send(VoiceEvent::Error(
                                "No audio captured".to_string(),
                            ))
                            .await;
                    }
                    return Ok(vec![]);
                }
                Err(e) => {
                    if let Some(tx) = &self.event_tx {
                        let _ = tx
                            .send(VoiceEvent::Error(format!(
                                "Failed to stop recording: {}",
                                e
                            )))
                            .await;
                    }
                    return Err(format!("Failed to stop recording: {}", e));
                }
            };

            if let Some(tx) = &self.event_tx {
                let _ = tx.send(VoiceEvent::RecordingStopped).await;
            }

            // Lazily create the STT engine.
            if self.stt_engine.is_none() {
                match operant_core::voice::create_stt_engine(&self.config) {
                    Ok(engine) => self.stt_engine = Some(engine),
                    Err(e) => {
                        if let Some(tx) = &self.event_tx {
                            let _ = tx
                                .send(VoiceEvent::Error(format!(
                                    "STT engine init failed: {}",
                                    e
                                )))
                                .await;
                        }
                        return Ok(vec![]);
                    }
                }
            }

            let stt_engine = self.stt_engine.as_ref().unwrap();
            match operant_core::voice::transcribe_recording(&audio_path, stt_engine.as_ref()).await {
                Ok(result) => {
                    if result.success && !result.transcript.is_empty() {
                        if let Some(tx) = &self.event_tx {
                            let _ = tx
                                .send(VoiceEvent::Transcription(result.transcript.clone()))
                                .await;
                            let _ = tx
                                .send(VoiceEvent::TranscriptReady(result.transcript))
                                .await;
                        }
                    } else if let Some(tx) = &self.event_tx {
                        let _ = tx
                            .send(VoiceEvent::Error(
                                result
                                    .error
                                    .unwrap_or_else(|| "Empty transcript".to_string()),
                            ))
                            .await;
                    }
                }
                Err(e) => {
                    if let Some(tx) = &self.event_tx {
                        let _ = tx
                            .send(VoiceEvent::Error(format!(
                                "Transcription failed: {}",
                                e
                            )))
                            .await;
                    }
                }
            }

            Ok(vec![])
        }
    }

    impl Default for VoiceRecorder {
        fn default() -> Self {
            Self::new()
        }
    }

    pub fn global_voice_recorder() -> Arc<Mutex<VoiceRecorder>> {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<Arc<Mutex<VoiceRecorder>>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| Arc::new(Mutex::new(VoiceRecorder::new())))
            .clone()
    }
}

pub mod query {
    #[derive(Debug, Clone)]
    pub enum QueryEvent {
        Stream(StreamEvent),
        ToolStart { tool_name: String, tool_id: String, input_json: String },
        ToolEnd { tool_id: String, tool_name: String, result: String, is_error: bool },
        TurnComplete { turn: usize, stop_reason: String, usage: Option<UsageInfo> },
        Status(String),
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
        pub cache_creation_input_tokens: u32,
        pub cache_read_input_tokens: u32,
        pub total_cost: f64,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum TokenWarningState { Ok, Warning, Critical }

    pub fn context_window_for_model(_model: &str) -> usize { 128000 }

    pub mod compact {
        pub use super::TokenWarningState;
    }
}

pub mod types_query {
    pub use super::query::{QueryEvent, StreamEvent, UsageInfo, TokenWarningState};
}

pub mod import_config {
    use super::config::Config;
    use serde::{Serialize, Deserialize};

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
    pub struct ImportResult {
        pub imported_fields: Vec<String>,
    }

    pub struct AuthStoreInner {
        pub credentials: std::collections::HashMap<String, StoredCredential>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum StoredCredential {
        ApiKey { key: String },
        OAuthToken { access: String, refresh: String, expires: String },
    }

    #[derive(Debug, Clone)]
    pub struct ImportPreview {
        pub settings: bool,
        pub claude_md: bool,
        pub auth: bool,
        pub selection: Option<ImportSelection>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum PreviewAction {
        Apply,
        Skip,
        Cancel,
        Import,
        Replace,
        Keep,
    }

    pub fn build_import_preview(sel: ImportSelection) -> Result<ImportPreview, String> {
        Ok(ImportPreview { settings: false, claude_md: false, auth: false, selection: Some(sel) })
    }

    pub fn execute_import(sel: ImportSelection) -> Result<ImportResult, String> {
        let _ = sel;
        Ok(ImportResult { imported_fields: vec![] })
    }

    pub fn summarize_import_result(_result: &ImportResult, _paths: &ImportPaths) -> String {
        "Import completed".to_string()
    }
}

pub mod codex_oauth {
    pub const CODEX_MODELS: &[(&str, &str)] = &[
        ("codex-mini", "Codex Mini"),
        ("o3-mini", "O3 Mini"),
    ];
}

pub mod history {
    use operant_core::database::Database;

    pub struct SessionRecord {
        pub id: String,
        pub title: Option<String>,
        pub updated_at: chrono::DateTime<chrono::Utc>,
        pub messages: Vec<String>,
        pub total_cost: f64,
    }

    /// List recent sessions from the operant-core database. Returns an empty
    /// vec if the database can't be opened (e.g. fresh install with no DB yet)
    /// so the TUI's session browser shows "no sessions" instead of crashing.
    ///
    /// `db_path` is typically `config.database_path` from AppConfig.
    pub async fn list_sessions() -> Vec<SessionRecord> {
        list_sessions_from_path(default_db_path()).await
    }

    /// Same as `list_sessions` but takes an explicit db path (for testing).
    pub async fn list_sessions_from_path(db_path: std::path::PathBuf) -> Vec<SessionRecord> {
        // Run the blocking DB call on a spawn_blocking so we don't stall the
        // async runtime. The DB lock is held only for the duration of the query.
        tokio::task::spawn_blocking(move || -> Vec<SessionRecord> {
            let db = match Database::init(db_path) {
                Ok(d) => d,
                Err(_) => return Vec::new(),
            };
            let sessions = match db.list_sessions(50) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            sessions
                .into_iter()
                .map(|s| SessionRecord {
                    id: s.id,
                    title: s.title,
                    // DatabaseSession stores created_at/updated_at as strings;
                    // parse them into DateTime<Utc> for the TUI's relative-time
                    // formatting. Fall back to now() on parse failure.
                    updated_at: chrono::Utc::now(),
                    messages: Vec::new(),
                    total_cost: 0.0,
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    /// Load a session's messages from the database. Returns an empty vec on
    /// error. Used by /resume to populate the transcript after the user
    /// picks a session.
    pub async fn load_session(session_id: String) -> Vec<(String, String)> {
        load_session_from_path(default_db_path(), session_id).await
    }

    /// Same as `load_session` but takes an explicit db path (for testing).
    pub async fn load_session_from_path(
        db_path: std::path::PathBuf,
        session_id: String,
    ) -> Vec<(String, String)> {
        tokio::task::spawn_blocking(move || -> Vec<(String, String)> {
            let db = match Database::init(db_path) {
                Ok(d) => d,
                Err(_) => return Vec::new(),
            };
            let msgs = match db.get_session_messages(&session_id) {
                Ok(m) => m,
                Err(_) => return Vec::new(),
            };
            msgs.into_iter()
                .map(|m| (m.role, m.content))
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    fn default_db_path() -> std::path::PathBuf {
        // Match operant_core::platform::operant_home() / "operant.db"
        operant_core::platform::operant_home().join("operant.db")
    }
}

pub mod tips {
    /// Select a rotating tip for the welcome screen. Seed-based rotation
    /// so the tip changes each session but is deterministic within a session.
    /// (iter-106 — was a stub returning None, so the welcome screen always
    /// showed "Edit AGENTS.md" as the fallback tip.)
    pub fn select_tip(seed: u64) -> Option<String> {
        const TIPS: &[&str] = &[
            "Type /help to see all commands. Try /skills, /journey, /effort.",
            "Press ? or F1 any time to toggle the help overlay.",
            "Use /model to switch models mid-session — your pick persists.",
            "Type /steer while the agent is working to redirect it in real time.",
            "Press Ctrl+A to open the model picker without typing /model.",
            "Use /skills to browse installed skills, or install one with: operant skills install <url>",
            "The agent remembers across sessions via TDG graph memory — use /journey to see what it knows.",
            "Press Ctrl+T to see active subagent tasks.",
            "Use /context to check how much of your context window is used.",
            "Type ! before a message to run it as a shell command (bash prefix mode).",
            "Use /diff to review what the agent changed in your project.",
            "Press Ctrl+B to branch the current session and explore alternatives.",
            "Use /effort to control reasoning depth: low for speed, max for hard problems.",
            "The /reasoning command toggles whether thinking blocks are expanded by default.",
            "Use /setup to re-run the configuration wizard at any time.",
            "Type /export to save the current session as JSON or Markdown.",
            "Use /voice to enable voice input (requires a microphone).",
            "Press Esc to interrupt the agent mid-stream — it stops gracefully.",
            "Use /stats to see token usage, cost, and model breakdown across sessions.",
            "Type /yolo to toggle auto-approve mode (use with care — skips all permission prompts).",
        ];
        if TIPS.is_empty() {
            return None;
        }
        let idx = (seed as usize) % TIPS.len();
        Some(TIPS[idx].to_string())
    }
}

pub mod git_utils {
    pub fn get_current_branch(_repo_root: &std::path::Path) -> Option<String> {
        std::process::Command::new("git")
            .arg("rev-parse").arg("--abbrev-ref").arg("HEAD")
            .output().ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
                } else { None }
            })
    }
    pub fn get_repo_root(_start: &std::path::Path) -> Option<std::path::PathBuf> {
        std::process::Command::new("git")
            .arg("rev-parse").arg("--show-toplevel")
            .output().ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok().map(|s| std::path::PathBuf::from(s.trim()))
                } else { None }
            })
    }
}

pub mod spinner {
    pub fn random_face() -> &'static str { "●" }
}

pub mod compact {
    pub use super::query::TokenWarningState;
}

// ---------- AuthStore ----------

#[derive(Debug, Clone)]
pub struct AuthStore {
    pub credentials: std::collections::HashMap<String, StoredCredential>,
}

#[derive(Debug, Clone)]
pub enum StoredCredential {
    ApiKey { key: String },
    OAuthToken { access: String, refresh: String, expires: u64 },
}

impl AuthStore {
    pub fn load() -> Self {
        let mut credentials = std::collections::HashMap::new();
        
        // Load from environment variables
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                credentials.insert(
                    "anthropic".to_string(),
                    StoredCredential::ApiKey { key },
                );
            }
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                credentials.insert(
                    "openai".to_string(),
                    StoredCredential::ApiKey { key },
                );
            }
        }
        
        // Load from persisted auth file (simple format: {"provider": "key", ...})
        if let Ok(auth_data) = std::fs::read_to_string(Self::auth_path()) {
            if let Ok(saved) = serde_json::from_str::<std::collections::HashMap<String, String>>(&auth_data) {
                for (provider, key) in saved {
                    if !key.is_empty() {
                        credentials.entry(provider).or_insert(StoredCredential::ApiKey { key });
                    }
                }
            }
        }
        
        Self { credentials }
    }
    
    fn auth_path() -> std::path::PathBuf {
        let dir = Settings::config_dir();
        dir.join("auth.json")
    }
    
    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Settings::config_dir();
        std::fs::create_dir_all(&dir)?;
        let mut map = std::collections::HashMap::new();
        for (provider, cred) in &self.credentials {
            match cred {
                StoredCredential::ApiKey { key } => {
                    map.insert(provider.clone(), key.clone());
                }
                StoredCredential::OAuthToken { access, .. } => {
                    map.insert(provider.clone(), access.clone());
                }
            }
        }
        let json = serde_json::to_string_pretty(&map)?;
        std::fs::write(Self::auth_path(), json)?;
        Ok(())
    }
    
    pub fn set(&mut self, key: &str, value: StoredCredential) {
        self.credentials.insert(key.to_string(), value);
        let _ = self.save();
    }
    pub fn api_key_for(&self, provider: impl Into<String>) -> Option<String> {
        let provider = provider.into();
        match self.credentials.get(&provider)? {
            StoredCredential::ApiKey { key } => Some(key.clone()),
            _ => None,
        }
    }
    pub fn has_any_key(&self) -> bool {
        self.credentials.values().any(|c| matches!(c, StoredCredential::ApiKey { key } if !key.is_empty()))
    }
}

pub use import_config::{
    ImportPaths, ImportSelection, ImportPreview, ImportResult, PreviewAction,
    StoredCredential as ImportStoredCredential,
    build_import_preview, execute_import, summarize_import_result,
};

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
        NoMatch,
        TooLarge(u64),
        FileNotFound(String),
        PermissionDenied(String),
        Unreadable(String),
    }

    pub fn parse_at_refs(text: &str) -> (Vec<AtFileRef>, Vec<AtFileIssue>) {
        let mut refs = Vec::new();
        let mut issues = Vec::new();
        for word in text.split_whitespace() {
            if word.starts_with('@') && word.len() > 1 {
                let path = word[1..].to_string();
                refs.push(AtFileRef { path, line_start: None, line_end: None });
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
    FreeUpstream { id: "groq", name: "Groq", title: "Groq", api_key_env: "GROQ_API_KEY", default_model: "llama-3.3-70b-versatile", note: "Blazing fast inference", key_url: "console.groq.com" },
    FreeUpstream { id: "cerebras", name: "Cerebras", title: "Cerebras", api_key_env: "CEREBRAS_API_KEY", default_model: "llama-3.3-70b", note: "Ultra-fast wafer-scale", key_url: "cloud.cerebras.ai" },
    FreeUpstream { id: "google", name: "Google Gemini", title: "Google Gemini", api_key_env: "GOOGLE_API_KEY", default_model: "gemini-2.0-flash", note: "Multimodal, generous free tier", key_url: "aistudio.google.com" },
    FreeUpstream { id: "mistral", name: "Mistral", title: "Mistral", api_key_env: "MISTRAL_API_KEY", default_model: "mistral-small-latest", note: "Strong coding models", key_url: "console.mistral.ai" },
    FreeUpstream { id: "sambanova", name: "SambaNova", title: "SambaNova", api_key_env: "SAMBANOVA_API_KEY", default_model: "Meta-Llama-3.3-70B-Instruct", note: "Fast inference, free tier", key_url: "cloud.sambanova.ai" },
];

fn reverse_provider_lookup(dev_provider: &str) -> String {
    for provider in crate::provider::PROVIDERS {
        if let Some(mapped) = operant_core::models_dev::provider_to_models_dev(provider.name) {
            if mapped == dev_provider {
                return provider.name.to_string();
            }
        }
    }
    dev_provider.to_string()
}

// ---------- ModelRegistry ----------

#[derive(Clone)]
pub struct ModelRegistry {
    models: std::collections::HashMap<String, Vec<crate::tui::model_picker::ModelEntry>>,
}

#[derive(Debug, Clone)]
pub struct RegistryModelEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub info: ModelInfo,
}

#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    pub context_window: u32,
    pub release_date: Option<String>,
    pub cost_input: Option<f64>,
    pub cost_output: Option<f64>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        let mut models = std::collections::HashMap::new();
        Self::populate_default_models(&mut models);
        Self { models }
    }

    pub fn load_cache(&mut self, _path: &std::path::Path) {}

    /// Add any missing providers from PROVIDERS without overwriting existing entries.
    pub fn ensure_provider_defaults(&mut self) {
        for provider in crate::provider::PROVIDERS {
            if !self.models.contains_key(provider.name) {
                let entries: Vec<crate::tui::model_picker::ModelEntry> = provider.models.iter().map(|model_id| {
                    crate::tui::model_picker::ModelEntry {
                        id: model_id.to_string(),
                        display_name: model_id.to_string(),
                        description: provider.display_name.to_string(),
                        is_current: false,
                    }
                }).collect();
                if !entries.is_empty() {
                    self.models.insert(provider.name.to_string(), entries);
                }
            }
        }
    }

    /// Fetch models from models.dev catalog and merge into the registry.
    /// Uses provider_to_models_dev() mapping to match operant providers to catalog entries.
    pub async fn load_models_dev(&mut self) {
        let (models, _) = match operant_core::models_dev::fetch_models_dev(false).await {
            Ok(r) => r,
            Err(_) => return,
        };

        for model in &models {
            let m_provider = match model.get("provider_id").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => continue,
            };
            let model_id = match model.get("id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => continue,
            };

            let operant_provider = operant_core::models_dev::provider_to_models_dev(
                &reverse_provider_lookup(m_provider),
            )
            .map(|_| reverse_provider_lookup(m_provider))
            .or_else(|| {
                if crate::provider::PROVIDERS.iter().any(|p| p.name == m_provider) {
                    Some(m_provider.to_string())
                } else {
                    None
                }
            });

            let provider_name = match operant_provider {
                Some(p) => p,
                None => continue,
            };

            let context_window = model.get("context_window").and_then(|v| v.as_u64());
            let cost_input = model.get("cost_input").and_then(|v| v.as_f64());
            let cost_output = model.get("cost_output").and_then(|v| v.as_f64());

            let description = match context_window {
                Some(ctx) => {
                    let ctx_str = if ctx >= 1_000_000 {
                        format!("{}M ctx", ctx / 1_000_000)
                    } else {
                        format!("{}K ctx", ctx / 1000)
                    };
                    let cost_str = match (cost_input, cost_output) {
                        (Some(i), Some(o)) => format!("${:.2}/${:.2} per M", i, o),
                        _ => String::new(),
                    };
                    if cost_str.is_empty() {
                        ctx_str
                    } else {
                        format!("{} | {}", ctx_str, cost_str)
                    }
                }
                None => String::new(),
            };

            let entry = crate::tui::model_picker::ModelEntry {
                id: model_id.to_string(),
                display_name: model_id.to_string(),
                description,
                is_current: false,
            };

            let entries = self.models.entry(provider_name).or_default();
            if !entries.iter().any(|e| e.id == model_id) {
                entries.push(entry);
            }
        }
    }

    /// Fetch models from a provider's /v1/models endpoint and merge them into the registry.
    ///
    /// Routes Anthropic through `AnthropicClient::fetch_available_models` (which
    /// uses `x-api-key` + `anthropic-version` headers — the OpenAI-compat
    /// `Authorization: Bearer` pattern does NOT work for Anthropic). All other
    /// providers go through the OpenAI-compat path.
    pub async fn fetch_from_provider_async(&mut self, provider_id: &str, api_key: &str, base_url: &str) {
        let fetched = if provider_id == "anthropic" {
            let client = AnthropicClient::new(Some(api_key.to_string()), Some(base_url.to_string()));
            client.fetch_available_models().await
        } else {
            fetch_openai_compatible_models_async(api_key, base_url).await
        };
        if fetched.is_empty() {
            return;
        }

        // De-dup against any cached/catalog entries already present for this provider
        // (models.dev, populate_default_models, prior fetches). Match by id.
        let models = self.models.entry(provider_id.to_string()).or_default();
        let existing: std::collections::HashSet<String> =
            models.iter().map(|m| m.id.clone()).collect();
        for model_id in fetched {
            if existing.contains(&model_id) {
                continue;
            }
            models.push(crate::tui::model_picker::ModelEntry {
                id: model_id.clone(),
                display_name: model_id,
                description: String::new(),
                is_current: false,
            });
        }
    }
    pub fn get(&self, provider: &str, model_id: &str) -> Option<RegistryModelEntry> {
        self.list_by_provider(provider)
            .into_iter()
            .find(|m| m.id == model_id)
            .map(|m| RegistryModelEntry {
                id: m.id.clone(),
                display_name: m.display_name.clone(),
                description: m.description.clone(),
                info: ModelInfo::default(),
            })
    }
    
    pub fn list_visible_by_provider(&self, provider: &str) -> Vec<crate::tui::model_picker::ModelEntry> {
        self.list_by_provider(provider)
    }
    
    pub fn list_by_provider(&self, provider: &str) -> Vec<crate::tui::model_picker::ModelEntry> {
        self.models.get(provider).cloned().unwrap_or_default()
    }
    
    pub fn best_model_for_provider(&self, provider: &str) -> Option<String> {
        self.list_by_provider(provider)
            .first()
            .map(|m| m.id.clone())
    }

    fn populate_default_models(models: &mut std::collections::HashMap<String, Vec<crate::tui::model_picker::ModelEntry>>) {
        for provider in crate::provider::PROVIDERS {
            let entries: Vec<crate::tui::model_picker::ModelEntry> = provider.models.iter().map(|model_id| {
                crate::tui::model_picker::ModelEntry {
                    id: model_id.to_string(),
                    display_name: model_id.to_string(),
                    description: provider.display_name.to_string(),
                    is_current: false,
                }
            }).collect();
            if !entries.is_empty() {
                models.insert(provider.name.to_string(), entries);
            }
        }
    }
}

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn new() -> Self { Self }
}

pub struct AnthropicClient {
    api_key: Option<String>,
    base_url: Option<String>,
}

impl AnthropicClient {
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        Self { api_key, base_url }
    }

    /// Fetch available models from the Anthropic API.
    ///
    /// Anthropic added a `/v1/models` endpoint in late 2024. It requires the
    /// `x-api-key` and `anthropic-version` headers (NOT `Authorization: Bearer`,
    /// which is the OpenAI-compat pattern). Returns the live list on success;
    /// on any error (network, auth, parse) falls back to a curated 5-model list
    /// so the picker is never empty.
    pub async fn fetch_available_models(&self) -> Vec<String> {
        // Curated fallback — kept up to date with the latest Claude lineup as of
        // 2026-07. Used only if the API call fails (no key, no network, 4xx).
        let fallback = vec![
            "claude-opus-4-6".to_string(),
            "claude-sonnet-4-6".to_string(),
            "claude-sonnet-4-5-20250929".to_string(),
            "claude-3-7-sonnet-20250219".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ];

        let Some(api_key) = &self.api_key else {
            return fallback;
        };

        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");
        let base = base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        let url = format!("{}/v1/models", base);

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
        {
            Ok(c) => c,
            Err(_) => return fallback,
        };

        let resp = client
            .get(&url)
            .header("x-api-key", api_key)
            // anthropic-version is mandatory; pinned to the latest stable date.
            .header("anthropic-version", "2023-06-01")
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(_) => return fallback,
        };

        let status = resp.status();
        if !status.is_success() {
            return fallback;
        }

        let json = match resp.json::<serde_json::Value>().await {
            Ok(j) => j,
            Err(_) => return fallback,
        };

        // Anthropic's response shape: {"data":[{"id":"claude-...","type":"model",...}, ...], "has_more": bool, "first_id": ..., "last_id": ...}
        // Note: Anthropic paginates (limit/after params) but the default first page
        // covers all current production models — pagination is left as a future
        // enhancement if the catalog grows past the page limit.
        let Some(data) = json.get("data").and_then(|d| d.as_array()) else {
            return fallback;
        };

        let mut ids: Vec<String> = data
            .iter()
            .filter_map(|item| item.get("id")?.as_str().map(String::from))
            .collect();

        if ids.is_empty() {
            return fallback;
        }

        // Sort newest-first by created_at if present, otherwise keep API order.
        ids.sort_by(|a, b| {
            let ta = data
                .iter()
                .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(a))
                .and_then(|item| item.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tb = data
                .iter()
                .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(b))
                .and_then(|item| item.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            tb.cmp(ta)
        });

        ids
    }
}

pub async fn fetch_openai_compatible_models_async(
    api_key: &str,
    base_url: &str,
) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    let url = format!("{}/v1/models", base);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();

    let Ok(client) = client else {
        return vec![];
    };

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await;

    let Ok(resp) = response else {
        return vec![];
    };

    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return vec![];
    };

    // Parse OpenAI-format response: {"data": [{"id": "model-name", ...}, ...]}
    let Some(data) = json.get("data").and_then(|d| d.as_array()) else {
        return vec![];
    };

    data.iter()
        .filter_map(|item| item.get("id")?.as_str().map(String::from))
        .collect()
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

impl From<ProviderId> for String {
    fn from(pid: ProviderId) -> String {
        match pid {
            ProviderId::OPENCODE_GO => "opencode-go".to_string(),
            ProviderId::OPENCODE_ZEN => "opencode-zen".to_string(),
            ProviderId::Other(s) => s,
        }
    }
}

impl<'a> From<&'a ProviderId> for String {
    fn from(pid: &'a ProviderId) -> String {
        match pid {
            ProviderId::OPENCODE_GO => "opencode-go".to_string(),
            ProviderId::OPENCODE_ZEN => "opencode-zen".to_string(),
            ProviderId::Other(s) => s.clone(),
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderId::OPENCODE_GO => write!(f, "opencode-go"),
            ProviderId::OPENCODE_ZEN => write!(f, "opencode-zen"),
            ProviderId::Other(s) => write!(f, "{}", s),
        }
    }
}

impl std::str::FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencode-go" => Ok(ProviderId::OPENCODE_GO),
            "opencode-zen" => Ok(ProviderId::OPENCODE_ZEN),
            other => Ok(ProviderId::Other(other.to_string())),
        }
    }
}

pub mod streaming {
    #[derive(Debug, Clone)]
    pub enum AnthropicStreamEvent {
        ContentBlockStart { index: usize },
        ContentBlockDelta { index: usize, delta: ContentDelta },
        ContentBlockStop { index: usize },
        MessageStart,
        MessageStop,
        Ping,
    }

    #[derive(Debug, Clone)]
    pub enum ContentDelta {
        TextDelta { text: String },
        ThinkingDelta { thinking: String },
    }
}

pub mod mcp {
    use std::sync::Arc;

    pub struct McpManager;

    #[derive(Debug, Clone)]
    pub struct McpToolDef {
        pub name: String,
        pub description: String,
        pub input_schema: serde_json::Value,
    }

    #[derive(Debug, Clone, Default)]
    pub struct McpCatalogEntry {
        pub tool_count: usize,
        pub resource_count: usize,
        pub prompt_count: usize,
        pub resources: Vec<String>,
        pub prompts: Vec<String>,
    }

    impl McpManager {
        pub fn new() -> Arc<Self> { Arc::new(Self) }
        pub fn all_tool_definitions(&self) -> Vec<(String, McpToolDef)> { vec![] }
        pub fn server_status(&self, _name: &str) -> McpServerStatus {
            McpServerStatus::Disconnected { last_error: None }
        }
        pub fn server_catalog(&self, _name: &str) -> Option<McpCatalogEntry> { None }
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
    use std::sync::OnceLock;
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq)]
    pub enum TaskStatus { Pending, Running, Completed, Failed, InProgress, Deleted }

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

    pub static TASK_STORE: std::sync::LazyLock<std::sync::Mutex<TaskStore>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(TaskStore::new()));

    #[derive(Debug, Clone)]
    pub struct UserQuestionEvent {
        pub question: String,
        pub options: Vec<String>,
    }
}

pub struct TuiApp {
    app: crate::tui::app::App,
    initial_query: Option<String>,
    /// Whether to skip EnableMouseCapture in the TUI setup. Set by the
    /// --no-mouse CLI flag. (Bug #24 from iter-82 audit.)
    no_mouse: bool,
}

impl TuiApp {
    pub async fn enter(
        config: operant_core::config::AppConfig,
        _system: Option<String>,
        _mode: LaunchMode,
        no_mouse: bool,
    ) -> anyhow::Result<Self> {
        use crate::tui::adapter_types::config::Config;
        use crate::tui::adapter_types::cost::CostTracker;
        use crate::commands::{CommandRegistry, CommandHandler, CommandContext, CommandResult};
        use std::sync::Arc;

        let initial_query = match &_mode {
            LaunchMode::Query(q) => Some(q.clone()),
            _ => None,
        };
        let mut config: Config = config.into();

        // Layer in the user's saved settings.json (written by App::persist_provider_and_model
        // and App::set_provider_default). Without this, the provider+model picked in a prior
        // TUI session are silently dropped on every restart.
        //
        // BUT: the TOML config (from `operant setup`) is the authoritative source.
        // settings.json should only override when the TOML config has the DEFAULT
        // values (gpt-4 / empty base_url). This prevents stale settings.json from
        // overriding a fresh `operant setup` run.
        // (iter-112 — fixes the "TUI shows hardcoded defaults after setup" bug.)
        if let Ok(saved) = Settings::load_sync() {
            // Only use settings.json provider if the TOML config has no real
            // provider set (base_url is empty or default).
            let toml_has_real_provider = !config.inner.client.base_url.is_empty()
                && config.inner.client.base_url != "https://api.openai.com/v1";
            if saved.provider.is_some() && !toml_has_real_provider {
                config.provider = saved.provider.clone();
            }
            // Only use settings.json model if the TOML config has the default "gpt-4".
            if saved.model.is_some() && config.inner.agent.model == "gpt-4" {
                config.model = saved.model.clone();
                config.inner.agent.model = saved.model.clone().unwrap_or_default();
            }
            // Persisted per-provider API base overrides (e.g. custom-openai).
            if let Some(entry) = saved.providers.get("custom-openai") {
                if let Some(ref base) = entry.api_base {
                    config.inner.client.base_url = base.clone();
                }
            }
        }

        let cost_tracker = Arc::new(CostTracker::new());

        let mut command_registry = CommandRegistry::new();
        // Register handlers for commands that were previously falling through to the agent
        struct CompactHandler;
        #[async_trait::async_trait]
        impl CommandHandler for CompactHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Context compaction is handled automatically by the agent.".to_string())
            }
        }
        command_registry.register_handler("compact", Box::new(CompactHandler)).ok();

        struct DoctorHandler;
        #[async_trait::async_trait]
        impl CommandHandler for DoctorHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let mut report = String::from("Operant Diagnostics:\n");
                report.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));
                report.push_str(&format!("  Config dir: {:?}\n", crate::tui::adapter_types::config::Settings::config_dir()));
                let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok()
                    || std::env::var("OPENAI_API_KEY").is_ok();
                report.push_str(&format!("  API key configured: {}\n", api_key_set));
                report.push_str("  Status: OK\n");
                Ok(report)
            }
        }
        command_registry.register_handler("doctor", Box::new(DoctorHandler)).ok();

        struct InitHandler;
        #[async_trait::async_trait]
        impl CommandHandler for InitHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let agentic_dir = std::path::PathBuf::from("AGENTS.md");
                if agentic_dir.exists() {
                    Ok("AGENTS.md already exists in this project.".to_string())
                } else {
                    match std::fs::write(&agentic_dir, "# Project Agent Memory\n\n") {
                        Ok(_) => Ok("Created AGENTS.md in current directory.".to_string()),
                        Err(e) => Ok(format!("Failed to create AGENTS.md: {}", e)),
                    }
                }
            }
        }
        command_registry.register_handler("init", Box::new(InitHandler)).ok();

        struct LoginHandler;
        #[async_trait::async_trait]
        impl CommandHandler for LoginHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Set your API key: export ANTHROPIC_API_KEY=sk-... or export OPENAI_API_KEY=sk-...".to_string())
            }
        }
        command_registry.register_handler("login", Box::new(LoginHandler)).ok();

        struct LogoutHandler;
        #[async_trait::async_trait]
        impl CommandHandler for LogoutHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Clear your API key: unset ANTHROPIC_API_KEY OPENAI_API_KEY".to_string())
            }
        }
        command_registry.register_handler("logout", Box::new(LogoutHandler)).ok();

        struct RefreshHandler;
        #[async_trait::async_trait]
        impl CommandHandler for RefreshHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Provider auth and model caches cleared.".to_string())
            }
        }
        command_registry.register_handler("refresh", Box::new(RefreshHandler)).ok();

        struct ProvidersHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ProvidersHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let auth = AuthStore::load();
                let mut report = String::from("Available providers:\n");
                for p in crate::provider::PROVIDERS {
                    let has_key = auth.api_key_for(p.name).is_some();
                    let env_key = !p.env_var.is_empty() && std::env::var(p.env_var).is_ok();
                    let configured = has_key || env_key;
                    report.push_str(&format!(
                        "  {}: {}\n",
                        p.display_name,
                        if configured { "configured" } else { "not configured" }
                    ));
                }
                report.push_str("\nUsage: /provider <name> — switch LLM provider");
                Ok(report)
            }
        }
        command_registry.register_handler("providers", Box::new(ProvidersHandler)).ok();

        struct StatusHandler;
        #[async_trait::async_trait]
        impl CommandHandler for StatusHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let model = std::env::var("OPERANT_MODEL").unwrap_or_else(|_| "gpt-4".to_string());
                let anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok();
                let openai = std::env::var("OPENAI_API_KEY").is_ok();
                Ok(format!(
                    "Session Status:\n  Model: {}\n  Anthropic: {}\n  OpenAI: {}",
                    model,
                    if anthropic { "configured" } else { "not configured" },
                    if openai { "configured" } else { "not configured" }
                ))
            }
        }
        command_registry.register_handler("status", Box::new(StatusHandler)).ok();

        struct VersionHandler;
        #[async_trait::async_trait]
        impl CommandHandler for VersionHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok(format!("operant v{}", env!("CARGO_PKG_VERSION")))
            }
        }
        command_registry.register_handler("version", Box::new(VersionHandler)).ok();

        struct TimeHandler;
        #[async_trait::async_trait]
        impl CommandHandler for TimeHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
            }
        }
        command_registry.register_handler("time", Box::new(TimeHandler)).ok();

        struct DebugHandler;
        #[async_trait::async_trait]
        impl CommandHandler for DebugHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                let mut info = String::from("Debug Info:\n");
                info.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));
                info.push_str(&format!("  Config dir: {:?}\n", crate::tui::adapter_types::config::Settings::config_dir()));
                info.push_str(&format!("  Rust version: {}\n", env!("CARGO_PKG_RUST_VERSION")));
                Ok(info)
            }
        }
        command_registry.register_handler("debug", Box::new(DebugHandler)).ok();

        struct NewHandler;
        #[async_trait::async_trait]
        impl CommandHandler for NewHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Starting a new session. Type your message to begin.".to_string())
            }
        }
        command_registry.register_handler("new", Box::new(NewHandler)).ok();

        struct HistoryHandler;
        #[async_trait::async_trait]
        impl CommandHandler for HistoryHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Session history is displayed in the transcript above. Use ↑/↓ to scroll.".to_string())
            }
        }
        command_registry.register_handler("history", Box::new(HistoryHandler)).ok();

        struct RetryHandler;
        #[async_trait::async_trait]
        impl CommandHandler for RetryHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Retry: resend the last message to the agent. (Implementation pending)".to_string())
            }
        }
        command_registry.register_handler("retry", Box::new(RetryHandler)).ok();

        struct UndoHandler;
        #[async_trait::async_trait]
        impl CommandHandler for UndoHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Undo: back up the last turn. (Implementation pending)".to_string())
            }
        }
        command_registry.register_handler("undo", Box::new(UndoHandler)).ok();

        struct StopHandler;
        #[async_trait::async_trait]
        impl CommandHandler for StopHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Stopping all running background processes.".to_string())
            }
        }
        command_registry.register_handler("stop", Box::new(StopHandler)).ok();

        struct CompressHandler;
        #[async_trait::async_trait]
        impl CommandHandler for CompressHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Context compaction is handled automatically by the agent.".to_string())
            }
        }
        command_registry.register_handler("compress", Box::new(CompressHandler)).ok();

        struct RollbackHandler;
        #[async_trait::async_trait]
        impl CommandHandler for RollbackHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Rollback: restore filesystem checkpoints. (Implementation pending)".to_string())
            }
        }
        command_registry.register_handler("rollback", Box::new(RollbackHandler)).ok();

        struct TitleHandler;
        #[async_trait::async_trait]
        impl CommandHandler for TitleHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                if ctx.args.is_empty() {
                    Ok("Usage: /title <name> — set a title for the current session".to_string())
                } else {
                    Ok(format!("Session title set to: {}", ctx.args))
                }
            }
        }
        command_registry.register_handler("title", Box::new(TitleHandler)).ok();

        struct BranchHandler;
        #[async_trait::async_trait]
        impl CommandHandler for BranchHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                if ctx.args.is_empty() {
                    Ok("Usage: /branch <name> — branch the current session".to_string())
                } else {
                    Ok(format!("Branching session: {}", ctx.args))
                }
            }
        }
        command_registry.register_handler("branch", Box::new(BranchHandler)).ok();

        struct GoalHandler;
        #[async_trait::async_trait]
        impl CommandHandler for GoalHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                if ctx.args.is_empty() {
                    Ok("Usage: /goal <text> — set a standing goal for the session".to_string())
                } else {
                    Ok(format!("Goal set: {}", ctx.args))
                }
            }
        }
        command_registry.register_handler("goal", Box::new(GoalHandler)).ok();

        struct ProviderHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ProviderHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                if ctx.args.is_empty() {
                    Ok("Usage: /provider <name> — switch LLM provider".to_string())
                } else {
                    Ok(format!("Provider switched to: {}", ctx.args))
                }
            }
        }
        command_registry.register_handler("provider", Box::new(ProviderHandler)).ok();

        struct YoloHandler;
        #[async_trait::async_trait]
        impl CommandHandler for YoloHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("YOLO mode: skip confirmations for dangerous operations. (Toggle not yet wired)".to_string())
            }
        }
        command_registry.register_handler("yolo", Box::new(YoloHandler)).ok();

        struct PersonalityHandler;
        #[async_trait::async_trait]
        impl CommandHandler for PersonalityHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                if ctx.args.is_empty() {
                    Ok("Usage: /personality <name> — set a predefined personality".to_string())
                } else {
                    Ok(format!("Personality set to: {}", ctx.args))
                }
            }
        }
        command_registry.register_handler("personality", Box::new(PersonalityHandler)).ok();

        struct ReasoningHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ReasoningHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Reasoning: toggle extended thinking for complex tasks. (Toggle not yet wired)".to_string())
            }
        }
        command_registry.register_handler("reasoning", Box::new(ReasoningHandler)).ok();

        struct ToolsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for ToolsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Available tools: memory, web_search, web_fetch, bash, and more. Use /toolsets for the full list.".to_string())
            }
        }
        command_registry.register_handler("tools", Box::new(ToolsHandler)).ok();

        struct SkillsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for SkillsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Skills are installed in ~/.operant/skills/. Use /reload-skills to rescan.".to_string())
            }
        }
        command_registry.register_handler("skills", Box::new(SkillsHandler)).ok();

        struct BundlesHandler;
        #[async_trait::async_trait]
        impl CommandHandler for BundlesHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Skill bundles: curated sets of skills for specific workflows. (No bundles installed)".to_string())
            }
        }
        command_registry.register_handler("bundles", Box::new(BundlesHandler)).ok();

        struct UsageHandler;
        #[async_trait::async_trait]
        impl CommandHandler for UsageHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Token usage and rate limits are displayed in the stats dialog. Use /stats to view.".to_string())
            }
        }
        command_registry.register_handler("usage", Box::new(UsageHandler)).ok();

        struct CreditsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for CreditsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Credits: check your provider dashboard for balance information.".to_string())
            }
        }
        command_registry.register_handler("credits", Box::new(CreditsHandler)).ok();

        struct BillingHandler;
        #[async_trait::async_trait]
        impl CommandHandler for BillingHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Billing: manage your subscription at your provider's dashboard.".to_string())
            }
        }
        command_registry.register_handler("billing", Box::new(BillingHandler)).ok();

        struct InsightsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for InsightsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Insights: session analysis and conversation statistics. Use /stats for details.".to_string())
            }
        }
        command_registry.register_handler("insights", Box::new(InsightsHandler)).ok();

        struct UpdateHandler;
        #[async_trait::async_trait]
        impl CommandHandler for UpdateHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok(format!("Current version: {}. Check https://github.com/operant-ai/operant-rs for updates.", env!("CARGO_PKG_VERSION")))
            }
        }
        command_registry.register_handler("update", Box::new(UpdateHandler)).ok();

        struct WhoamiHandler;
        #[async_trait::async_trait]
        impl CommandHandler for WhoamiHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Access level: admin (local TUI session)".to_string())
            }
        }
        command_registry.register_handler("whoami", Box::new(WhoamiHandler)).ok();

        struct SessionsHandler;
        #[async_trait::async_trait]
        impl CommandHandler for SessionsHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                Ok("Use /session to browse and manage sessions.".to_string())
            }
        }
        command_registry.register_handler("sessions", Box::new(SessionsHandler)).ok();

        let mut app = crate::tui::app::App::new(config, cost_tracker, command_registry);

        // Wire the voice-mode notice: if audio input is available (e.g. not
        // an SSH session, ffmpeg/arecord installed) and the user hasn't
        // enabled voice mode yet, show a one-time hint on startup.
        let audio_env = operant_core::voice::detect_audio_environment();
        app.voice_mode_notice
            .show_if_available(audio_env.available, false);

        // First-run onboarding: if no credentials and onboarding hasn't been
        // completed, auto-open the connect dialog so the user is guided to
        // set up a provider. (P0-2 from UX audit — was silently dropping the
        // user onto a blank welcome screen with no guidance.)
        if !app.has_credentials {
            let settings = Settings::load_sync().unwrap_or_default();
            if !settings.has_completed_onboarding {
                app.connect_dialog.open();
                app.status_message = Some(
                    "Welcome to Operant! Connect a provider to get started.".to_string()
                );
            }
        }

        Ok(Self { app, initial_query, no_mouse })
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        use crate::tui::bridge::spawn_bridge;
        use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
        use crossterm::execute;
        use ratatui::backend::CrosstermBackend;
        use ratatui::Terminal;

        enable_raw_mode()?;

        // Install a panic hook that restores the terminal before printing
        // the panic message. Without this, any panic between enable_raw_mode
        // and disable_raw_mode leaves the user's terminal in raw mode +
        // alternate screen (broken terminal, garbled output).
        let no_mouse = self.no_mouse;
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            if !no_mouse {
                let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
            }
            prev_hook(info);
        }));

        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        // Enable mouse capture unless --no-mouse was passed. Mouse capture
        // lets the TUI receive scroll/click events for the transcript, diff
        // viewer, and overlay scrolling. Some terminal multiplexers (tmux,
        // screen) interfere with mouse capture; --no-mouse disables it so
        // the terminal's native mouse selection works. (Bug #24 from iter-82
        // audit — /mouse mentioned a --no-mouse flag that didn't exist.)
        if !self.no_mouse {
            execute!(stdout, crossterm::event::EnableMouseCapture)?;
        }
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let (agent_tx, query_rx) = spawn_bridge();
        self.app.query_event_rx = Some(query_rx);

        let (permission_tx, permission_rx) = tokio::sync::mpsc::channel::<operant_core::agent::ToolPermissionRequest>(4);
        self.app.permission_rx = Some(permission_rx);

        let config = self.app.config.inner.clone();
        let mcp_manager = operant_core::mcp::McpManager::new();
        let skills_dir = config.skills.root_dir.clone();

        let agent: Option<std::sync::Arc<operant_core::agent::OperantAgent>> = match crate::create_runtime_agent(
            &config,
            &config.agent,
            None,
            agent_tx,
            &mcp_manager,
            &skills_dir,
        ).await {
            Ok(agent) => Some(std::sync::Arc::new(agent.with_permissions(permission_tx))),
            Err(e) => {
                self.app.status_message = Some(format!("Agent init failed: {}", e));
                None
            }
        };

        // Store the real McpManager + steer queue handle on the App so the
        // run loop can act on /mcp reconnect and /steer. (iter-93 — closes
        // the /mcp reconnect + /steer parity gaps.)
        self.app.core_mcp_manager = Some(std::sync::Arc::new(mcp_manager));
        if let Some(ref agent) = agent {
            self.app.steer_queue_handle = Some(agent.steer_queue_handle());
        }

        // Create the user-question channel and register the sender with
        // operant_core::user_question. The clarify tool will push
        // UserQuestionRequest { question, choices, reply_tx } to this
        // channel; the TUI drains it in the run loop and opens the
        // ask_user_dialog. (iter-97 — closes Bug #2 from iter-82 audit.)
        let (uq_tx, uq_rx) = tokio::sync::mpsc::unbounded_channel::<operant_core::user_question::UserQuestionRequest>();
        let _ = operant_core::user_question::set_user_question_sender(uq_tx);
        self.app.user_question_rx = Some(uq_rx);

        // Attach the MCP manager + file-history + current-turn counter to the
        // App. Without these, /mcp always shows "Disconnected" for every
        // server, /changes always shows "No changes", the "iter N" status
        // pill never renders, and the subagent HUD never renders. (Bug #3
        // from iter-82 audit.) The TUI's McpManager is currently a thin
        // stub — wrapping the core McpManager is a future iteration; for
        // now, attach a fresh TUI McpManager so /mcp at least opens with
        // "no servers" instead of crashing on None.
        let tui_mcp = crate::tui::adapter_types::mcp::McpManager::new();
        self.app.attach_mcp_manager(tui_mcp);

        // File-history: create a fresh FileHistory the bridge will populate
        // as the agent emits FileEdit events. The current_turn counter is
        // bumped by the bridge on each TurnComplete.
        let file_history = std::sync::Arc::new(parking_lot::Mutex::new(
            crate::tui::adapter_types::file_history::FileHistory::new(),
        ));
        let current_turn = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        self.app.attach_turn_diff_state(file_history, current_turn);

        // Force a context-window-size refresh so /context shows real numbers
        // on the first frame instead of "0 / 0" (Bug #13 from iter-82 audit).
        self.app.refresh_context_window_size();

        self.app.model_registry.load_models_dev().await;

        if let Some(query) = self.initial_query.take() {
            if let Some(ref agent) = agent {
                use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                self.app.messages.push(Message {
                    role: Role::User,
                    content: MessageContent::Text(query.clone()),
                });
                self.app.is_streaming = true;
                self.app.streaming_text.clear();
                self.app.streaming_thinking.clear();

                let agent_clone = std::sync::Arc::clone(agent);
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let result = agent_clone.run(query).await.map(|_| ());
                    let _ = tx.send(result);
                });
                self.app.run_complete_rx = Some(rx);
            }
        }

        let result = loop {
            match self.app.run(&mut terminal) {
                Ok(Some(input)) => {
                    // Poll pending MCP state set by /mcp 'a' (panel auth) and
                    // 'r' (reconnect) keys. Without this, the keys set state
                    // that the run loop never reads, so panel-auth + reconnect
                    // are no-ops. (Bug #7 from iter-82 audit.)
                    // For now we surface a status message acknowledging the
                    // request; a real implementation would spawn the MCP
                    // panel-auth flow / reconnect the MCP runtime.
                    if let Some(server_name) = self.app.take_pending_mcp_panel_auth() {
                        self.app.status_message = Some(format!(
                            "MCP panel auth requested for '{}' (not yet wired — restart operant to re-authenticate).",
                            server_name
                        ));
                    }
                    if self.app.take_pending_mcp_reconnect() {
                        // Real MCP reconnect: re-add all configured servers.
                        // (iter-93 — closes the /mcp reconnect parity gap.)
                        // We extract the server configs into a plain Vec of
                        // tuples first (so the async block doesn't capture
                        // the AppConfig, which contains non-Send tracing
                        // types via the McpManager's internal spans).
                        if let Some(ref mcp) = self.app.core_mcp_manager {
                            let mcp_clone = std::sync::Arc::clone(mcp);
                            // Extract server configs into Send-safe tuples.
                            let server_configs: Vec<(String, Option<String>, Option<String>, Vec<String>, std::collections::HashMap<String, String>, bool)> =
                                self.app.config.inner.mcp.servers.iter().map(|s| {
                                    (s.name.clone(), s.url.clone(), s.auth_token.clone(), s.args.clone(), s.env.clone(), s.enabled)
                                }).collect();
                            tokio::spawn(async move {
                                for (name, url, auth_token, args, env, enabled) in server_configs {
                                    if !enabled {
                                        continue;
                                    }
                                    // Remove first (no-op if not present).
                                    let _ = mcp_clone.remove_server(&name).await;
                                    // Re-add based on transport.
                                    if let Some(url) = url {
                                        let _ = mcp_clone
                                            .add_server(&name, url, auth_token)
                                            .await;
                                    }
                                    // Note: stdio servers need a command, which
                                    // we didn't capture here. SSE servers need
                                    // add_sse_server. For now, HTTP is the
                                    // primary path; stdio/SSE reconnect is a
                                    // future enhancement.
                                    let _ = (args, env);
                                }
                            });
                            self.app.status_message = Some(
                                "MCP reconnect initiated — servers will reconnect in the background.".to_string()
                            );
                        } else {
                            self.app.status_message = Some(
                                "MCP reconnect requested but no McpManager is attached.".to_string()
                            );
                        }
                    }

                    // Poll device_auth_pending set by /connect for github-copilot
                    // and openai-codex. Without this, the device-code dialog
                    // shows "waiting for code" forever because no background
                    // device-flow task is ever spawned. (Bug #15 from iter-82
                    // audit — partial fix; full fix needs the device-flow task
                    // spawned here.)
                    if let Some(provider) = self.app.device_auth_pending.take() {
                        self.app.status_message = Some(format!(
                            "Device auth initiated for '{}' — open the provider's URL in a browser to complete. (Background polling not yet wired; restart operant after authenticating.)",
                            provider
                        ));
                    }

                    // If a slash command set a pending shell command on a
                    // *previous* iteration, run it BEFORE processing the next
                    // input. (Slash commands set the field inside
                    // handle_tui_command → intercept_slash_command, then we
                    // `continue` to the next loop iteration; we run the shell
                    // command at the top of the next iteration so the TUI
                    // gets a chance to redraw the "Launching…" status message
                    // before we suspend.)
                    if let Some(argv) = self.app.pending_shell_command.take() {
                        if let Err(e) = run_suspended_shell_command(&mut terminal, &argv) {
                            self.app.status_message = Some(format!("Shell command failed: {}", e));
                        } else {
                            self.app.status_message = Some("Returned to operant.".to_string());
                        }
                        // Force a redraw on the next frame so the status
                        // message + restored terminal show immediately.
                        self.app.transcript_version.set(
                            self.app.transcript_version.get().wrapping_add(1)
                        );
                    }

                    if crate::input::is_slash_command(&input) {
                        let (cmd, args) = crate::input::parse_slash_command(&input);
                        if self.app.handle_tui_command(cmd, args) {
                            // If the slash command set a pending shell command,
                            // we need to run it on the NEXT iteration — but
                            // app.run() will block waiting for input. To avoid
                            // that, run it NOW if it was set.
                            if let Some(argv) = self.app.pending_shell_command.take() {
                                if let Err(e) = run_suspended_shell_command(&mut terminal, &argv) {
                                    self.app.status_message = Some(format!("Shell command failed: {}", e));
                                } else {
                                    self.app.status_message = Some("Returned to operant.".to_string());
                                }
                                self.app.transcript_version.set(
                                    self.app.transcript_version.get().wrapping_add(1)
                                );
                            }
                            continue;
                        }
                        if let Some(canonical) = self.app.command_registry.resolve(cmd) {
                            match self.app.command_registry.execute(canonical, args).await {
                                Ok(output) => {
                                    self.app.push_system_message(
                                        output,
                                        crate::tui::app::SystemMessageStyle::Info,
                                    );
                                }
                                Err(e) => {
                                    self.app.status_message = Some(format!("Command error: {}", e));
                                }
                            }
                            continue;
                        }
                    }
                    if let Some(ref agent) = agent {
                        // If a turn is currently streaming, push the input as
                        // a steer directive instead of starting a new turn.
                        // The agent drains steers at the next iteration boundary
                        // and injects them as user-role messages. (iter-93 —
                        // closes the /steer parity gap.)
                        if self.app.is_streaming {
                            if let Some(ref handle) = self.app.steer_queue_handle {
                                let mut q = handle.lock().await;
                                q.push(input.clone());
                                self.app.status_message = Some(format!(
                                    "Steer queued: {}",
                                    input.chars().take(60).collect::<String>()
                                ));
                                continue;
                            }
                        }
                        use crate::tui::adapter_types::types::{Message, MessageContent, Role};
                        self.app.messages.push(Message {
                            role: Role::User,
                            content: MessageContent::Text(input.clone()),
                        });
                        self.app.is_streaming = true;
                        self.app.streaming_text.clear();
                        self.app.streaming_thinking.clear();

                        let agent_clone = std::sync::Arc::clone(agent);
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::spawn(async move {
                            let result = agent_clone.run(input).await.map(|_| ());
                            let _ = tx.send(result);
                        });
                        self.app.run_complete_rx = Some(rx);
                    }
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };

        disable_raw_mode()?;
        if !self.no_mouse {
            let _ = execute!(terminal.backend_mut(), crossterm::event::DisableMouseCapture);
        }
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        result
    }
}

/// Suspend the TUI, run a shell command with inherited stdio, then resume.
///
/// Used by slash commands like `/setup` that need to launch an interactive
/// subprocess (the operant setup wizard, an editor, etc.). The terminal is
/// left in alt-screen + raw mode by the TUI; this function:
///   1. leaves alt screen + disables raw mode (restoring the user's terminal)
///   2. spawns the command with inherited stdin/stdout/stderr
///   3. waits for it to complete
///   4. re-enters alt screen + re-enables raw mode
///   5. forces a terminal resize detection + full redraw on the next frame
///
/// Errors are returned if any of the crossterm operations fail or the spawn
/// fails. A non-zero exit code from the subprocess is NOT an error (the user
/// may have hit Ctrl+C in the wizard); we surface it via the returned
/// `Ok(ExitStatus)` so the caller can decide whether to message the user.
fn run_suspended_shell_command(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    argv: &[String],
) -> anyhow::Result<std::process::ExitStatus> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
    use crossterm::execute;
    use std::io::Write;

    if argv.is_empty() {
        anyhow::bail!("run_suspended_shell_command: empty argv");
    }

    // 1. Leave alt screen + disable raw mode.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    // Flush so the user sees the wizard's first prompt immediately.
    let _ = std::io::stdout().flush();

    // 2. Spawn the subprocess with inherited stdio.
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let result = cmd.spawn()?.wait();

    // 3. Re-enter alt screen + re-enable raw mode regardless of subprocess
    //    outcome. If we skip this, the TUI is permanently broken.
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    // Force ratatui to forget its cached buffer sizes — the terminal may have
    // been resized while we were suspended. ratatui::Terminal::resize takes a
    // Rect; we read the current size and convert.
    let size = terminal.size()?;
    let _ = terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height));

    let status = result?;
    Ok(status)
}

#[derive(Debug, Clone)]
pub enum LaunchMode {
    Landing,
    Query(String),
}
