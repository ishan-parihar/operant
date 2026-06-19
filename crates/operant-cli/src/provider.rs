//! Canonical provider registry for Operant-RS CLI.
//!
//! This module centralises ALL provider metadata that was previously scattered
//! across `cmd_setup.rs`, `cmd_auth.rs`, and `config.rs`.  It is the single
//! source of truth for provider definitions, model lists, environment-variable
//! names, base URLs, signup URLs, and default models.
//!
//! # Architecture
//!
//! Two layers coexist:
//!
//! - **Static layer** (`ProviderDef` + `PROVIDERS`): zero-overhead `const` data
//!   for the built-in provider catalog.  Lookup via `provider_by_name()`.
//!
//! - **Dynamic layer** (`ProviderProfile` trait + `ProviderRegistry`): trait-based
//!   profiles with overridable methods (`prepare_messages`, `build_extra_body`,
//!   etc.) and runtime registration.  Matches Python's `ProviderProfile` class.
//!
//! # Usage
//!
//! ```ignore
//! use crate::provider::{ProviderDef, PROVIDERS, ProviderRegistry, ProviderProfile};
//!
//! // Static lookup (unchanged)
//! for p in PROVIDERS {
//!     println!("{}: {}", p.display_name, p.default_base_url);
//! }
//!
//! // Dynamic registry
//! let registry = ProviderRegistry::new();
//! registry.register(Arc::new(MyCustomProvider));
//! if let Some(profile) = registry.get("my_custom") {
//!     println!("base_url: {:?}", profile.base_url());
//! }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Dynamic provider profile trait
// ---------------------------------------------------------------------------

/// Trait for provider-specific behavior overrides.
///
/// Methods have default implementations that delegate to static `ProviderDef`
/// fields, so providers that need no customization work out of the box.
/// Override individual methods for providers with quirks (e.g. Anthropic's
/// header requirements, Kimi's temperature omission).
#[async_trait]
pub trait ProviderProfile: Send + Sync {
    fn name(&self) -> &str;
    fn display_name(&self) -> &str {
        self.name()
    }
    fn description(&self) -> &str {
        ""
    }
    fn base_url(&self) -> Option<&str> {
        None
    }
    fn api_key_env(&self) -> Option<&str> {
        None
    }
    fn default_model(&self) -> &str {
        ""
    }
    fn supports_vision(&self) -> bool {
        false
    }
    fn supports_streaming(&self) -> bool {
        true
    }
    fn auth_type(&self) -> &str {
        "api_key"
    }
    fn signup_url(&self) -> &str {
        ""
    }
    fn aliases(&self) -> Vec<&str> {
        vec![]
    }

    fn get_max_tokens(&self) -> Option<usize> {
        None
    }
    fn fallback_models(&self) -> Vec<String> {
        vec![]
    }

    fn prepare_messages(
        &self,
        messages: Vec<operant_core::client::Message>,
    ) -> Vec<operant_core::client::Message> {
        messages
    }

    fn build_extra_body(&self) -> Value {
        Value::Object(serde_json::Map::new())
    }

    async fn fetch_models(
        &self,
        _api_key: Option<&str>,
        _base_url: Option<&str>,
    ) -> Option<Vec<String>> {
        None
    }
}

/// Bridge: wraps a static `ProviderDef` to implement `ProviderProfile`.
pub struct StaticProviderProfile(&'static ProviderDef);

impl StaticProviderProfile {
    pub fn new(def: &'static ProviderDef) -> Self {
        Self(def)
    }
}

#[async_trait]
impl ProviderProfile for StaticProviderProfile {
    fn name(&self) -> &str {
        self.0.name
    }
    fn display_name(&self) -> &str {
        self.0.display_name
    }
    fn description(&self) -> &str {
        self.0.description
    }
    fn base_url(&self) -> Option<&str> {
        Some(self.0.default_base_url)
    }
    fn api_key_env(&self) -> Option<&str> {
        if self.0.env_var.is_empty() {
            None
        } else {
            Some(self.0.env_var)
        }
    }
    fn default_model(&self) -> &str {
        self.0.default_model
    }
    fn auth_type(&self) -> &str {
        self.0.auth_type
    }
    fn signup_url(&self) -> &str {
        self.0.signup_url
    }
    fn aliases(&self) -> Vec<&str> {
        vec![]
    }

    fn supports_vision(&self) -> bool {
        matches!(
            self.0.name,
            "openai" | "anthropic" | "google" | "xai" | "mistral"
        )
    }

    fn fallback_models(&self) -> Vec<String> {
        self.0.models.iter().map(|s| s.to_string()).collect()
    }

    async fn fetch_models(
        &self,
        api_key: Option<&str>,
        _base_url: Option<&str>,
    ) -> Option<Vec<String>> {
        let key = api_key?;
        Some(fetch_models_for_provider(self.0, key).await)
    }
}

// ---------------------------------------------------------------------------
// Dynamic provider registry
// ---------------------------------------------------------------------------

/// Thread-safe registry for dynamic provider profiles.
pub struct ProviderRegistry {
    profiles: RwLock<HashMap<String, Arc<dyn ProviderProfile>>>,
    aliases: RwLock<HashMap<String, String>>,
    fallback_chains: RwLock<Vec<Vec<String>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            profiles: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashMap::new()),
            fallback_chains: RwLock::new(Vec::new()),
        }
    }

    pub fn register(&self, profile: Arc<dyn ProviderProfile>) {
        let name = profile.name().to_string();
        let aliases: Vec<String> = profile.aliases().into_iter().map(String::from).collect();
        let mut profiles = self.profiles.write().unwrap();
        profiles.insert(name.clone(), profile);
        let mut al = self.aliases.write().unwrap();
        for alias in aliases {
            al.insert(alias, name.clone());
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ProviderProfile>> {
        let profiles = self.profiles.read().unwrap();
        if let Some(p) = profiles.get(name) {
            return Some(Arc::clone(p));
        }
        let al = self.aliases.read().unwrap();
        if let Some(resolved) = al.get(name) {
            return profiles.get(resolved).cloned();
        }
        None
    }

    pub fn list(&self) -> Vec<Arc<dyn ProviderProfile>> {
        let profiles = self.profiles.read().unwrap();
        profiles.values().cloned().collect()
    }

    pub fn resolve_alias(&self, name: &str) -> Option<String> {
        let al = self.aliases.read().unwrap();
        al.get(name).cloned()
    }

    pub fn add_alias(&self, alias: String, target: String) {
        let mut al = self.aliases.write().unwrap();
        al.insert(alias, target);
    }

    pub fn set_fallback_chain(&self, chain: Vec<String>) {
        let mut chains = self.fallback_chains.write().unwrap();
        chains.push(chain);
    }

    pub fn get_fallback_chain(&self, name: &str) -> Option<Vec<String>> {
        let chains = self.fallback_chains.read().unwrap();
        chains
            .iter()
            .find(|c| c.first().map(|s| s.as_str()) == Some(name))
            .cloned()
    }

    pub fn resolve_with_fallback(&self, name: &str) -> Vec<Arc<dyn ProviderProfile>> {
        let mut result = vec![];
        if let Some(p) = self.get(name) {
            result.push(p);
        }
        if let Some(chain) = self.get_fallback_chain(name) {
            for n in chain.iter().skip(1) {
                if let Some(p) = self.get(n) {
                    result.push(p);
                }
            }
        }
        result
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global provider registry instance.
pub fn global_registry() -> &'static ProviderRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<ProviderRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = ProviderRegistry::new();
        for def in PROVIDERS {
            registry
                .register(Arc::new(StaticProviderProfile::new(def)) as Arc<dyn ProviderProfile>);
        }
        registry.add_alias("claude".to_string(), "anthropic".to_string());
        registry.add_alias("gpt".to_string(), "openai".to_string());
        registry.add_alias("gemini".to_string(), "google".to_string());
        registry.add_alias("grok".to_string(), "xai".to_string());
        registry.add_alias("qwen".to_string(), "alibaba".to_string());
        registry.add_alias("glm".to_string(), "zai".to_string());
        registry.add_alias("kimi".to_string(), "kimi-coding".to_string());
        registry.add_alias("llama".to_string(), "groq".to_string());
        registry.add_alias("sonar".to_string(), "perplexity".to_string());
        registry.add_alias("command-r".to_string(), "cohere".to_string());
        registry
    })
}

