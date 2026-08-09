// settings_screen.rs — Flat searchable settings interface.
//
// Opened by /config or /settings commands. Shows all editable settings
// in a single scrollable list with live search filtering.
// Changes are persisted via Settings::save_sync() or settings.json writes.

use crate::tui::adapter_types::config::Settings;
use crate::tui::adapter_types::output_styles::{builtin_styles, find_style};
use crate::tui::overlays::{
    OPERANT_ACCENT, OPERANT_MUTED, OPERANT_PANEL_BG, centered_rect, modal_search_line,
    render_dark_overlay, render_dialog_bg,
};
use operant_core::config::AppConfig;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SettingKind {
    Bool,
    Enum { options: Vec<&'static str> },
    Number,
}

#[derive(Debug, Clone)]
pub struct SettingsEntry {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: SettingKind,
    pub value: String,
}

pub struct SettingsScreen {
    pub visible: bool,
    pub search_query: String,
    pub selected_idx: usize,
    pub scroll_offset: usize,
    /// Which field is being edited (field name as key).
    pub edit_field: Option<String>,
    /// Current buffer content while editing a field.
    pub edit_value: String,
    /// Snapshot of settings at open time.
    pub settings_snapshot: Settings,
    /// Pending changes (field_name → new_value string).
    pub pending_changes: HashMap<String, String>,

    // ---- Real settings fields ----
    pub auto_compact: bool,
    pub notifications: bool,
    pub show_turn_duration: bool,
    pub output_style: String,
    pub reduce_motion: bool,
    pub terminal_progress_bar: bool,
    pub verbose: bool,
    pub cursor_blink_enabled: bool,
    pub auto_copy_enabled: bool,
    pub show_cwd: bool,
    pub show_git_branch: bool,
    pub compact_threshold: String,
    pub auto_commits: bool,
    pub output_format: String,
    pub disable_claude_mds: bool,
    pub file_injection_enabled: bool,
    pub file_autocomplete_limit: String,
    pub file_autocomplete_show_hidden_files: bool,
    pub file_injection_max_size: String,
}

mod keys;
mod render;
mod state;

#[cfg(test)]
mod tests;

pub use keys::handle_settings_key;
pub use render::render_settings_screen;
pub(crate) use state::all_entries;
