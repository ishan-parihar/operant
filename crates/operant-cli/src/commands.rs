//! Interactive Slash Command Registry.
//!
//! Ported from operant-agent/operant_cli/commands.py COMMAND_REGISTRY.
//! All slash commands are defined here and consumed by the CLI chat loop,
//! gateway dispatchers, and any other interactive context.
//!
//! The registry provides:
//! - Static command metadata definitions (name, description, category, aliases)
//! - Dynamic handler registration for runtime dispatch
//! - Help text formatting organized by category
//! - Command resolution (name + alias → canonical name)

use std::collections::HashMap;
use std::fmt;

use anyhow::Result;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Context provided to a command handler on execution.
#[derive(Debug, Default)]
pub struct CommandContext<'a> {
    /// The raw argument string following the command name.
    pub args: &'a str,
}

/// Rich result type for slash command execution.
///
/// Instead of just returning a string, commands express their *intent* as data.
/// The TUI (or gateway) interprets the variant to perform the side effect.
/// This decouples command logic from the runtime environment.
///
/// Ported from the Claurst `CommandResult` pattern, adapted for Operant's TUI.
#[must_use]
#[non_exhaustive]
pub enum CommandResult {
    // ── Display ────────────────────────────────────────────────────────────
    /// Display a message to the user (does NOT go to the model).
    Message(String),

    // ── Conversation ───────────────────────────────────────────────────────

    // ── Configuration ──────────────────────────────────────────────────────

    // ── Overlay / UI ───────────────────────────────────────────────────────

    // ── Session state ──────────────────────────────────────────────────────

    // ── Clipboard ──────────────────────────────────────────────────────────

    // ── Shell ──────────────────────────────────────────────────────────────
    /// Run a shell command (e.g. /setup spawns an interactive subprocess).
    ShellCommand(Vec<String>),
    // ── Exit ───────────────────────────────────────────────────────────────
}

impl CommandResult {
    /// Convenience: create a Message variant.
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}

impl fmt::Debug for CommandResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(s) => write!(f, "Message({:?})", s),
            Self::ShellCommand(cmds) => write!(f, "ShellCommand({:?})", cmds),
        }
    }
}

impl fmt::Display for CommandResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(s) => write!(f, "{}", s),
            Self::ShellCommand(cmds) => write!(f, "ShellCommand({:?})", cmds),
        }
    }
}

impl From<String> for CommandResult {
    fn from(s: String) -> Self {
        Self::Message(s)
    }
}

impl From<&str> for CommandResult {
    fn from(s: &str) -> Self {
        Self::Message(s.to_string())
    }
}

/// A slash command handler that can be registered in the [`CommandRegistry`].
///
/// Implementations should be stateless or use interior mutability, since the
/// same handler may be called multiple times.
#[async_trait::async_trait]
pub trait CommandHandler: Send + Sync {
    /// Execute this command with the given context.
    /// Returns a [`CommandResult`] expressing the command's intent as data.
    async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult;
}

// ---------------------------------------------------------------------------
// CommandDef — static metadata
// ---------------------------------------------------------------------------

/// Metadata-only command definition.
///
/// This struct is `const`-compatible so the built-in registry can be a static
/// slice. Handler functions are registered separately at runtime via
/// [`CommandRegistry::register_handler`].
#[derive(Debug, Clone)]
pub struct CommandDef {
    /// Canonical name (without the leading slash).
    pub name: &'static str,
    /// One-line human-readable description.
    pub description: &'static str,
    /// Alternative names that resolve to this command.
    pub aliases: &'static [&'static str],
    /// Argument hint shown in help (e.g. `"<prompt>"`, `"<key> <value>"`).
    pub args_hint: &'static str,
    /// Only available in interactive CLI (not in gateway/messaging).
    pub cli_only: bool,
    /// Only available in messaging platforms (not in CLI).
    pub gateway_only: bool,
    /// Config dotpath that gates this command in gateway mode.
    /// When set on a `cli_only` command, the command becomes available
    /// in the gateway if the config value is truthy.
    pub gateway_config_gate: Option<&'static str>,
}

