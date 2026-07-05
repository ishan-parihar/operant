//! `operant tui debug` — CLI-driven TUI overlay simulation.
//!
//! Every TUI overlay has a data-loading path (SkillManager::load_all,
//! MemoryStore::read_memories, plugins_dir scan, etc.) that's normally
//! invoked when the user opens the overlay in the TUI. This subcommand
//! exposes those same data paths from the CLI so the user can:
//!
//!   1. Verify each overlay's data loads correctly without entering the TUI.
//!   2. Debug a broken overlay (e.g. /skills shows nothing) by running the
//!      same load path from the shell and inspecting the raw output.
//!   3. Script TUI state inspection in CI or automation.
//!
//! The subcommand does NOT render anything — it prints plain-text tables
//! and JSON. For the rendered TUI, use `operant chat` and open the overlay
//! interactively.
//!
//! Subcommands mirror the TUI overlays 1:1:
//!   `operant tui debug skills`        — same as /skills overlay data
//!   `operant tui debug plugins`       — same as /plugins overlay data
//!   `operant tui debug journey`       — same as /journey overlay data
//!   `operant tui debug mcp`           — same as /mcp overlay data
//!   `operant tui debug stats`         — same as /stats overlay data
//!   `operant tui debug context`       — same as /context overlay data
//!   `operant tui debug sessions`      — same as /resume overlay data
//!   `operant tui debug banner`        — same as the ASCII banner render
//!   `operant tui debug slash-commands`— list every intercepted slash command
//!   `operant tui debug state`         — dump the App struct's persistent state
//!   `operant tui debug cost`          — same as /cost / /heapdump / /mem data
//!
//! Each subcommand exits 0 on success, 1 on data-load failure (with a
//! diagnostic message), and 2 on argument-parse failure (handled by clap).

use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;

/// `operant tui debug` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum TuiDebugSubcommand {
    /// List installed skills (same data as the /skills overlay).
    Skills,
    /// List installed plugins + enabled state (same data as /plugins overlay).
    Plugins,
    /// Show skills + memories side-by-side (same data as /journey overlay).
    Journey,
    /// List configured MCP servers + status (same data as /mcp overlay).
    Mcp,
    /// Show token-usage stats (same data as /stats overlay).
    Stats,
    /// Show context-window + rate-limit usage (same data as /context overlay).
    Context,
    /// List recent sessions (same data as /resume overlay).
    Sessions,
    /// Render the OPERANT ASCII banner to stdout.
    Banner,
    /// List every intercepted slash command + what it does.
    SlashCommands,
    /// Dump the App struct's persistent state (settings.json + auth.json).
    State,
    /// Show cost / token / turn-count summary (same data as /cost /heapdump /mem).
    Cost,
}

/// Entry point dispatch.
pub async fn handle_tui_debug_command(config: &AppConfig, cmd: TuiDebugSubcommand) -> Result<()> {
    match cmd {
        TuiDebugSubcommand::Skills => debug_skills(config).await,
        TuiDebugSubcommand::Plugins => debug_plugins(config).await,
        TuiDebugSubcommand::Journey => debug_journey(config).await,
        TuiDebugSubcommand::Mcp => debug_mcp(config).await,
        TuiDebugSubcommand::Stats => debug_stats(config).await,
        TuiDebugSubcommand::Context => debug_context(config).await,
        TuiDebugSubcommand::Sessions => debug_sessions(config).await,
        TuiDebugSubcommand::Banner => debug_banner(config).await,
        TuiDebugSubcommand::SlashCommands => debug_slash_commands(config).await,
        TuiDebugSubcommand::State => debug_state(config).await,
        TuiDebugSubcommand::Cost => debug_cost(config).await,
    }
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

async fn debug_skills(config: &AppConfig) -> Result<()> {
    let skills_dir = config.skills.root_dir.clone();
    let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
    let skills = mgr.load_all()?;

    if skills.is_empty() {
        println!("No skills installed.");
        println!("Skills directory: {}", config.skills.root_dir.display());
        println!("Install one with: operant skills install <path-or-url>");
        return Ok(());
    }

    println!("Installed skills ({}):", skills.len());
    println!("Directory: {}", config.skills.root_dir.display());
    println!();
    println!(
        "{:<3}  {:<24} {:<14} {:<8}  {}",
        "#", "Name", "Category", "Version", "Description"
    );
    println!("{}", "-".repeat(100));

    for (i, skill) in skills.iter().enumerate() {
        let desc = skill
            .description
            .chars()
            .take(40)
            .collect::<String>();
        println!(
            "{:<3}  {:<24} {:<14} {:<8}  {}",
            i + 1,
            truncate(&skill.name, 24),
            truncate(&skill.category, 14),
            truncate(&skill.version, 8),
            desc,
        );
    }

    Ok(())
}

async fn debug_plugins(config: &AppConfig) -> Result<()> {
    let plugins_dir = crate::cmd_plugins::plugins_dir(config)?;

    if !plugins_dir.exists() {
        println!("No plugins directory: {}", plugins_dir.display());
        return Ok(());
    }

    let entries = std::fs::read_dir(&plugins_dir)?;
    let mut found: Vec<(String, bool, u64)> = Vec::new();

    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".enabled") {
            continue;
        }
        let marker = plugins_dir.join(format!("{}.enabled", name));
        let enabled = marker.exists();
        let size = dir_size(&entry.path());
        found.push((name, enabled, size));
    }

    found.sort_by(|a, b| a.0.cmp(&b.0));

    if found.is_empty() {
        println!("No plugins installed.");
        println!("Plugins directory: {}", plugins_dir.display());
        return Ok(());
    }

    println!("Installed plugins ({}):", found.len());
    println!("Directory: {}", plugins_dir.display());
    println!();
    println!(
        "{:<3}  {:<8}  {:<24}  {:>8}",
        "#", "Status", "Name", "Size"
    );
    println!("{}", "-".repeat(60));

    for (i, (name, enabled, size)) in found.iter().enumerate() {
        let status = if *enabled { "enabled" } else { "disabled" };
        println!(
            "{:<3}  {:<8}  {:<24}  {:>8}",
            i + 1,
            status,
            truncate(name, 24),
            format_size(*size),
        );
    }

    Ok(())
}

