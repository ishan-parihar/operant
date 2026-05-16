//! CLI subcommand for managing the computer-use (CUA) driver.
//!
//! Provides `hermes computer-use <subcommand>` for checking the status of
//! the CUA driver and triggering its installation.

use std::process::Command;

use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum ComputerUseSubcommand {
    /// Install the CUA driver
    Install,
    /// Check if the CUA driver is installed
    Status,
}

pub async fn handle_computer_use_command(
    _config: &AppConfig,
    cmd: ComputerUseSubcommand,
) -> Result<()> {
    match cmd {
        ComputerUseSubcommand::Install => cmd_install(),
        ComputerUseSubcommand::Status => cmd_status(),
    }
}

/// Check whether `cua-driver` is available on PATH.
fn cmd_status() -> Result<()> {
    println!("Computer-Use Driver Status");
    println!("──────────────────────────");
    println!();

    match find_cua_driver() {
        Some(path) => {
            println!("✓ cua-driver is installed");
            println!("  Path: {}", path);
            // Try to get version
            if let Ok(ver) = get_version(&path) {
                println!("  Version: {}", ver);
            }
        }
        None => {
            println!("✗ cua-driver is NOT installed");
            println!();
            println!("To install:");
            println!("  hermes computer-use install");
            println!();
            println!("Or manually:");
            println!("  pip install cua-driver");
            println!("  # or follow instructions at:");
            println!("  # https://github.com/anthropics/cua-driver");
        }
    }
    Ok(())
}

/// Print installation instructions (stub — actual install requires pip/npm).
fn cmd_install() -> Result<()> {
    println!("Installing CUA Driver");
    println!("─────────────────────");
    println!();

    if find_cua_driver().is_some() {
        println!("✓ cua-driver is already installed.");
        return Ok(());
    }

    println!("To install the CUA driver:");

    // Detect package manager
    if has_command("pip3") {
        println!();
        println!("  pip3 install cua-driver");
    } else if has_command("pip") {
        println!();
        println!("  pip install cua-driver");
    } else if has_command("npm") {
        println!();
        println!("  npm install -g cua-driver");
    } else {
        println!();
        println!("  pip install cua-driver");
        println!();
        println!("(Requires Python 3.8+. Install pip for your system if needed.)");
    }

    println!();
    println!("After installation, verify with:");
    println!("  hermes computer-use status");
    Ok(())
}

/// Look for cua-driver on PATH.
fn find_cua_driver() -> Option<String> {
    for name in &["cua-driver", "cua-driver.exe"] {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
        // Also try command -v as fallback
        if let Ok(output) = Command::new("command").arg("-v").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Try to get the version string from a binary.
fn get_version(path: &str) -> Result<String> {
    let output = Command::new(path).arg("--version").output()?;
    if output.status.success() {
        let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !ver.is_empty() {
            return Ok(ver);
        }
        let ver = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !ver.is_empty() {
            return Ok(ver);
        }
    }
    Ok("unknown".to_string())
}

/// Check if a command is available on PATH.
fn has_command(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .ok()
        .map_or(false, |o| o.status.success())
}
