//! Provider registry for cross-provider fallback.
//!
//! When the primary provider fails with auth/billing errors, the agent can
//! switch to a different provider (e.g., Anthropic → OpenAI) by selecting a
//! pre-configured client from the registry.
//!
//! Identity is structural: a chain entry is turned into a
//! [`RuntimeKey`] (vendor + model + pinned revision) and the anti-thrash
//! bookkeeping is keyed by [`VendorId`], not by whatever display string a
//! config file happened to contain. See [`chat_provider`] for how a provider is
//! registered.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{info, warn};

use super::chat_provider::ProviderCatalog;
use super::model_client::ModelClient;
use super::runtime_key::{RouteCost, RuntimeKey, VendorId};

/// A pre-configured provider entry in the fallback chain.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub name: String,
    pub model: String,
}

/// Registry of pre-constructed `ModelClient` instances for cross-provider fallback.
#[derive(Clone)]
pub struct ProviderRegistry {
    clients: HashMap<String, Arc<dyn ModelClient>>,
    fallback_chain: Vec<ProviderEntry>,
    active_index: Arc<std::sync::RwLock<usize>>,
    /// Anti-thrash: cooldown timestamps per vendor (VendorId -> cooldown_until).
    cooldowns: Arc<std::sync::RwLock<HashMap<VendorId, f64>>>,
    /// Anti-thrash: failure count per vendor for exponential backoff.
    failure_counts: Arc<std::sync::RwLock<HashMap<VendorId, usize>>>,
    /// Provider identities: vendor aliases, model-prefix ownership, and the
    /// models.dev id used for pricing lookups. Defaulted to the built-ins.
    catalog: ProviderCatalog,
}

impl ProviderRegistry {
    pub fn new(
        clients: HashMap<String, Arc<dyn ModelClient>>,
        fallback_chain: Vec<ProviderEntry>,
    ) -> Self {
        Self {
            clients,
            fallback_chain,
            active_index: Arc::new(std::sync::RwLock::new(0)),
            cooldowns: Arc::new(std::sync::RwLock::new(HashMap::new())),
            failure_counts: Arc::new(std::sync::RwLock::new(HashMap::new())),
            catalog: ProviderCatalog::with_builtins(),
        }
    }

    pub fn empty() -> Self {
        Self {
            clients: HashMap::new(),
            fallback_chain: Vec::new(),
            active_index: Arc::new(std::sync::RwLock::new(0)),
            cooldowns: Arc::new(std::sync::RwLock::new(HashMap::new())),
            failure_counts: Arc::new(std::sync::RwLock::new(HashMap::new())),
            catalog: ProviderCatalog::with_builtins(),
        }
    }

    /// Replace the provider catalog — the seam a test (or an embedder that
    /// ships extra providers) uses to register identities.
    pub fn with_catalog(mut self, catalog: ProviderCatalog) -> Self {
        self.catalog = catalog;
        self
    }

    pub fn catalog(&self) -> &ProviderCatalog {
        &self.catalog
    }

    fn now_secs() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    /// Fold a provider spelling onto its structural vendor id.
    ///
    /// A registered provider's alias (`claude` → `anthropic`) wins; anything
    /// the catalog does not claim (custom endpoints like `free`) falls back to
    /// its own normalized slug, so unregistered providers keep working
    /// exactly as before.
    pub fn vendor_of(&self, provider_name: &str) -> VendorId {
        self.catalog
            .resolve_vendor(provider_name)
            .unwrap_or_else(|| VendorId::new(provider_name))
    }

    /// Structural identity of a chain entry: vendor from the entry's provider
    /// name, model + revision from its model spelling.
    pub fn key_for(&self, entry: &ProviderEntry) -> RuntimeKey {
        let parsed = RuntimeKey::parse_in(&entry.model, &self.catalog);
        RuntimeKey::new(
            self.vendor_of(&entry.name),
            parsed.model().to_string(),
            parsed.revision().clone(),
        )
    }

    /// Structural identity of the currently active chain entry.
    pub fn active_key(&self) -> Option<RuntimeKey> {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        self.fallback_chain.get(idx).map(|e| self.key_for(e))
    }

    /// Check if a provider is currently in cooldown.
    pub fn is_in_cooldown(&self, provider_name: &str) -> bool {
        let vendor = self.vendor_of(provider_name);
        match self.cooldowns.read() {
            Ok(cooldowns) => {
                if let Some(&until) = cooldowns.get(&vendor) {
                    Self::now_secs() < until
                } else {
                    false
                }
            }
            Err(_) => {
                warn!(
                    provider = provider_name,
                    "Lock poisoned while checking cooldown — assuming not in cooldown"
                );
                false
            }
        }
    }

