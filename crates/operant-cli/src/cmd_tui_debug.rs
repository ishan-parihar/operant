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
//! Most subcommands print plain-text tables and JSON — they inspect the TUI's
//! *data* paths, not its pixels. One does render: `operant tui debug simulate`
//! drives the real `App::run` loop against a ratatui `TestBackend` and can
//! assert on, diff against a committed golden screen, and capture per-frame
//! text. For the interactive TUI itself, use `operant chat`.
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
        /// Optional subcommand: 'set `<level>`'. If omitted, shows the current level.
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
        /// Theme name: dark | light | default | deuteranopia | `<custom-name>`.
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
        /// Keystroke sequence to replay (e.g. "hello\n/quit\n" or `"<up><enter>"`).
        #[arg(long)]
        keys: String,

        /// Optional mouse event sequence to replay (e.g. "<left,10,10><drag,15,10><release,15,10>").
        /// Format: `<action,x,y>` where action is left/right/middle/up/drag/scroll_up/scroll_down.
        #[arg(long, default_value = "")]
        mouse: String,

        /// Optional JSON output file path to write the simulation log.
        #[arg(long)]
        output: Option<std::path::PathBuf>,

        /// Optional state assertions to evaluate (e.g. "help_overlay.visible == true").
        #[arg(long)]
        assert: Option<String>,

        /// Optional path to dump the final rendered screen (one text row per line).
        #[arg(long)]
        dump_screen: Option<std::path::PathBuf>,

        /// Optional path to dump the final rendered screen as a per-cell
        /// colour/modifier grid (`operant-style-v1` format: a header line
        /// then `height` lines of `width` `<fg>/<bg>/<mods>@<symbol>` tokens).
        /// A verbatim projection of the TestBackend buffer — nothing is
        /// normalised or collapsed, because a colour regression is invisible
        /// in `--dump-screen`. See tui_scenarios/schema.json →
        /// `baseline_formats.style`.
        #[arg(long)]
        dump_style: Option<std::path::PathBuf>,

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

        /// Path to a committed golden screen. After the run, the rendered
        /// screen is diffed against this file: match prints a confirmation
        /// and exits 0, drift prints a unified diff to stderr and exits
        /// non-zero, and a *missing* file is bootstrapped (created, exit 0).
        /// Format: one trimmed screen row per line, newline-terminated —
        /// byte-identical to `--dump-screen`.
        #[arg(long)]
        baseline: Option<std::path::PathBuf>,

        /// Rewrite `--baseline` (and `--style-baseline`, when given) in place
        /// from the current render and exit 0. Never implicit: a deliberate
        /// visual change must be accepted by a human running this flag.
        #[arg(long)]
        accept_baseline: bool,

        /// Committed golden for the per-cell style grid, diffed exactly like
        /// `--baseline`: a match confirms and exits 0, drift prints the first
        /// differing (row, col) with both tokens to stderr and exits
        /// non-zero, and a missing file bootstraps (created, exit 0). The two
        /// baseline flags are independent, so a surface can gate on text,
        /// style, or both, in one invocation.
        #[arg(long)]
        style_baseline: Option<std::path::PathBuf>,

        /// Comma-separated frame indices to capture as text, e.g. "0,3,7".
        /// Indices are 0-based over painted frames (`0` = the first frame
        /// the run loop draws). Each is written to `--capture-dir` as
        /// `frame-<NNNN>.txt` as it is painted. Indices past the end of the
        /// run are reported as an error rather than silently skipped.
        #[arg(long)]
        capture_frames: Option<String>,

        /// Output directory for `--capture-frames` files (default
        /// `tui-frames`). Created if missing.
        #[arg(long)]
        capture_dir: Option<std::path::PathBuf>,

        /// Start with the `--dangerously-skip-permissions` confirmation
        /// dialog already open, the same way the real flag does. The dialog
        /// is only raised in `TuiApp::enter`, before any event or key is
        /// processed, so it cannot be reached from an agent script.
        #[arg(long)]
        bypass_permissions: bool,
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
            mouse,
            output,
            assert,
            dump_screen,
            dump_style,
            assert_screen,
            agent_script,
            size,
            max_frames,
            baseline,
            accept_baseline,
            style_baseline,
            capture_frames,
            capture_dir,
            bypass_permissions,
        } => {
            debug_simulate(
                config,
                SimulateArgs {
                    keys,
                    mouse,
                    output,
                    assert_str: assert,
                    dump_screen,
                    dump_style,
                    assert_screen,
                    agent_script,
                    size,
                    max_frames,
                    baseline,
                    accept_baseline,
                    style_baseline,
                    capture_frames,
                    capture_dir,
                    bypass_permissions,
                },
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
            blocks.sort_by_key(|b| std::cmp::Reverse(b.created_at));
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
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(ctx) = v.get("context_window_size").and_then(|c| c.as_u64())
        {
            println!("Configured context window: {} tokens", ctx);
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
    println!("=== Registered Slash Commands ===");
    println!("Source: crates/operant-cli/src/tui/operant_app/app.rs::REGISTERED_COMMANDS");
    println!();
    let mut entries: Vec<_> = crate::tui::operant_app::app::registered_command_entries().collect();
    entries.sort();
    for (name, description) in entries {
        println!("{:<22} {}", name, description);
    }
    println!();
    println!(
        "Note: registration is the /help catalog, not a handler guarantee.          The command-sweep script (tests/tui_scenarios/command_sweep.py)          reports which of these are actually intercepted."
    );
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

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
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

#[expect(
    clippy::unwrap_used,
    reason = "invariant guaranteed by surrounding validation"
)]
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

// ---------------------------------------------------------------------------
// Settings load/save helpers
// ---------------------------------------------------------------------------

fn load_settings() -> crate::tui::adapter_types::config::Settings {
    crate::tui::adapter_types::config::Settings::load_sync().unwrap_or_default()
}

fn save_settings(settings: &crate::tui::adapter_types::config::Settings) -> Result<()> {
    settings.save_sync()
}

/// Parse a generic chord the explicit token table does not name:
/// `<ctrl+<letter>>` / `<alt+<letter>>` (optionally `+shift`), and the whole
/// `<f1>`..`<f12>` family.
///
/// `None` means "not a chord" and hands the token back to `parse_key_sequence`
/// to type as literal characters — the contract the corpus relies on, so a
/// typo'd binding shows up as typed text rather than a silent no-op.
fn parse_generic_chord(
    lower: &str,
) -> Option<(crossterm::event::KeyCode, crossterm::event::KeyModifiers)> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Digits only, so `<f13>` stays unrecognised rather than wrapping.
    if let Some(num) = lower.strip_prefix('f')
        && let Ok(n) = num.parse::<u8>()
        && (1..=12).contains(&n)
    {
        return Some((KeyCode::F(n), KeyModifiers::NONE));
    }

    let mut parts = lower.rsplitn(2, '+');
    let last = parts.next()?;
    let head = parts.next()?;
    if last.chars().count() != 1 || head.is_empty() {
        return None;
    }
    let mut modifiers = KeyModifiers::NONE;
    for seg in head.split('+') {
        match seg {
            "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" | "meta" | "option" => modifiers.insert(KeyModifiers::ALT),
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            _ => return None,
        }
    }
    if modifiers == KeyModifiers::NONE {
        return None;
    }
    Some((KeyCode::Char(last.chars().next()?), modifiers))
}

