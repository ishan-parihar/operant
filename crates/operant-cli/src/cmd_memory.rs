//! Memory CLI subcommand — (plan 08) ONE memory engine.
//!
//! Every subcommand reads and writes memory-wire (`~/.operant/memory_wire.sqlite`,
//! personal bank) — the SAME engine the agent recalls from, opened exactly
//! the way the runtime opens it (`MemoryWireProvider::new(operant_home())`).
//! The old implementation sat on `MemoryManager`/`MEMORY.md`/`USER.md`, a
//! store the agent never read: `operant memory store` was invisible to the
//! agent and `/whoami` said "I don't know much about you yet" while
//! memory-wire held hundreds of rows.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::memory_wire::MemoryWireProvider;
use operant_core::platform::operant_home;

/// Manage the memory-wire store (the agent's memory)
#[derive(Debug, Clone, Subcommand)]
pub enum MemorySubcommand {
    /// List memories (newest first)
    List {
        /// Show at most this many (default 50)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show one memory by id (alias of `show`)
    Get {
        /// Memory id
        id: String,
    },
    /// Search memory for a query (the engine's own recall)
    Search {
        /// Search query
        query: String,
    },
    /// Store a memory (the agent can recall it on later turns)
    Store {
        /// The memory text to retain
        text: String,
    },
    /// Delete a memory by id
    Delete {
        /// Memory id
        id: String,
    },
    /// Show memory statistics
    Stats,
    /// Import memories from a file (JSONL, or a legacy MEMORY.md/USER.md)
    Import {
        /// Path to the source file
        source: String,
    },
    /// Export memories to a file (JSONL)
    Export {
        /// Output file path (defaults to memories.jsonl in the current directory)
        output: Option<String>,
    },
    /// Clear ALL memories in the bank (with confirmation)
    Clear {
        /// Skip confirmation prompt
        #[arg(long, action = clap::ArgAction::SetTrue)]
        confirm: bool,
    },
}

/// Dispatch a memory subcommand.
pub async fn handle_memory_command(config: &AppConfig, cmd: MemorySubcommand) -> Result<()> {
    // The provider is opened exactly the way the runtime opens the agent's:
    // personal bank over `~/.operant/memory_wire.sqlite`.
    let _ = config;
    let provider = MemoryWireProvider::new(operant_home())
        .context("opening the memory-wire store (~/.operant/memory_wire.sqlite)")?;
    match cmd {
        MemorySubcommand::List { limit } => cmd_list(&provider, limit.unwrap_or(50)),
        MemorySubcommand::Get { id } => cmd_get(&provider, &id),
        MemorySubcommand::Search { query } => cmd_search(&provider, &query),
        MemorySubcommand::Store { text } => cmd_store(&provider, &text),
        MemorySubcommand::Delete { id } => cmd_delete(&provider, &id),
        MemorySubcommand::Stats => cmd_stats(&provider),
        MemorySubcommand::Import { source } => cmd_import(&provider, &source),
        MemorySubcommand::Export { output } => cmd_export(&provider, output),
        MemorySubcommand::Clear { confirm } => cmd_clear(&provider, confirm),
    }
}

/// Truncate a string for display, appending "…" when it exceeds the limit.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_chars).collect();
        truncated.push('…');
        truncated
    }
}

fn cmd_list(provider: &MemoryWireProvider, limit: usize) -> Result<()> {
    let rows = provider.list_page(limit, 0);
    if rows.is_empty() {
        println!(
            "No memories in bank {:?} yet — `operant memory store <text>` adds one.",
            provider.bank_id()
        );
        return Ok(());
    }
    println!(
        "{} memory(ies) in bank {:?} (showing up to {limit}):",
        rows.len(),
        provider.bank_id()
    );
    for m in &rows {
        let created = m.created_at.as_deref().unwrap_or("(unstamped)");
        println!("  {}  {}  {}", m.id, created, truncate(&m.content, 70));
    }
    Ok(())
}

fn cmd_get(provider: &MemoryWireProvider, id: &str) -> Result<()> {
    match provider.memory_by_id(id) {
        Some(m) => {
            println!("ID:      {}", m.id);
            println!("Bank:    {}", m.bank_id);
            println!(
                "Created: {}",
                m.created_at.as_deref().unwrap_or("(unstamped)")
            );
            println!("Content:");
            println!("{}", m.content);
            Ok(())
        }
        None => {
            println!("Memory '{id}' not found in bank {:?}.", provider.bank_id());
            Ok(())
        }
    }
}

