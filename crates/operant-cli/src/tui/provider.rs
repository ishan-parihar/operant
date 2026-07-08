/// Detect the provider from a model name.
/// Handles both prefixed ("openai/gpt-4.1") and bare ("claude-opus-4-6") formats.
pub fn infer_provider_from_model(model: &str) -> Option<String> {
    if model == "free/auto"
        || model.starts_with("free/")
        || model.starts_with("zen/")
        || model.starts_with("opencode-zen/")
    {
        return Some("free".to_string());
    }
    if let Some((provider, _)) = model.split_once('/') {
        let known = [
            "anthropic",
            "openai",
            "google",
            "groq",
            "cerebras",
            "deepseek",
            "mistral",
            "xai",
            "openrouter",
            "github-copilot",
            "codex",
            "cohere",
            "perplexity",
            "togetherai",
            "together-ai",
            "deepinfra",
            "venice",
            "minimax",
            "sambanova",
            "nvidia",
            "moonshotai",
            "zhipuai",
            "siliconflow",
            "ollama",
            "lmstudio",
            "llamacpp",
            "azure",
            "amazon-bedrock",
        ];
        if known.contains(&provider) {
            return Some(provider.to_string());
        }
    }
    let lower = model.to_lowercase();
    if lower.starts_with("claude") {
        Some("anthropic".to_string())
    } else if lower.starts_with("gpt") || lower.starts_with("o1") || lower.starts_with("o3") {
        Some("openai".to_string())
    } else if lower.starts_with("gemini") || lower.starts_with("gemma") {
        Some("google".to_string())
    } else if lower.starts_with("deepseek") {
        Some("deepseek".to_string())
    } else {
        None
    }
}