    /// Arm a cooldown for a provider after a failed switch attempt.
    /// Cooldown scales with failure count: 5s, 10s, 20s, 40s (cap 60s).
    pub fn arm_cooldown(&self, provider_name: &str) {
        let vendor = self.vendor_of(provider_name);
        // Increment failure count and compute exponential backoff.
        let count = match self.failure_counts.write() {
            Ok(mut counts) => {
                let entry = counts.entry(vendor.clone()).or_insert(0);
                *entry += 1;
                *entry
            }
            Err(_) => {
                warn!(
                    provider = provider_name,
                    "Lock poisoned while arming cooldown — using count=1"
                );
                1
            }
        };
        let base = 5.0_f64;
        let delay = (base * 2.0_f64.powi(count as i32 - 1)).min(60.0);
        let until = Self::now_secs() + delay;
        if let Ok(mut cooldowns) = self.cooldowns.write() {
            cooldowns.insert(vendor, until);
            warn!(
                provider = provider_name,
                failure = count,
                cooldown_secs = delay,
                "Provider anti-thrash cooldown armed"
            );
        }
    }

    /// Reset failure count for a provider (called on successful switch).
    pub fn clear_failure_count(&self, provider_name: &str) {
        if let Ok(mut counts) = self.failure_counts.write() {
            counts.remove(&self.vendor_of(provider_name));
        }
    }

    /// Look up a provider's client.
    ///
    /// Exact match first — the configured name always hits. Failing that, the
    /// name is folded onto its vendor id so a differently-spelled registration
    /// of the same vendor resolves. Only ever *adds* matches.
    pub fn get_client(&self, provider_name: &str) -> Option<Arc<dyn ModelClient>> {
        if let Some(client) = self.clients.get(provider_name) {
            return Some(client.clone());
        }
        let vendor = self.vendor_of(provider_name);
        if vendor.is_unknown() {
            return None;
        }
        self.clients.iter().find_map(|(k, v)| {
            if self.vendor_of(k) == vendor {
                Some(v.clone())
            } else {
                None
            }
        })
    }

    /// Per-route cost estimate for a chain entry, read from the models.dev
    /// cache this process already populates for cost reporting.
    ///
    /// `None` when the catalog has no price for that route — a caller must then
    /// keep its configured ordering rather than reorder on a guess.
    pub fn route_cost(&self, entry: &ProviderEntry) -> Option<RouteCost> {
        let key = self.key_for(entry);
        let dev_id = self.catalog.models_dev_id_for(key.vendor())?;
        let caps = crate::models_dev::cached_capabilities(dev_id, key.model())?;
        Some(RouteCost {
            input_per_million: caps.cost_input_per_million?,
            output_per_million: caps.cost_output_per_million?,
        })
    }

    pub fn active_provider(&self) -> Option<String> {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        self.fallback_chain.get(idx).map(|e| e.name.clone())
    }

    pub fn active_model(&self) -> Option<String> {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        self.fallback_chain.get(idx).map(|e| e.model.clone())
    }

    /// Commit a switch to chain index `next` and report the entry.
    fn commit_switch(&self, next: usize) -> Option<ProviderEntry> {
        if let Ok(mut idx) = self.active_index.write() {
            *idx = next;
        } else {
            warn!("Lock poisoned while switching provider — cannot advance");
            return None;
        }
        let entry = self.fallback_chain[next].clone();
        info!(
            provider = %entry.name,
            model = %entry.model,
            "Switched to fallback provider"
        );
        Some(entry)
    }

    /// Switch to the next provider in the fallback chain.
    /// Skips providers that are currently in cooldown.
    pub fn switch_to_next(&self) -> Option<ProviderEntry> {
        let start = self.active_index.read().map(|g| *g).unwrap_or(0);
        // Skip providers that are in cooldown.
        let mut next = start + 1;
        while next < self.fallback_chain.len() {
            let candidate = &self.fallback_chain[next];
            if !self.is_in_cooldown(&candidate.name) {
                break;
            }
            warn!(
                provider = %candidate.name,
                "Skipping provider in anti-thrash cooldown"
            );
            next += 1;
        }
        if next >= self.fallback_chain.len() {
            warn!(
                chain_len = self.fallback_chain.len(),
                "Fallback chain exhausted (all remaining providers in cooldown)"
            );
            return None;
        }
        self.commit_switch(next)
    }

