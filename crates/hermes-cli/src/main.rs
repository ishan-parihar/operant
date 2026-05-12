//! Hermes-RS CLI

mod autonomous;
pub(crate) mod config;
mod tui;
mod cmd_config;
mod cmd_sessions;
mod cmd_mcp;
mod cmd_skills;
mod cmd_model;
mod cmd_completion;
mod cmd_cron;
mod cmd_kanban;
mod cmd_gateway;
mod cmd_checkpoints;
mod cmd_memory;
mod cmd_profile;
mod cmd_auth;
mod cmd_version;
mod cmd_doctor;
mod cmd_status;
mod cmd_dump;
mod cmd_logs;
mod cmd_backup;
mod cmd_import;
mod cmd_uninstall;
mod cmd_update;
mod cmd_insights;
mod cmd_pairing;
mod cmd_webhook;
mod cmd_hooks;
mod cmd_debug;
mod cmd_computer_use;

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::CliConfig;
use anyhow::{Context, Result};
use clap::{ArgAction, Parser, Subcommand};
use hermes_core::agent::clients::openai::OpenAIModelClient;
use hermes_core::agent::{AgentConfig, AgentEvent, HermesAgent};
use hermes_core::client::{ClientConfig, OpenAIClient};
use hermes_core::config::{
    install_runtime_config, load_app_config, AppConfig, BehaviorSettings, LoggingSettings,
    McpServerConfig, McpTransportKind,
};
use hermes_core::mcp::McpManager;
use hermes_core::memory::MemoryManager;
use hermes_core::tools::{HermesTool, ToolContext, ToolRegistry};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::tui::{LaunchMode, TuiApp};
use hermes_core::cronjobs::CronDb;
use hermes_core::database::Database;
use hermes_core::kanban::KanbanDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogTarget {
    Stderr,
    Sink,
    File,
}

