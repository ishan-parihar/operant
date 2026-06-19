use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinColors {
    pub banner_border: String,
    pub banner_title: String,
    pub banner_accent: String,
    pub banner_dim: String,
    pub banner_text: String,
    pub response_border: String,
    #[serde(default)]
    pub ui_accent: String,
    #[serde(default = "default_ui_label")]
    pub ui_label: String,
    #[serde(default = "default_ui_ok")]
    pub ui_ok: String,
    #[serde(default = "default_ui_error")]
    pub ui_error: String,
    #[serde(default = "default_ui_warn")]
    pub ui_warn: String,
    #[serde(default = "default_panel")]
    pub panel: String,
    #[serde(default = "default_panel_alt")]
    pub panel_alt: String,
}

fn default_ui_label() -> String { "#DAA520".into() }
fn default_ui_ok() -> String { "#4caf50".into() }
fn default_ui_error() -> String { "#ef5350".into() }
fn default_ui_warn() -> String { "#ffa726".into() }
fn default_panel() -> String { "#1a1816".into() }
fn default_panel_alt() -> String { "#12110f".into() }

impl Default for SkinColors {
    fn default() -> Self {
        Self {
            banner_border: "#CD7F32".into(),
            banner_title: "#FFD700".into(),
            banner_accent: "#FFBF00".into(),
            banner_dim: "#B8860B".into(),
            banner_text: "#FFF8DC".into(),
            response_border: "#FFD700".into(),
            ui_accent: "#FFBF00".into(),
            ui_label: "#DAA520".into(),
            ui_ok: "#4caf50".into(),
            ui_error: "#ef5350".into(),
            ui_warn: "#ffa726".into(),
            panel: "#1a1816".into(),
            panel_alt: "#12110f".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpinnerConfig {
    #[serde(default)]
    pub waiting_faces: Vec<String>,
    #[serde(default)]
    pub thinking_faces: Vec<String>,
    #[serde(default)]
    pub thinking_verbs: Vec<String>,
    #[serde(default)]
    pub wings: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandingConfig {
    pub agent_name: String,
    pub welcome: String,
    pub response_label: String,
    pub prompt_symbol: String,
}

impl Default for BrandingConfig {
    fn default() -> Self {
        Self {
            agent_name: default_agent_name(),
            welcome: default_welcome(),
            response_label: default_response_label(),
            prompt_symbol: default_prompt_symbol(),
        }
    }
}

fn default_agent_name() -> String { "Hermes Agent".into() }
fn default_welcome() -> String { "Welcome to Hermes Agent! Type your message or /help for commands.".into() }
fn default_response_label() -> String { " \u{2695} Hermes ".into() }
fn default_prompt_symbol() -> String { "\u{276f}".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub colors: SkinColors,
    #[serde(default)]
    pub spinner: SpinnerConfig,
    #[serde(default)]
    pub branding: BrandingConfig,
    #[serde(default = "default_tool_prefix")]
    pub tool_prefix: String,
    #[serde(default)]
    pub tool_emojis: HashMap<String, String>,
}

fn default_tool_prefix() -> String { "\u{250a}".into() }

impl Default for SkinConfig {
    fn default() -> Self {
        Self {
            name: "default".into(),
            description: "Classic Hermes \u{2014} gold and kawaii".into(),
            colors: SkinColors::default(),
            spinner: SpinnerConfig::default(),
            branding: BrandingConfig::default(),
            tool_prefix: default_tool_prefix(),
            tool_emojis: HashMap::new(),
        }
    }
}

impl SkinConfig {
    pub fn parse_color(hex: &str) -> Color {
        let hex = hex.trim().trim_start_matches('#');
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                Color::Rgb(r, g, b)
            }
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0);
                Color::Rgb(r * 17, g * 17, b * 17)
            }
            _ => Color::Reset,
        }
    }

    pub fn color(&self, hex: &str) -> Color { Self::parse_color(hex) }
    pub fn accent(&self) -> Color { self.color(&self.colors.banner_accent) }
    pub fn text(&self) -> Color { self.color(&self.colors.banner_text) }
    pub fn muted(&self) -> Color { self.color(&self.colors.banner_dim) }

