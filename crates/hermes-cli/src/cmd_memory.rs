//! Memory CLI subcommand
//!
//! Provides `hermes memory list`, `hermes memory show <id>`,
//! `hermes memory search <query>`, `hermes memory store <key> <value>`,
//! `hermes memory get <id>`, `hermes memory delete <id>`,
//! `hermes memory stats`, and `hermes memory profile`.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::memory::{MemoryBlock, MemoryManager};
use hermes_core::platform::hermes_home;

/// Manage memory and user profile
#[derive(Debug, Clone, Subcommand)]
pub enum MemorySubcommand {
    /// List all stored memory sessions
    List,
    /// Show details for a memory session
    Show {
        /// Session ID to display
        id: String,
    },
    /// Search memory for a term
    Search {
        /// Search query
        query: String,
    },
    /// Store a memory entry
    Store {
        /// Memory key/ID
        key: String,
        /// Memory content/value
        value: String,
        /// Optional type name (default: "fact")
        #[arg(long)]
        type_name: Option<String>,
        /// Optional importance score (0–100, default: 50)
        #[arg(long)]
        importance: Option<u8>,
    },
    /// Get a specific memory entry by ID
    Get {
        /// Memory entry ID
        id: String,
    },
    /// Delete a memory session
    Delete {
        /// Session ID to delete
        id: String,
    },
    /// Show memory statistics
    Stats,
    /// Show the user profile stored in memory
    Profile,
}

/// Dispatch a memory subcommand.
pub async fn handle_memory_command(
    _config: &AppConfig,
    cmd: MemorySubcommand,
) -> Result<()> {
    match cmd {
        MemorySubcommand::List => cmd_list().await,
        MemorySubcommand::Show { id } => cmd_show(&id).await,
        MemorySubcommand::Search { query } => cmd_search(&query).await,
        MemorySubcommand::Store {
            key,
            value,
            type_name,
            importance,
        } => cmd_store(&key, &value, type_name, importance).await,
        MemorySubcommand::Get { id } => cmd_get(&id).await,
        MemorySubcommand::Delete { id } => cmd_delete(&id).await,
        MemorySubcommand::Stats => cmd_stats().await,
        MemorySubcommand::Profile => cmd_profile().await,
    }
}

/// Build a memory manager backed by `~/.hermes/memory`.
fn memory_manager() -> Result<MemoryManager> {
    let dir = hermes_home().join("memory");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create memory directory: {}", dir.display()))?;
    Ok(MemoryManager::with_storage_dir(dir))
}

/// Build a memory manager and pre-load data from disk.
async fn loaded_memory_manager() -> Result<MemoryManager> {
    let mm = memory_manager()?;
    mm.load_from_disk()
        .await
        .context("Failed to load memory from disk")?;
    Ok(mm)
}

/// Truncate a string for display, appending "…" when it exceeds the limit.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut truncated = s[..max_len].to_string();
        truncated.push('…');
        truncated
    }
}

async fn cmd_list() -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let sessions = mm.list_sessions().await;

    if sessions.is_empty() {
        println!("No memory sessions found.");
        return Ok(());
    }

    println!(
        "{:<4} {:<36} {:<28} {:<20} {:>8}",
        "#", "Session ID", "Title", "Last Activity", "Messages"
    );
    println!("{}", "-".repeat(100));

    for (i, session) in sessions.iter().enumerate() {
        let display_title = if session.title.len() > 26 {
            format!("{}…", &session.title[..25])
        } else {
            session.title.clone()
        };
        println!(
            "{:<4} {:<36} {:<28} {:<20} {:>8}",
            i + 1,
            session.id,
            display_title,
            session.last_activity,
            session.message_count,
        );
    }

    Ok(())
}

async fn cmd_show(id: &str) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let sessions = mm.list_sessions().await;
    let session = sessions.iter().find(|s| s.id == id);

    match session {
        Some(s) => {
            println!("Session ID:       {}", s.id);
            println!("Title:            {}", s.title);
            println!("Created At:       {}", s.created_at);
            println!("Last Activity:    {}", s.last_activity);
            println!("Message Count:    {}", s.message_count);
            println!("Total Tokens:     {}", s.total_tokens);
            println!("Archived:         {}", s.archived);
        }
        None => {
            println!("Session '{}' not found.", id);
        }
    }

    Ok(())
}

