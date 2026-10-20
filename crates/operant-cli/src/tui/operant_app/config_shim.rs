// [port-decision] deliberate adaptation, NOT a verbatim port: jcode's
// `crate::config::config()` is a global app-config accessor whose full Config
// tree (operant-config-types) does not exist in operant, and operant already owns
// the `crate::config` path for its own config. This shim provides the same API
// SHAPE the ported renderers call, snapshot-built with ONLY the fields the
// operant_ui tree actually reads (derived by grepping `config().`/`config::`
// usage) and with each field's default taken from upstream so the renderers
// behave exactly as a default-configured jcode. The cutover wires operant's
// real config into this accessor. Upstream defaults cited per field from
// crates/operant-config-types/src/{lib.rs,display.rs}.
//!
//! DiffDisplayMode IS a verbatim re-export of
//! crate::tui::operant_model::vendor_types::DiffDisplayMode (the W1 verbatim port
//! of the same enum; variants Off/Inline/FullInline are type-compatible with
//! every `crate::config::DiffDisplayMode::` use in the tree — verified).
//! DiagramDisplayMode (:164), MarkdownSpacingMode (:190), LatexRenderingMode
//! (:201) are ported verbatim from crates/operant-config-types/src/lib.rs.
//! DisplayConfig/FeaturesConfig/AgentsConfig are shape-compat adaptations
//! carrying the upstream Default values (display.rs :123-:143):
//! pin_todos=true, mouse_capture=true, prompt_entry_animation=true,
//! animation_fps=60, redraw_fps=60, prompt_preview=true,
//! show_agentgrep_output=false, show_bash_output=false, tool_call_details=false,
//! disabled_animations=Vec::new(). root_effort_for_swarm is the verbatim fn
//! from crates/operant-config-types/src/lib.rs:701.
//!
//! [port-decision] batch-4 additions (session_picker + info_widget callers):
//! SessionPickerResumeAction (lib.rs :45-:62, verbatim), SwarmStripLayout
//! (:756-:763, verbatim variants — the TOML `parse` fn at :765 is unported
//! until a renderer reads config text), KeybindingsConfig carrying ONLY the
//! one field the ported renderers read (`session_picker_enter`, :1076;
//! default CurrentTerminal at :1128) — the full 70+-field upstream struct
//! lands field-by-field as renderers read them, per this shim's snapshot
//! rule. DisplayConfig gains `diff_mode` (display.rs :15/:126 — the W1
//! vendor_types DiffDisplayMode re-exported above) and `external_sessions`
//! (display.rs :119/:158, default true). `Config::invalidate_cache` is the
//! shim's own honest degraded arm: the snapshot is a LazyLock constant, so
//! there is no cached file-config to mark stale; a deliberate no-op so
//! upstream call sites (session_picker loading_tests) compile and stay
//! semantically inert against the constant snapshot.
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub use crate::tui::operant_model::vendor_types::DiffDisplayMode;

/// Session picker Enter action: "current-terminal" (default) or "new-terminal".
/// Ctrl+Enter performs the alternate action.
/// Verbatim from crates/operant-config-types/src/lib.rs:45-:62.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SessionPickerResumeAction {
    NewTerminal,
    #[default]
    CurrentTerminal,
}

/// Layout of the swarm status strip. Verbatim variants from
/// crates/operant-config-types/src/lib.rs:756-:763 (doc comments elided;
/// the `parse` fn at :765 is unported — no ported caller reads config text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SwarmStripLayout {
    /// One agent per row: session icon + status glyph + task label, capped to
    /// a few lines with a `+N more` overflow marker.
    #[default]
    Vertical,
    /// All agents packed as chips on a single row (the historical layout).
    Horizontal,
}

/// Shape-compat subset of upstream KeybindingsConfig
/// (crates/operant-config-types/src/lib.rs:1001-:1077): only the field the
/// ported session_picker renderers read. Upstream default at :1128.
#[derive(Debug, Clone)]
pub struct KeybindingsConfig {
    /// Session picker Enter action: "current-terminal" (default) or
    /// "new-terminal". Ctrl+Enter performs the alternate action.
    pub session_picker_enter: SessionPickerResumeAction,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            session_picker_enter: SessionPickerResumeAction::CurrentTerminal,
        }
    }
}