    /// Like [`Self::switch_to_next`] but prefers the cheapest *priced* route.
    ///
    /// For a merely rate-limited primary this is the better substitute: the
    /// model is not broken, so a cheaper sibling beats the next configured
    /// one. Providers whose cost is unknown keep their configured order and are
    /// only reached when no priced route is available — the chain is never
    /// reordered on a price we do not have.
    pub fn switch_to_next_cheapest(&self) -> Option<ProviderEntry> {
        let start = self.active_index.read().map(|g| *g).unwrap_or(0);
        let mut best: Option<(usize, f64)> = None;
        for i in start + 1..self.fallback_chain.len() {
            let entry = &self.fallback_chain[i];
            if self.is_in_cooldown(&entry.name) {
                continue;
            }
            let Some(cost) = self.route_cost(entry) else {
                continue;
            };
            let blended = cost.blended_per_million();
            if best.is_none_or(|(_, cheapest)| blended < cheapest) {
                best = Some((i, blended));
            }
        }
        match best {
            Some((i, blended)) => {
                info!(
                    index = i,
                    blended_usd_per_million = blended,
                    "Preferring cheaper priced fallback route"
                );
                self.commit_switch(i)
            }
            None => self.switch_to_next(),
        }
    }

    /// Reset to the primary provider and clear all failure counts.
    /// Called at turn start to ensure provider fallback is temporary.
    /// Note: cooldowns are intentionally NOT cleared here — they are
    /// time-based and should persist across turns until they expire.
    pub fn reset_to_primary(&self) -> Option<ProviderEntry> {
        let mut idx = match self.active_index.write() {
            Ok(g) => g,
            Err(_) => {
                warn!("Lock poisoned while resetting to primary — cannot reset");
                return None;
            }
        };
        if self.fallback_chain.is_empty() {
            return None;
        }
        *idx = 0;
        // Clear all failure counts so providers that failed in the previous
        // turn don't carry stale state into the new turn.
        if let Ok(mut counts) = self.failure_counts.write() {
            counts.clear();
        }
        Some(self.fallback_chain[0].clone())
    }

    pub fn has_providers(&self) -> bool {
        !self.fallback_chain.is_empty()
    }
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        f.debug_struct("ProviderRegistry")
            .field("providers", &self.clients.keys().collect::<Vec<_>>())
            .field("chain", &self.fallback_chain)
            .field("active_index", &idx)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use futures::stream::BoxStream;

    use super::super::model_client::{ChatRequest, StreamChunk};
    use crate::client::{ChatResponse, Choice, MessageDelta, Role, Usage};
    use crate::error::{Error, Result};

    struct MockClient {
        name: &'static str,
        call_count: AtomicUsize,
    }

