use anyhow::Result;
use clap::Subcommand;
use operant_core::acp::AcpHandler;
use operant_core::acp::AgentState;
use operant_core::acp::AgentStateTracker;
use operant_core::config::AppConfig;
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
        AcpSubcommand::Server { accept_hooks } => {
            if accept_hooks {
                anyhow::bail!(
                    "--accept-hooks is not supported: the operant ACP server does not implement ACP hooks (accepting hook requests requires the full ACP session/permission machinery). Remove the flag."
                );
            }
            cmd_server(config).await
        }
    }
}

async fn cmd_server(config: &AppConfig) -> Result<()> {
    println!("Starting ACP server over stdio...");
    println!("Listening for JSON-RPC requests on stdin...");
    println!("Supported methods: ping, status, command, stop");

    let handler = Arc::new(AcpCliHandler::new(config.clone()));

    operant_core::acp::server::run_stdio_server(handler).await?;

    println!("ACP server shut down gracefully.");
    Ok(())
}

/// ACP handler that connects to the Operant agent runtime.
///
/// The handler tracks agent state (so `status` reports `running` while a
/// command executes, not a constant `idle`) and delegates `command` execution
/// via `spawn_blocking` to isolate non-Send async boundaries that arise from
/// the MCP/agent internals.
struct AcpCliHandler {
    config: Arc<AppConfig>,
    state: AgentStateTracker,
}

impl AcpCliHandler {
    fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            state: AgentStateTracker::new(),
        }
    }
}

#[async_trait::async_trait]
impl AcpHandler for AcpCliHandler {
    async fn agent_state(&self) -> AgentState {
        self.state.get()
    }

    async fn execute_command(&self, command: &str) -> Result<String, String> {
        // Report `running` for the duration of the command so `status` is
        // truthful. (R18: agent_state previously returned Idle unconditionally.)
        self.state.set(AgentState::Running);

        // Spawn a blocking task with its own tokio runtime to isolate
        // the non-Send async context that arises from McpManager's
        // internal tracing usage.
        let config = self.config.clone();
        let cmd = command.to_string();
        let state = self.state.clone();

        let result = match tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| format!("Failed to create runtime: {}", e))?;

            rt.block_on(execute_acp_command_inner(config, &cmd))
        })
        .await
        {
            Ok(inner) => inner,
            // The blocking task panicked/cancelled: reset the state so `status`
            // does not stay wedged at `running` forever. (R18 review catch.)
            Err(e) => {
                let msg = format!("Task join error: {}", e);
                state.set(AgentState::Error(msg.clone()));
                return Err(msg);
            }
        };

        // Restore a truthful state for the next request.
        match &result {
            Ok(_) => state.set(AgentState::Idle),
            Err(err) => state.set(AgentState::Error(err.clone())),
        }
        result
    }
}

/// Inner command execution: create a fresh agent and run the command.
async fn execute_acp_command_inner(
    config: Arc<AppConfig>,
    command: &str,
) -> Result<String, String> {
    let raw_client = operant_core::client::OpenAIClient::new(
        operant_core::client::ClientConfig::from(&config.client),
    );
    let mcp_manager = operant_core::mcp::McpManager::new();
    let database = std::sync::Arc::new(
        operant_core::database::Database::init(config.database_path.clone())
            .map_err(|e| format!("Database init error: {}", e))?,
    );

    let registry = crate::build_registry(
        &config,
        &mcp_manager,
        &raw_client,
        &config.agent.model,
        database.clone(),
        None,
    )
    .await
    .map_err(|e| format!("Registry init error: {}", e))?;

    let agent_config = crate::agent_config(&config, &config.agent, None);

    let memory_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let (memory_manager, memory_provider) = crate::load_memory_manager(memory_dir)
        .await
        .map_err(|e| format!("Memory init error: {}", e))?;

    // Same credential-pool treatment as the run/TUI/gateway paths so ACP
    // sessions get per-provider multi-key rotation too (hermes parity): the
    // model client is pooled per provider, and the agent shares the primary
    // provider's pool instance.
    let provider_name = crate::tui::provider::infer_provider_from_model(&config.agent.model)
        .unwrap_or_else(|| "openai".to_string());
    let (model_client, pool_registry) =
        crate::create_model_client_with_fallback(&provider_name, &config.agent.model, &config);

    let mut agent =
        operant_core::agent::OperantAgent::new(agent_config, model_client, registry, database)
            .with_memory_manager(memory_manager);
    if let Some(provider) = memory_provider {
        agent = agent.with_memory_provider(provider);
    }
    agent = crate::attach_credential_pool(agent, &provider_name, &config, pool_registry.as_ref());

    let response = agent
        .run(command.to_string())
        .await
        .map_err(|e| format!("Agent error: {}", e))?;

    Ok(response.content)
}
