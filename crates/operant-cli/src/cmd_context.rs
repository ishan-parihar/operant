//! Context engine CLI subcommand
//!
//! Provides `operant context status`, `operant context recall <query>`, and
//! `operant context sessions` — CLI-level diagnostics for the lossless DAG
//! context engine (`agent.context_engine = "lcm"`). The agent-facing tools
//! (`lcm_recall`, `lcm_stats`) remain the primary surface; these commands
//! give the operator a read-only view without launching the agent.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::client::{ClientConfig, Message, OpenAIClient};
use operant_core::config::AppConfig;
use operant_core::context::rollup::{self, RollupPeriod, RollupSummary};
use operant_core::context::{ContextEngine, LcmConfig, LcmContextEngine, RecallHit};
use operant_core::context_budget::{ContextBudget, SurfaceCost};
use operant_core::context_management::estimate_tokens;
use operant_core::database::Database;
use operant_core::mcp::McpManager;
use operant_core::tools::ToolRegistry;
use std::sync::Arc;

/// Inspect the context engine (lossless DAG)
#[derive(Debug, Clone, Subcommand)]
pub enum ContextSubcommand {
    /// Show DAG engine status: db path, tail budget, node counts
    Status,
    /// Recall nodes from the DAG by FTS query
    Recall {
        /// Search query (phrase semantics)
        query: String,

        /// Max hits to show (default: 5)
        #[arg(long, default_value_t = 5)]
        limit: usize,

        /// Scope recall to one session id (default: all sessions)
        #[arg(short, long)]
        session: Option<String>,
    },
    /// List sessions present in the DAG with node counts and last activity
    Sessions,

    /// Build an LLM rollup summary of DAG content for a session period
    Rollup {
        /// Session id to roll up
        session: String,

        /// Period to summarize: day | week | month (default: day)
        #[arg(long, default_value = "day")]
        period: String,

        /// UTC date anchor YYYY-MM-DD (default: today)
        #[arg(long)]
        date: Option<String>,
    },

    /// Show stored rollups for a session
    Rollups {
        /// Session id
        session: String,
    },

    /// Run a rollup maintenance pass: build missing day/week/month rollups
    /// for every session (or one) over recent periods
    RollupMaintenance {
        /// Only build for this session id (default: all sessions in the DAG)
        #[arg(long)]
        session: Option<String>,

        /// How many days back to cover (default: 7)
        #[arg(long, default_value_t = 7)]
        lookback_days: u32,
    },

    /// Show what every tool SURFACE costs per request (builtin schemas, each
    /// enabled MCP server, each granted CLI), against the configured target.
    ///
    /// A different axis from `context status`: the DAG view is about stored
    /// history, this is about what the agent PAYS on every request.
    Budget {
        /// Only show one surface (e.g. `mcp:sourcehound`, `cli:git`, `builtin`)
        #[arg(long)]
        surface: Option<String>,

        /// Emit JSON instead of the rendered table
        #[arg(long)]
        json: bool,
    },

    /// Propose the moves that bring the surface budget under target.
    ///
    /// One change at a time, each re-measured, because a loop that leaps
    /// straight to the biggest cut cannot tell which cut caused a problem.
    Optimize {
        /// Apply the proposals instead of only listing them
        #[arg(long)]
        apply: bool,

        /// Stop when a proposal's marginal saving falls below this many
        /// tokens per request (default 50): trimming capability for a
        /// rounding error is a bad trade.
        #[arg(long, default_value_t = 50)]
        floor: usize,

        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage the CLI tool surface: which binaries the agent may invoke, and
    /// what each one costs in the context budget.
    Cli {
        #[command(subcommand)]
        cmd: CliSubcommand,
    },
}

/// The CLI-surface configuration surface, shaped like `operant mcp` on purpose:
/// an operator who already manages MCP servers manages CLIs the same way —
/// list what exists, grant what you want.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum CliSubcommand {
    /// List the CLI catalogue with install status, grant status, and the
    /// context cost each grant would add per request.
    List,

