//! Gateway commands — Telegram bot command definitions, registry, and dispatch.
//!
//! This module provides the canonical set of built-in Telegram bot commands
//! for the Operant gateway. It handles command resolution, admin gating, and
//! generates the JSON payload used by Telegram's `setMyCommands` API.

use chrono::Utc;
use operant_core::config::{AppConfig, ToolProgressMode};
use operant_core::gateway::Gateway;
use operant_core::interrupt::InterruptFlag;

/// Split a `/model` argument into `(model_or_role, provider)`.
///
/// `--provider <name>` was declared in the command's `args_hint` from the day
/// the command was written and was never parsed — the whole trimmed argument
/// became the model id, so `--provider` was passed to the endpoint as part of a
/// model name. This is the parse it was always advertising.
///
/// `--provider` is recognised in any position and `--provider=name` is accepted
/// too, because an operator on a phone types whichever form they remember.
/// The model half keeps its internal spaces (`gpt 4o` is a model name, not
/// two); only the flag is removed.
pub fn split_model_args(args: &str) -> (&str, Option<&str>) {
    let args = args.trim();
    if let Some(rest) = args.strip_prefix("--provider") {
        let rest = rest.trim_start();
        if let Some(value) = rest.strip_prefix('=') {
            return ("", Some(value.trim()));
        }
        return ("", rest.split_whitespace().next());
    }
    // A trailing flag: everything before it is the model, the word after it
    // is the provider.
    if let Some(pos) = args.find("--provider") {
        let (model, rest) = args.split_at(pos);
        let rest = rest["--provider".len()..].trim_start();
        let provider = rest
            .strip_prefix('=')
            .unwrap_or(rest)
            .split_whitespace()
            .next()
            .unwrap_or("");
        (model.trim(), Some(provider))
    } else {
        (args, None)
    }
}

/// Runtime context passed to command handlers for stateful operations.
///
/// Provides handlers access to the gateway, configuration, and other
/// runtime state needed for full command parity with operant-agent Python.
pub struct CommandContext<'a> {
    /// Reference to the running gateway instance.
    pub gateway: Option<&'a Gateway>,
    /// Application configuration (agent model, skills, etc.).
    pub config: &'a AppConfig,
    /// Whether the invoking user is an admin.
    pub is_admin: bool,
    /// The user ID that invoked the command.
    pub user_id: &'a str,
    /// The platform the command came from (e.g. "telegram").
    pub platform: &'a str,
    /// The channel/chat ID the command came from.
    pub channel_id: &'a str,
    /// A handle on the live turn's interrupt flag for this session, when the
    /// caller has a turn in flight. `None` means the caller cannot signal —
    /// `/stop` must report that honestly rather than claiming it stopped
    /// something. This is the same `InterruptFlag` the agent's run loop already
    /// polls, so `/stop` signals existing machinery rather than adding a
    /// second cancellation mechanism.
    pub stop: Option<InterruptFlag>,
    /// Sticky per-session resume overrides. `None` means this context was
    /// built without a gateway runtime, so `/resume` cannot re-point the next
    /// turn and must say so rather than printing a hint it cannot honour.
    pub session_pins: Option<std::sync::Arc<crate::gateway_runner::SessionPins>>,
    /// The routing key the dispatch loop already derived for this message, or
    /// `""` when the caller has none. Passed in rather than re-derived here:
    /// the derivation depends on group/thread shape this context does not carry,
    /// and a third copy of it is exactly how `/stop` silently stops matching.
    pub session_key: &'a str,
    /// The agent's LIVE steer queue — the same `Arc<Mutex<Vec<String>>>` the
    /// TUI's `/steer` pushes to and `drain_steers` drains at the iteration
    /// boundary. `None` means no live agent (tests, detached contexts) and
    /// `/steer` must say so rather than writing a metadata flag nothing reads
    /// (RT-175: the old gateway /steer wrote `steer_message` — a key with zero
    /// readers — and replied "steer updated").
    pub steer_queue: Option<std::sync::Arc<tokio::sync::Mutex<Vec<String>>>>,
}

impl<'a> CommandContext<'a> {
    /// Create a new command context.
    pub fn new(
        gateway: Option<&'a Gateway>,
        config: &'a AppConfig,
        is_admin: bool,
        user_id: &'a str,
        platform: &'a str,
        channel_id: &'a str,
    ) -> Self {
        Self {
            gateway,
            config,
            is_admin,
            user_id,
            platform,
            channel_id,
            stop: None,
            session_pins: None,
            session_key: "",
            steer_queue: None,
        }
    }

    /// Attach the live turn's interrupt flag so `/stop` has something to signal.
    ///
    /// Takes `Option` deliberately. Defaulting a missing flag to
    /// `InterruptFlag::new()` would hand `/stop` a *valid, untriggered* flag when
    /// nothing is running, so it would report a successful cancellation of a turn
    /// that never existed — the exact defect this replaced.
    #[must_use]
    pub fn with_stop_flag(mut self, flag: Option<InterruptFlag>) -> Self {
        self.stop = flag;
        self
    }

    /// Attach the sticky resume pins so `/resume` can re-point the next turn.
    #[must_use]
    pub fn with_session_pins(
        mut self,
        pins: Option<std::sync::Arc<crate::gateway_runner::SessionPins>>,
    ) -> Self {
        self.session_pins = pins;
        self
    }

    /// Attach the routing key the dispatch loop derived for this message.
    #[must_use]
    pub fn with_session_key(mut self, session_key: &'a str) -> Self {
        self.session_key = session_key;
        self
    }

    /// Attach the agent's live steer queue (RT-175): the same queue
    /// `drain_steers` drains at the iteration boundary, so `/steer` from a
    /// channel reaches the RUNNING turn exactly like the TUI's `/steer`.
    #[must_use]
    pub fn with_steer_queue(
        mut self,
        queue: Option<std::sync::Arc<tokio::sync::Mutex<Vec<String>>>>,
    ) -> Self {
        self.steer_queue = queue;
        self
    }
}

/// Definition of a single Telegram bot command.
#[derive(Debug)]
pub struct CommandDef {
    /// Primary command name (e.g. "start", "help").
    pub name: &'static str,
    /// Short description shown in the bot's command menu.
    pub description: &'static str,
    /// Alternative names that also resolve to this command.
    pub aliases: &'static [&'static str],
    /// Display category for grouping in `/help`.
    pub category: &'static str,
    /// Argument hint shown in usage (e.g. "`<name>`", "" for none).
    pub args_hint: &'static str,
    /// Whether this command requires admin privileges.
    pub admin_only: bool,
}

/// Static registry of all built-in Telegram bot commands.
///
/// Grouped by category. Order determines display order in `/help`.
pub static COMMAND_REGISTRY: &[CommandDef] = &[
    // ── Session ──────────────────────────────────────────────
    CommandDef {
        name: "new",
        description: "Start a new session",
        aliases: &["reset", "clear"],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "stop",
        description: "Stop current task",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "status",
        description: "Show session status",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "retry",
        description: "Retry last response",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "undo",
        description: "Undo last action",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "resume",
        description: "Resume a saved session",
        aliases: &["continue"],
        category: "Session",
        args_hint: "[name]",
        admin_only: false,
    },
    CommandDef {
        name: "session",
        // Wave 2 (ORGANISM-ARCHITECTURE §2): show or switch the cast
        // employee this conversation executes as. Binding is a
        // conversation act, not an authority act — the bylaws still gates
        // every tool consult under the bound employee's policy at run
        // time, so an operator cannot /session their way past a lockdown
        // row.
        description: "Show the employee this session runs as, or bind a new one",
        aliases: &["use"],
        category: "Session",
        args_hint: "[employee]",
        admin_only: false,
    },
    CommandDef {
        name: "title",
        description: "Set session title",
        aliases: &[],
        category: "Session",
        args_hint: "[name]",
        admin_only: false,
    },
    CommandDef {
        name: "branch",
        description: "Fork into new session",
        aliases: &["fork"],
        category: "Session",
        args_hint: "[name]",
        admin_only: false,
    },
    CommandDef {
        name: "rollback",
        description: "Rollback to checkpoint",
        aliases: &[],
        category: "Session",
        args_hint: "[number]",
        admin_only: false,
    },
    CommandDef {
        name: "compress",
        description: "Compress session context",
        aliases: &[],
        category: "Session",
        args_hint: "[focus topic]",
        admin_only: false,
    },
    CommandDef {
        name: "background",
        description: "Run task in background",
        aliases: &["bg", "btw"],
        category: "Session",
        args_hint: "<prompt>",
        admin_only: false,
    },
    CommandDef {
        name: "agents",
        description: "Show active agents",
        aliases: &["tasks"],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "subagents",
        description: "List background subagents this gateway dispatched",
        aliases: &["subagent"],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "steer",
        description: "Steer conversation direction",
        aliases: &[],
        category: "Session",
        args_hint: "<prompt>",
        admin_only: false,
    },
    CommandDef {
        name: "goal",
        description: "Manage session goals",
        aliases: &[],
        category: "Session",
        args_hint: "[text|pause|resume|clear|status]",
        admin_only: false,
    },
    CommandDef {
        name: "subgoal",
        description: "Manage sub-goals",
        aliases: &[],
        category: "Session",
        args_hint: "[text|remove N|clear]",
        admin_only: false,
    },
    CommandDef {
        name: "topic",
        description: "Switch conversation topic",
        aliases: &[],
        category: "Session",
        args_hint: "[off|help|session-id]",
        admin_only: false,
    },
    CommandDef {
        name: "whoami",
        description: "Show user identity",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "profile",
        description: "Show active profile",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "sethome",
        description: "Set home session",
        aliases: &["set-home"],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "queue",
        description: "Queue a prompt for next turn",
        aliases: &["q"],
        category: "Session",
        args_hint: "<prompt>",
        admin_only: false,
    },
    CommandDef {
        name: "restart",
        description: "Restart the gateway",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: true,
    },
    // RT-177: the estop gate (gateway/mod.rs::route_message) rejects every
    // inbound message with "Resume with `operant resume`" — an instruction a
    // channel user cannot follow. Command dispatch runs BEFORE that gate
    // (gateway_runner.rs inbound loop), so these two commands are the only
    // way to operate the pause from the channel that is paused, and the
    // reason they must live here rather than inside the turn handler.
    // NOTE: the estop-clear command is `unpause`, NOT `resume` — /resume is
    // TAKEN (resume a saved session, above). The first draft of this change
    // duplicated the name; the RT-178 uniqueness guard is what catches that
    // shape now.
    CommandDef {
        name: "pause",
        description: "Pause operant globally (new work only)",
        aliases: &[],
        category: "Session",
        args_hint: "[reason]",
        admin_only: true,
    },
    CommandDef {
        name: "unpause",
        description: "Resume operant from a global pause",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: true,
    },
    CommandDef {
        name: "sessions",
        description: "Browse previous sessions",
        aliases: &[],
        category: "Session",
        args_hint: "",
        admin_only: false,
    },
    // ── Config ───────────────────────────────────────────────
    CommandDef {
        name: "model",
        description: "Switch AI model",
        aliases: &["provider"],
        category: "Config",
        args_hint: "[model] [--provider name]",
        admin_only: false,
    },
    CommandDef {
        name: "reasoning",
        description: "Toggle reasoning display",
        aliases: &[],
        category: "Config",
        args_hint: "[level|show|hide]",
        admin_only: false,
    },
    CommandDef {
        name: "effort",
        description: "Set reasoning effort (the TUI /effort picker, over this channel)",
        aliases: &[],
        category: "Config",
        args_hint: "[off|minimal|low|medium|high|xhigh|status]",
        admin_only: false,
    },
    CommandDef {
        name: "plan",
        description: "Toggle plan mode (read-only: gated tools are denied, not prompted)",
        aliases: &[],
        category: "Config",
        args_hint: "[on|off|status]",
        admin_only: false,
    },
    CommandDef {
        name: "context",
        description: "Show this session's context occupancy (messages, tokens, window)",
        aliases: &[],
        category: "Session",
        args_hint: "[status]",
        admin_only: false,
    },
    CommandDef {
        name: "export",
        description: "Export this session's transcript to a markdown file",
        aliases: &["transcript"],
        category: "Session",
        args_hint: "[path]",
        admin_only: false,
    },
    CommandDef {
        name: "memory",
        description: "Show the live memory engine, or recall entries for a query",
        aliases: &[],
        category: "Config",
        args_hint: "[query]",
        admin_only: false,
    },
    CommandDef {
        name: "footer",
        description: "Toggle message footer",
        aliases: &[],
        category: "Config",
        args_hint: "[on|off|status]",
        admin_only: false,
    },
    CommandDef {
        name: "yolo",
        description: "Toggle YOLO mode (auto-approves destructive tool prompts for this channel)",
        aliases: &[],
        category: "Config",
        args_hint: "[on|off|status]",
        admin_only: true,
    },
    CommandDef {
        name: "voice",
        description: "Toggle voice output",
        aliases: &[],
        category: "Config",
        args_hint: "[on|off|tts|status]",
        admin_only: false,
    },
    CommandDef {
        name: "personality",
        description: "Set AI personality",
        aliases: &[],
        category: "Config",
        args_hint: "[name]",
        admin_only: false,
    },
    CommandDef {
        name: "codex-runtime",
        description: "Toggle codex app-server runtime",
        aliases: &[],
        category: "Config",
        args_hint: "[auto|codex_app_server]",
        admin_only: false,
    },
    CommandDef {
        name: "verbose",
        description: "Cycle tool progress display",
        aliases: &[],
        category: "Config",
        args_hint: "[off|new|all|verbose]",
        admin_only: false,
    },
    CommandDef {
        name: "config",
        description: "View or change configuration",
        aliases: &["settings"],
        category: "Config",
        args_hint: "[key] [value]",
        admin_only: true,
    },
    // ── Tools ────────────────────────────────────────────────
    CommandDef {
        name: "reload-mcp",
        description: "Reload MCP servers",
        aliases: &["reload_mcp", "reloadmcp"],
        category: "Tools",
        args_hint: "",
        admin_only: true,
    },
    CommandDef {
        name: "reload-skills",
        description: "Reload skill registry",
        aliases: &["reload_skills", "reloadskills"],
        category: "Tools",
        args_hint: "",
        admin_only: true,
    },
    CommandDef {
        name: "kanban",
        description: "Manage kanban boards",
        aliases: &[],
        category: "Tools",
        args_hint: "[subcommand]",
        admin_only: false,
    },
    CommandDef {
        name: "curator",
        description: "Background skill maintenance",
        aliases: &[],
        category: "Tools",
        args_hint: "[status|run|pin|archive|list-archived]",
        admin_only: false,
    },
    CommandDef {
        name: "skills",
        description: "List installed skills, or expand one for this chat",
        aliases: &["skill"],
        category: "Tools",
        args_hint: "[name [instruction]]",
        admin_only: false,
    },
    CommandDef {
        name: "search",
        description: "Search past sessions (or browse recent ones with no query)",
        aliases: &["find"],
        category: "Session",
        args_hint: "<query>",
        admin_only: false,
    },
    // ── Info ─────────────────────────────────────────────────
    CommandDef {
        name: "start",
        description: "Start the bot",
        aliases: &[],
        category: "Info",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "help",
        description: "Show available commands",
        aliases: &[],
        category: "Info",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "commands",
        description: "List all commands",
        aliases: &[],
        category: "Info",
        args_hint: "[page]",
        admin_only: false,
    },
    CommandDef {
        name: "usage",
        description: "Show usage statistics",
        aliases: &[],
        category: "Info",
        args_hint: "[days]",
        admin_only: false,
    },
    CommandDef {
        name: "insights",
        description: "Show session insights",
        aliases: &[],
        category: "Info",
        args_hint: "[days]",
        admin_only: false,
    },
    CommandDef {
        name: "debug",
        description: "Show debug info",
        aliases: &[],
        category: "Info",
        args_hint: "",
        admin_only: false,
    },
    CommandDef {
        name: "update",
        description: "Check for updates",
        aliases: &[],
        category: "Info",
        args_hint: "",
        admin_only: true,
    },
    CommandDef {
        name: "platform",
        description: "Manage gateway platforms",
        aliases: &[],
        category: "Info",
        args_hint: "<pause|resume|list> [name]",
        admin_only: true,
    },
    // ── Admin ─────────────────────────────────────────────────
    CommandDef {
        name: "approve",
        description: "Approve pending action",
        aliases: &["yes", "y"],
        category: "Admin",
        args_hint: "[session|always]",
        admin_only: true,
    },
    CommandDef {
        name: "deny",
        description: "Deny pending action",
        aliases: &["no", "n"],
        category: "Admin",
        args_hint: "",
        admin_only: true,
    },
    // ── Org bylaws (P3) ─────────────────────────────────────────────────
    CommandDef {
        name: "permissions",
        description: "Show a seat's policy, standing grants and pending escalations",
        aliases: &[],
        category: "Admin",
        args_hint: "[seat]",
        admin_only: false,
    },
    CommandDef {
        name: "grant",
        description: "Grant a seat a capability (mints a TTL'd grant; seats an ungoverned seat)",
        aliases: &[],
        category: "Admin",
        args_hint: "<seat> <tool> [--days N]",
        admin_only: true,
    },
    CommandDef {
        name: "revoke",
        description: "Revoke a grant by id — the next run escalates again",
        aliases: &[],
        category: "Admin",
        args_hint: "<grant_id>",
        admin_only: true,
    },
];

/// The outcome of classifying inbound text as a command.
///
/// This is deliberately a three-state answer. `resolve_command` used to return a
/// bare `Option`, which collapsed "this is not a command" and "this is a command
/// whose name I do not recognise" into the same `None`. The live gateway treated
/// `None` as "not a command, therefore a prompt for the agent", so a mistyped
/// `/arop` was forwarded to the model and answered as if the user had asked it
/// something. The model then improvised a help reply, which read like a working
/// command handler that happened not to know the word — the opposite of the truth.
#[derive(Debug, Clone, Copy)]
pub enum CommandMatch<'a> {
    /// Ordinary prose, not a command.
    NotACommand,
    /// A well-formed command whose name is not in the registry.
    Unknown { token: &'a str },
    /// A registry entry matched, with the remainder of the text as its arguments.
    Found(&'static CommandDef, &'a str),
}