async fn debug_journey(config: &AppConfig) -> Result<()> {
    println!("=== Journey: Skills + Memories ===");
    println!();

    // Skills column.
    println!("--- Skills ---");
    let mut skills_mgr = operant_core::skills::SkillManager::new(config.skills.root_dir.clone());
    match skills_mgr.load_all() {
        Ok(skills) if skills.is_empty() => {
            println!("  (no skills installed)");
        }
        Ok(skills) => {
            for s in &skills {
                println!(
                    "  {:<24} {:<14} v{}",
                    truncate(&s.name, 24),
                    truncate(&s.category, 14),
                    s.version,
                );
            }
        }
        Err(e) => println!("  Error loading skills: {}", e),
    }

    println!();
    println!("--- Memories ---");
    let mem_dir = operant_core::platform::operant_home().join("memory");
    let store = operant_core::memory::MemoryStore::new(mem_dir.clone());
    match store.read_memories() {
        Ok(map) if map.is_empty() => {
            println!("  (no memories stored)");
            println!("  Memory dir: {}", mem_dir.display());
        }
        Ok(map) => {
            let mut blocks: Vec<_> = map.into_values().collect();
            blocks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            for m in &blocks {
                let content_preview: String = m.content.lines().next().unwrap_or("").chars().take(50).collect();
                println!(
                    "  [{:>3}] {:<10} {:<14} {}",
                    m.importance,
                    truncate(&m.block_type, 10),
                    truncate(&m.id, 14),
                    content_preview,
                );
            }
        }
        Err(e) => println!("  Error loading memories: {}", e),
    }

    Ok(())
}

async fn debug_mcp(config: &AppConfig) -> Result<()> {
    println!("=== MCP Servers ===");
    println!();

    if config.mcp.servers.is_empty() {
        println!("No MCP servers configured.");
        println!("Configure one in operant.toml under [mcp.servers.<name>].");
        return Ok(());
    }

    println!(
        "{:<3}  {:<24} {:<10} {:<8}  {}",
        "#", "Name", "Type", "Enabled", "URL / Command"
    );
    println!("{}", "-".repeat(90));

    for (i, server) in config.mcp.servers.iter().enumerate() {
        let url_or_cmd = server
            .url
            .clone()
            .unwrap_or_else(|| {
                server
                    .command
                    .clone()
                    .unwrap_or_else(|| "(none)".to_string())
            });
        println!(
            "{:<3}  {:<24} {:<10} {:<8}  {}",
            i + 1,
            truncate(&server.name, 24),
            format!("{:?}", server.transport),
            if server.enabled { "yes" } else { "no" },
            truncate(&url_or_cmd, 50),
        );
    }

    Ok(())
}

