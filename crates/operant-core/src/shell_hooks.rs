//! Shell hooks — the operator's own commands, run on the agent's lifecycle
//! events.
//!
//! A hook is one file in `~/.operant/data/hooks/`, named `<name>.hook`, with
//! a JSON body:
//!
//! ```json
//! { "event": "agent_end", "command": "notify-send done" }
//! ```
//!
//! Before iter-1125 the `operant hooks` CLI read files in that directory
//! while NOTHING wrote them and NOTHING loaded them: the family was a
//! façade over a store with no reader and no writer. Now the runtime loads
//! the files when it builds the agent and runs the command when the event
//! fires.
//!
//! Only the events the agent loop actually emits are accepted —
//! `agent_start`, `agent_end`, `turn_start`, `turn_end`. A hook on an event
//! that never fires is the one failure mode this design has to refuse.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::gateway_pipeline::{HookContext, HookEvent, HookRegistry};

/// The events a shell hook may fire on, with the names the CLI and the
/// files use. The list is the contract: every name here is emitted by the
/// agent loop.
pub const HOOK_EVENT_NAMES: [&str; 4] = ["agent_start", "agent_end", "turn_start", "turn_end"];

/// How long a hook command may run before it is killed.
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

/// One shell hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellHook {
    /// The event that fires this hook.
    pub event: String,
    /// The shell command to run.
    pub command: String,
}

impl ShellHook {
    /// The event this hook fires on, or `None` when the file names an event
    /// the agent never emits.
    pub fn hook_event(&self) -> Option<HookEvent> {
        parse_hook_event(&self.event)
    }
}

/// The directory the hook files live in.
pub fn hooks_dir() -> PathBuf {
    crate::platform::operant_data_dir().join("hooks")
}

/// Map an event name to the event the agent emits.
pub fn parse_hook_event(name: &str) -> Option<HookEvent> {
    match name.trim().to_ascii_lowercase().as_str() {
        "agent_start" => Some(HookEvent::AgentStart),
        "agent_end" => Some(HookEvent::AgentEnd),
        "turn_start" => Some(HookEvent::TurnStart),
        "turn_end" => Some(HookEvent::TurnEnd),
        _ => None,
    }
}

/// The file name of an event (the inverse of [`parse_hook_event`]).
pub fn event_name(event: &HookEvent) -> &'static str {
    match event {
        HookEvent::AgentStart => "agent_start",
        HookEvent::AgentEnd => "agent_end",
        HookEvent::TurnStart => "turn_start",
        HookEvent::TurnEnd => "turn_end",
        _ => "unknown",
    }
}

/// The error a hook file carries, if it is unusable.
#[derive(Debug, Clone)]
pub struct HookProblem {
    /// The file the problem came from.
    pub file: String,
    /// Why the file is unusable.
    pub reason: String,
}

/// Load every hook file in `dir`. Files that cannot be parsed, or that name
/// an event the agent never emits, come back as problems — the good ones
/// still load, because one broken file must not silence the rest.
pub fn load_shell_hooks(dir: &Path) -> (Vec<(String, ShellHook)>, Vec<HookProblem>) {
    let mut hooks = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (hooks, problems);
    };
    let mut files: Vec<_> = entries.filter_map(Result::ok).collect();
    files.sort_by_key(std::fs::DirEntry::file_name);
    for entry in files {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !file_name.ends_with(".hook") {
            continue;
        }
        let name = file_name.trim_end_matches(".hook").to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                problems.push(HookProblem {
                    file: name,
                    reason: format!("could not be read: {err}"),
                });
                continue;
            }
        };
        let hook: ShellHook = match serde_json::from_str(&content) {
            Ok(hook) => hook,
            Err(err) => {
                problems.push(HookProblem {
                    file: name,
                    reason: format!("is not valid hook JSON: {err}"),
                });
                continue;
            }
        };
        if hook.hook_event().is_none() {
            problems.push(HookProblem {
                file: name,
                reason: format!(
                    "names the event {:?}, which the agent never emits (valid: {})",
                    hook.event,
                    HOOK_EVENT_NAMES.join(", ")
                ),
            });
            continue;
        }
        if hook.command.trim().is_empty() {
            problems.push(HookProblem {
                file: name,
                reason: "has an empty command".to_string(),
            });
            continue;
        }
        hooks.push((name, hook));
    }
    (hooks, problems)
}