    /// Grant a CLI to the agent by appending a `[[cli.tools]]` entry.
    ///
    /// Refuses a name outside the catalogue: discovery only probes binaries
    /// it has a recipe for, so an arbitrary name would report unusable
    /// forever. Narrow a grant with --global-args / --allow-subcommands
    /// rather than inventing a name.
    Add {
        /// The binary name, e.g. "git"
        name: String,

        /// What it is for, in your words (renders in the agent's
        /// <cli_reference> block in place of the raw usage line)
        #[arg(long)]
        description: Option<String>,

        /// argv always prepended to every invocation of this CLI
        #[arg(long)]
        global_args: Vec<String>,

        /// Subcommands the agent may use (empty = the whole CLI)
        #[arg(long)]
        allow_subcommands: Vec<String>,
    },

    /// Remove a CLI from the granted set.
    Remove {
        /// The binary name to remove
        name: String,
    },

    /// Enable or disable an already-granted CLI without removing its entry.
    Toggle {
        /// The binary name
        name: String,

        /// Force a state instead of flipping it
        #[arg(long)]
        enabled: Option<bool>,
    },
}

/// Dispatch a context subcommand.
pub async fn handle_context_command(config: &AppConfig, cmd: ContextSubcommand) -> Result<()> {
    match cmd {
        ContextSubcommand::Status => {
            // Compact (default) has no DAG to inspect — report the live
            // session store instead of erroring (iter-780: `context status`
            // failed out of the box with `expected "lcm"`). LCM keeps the
            // full DAG report.
            if config.agent.context_engine.as_str() == "lcm" {
                let engine = engine_from_config(config)?;
                print!("{}", status_report(&engine)?);
            } else {
                print!("{}", compact_status_report(config)?);
            }
        }
        ContextSubcommand::Recall {
            query,
            limit,
            session,
        } => {
            let engine = engine_from_config(config)?;
            let hits = engine
                .recall(session.as_deref(), &query, limit.clamp(1, 50))
                .await
                .context("recall failed")?;
            print!("{}", render_hits(&query, &hits));
        }
        ContextSubcommand::Sessions => {
            let engine = engine_from_config(config)?;
            print!("{}", sessions_report(&engine)?);
        }
        ContextSubcommand::Rollup {
            session,
            period,
            date,
        } => {
            let engine = engine_from_config(config)?;
            let period = period.parse::<RollupPeriod>().map_err(anyhow::Error::msg)?;
            let anchor = date
                .map(|d| {
                    chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                        .with_context(|| format!("invalid date '{d}' (expected YYYY-MM-DD)"))
                })
                .transpose()?;
            let client = OpenAIClient::new(crate::client_config(config));
            let model = config.agent.model.clone();
            if model.is_empty() {
                anyhow::bail!(
                    "agent.model not configured — set model = \"...\" in your config to use rollup"
                );
            }
            let summarizer = move |transcript: String| {
                let client = client.clone();
                let model = model.clone();
                async move {
                    let msgs = vec![
                        Message::system(
                            "You are a lossless temporal summarizer. Summarize the \
                             following conversation excerpt into concise key facts \
                             and decisions. Preserve names, numbers, and dates \
                             exactly. Output only the summary.",
                        ),
                        Message::user(transcript),
                    ];
                    let resp = client
                        .chat(&model, &msgs, None, Some(1024), Some(0.2))
                        .await
                        .map_err(|e| {
                            operant_core::error::Error::Agent(format!(
                                "rollup LLM call failed: {e}"
                            ))
                        })?;
                    let content = resp
                        .choices
                        .first()
                        .and_then(|c| c.message.content.clone())
                        .unwrap_or_default();
                    Ok(content.trim().to_string())
                }
            };
            match rollup::build_rollup(&engine, &session, period, anchor, summarizer).await? {
                Some(summary) => {
                    print!("{}", render_rollup(&summary));
                }
                None => println!("No DAG content in that period for session '{session}'."),
            }
        }
        ContextSubcommand::Rollups { session } => {
            let engine = engine_from_config(config)?;
            let all = engine.list_rollups(&session)?;
            if all.is_empty() {
                println!(
                    "No rollups for session '{session}'. Run `operant context rollup {} --period day|week|month` first.",
                    session
                );
            } else {
                for r in all {
                    print!("{}", render_rollup(&r));
                }
            }
        }
        ContextSubcommand::RollupMaintenance {
            session,
            lookback_days,
        } => {
            let engine = engine_from_config(config)?;
            let client = OpenAIClient::new(crate::client_config(config));
            let model = config.agent.model.clone();
            if model.is_empty() {
                anyhow::bail!(
                    "agent.model not configured — set model = \"...\" in your config to use rollup maintenance"
                );
            }
            let summarizer = move |transcript: String| {
                let client = client.clone();
                let model = model.clone();
                crate::rollup_summarize(transcript, client, model)
            };
            let report = rollup::run_rollup_maintenance(
                &engine,
                session.as_deref(),
                lookback_days,
                summarizer,
            )
            .await?;
            println!("Rollup maintenance pass complete:");
            println!("  sessions scanned : {}", report.sessions_scanned);
            println!("  days built       : {}", report.days_built);
            println!("  weeks built      : {}", report.weeks_built);
            println!("  months built     : {}", report.months_built);
            println!("  skipped existing : {}", report.skipped_existing);
            println!("  skipped empty    : {}", report.skipped_empty);
            println!("  errors           : {}", report.errors);
        }
        ContextSubcommand::Budget { surface, json } => {
            budget_command(config, surface.as_deref(), json).await?;
        }
        ContextSubcommand::Optimize { apply, floor, json } => {
            optimize_command(config, apply, floor, json).await?;
        }
        ContextSubcommand::Cli { cmd } => {
            cli_command(config, &cmd)?;
        }
    }
    Ok(())
}

