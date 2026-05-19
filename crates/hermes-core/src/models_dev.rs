//! Models.dev registry — offline-first model catalog with capability metadata.
//!
//! Ported from hermes-agent's `agent/models_dev.py`.
//! Provides model lookup, capability queries, and agentic model filtering.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Mapping from Hermes provider names to models.dev provider IDs.
pub fn provider_to_models_dev(provider: &str) -> Option<&'static str> {
    match provider.to_lowercase().as_str() {
        "anthropic" => Some("anthropic"),
        "openai" => Some("openai"),
        "openrouter" => Some("openrouter"),
        "nous" => Some("nous"),
        "google" | "gemini" => Some("google"),
        "deepseek" => Some("deepseek"),
        "groq" => Some("groq"),
        "fireworks" => Some("fireworks-ai"),
        "together" => Some("together"),
        "xai" | "grok" => Some("xai"),
        "mistral" => Some("mistral"),
        "cohere" => Some("cohere"),
        "perplexity" => Some("perplexity-ai"),
        "cerebras" => Some("cerebras"),
        _ => None,
    }
}

/// Model capability metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub context_window: Option<u64>,
    pub max_output: Option<u64>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
    pub supports_structured_output: bool,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub cost_input_per_million: Option<f64>,
    pub cost_output_per_million: Option<f64>,
}

/// Disk cache entry for models.dev data.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsDevCache {
    models: Vec<serde_json::Value>,
    providers: Vec<serde_json::Value>,
    cached_at: f64,
}

const CACHE_TTL_SECS: f64 = 3600.0; // 1 hour

fn cache_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_default();
    path.push(".hermes");
    path.push("models_dev_cache.json");
    path
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Fetch models.dev catalog with offline-first cache hierarchy:
/// in-memory → disk cache → network → stale disk fallback.
pub async fn fetch_models_dev(force_refresh: bool) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    let cache_path = cache_path();

    if !force_refresh {
        if let Ok(content) = fs::read_to_string(&cache_path) {
            if let Ok(cache) = serde_json::from_str::<ModelsDevCache>(&content) {
                let age = now_secs() - cache.cached_at;
                if age < CACHE_TTL_SECS {
                    return Ok((cache.models, cache.providers));
                }
                let stale = Some(cache);
                match fetch_from_network().await {
                    Ok((models, providers)) => {
                        let new_cache = ModelsDevCache {
                            models: models.clone(),
                            providers: providers.clone(),
                            cached_at: now_secs(),
                        };
                        if let Ok(json) = serde_json::to_string(&new_cache) {
                            let _ = fs::write(&cache_path, json);
                        }
                        return Ok((models, providers));
                    }
                    Err(_) => {
                        if let Some(stale) = stale {
                            return Ok((stale.models, stale.providers));
                        }
                    }
                }
            }
        }
    }

    fetch_from_network().await
}

async fn fetch_from_network() -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get("https://models.dev/api/v1/models")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("models.dev returned {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let models = body
        .get("models")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let providers = body
        .get("providers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok((models, providers))
}

/// Look up context window for a provider+model combo.
pub async fn lookup_models_dev_context(provider: &str, model: &str) -> Option<u64> {
    let caps = get_model_capabilities(provider, model).await?;
    caps.context_window
}

/// Look up full capability metadata for a provider+model combo.
pub async fn get_model_capabilities(provider: &str, model: &str) -> Option<ModelCapabilities> {
    let (models, _) = fetch_models_dev(false).await.ok()?;

    let dev_provider = provider_to_models_dev(provider)?;

    for m in &models {
        let m_provider = m.get("provider_id").and_then(|v| v.as_str())?;
        let m_id = m.get("id").and_then(|v| v.as_str())?;

        if m_provider == dev_provider && (m_id == model || m_id.ends_with(&format!("/{model}"))) {
            return Some(ModelCapabilities {
                context_window: m.get("context_window").and_then(|v| v.as_u64()),
                max_output: m.get("max_output").and_then(|v| v.as_u64()),
                supports_tools: m.get("tool_call").and_then(|v| v.as_bool()).unwrap_or(false),
                supports_vision: m
                    .get("input_modalities")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().any(|v| v.as_str() == Some("image")))
                    .unwrap_or(false),
                supports_reasoning: m.get("reasoning").and_then(|v| v.as_bool()).unwrap_or(false),
                supports_structured_output: m
                    .get("structured_output")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                input_modalities: m
                    .get("input_modalities")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                output_modalities: m
                    .get("output_modalities")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                cost_input_per_million: m
                    .get("cost_input")
                    .and_then(|v| v.as_f64()),
                cost_output_per_million: m
                    .get("cost_output")
                    .and_then(|v| v.as_f64()),
            });
        }
    }

    None
}

/// List models suitable for agentic use (tool_call=True, exclude TTS/embedding/noise).
pub async fn list_agentic_models(provider: &str) -> Vec<String> {
    let (models, _) = match fetch_models_dev(false).await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let dev_provider = match provider_to_models_dev(provider) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let noise_keywords = [
        "tts", "text-to-speech", "embedding", "noise", "whisper",
        "music", "audio-generation", "speech-to-text", "stt",
    ];

    models
        .iter()
        .filter_map(|m| {
            let m_provider = m.get("provider_id").and_then(|v| v.as_str())?;
            if m_provider != dev_provider {
                return None;
            }

            let tool_call = m.get("tool_call").and_then(|v| v.as_bool()).unwrap_or(false);
            if !tool_call {
                return None;
            }

            let model_id = m.get("id").and_then(|v| v.as_str())?;
            let lower = model_id.to_lowercase();
            if noise_keywords.iter().any(|k| lower.contains(k)) {
                return None;
            }

            Some(model_id.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_mapping() {
        assert_eq!(provider_to_models_dev("anthropic"), Some("anthropic"));
        assert_eq!(provider_to_models_dev("openrouter"), Some("openrouter"));
        assert_eq!(provider_to_models_dev("google"), Some("google"));
        assert_eq!(provider_to_models_dev("unknown"), None);
    }

    #[test]
    fn test_cache_path() {
        let path = cache_path();
        assert!(path.to_string_lossy().contains("models_dev_cache"));
    }
}