/// Hand-written because [`CommandDef`] is not `PartialEq`; two `Found` values
/// match when they name the same command and carry the same arguments.
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
/// Extracts the first space-delimited token as the command name (stripping the
/// leading `/`) and matches it case-insensitively against the registry including
/// aliases. Unlike the previous `Option`-returning form this distinguishes an
/// unknown command from ordinary text, so the caller can reject the former
/// instead of leaking it to the agent.
///
/// Also accepts the group-chat mention form (`/stop@my_bot`) that Telegram
/// clients append when a command is sent by tapping the bot's name. The previous
/// form left the suffix attached, so the registry lookup missed and even
/// correctly-spelled commands silently failed when invoked that way.
pub fn resolve_command(text: &str) -> CommandMatch<'_> {
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

    match COMMAND_REGISTRY.iter().find(|def| {
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

/// Build a grouped help text from the command registry.
fn build_help_text() -> String {
    let mut text = String::from("*Available commands:*\n\n");
    let mut categories: Vec<&str> = Vec::new();

    for cmd in COMMAND_REGISTRY.iter() {
        if !cmd.admin_only && !categories.contains(&cmd.category) {
            categories.push(cmd.category);
        }
    }

    for category in categories {
        text.push_str(&format!("*{}*\n", category));
        for cmd in COMMAND_REGISTRY.iter() {
            if cmd.category != category || cmd.admin_only {
                continue;
            }
            let hint = if cmd.args_hint.is_empty() {
                String::new()
            } else {
                format!(" {}", cmd.args_hint)
            };
            text.push_str(&format!("/{}{} — {}\n", cmd.name, hint, cmd.description));
        }
        text.push('\n');
    }

    text.push_str("_Only non-admin commands are shown._");
    text
}

/// Take the pending permission request for `channel_id` out of the shared
/// store WITHOUT answering it, so the caller can decide the response first
/// (F2: a seat-policy escalation must mint its grant before the waiting run
/// is told anything, and a refused mint must reach the run as `Deny`).
/// `None` when no request is waiting.
///
/// Borrows the store's Arc (never `.take()`s the `Option`) so repeated
/// `/approve` / `/deny` commands keep working for the lifetime of the
/// process. Regression: the handler previously did `guard.take()`, which
/// permanently emptied `PENDING_PERMISSIONS` after the FIRST approval or
/// denial — every later button tap reported "No pending permission request
/// found" (while the timeout path, holding its own Arc clone, still found
/// the entry).
fn take_pending_permission(
    store: &crate::gateway_runner::PendingPermissions,
    channel_id: &str,
) -> Option<operant_core::agent::ToolPermissionRequest> {
    let Ok(guard) = store.lock() else {
        return None;
    };
    let pending = guard.as_ref()?;
    // Bounded retry: the permission receiver holds the inner tokio mutex
    // only for the brief insert window — yield instead of failing on a
    // transient collision with an in-flight insert.
    for _ in 0..50 {
        if let Ok(mut map) = pending.try_lock() {
            return map.remove(channel_id);
        }
        std::thread::yield_now();
    }
    None
}

/// How many messages of a stored session a resume would put back in context.
///
/// The gateway's own reload path replays the last 20 (`gateway_runner.rs`), so
/// reporting a different number would overstate what the agent can see.
const RESUME_WINDOW: usize = 20;

/// A session id plus the size of the transcript stored under it.
struct StoredTranscript {
    session_id: String,
    messages: usize,
}

/// Read how much of `session_id` is actually stored, from the same database the
/// agent writes its turns to.
///
/// The point is the count: `/resume` may only claim a resume when there is
/// something to continue. Setting an id with nothing behind it would leave the
/// next turn running against an empty context under a session label that
/// promises otherwise — a new lie, worse than the hint it replaced.
fn open_stored_transcript(
    ctx: &CommandContext<'_>,
    session_id: &str,
) -> anyhow::Result<StoredTranscript> {
    let db = operant_core::database::Database::init(ctx.config.database_path.clone())?;
    let stored = db.get_session_messages(session_id)?.len();
    Ok(StoredTranscript {
        session_id: session_id.to_string(),
        messages: stored,
    })
}

/// Handle a known command and return an optional response string.
///
/// The caller is expected to have already resolved the command name via
/// [`resolve_command`] (or by some other means) and passes the canonical
/// `cmd_name` (lowercase, e.g. `"start"`) together with any arguments.
///
/// Admin gating: when `cmd_name` belongs to an admin-only command and
/// `ctx.is_admin` is `false`, this function informs the user that the command
/// requires admin privileges instead of silently ignoring.
///
/// Plugin commands: when the command name is not found in the built-in
/// [`COMMAND_REGISTRY`], this function falls through to the plugin command
/// registry via [`operant_core::plugins::handle_plugin_command`].
/// Org mode checks a `/session <emp>` id against the Holon roster before the
/// bind, so an id no employee backs is refused instead of bound. Returns the
/// refusal to show, or `None` when the bind may proceed. Personal mode always
/// proceeds.
fn unknown_org_employee(config: &AppConfig, requested: &str) -> Option<String> {
    if !config.holon.enabled {
        return None;
    }
    match crate::cmd_org::roster_has_employee(config, requested) {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "`{requested}` is not on the Holon roster — nothing bound. List the ids with `operant org holon roster`."
        )),
        Err(e) => Some(format!(
            "Could not read the Holon roster, so nothing was bound: {e}"
        )),
    }
}

/// GW-5/GW-6: keep this chat's conversation id on its session row, so a
/// restart keeps the choice. Turns read the row when no in-memory pin is set.
fn persist_conversation(ctx: &CommandContext<'_>, conversation: &str) -> Option<()> {
    let store = ctx.gateway?.get_persistent_sessions()?;
    let entry = store.entry_for_source(ctx.platform, ctx.user_id, ctx.channel_id)?;
    store
        .set_conversation_id(&entry.session_key, conversation)
        .ok()
}

/// GW-5/GW-6: start a fresh conversation for this chat. The next turn runs on a
/// new transcript id. The old transcript stays stored under its id, so
/// `/resume` can still reach it.
fn start_fresh_conversation(ctx: &CommandContext<'_>) -> Option<String> {
    let conversation =
        operant_core::gateway_session::PersistentSessionStore::fresh_conversation_id();
    persist_conversation(ctx, &conversation)?;
    if let Some(pins) = &ctx.session_pins {
        pins.set(ctx.session_key, &conversation);
    }
    Some(conversation)
}

/// GW-9: the `/session` seat line. An unbound chat has no seat, so it is never
/// reported as the premiere (G+1: an unbound session is ungoverned).
fn session_seat_line(employee_id: Option<&str>) -> String {
    match employee_id {
        Some(employee) => format!("This session runs as `{employee}`."),
        None => "This session is unbound: it runs ungoverned, with no employee seat. `/session <employee>` binds it.".to_string(),
    }
}

