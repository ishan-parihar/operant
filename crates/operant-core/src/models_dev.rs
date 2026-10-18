//! Models.dev registry — offline-first model catalog with capability metadata.
//!
//! Ported from operant-agent's `agent/models_dev.py`.
//! Provides model lookup, capability queries, and agentic model filtering.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Mapping from Operant provider names to models.dev provider IDs.
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

/// After a failed network fetch, skip the network for this long. Without it
/// an unreachable models.dev made every caller (the per-turn cost lookup
/// included) wait out the full request timeout, every time.
const NETWORK_RETRY_BACKOFF_SECS: u64 = 300;

/// Unix seconds of the last failed network fetch (0 = none).
static LAST_NETWORK_FAILURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn cache_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_default();
    path.push(".operant");
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
pub async fn fetch_models_dev(
    force_refresh: bool,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    let cache_path = cache_path();

    if !force_refresh
        && let Ok(content) = fs::read_to_string(&cache_path)
        && let Ok(cache) = serde_json::from_str::<ModelsDevCache>(&content)
    {
        let age = now_secs() - cache.cached_at;
        if age < CACHE_TTL_SECS {
            return Ok((cache.models, cache.providers));
        }
        // Stale: refresh, falling back to the stale copy if offline.
        return Ok(fetch_and_store(&cache_path)
            .await
            .unwrap_or((cache.models, cache.providers)));
    }

    fetch_and_store(&cache_path).await
}

/// Network fetch that writes the disk cache on success — the cold path
/// used to skip the write, so a box without a cache refetched forever.
async fn fetch_and_store(
    cache_path: &PathBuf,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    let (models, providers) = fetch_from_network().await?;
    let new_cache = ModelsDevCache {
        models: models.clone(),
        providers: providers.clone(),
        cached_at: now_secs(),
    };
    if let Ok(json) = serde_json::to_string(&new_cache) {
        let _ = fs::write(cache_path, json);
    }
    Ok((models, providers))
}

async fn fetch_from_network() -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    use std::sync::atomic::Ordering;
    let last = LAST_NETWORK_FAILURE.load(Ordering::Relaxed);
    if last != 0 && (now_secs() as u64).saturating_sub(last) < NETWORK_RETRY_BACKOFF_SECS {
        return Err("models.dev fetch failed recently; backing off".to_string());
    }
    let result = fetch_from_network_uncached().await;
    if result.is_err() {
        LAST_NETWORK_FAILURE.store(now_secs() as u64, Ordering::Relaxed);
    }
    result
}