/// Static definition of an LLM provider.
///
/// Each field is `&'static str` / `&'static [&'static str]` so that the entire
/// registry is a single `const` slice with zero runtime overhead.
#[derive(Copy, Clone)]
pub struct ProviderDef {
    /// Machine-friendly provider key (e.g. `"openai"`, `"anthropic"`).
    pub name: &'static str,
    /// Human-readable display name (e.g. `"OpenAI"`, `"Anthropic"`).
    pub display_name: &'static str,
    /// Short description highlighting the provider's unique value.
    pub description: &'static str,
    /// Default API base URL.
    pub default_base_url: &'static str,
    /// Default model identifier.
    pub default_model: &'static str,
    /// Primary environment variable for the API key / token.
    ///
    /// This is the "canonical" env var for the provider and is always present
    /// in the `env_vars` list as well.
    pub env_var: &'static str,
    /// All relevant environment variables for this provider (key, URL override,
    /// region, etc.).
    pub env_vars: &'static [&'static str],
    /// URL where users can obtain an API key (or sign up / learn more).
    pub signup_url: &'static str,
    /// Authentication type: `"api_key"`, `"aws"`, `"oauth"`, or `"none"`.
    pub auth_type: &'static str,
    /// Recommended / commonly-used model identifiers for this provider.
    pub models: &'static [&'static str],
}

// ---------------------------------------------------------------------------
// Canonical provider list
// ---------------------------------------------------------------------------

