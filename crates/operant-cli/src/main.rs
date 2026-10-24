//! Operant-RS CLI

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
mod autonomous;
mod cmd_acp;
mod cmd_architecture;
mod cmd_auth;
mod cmd_backup;
mod cmd_budget;
mod cmd_channel;
mod cmd_checkpoints;
mod cmd_completion;
mod cmd_config;
mod cmd_context;
mod cmd_cookies;
mod cmd_cron;
mod cmd_curator;
mod cmd_dashboard;
mod cmd_debug;
mod cmd_doctor;
mod cmd_dump;
mod cmd_gateway;
mod cmd_hooks;
mod cmd_import;
mod cmd_insights;
mod cmd_kanban;
mod cmd_logs;
mod cmd_mcp;
mod cmd_memory;
mod cmd_migrate;
mod cmd_model;
mod cmd_org;
mod cmd_plugins;
mod cmd_profile;
mod cmd_service;
mod cmd_sessions;
mod cmd_setup;
mod cmd_skills;
mod cmd_sop;
mod cmd_status;
mod cmd_suggestions;
mod cmd_tools;
mod cmd_trajectory;
mod cmd_tui_debug;
mod cmd_uninstall;
mod cmd_update;
mod cmd_version;
mod cmd_webhook;
mod commands;
pub(crate) mod config;
mod dashboard_server;
mod env_store;
mod gateway_commands;
mod gateway_platforms;
mod gateway_runner;
mod mcp_serve;
mod memory_provider_tools;
#[cfg(feature = "plugins-wasm")]
mod plugin_memory;
#[cfg(feature = "plugins-wasm")]
mod plugin_tools;
pub mod plugins_install;
mod post_setup;
mod prompt_helpers;
pub mod provider;
mod tui;

pub(crate) use tui::free_mode_dialog;
pub(crate) use tui::image_paste;
pub(crate) use tui::input;
pub(crate) use tui::mcp_view;
pub(crate) use tui::notifications;
pub(crate) use tui::osc8;
pub(crate) use tui::permissions_picker;
pub(crate) use tui::prompt_input;
pub(crate) use tui::settings_screen;
pub(crate) use tui::theme_screen;

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::CliConfig;
use anyhow::{Context, Result};
use clap::{ArgAction, Parser, Subcommand};
#[cfg(feature = "anthropic")]
use operant_core::agent::clients::anthropic::AnthropicModelClient;
use operant_core::agent::clients::openai::OpenAIModelClient;
use operant_core::agent::{AgentConfig, AgentEvent, OperantAgent};
use operant_core::client::{ClientConfig, OpenAIClient};
use operant_core::config::{
    AppConfig, BehaviorSettings, LoggingSettings, McpServerConfig, McpTransportKind,
    ModelProviderConfig, install_runtime_config, load_app_config,
};
use operant_core::mcp::McpManager;
use operant_core::memory::MemoryManager;
use operant_core::memory_provider::MemoryProvider;
use operant_core::observer::{Observer, ObserverEvent, ObserverMetric};
use operant_core::skills::SkillManager;
use operant_core::tools::{ToolContext, ToolRegistry};
use operant_core::turn_summary::{TurnSummaryCollector, format_turn_summary};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{Level, warn};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::tui::{LaunchMode, TuiApp};
use operant_core::cronjobs::CronDb;
use operant_core::database::Database;
use operant_core::kanban::KanbanDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogTarget {
    Stderr,
    Sink,
    File,
}

/// The full `--version` string: semver plus the compiled memory-wire rev
/// (plan 11A) so a deployed binary always reports which upstream memory-wire
/// it carries. A compile-time literal built from build.rs's rustc-env
/// (clap's `version` attribute only accepts 'static strings; the project
/// forbids Box::leak, so the build owns this data).
const FULL_VERSION_LINE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (memory-wire ",
    env!("OPERANT_MEMORY_WIRE_REV_SHORT"),
    ")"
);

