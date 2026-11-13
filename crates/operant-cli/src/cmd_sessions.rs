//! Sessions CLI subcommand
//!
//! Provides `operant sessions list`, `operant sessions show <id>`,
//! `operant sessions delete <id>`, and `operant sessions stats`.

use std::io::{self, Write};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::database::Database;

/// Manage conversation sessions
#[derive(Debug, Clone, Subcommand)]
pub enum SessionsSubcommand {
    /// List recent sessions
    ///
    /// `--interrupted` filters to the sessions an exit marker flagged, so the
    /// operator asks "what did I leave half-done" in one token instead of
    /// reading `end_reason` out of the DB by hand.
    List {
        /// Only sessions flagged interrupted by an exit marker.
        #[arg(long)]
        interrupted: bool,
    },
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
    /// Search session messages for a query (FTS5 full-text search).
    /// Closes the TUI↔CLI parity gap for /search (audit item #6).
    Search {
        /// Search query (matched against message content via FTS5).
        query: String,
        /// Maximum number of results to return (default: 20).
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show a local recap of a session's recent activity (hermes
    /// session_recap.py parity — pure local computation, no LLM call).
    /// Defaults to the most recently updated session.
    Recap {
        /// Session ID to recap (default: most recent)
        id: Option<String>,
    },
    /// Recap a session and re-orient before continuing it. Prints the recap,
    /// the latest user prompt, and how to resume it in the TUI (/resume) or
    /// the gateway.
    ///
    /// `id` is optional so `operant sessions resume` alone works, and
    /// `--last` reads the interrupted-session marker instead of guessing from
    /// recency: with no marker it falls back to the same most-recent row, so
    /// the flag never *reduces* what the command can do. Codex has
    /// `codex resume --last "prompt"`; jcode has `jcode resume --last`.
    Resume {
        /// Session ID to resume (default: most recent)
        id: Option<String>,
        /// Resume the interrupted session the marker names, not merely the
        /// most recently updated row.
        #[arg(long)]
        last: bool,
    },
    /// Backfill titles for sessions left untitled by the auto-name wipe bug
    /// (iter-1485). One-shot and idempotent: each repaired session is named
    /// from its first user message through the same generator the live path
    /// uses, and a second run repairs nothing.
    FixTitles,
}

/// Dispatch a sessions subcommand.
pub async fn handle_sessions_command(
    config: &AppConfig,
    cmd: SessionsSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        SessionsSubcommand::List { interrupted } => {
            cmd_list_filtered(config, json, interrupted).await
        }
        SessionsSubcommand::Show { id } => cmd_show(config, &id).await,
        SessionsSubcommand::Delete { id } => cmd_delete(config, &id).await,
        SessionsSubcommand::Stats => cmd_stats(config, json).await,
        SessionsSubcommand::Export { id, output, format } => {
            cmd_export(config, &id, output, &format).await
        }
        SessionsSubcommand::Prune {
            older_than_days,
            force,
        } => cmd_prune(config, older_than_days, force).await,
        SessionsSubcommand::Rename { id, title } => cmd_rename(config, &id, &title).await,
        SessionsSubcommand::Browse => cmd_browse(config).await,
        SessionsSubcommand::Search { query, limit } => cmd_search(config, &query, limit).await,
        SessionsSubcommand::Recap { id } => cmd_recap(config, id.as_deref()).await,
        SessionsSubcommand::Resume { id, last } => cmd_resume(config, id.as_deref(), last).await,
        SessionsSubcommand::FixTitles => cmd_fix_titles(config).await,
    }
}

