//! Canonical provider registry for Operant-RS CLI.
//!
//! This module centralises the provider metadata that was previously scattered
//! across `cmd_setup.rs`, `cmd_auth.rs`, and `config.rs`: the built-in catalog
//! (`ProviderDef` + `PROVIDERS`) and each provider's `/models` wire family.
//! Model lists are never compiled in; `model_catalog` resolves them at runtime.
//!
//! ```ignore
//! // Static lookup
//! for p in PROVIDERS {
//!     println!("{}: {}", p.display_name, p.default_base_url);
//! }
//! ```

use operant_core::model_catalog::WireFamily;

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
        env_var: "OPENAI_API_KEY",
        env_vars: &["OPENAI_API_KEY", "OPENAI_BASE_URL"],
        signup_url: "https://platform.openai.com/api-keys",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "anthropic",
        display_name: "Anthropic",
        description: "Claude models — safety-focused, strong reasoning",
        default_base_url: "https://api.anthropic.com/v1",
        env_var: "ANTHROPIC_API_KEY",
        env_vars: &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
        ],
        signup_url: "https://console.anthropic.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "google",
        display_name: "Google AI Studio",
        description: "Gemini models — strong multimodal capabilities",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        env_var: "GOOGLE_API_KEY",
        env_vars: &["GOOGLE_API_KEY", "GEMINI_API_KEY", "GEMINI_BASE_URL"],
        signup_url: "https://aistudio.google.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "xai",
        display_name: "xAI",
        description: "Grok models — real-time knowledge, X/Twitter integration",
        default_base_url: "https://api.x.ai/v1",
        env_var: "XAI_API_KEY",
        env_vars: &["XAI_API_KEY", "XAI_BASE_URL"],
        signup_url: "https://console.x.ai/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "mistral",
        display_name: "Mistral",
        description: "Open-weight models — strong coding, European lab",
        default_base_url: "https://api.mistral.ai/v1",
        env_var: "MISTRAL_API_KEY",
        env_vars: &["MISTRAL_API_KEY"],
        signup_url: "https://console.mistral.ai/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "groq",
        display_name: "Groq",
        description: "Ultra-fast inference — open models on LPUs",
        default_base_url: "https://api.groq.com/openai/v1",
        env_var: "GROQ_API_KEY",
        env_vars: &["GROQ_API_KEY"],
        signup_url: "https://console.groq.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "deepseek",
        display_name: "DeepSeek",
        description: "R1 / V3 series — strong reasoning, cost-effective",
        default_base_url: "https://api.deepseek.com/v1",
        env_var: "DEEPSEEK_API_KEY",
        env_vars: &["DEEPSEEK_API_KEY", "DEEPSEEK_BASE_URL"],
        signup_url: "https://platform.deepseek.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "nvidia",
        display_name: "NVIDIA NIM",
        description: "Nemotron models — enterprise GPU cloud",
        default_base_url: "https://integrate.api.nvidia.com/v1",
        env_var: "NVIDIA_API_KEY",
        env_vars: &["NVIDIA_API_KEY", "NVIDIA_BASE_URL"],
        signup_url: "https://build.nvidia.com/",
        auth_type: "api_key",
    },
    // ── Aggregators ──────────────────────────────────────────────────────
    ProviderDef {
        name: "openrouter",
        display_name: "OpenRouter",
        description: "Universal API — 100+ models, pay-per-use",
        default_base_url: "https://openrouter.ai/api/v1",
        env_var: "OPENROUTER_API_KEY",
        env_vars: &["OPENROUTER_API_KEY", "OPENAI_API_KEY"],
        signup_url: "https://openrouter.ai/keys",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "together",
        display_name: "Together AI",
        description: "Hosted open-source models — inference API",
        default_base_url: "https://api.together.xyz/v1",
        env_var: "TOGETHER_API_KEY",
        env_vars: &["TOGETHER_API_KEY"],
        signup_url: "https://api.together.xyz/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "vercel",
        display_name: "Vercel AI Gateway",
        description: "Vercel AI Gateway — unified API endpoint",
        default_base_url: "https://ai-gateway.vercel.sh/v1",
        env_var: "AI_GATEWAY_API_KEY",
        env_vars: &["AI_GATEWAY_API_KEY", "AI_GATEWAY_BASE_URL"],
        signup_url: "https://vercel.com/docs/ai-gateway",
        auth_type: "api_key",
    },
    // ── Chinese providers ────────────────────────────────────────────────
    ProviderDef {
        name: "zai",
        display_name: "Z.AI / GLM",
        description: "GLM series — strong Chinese language support",
        default_base_url: "https://api.z.ai/api/paas/v4",
        env_var: "GLM_API_KEY",
        env_vars: &["GLM_API_KEY", "ZAI_API_KEY", "Z_AI_API_KEY", "GLM_BASE_URL"],
        signup_url: "https://z.ai/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "kimi-coding",
        display_name: "Kimi / Kimi Coding Plan",
        description: "Kimi coding models — Chinese-optimized",
        default_base_url: "https://api.moonshot.ai/v1",
        env_var: "KIMI_API_KEY",
        env_vars: &["KIMI_API_KEY", "KIMI_CODING_API_KEY", "KIMI_BASE_URL"],
        signup_url: "https://platform.moonshot.cn/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "kimi-coding-cn",
        display_name: "Kimi / Moonshot (China)",
        description: "Kimi coding via Chinese endpoint",
        default_base_url: "https://api.moonshot.cn/v1",
        env_var: "KIMI_CN_API_KEY",
        env_vars: &["KIMI_CN_API_KEY"],
        signup_url: "https://platform.moonshot.cn/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "moonshot",
        display_name: "Moonshot",
        description: "Moonshot models — Chinese market",
        default_base_url: "https://api.moonshot.ai/v1",
        env_var: "MOONSHOT_API_KEY",
        env_vars: &["MOONSHOT_API_KEY"],
        signup_url: "https://platform.moonshot.cn/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "stepfun",
        display_name: "StepFun Step Plan",
        description: "Step-series models — Chinese AI lab",
        default_base_url: "https://api.stepfun.ai/step_plan/v1",
        env_var: "STEPFUN_API_KEY",
        env_vars: &["STEPFUN_API_KEY", "STEPFUN_BASE_URL"],
        signup_url: "https://platform.stepfun.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "minimax",
        display_name: "MiniMax",
        description: "MiniMax models — text + voice, Chinese AI",
        default_base_url: "https://api.minimax.io/v1",
        env_var: "MINIMAX_API_KEY",
        env_vars: &["MINIMAX_API_KEY", "MINIMAX_BASE_URL"],
        signup_url: "https://www.minimax.io/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "minimax-cn",
        display_name: "MiniMax (China)",
        description: "MiniMax via Chinese endpoint",
        default_base_url: "https://api.minimaxi.com/v1",
        env_var: "MINIMAX_CN_API_KEY",
        env_vars: &["MINIMAX_CN_API_KEY", "MINIMAX_CN_BASE_URL"],
        signup_url: "https://www.minimaxi.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "alibaba",
        display_name: "Alibaba DashScope",
        description: "Qwen series — Alibaba Cloud",
        default_base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        env_var: "DASHSCOPE_API_KEY",
        env_vars: &["DASHSCOPE_API_KEY", "DASHSCOPE_BASE_URL"],
        signup_url: "https://modelstudio.console.alibabacloud.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "alibaba-coding-plan",
        display_name: "Alibaba Coding Plan",
        description: "Qwen coding models — Alibaba Cloud",
        default_base_url: "https://coding-intl.dashscope.aliyuncs.com/v1",
        env_var: "ALIBABA_CODING_PLAN_API_KEY",
        env_vars: &[
            "ALIBABA_CODING_PLAN_API_KEY",
            "DASHSCOPE_API_KEY",
            "ALIBABA_CODING_PLAN_BASE_URL",
        ],
        signup_url: "https://modelstudio.console.alibabacloud.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "xiaomi",
        display_name: "Xiaomi MiMo",
        description: "Xiaomi MiMo — mobile-first AI",
        default_base_url: "https://api.xiaomimimo.com/v1",
        env_var: "XIAOMI_API_KEY",
        env_vars: &["XIAOMI_API_KEY", "XIAOMI_BASE_URL"],
        signup_url: "https://platform.xiaomimimo.com",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "tencent-tokenhub",
        display_name: "Tencent TokenHub",
        description: "Tencent Hunyuan models",
        default_base_url: "https://tokenhub.tencentmaas.com/v1",
        env_var: "TOKENHUB_API_KEY",
        env_vars: &["TOKENHUB_API_KEY", "TOKENHUB_BASE_URL"],
        signup_url: "https://cloud.tencent.com/",
        auth_type: "api_key",
    },
    // ── OAuth / special-auth providers ───────────────────────────────────
    ProviderDef {
        name: "nous",
        display_name: "Nous Portal",
        description: "Nous Research — open-source fine-tunes",
        default_base_url: "https://inference-api.nousresearch.com/v1",
        env_var: "NOUS_API_KEY",
        env_vars: &["NOUS_API_KEY", "NOUS_BASE_URL"],
        signup_url: "https://portal.nousresearch.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "openai-codex",
        display_name: "OpenAI Codex",
        description: "Codex models — specialized for code",
        default_base_url: "https://chatgpt.com/backend-api/codex",
        env_var: "OPENAI_API_KEY",
        env_vars: &["OPENAI_API_KEY"],
        signup_url: "https://platform.openai.com/api-keys",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "copilot",
        display_name: "GitHub Copilot",
        description: "Copilot Chat — IDE-integrated coding",
        default_base_url: "https://api.githubcopilot.com",
        env_var: "GITHUB_TOKEN",
        env_vars: &[
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "COPILOT_GITHUB_TOKEN",
            "COPILOT_API_BASE_URL",
        ],
        signup_url: "https://github.com/settings/tokens",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "copilot-acp",
        display_name: "GitHub Copilot ACP",
        description: "Copilot Agent Mode — autonomous coding",
        default_base_url: "acp://copilot",
        env_var: "",
        env_vars: &["COPILOT_ACP_BASE_URL"],
        signup_url: "https://github.com/features/copilot",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "google-gemini-cli",
        display_name: "Google Gemini (OAuth)",
        description: "Google Gemini via CLI auth — gcloud",
        default_base_url: "cloudcode-pa://google",
        env_var: "",
        env_vars: &["HERMES_GEMINI_CLIENT_ID", "HERMES_GEMINI_CLIENT_SECRET"],
        signup_url: "https://aistudio.google.com/",
        auth_type: "oauth",
    },
    // ── OpenCode / aggregator providers ──────────────────────────────────
    ProviderDef {
        name: "opencode-zen",
        display_name: "OpenCode Zen",
        description: "OpenCode Zen models — portal access",
        default_base_url: "https://opencode.ai/zen/v1",
        env_var: "OPENCODE_ZEN_API_KEY",
        env_vars: &["OPENCODE_ZEN_API_KEY", "OPENCODE_ZEN_BASE_URL"],
        signup_url: "https://opencode.ai/auth",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "opencode-go",
        display_name: "OpenCode Go",
        description: "OpenCode Go — $10/month subscription",
        default_base_url: "https://opencode.ai/zen/go/v1",
        env_var: "OPENCODE_GO_API_KEY",
        env_vars: &["OPENCODE_GO_API_KEY", "OPENCODE_GO_BASE_URL"],
        signup_url: "https://opencode.ai/auth",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "kilocode",
        display_name: "Kilo Code",
        description: "Kilocode — coding-focused models",
        default_base_url: "https://api.kilo.ai/api/gateway",
        env_var: "KILOCODE_API_KEY",
        env_vars: &["KILOCODE_API_KEY", "KILOCODE_BASE_URL"],
        signup_url: "https://kilo.ai/",
        auth_type: "api_key",
    },
    // ── Niche / specialised providers ────────────────────────────────────
    ProviderDef {
        name: "cohere",
        display_name: "Cohere",
        description: "Command-R series — enterprise RAG focus",
        default_base_url: "https://api.cohere.ai/compatibility/v1",
        env_var: "COHERE_API_KEY",
        env_vars: &["COHERE_API_KEY"],
        signup_url: "https://dashboard.cohere.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "perplexity",
        display_name: "Perplexity",
        description: "Sonar models — search-augmented generation",
        default_base_url: "https://api.perplexity.ai",
        env_var: "PERPLEXITY_API_KEY",
        env_vars: &["PERPLEXITY_API_KEY"],
        signup_url: "https://www.perplexity.ai/settings/api",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "huggingface",
        display_name: "Hugging Face",
        description: "HuggingFace Inference API — 100k+ models",
        default_base_url: "https://router.huggingface.co/v1",
        env_var: "HF_TOKEN",
        env_vars: &["HF_TOKEN", "HF_BASE_URL", "HF_API_TOKEN"],
        signup_url: "https://huggingface.co/settings/tokens",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "arcee",
        display_name: "Arcee AI",
        description: "Arcee AI — specialized fine-tuned models",
        default_base_url: "https://api.arcee.ai/api/v1",
        env_var: "ARCEEAI_API_KEY",
        env_vars: &["ARCEEAI_API_KEY", "ARCEE_BASE_URL"],
        signup_url: "https://chat.arcee.ai/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "gmi",
        display_name: "GMI Cloud",
        description: "GMI Cloud — GPU cloud hosting",
        default_base_url: "https://api.gmi-serving.com/v1",
        env_var: "GMI_API_KEY",
        env_vars: &["GMI_API_KEY", "GMI_BASE_URL"],
        signup_url: "https://www.gmicloud.ai/",
        auth_type: "api_key",
    },
    // ── Cloud / local hosting ────────────────────────────────────────────
    ProviderDef {
        name: "azure",
        display_name: "Azure OpenAI",
        description: "Azure OpenAI — enterprise, Microsoft Cloud",
        default_base_url: "https://{resource}.openai.azure.com",
        env_var: "AZURE_OPENAI_API_KEY",
        env_vars: &[
            "AZURE_OPENAI_API_KEY",
            "AZURE_FOUNDRY_API_KEY",
            "AZURE_FOUNDRY_BASE_URL",
        ],
        signup_url: "https://portal.azure.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "azure-foundry",
        display_name: "Azure Foundry",
        description: "Azure AI Foundry — enterprise model catalog",
        default_base_url: "",
        env_var: "AZURE_FOUNDRY_API_KEY",
        env_vars: &["AZURE_FOUNDRY_API_KEY", "AZURE_FOUNDRY_BASE_URL"],
        signup_url: "https://ai.azure.com/",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "bedrock",
        display_name: "AWS Bedrock",
        description: "AWS Bedrock — enterprise, managed",
        default_base_url: "",
        env_var: "AWS_ACCESS_KEY_ID",
        env_vars: &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_REGION",
            "AWS_PROFILE",
        ],
        signup_url: "https://aws.amazon.com/bedrock/",
        auth_type: "aws",
    },
    ProviderDef {
        name: "ollama",
        display_name: "Ollama (Local)",
        description: "Local models via Ollama — run on your hardware",
        default_base_url: "http://localhost:11434/v1",
        env_var: "",
        env_vars: &["OLLAMA_BASE_URL"],
        signup_url: "",
        auth_type: "none",
    },
    ProviderDef {
        name: "ollama-cloud",
        display_name: "Ollama Cloud",
        description: "Ollama cloud-hosted models",
        default_base_url: "https://ollama.com/v1",
        env_var: "OLLAMA_API_KEY",
        env_vars: &["OLLAMA_API_KEY", "OLLAMA_BASE_URL"],
        signup_url: "https://ollama.com/settings",
        auth_type: "api_key",
    },
    ProviderDef {
        name: "lmstudio",
        display_name: "LM Studio",
        description: "Local models via LM Studio — desktop app",
        default_base_url: "http://127.0.0.1:1234/v1",
        env_var: "LM_API_KEY",
        env_vars: &["LM_API_KEY", "LM_BASE_URL"],
        signup_url: "",
        auth_type: "none",
    },
    // ── OAuth portal providers ───────────────────────────────────────────
    ProviderDef {
        name: "qwen-oauth",
        display_name: "Qwen OAuth (Portal)",
        description: "Qwen via OAuth portal — Alibaba Cloud",
        default_base_url: "https://portal.qwen.ai/v1",
        env_var: "",
        env_vars: &["HERMES_QWEN_BASE_URL"],
        signup_url: "https://chat.qwen.ai/",
        auth_type: "oauth",
    },
];

