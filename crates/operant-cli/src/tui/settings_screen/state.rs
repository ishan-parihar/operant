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
            auto_compact: false,
            notifications: true,
            show_turn_duration: false,
            output_style: "default".to_string(),
            reduce_motion: false,
            terminal_progress_bar: true,
            verbose: false,
            cursor_blink_enabled: false,
            auto_copy_enabled: false,
            show_cwd: false,
            show_git_branch: false,
            compact_threshold: "95".to_string(),
            auto_commits: false,
            output_format: "text".to_string(),
            disable_claude_mds: false,
            file_injection_enabled: true,
            file_autocomplete_limit: "15".to_string(),
            file_autocomplete_show_hidden_files: false,
            file_injection_max_size: "100".to_string(),
        };
        // Apply settings from snapshot immediately on initialization
        screen.apply_settings_from_snapshot();
        screen
    }

    /// Apply all settings from the snapshot to the screen fields.
    /// This is called on initialization and when opening the settings screen.
    fn apply_settings_from_snapshot(&mut self) {
        self.auto_compact = self.settings_snapshot.auto_compact;
        self.notifications = self.settings_snapshot.notifications;
        self.show_turn_duration = self.settings_snapshot.show_turn_duration;
        self.output_style = self
            .settings_snapshot
            .config
            .output_style
            .clone()
            .unwrap_or_else(|| "default".to_string());
        self.reduce_motion = self.settings_snapshot.reduce_motion;
        self.terminal_progress_bar = self.settings_snapshot.terminal_progress_bar;
        self.verbose = self.settings_snapshot.config.verbose;
        self.cursor_blink_enabled = self.settings_snapshot.config.cursor_blink_enabled;
        self.auto_copy_enabled = self.settings_snapshot.auto_copy_on_highlight;
        self.show_cwd = self.settings_snapshot.show_cwd;
        self.show_git_branch = self.settings_snapshot.show_git_branch;
        self.compact_threshold = self.settings_snapshot.config.compact_threshold.to_string();
        self.auto_commits = self.settings_snapshot.config.auto_commits.unwrap_or(false);
        self.output_format = match &self.settings_snapshot.config.output_format {
            crate::tui::adapter_types::config::OutputFormat::Text => "text".to_string(),
            crate::tui::adapter_types::config::OutputFormat::Json => "json".to_string(),
            crate::tui::adapter_types::config::OutputFormat::StreamJson => {
                "stream_json".to_string()
            }
        };
        self.disable_claude_mds = self.settings_snapshot.config.disable_claude_mds;
        self.file_injection_enabled = self.settings_snapshot.config.file_injection_enabled;
        self.file_autocomplete_limit = self
            .settings_snapshot
            .config
            .file_autocomplete_limit
            .to_string();
        self.file_autocomplete_show_hidden_files = self
            .settings_snapshot
            .config
            .file_autocomplete_show_hidden_files;
        self.file_injection_max_size = self
            .settings_snapshot
            .config
            .file_injection_max_size
            .to_string();
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
                "fileInjectionMaxSize" => {
                    if let Ok(n) = value.parse::<usize>() {
                        settings.config.file_injection_max_size = n;
                        self.file_injection_max_size = value.clone();
                    }
                }
                _ => {}
            }
        }
        self.settings_snapshot.config = crate::tui::adapter_types::config::InnerConfig {
            verbose: false,
            cursor_blink_enabled: false,
            auto_commits: None,
            disable_claude_mds: false,
            file_injection_enabled: false,
            file_autocomplete_limit: settings.config.file_autocomplete_limit,
            file_autocomplete_show_hidden_files: settings
                .config
                .file_autocomplete_show_hidden_files,
            file_injection_max_size: settings.config.file_injection_max_size,
            output_style: settings.output_style.clone(),
            output_format: crate::tui::adapter_types::config::OutputFormat::default(),
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
    let mut entries = vec![
        SettingsEntry {
            key: "max_tokens",
            label: "Max Tokens",
            description: "Maximum tokens per response.",
            kind: SettingKind::Number,
            value: screen.settings_snapshot.config.max_tokens.to_string(),
        },
        SettingsEntry {
            key: "auto_compact",
            label: "Auto-compact",
            description: "Automatically compact turns at threshold.",
            kind: SettingKind::Bool,
            value: if screen.auto_compact { "true" } else { "false" }.to_string(),
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
            key: "show_turn_duration",
            label: "Show turn duration",
            description: "Display elapsed time per turn in status bar.",
            kind: SettingKind::Bool,
            value: if screen.show_turn_duration {
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
            key: "terminal_progress_bar",
            label: "Terminal progress bar",
            description: "Show progress during tool use.",
            kind: SettingKind::Bool,
            value: if screen.terminal_progress_bar {
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
            key: "auto_copy_enabled",
            label: "Auto-copy on highlight",
            description: "Automatically copy highlighted text to clipboard.",
            kind: SettingKind::Bool,
            value: if screen.auto_copy_enabled {
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
        SettingsEntry {
            key: "disable_claude_mds",
            label: "Disable CLAUDE.md",
            description: "Ignore CLAUDE.md files in projects (use defaults instead).",
            kind: SettingKind::Bool,
            value: if screen.disable_claude_mds {
                "true"
            } else {
                "false"
            }
            .to_string(),
        },
        SettingsEntry {
            key: "fileInjectionEnabled",
            label: "File injection (@)",
            description: "Auto-inject @file references into message context.",
            kind: SettingKind::Bool,
            value: if screen.file_injection_enabled {
                "true"
            } else {
                "false"
            }
            .to_string(),
        },
    ];

    // Only show these if file injection is enabled
    if screen.file_injection_enabled {
        entries.push(SettingsEntry {
            key: "fileAutocompleteLimit",
            label: "File autocomplete limit",
            description: "Max suggestions shown in @ autocomplete (type more to narrow results).",
            kind: SettingKind::Number,
            value: screen.file_autocomplete_limit.clone(),
        });
        entries.push(SettingsEntry {
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
        });
        entries.push(SettingsEntry {
            key: "fileInjectionMaxSize",
            label: "File injection max size",
            description: "Max file size to auto-inject (KB, 0=no limit).",
            kind: SettingKind::Number,
            value: screen.file_injection_max_size.clone(),
        });
    }

    entries
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------
