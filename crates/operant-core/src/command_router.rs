//! Slash-command router: the ONE shared command contract.
//!
//! Every interactive surface in operant — the TUI (`tui/app/commands.rs`), the
//! gateway (`gateway_commands.rs` + `gateway_runner.rs`), and the CLI chat
//! loop — used to carry its own `CommandDef` type, its own `COMMAND_REGISTRY`
//! table, and its own dispatch. Three registries, one question ("is this a
//! slash command, and which one"), three answers that drifted: `/effort`
//! resolved in the gateway but not in the TUI registry, `/memory` was a
//! gateway command and a different TUI command, and the two `CommandResult`
//! shapes were unrelated. (Hydrogen round-2 F4, iter-2736.)
//!
//! This module is the shared half. It defines:
//!
//! - [`CommandDef`] — static per-command metadata (name, aliases, category,
//!   arg hint, CLI/gateway gating, admin gate). The single type all surfaces
//!   read.
//! - [`CommandResult`] — the structured intent a handler returns
//!   (`Message` / `ShellCommand`), which the surface renders. The TUI and the
//!   gateway both interpret the SAME variants, so "what does this command do"
//!   has one answer.
//! - [`CommandContext`] — the raw argument string a handler executes against.
//! - [`CommandHandler`] — the async trait a surface implements to bind a name
//!   to a behaviour.
//! - [`CommandRegistry`] — name/alias → [`CommandDef`] resolution plus the
//!   registered handlers, built from a static table.
//! - [`resolve_command`] — the three-state text classifier (`NotACommand` /
//!   `Unknown` / `Found`) the gateway's inbound loop uses to tell a mistyped
//!   command apart from prose.
//!
//! What stays OUT is everything host-specific: the static tables themselves
//! (`operant_cli::commands::COMMAND_REGISTRY`, `operant_cli::gateway_commands::COMMAND_REGISTRY`),
//! the TUI's handler closures, and the gateway's session-scoped execution. A
//! command's *behaviour* is host-bound; its *identity* (name, aliases,
//! category, arg hint) is not, and that identity is what this module owns.

use std::collections::HashMap;

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// CommandResult — the structured intent a handler returns
// ---------------------------------------------------------------------------

/// Rich result type for slash-command execution.
///
/// Instead of returning a string, a command expresses its *intent* as data.
/// The surface (TUI or gateway) interprets the variant to perform the side
/// effect. This decouples command logic from the runtime environment, so the
/// same handler can serve a terminal and a chat channel.
///
/// The TUI's `tui/app/commands.rs` and the gateway's `gateway_runner.rs`
/// both interpret these SAME variants — there is one `CommandResult`, not one
/// per surface.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum CommandResult {
    /// Display a message to the user (does NOT go to the model).
    Message(String),
    /// Run a shell command (e.g. `/setup` spawns an interactive subprocess).
    ShellCommand(Vec<String>),
}

impl CommandResult {
    /// Convenience: create a `Message` variant.
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}

impl std::fmt::Display for CommandResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(s) => write!(f, "{s}"),
            Self::ShellCommand(cmds) => write!(f, "ShellCommand({cmds:?})"),
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

// ---------------------------------------------------------------------------
// CommandContext — the raw arguments a handler executes against
// ---------------------------------------------------------------------------

/// Context provided to a command handler on execution.
///
/// The shared contract is deliberately minimal — just the raw argument
/// string. Hosts that need more (the gateway's session pins, steer queue,
/// turn lock; the TUI's `&mut self` app state) carry that in their own
/// handler types, NOT here. This struct stays constructible by every surface
/// without pulling in a single host dependency.
#[derive(Debug, Default)]
pub struct CommandContext<'a> {
    /// The raw argument string following the command name.
    pub args: &'a str,
}

// ---------------------------------------------------------------------------
// CommandHandler — the async trait a surface implements
// ---------------------------------------------------------------------------

/// A slash-command handler that can be registered in a [`CommandRegistry`].
///
/// Implementations should be stateless or use interior mutability, since the
/// same handler may be called multiple times.
// `async_trait` puts a bare `#[must_use]` on every async method it
// generates, and this one returns a `CommandResult` — which is ALREADY
// `#[must_use]` (the enum carries `#[must_use]` above). The lint's own
// suggestion cannot be applied to a macro-generated attribute, so the
// duplication is expected here.
#[allow(
    clippy::double_must_use,
    reason = "async_trait's generated #[must_use] on a must_use return type"
)]
#[async_trait]
pub trait CommandHandler: Send + Sync {
    /// Execute this command with the given context.
    /// Returns a [`CommandResult`] expressing the command's intent as data.
    async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult;
}

// ---------------------------------------------------------------------------
// CommandDef — static per-command metadata
// ---------------------------------------------------------------------------

