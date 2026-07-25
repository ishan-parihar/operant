//! Import config and turn management methods.

use super::*;

impl App {
    pub fn open_import_config_picker(&mut self) {
        self.import_config_picker =
            DialogSelectState::new("Import config", import_config_picker_items());
        self.import_config_picker.open();
    }

    pub(super) fn import_selection_from_picker(
        id: &str,
    ) -> Option<crate::tui::adapter_types::ImportSelection> {
        match id {
            "claude-md" => Some(crate::tui::adapter_types::ImportSelection::ClaudeMd),
            "settings" => Some(crate::tui::adapter_types::ImportSelection::Settings),
            "both" => Some(crate::tui::adapter_types::ImportSelection::Both),
            _ => None,
        }
    }

    pub(super) fn open_import_config_preview(
        &mut self,
        selection: crate::tui::adapter_types::ImportSelection,
    ) {
        match crate::tui::adapter_types::build_import_preview(selection) {
            Ok(preview) => {
                self.import_config_dialog.open(preview);
            }
            Err(err) => {
                self.status_message = Some(format!("Import failed: {}", err));
            }
        }
    }

    pub(super) fn perform_import_config(&mut self) {
        let Some(selection) = self.import_config_dialog.selection.clone() else {
            self.import_config_dialog.close();
            return;
        };
        match crate::tui::adapter_types::execute_import(selection) {
            Ok(result) => {
                let paths = crate::tui::adapter_types::ImportPaths::detect();
                let new_settings = Settings::load_sync().unwrap_or_default();
                let loaded = operant_core::config::load_app_config(None).unwrap_or_else(|_| {
                    operant_core::config::LoadedConfig {
                        config: AppConfig::default(),
                        source: None,
                    }
                });
                let result_message =
                    crate::tui::adapter_types::summarize_import_result(&result, &paths);
                let imported_mcp = result.imported_fields.iter().any(|f| f == "mcpServers");
                self.config = loaded.config;
                self.settings = new_settings;
                let model_to_resolve = self.config.agent.model.clone();
                self.model_name = self.resolve_stale_model(&model_to_resolve);
                if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
                    tracker.set_model(&self.model_name);
                }
                self.refresh_context_window_size();
                self.context_used_tokens = 0;
                self.has_credentials =
                    crate::tui::adapter_types::config::resolve_api_key().is_some();
                self.auth_store = crate::tui::adapter_types::AuthStore::load();
                self.plan_mode = matches!(
                    self.settings.permission_mode,
                    crate::tui::adapter_types::config::PermissionMode::Plan
                );
                self.output_style = match self.settings.output_style.as_deref() {
                    Some("stream") => "stream".to_string(),
                    Some("verbose") => "verbose".to_string(),
                    _ => "auto".to_string(),
                };
                if imported_mcp {
                    self.pending_mcp_reconnect = true;
                }
                self.status_message = Some(result_message);
                self.import_config_dialog.close();
            }
            Err(err) => {
                self.status_message = Some(format!("Import failed: {}", err));
                self.import_config_dialog.close();
            }
        }
    }

    pub(super) fn current_user_turn_index(&self) -> Option<usize> {
        self.messages
            .iter()
            .filter(|msg| msg.role == Role::User)
            .count()
            .checked_sub(1)
    }

}
