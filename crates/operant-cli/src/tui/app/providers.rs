//! Provider and model management methods.

use super::*;

// (plan 05A) Fetch the endpoint's own model list: OpenAI-style
// `GET {base_url}/models` with the client's key — the same endpoint the
// agent's requests hit. Never a row source on its own: the configured
// `[agent]` models are always merged in by the caller.
async fn fetch_endpoint_models(
    base_url: &str,
    api_key: Option<&str>,
) -> Vec<crate::tui::operant_app::tui_fns::PickerEntry> {
    let url = format!("{base_url}/models");
    let mut req = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(10));
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let parsed: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    parsed
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                .map(|id| inline_model_row(id, &format!("from {base_url}"), ""))
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    fn display_default_model_for_provider(&self, provider_id: &str) -> String {
        crate::tui::adapter_types::model_registry::default_model_for_provider(
            provider_id,
            &self.model_registry,
        )
    }

    // (plan 05B) open_model_picker_for_provider deleted with the legacy
    // overlay: the registry fallback now adapts its rows at the
    // model_fetch_rx boundary (see open_model_picker_inline), so the
    // provider-keyed opener has no callers and no state to open.

    /// (plan 05A) The /model picker for the AGENT'S OWN ENDPOINT: rows come
    /// from `GET {config.client.base_url}/models` with the client's own key
    /// — the endpoint the requests actually hit — plus the configured
    /// `[agent] model` and `fallback_models` (always shown, marked
    /// "configured", even when the endpoint is unreachable). The old
    /// provider-guessed catalogue listed models.dev rows for a provider
    /// inferred from the MODEL NAME, which ignored the endpoint entirely
    /// (an omp-gateway endpoint showed OpenAI's catalogue) and confirmed
    /// to a "{provider}/{id}" the endpoint does not know.
    /// (plan 05B) The unified inline picker over the AGENT'S OWN ENDPOINT:
    /// rows from `GET {base_url}/models` plus the configured `[agent]`
    /// model + fallback_models (marked "configured"), current row flagged.
    /// The fetch rides the same model_fetch_rx → drain path the old
    /// overlay used, so the picker opens instantly on the configured rows
    /// and refreshes in place when the endpoint answers.
    pub(super) fn open_model_picker_inline(&mut self) {
        let base_url = self
            .config
            .client
            .base_url
            .trim_end_matches('/')
            .to_string();
        let api_key = self.config.client.api_key.clone();
        let _host = reqwest::Url::parse(&base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "endpoint".to_string());

        let mut names: Vec<String> = std::iter::once(self.config.agent.model.clone())
            .chain(self.config.agent.fallback_models.iter().cloned())
            .filter(|m| !m.trim().is_empty())
            .collect();
        names.dedup();
        let current = self.config.agent.model.clone();
        let effort = {
            use crate::tui::model_effort::EffortLevel;
            match self.effort_level {
                EffortLevel::Low => "low",
                EffortLevel::Normal => "normal",
                EffortLevel::High => "high",
                EffortLevel::Max => "max",
            }
        };
        let mut entries: Vec<crate::tui::operant_app::tui_fns::PickerEntry> = names
            .iter()
            .map(|id| inline_model_row(id, "[agent] model / fallback_models", effort))
            .collect();
        for e in &mut entries {
            e.is_current = e.name == current;
        }

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        self.model_fetch_rx = Some(rx);
        self.model_picker_fetch_pending = false;
        // Headless callers (tests, non-runtime contexts) have no reactor —
        // the configured rows are already in place, so skip the fetch.
        if tokio::runtime::Handle::try_current().is_ok() {
            let rows_for_spawn = entries.clone();
            tokio::spawn(async move {
                let fetched = fetch_endpoint_models(&base_url, api_key.as_deref()).await;
                let mut rows = fetched;
                for c in rows_for_spawn {
                    if !rows.iter().any(|r| r.name == c.name) {
                        rows.push(c);
                    }
                }
                let _ = tx.send(Ok(rows)).await;
            });
        }

        let selected = entries.iter().position(|e| e.is_current).unwrap_or(0);
        let filtered = (0..entries.len()).collect();
        self.inline_interactive = Some(crate::tui::operant_app::tui_fns::InlineInteractiveState {
            kind: crate::tui::operant_app::tui_fns::PickerKind::Model,
            entries,
            filtered,
            selected,
            column: 0,
            filter: String::new(),
            preview: false,
        });
    }

    pub(super) fn activate_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        status_prefix: &str,
    ) {
        let _picker_title = provider_name.clone();
        self.set_provider_default(provider_id.clone());
        self.persist_provider_and_model();
        self.has_credentials = true;
        self.status_message = Some(format!("{} {}.", status_prefix, provider_name));
        // Mark onboarding as complete now that the user has connected a
        // provider. (P0-2 from UX audit — was never called.)
        let _ = Self::persist_onboarding_complete();
        // (plan 05B) the unified inline picker — same surface as /model.
        self.open_model_picker_inline();
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

    pub(super) fn persist_provider_and_model(&mut self) {
        // Provider+model live exclusively in operant.toml — persisted via
        // the ONE surgical writer (plan 09): agent.model only, in the file
        // the process actually loaded, comments and every other key kept
        // byte-for-byte. client.* is NEVER written from here: the old
        // sync_model_to_toml re-serialized the whole runtime config, which
        // stripped comments, resurrected retired default keys, clobbered
        // client.base_url with a provider-guessed URL, and wrote
        // env-sourced secrets to disk. settings.json is NOT written here;
        // it only stores visual prefs (iter-221).
        if let Err(e) =
            operant_core::config::persist_config_key("agent.model", &self.config.agent.model)
        {
            self.set_status_notice(format!("Could not persist model: {e}"));
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
            let resolved = crate::tui::adapter_types::model_registry::default_model_for_provider(
                provider,
                &self.model_registry,
            );
            if resolved != format!("{}/default", provider) {
                self.config.agent.model = resolved.clone();
                return resolved;
            }
        }
        model.to_string()
    }

    /// Alt+P: switch this session to the next configured model
    /// (`[agent] model` + `fallback_models`). In-session only — the picker
    /// (Ctrl+A) is what persists a choice.
    pub(crate) fn cycle_configured_model(&mut self) {
        if self.model_cycle.is_empty() {
            let mut models: Vec<String> = Vec::new();
            for model in std::iter::once(&self.config.agent.model)
                .chain(self.config.agent.fallback_models.iter())
            {
                if !model.trim().is_empty() && !models.contains(model) {
                    models.push(model.clone());
                }
            }
            self.model_cycle = models;
        }
        if self.model_cycle.len() < 2 {
            self.set_status_notice(
                "Only one model configured — add [agent] fallback_models to cycle.".to_string(),
            );
            return;
        }
        let next = self
            .model_cycle
            .iter()
            .position(|m| *m == self.model_name)
            .map_or(0, |i| (i + 1) % self.model_cycle.len());
        let model = self.model_cycle[next].clone();
        self.set_model(model.clone());
        // (plan 05A) the cycle reaches the agent through the same drain as
        // the picker confirm — the old path only updated the display copy.
        self.pending_model = Some(model.clone());
        self.set_status_notice(format!(
            "Model: {model} ({}/{})",
            next + 1,
            self.model_cycle.len()
        ));
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
    ///
    /// Also switches the process-global palette the renderers read, so the
    /// change is visible on the very next frame instead of only on the next
    /// launch.
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
        crate::tui::theme_colors::set_active_theme(theme_name);
        // (plan 07 §2) a theme change recolors every settled row — replay
        // re-emits history under the new palette.
        self.request_native_replay();
        self.status_message = Some(format!("Theme set to: {}", theme_name));
    }
}