pub fn handle_command(cmd_name: &str, _args: &str, ctx: &CommandContext<'_>) -> Option<String> {
    let args = _args;
    let def = COMMAND_REGISTRY.iter().find(|d| {
        d.name.eq_ignore_ascii_case(cmd_name)
            || d.aliases.iter().any(|a| a.eq_ignore_ascii_case(cmd_name))
    });

    let def = match def {
        Some(d) => d,
        None => {
            // Not a built-in command — check plugin commands.
            return operant_core::plugins::handle_plugin_command(cmd_name, _args);
        }
    };

    if def.admin_only && !ctx.is_admin {
        return Some("This command is only available to admins.".to_string());
    }

    Some(match def.name {
        // ── Info ─────────────────────────────────────────────────────
        "start" => "Hello! I'm Operant AI. Send me a message!".into(),

        "help" => build_help_text(),

        "commands" => {
            let mut text = String::from("*All commands:*\n");
            for cmd in COMMAND_REGISTRY.iter() {
                let hint = if cmd.args_hint.is_empty() {
                    String::new()
                } else {
                    format!(" {}", cmd.args_hint)
                };
                text.push_str(&format!("/{}{} — {}\n", cmd.name, hint, cmd.description));
            }
            text
        }

        // ── Session ──────────────────────────────────────────────────
        "new" => {
            let mut msg = String::from("🔄 **Starting a new session.** Previous conversation cleared.");
            if let Some(gateway) = ctx.gateway
                && let Some(store) = gateway.get_persistent_sessions()
            {
                // GW-5: close by the row's session key. The old code passed the
                // conversation id, which matches no row, so nothing was closed.
                if let Some(entry) = store.entry_for_source(ctx.platform, ctx.user_id, ctx.channel_id) {
                    let _ = store.close_session(&entry.session_key);
                }
                let _ = store.create_session(ctx.platform, ctx.user_id, ctx.channel_id);
            }
            if let Some(conversation) = start_fresh_conversation(ctx) {
                msg.push_str(&format!("\nNew conversation ID: `{conversation}`"));
            }
            msg
        }

        "session" => {
            // Wave 2 (ORGANISM-ARCHITECTURE §2). No args: report the bound
            // employee. With an employee id: validate against the registry
            // (fail with the known-employee list if unknown), bind the
            // session, and say the NEXT turn runs as that employee — the
            // binding is applied at turn start, so the consult seat and the
            // charter flip together, never mid-turn.
            let mut msg = String::new();
            if let Some(gateway) = ctx.gateway {
                if let Some(store) = gateway.get_persistent_sessions() {
                    let entry = store.entry_for_source(ctx.platform, ctx.user_id, ctx.channel_id);
                    match (args.trim(), entry) {
                        ("", Some(entry)) => {
                            msg.push_str(&session_seat_line(entry.employee_id.as_deref()));
                        }
                        ("", None) => {
                            msg.push_str(
                                "No session entry yet — the first message creates it unbound, with no employee seat.",
                            );
                        }
                        (requested, Some(entry)) => {
                            // Org mode checks the id against the Holon roster
                            // first. Personal mode binds the name; the seat
                            // policy resolves it.
                            if let Some(refusal) = unknown_org_employee(ctx.config, requested) {
                                msg.push_str(&refusal);
                            } else {
                                match store.bind_employee(&entry.session_key, requested) {
                                    Ok(Some(_)) => {
                                        // GW-6: a seat change starts a fresh transcript. The
                                        // old one stays stored under its id for `/resume`.
                                        if entry.employee_id.as_deref() != Some(requested) {
                                            start_fresh_conversation(ctx);
                                        }
                                        msg.push_str(&format!(
                                            "Bound this session to `{requested}` — the next turn runs as that employee, on a fresh conversation."
                                        ));
                                    }
                                    Ok(None) => msg.push_str(
                                        "Session entry vanished before binding — retry.",
                                    ),
                                    Err(e) => msg.push_str(&format!("Could not bind: {e}")),
                                }
                            }
                        }
                        (requested, None) => {
                            msg.push_str(&format!(
                                "No session entry for this chat yet — send a message first, then `/session {requested}`."
                            ));
                        }
                    }
                } else {
                    msg.push_str("Persistent sessions are not enabled for this gateway.");
                }
            } else {
                msg.push_str("Session commands need a running gateway.");
            }
            msg
        }

        "stop" => {
            // The agent's run loop already polls an `InterruptFlag`, so this
            // signals real machinery. What it must not do is claim success it
            // did not have: the previous version of this arm wrote
            // `interrupted=true` session metadata — read by nothing anywhere —
            // and returned "Stopping current agent turn" unconditionally, so a
            // `/stop` with no turn in flight reported a cancellation that never
            // happened.
            let mut msg = match &ctx.stop {
                Some(flag) if !flag.is_triggered() => {
                    flag.trigger();
                    tracing::info!(
                        platform = ctx.platform,
                        channel = ctx.channel_id,
                        user = ctx.user_id,
                        "Gateway /stop: signalled the running turn"
                    );
                    "⏹️ **Stopped the running turn.**".to_string()
                }
                Some(_) => {
                    tracing::info!(
                        platform = ctx.platform,
                        channel = ctx.channel_id,
                        user = ctx.user_id,
                        "Gateway /stop: already requested, nothing further signalled"
                    );
                    "⏹️ A stop was already requested for the running turn; it is winding down."
                        .to_string()
                }
                None => {
                    tracing::info!(
                        platform = ctx.platform,
                        channel = ctx.channel_id,
                        user = ctx.user_id,
                        "Gateway /stop: no turn in flight, nothing signalled"
                    );
                    "There is no turn in flight for this chat, so there was nothing to stop."
                        .to_string()
                }
            };
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    // Kept as a breadcrumb that the user asked to stop. Not what
                    // performs the cancellation.
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("interrupted".to_string(), "true".to_string()),
                    ]);
                    msg.push_str(&format!("\nSession `{}` noted the request.", session.session_id));
                }
            }
            msg
        }

        "status" => {
            let mut lines = vec!["*Session Status:*".to_string()];
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                {
                    lines.push(format!("• Session ID: `{}`", session.session_id));
                    lines.push(format!("• Platform: `{}`", session.platform));
                    lines.push(format!("• User: `{}`", session.platform_user_id));
                    lines.push(format!("• Created: `{}`", session.created_at));
                    lines.push(format!("• Last active: `{}`", session.last_active));
                    let meta_count = session.metadata.len();
                    if meta_count > 0 {
                        lines.push(format!("• Metadata keys: {}", meta_count));
                    }
                } else {
                    lines.push("• No active session for this channel.".to_string());
                }
            } else {
                lines.push("• Gateway not connected.".to_string());
            }
            lines.join("\n")
        }

        // RT-175: /retry executes in the DISPATCH LOOP via
        // handle_runtime_command (it needs the turn lock + the invoker's
        // loaded conversation + a real re-run). This sync arm exists for
        // NON-runtime contexts only (tests, detached handlers) and must
        // never claim a retry it cannot perform — the old arm wrote a
        // `retry_requested` flag no runtime ever read (theater).
        "retry" => {
            "🔁 Retry runs against the live agent — unavailable in this context. On a running gateway, send `/retry` again from the chat.".into()
        }

        // RT-175: /undo executes in the DISPATCH LOOP via
        // handle_runtime_command (rewind needs the turn lock + the
        // invoker's loaded conversation). This sync arm exists for
        // NON-runtime contexts only and must never claim an undo it
        // cannot perform — the old arm decremented a `message_count`
        // flag no runtime ever read (theater).
        "undo" => {
            "↩️ Undo runs against the live agent — unavailable in this context. On a running gateway, send `/undo` again from the chat.".into()
        }

        // RT-175: the TUI's /queue is the steer-queue surface — empty
        // lists the LIVE queue, text is an alias for /steer. The old arm
        // wrote a `queued_prompts` metadata key no runtime ever read
        // (theater). Mirroring the TUI exactly: try_lock mirrors its
        // contract — a contended queue means the agent is draining it
        // RIGHT NOW, which is a retry, not a failure.
        "queue" => {
            let a = _args.trim();
            if !a.is_empty() {
                // Reuse the /steer arm verbatim: same queue, same delivery
                // semantics (reaches the agent at the next iteration
                // boundary).
                return handle_command("steer", a, ctx);
            }
            let Some(queue) = ctx.steer_queue.as_ref() else {
                return Some("▶️ Queue is unavailable here — no live agent is attached to this gateway process.".to_string());
            };
            match queue.try_lock() {
                Ok(q) if q.is_empty() => "▶️ Queue is empty — nothing is waiting for the agent.".into(),
                Ok(q) => format!(
                    "▶️ Queued ({}): {}",
                    q.len(),
                    q.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join("; ")
                ),
                Err(_) => "▶️ The queue is busy — the agent is draining it right now.".into(),
            }
        }

        "restart" => {
            let mut msg = String::from("🔄 **Restarting gateway.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                let count = store.get_session_count();
                msg.push_str(&format!("\nActive sessions preserved: {}", count));
                msg.push_str("\nRestart initiated — reconnecting to Telegram...");
            } else {
                msg.push_str("\nGateway not connected — use CLI `operant gateway restart`.");
            }
            msg
        }

        // RT-177: pause/unpause operable from the channel. These call the
        // SAME estop machinery the CLI does (`operant pause`/`operant
        // resume`), so a channel user is never told to go to the CLI for
        // the one thing command dispatch can still do while paused —
        // dispatch runs before the estop gate in the inbound loop. Both
        // are admin_only because they stop/resume work for EVERY session,
        // every platform, not just this chat. In-flight work is never
        // killed: this is pause-NEW-work, same as the CLI. The estop-clear
        // command is `unpause` because `/resume` already means "resume a
        // saved session" — so the pause reply points at /unpause.
        "pause" => {
            let reason = _args.trim();
            let reason = if reason.is_empty() {
                format!("paused via /pause by {} on {}", ctx.user_id, ctx.platform)
            } else {
                reason.to_string()
            };
            match operant_core::estop::engage(Some(&reason)) {
                Ok(()) => {
                    tracing::info!(
                        user = %ctx.user_id,
                        platform = %ctx.platform,
                        "Gateway /pause: estop engaged"
                    );
                    format!(
                        "⏸️ **Operant paused** (new work only — in-flight turns finish).\nReason: {reason}\nResume with `/unpause` here, or `operant resume` on the CLI."
                    )
                }
                Err(e) => {
                    tracing::warn!(
                        user = %ctx.user_id,
                        error = %crate::gateway_runner::redact_err(&e),
                        "Gateway /pause: engaging the estop failed"
                    );
                    format!("⚠️ Could not pause: {e}")
                }
            }
        }

        "unpause" => {
            if !operant_core::estop::is_engaged() {
                return Some("▶️ Operant is not paused — nothing to resume.".into());
            }
            match operant_core::estop::disengage() {
                Ok(()) => {
                    tracing::info!(
                        user = %ctx.user_id,
                        platform = %ctx.platform,
                        "Gateway /unpause: estop disengaged"
                    );
                    "▶️ **Operant resumed.** New messages reach the agent again.".into()
                }
                Err(e) => {
                    tracing::warn!(
                        user = %ctx.user_id,
                        error = %crate::gateway_runner::redact_err(&e),
                        "Gateway /unpause: disengaging the estop failed"
                    );
                    format!("⚠️ Could not resume: {e}")
                }
            }
        }

        "sessions" => {
            let mut lines = vec!["*Active Sessions:*".to_string()];
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                let total = store.get_session_count();
                lines.push(format!("• Total active sessions: {}", total));
                let sessions = store.list_active_sessions(Some(ctx.platform));
                if sessions.is_empty() {
                    lines.push("• No sessions for this platform.".to_string());
                } else {
                    for s in &sessions {
                        lines.push(format!(
                            "• `{}` — user: `{}`, channel: `{}`",
                            s.session_id, s.platform_user_id, s.platform_channel_id
                        ));
                    }
                }
            } else {
                lines.push("• Gateway not connected. Use CLI `operant sessions list`.".to_string());
            }
            lines.join("\n")
        }

        "resume" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/resume <session-id>` — continues a saved session by ID on the next turn.\n\
                 Find session IDs via `operant sessions list` (or CLI `operant sessions show <id>`)."
                    .into()
            } else if let Some(pins) = ctx.session_pins.as_ref().filter(|_| !ctx.session_key.is_empty())
            {
                // Honesty gate: a session is only resumable if the transcript it
                // would continue actually exists. Without this check the command
                // would set the id and claim a resume the agent then performs
                // against an empty context.
                match open_stored_transcript(ctx, a) {
                    Ok(stored) if stored.messages == 0 => {
                        tracing::warn!(
                            platform = ctx.platform,
                            channel = ctx.channel_id,
                            user = ctx.user_id,
                            requested = a,
                            "Gateway /resume: no stored transcript for the requested session"
                        );
                        format!(
                            "❌ No stored conversation for session `{}` — nothing to resume.\n\
                             Session IDs come from `operant sessions list`.",
                            a
                        )
                    }
                    Ok(stored) => {
                        pins.set(ctx.session_key, &stored.session_id);
                        // GW-6: persisted too, so a restart keeps the resumed transcript.
                        persist_conversation(ctx, &stored.session_id);
                        tracing::info!(
                            platform = ctx.platform,
                            channel = ctx.channel_id,
                            user = ctx.user_id,
                            session_id = %stored.session_id,
                            messages = stored.messages,
                            "Gateway /resume: pinned this chat to the requested session"
                        );
                        let replayed = stored.messages.min(RESUME_WINDOW);
                        format!(
                            "✅ Continuing session `{}` — {} of its {} stored message(s) reloaded into context.\n\
                             Your next message in this chat continues that conversation.\n\
                             `/new` returns this chat to its own session.",
                            stored.session_id, replayed, stored.messages
                        )
                    }
                    Err(e) => {
                        tracing::error!(
                            platform = ctx.platform,
                            channel = ctx.channel_id,
                            user = ctx.user_id,
                            requested = a,
                            error = %crate::gateway_runner::redact_err(&e),
                            "Gateway /resume: transcript lookup failed"
                        );
                        format!("❌ Could not read session `{}`: {}", a, e)
                    }
                }
            } else {
                format!(
                    "Session resume requested for `{}`, but this context has no live agent to re-point.\n\
                     Resume from the CLI: `operant sessions resume {}`",
                    a, a
                )
            }
        }

        "title" => {
            let a = _args.trim();
            if a.is_empty() {
                let mut msg = String::from("*Session Info:*\n");
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                    {
                        msg.push_str(&format!("• Session ID: `{}`\n", session.session_id));
                        let title = session.metadata.get("title").map(|s| s.as_str()).unwrap_or("(not set)");
                        msg.push_str(&format!("• Title: `{}`\n", title));
                        msg.push_str(&format!("• Created: {}", session.created_at));
                    } else {
                        msg.push_str("No active session.");
                    }
                } else {
                    msg.push_str("Gateway not connected.");
                }
                msg
            } else {
                // RT-176: persist through the by-id API instead of mutating a
                // CLONE and dropping it. The old arm inserted into a local
                // `session` copy and admitted in a comment "we can't persist
                // metadata changes through the current SessionStore API" —
                // then replied success. `set_title` is that persistence.
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) =
                        store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                    {
                        if store.set_title(&session.session_id, a) {
                            format!("📝 Title set to: `{a}` on session `{}`.", session.session_id)
                        } else {
                            format!(
                                "⚠️ Could not set the title — session `{}` vanished before the write.",
                                session.session_id
                            )
                        }
                    } else {
                        "No active session.".into()
                    }
                } else {
                    "Gateway not connected.".into()
                }
            }
        }

        "branch" => {
            let a = _args.trim();
            let mut msg = String::from("🌿 **Branching session.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(source) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    let new_session = store.create_session(ctx.platform, ctx.user_id, ctx.channel_id);
                    if let Ok(new) = new_session {
                        let title = if a.is_empty() { format!("branch-of-{}", &source.session_id[..8]) } else { a.to_string() };
                        // RT-176: annotate the NEW branch session by id. The
                        // old arm passed the SOURCE triple to
                        // update_session_metadata, which re-annotated the
                        // source row and never touched `new` — while the
                        // reply claimed the branch had been created.
                        store.update_session_metadata_by_id(&new.session_id, &[
                            ("title".to_string(), title.to_string()),
                            ("branched_from".to_string(), source.session_id.clone()),
                        ]);
                        msg.push_str(&format!("\nNew branch session: `{}`", new.session_id));
                        msg.push_str(&format!("\nBranched from: `{}`", source.session_id));
                    }
                } else {
                    msg.push_str("\nNo active session to branch from.");
                }
            }
            msg
        }

        // RT-175: the TUI's /rollback points at /rewind ("use /rewind to
        // step back to an earlier turn") — there are no named checkpoints
        // in either surface. The old arm wrote `rollback_to`/
        // `rollback_requested` metadata no runtime ever read. Mirror the
        // TUI: surface the rewind surface honestly. The gateway's rewind
        // is `/undo [N]` (same fork-at-turn machinery as the TUI's
        // /rewind, implemented over `rewind_to_turn`).
        "rollback" => {
            let a = _args.trim();
            if a.is_empty() {
                "⏪ There are no named checkpoints to roll back to. To step back to an earlier turn, use `/undo` (rewind one turn) or `/undo <N>` (rewind N turns). The conversation is forked — the old path stays intact.".into()
            } else {
                format!(
                    "⏪ There are no named checkpoints — `{a}` is not one. To step back, use `/undo` or `/undo <N>`. The conversation is forked, never destroyed."
                )
            }
        }

        // RT-175: /compress executes in the DISPATCH LOOP via
        // handle_runtime_command (compact_history_now needs the turn
        // lock + the invoker's loaded conversation). This sync arm exists
        // for NON-runtime contexts only and must never claim a compaction
        // it cannot perform — the old arm wrote `compress_requested`/
        // `compress_focus` flags no runtime ever read (theater).
        "compress" => {
            "📦 Compress runs against the live agent — unavailable in this context. On a running gateway, send `/compress` again from the chat.".into()
        }

        // RT-179: /context executes in the DISPATCH LOOP via
        // handle_runtime_command (the report reads the live conversation
        // under the turn lock). This sync arm is reached only from
        // contexts with no live agent, so it must not quote a token count
        // it cannot see — it names the live command instead. The old
        // behavior was nothing at all: `/context` fell through
        // `resolve_command` as Unknown and the model answered as prose.
        "context" => {
            "📐 Context occupancy is served by the live gateway command — it reads this session's conversation. Send `/context` from the chat with the gateway running.".into()
        }

        // RT-179 item 3.4: /export executes in the DISPATCH LOOP via
        // handle_runtime_command (it writes THIS chat's loaded transcript
        // under the turn lock, so the file is the invoker's, not whatever
        // conversation happened to be loaded). This sync arm is reached
        // only from contexts with no live agent, so it must not claim a
        // file it cannot read — it names the live command instead. Same
        // shape as /context's stub above, for the same reason.
        "export" => {
            "📄 Export is served by the live gateway command — it reads this session's conversation. Send `/export` from the chat with the gateway running.".into()
        }

        // RT-179 item 3.2: /memory reports the LIVE provider (name, health,
        // bank) and routes a query through the agent's own recall. This sync
        // arm is reached only from contexts with no live agent, so it must
        // not quote an engine state it cannot see — it names the live
        // command and the CLI surface that answers the same question.
        "memory" => {
            let a = _args.trim();
            if a.is_empty() {
                "🧠 Memory status is served by the live gateway command — it reads the agent's own provider. Send `/memory` from the chat with the gateway running, or run `operant memory stats` on the CLI.".into()
            } else {
                format!(
                    "🧠 Recall is served by the live gateway command. Send `/memory {a}` from the chat with the gateway running, or run `operant memory search \"{a}\"` on the CLI."
                )
            }
        }


        // RT-175: honest refusal, mirroring the TUI's /background. The
        // gateway runs ONE shared agent with ONE event channel — there is
        // no second isolated agent to run a detached turn against, and a
        // concurrent agent.run() would interleave into the live transcript.
        // The old arm wrote `background_task`/`background_queued` metadata
        // no runtime ever read and claimed the work was queued. The real
        // detached-turn surface is the CLI: `operant run --query ... &`.
        "background" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/background <prompt>` — sends a prompt that does not need this chat's turn history.\n\
                 This gateway runs one agent at a time (same architecture as the TUI): there is no in-chat background executor.\n\
                 For detached work, run `operant run --query <prompt> &` on the host.".into()
            } else {
                format!(
                    "⏳ Background execution is not available in-chat — this gateway runs one agent with one turn pipeline, so a detached turn would corrupt the live conversation.\n\
                     For detached work: `operant run --query {a} &` on the host.\n\
                     To guide the RUNNING turn instead, use `/steer {a}`."
                )
            }
        }

        "agents" => {
            let mut lines = vec!["*Active Agents:*".to_string()];
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                let count = store.get_session_count();
                lines.push(format!("• Active sessions: {}", count));
                let sessions = store.list_active_sessions(None);
                if !sessions.is_empty() {
                    for s in &sessions {
                        lines.push(format!("• `{}` — platform: `{}`, user: `{}`", s.session_id, s.platform, s.platform_user_id));
                    }
                } else {
                    lines.push("• No active sessions.".to_string());
                }
            } else {
                lines.push("• Gateway not connected.".to_string());
            }
            lines.join("\n")
        }

        "subagents" => {
            // This is a DIFFERENT registry from /agents: /agents lists the
            // gateway's PLATFORM sessions (a telegram chat, a discord
            // channel), while this lists BACKGROUND DELEGATIONS — the
            // subagents this process dispatched via the async delegation
            // tool. The two never overlap, which is why /agents alone left
            // the operator with no view of a running subagent at all.
            //
            // Same-process scope, stated not implied: the registry is a
            // process-local `OnceLock<Mutex<HashMap>>`, so a gateway process
            // that dispatched nothing reports an empty list. That is the
            // truth about this process, not a global "no subagents".
            let a = _args.trim();
            match a {
                "list" | "" => {
                    let records = operant_core::tools::async_delegation::list_records();
                    if records.is_empty() {
                        "*Subagents:* none dispatched by this gateway process.\n\
                         Background delegations are per-process: a subagent \
                         dispatched by the TUI or another `operant run` is not \
                         in this list.\n\
                         `/agents` lists this gateway's platform sessions."
                            .into()
                    } else {
                        let mut lines = vec![format!(
                            "*Subagents:* {} record{}",
                            records.len(),
                            if records.len() == 1 { "" } else { "s" }
                        )];
                        // Elapsed is measured HERE rather than asked of the
                        // registry: core keeps `now_secs` private and its
                        // records carry dispatch_time/completed_time, so the
                        // duration belongs to this display layer.
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        for r in &records {
                            let (glyph, label) = match r.status {
                                operant_core::tools::async_delegation::AsyncDelegationStatus::Pending => {
                                    ("⏳", "running")
                                }
                                operant_core::tools::async_delegation::AsyncDelegationStatus::Completed => {
                                    ("✅", "completed")
                                }
                                operant_core::tools::async_delegation::AsyncDelegationStatus::Failed => {
                                    ("❌", "failed")
                                }
                            };
                            let elapsed = now.saturating_sub(r.dispatch_time);
                            let mins = elapsed / 60;
                            let when = if mins < 1 {
                                format!("{}s ago", elapsed)
                            } else if mins < 60 {
                                format!("{}m ago", mins)
                            } else {
                                format!("{}h{}m ago", mins / 60, mins % 60)
                            };
                            // The goal first and untruncated-by-accident: a
                            // delegation the operator cannot recognise is a row
                            // they must go re-dispatch to understand. It is
                            // squashed to one line so a multi-line prompt does
                            // not break the list layout.
                            let goal: String = r
                                .goal
                                .chars()
                                .take(80)
                                .collect::<String>()
                                .replace('\n', " ");
                            let detail = match (&r.result, &r.error) {
                                (Some(res), _) if !res.is_empty() => {
                                    let snippet: String = res.chars().take(120).collect();
                                    format!(" — {}", snippet.replace('\n', " "))
                                }
                                (_, Some(err)) if !err.is_empty() => {
                                    let snippet: String = err.chars().take(120).collect();
                                    format!(" — {}", snippet.replace('\n', " "))
                                }
                                _ => String::new(),
                            };
                            lines.push(format!(
                                "{} `{}` — {} · {} · {} · {}{}",
                                glyph,
                                r.delegation_id,
                                goal,
                                label,
                                r.model,
                                when,
                                detail
                            ));
                        }
                        lines.join("\n")
                    }
                }
                other => {
                    format!(
                        "Usage: `/subagents` — lists the background delegations this gateway process dispatched.\n\
                         Unknown subcommand `{other}`. There is no per-record view yet: `/subagents` is a list only."
                    )
                }
            }
        }

        "steer" => {
            // RT-175: push to the agent's LIVE steer queue — the same queue
            // `drain_steers` empties at the iteration boundary and the TUI's
            // /steer writes to — instead of the old `steer_message` metadata
            // flag, which nothing in the runtime ever read. try_lock mirrors
            // the TUI contract: a contended queue means the agent is draining
            // it RIGHT NOW, which is a retry, not a failure.
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/steer <direction>` — guides the conversation in a specific direction.\nThe steer message reaches the agent at its next iteration boundary.".into()
            } else {
                let Some(queue) = ctx.steer_queue.as_ref() else {
                    return Some(
                        "🧭 Steer is unavailable here — no live agent is attached to this gateway process. Nothing was queued.".into(),
                    );
                };
                match queue.try_lock() {
                    Ok(mut q) => {
                        q.push(a.to_string());
                        tracing::info!(
                            user = %ctx.user_id,
                            platform = %ctx.platform,
                            queued = q.len(),
                            "Gateway /steer: pushed to the live steer queue"
                        );
                        format!("🧭 **Steer queued.** It reaches the agent at its next iteration boundary.\nSteer: `{a}`")
                    }
                    Err(_) => {
                        tracing::info!(
                            user = %ctx.user_id,
                            platform = %ctx.platform,
                            "Gateway /steer: queue contended (agent draining) — nothing written"
                        );
                        "🧭 The steer queue is being drained by the agent right now — send `/steer` again in a moment. Nothing was written.".into()
                    }
                }
            }
        }

        // RT-175: /goal wrote `goal`/`goal_paused` metadata that no
        // runtime ever read — the agent never saw the goal. The TUI's
        // /goal SUBMITS "[session goal] <text>" as a real user message;
        // the gateway's turn-side companion (handle(), beside the
        // model_override read) now injects the same marker into the next
        // turn's query and consumes the pending flag, so the goal actually
        // reaches the agent. The metadata write stays: it is what the
        // status subcommand and the turn-side read consume.
        "goal" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/goal [text|pause|resume|clear|status]`\nSet or manage the current session goal.\nExamples:\n• `/goal Implement the login feature`\n• `/goal status` — show current goal\n• `/goal clear` — remove goal".into()
            } else {
                let lower = a.to_lowercase();
                match lower.as_str() {
                    "status" => {
                        let mut msg = String::from("*Goal Status:*\n");
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                                let goal = session.metadata.get("goal").map(|s| s.as_str()).unwrap_or("(not set)");
                                let paused = session.metadata.get("goal_paused").map(|s| s.as_str()) == Some("true");
                                let subgoal = session.metadata.get("subgoal").map(|s| s.as_str()).unwrap_or("(not set)");
                                msg.push_str(&format!("• Current goal: `{}`\n", goal));
                                msg.push_str(&format!("• Subgoal: `{}`\n", subgoal));
                                msg.push_str(&format!("• Paused: `{}`", if paused { "yes" } else { "no" }));
                            } else {
                                msg.push_str("No active session.");
                            }
                        } else {
                            msg.push_str("Gateway not connected.");
                        }
                        msg
                    }
                    "pause" => {
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("goal_paused".to_string(), "true".to_string()),
                            ]);
                        }
                        "⏸️ Goal paused. The goal will be preserved but not actively guided.".into()
                    }
                    "resume" => {
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("goal_paused".to_string(), "false".to_string()),
                            ]);
                        }
                        "▶️ Goal resumed.".into()
                    }
                    "clear" => {
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("goal".to_string(), String::new()),
                                ("goal_paused".to_string(), "false".to_string()),
                            ]);
                        }
                        "🗑️ Goal cleared.".into()
                    }
                    _ => {
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            // RT-175 live-acceptance defect: a command can be
                            // the FIRST message a chat ever sends (the acceptance
                            // harness hit this with `/goal` on a fresh chat), so
                            // no session row exists yet and update_session_metadata
                            // is a silent no-op — the goal flag vanished and the
                            // next turn injected nothing while the reply above
                            // promised it would. Create-or-find mirrors the /new
                            // command's store bootstrap, then the write lands.
                            if store
                                .find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                                .is_none()
                            {
                                let _ = store
                                    .create_session(ctx.platform, ctx.user_id, ctx.channel_id);
                            }
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("goal".to_string(), a.to_string()),
                                ("goal_paused".to_string(), "false".to_string()),
                                // RT-175: the turn-side read (handle(),
                                // beside model_override) consumes this flag
                                // and injects the goal marker into the next
                                // turn's query — without it the goal never
                                // reached the agent (theater).
                                ("goal_pending_inject".to_string(), "true".to_string()),
                            ]);
                        }
                        format!("🎯 Goal set: `{a}`\nThe agent sees this goal on your next message here (injected as `[session goal] {a}`).")
                    }
                }
            }
        }

        "subgoal" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/subgoal [text|remove N|clear]`\nSet or manage sub-goals for the current session goal.\nExamples:\n• `/subgoal Design the database schema`\n• `/subgoal remove 1`\n• `/subgoal clear`".into()
            } else {
                let lower = a.to_lowercase();
                match lower.as_str() {
                    "clear" => {
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("subgoal".to_string(), String::new()),
                            ]);
                        }
                        "🗑️ All sub-goals cleared.".into()
                    }
                    _ if lower.starts_with("remove ") => {
                        let num = &a[7..];
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                                let existing = session.metadata.get("subgoal").cloned().unwrap_or_default();
                                let mut parts: Vec<&str> = existing.split('\n').collect();
                                if let Ok(idx) = num.parse::<usize>()
                                    && idx > 0 && idx <= parts.len() {
                                        parts.remove(idx - 1);
                                        let new_val = parts.join("\n");
                                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                            ("subgoal".to_string(), new_val),
                                        ]);
                                    }
                            }
                        }
                        format!("🗑️ Sub-goal #{} removed.", num)
                    }
                    _ => {
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                                let existing = session.metadata.get("subgoal").cloned().unwrap_or_default();
                                let new_val = if existing.is_empty() {
                                    a.to_string()
                                } else {
                                    format!("{}\n{}", existing, a)
                                };
                                let store = gateway.get_session_store();
                                // RT-175 live-acceptance defect: same
                                // create-or-find as the /goal set arm — a
                                // /subgoal on a fresh chat must not lose its
                                // flag to the no-row no-op.
                                if store
                                    .find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                                    .is_none()
                                {
                                    let _ = store
                                        .create_session(ctx.platform, ctx.user_id, ctx.channel_id);
                                }
                                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                    ("subgoal".to_string(), new_val),
                                    // RT-175: same turn-side injection as
                                    // /goal — without a runtime read the
                                    // sub-goal never reached the agent.
                                    ("goal_pending_inject".to_string(), "true".to_string()),
                                ]);
                            }
                        }
                        format!("🎯 Sub-goal set: `{a}` — the agent sees it (with the session goal) on your next message here.")
                    }
                }
            }
        }

        // RT-175: /topic <session-id> was theater (a `topic_mode` metadata
        // key no runtime read). What the command actually promises —
        // "switch to a separate conversation context" — is EXACTLY what
        // /resume's pin does: point this chat at a stored session whose
        // transcript rehydrates on the next turn. Reuse that machinery
        // (same honesty gate: the transcript must exist). `/topic off`
        // returns to the chat's own derived session, same as /new without
        // the fresh-transcript ceremony.
        "topic" => {
            let a = _args.trim();
            if a.is_empty() || a == "help" {
                "Usage: `/topic [off|<session-id>]`\n\
                 Switch this chat to a topic conversation (a stored session), or return to the main one.\n\
                 Examples:\n• `/topic off` — return to the main conversation\n• `/topic <session-id>` — continue that session's conversation here\n\
                 Session IDs come from `operant sessions list` — the same ids /resume accepts.".into()
            } else if a == "off" {
                if let Some(conversation) = start_fresh_conversation(ctx) {
                    format!("🔄 Topic mode disabled. Back on the main conversation `{conversation}`.")
                } else {
                    "🔄 Topic mode disabled — but no persistent session store is attached, so the switch applies to this process only.".into()
                }
            } else if let Some(pins) =
                ctx.session_pins
                    .as_ref()
                    .filter(|_| !ctx.session_key.is_empty())
            {
                // The same existence gate /resume applies: a topic whose
                // transcript does not exist would pin an empty context.
                match open_stored_transcript(ctx, a) {
                    Ok(stored) if stored.messages == 0 => format!(
                        "❌ No stored conversation for session `{a}` — nothing to switch to.\n\
                         Session IDs come from `operant sessions list`."
                    ),
                    Ok(stored) => {
                        pins.set(ctx.session_key, &stored.session_id);
                        // GW-6: persisted too, so a restart keeps the topic.
                        persist_conversation(ctx, &stored.session_id);
                        let replayed = stored.messages.min(RESUME_WINDOW);
                        format!(
                            "🔄 Topic conversation on: continuing session `{}` — {} of its {} stored message(s) reloaded into context.\n\
                             Your next message continues that conversation. `/topic off` returns to the main one.",
                            stored.session_id, replayed, stored.messages
                        )
                    }
                    Err(e) => format!("❌ Could not read session `{a}`: {e}"),
                }
            } else {
                format!(
                    "Session `{a}` was requested as a topic, but this context has no live agent to re-point.\n\
                     Resume it from the CLI: `operant sessions resume {a}`"
                )
            }
        }

        "whoami" => {
            let admin_status = if ctx.is_admin { "✅ Admin" } else { "❌ User" };
            format!(
                "*Identity:*\n• User ID: `{}`\n• Platform: `{}`\n• Channel: `{}`\n• Role: {}",
                ctx.user_id, ctx.platform, ctx.channel_id, admin_status
            )
        }

        "profile" => {
            let model = &ctx.config.agent.model;
            // G4 (plan 2026-10-29): the TUI and the channel must not describe
            // the operator's reasoning setting differently. This printed the
            // raw `[agent] show_reasoning` BOOL, which answers "is it on?"
            // but not "which mode?" — the TUI's contract is three-state
            // (Off / Current / Full, config_shim::ReasoningDisplayMode) and
            // `Full` is unreachable from the config key, so the bool could not
            // say it.
            //
            // Resolve through the SAME shim the TUI boot bridged
            // (main.rs:3254 sets the override on BOTH the TUI and the
            // `gateway run` paths), so the channel reports the mode actually
            // in force rather than a second opinion about it.
            use crate::tui::operant_app::config_shim;
            let reasoning = config_shim::config().display.reasoning_display();
            format!(
                "*Active Profile:*\n• Model: `{}`\n• Max iterations: `{}`\n• Reasoning: `{}`\n\nUse CLI `operant profile` for full profile management.",
                model, ctx.config.agent.max_iterations, reasoning.label()
            )
        }

        "sethome" => {
            if let Some(gateway) = ctx.gateway {
                let dir = gateway.get_channel_directory();
                let is_admin = ctx.is_admin;
                match dir.register_channel(
                    ctx.channel_id,
                    ctx.platform,
                    Some(ctx.user_id),
                    operant_core::gateway::ChannelType::Direct,
                    if is_admin {
                        vec![ctx.user_id.to_string()]
                    } else {
                        Vec::new()
                    },
                ) {
                    Ok(_) => format!(
                        "🏠 Home set! Channel `{}` on `{}` registered.\nThis channel is now configured as your home session.",
                        ctx.channel_id, ctx.platform
                    ),
                    Err(e) => format!(
                        "⚠️ Channel already registered: {}\nUse CLI `operant channels list` for details.",
                        e
                    ),
                }
            } else {
                "⚠️ Gateway not connected. Cannot register channel.".into()
            }
        }

        // ── Config ───────────────────────────────────────────────────
        "config" => {
            "⚙️ Use CLI `operant config` for full configuration management.\nYou can use individual commands like `/model`, `/reasoning`, `/verbose` etc. in chat."
                .into()
        }

        "model" => {
            // (plan 2026-10-30) The role vocabulary from operant-core's
            // `model_roles` -- ONE resolver shared with the TUI's picker, so
            // `/model smol` on Telegram means exactly what Enter on the
            // `smol` row means in the TUI. Three subforms:
            //
            //   /model                  -> the catalogue (which slots exist,
            //                            which model each holds, and whether
            //                            it also switches the conversation)
            //   /model <role>           -> resolve the role's model and use it.
            //                            `smol` and `slow` are operant's
            //                            documented aliases for [recap] and
            //                            [auxiliary_models.memory].
            //   /model <id> [--provider]
            //                          -> a direct id override for the
            //                            conversation slot.
            //
            // The session row is created when missing (the RT-175
            // create-or-find `/goal` got): without it a first-message
            // `/model` is a silent no-op whose reply still promises a
            // switch.
            let (a, provider) = crate::gateway_commands::split_model_args(_args);
            if a.is_empty() {
                let mut msg = String::from("🤖 *Model roles*\n");
                msg.push_str(&operant_core::model_roles::describe_slots(ctx.config));

                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) =
                        store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                        && let Some(override_model) = session.metadata.get("model_override")
                    {
                        msg.push_str(&format!("\nSession override: `{}`", override_model));
                    }
                }
                msg.push_str(&format!(
                    "\n\nUsage: `/model <role>` · `/model <provider/model> [--provider name]`\nConfigured: `{}`",
                    ctx.config.agent.model
                ));
                msg
            } else {
                // Resolve the role BEFORE touching the store, so an unknown
                // word never writes an override the agent would then act on
                // as a typo'd model id.
                match operant_core::model_roles::resolve_selection(
                    a,
                    ctx.config,
                    operant_core::model_roles::EffortRef::Unspecified,
                ) {
                    Err(err) => format!("🤖 {err}"),
                    Ok(selection) => {
                        let slot = selection.slot;
                        let mut model = selection.model.clone();
                        if let Some(p) = provider {
                            let p = p.trim();
                            if !p.is_empty() && !model.contains('/') {
                                model = format!("{}/{}", p, model);
                            }
                        }
                        if let Some(gateway) = ctx.gateway {
                            let store = gateway.get_session_store();
                            // RT-175: create-or-find, exactly /goal's shape.
                            if store.find_session(ctx.platform, ctx.user_id, ctx.channel_id).is_none() {
                                let _ =
                                    store.create_session(ctx.platform, ctx.user_id, ctx.channel_id);
                            }
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("model_override".to_string(), model.clone()),
                            ]);
                        }
                        // A non-conversation slot is an ASSIGNMENT, not a
                        // live switch -- oh-my-pi's rule, stated in the reply
                        // so the operator does not read the confirmation as
                        // "the session model changed now".
                        if slot.switches_conversation() {
                            format!(
                                "🤖 Model switch requested: `{}`\nCurrent model: `{}`\nModel override applied for this session.",
                                model, ctx.config.agent.model
                            )
                        } else {
                            format!(
                                "🤖 `{}` slot set to `{}` for this session.\nThe conversation keeps running on `{}` -- this slot's own consumer uses the new model.",
                                slot.name(),
                                model,
                                ctx.config.agent.model
                            )
                        }
                    }
                }
            }
        }

        "reasoning" => {
            let current = ctx.config.agent.show_reasoning;
            let a = _args.trim();
            match a {
                "" | "status" => {
                    let mut msg = if current {
                        "🧠 Reasoning display is **enabled**.".to_string()
                    } else {
                        "🧠 Reasoning display is **disabled**.".to_string()
                    };
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                            && let Some(override_val) = session.metadata.get("reasoning_override") {
                                msg.push_str(&format!("\nSession override: `{}`", override_val));
                            }
                    }
                    msg.push_str("\nUsage: `/reasoning [show|hide|status]`");
                    msg
                }
                "show" | "on" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("reasoning_override".to_string(), "true".to_string()),
                        ]);
                    }
                    "🧠 Reasoning display **enabled**.".into()
                }
                "hide" | "off" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("reasoning_override".to_string(), "false".to_string()),
                        ]);
                    }
                    "🧠 Reasoning display **disabled**.".into()
                }
                _ => "Usage: `/reasoning [show|hide|status]`".into(),
            }
        }

        "effort" => {
            // RT-175 seam: the LIVE handler (gateway_runner's
            // handle_runtime_command) owns this command — it sets the
            // agent's real reasoning-effort cell via the turn lock. This
            // sync arm is reached only from contexts with no live agent
            // (tests, the help text path), so it must not claim to have
            // changed anything the agent will read.
            match _args.trim().to_ascii_lowercase().as_str() {
                "status" => "🧮 Reasoning effort status is served by the live gateway command (it reads the agent's current cell).".into(),
                "" | "off" | "minimal" | "low" | "medium" | "high" | "xhigh" => "🧮 Send this from a channel with the gateway running — the sync path has no agent to set.".into(),
                other => format!("Unknown effort `{other}` — use off|minimal|low|medium|high|xhigh"),
            }
        }

        "footer" => {
            let a = _args.trim();
            match a {
                "" | "status" => {
                    let mut msg = String::from("📝 Message footer status:");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                            let footer = session.metadata.get("footer_enabled").map(|s| s.as_str()) == Some("true");
                            msg.push_str(&format!(" Session override: `{}`", if footer { "on" } else { "off" }));
                        } else {
                            msg.push_str(" **off** (default).");
                        }
                    } else {
                        msg.push_str(" **off** (default).");
                    }
                    msg.push_str("\nWhen enabled, shows model info and stats after responses.\nUsage: `/footer [on|off|status]`");
                    msg
                }
                "on" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("footer_enabled".to_string(), "true".to_string()),
                        ]);
                    }
                    "📝 Message footer **enabled**.".into()
                }
                "off" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("footer_enabled".to_string(), "false".to_string()),
                        ]);
                    }
                    "📝 Message footer **disabled**.".into()
                }
                _ => "Usage: `/footer [on|off|status]`".into(),
            }
        }

        "yolo" => {
            let arg = _args.trim();
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                // Single source of truth: the live YOLO_CHANNELS set that the
                // permission receiver honors. (The persisted yolo_mode metadata
                // is kept as a record only — reading it here would diverge
                // after a gateway restart, since the set is in-memory.)
                let currently = crate::gateway_runner::yolo_enabled(ctx.platform, ctx.channel_id);
                let new_val = match arg {
                    "on" => true,
                    "off" => false,
                    "" => !currently, // no arg: toggle (legacy behavior)
                    "status" => {
                        let mut msg = format!(
                            "🤘 YOLO mode: `{}` for this channel.\nYOLO mode skips approval prompts for destructive operations.",
                            if currently { "ON" } else { "OFF" }
                        );
                        msg.push_str("\nToggle with `/yolo` (or `/yolo on|off`).");
                        return Some(msg);
                    }
                    _ => return Some("Usage: `/yolo [on|off|status]`".into()),
                };
                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[(
                    "yolo_mode".to_string(),
                    new_val.to_string(),
                )]);
                // Live wiring: the permission receiver honors this set, so YOLO
                // actually skips approval prompts. (R17: the metadata key alone
                // was never read — mirroring the TUI's BypassPermissions.)
                crate::gateway_runner::set_yolo(ctx.platform, ctx.channel_id, new_val);
                format!(
                    "🤘 YOLO mode: `{}` for this channel.\nYOLO mode skips approval prompts for destructive operations.",
                    if new_val { "ON" } else { "OFF" }
                )
            } else {
                "🤘 YOLO mode: gateway unavailable.".to_string()
            }
        }

        "plan" => {
            // RT-179 plan item 2.3 — the gateway's read-only posture, the
            // TUI's PermissionMode::Plan twin. Enforcement is NOT here:
            // the permission receiver reads the live
            // PLAN_CHANNELS set (gateway_runner::plan_mode_enabled)
            // and denies gated tools outright, which is why this arm is a
            // plain toggle rather than a stub. The R17 lesson (/yolo
            // wrote metadata nothing read) is the whole reason the set
            // and not a metadata key is the store.
            //
            // Not persisted (the file metadata record below is an audit
            // echo only): the TUI's own mode is re-picked per process,
            // and a read-only posture silently surviving a restart would
            // surprise an operator who forgot it was on.
            let arg = _args.trim().to_ascii_lowercase();
            let currently = crate::gateway_runner::plan_mode_enabled(ctx.platform, ctx.channel_id);
            let new_val = match arg.as_str() {
                "on" => true,
                "off" => false,
                "" => !currently,
                "status" => {
                    return Some(format!(
                        "📋 Plan mode: `{}` for this channel.\nPlan mode makes the agent read-only: gated tools are denied instead of prompted.\nToggle with `/plan` (or `/plan on|off`).",
                        if currently { "ON" } else { "OFF" }
                    ));
                }
                _ => return Some("Usage: `/plan [on|off|status]`".into()),
            };
            crate::gateway_runner::set_plan_mode(ctx.platform, ctx.channel_id, new_val);
            // Record for audit, exactly as /yolo does. Never read back as
            // the live answer — the set is (reload would otherwise
            // resurrect a posture the operator turned off).
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[(
                    "plan_mode".to_string(),
                    new_val.to_string(),
                )]);
            }
            format!(
                "📋 Plan mode: `{}` for this channel.\nGated tools (edits, commands, writes) are {}.\nYOLO cannot widen this: `/yolo off` then `/plan off` restores prompts.",
                if new_val { "ON" } else { "OFF" },
                if new_val {
                    "denied without a prompt"
                } else {
                    "prompted as usual"
                }
            )
        }

        "voice" => {
            let a = _args.trim();
            match a {
                "" | "status" => {
                    let mut msg = String::from("🔊 Voice output status:");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                            let voice = session.metadata.get("voice_enabled").map(|s| s.as_str()) == Some("true");
                            msg.push_str(&format!(" Session override: `{}`", if voice { "on" } else { "off" }));
                        } else {
                            msg.push_str(" **off** (default).");
                        }
                    } else {
                        msg.push_str(" **off** (default).");
                    }
                    msg.push_str("\nConfigure TTS in `tts` section of config.\nUsage: `/voice [on|off|status]`");
                    msg
                }
                "on" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("voice_enabled".to_string(), "true".to_string()),
                        ]);
                    }
                    "🔊 Voice output **enabled**.".into()
                }
                "off" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("voice_enabled".to_string(), "false".to_string()),
                        ]);
                    }
                    "🔊 Voice output **disabled**.".into()
                }
                _ => "Usage: `/voice [on|off|status]`".into(),
            }
        }

        "personality" => {
            let a = _args.trim();
            if a.is_empty() {
                let mut msg = String::from("🧑 Current personality:");
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                        let personality = session.metadata.get("personality").map(|s| s.as_str()).unwrap_or("(default)");
                        msg.push_str(&format!(" `{}`", personality));
                    } else {
                        msg.push_str(" `(default)`");
                    }
                } else {
                    msg.push_str(" `(default)`");
                }
                msg.push_str("\nUsage: `/personality <name>` — select an AI personality.\nUse CLI `operant personality list` to see available personalities.");
                msg
            } else {
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("personality".to_string(), a.to_string()),
                    ]);
                }
                format!(
                    "🧑 Personality `{}` selected.\nPersonality will be applied to future responses in this session.",
                    a
                )
            }
        }

        "codex-runtime" => {
            let a = _args.trim();
            match a {
                "" | "status" => {
                    let mut msg = String::from("💻 Codex runtime status:");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                            let runtime = session.metadata.get("codex_runtime").map(|s| s.as_str()).unwrap_or("auto");
                            msg.push_str(&format!(" `{}`", runtime));
                        } else {
                            msg.push_str(" `auto` (default).");
                        }
                    } else {
                        msg.push_str(" `auto` (default).");
                    }
                    msg.push_str("\nUsage: `/codex-runtime [auto|codex_app_server|status]`");
                    msg
                }
                "auto" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("codex_runtime".to_string(), "auto".to_string()),
                        ]);
                    }
                    "💻 Codex runtime set to **auto**.".into()
                }
                "codex_app_server" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("codex_runtime".to_string(), "codex_app_server".to_string()),
                        ]);
                    }
                    "💻 Codex runtime set to **codex_app_server**.".into()
                }
                _ => "Usage: `/codex-runtime [auto|codex_app_server|status]`".into(),
            }
        }

        "verbose" => {
            let current = &ctx.config.agent.tool_progress;
            let status_str = match current {
                ToolProgressMode::FinalOnly => "off",
                ToolProgressMode::Auto => "new",
                ToolProgressMode::PerStep => "all",
                ToolProgressMode::Streaming => "verbose",
            };
            let a = _args.trim();
            match a {
                "" => format!(
                    "Current tool progress: `{}`. Usage: `/verbose [off|new|all|verbose]`",
                    status_str
                ),
                "off" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("tool_progress_override".to_string(), "off".to_string()),
                        ]);
                    }
                    "Tool progress display set to: **off**.".into()
                }
                "new" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("tool_progress_override".to_string(), "new".to_string()),
                        ]);
                    }
                    "Tool progress display set to: **new** (shows new tool starts).".into()
                }
                "all" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("tool_progress_override".to_string(), "all".to_string()),
                        ]);
                    }
                    "Tool progress display set to: **all** (shows every step).".into()
                }
                "verbose" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("tool_progress_override".to_string(), "verbose".to_string()),
                        ]);
                    }
                    "Tool progress display set to: **verbose** (streaming output).".into()
                }
                _ => "Usage: `/verbose [off|new|all|verbose]`".into(),
            }
        }

        // ── Tools ────────────────────────────────────────────────────
        "reload-mcp" => {
            let mut msg = String::from("🔄 **Reloading MCP servers...**");
            if let Some(gateway) = ctx.gateway {
                // Signal MCP reload via session metadata so the gateway runner picks it up.
                let store = gateway.get_session_store();
                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                    ("mcp_reload_requested".to_string(), "true".to_string()),
                    ("mcp_reload_requested_at".to_string(), Utc::now().to_rfc3339()),
                ]);
                msg.push_str("\nMCP reload signal sent. Servers will be reconnected on next turn.");
                let dir = gateway.get_channel_directory();
                let channels = dir.list_channels(None);
                msg.push_str(&format!("\nRegistered channels: {}", channels.len()));
            } else {
                msg.push_str("\nGateway not connected. Use CLI `operant mcp restart`.");
            }
            msg
        }

        "reload-skills" => {
            let mut msg = String::from("🔄 **Reloading skill registry...**");
            if let Some(gateway) = ctx.gateway {
                // Signal skills reload via session metadata.
                let store = gateway.get_session_store();
                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                    ("skills_reload_requested".to_string(), "true".to_string()),
                    ("skills_reload_requested_at".to_string(), Utc::now().to_rfc3339()),
                ]);
                msg.push_str("\nSkills reload signal sent. Skills directory will be rescanned.");
                let skills_dir = &ctx.config.skills.root_dir;
                if skills_dir.exists() {
                    msg.push_str(&format!("\nSkills directory: `{}`", skills_dir.display()));
                }
            } else {
                msg.push_str("\nGateway not connected. Use CLI `operant skills reload`.");
            }
            msg
        }

        // /skills [name [instruction]] — RT-179 item 3.3. This command is
        // now the WHOLE skill surface over a channel: no args lists what is
        // installed, a name EXPANDS that skill for this chat through
        // `build_skill_invocation_message_in` — the same function the TUI
        // injects, so both surfaces produce one message shape.
        //
        // The sync arm expands on purpose: expanding a skill is a pure file
        // read, so unlike /memory this arm CAN answer the named form
        // honestly. What it must not do is point at the TUI's browse panel,
        // which a channel cannot render.
        //
        // The arm this replaces listed raw `read_dir` names, which is why
        // `/skills` could only ever be a listing: bare directory names are
        // not invoke-shaped, so the invoke form had nowhere to live.
        "skills" | "skill" => {
            let root = ctx.config.skills.root_dir.clone();
            let rest = args.trim();
            if rest.is_empty() {
                // The browse half, over a channel: the REAL installed list,
                // through the manager that also knows descriptions and
                // prerequisites, so an unmet-prerequisite skill is not
                // listed as usable.
                let mut mgr = operant_core::skills::SkillManager::new(root);
                let loaded = mgr.load_all().unwrap_or_default();
                let rows: Vec<(String, String)> = mgr
                    .list()
                    .into_iter()
                    .filter(|(name, _)| {
                        mgr.get(name).map(|s| mgr.is_available(s)).unwrap_or(false)
                    })
                    .collect();
                let mut reply = format!("🧩 {} installed skills:", loaded.len());
                if rows.is_empty() {
                    reply.push_str(
                        "\n(none — `operant skills list` on the CLI shows what is installed)",
                    );
                } else {
                    // Bounded head + count, the same shape /memory uses.
                    const CHANNEL_SKILL_HEAD: usize = 30;
                    for (name, desc) in rows.iter().take(CHANNEL_SKILL_HEAD) {
                        reply.push_str(&format!("\n• **{name}** — {desc}"));
                    }
                    if rows.len() > CHANNEL_SKILL_HEAD {
                        reply.push_str(&format!("\n…(and {} more)", rows.len() - CHANNEL_SKILL_HEAD));
                    }
                    reply.push_str("\n`/skill <name> [instruction]` to run one.");
                }
                reply
            } else {
                // `/skill <name> [instruction]`, split exactly as the TUI
                // splits it (first whitespace), so both surfaces invoke the
                // same way.
                let (name, instruction) = match rest.split_once(char::is_whitespace) {
                    Some((n, i)) => (n, i.trim().to_string()),
                    None => (rest, String::new()),
                };
                match operant_core::agent::skill_preprocessing::build_skill_invocation_message_in(
                    &root,
                    name,
                    &instruction,
                ) {
                    Some(msg) => {
                        const CHANNEL_SKILL_HEAD_CHARS: usize = 1200;
                        let total = msg.chars().count();
                        let shown: String = msg.chars().take(CHANNEL_SKILL_HEAD_CHARS).collect();
                        let mut out = format!(
                            "🧩 Skill `{name}`{} — send this to run it:\n\n{shown}",
                            if instruction.is_empty() {
                                String::new()
                            } else {
                                format!(" (with your instruction: {instruction})")
                            }
                        );
                        if total > CHANNEL_SKILL_HEAD_CHARS {
                            out.push_str("\n…(truncated)");
                        }
                        out
                    }
                    None => {
                        format!("Skill `{name}` not found — `/skills` with no args lists what is installed.")
                    }
                }
            }
        }

        // RT-179 item 3.5 — /search drives the session_search tool over the
        // live agent's store. See the runtime arm in gateway_runner.rs for
        // why the file-search reading of the TUI's /search is the WRONG
        // twin for a daemon with no project root.
        //
        // This sync arm runs only where no live agent exists, and the tool
        // needs a database the sync context does not carry — so it must not
        // pretend to have searched. It names the live command and the CLI
        // surface that answers the same question.
        "search" | "find" => {
            let a = _args.trim();
            // The CLI has no search subcommand (`operant sessions list` takes
            // no query flag), so the honest pointer is the live command and
            // the model's own tool — NOT a CLI path that does not exist.
            if a.is_empty() {
                "🔎 Session search is served by the live gateway command — it reads the agent's own store. Send `/search` from the chat with the gateway running.".into()
            } else {
                format!(
                    "🔎 Session search is served by the live gateway command. Send `/search {a}` from the chat with the gateway running."
                )
            }
        }

        "kanban" => {
            let a = _args.trim();
            if a.is_empty() {
                "📋 Kanban boards are available via CLI.\nUse `operant kanban list` to view boards.\nCommands: list, show, create, archive, complete, comment"
                    .into()
            } else {
                format!(
                    "📋 Kanban `{}` command received.\nFull kanban management is available via CLI: `operant kanban {}`",
                    a, a
                )
            }
        }

        "curator" => {
            let a = _args.trim();
            match a {
                "" | "status" => {
                    "🔧 Curator: **idle**.\nThe curator manages skill lifecycle (auto-archiving stale skills).\nUse `/curator run` to trigger maintenance.".into()
                }
                "run" => "🔧 Curator maintenance **triggered**.\nThe curator will scan skills and archive stale ones.".into(),
                "pin" | "unpin" | "archive" | "list-archived" | "pause" | "resume" | "restore" => {
                    format!(
                        "🔧 Curator `{}` command received.\nFull curator management via CLI: `operant curator {}`",
                        a, a
                    )
                }
                _ => "Usage: `/curator [status|run|pin|archive|list-archived]`".into(),
            }
        }

        // ── Info (extended) ──────────────────────────────────────────
        "usage" => {
            let a = _args.trim();
            let days = if a.is_empty() { 7 } else { a.parse::<u32>().unwrap_or(7) };
            let mut lines = vec![format!("*📊 Usage Statistics (last {} days):*", days)];
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                let total_sessions = store.get_session_count();
                let active = store.list_active_sessions(None);
                lines.push(format!("• Total sessions: {}", total_sessions));
                lines.push(format!("• Currently active: {}", active.len()));

                // Aggregate message counts from session metadata
                let mut total_messages: usize = 0;
                let mut sessions_with_msgs: usize = 0;
                for s in &active {
                    if let Some(count_str) = s.metadata.get("message_count")
                        && let Ok(count) = count_str.parse::<usize>() {
                            total_messages += count;
                            sessions_with_msgs += 1;
                        }
                }
                lines.push(format!("• Total messages (active sessions): {}", total_messages));
                if sessions_with_msgs > 0 {
                    lines.push(format!(
                        "• Avg messages/session: {}",
                        total_messages.checked_div(sessions_with_msgs).unwrap_or(0)
                    ));
                }

                // Model usage from config
                lines.push(format!("• Current model: `{}`", ctx.config.agent.model));
            } else {
                lines.push("• Gateway not connected. Use CLI `operant insights sessions`.".to_string());
            }
            lines.join("\n")
        }

        "insights" => {
            let a = _args.trim();
            if a.is_empty() {
                "📈 Session insights are available via CLI.\nUse `operant insights sessions` to view session analytics."
                    .into()
            } else {
                format!(
                    "📈 Insights for the last `{}` days: available via CLI `operant insights sessions --days {}`.",
                    a, a
                )
            }
        }

        "debug" => {
            let mut lines = vec!["*🔍 Debug Information:*".to_string()];
            lines.push(format!("• User ID: `{}`", ctx.user_id));
            lines.push(format!("• Platform: `{}`", ctx.platform));
            lines.push(format!("• Channel: `{}`", ctx.channel_id));
            lines.push(format!("• Admin: {}", ctx.is_admin));
            lines.push(format!("• Model: `{}`", ctx.config.agent.model));
            lines.push(format!("• Max iterations: `{}`", ctx.config.agent.max_iterations));
            lines.push(format!("• Reasoning: `{}`", ctx.config.agent.show_reasoning));
            let tp_str = match ctx.config.agent.tool_progress {
                ToolProgressMode::FinalOnly => "off",
                ToolProgressMode::Auto => "auto",
                ToolProgressMode::PerStep => "per step",
                ToolProgressMode::Streaming => "streaming",
            };
            lines.push(format!("• Tool progress: `{}`", tp_str));
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                lines.push(format!("• Active sessions: `{}`", store.get_session_count()));
                let dir = gateway.get_channel_directory();
                let channels = dir.list_channels(None);
                lines.push(format!("• Registered channels: `{}`", channels.len()));
            } else {
                lines.push("• Gateway: **not connected**".to_string());
            }
            lines.push(String::new());
            lines.push("For full debug report, use CLI: `operant debug share`".to_string());
            lines.join("\n")
        }

        "update" => {
            let version = env!("CARGO_PKG_VERSION");
            format!(
                "📦 Operant is at version **v{}**.\nCheck for updates via CLI: `operant update check`\nSee what's new: `operant changelog`",
                version
            )
        }

        "platform" => {
            let a = _args.trim();
            match a {
                "" | "list" => {
                    let mut msg = String::from("*Platform Status:*\n");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        let count = store.get_session_count();
                        msg.push_str(&format!("• Active sessions: {}\n", count));
                        msg.push_str(&format!("• Current platform: `{}`\n", ctx.platform));
                        msg.push_str(&format!("• Admin users configured: {}", ctx.config.gateway.admins.len()));
                        let dir = gateway.get_channel_directory();
                        let channels = dir.list_channels(None);
                        msg.push_str(&format!("\n*Registered Channels:* ({})\n", channels.len()));
                        for ch in &channels {
                            msg.push_str(&format!("• `{}` — platform: `{}`\n", ch.channel_id, ch.platform));
                        }
                    } else {
                        msg.push_str("• Gateway status: **not connected**\n");
                        msg.push_str(&format!("• Current platform: `{}`\n", ctx.platform));
                    }
                    msg.push_str("\nUsage: `/platform <pause|resume|list> [name]`");
                    msg
                }
                "pause" => {
                    let mut msg = String::from("⏸️ **Platform pause requested.**");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("platform_paused".to_string(), "true".to_string()),
                            ("platform_paused_at".to_string(), Utc::now().to_rfc3339()),
                        ]);
                        msg.push_str("\nPlatform will stop processing new messages after current turn.");
                    } else {
                        msg.push_str("\nGateway not connected.");
                    }
                    msg
                }
                "resume" => {
                    let mut msg = String::from("▶️ **Platform resume requested.**");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("platform_paused".to_string(), "false".to_string()),
                        ]);
                        msg.push_str("\nPlatform will resume processing messages.");
                    } else {
                        msg.push_str("\nGateway not connected.");
                    }
                    msg
                }
                _ => format!(
                    "Platform `{}` command received.\nFull platform management via CLI: `operant gateway`",
                    a
                ),
            }
        }

        // ── Admin ────────────────────────────────────────────────────
        "approve" => {
            // (iter-160: resolve pending permission request)
            // hermes parity: `/approve always` grants the tool permanently
            // (persisted allowlist, `approve_permanent`), `/approve` grants
            // it for the rest of the session (`approve_session`).
            //
            // F2: a seat-policy escalation mints its grant FIRST — a
            // refused mint (issue_grant biting) already resolved the row
            // Denied and the waiting run hears Deny; authority is never
            // widened around the check. Ungoverned prompts mint nothing and
            // behave exactly as before.
            let always = args.trim().eq_ignore_ascii_case("always");
            let mut msg = if always {
                String::from("✅✅ **Action approved — always allow.**")
            } else {
                String::from("✅ **Action approved.**")
            };
            let req = crate::gateway_runner::PENDING_PERMISSIONS
                .get()
                .and_then(|store| take_pending_permission(store, ctx.channel_id));
            if let Some(req) = req {
                match crate::gateway_runner::approve_seat_escalation(&req) {
                    // Not a seat escalation — today's response, unchanged.
                    None => {
                        let response = if always {
                            operant_core::agent::ToolPermissionResponse::AllowAlways
                        } else {
                            operant_core::agent::ToolPermissionResponse::AllowSession
                        };
                        let _ = req.response_tx.send(response);
                        msg.push_str(if always {
                            "\nTool execution resumed; the tool is now in the permanent allowlist."
                        } else {
                            "\nTool execution resumed."
                        });
                    }
                    Some(Ok(grant_id)) => {
                        // The grant mints session-and-beyond authority for the
                        // seat; the run's own verdict still obeys the
                        // operator's always/session choice for THIS waiting
                        // call.
                        let response = if always {
                            operant_core::agent::ToolPermissionResponse::AllowAlways
                        } else {
                            operant_core::agent::ToolPermissionResponse::AllowSession
                        };
                        let _ = req.response_tx.send(response);
                        msg.push_str(&format!(
                            "\nSeat-policy approval recorded: grant `{grant_id}` minted and the \
                             queued request resolved. The seat's next run consults the grant."
                        ));
                    }
                    Some(Err(refusal)) => {
                        // issue_grant refused — the row is already recorded
                        // Denied; the refusal text is surfaced and the run is
                        // told Deny. Never bypass.
                        let _ = req
                            .response_tx
                            .send(operant_core::agent::ToolPermissionResponse::Deny);
                        msg.push_str(&format!(
                            "\n⚠️ **Approval could not be recorded — the grant was refused.** \
                             The request is resolved as denied:\n{refusal}"
                        ));
                    }
                }
            } else {
                msg.push_str("\n(No pending permission request found.)");
            }
            msg
        }
        "deny" => {
            // (iter-160: resolve pending permission request)
            let mut msg = String::from("❌ **Action denied.**");
            let req = crate::gateway_runner::PENDING_PERMISSIONS
                .get()
                .and_then(|store| take_pending_permission(store, ctx.channel_id));
            if let Some(req) = req {
                // F2: a denied seat escalation resolves its queue row too.
                if let Some(ask) = req.seat_escalation.as_ref()
                    && let Some(approver) = crate::gateway_runner::seat_approver()
                    && let Err(e) = approver.deny(ask)
                {
                    tracing::warn!(request = %ask.request_id, error = %e, "recording the denial failed");
                }
                let _ = req
                    .response_tx
                    .send(operant_core::agent::ToolPermissionResponse::Deny);
                msg.push_str("\nThe pending action has been cancelled.");
            } else {
                msg.push_str("\n(No pending permission request found.)");
            }
            msg
        }

        // ── Org bylaws (P3) ─────────────────────────────────────────────
        "permissions" => {
            // `/permissions [seat]` — a seat's effective posture. No arg =
            // the invoking chat's own seat, derived EXACTLY as the turn
            // dispatcher derives it, so what is shown is what the run guard
            // consults. Naming another seat is operator-only (§6 Q5); a
            // non-operator reads only their own seat.
            let target = match args.trim() {
                "" => {
                    if ctx.session_key.is_empty() {
                        return Some(
                            "This command context has no routing key — name a seat explicitly: `/permissions <seat>`.".into(),
                        );
                    }
                    crate::gateway_runner::derived_gateway_session_id(ctx.session_key)
                }
                seat => {
                    if !ctx.is_admin {
                        return Some(
                            "Only the operator can inspect another seat. Your own: `/permissions`."
                                .into(),
                        );
                    }
                    seat.to_string()
                }
            };
            let Some(_approver) = crate::gateway_runner::seat_approver() else {
                return Some(
                    "The bylaws is not wired in this process — start the gateway first.".into(),
                );
            };
            let mut msg = format!("**Seat `{target}`**\n");
            // Policy row — `None` is the documented ungoverned default, not
            // an error.
            let policy_row = crate::gateway_runner::seat_policy_db()
                .and_then(|db| db.get(&target).ok())
                .flatten();
            match policy_row {
                Some(policy) => {
                    let mode = policy.mode.as_str();
                    msg.push_str(&format!("Policy: `{mode}`"));
                    if !policy.allow.is_empty() {
                        msg.push_str(&format!(" · allow: {}", policy.allow.join(", ")));
                    }
                    if !policy.deny.is_empty() {
                        msg.push_str(&format!(" · deny: {}", policy.deny.join(", ")));
                    }
                    msg.push('\n');
                }
                None => msg.push_str("Policy: none — ungoverned (today's behaviour)\n"),
            }
            // G+1: the grant ledger retired to Holon (an approved plan is
            // the grant); there is nothing to list.
            msg.push_str("Grants: none (an approved plan is the grant now)\n");
            msg.push_str("Pending escalations: the durable queue is Holon's board now; in-flight asks resolve interactively here.\n");
            msg
        }

        "grant" => "Grants are Holon's now: an approved plan is the grant (compiled to policy.json). `holon plan approve` governs.".into(),

        "revoke" => "Grants are Holon's now — nothing to revoke here. `holon plan approve` governs.".into(),

        _ => format!(
            "Unknown command: `{}`. Use /help to see available commands.",
            def.name
        ),
    })
}

