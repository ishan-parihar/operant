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
        return stop_running();
    }
    if status {
        return show_status();
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
    // Recorded so `server --stop` and `server --status` can find THIS process
    // without guessing from a port scan. Written before the browser opens so
    // a stop issued immediately after launch still sees it.
    write_pid(&url);

    if !no_open && let Err(e) = open::that(&url) {
        tracing::warn!("Failed to open browser: {}", e);
    }

    let served = crate::dashboard_server::serve(listener, app).await;
    // The pid file must not outlive the process it names, or the next
    // `--stop` would signal whatever pid the OS recycled.
    let _ = std::fs::remove_file(pid_file());
    served
}

/// Where a running dashboard records `pid` + `url` (two lines).
fn pid_file() -> std::path::PathBuf {
    operant_core::platform::operant_home().join("dashboard.pid")
}

/// Write the running server's pid and URL. Failure is not fatal: the
/// dashboard still serves, `--stop` just falls back to "not running".
fn write_pid(url: &str) {
    let path = pid_file();
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            "Could not create {} for the dashboard pid file: {e}",
            parent.display()
        );
        return;
    }
    let body = format!("{}\n{url}\n", std::process::id());
    if let Err(e) = std::fs::write(&path, body) {
        tracing::warn!(
            "Could not write the dashboard pid file {}: {e}",
            path.display()
        );
    }
}

/// Parse a pid file into `(pid, url)`. Anything malformed is `None`, never a
/// guess: a wrong pid here is a kill sent to an unrelated process.
fn read_pid_at(path: &std::path::Path) -> Option<(u32, String)> {
    let body = std::fs::read_to_string(path).ok()?;
    let mut lines = body.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let url = lines.next().unwrap_or_default().trim().to_string();
    Some((pid, url))
}

/// Is `pid` a live process? `kill(pid, 0)` asks without signalling.
fn pid_alive(pid: u32) -> bool {
    // pid 0 is not "the process with id zero": kill(0, sig) addresses the
    // CALLER'S PROCESS GROUP, so a `0` in a pid file would turn `--stop`
    // into "SIGTERM my own shell". Never a target.
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 performs the permission/existence check only; it
        // delivers nothing, so it cannot affect the target process.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        // EPERM means the process EXISTS but belongs to another user.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// `operant dashboard server --status`: report the recorded server, if any.
fn show_status() -> Result<()> {
    let path = pid_file();
    match read_pid_at(&path) {
        Some((pid, url)) if pid_alive(pid) => {
            println!("Dashboard running: {url} (pid {pid})");
        }
        Some((pid, _)) => {
            println!(
                "Dashboard server is not running (stale pid {pid}); removing {}",
                path.display()
            );
            let _ = std::fs::remove_file(&path);
        }
        None => println!("Dashboard server is not running"),
    }
    Ok(())
}

/// `operant dashboard server --stop`: SIGTERM the recorded server and wait
/// for it to go, so a following `dashboard server` can bind the port.
fn stop_running() -> Result<()> {
    let path = pid_file();
    let Some((pid, url)) = read_pid_at(&path) else {
        println!("Dashboard server is not running");
        return Ok(());
    };
    if !pid_alive(pid) {
        let _ = std::fs::remove_file(&path);
        println!("Dashboard server is not running (removed stale pid file)");
        return Ok(());
    }
    // SAFETY: the pid came from our own file and was checked alive; SIGTERM
    // is the same signal the shell's `kill` sends.
    let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if sent != 0 {
        anyhow::bail!(
            "could not signal the dashboard (pid {pid}): {}",
            std::io::Error::last_os_error()
        );
    }
    // The server removes its own pid file on the way out; poll for that as
    // the completion signal, with a bounded wait so a stuck process is
    // reported instead of hanging the terminal.
    for _ in 0..30 {
        if !pid_alive(pid) {
            let _ = std::fs::remove_file(&path);
            println!("Dashboard server stopped: {url} (pid {pid})");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("dashboard (pid {pid}) did not exit within 3s of SIGTERM")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pid_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant-dashboard-pid-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create the temp dir");
        dir.join("dashboard.pid")
    }

    #[test]
    fn a_written_pid_file_round_trips_pid_and_url() {
        let path = temp_pid_file("roundtrip");
        std::fs::write(&path, "4242\nhttp://127.0.0.1:9119\n").expect("write");
        let (pid, url) = read_pid_at(&path).expect("a well-formed pid file parses");
        assert_eq!(pid, 4242);
        assert_eq!(url, "http://127.0.0.1:9119");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A malformed pid file must read as "nothing recorded", never as a
    /// guessed pid: `--stop` signals whatever this returns.
    #[test]
    fn a_malformed_pid_file_reads_as_no_server() {
        let path = temp_pid_file("malformed");
        std::fs::write(&path, "not-a-pid\n").expect("write");
        assert_eq!(read_pid_at(&path), None);
        std::fs::remove_file(&path).expect("remove");
        assert_eq!(read_pid_at(&path), None, "a missing file is also no server");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_current_process_is_alive_and_pid_zero_is_not_a_target() {
        assert!(pid_alive(std::process::id()));
        assert!(
            !pid_alive(0),
            "pid 0 addresses the caller's process group, never a dashboard"
        );
    }
}