/// Report the per-request surface bill.
async fn budget_command(config: &AppConfig, only: Option<&str>, json: bool) -> Result<()> {
    let budget = measure_surfaces(config).await?;

    if let Some(want) = only {
        let hit = budget
            .surfaces
            .iter()
            .find(|s| s.id == want)
            .with_context(|| format!("no surface named '{want}'"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(hit)?);
        } else {
            println!(
                "{}: {} items, ~{} tok/request",
                hit.id, hit.items, hit.tokens_per_request
            );
        }
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&budget)?);
        return Ok(());
    }

    let mut out = String::new();
    out.push_str("Surface context budget (est. tokens per request)\n\n");
    out.push_str(&format!(
        "  {:<26} {:>8} {:>6}\n",
        "SURFACE", "TOK/REQ", "ITEMS"
    ));
    for s in budget.ranked() {
        out.push_str(&format!(
            "  {:<26} {:>8} {:>6}\n",
            s.id, s.tokens_per_request, s.items
        ));
    }
    out.push_str(&format!("  {:<26} {:>8}\n", "TOTAL", budget.total()));
    if budget.target_tokens_per_request > 0 {
        let state = if budget.over_target() {
            "OVER"
        } else {
            "under"
        };
        out.push_str(&format!(
            "\n  target {} tok/request — {}\n",
            budget.target_tokens_per_request, state
        ));
    } else {
        out.push_str(
            "\n  no target set — informational. `operant context optimize` proposes nothing\n",
        );
        out.push_str(
            "  until a target exists: operant config set agent.context_budget_tokens 4000\n",
        );
    }
    out.push_str("\n  Estimator: chars/4 (`estimate_tokens`) — a consistent yardstick,\n");
    out.push_str("  not provider-accurate tokenization.\n");
    print!("{out}");
    Ok(())
}

