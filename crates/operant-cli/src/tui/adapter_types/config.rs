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
// A settings.json written before a field existed must still load; without this
// the load fails and the next write replaces the whole file with defaults.
#[serde(default)]
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
    pub compact_threshold: Option<usize>,
    pub notifications: bool,
    pub show_turn_duration: bool,
    pub terminal_progress_bar: bool,
    pub show_git_branch: bool,
    pub config: InnerConfig,
    pub providers: HashMap<String, ProviderEntry>,
    pub has_completed_onboarding: bool,
}

impl Settings {
    pub fn save_sync(&self) -> anyhow::Result<()> {
        self.save_to(&Self::config_dir().join("settings.json"))
    }
    pub fn load_sync() -> anyhow::Result<Self> {
        Self::load_from(&Self::config_dir().join("settings.json"))
    }
    // Callers read with `unwrap_or_default()` and then write back, so an
    // unparseable file would be replaced silently. Keep it as `.bak` first.
    fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if path.exists() && Self::load_from(path).is_err() {
            let backup = path.with_extension("json.bak");
            tracing::warn!(
                path = %path.display(),
                backup = %backup.display(),
                "settings.json did not parse; keeping it as .bak before overwriting"
            );
            std::fs::copy(path, &backup)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        // Write-then-rename: a crash mid-write leaves the old file, not a truncated one.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
    fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let json = std::fs::read_to_string(path)?;
        let settings: Self = serde_json::from_str(&json)?;
        Ok(settings)
    }
    /// The directory the TUI's own stores live in: `settings.json`,
    /// `auth.json` and the user-level `AGENTS.md` selector.
    ///
    /// The ONE home resolver (`operant_core::platform::operant_home`), so a
    /// PROFILE gets its own visual settings and its own credentials
    /// instead of sharing the default profile's.
    pub fn config_dir() -> std::path::PathBuf {
        operant_core::platform::operant_home()
    }
}

pub fn resolve_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .ok()
        .filter(|k| !k.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_file_written_before_a_field_existed_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"theme":"Dark"}"#).unwrap();

        let loaded = Settings::load_from(&path).expect("a partial file loads");
        assert_eq!(loaded.theme, Theme::Dark);
        assert!(!loaded.vim_enabled, "a missing field takes its default");
    }

    #[test]
    fn saving_over_an_unparseable_file_keeps_the_original_as_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json").unwrap();

        let settings = Settings {
            theme: Theme::Light,
            ..Settings::default()
        };
        settings.save_to(&path).expect("save");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.json.bak")).unwrap(),
            "{not json",
            "the unparseable original is preserved"
        );
        assert_eq!(Settings::load_from(&path).unwrap().theme, Theme::Light);
    }

    #[test]
    fn saving_over_a_valid_file_writes_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        Settings::default().save_to(&path).unwrap();
        Settings::default().save_to(&path).unwrap();

        assert!(!dir.path().join("settings.json.bak").exists());
        assert!(
            !dir.path().join("settings.json.tmp").exists(),
            "the rename consumes the temp file"
        );
    }
}
