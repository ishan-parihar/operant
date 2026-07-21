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
        }
    }

    pub fn empty() -> Self {
        Self {
            clients: HashMap::new(),
            fallback_chain: Vec::new(),
            active_index: Arc::new(std::sync::RwLock::new(0)),
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
    pub fn switch_to_next(&self) -> Option<ProviderEntry> {
        let mut idx = self.active_index.write().ok()?;
        let next = *idx + 1;
        if next >= self.fallback_chain.len() {
            warn!(
                chain_len = self.fallback_chain.len(),
                "Fallback chain exhausted"
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

    /// Reset to the primary provider.
    pub fn reset_to_primary(&self) -> Option<ProviderEntry> {
        let mut idx = self.active_index.write().ok()?;
        if self.fallback_chain.is_empty() {
            return None;
        }
        *idx = 0;
        Some(self.fallback_chain[0].clone())
    }

    pub fn has_providers(&self) -> bool {
        !self.fallback_chain.is_empty()
    }

    pub fn has_next(&self) -> bool {
        let idx = self.active_index.read().map(|g| *g).unwrap_or(0);
        idx + 1 < self.fallback_chain.len()
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

    use crate::client::{ChatResponse, Choice, MessageDelta, Role, Usage};
    use crate::error::{Error, Result};
    use super::super::model_client::{ChatRequest, StreamChunk};

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
                usage: Usage { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
            })
        }
        async fn chat_streaming(&self, _request: ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
        }
    }

    fn mock_client(name: &'static str) -> Arc<dyn ModelClient> {
        Arc::new(MockClient { name, call_count: AtomicUsize::new(0) })
    }

    #[test]
    fn empty_registry() {
        let reg = ProviderRegistry::empty();
        assert!(!reg.has_providers());
        assert!(!reg.has_next());
        assert!(reg.active_provider().is_none());
    }

    #[test]
    fn switch_to_next() {
        let mut clients = HashMap::new();
        clients.insert("openai".to_string(), mock_client("openai"));
        clients.insert("anthropic".to_string(), mock_client("anthropic"));

        let chain = vec![
            ProviderEntry { name: "openai".to_string(), model: "gpt-4".to_string() },
            ProviderEntry { name: "anthropic".to_string(), model: "claude-3".to_string() },
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
            ProviderEntry { name: "openai".to_string(), model: "gpt-4".to_string() },
            ProviderEntry { name: "anthropic".to_string(), model: "claude-3".to_string() },
        ];

        let reg = ProviderRegistry::new(clients, chain);
        reg.switch_to_next();
        assert_eq!(reg.active_provider(), Some("anthropic".to_string()));

        let reset = reg.reset_to_primary().unwrap();
        assert_eq!(reset.name, "openai");
        assert_eq!(reg.active_provider(), Some("openai".to_string()));
    }
}