    pub fn help(&self) -> Color {
        let base = Self::parse_color(&self.colors.banner_dim);
        match base {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(36).min(255),
                g.saturating_add(36).min(255),
                b.saturating_add(36).min(255),
            ),
            _ => base,
        }
    }

    pub fn success(&self) -> Color { self.color(&self.colors.ui_ok) }
    pub fn error(&self) -> Color { self.color(&self.colors.ui_error) }
    pub fn warn(&self) -> Color { self.color(&self.colors.ui_warn) }
    pub fn panel(&self) -> Color { self.color(&self.colors.panel) }
    pub fn panel_alt(&self) -> Color { self.color(&self.colors.panel_alt) }
    pub fn border(&self) -> Color { self.color(&self.colors.banner_border) }
}

fn builtin_default() -> SkinConfig {
    SkinConfig {
        name: "default".into(),
        description: "Classic Hermes \u{2014} gold and kawaii".into(),
        colors: SkinColors::default(),
        branding: BrandingConfig::default(),
        tool_prefix: "\u{250a}".into(),
        ..Default::default()
    }
}

fn builtin_ares() -> SkinConfig {
    SkinConfig {
        name: "ares".into(),
        description: "War-god theme \u{2014} crimson and bronze".into(),
        colors: SkinColors {
            banner_border: "#9F1C1C".into(),
            banner_title: "#C7A96B".into(),
            banner_accent: "#DD4A3A".into(),
            banner_dim: "#6B1717".into(),
            banner_text: "#F1E6CF".into(),
            response_border: "#C7A96B".into(),
            ui_accent: "#DD4A3A".into(),
            ui_label: "#C7A96B".into(),
            ui_ok: "#7BC96F".into(),
            ui_error: "#EF5350".into(),
            ui_warn: "#C7A96B".into(),
            panel: "#2A1212".into(),
            panel_alt: "#1E0E0E".into(),
        },
        spinner: SpinnerConfig {
            waiting_faces: vec![
                "(\u{2694})".into(),
                "(\u{26E8})".into(),
                "(\u{25B2})".into(),
                "(<>)".into(),
                "(/)".into(),
            ],
            thinking_faces: vec![
                "(\u{2694})".into(),
                "(\u{26E8})".into(),
                "(\u{25B2})".into(),
                "(\u{2301})".into(),
                "(<>)".into(),
            ],
            thinking_verbs: vec![
                "forging".into(),
                "marching".into(),
                "sizing the field".into(),
                "holding the line".into(),
                "hammering plans".into(),
                "tempering steel".into(),
                "plotting impact".into(),
                "raising the shield".into(),
            ],
            wings: vec![
                vec!["\u{27EA}\u{2694}".into(), "\u{2694}\u{27EB}".into()],
                vec!["\u{27EA}\u{25B2}".into(), "\u{25B2}\u{27EB}".into()],
                vec!["\u{27EA}\u{257F}".into(), "\u{257E}\u{27EB}".into()],
                vec!["\u{27EA}\u{26E8}".into(), "\u{26E8}\u{27EB}".into()],
            ],
        },
        branding: BrandingConfig {
            agent_name: "Ares Agent".into(),
            welcome: "Welcome to Ares Agent! Type your message or /help for commands.".into(),
            response_label: " \u{2694} Ares ".into(),
            prompt_symbol: "\u{2694}".into(),
        },
        tool_prefix: "\u{257E}".into(),
        ..Default::default()
    }
}

fn builtin_mono() -> SkinConfig {
    SkinConfig {
        name: "mono".into(),
        description: "Monochrome \u{2014} clean grayscale".into(),
        colors: SkinColors {
            banner_border: "#555555".into(),
            banner_title: "#e6edf3".into(),
            banner_accent: "#aaaaaa".into(),
            banner_dim: "#444444".into(),
            banner_text: "#c9d1d9".into(),
            response_border: "#aaaaaa".into(),
            ui_accent: "#aaaaaa".into(),
            ui_label: "#888888".into(),
            ui_ok: "#888888".into(),
            ui_error: "#cccccc".into(),
            ui_warn: "#999999".into(),
            panel: "#1F1F1F".into(),
            panel_alt: "#161616".into(),
        },
        branding: BrandingConfig {
            agent_name: "Hermes Agent".into(),
            response_label: " \u{2695} Hermes ".into(),
            prompt_symbol: "\u{276f}".into(),
            ..Default::default()
        },
        tool_prefix: "\u{250a}".into(),
        ..Default::default()
    }
}

