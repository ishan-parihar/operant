use anyhow::Result;
use clap::Subcommand;
use hermes_core::acp::AcpHandler;
use hermes_core::acp::AgentState;
use hermes_core::config::AppConfig;
use std::sync::Arc;

#[derive(Debug, Clone, Subcommand)]
pub enum AcpSubcommand {
    /// Run the ACP (Agent Control Protocol) server
    Server {
        /// Accept incoming hooks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        accept_hooks: bool,
    },
}

pub async fn handle_acp_command(config: &AppConfig, cmd: AcpSubcommand) -> Result<()> {
    match cmd {
        AcpSubcommand::Server { accept_hooks: _ } => cmd_server(config).await,
    }
}

async fn cmd_server(config: &AppConfig) -> Result<()> {
    println!("Starting ACP server over stdio...");
    println!("Listening for JSON-RPC requests on stdin...");
    println!("Supported methods: ping, status, command, stop");

    let handler = Arc::new(AcpCliHandler::new(config.clone()));

    hermes_core::acp::server::run_stdio_server(handler).await?;

    println!("ACP server shut down gracefully.");
    Ok(())
}

/// ACP handler that connects to the Hermes agent runtime.
///
/// The handler tracks agent state and delegates `command` execution
/// via `spawn_blocking` to isolate non-Send async boundaries that
/// arise from the MCP/agent internals.
struct AcpCliHandler {
    config: Arc<AppConfig>,
}

impl AcpCliHandler {
    fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

#[async_trait::async_trait]
impl AcpHandler for AcpCliHandler {
    async fn agent_state(&self) -> AgentState {
        AgentState::Idle
    }

    async fn execute_command(&self, command: &str) -> Result<String, String> {
        // Spawn a blocking task with its own tokio runtime to isolate
        // the non-Send async context that arises from McpManager's
        // internal tracing usage.
        let config = self.config.clone();
        let cmd = command.to_string();

        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| format!("Failed to create runtime: {}", e))?;

            rt.block_on(execute_acp_command_inner(config, &cmd))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
}

/// Inner command execution: create a fresh agent and run the command.
async fn execute_acp_command_inner(
    config: Arc<AppConfig>,
    command: &str,
) -> Result<String, String> {
    let raw_client = hermes_core::client::OpenAIClient::new(
        hermes_core::client::ClientConfig::from(&config.client),
    );
    let mcp_manager = hermes_core::mcp::McpManager::new();
    let database = std::sync::Arc::new(
        hermes_core::database::Database::init(config.database_path.clone())
            .map_err(|e| format!("Database init error: {}", e))?,
    );

    let registry = crate::build_registry(
        &config,
        &mcp_manager,
        &raw_client,
        &config.agent.model,
        database.clone(),
    )
    .await
    .map_err(|e| format!("Registry init error: {}", e))?;

    let agent_config = crate::agent_config(&config, &config.agent, None);

    let memory_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let memory_manager = crate::load_memory_manager(memory_dir)
        .await
        .map_err(|e| format!("Memory init error: {}", e))?;

    let agent = hermes_core::agent::HermesAgent::new(
        agent_config,
        Box::new(hermes_core::agent::clients::openai::OpenAIModelClient::new(
            raw_client,
        )),
        registry,
        database,
    )
    .with_memory_manager(memory_manager);

    let response = agent
        .run(command.to_string())
        .await
        .map_err(|e| format!("Agent error: {}", e))?;

    Ok(response.content)
}
