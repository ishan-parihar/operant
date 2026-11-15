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
            apply_error: None,
            persist_warning: None,
            output_style: "default".to_string(),
            // The live cell, not the config: `tui_app` installs `[tui]
            // bar_style` before `App::new` builds this screen, so this reads
            // back what the status line is actually drawing. Reading
            // `runtime_config()` instead would read `AppConfig::default()` —
            // nothing in the tree ever loads a real config into it — and the
            // row would claim `dot` while the bar drew powerline.
            bar_style: crate::tui::status_line::style::style().key().to_string(),
            bar_style_options: crate::tui::status_line::style::SeparatorStyle::ALL
                .iter()
                .map(|s| s.key())
                .collect(),
            config_values: std::collections::HashMap::new(),
            reduce_motion: false,
            show_cwd: true,
            show_git_branch: true,
            file_autocomplete_limit: "15".to_string(),
            file_autocomplete_show_hidden_files: false,
            context_window: operant_core::config::runtime_config().agent.context_window,
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

    /// The current value of a CONFIG key, as the panel shows it.
    ///
    /// Read from the snapshot taken at open, not from the file per frame. A key
    /// with no recorded value renders empty rather than guessing a default: the
    /// registry's default column is source text (`<T>::default()`), not a value.
    pub(crate) fn config_value(&self, key: &str) -> String {
        self.config_values.get(key).cloned().unwrap_or_default()
    }

    pub fn open(&mut self, context_window: usize, config: &operant_core::config::AppConfig) {
        self.settings_snapshot = Settings::load_sync().unwrap_or_default();
        self.context_window = context_window;
        self.pending_changes.clear();
        self.edit_field = None;
        self.edit_value.clear();
        self.search_query.clear();
        self.selected_idx = 0;
        self.visible = true;

        // Wire real settings from snapshot
        self.apply_settings_from_snapshot();
        // The bar style is not a `settings.json` key — the TOML owns it and
        // the status line is its only reader — so the row re-reads the live
        // cell rather than a snapshot that never had it. That also picks up a
        // style chosen earlier in this session.
        self.bar_style = crate::tui::status_line::style::style().key().to_string();
        // Every CONFIG row reads its value from the live runtime config, taken
        // once at open. Reading the file per frame would put a syscall in the
        // render path, and reading it once per open is what makes the panel show
        // what the process is actually running rather than what is on disk.
        self.config_values = operant_core::config_registry::SETTING_ROWS
            .iter()
            .filter(|row| row.live)
            .map(|row| (row.key, read_dotted_config(config, row.key)))
            .collect();
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

    /// Apply one bar-style value to the live status line, the config and the
    /// TOML. The ONE writer for this setting, so the cycle row and the edit
    /// commit cannot disagree about what "applied" means.
    ///
    /// An unknown value changes nothing and is reported: a settings row that
    /// silently fell back to `dot` would leave the operator believing they had
    /// chosen a style. The live value is set BEFORE the persist is attempted
    /// and the persist is reported rather than gated, matching
    /// `context_window`'s reasoning — a config with no file on disk still gets
    /// the style for this session.
    pub(super) fn apply_bar_style(
        &mut self,
        config: &mut AppConfig,
        value: &str,
    ) -> Result<(), String> {
        let (style, known) = crate::tui::status_line::style::SeparatorStyle::parse(value);
        if !known {
            return Err(format!(
                "Status bar style \"{value}\" - expected one of: {}",
                crate::tui::status_line::style::SeparatorStyle::ALL
                    .iter()
                    .map(|s| s.key())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        config.tui.bar_style = style.key().to_string();
        self.bar_style = style.key().to_string();
        crate::tui::status_line::style::set_style(Some(style));
        let persisted = config.config_path.as_ref().map_or_else(
            || {
                Err(operant_core::error::Error::Config(
                    "the loaded config has no recorded source path".to_string(),
                ))
            },
            |path| operant_core::config::persist_config_key_at(path, "tui.bar_style", style.key()),
        );
        if let Err(e) = persisted {
            self.persist_warning = Some(format!(
                "Status bar style \"{}\" - applied for this session, but not persisted: {e}",
                style.key()
            ));
        }
        Ok(())
    }

    /// Apply all pending changes to settings and persist them.
    pub fn apply_and_save(&mut self, config: &mut AppConfig, settings: &mut Settings) {
        // (iter-1948) A new apply supersedes the previous verdict.
        self.apply_error = None;
        self.persist_warning = None;
        let mut unread: Vec<String> = Vec::new();
        // Cloned so the loop can call a `&mut self` writer (`apply_bar_style`)
        // without fighting the borrow of `pending_changes`. Same shape as
        // `theme_colors::apply_role_overrides`, which clones the override list
        // for the identical reason.
        let pending: Vec<(String, String)> = self
            .pending_changes
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (field, value) in &pending {
            let value: String = value.clone();
            match field.as_str() {
                "agent.context_window" => {
                    if let Ok(n) = value.parse::<usize>() {
                        if let Err(error) = operant_core::config::validate_context_window(n) {
                            unread.push(format!("Context window \"{value}\" ({error})"));
                            continue;
                        }
                        // The live value is applied FIRST and the persist is
                        // reported, not gated: an environment that loaded no
                        // config file makes persistence impossible, and
                        // withholding the live change would turn a persistence
                        // gap into a setting that does nothing at all.
                        config.agent.context_window = n;
                        self.context_window = n;
                        let persisted = config.config_path.as_ref().map_or_else(
                            || {
                                Err(operant_core::error::Error::Config(
                                    "the loaded config has no recorded source path".to_string(),
                                ))
                            },
                            |path| {
                                operant_core::config::persist_config_key_at(
                                    path,
                                    "agent.context_window",
                                    &n.to_string(),
                                )
                            },
                        );
                        if let Err(e) = persisted {
                            // A value that WAS applied but will not survive a
                            // restart is a warning, not a rejection: the
                            // operator's edit took effect, and the part they
                            // cannot see is the persistence.
                            self.persist_warning = Some(format!(
                                "Context window \"{value}\" - applied for this session, but not persisted: {e}"
                            ));
                        }
                    } else {
                        unread.push(format!("Context window \"{value}\""));
                    }
                }
                "output_style" => {
                    settings.output_style = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                }
                // (plan 2026-11-15 U1) The bar style is a `[tui]` key, not a
                // `settings.json` one, so it takes the same shape as
                // `context_window` above: apply live, then report the persist
                // rather than gate on it — an environment with no config file
                // still gets the style the operator picked.
                "tui.bar_style" => {
                    if let Err(message) = self.apply_bar_style(config, &value) {
                        unread.push(message);
                        continue;
                    }
                }
                "fileAutocompleteLimit" => {
                    if let Ok(n) = value.parse::<usize>() {
                        settings.config.file_autocomplete_limit = n;
                        self.file_autocomplete_limit = value.clone();
                    } else {
                        unread.push(format!("File autocomplete limit \"{value}\""));
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
                // Every CONFIG key the panel draws goes through the one write
                // seam. `persist_config_key` is `toml_edit`-based, so it keeps
                // the file's comments and its formatting, and it is atomic.
                // A row whose key is not a known config key is refused rather
                // than written somewhere the loader will not read it.
                _ => {
                    if !operant_core::config_registry::KNOWN_KEYS.contains(&field.as_str()) {
                        unread.push(format!("{field}"));
                        continue;
                    }
                    if let Err(error) =
                        operant_core::config::persist_config_key(field, value.as_str())
                    {
                        self.persist_warning =
                            Some(format!("{field} could not be saved: {error}"));
                        continue;
                    }
                    // The key set is fixed for the process (it comes from a
                    // `static` registry), so the leak is bounded and one-time
                    // per distinct key rather than per frame.
                    let key: &'static str =
                        Box::leak(field.clone().into_boxed_str());
                    self.config_values.insert(key, value.clone());
                }
            }
        }
        // The panel writes config keys through the surgical writer above, which
        // lands on DISK. The running process holds its own `AppConfig` behind
        // one `RwLock` with a single whole-struct writer, so a disk write the
        // process never re-reads is a value the panel shows and the runtime
        // ignores — the exact "applied but not in effect" gap the two report
        // states exist to prevent.
        //
        // So reload and re-install. A reload failure is REPORTED rather than
        // swallowed: the key is on disk, and the operator must be told the live
        // process did not pick it up rather than being left to assume it did.
        // No config FILE means there is nothing to reload: the key went to
        // whatever path this process loaded, and an environment that has no file
        // at all is a sandbox or a test, not a failed reload. Warning there
        // would report a problem the operator cannot act on.
        if let Some(path) = operant_core::config::loaded_config_path() {
            match operant_core::config::load_app_config(Some(path.as_path())) {
                Ok(loaded) => {
                    operant_core::config::install_runtime_config(loaded.config.clone());
                    *config = loaded.config;
                }
                Err(error) => {
                    // Do not CLOBBER a warning the write already raised: when the
                    // persist failed the file is absent, so the reload fails for
                    // the same reason and saying it twice hides the one the
                    // operator acts on — which row, which value, not persisted.
                    if self.persist_warning.is_none() {
                        self.persist_warning = Some(format!(
                            "saved to disk, but the running process did not reload it: {error}"
                        ));
                    }
                }
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
        };
        self.settings_snapshot.theme = settings.theme.clone();
        self.settings_snapshot.output_style = settings.output_style.clone();
        self.settings_snapshot.effort_level = settings.effort_level.clone();
        self.settings_snapshot.vim_enabled = settings.vim_enabled;

        if !unread.is_empty() {
            // The edit was accepted, so the operator has every reason to
            // believe it took effect. Saying nothing makes the row a control
            // that silently does nothing. The failed entries are consumed
            // with the rest: keeping them pending would make every later
            // save re-report a value the operator has already been told about.
            self.apply_error = Some(format!(
                "Not a number: {} — kept the previous value, nothing was saved.",
                unread.join(", ")
            ));
            self.pending_changes.clear();
            return;
        }
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
    let mut entries: Vec<SettingsEntry> = Vec::new();
    // The environment variables that beat the config file on this run.
    let env_shadowed: Vec<String> = operant_core::config::env_override_keys();

    // The TUI's OWN visual state first. These live in `settings.json`, not
    // `operant.toml`: they are per-profile UI state that must not leak into a
    // shared config file, so they are declared here rather than read from the
    // generated registry — which describes CONFIG keys only.
    entries.push(SettingsEntry {
        key: "output_style",
        label: "Output style",
        description: "Controls the verbosity and format of responses.",
        kind: SettingKind::Enum {
            options: vec!["default", "concise", "explanatory", "learning"],
        },
        value: screen.output_style.clone(),
        store: SettingStore::SettingsJson,
    });
    entries.push(SettingsEntry {
        key: "reduce_motion",
        label: "Reduce motion",
        description: "Disable UI animations.",
        kind: SettingKind::Bool,
        value: bool_text(screen.reduce_motion),
        store: SettingStore::SettingsJson,
    });
    entries.push(SettingsEntry {
        key: "show_cwd",
        label: "Show current directory",
        description: "Display the current working directory in the footer.",
        kind: SettingKind::Bool,
        value: bool_text(screen.show_cwd),
        store: SettingStore::SettingsJson,
    });
    entries.push(SettingsEntry {
        key: "show_git_branch",
        label: "Show git branch",
        description: "Display the current git branch in the footer.",
        kind: SettingKind::Bool,
        value: bool_text(screen.show_git_branch),
        store: SettingStore::SettingsJson,
    });
    entries.push(SettingsEntry {
        key: "fileAutocompleteLimit",
        label: "File autocomplete limit",
        description: "Max suggestions shown in @ autocomplete (type more to narrow results).",
        kind: SettingKind::Number,
        value: screen.file_autocomplete_limit.clone(),
        store: SettingStore::SettingsJson,
    });
    entries.push(SettingsEntry {
        key: "fileAutocompleteShowHiddenFiles",
        label: "Show hidden files",
        description: "Include hidden files (.) in @ autocomplete.",
        kind: SettingKind::Bool,
        value: bool_text(screen.file_autocomplete_show_hidden_files),
        store: SettingStore::SettingsJson,
    });
    // (plan 2026-11-15 U1) The options come from `SeparatorStyle::ALL` through
    // `key()`, never a second literal list.
    entries.push(SettingsEntry {
        // The DOTTED key, because the row writes `operant.toml` and the arm
        // below matches on it; a bare `bar_style` would fall to the generic
        // arm and be refused as an unknown key.
        key: "tui.bar_style",
        label: "Status line style",
        description: "Separator style the status line draws between cells.",
        kind: SettingKind::Enum {
            options: screen.bar_style_options.clone(),
        },
        value: screen.bar_style.clone(),
        store: SettingStore::Config,
    });

    // `context_window` is a CONFIG key with a bespoke live path: the screen
    // carries it as a field because the apply arm validates it against
    // `validate_context_window` and reports a persist warning the generic path
    // cannot express. Declared here so the row keeps that treatment.
    entries.push(SettingsEntry {
        key: "agent.context_window",
        label: "Context window",
        description: "Total prompt and response capacity in tokens.",
        kind: SettingKind::Number,
        value: screen.context_window.to_string(),
        store: SettingStore::Config,
    });

    // Then every CONFIG key the registry calls live and the panel can draw.
    // A row the generator could not type (`Widget::None`/`Unsupported`) is
    // omitted: an f64 ratio or a HashMap has no honest single-line editor, and
    // offering a text box would invite a value the loader rejects.
    for row in operant_core::config_registry::SETTING_ROWS {
        if !row.live {
            continue;
        }
        // A key an ENV VAR overrides is shown but not editable: writing it would
        // put a value in `operant.toml` that the environment silently beats on
        // every load, which is the defect omp's `Setting.layered()` exists to
        // prevent. The row says so instead of accepting an edit that cannot
        // take effect.
        if env_shadowed.iter().any(|k| k == row.key) {
            entries.push(SettingsEntry {
                key: row.key,
                label: label_for(row.key),
                description: env_shadow_note(row.key),
                kind: SettingKind::Unread,
                value: screen.config_value(row.key),
                store: SettingStore::Config,
            });
            continue;
        }
        let kind = match row.widget {
            operant_core::config_registry::Widget::Bool => SettingKind::Bool,
            operant_core::config_registry::Widget::Number => SettingKind::Number,
            operant_core::config_registry::Widget::Text => SettingKind::Text,
            operant_core::config_registry::Widget::Cycle => SettingKind::Enum {
                options: cycle_options(row.key),
            },
            operant_core::config_registry::Widget::Unsupported
            | operant_core::config_registry::Widget::None => continue,
        };
        // A key the screen already declared above is not shown twice.
        if entries.iter().any(|e| e.key == row.key) {
            continue;
        }
        entries.push(SettingsEntry {
            key: row.key,
            label: label_for(row.key),
            description: description_for(row.key),
            kind,
            value: screen.config_value(row.key),
            store: SettingStore::Config,
        });
    }

    entries
}

/// Read one dotted config key out of the LIVE `AppConfig`.
///
/// Via serde, so there is one reader rather than a match arm per key: the config
/// is already `Serialize`, and a per-key match is exactly the drift the registry
/// was generated to prevent. A key whose path does not resolve (a nested table,
/// a key the runtime shape does not carry) reads as empty, which the row shows
/// rather than inventing a value.
fn read_dotted_config(config: &operant_core::config::AppConfig, dotted: &str) -> String {
    let Ok(value) = serde_json::to_value(config) else {
        return String::new();
    };
    let mut cursor = &value;
    for segment in dotted.split('.') {
        match cursor.get(segment) {
            Some(next) => cursor = next,
            None => return String::new(),
        }
    }
    match cursor {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        // An array or a table has no single-line form the panel can edit, and
        // the generator already excludes those rows; reaching here means a key
        // changed shape, and showing the JSON would invite an unparseable edit.
        _ => String::new(),
    }
}

/// `"true"` / `"false"` for a boolean row.
fn bool_text(value: bool) -> String {
    if value { "true" } else { "false" }.to_string()
}

/// The option list for a cycle row.
///
/// An enum's variants are the wire strings its `Display` impl writes, which is
/// what the config file holds. An empty list degrades the row to a text field
/// rather than a cycle through nothing — a control the operator cannot leave.
fn cycle_options(key: &str) -> Vec<&'static str> {
    match key {
        "agent.tool_progress_mode" => vec!["off", "compact", "full"],
        "agent.reasoning_effort_param" => vec!["none", "thinking", "reasoning_effort"],
        _ => Vec::new(),
    }
}

/// The description for a row an environment variable shadows.
///
/// The key's own note, plus the fact that an edit cannot take effect. Naming the
/// variable matters: "this is overridden" without saying by what is a dead end,
/// and the operator's next move is to unset it in their shell.
fn env_shadow_note(key: &str) -> &'static str {
    let _ = key;
    "An environment variable overrides this setting while it is set, so the panel cannot change it. Unset it in your shell to edit it here."
}

/// A row label: the key's last segment, de-underscored.
///
/// The registry carries no label, because a label is presentation. Deriving one
/// keeps the panel honest about what it is showing — the config key — instead of
/// inventing a second vocabulary that drifts from the file.
fn label_for(key: &str) -> &'static str {
    // Leak is bounded and one-time: the row list is built per frame from a
    // `static` table, so the set of distinct keys is fixed for the process.
    Box::leak(
        key.rsplit('.')
            .next()
            .unwrap_or(key)
            .replace('_', " ")
            .into_boxed_str(),
    )
}

/// The note the registry generated from the field's own doc comment.
fn description_for(key: &str) -> &'static str {
    operant_core::config_registry::CONFIG_KEYS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, _, note)| *note)
        .unwrap_or("")
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------
