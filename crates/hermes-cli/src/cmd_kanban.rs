//! Kanban CLI subcommand
//!
//! Provides `hermes kanban` subcommands for managing kanban tasks:
//! list, show, create, complete, block, comment, link, and stats.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::kanban::KanbanDb;
use hermes_core::kanban::TaskStatus;

/// Manage kanban tasks
#[derive(Debug, Clone, Subcommand)]
pub enum KanbanSubcommand {
    /// List all kanban tasks in a formatted table
    List,
    /// Show a kanban task with its comments
    Show {
        /// Task ID to display
        id: String,
    },
    /// Create a new kanban task
    Create {
        /// Task title
        title: String,
        /// Task description
        #[arg(long)]
        description: Option<String>,
        /// Task priority (0-5, default 0)
        #[arg(long)]
        priority: Option<String>,
    },
    /// Mark a task as complete
    Complete {
        /// Task ID to complete
        id: String,
    },
    /// Block a task
    Block {
        /// Task ID to block
        id: String,
        /// Reason for blocking
        #[arg(long)]
        reason: Option<String>,
    },
    /// Add a comment to a task
    Comment {
        /// Task ID
        id: String,
        /// Comment text
        text: String,
    },
    /// Link two tasks (parent -> child)
    Link {
        /// Parent task ID
        from_id: String,
        /// Child task ID
        to_id: String,
    },
    /// Show kanban statistics
    Stats,
}

/// Dispatch a kanban subcommand.
pub async fn handle_kanban_command(
    config: &AppConfig,
    cmd: KanbanSubcommand,
) -> Result<()> {
    match cmd {
        KanbanSubcommand::List => cmd_list(config).await,
        KanbanSubcommand::Show { id } => cmd_show(config, &id).await,
        KanbanSubcommand::Create {
            title,
            description,
            priority,
        } => {
            cmd_create(config, &title, description.as_deref(), priority.as_deref())
                .await
        }
        KanbanSubcommand::Complete { id } => cmd_complete(config, &id).await,
        KanbanSubcommand::Block { id, reason } => {
            cmd_block(config, &id, reason.as_deref()).await
        }
        KanbanSubcommand::Comment { id, text } => cmd_comment(config, &id, &text).await,
        KanbanSubcommand::Link { from_id, to_id } => {
            cmd_link(config, &from_id, &to_id).await
        }
        KanbanSubcommand::Stats => cmd_stats(config).await,
    }
}

async fn cmd_list(config: &AppConfig) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    let tasks = db
        .list_tasks()
        .context("Failed to list tasks")?;

    if tasks.is_empty() {
        println!("No kanban tasks found.");
        return Ok(());
    }

    println!(
        "{:<4} {:<12} {:<26} {:<10} {:<8} {:<16}",
        "#", "ID", "Title", "Status", "Priority", "Created"
    );
    println!("{}", "-".repeat(86));

    for (i, task) in tasks.iter().enumerate() {
        let display_title = if task.title.len() > 24 {
            format!("{}…", &task.title[..23])
        } else {
            task.title.clone()
        };
        println!(
            "{:<4} {:<12} {:<26} {:<10} {:<8} {:<16}",
            i + 1,
            task.id,
            display_title,
            task.status.as_str(),
            task.priority,
            fmt_ts(task.created_at),
        );
    }

    Ok(())
}