fn cmd_search(provider: &MemoryWireProvider, query: &str) -> Result<()> {
    let block = provider.recall_text(query, 10);
    if block.trim().is_empty() {
        println!("No memories match {query:?}.");
        return Ok(());
    }
    println!("{block}");
    Ok(())
}

fn cmd_store(provider: &MemoryWireProvider, text: &str) -> Result<()> {
    match provider.retain_fact(text, &[]) {
        Some(id) => {
            println!("Stored memory {id} in bank {:?}.", provider.bank_id());
            Ok(())
        }
        None => {
            anyhow::bail!("the memory engine did not accept the retain (see the log above)");
        }
    }
}

fn cmd_delete(provider: &MemoryWireProvider, id: &str) -> Result<()> {
    if provider.delete_memory_by_id(id) {
        println!("Deleted memory {id}.");
    } else {
        println!("Memory '{id}' not found in bank {:?}.", provider.bank_id());
    }
    Ok(())
}

fn cmd_stats(provider: &MemoryWireProvider) -> Result<()> {
    match provider.memory_bank_stats() {
        Some(stats) => {
            println!("Bank:    {:?}", provider.bank_id());
            println!("Memories: {}", stats.memories);
            println!("Tags:     {}", stats.tags);
            println!(
                "Oldest:   {}",
                stats.oldest.as_deref().unwrap_or("(empty bank)")
            );
            println!(
                "Newest:   {}",
                stats.newest.as_deref().unwrap_or("(empty bank)")
            );
            Ok(())
        }
        None => {
            anyhow::bail!("memory stats unavailable (the engine is degraded — see the log)");
        }
    }
}

fn cmd_import(provider: &MemoryWireProvider, source: &str) -> Result<()> {
    let path = std::path::Path::new(source);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Legacy MEMORY.md / USER.md: the engine's own importer ran at the
    // 11C3 upgrade; a file the user still holds is read directly —
    // bullets and prose lines become memories tagged 'legacy'. Anything
    // else is JSONL, one memory per line.
    let is_legacy = name == "MEMORY.md" || name == "USER.md";
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {source}"))?;
    let mut imported = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (content, tag): (String, &str) = if is_legacy {
            let body = trimmed.trim_start_matches('-').trim();
            if body.is_empty() {
                continue;
            }
            (body.to_string(), "legacy")
        } else {
            // JSONL: {"content": "..."} — anything else is stored verbatim.
            let c = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| {
                    v.get("content")
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| line.to_string());
            (c, "import")
        };
        if provider.retain_fact(&content, &[tag.to_string()]).is_some() {
            imported += 1;
        }
    }
    println!("Imported {imported} memor(y/ies) from {source}.");
    Ok(())
}

fn cmd_export(provider: &MemoryWireProvider, output: Option<String>) -> Result<()> {
    let path = output.unwrap_or_else(|| "memories.jsonl".to_string());
    let mut w = std::fs::File::create(&path).with_context(|| format!("creating {path}"))?;
    use std::io::Write;
    let rows = provider.list_page(usize::MAX, 0);
    for m in &rows {
        let obj = serde_json::json!({
            "id": m.id,
            "bank": m.bank_id,
            "created_at": m.created_at,
            "content": m.content,
        });
        writeln!(w, "{obj}")?;
    }
    println!("Exported {} memor(y/ies) to {path}.", rows.len());
    Ok(())
}

fn cmd_clear(provider: &MemoryWireProvider, confirm: bool) -> Result<()> {
    if !confirm {
        print!(
            "Delete EVERY memory in bank {:?}? This cannot be undone. Type 'yes' to confirm: ",
            provider.bank_id()
        );
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .context("reading confirmation")?;
        if answer.trim() != "yes" {
            println!("Aborted — nothing deleted.");
            return Ok(());
        }
    }
    let rows = provider.list_page(usize::MAX, 0);
    let mut deleted = 0usize;
    for m in &rows {
        if provider.delete_memory_by_id(&m.id) {
            deleted += 1;
        }
    }
    println!(
        "Deleted {deleted} memor(y/ies) from bank {:?}.",
        provider.bank_id()
    );
    Ok(())
}