/// Static metadata for one slash command.
///
/// Each entry is metadata only; behaviour is bound at runtime via
/// [`CommandRegistry::register_handler`]. The SAME type describes a command
/// whether it is dispatched by the TUI or the gateway — the surfaces read
/// different subsets of the fields (the TUI palette reads `name`/`description`/
/// `aliases`; the gateway's `/help` reads `category`/`args_hint`/`admin_only`),
/// but there is ONE definition, not one per surface.
#[derive(Debug, Clone)]
pub struct CommandDef {
    /// Canonical name (without the leading slash).
    pub name: &'static str,
    /// One-line human-readable description.
    pub description: &'static str,
    /// Alternative names that resolve to this command.
    pub aliases: &'static [&'static str],
    /// Display category for grouping in `/help` (e.g. `"Session"`, `"Config"`).
    pub category: &'static str,
    /// Argument hint shown in help (e.g. `"<prompt>"`, `"[key] [value]"`).
    pub args_hint: &'static str,
    /// Only available in interactive CLI (not in gateway/messaging).
    pub cli_only: bool,
    /// Only available in messaging platforms (not in CLI).
    pub gateway_only: bool,
    /// Requires admin privileges to dispatch.
    pub admin_only: bool,
    /// Config dotpath that gates this command in gateway mode.
    ///
    /// When set on a `cli_only` command, the command becomes available in the
    /// gateway if the config value is truthy.
    pub gateway_config_gate: Option<&'static str>,
}

impl CommandDef {
    /// Create a new command definition with minimal fields.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            aliases: &[],
            category: "",
            args_hint: "",
            cli_only: false,
            gateway_only: false,
            admin_only: false,
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

    /// Set the help category.
    pub const fn with_category(mut self, category: &'static str) -> Self {
        self.category = category;
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

    /// Mark this command as admin-only.
    pub const fn admin_only(mut self) -> Self {
        self.admin_only = true;
        self
    }

    /// Set the gateway config gate (a dotpath into `AppConfig`).
    pub const fn with_config_gate(mut self, gate: &'static str) -> Self {
        self.gateway_config_gate = Some(gate);
        self
    }
}

// ---------------------------------------------------------------------------
// CommandRegistry — name/alias → CommandDef, plus registered handlers
// ---------------------------------------------------------------------------

/// A registry that maps command names and aliases to definitions, and
/// canonical names to handlers.
///
/// Built from a static [`COMMAND_REGISTRY`]-style slice via
/// [`CommandRegistry::new`], then extended with handlers at startup
/// (`register_handler`). The TUI registers its handlers in
/// `tui_app::register_tui_command_handlers` + `register_agent_flow_handlers`;
/// the gateway registers its session-scoped handlers in `gateway_runner`.
/// Neither side re-implements name/alias resolution — both go through here.
///
/// [`COMMAND_REGISTRY`]: CommandRegistry::new
pub struct CommandRegistry {
    /// Maps canonical names and aliases to command definitions.
    defs: HashMap<&'static str, &'static CommandDef>,
    /// Maps canonical command names to their handler implementations.
    handlers: HashMap<String, Box<dyn CommandHandler>>,
}

impl CommandRegistry {
    /// Create a new registry pre-populated from `table`.
    ///
    /// The table is a static slice of [`CommandDef`]s (each surface owns its
    /// own slice — the TUI's is `commands::COMMAND_REGISTRY`, the gateway's
    /// is `gateway_commands::COMMAND_REGISTRY`), so the registry can be built
    /// without pulling in the host.
    pub fn new(table: &'static [CommandDef]) -> Self {
        let mut defs = HashMap::new();
        for cmd in table {
            defs.insert(cmd.name, cmd);
            for alias in cmd.aliases {
                defs.insert(alias, cmd);
            }
        }
        Self {
            defs,
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for the given canonical command name.
    ///
    /// Returns an error if the name does not match any known command
    /// definition (canonical or alias).
    pub fn register_handler(
        &mut self,
        name: &str,
        handler: Box<dyn CommandHandler>,
    ) -> Result<(), String> {
        // Validate that this is a known canonical command (not just an alias).
        let Some(def) = self.defs.get(name) else {
            return Err(format!(
                "Unknown command '{name}'. Cannot register handler without a matching CommandDef."
            ));
        };
        self.handlers.insert(def.name.to_string(), handler);
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
                    CommandResult::message(format!("Unknown command: /{name}"))
                }
            }
        }
    }

    /// Whether a handler is registered for `name` (canonical or alias).
    pub fn is_wired(&self, name: &str) -> bool {
        self.resolve(name)
            .is_some_and(|canonical| self.handlers.contains_key(canonical))
    }
}

