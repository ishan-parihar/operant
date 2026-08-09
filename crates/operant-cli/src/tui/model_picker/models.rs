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