fn parse_key_sequence(seq: &str) -> Vec<crossterm::event::KeyEvent> {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    let mut events = Vec::new();
    let chars: Vec<char> = seq.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<'
            && let Some(close_idx) = chars[i..].iter().position(|&c| c == '>')
        {
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
                "pageup" | "pgup" => KeyCode::PageUp,
                "pagedown" | "pgdn" => KeyCode::PageDown,
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
                _ => match parse_generic_chord(&lower) {
                    Some((code, mods)) => {
                        modifiers = mods;
                        code
                    }
                    None => {
                        parsed = false;
                        KeyCode::Null
                    }
                },
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

/// Parse a mouse event sequence (e.g. "<left,10,20><drag,30,20><release,30,20>").
/// Format: `<action,x,y>` where action is one of:
///   left, right, middle, down, up, drag, scroll_up, scroll_down
/// Coordinates are viewport-relative u16.
fn parse_mouse_sequence(seq: &str) -> Vec<crossterm::event::MouseEvent> {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut events = Vec::new();
    let chars: Vec<char> = seq.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<'
            && let Some(close_idx) = chars[i..].iter().position(|&c| c == '>')
        {
            let name: String = chars[i + 1..i + close_idx].iter().collect();
            let parts: Vec<&str> = name.split(',').collect();
            if parts.len() >= 3 {
                let action = parts[0].to_lowercase();
                let x: u16 = parts[1].trim().parse().unwrap_or(0);
                let y: u16 = parts[2].trim().parse().unwrap_or(0);
                let kind = match action.as_str() {
                    "left" => MouseEventKind::Down(MouseButton::Left),
                    "right" => MouseEventKind::Down(MouseButton::Right),
                    "middle" => MouseEventKind::Down(MouseButton::Middle),
                    "up" | "release" => MouseEventKind::Up(MouseButton::Left),
                    "drag" => MouseEventKind::Drag(MouseButton::Left),
                    "scroll_up" => MouseEventKind::ScrollUp,
                    "scroll_down" => MouseEventKind::ScrollDown,
                    _ => {
                        i += close_idx + 1;
                        continue;
                    }
                };
                events.push(MouseEvent {
                    kind,
                    column: x,
                    row: y,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                });
            }
            i += close_idx + 1;
        } else {
            i += 1;
        }
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
                if op == "==" { eq } else { !eq }
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
///
/// Four variants do NOT map to an `AgentEvent` because the TUI receives them
/// on channels rather than the event stream: `User` seeds the transcript turn
/// that `build_transcript_turns` anchors on, and `PermissionRequest` /
/// `UserQuestion` / `BackgroundTask` feed the same plumbing the live agent
/// feeds. `into_agent_event` returns `None` for those; `run_headless`
/// dispatches them by hand.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum MockAgentEvent {
    /// A user turn. `build_transcript_turns` starts a new turn at every
    /// `Role::User` message, and tool blocks are only partitioned onto a turn
    /// that has one — so a script with tool events but no `user` event paints
    /// no tool block at all.
    User {
        text: String,
    },
    /// A tool-permission request, as the agent would raise it.
    PermissionRequest {
        tool: String,
        #[serde(default)]
        tool_id: String,
        #[serde(default)]
        description: String,
    },
    /// A model-initiated question, as the `clarify` tool would raise it.
    UserQuestion {
        question: String,
        #[serde(default)]
        choices: Option<Vec<String>>,
    },
    /// A background delegation record, registered through the same
    /// process-wide registry `SubAgentTool::dispatch_background` writes to.
    BackgroundTask {
        goal: String,
        #[serde(default)]
        model: String,
        /// `pending` (default), `completed` or `failed`.
        #[serde(default)]
        status: String,
    },
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
    /// `None` for the four channel-fed variants, which `run_headless`
    /// dispatches by hand instead of pushing onto `agent_event_rx`.
    pub(crate) fn into_agent_event(self) -> Option<operant_core::agent::AgentEvent> {
        use operant_core::agent::AgentEvent as AE;
        Some(match self {
            MockAgentEvent::User { .. }
            | MockAgentEvent::PermissionRequest { .. }
            | MockAgentEvent::UserQuestion { .. }
            | MockAgentEvent::BackgroundTask { .. } => return None,
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
                    timed_out: false,
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
                // The debug simulator scripts finish cleanly — the mock
                // always reports a normal text-response exit (S5 field).
                AE::Done {
                    message: msg,
                    reason: operant_core::agent::TurnExitReason::TextResponse,
                }
            }
            MockAgentEvent::Error { error } => AE::Error { error },
        })
    }
}

/// Every `operant tui debug simulate` option, bundled so the handler keeps a
/// stable signature as the harness grows more capture/diff flags.
struct SimulateArgs {
    keys: String,
    mouse: String,
    output: Option<std::path::PathBuf>,
    assert_str: Option<String>,
    dump_screen: Option<std::path::PathBuf>,
    dump_style: Option<std::path::PathBuf>,
    assert_screen: Option<String>,
    agent_script: Option<std::path::PathBuf>,
    size: Option<String>,
    max_frames: Option<u64>,
    baseline: Option<std::path::PathBuf>,
    accept_baseline: bool,
    style_baseline: Option<std::path::PathBuf>,
    capture_frames: Option<String>,
    capture_dir: Option<std::path::PathBuf>,
    bypass_permissions: bool,
}

async fn debug_simulate(config: &AppConfig, args: SimulateArgs) -> Result<()> {
    use crate::tui::adapter_types::{LaunchMode, TuiApp};
    use crate::tui::debug::TuiEvent;

    println!("Starting headless TUI simulation...");
    let parsed_keys = parse_key_sequence(&args.keys);
    let parsed_mouse = parse_mouse_sequence(&args.mouse);
    println!(
        "Parsed {} key events, {} mouse events.",
        parsed_keys.len(),
        parsed_mouse.len()
    );

    // Parse --size WxH (default 120x40).
    let dims = match args.size.as_deref() {
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
    let frame_cap = Some(args.max_frames.unwrap_or(100_000));

    // Parse --capture-frames "0,3,7" into 0-based painted-frame indices.
    let capture_requested: Option<Vec<u64>> = args
        .capture_frames
        .as_deref()
        .map(parse_frame_index_list)
        .transpose()?;
    let capture_dir = capture_requested
        .as_ref()
        .map(|_| args.capture_dir.clone().unwrap_or_else(default_capture_dir));
    if let (Some(frames), Some(dir)) = (&capture_requested, &capture_dir)
        && !frames.is_empty()
    {
        std::fs::create_dir_all(dir)
            .map_err(|e| anyhow::anyhow!("Cannot create capture dir {:?}: {}", dir, e))?;
        println!(
            "Capturing {} frame(s) into {:?}: {:?}",
            frames.len(),
            dir,
            frames
        );
    }
    let frame_capture = match (capture_requested.clone(), capture_dir.clone()) {
        (Some(frames), Some(dir)) if !frames.is_empty() => Some((frames, dir)),
        _ => None,
    };

    let script = if let Some(ref path) = args.agent_script {
        let raw = std::fs::read_to_string(path)?;
        let mock: Vec<MockAgentEvent> = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("Failed to parse agent script {:?}: {}", path, e))?;
        println!("Injecting {} mock agent events.", mock.len());
        Some(mock)
    } else {
        None
    };

    let tui_app = TuiApp::enter(
        config.clone(),
        None,
        LaunchMode::Landing,
        true,
        args.bypass_permissions,
    )
    .await?;
    let (events, app, screen, capture_status, style_dump) = tui_app
        .run_headless(
            parsed_keys,
            parsed_mouse,
            script,
            dims,
            frame_cap,
            frame_capture,
        )
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

    if let Some(ref out_path) = args.output {
        let json = serde_json::to_string_pretty(&events)?;
        std::fs::write(out_path, json)?;
        println!("Saved simulation event log to {:?}", out_path);
    }

    if let Some(ref screen_path) = args.dump_screen {
        std::fs::write(screen_path, screen.join("\n"))?;
        println!("Saved final rendered screen to {:?}", screen_path);
    }

    if let Some(ref style_path) = args.dump_style {
        std::fs::write(style_path, &style_dump)?;
        println!("Saved final rendered style grid to {:?}", style_path);
    }

    // Report per-frame capture outcome. A requested frame the run never
    // painted is an error: silently shipping fewer files than asked for is
    // exactly the failure this harness exists to catch.
    if let (Some((requested, captured)), Some(dir)) = (capture_status, capture_dir.as_deref()) {
        let missing: Vec<u64> = requested
            .iter()
            .copied()
            .filter(|i| !captured.contains(i))
            .collect();
        for index in &captured {
            println!(
                "Captured frame {} -> {:?}",
                index,
                dir.join(format!("frame-{index:04}.txt"))
            );
        }
        if !missing.is_empty() {
            anyhow::bail!(
                "--capture-frames: run ended before frame(s) {:?} were painted ({} frame(s) \
                 drawn in total). Re-run with an in-range index (0..{}), or raise --max-frames.",
                missing,
                app.debug_hub.frame_count(),
                app.debug_hub.frame_count()
            );
        }
    }

    if has_errors {
        anyhow::bail!("Simulation failed: Errors detected in TUI event log.");
    }

    if let Some(ref assert_val) = args.assert_str {
        println!("Evaluating state assertions: {}", assert_val);
        evaluate_assertions(&app, assert_val)?;
    }

    if let Some(ref screen_asserts) = args.assert_screen {
        println!("Evaluating screen assertions: {}", screen_asserts);
        evaluate_screen_assertions(&screen, screen_asserts)?;
    }

    if let Some(ref baseline) = args.baseline {
        check_baseline(baseline, &screen, args.accept_baseline)?;
    }

    if let Some(ref style_baseline) = args.style_baseline {
        check_style_baseline(style_baseline, &style_dump, args.accept_baseline)?;
    }

    println!("Simulation succeeded without errors.");
    Ok(())
}

/// Default output directory for `--capture-frames` when `--capture-dir` is
/// omitted. Deliberately repo-local and git-ignorable rather than `/tmp`, so
/// a committed capture sits next to its scenario.
fn default_capture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("tui-frames")
}

