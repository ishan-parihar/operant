//! Gateway commands — Telegram bot command definitions, registry, and dispatch.
//!
//! This module provides the canonical set of built-in Telegram bot commands
//! for the Hermes gateway. It handles command resolution, admin gating, and
//! generates the JSON payload used by Telegram's `setMyCommands` API.

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
/// `is_admin` is `false`, this function informs the user that the command
/// requires admin privileges instead of silently ignoring.
pub fn handle_command(cmd_name: &str, _args: &str, is_admin: bool) -> Option<String> {
    let def = COMMAND_REGISTRY.iter().find(|d| {
        d.name.eq_ignore_ascii_case(cmd_name)
            || d.aliases.iter().any(|a| a.eq_ignore_ascii_case(cmd_name))
    })?;

    if def.admin_only && !is_admin {
        return Some("This command is only available to admins.".to_string());
    }

    Some(match def.name {
        "start" => "Hello! I'm Hermes AI. Send me a message!".into(),

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

        "new" => "Starting a new session. Previous conversation cleared.".into(),
        "stop" => "Stopping current task.".into(),
        "status" => "Current session is active.".into(),
        "retry" => "Retrying last request...".into(),
        "undo" => "Last action undone.".into(),
        "session" => "Current session info is available via `hermes sessions list` in CLI.".into(),
        "resume" => {
            "Session resume is not yet supported via chat. Use `hermes sessions list` in CLI."
                .into()
        }
        "title" => "Session title changes are not yet supported via chat.".into(),
        "branch" => "Session branching is not yet supported via chat.".into(),
        "rollback" => "Session rollback is not yet supported via chat.".into(),
        "compress" => "Session compression is not yet supported via chat.".into(),
        "background" => "Background tasks are not yet supported via chat.".into(),
        "agents" => "Active agent management is not yet available via chat.".into(),
        "steer" => "Steering is handled automatically during conversation.".into(),
        "goal" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/goal [text|pause|resume|clear|status]`".into()
            } else {
                format!("Goal command received: `{}`. Goal management is not yet fully supported via chat.", a)
            }
        }
        "subgoal" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/subgoal [text|remove N|clear]`".into()
            } else {
                format!(
                    "Subgoal received: `{}`. Subgoal management is not yet supported via chat.",
                    a
                )
            }
        }
        "topic" => {
            let a = _args.trim();
            if a.is_empty() || a == "help" {
                "Usage: `/topic [off|help|<session-id>]`".into()
            } else {
                format!(
                    "Switching to topic session: `{}`. Not yet supported via chat.",
                    a
                )
            }
        }
        "whoami" => "You are the authenticated Hermes user.".into(),
        "profile" => "Profile information is available via `hermes profile list` in CLI.".into(),
        "sethome" => "Setting a home session is not yet supported via chat.".into(),

        "config" => {
            "Config commands are not available via chat. Use `hermes config` in CLI.".into()
        }
        "model" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/model [name] [--provider name]`. Model switching via chat is not yet supported.".into()
            } else {
                format!("Model switch to `{}` received. Model switching via chat is not yet implemented.", a)
            }
        }
        "reasoning" => {
            let a = _args.trim();
            match a {
                "" | "status" => "Reasoning display is currently enabled.".into(),
                "show" | "on" => "Reasoning display enabled.".into(),
                "hide" | "off" => "Reasoning display disabled.".into(),
                _ => "Usage: `/reasoning [show|hide|status]`".into(),
            }
        }
        "fast" => {
            let a = _args.trim();
            match a {
                "" | "status" => "Fast mode is currently disabled.".into(),
                "normal" => "Switched to normal mode.".into(),
                "fast" => "Switched to fast mode.".into(),
                _ => "Usage: `/fast [normal|fast|status]`".into(),
            }
        }
        "footer" => {
            let a = _args.trim();
            match a {
                "" | "status" => "Message footer is currently enabled.".into(),
                "on" => "Message footer enabled.".into(),
                "off" => "Message footer disabled.".into(),
                _ => "Usage: `/footer [on|off|status]`".into(),
            }
        }
        "yolo" => "YOLO mode toggled. Not yet fully supported via chat.".into(),
        "voice" => {
            let a = _args.trim();
            match a {
                "" | "status" => "Voice output is currently disabled.".into(),
                "on" => "Voice output enabled.".into(),
                "off" => "Voice output disabled.".into(),
                _ => "Usage: `/voice [on|off|status]`".into(),
            }
        }
        "personality" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage: `/personality [name]`. Personality switching is not yet supported via chat."
                    .into()
            } else {
                format!(
                    "Personality `{}` selected. Not yet fully supported via chat.",
                    a
                )
            }
        }

        "reload-mcp" => "MCP server reload requested. Not yet supported via chat.".into(),
        "reload-skills" => "Skill registry reload requested. Not yet supported via chat.".into(),
        "kanban" => {
            let a = _args.trim();
            if a.is_empty() {
                "Kanban boards are available via `hermes kanban list` in CLI.".into()
            } else {
                format!(
                    "Kanban `{}` command received. Not yet supported via chat.",
                    a
                )
            }
        }

        "usage" => {
            let a = _args.trim();
            if a.is_empty() {
                "Usage statistics are available via `hermes insights sessions` in CLI.".into()
            } else {
                format!(
                    "Usage for the last `{}` days is not yet available via chat.",
                    a
                )
            }
        }
        "insights" => {
            let a = _args.trim();
            if a.is_empty() {
                "Session insights are available via `hermes insights sessions` in CLI.".into()
            } else {
                format!(
                    "Insights for the last `{}` days is not yet available via chat.",
                    a
                )
            }
        }
        "debug" => "Debug info is available via `hermes debug share` in CLI.".into(),
        "update" => {
            "Hermes is at version 0.1.3. Check for updates via `hermes update check` in CLI.".into()
        }

        "approve" => "Action approved.".into(),
        "deny" => "Action denied.".into(),

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
        assert!(handle_command("nonexistent", "", true).is_none());
    }

    #[test]
    fn test_admin_gate() {
        assert!(handle_command("approve", "", false).is_some());
        let resp = handle_command("approve", "", false).unwrap();
        assert!(resp.contains("admin"));
    }

    #[test]
    fn test_admin_allowed() {
        let resp = handle_command("approve", "", true).unwrap();
        assert_eq!(resp, "Action approved.");
    }

    #[test]
    fn test_help_contains_categories() {
        let resp = handle_command("help", "", false).unwrap();
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
