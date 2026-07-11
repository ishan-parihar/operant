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

/// `operant tui` subcommands. Combines read-only debug subcommands (that
/// simulate TUI overlays from the CLI) with action subcommands (that set
/// TUI state persistently, closing the TUI↔CLI parity gaps from the audit).
#[derive(Debug, Clone, Subcommand)]
pub enum TuiSubcommand {
    /// Read-only debug subcommands — simulate TUI overlays from the CLI.
    /// Each runs the same data-loading path the TUI uses, but prints to stdout.
    Debug {
        #[command(subcommand)]
        cmd: TuiDebugSubcommand,
    },

    /// Show or set the reasoning effort level (parity gap #5).
    /// `operant tui effort` shows the current level;
    /// `operant tui effort set high` sets it.
    Effort {
        /// Optional subcommand: 'set <level>'. If omitted, shows the current level.
        #[command(subcommand)]
        cmd: Option<EffortSubcommand>,
    },

    /// Show or set the active mode (parity gap #8).
    /// `operant tui mode` shows the current permission mode;
    /// `operant tui mode yolo` sets permission_mode=BypassPermissions;
    /// `operant tui mode plan` sets permission_mode=Plan;
    /// `operant tui mode default` sets permission_mode=Default.
    Mode {
        /// Mode to set: yolo | plan | default | accept-edits. If omitted, shows current.
        mode: Option<String>,
    },

    /// Show or set the output style (parity gap #8).
    /// `operant tui output-style` shows current;
    /// `operant tui output-style verbose` sets it.
    OutputStyle {
        /// Style to set: auto | stream | verbose. If omitted, shows current.
        style: Option<String>,
    },

    /// List or set the TUI theme (parity gap #10).
    /// `operant tui theme` lists available themes;
    /// `operant tui theme set dark` sets the theme.
    Theme {
        #[command(subcommand)]
        cmd: Option<ThemeSubcommand>,
    },

    /// Toggle vim mode in the TUI prompt input (parity gap #10).
    /// `operant tui vim on` enables; `operant tui vim off` disables;
    /// `operant tui vim` shows current state.
    Vim {
        /// on | off. If omitted, shows current state.
        state: Option<String>,
    },

    /// Open the user keybindings file in $EDITOR (parity gap #10).
    Keybindings,

    /// Show voice-mode status (parity gap #9).
    /// Voice mode can't be toggled from the CLI because it requires the TUI's
    /// audio recorder + crossterm event loop; this command surfaces the
    /// current availability so the user knows whether /voice will work.
    Voice,
}

