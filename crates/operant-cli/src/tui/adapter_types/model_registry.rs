// adapter_types/model_registry.rs — the default model for a provider.
//
// There is no compiled-in model list. The sync default is the first agentic model
// the models.dev cache lists for the provider; a connect resolves the live list
// through `operant_core::model_catalog` and takes its default from there.

/// Return the provider-prefixed default model name for `provider_id`, from the
/// models.dev cache. `"{provider}/default"` when the cache lists nothing for it.
///
/// **Free exception** — the composite Zen → OpenRouter provider ships with
/// a synthetic `free/auto` default that the wrapper translates per upstream.
pub fn default_model_for_provider(provider_id: &str) -> String {
    if provider_id == "free" {
        return "free/auto".to_string();
    }
    match operant_core::models_dev::cached_agentic_models(provider_id)
        .into_iter()
        .next()
    {
        Some(best) => qualify_model(provider_id, &best),
        None => format!("{provider_id}/default"),
    }
}

/// `provider_id/model_id`, except **Anthropic**: anthropic models are emitted bare
/// (no `anthropic/` prefix) for backward-compatibility with config files that
/// pre-date the multi-provider era.
pub fn qualify_model(provider_id: &str, model_id: &str) -> String {
    if provider_id == "anthropic" {
        model_id.to_string()
    } else {
        format!("{provider_id}/{model_id}")
    }
}