/// All known LLM providers supported by Operant-RS.
///
/// Ordered roughly by popularity.  Each entry is a compile-time constant so
/// iteration and lookup are trivially optimised by the compiler.
pub const PROVIDERS: &[ProviderDef] = &[
    // ── Major Western providers ──────────────────────────────────────────
    ProviderDef {
        name: "openai",
        display_name: "OpenAI",
        description: "GPT-4, GPT-5 series — most widely used",
        default_base_url: "https://api.openai.com/v1",
        default_model: "gpt-5.4",
        env_var: "OPENAI_API_KEY",
        env_vars: &["OPENAI_API_KEY", "OPENAI_BASE_URL"],
        signup_url: "https://platform.openai.com/api-keys",
        auth_type: "api_key",
        models: &[
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5-mini",
            "gpt-5.3-codex",
            "gpt-5.2-codex",
            "gpt-4.1",
            "gpt-4o",
            "gpt-4o-mini",
        ],
    },
    ProviderDef {
        name: "anthropic",
        display_name: "Anthropic",
        description: "Claude models — safety-focused, strong reasoning",
        default_base_url: "https://api.anthropic.com",
        default_model: "claude-opus-4-7",
        env_var: "ANTHROPIC_API_KEY",
        env_vars: &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
        ],
        signup_url: "https://console.anthropic.com/",
        auth_type: "api_key",
        models: &[
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-haiku-4-5-20251001",
        ],
    },
    ProviderDef {
        name: "google",
        display_name: "Google AI Studio",
        description: "Gemini models — strong multimodal capabilities",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta",
        default_model: "gemini-3.1-pro-preview",
        env_var: "GOOGLE_API_KEY",
        env_vars: &["GOOGLE_API_KEY", "GEMINI_API_KEY", "GEMINI_BASE_URL"],
        signup_url: "https://aistudio.google.com/",
        auth_type: "api_key",
        models: &[
            "gemini-3.1-pro-preview",
            "gemini-3-pro-preview",
            "gemini-3-flash-preview",
            "gemini-3.1-flash-lite-preview",
        ],
    },
    ProviderDef {
        name: "xai",
        display_name: "xAI",
        description: "Grok models — real-time knowledge, X/Twitter integration",
        default_base_url: "https://api.x.ai/v1",
        default_model: "grok-4.20-0309-reasoning",
        env_var: "XAI_API_KEY",
        env_vars: &["XAI_API_KEY", "XAI_BASE_URL"],
        signup_url: "https://console.x.ai/",
        auth_type: "api_key",
        models: &[
            "grok-4.20-0309-reasoning",
            "grok-4.20-0309-non-reasoning",
            "grok-4-1-fast",
            "grok-4",
            "grok-code-fast-1",
        ],
    },
    ProviderDef {
        name: "mistral",
        display_name: "Mistral",
        description: "Open-weight models — strong coding, European lab",
        default_base_url: "https://api.mistral.ai/v1",
        default_model: "mistral-large-2501",
        env_var: "MISTRAL_API_KEY",
        env_vars: &["MISTRAL_API_KEY"],
        signup_url: "https://console.mistral.ai/",
        auth_type: "api_key",
        models: &[
            "mistral-large-2501",
            "mistral-small-2501",
            "codestral-2501",
            "pixtral-large-2411",
        ],
    },
    ProviderDef {
        name: "groq",
        display_name: "Groq",
        description: "Ultra-fast inference — open models on LPUs",
        default_base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-4-scout-17b",
        env_var: "GROQ_API_KEY",
        env_vars: &["GROQ_API_KEY"],
        signup_url: "https://console.groq.com/",
        auth_type: "api_key",
        models: &[
            "llama-4-scout-17b",
            "llama-4-maverick-17b",
            "llama-3.3-70b-versatile",
            "deepseek-r1-distill-llama-70b",
            "qwen-2.5-32b",
        ],
    },
    ProviderDef {
        name: "deepseek",
        display_name: "DeepSeek",
        description: "R1 / V3 series — strong reasoning, cost-effective",
        default_base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-v4-pro",
        env_var: "DEEPSEEK_API_KEY",
        env_vars: &["DEEPSEEK_API_KEY", "DEEPSEEK_BASE_URL"],
        signup_url: "https://platform.deepseek.com/",
        auth_type: "api_key",
        models: &[
            "deepseek-v4-pro",
            "deepseek-v4-flash",
            "deepseek-chat",
            "deepseek-reasoner",
        ],
    },
    ProviderDef {
        name: "nvidia",
        display_name: "NVIDIA NIM",
        description: "Nemotron models — enterprise GPU cloud",
        default_base_url: "https://integrate.api.nvidia.com/v1",
        default_model: "nvidia/nemotron-3-super-120b-a12b",
        env_var: "NVIDIA_API_KEY",
        env_vars: &["NVIDIA_API_KEY", "NVIDIA_BASE_URL"],
        signup_url: "https://build.nvidia.com/",
        auth_type: "api_key",
        models: &[
            "nvidia/nemotron-3-super-120b-a12b",
            "nvidia/nemotron-3-nano-30b-a3b",
            "nvidia/llama-3.3-nemotron-super-49b-v1.5",
            "qwen/qwen3.5-397b-a17b",
            "deepseek-ai/deepseek-v3.2",
            "moonshotai/kimi-k2.6",
            "minimaxai/minimax-m2.5",
            "z-ai/glm5",
        ],
    },
    // ── Aggregators ──────────────────────────────────────────────────────
    ProviderDef {
        name: "openrouter",
        display_name: "OpenRouter",
        description: "Universal API — 100+ models, pay-per-use",
        default_base_url: "https://openrouter.ai/api/v1",
        default_model: "openrouter/auto",
        env_var: "OPENROUTER_API_KEY",
        env_vars: &["OPENROUTER_API_KEY", "OPENAI_API_KEY"],
        signup_url: "https://openrouter.ai/keys",
        auth_type: "api_key",
        models: &[
            "openrouter/auto",
            "openrouter/optimized",
            "openrouter/pareto-code",
            "anthropic/claude-sonnet-4",
            "google/gemini-2.5-pro",
            "meta-llama/llama-4-maverick",
        ],
    },
    ProviderDef {
        name: "together",
        display_name: "Together AI",
        description: "Hosted open-source models — inference API",
        default_base_url: "https://api.together.xyz/v1",
        default_model: "meta-llama/Llama-4-Scout-17B",
        env_var: "TOGETHER_API_KEY",
        env_vars: &["TOGETHER_API_KEY"],
        signup_url: "https://api.together.xyz/",
        auth_type: "api_key",
        models: &[
            "meta-llama/Llama-4-Scout-17B",
            "meta-llama/Llama-4-Maverick-17B",
            "deepseek-ai/DeepSeek-V3",
            "deepseek-ai/DeepSeek-R1",
            "mistralai/Mixtral-8x22B-Instruct-v0.1",
        ],
    },
    ProviderDef {
        name: "vercel",
        display_name: "Vercel AI Gateway",
        description: "Vercel AI Gateway — unified API endpoint",
        default_base_url: "https://ai-gateway.vercel.sh/v1",
        default_model: "moonshotai/kimi-k2.6",
        env_var: "AI_GATEWAY_API_KEY",
        env_vars: &["AI_GATEWAY_API_KEY", "AI_GATEWAY_BASE_URL"],
        signup_url: "https://vercel.com/docs/ai-gateway",
        auth_type: "api_key",
        models: &[
            "moonshotai/kimi-k2.6",
            "alibaba/qwen3.6-plus",
            "zai/glm-5.1",
            "minimax/minimax-m2.7",
            "anthropic/claude-sonnet-4.6",
            "anthropic/claude-opus-4.7",
            "openai/gpt-5.4",
            "google/gemini-3.1-pro-preview",
            "google/gemini-3-flash",
            "xai/grok-4.20-reasoning",
        ],
    },
    // ── Chinese providers ────────────────────────────────────────────────
    ProviderDef {
        name: "zai",
        display_name: "Z.AI / GLM",
        description: "GLM series — strong Chinese language support",
        default_base_url: "https://api.z.ai/api/paas/v4",
        default_model: "glm-5.1",
        env_var: "GLM_API_KEY",
        env_vars: &["GLM_API_KEY", "ZAI_API_KEY", "Z_AI_API_KEY", "GLM_BASE_URL"],
        signup_url: "https://z.ai/",
        auth_type: "api_key",
        models: &[
            "glm-5.1",
            "glm-5",
            "glm-5v-turbo",
            "glm-5-turbo",
            "glm-4.7",
            "glm-4.5",
            "glm-4.5-flash",
        ],
    },
    ProviderDef {
        name: "kimi-coding",
        display_name: "Kimi / Kimi Coding Plan",
        description: "Kimi coding models — Chinese-optimized",
        default_base_url: "https://api.moonshot.ai/v1",
        default_model: "kimi-k2.6",
        env_var: "KIMI_API_KEY",
        env_vars: &["KIMI_API_KEY", "KIMI_CODING_API_KEY", "KIMI_BASE_URL"],
        signup_url: "https://platform.moonshot.cn/",
        auth_type: "api_key",
        models: &[
            "kimi-k2.6",
            "kimi-k2.5",
            "kimi-for-coding",
            "kimi-k2-thinking",
            "kimi-k2-thinking-turbo",
            "kimi-k2-turbo-preview",
        ],
    },
    ProviderDef {
        name: "kimi-coding-cn",
        display_name: "Kimi / Moonshot (China)",
        description: "Kimi coding via Chinese endpoint",
        default_base_url: "https://api.moonshot.cn/v1",
        default_model: "kimi-k2.6",
        env_var: "KIMI_CN_API_KEY",
        env_vars: &["KIMI_CN_API_KEY"],
        signup_url: "https://platform.moonshot.cn/",
        auth_type: "api_key",
        models: &[
            "kimi-k2.6",
            "kimi-k2.5",
            "kimi-k2-thinking",
            "kimi-k2-turbo-preview",
        ],
    },
    ProviderDef {
        name: "moonshot",
        display_name: "Moonshot",
        description: "Moonshot models — Chinese market",
        default_base_url: "https://api.moonshot.ai/v1",
        default_model: "kimi-k2.6",
        env_var: "MOONSHOT_API_KEY",
        env_vars: &["MOONSHOT_API_KEY"],
        signup_url: "https://platform.moonshot.cn/",
        auth_type: "api_key",
        models: &[
            "kimi-k2.6",
            "kimi-k2.5",
            "kimi-k2-thinking",
            "kimi-k2-turbo-preview",
        ],
    },
    ProviderDef {
        name: "stepfun",
        display_name: "StepFun Step Plan",
        description: "Step-series models — Chinese AI lab",
        default_base_url: "https://api.stepfun.ai/step_plan/v1",
        default_model: "step-3.5-flash",
        env_var: "STEPFUN_API_KEY",
        env_vars: &["STEPFUN_API_KEY", "STEPFUN_BASE_URL"],
        signup_url: "https://platform.stepfun.com/",
        auth_type: "api_key",
        models: &["step-3.5-flash", "step-3.5-flash-2603"],
    },
    ProviderDef {
        name: "minimax",
        display_name: "MiniMax",
        description: "MiniMax models — text + voice, Chinese AI",
        default_base_url: "https://api.minimax.io/anthropic",
        default_model: "MiniMax-M2.7",
        env_var: "MINIMAX_API_KEY",
        env_vars: &["MINIMAX_API_KEY", "MINIMAX_BASE_URL"],
        signup_url: "https://www.minimax.io/",
        auth_type: "api_key",
        models: &["MiniMax-M2.7", "MiniMax-M2.5", "MiniMax-M2.1", "MiniMax-M2"],
    },
    ProviderDef {
        name: "minimax-cn",
        display_name: "MiniMax (China)",
        description: "MiniMax via Chinese endpoint",
        default_base_url: "https://api.minimaxi.com/anthropic",
        default_model: "MiniMax-M2.7",
        env_var: "MINIMAX_CN_API_KEY",
        env_vars: &["MINIMAX_CN_API_KEY", "MINIMAX_CN_BASE_URL"],
        signup_url: "https://www.minimaxi.com/",
        auth_type: "api_key",
        models: &["MiniMax-M2.7", "MiniMax-M2.5", "MiniMax-M2.1", "MiniMax-M2"],
    },
    ProviderDef {
        name: "alibaba",
        display_name: "Alibaba DashScope",
        description: "Qwen series — Alibaba Cloud",
        default_base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen3.6-plus",
        env_var: "DASHSCOPE_API_KEY",
        env_vars: &["DASHSCOPE_API_KEY", "DASHSCOPE_BASE_URL"],
        signup_url: "https://modelstudio.console.alibabacloud.com/",
        auth_type: "api_key",
        models: &[
            "qwen3.6-plus",
            "kimi-k2.5",
            "qwen3.5-plus",
            "qwen3-coder-plus",
            "qwen3-coder-next",
            "glm-5",
            "glm-4.7",
            "MiniMax-M2.5",
        ],
    },
    ProviderDef {
        name: "alibaba-coding-plan",
        display_name: "Alibaba Coding Plan",
        description: "Qwen coding models — Alibaba Cloud",
        default_base_url: "https://coding-intl.dashscope.aliyuncs.com/v1",
        default_model: "qwen3.6-plus",
        env_var: "ALIBABA_CODING_PLAN_API_KEY",
        env_vars: &[
            "ALIBABA_CODING_PLAN_API_KEY",
            "DASHSCOPE_API_KEY",
            "ALIBABA_CODING_PLAN_BASE_URL",
        ],
        signup_url: "https://modelstudio.console.alibabacloud.com/",
        auth_type: "api_key",
        models: &[
            "qwen3.6-plus",
            "qwen3.5-plus",
            "qwen3-coder-plus",
            "qwen3-coder-next",
            "kimi-k2.5",
            "glm-5",
            "glm-4.7",
            "MiniMax-M2.5",
        ],
    },
    ProviderDef {
        name: "xiaomi",
        display_name: "Xiaomi MiMo",
        description: "Xiaomi MiMo — mobile-first AI",
        default_base_url: "https://api.xiaomimimo.com/v1",
        default_model: "mimo-v2.5-pro",
        env_var: "XIAOMI_API_KEY",
        env_vars: &["XIAOMI_API_KEY", "XIAOMI_BASE_URL"],
        signup_url: "https://platform.xiaomimimo.com",
        auth_type: "api_key",
        models: &[
            "mimo-v2.5-pro",
            "mimo-v2.5",
            "mimo-v2-pro",
            "mimo-v2-omni",
            "mimo-v2-flash",
        ],
    },
    ProviderDef {
        name: "tencent-tokenhub",
        display_name: "Tencent TokenHub",
        description: "Tencent Hunyuan models",
        default_base_url: "https://tokenhub.tencentmaas.com/v1",
        default_model: "hy3-preview",
        env_var: "TOKENHUB_API_KEY",
        env_vars: &["TOKENHUB_API_KEY", "TOKENHUB_BASE_URL"],
        signup_url: "https://cloud.tencent.com/",
        auth_type: "api_key",
        models: &["hy3-preview"],
    },
    // ── OAuth / special-auth providers ───────────────────────────────────
    ProviderDef {
        name: "nous",
        display_name: "Nous Portal",
        description: "Nous Research — open-source fine-tunes",
        default_base_url: "https://inference-api.nousresearch.com/v1",
        default_model: "anthropic/claude-opus-4.7",
        env_var: "NOUS_API_KEY",
        env_vars: &["NOUS_API_KEY", "NOUS_BASE_URL"],
        signup_url: "https://portal.nousresearch.com/",
        auth_type: "api_key",
        models: &[
            "anthropic/claude-opus-4.7",
            "anthropic/claude-opus-4.6",
            "anthropic/claude-sonnet-4.6",
            "moonshotai/kimi-k2.6",
            "qwen/qwen3.6-plus",
            "anthropic/claude-haiku-4.5",
            "openai/gpt-5.5",
            "openai/gpt-5.4-mini",
            "openai/gpt-5.3-codex",
            "xiaomi/mimo-v2.5-pro",
            "tencent/hy3-preview",
            "google/gemini-3-pro-preview",
            "google/gemini-3-flash-preview",
            "google/gemini-3.1-pro-preview",
            "qwen/qwen3.6-35b-a3b",
            "stepfun/step-3.5-flash",
            "minimax/minimax-m2.7",
            "z-ai/glm-5.1",
            "x-ai/grok-4.3",
            "nvidia/nemotron-3-super-120b-a12b",
            "deepseek/deepseek-v4-pro",
        ],
    },
    ProviderDef {
        name: "openai-codex",
        display_name: "OpenAI Codex",
        description: "Codex models — specialized for code",
        default_base_url: "https://chatgpt.com/backend-api/codex",
        default_model: "gpt-5.4",
        env_var: "OPENAI_API_KEY",
        env_vars: &["OPENAI_API_KEY"],
        signup_url: "https://platform.openai.com/api-keys",
        auth_type: "api_key",
        models: &[
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
            "gpt-5.2-codex",
            "gpt-5.1-codex-max",
            "gpt-5.1-codex-mini",
        ],
    },
    ProviderDef {
        name: "copilot",
        display_name: "GitHub Copilot",
        description: "Copilot Chat — IDE-integrated coding",
        default_base_url: "https://api.githubcopilot.com",
        default_model: "gpt-5.4",
        env_var: "GITHUB_TOKEN",
        env_vars: &[
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "COPILOT_GITHUB_TOKEN",
            "COPILOT_API_BASE_URL",
        ],
        signup_url: "https://github.com/settings/tokens",
        auth_type: "api_key",
        models: &[
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5-mini",
            "gpt-5.3-codex",
            "gpt-5.2-codex",
            "gpt-4.1",
            "gpt-4o",
            "gpt-4o-mini",
            "claude-sonnet-4.6",
            "claude-haiku-4.5",
            "gemini-3.1-pro-preview",
            "gemini-3-pro-preview",
            "gemini-3-flash-preview",
            "gemini-2.5-pro",
            "grok-code-fast-1",
        ],
    },
    ProviderDef {
        name: "copilot-acp",
        display_name: "GitHub Copilot ACP",
        description: "Copilot Agent Mode — autonomous coding",
        default_base_url: "acp://copilot",
        default_model: "copilot-acp",
        env_var: "",
        env_vars: &["COPILOT_ACP_BASE_URL"],
        signup_url: "https://github.com/features/copilot",
        auth_type: "api_key",
        models: &["copilot-acp"],
    },
    ProviderDef {
        name: "google-gemini-cli",
        display_name: "Google Gemini (OAuth)",
        description: "Google Gemini via CLI auth — gcloud",
        default_base_url: "cloudcode-pa://google",
        default_model: "gemini-3.1-pro-preview",
        env_var: "",
        env_vars: &["HERMES_GEMINI_CLIENT_ID", "HERMES_GEMINI_CLIENT_SECRET"],
        signup_url: "https://aistudio.google.com/",
        auth_type: "oauth",
        models: &[
            "gemini-3.1-pro-preview",
            "gemini-3-pro-preview",
            "gemini-3-flash-preview",
        ],
    },
    // ── OpenCode / aggregator providers ──────────────────────────────────
    ProviderDef {
        name: "opencode-zen",
        display_name: "OpenCode Zen",
        description: "OpenCode Zen models — portal access",
        default_base_url: "https://opencode.ai/zen/v1",
        default_model: "kimi-k2.5",
        env_var: "OPENCODE_ZEN_API_KEY",
        env_vars: &["OPENCODE_ZEN_API_KEY", "OPENCODE_ZEN_BASE_URL"],
        signup_url: "https://opencode.ai/auth",
        auth_type: "api_key",
        models: &[
            "kimi-k2.5",
            "gpt-5.4",
            "gpt-5.3-codex",
            "claude-sonnet-4-6",
            "gemini-3-flash",
            "glm-5",
            "kimi-k2-thinking",
            "minimax-m2.7",
            "qwen3-coder",
        ],
    },
    ProviderDef {
        name: "opencode-go",
        display_name: "OpenCode Go",
        description: "OpenCode Go — $10/month subscription",
        default_base_url: "https://opencode.ai/zen/go/v1",
        default_model: "kimi-k2.6",
        env_var: "OPENCODE_GO_API_KEY",
        env_vars: &["OPENCODE_GO_API_KEY", "OPENCODE_GO_BASE_URL"],
        signup_url: "https://opencode.ai/auth",
        auth_type: "api_key",
        models: &[
            "kimi-k2.6",
            "kimi-k2.5",
            "glm-5.1",
            "glm-5",
            "mimo-v2.5-pro",
            "mimo-v2.5",
            "minimax-m2.7",
            "minimax-m2.5",
            "qwen3.6-plus",
            "qwen3.5-plus",
        ],
    },
    ProviderDef {
        name: "kilocode",
        display_name: "Kilo Code",
        description: "Kilocode — coding-focused models",
        default_base_url: "https://api.kilo.ai/api/gateway",
        default_model: "anthropic/claude-opus-4.6",
        env_var: "KILOCODE_API_KEY",
        env_vars: &["KILOCODE_API_KEY", "KILOCODE_BASE_URL"],
        signup_url: "https://kilo.ai/",
        auth_type: "api_key",
        models: &[
            "anthropic/claude-opus-4.6",
            "anthropic/claude-sonnet-4.6",
            "openai/gpt-5.4",
            "google/gemini-3-pro-preview",
            "google/gemini-3-flash-preview",
        ],
    },
    // ── Niche / specialised providers ────────────────────────────────────
    ProviderDef {
        name: "cohere",
        display_name: "Cohere",
        description: "Command-R series — enterprise RAG focus",
        default_base_url: "https://api.cohere.com/v1",
        default_model: "command-r-plus",
        env_var: "COHERE_API_KEY",
        env_vars: &["COHERE_API_KEY"],
        signup_url: "https://dashboard.cohere.com/",
        auth_type: "api_key",
        models: &["command-r-plus", "command-r", "command-a"],
    },
    ProviderDef {
        name: "perplexity",
        display_name: "Perplexity",
        description: "Sonar models — search-augmented generation",
        default_base_url: "https://api.perplexity.ai",
        default_model: "sonar-pro",
        env_var: "PERPLEXITY_API_KEY",
        env_vars: &["PERPLEXITY_API_KEY"],
        signup_url: "https://www.perplexity.ai/settings/api",
        auth_type: "api_key",
        models: &["sonar-pro", "sonar", "sonar-deep-research"],
    },
    ProviderDef {
        name: "huggingface",
        display_name: "Hugging Face",
        description: "HuggingFace Inference API — 100k+ models",
        default_base_url: "https://router.huggingface.co/v1",
        default_model: "moonshotai/Kimi-K2.5",
        env_var: "HF_TOKEN",
        env_vars: &["HF_TOKEN", "HF_BASE_URL", "HF_API_TOKEN"],
        signup_url: "https://huggingface.co/settings/tokens",
        auth_type: "api_key",
        models: &[
            "moonshotai/Kimi-K2.5",
            "Qwen/Qwen3.5-397B-A17B",
            "Qwen/Qwen3.5-35B-A3B",
            "deepseek-ai/DeepSeek-V3.2",
            "MiniMaxAI/MiniMax-M2.5",
            "zai-org/GLM-5",
            "XiaomiMiMo/MiMo-V2-Flash",
            "moonshotai/Kimi-K2-Thinking",
            "moonshotai/Kimi-K2.6",
        ],
    },
    ProviderDef {
        name: "arcee",
        display_name: "Arcee AI",
        description: "Arcee AI — specialized fine-tuned models",
        default_base_url: "https://api.arcee.ai/api/v1",
        default_model: "trinity-large-thinking",
        env_var: "ARCEEAI_API_KEY",
        env_vars: &["ARCEEAI_API_KEY", "ARCEE_BASE_URL"],
        signup_url: "https://chat.arcee.ai/",
        auth_type: "api_key",
        models: &[
            "trinity-large-thinking",
            "trinity-large-preview",
            "trinity-mini",
        ],
    },
    ProviderDef {
        name: "gmi",
        display_name: "GMI Cloud",
        description: "GMI Cloud — GPU cloud hosting",
        default_base_url: "https://api.gmi-serving.com/v1",
        default_model: "zai-org/GLM-5.1-FP8",
        env_var: "GMI_API_KEY",
        env_vars: &["GMI_API_KEY", "GMI_BASE_URL"],
        signup_url: "https://www.gmicloud.ai/",
        auth_type: "api_key",
        models: &[
            "zai-org/GLM-5.1-FP8",
            "deepseek-ai/DeepSeek-V3.2",
            "moonshotai/Kimi-K2.5",
            "google/gemini-3.1-flash-lite-preview",
            "anthropic/claude-sonnet-4.6",
            "openai/gpt-5.4",
        ],
    },
    // ── Cloud / local hosting ────────────────────────────────────────────
    ProviderDef {
        name: "azure",
        display_name: "Azure OpenAI",
        description: "Azure OpenAI — enterprise, Microsoft Cloud",
        default_base_url: "https://{resource}.openai.azure.com",
        default_model: "gpt-4o",
        env_var: "AZURE_OPENAI_API_KEY",
        env_vars: &[
            "AZURE_OPENAI_API_KEY",
            "AZURE_FOUNDRY_API_KEY",
            "AZURE_FOUNDRY_BASE_URL",
        ],
        signup_url: "https://portal.azure.com/",
        auth_type: "api_key",
        models: &[
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4",
            "gpt-4-turbo",
            "gpt-35-turbo",
        ],
    },
    ProviderDef {
        name: "azure-foundry",
        display_name: "Azure Foundry",
        description: "Azure AI Foundry — enterprise model catalog",
        default_base_url: "",
        default_model: "",
        env_var: "AZURE_FOUNDRY_API_KEY",
        env_vars: &["AZURE_FOUNDRY_API_KEY", "AZURE_FOUNDRY_BASE_URL"],
        signup_url: "https://ai.azure.com/",
        auth_type: "api_key",
        models: &[],
    },
    ProviderDef {
        name: "bedrock",
        display_name: "AWS Bedrock",
        description: "AWS Bedrock — enterprise, managed",
        default_base_url: "",
        default_model: "us.anthropic.claude-sonnet-4-6",
        env_var: "AWS_ACCESS_KEY_ID",
        env_vars: &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_REGION",
            "AWS_PROFILE",
        ],
        signup_url: "https://aws.amazon.com/bedrock/",
        auth_type: "aws",
        models: &[
            "us.anthropic.claude-sonnet-4-6",
            "us.anthropic.claude-opus-4-6-v1",
            "us.anthropic.claude-haiku-4-5-20251001-v1:0",
            "us.amazon.nova-pro-v1:0",
            "us.amazon.nova-lite-v1:0",
            "us.amazon.nova-micro-v1:0",
            "deepseek.v3.2",
            "us.meta.llama4-maverick-17b-instruct-v1:0",
            "us.meta.llama4-scout-17b-instruct-v1:0",
        ],
    },
    ProviderDef {
        name: "ollama",
        display_name: "Ollama (Local)",
        description: "Local models via Ollama — run on your hardware",
        default_base_url: "http://localhost:11434/v1",
        default_model: "llama3.1",
        env_var: "",
        env_vars: &["OLLAMA_BASE_URL"],
        signup_url: "",
        auth_type: "none",
        models: &["llama3.1", "llama3", "mistral", "codellama", "mixtral"],
    },
    ProviderDef {
        name: "ollama-cloud",
        display_name: "Ollama Cloud",
        description: "Ollama cloud-hosted models",
        default_base_url: "https://ollama.com/v1",
        default_model: "llama3.1",
        env_var: "OLLAMA_API_KEY",
        env_vars: &["OLLAMA_API_KEY", "OLLAMA_BASE_URL"],
        signup_url: "https://ollama.com/settings",
        auth_type: "api_key",
        models: &["llama3.1", "llama3", "mistral", "mixtral"],
    },
    ProviderDef {
        name: "lmstudio",
        display_name: "LM Studio",
        description: "Local models via LM Studio — desktop app",
        default_base_url: "http://127.0.0.1:1234/v1",
        default_model: "local-model",
        env_var: "LM_API_KEY",
        env_vars: &["LM_API_KEY", "LM_BASE_URL"],
        signup_url: "",
        auth_type: "none",
        models: &["local-model"],
    },
    // ── OAuth portal providers ───────────────────────────────────────────
    ProviderDef {
        name: "qwen-oauth",
        display_name: "Qwen OAuth (Portal)",
        description: "Qwen via OAuth portal — Alibaba Cloud",
        default_base_url: "https://portal.qwen.ai/v1",
        default_model: "qwen3.6-plus",
        env_var: "",
        env_vars: &["HERMES_QWEN_BASE_URL"],
        signup_url: "https://chat.qwen.ai/",
        auth_type: "oauth",
        models: &["qwen3.6-plus", "qwen3.5-plus"],
    },
];

