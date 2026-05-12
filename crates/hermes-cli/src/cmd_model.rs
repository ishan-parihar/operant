//! CLI subcommand for viewing and modifying model configuration.
//!
//! # Usage
//!
//! - `hermes model` — show current model name, provider, base URL, streaming settings
//! - `hermes model set <name>` — change the model name at runtime (in-memory only)

use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::{install_runtime_config, runtime_config, AppConfig};
use serde_json::Value;

/// Manage the active model configuration.
///
/// Changes affect only the current session and are not persisted to disk.
#[derive(Debug, Clone, Subcommand)]
pub enum ModelSubcommand {
    /// Display the current model and provider configuration.
    Show,

    /// Change the model name in the runtime configuration.
    ///
    /// The new name takes effect immediately for the current session
    /// but is **not** written back to any config file.
    Set {
        /// The model identifier (e.g. "gpt-4", "claude-3-opus-20240229").
        name: String,
    },
}

/// Attempt to map a base URL to a human-readable provider name.
///
/// Falls back to the hostname portion of the URL when no well-known
/// provider pattern is detected.
fn infer_provider(base_url: &str) -> &str {
    let lower = base_url.to_lowercase();
    if lower.contains("openai.com") {
        "OpenAI"
    } else if lower.contains("azure.com") {
        "Azure OpenAI"
    } else if lower.contains("anthropic.com") {
        "Anthropic"
    } else if lower.contains("generativelanguage") || lower.contains("googleapis") {
        "Google Gemini"
    } else if lower.contains("groq.com") {
        "Groq"
    } else if lower.contains("together.xyz") {
        "Together AI"
    } else if lower.contains("deepseek.com") {
        "DeepSeek"
    } else if lower.contains("mistral.ai") {
        "Mistral"
    } else if lower.contains("cohere.ai") {
        "Cohere"
    } else if lower.contains("perplexity") || lower.contains("perplexica") {
        "Perplexity"
    } else if lower.contains("openrouter") {
        "OpenRouter"
    } else if lower.contains("xai") || lower.contains("x.ai") {
        "xAI"
    } else {
        // Fallback: extract the hostname from the original URL (not `lower`)
        // to avoid lifetime issues with the local variable
        "custom"
    }
}

/// Dispatch the model subcommand.
///
/// `config` is the loaded (but possibly CLI-overridden) configuration used for
/// the `Show` variant.  The `Set` variant reads and mutates the installed
/// runtime configuration so that any prior overrides are preserved.
pub async fn handle_model_command(config: &AppConfig, cmd: ModelSubcommand) -> Result<()> {
    match cmd {
        ModelSubcommand::Show => show_model(config),
        ModelSubcommand::Set { name } => set_model(&name).await,
    }
}

/// Print the current model configuration to stdout.
fn show_model(config: &AppConfig) -> Result<()> {
    let provider = infer_provider(&config.client.base_url);
    let stream_label = if config.agent.stream { "enabled" } else { "disabled" };
    let api_key_label = if config.client.api_key.is_some() {
        "configured"
    } else {
        "not set"
    };

    println!("── Model Configuration ──────────────────────────────");
    println!("  Model name:      {}", config.agent.model);
    println!("  Provider:        {}", provider);
    println!("  Base URL:        {}", config.client.base_url);
    println!("  API key:         {}", api_key_label);
    println!("  Streaming:       {}", stream_label);
    println!("  Context window:  {}", config.agent.context_window);
    println!("  Max iterations:  {}", config.agent.max_iterations);
    println!("────────────────────────────────────────────────────");

    Ok(())
}

/// Set the model name in the runtime (in-memory) configuration.
///
/// Uses the `serde_json::Value` round-trip approach to surgically change
/// only the `agent.model` field while preserving all other values.
async fn set_model(name: &str) -> Result<()> {
    let current = runtime_config();

    let mut value: Value = serde_json::to_value(&current)?;

    // Navigate to the nested "agent" object and insert the new model key.
    if let Some(agent) = value
        .as_object_mut()
        .and_then(|obj| obj.get_mut("agent"))
        .and_then(|v| v.as_object_mut())
    {
        agent.insert("model".to_string(), Value::String(name.to_string()));
    }

    let updated: AppConfig = serde_json::from_value(value)?;
    install_runtime_config(updated);

    println!("Model set to: {}", name);
    Ok(())
}
