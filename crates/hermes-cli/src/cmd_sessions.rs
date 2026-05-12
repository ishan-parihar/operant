//! Sessions CLI subcommand
//!
//! Provides `hermes sessions list`, `hermes sessions show <id>`,
//! `hermes sessions delete <id>`, and `hermes sessions stats`.

use std::io::{self, Write};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
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
    /// Export a session to JSON or Markdown
    Export {
        /// Session ID to export
        id: String,
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
        /// Output format: json, markdown (default: json)
        #[arg(short, long, default_value = "json")]
        format: String,
    },
    /// Prune sessions older than N days
    Prune {
        /// Age in days — sessions older than this will be deleted
        #[arg(long, default_value = "30")]
        older_than_days: u64,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Rename a session
    Rename {
        /// Session ID to rename
        id: String,
        /// New title for the session
        title: String,
    },
    /// Browse sessions interactively
    Browse,
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
        SessionsSubcommand::Export { id, output, format } => {
            cmd_export(config, &id, output, &format).await
        }
        SessionsSubcommand::Prune {
            older_than_days,
            force,
        } => cmd_prune(config, older_than_days, force).await,
        SessionsSubcommand::Rename { id, title } => {
            cmd_rename(config, &id, &title).await
        }
        SessionsSubcommand::Browse => cmd_browse(config).await,
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

async fn cmd_export(config: &AppConfig, id: &str, output: Option<String>, format: &str) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let messages = db
        .get_session_messages(id)
        .context("Failed to get session messages")?;

    match format {
        "json" => {
            let msgs: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "role": m.role,
                        "content": m.content,
                        "timestamp": m.timestamp,
                    })
                })
                .collect();
            let json = serde_json::to_string_pretty(&msgs)
                .context("Failed to serialize messages to JSON")?;
            match output {
                Some(path) => {
                    std::fs::write(&path, &json)
                        .with_context(|| format!("Failed to write to '{}'", path))?;
                    println!("Session '{}' exported to '{}'", id, path);
                }
                None => {
                    println!("{}", json);
                }
            }
        }
        "markdown" | "md" => {
            let mut md = format!("# Session: {}\n\n", id);
            md.push_str(&format!("**Total messages:** {}\n\n", messages.len()));
            for msg in &messages {
                md.push_str(&format!("## {}\n\n", msg.role));
                md.push_str(&format!("_{}_\n\n", msg.timestamp));
                md.push_str(&format!("{}\n\n", msg.content));
            }
            match output {
                Some(path) => {
                    std::fs::write(&path, &md)
                        .with_context(|| format!("Failed to write to '{}'", path))?;
                    println!("Session '{}' exported to '{}'", id, path);
                }
                None => {
                    println!("{}", md);
                }
            }
        }
        other => anyhow::bail!("Unsupported format '{}'. Use 'json' or 'markdown'.", other),
    }

    Ok(())
}

async fn cmd_prune(config: &AppConfig, older_than_days: u64, force: bool) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let sessions = db
        .list_sessions(10_000)
        .context("Failed to list sessions")?;

    let cutoff = Utc::now() - chrono::Duration::days(older_than_days as i64);

    let to_prune: Vec<&hermes_core::database::DatabaseSession> = sessions
        .iter()
        .filter(|s| {
            DateTime::parse_from_rfc3339(&s.updated_at)
                .ok()
                .map(|dt| dt.with_timezone(&Utc) < cutoff)
                .unwrap_or(false)
        })
        .collect();

    if to_prune.is_empty() {
        println!("No sessions older than {} days found.", older_than_days);
        return Ok(());
    }

    println!(
        "Found {} session(s) older than {} days:",
        to_prune.len(),
        older_than_days
    );
    for s in &to_prune {
        let title = s.title.as_deref().unwrap_or("(untitled)");
        println!("  • {} ({}) - last updated {}", s.id, title, s.updated_at);
    }

    if !force {
        print!("Delete these {} session(s)? [y/N] ", to_prune.len());
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => {}
            _ => {
                println!("Aborted.");
                return Ok(());
            }
        }
    }

    for s in &to_prune {
        db.delete_session(&s.id)
            .with_context(|| format!("Failed to delete session '{}'", s.id))?;
    }

    println!("Deleted {} session(s).", to_prune.len());
    Ok(())
}

async fn cmd_rename(config: &AppConfig, id: &str, title: &str) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    db.update_session_title(id, title)
        .with_context(|| format!("Failed to rename session '{}'", id))?;
    println!("Session '{}' renamed to '{}'", id, title);
    Ok(())
}

async fn cmd_browse(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let sessions = db.list_sessions(50).context("Failed to list sessions")?;

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    let selections: Vec<String> = sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let title = s.title.as_deref().unwrap_or("(untitled)");
            let display_title = if title.len() > 40 {
                format!("{}…", &title[..39])
            } else {
                title.to_string()
            };
            format!("{} | {:.8} | {} ({} msgs)", i + 1, s.id, display_title, s.message_count)
        })
        .collect();

    let selection = dialoguer::FuzzySelect::new()
        .with_prompt("Select a session to view")
        .items(&selections)
        .default(0)
        .interact()
        .context("Failed to get selection")?;

    let session = &sessions[selection];
    println!();
    cmd_show(config, &session.id).await
}
