//! Provider and model management methods.

use super::*;

impl App {
    fn display_default_model_for_provider(&self, provider_id: &str) -> String {
        crate::tui::model_picker::default_model_for_provider(provider_id, &self.model_registry)
    }

    pub(super) fn open_model_picker_for_provider(&mut self, provider_id: &str, title: Option<String>) {
        self.dismiss_error_notifications();

        let cache_path = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("operant")
            .join("models.json");
        if cache_path.exists() {
            self.model_registry.load_cache(&cache_path);
        }

        let models = crate::tui::model_picker::models_for_provider_from_registry(
            provider_id,
            &self.model_registry,
        );
        self.model_picker.set_models(models);
        self.model_picker_provider_id = Some(provider_id.to_string());
        self.model_picker_fetch_pending = true;

        // Fetch models from provider's API in background
        let settings = crate::tui::adapter_types::Settings::load_sync().unwrap_or_default();
        let api_key = self.auth_store.api_key_for(
            provider_id
                .parse::<crate::tui::adapter_types::ProviderId>()
                .unwrap_or(crate::tui::adapter_types::ProviderId::Other(
                    provider_id.to_string(),
                )),
        );
        let base_url = settings
            .providers
            .get(provider_id)
            .and_then(|p| p.api_base.clone())
            .or_else(|| {
                crate::provider::PROVIDERS
                    .iter()
                    .find(|p| p.name == provider_id)
                    .map(|p| p.default_base_url.to_string())
            });

        if let (Some(key), Some(url)) = (api_key, base_url) {
            let provider_id = provider_id.to_string();
            let mut registry = self.model_registry.clone();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.model_fetch_rx = Some(rx);
            tokio::spawn(async move {
                let _fetch_result = registry
                    .fetch_from_provider_async(&provider_id, &key, &url)
                    .await;
                // Check if fetch returned any models; if empty, it's likely an error
                let models = crate::tui::model_picker::models_for_provider_from_registry(
                    &provider_id,
                    &registry,
                );
                if models.is_empty() {
                    // Fetch failed - send error
                    let _ = tx.send(Err(format!(
                        "Failed to fetch models from {} (rate limit, auth error, or network issue)",
                        provider_id
                    )))
                    .await;
                } else {
                    let _ = tx.send(Ok(models)).await;
                }
            });
        }

        let provider_prefix = format!("{}/", provider_id);
        let current_model = if self.active_provider.as_deref() == Some(provider_id) {
            self.model_name
                .strip_prefix(&provider_prefix)
                .unwrap_or(self.model_name.as_str())
                .to_string()
        } else {
            let default_model = self.display_default_model_for_provider(provider_id);
            default_model
                .strip_prefix(&provider_prefix)
                .unwrap_or(default_model.as_str())
                .to_string()
        };

        self.model_picker.open_with_title(
            title.unwrap_or_else(|| "Select model".to_string()),
            &current_model,
            self.effort_level,
            self.fast_mode,
        );
    }

