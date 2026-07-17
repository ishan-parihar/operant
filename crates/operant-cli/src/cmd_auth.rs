//! CLI auth subcommand module for Operant-RS.
//!
//! Implements `operant auth` (list/add/remove/reset/status),
//! `operant fallback` (list/add/remove/clear), and
//! `operant login` / `operant logout`.
//!
//! # Credential safety
//!
//! This module **never** prints full API keys.  Key values are always
//! truncated to a short prefix hint (`sk-an...`) when displayed.

use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::credential_pool::{AuthType, CredentialPool, PooledCredential};

// ---------------------------------------------------------------------------
// Subcommand enums
// ---------------------------------------------------------------------------

/// Manage credentials for LLM providers.
#[derive(Debug, Clone, Subcommand)]
pub enum AuthSubcommand {
    /// List all configured credentials (shows provider + key hints only).
    List,

    /// Add a credential for a provider.
    Add {
        /// Provider name (e.g. "openai", "anthropic")
        provider: String,

        /// API key value
        key: String,

        /// Optional human-readable label
        label: Option<String>,
    },

    /// Remove a credential by provider name.
    Remove {
        /// Provider name to remove (e.g. "openai")
        provider: String,
    },

    /// Reset all credentials.
    Reset,

    /// Show which providers have credentials configured.
    Status,
}

// ---------------------------------------------------------------------------
// Known provider / env-var mappings
// ---------------------------------------------------------------------------

/// Static mapping of well-known provider names to their environment variable
/// names, a human-readable description, and a signup URL.  Used by `list`,
/// `status`, `seed`, and `login` operations.
const PROVIDER_ENV_VARS: &[(&str, &str, &str, &str)] = &[
    (
        "openai",
        "OPENAI_API_KEY",
        "OpenAI API key",
        "https://platform.openai.com/api-keys",
    ),
    (
        "anthropic",
        "ANTHROPIC_API_KEY",
        "Anthropic API key",
        "https://console.anthropic.com/",
    ),
    (
        "google",
        "GOOGLE_API_KEY",
        "Google AI API key",
        "https://aistudio.google.com/",
    ),
    ("xai", "XAI_API_KEY", "xAI API key", "https://console.x.ai/"),
    (
        "mistral",
        "MISTRAL_API_KEY",
        "Mistral API key",
        "https://console.mistral.ai/",
    ),
    (
        "groq",
        "GROQ_API_KEY",
        "Groq API key",
        "https://console.groq.com/",
    ),
    (
        "deepseek",
        "DEEPSEEK_API_KEY",
        "DeepSeek API key",
        "https://platform.deepseek.com/",
    ),
    (
        "together",
        "TOGETHER_API_KEY",
        "Together AI API key",
        "https://api.together.xyz/",
    ),
    (
        "openrouter",
        "OPENROUTER_API_KEY",
        "OpenRouter API key",
        "https://openrouter.ai/",
    ),
    (
        "nvidia",
        "NVIDIA_API_KEY",
        "NVIDIA API key",
        "https://build.nvidia.com/",
    ),
    (
        "cohere",
        "COHERE_API_KEY",
        "Cohere API key",
        "https://dashboard.cohere.com/",
    ),
    (
        "perplexity",
        "PERPLEXITY_API_KEY",
        "Perplexity API key",
        "https://www.perplexity.ai/settings/api",
    ),
    (
        "azure",
        "AZURE_OPENAI_API_KEY",
        "Azure OpenAI API key",
        "https://portal.azure.com/",
    ),
    (
        "bedrock",
        "AWS_ACCESS_KEY_ID",
        "AWS Access Key ID (for Bedrock)",
        "https://aws.amazon.com/bedrock/",
    ),
    (
        "huggingface",
        "HF_API_TOKEN",
        "HuggingFace API token",
        "https://huggingface.co/settings/tokens",
    ),
    ("ollama", "", "Ollama (local, no API key needed)", ""),
    (
        "replicate",
        "REPLICATE_API_KEY",
        "Replicate API key",
        "https://replicate.com/account",
    ),
    (
        "ai21",
        "AI21_API_KEY",
        "AI21 API key",
        "https://www.ai21.com/",
    ),
    (
        "stabilityai",
        "STABILITY_API_KEY",
        "Stability AI API key",
        "https://platform.stability.ai/",
    ),
    (
        "elevenlabs",
        "ELEVENLABS_API_KEY",
        "ElevenLabs API key",
        "https://elevenlabs.io/",
    ),
];

// ---------------------------------------------------------------------------
// Auth dispatcher
// ---------------------------------------------------------------------------

/// Dispatch and execute an auth subcommand.
pub async fn handle_auth_command(config: &AppConfig, cmd: AuthSubcommand) -> Result<()> {
    match cmd {
        AuthSubcommand::List => handle_auth_list(config),
        AuthSubcommand::Add {
            provider,
            key,
            label,
        } => handle_auth_add(&provider, &key, label.as_deref()),
        AuthSubcommand::Remove { provider } => handle_auth_remove(&provider),
        AuthSubcommand::Reset => handle_auth_reset(),
        AuthSubcommand::Status => handle_auth_status(config),
    }
}