/// Parse a `--capture-frames` list such as `"0,3,7"`. Indices are 0-based
/// over painted frames, so `0` is the first frame the loop draws.
fn parse_frame_index_list(list: &str) -> Result<Vec<u64>> {
    let mut out = Vec::new();
    for token in list.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let idx = token.parse::<u64>().map_err(|_| {
            anyhow::anyhow!(
                "Invalid --capture-frames entry '{}': expected 0-based frame indices",
                token
            )
        })?;
        if !out.contains(&idx) {
            out.push(idx);
        }
    }
    out.sort_unstable();
    if out.is_empty() {
        anyhow::bail!("--capture-frames was given but listed no frame indices");
    }
    Ok(out)
}

/// Canonical on-disk form of a rendered screen: one row per line with
/// trailing whitespace stripped, newline-terminated. Applied to BOTH sides of
/// a comparison so an editor that pads lines cannot manufacture a diff.
///
/// This is the baseline file format, and it is deliberately byte-identical to
/// `--dump-screen` output (minus the final newline) so a human can produce
/// one by hand if they want to.
fn baseline_text(screen: &[String]) -> String {
    let mut text = screen
        .iter()
        .map(|row| row.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

/// Compare `screen` against the golden file at `baseline`.
///
/// Semantics, in order:
///   * `--accept-baseline`, or a missing file → write the golden, print a
///     notice, return `Ok`. A missing baseline bootstraps the first run.
///   * content matches → print a confirmation, return `Ok`.
///   * content differs → print a unified diff to **stderr** and return an
///     error, so the process exits non-zero and CI fails on drift.
fn check_baseline(baseline: &std::path::Path, screen: &[String], accept: bool) -> Result<()> {
    let actual = baseline_text(screen);

    if accept {
        write_baseline(baseline, &actual)?;
        println!("Accepted baseline {:?} ({} bytes).", baseline, actual.len());
        return Ok(());
    }

    if !baseline.exists() {
        write_baseline(baseline, &actual)?;
        println!(
            "Baseline created at {:?} — no golden existed yet. Commit it; the next \
             run will diff against it and fail on drift.",
            baseline
        );
        return Ok(());
    }

    let expected = std::fs::read_to_string(baseline)
        .map_err(|e| anyhow::anyhow!("Cannot read baseline {:?}: {}", baseline, e))?;

    if normalize_baseline(&expected) == actual {
        println!("  ✓ baseline matches {:?}", baseline);
        return Ok(());
    }

    let diff = unified_screen_diff(baseline, &normalize_baseline(&expected), &actual);
    eprintln!("Screen drift from baseline {:?}:", baseline);
    eprintln!("{}", diff);
    eprintln!("\nIf this change is intended, re-run with --accept-baseline to rewrite the golden.");
    anyhow::bail!("Rendered screen differs from baseline {:?}", baseline);
}

/// Re-canonicalize a golden file read off disk so its comparison matches
/// `baseline_text` output: CRLF→LF, trailing whitespace stripped per line,
/// exactly one trailing newline.
fn normalize_baseline(raw: &str) -> String {
    let mut text: String = raw
        .replace("\r\n", "\n")
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

fn write_baseline(baseline: &std::path::Path, text: &str) -> Result<()> {
    if let Some(parent) = baseline.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Cannot create {:?}: {}", parent, e))?;
    }
    std::fs::write(baseline, text)
        .map_err(|e| anyhow::anyhow!("Cannot write baseline {:?}: {}", baseline, e))
}

/// Line-based unified diff of the golden (`expected`) against the current
/// render (`actual`), rendered like `diff -u`. Reuses the `similar` crate the
/// diff viewer already depends on rather than hand-rolling an LCS.
fn unified_screen_diff(baseline: &std::path::Path, expected: &str, actual: &str) -> String {
    use similar::TextDiff;
    TextDiff::from_lines(expected, actual)
        .unified_diff()
        .context_radius(3)
        .header(
            &format!("baseline {}", baseline.display()),
            "rendered screen",
        )
        .to_string()
        .trim_end()
        .to_string()
}

/// One place a style grid differs from its golden, in buffer coordinates.
#[derive(Debug, PartialEq, Eq)]
struct StyleDrift {
    /// 0-based row within the grid (the header line is not counted).
    row: usize,
    /// 0-based column within the row.
    col: usize,
    /// The token the golden holds, or the whole expected header line when
    /// `row`/`col` are not meaningful.
    expected: String,
    /// The token this run rendered.
    actual: String,
    /// How many cells differ in the whole grid, so a single wrong colour is
    /// visibly distinguishable from a whole repaint.
    cells: usize,
}

/// Compare the `operant-style-v1` style grid against a committed golden.
///
/// Same contract as [`check_baseline`] — `--accept-baseline` or a missing file
/// writes and exits 0, a match confirms and exits 0, drift exits non-zero and
/// leaves the golden intact — but the report is token-level. A unified diff
/// over a 120x40 grid replaces an entire 120-token line and never says WHICH
/// cell moved, so the first difference is located as (row, col) with both
/// tokens. No line diff is printed for the style grid at all: it would be
/// kilobytes of tokens to say the same thing far more slowly.
fn check_style_baseline(baseline: &std::path::Path, actual: &str, accept: bool) -> Result<()> {
    if accept {
        write_baseline(baseline, actual)?;
        println!(
            "Accepted style baseline {:?} ({} bytes).",
            baseline,
            actual.len()
        );
        return Ok(());
    }

    if !baseline.exists() {
        write_baseline(baseline, actual)?;
        println!(
            "Style baseline created at {:?} — no golden existed yet. Commit it; the next \
             run will diff against it and fail on drift.",
            baseline
        );
        return Ok(());
    }

    let expected = std::fs::read_to_string(baseline)
        .map_err(|e| anyhow::anyhow!("Cannot read style baseline {:?}: {}", baseline, e))?;
    let expected = normalize_baseline(&expected);

    if expected == actual {
        println!("  ✓ style baseline matches {:?}", baseline);
        return Ok(());
    }

    let Some(drift) = style_drift(&expected, actual) else {
        return Err(anyhow::anyhow!(
            "Style baseline {:?} differs, but the difference is in line shape, not in any \
             single cell token. Re-run with --accept-baseline if intended.",
            baseline
        ));
    };

    eprintln!("Style drift from baseline {:?}:", baseline);
    if drift.row == usize::MAX {
        eprintln!("  header line:");
        eprintln!("    expected: {}", drift.expected);
        eprintln!("    actual  : {}", drift.actual);
    } else {
        eprintln!(
            "  first differing cell at (row {}, col {}) of {} differing cell(s):",
            drift.row, drift.col, drift.cells
        );
        eprintln!("    expected: {}", drift.expected);
        eprintln!("    actual  : {}", drift.actual);
    }
    eprintln!("\nIf this change is intended, re-run with --accept-baseline to rewrite the golden.");
    anyhow::bail!(
        "Rendered style grid differs from baseline {:?} at (row {}, col {}): expected {}, got {}",
        baseline,
        drift.row,
        drift.col,
        drift.expected,
        drift.actual
    );
}

/// First cell where two `operant-style-v1` dumps disagree, plus the total
/// number of differing cells, or `None` when they differ only in line shape
/// (row count, or a row that is not a whole number of tokens) — that case is
/// reported without coordinates because there is no cell to point at.
/// `row == usize::MAX` marks a header-line difference.
fn style_drift(expected: &str, actual: &str) -> Option<StyleDrift> {
    let (Some(exp_header), Some(act_header)) = (expected.lines().next(), actual.lines().next())
    else {
        return None;
    };
    if exp_header != act_header {
        return Some(StyleDrift {
            row: usize::MAX,
            col: usize::MAX,
            expected: exp_header.to_string(),
            actual: act_header.to_string(),
            cells: 0,
        });
    }
    let exp_rows: Vec<&str> = expected.lines().skip(1).collect();
    let act_rows: Vec<&str> = actual.lines().skip(1).collect();
    if exp_rows.len() != act_rows.len() {
        return None;
    }
    let mut first: Option<StyleDrift> = None;
    let mut cells = 0usize;
    for (row, (exp_row, act_row)) in exp_rows.iter().zip(act_rows.iter()).enumerate() {
        let exp_tokens: Vec<&str> = exp_row.split(' ').collect();
        let act_tokens: Vec<&str> = act_row.split(' ').collect();
        if exp_tokens.len() != act_tokens.len() {
            return None;
        }
        for (col, (exp, act)) in exp_tokens.iter().zip(act_tokens.iter()).enumerate() {
            if exp == act {
                continue;
            }
            cells += 1;
            if first.is_none() {
                first = Some(StyleDrift {
                    row,
                    col,
                    expected: (*exp).to_string(),
                    actual: (*act).to_string(),
                    cells: 0,
                });
            }
        }
    }
    first.map(|mut d| {
        d.cells = cells;
        d
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the token-advance off-by-one: `close_idx` is relative to
    /// `chars[i..]`, so the old `i = close_idx + 1` only advanced the FIRST
    /// token and reprocessed the second forever (2+ token --mouse sequences
    /// hung the simulation before "Parsed…" ever printed).
    #[test]
    fn mouse_sequence_parses_every_token_and_terminates() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let evs = parse_mouse_sequence("<left,25,10><drag,55,12><scroll_up,3,4>");
        assert_eq!(evs.len(), 3);
        assert!(matches!(
            evs[0].kind,
            MouseEventKind::Down(MouseButton::Left)
        ));
        assert_eq!((evs[0].column, evs[0].row), (25, 10));
        assert!(matches!(
            evs[1].kind,
            MouseEventKind::Drag(MouseButton::Left)
        ));
        assert_eq!((evs[1].column, evs[1].row), (55, 12));
        assert!(matches!(evs[2].kind, MouseEventKind::ScrollUp));
        assert!(parse_mouse_sequence("").is_empty());
    }

    fn screen(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|r| r.to_string()).collect()
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("operant-tui-baseline-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn frame_index_list_parses_dedups_and_sorts() {
        assert_eq!(
            parse_frame_index_list("7,0,3,3").expect("parse"),
            vec![0, 3, 7]
        );
        assert_eq!(
            parse_frame_index_list(" 1 , 2 ").expect("parse"),
            vec![1, 2]
        );
    }

    #[test]
    fn frame_index_list_rejects_garbage_and_empty() {
        assert!(parse_frame_index_list("0,x").is_err());
        assert!(parse_frame_index_list("").is_err());
        assert!(parse_frame_index_list(" , ").is_err());
    }

    #[test]
    fn baseline_bootstraps_when_missing_then_matches() {
        let dir = tmpdir("bootstrap");
        let golden = dir.join("screen.golden");
        let rows = screen(&["alpha  ", "beta"]);

        // Missing golden → created, no error.
        check_baseline(&golden, &rows, false).expect("bootstrap succeeds");
        assert_eq!(
            std::fs::read_to_string(&golden).expect("read"),
            "alpha\nbeta\n"
        );

        // Identical render → still passes.
        check_baseline(&golden, &screen(&["alpha", "beta"]), false).expect("match succeeds");

        // Trailing whitespace on the rendered side must not count as drift.
        check_baseline(&golden, &screen(&["alpha", "beta   "]), false).expect("trim is stable");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn baseline_golden_with_crlf_and_padding_matches() {
        let dir = tmpdir("crlf");
        let golden = dir.join("screen.golden");
        // CRLF endings, per-line padding, and a missing final newline must all
        // canonicalise away rather than read as drift.
        std::fs::write(&golden, "alpha  \r\nbeta ").expect("seed");
        check_baseline(&golden, &screen(&["alpha", "beta"]), false).expect("canonicalised match");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn baseline_trailing_blank_row_is_real_drift() {
        let dir = tmpdir("blank");
        let golden = dir.join("screen.golden");
        // A blank screen row is content, not editor padding: it must NOT be
        // canonicalised away, or a baseline could hide a row that vanished.
        check_baseline(&golden, &screen(&["alpha", "beta"]), false).expect("bootstrap");
        assert!(check_baseline(&golden, &screen(&["alpha", "beta", ""]), false).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn baseline_drift_fails_with_diff_and_leaves_golden_intact() {
        let dir = tmpdir("drift");
        let golden = dir.join("screen.golden");
        check_baseline(&golden, &screen(&["alpha", "beta"]), false).expect("bootstrap");

        // One changed character must be enough to fail the gate.
        let err = check_baseline(&golden, &screen(&["alpha", "bxta"]), false)
            .expect_err("drift must fail");
        assert!(
            err.to_string().contains("differs from baseline"),
            "unexpected error: {err}"
        );
        // The golden is never rewritten by a failing comparison.
        assert_eq!(
            std::fs::read_to_string(&golden).expect("read"),
            "alpha\nbeta\n"
        );

        // ...and --accept-baseline is the only way to move it.
        check_baseline(&golden, &screen(&["alpha", "bxta"]), true).expect("accept");
        assert_eq!(
            std::fs::read_to_string(&golden).expect("read"),
            "alpha\nbxta\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unified_diff_has_hunk_header_and_markers() {
        let diff = unified_screen_diff(
            std::path::Path::new("screen.golden"),
            "alpha\nbeta\ngamma\n",
            "alpha\nbxta\ngamma\n",
        );
        assert!(diff.contains("@@"), "missing hunk header: {diff}");
        assert!(diff.contains("-beta"), "missing removal line: {diff}");
        assert!(diff.contains("+bxta"), "missing addition line: {diff}");
        assert!(
            diff.contains("--- baseline screen.golden"),
            "bad header: {diff}"
        );
    }

    #[test]
    fn explicit_tokens_keep_their_original_meaning() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let cases: Vec<(&str, KeyCode, KeyModifiers)> = vec![
            ("<enter>", KeyCode::Enter, KeyModifiers::NONE),
            ("<esc>", KeyCode::Esc, KeyModifiers::NONE),
            ("<escape>", KeyCode::Esc, KeyModifiers::NONE),
            ("<tab>", KeyCode::Tab, KeyModifiers::NONE),
            ("<up>", KeyCode::Up, KeyModifiers::NONE),
            ("<down>", KeyCode::Down, KeyModifiers::NONE),
            ("<left>", KeyCode::Left, KeyModifiers::NONE),
            ("<right>", KeyCode::Right, KeyModifiers::NONE),
            ("<backspace>", KeyCode::Backspace, KeyModifiers::NONE),
            ("<bs>", KeyCode::Backspace, KeyModifiers::NONE),
            ("<ctrl+a>", KeyCode::Char('a'), KeyModifiers::CONTROL),
            ("<ctrl+c>", KeyCode::Char('c'), KeyModifiers::CONTROL),
            ("<ctrl+t>", KeyCode::Char('t'), KeyModifiers::CONTROL),
            ("<ctrl+r>", KeyCode::Char('r'), KeyModifiers::CONTROL),
            ("<shift+tab>", KeyCode::BackTab, KeyModifiers::SHIFT),
        ];
        for (token, code, mods) in cases {
            let events = parse_key_sequence(token);
            assert_eq!(
                events.len(),
                1,
                "{token} should be one event, got {events:?}"
            );
            assert_eq!(events[0].code, code, "{token} code");
            assert_eq!(events[0].modifiers, mods, "{token} modifiers");
        }
    }

    #[test]
    fn generic_ctrl_and_alt_chords_parse() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let cases: Vec<(&str, KeyCode, KeyModifiers)> = vec![
            ("<ctrl+k>", KeyCode::Char('k'), KeyModifiers::CONTROL),
            ("<ctrl+j>", KeyCode::Char('j'), KeyModifiers::CONTROL),
            ("<ctrl+z>", KeyCode::Char('z'), KeyModifiers::CONTROL),
            ("<alt+k>", KeyCode::Char('k'), KeyModifiers::ALT),
            ("<alt+v>", KeyCode::Char('v'), KeyModifiers::ALT),
            ("<CTRL+K>", KeyCode::Char('k'), KeyModifiers::CONTROL),
            (
                "<ctrl+shift+m>",
                KeyCode::Char('m'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            (
                "<alt+shift+x>",
                KeyCode::Char('x'),
                KeyModifiers::ALT | KeyModifiers::SHIFT,
            ),
            (
                "<ctrl+alt+p>",
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        ];
        for (token, code, mods) in cases {
            let events = parse_key_sequence(token);
            assert_eq!(
                events.len(),
                1,
                "{token} should be one event, got {events:?}"
            );
            assert_eq!(events[0].code, code, "{token} code");
            assert_eq!(events[0].modifiers, mods, "{token} modifiers");
        }
    }

    #[test]
    fn function_key_family_parses() {
        use crossterm::event::KeyCode;
        for n in 1u8..=12 {
            let token = format!("<f{n}>");
            let events = parse_key_sequence(&token);
            assert_eq!(events.len(), 1, "{token} should be one event");
            assert_eq!(events[0].code, KeyCode::F(n), "{token}");
        }
    }

    #[test]
    fn unrecognised_token_falls_back_to_literal_characters() {
        // The literal-fallback contract: an unknown `<…>` is typed as its own
        // characters, so a scenario naming a binding the parser does not know
        // shows up as typed text rather than a silent no-op.
        for token in [
            "<f13>",
            "<f0>",
            "<foo>",
            "<ctrl+>",
            "<ctrl+shift+>",
            "<hyper+a>",
            "<a>",
        ] {
            let events = parse_key_sequence(token);
            let typed: String = events
                .iter()
                .filter_map(|e| match e.code {
                    crossterm::event::KeyCode::Char(c) => Some(c),
                    _ => None,
                })
                .collect();
            assert_eq!(typed, token, "{token} must type as its literal characters");
        }
    }

    #[test]
    fn tokens_and_literals_interleave_in_order() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let shape: Vec<(KeyCode, KeyModifiers)> =
            parse_key_sequence("/voice<enter><ctrl+k>ab<esc>")
                .iter()
                .map(|e| (e.code, e.modifiers))
                .collect();
        assert_eq!(
            shape,
            vec![
                (KeyCode::Char('/'), KeyModifiers::NONE),
                (KeyCode::Char('v'), KeyModifiers::NONE),
                (KeyCode::Char('o'), KeyModifiers::NONE),
                (KeyCode::Char('i'), KeyModifiers::NONE),
                (KeyCode::Char('c'), KeyModifiers::NONE),
                (KeyCode::Char('e'), KeyModifiers::NONE),
                (KeyCode::Enter, KeyModifiers::NONE),
                (KeyCode::Char('k'), KeyModifiers::CONTROL),
                (KeyCode::Char('a'), KeyModifiers::NONE),
                (KeyCode::Char('b'), KeyModifiers::NONE),
                (KeyCode::Esc, KeyModifiers::NONE),
            ]
        );
    }

    #[test]
    fn buffer_rows_trims_trailing_whitespace() {
        let area = ratatui::layout::Rect::new(0, 0, 6, 2);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf.set_string(0, 0, "ab  ", ratatui::style::Style::default());
        buf.set_string(0, 1, "cd", ratatui::style::Style::default());
        assert_eq!(
            crate::tui::debug::debug_hub::buffer_rows(&buf),
            vec!["ab", "cd"]
        );
    }

    // ── Style baseline gate ───────────────────────────────────────────

    /// A 2x2 `operant-style-v1` grid, optionally with one cell overridden.
    fn style_grid(cells: [(&str, &str); 4]) -> String {
        let mut out = String::from("operant-style-v1 2x2\n");
        for row in cells.chunks(2) {
            let tokens: Vec<String> = row
                .iter()
                .map(|(style, sym)| format!("{style}@{sym}"))
                .collect();
            out.push_str(&tokens.join(" "));
            out.push('\n');
        }
        out
    }

    fn plain_grid() -> String {
        style_grid([
            ("red/-/-", "a"),
            ("-/-/-", "b"),
            ("cyan/blue/b", "x"),
            ("-/-/-", "y"),
        ])
    }

    #[test]
    fn style_baseline_bootstraps_then_matches() {
        let dir = tmpdir("style-bootstrap");
        let golden = dir.join("screen.style.txt");
        let grid = plain_grid();

        check_style_baseline(&golden, &grid, false).expect("bootstrap succeeds");
        assert_eq!(
            std::fs::read_to_string(&golden).expect("read"),
            grid,
            "the golden is the dump verbatim"
        );

        check_style_baseline(&golden, &grid, false).expect("identical render matches");
        // CRLF + a missing final newline in the golden must canonicalise away
        // rather than read as drift, exactly like the text baseline.
        std::fs::write(&golden, grid.replace('\n', "\r\n").trim_end()).expect("seed crlf");
        check_style_baseline(&golden, &grid, false).expect("canonicalised match");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn style_baseline_drift_names_row_col_and_both_tokens() {
        let dir = tmpdir("style-drift");
        let golden = dir.join("screen.style.txt");
        check_style_baseline(&golden, &plain_grid(), false).expect("bootstrap");

        // ONE cell's colour changes, in row 1 col 1.
        let drifted = style_grid([
            ("red/-/-", "a"),
            ("-/-/-", "b"),
            ("cyan/blue/b", "x"),
            ("magenta/-/-", "y"),
        ]);
        let err = check_style_baseline(&golden, &drifted, false).expect_err("drift must fail");
        let msg = err.to_string();
        assert!(msg.contains("(row 1, col 1)"), "no coordinates: {msg}");
        assert!(msg.contains("-/-/-"), "no expected token: {msg}");
        assert!(msg.contains("magenta/-/-"), "no actual token: {msg}");
        // The golden survives a failing comparison.
        assert_eq!(
            std::fs::read_to_string(&golden).expect("read"),
            plain_grid()
        );

        // ...and --accept-baseline is the only way to move it.
        check_style_baseline(&golden, &drifted, true).expect("accept");
        assert_eq!(std::fs::read_to_string(&golden).expect("read"), drifted);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn style_drift_locates_the_exact_cell() {
        let expected = plain_grid();
        // Same drift the gate reports: row 1, col 1.
        let actual = style_grid([
            ("red/-/-", "a"),
            ("-/-/-", "b"),
            ("cyan/blue/b", "x"),
            ("magenta/-/-", "y"),
        ]);
        let drift = style_drift(&expected, &actual).expect("drift");
        assert_eq!((drift.row, drift.col), (1, 1));
        assert_eq!(drift.expected, "-/-/-@y");
        assert_eq!(drift.actual, "magenta/-/-@y");
        assert_eq!(drift.cells, 1, "one changed cell, not a repaint");

        // First-cell drift is (0, 0), not (1, 1) — the header is not a row.
        let head = style_grid([
            ("green/-/-", "a"),
            ("-/-/-", "b"),
            ("cyan/blue/b", "x"),
            ("-/-/-", "y"),
        ]);
        let drift = style_drift(&expected, &head).expect("drift");
        assert_eq!((drift.row, drift.col), (0, 0));
    }

    #[test]
    fn style_drift_counts_every_differing_cell() {
        let expected = plain_grid();
        // Row 0 fully repainted plus one cell in row 1. The FIRST cell is what
        // gets reported, but the count is what tells you which bug to go
        // looking for: one wrong colour, or a row that stopped rendering.
        let actual = style_grid([
            ("blue/-/-", "p"),
            ("blue/-/-", "q"),
            ("cyan/blue/b", "x"),
            ("magenta/-/-", "y"),
        ]);
        let drift = style_drift(&expected, &actual).expect("drift");
        assert_eq!((drift.row, drift.col), (0, 0));
        assert_eq!(drift.expected, "red/-/-@a");
        assert_eq!(drift.actual, "blue/-/-@p");
        assert_eq!(drift.cells, 3);
    }

    #[test]
    fn style_drift_reports_header_and_line_shape_without_cells() {
        let expected = plain_grid();
        assert_eq!(style_drift(&expected, &expected), None);

        // A different geometry is a header difference, not a cell difference.
        let resized = "operant-style-v1 4x2\nred/-/-@a -/-/-@b\nred/-/-@c -/-/-@d\n";
        let drift = style_drift(&expected, resized).expect("header drift");
        assert_eq!(drift.row, usize::MAX);
        assert_eq!(drift.expected, "operant-style-v1 2x2");
        assert_eq!(drift.actual, "operant-style-v1 4x2");

        // A short row and an extra row are line-shape problems: the gate
        // must still fail, but it has no cell to point at.
        let short_row = "operant-style-v1 2x2\nred/-/-@a -/-/-@b\n";
        assert_eq!(style_drift(&expected, short_row), None);
        let extra_row = "operant-style-v1 2x2\nred/-/-@a -/-/-@b\n\
                         red/-/-@a -/-/-@b\nred/-/-@a -/-/-@b\n";
        assert_eq!(style_drift(&expected, extra_row), None);
    }

    #[test]
    fn style_baseline_line_shape_drift_fails_without_coordinates() {
        let dir = tmpdir("style-shape");
        let golden = dir.join("screen.style.txt");
        check_style_baseline(&golden, &plain_grid(), false).expect("bootstrap");
        // A dropped grid row must still be a failure, never a pass.
        let truncated = "operant-style-v1 2x2\nred/-/-@a -/-/-@b\n";
        let err = check_style_baseline(&golden, truncated, false).expect_err("row count must fail");
        assert!(
            err.to_string().contains("line shape"),
            "unexpected error: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
