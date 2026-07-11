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
// CommandCategory
// ---------------------------------------------------------------------------

/// Category for grouping slash commands in help output.
///
/// Mirrors Python's `CommandDef.category` field with typed variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    /// Session management (new, stop, history, etc.)
    Session,
    /// Configuration (model, provider, config, etc.)
    Configuration,
    /// Tools and skills management
    ToolsAndSkills,
    /// Informational commands (help, status, memory, etc.)
    Info,
    /// Exit commands
    Exit,
}

impl CommandCategory {
    /// Return the display string matching the Python convention.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Configuration => "Configuration",
            Self::ToolsAndSkills => "Tools & Skills",
            Self::Info => "Info",
            Self::Exit => "Exit",
        }
    }
}

impl fmt::Display for CommandCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for CommandCategory {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        match s {
            "Session" => Ok(Self::Session),
            "Configuration" => Ok(Self::Configuration),
            "Tools & Skills" | "ToolsAndSkills" => Ok(Self::ToolsAndSkills),
            "Info" => Ok(Self::Info),
            "Exit" => Ok(Self::Exit),
            _ => anyhow::bail!("Unknown command category: {}", s),
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Context provided to a command handler on execution.
#[derive(Debug, Default)]
pub struct CommandContext<'a> {
    /// The raw argument string following the command name.
    pub args: &'a str,
}

/// Result type for slash command execution.
pub type CommandResult = Result<String>;

/// A slash command handler that can be registered in the [`CommandRegistry`].
///
/// Implementations should be stateless or use interior mutability, since the
/// same handler may be called multiple times.
#[async_trait::async_trait]
pub trait CommandHandler: Send + Sync {
    /// Execute this command with the given context.
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
    /// Category for grouping in help output (e.g. "Session", "Info").
    pub category: &'static str,
    /// Alternative names that resolve to this command.
    pub aliases: &'static [&'static str],
    /// Argument hint shown in help (e.g. "<prompt>", "[key] [value]").
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
    pub const fn new(
        name: &'static str,
        description: &'static str,
        category: &'static str,
    ) -> Self {
        Self {
            name,
            description,
            category,
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
    CommandDef::new(
        "start",
        "Acknowledge platform start pings without a reply",
        "Session",
    )
    .gateway_only(),
    CommandDef::new(
        "new",
        "Start a new session (fresh session ID + history)",
        "Session",
    )
    .with_args("[name]")
    .with_aliases(&["n", "clear"]),
    CommandDef::new(
        "topic",
        "Enable or inspect Telegram DM topic sessions",
        "Session",
    )
    .gateway_only()
    .with_args("[off|help|session-id]"),
    CommandDef::new(
        "redraw",
        "Force a full UI repaint (recovers from terminal drift)",
        "Session",
    )
    .cli_only(),
    CommandDef::new("history", "Show conversation history", "Session")
        .cli_only()
        .with_aliases(&["h"]),
    CommandDef::new("save", "Save the current conversation", "Session")
        .cli_only()
        .with_aliases(&["export"]),
    CommandDef::new(
        "retry",
        "Retry the last message (resend to agent)",
        "Session",
    ),
    CommandDef::new(
        "undo",
        "Back up N user turns and re-prompt (default 1)",
        "Session",
    )
    .with_args("[N]"),
    CommandDef::new("title", "Set a title for the current session", "Session").with_args("[name]"),
    CommandDef::new(
        "handoff",
        "Hand off this session to a messaging platform (Telegram, Discord, etc.)",
        "Session",
    )
    .with_args("<platform>")
    .cli_only(),
    CommandDef::new(
        "branch",
        "Branch the current session (explore a different path)",
        "Session",
    )
    .with_aliases(&["fork"])
    .with_args("[name]"),
    CommandDef::new(
        "compress",
        "Compress conversation context (add 'here [N]' to keep recent N turns)",
        "Session",
    )
    .with_args("[here [N] | focus topic]"),
    CommandDef::new(
        "rollback",
        "List or restore filesystem checkpoints",
        "Session",
    )
    .with_args("[number]"),
    CommandDef::new(
        "snapshot",
        "Create or restore state snapshots of Operant config/state",
        "Session",
    )
    .cli_only()
    .with_aliases(&["snap"])
    .with_args("[create|restore <id>|prune]"),
    CommandDef::new("stop", "Kill all running background processes", "Session"),
    CommandDef::new("approve", "Approve a pending dangerous command", "Session")
        .gateway_only()
        .with_args("[session|always]"),
    CommandDef::new("deny", "Deny a pending dangerous command", "Session").gateway_only(),
    CommandDef::new("background", "Run a prompt in the background", "Session")
        .with_aliases(&["bg", "btw"])
        .with_args("<prompt>"),
    CommandDef::new("agents", "Show active agents and running tasks", "Session")
        .with_aliases(&["tasks"]),
    CommandDef::new(
        "queue",
        "Queue a prompt for the next turn (doesn't interrupt)",
        "Session",
    )
    .with_aliases(&["q"])
    .with_args("<prompt>"),
    CommandDef::new(
        "steer",
        "Inject a message after the next tool call without interrupting",
        "Session",
    )
    .with_args("<prompt>"),
    CommandDef::new(
        "goal",
        "Set a standing goal Operant works on across turns until achieved",
        "Session",
    )
    .with_args("[text | pause | resume | clear | status]"),
    CommandDef::new(
        "subgoal",
        "Add or manage extra criteria on the active goal",
        "Session",
    )
    .with_args("[text | remove N | clear]"),
    CommandDef::new(
        "status",
        "Show session, model, token, and context info",
        "Session",
    ),
    CommandDef::new("resume", "Resume a previously-named session", "Session").with_args("[name]"),
    CommandDef::new("sethome", "Set this chat as the home channel", "Session")
        .gateway_only()
        .with_aliases(&["set-home"]),
    CommandDef::new("sessions", "Browse and resume previous sessions", "Session"),
    // ── Configuration ───────────────────────────────────────────────────────
    CommandDef::new("model", "Switch model for this session", "Configuration")
        .with_args("[model] [--provider name] [--global] [--refresh]"),
    CommandDef::new("provider", "Switch LLM provider", "Configuration").with_args("<name>"),
    CommandDef::new("config", "Show current configuration", "Configuration")
        .cli_only()
        .with_args("[key] [value]"),
    CommandDef::new("env", "View or set environment variables", "Configuration")
        .cli_only()
        .with_args("[key] [value]"),
    CommandDef::new(
        "codex-runtime",
        "Toggle codex app-server runtime for OpenAI/Codex models",
        "Configuration",
    )
    .with_aliases(&["codex_runtime"])
    .with_args("[auto|codex_app_server]"),
    CommandDef::new(
        "profile",
        "Show active profile name and home directory",
        "Info",
    ),
    CommandDef::new(
        "personality",
        "Set a predefined personality",
        "Configuration",
    )
    .with_args("[name]"),
    CommandDef::new(
        "statusbar",
        "Toggle the context/model status bar",
        "Configuration",
    )
    .cli_only()
    .with_aliases(&["sb"]),
    CommandDef::new(
        "verbose",
        "Cycle tool progress display: off -> new -> all -> verbose",
        "Configuration",
    )
    .cli_only()
    .with_config_gate("display.tool_progress_command"),
    CommandDef::new(
        "footer",
        "Toggle gateway runtime-metadata footer on final replies",
        "Configuration",
    )
    .with_args("[on|off|status]"),
    CommandDef::new(
        "yolo",
        "Toggle YOLO mode (skip all dangerous command approvals)",
        "Configuration",
    ),
    CommandDef::new(
        "reasoning",
        "Manage reasoning effort and display",
        "Configuration",
    )
    .with_args("[level|show|hide]"),
    CommandDef::new(
        "fast",
        "Toggle fast mode — OpenAI Priority Processing / Anthropic Fast Mode",
        "Configuration",
    )
    .with_args("[normal|fast|status]"),
    CommandDef::new(
        "skin",
        "Show or change the display skin/theme",
        "Configuration",
    )
    .cli_only()
    .with_args("[name]"),
    CommandDef::new(
        "indicator",
        "Pick the TUI busy-indicator style",
        "Configuration",
    )
    .cli_only()
    .with_args("[kaomoji|emoji|unicode|ascii]"),
    CommandDef::new("voice", "Toggle voice mode", "Configuration").with_args("[on|off|tts|status]"),
    CommandDef::new(
        "busy",
        "Control what Enter does while Operant is working",
        "Configuration",
    )
    .cli_only()
    .with_args("[queue|steer|interrupt|status]"),
    // ── Tools & Skills ──────────────────────────────────────────────────────
    CommandDef::new(
        "tools",
        "Manage tools: /tools [list|disable|enable] [name...]",
        "Tools & Skills",
    )
    .cli_only()
    .with_args("[list|disable|enable] [name...]"),
    CommandDef::new("toolsets", "List available toolsets", "Tools & Skills").cli_only(),
    CommandDef::new(
        "skills",
        "Search, install, inspect, or manage skills",
        "Tools & Skills",
    )
    .cli_only()
    .with_aliases(&["skill"])
    .with_config_gate("skills.write_approval"),
    CommandDef::new(
        "memory",
        "Review pending memory writes / toggle the approval gate",
        "Tools & Skills",
    )
    .with_aliases(&["mem"])
    .with_args("[pending|approve|reject|approval] [id|on|off]"),
    CommandDef::new(
        "bundles",
        "List skill bundles (aliases /<name> for multiple skills)",
        "Tools & Skills",
    ),
    CommandDef::new("cron", "Manage scheduled tasks", "Tools & Skills")
        .cli_only()
        .with_args("[subcommand]"),
    CommandDef::new(
        "suggestions",
        "Review suggested automations (accept/dismiss)",
        "Tools & Skills",
    )
    .with_aliases(&["suggest"])
    .with_args("[accept|dismiss N | catalog]"),
    CommandDef::new(
        "blueprint",
        "Set up an automation from a blueprint template",
        "Tools & Skills",
    )
    .with_aliases(&["bp"])
    .with_args("[name] [slot=value ...]"),
    CommandDef::new(
        "curator",
        "Background skill maintenance (status, run, pin, archive, list-archived)",
        "Tools & Skills",
    )
    .with_args("[subcommand]"),
    CommandDef::new(
        "kanban",
        "Multi-profile collaboration board (tasks, links, comments)",
        "Tools & Skills",
    )
    .with_aliases(&["k"])
    .with_args("[subcommand]"),
    CommandDef::new(
        "reload",
        "Reload .env variables into the running session",
        "Tools & Skills",
    )
    .cli_only(),
    CommandDef::new(
        "reload-mcp",
        "Reload MCP servers from config",
        "Tools & Skills",
    )
    .with_aliases(&["reload_mcp"]),
    CommandDef::new(
        "reload-skills",
        "Re-scan ~/.operant/skills/ for newly installed or removed skills",
        "Tools & Skills",
    )
    .with_aliases(&["reload_skills"]),
    CommandDef::new(
        "browser",
        "Connect browser tools to your live Chromium-family browser via CDP",
        "Tools & Skills",
    )
    .cli_only()
    .with_args("[connect|disconnect|status]"),
    CommandDef::new(
        "plugins",
        "List installed plugins and their status",
        "Tools & Skills",
    )
    .cli_only(),
    // ── Info ─────────────────────────────────────────────────────────────────
    CommandDef::new(
        "commands",
        "Browse all commands and skills (paginated)",
        "Info",
    )
    .gateway_only()
    .with_args("[page]"),
    CommandDef::new("help", "Show available commands", "Info").with_aliases(&["?", "h"]),
    CommandDef::new(
        "restart",
        "Gracefully restart the gateway after draining active runs",
        "Session",
    )
    .gateway_only(),
    CommandDef::new(
        "usage",
        "Show token usage and rate limits for the current session",
        "Info",
    ),
    CommandDef::new("credits", "Show Nous credit balance and top up", "Info"),
    CommandDef::new(
        "billing",
        "Manage Nous terminal billing — buy credits, auto-reload, limits",
        "Info",
    ),
    CommandDef::new("insights", "Show usage insights and analytics", "Info").with_args("[days]"),
    CommandDef::new(
        "platforms",
        "Show gateway/messaging platform status",
        "Info",
    )
    .cli_only()
    .with_aliases(&["gateway"]),
    CommandDef::new(
        "platform",
        "Pause, resume, or list a failing gateway platform",
        "Info",
    )
    .gateway_only()
    .with_args("<pause|resume|list> [name]"),
    CommandDef::new(
        "copy",
        "Copy the last assistant response to clipboard",
        "Info",
    )
    .cli_only()
    .with_args("[number]"),
    CommandDef::new(
        "paste",
        "Attach clipboard image from your clipboard",
        "Info",
    )
    .cli_only(),
    CommandDef::new(
        "image",
        "Attach a local image file for your next prompt",
        "Info",
    )
    .cli_only()
    .with_args("<path>"),
    CommandDef::new(
        "update",
        "Update Operant Agent to the latest version",
        "Info",
    ),
    CommandDef::new("version", "Show Operant Agent version", "Info").with_aliases(&["v"]),
    CommandDef::new(
        "debug",
        "Upload debug report (system info + logs) and get shareable links",
        "Info",
    ),
    CommandDef::new(
        "whoami",
        "Show your slash command access (admin / user)",
        "Info",
    ),
    CommandDef::new(
        "gquota",
        "Show Google Gemini Code Assist quota usage",
        "Info",
    )
    .cli_only(),
    CommandDef::new("time", "Show the current time", "Info"),
    CommandDef::new("session", "Show current session info", "Info").with_aliases(&["s"]),
    CommandDef::new("doctor", "Run diagnostics", "Info"),
    CommandDef::new("init", "Initialize AGENTS.md for this project", "Session"),
    CommandDef::new("login", "Log in to Operant", "Session"),
    CommandDef::new("logout", "Log out of Operant", "Session"),
    CommandDef::new(
        "refresh",
        "Clear saved provider auth and model caches",
        "Session",
    ),
    CommandDef::new(
        "providers",
        "List available AI providers and their status",
        "Info",
    ),
    // ── Exit ─────────────────────────────────────────────────────────────────
    CommandDef::new("exit", "Exit the CLI", "Exit")
        .cli_only()
        .with_aliases(&["quit", "q"]),
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
/// let result = registry.execute("status", "").await?;
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
    /// Returns the command's output as a string. If the command has no registered
    /// handler, a fallback message is returned instead.
    pub async fn execute(&self, name: &str, args: &str) -> CommandResult {
        let canonical = self.resolve(name).unwrap_or(name);
        let ctx = CommandContext { args };

        match self.handlers.get(canonical) {
            Some(handler) => handler.execute(&ctx).await,
            None => {
                // Look up metadata for a helpful message
                if let Some(def) = self.defs.get(canonical) {
                    Ok(format!(
                        "Command /{} is not yet wired to a handler. Description: {}",
                        def.name, def.description
                    ))
                } else {
                    Ok(format!("Unknown command: /{}", name))
                }
            }
        }
    }

    /// Return the number of registered commands.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Return true if no commands are registered.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Iterate over all command definitions.
    pub fn iter_defs(&self) -> impl Iterator<Item = &'static CommandDef> {
        // Use a set to deduplicate (each canonical name appears once)
        let mut seen = std::collections::HashSet::new();
        let mut defs = Vec::new();
        for (name, def) in &self.defs {
            if seen.insert(name) {
                defs.push(*def);
            }
        }
        defs.into_iter()
    }

    /// Collect commands grouped by category, in display order.
    pub fn commands_by_category(&self) -> Vec<(&'static str, Vec<&'static CommandDef>)> {
        let mut categories: Vec<(&str, Vec<&CommandDef>)> = Vec::new();
        let mut seen_canonical = std::collections::HashSet::new();

        for cmd in COMMAND_REGISTRY.iter() {
            if self.defs.contains_key(cmd.name) && seen_canonical.insert(cmd.name) {
                let cat = cmd.category;
                match categories.iter_mut().find(|(c, _)| *c == cat) {
                    Some((_, list)) => list.push(cmd),
                    None => categories.push((cat, vec![cmd])),
                }
            }
        }
        categories
    }

    /// Format help text for all registered commands, organized by category.
    pub fn format_help(&self) -> String {
        let mut output = String::from("Available commands:\n\n");

        for (category, commands) in self.commands_by_category() {
            output.push_str(&format!("  {}:\n", category));
            for cmd in &commands {
                let aliases = if cmd.aliases.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", cmd.aliases.join(", "))
                };
                let args = if cmd.args_hint.is_empty() {
                    String::new()
                } else {
                    format!(" {}", cmd.args_hint)
                };
                output.push_str(&format!(
                    "    /{:<12}{:<20}  {}\n",
                    format!("{}{}", cmd.name, args),
                    aliases,
                    cmd.description
                ));
            }
            output.push('\n');
        }

        output
    }

