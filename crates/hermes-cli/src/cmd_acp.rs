use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum AcpSubcommand {
    /// Run the ACP (Anthropic Client Protocol) server
    Server {
        /// Accept incoming hooks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        accept_hooks: bool,
    },
}

pub async fn handle_acp_command(config: &AppConfig, cmd: AcpSubcommand) -> Result<()> {
    match cmd {
        AcpSubcommand::Server { accept_hooks } => cmd_server(config, accept_hooks).await,
    }
}

async fn cmd_server(_config: &AppConfig, accept_hooks: bool) -> Result<()> {
    println!("ACP (Anthropic Client Protocol) server is a Python-only feature.");
    println!();
    println!("To run the ACP server, use the Python package:");
    println!("  cd hermes-agent && pip install -e '.[acp]' && python -m hermes_cli.main acp");
    if accept_hooks {
        println!("  Add --accept-hooks to enable hook acceptance.");
    }
    Ok(())
}