/// Generate a JSON string suitable for Telegram's `setMyCommands` API.
///
/// Only non-admin commands are included. Command names with hyphens are
/// converted to underscores (Telegram Bot API requires lowercase letters,
/// digits, and underscores only).
///
/// Plugin commands registered via [`operant_core::plugins::register_plugin_command`]
/// are appended after the built-in commands.
pub fn telegram_bot_commands() -> String {
    let mut json = String::from('[');
    let mut first = true;

    for cmd in COMMAND_REGISTRY.iter().filter(|c| !c.admin_only) {
        if !first {
            json.push(',');
        }
        first = false;

        let sanitized_name: String = cmd
            .name
            .chars()
            .map(|ch| if ch == '-' { '_' } else { ch })
            .collect();

        let escaped_desc: String = cmd
            .description
            .chars()
            .flat_map(|ch| match ch {
                '"' => vec!['\\', '"'],
                '\\' => vec!['\\', '\\'],
                c => vec![c],
            })
            .collect();

        json.push_str(&format!(
            r#"{{"command":"{}","description":"{}"}}"#,
            sanitized_name, escaped_desc
        ));
    }

    // Append plugin commands.
    for cmd in operant_core::plugins::get_plugin_commands() {
        if !first {
            json.push(',');
        }
        first = false;

        let sanitized_name: String = cmd
            .name
            .chars()
            .map(|ch| if ch == '-' { '_' } else { ch })
            .collect();

        let escaped_desc: String = cmd
            .description
            .chars()
            .flat_map(|ch| match ch {
                '"' => vec!['\\', '"'],
                '\\' => vec!['\\', '\\'],
                c => vec![c],
            })
            .collect();

        json.push_str(&format!(
            r#"{{"command":"{}","description":"{}"}}"#,
            sanitized_name, escaped_desc
        ));
    }

    json.push(']');
    json
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_unbound_chat_is_never_reported_as_the_premiere() {
        // GW-9: the premiere is not a fallback seat, so an unbound chat says so.
        assert_eq!(
            session_seat_line(Some("dispatcher")),
            "This session runs as `dispatcher`."
        );
        let unbound = session_seat_line(None);
        assert!(unbound.contains("unbound"));
        assert!(!unbound.contains("premiere"));
    }

    use super::*;

    /// The canonical name and arguments of a command that must resolve.
    fn found(text: &str) -> (&'static str, &str) {
        match resolve_command(text) {
            CommandMatch::Found(def, args) => (def.name, args),
            other => panic!("expected {text:?} to resolve, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_slash_command() {
        assert_eq!(found("/new").0, "new");
    }

    #[test]
    fn test_resolve_with_args() {
        let (name, args) = found("/model gpt-4");
        assert_eq!(name, "model");
        assert_eq!(args, "gpt-4");
    }

    #[test]
    fn test_resolve_alias() {
        assert_eq!(found("/reset").0, "new");
    }

    #[test]
    fn test_resolve_no_slash() {
        assert_eq!(resolve_command("hello"), CommandMatch::NotACommand);
    }

    /// An unknown command must be distinguishable from prose. It used to return
    /// the same `None` as ordinary text, which is exactly what let a mistyped
    /// command reach the model.
    #[test]
    fn test_resolve_unknown_is_not_prose() {
        assert_eq!(
            resolve_command("/nonexistent"),
            CommandMatch::Unknown {
                token: "nonexistent"
            }
        );
        assert_ne!(
            resolve_command("/nonexistent"),
            resolve_command("nonexistent"),
            "an unknown command must not classify the same as prose"
        );
    }

    /// `/arop` is the exact text that reached the agent in the reported bug.
    #[test]
    fn test_resolve_mistyped_command_is_unknown() {
        assert_eq!(
            resolve_command("/arop"),
            CommandMatch::Unknown { token: "arop" }
        );
    }

    /// Telegram appends `@botname` when a command is sent by tapping the bot's
    /// name in a group. The suffix used to defeat the registry lookup, so even
    /// correctly-spelled commands failed in that form.
    #[test]
    fn test_resolve_group_mention_suffix() {
        assert_eq!(found("/stop@my_bot").0, "stop");
        assert_eq!(found("/new@my_bot arg").1, "arg");
    }

    #[test]
    fn test_resolve_bare_slash_is_prose() {
        assert_eq!(resolve_command("/"), CommandMatch::NotACommand);
        assert_eq!(resolve_command("/ "), CommandMatch::NotACommand);
    }

    #[test]
    fn test_resolve_case_insensitive() {
        assert_eq!(found("/NEW").0, "new");
    }

    #[test]
    fn test_handle_unknown() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123");
        assert!(handle_command("nonexistent", "", &ctx).is_none());
    }

    /// G4 (plan 2026-10-29): `/profile` must report the reasoning MODE the
    /// TUI is actually in, not the raw `[agent] show_reasoning` bool.
    ///
    /// The bool can only say on/off, and the TUI's contract is three-state
    /// (`config_shim::ReasoningDisplayMode`: Off / Current / Full) — so a
    /// `Full` session reported `true`, which is a different word for a
    /// different state. MUTATION PROOF: restoring
    /// `ctx.config.agent.show_reasoning` as the third format arg makes this
    /// fail, because the fixture sets the mode to `Full` while the bool is
    /// `false`.
    ///
    /// Serialised on the shim's own guard: the reasoning cell is a
    /// process-global and a parallel test's write would land mid-read (the
    /// same flake `config_shim`'s own tests guard against).
    #[test]
    fn profile_reports_the_reasoning_mode_not_the_show_reasoning_bool() {
        use crate::tui::operant_app::config_shim::{self, ReasoningDisplayMode};
        let _guard = config_shim::reasoning_override_test_guard();

        // The operator's config says OFF, but the session is displaying FULL
        // (/thinking-display full). A bool reader prints `false` here and
        // contradicts the screen. The bool is set explicitly because the
        // shipped default is `true` (config.rs:865) — the fixture must
        // manufacture the disagreement, not hope for it.
        let mut cfg = AppConfig::default();
        cfg.agent.show_reasoning = false;
        config_shim::set_reasoning_display_override(Some(ReasoningDisplayMode::Full));
        assert!(!cfg.agent.show_reasoning, "fixture: the bool must disagree");
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123");
        let reply = handle_command("profile", "", &ctx).expect("a reply");
        assert!(
            reply.contains("• Reasoning: `Full`"),
            "the channel must name the mode in force (Full), got: {reply}"
        );
        assert!(
            !reply.contains("Reasoning: `false`"),
            "a raw bool leaked into the operator-facing reply: {reply}"
        );
    }

    #[test]
    fn test_admin_gate() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, false, "test", "telegram", "123");
        assert!(handle_command("approve", "", &ctx).is_some());
        let resp = handle_command("approve", "", &ctx).unwrap();
        assert!(resp.contains("admin"));
    }

    #[test]
    fn test_admin_allowed() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123");
        let resp = handle_command("approve", "", &ctx).unwrap();
        assert!(resp.contains("approved"));
    }

    // RT-177: /pause and /unpause must operate the SAME estop the CLI does,
    // through the scoped root so a test never touches the operator's real
    // sentinel (iter-935: test runs once touched a real emergency stop).
    // Serialised because the estop root is a process-global. `/resume` is
    // NOT the estop-clear command — it resumes a saved session; the first
    // draft of this change duplicated the name and the RT-178 guard caught
    // the shape.
    #[serial_test::serial]
    #[test]
    fn pause_command_engages_and_unpause_command_disengages_the_estop() {
        let dir = tempfile::tempdir().expect("temp estop root");
        let _root = operant_core::estop::scoped_root_for_tests(dir.path());
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");

        // Pre-fix state: neither command exists, so handle_command returns
        // None and the estop is never touched — a paused channel user is
        // stuck with "Resume with `operant resume`" and no way to follow it.
        assert!(!operant_core::estop::is_engaged());

        let paused =
            handle_command("pause", "testing RT-177", &ctx).expect("/pause must be handled");
        assert!(
            operant_core::estop::is_engaged(),
            "/pause must engage the estop"
        );
        assert!(
            paused.contains("paused"),
            "the reply must say the pause took hold, not claim success silently"
        );
        assert!(
            paused.contains("/unpause"),
            "the pause reply must point at the estop-clear command, not the session-resume /resume"
        );

        // While engaged, /unpause must still dispatch (the runner's command
        // interception runs BEFORE the estop gate) and clear the sentinel.
        let resumed = handle_command("unpause", "", &ctx).expect("/unpause must be handled");
        assert!(
            !operant_core::estop::is_engaged(),
            "/unpause must disengage"
        );
        assert!(resumed.contains("resumed"));

        // A second /unpause is honest about there being nothing to resume.
        let noop = handle_command("unpause", "", &ctx).expect("handled");
        assert!(noop.contains("not paused"));

        // And /resume stays the SESSION command: with the estop engaged it
        // must not clear the sentinel — it answers about sessions.
        operant_core::estop::engage(Some("session-command-must-not-clear-me")).expect("engage");
        let session_resume = handle_command("resume", "", &ctx)
            .expect("/resume must stay handled as the session command");
        assert!(
            operant_core::estop::is_engaged(),
            "/resume resumes a saved session — it must never clear the estop"
        );
        assert!(
            !session_resume.contains("Operant resumed"),
            "/resume must not speak in the estop-clear voice"
        );
    }

    // RT-175: /steer must push to the LIVE queue the agent drains — not
    // write the dead `steer_message` metadata flag. This is the exact
    // command the audit called out: the old arm replied "steer updated"
    // while nothing in the runtime ever read the flag it wrote.
    #[tokio::test]
    async fn steer_command_pushes_to_the_live_queue_and_writes_no_dead_flag() {
        let cfg = AppConfig::default();
        let queue = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

        // With the queue attached: the steer text lands in the queue.
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1")
            .with_steer_queue(Some(std::sync::Arc::clone(&queue)));
        let resp =
            handle_command("steer", "prefer the serde path", &ctx).expect("/steer must be handled");
        let drained: Vec<String> = queue.lock().await.drain(..).collect();
        assert_eq!(drained, vec!["prefer the serde path".to_string()]);
        assert!(resp.contains("queued"));
        assert!(
            !resp.contains("steer updated"),
            "the old theater reply must be gone"
        );

        // Without a queue: honest refusal, not a fake success.
        let no_q = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let refused = handle_command("steer", "x", &no_q).expect("handled");
        assert!(
            refused.contains("unavailable"),
            "no live agent must be said aloud"
        );

        // Contended queue (held across the call): retry guidance, nothing lost.
        let held = queue.lock().await;
        let contended = handle_command("steer", "y", &ctx).expect("handled");
        assert!(contended.contains("again in a moment"));
        assert!(
            held.is_empty(),
            "nothing may be written while the lock is held"
        );
        drop(held);
    }

    // RT-177 companion: both commands are admin_only — they stop/resume
    // work for every session on every platform, not just this chat.
    #[test]
    fn pause_and_unpause_are_admin_only() {
        let pause = COMMAND_REGISTRY
            .iter()
            .find(|c| c.name == "pause")
            .expect("/pause registry entry");
        let unpause = COMMAND_REGISTRY
            .iter()
            .find(|c| c.name == "unpause")
            .expect("/unpause registry entry");
        assert!(pause.admin_only, "/pause halts every session — admin only");
        assert!(
            unpause.admin_only,
            "/unpause un-halts every session — admin only"
        );
    }

    #[test]
    fn test_help_contains_categories() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, false, "test", "telegram", "123");
        let resp = handle_command("help", "", &ctx).unwrap();
        assert!(resp.contains("Session"));
        assert!(resp.contains("Config"));
        assert!(resp.contains("Tools"));
        assert!(resp.contains("Info"));
    }

    #[test]
    fn test_telegram_json_valid() {
        let json = telegram_bot_commands();
        assert!(json.starts_with('['));
        assert!(json.ends_with(']'));
        assert!(json.contains(r#""command":""#));
        assert!(!json.contains(r#""command":"reload-mcp""#));
    }

    #[test]
    fn test_all_cmds_have_category() {
        for cmd in COMMAND_REGISTRY {
            assert!(
                !cmd.category.is_empty(),
                "Command {} missing category",
                cmd.name
            );
        }
    }

    // RT-178: the registry carried a dead duplicate `session` entry whose
    // alias `status` could never match (resolve_command finds the FIRST
    // entry, and the real `/status` command sits above it). A second entry
    // with the same name is always one of two defects: unreachable dead
    // weight, or an accidental shadow of the live handler. Either way it
    // must fail the build, not sit in the list.
    #[test]
    fn registry_names_and_aliases_are_unique() {
        let mut seen: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
        for cmd in COMMAND_REGISTRY {
            assert!(
                seen.insert(cmd.name),
                "duplicate command name `{}` in COMMAND_REGISTRY — the second entry is dead weight or shadows the live handler (RT-178)",
                cmd.name
            );
            for alias in cmd.aliases {
                assert!(
                    seen.insert(alias),
                    "alias `{}` (on `{}`) collides with an earlier name or alias — resolve_command matches the FIRST entry, so this one can never fire (RT-178)",
                    alias,
                    cmd.name
                );
            }
        }
    }

    #[test]
    fn test_approve_resolves_without_draining_store() {
        use std::sync::Arc;
        use tokio::sync::oneshot;

        // Seed a store with two pending requests on different channels —
        // the shape of PENDING_PERMISSIONS after the first approval prompt.
        let map = {
            let mut m = std::collections::HashMap::new();
            for (ch, tool) in [("chan-a", "code_execution"), ("chan-b", "bash")] {
                let (tx, _rx) = oneshot::channel();
                m.insert(
                    ch.to_string(),
                    operant_core::agent::ToolPermissionRequest {
                        tool_name: tool.to_string(),
                        tool_id: String::new(),
                        description: String::new(),
                        danger_explanation: String::new(),
                        input_preview: None,
                        seat_escalation: None,
                        unattended: false,
                        response_tx: tx,
                    },
                );
            }
            m
        };
        let store: crate::gateway_runner::PendingPermissions =
            std::sync::Mutex::new(Some(Arc::new(tokio::sync::Mutex::new(map))));

        // First /approve takes chan-a.
        assert!(take_pending_permission(&store, "chan-a").is_some());
        // The store must STILL be usable — taking one channel must not
        // drain the Option out of the shared static. (Regression: guard.take()
        // permanently blinded PENDING_PERMISSIONS after the first /approve or
        // /deny, so every later tap reported "No pending permission request
        // found".)
        assert!(take_pending_permission(&store, "chan-b").is_some());
        // Already-taken channel now reports nothing pending.
        assert!(take_pending_permission(&store, "chan-a").is_none());
    }

    #[test]
    fn test_approve_and_deny_share_the_same_store() {
        use std::sync::Arc;
        use tokio::sync::oneshot;

        let (tx, _rx) = oneshot::channel();
        let mut m = std::collections::HashMap::new();
        m.insert(
            "chan-c".to_string(),
            operant_core::agent::ToolPermissionRequest {
                tool_name: "process".to_string(),
                tool_id: String::new(),
                description: String::new(),
                danger_explanation: String::new(),
                input_preview: None,
                seat_escalation: None,
                unattended: false,
                response_tx: tx,
            },
        );
        let store: crate::gateway_runner::PendingPermissions =
            std::sync::Mutex::new(Some(Arc::new(tokio::sync::Mutex::new(m))));

        // Deny takes the request; the store stays alive for the next one.
        assert!(take_pending_permission(&store, "chan-c").is_some());
        assert!(take_pending_permission(&store, "chan-c").is_none());
    }

    /// `/stop` with a live turn must actually signal it. It used to write
    /// session metadata read by nothing anywhere and return "Stopping current
    /// agent turn" unconditionally, so a stop that cancelled nothing was
    /// indistinguishable from one that worked.
    #[test]
    fn stop_triggers_the_live_turn_and_says_so() {
        let cfg = AppConfig::default();
        let flag = InterruptFlag::new();
        assert!(!flag.is_triggered(), "precondition: a turn is running");
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123")
            .with_stop_flag(Some(flag.clone()));

        let resp = handle_command("stop", "", &ctx).expect("stop must be handled");
        assert!(flag.is_triggered(), "/stop must signal the running turn");
        assert!(resp.contains("Stopped the running turn"), "{resp}");
    }

    /// The inverse, and the part that was actually broken: with nothing in
    /// flight, `/stop` must NOT report a cancellation. Defaulting a missing flag
    /// to a fresh untriggered one would hand the command a real flag and
    /// re-introduce precisely this lie.
    #[test]
    fn stop_without_a_running_turn_says_nothing_was_running() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123");

        let resp = handle_command("stop", "", &ctx).expect("stop must be handled");
        assert!(
            !resp.contains("Stopped the running turn"),
            "with no turn in flight /stop must not claim one was stopped: {resp}"
        );
        assert!(resp.contains("nothing to stop"), "{resp}");
    }

    /// A second `/stop` must not report a fresh cancellation, and must not clear
    /// the flag — a stop that un-stopped the turn would be worse than one that
    /// does nothing.
    #[test]
    fn second_stop_reports_already_requested() {
        let cfg = AppConfig::default();
        let flag = InterruptFlag::new();
        flag.trigger();
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123")
            .with_stop_flag(Some(flag.clone()));

        let resp = handle_command("stop", "", &ctx).expect("stop must be handled");
        assert!(resp.contains("already requested"), "{resp}");
        assert!(
            flag.is_triggered(),
            "a second /stop must not clear the flag"
        );
    }

    // ── P3: /permissions ──────────────────────────────────────────
    //
    // These globals (SEAT_APPROVERS / SEAT_POLICY_DBS) are process-wide;
    // every test in this family takes one lock and seeds fresh temp-dir
    // stores, mirroring `permissions_test_guard` in gateway_runner.rs.

    static BYLAWS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn bylaws_test_guard() -> std::sync::MutexGuard<'static, ()> {
        BYLAWS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `/permissions [seat]` shows the seat's policy posture. The grant
    /// ledger retired to Holon (G+1: an approved plan is the grant), so the
    /// view must print the redirect, never a minted grant id. Cross-seat
    /// reads stay operator-only.
    #[test]
    fn permissions_shows_posture_and_gates_other_seats() {
        let _serialised = bylaws_test_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let policies = std::sync::Arc::new(
            operant_core::org::seat_policy_db::SeatPolicyDb::init(
                dir.path().join("seat_policies.sqlite"),
            )
            .expect("SeatPolicyDb::init"),
        );
        policies
            .upsert(
                "emp-worker",
                &operant_core::org::seat_policy::SeatPolicy {
                    mode: operant_core::org::seat_policy::SeatMode::Yolo,
                    allow: vec![],
                    deny: vec!["bash".to_string()],
                },
            )
            .expect("upsert seat policy");
        crate::gateway_runner::store_seat_policy_db(std::sync::Arc::clone(&policies));
        crate::gateway_runner::store_seat_approver(std::sync::Arc::new(
            operant_core::org::seat_authority::SeatApprover::new("op-1"),
        ));

        let cfg = AppConfig::default();
        let admin = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let view = handle_command("permissions", "emp-worker", &admin).expect("view");
        assert!(view.contains("`yolo`"), "policy mode must show: {view}");
        assert!(view.contains("bash"), "the deny list must show: {view}");
        assert!(
            view.contains("an approved plan is the grant"),
            "the grant ledger is Holon's now — no grant id may appear: {view}"
        );

        // A non-operator may read their OWN seat but not name another's.
        let cfg = AppConfig::default();
        let plain = CommandContext::new(None, &cfg, false, "u-9", "telegram", "chan-1")
            .with_session_key("telegram:chan-1:u-9");
        let denied = handle_command("permissions", "emp-worker", &plain).expect("denied");
        assert!(
            denied.contains("Only the operator"),
            "cross-seat reads are operator-only, got: {denied}"
        );
        let own = handle_command("permissions", "", &plain).expect("own view");
        assert!(
            own.contains("ungoverned"),
            "the chat's own seat has no policy row: {own}"
        );
    }

    #[test]
    fn org_bind_refuses_an_id_off_the_roster() {
        use crate::cmd_org::roster_tests::{org_config, org_home};
        let dir = org_home(&["emp_a"]);
        let config = org_config(dir.path());
        assert!(unknown_org_employee(&config, "emp_a").is_none());
        let refusal = unknown_org_employee(&config, "hrmaster").expect("refused");
        assert!(refusal.contains("not on the Holon roster"), "{refusal}");
        let mut personal = org_config(dir.path());
        personal.holon.enabled = false;
        assert!(unknown_org_employee(&personal, "hrmaster").is_none());
    }
}

/// `/subagents` — the delegation registry's gateway surface.
///
/// The defect this pins: `/agents` listed the gateway's PLATFORM sessions
/// (a telegram chat, a discord channel) and the background DELEGATION
/// registry had no reader in the gateway at all, so an operator on a chat
/// platform could not see a running subagent by any means. The registry is
/// a process-global with no public clear, so these tests assert against the
/// rows THEY create (tagged with a unique marker) rather than assuming an
/// empty registry, and serialise on a module lock because two of them seed it.
#[cfg(test)]
mod subagents_command_tests {
    use super::*;
    use operant_core::tools::async_delegation::{create_record, mark_completed, mark_failed};

    // Two of these tests seed the process-global registry and one of them
    // reads it, so EVERY participant takes this lock — the seeders so they do
    // not interleave with each other, and the reader so it cannot observe a
    // registry a sibling is mid-way through seeding. The lock is only as good
    // as the tests that honour it: a reader that skips it turns the guarantee
    // into an intermittent failure that passes when run alone.
    static SUBAGENTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn ctx(cfg: &AppConfig) -> CommandContext<'_> {
        CommandContext::new(None, cfg, true, "op-1", "telegram", "chan-1")
    }

    /// A per-test-unique goal marker, so `list_records()` rows from other
    /// tests (or earlier in this process) can never satisfy an assertion.
    fn marker(test: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!("marker-{test}-{}", N.fetch_add(1, Ordering::SeqCst))
    }

    #[test]
    fn an_empty_registry_names_whose_subagents_are_missing() {
        // The reader takes the SAME lock the two seeding tests take. The
        // serialisation the comment above promises is only real if every
        // participant honours it, and this test originally did not: it read
        // the process-global registry unlocked, so a seeded sibling could
        // land mid-read and the reply could carry the list wording neither
        // branch asserts. That is the intermittent failure this closes —
        // observed once across four runs, passing alone and passing with the
        // module, which is the signature of a race rather than a logic error.
        let _guard = SUBAGENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The registry has no public clear, so this test cannot FORCE the
        // empty branch by draining. Instead it asserts the wording contract
        // that must hold in BOTH arms, and proves the empty arm's sentence
        // exists in the source by the only means available here: when the
        // process really has no records, the reply must be the scope
        // sentence. When it does have terminal history, the reply must be the
        // list — and the list must never claim a global "no subagents".
        let cfg = AppConfig::default();
        let resp = handle_command("subagents", "", &ctx(&cfg)).expect("handled");
        // "record" not "records": the reply pluralises on the count
        // (`"{} record{}"` with an empty suffix for 1), so a registry holding
        // exactly ONE record emits the singular and a `contains("records")`
        // arm test silently falls through to the empty branch. Proven
        // deterministically: run one seeding test and then this reader with
        // --test-threads=1 and it fails every time, no race required. The lock
        // above only hides this by making that state hard to observe.
        if resp.contains("record") {
            assert!(
                !resp.contains("none dispatched by this gateway process"),
                "a non-empty list must not claim the process dispatched nothing: {resp}"
            );
        } else {
            assert!(
                resp.contains("per-process") && resp.contains("/agents"),
                "the empty arm must state WHOSE subagents are missing and \
                 cross-link the other list: {resp}"
            );
        }
    }

    #[test]
    fn a_pending_delegation_is_named_running_and_shows_its_id_model_and_goal() {
        let _guard = SUBAGENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let goal = marker("pending");
        let id = create_record(&goal, "gpt-4.1");
        let cfg = AppConfig::default();

        let resp = handle_command("subagents", "list", &ctx(&cfg)).expect("handled");
        // Assert on the ROW this test created, by its unique id and goal.
        assert!(
            resp.contains(&id),
            "the dispatch handle must be shown: {resp}"
        );
        assert!(resp.contains(&goal), "the goal must show: {resp}");
        assert!(
            resp.contains("gpt-4.1"),
            "the model must show on the row: {resp}"
        );
        assert!(
            resp.contains("running"),
            "a Pending record is live work, not a past-tense row: {resp}"
        );

        mark_completed(&id, "done by test");
    }

    #[test]
    fn a_completed_delegation_shows_its_result_and_a_failed_one_its_error() {
        let _guard = SUBAGENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ok_goal = marker("ok");
        let ok = create_record(&ok_goal, "m");
        mark_completed(&ok, "the audit found four gaps");
        let bad_goal = marker("bad");
        let bad = create_record(&bad_goal, "m");
        mark_failed(&bad, "child timed out");
        let cfg = AppConfig::default();

        let resp = handle_command("subagents", "", &ctx(&cfg)).expect("handled");
        // The result/error strings are the unique markers here.
        assert!(
            resp.contains("the audit found four gaps"),
            "a finished subagent's answer is the question the operator has: {resp}"
        );
        assert!(
            resp.contains("completed"),
            "the status label must be past-tense for a finished row: {resp}"
        );
        assert!(
            resp.contains("child timed out"),
            "a failure must surface its reason, not just the fact: {resp}"
        );
    }

    #[test]
    fn the_alias_and_an_unknown_subcommand_both_resolve() {
        let cfg = AppConfig::default();
        let via_alias = handle_command("subagent", "", &ctx(&cfg)).expect("the alias must resolve");
        assert!(
            via_alias.contains("Subagents") || via_alias.contains("per-process"),
            "the alias must reach the same arm: {via_alias}"
        );

        let bogus = handle_command("subagents", "show 3", &ctx(&cfg)).expect("handled");
        assert!(
            bogus.contains("Unknown subcommand"),
            "a subcommand that does not exist must say so rather than silently \
             listing everything: {bogus}"
        );
        assert!(
            bogus.contains("/agents") || bogus.contains("list only"),
            "the refusal must name what the command actually does: {bogus}"
        );
    }

    #[test]
    fn subagents_and_agents_are_registered_as_distinct_commands() {
        let names: Vec<&str> = COMMAND_REGISTRY.iter().map(|d| d.name).collect();
        assert!(
            names.contains(&"agents") && names.contains(&"subagents"),
            "both lists must be reachable; they answer different questions"
        );
        // The registry's uniqueness guard is the one that would catch a name
        // clash, but assert the resolution directly so the intent is legible
        // at the call site: each text must land on its OWN command.
        let agents = resolve_command("/agents");
        let subagents = resolve_command("/subagents");
        assert!(
            matches!(agents, CommandMatch::Found(d, _) if d.name == "agents"),
            "/agents must resolve to the agents command"
        );
        assert!(
            matches!(subagents, CommandMatch::Found(d, _) if d.name == "subagents"),
            "/subagents must resolve to the subagents command"
        );
        assert_ne!(agents, subagents, "the two lists must not collapse");
        // The alias must land on the same definition as the long form.
        let via_alias = resolve_command("/subagent");
        assert_eq!(
            subagents, via_alias,
            "/subagent is an alias for /subagents, not a separate command"
        );
    }
}

// ── RT-175: the theater-command replacements' contract tests ─────────────
// Every one of these fails against the OLD arms: /queue wrote a metadata
// key, /background claimed a queued task, /rollback claimed a rollback,
// /topic wrote topic_mode. The new arms must either do real work or say
// honestly what cannot be done — never claim success with no runtime read.
#[cfg(test)]
mod rt175_tests {
    use super::*;

    /// /queue <text> is the /steer alias: without a live agent attached it
    /// must say so — the old arm replied "Prompt queued" and wrote
    /// `queued_prompts` metadata nothing read.
    #[test]
    fn queue_with_text_degrades_to_the_steer_honesty_gate() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let resp = handle_command("queue", "do the thing", &ctx).expect("a reply");
        assert!(
            resp.contains("unavailable") || resp.contains("no live agent"),
            "/queue must degrade honestly without a live agent, got: {resp}"
        );
    }

    /// /queue empty lists the live queue state, not a usage line for a
    /// metadata queue that does not exist.
    #[test]
    fn queue_empty_reports_live_queue_state() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let resp = handle_command("queue", "", &ctx).expect("a reply");
        assert!(
            resp.contains("unavailable") || resp.contains("Queue is"),
            "/queue must report the live queue, got: {resp}"
        );
    }

    /// /background must refuse honestly (single agent, no executor) and
    /// point at the real surfaces. The old arm wrote
    /// background_task/background_queued and claimed "queued".
    #[test]
    fn background_refuses_honestly_and_names_the_real_surfaces() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let resp = handle_command("background", "summarize the repo", &ctx).expect("a reply");
        assert!(
            !resp.contains("queued"),
            "/background must never claim a queued task, got: {resp}"
        );
        assert!(
            resp.contains("not available") || resp.contains("no in-chat background executor"),
            "/background must state the honest limitation, got: {resp}"
        );
        assert!(
            resp.contains("operant run"),
            "/background must name the real detached surface, got: {resp}"
        );
    }

    /// /rollback must not claim a rollback to a named checkpoint — there
    /// are no checkpoints. It must point at the rewind surface (/undo).
    #[test]
    fn rollback_names_the_rewind_surface_instead_of_a_fake_checkpoint() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let with_arg = handle_command("rollback", "cp-1", &ctx).expect("a reply");
        assert!(
            !with_arg.contains("Rolling back"),
            "/rollback must never claim a rollback it cannot do, got: {with_arg}"
        );
        assert!(
            with_arg.contains("/undo"),
            "/rollback must point at the rewind surface, got: {with_arg}"
        );
    }

    /// /topic without pins (no live runtime) must say it cannot re-point,
    /// naming the CLI surface. The old arm wrote topic_mode and claimed
    /// "Switching to topic session".
    #[test]
    fn topic_without_runtime_refuses_instead_of_claiming_a_switch() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        let resp = handle_command("topic", "sess_abc123", &ctx).expect("a reply");
        assert!(
            !resp.contains("Switching to topic session"),
            "/topic must not claim a switch it cannot make, got: {resp}"
        );
        assert!(
            resp.contains("no live agent") || resp.contains("operant sessions resume"),
            "/topic must name the real resume surface, got: {resp}"
        );
    }

    /// The sync undo/retry/compress arms are HONESTY STUBS: the live path
    /// is the dispatch loop's handle_runtime_command. The stubs must say
    /// "unavailable in this context" — the old arms wrote
    /// undo_requested/retry_requested/compress_requested and claimed
    /// success.
    #[test]
    fn undo_retry_compress_sync_arms_refuse_honestly() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-1");
        for name in ["undo", "retry", "compress"] {
            let resp = handle_command(name, "", &ctx).expect("a reply");
            assert!(
                resp.contains("unavailable in this context"),
                "/{name} sync arm must be an honest context stub, got: {resp}"
            );
            assert!(
                !resp.contains("**Undoing")
                    && !resp.contains("**Retrying")
                    && !resp.contains("**Compressing"),
                "/{name} sync arm must not claim the operation, got: {resp}"
            );
        }
    }
}