fn builtin_slate() -> SkinConfig {
    SkinConfig {
        name: "slate".into(),
        description: "Cool blue \u{2014} developer-focused".into(),
        colors: SkinColors {
            banner_border: "#4169e1".into(),
            banner_title: "#7eb8f6".into(),
            banner_accent: "#8EA8FF".into(),
            banner_dim: "#4b5563".into(),
            banner_text: "#c9d1d9".into(),
            response_border: "#7eb8f6".into(),
            ui_accent: "#7eb8f6".into(),
            ui_label: "#8EA8FF".into(),
            ui_ok: "#63D0A6".into(),
            ui_error: "#F7A072".into(),
            ui_warn: "#e6a855".into(),
            panel: "#151C2F".into(),
            panel_alt: "#111725".into(),
        },
        branding: BrandingConfig {
            agent_name: "Hermes Agent".into(),
            response_label: " \u{2695} Hermes ".into(),
            prompt_symbol: "\u{276f}".into(),
            ..Default::default()
        },
        tool_prefix: "\u{250a}".into(),
        ..Default::default()
    }
}

fn builtin_skin(name: &str) -> Option<SkinConfig> {
    match name {
        "default" => Some(builtin_default()),
        "ares" => Some(builtin_ares()),
        "mono" => Some(builtin_mono()),
        "slate" => Some(builtin_slate()),
        _ => None,
    }
}

fn builtin_skin_names() -> &'static [&'static str] {
    &["default", "ares", "mono", "slate"]
}

static ACTIVE_SKIN: OnceLock<Mutex<SkinConfig>> = OnceLock::new();

fn active_skin_lock() -> &'static Mutex<SkinConfig> {
    ACTIVE_SKIN.get_or_init(|| Mutex::new(builtin_default()))
}

pub fn init_skin(theme: &str) {
    let skin = load_skin(theme);
    if let Ok(mut guard) = active_skin_lock().lock() {
        *guard = skin;
    }
}

pub fn get_active() -> SkinConfig {
    active_skin_lock()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| builtin_default())
}

pub fn set_active(name: &str) -> SkinConfig {
    let skin = load_skin(name);
    if let Ok(mut guard) = active_skin_lock().lock() {
        *guard = skin.clone();
    }
    skin
}

pub fn load_skin(name: &str) -> SkinConfig {
    if let Some(user_skin) = load_user_skin(name) {
        return merge_with_default(user_skin);
    }
    if let Some(skin) = builtin_skin(name) {
        return skin;
    }
    builtin_default()
}