async fn debug_stats(_config: &AppConfig) -> Result<()> {
    println!("=== Token Usage Stats ===");
    println!();

    let stats_path = operant_core::platform::operant_home().join("stats.jsonl");
    if !stats_path.exists() {
        println!("No stats file: {}", stats_path.display());
        println!("Stats accumulate as you use operant.");
        return Ok(());
    }

    // Read the last 10 lines of the stats log.
    let content = std::fs::read_to_string(&stats_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let last_n = lines.len().min(10);
    let start = lines.len().saturating_sub(last_n);

    println!("Showing last {} entries from {}:", last_n, stats_path.display());
    println!();
    for line in &lines[start..] {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let ts = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("?");
            let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("?");
            let in_t = v.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            let out_t = v.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            let cost = v.get("cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0);
            println!(
                "  {}  {:<30} in={:<6} out={:<6}  ${:.4}",
                ts,
                truncate(model, 30),
                in_t,
                out_t,
                cost,
            );
        }
    }

    Ok(())
}

async fn debug_context(config: &AppConfig) -> Result<()> {
    println!("=== Context Window + Rate Limits ===");
    println!();

    let model = &config.agent.model;
    let provider = infer_provider_from_model(model);
    println!("Active model:    {}", model);
    println!("Active provider: {}", provider.as_deref().unwrap_or("(unknown)"));
    println!();

    // Context window size — read from the settings.json if present.
    let settings_path = operant_core::platform::operant_home().join("settings.json");
    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(ctx) = v.get("context_window_size").and_then(|c| c.as_u64()) {
                println!("Configured context window: {} tokens", ctx);
            }
        }
    }

    // The live context_used_tokens + rate_limit_5h_pct + rate_limit_7day_pct
    // are in-memory state in the App; they're not persisted. To get real
    // numbers, run `operant chat` and open /context in the TUI.
    println!();
    println!("Live context-used / rate-limit percentages are in-memory only.");
    println!("Run `operant chat` and open /context in the TUI for live numbers.");

    Ok(())
}

async fn debug_sessions(config: &AppConfig) -> Result<()> {
    println!("=== Recent Sessions ===");
    println!();

    let db = operant_core::database::Database::init(config.database_path.clone())?;
    let sessions = db.list_sessions(20)?;

    if sessions.is_empty() {
        println!("No sessions found in database: {}", config.database_path.display());
        return Ok(());
    }

    println!(
        "{:<3}  {:<36} {:<28} {:<20} {:>8}",
        "#", "Session ID", "Title", "Updated", "Messages"
    );
    println!("{}", "-".repeat(100));

    for (i, s) in sessions.iter().enumerate() {
        let title = s.title.as_deref().unwrap_or("(untitled)");
        println!(
            "{:<3}  {:<36} {:<28} {:<20} {:>8}",
            i + 1,
            truncate(&s.id, 36),
            truncate(title, 28),
            truncate(&s.updated_at, 20),
            s.message_count,
        );
    }

    Ok(())
}

