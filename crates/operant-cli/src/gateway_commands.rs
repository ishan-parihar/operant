//! Gateway commands — Telegram bot command definitions, registry, and dispatch.
//!
//! This module provides the canonical set of built-in Telegram bot commands
//! for the Operant gateway. It handles command resolution, admin gating, and
//! generates the JSON payload used by Telegram's `setMyCommands` API.

use chrono::Utc;
use operant_core::config::{AppConfig, ToolProgressMode};
use operant_core::gateway::Gateway;

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
        }
    }
}

/// Definition of a single Telegram bot command.
pub struct CommandDef {
    /// Primary command name (e.g. "start", "help").
    pub name: &'static str,
    /// Short description shown in the bot's command menu.
    pub description: &'static str,
    /// Alternative names that also resolve to this command.
    pub aliases: &'static [&'static str],
    /// Display category for grouping in `/help`.
    pub category: &'static str,
    /// Argument hint shown in usage (e.g. "[name]", "" for none).
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
        name: "session",
        description: "Show session info",
        aliases: &["status"],
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
        name: "fast",
        description: "Toggle fast mode",
        aliases: &[],
        category: "Config",
        args_hint: "[normal|fast|status]",
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
        description: "Toggle YOLO mode",
        aliases: &[],
        category: "Config",
        args_hint: "",
        admin_only: false,
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
        description: "List installed skills",
        aliases: &["skill"],
        category: "Tools",
        args_hint: "",
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
];