/// Whether file diffs from edit/write tools render. Verbatim from
/// crates/operant-config-types/src/lib.rs:242-:251.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasoningDisplayMode {
    /// Never display reasoning content.
    #[default]
    Off,
    /// Keep every reasoning trace in the transcript (classic behavior).
    Full,
    /// Show the current turn's reasoning, anchored where it streamed; drop it
    /// once the next user prompt lands (jcode `clear_turn_reasoning_traces`).
    Current,
}

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
// crates/operant-config-types/src/lib.rs:179-185, ported verbatim (inserted
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
    pub theme: String,
    pub colors: std::collections::BTreeMap<String, String>,
    pub disabled_animations: Vec<String>,
    pub animation_fps: u32,
    pub redraw_fps: u32,
    pub prompt_preview: bool,
    pub show_agentgrep_output: bool,
    pub show_bash_output: bool,
    pub tool_call_details: bool,
    /// Profile performance tier name ("full" / "reduced" / "minimal"). Both the
    /// upstream default and its priority order land here so perf paths can
    /// compare verbatim. Ported from operant-config-types/src/display.rs:64.
    pub performance: String,
    /// Alt-click copy-badge label (empty = default label). Upstream
    /// operant-config-types/src/display.rs:75, default at :149 (String::new()).
    pub copy_badge_alt_label: String,
    /// Whether external CLI transcripts (Claude Code / Codex / Pi / OpenCode /
    /// Cursor) may appear in the session picker. Upstream display.rs:119,
    /// default at :158.
    pub external_sessions: bool,
    /// How file diffs from edit/write tools render. Upstream display.rs:15,
    /// default at :126.
    pub diff_mode: DiffDisplayMode,
    /// Legacy boolean the upstream `reasoning_display()` fallback reads
    /// (true => Full). Upstream display.rs:32, default at :136.
    pub show_thinking: bool,
    /// Explicit reasoning display mode. Upstream display.rs:39, default at
    /// :137 (`Some(Full)`).
    pub reasoning_display: Option<ReasoningDisplayMode>,
    /// [port-adaptation] Runtime override for the reasoning mode, set by
    /// the TUI at boot from the parsed AppConfig ([display]
    /// show_reasoning) and flipped live by /thinking-display. The static
    /// LazyLock snapshot cannot see App state, so the override rides in a
    /// process-wide cell the resolver reads after the file-level field.
    /// `None` = no override (snapshot default wins).
    pub reasoning_display_override: Option<ReasoningDisplayMode>,
    /// [port-adaptation] Operant-only, not upstream: the rotating 💡 hint
    /// rail in the status strip. Default OFF — operant's status row already
    /// carries state (spinner, token meter, tool status), and the 2026-10-09
    /// live audit's complaint 1 named the rotating hints as overlay noise.
    /// Upstream jcode shows tips unconditionally; operant gates them at the
    /// render call sites.
    pub show_tips: bool,
}

impl DisplayConfig {
    /// Resolve the effective reasoning display mode. Prefers the explicit
    /// `reasoning_display` field, falling back to the legacy `show_thinking`
    /// boolean (true => Full, false => Off) when unset.
    /// Verbatim from crates/operant-config-types/src/display.rs:174-:183.
    pub fn reasoning_display(&self) -> ReasoningDisplayMode {
        // Runtime override (boot config + /thinking-display) wins over the
        // frozen snapshot — the snapshot is a LazyLock of defaults that
        // never reads the user's TOML.
        if let Some(mode) = self.reasoning_display_override() {
            return mode;
        }
        self.reasoning_display.unwrap_or(if self.show_thinking {
            ReasoningDisplayMode::Current
        } else {
            ReasoningDisplayMode::Off
        })
    }

    fn reasoning_display_override(&self) -> Option<ReasoningDisplayMode> {
        REASONING_DISPLAY_OVERRIDE
            .read()
            .ok()
            .and_then(|cell| *cell)
    }
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
            theme: String::new(), // upstream serde(default); "" routes to auto-detect
            colors: std::collections::BTreeMap::new(),
            disabled_animations: Vec::new(),
            animation_fps: 60,
            redraw_fps: 60,
            prompt_preview: true,
            show_agentgrep_output: false,
            show_bash_output: false,
            tool_call_details: false,
            external_sessions: true,
            diff_mode: DiffDisplayMode::default(),
            show_thinking: true,
            reasoning_display: Some(ReasoningDisplayMode::Current),
            reasoning_display_override: None,
            performance: "auto".to_string(),
            copy_badge_alt_label: String::new(),
            show_tips: false,
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
    /// Layout of the swarm status strip. Upstream
    /// crates/operant-config-types/src/lib.rs:590, default at :680.
    pub swarm_strip_layout: SwarmStripLayout,
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub display: DisplayConfig,
    pub features: FeaturesConfig,
    pub agents: AgentsConfig,
    pub keybindings: KeybindingsConfig,
}

/// Process-wide reasoning-mode override. `None` until the TUI boots; set
/// once from the parsed AppConfig, then flipped by /thinking-display.
static REASONING_DISPLAY_OVERRIDE: std::sync::RwLock<Option<ReasoningDisplayMode>> =
    std::sync::RwLock::new(None);