    pub(super) fn activate_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        status_prefix: &str,
    ) {
        let picker_title = provider_name.clone();
        self.fast_mode = false;
        self.set_provider_default(provider_id.clone());
        self.persist_provider_and_model();
        self.has_credentials = true;
        self.status_message = Some(format!("{} {}.", status_prefix, provider_name));
        // Mark onboarding as complete now that the user has connected a
        // provider. (P0-2 from UX audit — was never called.)
        let _ = Self::persist_onboarding_complete();
        self.open_model_picker_for_provider(&provider_id, Some(picker_title));
    }

    pub(super) fn persist_custom_provider_base_url(&self, base_url: &str) {
        let mut settings = Settings::load_sync().unwrap_or_default();
        let entry = settings
            .providers
            .entry("custom-openai".to_string())
            .or_default();
        entry.api_base = Some(base_url.to_string());
        entry.enabled = true;
        let _ = settings.save_sync();
    }

    pub(super) fn persist_provider_and_model(&self) {
        // Provider+model live exclusively in operant.toml — written via
        // sync_model_to_toml below. settings.json is NOT written here; it only
        // stores visual prefs (theme, vim_enabled, reduce_motion, etc.) which are
        // persisted separately. (iter-221: removed dead settings.json round-trip
        // that was a no-op after iter-220 removed provider/model from Settings.)
        self.sync_model_to_toml(&self.config.agent.model);
    }

    // Write the current model + provider to ~/.operant/operant.toml so that
    /// `operant setup` reads the actual current values instead of defaults.
    /// (iter-117 — fixes the config-source proliferation bug.)
    fn sync_model_to_toml(&self, model: &str) {
        // Load the existing TOML config, update the model field, and write back.
        // We use the runtime config (already loaded by main.rs) rather than
        // re-parsing the TOML file, to avoid format issues.
        let mut config = operant_core::config::runtime_config();
        config.agent.model = model.to_string();
        if let Some(ref provider) = self.active_provider {
            // Update base_url based on provider.
            if let Some(p) = crate::provider::PROVIDERS
                .iter()
                .find(|p| p.name == *provider)
            {
                if !p.default_base_url.is_empty() {
                    config.client.base_url = p.default_base_url.to_string();
                }
            }
        }
        // Write to ~/.operant/operant.toml.
        let config_path = dirs::home_dir()
            .map(|h| h.join(".operant").join("operant.toml"))
            .unwrap_or_else(|| std::path::PathBuf::from("operant.toml"));
        if let Ok(toml_str) = toml::to_string_pretty(&config) {
            let _ = std::fs::write(&config_path, &toml_str);
        }
    }

    /// Switch the active provider while clearing any explicit model override.
    fn set_provider_default(&mut self, provider_id: String) {
        self.active_provider = Some(provider_id.clone());

        let model = self.display_default_model_for_provider(&provider_id);
        if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
            tracker.set_model(&model);
        }
        self.model_name = model;
        self.refresh_context_window_size();
        self.context_used_tokens = 0;
    }

    /// Update the context window size from the model registry for the current model.
    pub fn refresh_context_window_size(&mut self) {
        let provider = self.active_provider.as_deref().unwrap_or("anthropic");
        let model_id = self
            .model_name
            .strip_prefix(&format!("{}/", provider))
            .unwrap_or(&self.model_name);
        if let Some(entry) = self.model_registry.get(provider, model_id) {
            self.context_window_size = entry.info.context_window as u64;
        } else {
            // Fallback: common defaults
            self.context_window_size = match provider {
                "anthropic" => 200_000,
                "openai" => 128_000,
                "google" => 1_048_576,
                _ => 128_000,
            };
        }
    }

    /// Resolve a stale `provider/default` model name to the best actual model
    /// for that provider. This handles the case where settings.json stores a
    /// fallback model name from a previous session when the registry was empty.
    pub(super) fn resolve_stale_model(&mut self, model: &str) -> String {
        if model.ends_with("/default") {
            let provider = model.strip_suffix("/default").unwrap_or(model);
            let resolved =
                crate::tui::model_picker::default_model_for_provider(provider, &self.model_registry);
            if resolved != format!("{}/default", provider) {
                self.config.agent.model = resolved.clone();
                return resolved;
            }
        }
        model.to_string()
    }

    /// Update the active model name (also updates config + cost tracker).
    pub fn set_model(&mut self, model: String) {
        if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
            tracker.set_model(&model);
        }
        self.model_name = model.clone();
        self.config.agent.model = model.clone();
        if let Some(provider) = crate::tui::provider::infer_provider_from_model(&model) {
            self.active_provider = Some(provider);
        }
        self.refresh_context_window_size();
        // Reset used tokens when switching models (context is fresh).
        self.context_used_tokens = 0;
    }

    /// Apply a theme by name, persisting it to config.
    pub fn apply_theme(&mut self, theme_name: &str) {
        let theme = match theme_name {
            "dark" => Theme::Dark,
            "light" => Theme::Light,
            "default" => Theme::Default,
            "deuteranopia" => Theme::Deuteranopia,
            other => Theme::Custom(other.to_string()),
        };
        self.settings.theme = theme.clone();
        self.config.tui.theme = theme_name.to_string();
        // Persist to settings file
        let mut settings = Settings::load_sync().unwrap_or_default();
        settings.theme = theme;
        let _ = settings.save_sync();
        self.status_message = Some(format!("Theme set to: {}", theme_name));
    }
}