async fn cmd_search(query: &str) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let results = mm.search(query).await;

    if results.is_empty() {
        println!("No memories found matching '{}'.", query);
        return Ok(());
    }

    println!(
        "Found {} memory block(s) matching '{}':",
        results.len(),
        query
    );
    println!();

    for block in &results {
        println!("  ID:         {}", block.id);
        println!("  Type:       {}", block.block_type);
        println!("  Importance: {}", block.importance);
        println!("  Tags:       {}", block.tags.join(", "));
        println!("  Created:    {}", block.created_at);
        println!("  Content:    {}", truncate(&block.content, 80));
        println!();
    }

    Ok(())
}

async fn cmd_store(
    key: &str,
    value: &str,
    type_name: Option<String>,
    importance: Option<u8>,
) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let block_type = type_name.unwrap_or_else(|| "fact".to_string());
    let imp = importance.unwrap_or(50);
    let mut block = MemoryBlock::new(key, &block_type, value);
    block = block.importance(imp.min(100));
    mm.store(block).await;

    println!(
        "Memory stored: {} (type: {}, importance: {})",
        key, block_type, imp
    );
    Ok(())
}

async fn cmd_get(id: &str) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    match mm.get(id).await {
        Some(block) => {
            println!("ID:         {}", block.id);
            println!("Type:       {}", block.block_type);
            println!("Importance: {}", block.importance);
            println!("Tags:       {}", block.tags.join(", "));
            println!("Created:    {}", block.created_at);
            println!("Accessed:   {}", block.last_accessed);
            println!("Content:    {}", block.content);
        }
        None => {
            println!("Memory entry '{}' not found.", id);
        }
    }
    Ok(())
}

async fn cmd_delete(id: &str) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    mm.delete_session(id).await;
    mm.save_to_disk()
        .await
        .context("Failed to save memory to disk")?;
    println!("Session '{}' deleted.", id);
    Ok(())
}

async fn cmd_stats() -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let sessions = mm.list_sessions().await;
    let all_memories = mm.search("").await;

    println!("Memory Statistics:");
    println!("  Total sessions:      {}", sessions.len());
    println!("  Total memory entries: {}", all_memories.len());

    let dir = hermes_home().join("memory");
    let memory_path = dir.join("MEMORY.md");
    let user_path = dir.join("USER.md");

    let mut total_size: u64 = 0;
    if let Ok(meta) = std::fs::metadata(&memory_path) {
        total_size += meta.len();
    }
    if let Ok(meta) = std::fs::metadata(&user_path) {
        total_size += meta.len();
    }

    if total_size >= 1_000_000_000 {
        println!(
            "  Storage size:        {:.2} GB",
            total_size as f64 / 1_000_000_000.0
        );
    } else if total_size >= 1_000_000 {
        println!(
            "  Storage size:        {:.2} MB",
            total_size as f64 / 1_000_000.0
        );
    } else if total_size >= 1_000 {
        println!(
            "  Storage size:        {:.2} KB",
            total_size as f64 / 1_000.0
        );
    } else {
        println!("  Storage size:        {} bytes", total_size);
    }

    let mut type_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for block in &all_memories {
        *type_counts.entry(block.block_type.clone()).or_insert(0) += 1;
    }

    if !type_counts.is_empty() {
        println!();
        println!("  By type:");
        let mut types: Vec<_> = type_counts.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1));
        for (type_name, count) in types {
            println!("    {:20} {}", type_name, count);
        }
    }

    Ok(())
}

async fn cmd_profile() -> Result<()> {
    let mm = loaded_memory_manager().await?;
    match mm.get_profile("default").await {
        Some(profile) => {
            println!("User Profile:");
            println!("  User ID:      {}", profile.user_id);
            if let Some(ref name) = profile.name {
                println!("  Name:         {}", name);
            }

            if !profile.preferences.is_empty() {
                println!();
                println!("  Preferences:");
                let mut prefs: Vec<_> = profile.preferences.iter().collect();
                prefs.sort_by(|a, b| a.0.cmp(b.0));
                for (key, value) in prefs {
                    println!("    {}: {}", key, value);
                }
            }

            if !profile.facts.is_empty() {
                println!();
                println!("  Facts:");
                for fact in &profile.facts {
                    println!("    [{}] {}", fact.block_type, fact.content);
                }
            }
        }
        None => {
            println!("No user profile found.");
        }
    }

    Ok(())
}