#[derive(Debug, Parser)]
#[command(
    name = "hermes",
    about = "Hermes-RS: A high-performance ReAct agent framework",
    version
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

    #[command(subcommand)]
    command: Commands,
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
    },
    Autonomous {
        #[arg(short, long)]
        system: Option<String>,
    },
    Tools {
        #[arg(short, long)]
        verbose: bool,
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
    },
    /// Manage MCP servers
    Mcp {
        #[command(subcommand)]
        cmd: cmd_mcp::McpSubcommand,
    },
    /// Manage installed skills
    Skills {
        #[command(subcommand)]
        cmd: cmd_skills::SkillsSubcommand,
    },
    /// View or change the active model configuration
    Model {
        #[command(subcommand)]
        cmd: cmd_model::ModelSubcommand,
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
    },
    /// Manage kanban tasks
    Kanban {
        #[command(subcommand)]
        cmd: cmd_kanban::KanbanSubcommand,
    },
    /// Manage gateway
    Gateway {
        #[command(subcommand)]
        cmd: cmd_gateway::GatewaySubcommand,
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
    /// Manage fallback models
    Fallback {
        #[command(subcommand)]
        cmd: cmd_auth::FallbackSubcommand,
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
    /// Check Hermes configuration and dependencies
    Doctor {
        /// Attempt to auto-fix common issues
        #[arg(long)]
        fix: bool,
    },
    /// Show system status overview
    Status {
        /// Show detailed status
        #[arg(long)]
        deep: bool,
    },
    /// Print a setup summary report
    Dump {
        /// Show all configuration keys as YAML
        #[arg(long)]
        all: bool,
    },
    /// View log files
    Logs {
        #[command(subcommand)]
        cmd: cmd_logs::LogsSubcommand,
    },
    /// Backup Hermes configuration and data
    Backup {
        #[command(subcommand)]
        cmd: cmd_backup::BackupSubcommand,
    },
    /// Import from a backup
    Import {
        #[command(subcommand)]
        cmd: cmd_import::ImportSubcommand,
    },
    /// Uninstall Hermes data
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
    /// Manage device pairing
    Pairing {
        #[command(subcommand)]
        cmd: cmd_pairing::PairingSubcommand,
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
    /// Manage the computer-use driver
    ComputerUse {
        #[command(subcommand)]
        cmd: cmd_computer_use::ComputerUseSubcommand,
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

    match select_log_target(logging, rich_output) {
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

fn client_config(config: &AppConfig) -> ClientConfig {
    ClientConfig::from(&config.client)
}

fn agent_config(
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
) -> Result<ToolRegistry> {
    let db_dir = config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let cron_path = db_dir.join("hermes_cron.db");
    let kanban_path = db_dir.join("hermes_kanban.db");
    let cron_db = Arc::new(CronDb::init(cron_path)?);
    let kanban_db = Arc::new(KanbanDb::init(kanban_path)?);
    let registry = ToolRegistry::new(Duration::from_secs(config.tools.registry_timeout_secs));
    hermes_core::tools::register_builtin_tools_with_sub_agent(
        &registry,
        client,
        model,
        database,
        cron_db,
        kanban_db,
        Some(mcp_manager.clone()),
    )
    .await?;
    registry.register(EchoTool::new()).await?;
    registry.register(CalculatorTool::new()).await?;

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

pub(crate) async fn create_runtime_agent(
    config: &AppConfig,
    behavior: &BehaviorSettings,
    system_prompt: Option<&str>,
    event_tx: mpsc::Sender<AgentEvent>,
    mcp_manager: &McpManager,
) -> Result<HermesAgent> {
    let raw_client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = build_registry(
        config,
        mcp_manager,
        &raw_client,
        &behavior.model,
        database.clone(),
    )
    .await?;
    let agent_config = agent_config(config, behavior, system_prompt);
    let memory_manager = load_repo_memory_manager().await?;

    Ok(HermesAgent::with_events(
        agent_config,
        Box::new(OpenAIModelClient::new(raw_client)),
        registry,
        database,
        event_tx,
    )
    .with_memory_manager(memory_manager))
}

async fn create_agent_without_events(
    config: &AppConfig,
    system_prompt: Option<&str>,
    mcp_manager: &McpManager,
) -> Result<HermesAgent> {
    let raw_client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = build_registry(
        config,
        mcp_manager,
        &raw_client,
        &config.agent.model,
        database.clone(),
    )
    .await?;
    let agent_config = agent_config(config, &config.agent, system_prompt);
    let memory_manager = load_repo_memory_manager().await?;

    Ok(HermesAgent::new(
        agent_config,
        Box::new(OpenAIModelClient::new(raw_client)),
        registry,
        database,
    )
    .with_memory_manager(memory_manager))
}

async fn load_repo_memory_manager() -> Result<MemoryManager> {
    let storage_dir = std::env::current_dir().context("Failed to determine current directory")?;
    load_memory_manager(storage_dir).await
}

async fn load_memory_manager(storage_dir: PathBuf) -> Result<MemoryManager> {
    let memory_manager = MemoryManager::with_storage_dir(storage_dir);
    memory_manager
        .load_from_disk()
        .await
        .context("Failed to load long-term memory")?;
    Ok(memory_manager)
}

async fn run_non_tui(config: &AppConfig, system_prompt: Option<&str>, query: &str) -> Result<()> {
    let mcp_manager = McpManager::new();
    let agent = create_agent_without_events(config, system_prompt, &mcp_manager).await?;
    let response = agent.run(query.to_string()).await?;
    println!("{}", response.content);
    Ok(())
}

async fn chat_non_tui(config: &AppConfig, system_prompt: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let agent = create_agent_without_events(config, system_prompt, &mcp_manager).await?;

    loop {
        print!("You: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }
        if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
            break;
        }
        if input.eq_ignore_ascii_case("clear") {
            agent.clear_history().await;
            println!("Conversation cleared.");
            continue;
        }

        match agent.run(input.to_string()).await {
            Ok(response) => println!("Assistant: {}\n", response.content),
            Err(error) => eprintln!("Error: {}\n", error),
        }
    }

    Ok(())
}

async fn list_tools(config: &AppConfig, verbose: bool) -> Result<()> {
    let mcp_manager = McpManager::new();
    let client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry =
        build_registry(config, &mcp_manager, &client, &config.agent.model, database).await?;
    let tools = registry.get_schemas().await;

    for tool in tools {
        println!("{}: {}", tool.name, tool.description);
        if verbose {
            println!("{}", serde_json::to_string_pretty(&tool.parameters)?);
        }
    }

    Ok(())
}

async fn test_tool(config: &AppConfig, tool_name: &str, args: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry =
        build_registry(config, &mcp_manager, &client, &config.agent.model, database).await?;
    let parsed_args: Value = if let Some(args) = args {
        serde_json::from_str(args).context("Failed to parse tool arguments as JSON")?
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

struct EchoTool;

impl EchoTool {
    fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl HermesTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo back the input message. Useful for testing."
    }

    fn schema(&self) -> hermes_core::schema::ToolSchema {
        use schemars::JsonSchema;

        #[derive(JsonSchema, Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct EchoArgs {
            message: String,
        }

        hermes_core::schema::ToolSchema::from_type::<EchoArgs>(
            "echo",
            "Echo back the input message",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> hermes_core::tools::ToolResult {
        if let Some(msg) = args.get("message").and_then(|value| value.as_str()) {
            hermes_core::tools::ToolResult::success("echo", serde_json::json!({ "echoed": msg }))
        } else {
            hermes_core::tools::ToolResult::error("echo", "Missing 'message' argument")
        }
    }
}

struct CalculatorTool;

impl CalculatorTool {
    fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl HermesTool for CalculatorTool {
    fn name(&self) -> &str {
        "calculate"
    }

    fn description(&self) -> &str {
        "Perform a calculation. Supports add, subtract, multiply, and divide."
    }

    fn schema(&self) -> hermes_core::schema::ToolSchema {
        use schemars::JsonSchema;

        #[derive(JsonSchema, Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct CalcArgs {
            operation: String,
            a: f64,
            b: f64,
        }

        hermes_core::schema::ToolSchema::from_type::<CalcArgs>("calculate", "Perform calculations")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> hermes_core::tools::ToolResult {
        let operation = args
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("add");
        let a = args
            .get("a")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let b = args
            .get("b")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);

        let result = match operation {
            "add" | "+" => a + b,
            "subtract" | "-" => a - b,
            "multiply" | "*" | "x" => a * b,
            "divide" | "/" => {
                if b == 0.0 {
                    return hermes_core::tools::ToolResult::error("calculate", "Division by zero");
                }
                a / b
            }
            _ => {
                return hermes_core::tools::ToolResult::error(
                    "calculate",
                    format!("Unknown operation: {}", operation),
                )
            }
        };

        hermes_core::tools::ToolResult::success(
            "calculate",
            serde_json::json!({
                "operation": operation,
                "operand_a": a,
                "operand_b": b,
                "result": result
            }),
        )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load CLI-level config (YAML-based with deep merge, .env, env expansion)
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
    if loaded.config.agent.model == "gpt-4" {
        loaded.config.agent.model = cli_app_config.agent.model;
    }

    loaded.config.apply_env_overrides()?;
    apply_cli_overrides(&cli, &mut loaded.config);
    install_runtime_config(loaded.config.clone());

    init_logging(
        cli.verbose,
        cli.log_level.as_deref(),
        &loaded.config.logging,
        loaded.config.tui.rich_output,
    );

    match &cli.command {
        Commands::Run {
            system,
            query,
            autonomous,
        } => {
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
                )
                .await?
                .run()
                .await?;
            } else {
                run_non_tui(&loaded.config, system.as_deref(), query).await?;
            }
        }
        Commands::Chat { system } => {
            if loaded.config.tui.rich_output {
                TuiApp::enter(loaded.config.clone(), system.clone(), LaunchMode::Landing)
                    .await?
                    .run()
                    .await?;
            } else {
                chat_non_tui(&loaded.config, system.as_deref()).await?;
            }
        }
        Commands::Tools { verbose } => {
            list_tools(&loaded.config, *verbose).await?;
        }
        Commands::Autonomous { system } => {
            autonomous::run_autonomous(loaded.config.clone(), system.clone()).await?;
        }
        Commands::Test { tool_name, args } => {
            test_tool(&loaded.config, tool_name, args.as_deref()).await?;
        }
        Commands::Config { cmd } => {
            cmd_config::handle_config_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Sessions { cmd } => {
            cmd_sessions::handle_sessions_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Mcp { cmd } => {
            let mcp_manager = hermes_core::mcp::McpManager::new();
            cmd_mcp::handle_mcp_command(&loaded.config, &mcp_manager, cmd.clone()).await?;
        }
        Commands::Skills { cmd } => {
            cmd_skills::handle_skills_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Model { cmd } => {
            cmd_model::handle_model_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Completion { cmd } => {
            cmd_completion::handle_completion_command(cmd.clone())?;
        }
        Commands::Cron { cmd } => {
            cmd_cron::handle_cron_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Kanban { cmd } => {
            cmd_kanban::handle_kanban_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Gateway { cmd } => {
            cmd_gateway::handle_gateway_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Checkpoints { cmd } => {
            cmd_checkpoints::handle_checkpoints_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Memory { cmd } => {
            cmd_memory::handle_memory_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Profile { cmd } => {
            cmd_profile::handle_profile_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Auth { cmd } => {
            cmd_auth::handle_auth_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Fallback { cmd } => {
            cmd_auth::handle_fallback_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Login => {
            cmd_auth::handle_login(&loaded.config).await?;
        }
        Commands::Logout => {
            cmd_auth::handle_logout(&loaded.config).await?;
        }
        Commands::Version { detailed } => {
            cmd_version::handle_version_command(&loaded.config, *detailed).await?;
        }
        Commands::Doctor { fix } => {
            cmd_doctor::handle_doctor_command(&loaded.config, *fix).await?;
        }
        Commands::Status { deep } => {
            cmd_status::handle_status_command(&loaded.config, *deep).await?;
        }
        Commands::Dump { all } => {
            cmd_dump::handle_dump_command(&loaded.config, *all).await?;
        }
        Commands::Logs { cmd } => {
            cmd_logs::handle_logs_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Backup { cmd } => {
            cmd_backup::handle_backup_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Import { cmd } => {
            cmd_import::handle_import_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Uninstall { cmd } => {
            cmd_uninstall::handle_uninstall_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Update { cmd } => {
            cmd_update::handle_update_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Insights { cmd } => {
            cmd_insights::handle_insights_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Pairing { cmd } => {
            cmd_pairing::handle_pairing_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Webhook { cmd } => {
            cmd_webhook::handle_webhook_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Hooks { cmd } => {
            cmd_hooks::handle_hooks_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::Debug { cmd } => {
            cmd_debug::handle_debug_command(&loaded.config, cmd.clone()).await?;
        }
        Commands::ComputerUse { cmd } => {
            cmd_computer_use::handle_computer_use_command(&loaded.config, cmd.clone()).await?;
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
            log_file: Some("hermes.log".to_string()),
            ..Default::default()
        };
        assert_eq!(select_log_target(&logging, true), LogTarget::File);
    }

    #[tokio::test]
    async fn load_memory_manager_reads_existing_memory_file() {
        let dir =
            std::env::temp_dir().join(format!("hermes_cli_memory_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let seed = MemoryManager::with_storage_dir(dir.clone());
        seed.store(
            hermes_core::memory::MemoryBlock::new("cli_fact", "fact", "Loaded memory fact")
                .importance(90),
        )
        .await;

        let loaded = load_memory_manager(dir.clone()).await.unwrap();

        assert_eq!(loaded.search("Loaded memory").await.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn autonomous_subcommand_parses() {
        let cli = Cli::try_parse_from(["hermes", "autonomous"]).unwrap();
        assert!(matches!(cli.command, Commands::Autonomous { .. }));
    }

    #[test]
    fn run_autonomous_flag_parses() {
        let cli = Cli::try_parse_from(["hermes", "run", "--autonomous"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Run {
                autonomous: true,
                ..
            }
        ));
    }
}