#[derive(Debug, Parser)]
#[command(
    name = "operant",
    about = "Operant-RS: A high-performance ReAct agent framework",
    version = FULL_VERSION_LINE,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[arg(short, long, global = true)]
    log_level: Option<String>,

    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[arg(long, global = true, env = "OPENAI_API_KEY")]
    api_key: Option<String>,

    #[arg(long, global = true, env = "OPENAI_BASE_URL")]
    base_url: Option<String>,

    #[arg(long, global = true)]
    model: Option<String>,

    #[arg(long, global = true)]
    max_iterations: Option<usize>,

    #[arg(long, global = true)]
    tool_timeout: Option<u64>,

    #[arg(long, global = true)]
    request_timeout: Option<u64>,

    #[arg(long, global = true)]
    context_window: Option<usize>,

    #[arg(long, global = true)]
    max_healing_attempts: Option<usize>,

    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "no_stream")]
    stream: bool,

    #[arg(long = "no-stream", global = true, action = ArgAction::SetTrue, conflicts_with = "stream")]
    no_stream: bool,

    #[arg(long, global = true)]
    debug_tui: bool,

    /// Disable mouse capture in the TUI. Useful when running inside a
    /// terminal multiplexer (tmux/screen) or when the user wants the
    /// terminal's native mouse selection to work. (Bug #24 from iter-82
    /// audit — /mouse mentioned a --no-mouse flag that didn't exist.)
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    no_mouse: bool,

    /// Skip all tool-permission prompts for the session (shows a confirmation first).
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    dangerously_skip_permissions: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run {
        #[arg(short, long)]
        system: Option<String>,

        #[arg(short, long)]
        query: Option<String>,

        #[arg(long, action = ArgAction::SetTrue)]
        autonomous: bool,

        /// Record this run as a trajectory (ReAct steps + messages) saved to
        /// ~/.operant/trajectories/. View with `operant trajectory list`.
        #[arg(long, action = ArgAction::SetTrue)]
        record_trajectory: bool,

        /// Enable a Mixture-of-Agents turn (hermes /moa parity, G5): the
        /// configured `moa` reference models advise on the query, an
        /// aggregator synthesizes their guidance, and it is injected into
        /// the agent's context for this run. Requires `moa` enabled and at
        /// least one reference model configured.
        #[arg(long, action = ArgAction::SetTrue)]
        moa: bool,
    },
    Autonomous {
        #[arg(short, long)]
        system: Option<String>,
    },
    /// List and manage available tools
    Tools {
        #[command(subcommand)]
        cmd: cmd_tools::ToolsSubcommand,
    },
    Chat {
        #[arg(short, long)]
        system: Option<String>,
    },
    Test {
        #[arg()]
        tool_name: String,

        #[arg(short, long)]
        args: Option<String>,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        cmd: cmd_config::ConfigSubcommand,
    },
    /// Manage conversation sessions
    Sessions {
        #[command(subcommand)]
        cmd: cmd_sessions::SessionsSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage MCP servers
    Mcp {
        #[command(subcommand)]
        cmd: cmd_mcp::McpSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Import / export / list browser cookies for the browser session
    /// (multi-browser cookie import from Chrome, Brave, Edge, Firefox, …)
    Cookies {
        #[command(subcommand)]
        cmd: cmd_cookies::CookiesSubcommand,
    },
    /// Manage installed skills
    Skills {
        #[command(subcommand)]
        cmd: cmd_skills::SkillsSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// View or change the active model configuration
    Model {
        #[command(subcommand)]
        cmd: cmd_model::ModelSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Generate shell completion scripts
    Completion {
        #[command(subcommand)]
        cmd: cmd_completion::CompletionSubcommand,
    },
    /// Manage cron jobs
    Cron {
        #[command(subcommand)]
        cmd: cmd_cron::CronSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Pause the agent (global emergency stop — new work only)
    Pause {
        /// Optional reason recorded in the sentinel
        reason: Option<String>,
    },
    /// Resume from a global pause
    Resume,
    /// Manage suggested automations
    Suggestions {
        #[command(subcommand)]
        cmd: cmd_suggestions::SuggestionsSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage kanban tasks
    Kanban {
        /// Board slug to operate on (default: "default")
        #[arg(long, default_value_t = String::from("default"), global = true)]
        board: String,
        #[command(subcommand)]
        cmd: cmd_kanban::KanbanSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Per-seat budget overrides and read-only views over a Holon organization
    Org {
        #[command(subcommand)]
        cmd: cmd_org::OrgSubcommand,
    },
    /// Manage gateway
    Gateway {
        #[command(subcommand)]
        cmd: cmd_gateway::GatewaySubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage checkpoints
    Checkpoints {
        #[command(subcommand)]
        cmd: cmd_checkpoints::CheckpointsSubcommand,
    },
    /// Manage memory
    Memory {
        #[command(subcommand)]
        cmd: cmd_memory::MemorySubcommand,
    },
    /// Inspect the context engine (lossless DAG)
    Context {
        #[command(subcommand)]
        cmd: cmd_context::ContextSubcommand,
    },
    /// Manage profiles
    Profile {
        #[command(subcommand)]
        cmd: cmd_profile::ProfileSubcommand,
    },
    /// Manage auth credentials
    Auth {
        #[command(subcommand)]
        cmd: cmd_auth::AuthSubcommand,
    },
    /// Login to a provider
    Login,
    /// Logout
    Logout,
    /// Show version information
    Version {
        #[arg(long)]
        detailed: bool,
    },
    /// Check Operant configuration and dependencies
    Doctor {
        /// Attempt to auto-fix common issues
        #[arg(long)]
        fix: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
    /// Show system status overview
    Status {
        /// Show detailed status
        #[arg(long)]
        deep: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
    /// Print a setup summary report
    Dump {
        /// Show all configuration keys as YAML
        #[arg(long)]
        all: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long)]
        json: bool,
    },
    /// View log files
    Logs {
        #[command(subcommand)]
        cmd: cmd_logs::LogsSubcommand,
    },
    /// Backup Operant configuration and data
    Backup {
        #[command(subcommand)]
        cmd: cmd_backup::BackupSubcommand,
    },
    /// Import from a backup
    Import {
        #[command(subcommand)]
        cmd: cmd_import::ImportSubcommand,
    },
    /// Uninstall Operant data
    Uninstall {
        #[command(subcommand)]
        cmd: cmd_uninstall::UninstallSubcommand,
    },
    /// Check for and apply updates
    Update {
        #[command(subcommand)]
        cmd: cmd_update::UpdateSubcommand,
    },
    /// Show usage insights
    Insights {
        #[command(subcommand)]
        cmd: cmd_insights::InsightsSubcommand,
    },
    /// Manage webhook subscriptions
    Webhook {
        #[command(subcommand)]
        cmd: cmd_webhook::WebhookSubcommand,
    },
    /// Manage shell hooks
    Hooks {
        #[command(subcommand)]
        cmd: cmd_hooks::HooksSubcommand,
    },
    /// Generate debug reports
    Debug {
        #[command(subcommand)]
        cmd: cmd_debug::DebugSubcommand,
    },
    /// Manage installed plugins
    Plugins {
        #[command(subcommand)]
        cmd: cmd_plugins::PluginsSubcommand,
    },
    /// Inspect the resolved harness architecture (plan 016)
    Architecture {
        #[command(subcommand)]
        cmd: cmd_architecture::ArchitectureSubcommand,
    },
    /// Manage communication channels (telegram, discord, slack, whatsapp)
    Channel {
        #[command(subcommand)]
        cmd: cmd_channel::ChannelSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage standard operating procedures (SOPs)
    Sop {
        #[command(subcommand)]
        cmd: cmd_sop::SopSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    // (plan 10) Hardware / Peripheral subcommands deleted with the
    // operant-hardware crate — STM32/Arduino flashing is not operant's
    // surface (the audit's default).
    /// Migrate data from other agent runtimes
    Migrate {
        #[command(subcommand)]
        cmd: cmd_migrate::MigrateSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage OS service lifecycle (systemd/launchd)
    Service {
        #[command(subcommand)]
        cmd: cmd_service::ServiceSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Manage the skill curator
    Curator {
        #[command(subcommand)]
        cmd: cmd_curator::CuratorSubcommand,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Interactive setup wizard
    Setup {
        /// Optional setup section (provider, terminal, tts, gateway, agent,
        /// governance)
        section: Option<String>,
        #[arg(long)]
        non_interactive: bool,
        #[arg(long)]
        reset: bool,
        #[arg(long)]
        reconfigure: bool,
        #[arg(long)]
        quick: bool,
        /// Output as JSON (for scripting/CI)
        #[arg(long, global = true)]
        json: bool,
    },
    /// Run the ACP server
    Acp {
        #[command(subcommand)]
        cmd: cmd_acp::AcpSubcommand,
    },
    /// Run the web dashboard
    Dashboard {
        #[command(subcommand)]
        cmd: cmd_dashboard::DashboardSubcommand,
    },
    /// Manage agent trajectories (ReAct step recordings for fine-tuning)
    Trajectory {
        #[command(subcommand)]
        cmd: cmd_trajectory::TrajectorySubcommand,
    },
    /// TUI debugging tools — simulate every TUI overlay from the CLI.
    ///
    /// Each `debug` subcommand runs the same data-loading path the TUI uses
    /// for the corresponding overlay, but prints to stdout instead of rendering.
    /// Use this to verify an overlay's data loads correctly without entering
    /// the TUI, or to debug a broken overlay from the shell.
    ///
    /// Action subcommands (effort, mode, output-style, theme, vim, keybindings,
    /// voice) set TUI state persistently, closing the TUI↔CLI parity gaps.
    Tui {
        #[command(subcommand)]
        cmd: cmd_tui_debug::TuiSubcommand,
    },
    /// Support development 💜
    Donate,
}

/// The size at which a configured log file is rotated.
///
/// A configured `logging.log_file` is opened in APPEND mode and was never
/// rotated: the date-stamped `operant-*.log` files the TUI logger writes
/// are pruned after 7 days (`cleanup_old_logs_in`), but a CONFIGURED file
/// was not on that list, so it grew without bound. Measured on this box:
/// `~/.operant/logs/gateway.log` had reached 90 MB and 1.7M lines after
/// about three weeks of the gateway running.
const LOG_FILE_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// How many rotated generations of a log file are kept.
///
/// One live file plus two rotated ones: the current one, the previous one
/// (for the incident that triggered the rotation) and the one before it.
/// A rotation renames `foo.log` to `foo.log.1` and `foo.log.1` to
/// `foo.log.2`, dropping whatever was at `.2`.
const LOG_FILE_GENERATIONS: usize = 2;

/// A `Write` target that rotates the log file when it grows past the cap.
///
/// `tracing_appender::rolling` does this, but it is not a dependency and
/// adding one for a size check is not worth it. The check is on the WRITE
/// path rather than on a timer: a daemon that emits a burst between timer
/// ticks would otherwise overshoot the cap by the whole burst, and the
/// point of the cap is to bound the file.
struct RollingLogFile {
    path: PathBuf,
    file: File,
    written: u64,
    max_bytes: u64,
    /// The bytes written so far, tracked so the size check does not stat
    /// the file on every line.
    rotated: bool,
}

impl RollingLogFile {
    fn open(path: &Path, max_bytes: u64) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            file,
            written,
            max_bytes,
            rotated: false,
        })
    }

    /// The path of the `generation`-th rotated file: `foo.log` →
    /// `foo.log.1` → `foo.log.2`.
    ///
    /// The extension is rebuilt rather than appended to, because
    /// `Path::with_extension` REPLACES the extension: `foo.log` with
    /// `with_extension("log.1")` is `foo.log.1`, which is what we want.
    fn generation_path(&self, generation: usize) -> PathBuf {
        let ext = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{e}.{generation}"))
            .unwrap_or_else(|| generation.to_string());
        self.path.with_extension(ext)
    }

    /// Move the current file aside and start a fresh one.
    ///
    /// The rotation is `foo.log` → `foo.log.1` → `foo.log.2`, dropping
    /// whatever was at `.2`. A rename on the same filesystem is atomic, so
    /// a reader never sees a half-written log.
    fn rotate(&mut self) {
        let _ = std::fs::remove_file(self.generation_path(LOG_FILE_GENERATIONS));

        for generation in (1..LOG_FILE_GENERATIONS).rev() {
            let _ = std::fs::rename(
                self.generation_path(generation),
                self.generation_path(generation + 1),
            );
        }
        let _ = std::fs::rename(&self.path, self.generation_path(1));

        // Re-open the (now absent) path in append mode, which creates it.
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                self.file = file;
                self.written = 0;
            }
            Err(_) => {
                // The rename failed, or the path cannot be re-created.
                // Keep writing to the OLD handle: a log that keeps growing
                // is better than a daemon that cannot log at all.
                self.rotated = true;
            }
        }
    }
}

impl Write for RollingLogFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.written + buf.len() as u64 > self.max_bytes && !self.rotated {
            self.rotate();
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn init_logging(
    verbose: bool,
    cli_log_level: Option<&str>,
    logging: &LoggingSettings,
    rich_output: bool,
) -> Result<()> {
    let env_filter = if verbose {
        EnvFilter::new(format!("{}", Level::DEBUG))
    } else if let Some(level) = cli_log_level {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(logging.level.clone()))
    };

    let subscriber = tracing_subscriber::registry().with(env_filter);
    let layer = fmt::layer()
        .with_target(logging.with_target)
        .with_thread_ids(logging.with_thread_ids)
        .with_file(logging.with_file)
        .with_line_number(logging.with_line_number);

    // When --verbose is passed, force logs to stderr so they are visible
    // even when tui.rich_output is true (which normally routes logs to sink).
    let effective_rich_output = rich_output && !verbose;
    // An empty `log_file` string is "unset" — the example template ships
    // `log_file = ""` as its documented default, and Some("") would
    // otherwise route to the File target and fail to open.
    let logging = if logging.log_file.as_deref().is_some_and(str::is_empty) {
        let mut resolved = logging.clone();
        resolved.log_file = None;
        resolved
    } else {
        logging.clone()
    };
    let logging = &logging;
    // (plan 2026-10-25 tooling-surface, RC6) A rich TUI with no configured
    // log file used to route tracing to a discard sink: a default-config
    // TUI run logged NOTHING to disk, so agent-loop warnings and guardrail
    // decisions were unrecoverable after the fact (the 2026-10-24 storm
    // was only diagnosable because the operator had configured one).
    // Default the interactive TUI to <home>/logs/operant.log — the
    // rotation cap bounds it; an explicit `[logging] log_file` still wins.
    // If the DEFAULT path cannot be opened, degrade to the sink rather
    // than failing startup: the user never asked for this file. The probe
    // handle is dropped immediately; the File arm below reopens for real.
    let logging = if tui_wants_default_log_file(logging, effective_rich_output) {
        let default_path = default_tui_log_file();
        if let Some(dir) = default_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&default_path)
        {
            Ok(_) => {
                let mut resolved = logging.clone();
                resolved.log_file = Some(default_path.to_string_lossy().into_owned());
                resolved
            }
            Err(_) => logging.clone(),
        }
    } else {
        logging.clone()
    };
    match select_log_target(&logging, effective_rich_output) {
        LogTarget::File => {
            let path = logging
                .log_file
                .as_ref()
                .context("log target is a file but no log_file is configured")?;
            // A configured log file is opened in APPEND mode and was never
            // rotated, so it grew without bound: the date-stamped
            // `operant-*.log` files are pruned after 7 days, but a
            // CONFIGURED file was not on that list. Measured on this box:
            // `~/.operant/logs/gateway.log` had reached 90 MB and 1.7M
            // lines after about three weeks.
            let file = RollingLogFile::open(Path::new(path), LOG_FILE_MAX_BYTES)
                .with_context(|| format!("failed to open log file {path}"))?;
            let writer = Mutex::new(file);
            match logging.format.as_str() {
                "json" => subscriber
                    .with(layer.with_writer(writer).with_ansi(false).json())
                    .init(),
                "compact" => subscriber
                    .with(layer.with_writer(writer).with_ansi(false).compact())
                    .init(),
                _ => subscriber
                    .with(layer.with_writer(writer).with_ansi(false).pretty())
                    .init(),
            }
        }
        LogTarget::Sink => match logging.format.as_str() {
            "json" => subscriber
                .with(layer.with_writer(io::sink).with_ansi(false).json())
                .init(),
            "compact" => subscriber
                .with(layer.with_writer(io::sink).with_ansi(false).compact())
                .init(),
            _ => subscriber
                .with(layer.with_writer(io::sink).with_ansi(false).pretty())
                .init(),
        },
        LogTarget::Stderr => match logging.format.as_str() {
            "json" => subscriber.with(layer.json()).init(),
            "compact" => subscriber.with(layer.compact()).init(),
            _ => subscriber.with(layer.pretty()).init(),
        },
    }
    Ok(())
}

fn select_log_target(logging: &LoggingSettings, rich_output: bool) -> LogTarget {
    if logging.log_file.is_some() {
        LogTarget::File
    } else if rich_output {
        LogTarget::Sink
    } else {
        LogTarget::Stderr
    }
}

/// The interactive TUI adopts the home log when the user configured
/// nothing: `[logging] log_file` unset and logs not forced to stderr by
/// `--verbose` (which stays visible by design). Testable seam for the
/// RC6 default.
fn tui_wants_default_log_file(logging: &LoggingSettings, effective_rich_output: bool) -> bool {
    effective_rich_output && logging.log_file.is_none()
}

/// Where the default TUI log lives: the canonical home log, next to the
/// gateway's, under the one resolver.
fn default_tui_log_file() -> std::path::PathBuf {
    operant_core::platform::operant_home()
        .join("logs")
        .join("operant.log")
}

fn apply_cli_overrides(cli: &Cli, config: &mut AppConfig) {
    if let Some(api_key) = &cli.api_key {
        config.client.api_key = Some(api_key.clone());
    }
    if let Some(base_url) = &cli.base_url {
        config.client.base_url = base_url.clone();
    }
    if let Some(model) = &cli.model {
        config.agent.model = model.clone();
    }
    if let Some(max_iterations) = cli.max_iterations {
        config.agent.max_iterations = max_iterations;
    }
    if let Some(timeout) = cli.tool_timeout {
        config.agent.tool_timeout_secs = timeout;
    }
    if let Some(timeout) = cli.request_timeout {
        config.agent.request_timeout_secs = timeout;
        config.client.timeout_secs = timeout;
    }
    if let Some(window) = cli.context_window {
        config.agent.context_window = window;
        config.client.max_context_length = window;
    }
    if let Some(healing) = cli.max_healing_attempts {
        config.agent.max_healing_attempts = healing;
    }
    if cli.stream {
        config.agent.stream = true;
    }
    if cli.no_stream {
        config.agent.stream = false;
    }
}

/// Create the appropriate ModelClient based on the provider.
fn create_model_client(
    provider: &str,
    config: &AppConfig,
) -> Box<dyn operant_core::agent::ModelClient> {
    match provider {
        #[cfg(feature = "anthropic")]
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .unwrap_or_else(|_| config.client.api_key.clone().unwrap_or_default());
            Box::new(
                AnthropicModelClient::new(api_key).with_cache_ttl(config.client.prompt_cache_ttl),
            )
        }
        _ => {
            // Default to OpenAI-compatible client for openai, deepseek, and others
            Box::new(OpenAIModelClient::new(OpenAIClient::new(client_config(
                config,
            ))))
        }
    }
}

/// Layer a named provider profile's overrides onto the primary client config.
/// Absent fields inherit the primary (base URL, key, timeout) so a profile
/// only needs to specify what differs (hermes `fallback_providers` parity).
fn client_config_from_provider(
    primary: &ClientConfig,
    profile: &ModelProviderConfig,
) -> ClientConfig {
    let mut ccfg = primary.clone();
    if let Some(base) = profile.base_url.as_deref().filter(|b| !b.is_empty()) {
        ccfg.base_url = base.to_string();
    }
    if let Some(key) = profile.api_key.as_deref().filter(|k| !k.is_empty()) {
        ccfg.api_key = Some(key.to_string());
    }
    if let Some(secs) = profile.timeout_secs.filter(|s| *s > 0) {
        ccfg.timeout = Duration::from_secs(secs);
    }
    ccfg
}

/// Build the per-provider credential-pool registry (hermes `load_pool`
/// parity). Every provider that can carry a key gets its own pool, seeded
/// from config, so same-provider multi-key rotation works under free-tier
/// pressure *and* each fallback provider in the chain rotates its own keys
/// instead of failing on the first 429.
///
/// Seeds:
/// * primary provider — `client.api_key` + `client.additional_api_keys` +
///   the provider's canonical env var;
/// * every `[providers] models.<name>` profile — its `api_key` (the effective
///   key after inheritance, i.e. profile key or primary key).
fn build_pool_registry(
    config: &AppConfig,
    primary_provider: &str,
    primary_cfg: &ClientConfig,
) -> std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry> {
    use operant_core::credential_pool::CredentialPoolRegistry;
    let registry = CredentialPoolRegistry::new(config.credential_pool.clone());

    // Primary provider: configured key + additional keys + env var.
    if let Some(key) = primary_cfg
        .api_key
        .as_deref()
        .filter(|k| !k.trim().is_empty())
    {
        registry.add_seed(primary_provider, "primary", key, "config (client.api_key)");
    }
    for (i, key) in config.client.additional_api_keys.iter().enumerate() {
        registry.add_seed(
            primary_provider,
            &format!("additional-{i}"),
            key,
            "config (client.additional_api_keys)",
        );
    }
    registry.add_seed(
        primary_provider,
        &crate::cmd_auth::provider_env_var(primary_provider),
        &std::env::var(crate::cmd_auth::provider_env_var(primary_provider)).unwrap_or_default(),
        "env",
    );

    // Profile providers: each gets its own pool keyed by its profile name,
    // seeded with the effective `api_key` plus the profile's own
    // `api_keys` vector (hermes multi-key-per-provider parity).
    for name in config.providers.models.keys() {
        let profile = &config.providers.models[name];
        let effective = client_config_from_provider(primary_cfg, profile);
        if let Some(key) = effective
            .api_key
            .as_deref()
            .filter(|k| !k.trim().is_empty())
        {
            registry.add_seed(
                name,
                &format!("{name}-primary"),
                key,
                "config ([providers] models)",
            );
        }
        for (i, key) in profile.api_keys.iter().enumerate() {
            registry.add_seed(
                name,
                &format!("{name}-extra-{i}"),
                key,
                "config ([providers] models api_keys)",
            );
        }
    }

    tracing::info!(
        providers = ?registry.providers(),
        "Per-provider credential pool registry ready"
    );
    std::sync::Arc::new(registry)
}

/// Wrap a provider client in a pooled client carrying that provider's
/// on-demand pool (hermes pool-per-provider parity). An empty pool degrades
/// to a passthrough, so wrapping is unconditional.
fn wrap_pooled(
    client: std::sync::Arc<dyn operant_core::agent::ModelClient>,
    provider: &str,
    registry: &std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>,
) -> std::sync::Arc<dyn operant_core::agent::ModelClient> {
    std::sync::Arc::new(operant_core::agent::PooledModelClient::new(
        client,
        registry.get_or_create(provider),
    ))
}

/// Build a [`ProviderRegistry`](operant_core::agent::provider_registry::ProviderRegistry) from the `[providers]` section — hermes
/// `fallback_providers` parity for cross-provider switching.
///
/// Chain order: primary (entry 0), then `providers.fallback_chain` in
/// configured order. Entries referencing an unknown profile key are skipped
/// with a warning. Returns `None` when no usable chain exists.
fn build_provider_registry(
    config: &AppConfig,
    primary_provider: &str,
    primary_model: &str,
    primary_cfg: &ClientConfig,
    pool_registry: &std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>,
) -> Option<std::sync::Arc<operant_core::agent::provider_registry::ProviderRegistry>> {
    use operant_core::agent::provider_registry::{ProviderEntry, ProviderRegistry};

    let models = &config.providers.models;
    if models.is_empty() && config.providers.fallback_chain.is_empty() {
        return None;
    }

    let mut clients: std::collections::HashMap<
        String,
        std::sync::Arc<dyn operant_core::agent::ModelClient>,
    > = std::collections::HashMap::new();
    // Primary client — entry 0 of the chain. Pooled so the primary provider
    // rotates its own keys too.
    let primary_arc = wrap_pooled(
        std::sync::Arc::new(OpenAIModelClient::new(OpenAIClient::new(
            primary_cfg.clone(),
        ))),
        primary_provider,
        pool_registry,
    );
    clients.insert(primary_provider.to_string(), primary_arc);

    // Profile clients, deterministic order (HashMap has no stable iteration).
    // Each is pooled with its own per-provider pool (hermes
    // `load_pool(fb_provider)` parity — the fallback provider rotates keys
    // under free-tier pressure instead of failing on the first 429).
    let mut names: Vec<&String> = models.keys().collect();
    names.sort();
    for name in names {
        let profile = &models[name];
        let ccfg = client_config_from_provider(primary_cfg, profile);
        let client = wrap_pooled(
            std::sync::Arc::new(OpenAIModelClient::new(OpenAIClient::new(ccfg))),
            name,
            pool_registry,
        );
        clients.insert(name.clone(), client);
    }

    let mut chain = vec![ProviderEntry {
        name: primary_provider.to_string(),
        model: primary_model.to_string(),
    }];
    for entry in &config.providers.fallback_chain {
        if !clients.contains_key(&entry.provider) {
            tracing::warn!(
                provider = %entry.provider,
                "providers.fallback_chain references an unknown profile (not in [providers.models]) — skipping"
            );
            continue;
        }
        if entry.model.is_empty() {
            continue;
        }
        chain.push(ProviderEntry {
            name: entry.provider.clone(),
            model: entry.model.clone(),
        });
    }
    if chain.len() < 2 {
        return None;
    }
    tracing::info!(
        chain_len = chain.len(),
        primary = %primary_provider,
        "cross-provider fallback chain active"
    );
    Some(std::sync::Arc::new(ProviderRegistry::new(clients, chain)))
}

/// Create the run-path model client wrapped with the configured fallback
/// chain (hermes `fallback_providers` parity):
///
/// * `agent.fallback_models` — same-client model swap on retryable errors
///   (5xx, 429, network).
/// * `[providers]` registry — cross-provider switch on auth/billing errors.
///
/// Falls back to a plain client when nothing is configured.
fn create_model_client_with_fallback(
    provider: &str,
    model: &str,
    config: &AppConfig,
    event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
) -> (
    Box<dyn operant_core::agent::ModelClient>,
    Option<std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>>,
) {
    let primary_cfg = client_config(config);
    let pool_registry = build_pool_registry(config, provider, &primary_cfg);
    // Primary client is pooled too: same-provider multi-key rotation happens
    // inside the client on 429/401/billing (hermes pool-per-provider parity).
    let primary_pooled: Box<dyn operant_core::agent::ModelClient> = Box::new(
        operant_core::agent::PooledModelClient::new(
            std::sync::Arc::from(create_model_client(provider, config)),
            pool_registry.get_or_create(provider),
        )
        // No key + a local endpoint is an unauthenticated server
        // (ollama, lm-studio), not a missing setup step.
        .with_keyless_ok(operant_core::client::is_local_endpoint(
            &primary_cfg.base_url,
        )),
    );

    let has_chain =
        !config.providers.models.is_empty() || !config.providers.fallback_chain.is_empty();
    if !config.agent.fallback_on_errors || (config.agent.fallback_models.is_empty() && !has_chain) {
        return (primary_pooled, Some(pool_registry));
    }

    let mut fallback = operant_core::agent::FallbackModelClient::new(
        std::sync::Arc::from(primary_pooled),
        model.to_string(),
        config.agent.fallback_models.clone(),
        true,
    );
    if let Some(registry) =
        build_provider_registry(config, provider, model, &primary_cfg, &pool_registry)
    {
        fallback = fallback.with_provider_registry(registry);
    }
    // Surface model/provider switches to the TUI (AgentEvent::ModelFallback).
    if let Some(tx) = event_tx {
        fallback = fallback.with_event_tx(tx);
    }
    (Box::new(fallback), Some(pool_registry))
}

pub(crate) fn client_config(config: &AppConfig) -> ClientConfig {
    ClientConfig::from(&config.client)
}

pub(crate) fn agent_config(
    config: &AppConfig,
    behavior: &BehaviorSettings,
    system_prompt: Option<&str>,
) -> AgentConfig {
    let mut agent = AgentConfig::from(behavior);
    if let Some(prompt) = system_prompt {
        agent.system_prompt = Some(prompt.to_string());
    }
    agent.request_timeout = Duration::from_secs(config.agent.request_timeout_secs);
    // Permanent tool-approval allowlist (hermes `command_allowlist` parity):
    // seed from the top-level `command_allowlist` config key and persist
    // "always allow" choices to a sidecar JSON in the operant data dir so
    // later sessions honor them too.
    agent.approval_allowlist = config.command_allowlist.clone();
    agent.approval_allowlist_path =
        Some(operant_core::platform::operant_home().join("approval_allowlist.json"));
    // Progressive tool disclosure (hermes parity): thread the CLI's
    // `tools.tool_search` block into the agent loop so MCP tool schemas
    // can be deferred behind the tool_search bridge. `mcp.deferred_loading
    // = false` is the master switch that forces eager loading regardless
    // of the tool_search block (hermes `mcp.deferred_loading` parity).
    let mut tool_search = config.tools.tool_search.clone();
    if !config.mcp.deferred_loading {
        tool_search.enabled = "off".to_string();
    }
    agent.tool_search = tool_search;
    agent
}

pub(crate) async fn build_registry(
    config: &AppConfig,
    mcp_manager: &McpManager,
    client: &OpenAIClient,
    model: &str,
    database: Arc<Database>,
    event_tx: Option<tokio::sync::mpsc::Sender<operant_core::agent::AgentEvent>>,
) -> Result<ToolRegistry> {
    let db_dir = config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let cron_path = db_dir.join("operant_cron.db");
    let kanban_path = db_dir.join("operant_kanban.db");
    let cron_db = Arc::new(CronDb::init(cron_path)?);
    let kanban_db = Arc::new(KanbanDb::init(kanban_path)?);
    let registry = ToolRegistry::new(Duration::from_secs(config.tools.registry_timeout_secs));
    let memory_dir = &config.skills.memory_dir;

    // Sub-agent delegation tool must inherit the parent's tool bans so
    // spawned children can never regain a disabled tool (hermes parity).
    // Passed explicitly because the bans are applied to the registry AFTER
    // registration below.
    let sub_agent_disabled_tools = config.tools.disabled_tools.iter().cloned().collect();
    let sub_agent_disabled_toolsets = config.tools.disabled_toolsets.iter().cloned().collect();

    operant_core::tools::register_builtin_tools_with_sub_agent(
        &registry,
        &config.skills.root_dir,
        memory_dir,
        client,
        model,
        database,
        cron_db,
        kanban_db,
        Some(mcp_manager.clone()),
        event_tx,
        sub_agent_disabled_tools,
        sub_agent_disabled_toolsets,
    )
    .await?;
    // (iter-153: EchoTool + CalculatorTool deleted — toy demo tools.
    // operant-core already has a real EchoTool in debug_helpers.rs.)

    // Phase E (docs/agentic-os/): native Holon tools — the agent's read/
    // submit surface for the organization, replacing the resident `holon mcp`
    // subprocess per run. Registration keys on the RUN ENV (audit O-03): a
    // Holon body runs with the adapter's posture configs, which carry no
    // [holon] section — a config gate here left every real body without its
    // native tools. The run env (HOLON_RUN_DIR/HOLON_BRIEF_DIR) is the only
    // signal the kernel actually controls; every tool's `is_available` gates
    // on the same env, so registering is inert outside a Holon run.
    if operant_core::platform::is_holon_run() || config.holon.enabled {
        operant_os::tools::register_all(&registry).await;
    }

    // Memory provider tools (agentmemory_*) are registered via the MCP
    // server path (config.mcp.servers → agentmemory) and/or the agent's
    // MemoryProvider hook — see load_memory_manager / create_runtime_agent.

    // Register AFT (Agent File Tools) if enabled in config.
    // AFT provides 18 IDE-grade coding tools (tree-sitter outline/zoom/
    // semantic search/callers, AST-aware edit/patch, safety undo/checkpoints)
    // via a subprocess that auto-updates from GitHub releases.
    //
    // Natural fallback: the basic file/terminal tools are ONLY disabled
    // when the aft bridge is proven live (a bounded ping through a spawned
    // bridge succeeds). If the binary is missing, broken, or the network
    // stalls, the native tools stay registered — the agent is never left
    // tool-less (audit finding: the old code disabled natives on the mere
    // success of registration, which happens even when the bridge later
    // fails to spawn).
    //
    // The replaced names are merged into `disabled_tools` BELOW — calling
    // `registry.disable_tool()` here is insufficient because the later
    // `set_disabled_tools(config...)` REPLACES the whole disabled set,
    // silently re-enabling them (found while verifying the fallback path).
    let mut aft_replaced_tools: Vec<String> = Vec::new();
    if config.tools.aft_enabled {
        let pool = std::sync::Arc::new(operant_core::aft_bridge::AftBridgePool::new());
        match operant_core::tools::register_aft_tools(&registry, pool.clone()).await {
            Ok(()) => {
                tracing::info!("AFT tools registered (18 IDE-grade coding tools)");
                // Callgraph-dependent tools (aft_callers, and aft_inspect's
                // dead-code projection) must wait out the persisted callgraph
                // cold-build on first use — the bridge retries the request
                // with backoff. Grant them a window that covers that build
                // instead of the generic 30s tool timeout, which killed them
                // mid-build in live testing.
                registry.set_tool_timeout("aft_callers", std::time::Duration::from_secs(180));
                registry.set_tool_timeout("aft_inspect", std::time::Duration::from_secs(180));
                let project_root =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let bridge_live = async {
                    match pool.get(&project_root).await {
                        Ok(bridge) => bridge
                            .call("ping", serde_json::json!({}))
                            .await
                            .map(|_| true)
                            .map_err(|e| {
                                tracing::warn!(error = %e, "AFT ping failed");
                                e
                            })
                            .is_ok(),
                        Err(e) => {
                            tracing::warn!(error = %e, "AFT bridge unavailable at startup");
                            false
                        }
                    }
                };
                let live =
                    match tokio::time::timeout(std::time::Duration::from_secs(10), bridge_live)
                        .await
                    {
                        Ok(live) => live,
                        Err(_) => {
                            tracing::warn!(
                                "AFT bridge ping timed out (10s) — keeping native tools"
                            );
                            false
                        }
                    };
                if live {
                    // AFT provides superior versions of the basic tools:
                    //   aft_read → file_read, aft_write → file_write,
                    //   aft_edit → patch, aft_bash → terminal,
                    //   aft_search → file_search, aft_glob → file_list
                    aft_replaced_tools.extend([
                        "file_read".to_string(),
                        "file_write".to_string(),
                        "patch".to_string(),
                        "terminal".to_string(),
                        "file_search".to_string(),
                        "file_list".to_string(),
                    ]);
                    tracing::info!(
                        "AFT verified live — basic file/terminal tools disabled (aft replaces them)"
                    );
                } else {
                    tracing::warn!(
                        "AFT not operational at startup — keeping native tools as fallback"
                    );
                }
            }
            Err(e) => tracing::warn!(
                error = %e,
                "AFT tool registration failed (non-fatal) — native tools remain"
            ),
        }
    }

    // WASM plugin tools: discover tool-capable plugins and register their
    // WasmTools into the registry (feature `plugins-wasm`). Best-effort — a
    // broken plugin is skipped, never fatal to startup.
    #[cfg(feature = "plugins-wasm")]
    {
        if let Err(e) = plugin_tools::register_plugin_tools(&registry, config).await {
            tracing::warn!(error = %e, "plugin tool registration failed (non-fatal)");
        }
    }

    // LCM context-engine tools: when agent.context_engine = "lcm" the
    // lossless DAG is active, so expose lcm_recall / lcm_stats to the model.
    // This engine instance is a read-only peer of the one attached to the
    // agent (same WAL database file) — recall is consistent either way.
    if config.agent.context_engine.as_str() == "lcm" {
        match operant_core::context::LcmContextEngine::new(lcm_config(config)) {
            Ok(engine) => {
                let engine = std::sync::Arc::new(engine);
                // P3 vector recall: when an embedding model is configured,
                // register lcm_vector_recall with an embedder. "local:hash"
                // uses the zero-dependency built-in (no external embedding
                // service); anything else is an OpenAI-compatible /embeddings
                // endpoint, defaulting to the chat provider's base URL and
                // key unless context_lcm_embedding_base_url overrides it.
                let embedder = config.agent.context_lcm_embedding_model.as_ref().map(|m| {
                    if m.trim() == "local:hash" {
                        tracing::info!(
                            "lcm_vector_recall: using zero-dependency local:hash embedder"
                        );
                        return std::sync::Arc::new(
                            operant_core::context::LocalHashEmbedder::default(),
                        )
                            as std::sync::Arc<dyn operant_core::context::Embedder>;
                    }
                    if config.agent.context_lcm_embedding_base_url.is_none() {
                        // The /embeddings call goes to the chat provider's
                        // endpoint. Many chat-only providers don't expose it
                        // — warn loudly so the failure isn't mysterious.
                        tracing::warn!(
                            "lcm_vector_recall: embedding model '{m}' will use the chat provider's \
                             base URL and key for /embeddings (set \
                             agent.context_lcm_embedding_base_url to a dedicated embeddings \
                             endpoint, or use \"local:hash\" for a zero-dependency embedder)"
                        );
                    }
                    let mut ccfg = client_config(config);
                    if let Some(eb) = &config.agent.context_lcm_embedding_base_url {
                        ccfg.base_url = eb.clone();
                    }
                    let client = OpenAIClient::new(ccfg);
                    std::sync::Arc::new(operant_core::context::OpenAIEmbedder::new(
                        client,
                        m.clone(),
                    )) as std::sync::Arc<dyn operant_core::context::Embedder>
                });
                // P3 assertion extraction (hermes ModelAssertionExtractor
                // parity, opt-in): when `context_lcm_assertion_extraction` is
                // true, lcm_assert gains action=extract — an LLM call over
                // the main agent model mines durable facts from recent DAG
                // nodes. Off by default (no extra LLM cost on stock installs).
                let extractor = if config.agent.context_lcm_assertion_extraction {
                    let llm = std::sync::Arc::new(OpenAIClient::new(client_config(config)));
                    Some(
                        std::sync::Arc::new(operant_core::context::LlmAssertionExtractor::new(
                            llm,
                            config.agent.model.clone(),
                        ))
                            as std::sync::Arc<dyn operant_core::context::AssertionExtractor>,
                    )
                } else {
                    None
                };
                match operant_core::tools::register_lcm_tools(
                    &registry, engine, embedder, extractor,
                )
                .await
                {
                    Ok(()) => {
                        // lcm_assert action=extract runs a reasoning-model LLM
                        // completion over recent DAG nodes — the generic 30s
                        // tool timeout killed it mid-thought in live testing
                        // (the model burns its budget on reasoning_content
                        // first). Grant the tool a longer execution window;
                        // save/query actions return in milliseconds regardless.
                        registry
                            .set_tool_timeout("lcm_assert", std::time::Duration::from_secs(180));
                        tracing::info!(
                            "LCM tools registered (lcm_recall / lcm_stats / lcm_assert / lcm_recall_round / lcm_recent / lcm_doctor) — lossless DAG active"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "LCM tool registration failed (non-fatal)")
                    }
                }
            }
            Err(e) => tracing::warn!(
                error = %e,
                "LCM engine init failed for tools — lcm_recall/lcm_stats unavailable"
            ),
        }
    }

    // Interactive tools block on HUMAN input (inline-keyboard taps / button
    // answers), not compute — the generic 30s cap killed real interactions
    // mid-prompt in live gateway testing (clarify timed out while the user
    // was still reading the question). Grant a 5-minute window; the channel
    // layer's own timeouts still bound the worst case.
    registry.set_tool_timeout("clarify", std::time::Duration::from_secs(300));
    registry.set_tool_timeout("approval_request", std::time::Duration::from_secs(300));

    // iter-1124: the disabled set is the TOML list UNION the CLI state file
    // (`operant tools disable <name>` writes `tool_state.json`). Before this
    // the file was read by nobody, so the CLI's disable was a no-op.
    let disabled_toolsets: std::collections::HashSet<String> =
        config.tools.disabled_toolsets.iter().cloned().collect();

    // Merge the AFT-replaced native tools into the config-driven set so a
    // single `set_disabled_tools` (which REPLACES, not unions) applies both.
    operant_core::tools::tool_state::apply_disabled_tools(
        &registry,
        config,
        aft_replaced_tools.into_iter().collect(),
    )
    .await;
    if !disabled_toolsets.is_empty() {
        registry.set_disabled_toolsets(disabled_toolsets).await;
    }

    if config.mcp.autoload {
        // Deferred servers (e.g. the injected agentmemory server) are NOT
        // connected here — they would spawn `npx @agentmemory/mcp` on every
        // invocation. They stay connectable on demand via `operant mcp`.
        for server in config
            .mcp
            .servers
            .iter()
            .filter(|server| server.enabled && !server.deferred)
        {
            if !mcp_manager.contains(&server.name).await {
                // A server that cannot be reached is NOT fatal. One broken
                // MCP server (a stale URL, a binary that is not installed, a
                // host that is down) used to abort the whole command with
                // `?`, so `operant tools list` and every registry-building
                // command exited 1 and the agent could not start at all —
                // for a tool the user may not even use. The failure is
                // recorded in the manager (so the MCP view can show it) and
                // reported here; the rest of the registry still builds.
                if let Err(e) = connect_mcp_server(mcp_manager, server).await {
                    tracing::warn!(
                        server = %server.name,
                        error = %e,
                        "MCP server failed to connect; continuing without it"
                    );
                }
            }
        }

        mcp_manager.sync_tools_to_registry(&registry).await;

        // G4 (hermes mcp_stdio_watchdog.py parity): when at least one
        // non-deferred stdio server is autoloaded, spawn a background
        // watchdog that detects crashed children (process exit) and
        // auto-reconnects them, re-syncing their tools into the registry.
        // Interval from `[mcp] watchdog_interval_secs` (0 disables). The
        // task dies with the process, so short-lived commands are unaffected.
        if config.mcp.watchdog_interval_secs > 0
            && config
                .mcp
                .servers
                .iter()
                .any(|s| s.enabled && !s.deferred && s.transport == McpTransportKind::Stdio)
        {
            mcp_manager.spawn_watchdog(
                registry.clone(),
                Duration::from_secs(config.mcp.watchdog_interval_secs),
            );
        }
    }

    Ok(registry)
}

async fn connect_mcp_server(mcp_manager: &McpManager, server: &McpServerConfig) -> Result<()> {
    match server.transport {
        McpTransportKind::Http => {
            let url = server
                .url
                .clone()
                .context("Configured HTTP MCP server is missing a URL")?;
            mcp_manager
                .add_server(server.name.clone(), url, server.auth_token.clone())
                .await?;
        }
        McpTransportKind::StreamableHttp => {
            // The streamable-HTTP transport (MCP spec 2025-06-18) needs
            // its own client: the plain-HTTP one sends no `Accept`
            // header, so a spec-compliant server answers `406 Not
            // Acceptable`, and it never echoes the `Mcp-Session-Id`, so
            // every request starts a new session.
            let url = server
                .url
                .clone()
                .context("Configured streamable-HTTP MCP server is missing a URL")?;
            mcp_manager
                .add_streamable_server(server.name.clone(), url, server.auth_token.clone())
                .await?;
        }
        McpTransportKind::Stdio => {
            let command = server
                .command
                .clone()
                .context("Configured stdio MCP server is missing a command")?;
            mcp_manager
                .add_stdio_server(
                    server.name.clone(),
                    command,
                    server.args.clone(),
                    server.env.clone(),
                )
                .await?;
        }
    }
    Ok(())
}

/// Shared components produced by the agent core builder.
struct AgentCore {
    database: Arc<Database>,
    registry: ToolRegistry,
    agent_config: AgentConfig,
    memory_manager: MemoryManager,
    /// Long-term memory provider (agentmemory/builtin). Attached to the
    /// agent so sync_turn/prefetch/session hooks actually fire — this was
    /// previously built-and-dropped (see docs/AUDIT_2026-08-02.md F1).
    memory_provider: Option<Arc<dyn MemoryProvider>>,
    skill_manager: SkillManager,
    /// Plan 016 G1 — composable provider runtime. Constructed when
    /// `config.harness.enabled = true`; `None` (dark-merge default) keeps
    /// the existing `ToolRegistry` / `HookRunner` paths untouched.
    harness: Option<Arc<operant_harness::Harness>>,
    /// C1 — kernel prompt-section slot, present only when the harness
    /// booted; handed to the agent builder so provider-installed sections
    /// render into the frozen prefix.
    harness_prompt_slot: Option<Arc<operant_core::harness_slots::PromptSlot>>,
    /// C1 — hook registry carrying the kernel hook bridge. `None` when
    /// the harness is disabled (the agent then emits no hook events, as
    /// before this wiring).
    hook_registry: Option<Arc<operant_core::gateway_pipeline::HookRegistry>>,
}

/// Build the shared core components needed by both agent constructors.
///
/// Single-agent default: the repo memory pair (fresh `MemoryManager` +
/// provider) is loaded internally per invocation. Hosts that build a SECOND
/// agent in the same process (gateway + cron) must NOT use this directly —
/// construct the pair once and thread it through
/// [`build_agent_core_with_memory`] so both agents share one memory
/// backend (BUGS.md D-5).
async fn build_agent_core(
    config: &AppConfig,
    system_prompt: Option<&str>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
    model_name: &str,
    behavior: &BehaviorSettings,
    event_tx: Option<tokio::sync::mpsc::Sender<operant_core::agent::AgentEvent>>,
) -> Result<AgentCore> {
    let (memory_manager, memory_provider) = load_repo_memory_manager().await?;
    build_agent_core_with_memory(
        config,
        system_prompt,
        mcp_manager,
        skills_dir,
        model_name,
        behavior,
        event_tx,
        memory_manager,
        memory_provider,
    )
    .await
}

/// BUGS.md D-5: build the agent core with a CALLER-SUPPLIED memory pair.
///
/// Loading per invocation (what `build_agent_core` does) gave a two-agent
/// process two `MemoryManager`s whole-file-writing one MEMORY.md, a second
/// build repointing the global `ACTIVE_MEMORY_MANAGER` that the first
/// agent's already-attached memory tools read, and a second provider
/// instance over the same store. The fix is hoisting: the host constructs
/// the `(MemoryManager, Option<Arc<dyn MemoryProvider>>)` pair ONCE and
/// threads the SAME pair through this overload for every agent it builds.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit dependency injection; a config struct would only rename the arguments"
)]
async fn build_agent_core_with_memory(
    config: &AppConfig,
    system_prompt: Option<&str>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
    model_name: &str,
    behavior: &BehaviorSettings,
    event_tx: Option<tokio::sync::mpsc::Sender<operant_core::agent::AgentEvent>>,
    memory_manager: MemoryManager,
    memory_provider: Option<Arc<dyn MemoryProvider>>,
) -> Result<AgentCore> {
    let raw_client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = build_registry(
        config,
        mcp_manager,
        &raw_client,
        model_name,
        database.clone(),
        event_tx,
    )
    .await?;
    let agent_config = agent_config(config, behavior, system_prompt);

    // Hermes-plugin parity: register the memory provider's own tool schemas
    // (memory_smart_search, memory_save, ...) directly in the registry. The
    // injected agentmemory MCP server is deferred (lazy), so these keep the
    // memory surface available to the model without spawning npx at startup.
    if let Some(provider) = &memory_provider {
        memory_provider_tools::register_provider_tools(&registry, provider.clone()).await;
    }

    // R-09 (AUDIT-2026-10-16-capability-layer): a Holon run's skill root is
    // the run-local, department-resolved library the kernel exported into
    // runs/<id>/skills and named in brief.json — one copy of every skill,
    // the department's fork included. A Holon brief that names none (a
    // kernel older than the field) lists no skills at all: the brief is
    // the render, never the org-wide standard export with a stale twin
    // beside every fork. Personal mode keeps config.skills.root_dir
    // byte-identical.
    let holon_skills = if operant_core::platform::is_holon_run() {
        Some(operant_os::run_env::holon_skills_dir().unwrap_or_else(|| {
            operant_core::platform::holon_run_dir()
                .unwrap_or_else(|| std::path::PathBuf::from(""))
                .join("skills")
        }))
    } else {
        None
    };
    let skills_dir: &Path = holon_skills.as_deref().unwrap_or(skills_dir);

    let mut skill_manager = SkillManager::new(skills_dir.to_path_buf());
    if let Err(e) = skill_manager.load_all() {
        warn!(
            error = %e,
            "Failed to load skills from {}",
            skills_dir.display()
        );
    }

    // Plan 016 G1 — adopt the harness when [harness].enabled = true.
    // The kernel is constructed, the tool seam is registered against the
    // same `ToolRegistry` the agent loop already uses, and (if the
    // operator configured a path) the `architecture.toml` is loaded and
    // every provider in it is mounted. When `enabled = false` (default)
    // `harness` stays None and the agent loop is byte-identical to the
    // pre-harness path.
    let harness = build_harness_host(config, registry.clone()).await;
    // C1 — the kernel's slots ride alongside the harness. The prompt slot
    // is attached to the agent builder (its sections render into the
    // frozen prefix); the hook slot is bridged onto the agent's hook
    // registry, which previously had no production constructor at all —
    // provider-installed hooks had nowhere to fire.
    let harness_prompt_slot = harness
        .as_ref()
        .map(|(_, slot, _)| std::sync::Arc::clone(slot));
    // iter-1125: the hook registry is ALWAYS created. It used to exist only
    // when the harness was enabled, so in a normal install NO hook could
    // fire at all — and the operator's own shell hooks (`operant hooks
    // add`) had nowhere to be registered.
    let hook_registry = Arc::new(operant_core::gateway_pipeline::HookRegistry::new());
    if let Some((_, _, hook_slot)) = harness.as_ref() {
        operant_core::gateway_pipeline::HookRegistry::register(
            &hook_registry,
            operant_core::gateway_pipeline::HookEvent::Hooks("*".to_string()),
            operant_core::harness_slots::HookSlot::bridge_handler(std::sync::Arc::clone(hook_slot)),
        )
        .await;
    }
    // The operator's shell hooks: one file per hook in the hooks directory,
    // run when the agent emits their event.
    operant_core::shell_hooks::install_shell_hooks(&hook_registry).await;
    let harness = harness.map(|(h, _, _)| h);
    // S2 — make harness_dump/mount/unmount visible to the model. The
    // tools themselves gate mount/unmount on approval (ToolContext.metadata["approval"]).
    if let Some(h) = &harness
        && let Err(e) =
            operant_core::tools::harness_tools::register_harness_tools(&registry, h.clone()).await
    {
        warn!(error = %e, "harness: tool registration failed");
    }

    Ok(AgentCore {
        database,
        registry,
        agent_config,
        memory_manager,
        memory_provider,
        skill_manager,
        harness,
        harness_prompt_slot,
        hook_registry: Some(hook_registry),
    })
}

/// Plan 016 G1 — construct a `Harness` + `HarnessHost` when
/// `config.harness.enabled = true`. Returns `None` when disabled (the
/// dark-merge default) so callers can treat the harness as opt-in.
///
/// Currently registered seams:
/// * `tool` — routed through the same `ToolRegistry` the agent loop
///   already consults. This is what makes the kernel adoption
///   transparent: harness-installed tools show up in the registry's
///   `get_schemas()` and execute via the same `ToolRegistry::execute`
///   path, while their lifecycle (mount/unmount/replace) is owned by
///   the kernel.
async fn build_harness_host(
    config: &AppConfig,
    registry: ToolRegistry,
) -> Option<(
    Arc<operant_harness::Harness>,
    Arc<operant_core::harness_slots::PromptSlot>,
    Arc<operant_core::harness_slots::HookSlot>,
)> {
    if !config.harness.enabled {
        return None;
    }

    use operant_core::harness_adapters::ToolSeam;
    use operant_core::harness_seams_r3::{GatewayCommandSeam, MemoryProviderSeam};
    use operant_core::harness_slots::{HookSlot, HookSlotSeam, PromptSlot, PromptSlotSeam};
    use operant_harness::{
        BuilderWithFactories, Harness, HarnessHost, HarnessMetrics, KernelOptions,
        PoolBundleProvider, PoolFamilyProvider,
    };

    // S6 — metrics. Host can read snapshot; we keep the Arc for the harness.
    let metrics = std::sync::Arc::new(HarnessMetrics::new());
    let harness = std::sync::Arc::new(
        Harness::new(KernelOptions { audit: true }).with_metrics(metrics.clone()),
    );
    let mut host = HarnessHost::with_harness(harness);
    host.add_seam(std::sync::Arc::new(ToolSeam::new(registry)));
    // S1 — R3 seams (tool-adjacent families) — additive, no host required.
    host.add_seam(std::sync::Arc::new(MemoryProviderSeam::new()));
    host.add_seam(std::sync::Arc::new(GatewayCommandSeam::new()));
    // C1 — prompt/hook seams against slots the live agent consumes: the
    // prompt slot lands in the agent's frozen prefix, the hook slot is
    // bridged onto the agent's HookRegistry (kernel hooks fire on the
    // live loop's events). Provider-installed sections/hooks stay inert
    // until the caller attaches the slot — boot is safe on its own.
    let prompt_slot = std::sync::Arc::new(PromptSlot::new());
    host.add_seam(std::sync::Arc::new(PromptSlotSeam::new(
        std::sync::Arc::clone(&prompt_slot),
    )));
    let hook_slot = std::sync::Arc::new(HookSlot::new());
    host.add_seam(std::sync::Arc::new(HookSlotSeam::new(
        std::sync::Arc::clone(&hook_slot),
    )));

    // S1+S5 — factories for wasm/pool. Pool dispatches by kind so family
    // late-binding and bundle tool materialization both work.
    let mut builder = BuilderWithFactories::new();
    builder.register_factory(
        "pool",
        std::sync::Arc::new(|row: operant_harness::ArchitectureRow| {
            match row.kind.as_deref() {
                Some("pool.bundle") => Ok(std::sync::Arc::new(PoolBundleProvider::new(row))
                    as std::sync::Arc<dyn operant_harness::Provider>),
                Some("pool.family") | None => {
                    // pool rows without kind (or family) are family claims holders.
                    Ok(std::sync::Arc::new(PoolFamilyProvider::new(row))
                        as std::sync::Arc<dyn operant_harness::Provider>)
                }
                Some(other) => Err(operant_harness::BuildError::NoConfigRowHandler(
                    row.id.clone(),
                    other.to_string(),
                )),
            }
        }),
    );
    // WASM rows without a host Extism factory fall back to NativeRowStub
    // (preserves dump shape); a real Extism host can replace this factory.

    // S3 — resolve architecture. If the user configured an explicit path, use it.
    // Otherwise try default locations: ./architecture.toml, ~/.operant/architecture.toml,
    // then fall back to the example file if it exists. Empty is still valid (dark merge).
    let patch_dir = operant_harness::default_patch_dir();
    let arch = if let Some(path) = &config.harness.architecture_toml {
        match operant_harness::resolve_boot_architecture(path, patch_dir.as_deref()) {
            Ok(a) => a,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "harness: architecture.toml + patches failed to resolve — kernel will start empty"
                );
                return Some((
                    host.harness().clone(),
                    std::sync::Arc::clone(&prompt_slot),
                    std::sync::Arc::clone(&hook_slot),
                ));
            }
        }
    } else {
        // Try default file locations before giving up.
        let candidates = [
            std::path::PathBuf::from("architecture.toml"),
            operant_core::platform::operant_home().join("architecture.toml"),
            std::path::PathBuf::from("operant/architecture.toml"),
        ];
        let mut resolved: Option<operant_harness::Architecture> = None;
        for cand in &candidates {
            if cand.exists() {
                match operant_harness::resolve_boot_architecture(cand, patch_dir.as_deref()) {
                    Ok(a) => {
                        tracing::info!(path = %cand.display(), "harness: resolved default architecture.toml");
                        resolved = Some(a);
                        break;
                    }
                    Err(e) => {
                        warn!(path = %cand.display(), error = %e, "harness: default architecture.toml parse failed");
                    }
                }
            }
        }
        if let Some(a) = resolved {
            a
        } else {
            // Also try architecture.toml.example as a last resort (so the
            // example file is useful without a copy step).
            let example = std::path::PathBuf::from("architecture.toml.example");
            if example.exists() {
                match operant_harness::resolve_boot_architecture(&example, patch_dir.as_deref()) {
                    Ok(a) => {
                        tracing::info!(
                            "harness: using architecture.toml.example (copy to architecture.toml to customize)"
                        );
                        a
                    }
                    Err(_) => operant_harness::Architecture::default(),
                }
            } else {
                tracing::info!(
                    "harness: enabled but no architecture.toml found — kernel is empty (create one or set [harness].architecture_toml)"
                );
                operant_harness::Architecture::default()
            }
        }
    };

    match host.boot_with_factories(&builder, &arch).await {
        Ok(activated) => tracing::info!(
            count = activated.len(),
            total = arch.rows.len(),
            "harness: architecture + patches loaded and providers mounted"
        ),
        Err(e) => warn!(error = %e, "harness: boot_with_factories failed"),
    }

    // C2 — opt-in WASM hot-swap. Off by default: this is a permanent
    // background poll that re-instantiates a module on every detected
    // change, so it must never start as a side effect of `[harness]
    // .enabled`. Requires the plugins-wasm feature AND a verifying
    // signature mode (assert_verifying_host refuses Disabled/Permissive —
    // a swap EXECUTES the module on every scan).
    #[cfg(feature = "plugins-wasm")]
    if config.harness.watch_wasm {
        spawn_wasm_watcher(config, host.harness().clone());
    }

    Some((host.harness().clone(), prompt_slot, hook_slot))
}

/// C2 — start the plugin-directory watcher and hot-swap verified plugins
/// into the running kernel. `harness` must be the live harness Arc so
/// `Harness::replace` affects the agent's actual providers.
///
/// Trusted publisher keys come from `[plugins.security]` — an empty key set
/// under `SignatureMode::Strict` would mark every signed plugin `Untrusted`
/// and the watcher would never swap anything, so an empty set is refused up
/// front rather than left as a silent no-op.
#[cfg(feature = "plugins-wasm")]
fn spawn_wasm_watcher(config: &AppConfig, harness: std::sync::Arc<operant_harness::Harness>) {
    use operant_plugins::{host::PluginHost, signature::SignatureMode, watcher::Watcher};

    use crate::plugin_tools::row_for_change;

    let Some(dir) = config.plugins.plugin_dirs.first().cloned() else {
        warn!("harness: watch_wasm enabled but plugins.plugin_dirs is empty — no watcher");
        return;
    };
    let Some(parent) = dir.parent() else {
        warn!(dir = %dir.display(), "harness: watch_wasm — plugins dir has no parent");
        return;
    };
    let trusted_keys: Vec<String> = config.plugins.trusted_publisher_keys.clone();
    if trusted_keys.is_empty() {
        warn!(
            "harness: watch_wasm refused — Strict signature mode needs trusted publisher keys \
             (set [plugins] trusted_publisher_keys; an empty key set would mark every plugin \
             Untrusted and never swap)"
        );
        return;
    }
    // The swap path always verifies, whatever the tool bridge is configured
    // for: a swap instantiates the module on every scan, so unsigned code
    // must never reach it. Checked BEFORE the host is built so a refusal
    // costs nothing, and named explicitly so an operator who configured
    // permissive/disabled sees what to set rather than a silent upgrade.
    let configured = PluginHost::parse_signature_mode(&config.plugins.signature_mode);
    if !matches!(configured, SignatureMode::Strict) {
        warn!(
            mode = %config.plugins.signature_mode,
            "harness: watch_wasm refused — the kernel swap path requires signature_mode = \"strict\" \
             (a swap executes the module on every scan); set [plugins] signature_mode = \"strict\""
        );
        return;
    }
    let host = match PluginHost::new(parent) {
        Ok(host) => host,
        Err(e) => {
            warn!(error = %e, "harness: watch_wasm — plugin host failed to initialize");
            return;
        }
    };
    let mut watcher = Watcher::new(host, operant_plugins::watcher::WatcherConfig::default())
        .with_signature(SignatureMode::Strict, trusted_keys);
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        tracing::info!(dir = %dir.display(), "harness: wasm watcher started");
        watcher
            .run_until(stop_rx, |changes| {
                for change in changes {
                    match row_for_change(change) {
                        Ok(bridged) => {
                            let provider = std::sync::Arc::new(
                                operant_harness::wasm_provider::WasmProvider::new(bridged.row),
                            );
                            let id = operant_harness::Provider::spec(provider.as_ref())
                                .id()
                                .to_string();
                            let harness = harness.clone();
                            // replace() is async; the watcher callback is
                            // sync, so hop onto the runtime.
                            tokio::spawn(async move {
                                // `replace` only swaps an ALREADY-MOUNTED
                                // provider; a first-time plugin must be
                                // mounted instead. No `get_provider`
                                // accessor exists on Harness, so membership
                                // comes from the dump the metrics surface
                                // already uses.
                                let already =
                                    harness.dump().await.providers.iter().any(|p| p.id == id);
                                let result = if already {
                                    harness.replace(provider).await
                                } else {
                                    harness.mount(provider).await.map(|_| "mounted".to_string())
                                };
                                match result {
                                    Ok(outcome) => tracing::info!(
                                        id = %id,
                                        outcome = %outcome,
                                        "harness: wasm provider applied"
                                    ),
                                    Err(e) => {
                                        warn!(id = %id, error = %e, "harness: wasm swap failed")
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            warn!(
                                plugin = %change.manifest.name,
                                error = %e,
                                "harness: plugin not eligible for a kernel swap"
                            );
                        }
                    }
                }
            })
            .await;
        drop(stop_tx);
    });
}

/// Build the runtime agent for a long-lived host (TUI / chat / gateway).
///
/// Thin wrapper over [`create_runtime_agent_with`] so existing single-agent
/// callers keep their shape and keep their background LCM maintenance workers.
pub(crate) async fn create_runtime_agent(
    config: &AppConfig,
    behavior: &BehaviorSettings,
    system_prompt: Option<&str>,
    event_tx: mpsc::Sender<AgentEvent>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
    metrics: Option<std::sync::Arc<operant_core::runtime_metrics::RuntimeMetrics>>,
) -> Result<OperantAgent> {
    create_runtime_agent_with(
        config,
        behavior,
        system_prompt,
        event_tx,
        mcp_manager,
        skills_dir,
        metrics,
        true,
        // Single-agent default: the memory pair is loaded inside
        // build_agent_core (BUGS.md D-5 hoisting is for multi-agent hosts).
        None,
    )
    .await
}

/// Build the runtime agent used by long-lived hosts (TUI / chat / gateway).
///
/// `spawn_long_lived_maintenance` distinguishes hosts that keep running from
/// one-shot invocations. It is `true` for long-lived hosts, where background
/// LCM workers (rollups + assertion extraction, each with its own
/// `OpenAIClient`) are wanted, and MUST be `false` for any SECOND agent built in
/// the same process — otherwise that agent starts a duplicate set of rollup
/// loops over the same context engine: duplicated LLM spend on every tick and
/// racing rollups. `build_context_engine` already takes this flag
/// (`main.rs:1917`); pass it straight through.
/// `build_context_engine` already takes this flag
/// (`main.rs:1917`); pass it straight through.
///
/// `memory` is the BUGS.md D-5 seam: `None` keeps the single-agent default
/// (the pair is loaded inside `build_agent_core`); multi-agent hosts pass
/// ONE `(MemoryManager, Option<Arc<dyn MemoryProvider>>)` pair constructed
/// at their entry point so every agent built in the process shares the
/// same memory backend.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit dependency injection; a config struct would only rename the arguments"
)]
pub(crate) async fn create_runtime_agent_with(
    config: &AppConfig,
    behavior: &BehaviorSettings,
    system_prompt: Option<&str>,
    event_tx: mpsc::Sender<AgentEvent>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
    metrics: Option<std::sync::Arc<operant_core::runtime_metrics::RuntimeMetrics>>,
    spawn_long_lived_maintenance: bool,
    memory: Option<(MemoryManager, Option<Arc<dyn MemoryProvider>>)>,
) -> Result<OperantAgent> {
    let core = match memory {
        Some((memory_manager, memory_provider)) => {
            build_agent_core_with_memory(
                config,
                system_prompt,
                mcp_manager,
                skills_dir,
                &behavior.model,
                behavior,
                Some(event_tx.clone()),
                memory_manager,
                memory_provider,
            )
            .await?
        }
        None => {
            build_agent_core(
                config,
                system_prompt,
                mcp_manager,
                skills_dir,
                &behavior.model,
                behavior,
                Some(event_tx.clone()),
            )
            .await?
        }
    };

    let provider = crate::tui::provider::infer_provider_from_model(&behavior.model)
        .unwrap_or_else(|| "openai".to_string());
    let (model_client, pool_registry) = create_model_client_with_fallback(
        &provider,
        &behavior.model,
        config,
        Some(event_tx.clone()),
    );

    let context_window = core.agent_config.context_window;
    Ok({
        let flag = operant_core::interrupt::InterruptFlag::new();
        // Spawn a Ctrl-C handler that triggers the flag. The agent loop
        // checks this flag at each iteration boundary + before each tool
        // call, so Ctrl-C produces a graceful exit instead of killing the
        // process mid-tool.
        let handler_flag = flag.clone();
        tokio::spawn(async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::debug!("ctrl_c signal handler error: {}", e);
            }
            tracing::info!("Ctrl-C received — triggering agent interrupt flag");
            handler_flag.trigger();
        });
        let mut agent = OperantAgent::with_events(
            core.agent_config,
            model_client,
            core.registry,
            core.database,
            event_tx,
        )
        .with_memory_manager(core.memory_manager)
        .with_skill_manager(core.skill_manager)
        .with_harness_prompt_slot_opt(core.harness_prompt_slot)
        .with_hook_registry_opt(core.hook_registry)
        .with_interrupt_flag(flag)
        .with_llm_compressor(operant_core::agent::llm_compressor::LlmCompressorConfig {
            context_window,
            enabled: config.agent.context_compression,
            threshold_percent: config.agent.context_compression_threshold,
            ..Default::default()
        });
        // Plan 016 G1 — attach the harness when [harness].enabled = true.
        // Default is `None`, so this is a no-op for the dark-merge path.
        if let Some(h) = core.harness.clone() {
            agent = agent.with_harness(h);
        }
        // Pluggable context engine (hermes-lcm parity): when configured
        // (agent.context_engine = "lcm"), build_messages assembles via the
        // lossless DAG + fresh-tail engine instead of lossy eviction.
        // Long-lived process (TUI / chat / gateway) → spawn background
        // maintenance workers (rollups + assertion extraction).
        if let Some(engine) = build_context_engine(config, spawn_long_lived_maintenance) {
            agent = agent.with_context_engine(engine);
        }
        // Share the external runtime-metrics registry (created by the TUI)
        // so stream-drop retries and memory-sync failures surface in the
        // status bar. When None, the agent keeps its own internal registry.
        if let Some(metrics) = metrics {
            agent = agent.with_metrics(metrics);
        }
        // Attach the long-term memory provider so turn/session hooks fire
        // (sync_turn, prefetch, on_session_end, ...). Closes audit gap F1.
        if let Some(provider) = core.memory_provider {
            agent = agent.with_memory_provider(provider);
        }
        configure_checkpoints(config);
        agent = attach_credential_pool(agent, &provider, config, pool_registry.as_ref());
        agent = apply_agent_config_defaults(agent, behavior);
        agent
    })
}