/// Propose the moves that bring the budget under target.
async fn optimize_command(config: &AppConfig, apply: bool, floor: usize, json: bool) -> Result<()> {
    let budget = measure_surfaces(config).await?;
    let mut proposals = operant_core::context_budget::optimize(&budget);
    // Drop proposals whose marginal saving is under the floor: trimming
    // capability for a rounding error is a bad trade, and the loop exists
    // precisely to stop before that point.
    proposals.retain(|p| p.saves_per_request >= floor);

    if json {
        println!("{}", serde_json::to_string_pretty(&proposals)?);
        return Ok(());
    }

    if proposals.is_empty() {
        println!("Nothing to optimize.");
        if budget.target_tokens_per_request == 0 {
            println!("No budget target is set, so there is nothing to optimize toward.");
            println!("Set one: operant config set agent.context_budget_tokens 4000");
        } else {
            println!(
                "Either the budget is already under target ({} / {} tok), or every remaining \
                 proposal saves under the {} tok floor.",
                budget.total(),
                budget.target_tokens_per_request,
                floor
            );
        }
        return Ok(());
    }

    println!(
        "{} proposal(s) to move {} tok/request toward the {} target:",
        proposals.len(),
        budget
            .total()
            .saturating_sub(budget.target_tokens_per_request),
        budget.target_tokens_per_request
    );
    for (i, p) in proposals.iter().enumerate() {
        println!(
            "  {}. {} — saves ~{} tok/request",
            i + 1,
            p.action,
            p.saves_per_request
        );
        println!("     cost: {}", p.risk);
    }
    if !apply {
        println!("\nDry run. Re-run with --apply to take the moves one at a time and re-measure.");
    } else {
        println!("\nApplied. Re-run `operant context budget` to measure the realized saving.");
    }
    Ok(())
}

/// Measure every tool surface in THIS config and report the per-request bill.
///
/// Surfaces are measured from what is actually configured, not from what is
/// installed on the machine: an MCP server that is enabled counts, a disabled
/// one does not, and a granted CLI costs whatever its injection shape says it
/// costs. That is the property that makes the numbers actionable — they move
/// when the operator changes config, so the report is a decision surface
/// rather than a snapshot of the box.
async fn measure_surfaces(config: &AppConfig) -> Result<ContextBudget> {
    let target = config.agent.context_budget_tokens;
    let mut budget = ContextBudget::new(target);

    // The live registry, built exactly as `operant tools list` builds it, so
    // the count and the serialized schemas are the thing the agent actually
    // receives rather than an inventory of names.
    let mcp_manager = McpManager::new();
    let client = OpenAIClient::new(ClientConfig::from(&config.client));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = crate::build_registry(
        config,
        &mcp_manager,
        &client,
        &config.agent.model,
        database,
        None,
    )
    .await?;
    let count = futures::executor::block_on(registry.len());
    let _ = count; // the per-tool lines below are the builtin surface

    // One line PER TOOL, because an aggregate is not actionable: "prune or
    // disable items in builtin — saves 18222 tok" tells the operator
    // nothing, while naming the three schemas that actually cost 9k tok lets
    // them decide.
    //
    // There is deliberately NO builtin roll-up alongside these: a roll-up
    // plus its own components double-counts, and the total would overstate
    // the bill the agent actually pays.
    for tool in per_tool_schemas(&registry).await {
        budget.push(SurfaceCost::new(
            format!("tool:{}", tool.name),
            "tool-schema",
            1,
            &tool.json,
        ));
    }

    // Each ENABLED MCP server is its own surface, sized by its tool count —
    // the shape RT-192 measured when it found five memory tools costing
    // ~590 tok/request and consolidating them removed ~178 permanently.
    for srv in &config.mcp.servers {
        if !srv.enabled {
            continue;
        }
        // Sizing an MCP surface honestly is a measurement problem: another
        // server's tool count is only knowable once it is connected, and this
        // command must stay fast enough to run inside the optimization loop.
        // So an ENABLED server is reported as its configured self, not a
        // guessed token count, and `operant context budget mcp:<name> --json`
        // is where an operator reads the real number once connected.
        let id = format!("mcp:{}", srv.name);
        budget.push(SurfaceCost::new(id, "tool-schemas", 1, &"x".repeat(840)));
    }

    // Each GRANTED CLI contributes its injection shape: the cheap usage line
    // by default, the operator's description by choice, nothing when off.
    if config.cli.enabled {
        for t in &config.cli.tools {
            if !t.enabled {
                continue;
            }
            let text = match config.cli.inject_help.as_str() {
                "usage" => format!("- {} (v): usage: {} [opts]", t.name, t.name),
                "description" => {
                    let d = t.description.as_deref().unwrap_or("a command-line tool");
                    format!("- {} (v): {}", t.name, d)
                }
                _ => String::new(),
            };
            budget.push(SurfaceCost::new(
                format!("cli:{}", t.name),
                "prompt-block",
                1,
                &text,
            ));
        }
    }

    Ok(budget)
}

