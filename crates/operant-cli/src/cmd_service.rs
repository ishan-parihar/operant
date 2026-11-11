use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use operant_core::config::AppConfig;

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceSubcommand {
    /// Install daemon service unit
    Install,
    /// Start daemon service
    Start,
    /// Stop daemon service
    Stop,
    /// Restart daemon service
    Restart,
    /// Check daemon service status
    Status,
    /// Uninstall daemon service unit
    Uninstall,
    /// Tail daemon service logs
    Logs {
        /// Number of lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,
    },
}

/// The systemd unit this surface manages.
///
/// Operant has ONE daemon — the gateway daemon. It used to be two:
/// `operant service install` wrote its own `operant.service` whose
/// `ExecStart` named a `daemon` subcommand the CLI never had, so the unit
/// could not start and `Restart=on-failure` turned it into an endless
/// restart loop (RT-507). This surface now manages the unit the gateway
/// installer writes.
const SERVICE_NAME: &str = "operant-gateway";

pub fn handle_service_command(
    config: &AppConfig,
    cmd: ServiceSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        ServiceSubcommand::Install => {
            let result = install_service(config);
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": if result.is_ok() { "installed" } else { "error" }, "error": result.err().map(|e| e.to_string())})
                );
            } else {
                match result {
                    Ok(msg) => println!("✅ {}", msg),
                    Err(e) => println!("❌ Installation failed: {}", e),
                }
            }
            Ok(())
        }
        ServiceSubcommand::Start => {
            let result = start_service();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": if result.is_ok() { "started" } else { "error" }, "error": result.err().map(|e| e.to_string())})
                );
            } else {
                match result {
                    Ok(msg) => println!("✅ {}", msg),
                    Err(e) => println!("❌ Start failed: {}", e),
                }
            }
            Ok(())
        }
        ServiceSubcommand::Stop => {
            let result = stop_service();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": if result.is_ok() { "stopped" } else { "error" }, "error": result.err().map(|e| e.to_string())})
                );
            } else {
                match result {
                    Ok(msg) => println!("✅ {}", msg),
                    Err(e) => println!("❌ Stop failed: {}", e),
                }
            }
            Ok(())
        }
        ServiceSubcommand::Restart => {
            let _ = stop_service();
            let result = start_service();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": if result.is_ok() { "restarted" } else { "error" }, "error": result.err().map(|e| e.to_string())})
                );
            } else {
                match result {
                    Ok(msg) => println!("✅ {}", msg),
                    Err(e) => println!("❌ Restart failed: {}", e),
                }
            }
            Ok(())
        }
        ServiceSubcommand::Status => {
            let status = check_service_status();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"service": SERVICE_NAME, "status": status.0, "pid": status.1})
                );
            } else {
                println!("{}: {}", SERVICE_NAME, status.0);
                if let Some(pid) = status.1 {
                    println!("PID: {}", pid);
                }
            }
            Ok(())
        }
        ServiceSubcommand::Uninstall => {
            let result = uninstall_service();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": if result.is_ok() { "uninstalled" } else { "error" }, "error": result.err().map(|e| e.to_string())})
                );
            } else {
                match result {
                    Ok(msg) => println!("✅ {}", msg),
                    Err(e) => println!("❌ Uninstall failed: {}", e),
                }
            }
            Ok(())
        }
        ServiceSubcommand::Logs { lines, follow } => {
            let log_dir = operant_core::platform::operant_home().join("logs");
            let log_file = log_dir.join("operant.log");

            if json {
                println!(
                    "{}",
                    serde_json::json!({"lines": lines, "follow": follow, "log_file": log_file.to_string_lossy()})
                );
            } else if log_file.exists() {
                println!("=== {} (last {} lines) ===", log_file.display(), lines);
                if follow {
                    println!("Following log output (Ctrl-C to stop)...");
                    // Use tail -f if available
                    std::process::Command::new("tail")
                        .args(["-n", &lines.to_string(), "-f"])
                        .arg(&log_file)
                        .status()?;
                } else {
                    let output = std::process::Command::new("tail")
                        .args(["-n", &lines.to_string()])
                        .arg(&log_file)
                        .output()?;
                    print!("{}", String::from_utf8_lossy(&output.stdout));
                }
            } else {
                println!("No log file found at {}", log_file.display());
            }
            Ok(())
        }
    }
}

/// Install the daemon service unit.
///
/// Delegates to the gateway installer, which is the ONE owner of the unit
/// text. The unit this surface used to write (`operant.service`, with
/// `ExecStart=<bin> daemon`) could never start: the CLI has no `daemon`
/// subcommand, and the unit's `Restart=on-failure` + `RestartSec=5` turned
/// every `systemctl --user start operant` into an endless restart loop
/// (RT-507).
fn install_service(_config: &AppConfig) -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        crate::cmd_gateway::cmd_install(_config, false, false)?;
        Ok(format!("Service installed (unit: {SERVICE_NAME})"))
    }

    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe()?;
        let plist = launchd_plist_content(&exe.display().to_string());
        let launch_dir = dirs::home_dir()
            .unwrap_or_default()
            .join("Library")
            .join("LaunchAgents");
        std::fs::create_dir_all(&launch_dir)?;
        let plist_file = launch_dir.join("com.operant.agent.plist");
        std::fs::write(&plist_file, plist)?;
        Ok(format!("Service installed to {}", plist_file.display()))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(anyhow::anyhow!(
            "Service installation not supported on this platform. Run `operant gateway run` directly."
        ))
    }
}

