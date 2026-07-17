//! Operant-RS CLI

mod autonomous;
mod cmd_acp;
mod cmd_auth;
mod cmd_backup;
mod cmd_checkpoints;
mod cmd_completion;
mod cmd_config;
mod cmd_cron;
mod cmd_curator;
mod cmd_dashboard;
mod cmd_debug;
mod cmd_doctor;
mod cmd_dump;
mod cmd_gateway;
mod cmd_hooks;
mod cmd_import;
mod cmd_insights;
mod cmd_kanban;
mod cmd_logs;
mod cmd_mcp;
mod cmd_memory;
mod cmd_model;
mod cmd_plugins;
mod cmd_profile;
mod cmd_sessions;
mod cmd_setup;
mod cmd_skills;
mod cmd_status;
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
use std::io::{self, Write};
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
    install_runtime_config, load_app_config,
};
use operant_core::mcp::McpManager;
use operant_core::memory::MemoryManager;
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
    /// Manage the skill curator
    Curator {
        #[command(subcommand)]
        cmd: cmd_curator::CuratorSubcommand,
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

fn client_config(config: &AppConfig) -> ClientConfig {
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
    operant_core::tools::register_builtin_tools_with_sub_agent(
        &registry,
        &config.skills.root_dir,
        client,
        model,
        database,
        cron_db,
        kanban_db,
        Some(mcp_manager.clone()),
        event_tx,
    )
    .await?;
    // (iter-153: EchoTool + CalculatorTool deleted — toy demo tools.
    // operant-core already has a real EchoTool in debug_helpers.rs.)

    // Register TDG tools only when the TDG memory provider is active.
    // This requires the TdgMemoryProvider to be initialized so we can
    // share its connection pool — if TDG init failed (and we fell back
    // to BuiltinProvider), the tools are skipped. Previously these tools
    // were registered unconditionally for every agent.
    #[cfg(feature = "tdg")]
    if config.memory.enabled && config.memory.provider == "tdg" {
        let storage_dir = operant_core::platform::operant_home();
        match operant_core::TdgMemoryProvider::new(storage_dir) {
            Ok(provider) => {
                let pool = provider.pool().clone();
                operant_core::tools::register_tdg_tools(&registry, pool).await?;
                tracing::info!("TDG tools registered (shared pool with memory provider)");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "TDG provider init failed — TDG tools not registered (builtin memory only)"
                );
            }
        }
    }

    // Register AFT (Agent File Tools) if enabled in config.
    // AFT provides 15 IDE-grade coding tools (tree-sitter outline/zoom/
    // search/callgraph, AST-aware edit/refactor, safety undo/checkpoints)
    // via a subprocess that auto-updates from GitHub releases.
    if config.tools.aft_enabled {
        let pool = std::sync::Arc::new(operant_core::aft_bridge::AftBridgePool::new());
        match operant_core::tools::register_aft_tools(&registry, pool).await {
            Ok(()) => {
                tracing::info!("AFT tools registered (15 IDE-grade coding tools)");
                // When AFT is enabled, disable the basic file/terminal tools
                // to avoid duplication. AFT provides superior versions:
                //   aft_read → replaces file_read
                //   aft_write → replaces file_write
                //   aft_edit → replaces patch
                //   aft_bash → replaces terminal
                //   aft_search → replaces file_search
                //   aft_glob → replaces file_list
                for tool in &[
                    "file_read",
                    "file_write",
                    "patch",
                    "terminal",
                    "file_search",
                    "file_list",
                ] {
                    registry.disable_tool(tool).await;
                }
                tracing::info!("Basic file/terminal tools disabled (replaced by AFT)");
            }
            Err(e) => tracing::warn!(error = %e, "AFT tool registration failed (non-fatal)"),
        }
    }

    let disabled_tools: std::collections::HashSet<String> =
        config.tools.disabled_tools.iter().cloned().collect();
    let disabled_toolsets: std::collections::HashSet<String> =
        config.tools.disabled_toolsets.iter().cloned().collect();

    if !disabled_tools.is_empty() {
        registry.set_disabled_tools(disabled_tools).await;
    }
    if !disabled_toolsets.is_empty() {
        registry.set_disabled_toolsets(disabled_toolsets).await;
    }

    if config.mcp.autoload {
        for server in config.mcp.servers.iter().filter(|server| server.enabled) {
            if !mcp_manager.contains(&server.name).await {
                connect_mcp_server(mcp_manager, server).await?;
            }
        }

        mcp_manager.sync_tools_to_registry(&registry).await;
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
    let memory_manager = load_repo_memory_manager().await?;

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
    let model_client = create_model_client(&provider, config);

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
        OperantAgent::with_events(
            core.agent_config,
            model_client,
            core.registry,
            core.database,
            event_tx,
        )
        .with_memory_manager(core.memory_manager)
        .with_skill_manager(core.skill_manager)
        .with_interrupt_flag(flag)
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
    let model_client = create_model_client(&provider, config);

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
        OperantAgent::new(
            core.agent_config,
            model_client,
            core.registry,
            core.database,
        )
        .with_memory_manager(core.memory_manager)
        .with_skill_manager(core.skill_manager)
        .with_interrupt_flag(flag)
    })
}