// ── RT-179 plan item 2.3: the functional-core gateway twins ──────────────
// /plan and /effort are the first two commands the TUI registry had with
// no defensible channel twin AND a live enforcement seam. Both tests fail
// against the pre-2.3 tree for the same reason: the names resolved to no
// registry entry at all, so `resolve_command` answered `Unknown` and the
// dispatch loop sent the command to the model as prose.
#[cfg(test)]
mod rt179_functional_core_tests {
    use super::*;
    use crate::gateway_runner::{context_headroom_line, gateway_context_window_for_model};

    /// The two commands must RESOLVE — a name that falls through
    /// `resolve_command` reaches the model as a literal, which is exactly
    /// the gap this slice closes.
    #[test]
    fn plan_and_effort_resolve_as_commands_not_prose() {
        for (text, name) in [("/plan on", "plan"), ("/effort high", "effort")] {
            match resolve_command(text) {
                CommandMatch::Found(def, _) => {
                    assert_eq!(def.name, name, "{text} must resolve to {name}")
                }
                other => panic!("{text} must be a command, got {other:?}"),
            }
        }
        // ...and the artifact list a platform syncs from the registry must
        // carry them, or the channel never SEES the command.
        let bot_commands = telegram_bot_commands();
        assert!(
            bot_commands.contains("\"plan\""),
            "missing /plan: {bot_commands}"
        );
        assert!(
            bot_commands.contains("\"effort\""),
            "missing /effort: {bot_commands}"
        );
    }

