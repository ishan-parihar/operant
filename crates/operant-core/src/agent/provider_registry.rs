//! Provider registry for cross-provider fallback.
//!
//! When the primary provider fails with auth/billing errors, the agent can
//! switch to a different provider (e.g., Anthropic → OpenAI) by selecting a
//! pre-configured client from the registry.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{info, warn};

use super::model_client::ModelClient;

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
    /// Anti-thrash: cooldown timestamps per provider (provider_name -> cooldown_until).
    cooldowns: Arc<std::sync::RwLock<HashMap<String, f64>>>,
    /// Anti-thrash: failure count per provider for exponential backoff.
    failure_counts: Arc<std::sync::RwLock<HashMap<String, usize>>>,
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
        }
    }

    pub fn empty() -> Self {
        Self {
            clients: HashMap::new(),
            fallback_chain: Vec::new(),
            active_index: Arc::new(std::sync::RwLock::new(0)),
            cooldowns: Arc::new(std::sync::RwLock::new(HashMap::new())),
            failure_counts: Arc::new(std::sync::RwLock::new(HashMap::new())),
        }
    }

    fn now_secs() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    /// Check if a provider is currently in cooldown.
    pub fn is_in_cooldown(&self, provider_name: &str) -> bool {
        match self.cooldowns.read() {
            Ok(cooldowns) => {
                if let Some(&until) = cooldowns.get(provider_name) {
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
        // Increment failure count and compute exponential backoff.
        let count = match self.failure_counts.write() {
            Ok(mut counts) => {
                let entry = counts.entry(provider_name.to_string()).or_insert(0);
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
            cooldowns.insert(provider_name.to_string(), until);
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
            counts.remove(provider_name);
        }
    }

    pub fn get_client(&self, provider_name: &str) -> Option<Arc<dyn ModelClient>> {
        self.clients.get(provider_name).cloned()
    }

    pub fn active_provider(&self) -> Option<String> {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        self.fallback_chain.get(idx).map(|e| e.name.clone())
    }

    pub fn active_model(&self) -> Option<String> {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        self.fallback_chain.get(idx).map(|e| e.model.clone())
    }

    /// Switch to the next provider in the fallback chain.
    /// Skips providers that are currently in cooldown.
    pub fn switch_to_next(&self) -> Option<ProviderEntry> {
        let mut idx = match self.active_index.write() {
            Ok(g) => g,
            Err(_) => {
                warn!("Lock poisoned while switching provider — cannot advance");
                return None;
            }
        };
        // Skip providers that are in cooldown.
        let mut next = *idx + 1;
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
        let entry = self.fallback_chain[next].clone();
        *idx = next;
        info!(
            provider = %entry.name,
            model = %entry.model,
            "Switched to fallback provider"
        );
        Some(entry)
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
