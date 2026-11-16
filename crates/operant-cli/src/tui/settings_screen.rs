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

/// Which tab a row belongs to.
///
/// (plan 2026-11-16 S2) The panel drew 276 rows in a flat list with no headings,
/// which is the operator's actual complaint about this pane. OMP's selector has
/// ten tabs (`tui/overlays/settings-defs.ts:22-32`); these are the ones operant
/// has settings for, derived from the registry's own key prefixes rather than
/// from a hand-kept list that drifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    /// `[tui]` — colours, bar style, composer.
    Appearance,
    /// `[agent]` — the model, context window, effort.
    Model,
    /// `[context]`, `[context_injection]` — what the model sees.
    Context,
    /// `[tools]`, `[browser]` — the tool surface and its limits.
    Tools,
    /// `[skills]`, `[mcp]`, `[plugin]` — what the agent can call.
    Skills,
    /// `[memory]` — cross-session memory.
    Memory,
    /// `[gateway]` — ports, posture, remote mode.
    Gateway,
    /// Everything the tabs above do not claim.
    Advanced,
}

impl SettingsTab {
    /// The stable order the strip renders in. OMP orders its tabs explicitly
    /// for the same reason (`SETTING_TABS`, `settings-defs.ts:22-32`).
    pub const ALL: [SettingsTab; 8] = [
        SettingsTab::Appearance,
        SettingsTab::Model,
        SettingsTab::Context,
        SettingsTab::Tools,
        SettingsTab::Skills,
        SettingsTab::Memory,
        SettingsTab::Gateway,
        SettingsTab::Advanced,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::Appearance => "Appearance",
            SettingsTab::Model => "Model",
            SettingsTab::Context => "Context",
            SettingsTab::Tools => "Tools",
            SettingsTab::Skills => "Skills",
            SettingsTab::Memory => "Memory",
            SettingsTab::Gateway => "Gateway",
            SettingsTab::Advanced => "Advanced",
        }
    }
}

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
    /// Which tab this row lives under. Every row names one; there is no default,
    /// because a row that silently lands in "Advanced" is a row nobody finds
    /// (red-team R5).
    pub tab: SettingsTab,
    /// The heading row this row sits under, inside its tab. `None` renders at
    /// the top of the tab, which is OMP's rule for ungrouped settings
    /// (`settings-defs.ts:113-115`).
    pub group: Option<&'static str>,
    /// A risk note rendered above the description, in warning styling. OMP
    /// reserves this for settings that can get the operator rate-limited or
    /// banned, not for merely advanced options (`settings-defs.ts:117-121`).
    pub warning: Option<&'static str>,
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
    /// Which tab the operator is looking at. The selection index is into
    /// `visible_entries()`, NOT into the whole panel, so switching tabs must
    /// clamp it — `select_tab` does.
    pub selected_tab: SettingsTab,
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

impl SettingsScreen {
    /// The rows the operator can actually reach right now: the selected tab,
    /// then the search query, in registry order.
    ///
    /// (plan 2026-11-16 S2) THE ONE FILTER. Four call sites had their own copy
    /// of `all_entries().filter(label.contains(query))` — the list, the
    /// description pane, `select_next_visible` and `toggle_or_cycle_current`. The
    /// keyboard counted one number and the painter sliced with another, which is
    /// the iter-1917 defect exactly: two windows, two numbers. Every reader now
    /// comes here, so the tab and the query narrow one list and one only.
    pub fn visible_entries(&self) -> Vec<SettingsEntry> {
        // OWNED, not borrowed: `all_entries` builds its rows fresh on every call,
        // so there is no long-lived vector to borrow from — returning references
        // would dangle. The caller gets its own copy of the visible subset, which
        // also ends the borrow before the apply arms take `&mut self`.
        all_entries(self)
            .into_iter()
            .filter(|entry| entry.tab == self.selected_tab)
            .filter(|entry| {
                self.search_query.is_empty()
                    || entry
                        .label
                        .to_lowercase()
                        .contains(&self.search_query.to_lowercase())
            })
            .collect()
    }

    /// Point the selection at a tab, clamped to what that tab actually holds.
    ///
    /// A tab with no rows must leave the selection at 0 rather than past the
    /// end, or the marker vanishes and the description pane describes nothing —
    /// the same "walked past the last painted row" failure iter-1917 fixed.
    pub fn select_tab(&mut self, tab: SettingsTab) {
        self.selected_tab = tab;
        self.selected_idx = 0;
        self.edit_field = None;
    }
}

mod keys;
mod render;
mod state;
mod tabs;

#[cfg(test)]
mod tests;

pub use keys::handle_settings_key;
pub(crate) use keys::{select_next_visible, select_prev_visible};
pub use render::render_settings_screen;
pub(crate) use state::all_entries;
pub(crate) use tabs::{group_for, tab_for};