// ---------------------------------------------------------------------------
// Dynamic model fetching
// ---------------------------------------------------------------------------

// Response structures for provider model-list APIs
#[derive(Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModel>,
}

#[derive(Deserialize)]
struct OpenAIModel {
    id: String,
}

#[derive(Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModel>,
}

#[derive(Deserialize)]
struct AnthropicModel {
    id: String,
}

#[derive(Deserialize)]
struct GoogleModelsResponse {
    models: Vec<GoogleModel>,
}

#[derive(Deserialize)]
struct GoogleModel {
    name: String,
}

/// Fetch available models from a provider's API endpoint.
///
/// Supports three API patterns:
/// - **OpenAI-compatible** (`/v1/models`, `Authorization: Bearer {key}`) — returns `data[].id`
/// - **Anthropic** (`/v1/models`, `x-api-key`, `anthropic-version`) — returns `data[].id`
/// - **Google** (`/v1beta/models?key={key}`) — strips `models/` prefix from `models[].name`
///
/// Returns the static model list (`provider.models`) on any error (network, auth, parse).
pub async fn fetch_models_for_provider(provider: &ProviderDef, api_key: &str) -> Vec<String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return provider.models.iter().map(|s| s.to_string()).collect(),
    };

    let result = match provider.name {
        "openai" | "xai" | "mistral" | "groq" | "deepseek" | "together" | "openrouter"
        | "nvidia" | "perplexity" | "cohere" | "huggingface" | "zai" | "kimi-coding"
        | "stepfun" | "minimax" | "alibaba" | "xiaomi" | "opencode-zen" | "opencode-go"
        | "kilocode" | "ollama-cloud" | "vercel" | "moonshot" | "tencent-tokenhub" => {
            fetch_openai_compatible(&client, provider.default_base_url, api_key).await
        }
        "anthropic" => fetch_anthropic_models(&client, provider.default_base_url, api_key).await,
        "google" => fetch_google_models(&client, provider.default_base_url, api_key).await,
        _ => None,
    };

    result.unwrap_or_else(|| provider.models.iter().map(|s| s.to_string()).collect())
}

