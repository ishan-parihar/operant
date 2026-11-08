// settings_screen/state.rs — SettingsScreen methods, Default, and entry
// enumeration.
//
// Extracted from the settings_screen.rs monolith.

use super::*;

impl SettingsScreen {
    pub fn new() -> Self {
        let settings_snapshot = Settings::load_sync().unwrap_or_default();
        let mut screen = Self {
            visible: false,
            search_query: String::new(),
            selected_idx: 0,
            edit_field: None,
            edit_value: String::new(),
            settings_snapshot: settings_snapshot.clone(),
            pending_changes: HashMap::new(),
            output_style: "default".to_string(),
            reduce_motion: false,
            show_cwd: true,
            show_git_branch: true,
            compact_threshold: "95".to_string(),
            file_autocomplete_limit: "15".to_string(),
            file_autocomplete_show_hidden_files: false,
        };
        // Apply settings from snapshot immediately on initialization
        screen.apply_settings_from_snapshot();
        screen
    }

    /// Apply all settings from the snapshot to the screen fields.
    /// This is called on initialization and when opening the settings screen.
    fn apply_settings_from_snapshot(&mut self) {
        self.output_style = self
            .settings_snapshot
            .config
            .output_style
            .clone()
            .unwrap_or_else(|| "default".to_string());
        self.reduce_motion = self.settings_snapshot.reduce_motion;
        // (iter-1897) `unwrap_or(true)`: the screen's row shows what the
        // header will actually paint, and an absent key means shown.
        self.show_cwd = self.settings_snapshot.show_working_dir();
        self.show_git_branch = self.settings_snapshot.show_branch();
        self.compact_threshold = self.settings_snapshot.config.compact_threshold.to_string();
        self.file_autocomplete_limit = self
            .settings_snapshot
            .config
            .file_autocomplete_limit
            .to_string();
        self.file_autocomplete_show_hidden_files = self
            .settings_snapshot
            .config
            .file_autocomplete_show_hidden_files;
    }

    pub fn open(&mut self) {
        self.settings_snapshot = Settings::load_sync().unwrap_or_default();
        self.pending_changes.clear();
        self.edit_field = None;
        self.edit_value.clear();
        self.search_query.clear();
        self.selected_idx = 0;
        self.visible = true;

        // Wire real settings from snapshot
        self.apply_settings_from_snapshot();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.edit_field = None;
        self.edit_value.clear();
    }

    pub fn push_search_char(&mut self, c: char) {
        self.search_query.push(c);
        self.selected_idx = 0;
    }

    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        self.selected_idx = 0;
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_next(&mut self, total_visible: usize) {
        if total_visible > 0 && self.selected_idx + 1 < total_visible {
            self.selected_idx += 1;
        }
    }

    /// Start editing a field by name, seeding the buffer with current value.
    pub fn start_edit(&mut self, field: &str, current_value: &str) {
        self.edit_field = Some(field.to_string());
        self.edit_value = current_value.to_string();
    }

    /// Commit the current edit to pending_changes.
    pub fn commit_edit(&mut self) {
        if let Some(field) = self.edit_field.take() {
            let value = std::mem::take(&mut self.edit_value);
            self.pending_changes.insert(field, value);
        }
    }

    /// Discard the current edit.
    pub fn cancel_edit(&mut self) {
        self.edit_field = None;
        self.edit_value.clear();
    }

