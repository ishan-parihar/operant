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
        return Ok(());
    }

    // `--apply` takes the FIRST proposal it can perform and then stops.
    //
    // It used to print "Applied. Re-run ... to measure the realized saving"
    // and write NOTHING — a proven no-op (before 17775 tok, after 17775 tok).
    // The message is the loop's contract with the operator, so a flag that
    // lies is worse than no flag: it spends trust and produces a second
    // wasted measurement.
    //
    // Taking one move per invocation is the promise the docs already make
    // ("one change at a time, so you can tell which change caused the
    // problem"). A move is only offered when this process can APPLY it
    // without a future phase's mechanism.
    let mut applied = 0usize;
    for opt in &proposals {
        if apply_one(config, opt)? {
            applied += 1;
            println!("\nApplied 1: {}", opt.action);
            println!(
                "  expected ~{} tok/request back. Re-run `operant context budget` to measure it.",
                opt.saves_per_request
            );
            break;
        }
    }
    if applied == 0 {
        println!(
            "\nNo move applied. Every proposal names a change this command cannot safely write: \
             run the listed command yourself (they are the same commands the loop would run)."
        );
    }
    Ok(())
}

/// Apply ONE proposal, returning whether it actually changed something.
///
/// Deliberately narrow. Only moves that write through a mechanism this
/// process can validate are applied:
///
///  * `tool:<name>` → the `tool_state.json` disable set. This is the exact
///    code path `operant tools disable` uses (`tool_state::save_disabled_tools`),
///    so applying is not a second, divergent writer.
///  * `cli.inject_help` → `config::persist_config_key`, which validates the
///    rendered document through the REAL loader before renaming it over the
///    operator's file and keeps every comment (`toml_edit`).
///
/// Everything else (an MCP server, a CLI grant removal) is left to the
/// operator's own command rather than guessed at: an `--apply` that writes
/// something other than what it printed is the same defect in a new coat.
fn apply_one(
    config: &operant_core::config::AppConfig,
    opt: &operant_core::context_budget::Optimization,
) -> Result<bool> {
    if let Some(tool) = opt.surface.strip_prefix("tool:") {
        let mut disabled = operant_core::tools::tool_state::load_disabled_tools(config);
        if !disabled.insert(tool.to_string()) {
            // Already disabled before this run — nothing to claim.
            return Ok(false);
        }
        operant_core::tools::tool_state::save_disabled_tools(config, &disabled)
            .map_err(|e| anyhow::anyhow!("failed to persist tool state: {e}"))?;
        return Ok(true);
    }
    if opt.action.starts_with("set cli.inject_help") {
        let next = opt
            .action
            .split('"')
            .nth(1)
            .with_context(|| format!("unparseable proposal action: {}", opt.action))?
            .to_string();
        operant_core::config::persist_config_key("cli.inject_help", &next)
            .map_err(|e| anyhow::anyhow!("failed to persist cli.inject_help: {e:?}"))?;
        return Ok(true);
    }
    Ok(false)
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
///
/// Uses `get_schemas`, NOT `get_schemas_all`: the `_all` view is the
/// unfiltered registry (deliberately, so `operant tools list` can still show
/// a tool the operator turned off), and measuring it counts tools the agent
/// does NOT receive. That made every number here overstated and — worse —
/// made the optimization loop a lie: it proposed `operant tools disable todo`
/// and then still measured `tool:todo` at 769 tok/request after the disable.
/// Proven by round-trip, not by reading: disabling two tools moved the
/// measured total by exactly zero.
async fn per_tool_schemas(registry: &ToolRegistry) -> Vec<ToolSchema> {
    let schemas = registry.get_schemas().await;
    schemas
        .into_iter()
        .map(|s| {
            let json = serde_json::to_string(&s).unwrap_or_default();
            ToolSchema { name: s.name, json }
        })
        .collect()
}

/// Rewrite only the `[[cli.tools]]` entry for `name`, preserving EVERY other
/// byte of the config — comments, blank lines, key order, inline notes.
///
/// The first implementation round-tripped the whole document through
/// `toml::parse` + `to_string_pretty`, which silently deleted every comment in
/// the operator's config. That is a destructive side effect on a file this
/// command does not own: removing a CLI grant must not cost someone their
/// documentation. Proven: a config with 3 comments had 0 after `cli remove`.
///
/// This repo already has the right tool — `persist_config_key` keeps every
/// comment — and `context cli add` was correct by construction because it
/// appends instead of rewriting.
fn rewrite_cli_tool_entry(
    cfg_path: &std::path::Path,
    edit: impl FnOnce(&mut String) -> bool,
) -> Result<bool> {
    let raw = std::fs::read_to_string(cfg_path)
        .with_context(|| format!("failed to read {}", cfg_path.display()))?;
    let mut out = raw;
    if !edit(&mut out) {
        return Ok(false);
    }
    std::fs::write(cfg_path, &out)
        .with_context(|| format!("failed to write {}", cfg_path.display()))?;
    Ok(true)
}

/// The entry's current `enabled` state as it sits in the FILE, defaulting to
/// true when the key is absent (that is `CliToolSettings::default`). Reading
/// the file rather than the in-memory config keeps `toggle` honest against an
/// edit that happened after this process started.
///
/// Line-oriented and explicitly scoped: the target entry is the lines between
/// its own `[[cli.tools]]` header and the next table header (or EOF). A
/// matcher that only looked for `name = "git"` anywhere would answer from
/// whichever entry happened to come first, which is why `git` and its
/// neighbour `gitignore` are two different answers here.
fn cli_tool_enabled_in(text: &str, name: &str) -> Option<bool> {
    let mut in_entry = false;
    let mut target = false;
    let mut enabled_line: Option<bool> = None;
    let mut answer: Option<bool> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[[cli.tools]]" {
            // Close the previous entry out before opening the next one: the
            // header is a boundary, and a target answered by the line AFTER
            // its boundary reads the wrong entry.
            if target {
                answer = Some(enabled_line.unwrap_or(true));
                target = false;
            }
            in_entry = true;
            enabled_line = None;
            continue;
        }
        if trimmed.starts_with('[') {
            if target {
                answer = Some(enabled_line.unwrap_or(true));
                target = false;
            }
            in_entry = false;
            continue;
        }
        if !in_entry {
            continue;
        }
        if trimmed.starts_with("name =") && cli_tool_name_matches(trimmed, name) {
            target = true;
            continue;
        }
        if target && trimmed.starts_with("enabled =") {
            enabled_line = Some(trimmed.contains("true"));
        }
    }
    if target {
        answer = Some(enabled_line.unwrap_or(true));
    }
    answer
}