    #[async_trait]
    impl ModelClient for MockClient {
        fn provider_name(&self) -> &str {
            self.name
        }
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                id: "resp_1".into(),
                object: "chat.completion".into(),
                created: 0,
                model: format!("{}-model", self.name),
                choices: vec![Choice {
                    index: 0,
                    message: MessageDelta {
                        role: Some(Role::Assistant),
                        content: Some("Hello!".into()),
                        reasoning_content: None,
                        tool_calls: None,
                    },
                    finish_reason: Some("stop".into()),
                }],
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                },
            })
        }
        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
        }
    }

    fn mock_client(name: &'static str) -> Arc<dyn ModelClient> {
        Arc::new(MockClient {
            name,
            call_count: AtomicUsize::new(0),
        })
    }

    /// A third provider that exists only in this test. Registering it is one
    /// `impl ChatProvider` plus one `register` line — no enum variant, no
    /// match arm, no other file.
    struct FakeVendor;
    impl crate::agent::chat_provider::ChatProvider for FakeVendor {
        fn vendor(&self) -> &str {
            "fakevendor"
        }
        fn vendor_aliases(&self) -> &'static [&'static str] {
            &["fv"]
        }
        fn model_prefixes(&self) -> &'static [&'static str] {
            &["fake-model-"]
        }
    }

    #[test]
    fn provider_registry_should_select_a_registered_third_party_provider() {
        // 1 impl (FakeVendor, above) + 1 line:
        let mut catalog = ProviderCatalog::with_builtins();
        catalog.register(FakeVendor);

        let mut clients = HashMap::new();
        clients.insert("openai".to_string(), mock_client("openai"));
        clients.insert("fakevendor".to_string(), mock_client("fakevendor"));

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "fakevendor".to_string(),
                model: "fake-model-1".to_string(),
            },
        ];
        let reg = ProviderRegistry::new(clients, chain).with_catalog(catalog);

        // Selectable: the switch reaches it and hands back a live client.
        let switched = reg
            .switch_to_next()
            .expect("chain must have a second entry");
        assert_eq!(switched.name, "fakevendor");
        assert_eq!(reg.active_provider().as_deref(), Some("fakevendor"));
        assert!(reg.get_client("fakevendor").is_some());

        // Its identity is structural, so the alias and a differently-cased
        // spelling both land on the same vendor and the same client.
        assert_eq!(reg.vendor_of("fv").as_str(), "fakevendor");
        assert_eq!(reg.vendor_of("FakeVendor").as_str(), "fakevendor");
        assert!(reg.get_client("FAKEVENDOR").is_some());
        let key = reg
            .active_key()
            .expect("an active entry has a structural key");
        assert_eq!(key.vendor().as_str(), "fakevendor");
        assert_eq!(key.model(), "fake-model-1");
        // …and the vendor is recovered from a bare model slug, too.
        assert_eq!(
            RuntimeKey::parse_in("fake-model-9", reg.catalog())
                .vendor()
                .as_str(),
            "fakevendor"
        );

        // Cooldown bookkeeping is keyed by the vendor id, not the spelling, so
        // arming under one spelling suppresses lookups under the others.
        reg.arm_cooldown("fakevendor");
        assert!(reg.is_in_cooldown("FAKEVENDOR"));
        assert!(reg.is_in_cooldown("fv"));
        reg.clear_failure_count("fv");
    }

    #[test]
    fn structural_identity_is_derived_from_the_chain_entry() {
        // Uses a built-in vendor so the assertions hold with or without the
        // `anthropic` cargo feature.
        let mut clients = HashMap::new();
        clients.insert("OpenAI".to_string(), mock_client("openai"));
        let chain = vec![ProviderEntry {
            name: "OpenAI".to_string(),
            model: "GPT-4".to_string(),
        }];
        let reg = ProviderRegistry::new(clients, chain);
        let entry = &reg.fallback_chain[0];
        let key = reg.key_for(entry);
        assert_eq!(key.vendor().as_str(), "openai");
        assert_eq!(key.model(), "gpt-4");
        // A registered alias resolves the same client.
        assert!(reg.get_client("oai").is_some());
        assert!(reg.get_client("OpenAI").is_some());
    }

    #[test]
    fn unknown_route_cost_keeps_the_configured_chain_order() {
        // No models.dev cache entry is assumed here, so cost is unknown and the
        // cheap-preferring switch must behave exactly like the plain one.
        let mut clients = HashMap::new();
        clients.insert("a".to_string(), mock_client("a"));
        clients.insert("b".to_string(), mock_client("b"));
        clients.insert("c".to_string(), mock_client("c"));
        let chain = vec![
            ProviderEntry {
                name: "a".into(),
                model: "a-model".into(),
            },
            ProviderEntry {
                name: "b".into(),
                model: "b-model".into(),
            },
            ProviderEntry {
                name: "c".into(),
                model: "c-model".into(),
            },
        ];
        let reg = ProviderRegistry::new(clients, chain);
        assert!(reg.route_cost(&reg.fallback_chain[1]).is_none());
        let picked = reg
            .switch_to_next_cheapest()
            .expect("unknown cost still advances the chain");
        assert_eq!(picked.name, "b");
    }

    #[test]
    fn empty_registry() {
        let reg = ProviderRegistry::empty();
        assert!(!reg.has_providers());
        assert!(reg.active_provider().is_none());
    }

    #[test]
    fn switch_to_next() {
        let mut clients = HashMap::new();
        clients.insert("openai".to_string(), mock_client("openai"));
        clients.insert("anthropic".to_string(), mock_client("anthropic"));

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "anthropic".to_string(),
                model: "claude-3".to_string(),
            },
        ];

        let reg = ProviderRegistry::new(clients, chain);
        assert_eq!(reg.active_provider(), Some("openai".to_string()));

        let switched = reg.switch_to_next().unwrap();
        assert_eq!(switched.name, "anthropic");
        assert_eq!(reg.active_provider(), Some("anthropic".to_string()));

        // Chain exhausted
        assert!(reg.switch_to_next().is_none());
    }

    #[test]
    fn reset_to_primary() {
        let mut clients = HashMap::new();
        clients.insert("openai".to_string(), mock_client("openai"));
        clients.insert("anthropic".to_string(), mock_client("anthropic"));

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "anthropic".to_string(),
                model: "claude-3".to_string(),
            },
        ];

        let reg = ProviderRegistry::new(clients, chain);
        reg.switch_to_next();
        assert_eq!(reg.active_provider(), Some("anthropic".to_string()));

        let reset = reg.reset_to_primary().unwrap();
        assert_eq!(reset.name, "openai");
        assert_eq!(reg.active_provider(), Some("openai".to_string()));
    }
}