    pub fn all_commands(&self) -> Vec<&'static CommandDef> {
        let mut seen = std::collections::HashSet::new();
        let mut cmds = Vec::new();
        for cmd in COMMAND_REGISTRY {
            if seen.insert(cmd.name) {
                cmds.push(cmd);
            }
        }
        cmds
    }

    pub fn slash_completions(&self, prefix: &str) -> Vec<String> {
        let trimmed = prefix.trim_start_matches('/');
        COMMAND_REGISTRY
            .iter()
            .filter(|cmd| {
                cmd.name.starts_with(trimmed) || cmd.aliases.iter().any(|a| a.starts_with(trimmed))
            })
            .flat_map(|cmd| {
                let mut names = vec![format!("/{}", cmd.name)];
                for alias in cmd.aliases {
                    names.push(format!("/{}", alias));
                }
                names
            })
            .collect()
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

/// Resolve a user-provided command name (with or without leading `/`) to its
/// canonical name. Returns `None` if no match.
pub fn resolve_command(input: &str) -> Option<&'static str> {
    let trimmed = input.trim().trim_start_matches('/');
    let map = build_command_map();
    map.get(trimmed).map(|cmd| cmd.name)
}

/// Format full help text for all built-in commands.
pub fn format_help_text() -> String {
    let mut output = String::from("Available commands:\n\n");
    let categories = commands_by_category();

    for (category, commands) in &categories {
        output.push_str(&format!("  {}:\n", category));
        for cmd in commands {
            let aliases = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!(" ({})", cmd.aliases.join(", "))
            };
            let args = if cmd.args_hint.is_empty() {
                String::new()
            } else {
                format!(" {}", cmd.args_hint)
            };
            output.push_str(&format!(
                "    /{:<12}{:<20}  {}\n",
                format!("{}{}", cmd.name, args),
                aliases,
                cmd.description
            ));
        }
        output.push('\n');
    }

    output
}