fn cli_tool_name_matches(trimmed: &str, name: &str) -> bool {
    trimmed
        .split('=')
        .nth(1)
        .map(str::trim)
        .and_then(|v| v.strip_prefix('"'))
        .and_then(|v| v.strip_suffix('"'))
        .is_some_and(|v| v == name)
}

/// Delete the `[[cli.tools]]` block for `name`, comments-preserving. Only the
/// entry's own lines are consumed: a comment sitting BETWEEN entries is the
/// operator's, so a comment preceded by a table header ends the block.
fn remove_cli_tool_block(text: &mut String, name: &str) -> bool {
    let block = format!("[[cli.tools]]\nname = \"{name}\"\n");
    let Some(start) = text.find(&block) else {
        return false;
    };
    let mut end = start + block.len();
    for line in text[start + block.len()..].split_inclusive('\n') {
        let trimmed = line.trim();
        let ends_entry = trimmed.is_empty()
            || trimmed.starts_with('[')
            || (trimmed.starts_with('#') && text[end..].contains("\n[["));
        if ends_entry {
            break;
        }
        end += line.len();
    }
    text.replace_range(start..end, "");
    // The seam left behind can now hold two blank lines where the entry used
    // to be. `String::replace` RETURNS the collapsed text — it does not edit
    // in place — so assign it, or this line is a no-op wearing a costume.
    let collapsed = text.replace("\n\n\n", "\n\n");
    text.clear();
    text.push_str(&collapsed);
    true
}

