//! Provider identity: one trait implementation plus one registration line.
//!
//! A `ChatProvider` is a *declaration* — which vendor this is, which alternate
//! spellings and model prefixes belong to it, and which models.dev id prices
//! it. The live [`ModelClient`](super::model_client::ModelClient) stays where
//! it is, in the fallback chain.
//!
//! Adding a provider is:
//!
//! ```ignore
//! pub struct MyVendor;
//! impl ChatProvider for MyVendor {
//!     fn vendor(&self) -> &str { "myvendor" }
//!     fn model_prefixes(&self) -> &'static [&'static str] { &["my-model-"] }
//!     fn models_dev_id(&self) -> Option<&'static str> { Some("myvendor") }
//! }
//! // …and in the builtin list:
//! catalog.register(MyVendor);
//! ```
//!
//! There is no enum to extend and no `match` to update.

use std::sync::Arc;

use super::runtime_key::VendorId;

/// Identity + capabilities the runtime needs to talk to one vendor.
pub trait ChatProvider: Send + Sync {
    /// Canonical vendor slug, lowercase (`"openai"`).
    fn vendor(&self) -> &str;

    /// Alternate spellings that name this vendor (`"grok"` → `xai`).
    fn vendor_aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Model-slug prefixes this vendor owns, used to recover the vendor from a
    /// bare slug. Longest prefix wins; a prefix claimed by two vendors is
    /// ambiguous and resolves to nothing — see [`ProviderCatalog::infer_vendor`].
    fn model_prefixes(&self) -> &'static [&'static str] {
        &[]
    }

    /// models.dev provider id, used for capability + pricing lookups.
    /// `None` means the catalog has no pricing entry for this vendor.
    fn models_dev_id(&self) -> Option<&'static str> {
        None
    }
}

/// The OpenAI wire-protocol family — also the path every OpenAI-compatible
/// endpoint (ollama, llama.cpp, vLLM, openrouter) is reached through.
pub struct OpenAIVendor;

impl ChatProvider for OpenAIVendor {
    fn vendor(&self) -> &str {
        "openai"
    }
    fn vendor_aliases(&self) -> &'static [&'static str] {
        &["oai", "gpt"]
    }
    fn model_prefixes(&self) -> &'static [&'static str] {
        &["gpt-", "chatgpt-", "o1", "o3", "o4"]
    }
    fn models_dev_id(&self) -> Option<&'static str> {
        Some("openai")
    }
}

/// The native Anthropic Messages API.
#[cfg(feature = "anthropic")]
pub struct AnthropicVendor;

#[cfg(feature = "anthropic")]
impl ChatProvider for AnthropicVendor {
    fn vendor(&self) -> &str {
        "anthropic"
    }
    fn vendor_aliases(&self) -> &'static [&'static str] {
        &["claude"]
    }
    fn model_prefixes(&self) -> &'static [&'static str] {
        &["claude-"]
    }
    fn models_dev_id(&self) -> Option<&'static str> {
        Some("anthropic")
    }
}

/// Registered providers, in registration order.
///
/// Cheap to clone (one `Arc`), and immutable once handed to a
/// [`ProviderRegistry`](super::provider_registry::ProviderRegistry).
#[derive(Clone, Default)]
pub struct ProviderCatalog {
    providers: Arc<Vec<Arc<dyn ChatProvider>>>,
}

