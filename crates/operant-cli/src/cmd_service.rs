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

const SERVICE_NAME: &str = "operant";

pub async fn handle_service_command(
    _config: &AppConfig,
    cmd: ServiceSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        ServiceSubcommand::Install => {
            let result = install_service().await;
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
            let result = start_service().await;
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
            let result = stop_service().await;
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
            let _ = stop_service().await;
            let result = start_service().await;
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
            let status = check_service_status().await;
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
            let result = uninstall_service().await;
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
            let log_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".operant")
                .join("logs");
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
                        .args(["-n", &lines.to_string(), "-f", log_file.to_str().unwrap()])
                        .status()?;
                } else {
                    let output = std::process::Command::new("tail")
                        .args(["-n", &lines.to_string(), log_file.to_str().unwrap()])
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

/// Install systemd/launchd service
async fn install_service() -> Result<String> {
    let exe = std::env::current_exe()?;

    #[cfg(target_os = "linux")]
    {
        let service_content = format!(
            r#"[Unit]
Description=Operant AI Agent Runtime
After=network.target

[Service]
Type=simple
ExecStart={} daemon
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#,
            exe.display()
        );

        let service_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("systemd")
            .join("user");
        std::fs::create_dir_all(&service_dir)?;
        let service_file = service_dir.join("operant.service");
        std::fs::write(&service_file, &service_content)?;

        // Reload systemd
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();

        Ok(format!("Service installed to {}", service_file.display()))
    }

    #[cfg(target_os = "macos")]
    {
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.operant.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>Restart</key>
    <string>on-failure</string>
</dict>
</plist>
"#,
            exe.display()
        );

        let launch_dir = dirs::home_dir()
            .unwrap_or_default()
            .join("Library")
            .join("LaunchAgents");
        std::fs::create_dir_all(&launch_dir)?;
        let plist_file = launch_dir.join("com.operant.agent.plist");
        std::fs::write(&plist_file, &plist_content)?;

        Ok(format!("Service installed to {}", plist_file.display()))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok("Service installation not supported on this platform. Run `operant daemon` directly.".to_string())
    }
}

async fn start_service() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "start", "operant"])
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
            "Service management not supported. Run `operant daemon` directly."
        ))
    }
}

async fn stop_service() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "stop", "operant"])
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
        Err(anyhow::anyhow!("Service management not supported on this platform."))
    }
}

async fn check_service_status() -> (String, Option<u32>) {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "status", "operant"])
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

async fn uninstall_service() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let _ = stop_service().await;
        let service_file = dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("systemd")
            .join("user")
            .join("operant.service");
        if service_file.exists() {
            std::fs::remove_file(&service_file)?;
        }
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        Ok("Service uninstalled".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        let _ = stop_service().await;
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
