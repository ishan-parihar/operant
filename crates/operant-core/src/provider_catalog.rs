//! provider_catalog — the static provider-name catalog `operant doctor`
//! validates configuration against.
//!
//! (iter-1215, closes RT-153) This table replaces the `operant-providers`
//! crate's entire role in the product: the runtime's model path is
//! `crate::client` (an OpenAI-compatible client and an Anthropic client),
//! and the only production caller of the factory was the doctor, which
//! asked exactly two questions per configured name — "is this name known"
//! and "which wire protocol does it speak". A 157-name static table
//! answers both; the factory, its ~20 provider backends and its private
//! dependency tree are gone.
//!
//! The name set is the factory's recognition surface, extracted verbatim
//! (every match-arm literal and every alias-family member). The wire APIs
//! mirror the trait defaults the factory exposed: everything speaks
//! `chat_completions` except the four native implementations.
//!
//! Known, deliberate differences from the factory:
//! - The factory's API-key pre-flight (prefix-mismatch heuristics against
//!   ambient env vars) is not reproduced — the doctor has its own key
//!   presence checks, and validation that depends on which env vars happen
//!   to be set was environment coupling, not validation.
//! - `default_provider_config` (per-provider default temperature/timeout
//!   pre-fill) is not reproduced — it had no caller even when the crate
//!   lived (recorded then as RT-90).

/// OpenAI-style chat completions — the runtime's primary wire protocol.
pub const WIRE_CHAT_COMPLETIONS: &str = "chat_completions";
/// Anthropic Messages API — the runtime's second wire protocol.
pub const WIRE_MESSAGES: &str = "messages";
/// AWS Bedrock Converse API — native, NOT spoken by the runtime.
pub const WIRE_CONVERSE: &str = "converse";
/// Gemini GenerateContent API — native, NOT spoken by the runtime.
pub const WIRE_GENERATE_CONTENT: &str = "generate_content";
/// OpenAI Responses API (Codex-style) — NOT spoken by the runtime.
pub const WIRE_RESPONSES: &str = "responses";

/// Providers the runtime serves through its OpenAI-compatible client
/// (plus the two native-protocol holdouts that keep their own entries).
const CHAT_COMPLETIONS_PROVIDERS: &[&str] = &[
    "01ai",
    "ai21",
    "ai21-labs",
    "aihubmix",
    "aliyun",
    "aliyun-bailian",
    "anyscale",
    "ark",
    "astrai",
    "atomic",
    "atomic-chat",
    "atomic_chat",
    "avian",
    "azure",
    "azure-openai",
    "azure_openai",
    "baichuan",
    "baidu",
    "bailian",
    "baseten",
    "bigmodel",
    "build.nvidia.com",
    "cerebras",
    "claude-code",
    "cloudflare",
    "cloudflare-ai",
    "cohere",
    "copilot",
    "dashscope",
    "dashscope-cn",
    "dashscope-international",
    "dashscope-intl",
    "dashscope-us",
    "deep-infra",
    "deep-myst",
    "deepinfra",
    "deepmyst",
    "deepseek",
    "doubao",
    "doubao-cn",
    "fireworks",
    "fireworks-ai",
    "friendli",
    "friendliai",
    "gemini-cli",
    "github-copilot",
    "github-models",
    "github_models",
    "glm",
    "glm-cn",
    "glm-global",
    "grok",
    "groq",
    "hf",
    "huggingface",
    "hunyuan",
    "hyperbolic",
    "kilo",
    "kilocli",
    "kimi",
    "kimi-cn",
    "kimi-code",
    "kimi-global",
    "kimi-intl",
    "kimi_coding",
    "kimi_for_coding",
    "lepton",
    "lepton-ai",
    "lingyiwanwu",
    "lite-llm",
    "litellm",
    "llama.cpp",
    "llamacpp",
    "lm-studio",
    "lmstudio",
    "manifest",
    "minimax",
    "minimax-cn",
    "minimax-global",
    "minimax-intl",
    "minimax-io",
    "minimax-oauth",
    "minimax-oauth-cn",
    "minimax-oauth-global",
    "minimax-portal",
    "minimax-portal-cn",
    "minimax-portal-global",
    "minimaxi",
    "mistral",
    "moonshot",
    "moonshot-cn",
    "moonshot-global",
    "moonshot-intl",
    "morph",
    "nebius",
    "novita",
    "nscale",
    "nvidia",
    "nvidia-nim",
    "ollama",
    "openai",
    "opencode",
    "opencode-go",
    "opencode-zen",
    "openrouter",
    "osaurus",
    "ovh",
    "ovhcloud",
    "perplexity",
    "qianfan",
    "qwen",
    "qwen-cn",
    "qwen-code",
    "qwen-international",
    "qwen-intl",
    "qwen-oauth",
    "qwen-us",
    "qwen_oauth",
    "reka",
    "sambanova",
    "sglang",
    "silicon-flow",
    "siliconflow",
    "step",
    "step-intl",
    "stepfun",
    "stepfun-intl",
    "synthetic",
    "telnyx",
    "tencent",
    "together",
    "together-ai",
    "venice",
    "vercel",
    "vercel-ai",
    "vllm",
    "volcengine",
    "xai",
    "yi",
    "z.ai",
    "z.ai-cn",
    "z.ai-global",
    "zai",
    "zai-cn",
    "zai-global",
    "zhipu",
    "zhipu-cn",
    "zhipu-global",
];