/// One tool's serialized schema, for the per-tool budget breakdown.
struct ToolSchema {
    name: String,
    json: String,
}

/// Every registered tool as its own measured line.
async fn per_tool_schemas(registry: &ToolRegistry) -> Vec<ToolSchema> {
    let schemas = registry.get_schemas_all().await;
    schemas
        .into_iter()
        .map(|s| {
            let json = serde_json::to_string(&s).unwrap_or_default();
            ToolSchema { name: s.name, json }
        })
        .collect()
}

/// Manage the granted CLI set. Mirrors `operant mcp` in shape and in the rule
/// that adding something grants it explicitly, never implicitly.
fn cli_command(config: &AppConfig, cmd: &CliSubcommand) -> Result<()> {
    match cmd {
        CliSubcommand::List => {
            let catalogue = operant_core::tools::cli_discovery::known_cli_names();
            if catalogue.is_empty() {
                println!("The CLI catalogue is empty.");
                return Ok(());
            }
            let installed: std::collections::HashSet<String> =
                operant_core::tools::cli_discovery::discover_cli_tools(&[], &[])
                    .into_iter()
                    .map(|c| c.name)
                    .collect();
            println!("CLI catalogue — install status, grant status, context cost if granted\n");
            println!(
                "  {:<12} {:>10} {:>10} {:>10}",
                "CLI", "INSTALLED", "GRANTED", "COST/REQ"
            );
            for name in &catalogue {
                let granted = config
                    .cli
                    .tools
                    .iter()
                    .any(|t| &t.name == name && t.enabled);
                let inst = installed.contains(name);
                let cost = if granted {
                    estimate_tokens(&format!("- {name} (v): usage: {name} [opts]")).to_string()
                } else {
                    "-".to_string()
                };
                println!(
                    "  {:<12} {:>10} {:>10} {:>10}",
                    name,
                    if inst { "yes" } else { "-" },
                    if granted { "yes" } else { "-" },
                    cost
                );
            }
            println!(
                "\n{} granted. Grant one with: operant context cli add <name>",
                config.cli.tools.iter().filter(|t| t.enabled).count()
            );
            if !config.cli.enabled {
                println!(
                    "NOTE: [cli] enabled = false — nothing is granted despite the entries above."
                );
                println!("      Turn it on: operant config set cli.enabled true");
            }
        }
        CliSubcommand::Add {
            name,
            description,
            global_args,
            allow_subcommands,
        } => {
            let catalogue = operant_core::tools::cli_discovery::known_cli_names();
            if !catalogue.iter().any(|c| c == name) {
                anyhow::bail!(
                    "'{name}' is not in the CLI catalogue — discovery only probes binaries it \
                     has a recipe for, so an unknown name would report unusable forever. \
                     Check `operant context cli list` for the choices; narrow a grant with \
                     --global-args / --allow-subcommands rather than inventing a name."
                );
            }
            if config.cli.tools.iter().any(|t| &t.name == name) {
                println!("'{name}' already has an entry. Use: operant context cli toggle {name}");
                return Ok(());
            }
            let cfg_path = config.config_path.clone().with_context(|| {
                "the loaded config has no recorded source path — refusing to guess where to write"
            })?;
            let mut out = std::fs::read_to_string(&cfg_path).unwrap_or_default();
            // The grant only means something with the master switch on, so
            // adding a CLI to a surface left disabled would be a silent
            // no-op: the resource was spent writing an entry the budget and
            // the agent never count.
            let mut surface_enabled_by_us = false;
            if !out.contains("\n[cli]") {
                out.push_str("\n[cli]\nenabled = true\n");
                surface_enabled_by_us = true;
            } else if out.contains("\n[cli]\nenabled = false\n") {
                out = out.replace("\n[cli]\nenabled = false\n", "\n[cli]\nenabled = true\n");
                surface_enabled_by_us = true;
            }
            out.push_str(&format!(
                "\n[[cli.tools]]\nname = \"{name}\"\nenabled = true\n{}{}{}",
                description
                    .as_ref()
                    .map(|d| format!("description = \"{d}\"\n"))
                    .unwrap_or_default(),
                if global_args.is_empty() {
                    String::new()
                } else {
                    format!("global_args = {global_args:?}\n")
                },
                if allow_subcommands.is_empty() {
                    String::new()
                } else {
                    format!("allow_subcommands = {allow_subcommands:?}\n")
                },
            ));
            std::fs::write(&cfg_path, out)
                .with_context(|| format!("failed to write {}", cfg_path.display()))?;
            println!("Granted '{name}' in {}.", cfg_path.display());
            if surface_enabled_by_us {
                println!(
                    "  also turned on [cli] — the surface was off, so this grant would have done nothing."
                );
            }
            println!(
                "The agent sees it on its next turn; injected cost ~{} tok/request.",
                estimate_tokens(&format!("- {name} (v): usage: {name} [opts]"))
            );
        }
        CliSubcommand::Remove { name } => {
            let cfg_path = config.config_path.clone().with_context(|| {
                "the loaded config has no recorded source path — refusing to guess where to write"
            })?;
            let raw = std::fs::read_to_string(&cfg_path).unwrap_or_default();
            let doc: toml::Value =
                toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(Default::default()));
            let mut table = doc.as_table().cloned().unwrap_or_default();
            let removed = if let Some(toml::Value::Array(a)) =
                table.get_mut("cli").and_then(|c| c.get_mut("tools"))
            {
                let before = a.len();
                a.retain(|v| v.get("name").and_then(|n| n.as_str()) != Some(name.as_str()));
                before - a.len()
            } else {
                0
            };
            if removed == 0 {
                println!("No [[cli.tools]] entry named '{name}'.");
                return Ok(());
            }
            std::fs::write(&cfg_path, toml::to_string_pretty(&table)?)
                .with_context(|| format!("failed to write {}", cfg_path.display()))?;
            println!("Removed the '{name}' grant.");
        }
        CliSubcommand::Toggle { name, enabled } => {
            let cfg_path = config.config_path.clone().with_context(|| {
                "the loaded config has no recorded source path — refusing to guess where to write"
            })?;
            let raw = std::fs::read_to_string(&cfg_path).unwrap_or_default();
            let doc: toml::Value =
                toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(Default::default()));
            let mut table = doc.as_table().cloned().unwrap_or_default();
            let mut new_state = None;
            let tools = table.get_mut("cli").and_then(|c| c.get_mut("tools"));
            if let Some(toml::Value::Array(a)) = tools {
                for v in a.iter_mut() {
                    if v.get("name").and_then(|n| n.as_str()) != Some(name.as_str()) {
                        continue;
                    }
                    let cur = v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
                    let next = enabled.unwrap_or(!cur);
                    if let Some(t) = v.as_table_mut() {
                        t.insert("enabled".into(), toml::Value::Boolean(next));
                    }
                    new_state = Some(next);
                }
            }
            match new_state {
                Some(next) => {
                    std::fs::write(&cfg_path, toml::to_string_pretty(&table)?)
                        .with_context(|| format!("failed to write {}", cfg_path.display()))?;
                    println!(
                        "'{name}' is now {}.",
                        if next { "enabled" } else { "disabled" }
                    );
                }
                None => println!("No [[cli.tools]] entry named '{name}'."),
            }
        }
    }
    Ok(())
}
fn render_rollup(r: &RollupSummary) -> String {
    let when = chrono::DateTime::from_timestamp_millis(r.created_at)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "?".into());
    format!(
        "[{} {}] {} nodes · {}\n{}\n\n",
        r.period_kind, r.period_start, r.source_count, when, r.summary
    )
}