pub fn list_skins() -> Vec<SkinInfo> {
    let mut result: Vec<SkinInfo> = builtin_skin_names()
        .iter()
        .map(|name| SkinInfo {
            name: name.to_string(),
            description: builtin_skin(name).map(|s| s.description).unwrap_or_default(),
            source: "builtin".into(),
        })
        .collect();

    if let Some(dir) = user_skins_dir() {
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                        if let Ok(raw) = std::fs::read_to_string(&path) {
                            if let Ok(data) = serde_yaml::from_str::<SkinConfig>(&raw) {
                                let skin_name = data.name.clone();
                                if !result.iter().any(|s| s.name == skin_name) {
                                    result.push(SkinInfo {
                                        name: skin_name,
                                        description: data.description,
                                        source: "user".into(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

#[derive(Debug, Clone)]
pub struct SkinInfo {
    pub name: String,
    pub description: String,
    pub source: String,
}

fn user_skins_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".hermes").join("skins"))
}

fn load_user_skin(name: &str) -> Option<SkinConfig> {
    let dir = user_skins_dir()?;
    let path = dir.join(format!("{}.yaml", name));
    if !path.is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_yaml::from_str::<SkinConfig>(&raw).ok()
}

fn merge_with_default(mut skin: SkinConfig) -> SkinConfig {
    let default = builtin_default();
    if skin.name.is_empty() {
        skin.name = default.name;
    }
    let dc = &default.colors;
    let c = &mut skin.colors;
    if c.banner_border.is_empty() { c.banner_border = dc.banner_border.clone(); }
    if c.banner_title.is_empty() { c.banner_title = dc.banner_title.clone(); }
    if c.banner_accent.is_empty() { c.banner_accent = dc.banner_accent.clone(); }
    if c.banner_dim.is_empty() { c.banner_dim = dc.banner_dim.clone(); }
    if c.banner_text.is_empty() { c.banner_text = dc.banner_text.clone(); }
    if c.response_border.is_empty() { c.response_border = dc.response_border.clone(); }
    if c.ui_accent.is_empty() { c.ui_accent = dc.ui_accent.clone(); }
    if c.ui_label.is_empty() { c.ui_label = dc.ui_label.clone(); }
    if c.ui_ok.is_empty() { c.ui_ok = dc.ui_ok.clone(); }
    if c.ui_error.is_empty() { c.ui_error = dc.ui_error.clone(); }
    if c.ui_warn.is_empty() { c.ui_warn = dc.ui_warn.clone(); }
    if c.panel.is_empty() { c.panel = dc.panel.clone(); }
    if c.panel_alt.is_empty() { c.panel_alt = dc.panel_alt.clone(); }
    if skin.branding.agent_name.is_empty() { skin.branding.agent_name = default.branding.agent_name; }
    if skin.branding.welcome.is_empty() { skin.branding.welcome = default.branding.welcome; }
    if skin.branding.response_label.is_empty() { skin.branding.response_label = default.branding.response_label; }
    if skin.branding.prompt_symbol.is_empty() { skin.branding.prompt_symbol = default.branding.prompt_symbol; }
    if skin.tool_prefix.is_empty() { skin.tool_prefix = default.tool_prefix; }
    skin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_colors() {
        assert_eq!(SkinConfig::parse_color("#FFD700"), Color::Rgb(255, 215, 0));
        assert_eq!(SkinConfig::parse_color("#000000"), Color::Rgb(0, 0, 0));
        assert_eq!(SkinConfig::parse_color("#FFFFFF"), Color::Rgb(255, 255, 255));
        assert_eq!(SkinConfig::parse_color("FFFFFF"), Color::Rgb(255, 255, 255));
        assert_eq!(SkinConfig::parse_color("#FFF"), Color::Rgb(255, 255, 255));
        assert_eq!(SkinConfig::parse_color("#F00"), Color::Rgb(255, 0, 0));
    }

    #[test]
    fn default_skin_has_correct_name() {
        let skin = builtin_default();
        assert_eq!(skin.name, "default");
        assert_eq!(skin.branding.agent_name, "Hermes Agent");
    }

    #[test]
    fn ares_skin_has_custom_spinner() {
        let skin = builtin_ares();
        assert_eq!(skin.name, "ares");
        assert!(!skin.spinner.thinking_verbs.is_empty());
        assert!(!skin.spinner.wings.is_empty());
    }

    #[test]
    fn list_skins_includes_all_builtins() {
        let names: Vec<String> = list_skins().iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"default".to_string()));
        assert!(names.contains(&"ares".to_string()));
        assert!(names.contains(&"mono".to_string()));
        assert!(names.contains(&"slate".to_string()));
    }

    #[test]
    fn load_skin_returns_fallback_for_unknown() {
        let skin = load_skin("nonexistent");
        assert_eq!(skin.name, "default");
    }

    #[test]
    fn skin_engine_roundtrip() {
        let original = set_active("ares");
        assert_eq!(original.name, "ares");
        let active = get_active();
        assert_eq!(active.name, "ares");
        set_active("default");
    }

    #[test]
    fn color_helpers_match_skin_values() {
        let skin = builtin_slate();
        assert_eq!(skin.accent(), Color::Rgb(142, 168, 255));
        assert_eq!(skin.text(), Color::Rgb(201, 209, 217));
    }

    #[test]
    fn user_skin_merge_fills_missing_fields() {
        let partial = SkinConfig {
            name: "test".into(),
            colors: SkinColors {
                banner_border: "#FF0000".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_with_default(partial);
        assert_eq!(merged.name, "test");
        assert_eq!(merged.colors.banner_border, "#FF0000");
        assert_eq!(merged.colors.banner_title, "#FFD700");
        assert_eq!(merged.branding.agent_name, "Hermes Agent");
    }
}