pub(crate) async fn create_agent_without_events(
    config: &AppConfig,
    system_prompt: Option<&str>,
    mcp_manager: &McpManager,
    skills_dir: &Path,
) -> Result<OperantAgent> {
    let core = build_agent_core(
        config,
        system_prompt,
        mcp_manager,
        skills_dir,
        &config.agent.model,
        &config.agent,
        None,
    )
    .await?;

    // A `hint:<name>` model asks for a ROUTE, not a model. The config
    // documents the usage — "pass `hint:reasoning` as the model parameter
    // to route the request" — and the resolution used to live only in
    // `operant-providers`' router, which the runtime never calls, so the
    // literal string was sent to the provider AS THE MODEL NAME (RT-96).
    //
    // Resolve it here, where the model name becomes a request. An
    // unresolvable hint is an ERROR, not a model name: silently sending
    // `hint:typo` to a provider produces a confusing 404 from the API.
    let (model_name, provider) = match config.providers.model_route_for_hint(&config.agent.model) {
        Some(route) => (route.model.clone(), route.provider.clone()),
        None => {
            if config.providers.is_model_hint(&config.agent.model) {
                anyhow::bail!(
                    "the model is {:?}, but no [[providers.model_routes]] entry has \
                     that hint. Add one, or use a model name directly.",
                    config.agent.model
                );
            }
            let provider = crate::tui::provider::infer_provider_from_model(&config.agent.model)
                .unwrap_or_else(|| "openai".to_string());
            (config.agent.model.clone(), provider)
        }
    };
    let (model_client, pool_registry) =
        create_model_client_with_fallback(&provider, &model_name, config, None);

    let context_window = core.agent_config.context_window;
    Ok({
        let flag = operant_core::interrupt::InterruptFlag::new();
        let handler_flag = flag.clone();
        tokio::spawn(async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::debug!("ctrl_c signal handler error: {}", e);
            }
            tracing::info!("Ctrl-C received — triggering agent interrupt flag");
            handler_flag.trigger();
        });
        let mut agent = OperantAgent::new(
            core.agent_config,
            model_client,
            core.registry,
            core.database,
        )
        .with_memory_manager(core.memory_manager)
        .with_skill_manager(core.skill_manager)
        .with_harness_prompt_slot_opt(core.harness_prompt_slot)
        .with_hook_registry_opt(core.hook_registry)
        .with_interrupt_flag(flag)
        .with_llm_compressor(operant_core::agent::llm_compressor::LlmCompressorConfig {
            context_window,
            enabled: config.agent.context_compression,
            threshold_percent: config.agent.context_compression_threshold,
            ..Default::default()
        });
        // Phase C (docs/agentic-os/): a Holon run — HOLON_RUN_DIR carries the
        // kernel's compiled policy.json — enforces that document through the
        // seat-authority consult on every tool call. Not a Holon run (no env
        // var, or the kernel predates policy.json): no authority, today's
        // behavior byte-identical — invariant 5.
        if let Some(authority) =
            operant_os::enforcement::holon_authority(&operant_core::platform::operant_home())
                .map_err(|e| anyhow::anyhow!("holon enforcement: {e}"))?
        {
            let seat = operant_os::enforcement::holon_seat();
            tracing::info!(
                seat = %seat,
                "Holon run detected — policy.json enforcement mounted"
            );
            // the consult keys on the seat: name the employee the kernel
            // exported so the run's policy row is the one consulted (the
            // gateway sets this same field for its own turns)
            agent.set_seat_id(seat);
            agent = agent.with_seat_authority(Some(authority));
        }
        if let Some(h) = core.harness.clone() {
            agent = agent.with_harness(h);
        }
        // Pluggable context engine (hermes-lcm parity) — same gate as the
        // runtime agent path. Bounded one-shot run / autonomous task → no
        // LLM-fired maintenance workers (they'd be killed at process exit;
        // see build_context_engine docs).
        if let Some(engine) = build_context_engine(config, false) {
            agent = agent.with_context_engine(engine);
        }
        // Attach the long-term memory provider so turn/session hooks fire
        // (sync_turn, prefetch, on_session_end, ...). Closes audit gap F1.
        if let Some(provider) = core.memory_provider {
            agent = agent.with_memory_provider(provider);
        }
        configure_checkpoints(config);
        agent = attach_credential_pool(agent, &provider, config, pool_registry.as_ref());
        agent = apply_agent_config_defaults(agent, &config.agent);
        agent
    })
}

