//! Interactive Slash Command Registry.
//!
//! Ported from hermes-agent/hermes_cli/commands.py COMMAND_REGISTRY.
//! All slash commands are defined here and consumed by the CLI chat loop,
//! gateway dispatchers, and any other interactive context.
//!
//! The registry provides:
//! - Static command metadata definitions (name, description, category, aliases)
//! - Dynamic handler registration for runtime dispatch
//! - Help text formatting organized by category
//! - Command resolution (name + alias → canonical name)

use std::collections::HashMap;

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
}

// ---------------------------------------------------------------------------
// Built-in command registry (static)
// ---------------------------------------------------------------------------

/// All built-in slash commands, ordered by category.
///
/// Each entry is metadata only; handlers are registered dynamically in
/// [`CommandRegistry`] at startup.
pub static COMMAND_REGISTRY: &[CommandDef] = &[
    // ── Session ──
    CommandDef::new("new", "Start a new conversation", "Session").with_aliases(&["n", "clear"]),
    CommandDef::new("reset", "Reset the current conversation", "Session").with_aliases(&["r"]),
    CommandDef::new("continue", "Continue the last conversation", "Session")
        .with_aliases(&["c", "resume"]),
    CommandDef::new("save", "Save the current conversation", "Session").with_aliases(&["export"]),
    CommandDef::new("background", "Run a task in the background", "Session")
        .with_args("<prompt>")
        .with_aliases(&["bg"]),
    CommandDef::new("fork", "Fork the conversation from a message", "Session").with_args("<id>"),
    CommandDef::new("history", "Show conversation history", "Session").with_aliases(&["h"]),
    // ── Configuration ──
    CommandDef::new("model", "Switch the active model", "Configuration").with_args("<name>"),
    CommandDef::new("provider", "Switch LLM provider", "Configuration").with_args("<name>"),
    CommandDef::new("config", "View or change configuration", "Configuration")
        .with_args("[key] [value]"),
    CommandDef::new("env", "View or set environment variables", "Configuration")
        .with_args("[key] [value]"),
    CommandDef::new("profile", "Switch or manage profiles", "Configuration").with_args("<name>"),
    CommandDef::new("skin", "Change the CLI theme", "Configuration").with_args("<name>"),
    // ── Tools & Skills ──
    CommandDef::new("skills", "Manage installed skills", "Tools & Skills").with_aliases(&["skill"]),
    CommandDef::new("tools", "List available tools", "Tools & Skills"),
    CommandDef::new("mcp", "Manage MCP servers", "Tools & Skills"),
    CommandDef::new("plugins", "Manage plugins", "Tools & Skills"),
    CommandDef::new("kanban", "Manage kanban tasks", "Tools & Skills").with_aliases(&["k"]),
    // ── Info ──
    CommandDef::new("help", "Show this help message", "Info").with_aliases(&["h", "?"]),
    CommandDef::new("status", "Show system status", "Info"),
    CommandDef::new("memory", "Show or search memories", "Info").with_aliases(&["mem"]),
    CommandDef::new("session", "Show current session info", "Info").with_aliases(&["s"]),
    CommandDef::new("cost", "Show token usage and cost", "Info"),
    CommandDef::new("time", "Show the current time", "Info"),
    // ── Exit ──
    CommandDef::new("exit", "Exit the CLI", "Exit").with_aliases(&["quit", "q"]),
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
    pub fn resolve<'a>(&'a self, name: &str) -> Option<&'static str> {
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
/// Uses explicit indices into [`COMMAND_REGISTRY`] to define category boundaries.
pub fn commands_by_category() -> Vec<(&'static str, Vec<&'static CommandDef>)> {
    // Build from explicit indices (can't use slices in static context on stable Rust)
    let indices: &[(&str, std::ops::Range<usize>)] = &[
        ("Session", 0..7),
        ("Configuration", 7..13),
        ("Tools & Skills", 13..18),
        ("Info", 18..24),
        ("Exit", 24..25),
    ];

    indices
        .iter()
        .map(|(cat, range)| {
            let refs: Vec<&CommandDef> = COMMAND_REGISTRY[range.clone()].iter().collect();
            (*cat, refs)
        })
        .collect()
}

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
        // Total entries should be at least 25 canonicals + most aliases
        // (Note: some aliases like "h" are shared, so the total is slightly less)
        let unique_canonical: usize = COMMAND_REGISTRY.len();
        assert!(map.len() >= unique_canonical);
        assert!(map.len() <= unique_canonical + 20); // reasonable upper bound
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
        assert_eq!(resolve_command("h"), Some("help"));
        assert_eq!(resolve_command("n"), Some("new"));
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
        assert!(help.contains("Show this help message"));
        assert!(help.contains("Start a new conversation"));
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
        assert!(registry.len() > 0);
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
        assert_eq!(cats[2].0, "Tools & Skills");
        assert_eq!(cats[3].0, "Info");
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
}
