//! Interactive setup wizard for Operant-RS.
//!
//! `operant setup` walks through model configuration, provider setup (API key),
//! terminal preferences, gateway platforms, tools, TTS, and agent settings.
//! Defaults are pre-filled from the existing (or built-in) configuration.
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

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use console::style;
use operant_core::config::{
    AppConfig, AuxiliaryModelConfig, GatewaySettings, SessionResetMode, TerminalBackend,
    ToolProgressMode, default_config_paths, install_runtime_config,
};

use crate::env_store::{remove_env_value, save_env_value};
use crate::gateway_platforms::all_platforms;
use crate::prompt_helpers::*;
use crate::provider::{PROVIDERS, fetch_models_for_provider, provider_by_name, provider_from_url};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Dispatch the `operant setup` command.
pub async fn handle_setup_command(
    config: &AppConfig,
    section: Option<&str>,
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

    // Default to quick mode unless --reconfigure is set. Quick mode asks
    // only 3 questions (provider, model, key) — enough to start chatting.
    // Use --reconfigure for the full 6-step wizard.
    // (iter-106 — P0 UX fix: setup completion rate was low because the
    // full wizard asks too many questions for a first-time user.)
    let effective_quick = quick || !reconfigure;

    // Interactive wizard — optionally scoped to a single section
    run_setup_wizard(config, section, effective_quick, reconfigure).await
}

// ---------------------------------------------------------------------------
// Non-interactive status report
// ---------------------------------------------------------------------------