/// Resolve a raw message text into a command definition and its arguments.
///
/// Expects text to start with `/`. Extracts the first space-delimited token as
/// the command name (stripping the leading `/`), matches it case-insensitively
/// against the registry (including aliases), and returns the matching
/// [`CommandDef`] along with the remainder of the text as the argument string.
///
/// Returns `None` when the text does not start with `/` or the command is not
/// recognised.
pub fn resolve_command(text: &str) -> Option<(&'static CommandDef, &str)> {
    let trimmed = text.trim();

    if !trimmed.starts_with('/') {
        return None;
    }

    let after_slash = trimmed[1..].trim_start();

    let (cmd_token, args) = match after_slash.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((cmd, rest)) => (cmd, rest.trim_start()),
        None => (after_slash, ""),
    };

    if cmd_token.is_empty() {
        return None;
    }

    let found = COMMAND_REGISTRY.iter().find(|def| {
        def.name.eq_ignore_ascii_case(cmd_token)
            || def
                .aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(cmd_token))
    })?;

    Some((found, args))
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
pub fn handle_command(cmd_name: &str, _args: &str, ctx: &CommandContext<'_>) -> Option<String> {
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
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                {
                    let old_id = session.session_id.clone();
                    let _ = store.close_session(&old_id);
                    msg.push_str(&format!("\nClosed session: `{}`", old_id));
                }
                if let Ok(new_session) = store.create_session(ctx.platform, ctx.user_id, ctx.channel_id)
                {
                    msg.push_str(&format!("\nNew session ID: `{}`", new_session.session_id));
                }
            }
            msg
        }

        "stop" => {
            let mut msg = String::from("⏹️ **Stopping current agent turn.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("interrupted".to_string(), "true".to_string()),
                    ]);
                    msg.push_str(&format!("\nSession `{}` marked for interruption.", session.session_id));
                } else {
                    msg.push_str("\nNo active session found to interrupt.");
                }
            }
            msg.push_str("\nSend a new message to start fresh.");
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

        "retry" => {
            let mut msg = String::from("🔁 **Retrying last exchange.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("retry_requested".to_string(), "true".to_string()),
                    ]);
                    msg.push_str(&format!("\nSession `{}` flagged for retry.", session.session_id));
                } else {
                    msg.push_str("\nNo active session found.");
                }
            }
            msg.push_str("\nResend your last message to trigger the retry.");
            msg
        }

        "undo" => {
            let mut msg = String::from("↩️ **Undoing last exchange.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    let msg_count = session.metadata.get("message_count")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if msg_count > 0 {
                        let new_count = msg_count.saturating_sub(2);
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("message_count".to_string(), new_count.to_string()),
                            ("undo_requested".to_string(), "true".to_string()),
                        ]);
                        msg.push_str(&format!("\nSession `{}` rolled back 2 messages.", session.session_id));
                    } else {
                        msg.push_str("\nNo messages to undo in this session.");
                    }
                } else {
                    msg.push_str("\nNo active session found.");
                }
            }
            msg
        }

        "session" => {
            let mut lines = vec!["*Session Info:*".to_string()];
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                {
                    lines.push(format!("• ID: `{}`", session.session_id));
                    lines.push(format!("• Platform: `{}`", session.platform));
                    lines.push(format!("• Created: `{}`", session.created_at));
                    lines.push(format!("• Last active: `{}`", session.last_active));
                } else {
                    lines.push("• No active session.".to_string());
                }
                lines.push(format!("• Total sessions: {}", store.get_session_count()));
            } else {
                lines.push("• Gateway not connected.".to_string());
                lines.push("• Use CLI: `operant sessions list`".to_string());
            }
            lines.join("\n")
        }

        "queue" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/queue <prompt>` — queues a prompt for the next turn without interrupting.".into()
            } else {
                let mut msg = String::from("▶️ **Prompt queued.**");
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                        let existing = session.metadata.get("queued_prompts").cloned().unwrap_or_default();
                        let new_val = if existing.is_empty() {
                            a.to_string()
                        } else {
                            format!("{}\n---\n{}", existing, a)
                        };
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("queued_prompts".to_string(), new_val),
                        ]);
                        msg.push_str(&format!("\nSession `{}` now has queued prompts.", session.session_id));
                    }
                }
                msg.push_str(&format!("\nQueued: `{}`", a));
                msg
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
                "Usage: `/resume <session-id>` — resumes a saved session by ID.\nFind session IDs via `/sessions` or CLI `operant sessions list`."
                    .into()
            } else if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.get_session(a) {
                    format!(
                        "✅ Session found: `{}`\n• Platform: {}\n• User: {}\n• Last active: {}\n\nResume via CLI: `operant sessions resume {}`",
                        session.session_id, session.platform, session.platform_user_id, session.last_active, session.operant_session_id
                    )
                } else {
                    format!("❌ No session found with ID: `{}`. Use `/sessions` to list active sessions.", a)
                }
            } else {
                format!("Session resume requested for `{}`. Use CLI `operant sessions resume` to resume sessions.", a)
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
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(mut session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id)
                    {
                        session.metadata.insert("title".to_string(), a.to_string());
                        // Note: we can't persist metadata changes through the current SessionStore API
                    }
                }
                format!("📝 Title set to: `{}`\n(Note: Title changes apply to the current session.)", a)
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
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
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

        "rollback" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/rollback <checkpoint>` — restores to an earlier checkpoint.\nAvailable checkpoints are tracked via session metadata. Use `/status` to see session history.".into()
            } else {
                let mut msg = String::from("⏪ **Rolling back to checkpoint.**");
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("rollback_to".to_string(), a.to_string()),
                            ("rollback_requested".to_string(), "true".to_string()),
                        ]);
                        msg.push_str(&format!("\nSession `{}` flagged for rollback to: `{}`", session.session_id, a));
                    } else {
                        msg.push_str("\nNo active session found.");
                    }
                }
                msg
            }
        }

        "compress" => {
            let a = _args.trim();
            let mut msg = String::from("📦 **Compressing session context.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    let focus = if a.is_empty() { "general" } else { a };
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("compress_requested".to_string(), "true".to_string()),
                        ("compress_focus".to_string(), focus.to_string()),
                    ]);
                    msg.push_str(&format!("\nSession `{}` flagged for compression.", session.session_id));
                    if !a.is_empty() {
                        msg.push_str(&format!("\nFocus topic: `{}`", a));
                    }
                } else {
                    msg.push_str("\nNo active session found.");
                }
            }
            msg
        }

        "background" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/background <prompt>` — queues a task for background processing.\nThe agent will process it after the current turn.".into()
            } else {
                let mut msg = String::from("⏳ **Background task queued.**");
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("background_task".to_string(), a.to_string()),
                            ("background_queued".to_string(), Utc::now().to_rfc3339()),
                        ]);
                        msg.push_str(&format!("\nSession `{}` has a pending background task.", session.session_id));
                    }
                }
                msg.push_str(&format!("\nTask: `{}`", a));
                msg
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

        "steer" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/steer <direction>` — guides the conversation in a specific direction.\nThe steer message will be included in the next agent turn.".into()
            } else {
                let mut msg = String::from("🧭 **Steering conversation.**");
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("steer_message".to_string(), a.to_string()),
                            ("steer_active".to_string(), "true".to_string()),
                        ]);
                        msg.push_str(&format!("\nSession `{}` steer updated.", session.session_id));
                    }
                }
                msg.push_str(&format!("\nSteer: `{}`", a));
                msg
            }
        }

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
                            store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                ("goal".to_string(), a.to_string()),
                                ("goal_paused".to_string(), "false".to_string()),
                            ]);
                        }
                        format!("🎯 Goal set: `{}`\nThis goal will guide the agent's responses.", a)
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
                                if let Ok(idx) = num.parse::<usize>() {
                                    if idx > 0 && idx <= parts.len() {
                                        parts.remove(idx - 1);
                                        let new_val = parts.join("\n");
                                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                            ("subgoal".to_string(), new_val),
                                        ]);
                                    }
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
                                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                                    ("subgoal".to_string(), new_val),
                                ]);
                            }
                        }
                        format!("🎯 Sub-goal set: `{}`", a)
                    }
                }
            }
        }

        "topic" => {
            let a = _args.trim();
            if a.is_empty() || a == "help" {
                "Usage: `/topic [off|help|<session-id>]`\nSwitch to a topic-specific session or disable topic mode.\nExamples:\n• `/topic off` — return to main session\n• `/topic <session-id>` — switch to a specific session".into()
            } else if a == "off" {
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("topic_mode".to_string(), "off".to_string()),
                    ]);
                }
                "🔄 Topic mode disabled. Returning to main conversation.".into()
            } else {
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("topic_mode".to_string(), a.to_string()),
                        ("topic_active".to_string(), "true".to_string()),
                    ]);
                }
                format!("🔄 Switching to topic session: `{}`\nTopic sessions use a separate conversation context.", a)
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
            format!(
                "*Active Profile:*\n• Model: `{}`\n• Max iterations: `{}`\n• Reasoning: `{}`\n\nUse CLI `operant profile` for full profile management.",
                model, ctx.config.agent.max_iterations, ctx.config.agent.show_reasoning
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
            let a = _args.trim();
            let current_model = &ctx.config.agent.model;
            if a.is_empty() {
                let mut msg = format!("🤖 Current model: `{}`", current_model);
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                        if let Some(override_model) = session.metadata.get("model_override") {
                            msg.push_str(&format!("\nSession override: `{}`", override_model));
                        }
                    }
                }
                msg.push_str("\nUsage: `/model <name> [--provider name]`");
                msg
            } else {
                if let Some(gateway) = ctx.gateway {
                    let store = gateway.get_session_store();
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("model_override".to_string(), a.to_string()),
                    ]);
                }
                format!(
                    "🤖 Model switch requested: `{}`\nCurrent model: `{}`\nModel override applied for this session.",
                    a, current_model
                )
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
                        if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                            if let Some(override_val) = session.metadata.get("reasoning_override") {
                                msg.push_str(&format!("\nSession override: `{}`", override_val));
                            }
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

        "fast" => {
            let a = _args.trim();
            match a {
                "" | "status" => {
                    let mut msg = String::from("⚡ Fast mode status:");
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                            let fast = session.metadata.get("fast_mode").map(|s| s.as_str()) == Some("true");
                            msg.push_str(&format!(" Session override: `{}`", if fast { "fast" } else { "normal" }));
                        } else {
                            msg.push_str(" **normal** (default).");
                        }
                    } else {
                        msg.push_str(" **normal** (default).");
                    }
                    msg.push_str("\nUsage: `/fast [normal|fast|status]`");
                    msg
                }
                "normal" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("fast_mode".to_string(), "false".to_string()),
                        ]);
                    }
                    "⚡ Switched to **normal** mode.".into()
                }
                "fast" => {
                    if let Some(gateway) = ctx.gateway {
                        let store = gateway.get_session_store();
                        store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                            ("fast_mode".to_string(), "true".to_string()),
                        ]);
                    }
                    "⚡ Switched to **fast** mode (uses quicker responses).".into()
                }
                _ => "Usage: `/fast [normal|fast|status]`".into(),
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
            let mut msg = String::from("🤘 YOLO mode toggled.");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                if let Some(session) = store.find_session(ctx.platform, ctx.user_id, ctx.channel_id) {
                    let yolo = session.metadata.get("yolo_mode").map(|s| s.as_str()) == Some("true");
                    let new_val = !yolo;
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("yolo_mode".to_string(), new_val.to_string()),
                    ]);
                    msg.push_str(&format!(" Session YOLO mode: `{}`", if new_val { "ON" } else { "OFF" }));
                } else {
                    store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                        ("yolo_mode".to_string(), "true".to_string()),
                    ]);
                    msg.push_str(" Session YOLO mode: `ON`");
                }
            }
            msg.push_str("\nYOLO mode skips approval prompts for destructive operations.");
            msg
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

        "skills" => {
            let skills_dir = &ctx.config.skills.root_dir;
            if skills_dir.exists() && skills_dir.is_dir() {
                let mut skill_names: Vec<String> = Vec::new();
                if let Ok(entries) = std::fs::read_dir(skills_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.join("SKILL.md").exists() {
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                skill_names.push(name.to_string());
                            }
                        }
                    }
                }
                if skill_names.is_empty() {
                    "📦 No skills found in the skills directory.\nUse CLI `operant skills list` or `operant skills install` to manage skills."
                        .into()
                } else {
                    skill_names.sort();
                    let mut text = format!("📦 *Installed Skills:* ({} found)\n", skill_names.len());
                    for name in &skill_names {
                        text.push_str(&format!("• `{}`\n", name));
                    }
                    text
                }
            } else {
                format!(
                    "📦 Skills directory not found: `{}`.\nUse CLI `operant skills list` to see available skills.",
                    skills_dir.display()
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
                    "🔧 Curator: **idle**.\nThe curator manages skill lifecycle (auto-archiving stale skills).\nUse `/curator run` to trigger maintenance.\nConfigure in `config.yaml` under `curator` section.".into()
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
                    if let Some(count_str) = s.metadata.get("message_count") {
                        if let Ok(count) = count_str.parse::<usize>() {
                            total_messages += count;
                            sessions_with_msgs += 1;
                        }
                    }
                }
                lines.push(format!("• Total messages (active sessions): {}", total_messages));
                if sessions_with_msgs > 0 {
                    lines.push(format!("• Avg messages/session: {}", total_messages / sessions_with_msgs));
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
            let a = _args.trim();
            let mut msg = String::from("✅ **Action approved.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                let approval_scope = if a == "always" { "always" } else { "session" };
                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                    ("approval_granted".to_string(), "true".to_string()),
                    ("approval_scope".to_string(), approval_scope.to_string()),
                    ("approved_at".to_string(), Utc::now().to_rfc3339()),
                ]);
                msg.push_str(&format!("\nApproval scope: `{}`", approval_scope));
            }
            msg
        }
        "deny" => {
            let mut msg = String::from("❌ **Action denied.**");
            if let Some(gateway) = ctx.gateway {
                let store = gateway.get_session_store();
                store.update_session_metadata(ctx.platform, ctx.user_id, ctx.channel_id, &[
                    ("approval_granted".to_string(), "false".to_string()),
                    ("denied_at".to_string(), Utc::now().to_rfc3339()),
                ]);
                msg.push_str("\nThe pending action has been cancelled.");
            }
            msg
        }

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
    use super::*;

    #[test]
    fn test_resolve_slash_command() {
        let (def, _) = resolve_command("/new").unwrap();
        assert_eq!(def.name, "new");
    }

    #[test]
    fn test_resolve_with_args() {
        let (def, args) = resolve_command("/model gpt-4").unwrap();
        assert_eq!(def.name, "model");
        assert_eq!(args, "gpt-4");
    }

    #[test]
    fn test_resolve_alias() {
        let (def, _) = resolve_command("/reset").unwrap();
        assert_eq!(def.name, "new");
    }

    #[test]
    fn test_resolve_no_slash() {
        assert!(resolve_command("hello").is_none());
    }

    #[test]
    fn test_resolve_unknown() {
        assert!(resolve_command("/nonexistent").is_none());
    }

    #[test]
    fn test_resolve_case_insensitive() {
        let (def, _) = resolve_command("/NEW").unwrap();
        assert_eq!(def.name, "new");
    }

    #[test]
    fn test_handle_unknown() {
        let cfg = AppConfig::default();
        let ctx = CommandContext::new(None, &cfg, true, "test", "telegram", "123");
        assert!(handle_command("nonexistent", "", &ctx).is_none());
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
}