/// Providers that speak Anthropic's Messages API natively.
const MESSAGES_PROVIDERS: &[&str] = &["anthropic"];

/// Providers that speak Bedrock's Converse API natively.
const CONVERSE_PROVIDERS: &[&str] = &["aws-bedrock", "bedrock"];

/// Providers that speak Gemini's GenerateContent API natively.
const GENERATE_CONTENT_PROVIDERS: &[&str] = &["gemini", "google", "google-gemini"];

/// Providers that speak the OpenAI Responses API natively.
const RESPONSES_PROVIDERS: &[&str] = &["codex", "openai-codex", "openai_codex"];

/// The wire protocol a configured provider name speaks, or `None` when the
/// name is not known.
///
/// URL-shaped escapes: `custom:<url>` names an arbitrary OpenAI-compatible
/// endpoint (chat completions); `anthropic-custom:<url>` an
/// Anthropic-compatible one (messages).
pub fn known_provider_wire_api(name: &str) -> Option<&'static str> {
    if let Some(url) = name.strip_prefix("custom:")
        && !url.trim().is_empty()
    {
        return Some(WIRE_CHAT_COMPLETIONS);
    }
    if let Some(url) = name.strip_prefix("anthropic-custom:")
        && !url.trim().is_empty()
    {
        return Some(WIRE_MESSAGES);
    }
    for table in [
        (CHAT_COMPLETIONS_PROVIDERS, WIRE_CHAT_COMPLETIONS),
        (MESSAGES_PROVIDERS, WIRE_MESSAGES),
        (CONVERSE_PROVIDERS, WIRE_CONVERSE),
        (GENERATE_CONTENT_PROVIDERS, WIRE_GENERATE_CONTENT),
        (RESPONSES_PROVIDERS, WIRE_RESPONSES),
    ] {
        if table.0.contains(&name) {
            return Some(table.1);
        }
    }
    None
}

/// The error text for an unknown provider name — the factory's tips,
/// unchanged, so the doctor's output stays familiar.
pub fn unknown_provider_error(name: &str) -> String {
    format!(
        "Unknown provider: {name}. Check README for supported providers or run `operant onboard` to reconfigure.\n\
         Tip: Use \"custom:https://your-api.com\" for OpenAI-compatible endpoints.\n\
         Tip: Use \"anthropic-custom:https://your-api.com\" for Anthropic-compatible endpoints."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_native_providers_keep_their_own_wire_apis() {
        assert_eq!(known_provider_wire_api("anthropic"), Some(WIRE_MESSAGES));
        assert_eq!(known_provider_wire_api("bedrock"), Some(WIRE_CONVERSE));
        assert_eq!(known_provider_wire_api("aws-bedrock"), Some(WIRE_CONVERSE));
        assert_eq!(
            known_provider_wire_api("gemini"),
            Some(WIRE_GENERATE_CONTENT)
        );
        assert_eq!(
            known_provider_wire_api("google-gemini"),
            Some(WIRE_GENERATE_CONTENT)
        );
        assert_eq!(known_provider_wire_api("codex"), Some(WIRE_RESPONSES));
        assert_eq!(
            known_provider_wire_api("openai-codex"),
            Some(WIRE_RESPONSES)
        );
    }

    #[test]
    fn compatible_providers_are_chat_completions() {
        for name in [
            "openai",
            "openrouter",
            "ollama",
            "groq",
            "mistral",
            "deepseek",
            "xai",
            "grok",
            "github-models",
            "huggingface",
            "hf",
            "kimi",
            "moonshot-intl",
            "minimax-portal-global",
            "glm-cn",
            "zhipu",
            "qwen-oauth",
            "dashscope-us",
            "zai",
            "z.ai-cn",
            "qianfan",
            "doubao",
            "bailian",
            "step",
            "yi",
            "01ai",
            "azure_openai",
            "llama.cpp",
            "build.nvidia.com",
        ] {
            assert_eq!(
                known_provider_wire_api(name),
                Some(WIRE_CHAT_COMPLETIONS),
                "{name} must stay known and chat-completions"
            );
        }
    }

    #[test]
    fn url_shaped_escapes_are_known() {
        assert_eq!(
            known_provider_wire_api("custom:https://gw.internal/v1"),
            Some(WIRE_CHAT_COMPLETIONS)
        );
        assert_eq!(
            known_provider_wire_api("anthropic-custom:https://proxy.example.com"),
            Some(WIRE_MESSAGES)
        );
        // An escape with no URL is not a provider.
        assert_eq!(known_provider_wire_api("custom:"), None);
    }

    #[test]
    fn unknown_names_error_with_the_factory_tips() {
        assert_eq!(known_provider_wire_api("not-a-provider"), None);
        let msg = unknown_provider_error("not-a-provider");
        assert!(msg.contains("custom:https://your-api.com"), "{msg}");
        assert!(msg.contains("anthropic-custom:"), "{msg}");
    }

    /// The tables are the recognition surface — a duplicate name across two
    /// wire APIs would make validation depend on table order.
    #[test]
    fn no_name_appears_in_two_wire_tables() {
        let mut seen: Vec<&str> = Vec::new();
        for table in [
            CHAT_COMPLETIONS_PROVIDERS,
            MESSAGES_PROVIDERS,
            CONVERSE_PROVIDERS,
            GENERATE_CONTENT_PROVIDERS,
            RESPONSES_PROVIDERS,
        ] {
            for name in table {
                assert!(!seen.contains(name), "duplicate provider name: {name}");
                seen.push(name);
            }
        }
    }
}
