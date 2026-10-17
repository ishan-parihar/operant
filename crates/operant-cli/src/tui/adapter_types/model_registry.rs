// adapter_types/model_registry.rs — Model discovery and caching.

use super::anthropic_client::{AnthropicClient, fetch_openai_compatible_models_async};
use super::free_catalog::reverse_provider_lookup;

#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: std::collections::HashMap<String, Vec<ModelEntry>>,
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
            models.push(ModelEntry {
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

    pub fn list_visible_by_provider(&self, provider: &str) -> Vec<ModelEntry> {
        self.list_by_provider(provider)
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

/// Helper to build a `ModelEntry` with `is_current = false`.
pub(crate) fn model_entry(id: &str, name: &str, desc: &str) -> ModelEntry {
    ModelEntry {
        id: id.to_string(),
        display_name: name.to_string(),
        description: desc.to_string(),
        is_current: false,
    }
}

/// Get models for a provider from the model registry (models.dev data).
///
/// Builds picker entries from the bundled / network-refreshed registry.
/// The registry is always populated (the embedded models.dev snapshot
/// contains ~118 providers / ~4500 models), so the only time the result
/// is empty is when the caller passed a truly unknown provider id — in
/// which case we synthesize a single `"default"` placeholder so the
/// picker isn't blank.
pub fn models_for_provider_from_registry(
    provider_id: &str,
    registry: &crate::tui::adapter_types::ModelRegistry,
) -> Vec<ModelEntry> {
    // "free" is the composite Zen → OpenRouter provider; the upstream
    // models.dev catalog has nothing under this id, so serve a curated list
    // directly.  `free/auto` is the default routing entry; the rest pin a
    // specific upstream model for users who care.
    if provider_id == "free" {
        return free_provider_models();
    }
    // Codex (ChatGPT-authenticated OpenAI) is not in the models.dev catalog —
    // serve the curated CODEX_MODELS list so the picker isn't empty.
    if provider_id == "codex" {
        return codex_provider_models();
    }

    let mut entries = registry.list_visible_by_provider(provider_id);

    // Fall back to all entries (including alpha/deprecated) if the visible
    // filter wiped the list — better to show something than nothing.
    if entries.is_empty() {
        entries = registry.list_by_provider(provider_id);
    }

    if entries.is_empty() {
        // Truly unknown provider — keep the picker non-empty so /model still
        // works against e.g. self-hosted endpoints.
        return vec![model_entry(
            "default",
            "Default model",
            "no catalog entry for this provider",
        )];
    }

    // Sort: alphabetical by id.
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    entries
        .iter()
        .map(|e| ModelEntry {
            id: e.id.clone(),
            display_name: e.display_name.clone(),
            description: e.description.clone(),
            is_current: false,
        })
        .collect()
}

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

/// Curated Codex (ChatGPT-authenticated OpenAI) model list used by
/// `models_for_provider_from_registry` because models.dev does not catalog
/// these endpoints.
fn codex_provider_models() -> Vec<ModelEntry> {
    crate::tui::adapter_types::codex_oauth::CODEX_MODELS
        .iter()
        .map(|(id, name)| {
            let ctx = match *id {
                "gpt-5.4" | "gpt-5.2" | "gpt-5.2-codex" | "gpt-5.1-codex"
                | "gpt-5.1-codex-mini" | "gpt-5.1-codex-max" => "400K ctx",
                _ => "128K ctx",
            };
            ModelEntry {
                id: id.to_string(),
                display_name: name.to_string(),
                description: format!("{} | ChatGPT-authenticated", ctx),
                is_current: false,
            }
        })
        .collect()
}

/// Curated free-mode model list used by `models_for_provider_from_registry`.
/// Always shows `free/auto` first; one pin entry per catalog upstream so the
/// user can target a specific provider when they need to.
fn free_provider_models() -> Vec<ModelEntry> {
    let mut entries = vec![ModelEntry {
        id: "free/auto".to_string(),
        display_name: "Auto (round-robin across configured providers)".to_string(),
        description: "stacks every free-tier key you've added · $0.00 per M".to_string(),
        is_current: false,
    }];

    for upstream in crate::tui::adapter_types::FREE_CATALOG {
        entries.push(ModelEntry {
            id: format!("{}/{}", upstream.id, upstream.default_model),
            display_name: format!("{} \u{2014} {}", upstream.title, upstream.default_model),
            description: format!("{} · $0.00 per M", upstream.note),
            is_current: false,
        });
    }

    entries
}
