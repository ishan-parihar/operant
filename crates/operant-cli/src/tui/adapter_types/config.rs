use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------- Theme (enum, not struct) ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum Theme {
    Dark,
    Light,
    #[default]
    Default,
    Deuteranopia,
    Custom(String),
}

impl Theme {
    /// Returns the theme name string for theme_colors::ColorPalette::for_theme()
    pub fn as_str(&self) -> &str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Default => "default",
            Theme::Deuteranopia => "deuteranopia",
            Theme::Custom(s) => s,
        }
    }
}

// ---------- PermissionMode ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum PermissionMode {
    AcceptEdits,
    #[default]
    Default,
    BypassPermissions,
    Plan,
}

// ---------- OutputFormat ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    StreamJson,
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
        dirs::home_dir().unwrap_or_default().join(".operant")
    }
}

pub fn resolve_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .ok()
        .filter(|k| !k.is_empty())
}