/// (plan 05B) One inline picker row for a model id.
pub(super) fn inline_model_row(
    id: &str,
    detail: &str,
    effort: &str,
) -> crate::tui::operant_app::tui_fns::PickerEntry {
    crate::tui::operant_app::tui_fns::PickerEntry {
        name: id.to_string(),
        options: vec![crate::tui::operant_app::tui_fns::PickerOption {
            provider: "openai-compatible".to_string(),
            api_method: "chat/completions".to_string(),
            available: true,
            detail: detail.to_string(),
            estimated_reference_cost_micros: None,
        }],
        action: crate::tui::operant_app::tui_fns::PickerAction::Model,
        selected_option: 0,
        is_current: false,
        is_default: false,
        is_favorite: false,
        recommended: false,
        recommendation_rank: 0,
        usage_score: 0,
        old: false,
        created_date: None,
        effort: if effort.is_empty() {
            None
        } else {
            Some(effort.to_string())
        },
    }
}

impl super::App {
    /// (plan 05B) The effort rows in the unified inline picker: one row per
    /// wire level, the current level flagged and preselected. Enter routes
    /// through the plan-03 `pending_effort` drain (the agent applies it to
    /// the very next request).
    pub(super) fn open_effort_picker_inline(&mut self) {
        use crate::tui::model_effort::EffortLevel;
        let current = self.effort_level;
        let rows: &[(EffortLevel, &str, &str)] = &[
            (EffortLevel::Low, "low", "minimal reasoning effort"),
            (EffortLevel::Normal, "normal", "balanced reasoning effort"),
            (EffortLevel::High, "high", "deep reasoning effort"),
            (EffortLevel::Max, "max", "maximum reasoning effort"),
        ];
        let entries: Vec<crate::tui::operant_app::tui_fns::PickerEntry> = rows
            .iter()
            .map(
                |(level, wire, detail)| crate::tui::operant_app::tui_fns::PickerEntry {
                    name: format!("{} {}", level.symbol(), wire),
                    options: vec![crate::tui::operant_app::tui_fns::PickerOption {
                        provider: "reasoning".to_string(),
                        api_method: "effort".to_string(),
                        available: true,
                        detail: detail.to_string(),
                        estimated_reference_cost_micros: None,
                    }],
                    action: crate::tui::operant_app::tui_fns::PickerAction::Model,
                    selected_option: 0,
                    is_current: *level == current,
                    is_default: *level == EffortLevel::Normal,
                    is_favorite: false,
                    recommended: false,
                    recommendation_rank: 0,
                    usage_score: 0,
                    old: false,
                    created_date: None,
                    // The apply step needs the WIRE value, not the display name.
                    effort: Some(wire.to_string()),
                },
            )
            .collect();
        let selected = entries.iter().position(|e| e.is_current).unwrap_or(1);
        let filtered = (0..entries.len()).collect();
        self.inline_interactive = Some(crate::tui::operant_app::tui_fns::InlineInteractiveState {
            kind: crate::tui::operant_app::tui_fns::PickerKind::Model,
            entries,
            filtered,
            selected,
            column: 0,
            filter: String::new(),
            preview: false,
        });
    }
}
