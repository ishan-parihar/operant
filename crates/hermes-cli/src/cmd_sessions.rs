//! Sessions CLI subcommand
//!
//! Provides `hermes sessions list`, `hermes sessions show <id>`,
//! `hermes sessions delete <id>`, and `hermes sessions stats`.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::database::Database;

/// Manage conversation sessions
#[derive(Debug, Clone, Subcommand)]
pub enum SessionsSubcommand {
    /// List recent sessions
    List,
    /// Show a session's messages
    Show {
        /// Session ID to display
        id: String,
    },
    /// Delete a session and all its messages
    Delete {
        /// Session ID to delete
        id: String,
    },
    /// Show database statistics
    Stats,
}

/// Dispatch a sessions subcommand.
pub async fn handle_sessions_command(
    config: &AppConfig,
    cmd: SessionsSubcommand,
) -> Result<()> {
    match cmd {
        SessionsSubcommand::List => cmd_list(config).await,
        SessionsSubcommand::Show { id } => cmd_show(config, &id).await,
        SessionsSubcommand::Delete { id } => cmd_delete(config, &id).await,
        SessionsSubcommand::Stats => cmd_stats(config).await,
    }
}

async fn cmd_list(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let sessions = db
        .list_sessions(20)
        .context("Failed to list sessions")?;

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!(
        "{:<4} {:<36} {:<28} {:<20} {:>8}",
        "#", "Session ID", "Title", "Updated At", "Messages"
    );
    println!("{}", "-".repeat(100));

    for (i, session) in sessions.iter().enumerate() {
        let title = session.title.as_deref().unwrap_or("(untitled)");
        let display_title = if title.len() > 26 {
            format!("{}…", &title[..25])
        } else {
            title.to_string()
        };
        println!(
            "{:<4} {:<36} {:<28} {:<20} {:>8}",
            i + 1,
            session.id,
            display_title,
            session.updated_at,
            session.message_count,
        );
    }

    Ok(())
}

async fn cmd_show(config: &AppConfig, id: &str) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let messages = db
        .get_session_messages(id)
        .context("Failed to get session messages")?;

    println!("Session: {}", id);
    println!("Messages: {}", messages.len());
    println!();

    if messages.is_empty() {
        println!("(no messages)");
        return Ok(());
    }

    for msg in &messages {
        println!("[{}] {}:", msg.timestamp, msg.role);
        for line in msg.content.lines() {
            println!("  {}", line);
        }
        println!();
    }

    Ok(())
}

async fn cmd_delete(config: &AppConfig, id: &str) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    db.delete_session(id)
        .context("Failed to delete session")?;
    println!("Session '{}' deleted successfully.", id);
    Ok(())
}

async fn cmd_stats(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let session_count = db
        .get_session_count()
        .context("Failed to get session count")?;

    println!("Database path: {}", config.database_path.display());
    println!("Total sessions: {}", session_count);

    if let Ok(meta) = std::fs::metadata(&config.database_path) {
        let size = meta.len();
        if size >= 1_000_000_000 {
            println!(
                "Database size: {:.2} GB",
                size as f64 / 1_000_000_000.0
            );
        } else if size >= 1_000_000 {
            println!("Database size: {:.2} MB", size as f64 / 1_000_000.0);
        } else if size >= 1_000 {
            println!("Database size: {:.2} KB", size as f64 / 1_000.0);
        } else {
            println!("Database size: {} bytes", size);
        }
    }

    Ok(())
}