fn show_setup_status(config: &AppConfig) {
    let provider_name = provider_name_for_url(&config.client.base_url);

    println!(
        "{}",
        style("── Operant Setup Status ──────────────────────")
            .bold()
            .cyan()
    );
    println!();

    // Model & provider
    let model_ok = !config.agent.model.is_empty() && config.agent.model != "gpt-4";
    print_status("Model", &config.agent.model, model_ok);
    print_status("Provider", provider_name, true);

    // API key
    let key_ok = config
        .client
        .api_key
        .as_ref()
        .is_some_and(|k| !k.is_empty());
    print_status(
        "API key",
        if key_ok { "configured" } else { "not set" },
        key_ok,
    );

    // Additional API keys
    let extra_keys = config.client.additional_api_keys.len();
    if extra_keys > 0 {
        print_status("Additional API keys", &extra_keys.to_string(), true);
    }

    // Base URL
    print_status("Base URL", &config.client.base_url, true);

    // Streaming
    print_status(
        "Streaming",
        if config.agent.stream {
            "enabled"
        } else {
            "disabled"
        },
        true,
    );

    // Terminal theme
    print_status("Theme", &config.tui.theme, true);

    // Rich output
    print_status(
        "Rich output",
        if config.tui.rich_output { "on" } else { "off" },
        true,
    );

    // MCP autoload
    print_status(
        "MCP autoload",
        if config.mcp.autoload {
            "enabled"
        } else {
            "disabled"
        },
        true,
    );

    // Database path
    print_status(
        "Database",
        &config.database_path.display().to_string(),
        true,
    );

    // TTS
    let tts_status = if config.tts.enabled {
        format!("{} (enabled)", config.tts.provider)
    } else {
        "disabled".to_string()
    };
    print_status("TTS", &tts_status, config.tts.enabled);

    // Vision
    let vision_status = match (&config.vision.provider, &config.vision.model) {
        (Some(p), Some(m)) => format!("{} ({})", m, p),
        _ => "not configured".to_string(),
    };
    print_status("Vision", &vision_status, config.vision.provider.is_some());

    // Gateway
    let mut gateway_parts = Vec::new();
    if config.gateway.telegram_enabled {
        gateway_parts.push(format!("{} Telegram", style("✓").green()));
    } else {
        gateway_parts.push(format!("{} Telegram", style("✗").red()));
    }
    if config.gateway.discord_enabled {
        gateway_parts.push(format!("{} Discord", style("✓").green()));
    } else {
        gateway_parts.push(format!("{} Discord", style("✗").red()));
    }
    let gateway_str = gateway_parts.join(", ");
    print_status(
        "Gateways",
        &gateway_str,
        config.gateway.telegram_enabled || config.gateway.discord_enabled,
    );

    // Credential Pool
    let cp_status = match &config.credential_pool.strategy {
        Some(s) if config.credential_pool.enabled => format!("{} (enabled)", s),
        _ => "disabled".to_string(),
    };
    print_status(
        "Credential Pool",
        &cp_status,
        config.credential_pool.enabled,
    );

    // Terminal backend
    let backend_str = match config.terminal_backend {
        TerminalBackend::Local => "Local",
        TerminalBackend::Docker => "Docker",
        TerminalBackend::Modal => "Modal",
        TerminalBackend::Ssh => "SSH",
        TerminalBackend::Daytona => "Daytona",
        TerminalBackend::VercelSandbox => "Vercel Sandbox",
        TerminalBackend::Singularity => "Singularity",
    };
    print_status("Terminal backend", backend_str, true);

    // Tool progress
    let tool_progress = match config.agent.tool_progress {
        ToolProgressMode::PerStep => "Per-step",
        ToolProgressMode::FinalOnly => "Final only",
        ToolProgressMode::Streaming => "Streaming",
        ToolProgressMode::Auto => "Auto",
    };
    print_status("Tool progress", tool_progress, true);

    // Session reset
    let session_reset = match config.agent.session_reset {
        SessionResetMode::Never => "Never",
        SessionResetMode::OnSystemPromptChange => "On system prompt change",
        SessionResetMode::OnToolChange => "On tool change",
        SessionResetMode::Always => "Always",
    };
    print_status("Session reset", session_reset, true);

    // Context compression
    print_status(
        "Context compression",
        if config.agent.context_compression {
            "enabled"
        } else {
            "disabled"
        },
        true,
    );

    println!();
    println!(
        "{}",
        style("──────────────────────────────────────────────")
            .bold()
            .cyan()
    );
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
    if !prompt_yes_no(
        "Reset all configuration to factory defaults? This cannot be undone.",
        false,
    )? {
        println!("Reset cancelled.");
        return Ok(());
    }

    let defaults = AppConfig::default();
    install_runtime_config(defaults.clone());
    persist_config(&defaults)?;

    print_success("Configuration reset to defaults.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive wizard
// ---------------------------------------------------------------------------

/// Run the interactive setup wizard.
///
/// When `section` is `None`, runs the FULL wizard spanning all sections.
/// When `section` is `Some(...)`, runs only the requested section.
pub async fn run_setup_wizard(
    config: &AppConfig,
    section: Option<&str>,
    quick: bool,
    _reconfigure: bool,
) -> Result<()> {
    let mut updated = config.clone();

    // ── Config backup ────────────────────────────────────────────────────────
    // Before any changes, back up existing config
    let paths = default_config_paths();
    let config_path = paths
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("operant.toml"));

    if config_path.exists() {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let backup = config_path.with_extension(format!("toml.bak.{}", timestamp));
        fs::copy(&config_path, &backup).ok();
        print_info(&format!(
            "Previous config backed up to: {}",
            backup.display()
        ));
        print_info(&format!(
            "Restore with: cp {} {}",
            backup.display(),
            config_path.display()
        ));
    }

    match section {
        Some("provider") => {
            // Section-specific calls are always full (not quick).
            step_provider_and_model(&mut updated, true).await?;
        }
        Some("terminal") => {
            step_terminal(&mut updated, true).await?;
            step_tools(&mut updated, true).await?;
        }
        Some("tts") => {
            step_tts(&mut updated, true).await?;
        }
        Some("browser") => {
            step_browser_and_skills(&mut updated).await?;
        }
        Some("gateway") => {
            step_gateway(&mut updated, true).await?;
        }
        Some("agent") => {
            step_agent_settings(&mut updated, true).await?;
        }
        None => {
            // Full wizard
            println!();
            println!(
                "{}",
                style("╔══════════════════════════════════════════════╗")
                    .bold()
                    .cyan()
            );
            println!(
                "{}",
                style("║         Operant Setup Wizard                  ║")
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

            // Step 1: Provider, model, API key, and auxiliary models
            // Pass `!quick` as the reconfigure flag so step_provider_and_model
            // knows whether to include Parts D (fallback) and E (auxiliary).
            step_provider_and_model(&mut updated, !quick).await?;

            if !quick {
                // Step 2: Gateway / Messaging platforms
                step_gateway(&mut updated, false).await?;

                // Step 3: Terminal preferences (stream, theme, reasoning, backend)
                step_terminal(&mut updated, false).await?;

                // Step 4: Tool & behaviour settings
                step_tools(&mut updated, false).await?;

                // Step 5: Text-to-Speech
                step_tts(&mut updated, false).await?;

                // Step 5b: Browser & Skills setup
                step_browser_and_skills(&mut updated).await?;

                // Step 6: Agent behaviour settings
                step_agent_settings(&mut updated, false).await?;
            }
        }
        Some(other) => {
            anyhow::bail!("Unknown setup section: {other}");
        }
    }

    // Apply changes
    install_runtime_config(updated.clone());
    persist_config(&updated)?;

    // Also sync to settings.json so the TUI picks up the new provider+model
    // immediately without needing a TUI-side /connect. Without this, the
    // TUI's settings.json (which overrides the TOML config) would still
    // have the old values. (iter-112 — fixes the "TUI shows hardcoded
    // defaults after setup" bug.)

    if section.is_none() {
        print_success("Setup complete! Configuration saved.");

        if quick {
            print_info(&format!(
                "  {} Run {} for full configuration.",
                style("ℹ").cyan(),
                style("operant setup").bold()
            ));
        }

        // Post-setup summary
        crate::post_setup::show_post_setup(&mut updated).await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Wizard steps
// ---------------------------------------------------------------------------

/// Step 1: Choose provider, model, and configure API key.
///
/// Flow: provider → model → API key (K/R/C) → fallback keys → strategy → auxiliary models.
pub(crate) async fn step_provider_and_model(config: &mut AppConfig, full: bool) -> Result<()> {
    print_page_header("Provider & Model Configuration");
    print_info("Select your LLM provider, model, and authentication.");
    println!();

    // ── Show current status ────────────────────────────────────────────────
    let current_provider = provider_from_url(&config.client.base_url)
        .map(|p| p.display_name)
        .unwrap_or("Custom");
    println!("  Current model:    {}", style(&config.agent.model).bold());
    println!(
        "  Active provider:  {}",
        style(current_provider).bold().green()
    );
    println!();

    // ── Part A — Select provider ──────────────────────────────────────────
    let current_provider_key = provider_key_for_url(&config.client.base_url);
    let current_provider_name = provider_name_for_url(&config.client.base_url);
    let current_model = &config.agent.model;

    let keep_idx: usize = 0;
    let mut provider_display: Vec<String> = vec![format!(
        "〇 Keep current: {} — {}",
        current_provider_name, current_model
    )];

    let first_provider_idx = provider_display.len();
    for p in PROVIDERS.iter() {
        if p.name == current_provider_key {
            provider_display.push(format!(
                "● {} — {} ← currently active",
                p.display_name, p.description
            ));
        } else {
            provider_display.push(format!("○ {} — {}", p.display_name, p.description));
        }
    }

    let custom_idx = provider_display.len();
    provider_display.push("○ Custom endpoint (enter URL manually)".to_string());
    let aux_idx = provider_display.len();
    provider_display.push("○ Configure auxiliary models".to_string());

    let sel = prompt_fuzzy_select("Select your LLM provider", &provider_display, keep_idx)?;

    // "Keep current" — skip everything
    if sel == keep_idx {
        print_info("Provider and model unchanged.");
        println!();
        return Ok(());
    }

    // "Configure auxiliary models" — skip provider change, just configure auxiliary
    if sel == aux_idx {
        step_auxiliary_models(config)?;
        return Ok(());
    }

    // "Custom endpoint" — prompt for URL and model manually
    if sel == custom_idx {
        let url = prompt_text(
            "Enter base URL (e.g. https://your-endpoint.com/v1)",
            &config.client.base_url,
        )?;
        config.client.base_url = url;

        let model = prompt_text("Enter model name", &config.agent.model)?;
        config.agent.model = model;

        print_success(&format!("Model set to {}", config.agent.model));
        println!();

        // API key for custom endpoint
        step_api_key(config, "custom", "Custom provider")?;

        // Same-provider fallback & strategy
        step_fallback_and_strategy(config, "custom", "Custom provider")?;

        // Auxiliary models
        step_auxiliary_models(config)?;

        return Ok(());
    }

    // Normal provider selection
    let provider_sel = sel - first_provider_idx;
    let selected_provider = &PROVIDERS[provider_sel];
    let provider_name = selected_provider.display_name;
    let provider_key = selected_provider.name;

    print_success(&format!("Provider set to {}", provider_name));
    println!();

    // Set base URL from the selected provider
    if !selected_provider.default_base_url.is_empty() {
        config.client.base_url = selected_provider.default_base_url.to_string();
    } else {
        let custom_url = prompt_text(
            &format!(
                "Enter {} base URL (e.g. https://your-endpoint.com/v1)",
                provider_name
            ),
            &config.client.base_url,
        )?;
        config.client.base_url = custom_url;
    }

    // ── Part B — API key FIRST (so live model fetch works) ────────────────
    // Reordered: was provider → model → API key. Now: provider → API key → model.
    // This ensures the API key is available when the user chooses "Fetch live
    // models", so the fetch actually works instead of returning an empty list.
    // (iter-119 — user-reported bug: live model fetch always failed.)
    step_api_key(config, provider_key, provider_name)?;

    // ── Part C — Select model (now with API key available for live fetch) ─
    let models_for_provider: Vec<&str> = if selected_provider.models.is_empty() {
        Vec::new()
    } else {
        selected_provider.models.to_vec()
    };

    if models_for_provider.is_empty() {
        let custom = prompt_text("Enter model name", &config.agent.model)?;
        config.agent.model = custom;
    } else {
        let fetch_label = format!("🔄  Fetch live models from {}", provider_name);
        let mut model_display: Vec<String> = vec![fetch_label];
        model_display.extend(models_for_provider.iter().map(|m| m.to_string()));
        model_display.push("✏️  Custom (enter model name manually)".to_string());

        let default_idx = model_display[1..model_display.len() - 1]
            .iter()
            .position(|m| m == &config.agent.model)
            .map(|i| i + 1)
            .unwrap_or(1);

        let model_sel = prompt_fuzzy_select(
            "Select a model",
            &model_display,
            default_idx.min(model_display.len().saturating_sub(1)),
        )?;

        if model_sel == 0 {
            // User chose "Fetch live models"
            let api_key = config.client.api_key.as_deref().unwrap_or("");
            let fetched: Vec<String> = if !api_key.is_empty() {
                fetch_models_for_provider(selected_provider, api_key).await
            } else {
                Vec::new()
            };

            if fetched.is_empty() {
                print_warning("Could not fetch models — using static list.");
                let mut fallback: Vec<&str> = models_for_provider.to_vec();
                fallback.push("Custom (enter model name manually)");
                let sub = prompt_fuzzy_select(
                    "Select a model",
                    &fallback.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    0,
                )?;
                config.agent.model = if sub == fallback.len() - 1 {
                    prompt_text("Enter model name", &config.agent.model)?
                } else {
                    fallback[sub].to_string()
                };
            } else {
                print_success(&format!(
                    "Loaded {} live models from {}",
                    fetched.len(),
                    provider_name
                ));
                let mut live = fetched.clone();
                live.push("Custom (enter model name manually)".to_string());
                let sub = prompt_fuzzy_select("Select a model", &live, 0)?;
                config.agent.model = if sub == live.len() - 1 {
                    prompt_text("Enter model name", &config.agent.model)?
                } else {
                    live[sub].clone()
                };
            }
        } else if model_sel == model_display.len() - 1 {
            // Custom
            let custom = prompt_text("Enter model name", &config.agent.model)?;
            config.agent.model = custom;
        } else {
            config.agent.model = model_display[model_sel].clone();
        }
    }

    print_success(&format!(
        "Model set to {}",
        style(&config.agent.model).bold()
    ));
    println!();

    // API key was already entered in Part B (before model selection) so that
    // live model fetch works. Don't ask again here.
    // Parts D (fallback & strategy) and E (auxiliary models) are power-user
    // features. Skip them in quick mode.
    if full {
        // ── Part D — Same-provider fallback & rotation ────────────────────────
        step_fallback_and_strategy(config, provider_key, provider_name)?;

        // ── Part E — Auxiliary Models ───────────────────────────────────────────
        step_auxiliary_models(config)?;
    }

    println!();
    Ok(())
}

/// Handle API key entry with [K]eep/[R]eplace/[C]lear for an existing key.
fn step_api_key(config: &mut AppConfig, provider_key: &str, provider_name: &str) -> Result<()> {
    let provider_info = provider_by_name(provider_key);

    // Skip if provider doesn't need an API key (e.g., ollama local)
    let needs_key = provider_info.is_none_or(|p| !p.env_var.is_empty());
    if !needs_key {
        print_info(&format!("{} does not require an API key.", provider_name));
        return Ok(());
    }

    if let Some(key) = &config.client.api_key {
        match prompt_key_action(provider_name, key)? {
            KeyAction::Keep => {}
            KeyAction::Replace => {
                let new_key = prompt_password(&format!("Enter {} API key", provider_name))?;
                if !new_key.is_empty() {
                    config.client.api_key = Some(new_key.clone());
                    if let Some(p) = provider_info {
                        save_env_value(p.env_var, &new_key).ok();
                    }
                }
            }
            KeyAction::Clear => {
                config.client.api_key = None;
                if let Some(p) = provider_info {
                    remove_env_value(p.env_var).ok();
                }
                print_info("API key cleared.");
            }
        }
    } else {
        let key = prompt_password(&format!("Enter {} API key", provider_name))?;
        if !key.is_empty() {
            config.client.api_key = Some(key.clone());
            if let Some(p) = provider_info {
                save_env_value(p.env_var, &key).ok();
            }
            print_success("API key set.");
        } else {
            print_warning("No API key set. You can set it later via 'operant auth'.");
        }
    }

    println!();
    Ok(())
}

/// Handle same-provider fallback keys and rotation strategy.
fn step_fallback_and_strategy(
    config: &mut AppConfig,
    provider_key: &str,
    _provider_name: &str,
) -> Result<()> {
    let provider_info = provider_by_name(provider_key);
    let needs_key = provider_info.is_none_or(|p| !p.env_var.is_empty());
    if !needs_key {
        return Ok(());
    }

    // Same-provider fallback
    print_header("Same-Provider Fallback & Rotation");
    print_info("Keep multiple credentials for one provider and rotate when exhausted.");

    let has_fallback = !config.client.additional_api_keys.is_empty();
    if prompt_yes_no(
        "Add another credential for same-provider fallback?",
        has_fallback,
    )? {
        loop {
            let extra_key = prompt_password("Enter additional API key")?;
            if extra_key.is_empty() {
                break;
            }
            // Save additional API key to .env before moving into Vec
            let env_key = format!(
                "{}_ADDITIONAL_{}",
                provider_key.to_uppercase().replace('-', "_"),
                config.client.additional_api_keys.len() + 1
            );
            let _ = save_env_value(&env_key, &extra_key);
            config.client.additional_api_keys.push(extra_key);
            if !prompt_yes_no("Add another?", false)? {
                break;
            }
        }
    }

    // Strategy selection if we have multiple keys
    let total_keys = 1 + config.client.additional_api_keys.len();
    let has_primary = config
        .client
        .api_key
        .as_deref()
        .is_some_and(|k| !k.is_empty());
    if (total_keys > 1 || has_primary) && prompt_yes_no("Configure rotation strategy?", true)? {
        let strategies = ["Fill-first / sticky", "Round robin", "Random"];
        let strategy_map = ["fill_first", "round_robin", "random"];
        let current = config
            .credential_pool
            .strategies
            .get(provider_key)
            .and_then(|s| strategy_map.iter().position(|m| *m == s))
            .unwrap_or(0);

        let sel = prompt_select("Select rotation strategy", &strategies, current)?;
        config
            .credential_pool
            .strategies
            .insert(provider_key.to_string(), strategy_map[sel].to_string());
        print_success(&format!("Rotation strategy: {}", strategies[sel]));

        // Auto-enable credential pool when multiple keys are configured
        if total_keys > 1 {
            config.credential_pool.enabled = true;
        }
    }

    println!();
    Ok(())
}

/// Configure auxiliary models for specialized tasks.
fn step_auxiliary_models(config: &mut AppConfig) -> Result<()> {
    if !prompt_yes_no("Configure auxiliary models for specialized tasks?", false)? {
        return Ok(());
    }

    let slots: [(&str, &str, &mut Option<AuxiliaryModelConfig>); 9] = [
        (
            "vision",
            "Vision tasks (image analysis)",
            &mut config.auxiliary_models.vision,
        ),
        (
            "compression",
            "Context compression",
            &mut config.auxiliary_models.compression,
        ),
        (
            "web_extract",
            "Web content extraction",
            &mut config.auxiliary_models.web_extract,
        ),
        (
            "image_gen",
            "Image generation",
            &mut config.auxiliary_models.image_gen,
        ),
        (
            "embeddings",
            "Embeddings",
            &mut config.auxiliary_models.embeddings,
        ),
        ("search", "Web search", &mut config.auxiliary_models.search),
        (
            "memory",
            "Memory/recall",
            &mut config.auxiliary_models.memory,
        ),
        (
            "code_execution",
            "Code execution",
            &mut config.auxiliary_models.code_execution,
        ),
        (
            "reasoning",
            "Deep reasoning",
            &mut config.auxiliary_models.reasoning,
        ),
    ];

    for (name, desc, slot) in slots {
        let already_configured = slot.is_some();
        if prompt_yes_no(
            &format!("Configure {} ({})?", name, desc),
            already_configured,
        )? {
            let default_provider = slot
                .as_ref()
                .and_then(|c| c.provider.as_deref())
                .unwrap_or("");
            let default_model = slot.as_ref().and_then(|c| c.model.as_deref()).unwrap_or("");

            let provider = prompt_text(&format!("Provider for {}", name), default_provider)?;
            let model = prompt_text(&format!("Model for {}", name), default_model)?;

            // Preserve existing base_url and api_key when reconfiguring
            let existing = slot.take().unwrap_or_default();
            *slot = Some(AuxiliaryModelConfig {
                provider: Some(provider),
                model: Some(model),
                base_url: existing.base_url,
                api_key: existing.api_key,
            });

            // Save auxiliary model API key to .env if present
            let env_key = format!(
                "AUXILIARY_{}_API_KEY",
                name.to_uppercase().replace('-', "_")
            );
            if let Some(api_key) = &slot.as_ref().unwrap().api_key {
                if !api_key.is_empty() {
                    let _ = save_env_value(&env_key, api_key);
                } else {
                    let _ = remove_env_value(&env_key);
                }
            } else {
                let _ = remove_env_value(&env_key);
            }
        }
    }

    Ok(())
}

/// Helper: check if a gateway platform is enabled in the current config.
fn is_gateway_enabled(settings: &GatewaySettings, platform_key: &str) -> bool {
    match platform_key {
        "telegram" => settings.telegram_enabled,
        "discord" => settings.discord_enabled,
        "slack" => settings.slack_enabled,
        "whatsapp" => settings.whatsapp_enabled,
        "email_smtp" => settings.email_enabled,
        "sms_twilio" => settings.sms_twilio_enabled,
        "webhooks" => settings.webhooks_enabled,
        _ => false,
    }
}

/// Step 2: Gateway / Messaging platforms.
///
/// Returns `true` if the gateway configuration changed, `false` otherwise.
async fn step_gateway(config: &mut AppConfig, _reconfigure: bool) -> Result<bool> {
    print_page_header("Gateway — Platform Integration");
    print_header("Messaging Platforms (Gateway)");
    print_info("Configure messaging platform integrations.");
    println!();

    let platforms = all_platforms();
    let items: Vec<String> = platforms
        .iter()
        .map(|p| format!("{}  — {}", p.name, p.description))
        .collect();

    // Pre-select platforms that are already enabled
    let defaults: Vec<usize> = platforms
        .iter()
        .enumerate()
        .filter(|(_, p)| is_gateway_enabled(&config.gateway, p.key))
        .map(|(i, _)| i)
        .collect();

    // Snapshot which platforms are enabled before changes
    let pre_enabled: Vec<String> = platforms
        .iter()
        .filter(|p| is_gateway_enabled(&config.gateway, p.key))
        .map(|p| p.key.to_string())
        .collect();

    let results = prompt_multi_select(
        "Select platforms to configure (Space to toggle, Enter to confirm)",
        &items,
        &defaults,
    )?;

    for &idx in &results {
        if idx < platforms.len() {
            (platforms[idx].setup_fn)(config)?;
            println!();
        }
    }

    // Snapshot enabled platforms after changes
    let post_enabled: Vec<String> = platforms
        .iter()
        .filter(|p| is_gateway_enabled(&config.gateway, p.key))
        .map(|p| p.key.to_string())
        .collect();

    let changed = pre_enabled != post_enabled;
    if changed {
        print_warning(
            "Gateway configuration changed. You may need to restart Operant for changes to take effect.",
        );
    }

    print_page_footer();
    Ok(changed)
}

/// Step 3: Terminal / TUI preferences.
async fn step_terminal(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    print_page_header("Terminal — Backend Configuration");
    print_header("Terminal Preferences");
    print_info("Customise how Operant behaves in the terminal.");

    // Stream toggle
    config.agent.stream = prompt_yes_no("Enable streaming responses?", config.agent.stream)?;

    // Theme
    // Keep this list in sync with the variants of `tui::adapter_types::Theme`
    // (Dark / Light / Default / Deuteranopia). Previously this listed
    // "opencode", "dracula", "monokai" which the TUI doesn't actually
    // distinguish — they all collapsed into `Theme::Custom(name)` and were
    // rendered identically to "default". (iter-125 — fixes the misleading
    // theme list flagged in the ponytail audit.)
    let themes = &["default", "dark", "light", "deuteranopia"];
    let current_theme_idx = themes
        .iter()
        .position(|t| *t == config.tui.theme)
        .unwrap_or(0);
    let theme_sel = prompt_select("Select terminal theme", themes, current_theme_idx)?;
    config.tui.theme = themes[theme_sel].to_string();

    // Show reasoning
    config.agent.show_reasoning = prompt_yes_no(
        "Show model reasoning in output?",
        config.agent.show_reasoning,
    )?;

    // Terminal backend
    let backends = [
        ("Local", "Run commands on your local machine"),
        ("Docker", "Run commands in a Docker container"),
        ("Modal", "Run on Modal serverless cloud"),
        ("SSH", "Run on a remote SSH server"),
        ("Daytona", "Run on Daytona"),
        ("Vercel Sandbox", "Run on Vercel"),
        ("Singularity", "Run on Singularity"),
    ];
    let backend_values: [TerminalBackend; 7] = [
        TerminalBackend::Local,
        TerminalBackend::Docker,
        TerminalBackend::Modal,
        TerminalBackend::Ssh,
        TerminalBackend::Daytona,
        TerminalBackend::VercelSandbox,
        TerminalBackend::Singularity,
    ];
    let current_backend_idx = backend_values
        .iter()
        .position(|b| *b == config.terminal_backend)
        .unwrap_or(0);
    let backend_sel = prompt_select_with_desc(
        "Select terminal execution backend",
        &backends,
        current_backend_idx,
    )?;
    if backend_sel < backend_values.len() {
        config.terminal_backend = backend_values[backend_sel].clone();
    }

    print_page_footer();
    Ok(())
}

/// Step 4: Tool & behaviour settings.
pub async fn step_tools(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    print_page_header("Tools — Agent Capabilities");
    print_header("Tool & Behaviour Settings");
    print_info("Configure tool usage and behaviour limits.");

    // Max iterations
    let iterations_str = prompt_text(
        "Maximum reasoning iterations",
        config.agent.max_iterations.to_string(),
    )?;
    if !iterations_str.is_empty() {
        config.agent.max_iterations = iterations_str.parse::<usize>().unwrap_or(90);
    }

    // Tool timeout
    let timeout_str = prompt_text(
        "Tool timeout (seconds)",
        config.agent.tool_timeout_secs.to_string(),
    )?;
    if !timeout_str.is_empty() {
        config.agent.tool_timeout_secs = timeout_str.parse::<u64>().unwrap_or(30);
    }

    // MCP autoload
    config.mcp.autoload = prompt_yes_no("Auto-load MCP servers?", config.mcp.autoload)?;

    // Rich output
    config.tui.rich_output = prompt_yes_no("Use rich TUI output?", config.tui.rich_output)?;

    print_page_footer();
    Ok(())
}

/// Step 5: Text-to-Speech (TTS) configuration.
async fn step_tts(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    print_page_header("Text-to-Speech (TTS) Configuration");
    print_info("Configure Text-to-Speech output.");
    println!();

    let enabled = prompt_yes_no("Enable Text-to-Speech (TTS)?", config.tts.enabled)?;

    if enabled {
        let tts_options = [
            ("Edge TTS", "Free, local via browser-edge-tts"),
            ("ElevenLabs", "High-quality cloud TTS, requires key"),
            ("OpenAI", "OpenAI TTS models (tts-1, tts-1-hd)"),
            ("xAI", "Grok TTS (if available through xAI)"),
            ("Mistral", "Mistral TTS (if available)"),
            ("Kokoro", "Free local TTS, no API key needed"),
            ("MiniMax", "MiniMax TTS voice synthesis"),
            ("Google Gemini", "Google Gemini TTS"),
            ("NeuTTS", "Local TTS via Neuphonic"),
            ("KittenTTS", "Local TTS, auto-installable"),
        ];
        let tts_values = [
            "edge",
            "elevenlabs",
            "openai",
            "xai",
            "mistral",
            "kokoro",
            "minimax",
            "google-gemini",
            "neutts",
            "kittentts",
        ];

        let current_idx = tts_values
            .iter()
            .position(|v| *v == config.tts.provider)
            .unwrap_or(0);

        let sel = prompt_select_with_desc("Select TTS provider", &tts_options, current_idx)?;
        if sel < tts_values.len() {
            config.tts.provider = tts_values[sel].to_string();
            config.tts.enabled = true;
            print_success(&format!(
                "TTS enabled with provider: {}",
                tts_options[sel].0
            ));

            // Download Kokoro model files if selected
            if tts_values[sel] == "kokoro" {
                let cache_dir = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".cache")
                    .join("kokoros");
                let model_path = cache_dir.join("kokoro-v1.0.onnx");
                let voices_path = cache_dir.join("voices-v1.0.bin");

                if !model_path.exists() || !voices_path.exists() {
                    print_info("Kokoro requires model files (~338 MB). Downloading now...");
                    std::fs::create_dir_all(&cache_dir).ok();

                    let client = reqwest::Client::new();
                    if !model_path.exists() {
                        print_info("  Downloading kokoro-v1.0.onnx (311 MB)...");
                        match download_file_to(&client, "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx", &model_path).await {
                            Ok(_) => print_success("  Model downloaded."),
                            Err(e) => print_warning(&format!("  Download failed: {}. TTS will download on first use.", e)),
                        }
                    }
                    if !voices_path.exists() {
                        print_info("  Downloading voices-v1.0.bin (27 MB)...");
                        match download_file_to(&client, "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin", &voices_path).await {
                            Ok(_) => print_success("  Voices downloaded."),
                            Err(e) => print_warning(&format!("  Download failed: {}. TTS will download on first use.", e)),
                        }
                    }
                } else {
                    print_success("Kokoro model files already present.");
                }
            }
        }
    } else {
        config.tts.enabled = false;
        print_success("TTS disabled");
    }

    println!();
    Ok(())
}

/// Download a file from URL to a local path.
async fn download_file_to(
    client: &reqwest::Client,
    url: &str,
    dest: &std::path::Path,
) -> Result<()> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    std::fs::write(dest, &bytes)?;
    Ok(())
}

/// Browser binary pre-install and skills directory setup.
async fn step_browser_and_skills(config: &mut AppConfig) -> Result<()> {
    print_page_header("Browser & Skills Setup");

    // ── Browser ──────────────────────────────────────────────────────────────
    let bin_path = operant_core::tools::browser_downloader::BrowserDownloader::default_bin_path();
    let browser_ok =
        operant_core::tools::browser_downloader::BrowserDownloader::verify_binary(&bin_path)
            .await
            .is_ok();

    if browser_ok {
        print_success(&format!(
            "Lightpanda browser already installed at {}",
            bin_path.display()
        ));
    } else if prompt_yes_no("Install Lightpanda browser for web automation?", true)? {
        print_info("Downloading Lightpanda browser binary...");
        match operant_core::tools::browser_downloader::BrowserDownloader::download_binary().await {
            Ok(path) => {
                config.tools.browser_binary_path = Some(path.clone());
                print_success(&format!("Browser installed at {}", path.display()));
            }
            Err(e) => {
                print_warning(&format!(
                    "Browser install failed: {}. Will retry on first use.",
                    e
                ));
            }
        }
    }

    // ── Skills directory ─────────────────────────────────────────────────────
    let skills_dir = &config.skills.root_dir;
    if skills_dir.exists() {
        let count = std::fs::read_dir(skills_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .count()
            })
            .unwrap_or(0);
        if count > 0 {
            print_success(&format!(
                "Skills directory: {} ({} skills)",
                skills_dir.display(),
                count
            ));
        } else {
            print_warning(&format!(
                "Skills directory exists but is empty: {}",
                skills_dir.display()
            ));
            print_info("Add skills with: operant skills install <name>");
        }
    } else {
        std::fs::create_dir_all(skills_dir).ok();
        print_info(&format!(
            "Created skills directory: {}",
            skills_dir.display()
        ));
        print_info("Add skills with: operant skills install <name>");
    }

    println!();
    Ok(())
}

/// Step 6: Agent behaviour settings.
async fn step_agent_settings(config: &mut AppConfig, _reconfigure: bool) -> Result<()> {
    print_page_header("Agent Behaviour Settings");
    print_info("Configure agent behaviour, progress reporting, and session management.");
    println!();
    print_info("Tool progress controls how you see each step of tool execution.");
    print_info("Choose the mode that matches your workflow:");
    println!("    Per-step     See each tool call as it happens (verbose)");
    println!("    Final only   See only the final result (quiet)");
    println!("    Streaming    Stream tool output in real-time");
    println!("    Auto         Automatically choose based on context");
    println!();

    // Tool progress mode
    let tool_modes = [
        ("Per-step", ToolProgressMode::PerStep),
        ("Final only", ToolProgressMode::FinalOnly),
        ("Streaming", ToolProgressMode::Streaming),
        ("Auto", ToolProgressMode::Auto),
    ];
    let tool_mode_names: Vec<&str> = tool_modes.iter().map(|(n, _)| *n).collect();
    let current_tool_mode = tool_modes
        .iter()
        .position(|(_, m)| *m == config.agent.tool_progress)
        .unwrap_or(3);
    let tool_sel = prompt_select(
        "Tool progress reporting mode",
        &tool_mode_names,
        current_tool_mode,
    )?;
    config.agent.tool_progress = tool_modes[tool_sel].1.clone();

    // Context compression
    print_header("Context Compression");
    print_info("As conversations grow, context windows fill up and API costs rise.");
    print_info("Context compression automatically summarises older messages");
    print_info("when the conversation exceeds a certain threshold.");
    print_info("This trades off some detail for lower costs and longer conversations.");
    config.agent.context_compression = prompt_yes_no(
        "Enable context compression?",
        config.agent.context_compression,
    )?;
    if config.agent.context_compression {
        let threshold = prompt_range(
            "Compression threshold",
            config.agent.context_compression_threshold,
            0.5,
            0.95,
        )?;
        config.agent.context_compression_threshold = threshold;
    }

    // Session reset
    print_header("Session Reset Policy");
    print_info("Messaging sessions accumulate context over time, growing API costs.");
    print_info("Session reset clears the conversation when certain conditions are met:");
    println!("    Never                  Keep context forever (highest cost)");
    println!("    On system prompt       Reset when system prompt changes");
    println!("    On tool change         Reset when tool configuration changes");
    println!("    Always                 Reset every new session (lowest cost)");
    println!();
    let reset_modes = [
        ("Never", SessionResetMode::Never),
        (
            "On system prompt change",
            SessionResetMode::OnSystemPromptChange,
        ),
        ("On tool change", SessionResetMode::OnToolChange),
        ("Always", SessionResetMode::Always),
    ];
    let reset_mode_names: Vec<&str> = reset_modes.iter().map(|(n, _)| *n).collect();
    let current_reset = reset_modes
        .iter()
        .position(|(_, m)| *m == config.agent.session_reset)
        .unwrap_or(0);
    let reset_sel = prompt_select("Session reset mode", &reset_mode_names, current_reset)?;
    config.agent.session_reset = reset_modes[reset_sel].1.clone();

    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Save the configuration to disk.
///
/// Writes to the first existing config path from `default_config_paths()`, or
/// creates `operant.toml` in the current directory if none exist yet.
fn persist_config(config: &AppConfig) -> Result<()> {
    // Always write to ~/.operant/operant.toml — the global config. The
    // setup wizard is for global settings, not project-specific overrides.
    // Previously this wrote to the FIRST existing path from
    // default_config_paths(), which could be ./operant.toml if the user
    // ran setup from a directory containing one. That caused the "setup
    // doesn't remember my config" bug — the wizard wrote to ./operant.toml
    // but subsequent runs from other directories loaded ~/.operant/operant.toml
    // which still had the old defaults.
    // (iter-112 — fixes the user's reported bug.)
    let config_path = dirs::home_dir()
        .map(|h| h.join(".operant").join("operant.toml"))
        .unwrap_or_else(|| PathBuf::from("operant.toml"));

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create config directory")?;
    }

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

/// Map a base URL to a human-readable provider name (for status display).
fn provider_name_for_url(base_url: &str) -> &'static str {
    provider_from_url(base_url)
        .map(|p| p.display_name)
        .unwrap_or("Custom")
}

/// Map a base URL to a provider key (for status display).
fn provider_key_for_url(base_url: &str) -> &'static str {
    provider_from_url(base_url).map(|p| p.name).unwrap_or("")
}