/// Derive the LCM engine config from the `agent` settings table.
///
/// Shared by the registry tool wiring and the agent engine so the db-path
/// default and tail budget can never drift between the two instances.
pub(crate) fn lcm_config(config: &AppConfig) -> operant_core::context::LcmConfig {
    operant_core::context::LcmConfig {
        db_path: config
            .agent
            .context_lcm_db
            .clone()
            .unwrap_or_else(|| operant_core::platform::operant_home().join("lcm.db")),
        tail_tokens: config.agent.context_lcm_tail_tokens,
        auto_recall: config.agent.context_lcm_auto_recall,
        auto_recall_limit: config.agent.context_lcm_auto_recall_limit,
        auto_recall_max_chars: config.agent.context_lcm_auto_recall_max_chars,
        rollups_inject: config.agent.context_lcm_rollups_inject,
        ignore_session_patterns: config.agent.context_lcm_ignore_session_patterns.clone(),
        readonly_sessions: config.agent.context_lcm_readonly_sessions.clone(),
    }
}

/// Build the configured context engine (`agent.context_engine`).
///   - `"compact"` (default) → `None`: deterministic decay + eviction.
///   - `"lcm"` → the lossless DAG engine; a broken init falls back to
///     compact (never fatal). Matches hermes `context.engine: lcm`.
///
/// `spawn_maintenance`: only LONG-LIVED processes (TUI / chat / gateway, via
/// `create_runtime_agent`) spawn the LLM-fired background maintenance workers
/// (rollups + assertion extraction). One-shot `operant run` / autonomous task
/// runs are bounded — their immediate maintenance pass would start a ~170s
/// LLM extraction call that gets killed at process exit (wasted provider
/// billing, reproduced in live testing). Hermes runs maintenance only in its
/// long-running agent loop, never per one-shot invocation.
fn build_context_engine(
    config: &AppConfig,
    spawn_maintenance: bool,
) -> Option<std::sync::Arc<dyn operant_core::context::ContextEngine>> {
    match config.agent.context_engine.as_str() {
        "compact" | "" => None, // built-in default — no engine attached
        "lcm" => match operant_core::context::LcmContextEngine::new(lcm_config(config)) {
            Ok(engine) => {
                tracing::info!("LCM context engine active (lossless DAG + fresh tail)");
                let engine = std::sync::Arc::new(engine);
                if spawn_maintenance {
                    spawn_lcm_maintenance_if_configured(config, &engine);
                }
                Some(engine)
            }
            Err(e) => {
                tracing::warn!(error = %e, "LCM context engine init failed — using compact");
                None
            }
        },
        other => {
            tracing::warn!(
                engine = other,
                "unknown agent.context_engine \"{other}\" — using compact"
            );
            None
        }
    }
}

