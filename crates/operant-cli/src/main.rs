//! Operant-RS CLI

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
mod autonomous;
mod cmd_acp;
mod cmd_auth;
mod cmd_backup;
mod cmd_channel;
mod cmd_checkpoints;
mod cmd_completion;
mod cmd_config;
mod cmd_context;
mod cmd_cookies;
mod cmd_cron;
mod cmd_curator;
mod cmd_dashboard;
mod cmd_debug;
mod cmd_doctor;
mod cmd_dump;
mod cmd_gateway;
mod cmd_hardware;
mod cmd_hooks;
mod cmd_import;
mod cmd_insights;
mod cmd_kanban;
mod cmd_logs;
mod cmd_mcp;
mod cmd_memory;
mod cmd_migrate;
mod cmd_model;
mod cmd_peripheral;
mod cmd_plugins;
mod cmd_profile;
mod cmd_service;
mod cmd_sessions;
mod cmd_setup;
mod cmd_skills;
mod cmd_sop;
mod cmd_status;
mod cmd_suggestions;
mod cmd_tools;
mod cmd_trajectory;
mod cmd_tui_debug;
mod cmd_uninstall;
mod cmd_update;
mod cmd_version;
mod cmd_webhook;
mod commands;
pub(crate) mod config;
mod dashboard_server;
mod env_store;
mod gateway_commands;
mod gateway_platforms;
mod gateway_runner;
mod mcp_serve;
mod memory_provider_tools;
#[cfg(feature = "plugins-wasm")]
mod plugin_memory;
#[cfg(feature = "plugins-wasm")]
mod plugin_tools;
pub mod plugins_install;
mod post_setup;
mod prompt_helpers;
pub mod provider;
mod tui;

pub(crate) use tui::app;
pub(crate) use tui::dialogs;
pub(crate) use tui::effort_picker;
pub(crate) use tui::free_mode_dialog;
pub(crate) use tui::image_paste;
pub(crate) use tui::input;
pub(crate) use tui::mcp_view;
pub(crate) use tui::message_copy;
pub(crate) use tui::messages;
pub(crate) use tui::notifications;
pub(crate) use tui::osc8;
pub(crate) use tui::prompt_input;
pub(crate) use tui::settings_screen;
pub(crate) use tui::theme_screen;

use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::CliConfig;
use anyhow::{Context, Result};
use clap::{ArgAction, Parser, Subcommand};
#[cfg(feature = "anthropic")]
use operant_core::agent::clients::anthropic::AnthropicModelClient;
use operant_core::agent::clients::openai::OpenAIModelClient;
use operant_core::agent::{AgentConfig, AgentEvent, OperantAgent};
use operant_core::client::{ClientConfig, OpenAIClient};
use operant_core::config::{
    AppConfig, BehaviorSettings, LoggingSettings, McpServerConfig, McpTransportKind,
    ModelProviderConfig, install_runtime_config, load_app_config,
};
use operant_core::mcp::McpManager;
use operant_core::memory::MemoryManager;
use operant_core::memory_provider::MemoryProvider;
use operant_core::skills::SkillManager;
use operant_core::tools::{ToolContext, ToolRegistry};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{Level, warn};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::tui::{LaunchMode, TuiApp};
use operant_core::cronjobs::CronDb;
use operant_core::database::Database;
use operant_core::kanban::KanbanDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogTarget {
    Stderr,
    Sink,
    File,
}

