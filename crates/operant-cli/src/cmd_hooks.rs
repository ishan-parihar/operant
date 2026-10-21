//! CLI subcommand for managing shell hooks (`operant hooks`).
//!
//! A shell hook is the operator's own command, run when the agent emits a
//! lifecycle event (`agent_start`, `agent_end`, `turn_start`, `turn_end`).
//! The hooks live in `~/.operant/data/hooks/<name>.hook`; the agent loads
//! them when it starts, so a hook written here fires on the next run.
//!
//! Before iter-1125 this family read files that nothing wrote and nothing
//! loaded. Now the same store is the one the runtime reads.

use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::gateway_pipeline::HookContext;
use operant_core::shell_hooks::{
    HOOK_EVENT_NAMES, hooks_dir, load_shell_hooks, remove_shell_hook, run_hook_command,
    write_shell_hook,
};

#[derive(Debug, Clone, Subcommand)]
pub enum HooksSubcommand {
    /// List the configured hooks
    List,
    /// Add a hook: a command to run when an event fires
    Add {
        /// The event that fires the hook (agent_start, agent_end, turn_start, turn_end)
        #[arg(long)]
        event: String,
        /// The hook name (defaults to the command's first word)
        #[arg(long)]
        name: Option<String>,
        /// The command to run (quote it)
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
    /// Run a hook's command now, without waiting for its event
    Test {
        /// Hook name to test
        name: String,
    },
    /// Revoke (remove) a hook by name
    Revoke {
        /// Hook name to revoke
        name: String,
        #[arg(long, hide = true)]
        remove: bool,
    },
    /// Run hook diagnostics
    Doctor,
}

pub async fn handle_hooks_command(_config: &AppConfig, cmd: HooksSubcommand) -> Result<()> {
    match cmd {
        HooksSubcommand::List => cmd_list(),
        HooksSubcommand::Add {
            event,
            name,
            command,
        } => cmd_add(event, name, command),
        HooksSubcommand::Test { name } => cmd_test(&name).await,
        HooksSubcommand::Revoke { name, .. } => cmd_revoke(&name),
        HooksSubcommand::Doctor => cmd_doctor(),
    }
}

/// List the hooks and what fires them.
fn cmd_list() -> Result<()> {
    let dir = hooks_dir();
    let (hooks, problems) = load_shell_hooks(&dir);
    if hooks.is_empty() && problems.is_empty() {
        println!("No hooks configured.");
        println!("Add one with: operant hooks add --event agent_end -- <command>");
        return Ok(());
    }

    println!("Configured Hooks");
    println!("───────────────");
    println!();
    for (name, hook) in &hooks {
        println!("  {name}");
        println!("    event:   {}", hook.event);
        println!("    command: {}", hook.command);
    }
    if !problems.is_empty() {
        println!();
        println!("Unusable hook files:");
        for problem in &problems {
            println!("  {} — {}", problem.file, problem.reason);
        }
    }
    Ok(())
}

/// Write a new hook file.
fn cmd_add(event: String, name: Option<String>, command: Vec<String>) -> Result<()> {
    let command = command.join(" ");
    let name = name.unwrap_or_else(|| {
        command
            .split_whitespace()
            .next()
            .unwrap_or("hook")
            .to_string()
    });
    let dir = hooks_dir();
    write_shell_hook(&dir, &name, &event, &command).map_err(anyhow::Error::msg)?;
    println!("Hook '{name}' added — fires on {event}.");
    println!("  {}", dir.join(format!("{name}.hook")).display());
    Ok(())
}

/// Run a hook's command now and report what happened.
async fn cmd_test(name: &str) -> Result<()> {
    let dir = hooks_dir();
    let (hooks, problems) = load_shell_hooks(&dir);
    let Some((_, hook)) = hooks.iter().find(|(hook_name, _)| hook_name == name) else {
        println!("Hook '{name}' not found.");
        if let Some(problem) = problems.iter().find(|p| p.file == name) {
            println!("  {} — {}", problem.file, problem.reason);
        }
        println!("Use `operant hooks list` to see the configured hooks.");
        return Ok(());
    };
    println!("Running hook '{name}' (event {}):", hook.event);
    println!("  {}", hook.command);
    let (status, output) = run_hook_command(&hook.command, &HookContext::new()).await;
    if !output.trim().is_empty() {
        println!();
        println!("{}", output.trim_end());
    }
    match status {
        Some(0) => println!("Hook '{name}' exited 0."),
        Some(code) => println!("Hook '{name}' exited {code}."),
        None => println!("Hook '{name}' did not run."),
    }
    Ok(())
}

/// Remove a hook file.
fn cmd_revoke(name: &str) -> Result<()> {
    let dir = hooks_dir();
    match remove_shell_hook(&dir, name) {
        Ok(true) => println!("Hook '{name}' revoked."),
        Ok(false) => println!("Hook '{name}' not found."),
        Err(err) => println!("Could not revoke hook '{name}': {err}"),
    }
    Ok(())
}

/// Report the health of the hook store.
fn cmd_doctor() -> Result<()> {
    let dir = hooks_dir();
    let (hooks, problems) = load_shell_hooks(&dir);
    println!("Hook System Health");
    println!("──────────────────");
    println!();
    println!("Hooks directory: {}", dir.display());
    println!("Hooks loaded:    {}", hooks.len());
    println!("Events:          {}", HOOK_EVENT_NAMES.join(", "));
    println!();
    if hooks.is_empty() && problems.is_empty() {
        println!("No hooks configured.");
        return Ok(());
    }
    for (name, hook) in &hooks {
        println!("  ok    {name} — fires on {}", hook.event);
    }
    for problem in &problems {
        println!("  BROKEN {} — {}", problem.file, problem.reason);
    }
    println!();
    println!("Hooks run when the agent emits their event — the agent must be");
    println!("started after a hook is added for it to fire.");
    Ok(())
}
