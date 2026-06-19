pub mod config {
    pub use operant_core::config::AppConfig as Config;

    #[derive(Debug, Clone, Default)]
    pub struct Settings {
        pub theme: Theme,
        pub permission_mode: PermissionMode,
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
}

pub mod constants {
    pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
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
        pub fn new() -> Self {
            Self::default()
        }

        pub fn push(&mut self, entry: String) {
            self.entries.push(entry);
        }

        pub fn entries(&self) -> &[String] {
            &self.entries
        }
    }
}

#[derive(Debug, Clone)]
pub enum ImageSource {
    Clipboard,
    File(String),
    Url(String),
}

pub mod keybindings {
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
}

pub mod types {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Role {
        User,
        Assistant,
        System,
    }

    #[derive(Debug, Clone)]
    pub enum ContentBlock {
        Text { text: String },
        Thinking { thinking: String, signature: Option<String> },
        ToolUse { id: String, name: String, input: serde_json::Value },
        ToolResult { tool_use_id: String, content: ToolResultContent, is_error: bool },
    }

    #[derive(Debug, Clone)]
    pub enum ToolResultContent {
        Text(String),
        Image { data: String, media_type: String },
    }

    #[derive(Debug, Clone)]
    pub struct Message {
        pub role: Role,
        pub content: Vec<ContentBlock>,
    }

    impl Message {
        pub fn content_blocks(&self) -> &[ContentBlock] {
            &self.content
        }

        pub fn text_content(&self) -> String {
            self.content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        }
    }

    #[derive(Debug, Clone)]
    pub struct ToolResult {
        pub tool_use_id: String,
        pub content: String,
        pub is_error: bool,
    }
}

pub use types::{ContentBlock, Message, Role, ToolResultContent};

pub mod output_styles {
    use std::collections::HashMap;

    pub struct StyleInfo {
        pub name: String,
        pub accent: ratatui::style::Color,
        pub muted: ratatui::style::Color,
    }

    pub fn builtin_styles() -> Vec<StyleInfo> {
        vec![
            StyleInfo {
                name: "default".to_string(),
                accent: ratatui::style::Color::Rgb(232, 165, 54),
                muted: ratatui::style::Color::Rgb(134, 132, 126),
            },
        ]
    }

    pub fn find_style(name: &str) -> Option<StyleInfo> {
        builtin_styles().into_iter().find(|s| s.name == name)
    }
}

pub fn sample_completion_verb() -> &'static str {
    "done"
}

pub fn sample_spinner_verb() -> &'static str {
    "thinking"
}

pub mod voice {
    #[derive(Debug, Clone)]
    pub enum VoiceEvent {
        RecordingStarted,
        RecordingStopped,
        Transcription(String),
        Error(String),
    }
}

pub mod query {
    pub use crate::tui::types_query::*;
}

pub mod types_query {
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
        MessageDelta { stop_reason: Option<String> },
    }

    #[derive(Debug, Clone, Default)]
    pub struct UsageInfo {
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub total_cost: f64,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum TokenWarningState {
        Normal,
        Warning,
        Critical,
    }
}

pub use query::{QueryEvent, StreamEvent, UsageInfo, TokenWarningState};

pub mod tools {
    #[derive(Debug, Clone, PartialEq)]
    pub enum TaskStatus {
        Pending,
        Running,
        Completed,
        Failed,
    }
}

pub mod compact {
    #[derive(Debug, Clone, PartialEq)]
    pub enum TokenWarningState {
        Normal,
        Warning,
        Critical,
    }
}

pub use compact::TokenWarningState;
