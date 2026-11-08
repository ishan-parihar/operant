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
            scroll_offset: 0,
            edit_field: None,
            edit_value: String::new(),
            settings_snapshot: settings_snapshot.clone(),
            pending_changes: HashMap::new(),
            notifications: true,
            output_style: "default".to_string(),
            reduce_motion: false,
            verbose: false,
            cursor_blink_enabled: false,
            show_cwd: true,
            show_git_branch: true,
            compact_threshold: "95".to_string(),
            auto_commits: false,
            output_format: "text".to_string(),
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
        self.notifications = self.settings_snapshot.notifications;
        self.output_style = self
            .settings_snapshot
            .config
            .output_style
            .clone()
            .unwrap_or_else(|| "default".to_string());
        self.reduce_motion = self.settings_snapshot.reduce_motion;
        self.verbose = self.settings_snapshot.config.verbose;
        self.cursor_blink_enabled = self.settings_snapshot.config.cursor_blink_enabled;
        // (iter-1897) `unwrap_or(true)`: the screen's row shows what the
        // header will actually paint, and an absent key means shown.
        self.show_cwd = self.settings_snapshot.show_working_dir();
        self.show_git_branch = self.settings_snapshot.show_branch();
        self.compact_threshold = self.settings_snapshot.config.compact_threshold.to_string();
        self.auto_commits = self.settings_snapshot.config.auto_commits.unwrap_or(false);
        self.output_format = match &self.settings_snapshot.config.output_format {
            crate::tui::adapter_types::config::OutputFormat::Text => "text".to_string(),
            crate::tui::adapter_types::config::OutputFormat::Json => "json".to_string(),
            crate::tui::adapter_types::config::OutputFormat::StreamJson => {
                "stream_json".to_string()
            }
        };
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
        self.scroll_offset = 0;
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
                "notifications" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.notifications = on;
                    self.notifications = on;
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
                "auto_commits" => {
                    settings.config.auto_commits =
                        Some(matches!(value.as_str(), "true" | "on" | "1" | "yes"));
                    self.auto_commits = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                }
                "output_format" => {
                    let parsed = match value.as_str() {
                        "json" => crate::tui::adapter_types::config::OutputFormat::Json,
                        "stream_json" => {
                            crate::tui::adapter_types::config::OutputFormat::StreamJson
                        }
                        _ => crate::tui::adapter_types::config::OutputFormat::Text,
                    };
                    settings.config.output_format = parsed.clone();
                    self.output_format = value.clone();
                }
                "cursor_blink_enabled" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.config.cursor_blink_enabled = on;
                    self.cursor_blink_enabled = on;
                }
                "verbose" => {
                    let on = matches!(value.as_str(), "true" | "on" | "1" | "yes");
                    settings.config.verbose = on;
                    self.verbose = on;
                }
                _ => {}
            }
        }
        // The snapshot is rebuilt wholesale below, and until iter-1176 that
        // rebuild ALSO threw away values the screen does not edit:
        // `verbose`, `cursor_blink_enabled` and `output_format` were written
        // as constants instead of carried
        // forward, so saving ANY setting reset them. Every field below is now
        // taken from `settings`, which this loop has just updated with
        // whatever the operator changed.
        self.settings_snapshot.config = crate::tui::adapter_types::config::InnerConfig {
            verbose: settings.config.verbose,
            cursor_blink_enabled: settings.config.cursor_blink_enabled,
            auto_commits: settings.config.auto_commits,
            file_autocomplete_limit: settings.config.file_autocomplete_limit,
            file_autocomplete_show_hidden_files: settings
                .config
                .file_autocomplete_show_hidden_files,
            output_style: settings.output_style.clone(),
            output_format: settings.config.output_format.clone(),
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
            key: "notifications",
            label: "Desktop notifications",
            description: "Notify when a turn completes.",
            kind: SettingKind::Bool,
            value: if screen.notifications {
                "true"
            } else {
                "false"
            }
            .to_string(),
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
            key: "verbose",
            label: "Verbose logging",
            description: "Log additional debug information. Takes effect on next session.",
            kind: SettingKind::Bool,
            value: if screen.verbose { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "cursor_blink_enabled",
            label: "Cursor blinking",
            description: "Enable cursor blinking in the chat prompt.",
            kind: SettingKind::Bool,
            value: if screen.cursor_blink_enabled {
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
        SettingsEntry {
            key: "auto_commits",
            label: "Auto-commits",
            description: "Automatically snapshot changes to git via shadow-git.",
            kind: SettingKind::Bool,
            value: if screen.auto_commits { "true" } else { "false" }.to_string(),
        },
        SettingsEntry {
            key: "output_format",
            label: "Output format",
            description: "How responses are formatted: text, JSON, or streaming JSON.",
            kind: SettingKind::Enum {
                options: vec!["text", "json", "streamjson"],
            },
            value: screen.output_format.clone(),
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