// ---------------------------------------------------------------------------
// Live model listing
// ---------------------------------------------------------------------------

/// Wire protocol of `name`'s own `/models` endpoint. `None` for providers whose
/// model list comes only from models.dev (or from the configured models).
pub fn wire_family(name: &str) -> Option<WireFamily> {
    match name {
        "anthropic" => Some(WireFamily::Anthropic),
        "google" => Some(WireFamily::Google),
        "openai" | "xai" | "mistral" | "groq" | "deepseek" | "together" | "openrouter"
        | "nvidia" | "perplexity" | "cohere" | "huggingface" | "zai" | "kimi-coding"
        | "stepfun" | "minimax" | "alibaba" | "xiaomi" | "opencode-zen" | "opencode-go"
        | "kilocode" | "ollama" | "ollama-cloud" | "vercel" | "moonshot" | "tencent-tokenhub" => {
            Some(WireFamily::OpenAiCompat)
        }
        _ => None,
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
    } else if lower.contains("cohere.com") || lower.contains("cohere.ai") {
        provider_by_name("cohere")
    } else if lower.contains("perplexity.ai") {
        provider_by_name("perplexity")
    } else if lower.contains("azure.com") {
        provider_by_name("azure")
    } else if lower.contains("huggingface") || lower.contains("hf.co") {
        provider_by_name("huggingface")
    } else if lower.contains("localhost") || lower.contains("127.0.0.1") {
        // Only the well-known local servers have a port that identifies them.
        // Any other loopback URL (a local proxy, a gateway) is a custom
        // endpoint: labelling it "Ollama" made `setup` show the wrong
        // "currently active" provider and skip the key prompt.
        match reqwest::Url::parse(base_url).ok().and_then(|u| u.port()) {
            Some(11434) => provider_by_name("ollama"),
            Some(1234) => provider_by_name("lmstudio"),
            _ => None,
        }
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

    #[test]
    fn loopback_urls_identify_only_known_local_servers() {
        assert_eq!(
            provider_from_url("http://localhost:11434/v1").unwrap().name,
            "ollama"
        );
        assert_eq!(
            provider_from_url("http://127.0.0.1:1234/v1").unwrap().name,
            "lmstudio"
        );
        // A local proxy is NOT Ollama: no provider is inferred from the port.
        assert!(provider_from_url("http://127.0.0.1:20128/v1").is_none());
        assert!(provider_from_url("http://localhost:8000/v1").is_none());
    }

    /// The agent talks `{base_url}/chat/completions` (OpenAI-compatible). A
    /// catalog default that is the vendor's NATIVE-protocol root 404s on the
    /// first request of a freshly connected provider.
    #[test]
    fn catalog_urls_are_openai_compatible_roots() {
        let url = |n: &str| provider_by_name(n).unwrap().default_base_url;
        assert_eq!(url("anthropic"), "https://api.anthropic.com/v1");
        assert_eq!(
            url("google"),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        assert_eq!(url("cohere"), "https://api.cohere.ai/compatibility/v1");
        for p in PROVIDERS
            .iter()
            .filter(|p| p.default_base_url.starts_with("http"))
        {
            assert!(
                !p.default_base_url.ends_with("/anthropic"),
                "{} points at an Anthropic-protocol root",
                p.name
            );
        }
        // Detection still recognises the corrected URLs.
        assert_eq!(
            provider_from_url(url("anthropic")).unwrap().name,
            "anthropic"
        );
        assert_eq!(provider_from_url(url("cohere")).unwrap().name, "cohere");
    }
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
}