async fn load_repo_memory_manager() -> Result<MemoryManager> {
    let storage_dir = operant_core::platform::operant_home();
    load_memory_manager(storage_dir).await
}

pub(crate) async fn load_memory_manager(storage_dir: PathBuf) -> Result<MemoryManager> {
    let memory_manager = MemoryManager::with_storage_dir(storage_dir.clone());
    memory_manager
        .load_from_disk()
        .await
        .context("Failed to load long-term memory")?;

    // If configured provider is not "builtin", initialise it now.
    // The provider is available via operant_core::memory_provider::build_memory_provider.
    let cfg = operant_core::config::runtime_config();
    if cfg.memory.enabled && cfg.memory.provider != "builtin" && cfg.memory.provider != "disabled" {
        let provider =
            operant_core::memory_provider::build_memory_provider(&cfg.memory.provider, storage_dir);
        // Initialize in the background; failures are non-fatal.
        tokio::spawn(async move {
            if let Err(e) = provider.initialize("main").await {
                tracing::warn!(provider = %provider.name(), error = %e, "Memory provider init failed");
            }
        });
    }

    Ok(memory_manager)
}

async fn run_non_tui(
    config: &AppConfig,
    system_prompt: Option<&str>,
    query: &str,
    record_trajectory: bool,
) -> Result<()> {
    let mcp_manager = McpManager::new();
    let agent =
        create_agent_without_events(config, system_prompt, &mcp_manager, &config.skills.root_dir)
            .await?
            .with_trajectory_recording(record_trajectory);
    if record_trajectory {
        println!("Trajectory recording enabled — run will be saved to ~/.operant/trajectories/");
    }
    let response = agent.run(query.to_string()).await?;
    println!("{}", response.content);
    Ok(())
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load CLI-level config (.env file + HERMES_* env overrides; config.yaml
    // is deprecated — operant.toml below is the sole file-based config source)
    let cli_config = CliConfig::load().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load CLI config: {}. Using defaults.", e);
        CliConfig::default()
    });

    // Load core AppConfig (TOML-based) and layer on any values from CliConfig
    let mut loaded = load_app_config(cli.config.as_deref())?;

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
            if loaded.config.tui.rich_output {
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
                run_non_tui(&loaded.config, system.as_deref(), query, *record_trajectory).await?;
            }
        }
        Some(Commands::Chat { system }) => {
            if loaded.config.tui.rich_output {
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
        Some(Commands::Curator { cmd }) => {
            cmd_curator::handle_curator_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Setup {
            section,
            non_interactive,
            reset,
            reconfigure,
            quick,
        }) => {
            cmd_setup::handle_setup_command(
                &loaded.config,
                section.as_deref(),
                *non_interactive,
                *reset,
                *reconfigure,
                *quick,
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

        let loaded = load_memory_manager(dir.clone()).await.unwrap();

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
}
