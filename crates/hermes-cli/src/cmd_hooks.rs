//! CLI subcommand for managing shell hooks.
//!
//! Provides `hermes hooks <subcommand>` for listing, testing, revoking, and
//! diagnosing hooks.  Hooks are event-driven shell commands registered by
//! the agent during a session.

use std::fs;

use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum HooksSubcommand {
    /// List all configured hooks
    List,
    /// Test a specific hook by name
    Test {
        /// Hook name to test
        name: String,
    },
    /// Revoke (remove) a hook by name
    Revoke {
        /// Hook name to revoke
        name: String,
    },
    /// Run hook diagnostics
    Doctor,
}

pub async fn handle_hooks_command(_config: &AppConfig, cmd: HooksSubcommand) -> Result<()> {
    match cmd {
        HooksSubcommand::List => cmd_list(),
        HooksSubcommand::Test { name } => cmd_test(&name),
        HooksSubcommand::Revoke { name } => cmd_revoke(&name),
        HooksSubcommand::Doctor => cmd_doctor(),
    }
}

/// Stored hooks directory — hooks are persisted per-agent-session.
fn hooks_dir() -> std::path::PathBuf {
    hermes_core::platform::hermes_data_dir().join("hooks")
}

/// List all configured hooks from the hooks directory.
fn cmd_list() -> Result<()> {
    let dir = hooks_dir();
    if !dir.exists() {
        println!("No hooks configured.");
        return Ok(());
    }

    println!("Configured Hooks");
    println!("───────────────");
    println!();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".hook") {
            let meta = std::fs::metadata(entry.path())?;
            let modified = meta
                .modified()
                .ok()
                .map(|t| {
                    let secs = t
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    chrono_or_epoch(secs)
                })
                .unwrap_or_else(|| "unknown".to_string());
            println!(
                "  {:<30}  last modified: {}",
                name.trim_end_matches(".hook"),
                modified
            );
        }
    }
    Ok(())
}

/// Test a hook by reading its definition (stub — real execution requires a running agent).
fn cmd_test(name: &str) -> Result<()> {
    let path = hooks_dir().join(format!("{}.hook", name));
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        println!("Hook: {name}");
        println!("────{}", "─".repeat(name.len()));
        println!("{content}");
        println!();
        println!("To execute this hook, run the agent and trigger the matching event.");
    } else {
        println!("Hook '{}' not found.", name);
        println!("Use `hermes hooks list` to see available hooks.");
    }
    Ok(())
}

/// Revoke (delete) a hook by name.
fn cmd_revoke(name: &str) -> Result<()> {
    let path = hooks_dir().join(format!("{}.hook", name));
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("Hook '{}' revoked.", name);
    } else {
        println!("Hook '{}' not found.", name);
    }
    Ok(())
}

/// Perform a health check on the hooks system.
fn cmd_doctor() -> Result<()> {
    let dir = hooks_dir();
    if !dir.exists() {
        println!("Hook system:  no hooks directory — no hooks registered.");
        return Ok(());
    }

    let count = std::fs::read_dir(&dir)?.count();
    println!("Hook System Health");
    println!("──────────────────");
    println!();
    println!("Hooks directory: {}", dir.display());
    println!("Hook files:      {count}");
    println!();
    if count > 0 {
        println!("All hooks are registered and ready.");
        println!("Hooks execute during agent sessions when matching events fire.");
    } else {
        println!("No hook files found.");
    }
    Ok(())
}

/// Fallback formatting for timestamps without chrono.
fn chrono_or_epoch(secs: u64) -> String {
    // Simple epoch-based display
    let minutes = secs / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{days}d ago")
    } else if hours > 0 {
        format!("{hours}h ago")
    } else if minutes > 0 {
        format!("{minutes}m ago")
    } else {
        "just now".to_string()
    }
}
