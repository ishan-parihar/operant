// adapter_types/model_registry.rs — Model discovery and caching.

use super::free_catalog::reverse_provider_lookup;

#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: std::collections::HashMap<String, Vec<ModelEntry>>,
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
                let entries: Vec<ModelEntry> = provider
                    .models
                    .iter()
                    .map(|model_id| ModelEntry {
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

            let entry = ModelEntry {
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

    pub fn list_by_provider(&self, provider: &str) -> Vec<ModelEntry> {
        self.models.get(provider).cloned().unwrap_or_default()
    }

    pub fn best_model_for_provider(&self, provider: &str) -> Option<String> {
        self.list_by_provider(provider)
            .first()
            .map(|m| m.id.clone())
    }

    fn populate_default_models(models: &mut std::collections::HashMap<String, Vec<ModelEntry>>) {
        for provider in crate::provider::PROVIDERS {
            let entries: Vec<ModelEntry> = provider
                .models
                .iter()
                .map(|model_id| ModelEntry {
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

// ── (plan 05B) moved WHOLE from the deleted tui/model_picker/models.rs ──

// model_picker/models.rs — Model entry definitions and provider model lists.
//
// Extracted from the model_picker.rs monolith. ModelEntry struct, registry
// lookups, default-model resolution, and provider-specific model lists.

/// A single model entry shown in the picker.
#[expect(
    dead_code,
    reason = "picker row data; display fields are rendered by the native selector"
)]
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    /// Whether this is the currently active model.
    pub is_current: bool,
}

// ---------------------------------------------------------------------------
// Provider-aware model lists
// ---------------------------------------------------------------------------

/// Return the provider-prefixed default model name for a given provider,
/// consulting the registry first and falling back to a `provider/default`
/// placeholder for unknown providers.
///
/// **Anthropic exception** — anthropic models are emitted bare (no
/// `anthropic/` prefix) for backward-compatibility with config files that
/// pre-date the multi-provider era.
///
/// **Free exception** — the composite Zen → OpenRouter provider ships with
/// a synthetic `free/auto` default that the wrapper translates per upstream.
pub fn default_model_for_provider(
    provider_id: &str,
    registry: &crate::tui::adapter_types::ModelRegistry,
) -> String {
    if provider_id == "free" {
        return "free/auto".to_string();
    }
    if let Some(best) = registry.best_model_for_provider(provider_id) {
        if provider_id == "anthropic" {
            best
        } else {
            format!("{}/{}", provider_id, best)
        }
    } else {
        format!("{}/default", provider_id)
    }
}