/// Write a hook file. The name becomes `<name>.hook`.
pub fn write_shell_hook(dir: &Path, name: &str, event: &str, command: &str) -> Result<(), String> {
    if parse_hook_event(event).is_none() {
        return Err(format!(
            "unknown event {event:?} (valid: {})",
            HOOK_EVENT_NAMES.join(", ")
        ));
    }
    if command.trim().is_empty() {
        return Err("the hook command is empty".to_string());
    }
    let safe_name = name.trim();
    if safe_name.is_empty() || safe_name.contains('/') || safe_name.contains("..") {
        return Err(format!("invalid hook name {name:?}"));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join(format!("{safe_name}.hook"));
    let hook = ShellHook {
        event: event.trim().to_ascii_lowercase(),
        command: command.to_string(),
    };
    let content = serde_json::to_string_pretty(&hook)
        .map_err(|e| format!("could not serialise the hook: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Remove a hook file. `Ok(false)` means there was no such hook.
pub fn remove_shell_hook(dir: &Path, name: &str) -> Result<bool, String> {
    let path = dir.join(format!("{}.hook", name.trim()));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("could not remove {}: {e}", path.display(), e = err)),
    }
}

/// Run one hook command now (the `operant hooks test` path). Returns the
/// exit status and whatever the command printed.
pub async fn run_hook_command(command: &str, ctx: &HookContext) -> (Option<i32>, String) {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.env("OPERANT_HOOK_EVENT", "test");
    if let Some(session) = &ctx.session_id {
        cmd.env("OPERANT_HOOK_SESSION", session);
    }
    if let Some(channel) = &ctx.channel_id {
        cmd.env("OPERANT_HOOK_CHANNEL", channel);
    }
    if let Some(user) = &ctx.user_id {
        cmd.env("OPERANT_HOOK_USER", user);
    }
    match tokio::time::timeout(HOOK_TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).to_string(),
        ),
        Ok(Err(err)) => (None, format!("could not run the hook: {err}")),
        Err(_) => (
            None,
            format!("the hook timed out after {}s", HOOK_TIMEOUT.as_secs()),
        ),
    }
}

/// The production wiring: load the operator's hooks from the hooks
/// directory and register them. The agent build calls this; a broken file
/// is reported and skipped, never fatal.
pub async fn install_shell_hooks(registry: &HookRegistry) -> usize {
    let (hooks, problems) = load_shell_hooks(&hooks_dir());
    for problem in &problems {
        tracing::warn!(
            hook = %problem.file,
            reason = %problem.reason,
            "shell hook ignored"
        );
    }
    let count = hooks.len();
    register_shell_hooks(registry, hooks).await;
    count
}

