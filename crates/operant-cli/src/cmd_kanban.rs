//! Kanban CLI subcommand
//!
//! Provides `operant kanban` subcommands for managing kanban tasks:
//! list, show, create, complete, block, comment, link, and stats.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::kanban::{KanbanDb, KanbanManager, TaskStatus};
use std::path::PathBuf;

#[derive(Debug, Clone, Subcommand)]
pub enum NotifyAction {
    /// Subscribe to notifications for a task
    Subscribe {
        id: String,
        platform: String,
        chat_id: String,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Unsubscribe from task notifications
    Unsubscribe {
        id: String,
        platform: String,
        chat_id: String,
    },
    /// List notification subscriptions for a task
    List { id: String },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AssigneesAction {
    List { id: String },
    Add { id: String, assignee: String },
    Remove { id: String, assignee: String },
}

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
        message: String,
    },
    /// Link two tasks (parent -> child)
    Link {
        /// Parent task ID
        from_id: String,
        /// Child task ID
        to_id: String,
    },
    /// Unlink two tasks
    Unlink {
        /// Parent task ID
        from_id: String,
        /// Child task ID
        to_id: String,
    },
    /// Show kanban statistics
    Stats,
    /// Initialize kanban database/schema
    Init,
    /// Assign a task to someone
    Assign {
        /// Task ID to assign
        id: String,
        /// Assignee name
        assignee: String,
    },
    /// Manage task assignees (list/add/remove)
    Assignees {
        #[command(subcommand)]
        action: AssigneesAction,
    },
    /// Unblock a blocked task
    Unblock {
        /// Task ID to unblock
        id: String,
        /// Reason for unblocking
        reason: String,
    },
    /// Archive a completed task
    Archive {
        /// Task ID to archive
        id: String,
    },
    /// Show recent activity for a task
    Tail {
        /// Task ID
        id: String,
        /// Number of events to show
        lines: Option<usize>,
    },
    /// Show run history for a task
    Runs {
        /// Task ID
        id: String,
    },
    /// Add a log entry to a task
    Log {
        /// Task ID
        id: String,
        /// Log message
        message: String,
    },
    /// Manage notification subscriptions
    Notify {
        #[command(subcommand)]
        action: NotifyAction,
    },
    /// Process pending automatic dispatches
    Dispatch,
    /// Garbage collect old/completed task data
    Gc,
    /// Run diagnostics on the kanban system
    Diagnostics,
    /// Build triage context for LLM analysis
    Triage {
        id: String,
        #[arg(long)]
        instruction: Option<String>,
    },
    /// Reset a task to 'todo' status (for stuck tasks)
    Reclaim {
        /// Task ID to reclaim
        id: String,
    },
    /// Update a task's assignee
    Reassign {
        /// Task ID
        id: String,
        /// New assignee name
        assignee: String,
    },
    /// Update a task's title and/or description
    Edit {
        /// Task ID
        id: String,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New description
        #[arg(long)]
        body: Option<String>,
    },
    /// Manually claim a task via the dispatcher
    Claim {
        /// Task ID to claim
        id: String,
    },
    /// Send a heartbeat for a task
    Heartbeat {
        /// Task ID
        id: String,
        /// Optional heartbeat note
        #[arg(long)]
        note: Option<String>,
        /// Expected run ID
        #[arg(long)]
        run_id: Option<i64>,
    },
    /// Show the triage context for a task
    Context {
        /// Task ID
        id: String,
    },
    /// List all available kanban boards
    ListBoards,
    /// Create a new kanban board with the given slug
    CreateBoard {
        /// Board slug (e.g. "work", "personal")
        slug: String,
    },
    /// Delete a kanban board (cannot delete "default")
    DeleteBoard {
        /// Board slug to delete
        slug: String,
    },
}