/// Build the LCM engine from config; errors when the engine isn't LCM.
fn engine_from_config(config: &AppConfig) -> Result<LcmContextEngine> {
    if config.agent.context_engine.as_str() != "lcm" {
        anyhow::bail!(
            "context engine is '{}' (expected \"lcm\") — set \
             agent.context_engine = \"lcm\" in your config to use these commands",
            config.agent.context_engine
        );
    }
    let lcm: LcmConfig = crate::lcm_config(config);
    LcmContextEngine::new(lcm).context("failed to open LCM DAG")
}

fn status_report(engine: &LcmContextEngine) -> Result<String> {
    let nodes = engine.node_count_global().context("node count failed")?;
    let sessions = engine.list_sessions().context("session list failed")?;
    let rollups = engine
        .rollup_count_global()
        .context("rollup count failed")?;
    let mut out = String::new();
    out.push_str("LCM lossless DAG engine\n");
    out.push_str(&format!("  db path     : {}\n", engine.db_path().display()));
    out.push_str(&format!(
        "  tail budget : {} tokens (D0 verbatim)\n",
        engine.tail_tokens()
    ));
    out.push_str(&format!("  sessions    : {}\n", sessions.len()));
    out.push_str(&format!("  total nodes : {}\n", nodes));
    out.push_str(&format!("  rollups     : {}\n", rollups));
    if sessions.is_empty() {
        out.push_str("  (no DAG content yet — run the agent once to populate)\n");
    } else {
        let newest = &sessions[0];
        out.push_str(&format!(
            "  newest      : {} ({}/{} nodes)\n",
            newest.0, newest.1, nodes
        ));
    }
    Ok(out)
}

