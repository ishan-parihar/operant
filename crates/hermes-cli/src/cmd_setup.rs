//! Interactive setup wizard for Hermes-RS.
//!
//! `hermes setup` walks through model configuration, provider setup (API key),
//! terminal preferences, and tool settings.  Defaults are pre-filled from the
//! existing (or built-in) configuration.
//!
//! # Modes
//!
//! | Flag             | Behaviour                                                |
//! |------------------|----------------------------------------------------------|
//! | (none)           | Interactive wizard – prompts for each section            |
//! | `--non-interactive` | Print current setup report, exit                   |
//! | `--reset`        | Reset config to factory defaults (after confirmation)    |
//! | `--reconfigure`  | Re-run **all** wizard steps (ignore saved values)        |
//! | `--quick`        | Only ask model + API key, skip advanced sections         |

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use console::style;
use hermes_core::config::{default_config_paths, install_runtime_config, AppConfig};

// ---------------------------------------------------------------------------
// Subcommand enum (non-interactive modes)
// ---------------------------------------------------------------------------

/// Non-interactive setup operations.
#[derive(Debug, Clone, Subcommand)]
pub enum SetupSubcommand {
    /// Show current setup status (what's configured, what's missing)
    Status,
    /// Reset configuration to factory defaults
    Reset,
    /// Re-run all wizard steps
    Reconfigure,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Dispatch the `hermes setup` command.
pub async fn handle_setup_command(
    config: &AppConfig,
    non_interactive: bool,
    reset: bool,
    reconfigure: bool,
    quick: bool,
) -> Result<()> {
    // --non-interactive: show report only
    if non_interactive {
        show_setup_status(config);
        return Ok(());
    }

    // --reset: confirm and reset
    if reset {
        return reset_config().await;
    }

    // Interactive wizard (default)
    run_setup_wizard(config, reconfigure, quick).await
}

// ---------------------------------------------------------------------------
// Non-interactive status report
// ---------------------------------------------------------------------------

fn show_setup_status(config: &AppConfig) {
    let provider = infer_provider(&config.client.base_url);

    println!("{}", style("── Hermes Setup Status ──────────────────────").bold().cyan());
    println!();

    // Model & provider
    let model_ok = !config.agent.model.is_empty() && config.agent.model != "gpt-4";
    print_status("Model", &config.agent.model, model_ok);
    print_status("Provider", provider, true);

    // API key
    let key_ok = config.client.api_key.as_ref().map_or(false, |k| !k.is_empty());
    print_status(
        "API key",
        if key_ok { "configured" } else { "not set" },
        key_ok,
    );

    // Base URL
    print_status("Base URL", &config.client.base_url, true);

    // Streaming
    print_status(
        "Streaming",
        if config.agent.stream { "enabled" } else { "disabled" },
        true,
    );

    // Terminal theme
    print_status("Theme", &config.tui.theme, true);

    // Rich output
    print_status("Rich output", if config.tui.rich_output { "on" } else { "off" }, true);

    // MCP autoload
    print_status(
        "MCP autoload",
        if config.mcp.autoload { "enabled" } else { "disabled" },
        true,
    );

    // Database path
    print_status("Database", &config.database_path.display().to_string(), true);

    println!();
    println!("{}", style("──────────────────────────────────────────────").bold().cyan());
}

fn print_status(label: &str, value: &str, ok: bool) {
    let icon = if ok {
        style("✓").green()
    } else {
        style("✗").red()
    };
    println!("  {} {}: {}", icon, style(label).bold(), value);
}

// ---------------------------------------------------------------------------
// Reset
// ---------------------------------------------------------------------------

async fn reset_config() -> Result<()> {
    let proceed = dialoguer::Confirm::new()
        .with_prompt("Reset all configuration to factory defaults? This cannot be undone.")
        .default(false)
        .interact()
        .context("Failed to read confirmation")?;

    if !proceed {
        println!("Reset cancelled.");
        return Ok(());
    }

    let defaults = AppConfig::default();
    install_runtime_config(defaults.clone());
    persist_config(&defaults)?;

    println!("{} Configuration reset to defaults.", style("✓").green());
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive wizard
// ---------------------------------------------------------------------------

async fn run_setup_wizard(config: &AppConfig, reconfigure: bool, quick: bool) -> Result<()> {
    let mut updated = config.clone();

    println!();
    println!(
        "{}",
        style("╔══════════════════════════════════════════════╗")
            .bold()
            .cyan()
    );
    println!(
        "{}",
        style("║         Hermes Setup Wizard                  ║")
            .bold()
            .cyan()
    );
    println!(
        "{}",
        style("╚══════════════════════════════════════════════╝")
            .bold()
            .cyan()
    );
    println!();

    // Step 1: Model configuration
    step_model(&mut updated, reconfigure).await?;

    // Step 2: Provider & API key
    step_provider(&mut updated, reconfigure).await?;

    if !quick {
        // Step 3: Terminal preferences
        step_terminal(&mut updated, reconfigure).await?;

        // Step 4: Tool & behaviour settings
        step_tools(&mut updated, reconfigure).await?;
    }

    // Apply changes
    install_runtime_config(updated.clone());
    persist_config(&updated)?;

    println!();
    println!(
        "{} {}",
        style("✓").green().bold(),
        style("Setup complete! Configuration saved.").green().bold()
    );

    if quick {
        println!(
            "  {} Run {} for full configuration.",
            style("ℹ").cyan(),
            style("hermes setup").bold()
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Wizard steps
// ---------------------------------------------------------------------------

/// Step 1: Choose a model.
async fn step_model(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    println!("{}", style("── Step 1: Model Configuration ───────────────").bold().cyan());
    println!("Choose the LLM model Hermes will use.");

    let providers = &[
        ("OpenAI", vec![
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
            "gpt-4",
            "gpt-3.5-turbo",
        ]),
        ("Anthropic", vec![
            "claude-3-5-sonnet-20241022",
            "claude-3-opus-20240229",
            "claude-3-sonnet-20240229",
            "claude-3-haiku-20240307",
        ]),
        ("Google", vec![
            "gemini-1.5-pro",
            "gemini-1.5-flash",
            "gemini-1.0-pro",
        ]),
        ("Mistral", vec![
            "mistral-large-latest",
            "mistral-medium-latest",
            "mistral-small-latest",
        ]),
        ("Groq", vec![
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "mixtral-8x7b-32768",
        ]),
        ("DeepSeek", vec![
            "deepseek-chat",
            "deepseek-reasoner",
        ]),
        ("Together AI", vec!["mistralai/Mixtral-8x22B-Instruct-v0.1"]),
        ("OpenRouter", vec![
            "openrouter/auto",
            "anthropic/claude-3.5-sonnet",
        ]),
    ];

    // Build flat list of model options + "Custom" at the end
    let mut model_options: Vec<String> = Vec::new();
    let mut model_to_provider: Vec<(&str, usize)> = Vec::new(); // (model_name, provider_idx)

    for (p_idx, (provider, models)) in providers.iter().enumerate() {
        for model in models {
            model_options.push(format!("{}  ({})", model, provider));
            model_to_provider.push((model, p_idx));
        }
    }
    model_options.push("Custom (enter model name manually)".to_string());

    let default_idx = if config.agent.model != "gpt-4" {
        model_to_provider
            .iter()
            .position(|(m, _)| *m == config.agent.model)
            .unwrap_or(0)
    } else {
        0
    };

    let selection = dialoguer::FuzzySelect::new()
        .with_prompt("Select a model")
        .items(&model_options)
        .default(default_idx)
        .interact()
        .context("Failed to select model")?;

    if selection == model_options.len() - 1 {
        // Custom model
        let custom: String = dialoguer::Input::new()
            .with_prompt("Enter model name")
            .default(config.agent.model.clone())
            .interact_text()
            .context("Failed to read model name")?;
        config.agent.model = custom;
    } else {
        let (model_name, p_idx) = model_to_provider[selection];
        config.agent.model = model_name.to_string();

        // Auto-set base URL + model for known providers
        let (base_url, model) = provider_defaults(providers[p_idx].0);
        if let Some(url) = base_url {
            config.client.base_url = url;
        }
        if let Some(m) = model {
            config.agent.model = m;
        }

        // If user selected this provider, show the provider name
        println!(
            "  {} Set model to {} ({})",
            style("✓").green(),
            style(&config.agent.model).bold(),
            providers[p_idx].0
        );
    }

    println!();
    Ok(())
}

/// Step 2: Provider / API key.
async fn step_provider(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    println!("{}", style("── Step 2: Provider & API Key ─────────────────").bold().cyan());
    println!("Configure your LLM provider endpoint and authentication.");

    // Provider selection (base URL)
    let known_providers = &[
        ("OpenAI",   "https://api.openai.com/v1"),
        ("Anthropic","https://api.anthropic.com/v1"),
        ("Google Gemini", "https://generativelanguage.googleapis.com/v1beta"),
        ("Groq",     "https://api.groq.com/openai/v1"),
        ("DeepSeek", "https://api.deepseek.com/v1"),
        ("Mistral",  "https://api.mistral.ai/v1"),
        ("Together AI", "https://api.together.xyz/v1"),
        ("OpenRouter", "https://openrouter.ai/api/v1"),
        ("xAI",      "https://api.x.ai/v1"),
        ("Custom",   ""),
    ];

    let provider_names: Vec<&str> = known_providers.iter().map(|(n, _)| *n).collect();
    let current_provider_name = infer_provider(&config.client.base_url);

    let default_provider = known_providers
        .iter()
        .position(|(n, _)| n == &current_provider_name)
        .unwrap_or(0);

    let sel = dialoguer::Select::new()
        .with_prompt("Select your LLM provider")
        .items(&provider_names)
        .default(default_provider)
        .interact()
        .context("Failed to select provider")?;

    let (_, base_url) = known_providers[sel];
    if !base_url.is_empty() {
        config.client.base_url = base_url.to_string();
        println!("  {} Base URL set to {}", style("✓").green(), base_url);
    } else {
        // Custom URL
        let custom_url = dialoguer::Input::new()
            .with_prompt("Enter base URL (e.g. https://api.openai.com/v1)")
            .default(config.client.base_url.clone())
            .interact_text()
            .context("Failed to read base URL")?;
        config.client.base_url = custom_url;
    }

    println!();

    // API key
    let current_key = config.client.api_key.as_deref().unwrap_or("");
    let masked = if current_key.is_empty() {
        String::new()
    } else if current_key.len() > 8 {
        format!("{}…{}", &current_key[..4], &current_key[current_key.len() - 4..])
    } else {
        "configured".to_string()
    };

    let prompt = if masked.is_empty() {
        "Enter your API key".to_string()
    } else {
        format!("Enter your API key (currently {})", masked)
    };

        let key: String = dialoguer::Input::new()
            .with_prompt(&prompt)
            .allow_empty(true)
            .interact_text()
            .context("Failed to read API key")?;

    if !key.is_empty() {
        config.client.api_key = Some(key);
        println!("  {} API key updated", style("✓").green());
    } else if masked.is_empty() {
        println!(
            "  {} {}",
            style("⚠").yellow(),
            style("No API key set. Set OPENAI_API_KEY or run setup again.").yellow()
        );
    }

    println!();
    Ok(())
}

/// Step 3: Terminal / TUI preferences.
async fn step_terminal(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    println!("{}", style("── Step 3: Terminal Preferences ───────────────").bold().cyan());
    println!("Customise how Hermes behaves in the terminal.");

    // Stream toggle
    let stream = dialoguer::Confirm::new()
        .with_prompt("Enable streaming responses?")
        .default(config.agent.stream)
        .interact()
        .context("Failed to read streaming preference")?;
    config.agent.stream = stream;

    // Theme
    let themes = &["opencode", "dark", "light", "dracula", "monokai"];
    let current_theme_idx = themes
        .iter()
        .position(|t| *t == config.tui.theme)
        .unwrap_or(0);

    let theme_sel = dialoguer::Select::new()
        .with_prompt("Select terminal theme")
        .items(themes)
        .default(current_theme_idx)
        .interact()
        .context("Failed to select theme")?;
    config.tui.theme = themes[theme_sel].to_string();

    // Show reasoning
    let show_reasoning = dialoguer::Confirm::new()
        .with_prompt("Show model reasoning in output?")
        .default(config.agent.show_reasoning)
        .interact()
        .context("Failed to read reasoning preference")?;
    config.agent.show_reasoning = show_reasoning;

    println!();
    Ok(())
}

/// Step 4: Tool & behaviour settings.
async fn step_tools(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    println!("{}", style("── Step 4: Tool & Behaviour Settings ──────────").bold().cyan());
    println!("Configure tool usage and behaviour limits.");

    // Max iterations
    let iterations = dialoguer::Input::new()
        .with_prompt("Maximum reasoning iterations")
        .default(config.agent.max_iterations.to_string())
        .validate_with(|input: &String| -> Result<(), &str> {
            input
                .parse::<usize>()
                .map(|_| ())
                .map_err(|_| "Enter a valid number")
        })
        .interact_text()
        .context("Failed to read iterations")?;
    config.agent.max_iterations = iterations.parse::<usize>().unwrap_or(20);

    // Tool timeout
    let tool_timeout = dialoguer::Input::new()
        .with_prompt("Tool timeout (seconds)")
        .default(config.agent.tool_timeout_secs.to_string())
        .validate_with(|input: &String| -> Result<(), &str> {
            input.parse::<u64>().map(|_| ()).map_err(|_| "Enter a valid number")
        })
        .interact_text()
        .context("Failed to read tool timeout")?;
    config.agent.tool_timeout_secs = tool_timeout.parse::<u64>().unwrap_or(30);

    // MCP autoload
    let autoload = dialoguer::Confirm::new()
        .with_prompt("Auto-load MCP servers?")
        .default(config.mcp.autoload)
        .interact()
        .context("Failed to read MCP autoload preference")?;
    config.mcp.autoload = autoload;

    // Rich output
    let rich_output = dialoguer::Confirm::new()
        .with_prompt("Use rich TUI output?")
        .default(config.tui.rich_output)
        .interact()
        .context("Failed to read rich output preference")?;
    config.tui.rich_output = rich_output;

    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Save the configuration to disk.
///
/// Writes to the first existing config path from `default_config_paths()`, or
/// creates `hermes.toml` in the current directory if none exist yet.
fn persist_config(config: &AppConfig) -> Result<()> {
    let paths = default_config_paths();
    let config_path = paths
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("hermes.toml"));

    let toml_str =
        toml::to_string_pretty(config).context("Failed to serialise configuration as TOML")?;

    std::fs::write(&config_path, &toml_str)
        .with_context(|| format!("Failed to write config to '{}'", config_path.display()))?;

    println!(
        "  {} Configuration written to {}",
        style("✓").green(),
        style(config_path.display()).bold()
    );

    Ok(())
}

/// Map a base URL to a human-readable provider name.
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
    } else if lower.contains("openrouter") {
        "OpenRouter"
    } else if lower.contains("xai") || lower.contains("x.ai") {
        "xAI"
    } else {
        "Custom"
    }
}

/// Return (base_url, default_model) for a well-known provider.
fn provider_defaults(provider: &str) -> (Option<String>, Option<String>) {
    match provider {
        "OpenAI" => (
            Some("https://api.openai.com/v1".into()),
            Some("gpt-4o".into()),
        ),
        "Anthropic" => (
            Some("https://api.anthropic.com/v1".into()),
            Some("claude-3-5-sonnet-20241022".into()),
        ),
        "Google" => (
            Some("https://generativelanguage.googleapis.com/v1beta".into()),
            Some("gemini-1.5-pro".into()),
        ),
        "Groq" => (
            Some("https://api.groq.com/openai/v1".into()),
            Some("llama-3.3-70b-versatile".into()),
        ),
        "DeepSeek" => (
            Some("https://api.deepseek.com/v1".into()),
            Some("deepseek-chat".into()),
        ),
        "Mistral" => (
            Some("https://api.mistral.ai/v1".into()),
            Some("mistral-large-latest".into()),
        ),
        "Together AI" => (
            Some("https://api.together.xyz/v1".into()),
            Some("mistralai/Mixtral-8x22B-Instruct-v0.1".into()),
        ),
        "OpenRouter" => (
            Some("https://openrouter.ai/api/v1".into()),
            Some("openrouter/auto".into()),
        ),
        "xAI" => (Some("https://api.x.ai/v1".into()), Some("grok-2-latest".into())),
        _ => (None, None),
    }
}