/// The launchd agent that starts the gateway on macOS.
///
/// `gateway run` — NOT `daemon`, which the CLI has never had (RT-507).
/// Compiled for macOS and for the test that pins the argument, so the test
/// runs on every platform without carrying a dead function on Linux.
#[cfg(any(target_os = "macos", test))]
fn launchd_plist_content(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.operant.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>gateway</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>Restart</key>
    <string>on-failure</string>
</dict>
</plist>
"#,
        exe
    )
}

fn start_service() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "start", SERVICE_NAME])
            .status()?;
        if status.success() {
            Ok("Service started".to_string())
        } else {
            Err(anyhow::anyhow!("systemctl start failed with {}", status))
        }
    }

    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("launchctl")
            .args(["start", "com.operant.agent"])
            .status()?;
        if status.success() {
            Ok("Service started".to_string())
        } else {
            Err(anyhow::anyhow!("launchctl start failed with {}", status))
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(anyhow::anyhow!(
            "Service management not supported. Run `operant gateway run` directly."
        ))
    }
}

fn stop_service() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .status()?;
        if status.success() {
            Ok("Service stopped".to_string())
        } else {
            Err(anyhow::anyhow!("systemctl stop failed with {}", status))
        }
    }

    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("launchctl")
            .args(["stop", "com.operant.agent"])
            .status()?;
        if status.success() {
            Ok("Service stopped".to_string())
        } else {
            Err(anyhow::anyhow!("launchctl stop failed with {}", status))
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(anyhow::anyhow!(
            "Service management not supported on this platform."
        ))
    }
}

fn check_service_status() -> (String, Option<u32>) {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "status", SERVICE_NAME])
            .output();
        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                if stdout.contains("Active: active") {
                    let pid = stdout
                        .lines()
                        .find(|l| l.contains("Main PID:"))
                        .and_then(|l| l.split_whitespace().nth(2))
                        .and_then(|s| s.parse().ok());
                    ("active".to_string(), pid)
                } else {
                    ("inactive".to_string(), None)
                }
            }
            Err(_) => ("not installed".to_string(), None),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("launchctl")
            .args(["list", "com.operant.agent"])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let pid = stdout
                    .lines()
                    .find(|l| l.starts_with("PID"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse().ok());
                ("active".to_string(), pid)
            }
            _ => ("inactive".to_string(), None),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        ("not supported".to_string(), None)
    }
}

fn uninstall_service() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let _ = stop_service();
        let user_unit_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("systemd")
            .join("user");
        let service_file = user_unit_dir.join(format!("{SERVICE_NAME}.service"));
        if service_file.exists() {
            std::fs::remove_file(&service_file)?;
        }
        // The unit the old installer wrote (`operant.service`, whose
        // `ExecStart` named a subcommand that does not exist) is removed
        // too: an operator who ran it is left with a service that restarts
        // forever (RT-507).
        let legacy_file = user_unit_dir.join("operant.service");
        if legacy_file.exists() {
            std::fs::remove_file(&legacy_file)?;
        }
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        Ok("Service uninstalled".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        let _ = stop_service();
        let plist_file = dirs::home_dir()
            .unwrap_or_default()
            .join("Library")
            .join("LaunchAgents")
            .join("com.operant.agent.plist");
        if plist_file.exists() {
            std::fs::remove_file(&plist_file)?;
        }
        Ok("Service uninstalled".to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok("Service management not supported on this platform.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The subcommand the installed unit starts, and the CLI subcommands
    /// that exist. The old unit named `daemon`, which the CLI has never
    /// had — the unit could not start (RT-507).
    fn unit_subcommand(unit: &str) -> String {
        let exec = unit
            .lines()
            .find(|l| l.starts_with("ExecStart="))
            .expect("the unit has an ExecStart line");
        let argv: Vec<&str> = exec
            .trim_start_matches("ExecStart=")
            .split_whitespace()
            .collect();
        argv[1].to_string()
    }

    #[test]
    fn the_installed_unit_names_a_subcommand_the_cli_actually_has() {
        let unit = crate::cmd_gateway::systemd_unit_content("/usr/bin/operant", "");
        let sub = unit_subcommand(&unit);
        let names: Vec<String> = crate::Cli::command()
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n == &sub),
            "the installed unit starts `{sub}`, which is not a CLI subcommand ({names:?})"
        );
    }

    #[test]
    fn the_launchd_agent_starts_the_gateway_subcommand() {
        let plist = launchd_plist_content("/usr/local/bin/operant");
        assert!(
            plist.contains("<string>gateway</string>"),
            "the launchd agent must run the gateway subcommand: {plist}"
        );
        assert!(
            !plist.contains("<string>daemon</string>"),
            "the launchd agent must not name the `daemon` subcommand the CLI never had"
        );
    }

    #[test]
    fn the_service_surface_manages_the_gateway_unit() {
        assert_eq!(SERVICE_NAME, "operant-gateway");
    }
}