impl ProviderCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in providers. One `register` line per provider.
    pub fn with_builtins() -> Self {
        let mut catalog = Self::new();
        catalog.register(OpenAIVendor);
        #[cfg(feature = "anthropic")]
        catalog.register(AnthropicVendor);
        catalog
    }

    /// Register a provider. This one call is the entire cost of adding one.
    pub fn register(&mut self, provider: impl ChatProvider + 'static) {
        Arc::make_mut(&mut self.providers).push(Arc::new(provider));
    }

    /// Fold a vendor spelling onto a registered vendor, following aliases.
    /// `None` when nothing registered claims that spelling.
    pub fn resolve_vendor(&self, spelling: &str) -> Option<VendorId> {
        let candidate = VendorId::new(spelling);
        if candidate.is_unknown() {
            return None;
        }
        self.providers.iter().find_map(|p| {
            let vendor = VendorId::new(p.vendor());
            let alias_hit = p
                .vendor_aliases()
                .iter()
                .any(|a| VendorId::new(a) == candidate);
            (vendor == candidate || alias_hit).then_some(vendor)
        })
    }

    /// Recover the vendor of a bare model slug from the longest matching
    /// `model_prefixes` entry.
    ///
    /// `None` when no provider claims the slug **or** when two providers claim
    /// the same longest prefix. Guessing wrong here would merge two vendors'
    /// runtimes into one [`RuntimeKey`](super::runtime_key::RuntimeKey), so
    /// ambiguity resolves to nothing.
    pub fn infer_vendor(&self, model_slug: &str) -> Option<VendorId> {
        if model_slug.is_empty() {
            return None;
        }
        // Model ids are case-insensitive, so fold before matching prefixes.
        let slug = model_slug.to_ascii_lowercase();
        let mut best: Option<(usize, VendorId)> = None;
        for p in self.providers.iter() {
            for prefix in p.model_prefixes() {
                if !slug.starts_with(prefix) {
                    continue;
                }
                let len = prefix.len();
                let vendor = VendorId::new(p.vendor());
                match &best {
                    Some((best_len, best_vendor)) if *best_len > len => {}
                    Some((best_len, best_vendor)) if *best_len == len && *best_vendor != vendor => {
                        // Tie between two vendors: ambiguous, refuse to guess.
                        return None;
                    }
                    _ => best = Some((len, vendor)),
                }
            }
        }
        best.map(|(_, v)| v)
    }

    /// models.dev provider id for a vendor, for capability + pricing lookups.
    pub fn models_dev_id_for(&self, vendor: &VendorId) -> Option<&'static str> {
        if vendor.is_unknown() {
            return None;
        }
        self.providers
            .iter()
            .find(|p| VendorId::new(p.vendor()) == *vendor)
            .and_then(|p| p.models_dev_id())
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Registered vendors, in registration order.
    pub fn vendors(&self) -> Vec<VendorId> {
        self.providers
            .iter()
            .map(|p| VendorId::new(p.vendor()))
            .collect()
    }
}

impl std::fmt::Debug for ProviderCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCatalog")
            .field("vendors", &self.vendors())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_resolve_their_own_spellings() {
        let cat = ProviderCatalog::with_builtins();
        assert!(!cat.is_empty());
        assert!(cat.resolve_vendor("openai").is_some());
        assert!(cat.resolve_vendor("oai").is_some());
        assert_eq!(
            cat.infer_vendor("gpt-4o").map(|v| v.to_string()),
            Some("openai".into())
        );
        assert_eq!(
            cat.models_dev_id_for(&VendorId::new("openai")),
            Some("openai")
        );
        assert_eq!(cat.models_dev_id_for(&VendorId::new("nope")), None);
        assert!(cat.models_dev_id_for(&VendorId::new("")).is_none());
    }

    /// The anthropic vendor exists only under the `anthropic` feature, so the
    /// built-in catalog is exactly as large as that feature set allows.
    #[test]
    fn builtins_track_the_anthropic_feature() {
        let cat = ProviderCatalog::with_builtins();
        let has_anthropic = cat.resolve_vendor("anthropic").is_some();
        assert_eq!(has_anthropic, cfg!(feature = "anthropic"));
        assert_eq!(cat.len(), if has_anthropic { 2 } else { 1 });
    }

    #[test]
    fn register_is_the_whole_cost_of_adding_a_provider() {
        struct MyVendor;
        impl ChatProvider for MyVendor {
            fn vendor(&self) -> &str {
                "myvendor"
            }
            fn vendor_aliases(&self) -> &'static [&'static str] {
                &["mv"]
            }
            fn model_prefixes(&self) -> &'static [&'static str] {
                &["my-model-"]
            }
            fn models_dev_id(&self) -> Option<&'static str> {
                Some("myvendor")
            }
        }
        let mut cat = ProviderCatalog::new();
        assert!(cat.is_empty());
        cat.register(MyVendor);
        assert_eq!(cat.len(), 1);
        assert_eq!(
            cat.resolve_vendor("MV").map(|v| v.to_string()),
            Some("myvendor".into())
        );
        assert_eq!(
            cat.infer_vendor("my-model-x").map(|v| v.to_string()),
            Some("myvendor".into())
        );
        assert_eq!(
            cat.models_dev_id_for(&VendorId::new("myvendor")),
            Some("myvendor")
        );
    }

    #[test]
    fn infer_vendor_is_case_and_whitespace_tolerant() {
        let cat = ProviderCatalog::with_builtins();
        assert!(cat.infer_vendor("").is_none());
        assert_eq!(
            cat.infer_vendor("GPT-4O").map(|v| v.to_string()),
            Some("openai".into())
        );
    }
}