async fn cmd_show(config: &AppConfig, id: &str) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    let task = db
        .get_task(id)
        .context("Failed to get task")?
        .with_context(|| format!("Task '{}' not found", id))?;

    println!("ID:        {}", task.id);
    println!("Title:     {}", task.title);
    if let Some(ref body) = task.body {
        for line in body.lines() {
            println!("           {}", line);
        }
    }
    println!("Status:    {}", task.status.as_str());
    println!("Priority:  {}", task.priority);
    if let Some(ref assignee) = task.assignee {
        println!("Assignee:  {}", assignee);
    }
    println!("Created:   {}", fmt_ts(task.created_at));
    if let Some(started) = task.started_at {
        println!("Started:   {}", fmt_ts(started));
    }
    if let Some(completed) = task.completed_at {
        println!("Completed: {}", fmt_ts(completed));
    }
    println!();

    let comments = db
        .list_comments(id)
        .context("Failed to list comments")?;

    if comments.is_empty() {
        println!("Comments:  (none)");
    } else {
        println!("Comments:");
        for comment in &comments {
            println!(
                "  [{}] <{}> {}",
                fmt_ts(comment.created_at),
                comment.author,
                comment.body,
            );
        }
    }

    Ok(())
}

async fn cmd_create(
    config: &AppConfig,
    title: &str,
    description: Option<&str>,
    priority: Option<&str>,
) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;

    let prio: i32 = priority
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);

    let task_id = db
        .create_task(
            title,
            description,
            None, /* assignee */
            Some("cli"), /* created_by */
            "local", /* workspace_kind */
            None, /* workspace_path */
            None, /* tenant */
            prio,
            &[], /* parents */
            false, /* triage */
            None, /* idempotency_key */
            None, /* max_runtime_seconds */
            None, /* skills */
            None, /* max_retries */
        )
        .context("Failed to create task")?;

    println!("Created task: {}", task_id);
    Ok(())
}

async fn cmd_complete(config: &AppConfig, id: &str) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    db.complete_task(id, None, None, None, None, None)
        .context("Failed to complete task")?;
    println!("Task '{}' marked as complete.", id);
    Ok(())
}

async fn cmd_block(config: &AppConfig, id: &str, reason: Option<&str>) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    db.block_task(id, reason.unwrap_or("Blocked via CLI"), None)
        .context("Failed to block task")?;
    println!("Task '{}' blocked.", id);
    Ok(())
}

async fn cmd_comment(config: &AppConfig, id: &str, text: &str) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    db.add_comment(id, "cli", text)
        .context("Failed to add comment")?;
    println!("Comment added to task '{}'.", id);
    Ok(())
}

async fn cmd_link(config: &AppConfig, from_id: &str, to_id: &str) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    db.link_tasks(from_id, to_id)
        .context("Failed to link tasks")?;
    println!("Linked task '{}' -> '{}'.", from_id, to_id);
    Ok(())
}

async fn cmd_stats(config: &AppConfig) -> Result<()> {
    let db = KanbanDb::init(config.database_path.clone())
        .context("Failed to open kanban database")?;
    let tasks = db
        .list_tasks()
        .context("Failed to list tasks")?;

    let total = tasks.len();
    let mut by_status = std::collections::BTreeMap::new();
    for task in &tasks {
        *by_status
            .entry(task.status.as_str().to_string())
            .or_insert(0) += 1;
    }

    let done_count = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .count();
    let completion_rate = if total > 0 {
        (done_count as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    println!("Kanban Statistics");
    println!("{}", "-".repeat(40));
    println!("Database path: {}", config.database_path.display());
    println!("Total tasks:   {}", total);
    println!("Completed:     {}", done_count);
    println!("Completion:    {:.1}%", completion_rate);
    println!();

    if !by_status.is_empty() {
        println!("By status:");
        for (status, count) in &by_status {
            println!("  {:<12} {}", status, count);
        }
    }

    Ok(())
}

/// Format a Unix timestamp as a relative time string.
fn fmt_ts(unix_ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let diff = now - unix_ts;
    if diff < 0 {
        return "in the future".into();
    }
    if diff < 60 {
        return format!("{}s ago", diff);
    }
    if diff < 3600 {
        return format!("{}m ago", diff / 60);
    }
    if diff < 86400 {
        return format!("{}h ago", diff / 3600);
    }
    if diff < 86400 * 30 {
        return format!("{}d ago", diff / 86400);
    }
    format!("{}mo ago", diff / (86400 * 30))
}
