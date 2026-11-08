// settings_screen.rs — Flat searchable settings interface.
//
// Opened by /config or /settings commands. Shows all editable settings
// in a single scrollable list with live search filtering.
// Changes are persisted via Settings::save_sync() or settings.json writes.

use crate::tui::adapter_types::config::Settings;
use crate::tui::adapter_types::output_styles::{builtin_styles, find_style};
use crate::tui::overlays::{modal_search_line, render_dark_overlay, render_dialog_bg};
use crate::tui::theme_colors;
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
    /// Which field is being edited (field name as key).
    pub edit_field: Option<String>,
    /// Current buffer content while editing a field.
    pub edit_value: String,
    /// Snapshot of settings at open time.
    pub settings_snapshot: Settings,
    /// Pending changes (field_name → new_value string).
    pub pending_changes: HashMap<String, String>,

    // ---- Real settings fields ----
    pub output_style: String,
    pub reduce_motion: bool,
    pub show_cwd: bool,
    pub show_git_branch: bool,
    pub compact_threshold: String,
    pub file_autocomplete_limit: String,
    pub file_autocomplete_show_hidden_files: bool,
}

mod keys;
mod render;
mod state;

#[cfg(test)]
mod tests;

pub use keys::handle_settings_key;
pub(crate) use keys::{select_next_visible, select_prev_visible};
pub use render::render_settings_screen;
pub(crate) use state::all_entries;