/// Return commands grouped by category in display order.
///
/// Dynamically builds category boundaries from the registry instead of
/// relying on hardcoded index ranges.
pub fn commands_by_category() -> Vec<(&'static str, Vec<&'static CommandDef>)> {
    let mut categories: Vec<(&str, Vec<&CommandDef>)> = Vec::new();
    let mut seen_canonical = std::collections::HashSet::new();

    for cmd in COMMAND_REGISTRY {
        if seen_canonical.insert(cmd.name) {
            let cat = cmd.category;
            match categories.iter_mut().find(|(c, _)| *c == cat) {
                Some((_, list)) => list.push(cmd),
                None => categories.push((cat, vec![cmd])),
            }
        }
    }
    categories
}

// ---------------------------------------------------------------------------
// Gateway helpers
// ---------------------------------------------------------------------------

/// Set of all command names + aliases recognized by the gateway.
/// Includes config-gated commands so the gateway can dispatch them
/// (the handler checks the config gate at runtime).

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- CommandDef tests ---------------------------------------------------

    #[test]
    fn test_command_def_new() {
        let def = CommandDef::new("test", "A test command", "Testing");
        assert_eq!(def.name, "test");
        assert_eq!(def.description, "A test command");
        assert_eq!(def.category, "Testing");
        assert!(def.aliases.is_empty());
        assert!(def.args_hint.is_empty());
        assert!(!def.cli_only);
        assert!(!def.gateway_only);
    }

    #[test]
    fn test_command_def_with_aliases() {
        let def = CommandDef::new("cmd", "Has aliases", "Test").with_aliases(&["c", "command"]);
        assert_eq!(def.aliases, &["c", "command"]);
    }

    #[test]
    fn test_command_def_with_args() {
        let def = CommandDef::new("cmd", "Takes args", "Test").with_args("<name>");
        assert_eq!(def.args_hint, "<name>");
    }

    #[test]
    fn test_command_def_cli_only() {
        let def = CommandDef::new("cmd", "CLI only", "Test").cli_only();
        assert!(def.cli_only);
        assert!(!def.gateway_only);
    }

    #[test]
    fn test_command_def_gateway_only() {
        let def = CommandDef::new("cmd", "Gateway only", "Test").gateway_only();
        assert!(def.gateway_only);
        assert!(!def.cli_only);
    }

    #[test]
    fn test_command_def_with_config_gate() {
        let def = CommandDef::new("cmd", "Gated", "Test")
            .cli_only()
            .with_config_gate("display.tool_progress_command");
        assert!(def.cli_only);
        assert_eq!(
            def.gateway_config_gate,
            Some("display.tool_progress_command")
        );
    }

    #[test]
    fn test_command_category_display() {
        assert_eq!(CommandCategory::Session.as_str(), "Session");
        assert_eq!(CommandCategory::Configuration.as_str(), "Configuration");
        assert_eq!(CommandCategory::ToolsAndSkills.as_str(), "Tools & Skills");
        assert_eq!(CommandCategory::Info.as_str(), "Info");
        assert_eq!(CommandCategory::Exit.as_str(), "Exit");
    }

    #[test]
    fn test_command_category_try_from_str() {
        assert_eq!(
            CommandCategory::try_from("Session").unwrap(),
            CommandCategory::Session
        );
        assert_eq!(
            CommandCategory::try_from("Tools & Skills").unwrap(),
            CommandCategory::ToolsAndSkills
        );
        assert_eq!(
            CommandCategory::try_from("ToolsAndSkills").unwrap(),
            CommandCategory::ToolsAndSkills
        );
        assert!(CommandCategory::try_from("Unknown").is_err());
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

    #[test]
    fn test_all_categories_have_commands() {
        let cats = commands_by_category();
        assert!(!cats.is_empty(), "Must have at least one category");
        for (category, defs) in &cats {
            assert!(
                !defs.is_empty(),
                "Category '{}' must have at least one command",
                category
            );
        }
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

    #[test]
    fn test_resolve_canonical() {
        assert_eq!(resolve_command("help"), Some("help"));
        assert_eq!(resolve_command("exit"), Some("exit"));
    }

    #[test]
    fn test_resolve_with_slash() {
        assert_eq!(resolve_command("/help"), Some("help"));
        assert_eq!(resolve_command("/exit"), Some("exit"));
    }

    #[test]
    fn test_resolve_alias() {
        assert_eq!(resolve_command("q"), Some("exit"));
        assert_eq!(resolve_command("n"), Some("new"));
        assert_eq!(resolve_command("bg"), Some("background"));
    }

    #[test]
    fn test_resolve_unknown() {
        assert_eq!(resolve_command("nonexistent"), None);
        assert_eq!(resolve_command(""), None);
    }

    #[test]
    fn test_resolve_with_leading_whitespace() {
        assert_eq!(resolve_command("  help"), Some("help"));
    }

    // -- format_help_text tests ---------------------------------------------

    #[test]
    fn test_format_help_text_includes_categories() {
        let help = format_help_text();
        assert!(help.contains("Session"));
        assert!(help.contains("Configuration"));
        assert!(help.contains("Tools & Skills"));
        assert!(help.contains("Info"));
        assert!(help.contains("Exit"));
    }

    #[test]
    fn test_format_help_text_includes_commands() {
        let help = format_help_text();
        assert!(help.contains("/help"));
        assert!(help.contains("/exit"));
        assert!(help.contains("/new"));
    }

    #[test]
    fn test_format_help_text_includes_descriptions() {
        let help = format_help_text();
        assert!(help.contains("Show available commands"));
        assert!(help.contains("Start a new session"));
    }

    #[test]
    fn test_format_help_text_contains_aliases() {
        let help = format_help_text();
        assert!(help.contains("n") || help.contains("clear"));
        assert!(help.contains("q") || help.contains("quit"));
    }

    // -- CommandRegistry tests ----------------------------------------------

    #[test]
    fn test_registry_new_is_not_empty() {
        let registry = CommandRegistry::new();
        assert!(!registry.is_empty());
    }

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
                Ok("ok".to_string())
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
                Ok("ok".to_string())
            }
        }

        let result = registry.register_handler("help", Box::new(DummyHandler));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_registry_execute_unhandled_returns_fallback() {
        let registry = CommandRegistry::new();
        let result = registry.execute("help", "").await.unwrap();
        assert!(result.contains("not yet wired"));
    }

    #[tokio::test]
    async fn test_registry_execute_unknown() {
        let registry = CommandRegistry::new();
        let result = registry.execute("nonexistent", "").await.unwrap();
        assert!(result.contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_registry_execute_registered_handler() {
        let mut registry = CommandRegistry::new();

        struct EchoHandler;
        #[async_trait::async_trait]
        impl CommandHandler for EchoHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                Ok(format!("echo: {}", ctx.args))
            }
        }

        registry
            .register_handler("help", Box::new(EchoHandler))
            .unwrap();

        let result = registry.execute("help", "hello").await.unwrap();
        assert_eq!(result, "echo: hello");
    }

    #[tokio::test]
    async fn test_registry_execute_resolves_alias() {
        let mut registry = CommandRegistry::new();

        struct EchoHandler;
        #[async_trait::async_trait]
        impl CommandHandler for EchoHandler {
            async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
                Ok(format!("echo: {}", ctx.args))
            }
        }

        registry
            .register_handler("exit", Box::new(EchoHandler))
            .unwrap();

        // Execute via alias "q"
        let result = registry.execute("q", "bye").await.unwrap();
        assert_eq!(result, "echo: bye");
    }

    #[test]
    fn test_registry_format_help() {
        let registry = CommandRegistry::new();
        let help = registry.format_help();
        assert!(help.contains("Session"));
        assert!(help.contains("Configuration"));
        assert!(help.contains("Tools & Skills"));
        assert!(help.contains("Info"));
        assert!(help.contains("Exit"));
        assert!(help.contains("/help"));
        assert!(help.contains("/exit"));
    }

    #[test]
    fn test_commands_by_category_order() {
        let cats = commands_by_category();
        assert_eq!(cats[0].0, "Session");
        assert_eq!(cats[1].0, "Configuration");
        // Info appears before Tools & Skills in COMMAND_REGISTRY (profile cmd)
        assert_eq!(cats[2].0, "Info");
        assert_eq!(cats[3].0, "Tools & Skills");
        assert_eq!(cats[4].0, "Exit");
    }

    #[test]
    fn test_registry_commands_by_category() {
        let registry = CommandRegistry::new();
        let cats = registry.commands_by_category();
        assert!(!cats.is_empty());
        // All expected categories present
        let cat_names: Vec<&str> = cats.iter().map(|(c, _)| *c).collect();
        assert!(cat_names.contains(&"Session"));
        assert!(cat_names.contains(&"Info"));
        assert!(cat_names.contains(&"Exit"));
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

    #[test]
    fn test_all_commands_returns_all_canonicals() {
        let registry = CommandRegistry::new();
        let all = registry.all_commands();
        assert_eq!(all.len(), COMMAND_REGISTRY.len());
        let names: Vec<&str> = all.iter().map(|c| c.name).collect();
        assert!(names.contains(&"help"));
        assert!(names.contains(&"stop"));
        assert!(names.contains(&"sessions"));
    }

    // -- slash_completions tests ---------------------------------------------

    #[test]
    fn test_slash_completions_empty_prefix() {
        let registry = CommandRegistry::new();
        let completions = registry.slash_completions("/");
        assert!(!completions.is_empty());
        assert!(completions.contains(&"/help".to_string()));
        assert!(completions.contains(&"/exit".to_string()));
    }

    #[test]
    fn test_slash_completions_partial_match() {
        let registry = CommandRegistry::new();
        let completions = registry.slash_completions("/he");
        assert!(completions.contains(&"/help".to_string()));
        assert!(!completions.contains(&"/exit".to_string()));
    }

    #[test]
    fn test_slash_completions_alias_match() {
        let registry = CommandRegistry::new();
        let completions = registry.slash_completions("/q");
        assert!(completions.contains(&"/quit".to_string()));
        assert!(completions.contains(&"/q".to_string()));
    }

    #[test]
    fn test_slash_completions_no_match() {
        let registry = CommandRegistry::new();
        let completions = registry.slash_completions("/zzz");
        assert!(completions.is_empty());
    }

    #[test]
    fn test_gateway_config_gate_default_none() {
        let def = CommandDef::new("test", "Test", "Info");
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

    #[test]
    fn test_snapshot_alias_snap() {
        let map = build_command_map();
        assert!(
            map.contains_key("snap"),
            "Alias 'snap' must resolve to 'snapshot'"
        );
        assert_eq!(map.get("snap").unwrap().name, "snapshot");
    }

    #[test]
    fn test_branch_alias_fork() {
        let map = build_command_map();
        assert!(
            map.contains_key("fork"),
            "Alias 'fork' must resolve to 'branch'"
        );
        assert_eq!(map.get("fork").unwrap().name, "branch");
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
        assert!(
            map.contains_key("btw"),
            "Alias 'btw' must resolve to 'background'"
        );
        assert_eq!(map.get("btw").unwrap().name, "background");
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

    #[test]
    fn test_blueprint_alias_bp() {
        let map = build_command_map();
        assert!(
            map.contains_key("bp"),
            "Alias 'bp' must resolve to 'blueprint'"
        );
        assert_eq!(map.get("bp").unwrap().name, "blueprint");
    }

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
}