/// One LLM rollup summarizer call (shared by the `context rollup` CLI and
/// the background maintenance scheduler so the prompt can never drift).
pub(crate) async fn rollup_summarize(
    transcript: String,
    client: OpenAIClient,
    model: String,
) -> operant_core::error::Result<String> {
    let msgs = vec![
        operant_core::client::Message::system(
            "You are a lossless temporal summarizer. Summarize the \
             following conversation excerpt into concise key facts \
             and decisions. Preserve names, numbers, and dates \
             exactly. Output only the summary.",
        ),
        operant_core::client::Message::user(transcript),
    ];
    let resp = client
        .chat(&model, &msgs, None, Some(1024), Some(0.2))
        .await
        .map_err(|e| operant_core::error::Error::Agent(format!("rollup LLM call failed: {e}")))?;
    let content = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    Ok(content.trim().to_string())
}

/// Spawn the background maintenance tasks when configured
/// (`context_lcm_rollup_interval_minutes > 0`): one immediate pass, then
/// every interval — hermes `_RollupMaintenanceScheduler` parity, bounded.
///
/// Two workers share the cadence:
///   1. rollup summarization (old message nodes → rollup nodes), and
///   2. assertion extraction (the same opt-in LLM extractor that backs
///      `lcm_assert action="extract"` mines durable facts from recent DAG
///      nodes automatically, so the store stays fresh without the agent ever
///      calling the tool — hermes `_assertion_extraction` maintenance
///      parity).
fn spawn_lcm_maintenance_if_configured(
    config: &AppConfig,
    engine: &std::sync::Arc<operant_core::context::LcmContextEngine>,
) {
    let minutes = config.agent.context_lcm_rollup_interval_minutes;
    if minutes == 0 {
        return;
    }
    let model = config.agent.model.clone();
    if model.is_empty() {
        tracing::warn!("lcm maintenance skipped: agent.model not configured");
        return;
    }
    let interval = std::time::Duration::from_secs(minutes * 60);

    let client = OpenAIClient::new(client_config(config));
    let summarizer = move |transcript: String| {
        let client = client.clone();
        let model = model.clone();
        rollup_summarize(transcript, client, model)
    };
    operant_core::context::rollup::spawn_rollup_maintenance(
        engine.clone(),
        interval,
        7,
        summarizer,
    );
    tracing::info!(minutes, "lcm rollup maintenance scheduler active");

    // Assertion-extraction worker on the same cadence (hermes
    // `_assertion_extraction` maintenance parity). Off when the opt-in gate
    // is off — no extra LLM cost on stock installs.
    if config.agent.context_lcm_assertion_extraction {
        let llm = std::sync::Arc::new(OpenAIClient::new(client_config(config)));
        let extractor: std::sync::Arc<dyn operant_core::context::AssertionExtractor> =
            std::sync::Arc::new(operant_core::context::LlmAssertionExtractor::new(
                llm,
                config.agent.model.clone(),
            ));
        operant_core::context::rollup::spawn_assertion_extraction_scheduler(
            engine.clone(),
            extractor,
            interval,
            40,
        );
        tracing::info!(minutes, "lcm assertion extraction scheduler active");
    }
}

/// Build and attach a credential pool when configured, seeded from the
/// active provider's env var plus `client.additional_api_keys` (hermes's
/// `load_pool` rotation vector). Without this, `try_rotate_credential`
/// always sees `None` and multi-key rotation is dead even when enabled.
/// Wire the global checkpoint manager from config. Checkpoints are opt-in
/// (`[checkpoints] enabled = true`); when enabled, snapshots land in an
/// isolated shadow store under `~/.operant/checkpoints` (hermes parity) so the
/// user's git repositories are never modified.
fn configure_checkpoints(config: &AppConfig) {
    let settings = &config.checkpoints;
    let base_dir = settings
        .base_dir
        .clone()
        .unwrap_or_else(|| operant_core::platform::operant_home().join("checkpoints"));
    operant_core::tools::get_checkpoint_manager().configure(
        operant_core::tools::CheckpointConfig {
            base_dir,
            max_snapshots: settings.max_snapshots,
            // The size bounds were declared on the config and never read,
            // so nothing bounded the bytes a snapshot could take.
            max_total_size_mb: settings.max_total_size_mb,
            max_file_size_mb: settings.max_file_size_mb,
            enabled: settings.enabled,
        },
    );
}

/// `[agent]` config-file DEFAULTS for the env-armed knobs (iter-723): the
/// guardian-LLM approval arm (iter-713) and micro-compaction (iter-714).
/// Each applies only when its `OPERANT_*` env var is UNSET — the env var
/// (and the explicit `with_*` builders) still win when present.
pub(crate) fn apply_agent_config_defaults(
    mut agent: OperantAgent,
    behavior: &BehaviorSettings,
) -> OperantAgent {
    if std::env::var_os("OPERANT_GUARDIAN_LLM").is_none() && behavior.guardian_llm {
        agent = agent.with_guardian_llm(true);
    }
    if std::env::var_os("OPERANT_MICRO_COMPACTION").is_none() && behavior.micro_compaction {
        agent = agent.with_micro_compaction(true);
    }
    agent
}

fn attach_credential_pool(
    agent: OperantAgent,
    provider: &str,
    config: &AppConfig,
    pool_registry: Option<&std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry>>,
) -> OperantAgent {
    if !config.credential_pool.enabled {
        return agent;
    }
    let pool = match pool_registry {
        Some(registry) => {
            let pool = registry.get_or_create(provider);
            if !pool.has_credentials() {
                return agent;
            }
            pool
        }
        None => {
            let mut pool = operant_core::credential_pool::CredentialPool::new(provider);
            pool.seed_from_env(&crate::cmd_auth::provider_env_var(provider));
            for key in &config.client.additional_api_keys {
                if !key.trim().is_empty() {
                    pool.add(operant_core::credential_pool::PooledCredential::new(
                        &format!("additional-{}", key.len()),
                        operant_core::credential_pool::AuthType::ApiKey,
                        key,
                        "config (client.additional_api_keys)",
                    ));
                }
            }
            // Per-provider strategy (hermes `credential_pool_strategies`
            // parity): the provider's entry wins, else global, else
            // fill_first — hermes `get_pool_strategy()` fail-closed default.
            pool.set_strategy(config.credential_pool.strategy_for(provider));
            if !pool.has_credentials() {
                return agent;
            }
            std::sync::Arc::new(pool)
        }
    };
    tracing::info!(
        provider = %provider,
        creds = pool.len(),
        strategy = %pool.strategy().as_str(),
        "Attached credential pool"
    );
    agent.with_credential_pool(pool)
}

async fn load_repo_memory_manager() -> Result<(MemoryManager, Option<Arc<dyn MemoryProvider>>)> {
    let storage_dir = operant_core::platform::operant_home();
    load_memory_manager(storage_dir).await
}