async fn debug_banner(_config: &AppConfig) -> Result<()> {
    println!("=== OPERANT ASCII Banner ===");
    println!();
    // Print the full art at 80 cols (terminal width doesn't matter for stdout).
    for line in crate::tui::banner::FULL_ART.iter() {
        println!("{}", line);
    }
    println!("                    v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

async fn debug_slash_commands(_config: &AppConfig) -> Result<()> {
    println!("=== Intercepted Slash Commands ===");
    println!();
    println!("These commands are intercepted by the TUI's intercept_slash_command");
    println!("(app.rs ~line 1974). Commands NOT in this list fall through to the");
    println!("basic command registry and print a one-line help text.");
    println!();

    let commands: &[(&str, &str)] = &[
        ("config / settings", "Open the settings screen"),
        ("theme", "Open the theme picker"),
        ("stats", "Open the 4-tab stats dialog"),
        ("mcp", "Open the MCP server view"),
        ("agents", "Open the subagent spawn-tree view"),
        ("diff / review", "Open the diff viewer (git diff)"),
        ("changes", "Open the diff viewer (per-turn)"),
        ("search / find", "Open the conversation search overlay"),
        ("survey / feedback", "Open the feedback survey"),
        ("memory", "Open the AGENTS.md memory file selector"),
        ("skills", "Open the skills browser (iter-75)"),
        ("plugins", "Open the plugins hub (iter-76)"),
        ("journey", "Open the skills+memories journey view (iter-79)"),
        ("hooks", "Open the hooks config menu"),
        ("import-config", "Open the import-config picker"),
        ("connect", "Open the connect-a-provider dialog"),
        ("model", "Open the model picker for the active provider"),
        ("clear", "Clear the transcript"),
        ("vim", "Toggle vim mode in the prompt input"),
        ("fast", "Toggle fast mode (low effort)"),
        ("plan", "Toggle plan mode"),
        ("copy", "Copy last assistant message to clipboard"),
        ("output-style", "Cycle output style (auto/stream/verbose)"),
        ("effort", "Open the effort picker"),
        ("voice", "Toggle voice mode"),
        ("cost", "Show cost summary"),
        ("rewind", "Open the rewind flow"),
        ("export", "Open the export dialog"),
        ("context", "Open the context-viz overlay"),
        ("rename", "Rename the current session"),
        ("keybindings", "Open the keybindings file"),
        ("help", "Toggle the help overlay"),
        // iter-77 backfill:
        ("yolo", "Toggle bypass-permissions mode"),
        ("busy", "Toggle auto-compact"),
        ("verbose", "Cycle output style"),
        ("reasoning", "Show reasoning-stream status"),
        ("personality", "Show current personality"),
        ("steer", "Show steer-mode hint"),
        ("queue", "Show queued messages"),
        ("background", "Show background-task hint"),
        ("rollback", "Open diff viewer (turn mode)"),
        ("reload / reload-mcp / reload-skills", "Show reload hint"),
        ("browser", "Show browser-backend info"),
        ("indicator / statusbar", "Toggle status bar"),
        ("mouse", "Show mouse-capture info"),
        ("terminal-setup", "Show terminal-setup info"),
        ("redraw", "Force a full redraw"),
        ("billing / credits", "Show BYOK billing info"),
        ("update", "Show update hint"),
        ("heapdump / mem", "Show debug snapshot"),
        ("pet", "Easter-egg: trigger Rustle pose"),
        ("skin", "Alias for /theme"),
        ("replay / replay-diff", "Show planned-overlay status"),
        ("setup", "Suspend TUI + run operant setup wizard (iter-80)"),
    ];

    for (cmd, desc) in commands {
        println!("  /{:<32} {}", cmd, desc);
    }

    Ok(())
}

async fn debug_state(_config: &AppConfig) -> Result<()> {
    println!("=== TUI Persistent State ===");
    println!();

    let home = operant_core::platform::operant_home();
    println!("Operant home: {}", home.display());
    println!();

    // settings.json
    let settings_path = home.join("settings.json");
    println!("--- settings.json ---");
    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        println!("{}", content);
    } else {
        println!("(does not exist — defaults will be used)");
    }
    println!();

    // auth.json
    let auth_path = home.join("auth.json");
    println!("--- auth.json ---");
    if auth_path.exists() {
        let content = std::fs::read_to_string(&auth_path)?;
        // Mask API keys before printing.
        let masked = mask_api_keys(&content);
        println!("{}", masked);
    } else {
        println!("(does not exist — no credentials stored)");
    }

    Ok(())
}

async fn debug_cost(_config: &AppConfig) -> Result<()> {
    println!("=== Cost / Token / Turn Summary ===");
    println!();

    let stats_path = operant_core::platform::operant_home().join("stats.jsonl");
    if !stats_path.exists() {
        println!("No stats file. Cost data accumulates as you use operant.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&stats_path)?;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cost: f64 = 0.0;
    let mut turn_count: u64 = 0;

    for line in content.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            total_input += v.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            total_output += v.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            total_cost += v.get("cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0);
            turn_count += 1;
        }
    }

    println!("Total turns:        {}", turn_count);
    println!("Total input tokens: {}", total_input);
    println!("Total output tokens: {}", total_output);
    println!("Total tokens:       {}", total_input + total_output);
    println!("Total cost:         ${:.4}", total_cost);

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> String {
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

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&p) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_file() {
                total += meta.len();
            } else if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    total
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    if bytes == 0 {
        return "0B".to_string();
    }
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{}B", bytes)
    } else {
        format!("{:.0}{}", size, UNITS[unit_idx])
    }
}

fn infer_provider_from_model(model: &str) -> Option<String> {
    if model == "free/auto" || model.starts_with("free/") || model.starts_with("zen/") {
        return Some("free".to_string());
    }
    if let Some((provider, _)) = model.split_once('/') {
        let known = [
            "anthropic", "openai", "google", "groq", "cerebras", "deepseek", "mistral",
            "xai", "openrouter", "github-copilot", "codex", "cohere", "perplexity",
            "togetherai", "together-ai", "deepinfra", "venice", "minimax", "sambanova",
            "nvidia", "moonshotai", "zhipuai", "siliconflow",
        ];
        if known.contains(&provider) {
            return Some(provider.to_string());
        }
    }
    None
}

/// Mask API keys in a JSON string before printing. Replaces the value of any
/// key named `api_key`, `token`, `secret`, `password`, etc. with `***`.
fn mask_api_keys(s: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)("(?:api_key|token|secret|password|api_token|access_token|refresh_token)"\s*:\s*")([^"]+)(")"#)
            .expect("Invalid mask regex")
    });
    KEY_RE.replace_all(s, r"${1}***${3}").to_string()
}