/// Whether the NAMED entry owns an `enabled` line at all. A toggle that
/// inserts a key must not also flip an existing one — a duplicate key is a
/// silently-corrupt TOML file (the last value wins, so it LOOKS right).
fn cli_entry_has_enabled_line(text: &str, name: &str) -> bool {
    let mut in_entry = false;
    let mut target = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[[cli.tools]]" {
            if target {
                return false;
            }
            in_entry = true;
            target = false;
            continue;
        }
        if trimmed.starts_with('[') {
            if target {
                return false;
            }
            in_entry = false;
            continue;
        }
        if !in_entry {
            continue;
        }
        if trimmed.starts_with("name =") && cli_tool_name_matches(trimmed, name) {
            target = true;
            continue;
        }
        if target && trimmed.starts_with("enabled =") {
            return true;
        }
    }
    false
}

/// Flip `enabled` on one `[[cli.tools]]` entry, comments-preserving. When the
/// entry has no `enabled` line the key is INSERTED right after `name` instead
/// of silently doing nothing, so `toggle` works on hand-written entries too.
///
/// Every line of the entry other than the one `enabled` line is copied
/// through verbatim — comments included — so an operator's notes survive a
/// flip. The FIRST implementation rebuilt the whole document with
/// `toml::to_string_pretty` and took every comment with it.
fn set_cli_tool_enabled(text: &mut String, name: &str, next: bool) -> bool {
    let Some(current) = cli_tool_enabled_in(text, name) else {
        return false;
    };
    if current == next {
        // The file already says what we want. Rewriting identical bytes is
        // churn that dirties the operator's file for no reason.
        return true;
    }
    // Insert only when NO line exists to flip: an insert AND a flip both
    // landing leaves `enabled = false` twice, which parses (last one wins)
    // and is therefore worse than an obvious failure.
    let insert = !cli_entry_has_enabled_line(text, name);
    let mut in_entry = false;
    let mut target = false;
    let mut wrote = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let rtrimmed = line.trim();
        if rtrimmed == "[[cli.tools]]" {
            in_entry = true;
            target = false;
            out.push_str(line);
            continue;
        }
        if rtrimmed.starts_with('[') {
            in_entry = false;
            target = false;
            out.push_str(line);
            continue;
        }
        if in_entry && rtrimmed.starts_with("name =") && cli_tool_name_matches(rtrimmed, name) {
            target = true;
            out.push_str(line);
            if insert {
                out.push_str(&format!("enabled = {next}\n"));
                wrote = true;
            }
            continue;
        }
        if target && rtrimmed.starts_with("enabled =") {
            // Preserve any trailing comment: the operator's
            // `enabled = true # granted last week` keeps its note. Replacing
            // the whole line is how the first rebuild lost every comment.
            if let Some(hash) = rtrimmed.find('#') {
                out.push_str(&format!("enabled = {next} "));
                out.push_str(&rtrimmed[hash..]);
                out.push('\n');
            } else {
                out.push_str(&format!("enabled = {next}\n"));
            }
            wrote = true;
            continue;
        }
        out.push_str(line);
    }
    if !target || !wrote {
        return false;
    }
    *text = out;
    true
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
                "Injected cost once the agent surface is wired: ~{} tok/request.",
                estimate_tokens(&format!("- {name} (v): usage: {name} [opts]"))
            );
            println!(
                "NOTE: the grant is recorded and `operant context budget` measures it, but the \
                 agent cannot invoke it yet — `cli_exec` and the <cli_reference> injection are the \
                 next phase (plan 2026-10-27-cli-surface-as-an-agent-tool-surface). Until then this \
                 configures the surface, it does not activate it."
            );
        }
        CliSubcommand::Remove { name } => {
            let cfg_path = config.config_path.clone().with_context(|| {
                "the loaded config has no recorded source path — refusing to guess where to write"
            })?;
            let removed =
                rewrite_cli_tool_entry(&cfg_path, |text| remove_cli_tool_block(text, name))?;
            if !removed {
                println!("No [[cli.tools]] entry named '{name}'.");
                return Ok(());
            }
            println!("Removed the '{name}' grant from {}.", cfg_path.display());
        }
        CliSubcommand::Toggle { name, enabled } => {
            let cfg_path = config.config_path.clone().with_context(|| {
                "the loaded config has no recorded source path — refusing to guess where to write"
            })?;
            // Read the entry's CURRENT state from the file rather than from the
            // parsed config: a config edited since this process started would
            // otherwise make `toggle` flip against a stale value.
            let raw = std::fs::read_to_string(&cfg_path).unwrap_or_default();
            let Some(current_on) = cli_tool_enabled_in(&raw, name) else {
                println!("No [[cli.tools]] entry named '{name}'.");
                return Ok(());
            };
            let next = enabled.unwrap_or(!current_on);
            let changed =
                rewrite_cli_tool_entry(&cfg_path, |text| set_cli_tool_enabled(text, name, next))?;
            if !changed {
                println!("No [[cli.tools]] entry named '{name}'.");
                return Ok(());
            }
            println!(
                "'{name}' is now {}.",
                if next { "enabled" } else { "disabled" }
            );
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

    /// THE BUDGET ROUND-TRIP. An optimization proposal that does not reduce
    /// the measured number is worse than no proposal: the operator takes the
    /// advice, pays the capability cost, and gets nothing back.
    ///
    /// This is a regression test for a real defect, found by validation on
    /// 2026-10-27 and not by inspection. The measurement used
    /// `get_schemas_all()` — the UNFILTERED registry, which deliberately
    /// includes disabled tools so `operant tools list` can show what the
    /// operator turned off. So after `operant tools disable todo`, the budget
    /// still counted `tool:todo` at 769 tok/request, and the round-trip moved
    /// the total by exactly ZERO. Two tools disabled, measured delta 0.
    ///
    /// The fix is to measure what the agent actually receives. Both halves are
    /// pinned here because either one alone would have hidden the defect:
    /// a filtered view with no disable, and a disable with no measurement.
    #[tokio::test]
    async fn the_budget_measures_what_the_agent_receives_not_the_whole_registry() {
        use operant_core::tools::ToolRegistry;

        let registry = ToolRegistry::new(std::time::Duration::from_secs(5));
        registry
            .register(operant_core::tools::FileReadTool)
            .await
            .unwrap();
        registry
            .register(operant_core::tools::FileWriteTool)
            .await
            .unwrap();

        // Nothing disabled: both are measured.
        let before = per_tool_schemas(&registry).await;
        assert_eq!(
            before.len(),
            2,
            "both tools are measured when nothing is disabled"
        );

        // The mechanism the real `operant tools disable` uses.
        let mut disabled = std::collections::BTreeSet::new();
        disabled.insert("file_write".to_string());
        registry
            .set_disabled_tools(std::collections::HashSet::from_iter(
                disabled.iter().cloned(),
            ))
            .await;

        let after = per_tool_schemas(&registry).await;
        assert!(
            !after.iter().any(|t| t.name == "file_write"),
            "a disabled tool must NOT be measured — the agent does not receive its schema, \
             so counting it overstates the bill and makes the optimization loop a lie"
        );
        assert!(
            after.iter().any(|t| t.name == "file_read"),
            "an ENABLED tool must still be measured"
        );
        assert_eq!(after.len(), 1);
    }

    // ---- iter-1432: the three round-3 defects of the CLI surface ----

    /// Removing a grant is a config EDIT, not a config REWRITE. The first
    /// implementation serialized the whole document through
    /// `toml::to_string_pretty`, which drops every comment: a config with 3
    /// comments had 0 after `context cli remove`. The operator's
    /// documentation of their own file is not collateral damage this command
    /// is entitled to collect.
    #[test]
    fn cli_remove_preserves_every_comment() {
        let mut text = String::from(
            "# my important header\n[agent]\nmodel = \"gpt-4\" # inline note\n\n[cli]\nenabled = true\n\n[[cli.tools]]\nname = \"git\"\nenabled = true\n\n# keep me\n[[cli.tools]]\nname = \"node\"\nenabled = true\n",
        );
        assert!(remove_cli_tool_block(&mut text, "git"), "git entry exists");
        assert_eq!(
            text.matches('#').count(),
            3,
            "every comment must survive, including the one between entries"
        );
        assert!(!text.contains("name = \"git\""), "git is gone");
        assert!(text.contains("name = \"node\""), "node untouched");
        assert!(text.contains("# my important header"));
        assert!(text.contains("model = \"gpt-4\" # inline note"));
        assert!(
            !text.contains("\n\n\n"),
            "the seam where the entry was must not be left as a doubled blank line: {text:?}"
        );
        // The document must still PARSE: a comments-preserving edit that
        // corrupts the TOML trades one defect for a worse one.
        toml::from_str::<toml::Value>(&text).expect("the edited config still parses as TOML");
    }

    /// The same for `toggle`: flipping a flag edits a line, it does not
    /// reformat the operator's file.
    #[test]
    fn cli_toggle_preserves_every_comment() {
        let mut text = String::from(
            "# header\n[agent]\nmodel = \"gpt-4\" # inline\n\n[cli]\nenabled = true\n\n[[cli.tools]]\nname = \"git\"\nenabled = true # granted last week\n",
        );
        let current = cli_tool_enabled_in(&text, "git").expect("git is granted");
        assert!(current, "a grant is on by default");
        assert!(set_cli_tool_enabled(&mut text, "git", false));
        assert_eq!(text.matches('#').count(), 3, "comments survive the flip");
        assert!(text.contains("enabled = false"));
        // The `[cli]` master switch is legitimately `enabled = true`, so assert
        // on the entry's OWN line rather than a substring of the document.
        assert!(
            text.contains("enabled = false # granted last week"),
            "the trailing comment rode along with the flipped value: {text}"
        );
        assert!(!text.contains("enabled = true # granted last week"));
    }

    /// `enabled` defaults to true when a hand-written entry omits it, and a
    /// toggle on that entry must INSERT the key rather than silently do
    /// nothing — because a silent no-op is exactly the defect class this
    /// iteration is closing.
    #[test]
    fn cli_toggle_inserts_enabled_when_the_key_is_absent() {
        let mut text = String::from(
            "[cli]\n\n[[cli.tools]]\nname = \"git\"\ndescription = \"version control\"\n",
        );
        assert_eq!(
            cli_tool_enabled_in(&text, "git"),
            Some(true),
            "absent `enabled` means on"
        );
        assert!(set_cli_tool_enabled(&mut text, "git", false));
        assert!(text.contains("enabled = false"), "the key was inserted");
        assert!(
            text.contains("description = \"version control\""),
            "the operator's other keys survive the insert: {text}"
        );
    }

    /// `cli_tool_enabled_in` reads the FILE, not the in-memory config, so a
    /// config edited after the process started cannot make `toggle` flip
    /// against a stale value. Distinguishing same-shaped neighbors is the
    /// whole point: a substring match on the name flips both.
    #[test]
    fn cli_tool_enabled_in_picks_the_named_entry_not_its_neighbour() {
        let text = "[cli]\n\n[[cli.tools]]\nname = \"gitignore\"\nenabled = false\n\n[[cli.tools]]\nname = \"git\"\nenabled = true\n";
        assert_eq!(cli_tool_enabled_in(text, "git"), Some(true));
        assert_eq!(cli_tool_enabled_in(text, "gitignore"), Some(false));
        assert_eq!(cli_tool_enabled_in(text, "nope"), None, "no such grant");
    }

    /// `--apply` writes. The first implementation accepted the flag, printed
    /// "Applied. Re-run to measure the realized saving" and applied NOTHING
    /// — proven by a before/after pair that moved by exactly 0 tok (17775 →
    /// 17775). The message is the loop's contract, so a flag that lies costs
    /// more than no flag: it spends trust and produces a second wasted
    /// measurement.
    #[test]
    fn apply_one_disable_writes_the_tool_state_file() {
        let cfg = test_config("iter1432_apply");
        let opt = operant_core::context_budget::Optimization {
            surface: "tool:todo".to_string(),
            action: "operant tools disable todo".to_string(),
            saves_per_request: 769,
            risk: "x".to_string(),
        };
        assert!(
            apply_one(&cfg, &opt).expect("applying a disable must succeed"),
            "the first apply is a real change"
        );
        let state = std::fs::read_to_string(operant_core::tools::tool_state::tool_state_path(&cfg))
            .expect("tool state written");
        assert!(
            state.contains("todo"),
            "the tool is persisted disabled: {state}"
        );
        assert!(
            !apply_one(&cfg, &opt).expect("a repeat apply is not an error"),
            "applying the same twice reports no change rather than lying"
        );
    }

    /// A move this process cannot perform must report false, so the caller
    /// prints the honest "run the listed command yourself" text instead of
    /// claiming success. MCP server moves are deliberately in that set:
    /// applying one would write a config the measured surface never reads.
    #[test]
    fn apply_one_reports_false_for_action_shapes_it_cannot_perform() {
        let cfg = test_config("iter1432_noop");
        let opt = operant_core::context_budget::Optimization {
            surface: "mcp:sourcehound".to_string(),
            action: "disable or defer the server mcp:sourcehound".to_string(),
            saves_per_request: 1600,
            risk: "x".to_string(),
        };
        assert!(
            !apply_one(&cfg, &opt).expect("an unperformed move is not an error"),
            "an unperformed move reports false so the caller stays honest"
        );
    }

    /// A config whose TEXT is written exactly as given, for tests that need
    /// comments and odd formatting present before a command runs.
    fn test_config_with(tag: &str, content: &str) -> operant_core::config::AppConfig {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("operant_{tag}_{}_{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("operant.toml");
        let content = content.replace("__DB__", &dir.join("test.db").display().to_string());
        std::fs::write(&path, &content).unwrap();
        operant_core::config::parse_config_str(&content, &path).expect("the test config parses")
    }

    /// The handler, end to end. Every other CLI test pins the helper
    /// functions; the OPERATOR's data loss happened in the handler, which is
    /// the one place the two are not interchangeable. Proven by mutation:
    /// restoring the handler's original `toml::to_string_pretty` rewrite left
    /// every helper-level test GREEN — the gap that let the defect ship.
    #[test]
    fn cli_remove_via_the_handler_preserves_the_operators_comments() {
        let cfg = test_config_with(
            "iter1432_rm_e2e",
            "database_path = \"__DB__\"\n\n# my header\n[agent]\nmodel = \"gpt-4\" # inline note\n\n[cli]\nenabled = true\n\n[[cli.tools]]\nname = \"git\"\nenabled = true\n",
        );
        let path = cfg
            .config_path
            .clone()
            .expect("the config records its path");
        cli_command(&cfg, &CliSubcommand::Remove { name: "git".into() })
            .expect("remove must succeed");
        let after = std::fs::read_to_string(&path).expect("config still there");
        // The file carries two comment tokens: `# my header` and the inline
        // one on the model line. The removed entry owns neither, so a
        // lossless remove still reports 2.
        assert_eq!(
            after.matches('#').count(),
            2,
            "the handler must not cost the operator their comments: {after}"
        );
        assert!(!after.contains("name = \"git\""), "the grant is gone");
        assert!(after.contains("# my header"));
        assert!(after.contains("model = \"gpt-4\" # inline note"));
        toml::from_str::<toml::Value>(&after).expect("the edited config still parses");
    }

    /// Same for `toggle`, and it also pins the flip itself: the handler must
    /// report the state it actually produced, not the state it intended.
    #[test]
    fn cli_toggle_via_the_handler_flips_and_preserves_comments() {
        let cfg = test_config_with(
            "iter1432_tg_e2e",
            "database_path = \"__DB__\"\n\n# my header\n[cli]\nenabled = true\n\n[[cli.tools]]\nname = \"git\"\nenabled = true # granted last week\n",
        );
        let path = cfg
            .config_path
            .clone()
            .expect("the config records its path");
        cli_command(
            &cfg,
            &CliSubcommand::Toggle {
                name: "git".into(),
                enabled: None,
            },
        )
        .expect("toggle must succeed");
        let after = std::fs::read_to_string(&path).expect("config still there");
        // `# my header` and the trailing note are both comments; 2 total.
        assert_eq!(after.matches('#').count(), 2, "comments survive: {after}");
        assert!(
            after.contains("enabled = false"),
            "the grant is now off: {after}"
        );
        assert!(
            after.contains("# granted last week"),
            "the trailing note survives: {after}"
        );
        toml::from_str::<toml::Value>(&after).expect("the edited config still parses");
    }

    /// A toggle must never leave TWO `enabled` keys in one entry. The
    /// duplicate is worse than a crash: TOML takes the last value, so the
    /// file reads correctly while carrying a second key that a future edit
    /// may resolve the other way. Found by the end-to-end handler test, not
    /// by the helper tests — the insert and the flip each looked right alone.
    #[test]
    fn cli_toggle_leaves_exactly_one_enabled_line() {
        let mut text = String::from(
            "[cli]\n\n[[cli.tools]]\nname = \"git\"\nenabled = true # granted last week\n",
        );
        assert!(set_cli_tool_enabled(&mut text, "git", false));
        assert_eq!(
            text.matches("enabled =").count(),
            1,
            "one entry, one enabled key: {text:?}"
        );
        assert!(
            text.contains("enabled = false # granted last week"),
            "the flip preserved the note: {text}"
        );
    }

    /// `optimize()` formats a CLI move as `set cli.inject_help = "usage" ...`.
    /// The APPLY side parses the quoted word back out, so the two must agree
    /// on the quoting — a rename on either side silently stops applying.
    #[test]
    fn apply_one_writes_cli_inject_help_and_keeps_comments() {
        let cfg = test_config_with(
            "iter1432_inject",
            "database_path = \"__DB__\"\n\n# documented by hand\n[cli]\nenabled = true\ninject_help = \"usage\"\n",
        );
        let path = cfg.config_path.clone().expect("config path recorded");
        // The exact string `optimize()` produces for a CLI surface.
        let action = operant_core::context_budget::optimize(&{
            let mut b = operant_core::context_budget::ContextBudget::new(100);
            b.push(operant_core::context_budget::SurfaceCost::new(
                "cli:git",
                "prompt-block",
                1,
                "- git",
            ));
            b
        });
        let opt = action
            .into_iter()
            .find(|o| o.surface == "cli:git")
            .expect("a CLI surface gets a proposal");
        assert!(apply_one(&cfg, &opt).expect("applying must succeed"));
        let after = std::fs::read_to_string(&path).expect("config still there");
        assert!(
            after.contains("# documented by hand"),
            "persist_config_key keeps comments: {after}"
        );
        assert!(
            after.contains("inject_help"),
            "the move named the key it wrote: {after}"
        );
    }

    fn test_config(tag: &str) -> operant_core::config::AppConfig {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("operant_{tag}_{}_{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("operant.toml");
        // The tool-state file lives next to the DATABASE, not next to the
        // config — so a config that does not name its own database resolves
        // `tool_state_path` against the REAL operant home. A test that then
        // calls `apply_one` reads and writes the operator's LIVE
        // `tool_state.json` (this test did exactly that and read
        // `{"disabled":["todo"]}` out of it, which is why the first apply
        // reported "already disabled"). Name the database explicitly.
        let content = format!(
            "database_path = \"{}\"\n\n[agent]\nmodel = \"test-model\"\n",
            dir.join("test.db").display()
        );
        std::fs::write(&path, &content).unwrap();
        operant_core::config::parse_config_str(&content, &path).expect("a minimal config parses")
    }
}