impl CommandDef {
    /// Create a new command definition with minimal fields.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            aliases: &[],
            args_hint: "",
            cli_only: false,
            gateway_only: false,
            gateway_config_gate: None,
        }
    }

    /// Set aliases for this command.
    pub const fn with_aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    /// Set the argument hint.
    pub const fn with_args(mut self, hint: &'static str) -> Self {
        self.args_hint = hint;
        self
    }

    /// Mark this command as CLI-only.
    pub const fn cli_only(mut self) -> Self {
        self.cli_only = true;
        self
    }

    /// Mark this command as gateway-only.
    pub const fn gateway_only(mut self) -> Self {
        self.gateway_only = true;
        self
    }

    pub const fn with_config_gate(mut self, gate: &'static str) -> Self {
        self.gateway_config_gate = Some(gate);
        self
    }
}

// ---------------------------------------------------------------------------
// Built-in command registry (static)
// ---------------------------------------------------------------------------

/// All built-in slash commands, ordered by category.
///
/// Each entry is metadata only; handlers are registered dynamically in
/// [`CommandRegistry`] at startup.
pub static COMMAND_REGISTRY: &[CommandDef] = &[
    // ── Session ─────────────────────────────────────────────────────────────
    CommandDef::new("start", "Acknowledge platform start pings without a reply").gateway_only(),
    CommandDef::new("new", "Start a new session (fresh session ID + history)")
        .with_args("[name]")
        .with_aliases(&["n", "clear"]),
    CommandDef::new("topic", "Enable or inspect Telegram DM topic sessions")
        .gateway_only()
        .with_args("[off|help|session-id]"),
    CommandDef::new(
        "redraw",
        "Force a full UI repaint (recovers from terminal drift)",
    )
    .cli_only(),
    CommandDef::new("save", "Save the current conversation")
        .cli_only()
        .with_aliases(&["export"]),
    CommandDef::new("retry", "Retry the last message (resend to agent)"),
    CommandDef::new("undo", "Back up N user turns and re-prompt (default 1)").with_args("[N]"),
    CommandDef::new("title", "Set a title for the current session").with_args("[name]"),
    CommandDef::new(
        "branch",
        "Branch the current session (explore a different path)",
    )
    .with_args("[name]"),
    CommandDef::new(
        "compress",
        "Compress conversation context (add 'here [N]' to keep recent N turns)",
    )
    .with_args("[here [N] | focus topic]"),
    CommandDef::new("rollback", "List or restore filesystem checkpoints").with_args("[number]"),
    CommandDef::new("stop", "Kill all running background processes"),
    CommandDef::new("approve", "Approve a pending dangerous command")
        .gateway_only()
        .with_args("[session|always]"),
    CommandDef::new("deny", "Deny a pending dangerous command").gateway_only(),
    CommandDef::new("background", "Run a prompt in the background")
        .with_aliases(&["bg"])
        .with_args("<prompt>"),
    CommandDef::new("agents", "Show active agents and running tasks").with_aliases(&["tasks"]),
    CommandDef::new(
        "queue",
        "Queue a prompt for the next turn (doesn't interrupt)",
    )
    .with_args("<prompt>"),
    CommandDef::new(
        "steer",
        "Inject a message after the next tool call without interrupting",
    )
    .with_args("<prompt>"),
    CommandDef::new(
        "goal",
        "Set a standing goal Operant works on across turns until achieved",
    )
    .with_args("[text | pause | resume | clear | status]"),
    CommandDef::new("subgoal", "Add or manage extra criteria on the active goal")
        .with_args("[text | remove N | clear]"),
    CommandDef::new("status", "Show session, model, token, and context info"),
    CommandDef::new("resume", "Resume a previously-named session").with_args("[name]"),
    CommandDef::new("sethome", "Set this chat as the home channel")
        .gateway_only()
        .with_aliases(&["set-home"]),
    CommandDef::new("sessions", "Browse and resume previous sessions"),
    // ── Configuration ───────────────────────────────────────────────────────
    CommandDef::new("model", "Switch model for this session")
        .with_args("[model] [--provider name] [--global] [--refresh]"),
    CommandDef::new("provider", "Switch LLM provider").with_args("<name>"),
    CommandDef::new("config", "Show current configuration")
        .cli_only()
        .with_args("[key] [value]"),
    CommandDef::new(
        "codex-runtime",
        "Toggle codex app-server runtime for OpenAI/Codex models",
    )
    .with_aliases(&["codex_runtime"])
    .with_args("[auto|codex_app_server]"),
    CommandDef::new("profile", "Show active profile name and home directory"),
    CommandDef::new("personality", "Set a predefined personality").with_args("[name]"),
    CommandDef::new("statusbar", "Toggle the context/model status bar")
        .cli_only()
        .with_aliases(&["sb"]),
    CommandDef::new(
        "verbose",
        "Cycle tool progress display: off -> new -> all -> verbose",
    )
    .cli_only()
    .with_config_gate("display.tool_progress_command"),
    CommandDef::new(
        "footer",
        "Toggle gateway runtime-metadata footer on final replies",
    )
    .with_args("[on|off|status]"),
    CommandDef::new(
        "yolo",
        "Toggle YOLO mode (skip all dangerous command approvals)",
    ),
    CommandDef::new("reasoning", "Manage reasoning effort and display")
        .with_args("[level|show|hide]"),
    CommandDef::new("skin", "Show or change the display skin/theme")
        .cli_only()
        .with_args("[name]"),
    CommandDef::new("indicator", "Pick the TUI busy-indicator style")
        .cli_only()
        .with_args("[kaomoji|emoji|unicode|ascii]"),
    CommandDef::new("busy", "Control what Enter does while Operant is working")
        .cli_only()
        .with_args("[queue|steer|interrupt|status]"),
    // ── Tools & Skills ──────────────────────────────────────────────────────
    CommandDef::new(
        "tools",
        "Manage tools: /tools [list|disable|enable] [name...]",
    )
    .cli_only()
    .with_args("[list|disable|enable] [name...]"),
    CommandDef::new("skills", "Search, install, inspect, or manage skills")
        .cli_only()
        .with_aliases(&["skill"])
        .with_config_gate("skills.write_approval"),
    CommandDef::new(
        "memory",
        "Review pending memory writes / toggle the approval gate",
    )
    .with_aliases(&["mem"])
    .with_args("[pending|approve|reject|approval] [id|on|off]"),
    CommandDef::new(
        "bundles",
        "List skill bundles (aliases /<name> for multiple skills)",
    ),
    CommandDef::new("cron", "Manage scheduled tasks")
        .cli_only()
        .with_args("[subcommand]"),
    CommandDef::new(
        "suggestions",
        "Review suggested automations (accept/dismiss)",
    )
    .with_aliases(&["suggest"])
    .with_args("[accept|dismiss N | catalog]"),
    CommandDef::new(
        "curator",
        "Background skill maintenance (status, run, pin, archive, list-archived)",
    )
    .with_args("[subcommand]"),
    CommandDef::new(
        "kanban",
        "Multi-profile collaboration board (tasks, links, comments)",
    )
    .with_aliases(&["k"])
    .with_args("[subcommand]"),
    CommandDef::new("reload", "Reload .env variables into the running session").cli_only(),
    CommandDef::new("reload-mcp", "Reload MCP servers from config").with_aliases(&["reload_mcp"]),
    CommandDef::new(
        "reload-skills",
        "Re-scan ~/.operant/skills/ for newly installed or removed skills",
    )
    .with_aliases(&["reload_skills"]),
    CommandDef::new(
        "browser",
        "Connect browser tools to your live Chromium-family browser via CDP",
    )
    .cli_only()
    .with_args("[connect|disconnect|status]"),
    CommandDef::new("plugins", "List installed plugins and their status").cli_only(),
    // ── Info ─────────────────────────────────────────────────────────────────
    CommandDef::new("commands", "Browse all commands and skills (paginated)")
        .gateway_only()
        .with_args("[page]"),
    CommandDef::new("help", "Show available commands").with_aliases(&["?", "h"]),
    CommandDef::new(
        "restart",
        "Gracefully restart the gateway after draining active runs",
    )
    .gateway_only(),
    CommandDef::new(
        "usage",
        "Show token usage and rate limits for the current session",
    ),
    CommandDef::new("insights", "Show usage insights and analytics").with_args("[days]"),
    CommandDef::new("platforms", "Show gateway/messaging platform status")
        .cli_only()
        .with_aliases(&["gateway"]),
    CommandDef::new(
        "platform",
        "Pause, resume, or list a failing gateway platform",
    )
    .gateway_only()
    .with_args("<pause|resume|list> [name]"),
    CommandDef::new("copy", "Copy the last assistant response to clipboard")
        .cli_only()
        .with_args("[number]"),
    CommandDef::new("paste", "Attach clipboard image from your clipboard").cli_only(),
    CommandDef::new("image", "Attach a local image file for your next prompt")
        .cli_only()
        .with_args("<path>"),
    CommandDef::new("update", "Update Operant Agent to the latest version"),
    CommandDef::new("version", "Show Operant Agent version").with_aliases(&["v"]),
    CommandDef::new(
        "debug",
        "Upload debug report (system info + logs) and get shareable links",
    ),
    CommandDef::new("whoami", "Show your slash command access (admin / user)"),
    CommandDef::new("time", "Show the current time"),
    CommandDef::new("session", "Show current session info").with_aliases(&["s"]),
    CommandDef::new("doctor", "Run diagnostics"),
    CommandDef::new("init", "Initialize AGENTS.md for this project"),
    CommandDef::new("login", "Log in to Operant"),
    CommandDef::new("logout", "Log out of Operant"),
    CommandDef::new("refresh", "Clear saved provider auth and model caches"),
    CommandDef::new("providers", "List available AI providers and their status"),
    // ── Exit ─────────────────────────────────────────────────────────────────
    CommandDef::new("exit", "Exit the CLI")
        .cli_only()
        .with_aliases(&["quit", "q"]),
    // ── TUI-only commands (intercepted by the TUI, not in gateway) ────────
    CommandDef::new("rename", "Set a title for the current session")
        .cli_only()
        .with_args("[name]"),
    CommandDef::new("journey", "Browse skills + memories timeline").cli_only(),
    CommandDef::new("setup", "Re-run the setup wizard").cli_only(),
    CommandDef::new("mouse", "Show mouse capture info").cli_only(),
    CommandDef::new("terminal-setup", "Show terminal capability info").cli_only(),
    // ── Agent flows (tier 3) ─────────────────────────────────────────────
    // One-shot agent sessions launched via the /setup suspend-and-run escape
    // hatch; handlers registered in register_agent_flow_handlers (tui_app.rs).
    CommandDef::new(
        "improve",
        "Autonomously improve the repository (TODO.md ledger)",
    )
    .with_aliases(&["cut-improve"]),
    CommandDef::new(
        "judge",
        "Launch a one-shot judge session over the current conversation",
    ),
    CommandDef::new(
        "refactor",
        "Run a safe refactor loop with independent review",
    ),
    CommandDef::new("fix", "Recover when the model cannot continue"),
    CommandDef::new(
        "triage",
        "Triage new GitHub issues and autonomously fix the safe ones",
    ),
];