fn kanban_dir(config: &AppConfig) -> PathBuf {
    config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn open_db(config: &AppConfig, board_slug: &str) -> Result<KanbanDb, anyhow::Error> {
    let mgr = KanbanManager::new(kanban_dir(config));
    mgr.open_board(board_slug)
        .context("Failed to open kanban database")
}

/// Dispatch a kanban subcommand.
pub async fn handle_kanban_command(
    config: &AppConfig,
    board_slug: &str,
    cmd: KanbanSubcommand,
) -> Result<()> {
    match cmd {
        KanbanSubcommand::ListBoards => cmd_list_boards(config).await,
        KanbanSubcommand::CreateBoard { slug } => cmd_create_board(config, &slug).await,
        KanbanSubcommand::DeleteBoard { slug } => cmd_delete_board(config, &slug).await,
        KanbanSubcommand::List => cmd_list(config, board_slug).await,
        KanbanSubcommand::Show { id } => cmd_show(config, board_slug, &id).await,
        KanbanSubcommand::Create {
            title,
            description,
            priority,
        } => {
            cmd_create(
                config,
                board_slug,
                &title,
                description.as_deref(),
                priority.as_deref(),
            )
            .await
        }
        KanbanSubcommand::Complete { id } => cmd_complete(config, board_slug, &id).await,
        KanbanSubcommand::Block { id, reason } => {
            cmd_block(config, board_slug, &id, reason.as_deref()).await
        }
        KanbanSubcommand::Comment { id, message } => {
            cmd_comment(config, board_slug, &id, &message).await
        }
        KanbanSubcommand::Link { from_id, to_id } => {
            cmd_link(config, board_slug, &from_id, &to_id).await
        }
        KanbanSubcommand::Unlink { from_id, to_id } => {
            cmd_unlink(config, board_slug, &from_id, &to_id).await
        }
        KanbanSubcommand::Stats => cmd_stats(config, board_slug).await,
        KanbanSubcommand::Init => cmd_init(config, board_slug).await,
        KanbanSubcommand::Assign { id, assignee } => {
            cmd_assign(config, board_slug, &id, &assignee).await
        }
        KanbanSubcommand::Assignees { action } => cmd_assignees(config, board_slug, action).await,
        KanbanSubcommand::Unblock { id, reason } => {
            cmd_unblock(config, board_slug, &id, &reason).await
        }
        KanbanSubcommand::Archive { id } => cmd_archive(config, board_slug, &id).await,
        KanbanSubcommand::Tail { id, lines } => cmd_tail(config, board_slug, &id, lines).await,
        KanbanSubcommand::Runs { id } => cmd_runs(config, board_slug, &id).await,
        KanbanSubcommand::Log { id, message } => cmd_log(config, board_slug, &id, &message).await,
        KanbanSubcommand::Notify { action } => cmd_notify(config, board_slug, action).await,
        KanbanSubcommand::Dispatch => cmd_dispatch(config, board_slug).await,
        KanbanSubcommand::Gc => cmd_gc(config, board_slug).await,
        KanbanSubcommand::Diagnostics => cmd_diagnostics(config, board_slug).await,
        KanbanSubcommand::Triage { id, instruction } => {
            cmd_triage(config, board_slug, &id, instruction.as_deref()).await
        }
        KanbanSubcommand::Reclaim { id } => cmd_reclaim(config, board_slug, &id).await,
        KanbanSubcommand::Reassign { id, assignee } => {
            cmd_reassign(config, board_slug, &id, &assignee).await
        }
        KanbanSubcommand::Edit { id, title, body } => {
            cmd_edit(config, board_slug, &id, title.as_deref(), body.as_deref()).await
        }
        KanbanSubcommand::Claim { id } => cmd_claim(config, board_slug, &id).await,
        KanbanSubcommand::Heartbeat { id, note, run_id } => {
            cmd_heartbeat(config, board_slug, &id, note.as_deref(), run_id).await
        }
        KanbanSubcommand::Context { id } => cmd_context(config, board_slug, &id).await,
    }
}

async fn cmd_list_boards(config: &AppConfig) -> Result<()> {
    let mgr = KanbanManager::new(kanban_dir(config));
    let boards = mgr.list_boards().context("Failed to list boards")?;

    if boards.is_empty() {
        println!("No kanban boards found.");
        return Ok(());
    }

    println!("{:<16} {:<10} {:<8}", "Board", "Tasks", "Exists");
    println!("{}", "-".repeat(36));
    for b in &boards {
        println!(
            "{:<16} {:<10} {:<8}",
            b.slug,
            b.task_count,
            if b.exists { "yes" } else { "no" },
        );
    }
    Ok(())
}

async fn cmd_create_board(config: &AppConfig, slug: &str) -> Result<()> {
    let mgr = KanbanManager::new(kanban_dir(config));
    mgr.create_board(slug)
        .with_context(|| format!("Failed to create board '{}'", slug))?;
    println!("Created kanban board: {}", slug);
    Ok(())
}

async fn cmd_delete_board(config: &AppConfig, slug: &str) -> Result<()> {
    let mgr = KanbanManager::new(kanban_dir(config));
    mgr.delete_board(slug)
        .with_context(|| format!("Failed to delete board '{}'", slug))?;
    println!("Deleted kanban board: {}", slug);
    Ok(())
}

async fn cmd_list(config: &AppConfig, board_slug: &str) -> Result<()> {
    let mgr = KanbanManager::new(kanban_dir(config));
    let db = mgr
        .open_board(board_slug)
        .context("Failed to open kanban database")?;
    let tasks = db.list_tasks().context("Failed to list tasks")?;

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

async fn cmd_show(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
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

    let comments = db.list_comments(id).context("Failed to list comments")?;

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
    board_slug: &str,
    title: &str,
    description: Option<&str>,
    priority: Option<&str>,
) -> Result<()> {
    let db = open_db(config, board_slug)?;

    let prio: i32 = priority.and_then(|p| p.parse().ok()).unwrap_or(0);

    let task_id = db
        .create_task(
            title,
            description,
            None,        /* assignee */
            Some("cli"), /* created_by */
            "local",     /* workspace_kind */
            None,        /* workspace_path */
            None,        /* tenant */
            prio,
            &[],   /* parents */
            false, /* triage */
            None,  /* idempotency_key */
            None,  /* max_runtime_seconds */
            None,  /* skills */
            None,  /* max_retries */
        )
        .context("Failed to create task")?;

    println!("Created task: {}", task_id);
    Ok(())
}

async fn cmd_complete(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.complete_task(id, None, None, None, None, None)
        .context("Failed to complete task")?;
    println!("Task '{}' marked as complete.", id);
    Ok(())
}

async fn cmd_block(
    config: &AppConfig,
    board_slug: &str,
    id: &str,
    reason: Option<&str>,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.block_task(id, reason.unwrap_or("Blocked via CLI"), None)
        .context("Failed to block task")?;
    println!("Task '{}' blocked.", id);
    Ok(())
}

async fn cmd_comment(config: &AppConfig, board_slug: &str, id: &str, text: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.add_comment(id, "cli", text)
        .context("Failed to add comment")?;
    println!("Comment added to task '{}'.", id);
    Ok(())
}

async fn cmd_link(config: &AppConfig, board_slug: &str, from_id: &str, to_id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.link_tasks(from_id, to_id)
        .context("Failed to link tasks")?;
    println!("Linked task '{}' -> '{}'.", from_id, to_id);
    Ok(())
}

async fn cmd_unlink(
    config: &AppConfig,
    board_slug: &str,
    from_id: &str,
    to_id: &str,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.unlink_tasks(from_id, to_id)
        .context("Failed to unlink tasks")?;
    println!("Unlinked task '{}' -> '{}'.", from_id, to_id);
    Ok(())
}

async fn cmd_assignees(
    config: &AppConfig,
    board_slug: &str,
    action: AssigneesAction,
) -> Result<()> {
    match action {
        AssigneesAction::List { id } => {
            let db = open_db(config, board_slug)?;
            let assignees = db.list_assignees(&id).context("Failed to list assignees")?;
            if assignees.is_empty() {
                println!("No assignees for task '{}'.", id);
            } else {
                println!("Assignees for task '{}':", id);
                for a in assignees {
                    println!("  - {}", a);
                }
            }
            Ok(())
        }
        AssigneesAction::Add { id, assignee } => {
            let db = open_db(config, board_slug)?;
            db.add_assignee(&id, &assignee)
                .context("Failed to add assignee")?;
            println!("Added assignee '{}' to task '{}'.", assignee, id);
            Ok(())
        }
        AssigneesAction::Remove { id, assignee } => {
            let db = open_db(config, board_slug)?;
            db.remove_assignee(&id, &assignee)
                .context("Failed to remove assignee")?;
            println!("Removed assignee '{}' from task '{}'.", assignee, id);
            Ok(())
        }
    }
}

async fn cmd_notify(config: &AppConfig, board_slug: &str, action: NotifyAction) -> Result<()> {
    match action {
        NotifyAction::Subscribe {
            id,
            platform,
            chat_id,
            user_id,
        } => {
            let db = open_db(config, board_slug)?;
            let manager = operant_core::kanban::NotifyManager::new(db.conn().clone());
            manager
                .subscribe(&id, &platform, &chat_id, user_id.as_deref())
                .context("Failed to subscribe")?;
            println!(
                "Subscribed to notifications for task '{}' on {}.",
                id, platform
            );
            Ok(())
        }
        NotifyAction::Unsubscribe {
            id,
            platform,
            chat_id,
        } => {
            let db = open_db(config, board_slug)?;
            let manager = operant_core::kanban::NotifyManager::new(db.conn().clone());
            manager
                .unsubscribe(&id, &platform, &chat_id)
                .context("Failed to unsubscribe")?;
            println!(
                "Unsubscribed from notifications for task '{}' on {}.",
                id, platform
            );
            Ok(())
        }
        NotifyAction::List { id } => {
            let db = open_db(config, board_slug)?;
            let manager = operant_core::kanban::NotifyManager::new(db.conn().clone());
            let subs = manager
                .list_subscriptions(&id)
                .context("Failed to list subscriptions")?;
            if subs.is_empty() {
                println!("No subscriptions for task '{}'.", id);
            } else {
                println!("Subscriptions for task '{}':", id);
                for s in subs {
                    println!(
                        "  - {} on {} (chat: {}) [user: {}]",
                        s.task_id,
                        s.platform,
                        s.chat_id,
                        s.user_id.as_deref().unwrap_or("none")
                    );
                }
            }
            Ok(())
        }
    }
}

async fn cmd_stats(config: &AppConfig, board_slug: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let tasks = db.list_tasks().context("Failed to list tasks")?;

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

async fn cmd_init(config: &AppConfig, board_slug: &str) -> Result<()> {
    let mgr = KanbanManager::new(kanban_dir(config));
    mgr.open_board(board_slug)
        .context("Failed to initialize kanban database")?;
    let path = mgr.resolve_path(board_slug);
    println!("Kanban database initialized at {}", path.display());
    Ok(())
}

async fn cmd_assign(config: &AppConfig, board_slug: &str, id: &str, assignee: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.add_comment(id, "cli", &format!("Assigned to {}", assignee))
        .context("Failed to record assignment")?;
    println!("Task '{}' assigned to '{}'.", id, assignee);
    Ok(())
}

async fn cmd_unblock(config: &AppConfig, board_slug: &str, id: &str, reason: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.add_comment(id, "cli", &format!("Unblocked: {}", reason))
        .context("Failed to record unblock")?;
    println!("Task '{}' unblocked.", id);
    Ok(())
}

async fn cmd_archive(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.add_comment(id, "cli", "Task archived")
        .context("Failed to record archivation")?;
    println!("Task '{}' archived.", id);
    Ok(())
}

async fn cmd_tail(
    config: &AppConfig,
    board_slug: &str,
    id: &str,
    lines: Option<usize>,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let events = db
        .list_events(id)
        .with_context(|| format!("Failed to list events for task '{}'", id))?;

    let limit = lines.unwrap_or(10);
    let tail: Vec<_> = events.iter().rev().take(limit).rev().collect();

    if tail.is_empty() {
        println!("No events for task '{}'.", id);
        return Ok(());
    }

    println!("Recent activity for task '{}' (last {}):", id, tail.len());
    for event in tail {
        let payload_str = event
            .payload
            .as_ref()
            .map(|p| p.to_string())
            .unwrap_or_default();
        println!(
            "  [{}] {} {}",
            fmt_ts(event.created_at),
            event.kind,
            payload_str
        );
    }
    Ok(())
}

async fn cmd_runs(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let runs = db
        .list_runs(id)
        .with_context(|| format!("Failed to list runs for task '{}'", id))?;

    if runs.is_empty() {
        println!("No runs found for task '{}'.", id);
        return Ok(());
    }

    println!(
        "{:<4} {:<10} {:<16} {:<24} {:<20}",
        "#", "Run ID", "Status", "Started", "Outcome"
    );
    println!("{}", "-".repeat(78));
    for (i, run) in runs.iter().enumerate() {
        println!(
            "{:<4} {:<10} {:<16} {:<24} {:<20}",
            i + 1,
            run.id,
            run.status,
            fmt_ts(run.started_at),
            run.outcome.as_deref().unwrap_or("—"),
        );
    }
    Ok(())
}

async fn cmd_log(config: &AppConfig, board_slug: &str, id: &str, message: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.add_comment(id, "cli", message)
        .context("Failed to add log entry")?;
    println!("Log entry added to task '{}'.", id);
    Ok(())
}

async fn cmd_dispatch(config: &AppConfig, board_slug: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let dispatcher = operant_core::kanban::Dispatcher::new(db.conn().clone());

    let pending = dispatcher
        .pending_tasks(10)
        .context("Failed to query pending tasks")?;

    if pending.is_empty() {
        println!("No pending tasks to dispatch.");
        return Ok(());
    }

    println!("Dispatching {} task(s):", pending.len());
    for (task_id, title, _max_runtime) in &pending {
        match dispatcher.claim_task(task_id, "cli-dispatcher") {
            Ok(run_id) => {
                println!("  Claimed task '{}' (run {}): {}", task_id, run_id, title);
                dispatcher
                    .complete_run(task_id, run_id, "claimed", Some("Dispatched via CLI"))
                    .context("Failed to auto-complete claim")?;
                println!("  -> Marked as done (CLI dispatch is supervisory)");
            }
            Err(e) => {
                println!("  Failed to claim task '{}': {}", task_id, e);
            }
        }
    }
    Ok(())
}

async fn cmd_gc(config: &AppConfig, board_slug: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let dispatcher = operant_core::kanban::Dispatcher::new(db.conn().clone());

    let (tasks, runs, events) = dispatcher
        .gc(30)
        .context("Failed to run garbage collection")?;

    println!("Garbage collection complete:");
    println!("  Removed archived tasks: {}", tasks);
    println!("  Removed old runs:       {}", runs);
    println!("  Removed old events:     {}", events);
    Ok(())
}

async fn cmd_diagnostics(config: &AppConfig, board_slug: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let diag = operant_core::kanban::KanbanDiagnostics::new(db.conn().clone());

    let issues = diag.run_checks().context("Failed to run diagnostics")?;

    if issues.is_empty() {
        println!("No issues found — kanban system is healthy.");
        return Ok(());
    }

    let errors = issues.iter().filter(|i| i.severity == "error").count();
    let warnings = issues.iter().filter(|i| i.severity == "warning").count();

    println!(
        "Kanban Diagnostics: {} error(s), {} warning(s)",
        errors, warnings
    );
    println!();

    for issue in &issues {
        let badge = match issue.severity.as_str() {
            "error" => "E",
            "warning" => "W",
            _ => "I",
        };
        println!(
            "[{}] [{}] {} — {}",
            badge, issue.category, issue.task_id, issue.description
        );
        println!("     Action: {}", issue.action);
        println!();
    }

    Ok(())
}

async fn cmd_triage(
    config: &AppConfig,
    board_slug: &str,
    task_id: &str,
    instruction: Option<&str>,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let triage = operant_core::kanban::TriageSpecifier::new(db.conn().clone());
    let prompt = triage
        .build_prompt(task_id, instruction)
        .context("Failed to build triage context")?;
    println!("{}", prompt);
    Ok(())
}

async fn cmd_reclaim(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let conn = db.conn();
    conn.lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET status = 'todo', current_run_id = NULL WHERE id = ?1",
            (&id,),
        )
        .map_err(|e| anyhow::anyhow!("Failed to reclaim task '{}': {}", id, e))?;
    println!("Task '{}' reclaimed (reset to todo).", id);
    Ok(())
}

async fn cmd_reassign(
    config: &AppConfig,
    board_slug: &str,
    id: &str,
    assignee: &str,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let conn = db.conn();
    conn.lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET assignee = ?1 WHERE id = ?2",
            (&assignee, &id),
        )
        .map_err(|e| anyhow::anyhow!("Failed to reassign task '{}': {}", id, e))?;
    println!("Task '{}' reassigned to '{}'.", id, assignee);
    Ok(())
}

async fn cmd_edit(
    config: &AppConfig,
    board_slug: &str,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let conn = db.conn();
    let locked = conn.lock().unwrap();

    let mut updated = Vec::new();

    if let Some(t) = title {
        locked
            .execute("UPDATE tasks SET title = ?1 WHERE id = ?2", (&t, &id))
            .map_err(|e| anyhow::anyhow!("Failed to update title: {}", e))?;
        updated.push("title");
    }
    if let Some(b) = body {
        locked
            .execute("UPDATE tasks SET body = ?1 WHERE id = ?2", (&b, &id))
            .map_err(|e| anyhow::anyhow!("Failed to update body: {}", e))?;
        updated.push("description");
    }

    if updated.is_empty() {
        println!("No fields to update. Use --title and/or --body to specify changes.");
    } else {
        println!("Task '{}' updated ({}).", id, updated.join(", "));
    }
    Ok(())
}

async fn cmd_claim(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let dispatcher = operant_core::kanban::Dispatcher::new(db.conn().clone());
    let run_id = dispatcher
        .claim_task(id, "cli-claim")
        .context("Failed to claim task")?;
    println!("Claimed task '{}' (run {}).", id, run_id);
    Ok(())
}

async fn cmd_heartbeat(
    config: &AppConfig,
    board_slug: &str,
    id: &str,
    note: Option<&str>,
    run_id: Option<i64>,
) -> Result<()> {
    let db = open_db(config, board_slug)?;
    db.heartbeat_worker(id, note, run_id)
        .context("Failed to send heartbeat")?;
    println!("Heartbeat sent for task '{}'.", id);
    Ok(())
}

async fn cmd_context(config: &AppConfig, board_slug: &str, id: &str) -> Result<()> {
    let db = open_db(config, board_slug)?;
    let context = db
        .build_worker_context(id)
        .context("Failed to build context")?;
    println!("{}", context);
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