// ---------------------------------------------------------------------------
// Auth handlers
// ---------------------------------------------------------------------------

/// List credentials from environment variables and config.
///
/// For every known provider, attempts to seed a credential pool from the
/// corresponding environment variable.  Prints provider name, a key hint
/// (never the full key), and the source.
fn handle_auth_list(config: &AppConfig) -> Result<()> {
    println!("── Credentials ────────────────────────────────────────");
    println!(" {:<14} {:<28} Source", "Provider", "Key");
    println!(" ─────────────────────────────────────────────────────");

    let mut found_any = false;

    for (provider, env_var, _, _) in PROVIDER_ENV_VARS {
        let pool = CredentialPool::new(provider);
        pool.seed_from_env(env_var);
        let credentials = pool.list();

        if credentials.is_empty() {
            continue;
        }

        found_any = true;
        for cred in &credentials {
            let hint = key_hint(&cred.value, 8);
            println!(" {:<14} {:<28} {}", provider, hint, cred.source);
        }
    }

    // Also check the config-level api_key
    if let Some(ref key) = config.client.api_key {
        if !key.is_empty() {
            found_any = true;
            let hint = key_hint(key, 8);
            println!(" {:<14} {:<28} config (client.api_key)", "default", hint);
        }
    }

    if !found_any {
        println!("  No credentials configured.");
        println!();
        println!("  Set OPENAI_API_KEY or another provider's env var, or use");
        println!("    operant auth add <provider> <key>");
    }

    println!("─────────────────────────────────────────────────────");
    Ok(())
}

/// Add a credential to the in-memory pool for the given provider.
///
/// Note: credentials added this way are ephemeral and do not persist
/// to disk or to environment variables across CLI invocations.
fn handle_auth_add(provider: &str, key: &str, label: Option<&str>) -> Result<()> {
    let pool = CredentialPool::new(provider);
    let name = label.unwrap_or(provider);
    let cred = PooledCredential::new(name, AuthType::ApiKey, key, "manual");
    let id = pool.add(cred);

    // Persist to ~/.operant/.env so the key survives across CLI invocations.
    let env_var = provider_env_var(provider);
    if let Err(e) = crate::env_store::save_env_value(&env_var, key) {
        println!("  Warning: could not persist key to ~/.operant/.env: {}", e);
    } else {
        unsafe {
            std::env::set_var(&env_var, key);
        }
    }

    let hint = key_hint(key, 8);
    println!("Added credential for '{}':", provider);
    println!("  Key hint: {}", hint);
    println!("  Label:    {}", name);
    println!("  ID:       {}", id);
    println!();
    if crate::env_store::get_env_value(&env_var).is_some() {
        println!("  Persisted to ~/.operant/.env as {}", env_var);
    } else {
        println!(
            "Note: could not persist. Set the {} environment variable manually.",
            env_var
        );
    }

    Ok(())
}

/// Remove a credential by provider name.
///
/// Seeds a pool for the given provider and removes any matching
/// credentials.  Reports success even if no credential was found.
fn handle_auth_remove(provider: &str) -> Result<()> {
    let pool = CredentialPool::new(provider);
    let env_var = provider_env_var(provider);
    pool.seed_from_env(&env_var);
    let credentials = pool.list();

    let removed: Vec<String> = credentials
        .iter()
        .map(|c| {
            pool.remove(&c.id);
            c.id.clone()
        })
        .collect();

    if removed.is_empty() {
        println!("No credentials found for '{}'.", provider);
    } else {
        println!(
            "Removed {} credential(s) for '{}'.",
            removed.len(),
            provider
        );
        println!(
            "To permanently remove, unset the {} environment variable.",
            env_var
        );
    }

    Ok(())
}

/// Reset all credentials across known providers.
fn handle_auth_reset() -> Result<()> {
    let mut count = 0usize;

    for (provider, env_var, _, _) in PROVIDER_ENV_VARS {
        let pool = CredentialPool::new(provider);
        pool.seed_from_env(env_var);
        let credentials = pool.list();

        for cred in &credentials {
            pool.remove(&cred.id);
            count += 1;
        }
    }

    if count > 0 {
        println!(
            "Reset {} credential(s) across {} provider(s).",
            count,
            PROVIDER_ENV_VARS.len()
        );
    } else {
        println!("No credentials to reset.");
    }

    println!("To permanently remove credentials, unset the corresponding environment variables.");

    Ok(())
}

/// Show which providers have credentials configured.
fn handle_auth_status(config: &AppConfig) -> Result<()> {
    println!("── Auth Status ────────────────────────────────────────");
    println!(" {:<14} Status", "Provider");
    println!(" ─────────────────────────────────────────────────────");

    for (provider, env_var, _, _) in PROVIDER_ENV_VARS {
        let value = std::env::var(env_var).ok();
        let status = match value {
            Some(v) if !v.trim().is_empty() => {
                let hint = key_hint(&v, 4);
                format!("configured ({})", hint)
            }
            _ => "not set".to_string(),
        };
        println!(" {:<14} {}", provider, status);
    }

    // Config-level key
    let config_status = match &config.client.api_key {
        Some(k) if !k.is_empty() => {
            let hint = key_hint(k, 4);
            format!("configured ({})", hint)
        }
        _ => "not set".to_string(),
    };
    println!(" {:<14} {} (client.api_key)", "default", config_status);

    println!("─────────────────────────────────────────────────────");
    Ok(())
}