#[derive(Debug, Clone, Subcommand)]
pub enum EffortSubcommand {
    /// Set the effort level.
    Set {
        /// Effort level: low | normal | high | max.
        level: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ThemeSubcommand {
    /// List available themes.
    List,
    /// Set the theme.
    Set {
        /// Theme name: dark | light | default | deuteranopia | <custom-name>.
        name: String,
    },
}

/// `operant tui debug` subcommands (read-only).
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

    /// Headless TUI simulator to replay a key sequence and assert correctness.
    Simulate {
        /// Keystroke sequence to replay (e.g. "hello\n/quit\n" or "<up><enter>").
        #[arg(long)]
        keys: String,

        /// Optional JSON output file path to write the simulation log.
        #[arg(long)]
        output: Option<std::path::PathBuf>,

        /// Optional state assertions to evaluate (e.g. "help_overlay.visible == true").
        #[arg(long)]
        assert: Option<String>,

        /// Optional path to dump the final rendered screen (one text row per line).
        #[arg(long)]
        dump_screen: Option<std::path::PathBuf>,

        /// Optional screen-content assertions, comma-separated
        /// (e.g. "contains:Help,not-contains:Error"). Matched against the
        /// full rendered screen text. Fails the run on mismatch.
        #[arg(long)]
        assert_screen: Option<String>,

        /// Optional path to a JSON file of mock agent events to inject
        /// instead of spawning a real network agent. Deterministic, offline.
        /// Format: a JSON array of tagged objects, e.g.
        /// [{"type":"content","text":"hi"},{"type":"done","text":"hi"}].
        #[arg(long)]
        agent_script: Option<std::path::PathBuf>,

        /// Terminal size as WxH (default 120x40). Reproduce layout/wrapping
        /// bugs at specific dimensions.
        #[arg(long)]
        size: Option<String>,

        /// Max frames before the simulation force-exits (default 100000).
        /// Guards against a scenario that never stops streaming.
        #[arg(long)]
        max_frames: Option<u64>,
    },
}

/// Entry point dispatch for `operant tui <subcommand>`.
pub async fn handle_tui_command(config: &AppConfig, cmd: TuiSubcommand) -> Result<()> {
    match cmd {
        TuiSubcommand::Debug { cmd } => handle_tui_debug_command(config, cmd).await,
        TuiSubcommand::Effort { cmd } => handle_effort(config, cmd).await,
        TuiSubcommand::Mode { mode } => handle_mode(config, mode).await,
        TuiSubcommand::OutputStyle { style } => handle_output_style(config, style).await,
        TuiSubcommand::Theme { cmd } => handle_theme(config, cmd).await,
        TuiSubcommand::Vim { state } => handle_vim(config, state).await,
        TuiSubcommand::Keybindings => handle_keybindings(config).await,
        TuiSubcommand::Voice => handle_voice(config).await,
    }
}

/// Entry point dispatch for `operant tui debug <subcommand>`.
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
        TuiDebugSubcommand::Simulate {
            keys,
            output,
            assert,
            dump_screen,
            assert_screen,
            agent_script,
            size,
            max_frames,
        } => {
            debug_simulate(
                config,
                keys,
                output,
                assert,
                dump_screen,
                assert_screen,
                agent_script,
                size,
                max_frames,
            )
            .await
        }
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
        "{:<3}  {:<24} {:<14} {:<8}  Description",
        "#", "Name", "Category", "Version"
    );
    println!("{}", "-".repeat(100));

    for (i, skill) in skills.iter().enumerate() {
        let desc = skill.description.chars().take(40).collect::<String>();
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
    println!("{:<3}  {:<8}  {:<24}  {:>8}", "#", "Status", "Name", "Size");
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
                let content_preview: String = m
                    .content
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect();
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
        "{:<3}  {:<24} {:<10} {:<8}  URL / Command",
        "#", "Name", "Type", "Enabled"
    );
    println!("{}", "-".repeat(90));

    for (i, server) in config.mcp.servers.iter().enumerate() {
        let url_or_cmd = server.url.clone().unwrap_or_else(|| {
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

    println!(
        "Showing last {} entries from {}:",
        last_n,
        stats_path.display()
    );
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
    println!(
        "Active provider: {}",
        provider.as_deref().unwrap_or("(unknown)")
    );
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
        println!(
            "No sessions found in database: {}",
            config.database_path.display()
        );
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
    println!("Use: operant tui debug slash-commands");
    println!("Source: crates/operant-cli/src/tui/app.rs::PROMPT_SLASH_COMMANDS");
    // (iter-154: hardcoded 50-command list deleted — was duplicating PROMPT_SLASH_COMMANDS)
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
            "anthropic",
            "openai",
            "google",
            "groq",
            "cerebras",
            "deepseek",
            "mistral",
            "xai",
            "openrouter",
            "github-copilot",
            "codex",
            "cohere",
            "perplexity",
            "togetherai",
            "together-ai",
            "deepinfra",
            "venice",
            "minimax",
            "sambanova",
            "nvidia",
            "moonshotai",
            "zhipuai",
            "siliconflow",
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

// ---------------------------------------------------------------------------
// Action subcommand handlers (parity-gap closures)
// ---------------------------------------------------------------------------

/// `operant tui effort` — show or set the reasoning effort level.
/// Stored in settings.json under `effort_level`.
async fn handle_effort(_config: &AppConfig, cmd: Option<EffortSubcommand>) -> Result<()> {
    let mut settings = load_settings();
    match cmd {
        None => {
            let cur = settings
                .effort_level
                .clone()
                .unwrap_or_else(|| "normal".to_string());
            println!("Current effort level: {}", cur);
            println!();
            println!("Set with: operant tui effort set low|normal|high|max");
            println!("  low    — fast, cheap, minimal reasoning");
            println!("  normal — balanced (default)");
            println!("  high   — more reasoning tokens");
            println!("  max    — maximum reasoning budget");
        }
        Some(EffortSubcommand::Set { level }) => {
            let lvl = level.to_lowercase();
            match lvl.as_str() {
                "low" | "normal" | "high" | "max" => {
                    settings.effort_level = Some(lvl.clone());
                    save_settings(&settings)?;
                    println!("Effort level set to: {}", lvl);
                }
                _ => {
                    anyhow::bail!(
                        "Invalid effort level '{}'. Must be one of: low, normal, high, max",
                        level
                    );
                }
            }
        }
    }
    Ok(())
}

/// `operant tui mode [yolo|plan|default|accept-edits]` — show or set
/// permission_mode. Closes the parity gap for /yolo + /plan.
async fn handle_mode(_config: &AppConfig, mode: Option<String>) -> Result<()> {
    let mut settings = load_settings();
    match mode {
        None => {
            let cur = format!("{:?}", settings.permission_mode);
            println!("Current permission mode: {}", cur);
            println!();
            println!("Set with: operant tui mode <mode>");
            println!("  yolo         — BypassPermissions (auto-approve everything)");
            println!("  plan         — Plan (agent proposes, doesn't execute)");
            println!("  default      — Default (prompt per tool)");
            println!("  accept-edits — AcceptEdits (auto-approve file edits)");
        }
        Some(m) => {
            let new_mode = match m.to_lowercase().as_str() {
                "yolo" | "bypass" | "bypasspermissions" => {
                    crate::tui::adapter_types::config::PermissionMode::BypassPermissions
                }
                "plan" => crate::tui::adapter_types::config::PermissionMode::Plan,
                "default" | "normal" => crate::tui::adapter_types::config::PermissionMode::Default,
                "accept-edits" | "acceptedits" | "accept_edits" => {
                    crate::tui::adapter_types::config::PermissionMode::AcceptEdits
                }
                _ => {
                    anyhow::bail!(
                        "Invalid mode '{}'. Must be one of: yolo, plan, default, accept-edits",
                        m
                    );
                }
            };
            settings.permission_mode = new_mode.clone();
            save_settings(&settings)?;
            println!("Permission mode set to: {:?}", new_mode);
        }
    }
    Ok(())
}

/// `operant tui output-style [auto|stream|verbose]` — show or set output_style.
async fn handle_output_style(_config: &AppConfig, style: Option<String>) -> Result<()> {
    let mut settings = load_settings();
    match style {
        None => {
            let cur = settings
                .output_style
                .clone()
                .unwrap_or_else(|| "auto".to_string());
            println!("Current output style: {}", cur);
            println!();
            println!("Set with: operant tui output-style <style>");
            println!("  auto    — stream when reasonable, verbose for long outputs");
            println!("  stream  — always stream");
            println!("  verbose — always show full output");
        }
        Some(s) => {
            let s_lower = s.to_lowercase();
            match s_lower.as_str() {
                "auto" | "stream" | "verbose" => {
                    settings.output_style = Some(s_lower.clone());
                    save_settings(&settings)?;
                    println!("Output style set to: {}", s_lower);
                }
                _ => {
                    anyhow::bail!(
                        "Invalid output style '{}'. Must be one of: auto, stream, verbose",
                        s
                    );
                }
            }
        }
    }
    Ok(())
}

/// `operant tui theme [list|set <name>]` — list or set the TUI theme.
async fn handle_theme(_config: &AppConfig, cmd: Option<ThemeSubcommand>) -> Result<()> {
    let mut settings = load_settings();
    match cmd {
        None => {
            let cur = format!("{:?}", settings.theme);
            println!("Current theme: {}", cur);
            println!();
            println!("Available themes:");
            println!("  dark         — dark background (default)");
            println!("  light        — light background");
            println!("  default      — terminal default");
            println!("  deuteranopia — color-blind friendly");
            println!("  <custom>     — any name; the TUI will look for a matching palette");
            println!();
            println!("Set with: operant tui theme set <name>");
            println!("List with: operant tui theme list");
        }
        Some(ThemeSubcommand::List) => {
            println!("Available themes:");
            println!("  dark");
            println!("  light");
            println!("  default");
            println!("  deuteranopia");
        }
        Some(ThemeSubcommand::Set { name }) => {
            let theme = match name.to_lowercase().as_str() {
                "dark" => crate::tui::adapter_types::config::Theme::Dark,
                "light" => crate::tui::adapter_types::config::Theme::Light,
                "default" => crate::tui::adapter_types::config::Theme::Default,
                "deuteranopia" => crate::tui::adapter_types::config::Theme::Deuteranopia,
                other => crate::tui::adapter_types::config::Theme::Custom(other.to_string()),
            };
            settings.theme = theme.clone();
            save_settings(&settings)?;
            println!("Theme set to: {:?}", theme);
        }
    }
    Ok(())
}

/// `operant tui vim [on|off]` — toggle vim mode.
async fn handle_vim(_config: &AppConfig, state: Option<String>) -> Result<()> {
    let mut settings = load_settings();
    match state {
        None => {
            println!(
                "Vim mode: {}",
                if settings.vim_enabled { "on" } else { "off" }
            );
            println!();
            println!("Set with: operant tui vim on | operant tui vim off");
        }
        Some(s) => match s.to_lowercase().as_str() {
            "on" | "true" | "1" | "yes" => {
                settings.vim_enabled = true;
                save_settings(&settings)?;
                println!("Vim mode enabled.");
            }
            "off" | "false" | "0" | "no" => {
                settings.vim_enabled = false;
                save_settings(&settings)?;
                println!("Vim mode disabled.");
            }
            _ => {
                anyhow::bail!("Invalid state '{}'. Must be one of: on, off", s);
            }
        },
    }
    Ok(())
}

/// `operant tui keybindings` — open the user keybindings file in $EDITOR.
async fn handle_keybindings(_config: &AppConfig) -> Result<()> {
    let kb_path =
        crate::tui::adapter_types::config::Settings::config_dir().join("keybindings.json");
    if !kb_path.exists() {
        // Write a default empty keybindings file.
        std::fs::create_dir_all(kb_path.parent().unwrap())?;
        std::fs::write(
            &kb_path,
            "{\n  \"//\": \"User keybindings. See docs for the schema.\"\n}\n",
        )?;
        println!("Created default keybindings file: {}", kb_path.display());
    }

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    println!("Opening {} in {}…", kb_path.display(), editor);
    let status = std::process::Command::new(&editor).arg(&kb_path).status()?;
    if !status.success() {
        anyhow::bail!("Editor exited with non-zero status");
    }
    Ok(())
}

/// `operant tui voice` — show voice-mode availability.
/// Voice mode can't be toggled from the CLI (requires the TUI's audio
/// recorder + crossterm event loop), but we can surface whether the
/// recorder is available so the user knows whether /voice will work.
async fn handle_voice(_config: &AppConfig) -> Result<()> {
    println!("=== Voice Mode Status ===");
    println!();

    // Check if the voice recorder feature is compiled in.
    let recorder = crate::tui::adapter_types::voice::global_voice_recorder();
    let is_available = if let Ok(r) = recorder.lock() {
        r.is_available()
    } else {
        false
    };

    println!(
        "Voice recorder available: {}",
        if is_available { "yes" } else { "no" }
    );
    if !is_available {
        println!();
        println!("Voice mode requires:");
        println!("  - A working microphone (arecord / rec / ffmpeg)");
        println!("  - The 'voice' cargo feature compiled in");
        println!("  - An audio output device for TTS playback");
    } else {
        println!();
        println!("To enable voice mode, run `operant chat` and press /voice.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Settings load/save helpers
// ---------------------------------------------------------------------------

fn load_settings() -> crate::tui::adapter_types::config::Settings {
    crate::tui::adapter_types::config::Settings::load_sync().unwrap_or_default()
}

fn save_settings(settings: &crate::tui::adapter_types::config::Settings) -> Result<()> {
    settings.save_sync()
}

fn parse_key_sequence(seq: &str) -> Vec<crossterm::event::KeyEvent> {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    let mut events = Vec::new();
    let chars: Vec<char> = seq.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(close_idx) = chars[i..].iter().position(|&c| c == '>') {
                let name: String = chars[i + 1..i + close_idx].iter().collect();
                let lower = name.to_lowercase();
                let mut modifiers = KeyModifiers::NONE;
                let mut parsed = true;
                let code = match lower.as_str() {
                    "enter" => KeyCode::Enter,
                    "esc" | "escape" => KeyCode::Esc,
                    "tab" => KeyCode::Tab,
                    "up" => KeyCode::Up,
                    "down" => KeyCode::Down,
                    "left" => KeyCode::Left,
                    "right" => KeyCode::Right,
                    "backspace" | "bs" => KeyCode::Backspace,
                    "ctrl+a" => {
                        modifiers.insert(KeyModifiers::CONTROL);
                        KeyCode::Char('a')
                    }
                    "ctrl+c" => {
                        modifiers.insert(KeyModifiers::CONTROL);
                        KeyCode::Char('c')
                    }
                    "ctrl+t" => {
                        modifiers.insert(KeyModifiers::CONTROL);
                        KeyCode::Char('t')
                    }
                    "ctrl+r" => {
                        modifiers.insert(KeyModifiers::CONTROL);
                        KeyCode::Char('r')
                    }
                    "shift+tab" => {
                        modifiers.insert(KeyModifiers::SHIFT);
                        KeyCode::BackTab
                    }
                    _ => {
                        parsed = false;
                        KeyCode::Null
                    }
                };
                if parsed {
                    events.push(KeyEvent {
                        code,
                        modifiers,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    });
                    i += close_idx + 1;
                    continue;
                }
            }
        }

        // Handle escaped newlines or tabs
        let (code, modifiers) = if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            let res = match next {
                'n' => (KeyCode::Enter, KeyModifiers::NONE),
                't' => (KeyCode::Tab, KeyModifiers::NONE),
                '\\' => (KeyCode::Char('\\'), KeyModifiers::NONE),
                _ => (KeyCode::Char('\\'), KeyModifiers::NONE),
            };
            if next == 'n' || next == 't' || next == '\\' {
                i += 1;
            }
            res
        } else {
            (KeyCode::Char(chars[i]), KeyModifiers::NONE)
        };
        events.push(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        i += 1;
    }
    events
}

/// Evaluate comma-separated state assertions against `App::debug_snapshot()`.
/// Each clause is `path OP value`, where OP is `==`, `!=`, or `contains` and
/// `path` is a dot-path into the snapshot JSON (e.g. `overlays.model_picker`,
/// `messages`, `model`). Values are matched against booleans, numbers, and
/// strings. Legacy `<name>.visible` keys are auto-mapped to `overlays.<name>`.
fn evaluate_assertions(app: &crate::tui::app::App, assertions_str: &str) -> Result<()> {
    let snapshot = app.debug_snapshot();
    for assertion in assertions_str.split(',') {
        let assertion = assertion.trim();
        if assertion.is_empty() {
            continue;
        }

        // Detect operator. `contains` is whitespace-delimited to avoid
        // colliding with substrings; `==`/`!=` are symbolic.
        let (key, op, val_raw) = if let Some((k, v)) = assertion.split_once("==") {
            (k.trim(), "==", v.trim())
        } else if let Some((k, v)) = assertion.split_once("!=") {
            (k.trim(), "!=", v.trim())
        } else if let Some((k, v)) = assertion.split_once(" contains ") {
            (k.trim(), "contains", v.trim())
        } else {
            anyhow::bail!(
                "Invalid assertion '{}': expected 'path == value', 'path != value', or 'path contains text'.",
                assertion
            );
        };

        // Strip outer quotes from the value if present.
        let val_str = val_raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| {
                val_raw
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
            })
            .unwrap_or(val_raw);

        // Legacy compatibility: `foo.visible` → `overlays.foo`.
        let path = key
            .strip_suffix(".visible")
            .map(|base| format!("overlays.{base}"))
            .unwrap_or_else(|| key.to_string());

        // Navigate the snapshot by dot-path.
        let mut node = &snapshot;
        for seg in path.split('.') {
            node = node
                .get(seg)
                .ok_or_else(|| anyhow::anyhow!("Unknown assertion path: '{}'", key))?;
        }

        let actual_display = match node {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };

        let matched = match op {
            "contains" => actual_display.contains(val_str),
            "==" | "!=" => {
                let eq = match node {
                    serde_json::Value::Bool(b) => {
                        val_str.parse::<bool>().map(|v| v == *b).unwrap_or(false)
                    }
                    serde_json::Value::Number(n) => val_str
                        .parse::<f64>()
                        .map(|v| n.as_f64() == Some(v))
                        .unwrap_or(false),
                    serde_json::Value::String(s) => s == val_str,
                    serde_json::Value::Null => val_str == "null",
                    _ => actual_display == val_str,
                };
                if op == "==" {
                    eq
                } else {
                    !eq
                }
            }
            _ => unreachable!(),
        };

        if !matched {
            anyhow::bail!(
                "Assertion failed: {} {} {} (actual: {})",
                key,
                op,
                val_str,
                actual_display
            );
        }
        println!("  ✓ {} {} {}", key, op, val_str);
    }
    Ok(())
}

/// A serde-friendly mock agent event for the headless simulator. Maps to a
/// subset of `operant_core::agent::AgentEvent` — enough to drive the TUI's
/// streaming/tool/done/error rendering deterministically offline, without
/// adding serde derives to the core event type.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MockAgentEvent {
    Thinking {
        content: String,
    },
    Reasoning {
        text: String,
    },
    Content {
        text: String,
    },
    ToolStart {
        id: String,
        name: String,
        #[serde(default)]
        arguments: String,
    },
    ToolComplete {
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        output: String,
    },
    ToolError {
        id: String,
        #[serde(default)]
        name: String,
        error: String,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    Done {
        #[serde(default)]
        text: String,
        #[serde(default)]
        reasoning: Option<String>,
    },
    Error {
        error: String,
    },
}

impl MockAgentEvent {
    fn into_agent_event(self) -> operant_core::agent::AgentEvent {
        use operant_core::agent::AgentEvent as AE;
        match self {
            MockAgentEvent::Thinking { content } => AE::Thinking { content },
            MockAgentEvent::Reasoning { text } => AE::Reasoning { text },
            MockAgentEvent::Content { text } => AE::Content { text },
            MockAgentEvent::ToolStart {
                id,
                name,
                arguments,
            } => AE::ToolStart {
                tool_call_id: id,
                name,
                arguments,
            },
            MockAgentEvent::ToolComplete { id, name, output } => AE::ToolComplete {
                result: operant_core::tools::ToolResult {
                    tool_call_id: id,
                    name,
                    success: true,
                    content: output,
                    error: None,
                },
            },
            MockAgentEvent::ToolError { id, name, error } => AE::ToolError {
                tool_call_id: id,
                name,
                error,
            },
            MockAgentEvent::Usage {
                input_tokens,
                output_tokens,
            } => AE::Usage {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
            },
            MockAgentEvent::Done { text, reasoning } => {
                let mut msg = operant_core::client::Message::assistant(text);
                msg.reasoning = reasoning;
                AE::Done { message: msg }
            }
            MockAgentEvent::Error { error } => AE::Error { error },
        }
    }
}

async fn debug_simulate(
    config: &AppConfig,
    keys: String,
    output: Option<std::path::PathBuf>,
    assert_str: Option<String>,
    dump_screen: Option<std::path::PathBuf>,
    assert_screen: Option<String>,
    agent_script: Option<std::path::PathBuf>,
    size: Option<String>,
    max_frames: Option<u64>,
) -> Result<()> {
    use crate::tui::adapter_types::{LaunchMode, TuiApp};
    use crate::tui::debug::TuiEvent;

    println!("Starting headless TUI simulation...");
    let parsed_keys = parse_key_sequence(&keys);
    println!("Parsed {} key events.", parsed_keys.len());

    // Parse --size WxH (default 120x40).
    let dims = match size.as_deref() {
        None => (120u16, 40u16),
        Some(s) => {
            let (w, h) = s.split_once(['x', 'X']).ok_or_else(|| {
                anyhow::anyhow!("Invalid --size '{}': expected WxH, e.g. 80x24", s)
            })?;
            (
                w.trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("Invalid width in --size '{}'", s))?,
                h.trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("Invalid height in --size '{}'", s))?,
            )
        }
    };
    let frame_cap = Some(max_frames.unwrap_or(100_000));

    let script = if let Some(ref path) = agent_script {
        let raw = std::fs::read_to_string(path)?;
        let mock: Vec<MockAgentEvent> = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("Failed to parse agent script {:?}: {}", path, e))?;
        println!("Injecting {} mock agent events.", mock.len());
        Some(
            mock.into_iter()
                .map(MockAgentEvent::into_agent_event)
                .collect(),
        )
    } else {
        None
    };

    let tui_app = TuiApp::enter(config.clone(), None, LaunchMode::Landing, true).await?;
    let (events, app, screen) = tui_app
        .run_headless(parsed_keys, script, dims, frame_cap)
        .await?;

    println!("Simulation completed. Analyzing events...");
    let mut has_errors = false;
    for event in &events {
        if let TuiEvent::Error {
            source, message, ..
        } = event
        {
            eprintln!("TUI ERROR [{}]: {}", source, message);
            has_errors = true;
        }
    }

    if let Some(ref out_path) = output {
        let json = serde_json::to_string_pretty(&events)?;
        std::fs::write(out_path, json)?;
        println!("Saved simulation event log to {:?}", out_path);
    }

    if let Some(ref screen_path) = dump_screen {
        std::fs::write(screen_path, screen.join("\n"))?;
        println!("Saved final rendered screen to {:?}", screen_path);
    }

    if has_errors {
        anyhow::bail!("Simulation failed: Errors detected in TUI event log.");
    }

    if let Some(ref assert_val) = assert_str {
        println!("Evaluating state assertions: {}", assert_val);
        evaluate_assertions(&app, assert_val)?;
    }

    if let Some(ref screen_asserts) = assert_screen {
        println!("Evaluating screen assertions: {}", screen_asserts);
        evaluate_screen_assertions(&screen, screen_asserts)?;
    }

    println!("Simulation succeeded without errors.");
    Ok(())
}

/// Evaluate comma-separated screen-content assertions against the rendered
/// screen text. Each clause is `contains:TEXT` or `not-contains:TEXT`.
/// Returns an error (failing the run) on the first mismatch.
fn evaluate_screen_assertions(screen: &[String], assertions_str: &str) -> Result<()> {
    let haystack = screen.join("\n");
    for clause in assertions_str.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }
        let (negate, needle) = if let Some(rest) = clause.strip_prefix("not-contains:") {
            (true, rest)
        } else if let Some(rest) = clause.strip_prefix("contains:") {
            (false, rest)
        } else {
            anyhow::bail!(
                "Invalid screen assertion '{}': expected 'contains:TEXT' or 'not-contains:TEXT'",
                clause
            );
        };
        let present = haystack.contains(needle);
        if negate && present {
            anyhow::bail!(
                "Screen assertion failed: expected NOT to contain '{}'",
                needle
            );
        }
        if !negate && !present {
            anyhow::bail!("Screen assertion failed: expected to contain '{}'", needle);
        }
        println!("  ✓ {}", clause);
    }
    Ok(())
}
