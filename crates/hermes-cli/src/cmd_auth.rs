//! CLI auth subcommand module for Hermes-RS.
//!
//! Implements `hermes auth` (list/add/remove/reset/status),
//! `hermes fallback` (list/add/remove/clear), and
//! `hermes login` / `hermes logout`.
//!
//! # Credential safety
//!
//! This module **never** prints full API keys.  Key values are always
//! truncated to a short prefix hint (`sk-an...`) when displayed.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::credential_pool::{AuthType, CredentialPool, PooledCredential};

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

/// Manage fallback models.
#[derive(Debug, Clone, Subcommand)]
pub enum FallbackSubcommand {
    /// List configured fallback models.
    List,

    /// Add a fallback model.
    Add {
        /// Model identifier (e.g. "gpt-4", "claude-3-opus-20240229")
        model: String,
    },

    /// Remove a fallback model.
    Remove {
        /// Model identifier to remove
        model: String,
    },

    /// Clear all fallback models.
    Clear,
}

// ---------------------------------------------------------------------------
// Known provider / env-var mappings
// ---------------------------------------------------------------------------

/// Static mapping of well-known provider names to their environment variable
/// names.  Used by `list`, `status`, and `seed` operations.
const PROVIDER_ENV_VARS: &[(&str, &str)] = &[
    ("openai", "OPENAI_API_KEY"),
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("azure", "AZURE_OPENAI_API_KEY"),
    ("groq", "GROQ_API_KEY"),
    ("mistral", "MISTRAL_API_KEY"),
    ("cohere", "COHERE_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
    ("together", "TOGETHER_API_KEY"),
    ("google", "GOOGLE_API_KEY"),
];

// ---------------------------------------------------------------------------
// Auth dispatcher
// ---------------------------------------------------------------------------

/// Dispatch and execute an auth subcommand.
pub async fn handle_auth_command(config: &AppConfig, cmd: AuthSubcommand) -> Result<()> {
    match cmd {
        AuthSubcommand::List => handle_auth_list(config),
        AuthSubcommand::Add { provider, key, label } => {
            handle_auth_add(&provider, &key, label.as_deref())
        }
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

    for (provider, env_var) in PROVIDER_ENV_VARS {
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
        println!("    hermes auth add <provider> <key>");
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

    let hint = key_hint(key, 8);
    println!("Added credential for '{}':", provider);
    println!("  Key hint: {}", hint);
    println!("  Label:    {}", name);
    println!("  ID:       {}", id);
    println!();
    println!("Note: This credential is stored in-memory for the current session.");
    println!("To persist, set the {} environment variable.", provider_env_var(provider));

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
        println!("Removed {} credential(s) for '{}'.", removed.len(), provider);
        println!("To permanently remove, unset the {} environment variable.", env_var);
    }

    Ok(())
}

/// Reset all credentials across known providers.
fn handle_auth_reset() -> Result<()> {
    let mut count = 0usize;

    for (provider, env_var) in PROVIDER_ENV_VARS {
        let pool = CredentialPool::new(provider);
        pool.seed_from_env(env_var);
        let credentials = pool.list();

        for cred in &credentials {
            pool.remove(&cred.id);
            count += 1;
        }
    }

    if count > 0 {
        println!("Reset {} credential(s) across {} provider(s).", count, PROVIDER_ENV_VARS.len());
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

    for (provider, env_var) in PROVIDER_ENV_VARS {
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
// Fallback dispatcher
// ---------------------------------------------------------------------------

/// Dispatch and execute a fallback subcommand.
pub async fn handle_fallback_command(config: &AppConfig, cmd: FallbackSubcommand) -> Result<()> {
    match cmd {
        FallbackSubcommand::List => handle_fallback_list(config),
        FallbackSubcommand::Add { model } => handle_fallback_add(&model),
        FallbackSubcommand::Remove { model } => handle_fallback_remove(&model),
        FallbackSubcommand::Clear => handle_fallback_clear(),
    }
}

// ---------------------------------------------------------------------------
// Fallback handlers
// ---------------------------------------------------------------------------

/// In-memory list of fallback models for the current session.
///
/// Persistence across CLI invocations would require config-file or database
/// storage, which is outside this module's responsibility.
static FALLBACK_MODELS: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Print the primary model and any configured fallback models.
///
/// The primary model is read from `config.agent.model`.  Fallback models
/// are maintained in-memory for the current session.
fn handle_fallback_list(config: &AppConfig) -> Result<()> {
    let guard = FALLBACK_MODELS
        .lock()
        .unwrap();

    println!("── Fallback Models ────────────────────────────────────");
    println!("  Primary:  {}", config.agent.model);

    if guard.is_empty() {
        println!("  Fallback: (none configured)");
    } else {
        println!("  Fallback:");
        for (i, model) in guard.iter().enumerate() {
            println!("    {}. {}", i + 1, model);
        }
    }

    println!("─────────────────────────────────────────────────────");
    Ok(())
}

/// Add a fallback model.
fn handle_fallback_add(model: &str) -> Result<()> {
    let mut guard = FALLBACK_MODELS
        .lock()
        .unwrap();

    if guard.contains(&model.to_string()) {
        println!("Fallback model '{}' is already configured.", model);
        return Ok(());
    }

    guard.push(model.to_string());
    println!("Added fallback model: {}", model);
    Ok(())
}

/// Remove a fallback model.
fn handle_fallback_remove(model: &str) -> Result<()> {
    let mut guard = FALLBACK_MODELS
        .lock()
        .unwrap();

    let initial_len = guard.len();
    guard.retain(|m| m != model);

    if guard.len() < initial_len {
        println!("Removed fallback model: {}", model);
    } else {
        println!("Fallback model '{}' not found.", model);
    }

    Ok(())
}

/// Clear all fallback models.
fn handle_fallback_clear() -> Result<()> {
    let mut guard = FALLBACK_MODELS
        .lock()
        .unwrap();

    let count = guard.len();
    guard.clear();
    println!("Cleared {} fallback model(s).", count);
    Ok(())
}

// ---------------------------------------------------------------------------
// Login / Logout
// ---------------------------------------------------------------------------

/// Check login status.
///
/// Returns success and prints whether an API key is available from the
/// environment or config.
pub async fn handle_login(config: &AppConfig) -> Result<()> {
    let env_key = std::env::var("OPENAI_API_KEY").ok();
    let config_key = config.client.api_key.as_ref().cloned();

    match (env_key, config_key) {
        (Some(env), _) if !env.trim().is_empty() => {
            let hint = key_hint(&env, 4);
            println!("✓ Logged in (OPENAI_API_KEY = {})", hint);
            println!("  Provider: OpenAI-compatible");
            Ok(())
        }
        (_, Some(cfg)) if !cfg.trim().is_empty() => {
            let hint = key_hint(&cfg, 4);
            println!("✓ Logged in (client.api_key = {})", hint);
            println!("  Provider: OpenAI-compatible");
            Ok(())
        }
        _ => {
            println!("✗ Not logged in.");
            println!();
            println!("  Set the OPENAI_API_KEY environment variable or configure");
            println!("  client.api_key in your hermes.toml config file.");
            println!();
            println!("  Quick start:");
            println!("    export OPENAI_API_KEY=sk-...");
            println!("    hermes login");
            Ok(())
        }
    }
}

/// Log out by clearing credential pool entries.
///
/// Does **not** unset environment variables or modify config files.
pub async fn handle_logout(config: &AppConfig) -> Result<()> {
    // Clear all credential pools
    let mut count = 0usize;

    for (provider, env_var) in PROVIDER_ENV_VARS {
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
        println!("✓ Logged out — cleared {} credential(s) from session.", count);
    } else if config.client.api_key.is_some() {
        println!("✓ Logged out (config-level key remains in hermes.toml).");
    } else {
        println!("Already logged out — no credentials found.");
    }

    println!("  To fully remove keys, unset your environment variables or edit hermes.toml.");

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
    if chars.len() <= n {
        format!("{}...", prefix)
    } else {
        format!("{}...", prefix)
    }
}

/// Resolve the canonical environment variable name for a provider.
///
/// Falls back to `"{PROVIDER}_API_KEY"` (uppercased) for unknown providers.
fn provider_env_var(provider: &str) -> String {
    for (name, var) in PROVIDER_ENV_VARS {
        if name.eq_ignore_ascii_case(provider) {
            return var.to_string();
        }
    }
    format!("{}_API_KEY", provider.to_uppercase())
}