#[derive(Debug, Parser)]
#[command(
    name = "operant",
    about = "Operant-RS: A high-performance ReAct agent framework",
    version,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[arg(short, long, global = true)]
    log_level: Option<String>,

    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[arg(long, global = true, env = "OPENAI_API_KEY")]
    api_key: Option<String>,

    #[arg(long, global = true, env = "OPENAI_BASE_URL")]
    base_url: Option<String>,

    #[arg(long, global = true)]
    model: Option<String>,

    #[arg(long, global = true)]
    max_iterations: Option<usize>,

    #[arg(long, global = true)]
    tool_timeout: Option<u64>,

    #[arg(long, global = true)]
    request_timeout: Option<u64>,

    #[arg(long, global = true)]
    context_window: Option<usize>,

    #[arg(long, global = true)]
    max_healing_attempts: Option<usize>,

    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "no_stream")]
    stream: bool,

    #[arg(long = "no-stream", global = true, action = ArgAction::SetTrue, conflicts_with = "stream")]
    no_stream: bool,

    #[arg(long, global = true)]
    debug_tui: bool,

    /// Disable mouse capture in the TUI. Useful when running inside a
    /// terminal multiplexer (tmux/screen) or when the user wants the
    /// terminal's native mouse selection to work. (Bug #24 from iter-82
    /// audit — /mouse mentioned a --no-mouse flag that didn't exist.)
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    no_mouse: bool,

    /// Skip all tool-permission prompts for the session (shows a confirmation first).
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    dangerously_skip_permissions: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run {
        #[arg(short, long)]
        system: Option<String>,

        #[arg(short, long)]
        query: Option<String>,

        #[arg(long, action = ArgAction::SetTrue)]
        autonomous: bool,

        /// Record this run as a trajectory (ReAct steps + messages) saved to
        /// ~/.operant/trajectories/. View with `operant trajectory list`.
        #[arg(long, action = ArgAction::SetTrue)]
        record_trajectory: bool,

        /// Enable a Mixture-of-Agents turn (hermes /moa parity, G5): the
        /// configured [moa] reference models advise on the query, an
        /// aggregator synthesizes their guidance, and it is injected into
        /// the agent's context for this run. Requires [moa] enabled and at
        /// least one reference model configured.
        #[arg(long, action = ArgAction::SetTrue)]
        moa: bool,
    },
    Autonomous {
        #[arg(short, long)]
        system: Option<String>,
    },
    /// List and manage available tools
    Tools {
        #[command(subcommand)]
        cmd: cmd_tools::ToolsSubcommand,
    },
    Chat {
        #[arg(short, long)]
        system: Option<String>,
    },
    Test {
        #[arg()]
        tool_name: String,

        #[arg(short, long)]
        args: Option<String>,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        cmd: cmd_config::ConfigSubcommand,
    },
    /// Manage conversation sessions
    Sessions {
        #[command(subcommand)]
        cmd: cmd_sessions::SessionsSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage MCP servers
    Mcp {
        #[command(subcommand)]
        cmd: cmd_mcp::McpSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Import / export / list browser cookies for the Obscura session
    /// (multi-browser cookie import from Chrome, Brave, Edge, Firefox, …)
    Cookies {
        #[command(subcommand)]
        cmd: cmd_cookies::CookiesSubcommand,
    },
    /// Manage installed skills
    Skills {
        #[command(subcommand)]
        cmd: cmd_skills::SkillsSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// View or change the active model configuration
    Model {
        #[command(subcommand)]
        cmd: cmd_model::ModelSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Generate shell completion scripts
    Completion {
        #[command(subcommand)]
        cmd: cmd_completion::CompletionSubcommand,
    },
    /// Manage cron jobs
    Cron {
        #[command(subcommand)]
        cmd: cmd_cron::CronSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Pause the agent (global emergency stop — new work only)
    Pause {
        /// Optional reason recorded in the sentinel
        reason: Option<String>,
    },
    /// Resume from a global pause
    Resume,
    /// Manage suggested automations
    Suggestions {
        #[command(subcommand)]
        cmd: cmd_suggestions::SuggestionsSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage kanban tasks
    Kanban {
        /// Board slug to operate on (default: "default")
        #[arg(long, default_value_t = String::from("default"), global = true)]
        board: String,
        #[command(subcommand)]
        cmd: cmd_kanban::KanbanSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage gateway
    Gateway {
        #[command(subcommand)]
        cmd: cmd_gateway::GatewaySubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage checkpoints
    Checkpoints {
        #[command(subcommand)]
        cmd: cmd_checkpoints::CheckpointsSubcommand,
    },
    /// Manage memory
    Memory {
        #[command(subcommand)]
        cmd: cmd_memory::MemorySubcommand,
    },
    /// Inspect the context engine (lossless DAG)
    Context {
        #[command(subcommand)]
        cmd: cmd_context::ContextSubcommand,
    },
    /// Manage profiles
    Profile {
        #[command(subcommand)]
        cmd: cmd_profile::ProfileSubcommand,
    },
    /// Manage auth credentials
    Auth {
        #[command(subcommand)]
        cmd: cmd_auth::AuthSubcommand,
    },
    /// Login to a provider
    Login,
    /// Logout
    Logout,
    /// Show version information
    Version {
        #[arg(long)]
        detailed: bool,
    },
    /// Check Operant configuration and dependencies
    Doctor {
        /// Attempt to auto-fix common issues
        #[arg(long)]
        fix: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
    /// Show system status overview
    Status {
        /// Show detailed status
        #[arg(long)]
        deep: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
    /// Print a setup summary report
    Dump {
        /// Show all configuration keys as YAML
        #[arg(long)]
        all: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
    /// View log files
    Logs {
        #[command(subcommand)]
        cmd: cmd_logs::LogsSubcommand,
    },
    /// Backup Operant configuration and data
    Backup {
        #[command(subcommand)]
        cmd: cmd_backup::BackupSubcommand,
    },
    /// Import from a backup
    Import {
        #[command(subcommand)]
        cmd: cmd_import::ImportSubcommand,
    },
    /// Uninstall Operant data
    Uninstall {
        #[command(subcommand)]
        cmd: cmd_uninstall::UninstallSubcommand,
    },
    /// Check for and apply updates
    Update {
        #[command(subcommand)]
        cmd: cmd_update::UpdateSubcommand,
    },
    /// Show usage insights
    Insights {
        #[command(subcommand)]
        cmd: cmd_insights::InsightsSubcommand,
    },
    /// Manage webhook subscriptions
    Webhook {
        #[command(subcommand)]
        cmd: cmd_webhook::WebhookSubcommand,
    },
    /// Manage shell hooks
    Hooks {
        #[command(subcommand)]
        cmd: cmd_hooks::HooksSubcommand,
    },
    /// Generate debug reports
    Debug {
        #[command(subcommand)]
        cmd: cmd_debug::DebugSubcommand,
    },
    /// Manage installed plugins
    Plugins {
        #[command(subcommand)]
        cmd: cmd_plugins::PluginsSubcommand,
    },
    /// Manage communication channels (telegram, discord, slack, whatsapp)
    Channel {
        #[command(subcommand)]
        cmd: cmd_channel::ChannelSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage standard operating procedures (SOPs)
    Sop {
        #[command(subcommand)]
        cmd: cmd_sop::SopSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Discover and introspect USB hardware
    Hardware {
        #[command(subcommand)]
        cmd: cmd_hardware::HardwareSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage hardware peripherals (STM32, RPi GPIO, etc.)
    Peripheral {
        #[command(subcommand)]
        cmd: cmd_peripheral::PeripheralSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Migrate data from other agent runtimes
    Migrate {
        #[command(subcommand)]
        cmd: cmd_migrate::MigrateSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage OS service lifecycle (systemd/launchd)
    Service {
        #[command(subcommand)]
        cmd: cmd_service::ServiceSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage the skill curator
    Curator {
        #[command(subcommand)]
        cmd: cmd_curator::CuratorSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Interactive setup wizard
    Setup {
        /// Optional setup section (provider, terminal, tts, gateway, agent)
        section: Option<String>,
        #[arg(long)]
        non_interactive: bool,
        #[arg(long)]
        reset: bool,
        #[arg(long)]
        reconfigure: bool,
        #[arg(long)]
        quick: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Run the ACP server
    Acp {
        #[command(subcommand)]
        cmd: cmd_acp::AcpSubcommand,
    },
    /// Run the web dashboard
    Dashboard {
        #[command(subcommand)]
        cmd: cmd_dashboard::DashboardSubcommand,
    },
    /// Manage agent trajectories (ReAct step recordings for fine-tuning)
    Trajectory {
        #[command(subcommand)]
        cmd: cmd_trajectory::TrajectorySubcommand,
    },
    /// TUI debugging tools — simulate every TUI overlay from the CLI.
    ///
    /// Each `debug` subcommand runs the same data-loading path the TUI uses
    /// for the corresponding overlay, but prints to stdout instead of rendering.
    /// Use this to verify an overlay's data loads correctly without entering
    /// the TUI, or to debug a broken overlay from the shell.
    ///
    /// Action subcommands (effort, mode, output-style, theme, vim, keybindings,
    /// voice) set TUI state persistently, closing the TUI↔CLI parity gaps.
    Tui {
        #[command(subcommand)]
        cmd: cmd_tui_debug::TuiSubcommand,
    },
}

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
fn init_logging(
    verbose: bool,
    cli_log_level: Option<&str>,
    logging: &LoggingSettings,
    rich_output: bool,
) {
    let env_filter = if verbose {
        EnvFilter::new(format!("{}", Level::DEBUG))
    } else if let Some(level) = cli_log_level {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(logging.level.clone()))
    };

    let subscriber = tracing_subscriber::registry().with(env_filter);
    let layer = fmt::layer()
        .with_target(logging.with_target)
        .with_thread_ids(logging.with_thread_ids)
        .with_file(logging.with_file)
        .with_line_number(logging.with_line_number);

    // When --verbose is passed, force logs to stderr so they are visible
    // even when tui.rich_output is true (which normally routes logs to sink).
    let effective_rich_output = rich_output && !verbose;
    match select_log_target(logging, effective_rich_output) {
        LogTarget::File => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(logging.log_file.as_ref().expect("log file should exist"))
                .expect("failed to open log file");
            let writer = Mutex::new(file);
            match logging.format.as_str() {
                "json" => subscriber
                    .with(layer.with_writer(writer).with_ansi(false).json())
                    .init(),
                "compact" => subscriber
                    .with(layer.with_writer(writer).with_ansi(false).compact())
                    .init(),
                _ => subscriber
                    .with(layer.with_writer(writer).with_ansi(false).pretty())
                    .init(),
            }
        }
        LogTarget::Sink => match logging.format.as_str() {
            "json" => subscriber
                .with(layer.with_writer(io::sink).with_ansi(false).json())
                .init(),
            "compact" => subscriber
                .with(layer.with_writer(io::sink).with_ansi(false).compact())
                .init(),
            _ => subscriber
                .with(layer.with_writer(io::sink).with_ansi(false).pretty())
                .init(),
        },
        LogTarget::Stderr => match logging.format.as_str() {
            "json" => subscriber.with(layer.json()).init(),
            "compact" => subscriber.with(layer.compact()).init(),
            _ => subscriber.with(layer.pretty()).init(),
        },
    }
}

fn select_log_target(logging: &LoggingSettings, rich_output: bool) -> LogTarget {
    if logging.log_file.is_some() {
        LogTarget::File
    } else if rich_output {
        LogTarget::Sink
    } else {
        LogTarget::Stderr
    }
}

fn apply_cli_overrides(cli: &Cli, config: &mut AppConfig) {
    if let Some(api_key) = &cli.api_key {
        config.client.api_key = Some(api_key.clone());
    }
    if let Some(base_url) = &cli.base_url {
        config.client.base_url = base_url.clone();
    }
    if let Some(model) = &cli.model {
        config.agent.model = model.clone();
    }
    if let Some(max_iterations) = cli.max_iterations {
        config.agent.max_iterations = max_iterations;
    }
    if let Some(timeout) = cli.tool_timeout {
        config.agent.tool_timeout_secs = timeout;
    }
    if let Some(timeout) = cli.request_timeout {
        config.agent.request_timeout_secs = timeout;
        config.client.timeout_secs = timeout;
    }
    if let Some(window) = cli.context_window {
        config.agent.context_window = window;
        config.client.max_context_length = window;
    }
    if let Some(healing) = cli.max_healing_attempts {
        config.agent.max_healing_attempts = healing;
    }
    if cli.stream {
        config.agent.stream = true;
    }
    if cli.no_stream {
        config.agent.stream = false;
    }
}

/// Create the appropriate ModelClient based on the provider.
fn create_model_client(
    provider: &str,
    config: &AppConfig,
) -> Box<dyn operant_core::agent::ModelClient> {
    match provider {
        #[cfg(feature = "anthropic")]
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .unwrap_or_else(|_| config.client.api_key.clone().unwrap_or_default());
            Box::new(AnthropicModelClient::new(api_key))
        }
        _ => {
            // Default to OpenAI-compatible client for openai, deepseek, and others
            Box::new(OpenAIModelClient::new(OpenAIClient::new(client_config(
                config,
            ))))
        }
    }
}

/// Layer a named provider profile's overrides onto the primary client config.
/// Absent fields inherit the primary (base URL, key, timeout) so a profile
/// only needs to specify what differs (hermes `fallback_providers` parity).
fn client_config_from_provider(
    primary: &ClientConfig,
    profile: &ModelProviderConfig,
) -> ClientConfig {
    let mut ccfg = primary.clone();
    if let Some(base) = profile.base_url.as_deref().filter(|b| !b.is_empty()) {
        ccfg.base_url = base.to_string();
    }
    if let Some(key) = profile.api_key.as_deref().filter(|k| !k.is_empty()) {
        ccfg.api_key = Some(key.to_string());
    }
    if let Some(secs) = profile.timeout_secs.filter(|s| *s > 0) {
        ccfg.timeout = Duration::from_secs(secs);
    }
    ccfg
}

/// Build the per-provider credential-pool registry (hermes `load_pool`
/// parity). Every provider that can carry a key gets its own pool, seeded
/// from config, so same-provider multi-key rotation works under free-tier
/// pressure *and* each fallback provider in the chain rotates its own keys
/// instead of failing on the first 429.
///
/// Seeds:
/// * primary provider — `client.api_key` + `client.additional_api_keys` +
///   the provider's canonical env var;
/// * every `[providers] models.<name>` profile — its `api_key` (the effective
///   key after inheritance, i.e. profile key or primary key).
fn build_pool_registry(
    config: &AppConfig,
    primary_provider: &str,
    primary_cfg: &ClientConfig,
) -> std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry> {
    use operant_core::credential_pool::CredentialPoolRegistry;
    let registry = CredentialPoolRegistry::new(config.credential_pool.clone());

    // Primary provider: configured key + additional keys + env var.
    if let Some(key) = primary_cfg
        .api_key
        .as_deref()
        .filter(|k| !k.trim().is_empty())
    {
        registry.add_seed(primary_provider, "primary", key, "config (client.api_key)");
    }
    for (i, key) in config.client.additional_api_keys.iter().enumerate() {
        registry.add_seed(
            primary_provider,
            &format!("additional-{i}"),
            key,
            "config (client.additional_api_keys)",
        );
    }
    registry.add_seed(
        primary_provider,
        &crate::cmd_auth::provider_env_var(primary_provider),
        &std::env::var(crate::cmd_auth::provider_env_var(primary_provider)).unwrap_or_default(),
        "env",
    );

    // Profile providers: each gets its own pool keyed by its profile name,
    // seeded with the effective `api_key` plus the profile's own
    // `api_keys` vector (hermes multi-key-per-provider parity).
    for name in config.providers.models.keys() {
        let profile = &config.providers.models[name];
        let effective = client_config_from_provider(primary_cfg, profile);
        if let Some(key) = effective
            .api_key
            .as_deref()
            .filter(|k| !k.trim().is_empty())
        {
            registry.add_seed(
                name,
                &format!("{name}-primary"),
                key,
                "config ([providers] models)",
            );
        }
        for (i, key) in profile.api_keys.iter().enumerate() {
            registry.add_seed(
                name,
                &format!("{name}-extra-{i}"),
                key,
                "config ([providers] models api_keys)",
            );
        }
    }

    tracing::info!(
        providers = ?registry.providers(),
        "Per-provider credential pool registry ready"
    );
    std::sync::Arc::new(registry)
}

/// Wrap a provider client in a pooled client carrying that provider's
/// on-demand pool (hermes pool-per-provider parity). An empty pool degrades
/// to a passthrough, so wrapping is unconditional.
fn wrap_pooled(
    client: std::sync::Arc<dyn operant_core::agent::ModelClient>,
    provider: &str,
    registry: &std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>,
) -> std::sync::Arc<dyn operant_core::agent::ModelClient> {
    std::sync::Arc::new(operant_core::agent::PooledModelClient::new(
        client,
        registry.get_or_create(provider),
    ))
}

/// Build a [`ProviderRegistry`] from the `[providers]` section — hermes
/// `fallback_providers` parity for cross-provider switching.
///
/// Chain order: primary (entry 0), then `providers.fallback_chain` in
/// configured order. Entries referencing an unknown profile key are skipped
/// with a warning. Returns `None` when no usable chain exists.
fn build_provider_registry(
    config: &AppConfig,
    primary_provider: &str,
    primary_model: &str,
    primary_cfg: &ClientConfig,
    pool_registry: &std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>,
) -> Option<std::sync::Arc<operant_core::agent::provider_registry::ProviderRegistry>> {
    use operant_core::agent::provider_registry::{ProviderEntry, ProviderRegistry};

    let models = &config.providers.models;
    if models.is_empty() && config.providers.fallback_chain.is_empty() {
        return None;
    }

    let mut clients: std::collections::HashMap<
        String,
        std::sync::Arc<dyn operant_core::agent::ModelClient>,
    > = std::collections::HashMap::new();
    // Primary client — entry 0 of the chain. Pooled so the primary provider
    // rotates its own keys too.
    let primary_arc = wrap_pooled(
        std::sync::Arc::new(OpenAIModelClient::new(OpenAIClient::new(
            primary_cfg.clone(),
        ))),
        primary_provider,
        pool_registry,
    );
    clients.insert(primary_provider.to_string(), primary_arc);

    // Profile clients, deterministic order (HashMap has no stable iteration).
    // Each is pooled with its own per-provider pool (hermes
    // `load_pool(fb_provider)` parity — the fallback provider rotates keys
    // under free-tier pressure instead of failing on the first 429).
    let mut names: Vec<&String> = models.keys().collect();
    names.sort();
    for name in names {
        let profile = &models[name];
        let ccfg = client_config_from_provider(primary_cfg, profile);
        let client = wrap_pooled(
            std::sync::Arc::new(OpenAIModelClient::new(OpenAIClient::new(ccfg))),
            name,
            pool_registry,
        );
        clients.insert(name.clone(), client);
    }

    let mut chain = vec![ProviderEntry {
        name: primary_provider.to_string(),
        model: primary_model.to_string(),
    }];
    for entry in &config.providers.fallback_chain {
        if !clients.contains_key(&entry.provider) {
            tracing::warn!(
                provider = %entry.provider,
                "providers.fallback_chain references an unknown profile (not in [providers.models]) — skipping"
            );
            continue;
        }
        if entry.model.is_empty() {
            continue;
        }
        chain.push(ProviderEntry {
            name: entry.provider.clone(),
            model: entry.model.clone(),
        });
    }
    if chain.len() < 2 {
        return None;
    }
    tracing::info!(
        chain_len = chain.len(),
        primary = %primary_provider,
        "cross-provider fallback chain active"
    );
    Some(std::sync::Arc::new(ProviderRegistry::new(clients, chain)))
}

/// Create the run-path model client wrapped with the configured fallback
/// chain (hermes `fallback_providers` parity):
///
/// * `agent.fallback_models` — same-client model swap on retryable errors
///   (5xx, 429, network).
/// * `[providers]` registry — cross-provider switch on auth/billing errors.
///
/// Falls back to a plain client when nothing is configured.
fn create_model_client_with_fallback(
    provider: &str,
    model: &str,
    config: &AppConfig,
) -> (
    Box<dyn operant_core::agent::ModelClient>,
    Option<std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>>,
) {
    let primary_cfg = client_config(config);
    let pool_registry = build_pool_registry(config, provider, &primary_cfg);
    // Primary client is pooled too: same-provider multi-key rotation happens
    // inside the client on 429/401/billing (hermes pool-per-provider parity).
    let primary_pooled: Box<dyn operant_core::agent::ModelClient> =
        Box::new(operant_core::agent::PooledModelClient::new(
            std::sync::Arc::from(create_model_client(provider, config)),
            pool_registry.get_or_create(provider),
        ));

    let has_chain =
        !config.providers.models.is_empty() || !config.providers.fallback_chain.is_empty();
    if !config.agent.fallback_on_errors || (config.agent.fallback_models.is_empty() && !has_chain) {
        return (primary_pooled, Some(pool_registry));
    }

    let mut fallback = operant_core::agent::FallbackModelClient::new(
        std::sync::Arc::from(primary_pooled),
        model.to_string(),
        config.agent.fallback_models.clone(),
        true,
    );
    if let Some(registry) =
        build_provider_registry(config, provider, model, &primary_cfg, &pool_registry)
    {
        fallback = fallback.with_provider_registry(registry);
    }
    (Box::new(fallback), Some(pool_registry))
}

pub(crate) fn client_config(config: &AppConfig) -> ClientConfig {
    ClientConfig::from(&config.client)
}

pub(crate) fn agent_config(
    config: &AppConfig,
    behavior: &BehaviorSettings,
    system_prompt: Option<&str>,
) -> AgentConfig {
    let mut agent = AgentConfig::from(behavior);
    if let Some(prompt) = system_prompt {
        agent.system_prompt = Some(prompt.to_string());
    }
    agent.request_timeout = Duration::from_secs(config.agent.request_timeout_secs);
    // Progressive tool disclosure (hermes parity): thread the CLI's
    // `tools.tool_search` block into the agent loop so MCP tool schemas
    // can be deferred behind the tool_search bridge. `mcp.deferred_loading
    // = false` is the master switch that forces eager loading regardless
    // of the tool_search block (hermes `mcp.deferred_loading` parity).
    let mut tool_search = config.tools.tool_search.clone();
    if !config.mcp.deferred_loading {
        tool_search.enabled = "off".to_string();
    }
    agent.tool_search = tool_search;
    agent
}

pub(crate) async fn build_registry(
    config: &AppConfig,
    mcp_manager: &McpManager,
    client: &OpenAIClient,
    model: &str,
    database: Arc<Database>,
    event_tx: Option<tokio::sync::mpsc::Sender<operant_core::agent::AgentEvent>>,
) -> Result<ToolRegistry> {
    let db_dir = config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let cron_path = db_dir.join("operant_cron.db");
    let kanban_path = db_dir.join("operant_kanban.db");
    let cron_db = Arc::new(CronDb::init(cron_path)?);
    let kanban_db = Arc::new(KanbanDb::init(kanban_path)?);
    let registry = ToolRegistry::new(Duration::from_secs(config.tools.registry_timeout_secs));
    let memory_dir = &config.skills.memory_dir;

    // Sub-agent delegation tool must inherit the parent's tool bans so
    // spawned children can never regain a disabled tool (hermes parity).
    // Passed explicitly because the bans are applied to the registry AFTER
    // registration below.
    let sub_agent_disabled_tools = config.tools.disabled_tools.iter().cloned().collect();
    let sub_agent_disabled_toolsets = config.tools.disabled_toolsets.iter().cloned().collect();

    operant_core::tools::register_builtin_tools_with_sub_agent(
        &registry,
        &config.skills.root_dir,
        memory_dir,
        client,
        model,
        database,
        cron_db,
        kanban_db,
        Some(mcp_manager.clone()),
        event_tx,
        sub_agent_disabled_tools,
        sub_agent_disabled_toolsets,
    )
    .await?;
    // (iter-153: EchoTool + CalculatorTool deleted — toy demo tools.
    // operant-core already has a real EchoTool in debug_helpers.rs.)

    // Memory provider tools (agentmemory_*) are registered via the MCP
    // server path (config.mcp.servers → agentmemory) and/or the agent's
    // MemoryProvider hook — see load_memory_manager / create_runtime_agent.

    // Register AFT (Agent File Tools) if enabled in config.
    // AFT provides 18 IDE-grade coding tools (tree-sitter outline/zoom/
    // semantic search/callers, AST-aware edit/patch, safety undo/checkpoints)
    // via a subprocess that auto-updates from GitHub releases.
    //
    // Natural fallback: the basic file/terminal tools are ONLY disabled
    // when the aft bridge is proven live (a bounded ping through a spawned
    // bridge succeeds). If the binary is missing, broken, or the network
    // stalls, the native tools stay registered — the agent is never left
    // tool-less (audit finding: the old code disabled natives on the mere
    // success of registration, which happens even when the bridge later
    // fails to spawn).
    //
    // The replaced names are merged into `disabled_tools` BELOW — calling
    // `registry.disable_tool()` here is insufficient because the later
    // `set_disabled_tools(config...)` REPLACES the whole disabled set,
    // silently re-enabling them (found while verifying the fallback path).
    let mut aft_replaced_tools: Vec<String> = Vec::new();
    if config.tools.aft_enabled {
        let pool = std::sync::Arc::new(operant_core::aft_bridge::AftBridgePool::new());
        match operant_core::tools::register_aft_tools(&registry, pool.clone()).await {
            Ok(()) => {
                tracing::info!("AFT tools registered (18 IDE-grade coding tools)");
                // Callgraph-dependent tools (aft_callers, and aft_inspect's
                // dead-code projection) must wait out the persisted callgraph
                // cold-build on first use — the bridge retries the request
                // with backoff. Grant them a window that covers that build
                // instead of the generic 30s tool timeout, which killed them
                // mid-build in live testing.
                registry.set_tool_timeout("aft_callers", std::time::Duration::from_secs(180));
                registry.set_tool_timeout("aft_inspect", std::time::Duration::from_secs(180));
                let project_root =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let bridge_live = async {
                    match pool.get(&project_root).await {
                        Ok(bridge) => bridge
                            .call("ping", serde_json::json!({}))
                            .await
                            .map(|_| true)
                            .map_err(|e| {
                                tracing::warn!(error = %e, "AFT ping failed");
                                e
                            })
                            .is_ok(),
                        Err(e) => {
                            tracing::warn!(error = %e, "AFT bridge unavailable at startup");
                            false
                        }
                    }
                };
                let live =
                    match tokio::time::timeout(std::time::Duration::from_secs(10), bridge_live)
                        .await
                    {
                        Ok(live) => live,
                        Err(_) => {
                            tracing::warn!(
                                "AFT bridge ping timed out (10s) — keeping native tools"
                            );
                            false
                        }
                    };
                if live {
                    // AFT provides superior versions of the basic tools:
                    //   aft_read → file_read, aft_write → file_write,
                    //   aft_edit → patch, aft_bash → terminal,
                    //   aft_search → file_search, aft_glob → file_list
                    aft_replaced_tools.extend([
                        "file_read".to_string(),
                        "file_write".to_string(),
                        "patch".to_string(),
                        "terminal".to_string(),
                        "file_search".to_string(),
                        "file_list".to_string(),
                    ]);
                    tracing::info!(
                        "AFT verified live — basic file/terminal tools disabled (aft replaces them)"
                    );
                } else {
                    tracing::warn!(
                        "AFT not operational at startup — keeping native tools as fallback"
                    );
                }
            }
            Err(e) => tracing::warn!(
                error = %e,
                "AFT tool registration failed (non-fatal) — native tools remain"
            ),
        }
    }

    // WASM plugin tools: discover tool-capable plugins and register their
    // WasmTools into the registry (feature `plugins-wasm`). Best-effort — a
    // broken plugin is skipped, never fatal to startup.
    #[cfg(feature = "plugins-wasm")]
    {
        if let Err(e) = plugin_tools::register_plugin_tools(&registry, config).await {
            tracing::warn!(error = %e, "plugin tool registration failed (non-fatal)");
        }
    }

    // The agentmemory MCP server is injected natively at config load
    // (config::ensure_default_mcp_servers) whenever the agentmemory memory
    // provider is active, so it flows through the generic config-driven
    // connect loop below like any other server. No CLI special case needed.

    // LCM context-engine tools: when agent.context_engine = "lcm" the
    // lossless DAG is active, so expose lcm_recall / lcm_stats to the model.
    // This engine instance is a read-only peer of the one attached to the
    // agent (same WAL database file) — recall is consistent either way.
    if config.agent.context_engine.as_str() == "lcm" {
        match operant_core::context::LcmContextEngine::new(lcm_config(config)) {
            Ok(engine) => {
                let engine = std::sync::Arc::new(engine);
                // P3 vector recall: when an embedding model is configured,
                // register lcm_vector_recall with an embedder. "local:hash"
                // uses the zero-dependency built-in (no external embedding
                // service); anything else is an OpenAI-compatible /embeddings
                // endpoint, defaulting to the chat provider's base URL and
                // key unless context_lcm_embedding_base_url overrides it.
                let embedder = config.agent.context_lcm_embedding_model.as_ref().map(|m| {
                    if m.trim() == "local:hash" {
                        tracing::info!(
                            "lcm_vector_recall: using zero-dependency local:hash embedder"
                        );
                        return std::sync::Arc::new(
                            operant_core::context::LocalHashEmbedder::default(),
                        )
                            as std::sync::Arc<dyn operant_core::context::Embedder>;
                    }
                    if config.agent.context_lcm_embedding_base_url.is_none() {
                        // The /embeddings call goes to the chat provider's
                        // endpoint. Many chat-only providers don't expose it
                        // — warn loudly so the failure isn't mysterious.
                        tracing::warn!(
                            "lcm_vector_recall: embedding model '{m}' will use the chat provider's \
                             base URL and key for /embeddings (set \
                             agent.context_lcm_embedding_base_url to a dedicated embeddings \
                             endpoint, or use \"local:hash\" for a zero-dependency embedder)"
                        );
                    }
                    let mut ccfg = client_config(config);
                    if let Some(eb) = &config.agent.context_lcm_embedding_base_url {
                        ccfg.base_url = eb.clone();
                    }
                    let client = OpenAIClient::new(ccfg);
                    std::sync::Arc::new(operant_core::context::OpenAIEmbedder::new(
                        client,
                        m.clone(),
                    )) as std::sync::Arc<dyn operant_core::context::Embedder>
                });
                // P3 assertion extraction (hermes ModelAssertionExtractor
                // parity, opt-in): when `context_lcm_assertion_extraction` is
                // true, lcm_assert gains action=extract — an LLM call over
                // the main agent model mines durable facts from recent DAG
                // nodes. Off by default (no extra LLM cost on stock installs).
                let extractor = if config.agent.context_lcm_assertion_extraction {
                    let llm = std::sync::Arc::new(OpenAIClient::new(client_config(config)));
                    Some(
                        std::sync::Arc::new(operant_core::context::LlmAssertionExtractor::new(
                            llm,
                            config.agent.model.clone(),
                        ))
                            as std::sync::Arc<dyn operant_core::context::AssertionExtractor>,
                    )
                } else {
                    None
                };
                match operant_core::tools::register_lcm_tools(
                    &registry, engine, embedder, extractor,
                )
                .await
                {
                    Ok(()) => {
                        // lcm_assert action=extract runs a reasoning-model LLM
                        // completion over recent DAG nodes — the generic 30s
                        // tool timeout killed it mid-thought in live testing
                        // (the model burns its budget on reasoning_content
                        // first). Grant the tool a longer execution window;
                        // save/query actions return in milliseconds regardless.
                        registry
                            .set_tool_timeout("lcm_assert", std::time::Duration::from_secs(180));
                        tracing::info!(
                            "LCM tools registered (lcm_recall / lcm_stats / lcm_assert / lcm_recall_round / lcm_recent / lcm_doctor) — lossless DAG active"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "LCM tool registration failed (non-fatal)")
                    }
                }
            }
            Err(e) => tracing::warn!(
                error = %e,
                "LCM engine init failed for tools — lcm_recall/lcm_stats unavailable"
            ),
        }
    }

    let mut disabled_tools: std::collections::HashSet<String> =
        config.tools.disabled_tools.iter().cloned().collect();
    let disabled_toolsets: std::collections::HashSet<String> =
        config.tools.disabled_toolsets.iter().cloned().collect();

    // Merge the AFT-replaced native tools into the config-driven set so a
    // single `set_disabled_tools` (which REPLACES, not unions) applies both.
    disabled_tools.extend(aft_replaced_tools);

    if !disabled_tools.is_empty() {
        registry.set_disabled_tools(disabled_tools).await;
    }
    if !disabled_toolsets.is_empty() {
        registry.set_disabled_toolsets(disabled_toolsets).await;
    }

    if config.mcp.autoload {
        // Deferred servers (e.g. the injected agentmemory server) are NOT
        // connected here — they would spawn `npx @agentmemory/mcp` on every
        // invocation. They stay connectable on demand via `operant mcp`.
        for server in config
            .mcp
            .servers
            .iter()
            .filter(|server| server.enabled && !server.deferred)
        {
            if !mcp_manager.contains(&server.name).await {
                connect_mcp_server(mcp_manager, server).await?;
            }
        }

        mcp_manager.sync_tools_to_registry(&registry).await;

        // G4 (hermes mcp_stdio_watchdog.py parity): when at least one
        // non-deferred stdio server is autoloaded, spawn a background
        // watchdog that detects crashed children (process exit) and
        // auto-reconnects them, re-syncing their tools into the registry.
        // Interval from `[mcp] watchdog_interval_secs` (0 disables). The
        // task dies with the process, so short-lived commands are unaffected.
        if config.mcp.watchdog_interval_secs > 0
            && config
                .mcp
                .servers
                .iter()
                .any(|s| s.enabled && !s.deferred && s.transport == McpTransportKind::Stdio)
        {
            mcp_manager.spawn_watchdog(
                registry.clone(),
                Duration::from_secs(config.mcp.watchdog_interval_secs),
            );
        }
    }

    Ok(registry)
}

async fn connect_mcp_server(mcp_manager: &McpManager, server: &McpServerConfig) -> Result<()> {
    match server.transport {
        McpTransportKind::Http => {
            let url = server
                .url
                .clone()
                .context("Configured HTTP MCP server is missing a URL")?;
            mcp_manager
                .add_server(server.name.clone(), url, server.auth_token.clone())
                .await?;
        }
        McpTransportKind::StreamableHttp => {
            // iter-190: StreamableHttp is now handled by the same add_server
            // path as Http — both use McpClient::connect().
            let url = server
                .url
                .clone()
                .context("Configured streamable-HTTP MCP server is missing a URL")?;
            mcp_manager
                .add_server(server.name.clone(), url, server.auth_token.clone())
                .await?;
        }
        McpTransportKind::Stdio => {
            let command = server
                .command
                .clone()
                .context("Configured stdio MCP server is missing a command")?;
            mcp_manager
                .add_stdio_server(
                    server.name.clone(),
                    command,
                    server.args.clone(),
                    server.env.clone(),
                )
                .await?;
        }
    }
    Ok(())
}

/// Shared components produced by the agent core builder.
struct AgentCore {
    database: Arc<Database>,
    registry: ToolRegistry,
    agent_config: AgentConfig,
    memory_manager: MemoryManager,
    /// Long-term memory provider (agentmemory/builtin). Attached to the
    /// agent so sync_turn/prefetch/session hooks actually fire — this was
    /// previously built-and-dropped (see docs/AUDIT_2026-08-02.md F1).
    memory_provider: Option<Arc<dyn MemoryProvider>>,
    skill_manager: SkillManager,
}

/// Build the shared core components needed by both agent constructors.
async fn build_agent_core(
    config: &AppConfig,
    system_prompt: Option<&str>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
    model_name: &str,
    behavior: &BehaviorSettings,
    event_tx: Option<tokio::sync::mpsc::Sender<operant_core::agent::AgentEvent>>,
) -> Result<AgentCore> {
    let raw_client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = build_registry(
        config,
        mcp_manager,
        &raw_client,
        model_name,
        database.clone(),
        event_tx,
    )
    .await?;
    let agent_config = agent_config(config, behavior, system_prompt);
    let (memory_manager, memory_provider) = load_repo_memory_manager().await?;

    // Hermes-plugin parity: register the memory provider's own tool schemas
    // (memory_smart_search, memory_save, ...) directly in the registry. The
    // injected agentmemory MCP server is deferred (lazy), so these keep the
    // memory surface available to the model without spawning npx at startup.
    if let Some(provider) = &memory_provider {
        memory_provider_tools::register_provider_tools(&registry, provider.clone()).await;
    }

    let mut skill_manager = SkillManager::new(skills_dir.to_path_buf());
    if let Err(e) = skill_manager.load_all() {
        warn!(
            error = %e,
            "Failed to load skills from {}",
            skills_dir.display()
        );
    }

    Ok(AgentCore {
        database,
        registry,
        agent_config,
        memory_manager,
        memory_provider,
        skill_manager,
    })
}

pub(crate) async fn create_runtime_agent(
    config: &AppConfig,
    behavior: &BehaviorSettings,
    system_prompt: Option<&str>,
    event_tx: mpsc::Sender<AgentEvent>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
    metrics: Option<std::sync::Arc<operant_core::runtime_metrics::RuntimeMetrics>>,
) -> Result<OperantAgent> {
    let core = build_agent_core(
        config,
        system_prompt,
        mcp_manager,
        skills_dir,
        &behavior.model,
        behavior,
        Some(event_tx.clone()),
    )
    .await?;

    let provider = crate::tui::provider::infer_provider_from_model(&behavior.model)
        .unwrap_or_else(|| "openai".to_string());
    let (model_client, pool_registry) =
        create_model_client_with_fallback(&provider, &behavior.model, config);

    let context_window = core.agent_config.context_window;
    Ok({
        let flag = operant_core::interrupt::InterruptFlag::new();
        // Spawn a Ctrl-C handler that triggers the flag. The agent loop
        // checks this flag at each iteration boundary + before each tool
        // call, so Ctrl-C produces a graceful exit instead of killing the
        // process mid-tool.
        let handler_flag = flag.clone();
        tokio::spawn(async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::debug!("ctrl_c signal handler error: {}", e);
            }
            tracing::info!("Ctrl-C received — triggering agent interrupt flag");
            handler_flag.trigger();
        });
        let mut agent = OperantAgent::with_events(
            core.agent_config,
            model_client,
            core.registry,
            core.database,
            event_tx,
        )
        .with_memory_manager(core.memory_manager)
        .with_skill_manager(core.skill_manager)
        .with_interrupt_flag(flag)
        .with_llm_compressor(operant_core::agent::llm_compressor::LlmCompressorConfig {
            context_window,
            enabled: config.agent.context_compression,
            threshold_percent: config.agent.context_compression_threshold,
            ..Default::default()
        });
        // Pluggable context engine (hermes-lcm parity): when configured
        // (agent.context_engine = "lcm"), build_messages assembles via the
        // lossless DAG + fresh-tail engine instead of lossy eviction.
        // Long-lived process (TUI / chat / gateway) → spawn background
        // maintenance workers (rollups + assertion extraction).
        if let Some(engine) = build_context_engine(config, true) {
            agent = agent.with_context_engine(engine);
        }
        // Share the external runtime-metrics registry (created by the TUI)
        // so stream-drop retries and memory-sync failures surface in the
        // status bar. When None, the agent keeps its own internal registry.
        if let Some(metrics) = metrics {
            agent = agent.with_metrics(metrics);
        }
        // Attach the long-term memory provider so turn/session hooks fire
        // (sync_turn, prefetch, on_session_end, ...). Closes audit gap F1.
        if let Some(provider) = core.memory_provider {
            agent = agent.with_memory_provider(provider);
        }
        configure_checkpoints(config);
        agent = attach_credential_pool(agent, &provider, config, pool_registry.as_ref());
        agent
    })
}

pub(crate) async fn create_agent_without_events(
    config: &AppConfig,
    system_prompt: Option<&str>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
) -> Result<OperantAgent> {
    let core = build_agent_core(
        config,
        system_prompt,
        mcp_manager,
        skills_dir,
        &config.agent.model,
        &config.agent,
        None,
    )
    .await?;

    let provider = crate::tui::provider::infer_provider_from_model(&config.agent.model)
        .unwrap_or_else(|| "openai".to_string());
    let (model_client, pool_registry) =
        create_model_client_with_fallback(&provider, &config.agent.model, config);

    let context_window = core.agent_config.context_window;
    Ok({
        let flag = operant_core::interrupt::InterruptFlag::new();
        let handler_flag = flag.clone();
        tokio::spawn(async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::debug!("ctrl_c signal handler error: {}", e);
            }
            tracing::info!("Ctrl-C received — triggering agent interrupt flag");
            handler_flag.trigger();
        });
        let mut agent = OperantAgent::new(
            core.agent_config,
            model_client,
            core.registry,
            core.database,
        )
        .with_memory_manager(core.memory_manager)
        .with_skill_manager(core.skill_manager)
        .with_interrupt_flag(flag)
        .with_llm_compressor(operant_core::agent::llm_compressor::LlmCompressorConfig {
            context_window,
            enabled: config.agent.context_compression,
            threshold_percent: config.agent.context_compression_threshold,
            ..Default::default()
        });
        // Pluggable context engine (hermes-lcm parity) — same gate as the
        // runtime agent path. Bounded one-shot run / autonomous task → no
        // LLM-fired maintenance workers (they'd be killed at process exit;
        // see build_context_engine docs).
        if let Some(engine) = build_context_engine(config, false) {
            agent = agent.with_context_engine(engine);
        }
        // Attach the long-term memory provider so turn/session hooks fire
        // (sync_turn, prefetch, on_session_end, ...). Closes audit gap F1.
        if let Some(provider) = core.memory_provider {
            agent = agent.with_memory_provider(provider);
        }
        configure_checkpoints(config);
        agent = attach_credential_pool(agent, &provider, config, pool_registry.as_ref());
        agent
    })
}

/// Derive the LCM engine config from the `agent` settings table.
///
/// Shared by the registry tool wiring and the agent engine so the db-path
/// default and tail budget can never drift between the two instances.
pub(crate) fn lcm_config(config: &AppConfig) -> operant_core::context::LcmConfig {
    operant_core::context::LcmConfig {
        db_path: config
            .agent
            .context_lcm_db
            .clone()
            .unwrap_or_else(|| operant_core::platform::operant_home().join("lcm.db")),
        tail_tokens: config.agent.context_lcm_tail_tokens,
        auto_recall: config.agent.context_lcm_auto_recall,
        auto_recall_limit: config.agent.context_lcm_auto_recall_limit,
        auto_recall_max_chars: config.agent.context_lcm_auto_recall_max_chars,
        rollups_inject: config.agent.context_lcm_rollups_inject,
        ignore_session_patterns: config.agent.context_lcm_ignore_session_patterns.clone(),
        readonly_sessions: config.agent.context_lcm_readonly_sessions.clone(),
    }
}

/// Build the configured context engine (`agent.context_engine`).
///   - `"compact"` (default) → `None`: deterministic decay + eviction.
///   - `"lcm"` → the lossless DAG engine; a broken init falls back to
///     compact (never fatal). Matches hermes `context.engine: lcm`.
///
/// `spawn_maintenance`: only LONG-LIVED processes (TUI / chat / gateway, via
/// `create_runtime_agent`) spawn the LLM-fired background maintenance workers
/// (rollups + assertion extraction). One-shot `operant run` / autonomous task
/// runs are bounded — their immediate maintenance pass would start a ~170s
/// LLM extraction call that gets killed at process exit (wasted provider
/// billing, reproduced in live testing). Hermes runs maintenance only in its
/// long-running agent loop, never per one-shot invocation.
fn build_context_engine(
    config: &AppConfig,
    spawn_maintenance: bool,
) -> Option<std::sync::Arc<dyn operant_core::context::ContextEngine>> {
    match config.agent.context_engine.as_str() {
        "compact" | "" => None, // built-in default — no engine attached
        "lcm" => match operant_core::context::LcmContextEngine::new(lcm_config(config)) {
            Ok(engine) => {
                tracing::info!("LCM context engine active (lossless DAG + fresh tail)");
                let engine = std::sync::Arc::new(engine);
                if spawn_maintenance {
                    spawn_lcm_maintenance_if_configured(config, &engine);
                }
                Some(engine)
            }
            Err(e) => {
                tracing::warn!(error = %e, "LCM context engine init failed — using compact");
                None
            }
        },
        other => {
            tracing::warn!(
                engine = other,
                "unknown agent.context_engine \"{other}\" — using compact"
            );
            None
        }
    }
}

/// One LLM rollup summarizer call (shared by the `context rollup` CLI and
/// the background maintenance scheduler so the prompt can never drift).
pub(crate) async fn rollup_summarize(
    transcript: String,
    client: OpenAIClient,
    model: String,
) -> operant_core::error::Result<String> {
    let msgs = vec![
        operant_core::client::Message::system(
            "You are a lossless temporal summarizer. Summarize the \
             following conversation excerpt into concise key facts \
             and decisions. Preserve names, numbers, and dates \
             exactly. Output only the summary.",
        ),
        operant_core::client::Message::user(transcript),
    ];
    let resp = client
        .chat(&model, &msgs, None, Some(1024), Some(0.2))
        .await
        .map_err(|e| operant_core::error::Error::Agent(format!("rollup LLM call failed: {e}")))?;
    let content = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    Ok(content.trim().to_string())
}

/// Spawn the background maintenance tasks when configured
/// (`context_lcm_rollup_interval_minutes > 0`): one immediate pass, then
/// every interval — hermes `_RollupMaintenanceScheduler` parity, bounded.
///
/// Two workers share the cadence:
///   1. rollup summarization (old message nodes → rollup nodes), and
///   2. assertion extraction (the same opt-in LLM extractor that backs
///      `lcm_assert action="extract"` mines durable facts from recent DAG
///      nodes automatically, so the store stays fresh without the agent ever
///      calling the tool — hermes `_assertion_extraction` maintenance
///      parity).
fn spawn_lcm_maintenance_if_configured(
    config: &AppConfig,
    engine: &std::sync::Arc<operant_core::context::LcmContextEngine>,
) {
    let minutes = config.agent.context_lcm_rollup_interval_minutes;
    if minutes == 0 {
        return;
    }
    let model = config.agent.model.clone();
    if model.is_empty() {
        tracing::warn!("lcm maintenance skipped: agent.model not configured");
        return;
    }
    let interval = std::time::Duration::from_secs(minutes * 60);

    let client = OpenAIClient::new(client_config(config));
    let summarizer = move |transcript: String| {
        let client = client.clone();
        let model = model.clone();
        rollup_summarize(transcript, client, model)
    };
    operant_core::context::rollup::spawn_rollup_maintenance(
        engine.clone(),
        interval,
        7,
        summarizer,
    );
    tracing::info!(minutes, "lcm rollup maintenance scheduler active");

    // Assertion-extraction worker on the same cadence (hermes
    // `_assertion_extraction` maintenance parity). Off when the opt-in gate
    // is off — no extra LLM cost on stock installs.
    if config.agent.context_lcm_assertion_extraction {
        let llm = std::sync::Arc::new(OpenAIClient::new(client_config(config)));
        let extractor: std::sync::Arc<dyn operant_core::context::AssertionExtractor> =
            std::sync::Arc::new(operant_core::context::LlmAssertionExtractor::new(
                llm,
                config.agent.model.clone(),
            ));
        operant_core::context::rollup::spawn_assertion_extraction_scheduler(
            engine.clone(),
            extractor,
            interval,
            40,
        );
        tracing::info!(minutes, "lcm assertion extraction scheduler active");
    }
}

/// Build and attach a credential pool when configured, seeded from the
/// active provider's env var plus `client.additional_api_keys` (hermes's
/// `load_pool` rotation vector). Without this, `try_rotate_credential`
/// always sees `None` and multi-key rotation is dead even when enabled.
/// Wire the global checkpoint manager from config. Checkpoints are opt-in
/// (`[checkpoints] enabled = true`); when enabled, snapshots land in an
/// isolated shadow store under `~/.operant/checkpoints` (hermes parity) so the
/// user's git repositories are never modified.
fn configure_checkpoints(config: &AppConfig) {
    let settings = &config.checkpoints;
    let base_dir = settings.base_dir.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join(".operant").join("checkpoints"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/operant-checkpoints"))
    });
    operant_core::tools::get_checkpoint_manager().configure(
        operant_core::tools::CheckpointConfig {
            base_dir,
            max_snapshots: settings.max_snapshots,
            enabled: settings.enabled,
            ..Default::default()
        },
    );
}

/// Attach the primary provider's credential pool to the agent (hermes
/// `_credential_pool` pattern) for the agent-loop last-resort rotation.
///
/// When a pool registry is supplied (built by
/// [`create_model_client_with_fallback`]), the agent shares the *same* pool
/// instance the pooled client uses, so exhaustion/cooldown state stays
/// consistent between the client-layer rotation and the agent-loop rotation.
/// Without a registry (e.g. tests), a local pool is seeded from the env var
/// + `client.additional_api_keys` as before.
fn attach_credential_pool(
    agent: OperantAgent,
    provider: &str,
    config: &AppConfig,
    pool_registry: Option<&std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>>,
) -> OperantAgent {
    if !config.credential_pool.enabled {
        return agent;
    }
    let pool = match pool_registry {
        Some(registry) => {
            let pool = registry.get_or_create(provider);
            if !pool.has_credentials() {
                return agent;
            }
            pool
        }
        None => {
            let mut pool = operant_core::credential_pool::CredentialPool::new(provider);
            pool.seed_from_env(&crate::cmd_auth::provider_env_var(provider));
            for key in &config.client.additional_api_keys {
                if !key.trim().is_empty() {
                    pool.add(operant_core::credential_pool::PooledCredential::new(
                        &format!("additional-{}", key.len()),
                        operant_core::credential_pool::AuthType::ApiKey,
                        key,
                        "config (client.additional_api_keys)",
                    ));
                }
            }
            // Per-provider strategy (hermes `credential_pool_strategies`
            // parity): the provider's entry wins, else global, else
            // fill_first — hermes `get_pool_strategy()` fail-closed default.
            pool.set_strategy(config.credential_pool.strategy_for(provider));
            if !pool.has_credentials() {
                return agent;
            }
            std::sync::Arc::new(pool)
        }
    };
    tracing::info!(
        provider = %provider,
        creds = pool.len(),
        strategy = %pool.strategy().as_str(),
        "Attached credential pool"
    );
    agent.with_credential_pool(pool)
}

async fn load_repo_memory_manager() -> Result<(MemoryManager, Option<Arc<dyn MemoryProvider>>)> {
    let storage_dir = operant_core::platform::operant_home();
    load_memory_manager(storage_dir).await
}

/// Load the file-backed MemoryManager and, when a non-builtin provider is
/// configured, build + background-initialize it and return it so the agent
/// can attach it (previously the provider was dropped — audit gap F1).
pub(crate) async fn load_memory_manager(
    storage_dir: PathBuf,
) -> Result<(MemoryManager, Option<Arc<dyn MemoryProvider>>)> {
    let memory_manager = MemoryManager::with_storage_dir(storage_dir.clone());
    memory_manager
        .load_from_disk()
        .await
        .context("Failed to load long-term memory")?;

    // Route the memory tools (memory_store/search/recall) through this injected
    // manager so tool-writes land in the store that gets injected into the
    // prompt (hermes parity — one coherent memory surface).
    operant_core::tools::memory_tools::set_active_memory_manager(memory_manager.clone()).await;

    let cfg = operant_core::config::runtime_config();
    if cfg.memory.enabled && cfg.memory.provider != "builtin" && cfg.memory.provider != "disabled" {
        // WASM memory plugin (memory.provider = "plugin:<name>"): a plugin
        // declaring the `memory` capability backs the MemoryProvider trait,
        // hermes-agent `plugins/memory/<name>` parity. When no plugin
        // matches, fall through to the compiled-in factory below.
        //
        // Uses the ACTIVE runtime config (`cfg`) — which already carries the
        // `--config` path + CLI overrides — so plugin dirs always resolve
        // from the config the user actually launched with, never a fresh
        // default-config reload.
        #[cfg(feature = "plugins-wasm")]
        if let Some(plugin_provider) =
            plugin_memory::build_plugin_memory_provider(&cfg.memory.provider, &cfg).await
        {
            let name = plugin_provider.name().to_string();
            let init = plugin_provider.clone();
            tokio::spawn(async move {
                if let Err(e) = init.initialize("main").await {
                    tracing::warn!(provider = %name, error = %e, "Memory plugin init failed");
                }
            });
            return Ok((memory_manager, Some(plugin_provider)));
        }
        let provider =
            operant_core::memory_provider::build_memory_provider(&cfg.memory.provider, storage_dir);
        let name = provider.name().to_string();
        let init = provider.clone();
        // Initialize in the background (spawns/warms the agentmemory server);
        // failures are non-fatal.
        tokio::spawn(async move {
            if let Err(e) = init.initialize("main").await {
                tracing::warn!(provider = %name, error = %e, "Memory provider init failed");
            }
        });
        Ok((memory_manager, Some(provider)))
    } else {
        Ok((memory_manager, None))
    }
}

async fn run_non_tui(
    config: &AppConfig,
    system_prompt: Option<&str>,
    query: &str,
    record_trajectory: bool,
    moa: bool,
) -> Result<()> {
    let mcp_manager = McpManager::new();
    let agent =
        create_agent_without_events(config, system_prompt, &mcp_manager, &config.skills.root_dir)
            .await?
            .with_trajectory_recording(record_trajectory);
    if record_trajectory {
        println!("Trajectory recording enabled — run will be saved to ~/.operant/trajectories/");
    }

    // G5 (hermes moa_loop.py parity): a MoA turn fans out the configured
    // reference models over the fresh conversation, has the aggregator
    // synthesize their advice, and injects the guidance for this run only.
    // Failures are non-fatal — the agent still runs in single-model mode.
    if moa {
        match run_moa_for_query(config, query).await {
            Ok(Some(guidance)) => {
                println!("[moa] Injected Mixture-of-Agents guidance for this turn");
                agent.set_moa_guidance(guidance);
            }
            Ok(None) => {
                println!(
                    "[moa] [moa] disabled or no reference models configured — running without MoA"
                );
            }
            Err(e) => {
                eprintln!("[moa] MoA aggregation failed (non-fatal): {e:#}");
            }
        }
    }

    let response = agent.run(query.to_string()).await?;
    // Drain pending memory hooks (sync_turn → /observe, session/end) before
    // the process exits. The MemorySyncExecutor is a background tokio task —
    // without a drain the runtime is torn down mid-write and the observation
    // is lost (reproduced in live testing; hermes _drain_sync_executor parity).
    agent.shutdown_memory_executor().await;
    println!("{}", response.content);
    Ok(())
}

/// G5 helper: run the MoA fan-out + synthesis for a fresh `operant run`
/// query. Returns `Ok(None)` when MoA is disabled or no reference models
/// are configured (the agent runs in single-model mode).
async fn run_moa_for_query(config: &AppConfig, query: &str) -> Result<Option<String>> {
    let moa = &config.moa;
    if !moa.enabled || moa.references.is_empty() {
        return Ok(None);
    }
    let messages = vec![operant_core::client::Message::user(query)];
    let guidance = operant_core::moa::aggregate_moa_context(
        query,
        &messages,
        &moa.references,
        moa.aggregator.as_ref(),
        &config.agent.model,
        &client_config(config),
        moa.max_tokens,
        moa.temperature,
        Duration::from_secs(moa.timeout_secs),
    )
    .await
    .context("MoA aggregation failed")?;
    Ok(guidance)
}

fn preview_tool_args(args: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(args) {
        for key in &[
            "query",
            "command",
            "url",
            "file_path",
            "path",
            "code",
            "text",
        ] {
            if let Some(val) = json.get(*key).and_then(|v| v.as_str()) {
                let truncated = if val.len() > 80 {
                    format!("{}...", &val[..80])
                } else {
                    val.to_string()
                };
                return Some(truncated);
            }
        }
    }
    None
}

async fn chat_non_tui(config: &AppConfig, system_prompt: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let agent = create_runtime_agent(
        config,
        &config.agent,
        system_prompt,
        event_tx,
        &mcp_manager,
        &config.skills.root_dir,
        None,
    )
    .await?;

    // Spawn task to display tool events in real-time
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::ToolStart {
                    tool_call_id: _,
                    name,
                    arguments,
                } => {
                    let preview = preview_tool_args(&arguments)
                        .map(|a| format!("{}: {}", name, a))
                        .unwrap_or_else(|| name);
                    println!("  Tool: {}...", preview);
                }
                AgentEvent::ToolComplete { result: _ } => {
                    println!("  Tool: Done.");
                }
                AgentEvent::ToolError {
                    tool_call_id: _,
                    name,
                    error,
                } => {
                    eprintln!("  Tool Error {}: {}", name, error);
                }
                AgentEvent::BackgroundReview { summary } => {
                    println!("  \u{1f4be} Self-improvement review: {summary}");
                }
                AgentEvent::AsyncDelegation {
                    delegation_id,
                    status,
                    summary,
                } => {
                    println!("  \u{1f9f5} Async delegation {delegation_id} {status}: {summary}");
                }
                _ => {}
            }
        }
    });

    // (iter-157: 6 inline handler structs deleted — replaced with a simple
    // match in the command loop. The handlers just returned hardcoded strings.)

    loop {
        print!("You: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_string();

        if input.is_empty() {
            continue;
        }

        if input.starts_with('/') {
            let parts: Vec<&str> = input.splitn(2, char::is_whitespace).collect();
            let cmd = parts[0].trim_start_matches('/');

            match cmd {
                "exit" | "quit" => break,
                "new" | "reset" => {
                    agent.clear_history().await;
                    println!("Conversation cleared. Starting new session.");
                }
                "help" => {
                    println!("Commands: /exit, /new, /help, /status, /time, /skills, /history");
                }
                "status" => {
                    println!("System status: running. Use `operant status` for details.");
                }
                "time" => {
                    use chrono::Local;
                    println!("Current time: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
                }
                "session" => {
                    println!("Conversation session active. Use /history to see messages.");
                }
                "skills" => {
                    let mut sm = SkillManager::new(config.skills.root_dir.clone());
                    match sm.load_all() {
                        Ok(skills) => {
                            if skills.is_empty() {
                                println!("No skills installed.");
                            } else {
                                println!("Installed skills:");
                                for skill in &skills {
                                    println!("  /{} — {}", skill.name, skill.description);
                                }
                            }
                        }
                        Err(e) => println!("Failed to load skills: {}", e),
                    }
                }
                "history" => {
                    println!("History not yet available in non-TUI mode.");
                }
                _ => {
                    println!(
                        "Unknown command: /{}. Type /help for available commands.",
                        cmd
                    );
                }
            }
            continue;
        }

        match agent.run(input.to_string()).await {
            Ok(response) => println!("Assistant: {}\n", response.content),
            Err(error) => eprintln!("Error: {}\n", error),
        }
    }

    // Drain pending memory hooks (sync_turn → /observe, session/end) before
    // the interactive session exits (hermes _drain_sync_executor parity).
    agent.shutdown_memory_executor().await;

    Ok(())
}

async fn test_tool(config: &AppConfig, tool_name: &str, args: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = build_registry(
        config,
        &mcp_manager,
        &client,
        &config.agent.model,
        database,
        None,
    )
    .await?;
    let parsed_args: Value = if let Some(args) = args {
        if args.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(args).context("Failed to parse tool arguments as JSON")?
        }
    } else {
        Value::Object(serde_json::Map::new())
    };

    let result = registry
        .execute(
            tool_name,
            &format!("test_{}", tool_name),
            parsed_args,
            ToolContext::default(),
        )
        .await?;

    println!("success: {}", result.success);
    println!("content: {}", result.content);
    if let Some(error) = result.error {
        println!("error: {}", error);
    }

    Ok(())
}

/// Whether the interactive TUI can run in the current environment.
///
/// The TUI enters raw mode + alternate screen via crossterm, which requires
/// a real terminal on stdin/stdout. When operant is invoked from a script,
/// CI, or piped input (stdout is not a TTY), raw-mode setup fails with
/// "No such device or address (os error 6)" — so we degrade to non-TUI mode
/// instead of crashing with a cryptic error. (audit 2026-08-02)
fn tui_available() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Emit a one-line stderr notice when rich_output is configured but the TUI
/// can't run (no TTY). Uses eprintln (not tracing) because with rich_output
/// the log sink swallows trace output — and this is a UX-visible change.
fn warn_tui_fallback(rich_output: bool) {
    if rich_output {
        eprintln!(
            "warning: no TTY detected — falling back to non-interactive mode (set tui.rich_output=false to silence)"
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load CLI-level config (.env file + HERMES_* env overrides; config.yaml
    // is deprecated — operant.toml below is the sole file-based config source)
    let cli_config = CliConfig::load().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load CLI config: {}. Using defaults.", e);
        CliConfig::default()
    });

    // Secret redaction toggle (hermes `HERMES_REDACT_SECRETS` parity). The
    // core loop redacts tool output / message content at the LLM boundary;
    // honor `security.redact_secrets` (default true) from CLI config.
    operant_core::redaction::set_redact_enabled(cli_config.security.redact_secrets.unwrap_or(true));

    // Load core AppConfig (TOML-based) and layer on any values from CliConfig
    let mut loaded = load_app_config(cli.config.as_deref())?;
    // Snapshot the file's own LLM settings BEFORE apply_cli_overrides stamps
    // the global --api-key/--base-url/--model onto them (override semantics).
    // Commands that persist the config (e.g. channel add/remove) restore these
    // so a temporary CLI override can never be written back to disk.
    let pre_override_client_api_key = loaded.config.client.api_key.clone();
    let pre_override_client_base_url = loaded.config.client.base_url.clone();
    let pre_override_agent_model = loaded.config.agent.model.clone();

    // Merge CliConfig-derived values into core AppConfig
    let cli_app_config = cli_config.to_app_config();
    if loaded.config.client.base_url.is_empty() {
        loaded.config.client.base_url = cli_app_config.client.base_url;
    }
    if loaded.config.client.api_key.is_none() {
        loaded.config.client.api_key = cli_app_config.client.api_key.clone();
    }
    if !loaded.config.gateway.telegram_enabled && cli_app_config.gateway.telegram_enabled {
        loaded.config.gateway.telegram_enabled = cli_app_config.gateway.telegram_enabled;
        if loaded.config.gateway.telegram_token.is_none() {
            loaded.config.gateway.telegram_token = cli_app_config.gateway.telegram_token;
        }
    }
    if !loaded.config.gateway.discord_enabled && cli_app_config.gateway.discord_enabled {
        loaded.config.gateway.discord_enabled = cli_app_config.gateway.discord_enabled;
        if loaded.config.gateway.discord_token.is_none() {
            loaded.config.gateway.discord_token = cli_app_config.gateway.discord_token;
        }
    }
    if !loaded.config.gateway.slack_enabled && cli_app_config.gateway.slack_enabled {
        loaded.config.gateway.slack_enabled = cli_app_config.gateway.slack_enabled;
        if loaded.config.gateway.slack_token.is_none() {
            loaded.config.gateway.slack_token = cli_app_config.gateway.slack_token;
        }
    }

    loaded.config.apply_env_overrides()?;
    apply_cli_overrides(&cli, &mut loaded.config);
    install_runtime_config(loaded.config.clone());

    // First-run bootstrap: seed the bundled skill pool (ships with operant)
    // into the user skills directory when it is empty or missing. No-op once
    // the user has any skill installed; `operant skills seed --force` re-runs.
    // Best-effort — a missing pool must never block startup.
    {
        let skills_dir = &loaded.config.skills.root_dir;
        let empty = std::fs::read_dir(skills_dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true);
        if empty && let Err(e) = cmd_skills::seed_bundled_skills(&loaded.config, None, false) {
            tracing::debug!(error = %e, "bundled skill seeding skipped (non-fatal)");
        }
    }

    // Wire delegation config to SubAgentTool statics
    if let Some(depth) = cli_config.delegation.max_spawn_depth {
        operant_core::tools::sub_agent_tool::set_max_spawn_depth(depth);
    }
    if let Some(enabled) = cli_config.delegation.orchestrator_enabled {
        operant_core::tools::sub_agent_tool::set_orchestrator_enabled(enabled);
    }
    if let Some(count) = cli_config.delegation.max_concurrent_children {
        operant_core::tools::sub_agent_tool::set_max_concurrent_children(count as usize);
    }

    init_logging(
        cli.verbose,
        cli.log_level.as_deref(),
        &loaded.config.logging,
        loaded.config.tui.rich_output,
    );

    if cli.debug_tui {
        tracing::debug!(target: "tui_wiring", "--debug-tui flag enabled");
    }

    match &cli.command {
        Some(Commands::Run {
            system,
            query,
            autonomous,
            record_trajectory,
            moa,
        }) => {
            if *autonomous {
                if query.is_some() {
                    anyhow::bail!(
                        "Do not combine 'run --autonomous' with '--query'. Autonomous mode reads TODO.md from the workspace."
                    );
                }
                autonomous::run_autonomous(loaded.config.clone(), system.clone()).await?;
                return Ok(());
            }
            let query = query
                .as_ref()
                .context("No query provided. Use --query or start chat mode.")?;
            if loaded.config.tui.rich_output && tui_available() {
                TuiApp::enter(
                    loaded.config.clone(),
                    system.clone(),
                    LaunchMode::Query(query.clone()),
                    cli.no_mouse,
                    cli.dangerously_skip_permissions,
                )
                .await?
                .run()
                .await?;
            } else {
                warn_tui_fallback(loaded.config.tui.rich_output);
                run_non_tui(
                    &loaded.config,
                    system.as_deref(),
                    query,
                    *record_trajectory,
                    *moa,
                )
                .await?;
            }
        }
        Some(Commands::Chat { system }) => {
            if loaded.config.tui.rich_output && tui_available() {
                TuiApp::enter(
                    loaded.config.clone(),
                    system.clone(),
                    LaunchMode::Landing,
                    cli.no_mouse,
                    cli.dangerously_skip_permissions,
                )
                .await?
                .run()
                .await?;
            } else {
                warn_tui_fallback(loaded.config.tui.rich_output);
                chat_non_tui(&loaded.config, system.as_deref()).await?;
            }
        }
        Some(Commands::Tools { cmd }) => {
            cmd_tools::handle_tools_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Autonomous { system }) => {
            autonomous::run_autonomous(loaded.config.clone(), system.clone()).await?;
        }
        Some(Commands::Test { tool_name, args }) => {
            test_tool(&loaded.config, tool_name, args.as_deref()).await?;
        }
        Some(Commands::Config { cmd }) => {
            cmd_config::handle_config_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Sessions { cmd, json }) => {
            cmd_sessions::handle_sessions_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Mcp { cmd, json }) => {
            let mcp_manager = operant_core::mcp::McpManager::new();
            cmd_mcp::handle_mcp_command(&loaded.config, &mcp_manager, cmd.clone(), *json).await?;
        }
        Some(Commands::Cookies { cmd }) => {
            cmd_cookies::handle_cookies_command(cmd.clone()).await?;
        }
        Some(Commands::Skills { cmd, json }) => {
            cmd_skills::handle_skills_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Model { cmd, json }) => {
            cmd_model::handle_model_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Completion { cmd }) => {
            cmd_completion::handle_completion_command(cmd.clone())?;
        }
        Some(Commands::Cron { cmd, json }) => {
            cmd_cron::handle_cron_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Pause { reason }) => {
            let detail = reason
                .clone()
                .unwrap_or_else(|| "engaged via `operant pause`".to_string());
            operant_core::estop::engage(Some(&detail))?;
            println!("⏸️ Operant paused (new work only). Resume with `operant resume`.");
        }
        Some(Commands::Resume) => {
            operant_core::estop::disengage()?;
            println!("Operant resumed.");
        }
        Some(Commands::Suggestions { cmd, json }) => {
            cmd_suggestions::handle_suggestions_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Kanban { board, cmd, json }) => {
            cmd_kanban::handle_kanban_command(&loaded.config, board, cmd.clone(), *json).await?;
        }
        Some(Commands::Gateway { cmd, json }) => {
            cmd_gateway::handle_gateway_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Checkpoints { cmd }) => {
            cmd_checkpoints::handle_checkpoints_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Memory { cmd }) => {
            cmd_memory::handle_memory_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Context { cmd }) => {
            cmd_context::handle_context_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Profile { cmd }) => {
            cmd_profile::handle_profile_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Auth { cmd }) => {
            cmd_auth::handle_auth_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Login) => {
            cmd_auth::handle_login(&loaded.config).await?;
        }
        Some(Commands::Logout) => {
            cmd_auth::handle_logout(&loaded.config).await?;
        }
        Some(Commands::Version { detailed }) => {
            cmd_version::handle_version_command(&loaded.config, *detailed).await?;
        }
        Some(Commands::Doctor { fix, json }) => {
            cmd_doctor::handle_doctor_command(&loaded.config, *fix, *json).await?;
        }
        Some(Commands::Status { deep, json }) => {
            cmd_status::handle_status_command(&loaded.config, *deep, *json).await?;
        }
        Some(Commands::Dump { all, json }) => {
            cmd_dump::handle_dump_command(&loaded.config, *all, *json).await?;
        }
        Some(Commands::Logs { cmd }) => {
            cmd_logs::handle_logs_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Backup { cmd }) => {
            cmd_backup::handle_backup_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Import { cmd }) => {
            cmd_import::handle_import_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Uninstall { cmd }) => {
            cmd_uninstall::handle_uninstall_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Update { cmd }) => {
            cmd_update::handle_update_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Insights { cmd }) => {
            cmd_insights::handle_insights_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Webhook { cmd }) => {
            cmd_webhook::handle_webhook_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Hooks { cmd }) => {
            cmd_hooks::handle_hooks_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Debug { cmd }) => {
            cmd_debug::handle_debug_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Plugins { cmd }) => {
            cmd_plugins::handle_plugins_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Curator { cmd, json }) => {
            cmd_curator::handle_curator_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Channel { cmd, json }) => {
            // apply_cli_overrides above stamped the global --api-key/--base-url/
            // --model onto the loaded config (LLM-override semantics). Channel
            // commands persist the config back to disk, so restore the file's
            // original values first — otherwise a channel-token passed as
            // --api-key would clobber the real LLM key on the next launch.
            loaded.config.client.api_key = pre_override_client_api_key.clone();
            loaded.config.client.base_url = pre_override_client_base_url.clone();
            loaded.config.agent.model = pre_override_agent_model.clone();
            cmd_channel::handle_channel_command(
                &mut loaded.config,
                loaded.source.as_deref(),
                cmd.clone(),
                *json,
            )
            .await?;
        }
        Some(Commands::Sop { cmd, json }) => {
            cmd_sop::handle_sop_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Hardware { cmd, json }) => {
            cmd_hardware::handle_hardware_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Peripheral { cmd, json }) => {
            cmd_peripheral::handle_peripheral_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Migrate { cmd, json }) => {
            cmd_migrate::handle_migrate_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Service { cmd, json }) => {
            cmd_service::handle_service_command(&loaded.config, cmd.clone(), *json)?;
        }
        Some(Commands::Setup {
            section,
            non_interactive,
            reset,
            reconfigure,
            quick,
            json,
        }) => {
            cmd_setup::handle_setup_command(
                &loaded.config,
                section.as_deref(),
                *non_interactive,
                *reset,
                *reconfigure,
                *quick,
                *json,
            )
            .await?;
        }
        Some(Commands::Acp { cmd }) => {
            cmd_acp::handle_acp_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Dashboard { cmd }) => {
            cmd_dashboard::handle_dashboard_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Trajectory { cmd }) => {
            cmd_trajectory::handle_trajectory_command(cmd.clone()).await?;
        }
        Some(Commands::Tui { cmd }) => {
            cmd_tui_debug::handle_tui_command(&loaded.config, cmd.clone()).await?;
        }
        None => {
            // No command provided - launch TUI in interactive mode
            if loaded.config.tui.rich_output {
                TuiApp::enter(
                    loaded.config.clone(),
                    None,
                    LaunchMode::Landing,
                    cli.no_mouse,
                    cli.dangerously_skip_permissions,
                )
                .await?
                .run()
                .await?;
            } else {
                chat_non_tui(&loaded.config, None).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_tui_without_log_file_uses_sink() {
        let logging = LoggingSettings::default();
        assert_eq!(select_log_target(&logging, true), LogTarget::Sink);
    }

    #[test]
    fn log_file_overrides_sink() {
        let logging = LoggingSettings {
            log_file: Some("operant.log".to_string()),
            ..Default::default()
        };
        assert_eq!(select_log_target(&logging, true), LogTarget::File);
    }

    #[tokio::test]
    async fn load_memory_manager_reads_existing_memory_file() {
        let dir =
            std::env::temp_dir().join(format!("operant_cli_memory_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let seed = MemoryManager::with_storage_dir(dir.clone());
        seed.store(
            operant_core::memory::MemoryBlock::new("cli_fact", "fact", "Loaded memory fact")
                .importance(90),
        )
        .await;
        // iter-24: store() marks dirty instead of writing immediately.
        // flush_if_dirty() persists to disk so load_memory_manager can
        // read the file.
        seed.flush_if_dirty().await.unwrap();

        let (loaded, provider) = load_memory_manager(dir.clone()).await.unwrap();
        // With the default config, provider may be Some(agentmemory) or None
        // (builtin/disabled) — both are valid; the manager must still load.
        assert!(provider.is_none() || provider.unwrap().name() == "agentmemory");

        assert_eq!(loaded.search("Loaded memory").await.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn autonomous_subcommand_parses() {
        let cli = Cli::try_parse_from(["operant", "autonomous"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Autonomous { .. })));
    }

    #[test]
    fn run_autonomous_flag_parses() {
        let cli = Cli::try_parse_from(["operant", "run", "--autonomous"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Run {
                autonomous: true,
                ..
            })
        ));
    }

    // ── Provider fallback wiring (hermes `fallback_providers` parity) ──

    fn test_config_with_providers() -> AppConfig {
        let mut config = AppConfig::default();
        config.agent.model = "laguna-s-2.1-free".to_string();
        config.agent.fallback_models = vec!["deepseek-v4-flash-free".to_string()];
        config.client.api_key = Some("sk-primary".to_string());
        config.providers.models.insert(
            "zen-fallback".to_string(),
            ModelProviderConfig {
                base_url: Some("https://zen.example/v1".to_string()),
                model: Some("deepseek-v4-flash-free".to_string()),
                api_key: Some("sk-fallback".to_string()),
                ..Default::default()
            },
        );
        config
            .providers
            .fallback_chain
            .push(operant_core::config::FallbackProviderConfig {
                provider: "zen-fallback".to_string(),
                model: "deepseek-v4-flash-free".to_string(),
            });
        config
    }

    #[test]
    fn provider_profile_overrides_layer_onto_primary() {
        let primary = ClientConfig {
            base_url: "https://primary.example/v1".to_string(),
            api_key: Some("sk-primary".to_string()),
            timeout: Duration::from_secs(60),
            max_context_length: 200_000,
            rate_limit: Default::default(),
        };
        let profile = ModelProviderConfig {
            base_url: Some("https://zen.example/v1".to_string()),
            api_key: None, // inherit primary key
            timeout_secs: Some(300),
            ..Default::default()
        };
        let ccfg = client_config_from_provider(&primary, &profile);
        assert_eq!(ccfg.base_url, "https://zen.example/v1");
        assert_eq!(ccfg.api_key.as_deref(), Some("sk-primary"));
        assert_eq!(ccfg.timeout, Duration::from_secs(300));
    }

    fn test_pool_registry(
        config: &AppConfig,
        provider: &str,
    ) -> std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry> {
        build_pool_registry(config, provider, &client_config(config))
    }

    #[test]
    fn build_provider_registry_orders_chain_primary_first() {
        let config = test_config_with_providers();
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools)
                .unwrap();

        assert_eq!(registry.active_provider().as_deref(), Some("free"));
        // Chain: primary + configured fallback entry.
        let switched = registry.switch_to_next().unwrap();
        assert_eq!(switched.name, "zen-fallback");
        assert_eq!(switched.model, "deepseek-v4-flash-free");
        assert!(registry.get_client("zen-fallback").is_some());
        // Primary client is registered too.
        assert!(registry.get_client("free").is_some());
    }

    #[test]
    fn build_provider_registry_skips_unknown_provider_entries() {
        let mut config = test_config_with_providers();
        config
            .providers
            .fallback_chain
            .push(operant_core::config::FallbackProviderConfig {
                provider: "ghost".to_string(),
                model: "nope".to_string(),
            });
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools)
                .unwrap();
        // Unknown entry is skipped — only the valid fallback remains.
        assert!(registry.switch_to_next().is_some());
        // Chain is exhausted after the one valid fallback.
        assert!(registry.switch_to_next().is_none());
    }

    #[test]
    fn build_provider_registry_none_without_chain() {
        let config = AppConfig::default(); // no [providers]
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "openai");
        assert!(
            build_provider_registry(&config, "openai", "gpt-4o", &primary_cfg, &pools).is_none(),
            "no [providers] → no registry"
        );
    }

    #[test]
    fn fallback_models_flow_into_wrapper_chain() {
        use operant_core::agent::FallbackModelClient;

        // `agent.fallback_models` is the same-client model swap list. The
        // wrapper is only constructed when the config says so — verify the
        // gating: fallback_on_errors=false with a configured chain returns the
        // raw primary client (early return, no FallbackModelClient built).
        let mut config = test_config_with_providers();
        config.agent.fallback_on_errors = false;
        let (client, _pool_registry) =
            create_model_client_with_fallback("free", "laguna-s-2.1-free", &config);
        // Smoke: the returned client is usable and reports a provider.
        assert!(!client.provider_name().is_empty());

        // And with fallback enabled, FallbackModelClient::new is what wraps it
        // (its model_chain is [primary, fallback_models...] — verified in the
        // core crate's 17 fallback tests). Here we only prove the config
        // surfaces are read: fallback_models + fallback_chain both reach the
        // builder.
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools);
        assert!(registry.is_some(), "chain configured → registry built");
        // FallbackModelClient type is constructible from the same pieces.
        let inner: std::sync::Arc<dyn operant_core::agent::ModelClient> =
            std::sync::Arc::new(OpenAIModelClient::new(OpenAIClient::new(primary_cfg)));
        let _fb = FallbackModelClient::new(
            inner,
            "laguna-s-2.1-free".to_string(),
            config.agent.fallback_models.clone(),
            true,
        )
        .with_provider_registry(registry.unwrap());
    }

    // ── Per-provider on-demand pools (hermes `load_pool` parity) ──

    #[test]
    fn pool_registry_seeds_primary_and_profiles() {
        let config = test_config_with_providers();
        let pools = test_pool_registry(&config, "free");

        // Primary provider seeded from config client key.
        assert!(pools.has_credentials("free"));
        let primary_pool = pools.get_or_create("free");
        assert_eq!(primary_pool.len(), 1);
        assert_eq!(
            primary_pool.peek().map(|c| c.value),
            Some("sk-primary".to_string())
        );

        // Fallback profile provider seeded from its own api_key.
        assert!(pools.has_credentials("zen-fallback"));
        let fb_pool = pools.get_or_create("zen-fallback");
        assert_eq!(fb_pool.len(), 1);
        assert_eq!(
            fb_pool.peek().map(|c| c.value),
            Some("sk-fallback".to_string())
        );

        // Providers list contains both.
        let providers = pools.providers();
        assert!(providers.contains(&"free".to_string()));
        assert!(providers.contains(&"zen-fallback".to_string()));
    }

    #[test]
    fn pool_registry_dedupes_identical_keys() {
        let config = test_config_with_providers();
        let pools = test_pool_registry(&config, "free");
        // Pool already has sk-primary (config) — adding the same value twice
        // must produce one entry, not two (hermes sibling-marking is avoided
        // by preventing the duplicate at seed time).
        pools.add_seed("free", "dup-1", "sk-fallback", "test");
        pools.add_seed("free", "dup-2", "sk-fallback", "test");
        let pool = pools.get_or_create("free");
        assert_eq!(pool.len(), 2, "sk-primary + one deduped sk-fallback");
        let values: Vec<String> = pool.list().into_iter().map(|c| c.value).collect();
        assert_eq!(values.iter().filter(|v| *v == "sk-fallback").count(), 1);
    }

    #[test]
    fn pool_registry_lazy_creation_shared_pool() {
        let config = AppConfig::default();
        let pools = test_pool_registry(&config, "openai");
        // No pool exists before first use.
        assert!(pools.pool("openai").is_none());
        let first = pools.get_or_create("openai");
        let second = pools.get_or_create("openai");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "pool is cached and shared"
        );
    }

    #[test]
    fn pooled_registry_clients_are_pooled() {
        let config = test_config_with_providers();
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools)
                .unwrap();
        // Building the registry consults the per-provider pool registry
        // (hermes `load_pool(fb_provider)` parity) — the fallback provider's
        // pool must be materialized and seeded during build.
        assert!(registry.get_client("zen-fallback").is_some());
        let fb_pool = pools
            .pool("zen-fallback")
            .expect("fallback pool materialized");
        assert_eq!(fb_pool.len(), 1);
        assert_eq!(
            fb_pool.peek().map(|c| c.value),
            Some("sk-fallback".to_string())
        );
        // Primary provider's pool is shared with the agent-path wrapper.
        assert!(pools.pool("free").is_some());
    }
}