/// `sessions list`, optionally filtered to interrupted sessions only.
///
/// The filter is applied here rather than in a second loader because
/// `list_sessions_rich` ALREADY carries the flag: iter-1634 added
/// `RowStatus::Interrupted` to its row shape, so a `status` field on each row
/// is all the information the filter needs. A second SQL loader would be a
/// second ordering rule to keep in step for zero information gained — the
/// ponytail call the parent plan made about Gap 6 applies to this too.
async fn cmd_list_filtered(config: &AppConfig, json: bool, interrupted_only: bool) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    // The unified record (plan 2026-10-28 iter-A): one loader for the CLI,
    // the session hub, and the gateway routes — RT-A closed here too.
    let sessions = db
        .list_sessions_rich(20)
        .context("Failed to list sessions")?;

    let is_interrupted = |s: &operant_core::session::listing::SessionEntryV2| {
        matches!(
            s.status,
            operant_core::session::listing::RowStatus::Interrupted
        )
    };
    let sessions: Vec<_> = if interrupted_only {
        // Borrow, then hand the borrowed slice on: cloning the whole row to
        // filter it would allocate 20 structs to answer a yes/no question.
        sessions.iter().filter(|s| is_interrupted(s)).collect()
    } else {
        sessions.iter().collect()
    };
    let sessions = if interrupted_only && sessions.is_empty() {
        // An honest empty, not a silent one: the operator asked a specific
        // question and deserves to be told the answer is "none" rather than a
        // header with nothing under it.
        if json {
            println!("[]");
        } else {
            println!("No interrupted sessions.");
        }
        return Ok(());
    } else {
        sessions
    };

    if json {
        let items: Vec<serde_json::Value> = sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                serde_json::json!({
                    "index": i + 1,
                    "id": s.id,
                    "title": s.title.as_deref().unwrap_or("(untitled)"),
                    "updated_at": s.updated_at.to_rfc3339(),
                    "employee_id": s.employee_id,
                    "message_count": s.message_count,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!(
        "{:<4} {:<36} {:<28} {:<12} {:<20} {:>8}",
        "#", "Session ID", "Title", "Employee", "Updated At", "Messages"
    );
    println!("{}", "-".repeat(112));

    for (i, session) in sessions.iter().enumerate() {
        let title = session.title.as_deref().unwrap_or("(untitled)");
        let display_title = display_title(title);
        println!(
            "{:<4} {:<36} {:<28} {:<12} {:<20} {:>8}",
            i + 1,
            session.id,
            display_title,
            session.employee_id.as_deref().unwrap_or("-"),
            session.updated_at.to_rfc3339(),
            session.message_count,
        );
    }

    Ok(())
}

async fn cmd_show(config: &AppConfig, id: &str) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
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
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    db.delete_session(id).context("Failed to delete session")?;
    println!("Session '{}' deleted successfully.", id);
    Ok(())
}

async fn cmd_stats(config: &AppConfig, json: bool) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let session_count = db
        .get_session_count()
        .context("Failed to get session count")?;

    let db_size = std::fs::metadata(&config.database_path)
        .map(|m| m.len())
        .ok();

    if json {
        let stats = serde_json::json!({
            "database_path": config.database_path.display().to_string(),
            "total_sessions": session_count,
            "database_size_bytes": db_size,
        });
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }

    println!("Database path: {}", config.database_path.display());
    println!("Total sessions: {}", session_count);

    if let Some(size) = db_size {
        if size >= 1_000_000_000 {
            println!("Database size: {:.2} GB", size as f64 / 1_000_000_000.0);
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

async fn cmd_export(
    config: &AppConfig,
    id: &str,
    output: Option<String>,
    format: &str,
) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
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
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let sessions = db
        .list_sessions_rich(10_000)
        .context("Failed to list sessions")?;

    let cutoff = Utc::now() - chrono::Duration::days(older_than_days as i64);

    let to_prune: Vec<&operant_core::session::listing::SessionEntryV2> =
        sessions.iter().filter(|s| s.updated_at < cutoff).collect();

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
        println!(
            "  • {} ({}) - last updated {}",
            s.id,
            title,
            s.updated_at.to_rfc3339()
        );
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
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    db.update_session_title(id, title)
        .with_context(|| format!("Failed to rename session '{}'", id))?;
    println!("Session '{}' renamed to '{}'", id, title);
    Ok(())
}

/// Repair the titles the iter-1485 auto-name wipe destroyed.
///
/// Destructive is exactly what this is not: every write is guarded by
/// `title IS NULL OR trim(title) = ''`, so a session the operator renamed by
/// hand is never overwritten, and the command is safe to run any number of
/// times. It prints how many rows it repaired so a second run showing `0` is
/// proof the earlier one worked rather than a silent failure.
async fn cmd_fix_titles(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let repaired = db
        .backfill_session_titles()
        .context("Failed to backfill session titles")?;
    if repaired == 0 {
        println!(
            "No untitled sessions needed a title (every session is named or has no user message)."
        );
    } else {
        println!(
            "Named {} session(s) from their first user message. Re-running repairs nothing.",
            repaired
        );
    }
    Ok(())
}

async fn cmd_browse(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let sessions = db
        .list_sessions_rich(50)
        .context("Failed to list sessions")?;

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
            format!(
                "{} | {:.8} | {} ({} msgs)",
                i + 1,
                s.id,
                display_title,
                s.message_count
            )
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

/// Search session messages for a query using FTS5 full-text search.
/// Closes the TUI↔CLI parity gap for /search (audit item #6) — the user can
/// now find a prior exchange from the shell without scrolling the TUI.
async fn cmd_search(config: &AppConfig, query: &str, limit: usize) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let results = db
        .search_sessions(query, limit)
        .context("Failed to search sessions")?;

    if results.is_empty() {
        println!("No matches for '{}'.", query);
        return Ok(());
    }

    println!("Found {} match(es) for '{}':", results.len(), query);
    println!();
    println!("{:<3}  {:<36} {:<28} Snippet", "#", "Session ID", "Title");
    println!("{}", "-".repeat(110));

    for (i, r) in results.iter().enumerate() {
        let title = r.title.as_deref().unwrap_or("(untitled)");
        // Truncate the snippet to 50 chars for the table.
        let snippet: String = r.content.chars().take(50).collect();
        let snippet = if r.content.chars().count() > 50 {
            format!("{}…", snippet)
        } else {
            snippet
        };
        println!(
            "{:<3}  {:<36} {:<28} {}",
            i + 1,
            truncate_str(&r.session_id, 36),
            truncate_str(title, 28),
            snippet.replace('\n', " "),
        );
    }

    println!();
    println!("View a full session with: operant sessions show <session-id>");

    Ok(())
}

/// The session title as the LIST table shows it.
///
/// CHAR-safe, and it has to be: the operator's own session title carries an
/// em-dash, and the byte slice this replaced PANICKED on it (2026-11-06:
/// "end byte index 25 is not a char boundary") — in the one command an
/// operator runs to SEE their sessions.
fn display_title(title: &str) -> String {
    truncate_str(title, 26)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

/// Resolve the session to recap: the given id, or the most recent session.
fn resolve_session(
    db: &Database,
    id: Option<&str>,
) -> Result<operant_core::database::DatabaseSession> {
    match id {
        Some(id) => {
            let sessions = db
                .list_sessions(10_000)
                .context("Failed to list sessions")?;
            sessions
                .into_iter()
                .find(|s| s.id == id)
                .context(format!("Session not found: {}", id))
        }
        None => {
            let sessions = db.list_sessions(1).context("Failed to list sessions")?;
            sessions.into_iter().next().context("No sessions found")
        }
    }
}

/// Show a local recap of a session's recent activity (hermes
/// `hermes_cli/session_recap.py` parity — pure local computation, no LLM
/// call; the user re-orients instantly and for free).
async fn cmd_recap(config: &AppConfig, id: Option<&str>) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let session = resolve_session(&db, id)?;
    let messages = db
        .get_session_messages_full(&session.id)
        .context("Failed to get session messages")?;

    let recap = operant_core::session_recap::build_recap(
        &messages,
        session.title.as_deref(),
        Some(&session.id),
    );
    println!("{}", recap);
    println!();
    println!("Session: {}", session.id);
    Ok(())
}

/// Recap a session and print how to continue it (hermes `/resume` parity:
/// re-orient first, then restore history and keep going).
/// `sessions resume` — re-orient on a session and say how to continue it.
///
/// `id` is optional (falls back to most recent, which `resolve_session` already
/// does). `last` instead reads the interrupted-session marker and names THAT
/// session, so the operator sees why this session and not another: with no
/// marker it degrades to the same most-recent fallback rather than failing,
/// which is what keeps `--last` safe to add to any alias or script.
async fn cmd_resume(config: &AppConfig, id: Option<&str>, last: bool) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;

    let mut interrupted_note: Option<String> = None;
    let target_id: Option<String> = if let Some(id) = id {
        Some(id.to_string())
    } else if last {
        // The marker is the answer to "what was I doing", not a guess from
        // recency. Consuming it here is correct: this IS the resume, so the
        // one-generation notice has served its purpose.
        match operant_core::session_interrupted::consume_marker() {
            Some(marker) => {
                let live = operant_core::active_sessions::session_is_live(&marker.session_id);
                interrupted_note = Some(if live {
                    // Live in another process: say so rather than pretending a
                    // clean resumption is safe. Two processes writing one
                    // session's rows is the race the registry prevents.
                    format!(
                        "Session {} was interrupted but is still live in another process.",
                        marker.session_id
                    )
                } else {
                    format!("Session {} was interrupted.", marker.session_id)
                });
                Some(marker.session_id)
            }
            None => None,
        }
    } else {
        None
    };

    let session = resolve_session(&db, target_id.as_deref())?;
    if let Some(note) = interrupted_note {
        println!("{note}");
        println!();
    }
    let messages = db
        .get_session_messages_full(&session.id)
        .context("Failed to get session messages")?;

    let recap = operant_core::session_recap::build_recap(
        &messages,
        session.title.as_deref(),
        Some(&session.id),
    );
    println!("{}", recap);

    if let Some(prompt) = messages
        .iter()
        .rev()
        .find(|m| m.role == "user" && m.content.as_deref().is_some_and(|c| !c.trim().is_empty()))
        .and_then(|m| m.content.clone())
    {
        println!();
        println!("Last user prompt:");
        for line in prompt.lines() {
            println!("  {}", line);
        }
    }

    println!();
    println!("To continue this session:");
    // The one-line command comes FIRST: this notice exists to re-orient, and
    // the operator who ran `sessions resume <id>` already has the id — the
    // shortest path from here is the command that attaches, not a walk through
    // the TUI's own picker.
    println!("  • Shell:    {}", crate::cmd_continue::continue_command_line(&session.id));
    println!("  • TUI:      operant chat  →  /resume  →  pick this session");
    println!("  • Gateway:  send a message on the same platform channel");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list table must not die on a title with a multi-byte character.
    /// The operator's own session title carries an em-dash, and the byte
    /// slice it used to take panicked on it.
    #[test]
    fn a_title_with_a_multi_byte_character_is_truncated_without_panicking() {
        let title = "Capacity retry probe — echo status and current UTC time";
        let shown = display_title(title);
        assert!(
            shown.chars().count() <= 26,
            "the table column is 26 characters wide, got {:?}",
            shown
        );
        assert!(shown.ends_with('…'), "a cut title is ellipsised: {shown:?}");
        assert!(
            shown.chars().take(25).all(|c| title.contains(c)),
            "the cut must be a prefix of the title: {shown:?}"
        );
    }

    fn test_db() -> Database {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_sessions_cli_{}_{}.db",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_file(&path);
        Database::init(path).expect("Failed to create test database")
    }

    /// The predicate `sessions list --interrupted` filters on. Extracted so the
    /// test asserts the FILTER rather than the print format: the flag's whole
    /// contract is "only interrupted rows", and a rendering test would pass
    /// while the filter selected the wrong set.
    fn is_interrupted(s: &operant_core::session::listing::SessionEntryV2) -> bool {
        matches!(
            s.status,
            operant_core::session::listing::RowStatus::Interrupted
        )
    }

    #[test]
    fn the_interrupted_filter_selects_only_interrupted_rows() {
        let db = test_db();
        let now = chrono::Utc::now().to_rfc3339();
        db.save_session("kept", None, "local", &now, &now).unwrap();
        db.save_session("skipped", None, "local", &now, &now)
            .unwrap();
        db.mark_session_interrupted("kept").unwrap();

        let rows = db.list_sessions_rich(10).unwrap();
        let selected: Vec<&str> = rows
            .iter()
            .filter(|r| is_interrupted(r))
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(
            selected,
            vec!["kept"],
            "the filter must pass the interrupted row and drop the clean one"
        );
    }

    #[test]
    fn the_interrupted_filter_is_empty_not_all_when_nothing_is_interrupted() {
        // The failure mode that matters: a filter that degrades to "show
        // everything" when it matches nothing would answer the operator's
        // question with the unfiltered list and quietly lie.
        let db = test_db();
        let now = chrono::Utc::now().to_rfc3339();
        db.save_session("a", None, "local", &now, &now).unwrap();
        db.save_session("b", None, "local", &now, &now).unwrap();

        let rows = db.list_sessions_rich(10).unwrap();
        assert_eq!(rows.len(), 2, "sanity: both rows are listed unfiltered");
        assert_eq!(
            rows.iter().filter(|r| is_interrupted(r)).count(),
            0,
            "with nothing interrupted the filter must select nothing, not everything"
        );
    }

    #[test]
    fn resume_without_an_id_and_without_a_marker_still_resolves_the_most_recent() {
        // The parent plan's own test list names this: `--last` must never
        // REDUCE what the command can already do. `resolve_session(None)`
        // already defaults to most-recent, and `--last` with no marker must
        // fall back to exactly that rather than failing.
        let db = test_db();
        let older = "2026-01-01T00:00:00Z";
        let newer = "2026-03-01T00:00:00Z";
        db.save_session("older", None, "local", older, older)
            .unwrap();
        db.save_session("newer", None, "local", newer, newer)
            .unwrap();

        let resolved = resolve_session(&db, None).expect("no-id resume must resolve");
        assert_eq!(resolved.id, "newer", "no id means most recent");
    }
}
