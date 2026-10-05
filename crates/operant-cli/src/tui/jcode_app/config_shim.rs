// [port-decision] deliberate adaptation, NOT a verbatim port: jcode's
// `crate::config::config()` is a global app-config accessor whose full Config
// tree (jcode-config-types) does not exist in operant, and operant already owns
// the `crate::config` path for its own config. This shim provides the same API
// SHAPE the ported renderers call, snapshot-built with ONLY the fields the
// jcode_ui tree actually reads (derived by grepping `config().`/`config::`
// usage) and with each field's default taken from upstream so the renderers
// behave exactly as a default-configured jcode. The cutover wires operant's
// real config into this accessor. Upstream defaults cited per field from
// crates/jcode-config-types/src/{lib.rs,display.rs}.
//!
//! DiffDisplayMode IS a verbatim re-export of
//! crate::tui::jcode_model::vendor_types::DiffDisplayMode (the W1 verbatim port
//! of the same enum; variants Off/Inline/FullInline are type-compatible with
//! every `crate::config::DiffDisplayMode::` use in the tree — verified).
//! DiagramDisplayMode (:164), MarkdownSpacingMode (:190), LatexRenderingMode
//! (:201) are ported verbatim from crates/jcode-config-types/src/lib.rs.
//! DisplayConfig/FeaturesConfig/AgentsConfig are shape-compat adaptations
//! carrying the upstream Default values (display.rs :123-:143):
//! pin_todos=true, mouse_capture=true, prompt_entry_animation=true,
//! animation_fps=60, redraw_fps=60, prompt_preview=true,
//! show_agentgrep_output=false, show_bash_output=false, tool_call_details=false,
//! disabled_animations=Vec::new(). root_effort_for_swarm is the verbatim fn
//! from crates/jcode-config-types/src/lib.rs:701.
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub use crate::tui::jcode_model::vendor_types::DiffDisplayMode;

/// How to display mermaid diagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagramDisplayMode {
    /// Don't show diagrams in dedicated widgets (only inline in messages).
    ///
    /// `inline`/`off` are accepted spellings: diagrams still render inline in
    /// the transcript in this mode, and users reasonably write `"inline"`
    /// (issue #689).
    #[default]
    #[serde(alias = "inline", alias = "off")]
    None,
    /// Show diagrams in info widget margins (opportunistic, if space available).
    Margin,
    /// Show diagrams in a dedicated pinned pane (forces space allocation).
    Pinned,
}

// [port-decision] added at batch-3: TuiState trait dependency
// (tui_state.rs `diagram_pane_position`); upstream
// crates/jcode-config-types/src/lib.rs:179-185, ported verbatim (inserted
// between DiagramDisplayMode and MarkdownSpacingMode to mirror upstream
// order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagramPanePosition {
    #[default]
    Side,
    Top,
}

/// How much vertical spacing to use when rendering markdown blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownSpacingMode {
    /// Compact chat/TUI-oriented spacing.
    #[default]
    Compact,
    /// Document-style spacing between top-level blocks.
    Document,
}

/// How LaTeX math is rendered in terminal markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LatexRenderingMode {
    /// Preserve the original LaTeX source and delimiters.
    None,
    /// Convert supported notation to terminal-friendly Unicode text.
    Unicode,
    /// Typeset formulas to PNG and display them with the terminal image protocol.
    #[default]
    Image,
}

impl AgentsConfig {
    /// Resolve a swarm mode's root effort without allowing orchestration
    /// sentinels to recurse into another mode. Unknown values preserve the
    /// historical maximum-effort behavior without invalidating other settings.
    /// Resolve a swarm mode's root effort without allowing orchestration
    /// sentinels to recurse into another mode. Unknown values preserve the
    /// historical maximum-effort behavior without invalidating other settings.
    pub fn root_effort_for_swarm(&self, deep: bool) -> &'static str {
        let configured = if deep {
            self.swarm_deep_root_effort.as_deref()
        } else {
            self.swarm_root_effort.as_deref()
        };
        let value = configured.unwrap_or("max").trim();
        ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
            .into_iter()
            .find(|level| level.eq_ignore_ascii_case(value))
            .unwrap_or("max")
    }
}

// [port-decision] dedup: derive(Default) collided with the manual impl below;
// dropped Default from the derive, kept the manual impl (carries real values).
#[derive(Debug, Clone)]
pub struct DisplayConfig {
    pub pin_todos: bool,
    pub mouse_capture: bool,
    pub diagram_mode: DiagramDisplayMode,
    pub markdown_spacing: MarkdownSpacingMode,
    pub latex_rendering: LatexRenderingMode,
    pub prompt_entry_animation: bool,
    pub disabled_animations: Vec<String>,
    pub animation_fps: u32,
    pub redraw_fps: u32,
    pub prompt_preview: bool,
    pub show_agentgrep_output: bool,
    pub show_bash_output: bool,
    pub tool_call_details: bool,
    /// Profile performance tier name ("full" / "reduced" / "minimal"). Both the
    /// upstream default and its priority order land here so perf paths can
    /// compare verbatim. Ported from jcode-config-types/src/display.rs:64.
    pub performance: String,
    /// Alt-click copy-badge label (empty = default label). Upstream
    /// jcode-config-types/src/display.rs:75, default at :149 (String::new()).
    pub copy_badge_alt_label: String,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            pin_todos: true,
            mouse_capture: true,
            diagram_mode: DiagramDisplayMode::default(),
            markdown_spacing: MarkdownSpacingMode::default(),
            latex_rendering: LatexRenderingMode::default(),
            prompt_entry_animation: true,
            disabled_animations: Vec::new(),
            animation_fps: 60,
            redraw_fps: 60,
            prompt_preview: true,
            show_agentgrep_output: false,
            show_bash_output: false,
            tool_call_details: false,
            performance: "auto".to_string(),
            copy_badge_alt_label: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeaturesConfig {
    pub mermaid: bool,
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self { mermaid: true }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentsConfig {
    pub swarm_root_effort: Option<String>,
    pub swarm_deep_root_effort: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub display: DisplayConfig,
    pub features: FeaturesConfig,
    pub agents: AgentsConfig,
}

/// Shape-compatible stand-in for jcode's `crate::config::config()`.
pub fn config() -> &'static Config {
    static SNAPSHOT: LazyLock<Config> = LazyLock::new(Config::default);
    &SNAPSHOT
}