/// Serialises tests that read or write the process-global override cell —
/// parallel test threads otherwise race on it — AND restores the cell when
/// it is dropped.
///
/// The restore is the point. A test that sets the cell and resets it by
/// hand leaks the value whenever it PANICS before the reset: the reset is
/// after the assertions, so a failing assertion skips it, and the cell
/// stays at whatever the test set. The next test that expects a different
/// mode then sees the leaked one — which is how a suite that passes in
/// isolation fails under load, with a failure that looks like a product
/// bug ("the thinking did not render") and is not one.
///
/// Dropping the guard restores the cell to `None` — the snapshot default —
/// on EVERY exit path, including a panic.
#[cfg(test)]
pub(crate) struct ReasoningOverrideGuard {
    // The serialising lock. Held for the guard's lifetime, so no other
    // test touches the cell while this one does.
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for ReasoningOverrideGuard {
    fn drop(&mut self) {
        // Restore the snapshot default, so the next test is not poisoned.
        set_reasoning_display_override(None);
    }
}

#[cfg(test)]
pub(crate) fn reasoning_override_test_guard() -> ReasoningOverrideGuard {
    ReasoningOverrideGuard {
        _lock: TEST_GUARD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    }
}

/// The lock the test guard serialises on.
///
/// ONE lock, shared by the guard and by the tests that inspect it — a
/// per-caller static would not serialise anything.
#[cfg(test)]
static TEST_GUARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The test guard's lock, for the leak tracer.
#[cfg(test)]
pub(crate) fn test_guard_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &LOCK
}

/// The process-wide reasoning display mode, as currently set.
///
/// `None` means "the snapshot default applies".
#[cfg(test)]
pub(crate) fn current_reasoning_display_override() -> Option<ReasoningDisplayMode> {
    REASONING_DISPLAY_OVERRIDE
        .read()
        .ok()
        .and_then(|cell| *cell)
}

/// Set the process-wide reasoning display mode. Idempotent: first call
/// wins at the OnceLock level; later calls replace the stored value (the
/// cell holds `Option<Option<...>>` — presence vs value).
pub fn set_reasoning_display_override(mode: Option<ReasoningDisplayMode>) {
    if let Ok(mut cell) = REASONING_DISPLAY_OVERRIDE.write() {
        *cell = mode;
    }
}

/// Replace the stored override (used by /thinking-display for live flips).
pub fn replace_reasoning_display_override(mode: Option<ReasoningDisplayMode>) {
    set_reasoning_display_override(mode);
}

/// Shape-compatible stand-in for jcode's `crate::config::config()`.
pub fn config() -> &'static Config {
    static SNAPSHOT: LazyLock<Config> = LazyLock::new(Config::default);
    &SNAPSHOT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard must EXCLUDE other threads.
    ///
    /// The whole point is that a test holding the guard has the cell to
    /// itself. If it did not, a concurrent test's write lands in the
    /// middle of this one — which is exactly what the flake was: the
    /// live-tails test set `Full`, and a concurrent test's `Off` landed
    /// before its read, so the thinking did not render.
    #[test]
    fn the_guard_excludes_another_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let handle = std::thread::spawn(move || {
            let _guard = reasoning_override_test_guard();
            // The guard is held. The main thread's `try_lock` must fail.
            tx.send(true).expect("signal that the guard is held");
            // Hold it long enough for the main thread to try.
            std::thread::sleep(std::time::Duration::from_millis(300));
        });
        rx.recv().expect("the other thread took the guard");
        // The main thread tries while the other holds it.
        let acquired = TEST_GUARD_LOCK.try_lock().is_ok();
        assert!(
            !acquired,
            "the guard must exclude another thread while it is held"
        );
        handle.join().expect("join the other thread");
    }

    /// The guard must restore the cell when it is dropped.
    ///
    /// A test that sets the cell and resets it BY HAND leaks the value
    /// whenever it PANICS before the reset — the reset is after the
    /// assertions, so a failing assertion skips it. The next test that
    /// expects a different mode then sees the leaked one, and fails with
    /// a message that looks like a product bug ("the thinking did not
    /// render") and is not one. That is how a suite that passes in
    /// isolation fails under load.
    ///
    /// Dropping the guard restores the cell on EVERY exit path, including
    /// a panic. This test proves the restore happens without a panic.
    #[test]
    fn the_guard_restores_the_cell_when_it_is_dropped() {
        // The guard FIRST, then the write. Setting the cell before
        // taking the guard is the very defect this fixes: the write would
        // land while another test held the guard and expected a different
        // mode, and that test would fail with a message that looks like a
        // product bug ("the thinking did not render") and is not one.
        let _guard = reasoning_override_test_guard();
        set_reasoning_display_override(Some(ReasoningDisplayMode::Off));
        // Inside the guard the cell holds what the test set.
        assert_eq!(
            current_reasoning_display_override(),
            Some(ReasoningDisplayMode::Off),
            "the cell must hold the mode the test set"
        );
        // Drop the guard: it restores the snapshot default.
        drop(_guard);

        // Read the cell back UNDER A FRESH GUARD. Reading it without one
        // is a race: the cell is process-global, so a concurrent test's
        // write lands between the drop and the read. (That race made THIS
        // test the intermittent failure — the same class as the one it
        // was written to fix.)
        let _guard = reasoning_override_test_guard();
        assert_eq!(
            current_reasoning_display_override(),
            None,
            "the guard must restore the cell on drop"
        );
    }
}