// ---------------------------------------------------------------------------
// Login / Logout
// ---------------------------------------------------------------------------

/// Interactive login prompt.
///
/// Presents a list of known providers, prompts for an API key, and stores
/// it in the process environment.
pub async fn handle_login(config: &AppConfig) -> Result<()> {
    use dialoguer::{Password, Select};

    // Build the list of provider items for the selector
    let items: Vec<String> = PROVIDER_ENV_VARS
        .iter()
        .map(|(name, _, desc, url)| {
            if url.is_empty() {
                format!("{}  ─ {}", name, desc)
            } else {
                format!("{}  ─ {}  ({})", name, desc, url)
            }
        })
        .collect();

    println!("── Operant Login ────────────────────────────────────────");
    println!();

    let selection = Select::new()
        .with_prompt("Select a provider")
        .items(&items)
        .default(0)
        .interact()?;

    let (provider, env_var, desc, signup_url) = PROVIDER_ENV_VARS[selection];

    // If the provider has no API key (ollama), just inform
    if env_var.is_empty() {
        println!();
        println!("✓ {} does not require an API key.", provider);
        println!("  It is available out of the box.");
        println!();
        println!("─────────────────────────────────────────────────────");
        return Ok(());
    }

    println!();
    println!("  Provider:  {}", provider);
    println!("  Endpoint:  {}", desc);

    if signup_url.is_empty() {
        println!("  Sign up at the provider's website.");
    } else {
        println!("  Sign up:   {}", signup_url);
    }

    println!();

    let prompt = format!("Enter your API key for {}", provider);
    let key = Password::new().with_prompt(&prompt).interact()?;

    if key.trim().is_empty() {
        println!("No key entered. Login cancelled.");
        return Ok(());
    }

    // Store in process environment (for current process)
    unsafe {
        std::env::set_var(env_var, &key);
    }

    // Persist to ~/.operant/.env so the key survives across CLI invocations.
    // Without this, `operant login` sets the key in-process only — the next
    // `operant` call won't see it.
    if let Err(e) = crate::env_store::save_env_value(env_var, &key) {
        println!("  Warning: could not persist key to ~/.operant/.env: {}", e);
    } else {
        println!("  Persisted to ~/.operant/.env (will load on next run)");
    }

    // Also add to the in-memory credential pool
    {
        let pool = CredentialPool::new(provider);
        let cred = PooledCredential::new(provider, AuthType::ApiKey, &key, "login");
        pool.add(cred);
    }

    // Also update config-level api_key if this is "openai" (backward compat)
    if provider == "openai" {
        let mut updated = config.clone();
        updated.client.api_key = Some(key.clone());
        operant_core::config::install_runtime_config(updated);
    }

    let hint = key_hint(&key, 4);
    println!();
    println!("✓ Logged in to {}  (key: {}...✓)", provider, hint);
    println!("  Environment variable set: {}", env_var);
    println!("─────────────────────────────────────────────────────");
    Ok(())
}

/// Log out by clearing credential pool entries.
///
/// Does **not** unset environment variables or modify config files.
pub async fn handle_logout(config: &AppConfig) -> Result<()> {
    // Clear all credential pools
    let mut count = 0usize;

    for (provider, env_var, _, _) in PROVIDER_ENV_VARS {
        let pool = CredentialPool::new(provider);
        pool.seed_from_env(env_var);
        let credentials = pool.list();
        for cred in &credentials {
            pool.remove(&cred.id);
            count += 1;
        }
    }

    // Also show what was cleared
    if count > 0 {
        println!(
            "✓ Logged out — cleared {} credential(s) from session.",
            count
        );
    } else if config.client.api_key.is_some() {
        println!("✓ Logged out (config-level key remains in operant.toml).");
    } else {
        println!("Already logged out — no credentials found.");
    }

    println!("  To fully remove keys, unset your environment variables or edit operant.toml.");

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return a truncated key hint for display.
///
/// Shows the first `n` characters followed by `"..."`.  If the key is
/// shorter than `n`, shows the entire key followed by `"..."`.
fn key_hint(key: &str, n: usize) -> String {
    let chars: Vec<char> = key.chars().collect();
    let prefix: String = chars.iter().take(n).collect();
    format!("{}...", prefix)
}

/// Resolve the canonical environment variable name for a provider.
///
/// Falls back to `"{PROVIDER}_API_KEY"` (uppercased) for unknown providers.
fn provider_env_var(provider: &str) -> String {
    for (name, var, _, _) in PROVIDER_ENV_VARS {
        if name.eq_ignore_ascii_case(provider) {
            return var.to_string();
        }
    }
    format!("{}_API_KEY", provider.to_uppercase())
}
