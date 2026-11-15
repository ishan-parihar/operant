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
    Enum {
        options: Vec<&'static str>,
    },
    Number,
    /// Free text, parsed as the field's own type.
    Text,
    /// A key the config parses and no code reads. Rendered greyed, and never
    /// written: a control that does nothing is the phantom surface the
    /// registry's `live` flag exists to keep out of the panel.
    Unread,
}

#[derive(Debug, Clone)]
pub struct SettingsEntry {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: SettingKind,
    pub value: String,
    /// Which store this row writes. The TUI's own visual state lives in
    /// `settings.json` and must not leak into a shared `operant.toml`; every
    /// other key is a config key. Carried on the row so no arm has to know.
    pub store: SettingStore,
}

/// Where a row's value is persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingStore {
    /// `settings.json` — the TUI's own per-profile visual state.
    SettingsJson,
    /// `operant.toml`, through `persist_config_key`.
    Config,
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
    /// Why the last apply could not take effect — the settings screen accepted
    /// the operator's edit and then silently discarded it, which is the same
    /// "an unread argument made that a lie" class /rewind had (iter-1947).
    pub apply_error: Option<String>,
    /// A value that WAS applied but could not be persisted — a different fact
    /// from `apply_error`, which means the value was never taken.
    pub persist_warning: Option<String>,

    // ---- Real settings fields ----
    pub output_style: String,
    /// Which glyph draws the boundary between two status-line segments.
    ///
    /// Mirrors `[tui] bar_style` rather than living in `settings.json`: the
    /// status line is the bar's only consumer and the config key already owns
    /// the value. Before this row existed that key's only control was a
    /// hand-edited TOML line, so the operator had eight styles and no way to
    /// reach one (plan 2026-11-15 U1).
    pub bar_style: String,
    /// The bar styles `SeparatorStyle::ALL` offers, cached so a frame does not
    /// rebuild the list per row.
    pub bar_style_options: Vec<&'static str>,
    /// The live `operant.toml` values, read once per open. A row the panel
    /// draws from the registry reads its current value from here, so the panel
    /// never reads the file per frame.
    pub config_values: std::collections::HashMap<&'static str, String>,
    pub reduce_motion: bool,
    pub show_cwd: bool,
    pub show_git_branch: bool,
    pub file_autocomplete_limit: String,
    pub file_autocomplete_show_hidden_files: bool,
    /// Authoritative context capacity, seeded from the live AppConfig.
    pub context_window: usize,
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
