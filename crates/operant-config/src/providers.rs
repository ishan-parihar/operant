//! Provider section (`[providers]`) types: model profiles, routing, fallback.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::schema::{
    EmbeddingRouteConfig, FallbackProviderConfig, ModelProviderConfig, ModelRouteConfig,
};

/// Top-level `[providers]` section. Wraps model provider profiles, routing rules,
/// and an optional fallback reference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ProvidersConfig {
    /// Key of the provider entry to use when no route matches.
    /// Optional — if unset, requests without a matching route fail at runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,

    /// Named model provider profiles keyed by id.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, ModelProviderConfig>,

    /// Model routing rules — route `hint:<name>` to specific provider+model combos.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_routes: Vec<ModelRouteConfig>,

    /// Ordered cross-provider fallback chain (hermes `fallback_providers`
    /// parity). Tried strictly in order after the primary provider fails.
    /// Entries referencing an unknown profile key (not in `models`) are
    /// skipped with a warning at runtime.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_chain: Vec<FallbackProviderConfig>,

    /// Embedding routing rules — route `hint:<name>` to specific provider+model combos.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding_routes: Vec<EmbeddingRouteConfig>,
}

impl ProvidersConfig {
    /// The `hint:` prefix a model name carries to ask for a ROUTE rather
    /// than a model.
    pub const HINT_PREFIX: &str = "hint:";

    /// Resolve a `hint:<name>` model name to the route that names it.
    ///
    /// The config's own doc says: "Route a task hint to a specific
    /// provider + model… Usage: pass `hint:reasoning` as the model
    /// parameter to route the request." The resolution existed only in
    /// `operant-providers`' router, which the runtime never calls — so
    /// the literal string `hint:reasoning` was sent to the provider AS
    /// THE MODEL NAME, and the provider rejected it (RT-96).
    ///
    /// This is the lookup the runtime needs, on the data it already has.
    /// `None` means the model is not a hint, or the hint names no route.
    pub fn model_route_for_hint(&self, model: &str) -> Option<&ModelRouteConfig> {
        let hint = model.strip_prefix(Self::HINT_PREFIX)?.trim();
        if hint.is_empty() {
            return None;
        }
        self.model_routes
            .iter()
            .find(|route| route.hint.trim() == hint)
    }

    /// Whether a model name is a `hint:` request — whether or not the
    /// hint names a route.
    ///
    /// A caller uses this to REJECT an unresolvable hint with a clear
    /// error instead of sending `hint:...` to the provider.
    pub fn is_model_hint(&self, model: &str) -> bool {
        model.trim_start().starts_with(Self::HINT_PREFIX)
    }

    /// Resolve the fallback provider entry by name, if configured.
    pub fn fallback_provider(&self) -> Option<&ModelProviderConfig> {
        self.fallback
            .as_deref()
            .and_then(|name| self.models.get(name))
    }
    /// Mutably resolve the fallback provider entry by name, if configured.
    pub fn fallback_provider_mut(&mut self) -> Option<&mut ModelProviderConfig> {
        let name = self.fallback.clone()?;
        self.models.get_mut(&name)
    }

    /// Return the first concrete `model` string available for use as a default.
    ///
    /// Resolution order:
    ///
    /// 1. The fallback provider's `model` field, if set.
    /// 2. The first entry in `models` (iteration order) that has `model` set.
    ///
    /// Returns `None` only when no provider entry has any model configured at all.
    /// Callers should treat `None` as a configuration error and surface it rather
    /// than silently substituting a hardcoded model identifier.
    pub fn resolve_default_model(&self) -> Option<String> {
        if let Some(model) = self
            .fallback_provider()
            .and_then(|e| e.model.as_deref())
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            return Some(model.to_string());
        }

        self.models
            .values()
            .filter_map(|entry| entry.model.as_deref().map(str::trim))
            .find(|m| !m.is_empty())
            .map(ToString::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_routes() -> ProvidersConfig {
        ProvidersConfig {
            fallback: None,
            models: HashMap::new(),
            model_routes: vec![
                ModelRouteConfig {
                    hint: "reasoning".to_string(),
                    provider: "openrouter".to_string(),
                    model: "anthropic/claude-opus-4".to_string(),
                    api_key: None,
                },
                ModelRouteConfig {
                    hint: "fast".to_string(),
                    provider: "groq".to_string(),
                    model: "llama-3.3-70b".to_string(),
                    api_key: None,
                },
            ],
            fallback_chain: Vec::new(),
            embedding_routes: Vec::new(),
        }
    }

    /// A `hint:<name>` model resolves to the route that names it.
    ///
    /// The config documents this usage — "pass `hint:reasoning` as the
    /// model parameter to route the request" — and the resolution used to
    /// live only in `operant-providers`' router, which the runtime never
    /// calls, so the literal string was sent to the provider AS THE MODEL
    /// NAME (RT-96).
    #[test]
    fn a_hint_model_resolves_to_its_route() {
        let config = config_with_routes();
        let route = config
            .model_route_for_hint("hint:reasoning")
            .expect("the hint resolves");
        assert_eq!(route.provider, "openrouter");
        assert_eq!(route.model, "anthropic/claude-opus-4");
    }

    /// A model name that is not a hint resolves to nothing.
    #[test]
    fn a_plain_model_name_is_not_a_hint() {
        let config = config_with_routes();
        assert!(config.model_route_for_hint("gpt-4o").is_none());
        assert!(!config.is_model_hint("gpt-4o"));
    }

    /// An unknown hint is still a HINT — so the caller can reject it
    /// instead of sending `hint:typo` to the provider as a model name.
    #[test]
    fn an_unknown_hint_is_still_a_hint() {
        let config = config_with_routes();
        assert!(config.model_route_for_hint("hint:typo").is_none());
        assert!(
            config.is_model_hint("hint:typo"),
            "an unknown hint must be recognisable as a hint"
        );
    }
}