async fn fetch_openai_compatible(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: OpenAIModelsResponse = resp.json().await.ok()?;
    let models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
}

async fn fetch_anthropic_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: AnthropicModelsResponse = resp.json().await.ok()?;
    let models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
}

async fn fetch_google_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<Vec<String>> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{}/v1beta/models?key={}", base, api_key);
    let resp = client.get(&url).send().await.ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: GoogleModelsResponse = resp.json().await.ok()?;
    let models: Vec<String> = body
        .models
        .into_iter()
        .map(|m| {
            m.name
                .strip_prefix("models/")
                .unwrap_or(&m.name)
                .to_string()
        })
        .collect();
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Find a provider definition by its machine-friendly `name`.
///
/// Returns `None` when no provider matches.
///
/// # Example
///
/// ```ignore
/// let p = provider_by_name("openai").unwrap();
/// assert_eq!(p.display_name, "OpenAI");
/// ```
pub fn provider_by_name(name: &str) -> Option<&'static ProviderDef> {
    PROVIDERS.iter().find(|p| p.name == name)
}

/// Find a provider definition by its `display_name`.
///
/// This is case-insensitive and will match partial prefixes (e.g. `"open"`
/// will not match, but `"OpenAI"` will).
///
/// # Example
///
/// ```ignore
/// let p = provider_by_display("OpenAI").unwrap();
/// assert_eq!(p.name, "openai");
/// ```
pub fn provider_by_display(display: &str) -> Option<&'static ProviderDef> {
    PROVIDERS
        .iter()
        .find(|p| p.display_name.eq_ignore_ascii_case(display))
}