// ---------------------------------------------------------------------------
// CommandRegistry — runtime dispatch
// ---------------------------------------------------------------------------

/// A registry that maps command names to metadata and handlers.
///
/// The registry is built at startup from the static [`COMMAND_REGISTRY`] slice
/// and can be extended with additional commands from plugins.
///
/// # Example
///
/// ```ignore
/// let mut registry = CommandRegistry::new();
/// registry.register_handler("status", Box::new(StatusHandler::new(config)));
/// let result = registry.execute("status", "").await;
/// match result {
///     CommandResult::Message(text) => println!("{}", text),
///     CommandResult::Error(e) => eprintln!("Error: {}", e),
///     _ => { /* handle other intents */ }
/// }
/// ```
pub struct CommandRegistry {
    /// Maps canonical names and aliases to command definitions.
    defs: HashMap<&'static str, &'static CommandDef>,
    /// Maps canonical command names to their handler implementations.
    handlers: HashMap<String, Box<dyn CommandHandler>>,
}

impl CommandRegistry {
    /// Create a new registry pre-populated with all built-in commands.
    pub fn new() -> Self {
        let defs = build_command_map();
        Self {
            defs,
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for the given canonical command name.
    ///
    /// Returns an error if the name does not match any known command definition.
    pub fn register_handler(&mut self, name: &str, handler: Box<dyn CommandHandler>) -> Result<()> {
        // Validate that this is a known canonical command (not just an alias).
        if !self.defs.contains_key(name) {
            anyhow::bail!(
                "Unknown command '{}'. Cannot register handler without a matching CommandDef.",
                name
            );
        }
        self.handlers.insert(name.to_string(), handler);
        Ok(())
    }

    /// Resolve a command name (or alias) to its canonical name.
    ///
    /// Returns `None` if no command matches.
    pub fn resolve(&self, name: &str) -> Option<&'static str> {
        let trimmed = name.trim().trim_start_matches('/');
        self.defs.get(trimmed).map(|def| def.name)
    }

    /// Execute a slash command by its canonical name.
    ///
    /// Returns a [`CommandResult`] expressing the command's intent as data.
    /// If the command has no registered handler, a fallback `Message` variant
    /// is returned with a helpful description.
    pub async fn execute(&self, name: &str, args: &str) -> CommandResult {
        let canonical = self.resolve(name).unwrap_or(name);
        let ctx = CommandContext { args };

        match self.handlers.get(canonical) {
            Some(handler) => handler.execute(&ctx).await,
            None => {
                if let Some(def) = self.defs.get(canonical) {
                    CommandResult::message(format!(
                        "Command /{} is not yet wired to a handler. Description: {}",
                        def.name, def.description
                    ))
                } else {
                    CommandResult::message(format!("Unknown command: /{}", name))
                }
            }
        }
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Build a canonical-name → CommandDef lookup map that also includes aliases.
pub fn build_command_map() -> HashMap<&'static str, &'static CommandDef> {
    let mut map = HashMap::new();
    for cmd in COMMAND_REGISTRY {
        map.insert(cmd.name, cmd);
        for alias in cmd.aliases {
            map.insert(alias, cmd);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// TUI command data (single source of truth for typeahead + help overlay)
// ---------------------------------------------------------------------------

/// TUI-specific category labels for grouping commands in the help overlay.
/// These are separate from the registry's `category` field because the TUI
/// groups commands by user-facing intent (e.g. "Model & Provider" vs. "Configuration").
pub enum TuiCategory {
    ModelAndProvider,
    ReviewAndHistory,
    Diagnostics,
    Workspace,
    Tools,
    Sessions,
    Preferences,
    Session,
    Context,
    Account,
    Other,
}

impl TuiCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ModelAndProvider => "Model & Provider",
            Self::ReviewAndHistory => "Review & History",
            Self::Diagnostics => "Diagnostics",
            Self::Workspace => "Workspace",
            Self::Tools => "Tools",
            Self::Sessions => "Sessions",
            Self::Preferences => "Preferences",
            Self::Session => "Session",
            Self::Context => "Context",
            Self::Account => "Account",
            Self::Other => "Other",
        }
    }
}

/// Return the TUI category for a given command name.
pub fn tui_category(name: &str) -> &'static str {
    match name {
        "connect" | "model" | "effort" | "provider" => TuiCategory::ModelAndProvider.as_str(),
        "changes" | "diff" | "review" | "rewind" | "export" | "copy" | "history" | "retry"
        | "undo" | "rollback" | "stop" | "refresh" | "rename" | "replay" | "replay-diff" => {
            TuiCategory::ReviewAndHistory.as_str()
        }
        "stats" | "cost" | "context" | "doctor" | "status" | "version" | "time" | "debug"
        | "whoami" | "credits" | "billing" | "insights" | "usage" | "mouse" | "terminal-setup"
        | "heapdump" | "mem" | "pet" => TuiCategory::Diagnostics.as_str(),
        "config" | "theme" | "keybindings" | "hooks" | "mcp" | "import-config" | "output-style"
        | "setup" | "indicator" | "statusbar" | "skin" | "busy" | "verbose" => {
            TuiCategory::Workspace.as_str()
        }
        "agents" | "memory" | "tools" | "skills" | "bundles" | "journey" => {
            TuiCategory::Tools.as_str()
        }
        "search" | "sessions" | "session" => TuiCategory::Sessions.as_str(),
        "plan" | "vim" | "yolo" => TuiCategory::Preferences.as_str(),
        "goal" | "title" | "branch" | "personality" | "reasoning" => TuiCategory::Session.as_str(),
        "compact" | "compress" => TuiCategory::Context.as_str(),
        "login" | "logout" | "update" => TuiCategory::Account.as_str(),
        _ => TuiCategory::Other.as_str(),
    }
}

/// A single entry for TUI typeahead suggestions and help overlay.
#[expect(
    dead_code,
    reason = "description/aliases are table data rendered only by the legacy help path"
)]
#[derive(Clone)]
pub struct TuiSlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub aliases: &'static [&'static str],
}