/// Load the file-backed MemoryManager and, when a non-builtin provider is
/// configured, build + background-initialize it and return it so the agent
/// can attach it (previously the provider was dropped — audit gap F1).
pub(crate) async fn load_memory_manager(
    storage_dir: PathBuf,
) -> Result<(MemoryManager, Option<Arc<dyn MemoryProvider>>)> {
    // R-08 (AUDIT-2026-10-16-capability-layer): a Holon run opens NO memory
    // provider — the kernel's context bundle is the run's memory, and
    // learnings leave through result.json (W6). The trace that forced this:
    // the adapter used to pin `[memory] provider = "builtin"`, which this
    // loader treats as legacy no-op → no provider ever existed (that is why
    // six live Holon runs left no bank anywhere); the pin's removal left
    // `enabled = true` + the `agentmemory` default, which WOULD open one
    // memory-wire bank in the shared adapter HERMES_HOME — every employee
    // sharing it, and the gateway's cron agent's lessons beside org work.
    // Gated before construction so not even the sqlite file appears.
    // Personal mode keeps its provider (a design preference — unchanged).
    if operant_core::platform::is_holon_run() {
        return Ok((MemoryManager::with_storage_dir(storage_dir), None));
    }
    // (plan 11C3) memory-wire is the ONLY engine. The MemoryManager half of
    // this pair is now inert plumbing the agent still accepts (kept so the
    // D-5 threading compiles; nothing reads it into prompts anymore), and
    // the one-time MEMORY.md/USER.md import runs inside the factory. The
    // concrete provider is built here so the memory tools' global can hold
    // the same handle the agent recalls through.
    let memory_manager = MemoryManager::with_storage_dir(storage_dir.clone());

    let cfg = operant_core::config::runtime_config();
    if cfg.memory.enabled && cfg.memory.provider != "builtin" && cfg.memory.provider != "disabled" {
        // WASM memory plugin (memory.provider = "plugin:<name>"): a plugin
        // declaring the `memory` capability backs the MemoryProvider trait,
        // hermes-agent `plugins/memory/<name>` parity. When no plugin
        // matches, fall through to the compiled-in factory below.
        //
        // Uses the ACTIVE runtime config (`cfg`) — which already carries the
        // `--config` path + CLI overrides — so plugin dirs always resolve
        // from the config the user actually launched with, never a fresh
        // default-config reload.
        #[cfg(feature = "plugins-wasm")]
        if let Some(plugin_provider) =
            plugin_memory::build_plugin_memory_provider(&cfg.memory.provider, &cfg).await
        {
            let name = plugin_provider.name().to_string();
            let init = plugin_provider.clone();
            tokio::spawn(async move {
                if let Err(e) = init.initialize("main").await {
                    tracing::warn!(provider = %name, error = %e, "Memory plugin init failed");
                }
            });
            return Ok((memory_manager, Some(plugin_provider)));
        }
        // (plan 11C3) build the CONCRETE provider so the memory tools'
        // global holds the same handle the agent recalls through, then
        // coerce for the trait-object return. The [memory] provider name
        // warning fires at config load (MemorySettings::warn_legacy_keys).
        match operant_core::memory_wire::MemoryWireProvider::new(storage_dir.clone()) {
            Ok(concrete) => {
                let _ = concrete.import_legacy_files(&storage_dir);
                let concrete = std::sync::Arc::new(concrete);
                operant_core::tools::memory_tools::set_active_memory_provider(
                    std::sync::Arc::clone(&concrete),
                )
                .await;
                let provider: Arc<dyn MemoryProvider> = concrete;
                let name = provider.name().to_string();
                let init = provider.clone();
                tokio::spawn(async move {
                    if let Err(e) = init.initialize("main").await {
                        tracing::warn!(provider = %name, error = %e, "Memory provider init failed");
                    }
                });
                Ok((memory_manager, Some(provider)))
            }
            Err(e) => {
                // No provider (memory off for the session): the tools'
                // honest no-backend error is the contract.
                tracing::error!(error = %e, "memory provider init failed — memory disabled");
                Ok((memory_manager, None))
            }
        }
    } else {
        Ok((memory_manager, None))
    }
}

/// Per-turn accounting line (R5 — hermes `agent/turn_summary.py` parity).
/// Rides the observer feed (which already sees every tool call) and prints
/// one dim summary line after the turn, e.g.
/// `⋯ 12.4s · edited 2 files +18 -3 · read 4 files · ran 3 commands`.
struct TurnSummaryObserver {
    collector: std::sync::Mutex<TurnSummaryCollector>,
}

impl TurnSummaryObserver {
    fn new() -> Self {
        Self {
            collector: std::sync::Mutex::new(TurnSummaryCollector::new()),
        }
    }
}

impl Observer for TurnSummaryObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { .. } => {
                if let Ok(mut c) = self.collector.lock() {
                    c.begin();
                }
            }
            ObserverEvent::ToolCall { tool, .. } => {
                if let Ok(mut c) = self.collector.lock() {
                    c.record_tool(tool, "");
                }
            }
            ObserverEvent::AgentEnd { duration, .. } => {
                let line = self
                    .collector
                    .lock()
                    .map(|c| format_turn_summary(c.tally(), duration.as_secs_f64()))
                    .unwrap_or_default();
                if !line.is_empty() {
                    println!("{line}");
                }
            }
            _ => {}
        }
    }

    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &str {
        "turn-summary"
    }
}

async fn run_non_tui(
    config: &AppConfig,
    system_prompt: Option<&str>,
    query: &str,
    record_trajectory: bool,
    moa: bool,
) -> Result<()> {
    let mcp_manager = McpManager::new();
    // Phase D (docs/agentic-os/, H4): a Holon run always records — the
    // trajectory IS the gold standard input, not an optional extra.
    let record_trajectory = record_trajectory || operant_core::platform::is_holon_run();
    let agent =
        create_agent_without_events(config, system_prompt, &mcp_manager, &config.skills.root_dir)
            .await?
            .with_trajectory_recording(record_trajectory);
    if record_trajectory {
        println!("Trajectory recording enabled — run will be saved to ~/.operant/trajectories/");
    }

    // G5 (hermes moa_loop.py parity): a MoA turn fans out the configured
    // reference models over the fresh conversation, has the aggregator
    // synthesize their advice, and injects the guidance for this run only.
    // Failures are non-fatal — the agent still runs in single-model mode.
    if moa {
        match run_moa_for_query(config, query).await {
            Ok(Some(guidance)) => {
                println!("[moa] Injected Mixture-of-Agents guidance for this turn");
                agent.set_moa_guidance(guidance);
            }
            Ok(None) => {
                println!(
                    "[moa] [moa] disabled or no reference models configured — running without MoA"
                );
            }
            Err(e) => {
                eprintln!("[moa] MoA aggregation failed (non-fatal): {e:#}");
            }
        }
    }

    // R5 — per-turn accounting line: ride the observer feed so the summary
    // prints after the one-shot run (hermes turn_summary.py parity).
    let summary_observer: std::sync::Arc<dyn Observer> =
        std::sync::Arc::new(TurnSummaryObserver::new());
    let agent = agent.with_observer(summary_observer.clone());

    let response = agent.run(query.to_string()).await?;
    // Drain pending memory hooks (sync_turn → /observe, session/end) before
    // the process exits. The MemorySyncExecutor is a background tokio task —
    // without a drain the runtime is torn down mid-write and the observation
    // is lost (reproduced in live testing; hermes _drain_sync_executor parity).
    agent.shutdown_memory_executor().await;
    println!("{}", response.content);
    Ok(())
}

/// G5 helper: run the MoA fan-out + synthesis for a fresh `operant run`
/// query. Returns `Ok(None)` when MoA is disabled or no reference models
/// are configured (the agent runs in single-model mode).
async fn run_moa_for_query(config: &AppConfig, query: &str) -> Result<Option<String>> {
    let moa = &config.moa;
    if !moa.enabled || moa.references.is_empty() {
        return Ok(None);
    }
    let messages = vec![operant_core::client::Message::user(query)];
    let guidance = operant_core::moa::aggregate_moa_context(
        query,
        &messages,
        &moa.references,
        moa.aggregator.as_ref(),
        &config.agent.model,
        &client_config(config),
        moa.max_tokens,
        moa.temperature,
        Duration::from_secs(moa.timeout_secs),
    )
    .await
    .context("MoA aggregation failed")?;
    Ok(guidance)
}

fn preview_tool_args(args: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(args) {
        for key in &[
            "query",
            "command",
            "url",
            "file_path",
            "path",
            "code",
            "text",
        ] {
            if let Some(val) = json.get(*key).and_then(|v| v.as_str()) {
                let truncated = if val.len() > 80 {
                    format!("{}...", &val[..80])
                } else {
                    val.to_string()
                };
                return Some(truncated);
            }
        }
    }
    None
}

/// Read one line, returning `None` on EOF.
///
/// `read_line` reports a BYTE COUNT, and 0 means the stream is exhausted. That
/// is not the same as a blank line: an empty `String` could mean "the user
/// pressed Enter" or "there is no more input", and collapsing the two makes an
/// exhausted stdin indistinguishable from an idle one.
///
/// That distinction was a live hang. `operant chat` with stdin closed or
/// redirected reprints its `You: ` prompt forever — measured at 33 MB of output
/// in 20 seconds, never exiting — because EOF produced an empty string, the
/// blank-line `continue` fired, and the loop spun. It never surfaced in
/// interactive use, where a human supplies the terminating `/exit`, which is
/// exactly why it survived: the exit path that everyone exercises is not the
/// exit path a script or a CI job takes.
///
/// Generic over `BufRead` so the three cases are testable without replacing
/// process-wide stdin.
fn read_line_or_eof(reader: &mut impl std::io::BufRead) -> std::io::Result<Option<String>> {
    let mut buf = String::new();
    if reader.read_line(&mut buf)? == 0 {
        return Ok(None);
    }
    Ok(Some(buf))
}

async fn chat_non_tui(config: &AppConfig, system_prompt: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    // R5 — per-turn accounting line in interactive mode: the summary
    // observer rides the shared feed and prints after each turn.
    let summary_observer: std::sync::Arc<dyn Observer> =
        std::sync::Arc::new(TurnSummaryObserver::new());
    let agent = create_runtime_agent(
        config,
        &config.agent,
        system_prompt,
        event_tx,
        &mcp_manager,
        &config.skills.root_dir,
        None,
    )
    .await?
    .with_observer(summary_observer);

    // Spawn task to display tool events in real-time
    tokio::spawn(async move {
        // A tool call is announced twice (queued, then running once a worker
        // permit is acquired). Print it once.
        let mut announced: std::collections::HashSet<String> = std::collections::HashSet::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::ToolStart {
                    tool_call_id,
                    name,
                    arguments,
                } => {
                    if !announced.insert(tool_call_id) {
                        continue;
                    }
                    let preview = preview_tool_args(&arguments)
                        .map(|a| format!("{}: {}", name, a))
                        .unwrap_or_else(|| name);
                    println!("  Tool: {}...", preview);
                }
                AgentEvent::ToolComplete { result: _ } => {
                    println!("  Tool: Done.");
                }
                AgentEvent::ToolError {
                    tool_call_id: _,
                    name,
                    error,
                } => {
                    eprintln!("  Tool Error {}: {}", name, error);
                }
                AgentEvent::BackgroundReview { summary, .. } => {
                    println!("  \u{1f4be} Self-improvement review: {summary}");
                }
                AgentEvent::AsyncDelegation {
                    delegation_id,
                    status,
                    summary,
                } => {
                    println!("  \u{1f9f5} Async delegation {delegation_id} {status}: {summary}");
                }
                _ => {}
            }
        }
    });

    // (iter-157: 6 inline handler structs deleted — replaced with a simple
    // match in the command loop. The handlers just returned hardcoded strings.)

    loop {
        print!("You: ");
        io::stdout().flush()?;
        // EOF breaks the loop. A blank line still continues, because in
        // interactive use pressing Enter should just reprint the prompt.
        // `Stdin` itself does not implement `BufRead` (its `read_line` is an
        // inherent method), so the lock guard is what satisfies the bound.
        let Some(raw) = read_line_or_eof(&mut io::stdin().lock())? else {
            println!();
            break;
        };
        let input = raw.trim().to_string();

        if input.is_empty() {
            continue;
        }

        if input.starts_with('/') {
            let parts: Vec<&str> = input.splitn(2, char::is_whitespace).collect();
            let cmd = parts[0].trim_start_matches('/');

            match cmd {
                "exit" | "quit" => break,
                "new" | "reset" => {
                    agent.clear_history().await;
                    println!("Conversation cleared. Starting new session.");
                }
                "help" => {
                    println!("Commands: /exit, /new, /help, /status, /time, /skills, /history");
                }
                "status" => {
                    println!("System status: running. Use `operant status` for details.");
                }
                "time" => {
                    use chrono::Local;
                    println!("Current time: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
                }
                "session" => {
                    println!("Conversation session active. Use /history to see messages.");
                }
                "skills" => {
                    let mut sm = SkillManager::new(config.skills.root_dir.clone());
                    match sm.load_all() {
                        Ok(skills) => {
                            if skills.is_empty() {
                                println!("No skills installed.");
                            } else {
                                println!("Installed skills:");
                                for skill in &skills {
                                    println!("  /{} — {}", skill.name, skill.description);
                                }
                            }
                        }
                        Err(e) => println!("Failed to load skills: {}", e),
                    }
                }
                "history" => {
                    println!("History not yet available in non-TUI mode.");
                }
                _ => {
                    println!(
                        "Unknown command: /{}. Type /help for available commands.",
                        cmd
                    );
                }
            }
            continue;
        }

        match agent.run(input.to_string()).await {
            Ok(response) => println!("Assistant: {}\n", response.content),
            Err(error) => eprintln!("Error: {}\n", error),
        }
    }

    // Drain pending memory hooks (sync_turn → /observe, session/end) before
    // the interactive session exits (hermes _drain_sync_executor parity).
    agent.shutdown_memory_executor().await;

    Ok(())
}

async fn test_tool(config: &AppConfig, tool_name: &str, args: Option<&str>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let client = OpenAIClient::new(client_config(config));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = build_registry(
        config,
        &mcp_manager,
        &client,
        &config.agent.model,
        database,
        None,
    )
    .await?;
    let parsed_args: Value = if let Some(args) = args {
        if args.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(args).context("Failed to parse tool arguments as JSON")?
        }
    } else {
        Value::Object(serde_json::Map::new())
    };

    let result = registry
        .execute(
            tool_name,
            &format!("test_{}", tool_name),
            parsed_args,
            ToolContext::default(),
        )
        .await?;

    println!("success: {}", result.success);
    println!("content: {}", result.content);
    if let Some(error) = result.error {
        println!("error: {}", error);
    }

    Ok(())
}

/// Whether the interactive TUI can run in the current environment.
///
/// The TUI enters raw mode + alternate screen via crossterm, which requires
/// a real terminal on stdin/stdout. When operant is invoked from a script,
/// CI, or piped input (stdout is not a TTY), raw-mode setup fails with
/// "No such device or address (os error 6)" — so we degrade to non-TUI mode
/// instead of crashing with a cryptic error. (audit 2026-08-02)
fn tui_available() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Emit a one-line stderr notice when rich_output is configured but the TUI
/// can't run (no TTY). Uses eprintln (not tracing) because with rich_output
/// the log sink swallows trace output — and this is a UX-visible change.
fn warn_tui_fallback(rich_output: bool) {
    if rich_output {
        eprintln!(
            "warning: no TTY detected — falling back to non-interactive mode (set tui.rich_output=false to silence)"
        );
    }
}

/// Restore the default `SIGPIPE` disposition.
///
/// Rust's runtime sets `SIGPIPE` to `SIG_IGN` before `main` so that a write to
/// a closed pipe returns `EPIPE` instead of killing the process outright. That
/// is the right default for a library, and the wrong one for a CLI: `println!`
/// treats the resulting `Err` as a broken invariant and panics with "failed
/// printing to stdout: Broken pipe". Combined with `panic = "abort"` in the
/// release profile that is not a tidy failure — the process aborts and dumps
/// core, so `operant doctor | head -5` printed a Rust panic message and left a
/// `core` file behind.
///
/// Restoring `SIG_DFL` makes a closed pipe do what every other Unix tool does:
/// die quietly from the signal, which the shell reports as 141. `yes | head -1`
/// behaves identically, and nothing is written to stderr.
///
/// This must run before any output. It is Unix-only; on Windows `SIGPIPE` has
/// no process-wide disposition to set and Rust's behaviour already matches
/// expectations, so there is nothing to do.
fn restore_default_sigpipe_disposition() {
    #[cfg(unix)]
    // Safety: `signal` is async-signal-safe and this runs single-threaded,
    // before any worker exists. `SIG_DFL` is a valid disposition constant.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn main() -> Result<()> {
    restore_default_sigpipe_disposition();

    let cli = Cli::parse();

    // Load CLI-level config (.env file + HERMES_* env overrides; config.yaml
    // is deprecated — operant.toml below is the sole file-based config source).
    // This runs BEFORE the tokio runtime is built: `CliConfig::load()` writes
    // `.env` values into the process environment, and that is only sound while
    // no worker thread exists to read it concurrently.
    let cli_config = CliConfig::load().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load CLI config: {}. Using defaults.", e);
        CliConfig::default()
    });

    // Apply the `[proxy]` table before the runtime exists, so the environment
    // export cannot race a runtime thread (see `apply_startup_proxy`).
    // The config is loaded once, here, and `run` reuses it, so each load-time
    // warning prints once (BUGS CR-5).
    let loaded = load_app_config(cli.config.as_deref())?;
    operant_core::config::apply_startup_proxy(&loaded)?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build the tokio runtime")?
        .block_on(run(cli, cli_config, loaded))
}