    /// /plan toggles the set the permission receiver READS. The R17 lesson
    /// (a metadata key nothing reads is a lie) is the whole point: this
    /// test reads the live accessor, not session metadata.
    #[test]
    fn plan_toggles_the_live_posture_the_receiver_reads() {
        // Isolate: another test's channel key must not leak in, and ours
        // must not leak out.
        let platform = "test";
        let channel = "rt179-plan";
        crate::gateway_runner::set_plan_mode(platform, channel, false);
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", platform, channel);

        let on = handle_command("plan", "on", &ctx).expect("a reply");
        assert!(
            on.contains("ON"),
            "/plan on must report the posture now in force, got: {on}"
        );
        assert!(
            crate::gateway_runner::plan_mode_enabled(platform, channel),
            "/plan on must set the channel in the live plan set"
        );

        // `status` reads the same live state rather than echoing the arg.
        let status = handle_command("plan", "status", &ctx).expect("a reply");
        assert!(
            status.contains("ON"),
            "/plan status must read the live set: {status}"
        );

        let off = handle_command("plan", "off", &ctx).expect("a reply");
        assert!(
            !off.contains("OFF for this channel") || off.contains("prompted"),
            "/plan off must report prompts restored, got: {off}"
        );
        assert!(
            !crate::gateway_runner::plan_mode_enabled(platform, channel),
            "/plan off must clear the live plan set"
        );
    }