impl Default for CommandRegistry {
    /// A registry with an empty table and no handlers — the base a surface
    /// extends by registering handlers against its own table.
    fn default() -> Self {
        Self {
            defs: HashMap::new(),
            handlers: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// resolve_command — the three-state text classifier
// ---------------------------------------------------------------------------

/// The outcome of classifying inbound text as a command.
///
/// This is deliberately a three-state answer. A bare `Option` collapses "this
/// is not a command" and "this is a command whose name I do not recognise"
/// into the same `None`. The live gateway treats `None` as "not a command,
/// therefore a prompt for the agent", so a mistyped `/arop` is forwarded to
/// the model and answered as if the user had asked it something — the model
/// then improvises a help reply, which reads like a working command handler
/// that happened not to know the word. The three states keep those apart.
#[derive(Debug, Clone, Copy)]
pub enum CommandMatch<'a> {
    /// Ordinary prose, not a command.
    NotACommand,
    /// A well-formed command whose name is not in the registry.
    Unknown { token: &'a str },
    /// A registry entry matched, with the remainder of the text as its arguments.
    Found(&'static CommandDef, &'a str),
}

impl PartialEq for CommandMatch<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CommandMatch::NotACommand, CommandMatch::NotACommand) => true,
            (CommandMatch::Unknown { token: a }, CommandMatch::Unknown { token: b }) => a == b,
            (CommandMatch::Found(a, x), CommandMatch::Found(b, y)) => a.name == b.name && x == y,
            _ => false,
        }
    }
}

