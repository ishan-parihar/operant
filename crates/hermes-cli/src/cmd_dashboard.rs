use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum DashboardSubcommand {
    /// Run the web dashboard server
    Server {
        /// Port to bind the dashboard server
        #[arg(long, default_value_t = 9119)]
        port: u16,

        /// Host address to bind
        #[arg(long, default_value_t = String::from("127.0.0.1"))]
        host: String,

        /// Skip opening the browser automatically
        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_open: bool,

        /// Run in insecure mode (disable auth)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        insecure: bool,

        /// Run in TUI mode
        #[arg(long, action = clap::ArgAction::SetTrue)]
        tui: bool,

        /// Stop the running dashboard server
        #[arg(long, action = clap::ArgAction::SetTrue)]
        stop: bool,

        /// Show dashboard server status
        #[arg(long, action = clap::ArgAction::SetTrue)]
        status: bool,
    },
}

pub async fn handle_dashboard_command(config: &AppConfig, cmd: DashboardSubcommand) -> Result<()> {
    match cmd {
        DashboardSubcommand::Server {
            port,
            host,
            no_open,
            insecure,
            tui,
            stop,
            status,
        } => cmd_server(config, port, host, no_open, insecure, tui, stop, status).await,
    }
}

async fn cmd_server(
    _config: &AppConfig,
    _port: u16,
    _host: String,
    no_open: bool,
    insecure: bool,
    tui: bool,
    stop: bool,
    status: bool,
) -> Result<()> {
    println!("The web dashboard is a Python-only feature in the Hermes agent framework.");
    println!();
    println!("To run it:");
    println!("  cd hermes-agent && pip install -e '.[dashboard]' && hermes dashboard --server");
    println!();

    let mut flags = Vec::new();

    if stop {
        flags.push("--stop");
    }
    if status {
        flags.push("--status");
    }
    if no_open {
        flags.push("--no-open");
    }
    if insecure {
        flags.push("--insecure");
    }
    if tui {
        flags.push("--tui");
    }

    if !flags.is_empty() {
        println!("Active flags: {}", flags.join(", "));
    }

    Ok(())
}