    /// Apply all pending changes to settings and persist them.
    pub fn apply_and_save(&mut self, config: &mut AppConfig, settings: &mut Settings) {
        for (field, value) in &self.pending_changes {
            match field.as_str() {
                "max_tokens" => {
                    if let Ok(n) = value.parse::<usize>() {
                        config.agent.context_window = n;
                    }
                }
                "output_style" => {
                    settings.output_style = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                }
                "compact_threshold" => {
                    if let Ok(n) = value.parse::<f64>() {
                        config.agent.context_compression_threshold = n;
                        self.compact_threshold = value.clone();
                    }
                }
                "fileAutocompleteLimit" => {
                    if let Ok(n) = value.parse::<usize>() {
                        settings.config.file_autocomplete_limit = n;
                        self.file_autocomplete_limit = value.clone();
                    }
                }
                "fileAutocompleteShowHiddenFiles" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.config.file_autocomplete_show_hidden_files = on;
                    self.file_autocomplete_show_hidden_files = on;
                }
                "reduce_motion" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.reduce_motion = on;
                    self.reduce_motion = on;
                }
                "show_cwd" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.show_cwd = Some(on);
                    self.show_cwd = on;
                }
                "show_git_branch" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.show_git_branch = Some(on);
                    self.show_git_branch = on;
                }
                _ => {}
            }
        }
        // The snapshot is rebuilt wholesale below, and until iter-1176 that
        // rebuild ALSO threw away values the screen does not edit:
        self.settings_snapshot.config = crate::tui::adapter_types::config::InnerConfig {
            file_autocomplete_limit: settings.config.file_autocomplete_limit,
            file_autocomplete_show_hidden_files: settings
                .config
                .file_autocomplete_show_hidden_files,
            output_style: settings.output_style.clone(),
            compact_threshold: config.agent.context_compression_threshold,
            theme: settings.theme.clone(),
            max_tokens: config.agent.context_window,
        };
        self.settings_snapshot.theme = settings.theme.clone();
        self.settings_snapshot.output_style = settings.output_style.clone();
        self.settings_snapshot.effort_level = settings.effort_level.clone();
        self.settings_snapshot.vim_enabled = settings.vim_enabled;

        let _ = self.settings_snapshot.save_sync();
        self.pending_changes.clear();
    }
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Settings entries definition
// ---------------------------------------------------------------------------

pub(crate) fn all_entries(screen: &SettingsScreen) -> Vec<SettingsEntry> {
    let entries = vec![
        SettingsEntry {
            key: "max_tokens",
            label: "Max Tokens",
            description: "Maximum tokens per response.",
            kind: SettingKind::Number,
            value: screen.settings_snapshot.config.max_tokens.to_string(),
        },
        SettingsEntry {
            key: "output_style",
            label: "Output Style",
            description: "Controls the verbosity and format of responses.",
            kind: SettingKind::Enum {
                options: vec!["default", "concise", "explanatory", "learning"],
            },
            value: screen.output_style.clone(),
        },
        SettingsEntry {
            key: "reduce_motion",
            label: "Reduce motion",
            description: "Disable UI animations.",
            kind: SettingKind::Bool,
            value: if screen.reduce_motion {
                "true"
            } else {
                "false"
            }
            .to_string(),
        },
        SettingsEntry {
            key: "show_cwd",
            label: "Show current directory",
            description: "Display the current working directory in the footer.",
            kind: SettingKind::Bool,
            value: if screen.show_cwd { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "show_git_branch",
            label: "Show git branch",
            description: "Display the current git branch in the footer.",
            kind: SettingKind::Bool,
            value: if screen.show_git_branch {
                "true"
            } else {
                "false"
            }
            .to_string(),
        },
        SettingsEntry {
            key: "compact_threshold",
            label: "Auto-compact threshold",
            description: "Context usage % at which to trigger auto-compact (0-100).",
            kind: SettingKind::Number,
            value: screen.compact_threshold.clone(),
        },
        // (iter-1895) The @-autocomplete's own two settings are UNCONDITIONAL
        // now. They used to be pushed only when "File injection (@)" was on —
        // a toggle nothing ever read, so turning it off hid the settings of a
        // feature that kept working exactly as before.
        SettingsEntry {
            key: "fileAutocompleteLimit",
            label: "File autocomplete limit",
            description: "Max suggestions shown in @ autocomplete (type more to narrow results).",
            kind: SettingKind::Number,
            value: screen.file_autocomplete_limit.clone(),
        },
        SettingsEntry {
            key: "fileAutocompleteShowHiddenFiles",
            label: "Show hidden files",
            description: "Include hidden files (.) in @ autocomplete.",
            kind: SettingKind::Bool,
            value: if screen.file_autocomplete_show_hidden_files {
                "true"
            } else {
                "false"
            }
            .to_string(),
        },
    ];

    entries
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------