/// Classify a raw message text as a command, its arguments, or prose.
///
/// Extracts the first space-delimited token as the command name (stripping
/// the leading `/`) and matches it case-insensitively against `table`
/// including aliases. Also accepts the group-chat mention form
/// (`/stop@my_bot`) that Telegram clients append when a command is sent by
/// tapping the bot's name.
pub fn resolve_command<'a>(text: &'a str, table: &'static [CommandDef]) -> CommandMatch<'a> {
    let trimmed = text.trim();

    let Some(after_slash) = trimmed.strip_prefix('/') else {
        return CommandMatch::NotACommand;
    };
    let after_slash = after_slash.trim_start();

    let (cmd_token, args) = match after_slash.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((cmd, rest)) => (cmd, rest.trim_start()),
        None => (after_slash, ""),
    };

    let cmd_token = cmd_token
        .split_once('@')
        .map_or(cmd_token, |(name, _)| name);

    if cmd_token.is_empty() {
        return CommandMatch::NotACommand;
    }

    match table.iter().find(|def| {
        def.name.eq_ignore_ascii_case(cmd_token)
            || def
                .aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(cmd_token))
    }) {
        Some(def) => CommandMatch::Found(def, args),
        None => CommandMatch::Unknown { token: cmd_token },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- CommandDef builder tests --------------------------------------------

    #[test]
    fn command_def_with_aliases() {
        let def = CommandDef::new("cmd", "Has aliases").with_aliases(&["c", "command"]);
        assert_eq!(def.aliases, &["c", "command"]);
    }

    #[test]
    fn command_def_with_args() {
        let def = CommandDef::new("cmd", "Takes args").with_args("<name>");
        assert_eq!(def.args_hint, "<name>");
    }

    #[test]
    fn command_def_with_category() {
        let def = CommandDef::new("cmd", "Categorised").with_category("Config");
        assert_eq!(def.category, "Config");
    }

    #[test]
    fn command_def_cli_only() {
        let def = CommandDef::new("cmd", "CLI only").cli_only();
        assert!(def.cli_only);
        assert!(!def.gateway_only);
    }

    #[test]
    fn command_def_gateway_only() {
        let def = CommandDef::new("cmd", "Gateway only").gateway_only();
        assert!(def.gateway_only);
        assert!(!def.cli_only);
    }

    #[test]
    fn command_def_admin_only() {
        let def = CommandDef::new("cmd", "Admin only").admin_only();
        assert!(def.admin_only);
    }

    #[test]
    fn command_def_with_config_gate() {
        let def = CommandDef::new("cmd", "Gated")
            .cli_only()
            .with_config_gate("display.tool_progress_command");
        assert!(def.cli_only);
        assert_eq!(
            def.gateway_config_gate,
            Some("display.tool_progress_command")
        );
    }

    // -- CommandResult tests -------------------------------------------------

    #[test]
    fn command_result_message_constructor() {
        let r = CommandResult::message("hello");
        assert_eq!(r, CommandResult::Message("hello".to_string()));
    }

    #[test]
    fn command_result_from_string() {
        let r: CommandResult = "hi".into();
        assert_eq!(r, CommandResult::Message("hi".to_string()));
    }

    #[test]
    fn command_result_shell_command() {
        let r = CommandResult::ShellCommand(vec!["echo".to_string(), "hi".to_string()]);
        let CommandResult::ShellCommand(argv) = r else {
            panic!("expected ShellCommand");
        };
        assert_eq!(argv, &["echo", "hi"]);
    }

    // -- CommandRegistry tests -----------------------------------------------

    static TABLE: &[CommandDef] = &[
        CommandDef::new("help", "Show help").with_aliases(&["?", "h"]),
        CommandDef::new("exit", "Exit")
            .cli_only()
            .with_aliases(&["quit", "q"]),
        CommandDef::new("status", "Show status").with_category("Info"),
    ];

    struct EchoHandler;
    #[async_trait]
    impl CommandHandler for EchoHandler {
        async fn execute(&self, ctx: &CommandContext<'_>) -> CommandResult {
            CommandResult::message(format!("echo: {}", ctx.args))
        }
    }

    #[test]
    fn registry_new_builds_def_map() {
        let registry = CommandRegistry::new(TABLE);
        assert_eq!(registry.resolve("help"), Some("help"));
        assert_eq!(registry.resolve("h"), Some("help"));
        assert_eq!(registry.resolve("?"), Some("help"));
        assert_eq!(registry.resolve("/exit"), Some("exit"));
        assert_eq!(registry.resolve("q"), Some("exit"));
        assert_eq!(registry.resolve("nonexistent"), None);
    }

    #[test]
    fn registry_register_handler_rejects_unknown() {
        let mut registry = CommandRegistry::new(TABLE);
        let err = registry.register_handler("does_not_exist", Box::new(EchoHandler));
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Unknown"));
    }

    #[test]
    fn registry_register_handler_known() {
        let mut registry = CommandRegistry::new(TABLE);
        let result = registry.register_handler("help", Box::new(EchoHandler));
        assert!(result.is_ok());
    }

    #[test]
    fn registry_register_handler_resolves_via_alias() {
        let mut registry = CommandRegistry::new(TABLE);
        // Registering via an ALIAS binds the canonical name.
        let result = registry.register_handler("h", Box::new(EchoHandler));
        assert!(result.is_ok());
        assert!(registry.is_wired("help"));
        assert!(registry.is_wired("h"));
    }

    #[test]
    fn registry_execute_unwired_returns_fallback_message() {
        let registry = CommandRegistry::new(TABLE);
        let result = futures::executor::block_on(registry.execute("help", ""));
        let CommandResult::Message(text) = result else {
            panic!("expected Message fallback");
        };
        assert!(
            text.contains("not yet wired to a handler"),
            "unwired fallback must say so: {text}"
        );
    }

    #[test]
    fn registry_execute_wired_dispatches_to_handler() {
        let mut registry = CommandRegistry::new(TABLE);
        registry
            .register_handler("status", Box::new(EchoHandler))
            .expect("register");
        let result = futures::executor::block_on(registry.execute("status", "all good"));
        assert_eq!(result, CommandResult::Message("echo: all good".to_string()));
    }

    #[test]
    fn registry_is_wired_false_when_no_handler() {
        let registry = CommandRegistry::new(TABLE);
        assert!(!registry.is_wired("help"));
    }

    // -- resolve_command tests -----------------------------------------------

    #[test]
    fn resolve_command_not_a_command() {
        assert_eq!(
            resolve_command("hello world", TABLE),
            CommandMatch::NotACommand
        );
        assert_eq!(resolve_command("", TABLE), CommandMatch::NotACommand);
    }

    #[test]
    fn resolve_command_found_with_args() {
        let m = resolve_command("/status all good", TABLE);
        let CommandMatch::Found(def, args) = m else {
            panic!("expected Found");
        };
        assert_eq!(def.name, "status");
        assert_eq!(args, "all good");
    }

    #[test]
    fn resolve_command_found_no_args() {
        let m = resolve_command("/help", TABLE);
        let CommandMatch::Found(def, args) = m else {
            panic!("expected Found");
        };
        assert_eq!(def.name, "help");
        assert_eq!(args, "");
    }

    #[test]
    fn resolve_command_found_via_alias() {
        let m = resolve_command("/q", TABLE);
        let CommandMatch::Found(def, _) = m else {
            panic!("expected Found");
        };
        assert_eq!(def.name, "exit");
    }

    #[test]
    fn resolve_command_unknown() {
        let m = resolve_command("/nonexistent", TABLE);
        assert_eq!(
            m,
            CommandMatch::Unknown {
                token: "nonexistent"
            }
        );
    }

    #[test]
    fn resolve_command_telegram_mention_form() {
        // `/stop@my_bot` — Telegram clients append the bot name.
        let m = resolve_command("/help@operant_bot", TABLE);
        let CommandMatch::Found(def, _) = m else {
            panic!("expected Found");
        };
        assert_eq!(def.name, "help");
    }

    #[test]
    fn resolve_command_case_insensitive() {
        let m = resolve_command("/HELP", TABLE);
        let CommandMatch::Found(def, _) = m else {
            panic!("expected Found");
        };
        assert_eq!(def.name, "help");
    }
}