// ---------------------------------------------------------------------------
// Skill command discovery (cached via LazyLock)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Gateway helpers
// ---------------------------------------------------------------------------

// Set of all command names + aliases recognized by the gateway.
// Includes config-gated commands so the gateway can dispatch them
// (the handler checks the config gate at runtime).

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- CommandDef tests ---------------------------------------------------

    #[test]
    fn test_command_def_with_aliases() {
        let def = CommandDef::new("cmd", "Has aliases").with_aliases(&["c", "command"]);
        assert_eq!(def.aliases, &["c", "command"]);
    }

    #[test]
    fn test_command_def_with_args() {
        let def = CommandDef::new("cmd", "Takes args").with_args("<name>");
        assert_eq!(def.args_hint, "<name>");
    }

    #[test]
    fn test_command_def_cli_only() {
        let def = CommandDef::new("cmd", "CLI only").cli_only();
        assert!(def.cli_only);
        assert!(!def.gateway_only);
    }

    #[test]
    fn test_command_def_gateway_only() {
        let def = CommandDef::new("cmd", "Gateway only").gateway_only();
        assert!(def.gateway_only);
        assert!(!def.cli_only);
    }

    #[test]
    fn test_command_def_with_config_gate() {
        let def = CommandDef::new("cmd", "Gated")
            .cli_only()
            .with_config_gate("display.tool_progress_command");
        assert!(def.cli_only);
        assert_eq!(
            def.gateway_config_gate,
            Some("display.tool_progress_command")
        );
    }

    // -- Static registry tests ----------------------------------------------

    #[test]
    fn test_static_registry_not_empty() {
        assert!(!COMMAND_REGISTRY.is_empty());
    }

    #[test]
    fn test_static_registry_has_help() {
        let has_help = COMMAND_REGISTRY.iter().any(|c| c.name == "help");
        assert!(has_help, "Registry must contain a 'help' command");
    }

    #[test]
    fn test_static_registry_has_exit() {
        let has_exit = COMMAND_REGISTRY.iter().any(|c| c.name == "exit");
        assert!(has_exit, "Registry must contain an 'exit' command");
    }

    #[test]
    fn test_static_registry_count_gte_70() {
        assert!(
            COMMAND_REGISTRY.len() >= 70,
            "Registry should have >= 70 commands, got {}",
            COMMAND_REGISTRY.len()
        );
    }

    // -- build_command_map tests --------------------------------------------

    #[test]
    fn test_build_command_map_includes_canonical() {
        let map = build_command_map();
        assert!(map.contains_key("help"));
        assert!(map.contains_key("exit"));
        assert!(map.contains_key("new"));
    }

    #[test]
    fn test_build_command_map_includes_aliases() {
        let map = build_command_map();
        // "n" is alias for "new"
        assert!(map.contains_key("n"), "Alias 'n' must be in the map");
        assert_eq!(map.get("n").unwrap().name, "new");

        // "q" is alias for "exit"
        assert!(map.contains_key("q"));
        assert_eq!(map.get("q").unwrap().name, "exit");

        // "?" is alias for "help"
        assert!(map.contains_key("?"));
        assert_eq!(map.get("?").unwrap().name, "help");
    }

    #[test]
    fn test_build_command_map_size() {
        let map = build_command_map();
        let unique_canonical: usize = COMMAND_REGISTRY.len();
        assert!(map.len() >= unique_canonical);
        assert!(map.len() <= unique_canonical + 80); // reasonable upper bound
    }

    // -- resolve_command tests ----------------------------------------------

    // -- format_help_text tests ---------------------------------------------

    // -- CommandRegistry tests ----------------------------------------------

    #[test]
    fn test_registry_resolve() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.resolve("help"), Some("help"));
        assert_eq!(registry.resolve("/exit"), Some("exit"));
        assert_eq!(registry.resolve("q"), Some("exit"));
        assert_eq!(registry.resolve("nonexistent"), None);
    }

    #[test]
    fn test_registry_register_handler_rejects_unknown() {
        let mut registry = CommandRegistry::new();
        struct DummyHandler;
        #[async_trait::async_trait]
        impl CommandHandler for DummyHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("ok")
            }
        }

        let result = registry.register_handler("does_not_exist", Box::new(DummyHandler));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown"));
    }

    #[test]
    fn test_registry_register_handler_known() {
        let mut registry = CommandRegistry::new();
        struct DummyHandler;
        #[async_trait::async_trait]
        impl CommandHandler for DummyHandler {
            async fn execute(&self, _ctx: &CommandContext<'_>) -> CommandResult {
                CommandResult::message("ok")
            }
        }

        let result = registry.register_handler("help", Box::new(DummyHandler));
        assert!(result.is_ok());
    }

    // -- Core 10 commands tests ----------------------------------------------

    #[test]
    fn test_core_commands_present() {
        let registry = CommandRegistry::new();
        let core = [
            "help", "status", "new", "stop", "model", "skills", "tools", "memory", "sessions",
            "quit",
        ];
        for name in &core {
            assert!(
                registry.resolve(name).is_some(),
                "Core command /{} must be resolvable",
                name
            );
        }
    }

    #[test]
    fn test_stop_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "stop");
        assert!(has, "Registry must contain a 'stop' command");
    }

    #[test]
    fn test_sessions_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "sessions");
        assert!(has, "Registry must contain a 'sessions' command");
    }

    // -- all_commands tests --------------------------------------------------

    // -- slash_completions tests ---------------------------------------------

    #[test]
    fn test_gateway_config_gate_default_none() {
        let def = CommandDef::new("test", "Test");
        assert!(def.gateway_config_gate.is_none());
    }

    // -- New Python-ported command tests -------------------------------------

    #[test]
    fn test_retry_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "retry");
        assert!(has, "Registry must contain a 'retry' command");
    }

    #[test]
    fn test_undo_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "undo");
        assert!(has, "Registry must contain an 'undo' command");
    }

    #[test]
    fn test_compress_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "compress");
        assert!(has, "Registry must contain a 'compress' command");
    }

    #[test]
    fn test_rollback_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "rollback");
        assert!(has, "Registry must contain a 'rollback' command");
    }

    #[test]
    fn test_personality_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "personality");
        assert!(has, "Registry must contain a 'personality' command");
    }

    #[test]
    fn test_reasoning_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "reasoning");
        assert!(has, "Registry must contain a 'reasoning' command");
    }

    #[test]
    fn test_cron_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "cron");
        assert!(has, "Registry must contain a 'cron' command");
    }

    #[test]
    fn test_browser_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "browser");
        assert!(has, "Registry must contain a 'browser' command");
    }

    #[test]
    fn test_usage_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "usage");
        assert!(has, "Registry must contain a 'usage' command");
    }

    #[test]
    fn test_version_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "version");
        assert!(has, "Registry must contain a 'version' command");
        // Also check alias "v"
        let map = build_command_map();
        assert!(map.contains_key("v"), "Alias 'v' must resolve to 'version'");
        assert_eq!(map.get("v").unwrap().name, "version");
    }

    #[test]
    fn test_debug_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "debug");
        assert!(has, "Registry must contain a 'debug' command");
    }

    #[test]
    fn test_agents_command_exists() {
        let has = COMMAND_REGISTRY.iter().any(|c| c.name == "agents");
        assert!(has, "Registry must contain an 'agents' command");
        let map = build_command_map();
        assert!(
            map.contains_key("tasks"),
            "Alias 'tasks' must resolve to 'agents'"
        );
    }

    #[test]
    fn test_approve_deny_gateway_only() {
        let approve = COMMAND_REGISTRY
            .iter()
            .find(|c| c.name == "approve")
            .unwrap();
        assert!(approve.gateway_only, "/approve must be gateway_only");
        let deny = COMMAND_REGISTRY.iter().find(|c| c.name == "deny").unwrap();
        assert!(deny.gateway_only, "/deny must be gateway_only");
    }

    // (plan 04) test_snapshot_alias_bp and test_blueprint_alias_bp deleted:
    // the snapshot and blueprint commands were dead (never wired in the TUI,
    // never handled by the gateway) and were removed from COMMAND_REGISTRY —
    // alias pins for deleted rows pin the lie.

    #[test]
    fn test_branch_has_no_dead_alias() {
        // 2026-10-14 batch, plan 02: the 'fork' alias was dead in the TUI and
        // only fed the "not yet wired" fallback — removed with the command.
        let map = build_command_map();
        assert!(map.contains_key("branch"));
        assert!(
            !map.contains_key("fork"),
            "dead alias 'fork' must stay gone"
        );
    }

    #[test]
    fn test_statusbar_alias_sb() {
        let map = build_command_map();
        assert!(
            map.contains_key("sb"),
            "Alias 'sb' must resolve to 'statusbar'"
        );
        assert_eq!(map.get("sb").unwrap().name, "statusbar");
    }

    #[test]
    fn test_background_aliases() {
        let map = build_command_map();
        assert!(
            map.contains_key("bg"),
            "Alias 'bg' must resolve to 'background'"
        );
        assert_eq!(map.get("bg").unwrap().name, "background");
        // 2026-10-14 batch, plan 02: 'btw' was a dead alias — removed.
        assert!(!map.contains_key("btw"), "dead alias 'btw' must stay gone");
    }

    #[test]
    fn test_platforms_alias_gateway() {
        let map = build_command_map();
        assert!(
            map.contains_key("gateway"),
            "Alias 'gateway' must resolve to 'platforms'"
        );
        assert_eq!(map.get("gateway").unwrap().name, "platforms");
    }

    #[test]
    fn test_suggestions_alias_suggest() {
        let map = build_command_map();
        assert!(
            map.contains_key("suggest"),
            "Alias 'suggest' must resolve to 'suggestions'"
        );
        assert_eq!(map.get("suggest").unwrap().name, "suggestions");
    }

    // (plan 04) test_blueprint_alias_bp deleted with the blueprint row:
    // dead command, dead alias pin.

    #[test]
    fn test_config_gate_on_verbose() {
        let verbose = COMMAND_REGISTRY
            .iter()
            .find(|c| c.name == "verbose")
            .unwrap();
        assert_eq!(
            verbose.gateway_config_gate,
            Some("display.tool_progress_command")
        );
        assert!(verbose.cli_only);
    }

    #[test]
    fn test_config_gate_on_skills() {
        let skills = COMMAND_REGISTRY
            .iter()
            .find(|c| c.name == "skills")
            .unwrap();
        assert_eq!(skills.gateway_config_gate, Some("skills.write_approval"));
    }

    // --- Display variant tests (per reviewer suggestion) ---

    #[test]
    fn test_display_shell_command() {
        let r = CommandResult::ShellCommand(vec!["ls".to_string(), "-la".to_string()]);
        assert_eq!(format!("{}", r), "ShellCommand([\"ls\", \"-la\"])");
    }

    #[test]
    fn test_display_shell_command_single() {
        let r = CommandResult::ShellCommand(vec!["echo hello".to_string()]);
        assert_eq!(format!("{}", r), "ShellCommand([\"echo hello\"])");
    }

    #[test]
    fn test_display_shell_command_empty() {
        let r = CommandResult::ShellCommand(vec![]);
        assert_eq!(format!("{}", r), "ShellCommand([])");
    }
    #[test]
    fn test_resolve_canonical() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.resolve("help"), Some("help"));
        assert_eq!(registry.resolve("exit"), Some("exit"));
    }

    #[test]
    fn test_resolve_with_slash() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.resolve("/help"), Some("help"));
        assert_eq!(registry.resolve("/exit"), Some("exit"));
    }

    #[test]
    fn test_resolve_alias() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.resolve("q"), Some("exit"));
        assert_eq!(registry.resolve("n"), Some("new"));
        assert_eq!(registry.resolve("bg"), Some("background"));
    }

    #[test]
    fn test_resolve_unknown() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.resolve("nonexistent"), None);
        assert_eq!(registry.resolve(""), None);
    }

    #[test]
    fn test_resolve_with_leading_whitespace() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.resolve("  help"), Some("help"));
    }

    struct EchoHandler;

    #[async_trait::async_trait]
    impl CommandHandler for EchoHandler {
        async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
            CommandResult::message(format!("echo: {}", ctx.args))
        }
    }

    #[tokio::test]
    async fn test_registry_execute_registered_handler() {
        let mut registry = CommandRegistry::new();
        registry
            .register_handler("help", Box::new(EchoHandler))
            .unwrap();

        let CommandResult::Message(text) = registry.execute("help", "hello").await else {
            panic!("expected Message");
        };
        assert_eq!(text, "echo: hello");
    }

    #[tokio::test]
    async fn test_registry_execute_resolves_alias() {
        let mut registry = CommandRegistry::new();
        registry
            .register_handler("exit", Box::new(EchoHandler))
            .unwrap();

        // Execute via alias "q"
        let CommandResult::Message(text) = registry.execute("q", "bye").await else {
            panic!("expected Message");
        };
        assert_eq!(text, "echo: bye");
    }

    #[tokio::test]
    async fn test_registry_execute_unknown() {
        let registry = CommandRegistry::new();
        let CommandResult::Message(text) = registry.execute("nonexistent", "").await else {
            panic!("expected Message");
        };
        assert!(text.contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_registry_execute_unhandled_returns_fallback() {
        let registry = CommandRegistry::new();
        let CommandResult::Message(text) = registry.execute("help", "").await else {
            panic!("expected Message");
        };
        assert!(text.contains("not yet wired"));
    }
}