/// Register the hooks on a registry. Each hook runs on its own event; a
/// failing command is reported and never breaks the agent loop.
pub async fn register_shell_hooks(registry: &HookRegistry, hooks: Vec<(String, ShellHook)>) {
    for (name, hook) in hooks {
        let Some(event) = hook.hook_event() else {
            continue;
        };
        let command = hook.command.clone();
        let hook_name = name.clone();
        let handler: crate::gateway_pipeline::HookHandler = Arc::new(move |_event, ctx| {
            let command = command.clone();
            let hook_name = hook_name.clone();
            Box::pin(async move {
                let (status, output) = run_hook_command(&command, &ctx).await;
                match status {
                    Some(0) => {
                        tracing::info!(hook = %hook_name, "shell hook finished");
                    }
                    Some(code) => {
                        tracing::warn!(
                            hook = %hook_name,
                            code,
                            "shell hook exited non-zero"
                        );
                    }
                    None => {
                        tracing::warn!(hook = %hook_name, "shell hook did not run");
                    }
                }
                if !output.trim().is_empty() {
                    tracing::debug!(hook = %hook_name, output = %output.trim());
                }
            })
        });
        registry.register(event, handler).await;
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn only_the_emitted_events_parse() {
        assert!(matches!(
            parse_hook_event("agent_start"),
            Some(HookEvent::AgentStart)
        ));
        assert!(matches!(
            parse_hook_event("AGENT_END"),
            Some(HookEvent::AgentEnd)
        ));
        assert!(parse_hook_event("session_start").is_none());
        assert!(parse_hook_event("before_tool").is_none());
    }

    #[test]
    fn a_broken_file_does_not_silence_the_good_ones() {
        let dir = std::env::temp_dir().join(format!("operant-shell-hooks-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        write_shell_hook(&dir, "good", "agent_end", "true").expect("write");
        std::fs::write(dir.join("broken.hook"), "{not json").expect("write");
        std::fs::write(
            dir.join("wrong_event.hook"),
            r#"{"event":"session_start","command":"true"}"#,
        )
        .expect("write");

        let (hooks, problems) = load_shell_hooks(&dir);
        assert_eq!(hooks.len(), 1, "{hooks:?}");
        assert_eq!(problems.len(), 2, "{problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The production path: a hook file, loaded, registered, and fired by
    /// the event the agent emits — the command really runs.
    #[tokio::test]
    async fn a_registered_hook_runs_when_its_event_fires() {
        let dir =
            std::env::temp_dir().join(format!("operant-shell-hooks-run-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let marker = dir.join("marker");
        let command = format!("echo fired > {}", marker.display());
        write_shell_hook(&dir, "marker", "agent_end", &command).expect("write");

        let (hooks, problems) = load_shell_hooks(&dir);
        assert!(problems.is_empty(), "{problems:?}");
        let registry = HookRegistry::new();
        register_shell_hooks(&registry, hooks).await;
        assert_eq!(registry.len(), 1);

        registry.emit(HookEvent::AgentEnd, HookContext::new()).await;
        // The handler runs in its own task — wait for it to land.
        let mut waited = 0;
        while !marker.exists() && waited < 100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            waited += 1;
        }
        assert!(marker.exists(), "the hook command did not run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A hook on an event the agent never emits must not be registered —
    /// otherwise it is a promise that can never be kept.
    #[tokio::test]
    async fn a_hook_on_an_unemitted_event_is_not_registered() {
        let dir =
            std::env::temp_dir().join(format!("operant-shell-hooks-skip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // Hand-written file: the writer refuses an unknown event, but an
        // operator can always edit the file by hand.
        std::fs::write(
            dir.join("never.hook"),
            r#"{"event":"session_start","command":"true"}"#,
        )
        .expect("write");
        let (hooks, problems) = load_shell_hooks(&dir);
        assert!(hooks.is_empty(), "{hooks:?}");
        assert_eq!(problems.len(), 1, "{problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole production path: the hooks directory is loaded when the
    /// agent is built, and the command runs when the event fires.
    #[tokio::test]
    async fn install_shell_hooks_loads_the_hooks_directory() {
        let _guard = crate::platform::test_env_lock().await;
        let home = std::env::temp_dir().join(format!("operant-hook-home-{}", std::process::id()));
        let dir = home.join("data").join("hooks");
        let _ = std::fs::create_dir_all(&dir);
        let marker = home.join("marker");
        std::fs::write(
            dir.join("installed.hook"),
            serde_json::to_string(&ShellHook {
                event: "agent_end".to_string(),
                command: format!("echo fired > {}", marker.display()),
            })
            .expect("serialise"),
        )
        .expect("write");

        unsafe { std::env::set_var("HERMES_HOME", &home) };
        let registry = HookRegistry::new();
        let installed = install_shell_hooks(&registry).await;
        assert_eq!(installed, 1);
        assert_eq!(registry.len(), 1);

        registry.emit(HookEvent::AgentEnd, HookContext::new()).await;
        let mut waited = 0;
        while !marker.exists() && waited < 100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            waited += 1;
        }
        assert!(marker.exists(), "the installed hook did not run");
        unsafe { std::env::remove_var("HERMES_HOME") };
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn writing_a_hook_rejects_an_unknown_event() {
        let dir =
            std::env::temp_dir().join(format!("operant-shell-hooks-write-{}", std::process::id()));
        let err = write_shell_hook(&dir, "bad", "session_start", "true").expect_err("must fail");
        assert!(err.contains("unknown event"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn running_a_hook_reports_its_exit_code() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let (status, _) = run_hook_command("exit 3", &HookContext::new()).await;
        assert_eq!(status, Some(3));
        CALLS.fetch_add(1, Ordering::SeqCst);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
