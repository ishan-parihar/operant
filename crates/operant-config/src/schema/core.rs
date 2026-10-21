//! `core` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Named provider profile definition.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ModelProviderConfig {
    /// API token for this provider — grab it from the provider's dashboard
    /// (OpenAI platform, Anthropic console, OpenRouter keys page, etc.).
    ///
    /// **It is written to `operant.toml` in PLAINTEXT** (the file itself is
    /// 0600, owner-only). It used to be marked `#[secret]`, and the field
    /// claimed the key was "stored via the OS keyring when possible" — but
    /// the only code that honoured that marker was the config AGGREGATE's
    /// `save()`, which was never loaded in production (RT-141). The live
    /// write path (`operant_core::config::persist_config_key`) writes the
    /// value as given, and the live loader never decrypts one, so a key
    /// here is plaintext. The ENCRYPTED store is the provider auth-profile
    /// file (`operant-providers`' `auth-profiles.json`, written through
    /// `SecretStore`); use that for long-lived OAuth tokens, and keep
    /// `operant.toml` out of a shared git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Additional API keys for this provider's credential pool (hermes
    /// multi-key-per-provider parity). The primary `api_key` is tried first;
    /// these rotate in on 401/429/billing exhaustion, each with its own
    /// error-class bench (hermes `load_pool(provider)` seeds every key a
    /// provider owns). Empty by default.
    ///
    /// Plaintext in `operant.toml`, exactly like `api_key` above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<String>,
    /// Override the provider type label. Rarely needed — only useful when you run two profiles against the same provider type (e.g. two different OpenAI-compatible gateways) and want to tell them apart in logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// HTTPS endpoint the client hits. Override when pointing at a self-hosted gateway (LiteLLM, vLLM, Ollama), a regional endpoint, or a proxy; leave unset to use the provider's public endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Path suffix appended to the base URL. Almost no one needs this — only touch it for custom reverse-proxy routing where your gateway mounts the API under a non-standard prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_path: Option<String>,
    /// Model identifier to send with each request — the ID string from the provider's catalog (e.g. `gpt-4o`, `claude-sonnet-4-5`, `llama-3.3-70b`). Must match a model the provider actually serves on this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Sampling temperature passed to the model. Lower values (0.0–0.3) give
    /// deterministic, near-verbatim output — fits code, routing, summarization.
    /// Higher values (0.7–1.2) give more varied output — fits open-ended chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// HTTP request timeout in seconds. Bump this for slow local providers (Ollama on CPU, big local models) or high-latency networks; leave unset otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Extra HTTP headers sent with every request. Niche — used for auth bridges, corporate proxies, or custom gateways that demand a tracing header. Most users never touch this; edit `config.toml` directly if you need it.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra_headers: HashMap<String, String>,
    /// Wire protocol flavor: `"responses"` for OpenAI's Codex/Responses API, `"chat_completions"` for everything else (OpenAI chat, Anthropic, OpenRouter, Groq, local gateways). Auto-selected per provider — only override if you're forcing an unusual combination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_api: Option<String>,
    /// When true, the client pulls credentials from `OPENAI_API_KEY` or `~/.codex/auth.json` instead of the `api_key` field above. Turn on only for the OpenAI Codex provider; leave off for standard API-key providers.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_openai_auth: bool,
    /// Azure OpenAI resource name (the `<resource>` part of `<resource>.openai.azure.com`). Azure-only; ignore for OpenAI, Anthropic, and everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure_openai_resource: Option<String>,
    /// Azure OpenAI deployment name — the deployment you created in Azure AI Studio that wraps a specific model. Azure-only; ignore for other providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure_openai_deployment: Option<String>,
    /// Azure OpenAI API version string (e.g. `2024-10-21`). Azure-only; must match a version your resource supports. Ignore for other providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure_openai_api_version: Option<String>,
    /// Hard cap on response length in tokens. Most models enforce sensible built-in limits already — leave unset unless you specifically need to clip long outputs for cost or latency reasons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Provider-specific quirk: fold the system prompt into the first user message instead of sending a separate system role. Only needed for models that reject (or mishandle) a standalone system role — e.g. certain older Mistral variants.
    #[serde(default, skip_serializing_if = "is_false")]
    pub merge_system_into_user: bool,
    /// Extra JSON parameters to include in API requests.
    /// Merged at the top level of the request body, allowing provider-specific
    /// features (routing, transforms, etc.) without code changes.
    /// Example: `provider_extra = { provider = { only = ["Anthropic"] } }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_extra: Option<serde_json::Value>,
    /// Per-provider input/output token pricing (USD per 1M tokens). When set,
    /// merged into the cost-tracking lookup at `<provider_id>/<model>` so the
    /// budget surface attributes spend correctly even when the same model is
    /// served by different providers at different rates. Top-level
    /// `[cost.prices.<key>]` entries continue to take precedence on conflict;
    /// this field is purely additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    /// Override the provider's default for native tool calling.
    /// `None` (default) honors the provider's built-in choice. `Some(true)`
    /// forces native tool calls on, `Some(false)` forces text-fallback.
    /// Currently consulted only by the Groq factory, which defaults to
    /// text-fallback because llama-family Groq models reject native tool
    /// calls with HTTP 400. Setting `native_tools = true` re-enables native
    /// tool calling for Groq models that support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_tools: Option<bool>,
    /// Enable or disable chain-of-thought thinking for models that support it
    /// (e.g. Qwen3, GLM-4). `true` turns thinking on, `false` turns it off.
    /// `None` (default) lets the model decide. Forwarded as `enable_thinking`
    /// in the request body; mirrors the Ollama provider's `think` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think: Option<bool>,
    /// Arbitrary key/value pairs forwarded verbatim as `chat_template_kwargs`
    /// in the request body (llama.cpp-specific). Use this to pass model-family
    /// template variables that control behaviour not exposed by other fields.
    /// Example (Qwen3 thinking suppression):
    ///   `chat_template_kwargs = { enable_thinking = false }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<serde_json::Value>,
    /// Override the Ollama `num_ctx` (context window, in tokens) sent on
    /// every `/api/chat` request. Only consulted when this profile resolves
    /// to the `ollama` provider. Defaults to the framework constant
    /// (`OLLAMA_DEFAULT_NUM_CTX`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_num_ctx: Option<u32>,
    /// Override the Ollama `num_predict` (max output tokens) sent on every
    /// `/api/chat` request. Only consulted when this profile resolves to
    /// the `ollama` provider. Defaults to the framework constant
    /// (`OLLAMA_DEFAULT_NUM_PREDICT`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_num_predict: Option<i32>,
    /// Force every Ollama `/api/chat` request to use this temperature,
    /// overriding the per-call value passed through
    /// `Provider::chat_with_system(.., temperature)`. When unset
    /// (`None`, the default), the per-call temperature wins — full
    /// backward compatibility. Only consulted when this profile
    /// resolves to the `ollama` provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_temperature_override: Option<f64>,
}

/// Valid temperature range for all paths (config, CLI, env override).
pub const TEMPERATURE_RANGE: std::ops::RangeInclusive<f64> = 0.0..=2.0;

pub(crate) fn normalize_service_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = normalize_comma_values(values)
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}