/// Infer a provider from its base URL.
///
/// This mirrors the logic in `cmd_setup::infer_provider` but uses the
/// canonical registry rather than its own string matching.
///
/// Returns the `ProviderDef` whose base URL host most closely matches the
/// given URL, or `None` if no known provider matches.
pub fn provider_from_url(base_url: &str) -> Option<&'static ProviderDef> {
    let lower = base_url.to_lowercase();
    if lower.contains("openai.com") {
        provider_by_name("openai")
    } else if lower.contains("anthropic.com") {
        provider_by_name("anthropic")
    } else if lower.contains("generativelanguage") || lower.contains("googleapis") {
        provider_by_name("google")
    } else if lower.contains("groq.com") {
        provider_by_name("groq")
    } else if lower.contains("together.xyz") {
        provider_by_name("together")
    } else if lower.contains("deepseek.com") {
        provider_by_name("deepseek")
    } else if lower.contains("mistral.ai") {
        provider_by_name("mistral")
    } else if lower.contains("openrouter") {
        provider_by_name("openrouter")
    } else if lower.contains("xai") || lower.contains("x.ai") {
        provider_by_name("xai")
    } else if lower.contains("nvidia.com") {
        provider_by_name("nvidia")
    } else if lower.contains("cohere.com") {
        provider_by_name("cohere")
    } else if lower.contains("perplexity.ai") {
        provider_by_name("perplexity")
    } else if lower.contains("azure.com") {
        provider_by_name("azure")
    } else if lower.contains("huggingface") || lower.contains("hf.co") {
        provider_by_name("huggingface")
    } else if lower.contains("localhost") || lower.contains("127.0.0.1") {
        provider_by_name("ollama")
    } else if lower.contains("minimax.io") || lower.contains("minimaxi.com") {
        provider_by_name("minimax")
    } else if lower.contains("z.ai") || lower.contains("bigmodel.cn") {
        provider_by_name("zai")
    } else if lower.contains("moonshot") || lower.contains("kimi.com") {
        provider_by_name("kimi-coding")
    } else if lower.contains("stepfun") || lower.contains("stepfun") {
        provider_by_name("stepfun")
    } else if lower.contains("dashscope") || lower.contains("aliyuncs.com") {
        provider_by_name("alibaba")
    } else if lower.contains("xiaomimimo") {
        provider_by_name("xiaomi")
    } else if lower.contains("tokenhub") || lower.contains("tencentmaas") {
        provider_by_name("tencent-tokenhub")
    } else if lower.contains("arcee.ai") {
        provider_by_name("arcee")
    } else if lower.contains("gmi-serving") {
        provider_by_name("gmi")
    } else if lower.contains("opencode.ai") {
        provider_by_name("opencode-zen")
    } else if lower.contains("kilo.ai") {
        provider_by_name("kilocode")
    } else if lower.contains("ollama.com") {
        provider_by_name("ollama-cloud")
    } else if lower.contains("ai-gateway") || lower.contains("vercel.sh") {
        provider_by_name("vercel")
    } else if lower.contains("nousresearch") {
        provider_by_name("nous")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_providers_have_non_empty_names() {
        for p in PROVIDERS {
            assert!(!p.name.is_empty(), "provider name is empty");
            assert!(!p.display_name.is_empty(), "provider display_name is empty");
            assert!(
                !p.description.is_empty(),
                "{}: provider description is empty",
                p.name
            );
        }
    }

    #[test]
    fn provider_count() {
        assert_eq!(PROVIDERS.len(), 42);
    }

    #[test]
    fn lookup_by_name() {
        assert!(provider_by_name("openai").is_some());
        assert!(provider_by_name("anthropic").is_some());
        assert!(provider_by_name("nous").is_some());
        assert!(provider_by_name("zai").is_some());
        assert!(provider_by_name("nonexistent").is_none());
    }

    #[test]
    fn lookup_by_display() {
        assert!(provider_by_display("OpenAI").is_some());
        assert!(provider_by_display("openai").is_some()); // case-insensitive
        assert!(provider_by_display("Anthropic").is_some());
    }

    #[test]
    fn infer_from_url() {
        assert_eq!(
            provider_from_url("https://api.openai.com/v1").unwrap().name,
            "openai"
        );
        assert_eq!(
            provider_from_url("https://api.anthropic.com/v1")
                .unwrap()
                .name,
            "anthropic"
        );
        assert!(provider_from_url("https://example.com").is_none());
    }

    #[test]
    fn primary_env_var_is_in_env_vars_list() {
        for p in PROVIDERS {
            if !p.env_var.is_empty() {
                assert!(
                    p.env_vars.contains(&p.env_var),
                    "{}: primary env var '{}' not in env_vars list",
                    p.name,
                    p.env_var
                );
            }
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = PROVIDERS.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), PROVIDERS.len());
    }

    #[test]
    fn all_existing_providers_are_present() {
        // Verify that all 16 original providers are still present
        for name in &[
            "openai",
            "anthropic",
            "google",
            "xai",
            "mistral",
            "groq",
            "deepseek",
            "together",
            "openrouter",
            "nvidia",
            "cohere",
            "perplexity",
            "azure",
            "bedrock",
            "ollama",
            "huggingface",
        ] {
            assert!(
                provider_by_name(name).is_some(),
                "Original provider '{}' is missing",
                name
            );
        }
    }

    #[test]
    fn new_providers_are_present() {
        for name in &[
            "nous",
            "openai-codex",
            "copilot",
            "copilot-acp",
            "google-gemini-cli",
            "zai",
            "kimi-coding",
            "kimi-coding-cn",
            "moonshot",
            "stepfun",
            "minimax",
            "minimax-cn",
            "alibaba",
            "alibaba-coding-plan",
            "xiaomi",
            "tencent-tokenhub",
            "arcee",
            "gmi",
            "opencode-zen",
            "opencode-go",
            "kilocode",
            "ollama-cloud",
            "lmstudio",
            "vercel",
            "qwen-oauth",
            "azure-foundry",
        ] {
            assert!(
                provider_by_name(name).is_some(),
                "New provider '{}' is missing",
                name
            );
        }
    }

    #[test]
    fn global_registry_has_all_providers() {
        let registry = global_registry();
        let list = registry.list();
        assert_eq!(list.len(), 42);
    }

    #[test]
    fn global_registry_alias_resolution() {
        let registry = global_registry();
        assert_eq!(
            registry.resolve_alias("claude"),
            Some("anthropic".to_string())
        );
        assert_eq!(registry.resolve_alias("gpt"), Some("openai".to_string()));
        assert_eq!(registry.resolve_alias("gemini"), Some("google".to_string()));
        assert_eq!(registry.resolve_alias("nonexistent"), None);
    }

    #[test]
    fn global_registry_get_via_alias() {
        let registry = global_registry();
        let claude = registry.get("claude");
        assert!(claude.is_some());
        assert_eq!(claude.unwrap().name(), "anthropic");
    }

    #[test]
    fn global_registry_get_direct() {
        let registry = global_registry();
        let openai = registry.get("openai");
        assert!(openai.is_some());
        assert_eq!(openai.unwrap().display_name(), "OpenAI");
    }

    #[test]
    fn provider_profile_static_bridge() {
        let def = provider_by_name("anthropic").unwrap();
        let profile = StaticProviderProfile::new(def);
        assert_eq!(profile.name(), "anthropic");
        assert_eq!(profile.display_name(), "Anthropic");
        assert_eq!(profile.base_url(), Some("https://api.anthropic.com"));
        assert_eq!(profile.api_key_env(), Some("ANTHROPIC_API_KEY"));
        assert!(profile.supports_vision());
        assert!(profile.supports_streaming());
        assert_eq!(profile.auth_type(), "api_key");
        assert!(!profile.signup_url().is_empty());
    }

    #[test]
    fn provider_profile_default_methods() {
        struct DummyProvider;
        impl ProviderProfile for DummyProvider {
            fn name(&self) -> &str {
                "dummy"
            }
        }

        let p = DummyProvider;
        assert_eq!(p.display_name(), "dummy");
        assert_eq!(p.description(), "");
        assert!(p.base_url().is_none());
        assert!(p.api_key_env().is_none());
        assert_eq!(p.default_model(), "");
        assert!(!p.supports_vision());
        assert!(p.supports_streaming());
        assert_eq!(p.auth_type(), "api_key");
        assert!(p.signup_url().is_empty());
        assert!(p.aliases().is_empty());
        assert!(p.get_max_tokens().is_none());
        assert!(p.fallback_models().is_empty());
        assert_eq!(p.build_extra_body(), serde_json::json!({}));
    }

    #[test]
    fn custom_registry_register_and_get() {
        struct CustomProvider;
        impl ProviderProfile for CustomProvider {
            fn name(&self) -> &str {
                "custom"
            }
            fn display_name(&self) -> &str {
                "Custom Provider"
            }
            fn base_url(&self) -> Option<&str> {
                Some("https://custom.api/v1")
            }
            fn aliases(&self) -> Vec<&str> {
                vec!["cstm"]
            }
        }

        let registry = ProviderRegistry::new();
        registry.register(Arc::new(CustomProvider) as Arc<dyn ProviderProfile>);
        assert!(registry.get("custom").is_some());
        assert_eq!(
            registry.get("custom").unwrap().display_name(),
            "Custom Provider"
        );
        assert_eq!(registry.resolve_alias("cstm"), Some("custom".to_string()));
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn custom_registry_fallback_chain() {
        let registry = ProviderRegistry::new();
        registry.set_fallback_chain(vec!["primary".to_string(), "secondary".to_string()]);
        let chain = registry.get_fallback_chain("primary");
        assert!(chain.is_some());
        assert_eq!(
            chain.unwrap(),
            vec!["primary".to_string(), "secondary".to_string()]
        );
    }

    #[test]
    fn static_provider_profile_fallback_models() {
        let def = provider_by_name("openai").unwrap();
        let profile = StaticProviderProfile::new(def);
        let models = profile.fallback_models();
        assert!(!models.is_empty());
        assert!(models.contains(&"gpt-5.4".to_string()));
    }
}
