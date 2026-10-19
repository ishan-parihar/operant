use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;

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

#[allow(clippy::too_many_arguments)]
async fn cmd_server(
    config: &AppConfig,
    port: u16,
    host: String,
    no_open: bool,
    insecure: bool,
    tui: bool,
    stop: bool,
    status: bool,
) -> Result<()> {
    if stop {
        println!("Dashboard stop not yet implemented (kill the process)");
        return Ok(());
    }
    if status {
        println!("Dashboard status: use the /api/status endpoint when running");
        return Ok(());
    }
    if tui {
        anyhow::bail!("Use `operant chat --tui` or `operant autonomous` for TUI mode");
    }

    // The session token is generated HERE, not inside the server, because
    // the browser has to open at a URL that carries it: the index route
    // only serves the token-bearing page to a request that already holds
    // the token. Without the token in the URL the dashboard would boot
    // empty.
    let session_token = if insecure {
        tracing::warn!("Dashboard running in INSECURE mode — no auth token required");
        None
    } else {
        let token = crate::dashboard_server::new_session_token();
        println!("Dashboard session token: {}", token);
        println!(
            "  (API requests must include header: Authorization: Bearer {})",
            token
        );
        Some(token)
    };

    // Bind BEFORE opening the browser: a browser that launches before the
    // server is listening gets a connection-refused on its first request,
    // and the operator sees a dead dashboard.
    let listener = crate::dashboard_server::bind_listener(&host, port).await?;
    let app = crate::dashboard_server::build_app(config, &host, port, session_token.clone());

    let url = crate::dashboard_server::dashboard_url(&host, port, session_token.as_deref());
    println!("Starting Operant Dashboard on {}", url);

    if !no_open && let Err(e) = open::that(&url) {
        tracing::warn!("Failed to open browser: {}", e);
    }

    crate::dashboard_server::serve(listener, app).await
}