    /// A read-only posture must never be widened by YOLO. F2's seat-policy
    /// lesson generalizes: enforcement order is plan BEFORE yolo, so a
    /// channel in both sets denies. Pinned here as an accessor-ordering
    /// contract so the receiver's arm cannot be reordered silently.
    #[test]
    fn plan_mode_is_the_stricter_posture_by_design() {
        // Contract pinned by comment+behavior: plan mode DENIES outright
        // (never prompts, never approves). The receiver checks
        // plan_mode_enabled() before the YOLO arm — this test guards the
        // function pair's independent semantics, which is what lets that
        // ordering be a fact rather than an assumption.
        let platform = "test";
        let channel = "rt179-order";
        crate::gateway_runner::set_plan_mode(platform, channel, false);
        crate::gateway_runner::set_yolo(platform, channel, false);
        assert!(!crate::gateway_runner::plan_mode_enabled(platform, channel));

        crate::gateway_runner::set_plan_mode(platform, channel, true);
        // Setting YOLO afterwards must NOT clear plan mode: the two sets
        // are independent, which is what makes "plan wins" true.
        crate::gateway_runner::set_yolo(platform, channel, true);
        assert!(
            crate::gateway_runner::plan_mode_enabled(platform, channel),
            "yolo must not silently clear plan mode"
        );
        // Cleanup — leaves the shared process-global as found.
        crate::gateway_runner::set_yolo(platform, channel, false);
        crate::gateway_runner::set_plan_mode(platform, channel, false);
    }