async fn run(
    cli: Cli,
    cli_config: CliConfig,
    mut loaded: operant_core::config::LoadedConfig,
) -> Result<()> {
    // Secret redaction toggle (hermes `HERMES_REDACT_SECRETS` parity). The
    // core loop redacts tool output / message content at the LLM boundary;
    // honor `security.redact_secrets` (default true) from CLI config.
    operant_core::redaction::set_redact_enabled(cli_config.security.redact_secrets.unwrap_or(true));

    // Load core AppConfig (TOML-based) and layer on any values from CliConfig
    // Snapshot the file's own LLM settings BEFORE apply_cli_overrides stamps
    // the global --api-key/--base-url/--model onto them (override semantics).
    // Commands that persist the config (e.g. channel add/remove) restore these
    // so a temporary CLI override can never be written back to disk.
    let pre_override_client_api_key = loaded.config.client.api_key.clone();
    let pre_override_client_base_url = loaded.config.client.base_url.clone();
    let pre_override_agent_model = loaded.config.agent.model.clone();

    // Merge CliConfig-derived values into core AppConfig
    let cli_app_config = cli_config.to_app_config();
    if loaded.config.client.base_url.is_empty() {
        loaded.config.client.base_url = cli_app_config.client.base_url;
    }
    if loaded.config.client.api_key.is_none() {
        loaded.config.client.api_key = cli_app_config.client.api_key.clone();
    }
    if !loaded.config.gateway.telegram_enabled && cli_app_config.gateway.telegram_enabled {
        loaded.config.gateway.telegram_enabled = cli_app_config.gateway.telegram_enabled;
        if loaded.config.gateway.telegram_token.is_none() {
            loaded.config.gateway.telegram_token = cli_app_config.gateway.telegram_token;
        }
    }
    if !loaded.config.gateway.discord_enabled && cli_app_config.gateway.discord_enabled {
        loaded.config.gateway.discord_enabled = cli_app_config.gateway.discord_enabled;
        if loaded.config.gateway.discord_token.is_none() {
            loaded.config.gateway.discord_token = cli_app_config.gateway.discord_token;
        }
    }
    if !loaded.config.gateway.slack_enabled && cli_app_config.gateway.slack_enabled {
        loaded.config.gateway.slack_enabled = cli_app_config.gateway.slack_enabled;
        if loaded.config.gateway.slack_token.is_none() {
            loaded.config.gateway.slack_token = cli_app_config.gateway.slack_token;
        }
    }

    loaded.config.apply_env_overrides()?;
    apply_cli_overrides(&cli, &mut loaded.config);
    install_runtime_config(loaded.config.clone());

    // First-run bootstrap: seed the bundled skill pool (ships with operant)
    // into the user skills directory when it is empty or missing. No-op once
    // the user has any skill installed; `operant skills seed --force` re-runs.
    // Best-effort — a missing pool must never block startup.
    {
        let skills_dir = &loaded.config.skills.root_dir;
        let empty = std::fs::read_dir(skills_dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true);
        if empty && let Err(e) = cmd_skills::seed_bundled_skills(&loaded.config, None, false) {
            tracing::debug!(error = %e, "bundled skill seeding skipped (non-fatal)");
        }
    }

    // Wire delegation config to SubAgentTool statics
    if let Some(depth) = cli_config.delegation.max_spawn_depth {
        operant_core::tools::sub_agent_tool::set_max_spawn_depth(depth);
    }
    if let Some(enabled) = cli_config.delegation.orchestrator_enabled {
        operant_core::tools::sub_agent_tool::set_orchestrator_enabled(enabled);
    }
    if let Some(count) = cli_config.delegation.max_concurrent_children {
        operant_core::tools::sub_agent_tool::set_max_concurrent_children(count as usize);
    }

    // The gateway daemon is headless — TUI rich-output mode would route its
    // logs to io::sink() (silently discarded), which makes the gateway
    // undebuggable. A daemon must never silently lose logs: `gateway run`
    // forces stderr (or the configured log_file) regardless of rich_output.
    let is_gateway_daemon = matches!(
        cli.command,
        Some(Commands::Gateway {
            cmd: cmd_gateway::GatewaySubcommand::Run,
            ..
        })
    );
    let effective_rich_output = loaded.config.tui.rich_output && !is_gateway_daemon;

    // Bridge [agent] show_reasoning into the TUI display shim before any TUI
    // launch: the shim's static snapshot never reads TOML, so the reasoning
    // display mode rides the runtime override cell. true = Current: the turn's
    // trace stays where it streamed until the next prompt (jcode); false = Off.
    // `/thinking-display full` keeps every trace.
    {
        use crate::tui::operant_app::config_shim::{self, ReasoningDisplayMode};
        let mode = if loaded.config.agent.show_reasoning {
            ReasoningDisplayMode::Current
        } else {
            ReasoningDisplayMode::Off
        };
        config_shim::set_reasoning_display_override(Some(mode));
    }

    init_logging(
        cli.verbose,
        cli.log_level.as_deref(),
        &loaded.config.logging,
        effective_rich_output,
    )?;

    if cli.debug_tui {
        tracing::debug!(target: "tui_wiring", "--debug-tui flag enabled");
    }

    match &cli.command {
        Some(Commands::Run {
            system,
            query,
            autonomous,
            record_trajectory,
            moa,
        }) => {
            if *autonomous {
                if query.is_some() {
                    anyhow::bail!(
                        "Do not combine 'run --autonomous' with '--query'. Autonomous mode reads TODO.md from the workspace."
                    );
                }
                autonomous::run_autonomous(loaded.config.clone(), system.clone()).await?;
                return Ok(());
            }
            let query = query
                .as_ref()
                .context("No query provided. Use --query or start chat mode.")?;
            if loaded.config.tui.rich_output && tui_available() {
                TuiApp::enter(
                    loaded.config.clone(),
                    system.clone(),
                    LaunchMode::Query(query.clone()),
                    cli.no_mouse,
                    cli.dangerously_skip_permissions,
                )
                .await?
                .run()
                .await?;
            } else {
                warn_tui_fallback(loaded.config.tui.rich_output);
                run_non_tui(
                    &loaded.config,
                    system.as_deref(),
                    query,
                    *record_trajectory,
                    *moa,
                )
                .await?;
            }
        }
        Some(Commands::Chat { system }) => {
            if loaded.config.tui.rich_output && tui_available() {
                TuiApp::enter(
                    loaded.config.clone(),
                    system.clone(),
                    LaunchMode::Landing,
                    cli.no_mouse,
                    cli.dangerously_skip_permissions,
                )
                .await?
                .run()
                .await?;
            } else {
                warn_tui_fallback(loaded.config.tui.rich_output);
                chat_non_tui(&loaded.config, system.as_deref()).await?;
            }
        }
        Some(Commands::Tools { cmd }) => {
            cmd_tools::handle_tools_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Autonomous { system }) => {
            autonomous::run_autonomous(loaded.config.clone(), system.clone()).await?;
        }
        Some(Commands::Test { tool_name, args }) => {
            test_tool(&loaded.config, tool_name, args.as_deref()).await?;
        }
        Some(Commands::Config { cmd }) => {
            cmd_config::handle_config_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Sessions { cmd, json }) => {
            cmd_sessions::handle_sessions_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Mcp { cmd, json }) => {
            let mcp_manager = operant_core::mcp::McpManager::new();
            cmd_mcp::handle_mcp_command(&loaded.config, &mcp_manager, cmd.clone(), *json).await?;
        }
        Some(Commands::Cookies { cmd }) => {
            cmd_cookies::handle_cookies_command(cmd.clone()).await?;
        }
        Some(Commands::Skills { cmd, json }) => {
            cmd_skills::handle_skills_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Model { cmd, json }) => {
            cmd_model::handle_model_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Completion { cmd }) => {
            cmd_completion::handle_completion_command(cmd.clone())?;
        }
        Some(Commands::Cron { cmd, json }) => {
            cmd_cron::handle_cron_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Pause { reason }) => {
            let detail = reason
                .clone()
                .unwrap_or_else(|| "engaged via `operant pause`".to_string());
            operant_core::estop::engage(Some(&detail))?;
            println!("⏸️ Operant paused (new work only). Resume with `operant resume`.");
        }
        Some(Commands::Resume) => {
            operant_core::estop::disengage()?;
            println!("Operant resumed.");
        }
        Some(Commands::Suggestions { cmd, json }) => {
            cmd_suggestions::handle_suggestions_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Kanban { board, cmd, json }) => {
            cmd_kanban::handle_kanban_command(&loaded.config, board, cmd.clone(), *json).await?;
        }
        Some(Commands::Org { cmd }) => {
            cmd_org::handle_org_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Gateway { cmd, json }) => {
            cmd_gateway::handle_gateway_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Checkpoints { cmd }) => {
            cmd_checkpoints::handle_checkpoints_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Memory { cmd }) => {
            cmd_memory::handle_memory_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Context { cmd }) => {
            cmd_context::handle_context_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Profile { cmd }) => {
            cmd_profile::handle_profile_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Auth { cmd }) => {
            cmd_auth::handle_auth_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Login) => {
            cmd_auth::handle_login(&loaded.config).await?;
        }
        Some(Commands::Logout) => {
            cmd_auth::handle_logout(&loaded.config).await?;
        }
        Some(Commands::Version { detailed }) => {
            cmd_version::handle_version_command(&loaded.config, *detailed).await?;
        }
        Some(Commands::Doctor { fix, json }) => {
            cmd_doctor::handle_doctor_command(&loaded.config, *fix, *json).await?;
        }
        Some(Commands::Status { deep, json }) => {
            cmd_status::handle_status_command(&loaded.config, *deep, *json).await?;
        }
        Some(Commands::Dump { all, json }) => {
            cmd_dump::handle_dump_command(&loaded.config, *all, *json).await?;
        }
        Some(Commands::Logs { cmd }) => {
            cmd_logs::handle_logs_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Backup { cmd }) => {
            cmd_backup::handle_backup_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Import { cmd }) => {
            cmd_import::handle_import_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Uninstall { cmd }) => {
            cmd_uninstall::handle_uninstall_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Update { cmd }) => {
            cmd_update::handle_update_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Insights { cmd }) => {
            cmd_insights::handle_insights_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Webhook { cmd }) => {
            cmd_webhook::handle_webhook_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Hooks { cmd }) => {
            cmd_hooks::handle_hooks_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Debug { cmd }) => {
            cmd_debug::handle_debug_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Architecture { cmd }) => {
            cmd_architecture::handle_architecture_command(cmd.clone()).await?;
        }
        Some(Commands::Plugins { cmd }) => {
            cmd_plugins::handle_plugins_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Curator { cmd, json }) => {
            cmd_curator::handle_curator_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Channel { cmd, json }) => {
            // apply_cli_overrides above stamped the global --api-key/--base-url/
            // --model onto the loaded config (LLM-override semantics). Channel
            // commands persist the config back to disk, so restore the file's
            // original values first — otherwise a channel-token passed as
            // --api-key would clobber the real LLM key on the next launch.
            loaded.config.client.api_key = pre_override_client_api_key.clone();
            loaded.config.client.base_url = pre_override_client_base_url.clone();
            loaded.config.agent.model = pre_override_agent_model.clone();
            cmd_channel::handle_channel_command(
                &mut loaded.config,
                loaded.source.as_deref(),
                cmd.clone(),
                *json,
            )
            .await?;
        }
        Some(Commands::Sop { cmd, json }) => {
            cmd_sop::handle_sop_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Migrate { cmd, json }) => {
            cmd_migrate::handle_migrate_command(&loaded.config, cmd.clone(), *json).await?;
        }
        Some(Commands::Service { cmd, json }) => {
            cmd_service::handle_service_command(&loaded.config, cmd.clone(), *json)?;
        }
        Some(Commands::Setup {
            section,
            non_interactive,
            reset,
            reconfigure,
            quick,
            json,
        }) => {
            cmd_setup::handle_setup_command(
                &loaded.config,
                section.as_deref(),
                *non_interactive,
                *reset,
                *reconfigure,
                *quick,
                *json,
            )
            .await?;
        }
        Some(Commands::Acp { cmd }) => {
            cmd_acp::handle_acp_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Dashboard { cmd }) => {
            cmd_dashboard::handle_dashboard_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Trajectory { cmd }) => {
            cmd_trajectory::handle_trajectory_command(cmd.clone()).await?;
        }
        Some(Commands::Tui { cmd }) => {
            cmd_tui_debug::handle_tui_command(&loaded.config, cmd.clone()).await?;
        }
        Some(Commands::Donate) => {
            println!("Star the repo  : https://github.com/ishan-parihar/operant");
            println!("GitHub Sponsors: https://github.com/sponsors/ishan-parihar");
            println!("Razorpay (UPI) : https://rzp.io/rzp/ishan-parihar");
        }
        None => {
            // No command provided - launch TUI in interactive mode
            if loaded.config.tui.rich_output {
                TuiApp::enter(
                    loaded.config.clone(),
                    None,
                    LaunchMode::Landing,
                    cli.no_mouse,
                    cli.dangerously_skip_permissions,
                )
                .await?
                .run()
                .await?;
            } else {
                chat_non_tui(&loaded.config, None).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A log path that cannot be opened is a startup error that names the path,
    /// not a panic. The open happens before the subscriber is installed, so
    /// this test does not touch the process-wide tracing subscriber.
    #[test]
    fn init_logging_reports_an_unopenable_log_file_instead_of_panicking() {
        let logging = LoggingSettings {
            log_file: Some("/nonexistent-operant-dir/operant.log".to_string()),
            ..LoggingSettings::default()
        };
        let err = init_logging(false, None, &logging, false)
            .expect_err("an unopenable log file must be an error");
        let text = format!("{err:#}");
        assert!(text.contains("failed to open log file"), "{text}");
        assert!(
            text.contains("/nonexistent-operant-dir/operant.log"),
            "{text}"
        );
    }

    /// A configured log file is rotated when it grows past the cap.
    ///
    /// The file was opened in APPEND mode and never rotated: the
    /// date-stamped `operant-*.log` files are pruned after 7 days, but a
    /// CONFIGURED file was not on that list. Measured on this box:
    /// `~/.operant/logs/gateway.log` had reached 90 MB and 1.7M lines after
    /// about three weeks of the gateway running.
    #[test]
    fn a_configured_log_file_is_rotated_when_it_grows_past_the_cap() {
        let dir = std::env::temp_dir().join(format!(
            "operant_log_rotate_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("gateway.log");

        // A small cap so the test does not have to write 32 MB. TWO caps'
        // worth of bytes, so a rotation is guaranteed to happen.
        let mut writer = RollingLogFile::open(&path, 100).expect("open the log file");
        for _ in 0..20 {
            writer.write_all(b"0123456789").expect("write a line");
        }
        writer.flush().expect("flush");

        // The live file is bounded by the cap, and the overflow is in a
        // rotated generation.
        let live = std::fs::metadata(&path)
            .expect("the live file exists")
            .len();
        assert!(
            live <= 100,
            "the live file must stay at or under the cap, got {live}"
        );
        let rotated = dir.join("gateway.log.1");
        assert!(
            rotated.exists(),
            "the overflow must land in a rotated generation"
        );
        let rotated_len = std::fs::metadata(&rotated).expect("the rotated file").len();
        assert!(
            rotated_len > 0,
            "the rotated generation must hold the overflow"
        );
        // And the content is all there, split across the two files.
        let live_text = std::fs::read_to_string(&path).expect("read the live file");
        let rotated_text = std::fs::read_to_string(&rotated).expect("read the rotated file");
        assert_eq!(
            live_text.len() + rotated_text.len(),
            200,
            "no log line may be lost by the rotation"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rotation keeps a BOUNDED number of generations: the live file plus
    /// two rotated ones. An unbounded rotation would just move the growth
    /// problem.
    #[test]
    fn a_log_rotation_keeps_a_bounded_number_of_generations() {
        let dir = std::env::temp_dir().join(format!(
            "operant_log_gens_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("gateway.log");

        // Three rotations, so a fourth generation would be needed to
        // overflow the bound.
        for _ in 0..3 {
            let mut writer = RollingLogFile::open(&path, 10).expect("open the log file");
            writer.write_all(b"0123456789abcdef").expect("write");
            writer.write_all(b"0123456789abcdef").expect("write");
            writer.flush().expect("flush");
        }

        assert!(path.exists(), "the live file survives");
        assert!(dir.join("gateway.log.1").exists(), ".1 survives");
        assert!(dir.join("gateway.log.2").exists(), ".2 survives");
        assert!(
            !dir.join("gateway.log.3").exists(),
            "a third generation must not be kept: the bound is two"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The regression this pins: with stdin closed, `operant chat` spun forever
    /// printing `You: ` because EOF and a blank line both produced an empty
    /// string. EOF must be `None`.
    #[test]
    fn read_line_reports_eof_as_none() {
        let mut empty = Cursor::new(b"");
        assert_eq!(
            read_line_or_eof(&mut empty).unwrap(),
            None,
            "an exhausted stream must be None, or the chat loop spins forever"
        );
    }

    /// A blank line is NOT EOF. The chat loop's `continue` on a blank line is
    /// correct interactive behaviour — pressing Enter should just reprint the
    /// prompt — so collapsing the two cases would fix the hang by breaking
    /// that.
    #[test]
    fn read_line_distinguishes_blank_line_from_eof() {
        let mut blank = Cursor::new(b"\n");
        assert_eq!(
            read_line_or_eof(&mut blank).unwrap(),
            Some("\n".to_string()),
            "a blank line must read as Some, not None"
        );
    }

    #[test]
    fn read_line_returns_input() {
        let mut input = Cursor::new(b"hello world\n");
        assert_eq!(
            read_line_or_eof(&mut input).unwrap(),
            Some("hello world\n".to_string())
        );
    }

    /// The exact loop shape: one real line, then a blank line, then EOF. This
    /// is what a piped stdin that ends in a newline produces, and it is the
    /// sequence that hung.
    #[test]
    fn read_line_walks_a_piped_stream_to_eof() {
        let mut reader = Cursor::new(b"first\n\n");
        assert_eq!(
            read_line_or_eof(&mut reader).unwrap(),
            Some("first\n".to_string())
        );
        assert_eq!(
            read_line_or_eof(&mut reader).unwrap(),
            Some("\n".to_string())
        );
        assert_eq!(
            read_line_or_eof(&mut reader).unwrap(),
            None,
            "the stream must terminate rather than yield blank lines forever"
        );
    }

    /// RC6 (plan 2026-10-25 tooling-surface): the interactive TUI with no
    /// configured log file defaults to the home log instead of the old
    /// discard sink. The pure seam is pinned here; the end-to-end
    /// "a fresh HERMES_HOME run writes the file" contract is verified
    /// out-of-process (the binary run) because `operant_home()` reads
    /// `HERMES_HOME` process-wide and test threads race on it.
    #[test]
    fn a_rich_tui_without_a_log_file_adopts_the_home_log() {
        let logging = LoggingSettings::default();
        assert!(
            tui_wants_default_log_file(&logging, true),
            "the default applies to a rich TUI with no log_file"
        );
        assert!(
            !tui_wants_default_log_file(&logging, false),
            "non-TUI paths keep stderr"
        );
        let explicit = LoggingSettings {
            log_file: Some("my.log".to_string()),
            ..Default::default()
        };
        assert!(
            !tui_wants_default_log_file(&explicit, true),
            "an explicit log_file always wins"
        );
        assert!(
            default_tui_log_file()
                .to_string_lossy()
                .ends_with("logs/operant.log"),
            "the default lives under the home's logs dir: {:?}",
            default_tui_log_file()
        );
        // The raw selector is unchanged: it still routes a None+rich
        // request to the sink — the DEFAULT above rewrites log_file
        // before this selector runs, which is the whole point.
        assert_eq!(select_log_target(&logging, true), LogTarget::Sink);
    }

    /// The example template ships `log_file = ""`; an empty string is
    /// unset, not a file named "".
    #[test]
    fn an_empty_log_file_string_is_unsettled() {
        let logging = LoggingSettings {
            log_file: Some(String::new()),
            ..Default::default()
        };
        assert!(
            !tui_wants_default_log_file(&logging, false),
            "sanity: the seam reads the normalized flag, not the raw string"
        );
        // The normalization itself lives in init_logging; exercise it via
        // the same predicate the call site uses after normalizing.
        let normalized = if logging.log_file.as_deref().is_some_and(str::is_empty) {
            LoggingSettings {
                log_file: None,
                ..logging.clone()
            }
        } else {
            logging.clone()
        };
        assert!(normalized.log_file.is_none());
        assert!(tui_wants_default_log_file(&normalized, true));
    }

    #[test]
    fn log_file_overrides_sink() {
        let logging = LoggingSettings {
            log_file: Some("operant.log".to_string()),
            ..Default::default()
        };
        assert_eq!(select_log_target(&logging, true), LogTarget::File);
    }

    // (plan 08) load_memory_manager_reads_existing_memory_file deleted:
    // it pinned the LEGACY MemoryManager file round-trip (MEMORY.md seed →
    // read back), exactly the split-surface path plan 08 retires. The
    // engine the agent actually recalls from is memory-wire, covered by
    // memory_wire.rs's own wiring tests; the manager's remaining role
    // (turn sequencing) carries no file contract to pin here.

    /// BUGS.md D-5 — the hoisted memory pair must be the pair the agent
    /// core is built with. Two `build_agent_core_with_memory` calls given
    /// ONE pair must yield `Arc::ptr_eq` providers (the SAME backend), not
    /// a second construction. The gateway relies on exactly this when it
    /// threads one `load_repo_memory_manager()` pair into the gateway
    /// agent AND the cron agent; red here means a build silently reloaded
    /// and the two agents are clobbering each other's MEMORY.md again.
    #[tokio::test]
    async fn hoisted_memory_pair_is_shared_across_two_core_builds() {
        let dir =
            std::env::temp_dir().join(format!("operant_cli_memory_hoist_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = AppConfig {
            database_path: dir.join("database.db"),
            ..AppConfig::default()
        };
        config.skills.root_dir = dir.join("skills");
        config.skills.memory_dir = dir.join("memory");
        std::fs::create_dir_all(&config.skills.root_dir).unwrap();

        // The hoisted pair, exactly as the gateway constructs it once: one
        // manager + one provider over the same store.
        let memory_manager = MemoryManager::with_storage_dir(dir.clone());
        let memory_provider: Arc<dyn MemoryProvider> =
            operant_core::memory_provider::build_memory_provider("builtin", dir.clone());

        let mcp_manager = McpManager::new();
        let build_core = || async {
            build_agent_core_with_memory(
                &config,
                None,
                &mcp_manager,
                &config.skills.root_dir,
                &config.agent.model,
                &config.agent,
                None,
                memory_manager.clone(),
                Some(memory_provider.clone()),
            )
            .await
            .unwrap()
        };

        let core_one = build_core().await;
        let core_two = build_core().await;

        let first = core_one.memory_provider.as_ref().unwrap();
        let second = core_two.memory_provider.as_ref().unwrap();
        assert!(
            Arc::ptr_eq(first, second),
            "D-5: two agent-core builds over one hoisted pair must share the SAME provider instance"
        );
        assert!(
            Arc::ptr_eq(first, &memory_provider),
            "D-5: the core must use the caller-supplied provider, not a freshly constructed one"
        );

        // The manager half must be shared state too (MemoryManager is all
        // Arc<RwLock> inside, so a clone is the SAME state): what one build
        // stores, the other must see. Two independent managers over one
        // MEMORY.md is the clobber in the bug title.
        core_one
            .memory_manager
            .store(
                operant_core::memory::MemoryBlock::new("d5_probe", "fact", "hoisted pair probe")
                    .importance(50),
            )
            .await;
        assert_eq!(
            core_two
                .memory_manager
                .search("hoisted pair probe")
                .await
                .len(),
            1,
            "D-5: the two cores must share ONE MemoryManager state; a fresh manager would see nothing"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // R-08 (AUDIT-2026-10-16-capability-layer): the loader is the one place
    // a process decides to OPEN a memory provider, so the Holon gate lives
    // here and nowhere else: under HOLON_RUN_DIR no provider is built and
    // not even the sqlite appears; personal mode (same default config,
    // memory.enabled = true) builds one. #[serial]: HOLON_RUN_DIR is
    // process-global and runtime_config() is read inside.
    #[serial_test::serial]
    #[tokio::test]
    async fn a_holon_run_opens_no_memory_provider_r08() {
        let dir = std::env::temp_dir().join(format!("operant_cli_r08_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("holon")).unwrap();
        std::fs::create_dir_all(dir.join("personal")).unwrap();

        // SAFETY: #[serial] — the only test running while this is set.
        unsafe { std::env::set_var("HOLON_RUN_DIR", dir.join("holon")) };
        let (_m, provider) = load_memory_manager(dir.join("holon")).await.unwrap();
        assert!(
            provider.is_none(),
            "a Holon run opens no memory provider — the kernel's bundle is the run's memory (W6)"
        );
        assert!(
            !dir.join("holon").join("memory_wire.sqlite").exists(),
            "gated before construction: not even the bank file appears in the run's home"
        );
        unsafe { std::env::remove_var("HOLON_RUN_DIR") };

        let (_m, provider) = load_memory_manager(dir.join("personal")).await.unwrap();
        assert!(
            provider.is_some(),
            "personal mode keeps its memory provider under the same default config"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn autonomous_subcommand_parses() {
        let cli = Cli::try_parse_from(["operant", "autonomous"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Autonomous { .. })));
    }

    #[test]
    fn run_autonomous_flag_parses() {
        let cli = Cli::try_parse_from(["operant", "run", "--autonomous"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Run {
                autonomous: true,
                ..
            })
        ));
    }

    // ── Provider fallback wiring (hermes `fallback_providers` parity) ──

    fn test_config_with_providers() -> AppConfig {
        let mut config = AppConfig::default();
        config.agent.model = "laguna-s-2.1-free".to_string();
        config.agent.fallback_models = vec!["deepseek-v4-flash-free".to_string()];
        config.client.api_key = Some("sk-primary".to_string());
        config.providers.models.insert(
            "zen-fallback".to_string(),
            ModelProviderConfig {
                base_url: Some("https://zen.example/v1".to_string()),
                model: Some("deepseek-v4-flash-free".to_string()),
                api_key: Some("sk-fallback".to_string()),
                ..Default::default()
            },
        );
        config
            .providers
            .fallback_chain
            .push(operant_core::config::FallbackProviderConfig {
                provider: "zen-fallback".to_string(),
                model: "deepseek-v4-flash-free".to_string(),
            });
        config
    }

    #[test]
    fn provider_profile_overrides_layer_onto_primary() {
        let primary = ClientConfig {
            base_url: "https://primary.example/v1".to_string(),
            api_key: Some("sk-primary".to_string()),
            timeout: Duration::from_secs(60),
            max_context_length: 200_000,
            rate_limit: Default::default(),
            prompt_cache_ttl: Default::default(),
        };
        let profile = ModelProviderConfig {
            base_url: Some("https://zen.example/v1".to_string()),
            api_key: None, // inherit primary key
            timeout_secs: Some(300),
            ..Default::default()
        };
        let ccfg = client_config_from_provider(&primary, &profile);
        assert_eq!(ccfg.base_url, "https://zen.example/v1");
        assert_eq!(ccfg.api_key.as_deref(), Some("sk-primary"));
        assert_eq!(ccfg.timeout, Duration::from_secs(300));
    }

    fn test_pool_registry(
        config: &AppConfig,
        provider: &str,
    ) -> std::sync::Arc<operant_core::credential_pool::CredentialPoolRegistry> {
        build_pool_registry(config, provider, &client_config(config))
    }

    #[test]
    fn build_provider_registry_orders_chain_primary_first() {
        let config = test_config_with_providers();
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools)
                .unwrap();

        assert_eq!(registry.active_provider().as_deref(), Some("free"));
        // Chain: primary + configured fallback entry.
        let switched = registry.switch_to_next().unwrap();
        assert_eq!(switched.name, "zen-fallback");
        assert_eq!(switched.model, "deepseek-v4-flash-free");
        assert!(registry.get_client("zen-fallback").is_some());
        // Primary client is registered too.
        assert!(registry.get_client("free").is_some());
    }

    #[test]
    fn build_provider_registry_skips_unknown_provider_entries() {
        let mut config = test_config_with_providers();
        config
            .providers
            .fallback_chain
            .push(operant_core::config::FallbackProviderConfig {
                provider: "ghost".to_string(),
                model: "nope".to_string(),
            });
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools)
                .unwrap();
        // Unknown entry is skipped — only the valid fallback remains.
        assert!(registry.switch_to_next().is_some());
        // Chain is exhausted after the one valid fallback.
        assert!(registry.switch_to_next().is_none());
    }

    #[test]
    fn build_provider_registry_none_without_chain() {
        let config = AppConfig::default(); // no [providers]
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "openai");
        assert!(
            build_provider_registry(&config, "openai", "gpt-4o", &primary_cfg, &pools).is_none(),
            "no [providers] → no registry"
        );
    }

    #[test]
    fn fallback_models_flow_into_wrapper_chain() {
        use operant_core::agent::FallbackModelClient;

        // `agent.fallback_models` is the same-client model swap list. The
        // wrapper is only constructed when the config says so — verify the
        // gating: fallback_on_errors=false with a configured chain returns the
        // raw primary client (early return, no FallbackModelClient built).
        let mut config = test_config_with_providers();
        config.agent.fallback_on_errors = false;
        let (client, _pool_registry) =
            create_model_client_with_fallback("free", "laguna-s-2.1-free", &config, None);
        // Smoke: the returned client is usable and reports a provider.
        assert!(!client.provider_name().is_empty());

        // And with fallback enabled, FallbackModelClient::new is what wraps it
        // (its model_chain is [primary, fallback_models...] — verified in the
        // core crate's 17 fallback tests). Here we only prove the config
        // surfaces are read: fallback_models + fallback_chain both reach the
        // builder.
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools);
        assert!(registry.is_some(), "chain configured → registry built");
        // FallbackModelClient type is constructible from the same pieces.
        let inner: std::sync::Arc<dyn operant_core::agent::ModelClient> =
            std::sync::Arc::new(OpenAIModelClient::new(OpenAIClient::new(primary_cfg)));
        let _fb = FallbackModelClient::new(
            inner,
            "laguna-s-2.1-free".to_string(),
            config.agent.fallback_models.clone(),
            true,
        )
        .with_provider_registry(registry.unwrap());
    }

    // ── Per-provider on-demand pools (hermes `load_pool` parity) ──

    #[test]
    fn pool_registry_seeds_primary_and_profiles() {
        let config = test_config_with_providers();
        let pools = test_pool_registry(&config, "free");

        // Primary provider seeded from config client key.
        assert!(pools.has_credentials("free"));
        let primary_pool = pools.get_or_create("free");
        assert_eq!(primary_pool.len(), 1);
        assert_eq!(
            primary_pool.peek().map(|c| c.value),
            Some("sk-primary".to_string())
        );

        // Fallback profile provider seeded from its own api_key.
        assert!(pools.has_credentials("zen-fallback"));
        let fb_pool = pools.get_or_create("zen-fallback");
        assert_eq!(fb_pool.len(), 1);
        assert_eq!(
            fb_pool.peek().map(|c| c.value),
            Some("sk-fallback".to_string())
        );

        // Providers list contains both.
        let providers = pools.providers();
        assert!(providers.contains(&"free".to_string()));
        assert!(providers.contains(&"zen-fallback".to_string()));
    }

    #[test]
    fn pool_registry_dedupes_identical_keys() {
        let config = test_config_with_providers();
        let pools = test_pool_registry(&config, "free");
        // Pool already has sk-primary (config) — adding the same value twice
        // must produce one entry, not two (hermes sibling-marking is avoided
        // by preventing the duplicate at seed time).
        pools.add_seed("free", "dup-1", "sk-fallback", "test");
        pools.add_seed("free", "dup-2", "sk-fallback", "test");
        let pool = pools.get_or_create("free");
        assert_eq!(pool.len(), 2, "sk-primary + one deduped sk-fallback");
        let values: Vec<String> = pool.list().into_iter().map(|c| c.value).collect();
        assert_eq!(values.iter().filter(|v| *v == "sk-fallback").count(), 1);
    }

    #[test]
    fn pool_registry_lazy_creation_shared_pool() {
        let config = AppConfig::default();
        let pools = test_pool_registry(&config, "openai");
        // No pool exists before first use.
        assert!(pools.pool("openai").is_none());
        let first = pools.get_or_create("openai");
        let second = pools.get_or_create("openai");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "pool is cached and shared"
        );
    }

    #[test]
    fn pooled_registry_clients_are_pooled() {
        let config = test_config_with_providers();
        let primary_cfg = client_config(&config);
        let pools = test_pool_registry(&config, "free");
        let registry =
            build_provider_registry(&config, "free", "laguna-s-2.1-free", &primary_cfg, &pools)
                .unwrap();
        // Building the registry consults the per-provider pool registry
        // (hermes `load_pool(fb_provider)` parity) — the fallback provider's
        // pool must be materialized and seeded during build.
        assert!(registry.get_client("zen-fallback").is_some());
        let fb_pool = pools
            .pool("zen-fallback")
            .expect("fallback pool materialized");
        assert_eq!(fb_pool.len(), 1);
        assert_eq!(
            fb_pool.peek().map(|c| c.value),
            Some("sk-fallback".to_string())
        );
        // Primary provider's pool is shared with the agent-path wrapper.
        assert!(pools.pool("free").is_some());
    }
}