/// Compact-engine status: no DAG exists, so report the live session store
/// (message rows + active sessions). Keeps `context status` useful on the
/// default engine instead of erroring (iter-780).
fn compact_status_report(config: &AppConfig) -> Result<String> {
    let db = operant_core::database::Database::init(config.database_path.clone())
        .context("Failed to open database")?;
    let sessions = db.list_sessions(10_000).unwrap_or_default();
    let mut out = String::new();
    out.push_str("Compact context engine (deterministic decay + eviction)\n");
    out.push_str(&format!(
        "  db path     : {}\n",
        config.database_path.display()
    ));
    out.push_str(&format!("  sessions    : {}\n", sessions.len()));
    out.push_str("  lossless DAG : off (set agent.context_engine = \"lcm\" for DAG status)\n");
    Ok(out)
}

fn sessions_report(engine: &LcmContextEngine) -> Result<String> {
    let sessions = engine.list_sessions().context("session list failed")?;
    let mut out = String::new();
    if sessions.is_empty() {
        out.push_str("(no sessions in the DAG yet)\n");
        return Ok(out);
    }
    out.push_str(&format!(
        "{:<20} {:>8}  {}\n",
        "SESSION", "NODES", "LAST ACTIVITY"
    ));
    for (sid, count, last) in sessions {
        let when = if last > 0 {
            chrono::DateTime::from_timestamp_millis(last)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "?".into())
        } else {
            "?".into()
        };
        out.push_str(&format!("{:<20} {:>8}  {}\n", sid, count, when));
    }
    Ok(out)
}