    /// /effort with no live agent must not claim the field was set. Its
    /// twin lives on the agent's own reasoning-effort cell; the sync path
    /// has no agent, so the honest answer is "this needs the gateway".
    #[test]
    fn effort_sync_arm_refuses_to_claim_a_set() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-e");
        let resp = handle_command("effort", "high", &ctx).expect("a reply");
        assert!(
            resp.contains("gateway running"),
            "/effort without a live agent must say so, got: {resp}"
        );
        // A junk level is refused by name, not silently accepted.
        let junk = handle_command("effort", "banana", &ctx).expect("a reply");
        assert!(
            junk.contains("Unknown effort"),
            "/effort must reject an unknown level, got: {junk}"
        );
    }

    /// /context must RESOLVE and reach the synced command list — the same
    /// failure mode /plan and /effort had: an unresolved name falls through
    /// `resolve_command` as Unknown and the dispatch loop feeds the literal
    /// to the model, which improvises a help-shaped answer.
    #[test]
    fn context_resolves_as_a_command_not_prose() {
        match resolve_command("/context") {
            CommandMatch::Found(def, _) => {
                assert_eq!(def.name, "context", "/context must resolve to context")
            }
            other => panic!("/context must be a command, got {other:?}"),
        }
        // `status` is the explicit form of the same read; it must not be a
        // separate name that can drift.
        match resolve_command("/context status") {
            CommandMatch::Found(def, args) => {
                assert_eq!(def.name, "context");
                assert_eq!(args, "status");
            }
            other => panic!("/context status must be a command, got {other:?}"),
        }
        let bot_commands = telegram_bot_commands();
        assert!(
            bot_commands.contains("\"context\""),
            "missing /context: {bot_commands}"
        );
    }

    /// The sync arm is the honest-context stub: it names the live command
    /// instead of quoting numbers it cannot see. A channel user never meets
    /// this arm (the dispatch loop's runtime arm answers first), so its only
    /// job is to be honest when reached.
    #[test]
    fn context_sync_arm_refuses_to_quote_numbers_it_cannot_see() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-ctx");
        let resp = handle_command("context", "", &ctx).expect("a reply");
        assert!(
            resp.contains("live gateway command"),
            "the sync arm must point at the live command, got: {resp}"
        );
        assert!(
            !resp.contains("Messages:"),
            "the sync arm has no conversation to count — it must not quote one: {resp}"
        );
    }

    /// The headroom line's arithmetic, and its ceiling. Two surfaces quoting
    /// different denominators is the iter-1207 lie; this pins the ONE place
    /// the share is computed so it cannot silently round past the window.
    #[test]
    fn context_headroom_line_reports_the_share_and_never_exceeds_the_window() {
        // 160k of a 200k window is 80%.
        let line = context_headroom_line(160_000, 200_000);
        assert!(line.contains("80%"), "160k of 200k is 80%: {line}");
        assert!(
            line.contains("160000 of 200000"),
            "the fraction must show: {line}"
        );

        // An unresolved window says so, rather than dividing by zero.
        let unknown = context_headroom_line(1_000, 0);
        assert!(
            unknown.contains("unknown"),
            "a zero window must report unknown: {unknown}"
        );

        // Over the window (possible when the estimator overshoots a very
        // small window): capped at 100 and at the window itself, never a
        // nonsensical 130% or a used count above the window.
        let over = context_headroom_line(400_000, 200_000);
        assert!(over.contains("100%"), "the share is capped: {over}");
        assert!(
            over.contains("200000 of 200000"),
            "the used count is capped at the window: {over}"
        );
    }

    /// The window resolver must agree with the TUI's own fallback table —
    /// the iter-1207 defect was TWO denominators, and the way it reappears
    /// is a gateway resolver that quietly drifts from the TUI's.
    #[test]
    fn gateway_context_window_matches_the_tui_fallback_table() {
        // The TUI table (tui/app/providers.rs refresh_context_window_size):
        // anthropic 200_000, openai 128_000, google 1_048_576, else 128_000.
        // These are the OFFLINE fallbacks — the models_dev cache may override
        // them when populated, so this asserts the FALLBACK path by using
        // model ids the cache cannot resolve.
        assert_eq!(
            gateway_context_window_for_model("anthropic/__nope__"),
            200_000
        );
        assert_eq!(gateway_context_window_for_model("openai/__nope__"), 128_000);
        assert_eq!(
            gateway_context_window_for_model("google/__nope__"),
            1_048_576
        );
        // An unknown provider prefix falls to the flat default, not zero and
        // not infinity — a division by zero would hang the percentage.
        assert_eq!(
            gateway_context_window_for_model("mystery/__nope__"),
            128_000
        );
        // A bare model with no provider prefix still resolves to a finite
        // window rather than an unwindowed divisor.
        assert!(gateway_context_window_for_model("some-bare-model") > 0);
    }
}

// ── RT-179 item 3.4: /export's channel-twin contract ────────────────────
// Resolves as a command (not prose), reaches the platform's command list,
// and the sync arm refuses to claim a file it cannot read.
#[cfg(test)]
mod rt179_export_tests {
    use super::*;

    /// The name must resolve and reach the platform's command list. A name
    /// that falls through `resolve_command` reaches the model as a literal,
    /// which is the gap this slice closes.
    #[test]
    fn export_resolves_as_a_command_and_reaches_the_command_list() {
        for (text, name) in [
            ("/export", "export"),
            ("/transcript", "export"),
            ("/export /tmp/x.md", "export"),
        ] {
            match resolve_command(text) {
                CommandMatch::Found(def, _) => {
                    assert_eq!(def.name, name, "{text} must resolve to {name}")
                }
                other => panic!("{text} must be a command, got {other:?}"),
            }
        }
        let bot_commands = telegram_bot_commands();
        assert!(
            bot_commands.contains("\"export\""),
            "missing /export: {bot_commands}"
        );
    }

    /// Without a live agent the sync arm must not claim a file was written.
    /// The old behavior was an `Unknown` match, which sent the literal to
    /// the model as prose.
    #[test]
    fn export_sync_arm_refuses_to_claim_a_file_it_cannot_read() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-x");
        let resp = handle_command("export", "/tmp/x.md", &ctx).expect("a reply");
        assert!(
            resp.contains("gateway running"),
            "the sync arm must name the live command, got: {resp}"
        );
        assert!(
            !resp.contains("Exported"),
            "the sync arm must not claim a write, got: {resp}"
        );
    }
}

// ── RT-179 item 3.2: /memory's channel-twin contract ────────────────────
// Resolves as a command, reaches the platform's command list, and the sync
// arm refuses to quote an engine state it cannot see.
#[cfg(test)]
mod rt179_memory_tests {
    use super::*;

    /// The name must resolve and reach the platform's command list — a name
    /// that falls through `resolve_command` reaches the model as a literal,
    /// which is the gap this slice closes. Note the TUI's `/memory` shells
    /// out to the CLI, so this is the FIRST surface that answers the
    /// question in-process.
    #[test]
    fn memory_resolves_as_a_command_and_reaches_the_command_list() {
        for (text, name) in [("/memory", "memory"), ("/memory deploy notes", "memory")] {
            match resolve_command(text) {
                CommandMatch::Found(def, _) => {
                    assert_eq!(def.name, name, "{text} must resolve to {name}")
                }
                other => panic!("{text} must be a command, got {other:?}"),
            }
        }
        let bot_commands = telegram_bot_commands();
        assert!(
            bot_commands.contains("\"memory\""),
            "missing /memory: {bot_commands}"
        );
    }

    /// Without a live agent the sync arm must name the real surfaces rather
    /// than quoting an engine state it cannot read. The CLI pointer matters:
    /// it is the same question's other answer, so a stuck operator is never
    /// stranded.
    #[test]
    fn memory_sync_arm_names_the_live_command_and_the_cli_surface() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-m");
        let bare = handle_command("memory", "", &ctx).expect("a reply");
        assert!(
            bare.contains("gateway running") && bare.contains("operant memory stats"),
            "bare /memory must name both surfaces, got: {bare}"
        );
        let queried = handle_command("memory", "deploy notes", &ctx).expect("a reply");
        assert!(
            queried.contains("gateway running") && queried.contains("operant memory search"),
            "a query must name both surfaces, got: {queried}"
        );
        // Neither arm may claim an engine state.
        assert!(
            !bare.contains("reachable") && !queried.contains("reachable"),
            "the sync arm must not report engine health it cannot read"
        );
    }
}

// ── RT-179 item 3.3: /skill's channel-twin contract ────────────────────
// The named form must EXPAND a real skill, and the expansion must be the
// same message the TUI injects — one code path, one format. A /skill that
// replied "see the TUI" would be the theater this slice exists to close.
#[cfg(test)]
mod rt179_skill_tests {
    use super::*;

    /// A REAL skill on disk (not a stub), so the named form is proven to
    /// read the real `[skills]` root rather than a fixture baked into the
    /// binary. The invocation marker is what the agent actually follows, so
    /// its presence — not a substring of the description — is the assertion.
    #[test]
    fn skill_expands_a_real_installed_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_dir = dir.path().join("fizzbuzz");
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: fizzbuzz\ndescription: Count the fizzy numbers\n---\n\n# Count every number that is fizzy.\n",
        )
        .expect("SKILL.md");

        let mut cfg = AppConfig::default();
        cfg.skills.root_dir = dir.path().to_path_buf();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-s");

        let reply = handle_command("skill", "fizzbuzz count to 100", &ctx)
            .expect("a named /skill must expand a real skill");
        assert!(
            reply.contains("Count every number that is fizzy"),
            "the expansion must carry the skill's body, got: {reply}"
        );
        assert!(
            reply.contains("has invoked the \"fizzbuzz\" skill"),
            "the expansion must be the invocation message the agent follows, got: {reply}"
        );
        // The trailing instruction is the TUI's own contract — it must be
        // folded in, not dropped.
        assert!(
            reply.contains("count to 100"),
            "the trailing instruction must ride the expansion, got: {reply}"
        );
    }

    /// An unknown name must say so and point at the listing — never reply
    /// with a success-shaped message that expands nothing.
    #[test]
    fn skill_unknown_name_says_not_found_and_names_the_listing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = AppConfig::default();
        cfg.skills.root_dir = dir.path().to_path_buf();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-s");

        let reply = handle_command("skill", "no-such-skill", &ctx).expect("a reply");
        assert!(
            reply.contains("not found") && reply.contains("/skill"),
            "an unknown name must say not found and name the listing, got: {reply}"
        );
        assert!(
            !reply.contains("has invoked"),
            "a nonexistent skill must not produce an invocation message, got: {reply}"
        );
    }

    /// The command must resolve (so the dispatch loop stops feeding the
    /// literal to the model) and reach the platform's command list.
    ///
    /// ONE command, not two: `skill` is `skills`' alias, and the registry's
    /// uniqueness guard (RT-178) refuses a name that collides with an
    /// existing alias. So `/skill` resolves to `skills` and carries the
    /// BOTH-forms contract — no args lists, a name expands — which is also
    /// why the reply names `/skill` for the invoke form while the platform's
    /// command list carries `skills`.
    #[test]
    fn skill_resolves_and_reaches_the_command_list() {
        for (text, expected) in [
            ("/skills", "skills"),
            ("/skill", "skills"),
            ("/skill fizzbuzz count", "skills"),
        ] {
            assert!(
                matches!(resolve_command(text), CommandMatch::Found(d, _) if d.name == expected),
                "{text} must resolve to the `{expected}` command (its alias), not prose"
            );
        }
        let bot_commands = telegram_bot_commands();
        assert!(
            bot_commands.contains("\"skills\""),
            "missing /skills: {bot_commands}"
        );
    }
}

// ── RT-179 item 3.5: /search's channel-twin contract ───────────────────
#[cfg(test)]
mod rt179_search_tests {
    use super::*;

    /// Both names must resolve and reach the platform's command list. `/find`
    /// is the alias the TUI itself carries, so the channel honours it rather
    /// than making the operator learn a second spelling.
    #[test]
    fn search_resolves_under_both_names_and_reaches_the_command_list() {
        for (text, expected) in [
            ("/search", "search"),
            ("/search deploy notes", "search"),
            ("/find", "search"),
        ] {
            assert!(
                matches!(resolve_command(text), CommandMatch::Found(d, _) if d.name == expected),
                "{text} must resolve to `{expected}`, not prose"
            );
        }
        let bot_commands = telegram_bot_commands();
        assert!(
            bot_commands.contains("\"search\""),
            "missing /search: {bot_commands}"
        );
    }

    /// Without a live agent the sync arm cannot search — the tool needs the
    /// agent's database. It must name the live command rather than report an
    /// empty result that reads as "nothing anywhere", AND it must not point
    /// at a CLI search that does not exist: `operant sessions list` takes no
    /// query flag (verified against the deployed binary), so the honest
    /// pointer is the gateway command alone.
    #[test]
    fn search_sync_arm_names_the_live_command_and_refuses_to_invent_a_cli_search() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "op-1", "telegram", "chan-q");
        for (args, label) in [("", "bare"), ("deploy notes", "queried")] {
            let reply = handle_command("search", args, &ctx).expect("a reply");
            assert!(
                reply.contains("gateway running"),
                "{label} /search must name the live command, got: {reply}"
            );
            assert!(
                !reply.contains("operant sessions"),
                "{label} /search must not point at a CLI search that does not exist (no --query on `sessions list`), got: {reply}"
            );
            assert!(
                !reply.contains("No sessions"),
                "{label} /search must not report a result it never computed, got: {reply}"
            );
        }
    }
}

/// `/model` role resolution (plan 2026-10-30). The gateway arm must resolve
/// through the SAME `operant_core::model_roles` seam the TUI picker calls, so
/// a role name typed on Telegram and Enter on a picker row cannot drift.
#[cfg(test)]
mod model_role_tests {
    use super::*;

    fn ctx(cfg: &AppConfig) -> CommandContext<'_> {
        CommandContext::new(None, cfg, true, "op-1", "telegram", "chan-1")
    }

    #[test]
    fn split_model_args_separates_the_provider_flag() {
        // The flag was advertised in args_hint for the whole life of the
        // command and never parsed — this is the parse it promised.
        assert_eq!(split_model_args("gpt-4o"), ("gpt-4o", None));
        assert_eq!(
            split_model_args("gpt-4o --provider openai"),
            ("gpt-4o", Some("openai"))
        );
        assert_eq!(split_model_args("--provider openai"), ("", Some("openai")));
        assert_eq!(split_model_args("--provider=openai"), ("", Some("openai")));
        assert_eq!(
            split_model_args("gpt-4o --provider=openai"),
            ("gpt-4o", Some("openai"))
        );
        // A model id keeps its internal spaces; only the flag is removed.
        assert_eq!(
            split_model_args("  gpt 4o --provider openai  "),
            ("gpt 4o", Some("openai"))
        );
    }

    #[test]
    fn a_bare_model_command_lists_the_configured_slots() {
        let cfg = AppConfig::default();
        let reply = handle_command("model", "", &ctx(&cfg)).expect("a reply");
        // The catalogue names the roles, not just the current model — the
        // old reply printed one line and left the operator to guess ids.
        assert!(
            reply.contains("default") && reply.contains("Model roles"),
            "the catalogue must name the slots: {reply}"
        );
        assert!(reply.contains("Usage:"), "{reply}");
    }

    /// The one case the resolver still refuses: a ROLE NAME with no slot set.
    /// `/model vision` on a config with no `[auxiliary_models.vision]` must not
    /// silently switch the conversation to the literal word `vision`.
    #[test]
    fn a_known_role_with_an_empty_slot_is_refused_not_hijacked() {
        let cfg = AppConfig::default();
        let reply = handle_command("model", "vision", &ctx(&cfg)).expect("a reply");
        assert!(
            reply.contains("not set in this config"),
            "an unset slot must be refused with a reason: {reply}"
        );
        assert!(
            !reply.contains("Model switch requested"),
            "a refused role must never confirm a switch: {reply}"
        );
        assert!(
            reply.contains("vision tool"),
            "the reply must name the consumer so the operator knows what to set: {reply}"
        );
    }

    /// An unrecognised word is NOT refused — it is the command's main use, a
    /// direct id the operator wants. Guarded here so a future resolver change
    /// that starts rejecting unknown words cannot silently break
    /// `/model gpt-4o`.
    #[test]
    fn an_unknown_word_is_treated_as_a_direct_model_id() {
        let cfg = AppConfig::default();
        let reply = handle_command("model", "some-new-model", &ctx(&cfg)).expect("a reply");
        assert!(
            reply.contains("Model switch requested"),
            "a direct id must confirm the switch: {reply}"
        );
    }

    #[test]
    fn a_resolved_non_conversation_slot_is_announced_as_an_assignment() {
        let mut cfg = AppConfig::default();
        cfg.agent.model = "conversation-model".into();
        cfg.auxiliary_models.vision = Some(operant_core::config::AuxiliaryModelConfig {
            model: Some("vision-model".to_string()),
            ..Default::default()
        });
        let reply = handle_command("model", "vision", &ctx(&cfg)).expect("a reply");
        assert!(
            reply.contains("`vision` slot set to"),
            "a non-conversation slot must be described as an assignment: {reply}"
        );
        // oh-my-pi's rule made explicit: the conversation did NOT move.
        assert!(
            reply.contains("conversation keeps running on"),
            "the reply must say the conversation is unchanged: {reply}"
        );
        assert!(!reply.contains("Model switch requested"), "{reply}");
    }

    #[test]
    fn the_omp_smol_alias_reaches_operants_recap_slot() {
        let mut cfg = AppConfig::default();
        cfg.agent.model = "conversation-model".into();
        cfg.recap.model = operant_core::config::AuxiliaryModelConfig {
            model: Some("cheap-model".to_string()),
            ..Default::default()
        };
        let reply = handle_command("model", "smol", &ctx(&cfg)).expect("a reply");
        assert!(reply.contains("`smol` slot set to"), "{reply}");
        assert!(reply.contains("cheap-model"), "{reply}");
    }

    #[test]
    fn a_direct_model_id_still_switches_the_conversation() {
        let cfg = AppConfig::default();
        let reply = handle_command("model", "gpt-4o", &ctx(&cfg)).expect("a reply");
        // An exact id is not a role name; it resolves to the conversation slot.
        assert!(
            reply.contains("Model switch requested"),
            "a direct id must confirm the switch: {reply}"
        );
        assert!(reply.contains("gpt-4o"), "{reply}");
    }

    #[test]
    fn a_provider_flag_qualifies_an_unqualified_model() {
        let cfg = AppConfig::default();
        let reply =
            handle_command("model", "my-model --provider openai", &ctx(&cfg)).expect("a reply");
        assert!(
            reply.contains("openai/my-model"),
            "the provider flag must qualify the id: {reply}"
        );
    }
}
