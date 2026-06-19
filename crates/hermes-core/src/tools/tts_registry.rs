use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, warn};

use super::tts_provider::{AudioFormat, SynthesisResult, TtsError, TtsProvider};

/// Built-in provider names that cannot be shadowed by plugins.
/// Kept in sync with the match arms in `TtsTool::generate_speech`.
const BUILTIN_NAMES: &[&str] = &[
    "edge", "elevenlabs", "openai", "minimax", "mistral", "gemini",
    "xai", "neutts", "kittentts", "piper", "kokoro",
];

/// Central map of registered TTS providers.
/// Populated by plugins; consumed by TtsTool for dispatch.
pub struct TtsPluginRegistry {
    providers: RwLock<HashMap<String, Arc<dyn TtsProvider>>>,
}

impl TtsPluginRegistry {
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a TTS provider. Rejects built-in names and non-provider types.
    pub async fn register(&self, provider: Arc<dyn TtsProvider>) -> Result<(), TtsError> {
        let name = provider.name().to_lowercase();
        if name.trim().is_empty() {
            return Err(TtsError::ConfigError(
                "TTS provider name must be non-empty".into(),
            ));
        }
        if BUILTIN_NAMES.contains(&name.as_str()) {
            warn!(
                provider = %name,
                "TTS provider shadows a built-in name; registration ignored"
            );
            return Ok(());
        }
        let mut providers = self.providers.write().await;
        if providers.contains_key(&name) {
            debug!(provider = %name, "TTS provider re-registered");
        }
        providers.insert(name, provider);
        Ok(())
    }

    /// Return all registered providers, sorted by name.
    pub async fn list_providers(&self) -> Vec<Arc<dyn TtsProvider>> {
        let providers = self.providers.read().await;
        let mut items: Vec<_> = providers.values().cloned().collect();
        items.sort_by(|a, b| a.name().cmp(b.name()));
        items
    }

    /// Return the provider registered under `name`, or None.
    pub async fn get_provider(&self, name: &str) -> Option<Arc<dyn TtsProvider>> {
        let providers = self.providers.read().await;
        providers.get(&name.to_lowercase()).cloned()
    }

    /// Check if a name is a built-in provider.
    pub fn is_builtin(name: &str) -> bool {
        BUILTIN_NAMES.contains(&name.to_lowercase().as_str())
    }

    /// Synthesize text using a registered provider.
    pub async fn synthesize(
        &self,
        provider_name: &str,
        text: &str,
        output_path: &str,
        voice: Option<&str>,
        model: Option<&str>,
        format: AudioFormat,
    ) -> Result<SynthesisResult, TtsError> {
        let provider = self
            .get_provider(provider_name)
            .await
            .ok_or_else(|| TtsError::ProviderUnavailable {
                provider: provider_name.to_string(),
                reason: "not registered".into(),
            })?;

        provider
            .synthesize(text, output_path, voice, model, format)
            .await
    }
}

impl Default for TtsPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tts_provider::*;

    struct MockProvider {
        name: String,
    }

    #[async_trait::async_trait]
    impl TtsProvider for MockProvider {
        fn name(&self) -> &str {
            &self.name
        }

        async fn synthesize(
            &self,
            _text: &str,
            output_path: &str,
            _voice: Option<&str>,
            _model: Option<&str>,
            format: AudioFormat,
        ) -> Result<SynthesisResult, TtsError> {
            Ok(SynthesisResult {
                output_path: output_path.to_string(),
                format,
                voice_compatible: false,
            })
        }
    }

    #[tokio::test]
    async fn test_register_and_list() {
        let registry = TtsPluginRegistry::new();
        let provider = Arc::new(MockProvider {
            name: "cartesia".into(),
        });
        registry.register(provider).await.unwrap();

        let providers = registry.list_providers().await;
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name(), "cartesia");
    }

    #[tokio::test]
    async fn test_builtin_name_rejected() {
        let registry = TtsPluginRegistry::new();
        let provider = Arc::new(MockProvider {
            name: "openai".into(),
        });
        registry.register(provider).await.unwrap();
        assert!(registry.get_provider("openai").await.is_none());
    }

    #[tokio::test]
    async fn test_get_provider() {
        let registry = TtsPluginRegistry::new();
        let provider = Arc::new(MockProvider {
            name: "cartesia".into(),
        });
        registry.register(provider).await.unwrap();

        assert!(registry.get_provider("cartesia").await.is_some());
        assert!(registry.get_provider("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_empty_name_rejected() {
        let registry = TtsPluginRegistry::new();
        let provider = Arc::new(MockProvider {
            name: "".into(),
        });
        assert!(registry.register(provider).await.is_err());
    }
}
