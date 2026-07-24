// adapter_types/model_registry.rs — Model discovery and caching.

use super::free_catalog::reverse_provider_lookup;
use super::anthropic_client::{AnthropicClient, fetch_openai_compatible_models_async};


#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: std::collections::HashMap<String, Vec<crate::tui::model_picker::ModelEntry>>,
}

#[derive(Debug, Clone)]
pub struct RegistryModelEntry {
    pub info: ModelInfo,
}

#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    pub context_window: u32,
}

impl ModelRegistry {
    pub fn new() -> Self {
        let mut models = std::collections::HashMap::new();
        Self::populate_default_models(&mut models);
        Self { models }
    }

    pub fn load_cache(&mut self, _path: &std::path::Path) {}

    /// Add any missing providers from PROVIDERS without overwriting existing entries.
    #[allow(dead_code)] // Provider defaults initialization
    pub fn ensure_provider_defaults(&mut self) {
        for provider in crate::provider::PROVIDERS {
            if !self.models.contains_key(provider.name) {
                let entries: Vec<crate::tui::model_picker::ModelEntry> = provider
                    .models
                    .iter()
                    .map(|model_id| crate::tui::model_picker::ModelEntry {
                        id: model_id.to_string(),
                        display_name: model_id.to_string(),
                        description: provider.display_name.to_string(),
                        is_current: false,
                    })
                    .collect();
                if !entries.is_empty() {
                    self.models.insert(provider.name.to_string(), entries);
                }
            }
        }
    }

    /// Fetch models from models.dev catalog and merge into the registry.
    /// Uses provider_to_models_dev() mapping to match operant providers to catalog entries.
    pub async fn load_models_dev(&mut self) {
        let (models, _) = match operant_core::models_dev::fetch_models_dev(false).await {
            Ok(r) => r,
            Err(_) => return,
        };

        for model in &models {
            let m_provider = match model.get("provider_id").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => continue,
            };
            let model_id = match model.get("id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => continue,
            };

            let operant_provider = operant_core::models_dev::provider_to_models_dev(
                &reverse_provider_lookup(m_provider),
            )
            .map(|_| reverse_provider_lookup(m_provider))
            .or_else(|| {
                if crate::provider::PROVIDERS
                    .iter()
                    .any(|p| p.name == m_provider)
                {
                    Some(m_provider.to_string())
                } else {
                    None
                }
            });

            let provider_name = match operant_provider {
                Some(p) => p,
                None => continue,
            };

            let context_window = model.get("context_window").and_then(|v| v.as_u64());
            let cost_input = model.get("cost_input").and_then(|v| v.as_f64());
            let cost_output = model.get("cost_output").and_then(|v| v.as_f64());

            let description = match context_window {
                Some(ctx) => {
                    let ctx_str = if ctx >= 1_000_000 {
                        format!("{}M ctx", ctx / 1_000_000)
                    } else {
                        format!("{}K ctx", ctx / 1000)
                    };
                    let cost_str = match (cost_input, cost_output) {
                        (Some(i), Some(o)) => format!("${:.2}/${:.2} per M", i, o),
                        _ => String::new(),
                    };
                    if cost_str.is_empty() {
                        ctx_str
                    } else {
                        format!("{} | {}", ctx_str, cost_str)
                    }
                }
                None => String::new(),
            };

            let entry = crate::tui::model_picker::ModelEntry {
                id: model_id.to_string(),
                display_name: model_id.to_string(),
                description,
                is_current: false,
            };

            let entries = self.models.entry(provider_name).or_default();
            if !entries.iter().any(|e| e.id == model_id) {
                entries.push(entry);
            }
        }
    }

    /// Fetch models from a provider's /v1/models endpoint and merge them into the registry.
    ///
    /// Routes Anthropic through `AnthropicClient::fetch_available_models` (which
    /// uses `x-api-key` + `anthropic-version` headers — the OpenAI-compat
    /// `Authorization: Bearer` pattern does NOT work for Anthropic). All other
    /// providers go through the OpenAI-compat path.
    pub async fn fetch_from_provider_async(
        &mut self,
        provider_id: &str,
        api_key: &str,
        base_url: &str,
    ) {
        let fetched: Vec<String> = if provider_id == "anthropic" {
            let client =
                AnthropicClient::new(Some(api_key.to_string()), Some(base_url.to_string()));
            client.fetch_available_models().await
        } else {
            fetch_openai_compatible_models_async(api_key, base_url).await
        };
        if fetched.is_empty() {
            return;
        }

        // De-dup against any cached/catalog entries already present for this provider
        // (models.dev, populate_default_models, prior fetches). Match by id.
        let models = self.models.entry(provider_id.to_string()).or_default();
        let existing: std::collections::HashSet<String> =
            models.iter().map(|m| m.id.clone()).collect();
        for model_id in fetched {
            if existing.contains(model_id.as_str()) {
                continue;
            }
            models.push(crate::tui::model_picker::ModelEntry {
                id: model_id.clone(),
                display_name: model_id,
                description: String::new(),
                is_current: false,
            });
        }
    }
    pub fn get(&self, provider: &str, model_id: &str) -> Option<RegistryModelEntry> {
        self.list_by_provider(provider)
            .into_iter()
            .find(|m| m.id == model_id)
            .map(|_| RegistryModelEntry {
                info: ModelInfo::default(),
            })
    }

    pub fn list_visible_by_provider(
        &self,
        provider: &str,
    ) -> Vec<crate::tui::model_picker::ModelEntry> {
        self.list_by_provider(provider)
    }

    pub fn list_by_provider(&self, provider: &str) -> Vec<crate::tui::model_picker::ModelEntry> {
        self.models.get(provider).cloned().unwrap_or_default()
    }

    pub fn best_model_for_provider(&self, provider: &str) -> Option<String> {
        self.list_by_provider(provider)
            .first()
            .map(|m| m.id.clone())
    }

    fn populate_default_models(
        models: &mut std::collections::HashMap<String, Vec<crate::tui::model_picker::ModelEntry>>,
    ) {
        for provider in crate::provider::PROVIDERS {
            let entries: Vec<crate::tui::model_picker::ModelEntry> = provider
                .models
                .iter()
                .map(|model_id| crate::tui::model_picker::ModelEntry {
                    id: model_id.to_string(),
                    display_name: model_id.to_string(),
                    description: provider.display_name.to_string(),
                    is_current: false,
                })
                .collect();
            if !entries.is_empty() {
                models.insert(provider.name.to_string(), entries);
            }
        }
    }
}

