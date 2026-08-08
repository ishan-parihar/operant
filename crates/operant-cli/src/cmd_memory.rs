//! Memory CLI subcommand
//!
//! Provides `operant memory list`, `operant memory show <id>`,
//! `operant memory search <query>`, `operant memory store <key> <value>`,
//! `operant memory get <id>`, `operant memory delete <id>`,
//! `operant memory stats`, and `operant memory profile`.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::memory::{MemoryBlock, MemoryManager};
use operant_core::platform::operant_home;

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
    /// Import memories from a file
    Import {
        /// Path to the source file
        source: String,
    },
    /// Export memories to a file
    Export {
        /// Output file path (defaults to memories.json in current directory)
        output: Option<String>,
        /// Output format (json or text, default: json)
        #[arg(long)]
        format: Option<String>,
    },
    /// Prune old/low-importance memories
    Prune {
        /// Prune memories older than this many days
        older_than_days: Option<u64>,
    },
    /// Clear all memories (with confirmation)
    Clear {
        /// Skip confirmation prompt
        #[arg(long, action = clap::ArgAction::SetTrue)]
        confirm: bool,
    },
}

/// Dispatch a memory subcommand.
pub async fn handle_memory_command(_config: &AppConfig, cmd: MemorySubcommand) -> Result<()> {
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
        MemorySubcommand::Import { source } => cmd_import(&source).await,
        MemorySubcommand::Export { output, format } => cmd_export(output, format).await,
        MemorySubcommand::Prune { older_than_days } => cmd_prune(older_than_days).await,
        MemorySubcommand::Clear { confirm } => cmd_clear(confirm).await,
    }
}

/// Build a memory manager backed by the operant home root
/// (`~/.operant/MEMORY.md` / `USER.md`).
///
/// This MUST be the same store the agent's memory tools use — the agent
/// builds its `MemoryManager` with `operant_home()` in
/// `load_repo_memory_manager` (main.rs). This command previously pointed at
/// `~/.operant/memory/`, a phantom directory: `operant memory
/// list/search/store` operated on a store the agent never read, so
/// memories written by the agent were invisible to the CLI and vice-versa
/// (audit R5-1).
fn memory_manager() -> Result<MemoryManager> {
    let dir = operant_home();
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
    // `store` only marks the manager dirty (batch-write optimization); the
    // process exits right after this command, so flush synchronously or the
    // write is silently lost (audit R5-1b).
    mm.save_to_disk()
        .await
        .context("Failed to save memory to disk")?;

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
    // Memory *entries* (MEMORY.md blocks) and *sessions* are two distinct
    // namespaces: delete_session() only removes a session record, so the
    // old code silently no-oped on memory ids while still printing
    // success (audit R14-2 — live-verified: the entry stayed on disk).
    // Try the block namespace first — what `memory list/search/get`
    // surface — then fall back to the legacy session namespace.
    let removed_block = mm.remove_block(id).await;
    if removed_block {
        mm.save_to_disk()
            .await
            .context("Failed to save memory to disk")?;
        println!("Memory entry '{}' deleted.", id);
    } else {
        mm.delete_session(id).await;
        mm.save_to_disk()
            .await
            .context("Failed to save memory to disk")?;
        println!("Session '{}' deleted.", id);
    }
    Ok(())
}

async fn cmd_stats() -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let sessions = mm.list_sessions().await;
    let all_memories = mm.search("").await;

    println!("Memory Statistics:");
    println!("  Total sessions:      {}", sessions.len());
    println!("  Total memory entries: {}", all_memories.len());

    let dir = operant_home();
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

async fn cmd_import(source: &str) -> Result<()> {
    let content =
        std::fs::read_to_string(source).with_context(|| format!("Failed to read '{}'", source))?;

    let mm = loaded_memory_manager().await?;
    let block_id = format!(
        "import_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let block = MemoryBlock::new(&block_id, "import", &content);
    mm.store(block).await;
    // Flush synchronously — `store` only marks dirty (audit R5-1b).
    mm.save_to_disk()
        .await
        .context("Failed to save memory to disk")?;

    println!("Imported memory from '{}'", source);
    Ok(())
}

async fn cmd_export(output: Option<String>, format: Option<String>) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let blocks = mm.search("").await;

    let fmt = format.as_deref().unwrap_or("json");
    let output_path = output.unwrap_or_else(|| "memories.json".to_string());

    match fmt {
        "json" => {
            let json = serde_json::to_string_pretty(&blocks)
                .context("Failed to serialize memories to JSON")?;
            std::fs::write(&output_path, &json)
                .with_context(|| format!("Failed to write to '{}'", output_path))?;
        }
        "text" => {
            let mut text = String::new();
            for block in &blocks {
                text.push_str(&format!(
                    "[{}] {} (importance: {})\n  {}\n\n",
                    block.block_type, block.id, block.importance, block.content
                ));
            }
            std::fs::write(&output_path, &text)
                .with_context(|| format!("Failed to write to '{}'", output_path))?;
        }
        _ => anyhow::bail!("Unsupported format '{}'. Use 'json' or 'text'.", fmt),
    }

    println!(
        "Exported {} memory block(s) to {}",
        blocks.len(),
        output_path
    );
    Ok(())
}

async fn cmd_prune(older_than_days: Option<u64>) -> Result<()> {
    let mm = loaded_memory_manager().await?;
    let blocks = mm.search("").await;

    let days = older_than_days.unwrap_or(90);
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        - (days as i64 * 86_400);

    let to_prune: Vec<_> = blocks
        .iter()
        .filter(|b| b.created_at < cutoff && b.importance < 30)
        .collect();

    if to_prune.is_empty() {
        println!(
            "No memories older than {} days with low importance found.",
            days
        );
        return Ok(());
    }

    println!(
        "Found {} memory block(s) eligible for pruning (older than {} days, importance < 30):",
        to_prune.len(),
        days
    );
    for block in &to_prune {
        println!(
            "  [{}] {} (importance: {}, created: {})",
            block.block_type, block.id, block.importance, block.created_at
        );
    }

    // Actually prune (previously a preview-only no-op — audit R5-1d).
    let mut removed = 0usize;
    for block in &to_prune {
        if mm.remove_block(&block.id).await {
            removed += 1;
        }
    }
    if removed > 0 {
        mm.save_to_disk()
            .await
            .context("Failed to save memory to disk")?;
    }
    println!();
    println!("Pruned {} memory block(s).", removed);

    Ok(())
}

async fn cmd_clear(confirm: bool) -> Result<()> {
    if !confirm {
        println!("Warning: This will permanently delete ALL memories, sessions, and profiles.");
        println!("Use --confirm to proceed.");
        return Ok(());
    }

    let mm = loaded_memory_manager().await?;
    mm.clear_all().await;
    mm.save_to_disk()
        .await
        .context("Failed to save after clearing")?;

    println!("All memories, sessions, and profiles have been cleared.");
    Ok(())
}