async fn fetch_from_network_uncached()
-> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(MODELS_DEV_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("models.dev returned {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let (models, providers) = normalize_catalog(&body);
    if models.is_empty() {
        return Err("models.dev catalog had no models".to_string());
    }
    Ok((models, providers))
}

/// models.dev's full catalog: a provider-keyed object, one entry per provider.
const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// Flatten models.dev's provider-keyed catalog into the flat rows the
/// capability readers use (`provider_id`, `context_window`, `cost_input`, ...).
///
/// The nested schema keeps the window at `limit.context` and the price at
/// `cost.input` (USD per million tokens); the readers used to expect flat
/// keys, so every real row read as "unknown".
pub(crate) fn normalize_catalog(
    root: &serde_json::Value,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    use serde_json::{Value, json};

    let mut models = Vec::new();
    let mut providers = Vec::new();
    let Some(root) = root.as_object() else {
        return (models, providers);
    };
    let at = |m: &Value, path: &str| m.pointer(path).cloned().unwrap_or(Value::Null);

    for (provider_key, provider) in root {
        let Some(provider) = provider.as_object() else {
            continue;
        };
        providers.push(json!({
            "id": provider_key,
            "name": provider.get("name").cloned().unwrap_or(Value::Null),
            "env": provider.get("env").cloned().unwrap_or(Value::Null),
            "doc": provider.get("doc").cloned().unwrap_or(Value::Null),
        }));
        let Some(entries) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (model_key, m) in entries {
            models.push(json!({
                "provider_id": provider_key,
                "id": m.get("id").and_then(Value::as_str).unwrap_or(model_key),
                "name": at(m, "/name"),
                "context_window": at(m, "/limit/context"),
                "max_output": at(m, "/limit/output"),
                "tool_call": m.get("tool_call").and_then(Value::as_bool).unwrap_or(false),
                "reasoning": m.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
                "structured_output": m
                    .get("structured_output")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "input_modalities": at(m, "/modalities/input"),
                "output_modalities": at(m, "/modalities/output"),
                "cost_input": at(m, "/cost/input"),
                "cost_output": at(m, "/cost/output"),
            }));
        }
    }
    (models, providers)
}

/// Look up context window for a provider+model combo.
pub async fn lookup_models_dev_context(provider: &str, model: &str) -> Option<u64> {
    let caps = get_model_capabilities(provider, model).await?;
    caps.context_window
}

/// Look up full capability metadata for a provider+model combo.
pub async fn get_model_capabilities(provider: &str, model: &str) -> Option<ModelCapabilities> {
    // Map first: an unmapped provider can never match, so it must not pay
    // for a (possibly 15s-timeout) catalog fetch on every turn.
    let dev_provider = provider_to_models_dev(provider)?;
    let (models, _) = fetch_models_dev(false).await.ok()?;
    match_capabilities(&models, dev_provider, model)
}

/// Synchronous capability lookup against the on-disk models.dev cache.
///
/// Reads the same cache [`get_model_capabilities`] populates — no network, no
/// second pricing table. Exists for the synchronous fallback path, where
/// awaiting a fetch is not an option. `None` when the cache is cold or the
/// model is absent; callers treat that as "price unknown" and keep their
/// configured ordering rather than guessing one.
pub fn cached_capabilities(provider: &str, model: &str) -> Option<ModelCapabilities> {
    let dev_provider = provider_to_models_dev(provider)?;
    let content = fs::read_to_string(cache_path()).ok()?;
    let cache: ModelsDevCache = serde_json::from_str(&content).ok()?;
    match_capabilities(&cache.models, dev_provider, model)
}

/// Find one model's capabilities in an already-fetched catalog slice.
fn match_capabilities(
    models: &[serde_json::Value],
    dev_provider: &str,
    model: &str,
) -> Option<ModelCapabilities> {
    for m in models {
        // A malformed row is skipped, not allowed to hide the rows after it.
        let (Some(m_provider), Some(m_id)) = (
            m.get("provider_id").and_then(|v| v.as_str()),
            m.get("id").and_then(|v| v.as_str()),
        ) else {
            continue;
        };

        if m_provider == dev_provider && (m_id == model || m_id.ends_with(&format!("/{model}"))) {
            return Some(ModelCapabilities {
                context_window: m.get("context_window").and_then(|v| v.as_u64()),
                max_output: m.get("max_output").and_then(|v| v.as_u64()),
                supports_tools: m
                    .get("tool_call")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                supports_vision: m
                    .get("input_modalities")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().any(|v| v.as_str() == Some("image")))
                    .unwrap_or(false),
                supports_reasoning: m
                    .get("reasoning")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
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
                cost_input_per_million: m.get("cost_input").and_then(|v| v.as_f64()),
                cost_output_per_million: m.get("cost_output").and_then(|v| v.as_f64()),
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
    agentic_ids(&models, provider)
}

/// Agentic model ids for `provider` read from the on-disk cache only: no network,
/// any age. Empty when there is no cache or it does not parse.
pub fn cached_agentic_models(provider: &str) -> Vec<String> {
    let Ok(content) = fs::read_to_string(cache_path()) else {
        return Vec::new();
    };
    let Ok(cache) = serde_json::from_str::<ModelsDevCache>(&content) else {
        return Vec::new();
    };
    agentic_ids(&cache.models, provider)
}

/// The normalized rows of `provider` that support tool calls, in catalog order,
/// with TTS, embedding, and other non-chat models dropped.
fn agentic_ids(models: &[serde_json::Value], provider: &str) -> Vec<String> {
    let noise_keywords = [
        "tts",
        "text-to-speech",
        "embedding",
        "noise",
        "whisper",
        "music",
        "audio-generation",
        "speech-to-text",
        "stt",
    ];

    let Some(dev_provider) = provider_to_models_dev(provider) else {
        return Vec::new();
    };

    models
        .iter()
        .filter_map(|m| {
            let m_provider = m.get("provider_id").and_then(|v| v.as_str())?;
            if m_provider != dev_provider {
                return None;
            }

            let tool_call = m
                .get("tool_call")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
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
    fn agentic_ids_keep_tool_callers_for_the_provider_and_drop_noise() {
        let models = vec![
            serde_json::json!({"provider_id": "anthropic", "id": "claude-a", "tool_call": true}),
            serde_json::json!({"provider_id": "anthropic", "id": "claude-b", "tool_call": false}),
            serde_json::json!({"provider_id": "anthropic", "id": "tts-x", "tool_call": true}),
            serde_json::json!({"provider_id": "openai", "id": "gpt-x", "tool_call": true}),
        ];
        assert_eq!(agentic_ids(&models, "anthropic"), ["claude-a"]);
        assert!(agentic_ids(&models, "no-such-provider").is_empty());
    }

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

    /// Rows copied from models.dev's nested schema (values as published).
    fn nested_fixture() -> serde_json::Value {
        serde_json::json!({
            "anthropic": {
                "id": "anthropic", "name": "Anthropic", "env": ["ANTHROPIC_API_KEY"],
                "models": {
                    "claude-haiku-4-5": {
                        "id": "claude-haiku-4-5", "name": "Claude Haiku 4.5 (latest)",
                        "reasoning": true, "tool_call": true, "structured_output": true,
                        "modalities": {"input": ["text", "image", "pdf"], "output": ["text"]},
                        "limit": {"context": 200000, "output": 64000},
                        "cost": {"input": 1, "output": 5, "cache_read": 0.1}
                    }
                }
            },
            "openai": {
                "id": "openai", "name": "OpenAI", "env": ["OPENAI_API_KEY"],
                "models": {
                    "gpt-4o": {
                        "id": "gpt-4o", "reasoning": false, "tool_call": true,
                        "modalities": {"input": ["text", "image"], "output": ["text"]},
                        "limit": {"context": 128000, "output": 16384},
                        "cost": {"input": 2.5, "output": 10}
                    }
                }
            }
        })
    }

    #[test]
    fn normalize_flattens_the_nested_schema_into_the_reader_shape() {
        let (models, providers) = normalize_catalog(&nested_fixture());
        assert_eq!(providers.len(), 2);
        assert_eq!(models.len(), 2);
        let haiku = models
            .iter()
            .find(|m| m["id"] == "claude-haiku-4-5")
            .unwrap();
        assert_eq!(haiku["provider_id"], "anthropic");
        assert_eq!(haiku["context_window"], 200000);
        assert_eq!(haiku["max_output"], 64000);
        assert_eq!(haiku["cost_input"], 1.0);
        assert_eq!(haiku["cost_output"], 5.0);
    }

    #[test]
    fn capabilities_are_read_from_a_normalized_catalog() {
        let (models, _) = normalize_catalog(&nested_fixture());
        let caps = match_capabilities(&models, "anthropic", "claude-haiku-4-5").unwrap();
        assert_eq!(caps.context_window, Some(200000));
        assert_eq!(caps.max_output, Some(64000));
        assert!(caps.supports_tools && caps.supports_reasoning && caps.supports_vision);
        assert_eq!(caps.cost_output_per_million, Some(5.0));
        assert!(match_capabilities(&models, "openai", "claude-haiku-4-5").is_none());
    }

    #[test]
    fn a_malformed_row_does_not_hide_the_rows_after_it() {
        let (mut models, _) = normalize_catalog(&nested_fixture());
        models.insert(0, serde_json::json!({"id": "orphan-without-provider"}));
        assert!(match_capabilities(&models, "anthropic", "claude-haiku-4-5").is_some());
    }
}