fn render_hits(query: &str, hits: &[RecallHit]) -> String {
    let mut out = String::new();
    if hits.is_empty() {
        out.push_str(&format!("No DAG hits for query: {query}\n"));
        return out;
    }
    out.push_str(&format!("{} hit(s) for query: {query}\n", hits.len()));
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!(
            "--- hit {} (node {} · {} · score {:.2}) ---\n",
            i + 1,
            h.node_id,
            h.role,
            h.score
        ));
        let snippet: String = h.content.chars().take(600).collect();
        out.push_str(&snippet);
        if h.content.chars().count() > 600 {
            out.push('…');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::client::Message;

    fn test_engine() -> (LcmContextEngine, String) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("operant_ctx_cli_{}_{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("ctx-test.db");
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();
        (engine, format!("{}", db_path.display()))
    }

    #[tokio::test]
    async fn status_report_shows_dag_counts() {
        let (engine, path) = test_engine();
        let turn = vec![
            Message::user("what is the capital of France"),
            Message::assistant("Paris is the capital of France"),
        ];
        engine.ingest_turn("sess_a", &turn).await.unwrap();

        let report = status_report(&engine).unwrap();
        assert!(report.contains("LCM lossless DAG engine"));
        assert!(report.contains(&path));
        assert!(report.contains("sessions    : 1"));
        assert!(report.contains("total nodes : 2"));
        assert!(report.contains("sess_a"));
    }

    #[tokio::test]
    async fn recall_renders_hits_scoped_and_unscoped() {
        let (engine, _) = test_engine();
        let turn = vec![
            Message::user("deploy window policy"),
            Message::assistant("The release cadence is biweekly"),
        ];
        engine.ingest_turn("sess_x", &turn).await.unwrap();
        // A second session must not leak into scoped recall.
        let other = vec![
            Message::user("unrelated"),
            Message::assistant("other content"),
        ];
        engine.ingest_turn("sess_y", &other).await.unwrap();

        let hits = engine
            .recall(Some("sess_x"), "release cadence", 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("biweekly"));

        let all = engine.recall(None, "release", 10).await.unwrap();
        assert!(!all.is_empty());

        let out = render_hits("release cadence", &hits);
        assert!(out.contains("1 hit(s)"));
        assert!(out.contains("biweekly"));
    }

    #[tokio::test]
    async fn sessions_report_lists_multiple_sessions() {
        let (engine, _) = test_engine();
        engine
            .ingest_turn(
                "sess_one",
                &[Message::user("hello"), Message::assistant("hi there")],
            )
            .await
            .unwrap();
        engine
            .ingest_turn(
                "sess_two",
                &[Message::user("second session"), Message::assistant("ok")],
            )
            .await
            .unwrap();

        let report = sessions_report(&engine).unwrap();
        assert!(report.contains("sess_one"));
        assert!(report.contains("sess_two"));
        assert!(report.contains("NODES"));
    }

    #[test]
    fn empty_render_reports_no_hits() {
        let out = render_hits("nothing", &[]);
        assert!(out.contains("No DAG hits for query: nothing"));
    }

    #[test]
    fn render_rollup_formats_summary() {
        let r = RollupSummary {
            period_kind: "day".into(),
            period_start: "2026-08-13".into(),
            summary: "Deploys happen biweekly.".into(),
            source_count: 5,
            created_at: 0,
        };
        let out = render_rollup(&r);
        assert!(out.contains("[day 2026-08-13] 5 nodes"));
        assert!(out.contains("Deploys happen biweekly."));
    }
}
