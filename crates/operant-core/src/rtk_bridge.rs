//! RTK bridge — resolve the `rtk` binary and ask it how it would run a command.
//!
//! RTK is a shell-output compressor. It rewrites a command such as `git status`
//! into `rtk git status`; running the rewritten form returns a much smaller
//! transcript. This module is the seam between operant and that binary:
//!
//! * [`find_rtk`] resolves the binary (`RTK_BINARY` override → `PATH`),
//! * [`rewrite_command`] asks rtk what it would run,
//! * [`rtk_version`] reports the binary's version.
//!
//! ## The exit-code contract
//!
//! `rtk rewrite` documents four exit codes (rtk 1.0.0, `rtk rewrite --help`):
//!
//! | code | rtk's meaning |
//! |------|---------------|
//! | 0 | rewrite printed; allow rules cover every command (a host may auto-allow) |
//! | 1 | nothing printed; no RTK equivalent (excluded, or unattestable input such as `$(...)`, a redirect, or a multi-line block) |
//! | 2 | a deny rule matched |
//! | 3 | rewrite printed; an ask rule matched, or the rules do not cover every command |
//!
//! Codes 0 **and** 3 print the rewritten command on stdout — exit 3 is still a
//! rewrite ("Exit 3 is still a rewrite", rtk's own help text), and exit 3 is
//! the DEFAULT verdict on a host with no rules. Exit 0 is a permission
//! verdict, never a success signal: operant keeps its own approval gate in
//! both cases.
//!
//! MEASURED on this host (2026-11-12, rtk 1.0.0): `rtk rewrite "git status"`
//! prints `rtk git status` — 16 raw bytes, no brackets, no trailing newline —
//! and exits 3; `rtk rewrite "ls | grep foo"` prints `ls | rtk grep foo`;
//! `rtk rewrite "echo $(ls)"` prints nothing and exits 1. The square-bracket
//! form (`[rtk git status]`) is rtk's DISPLAY convention, not the raw output,
//! so this bridge never strips brackets from what it receives.
//!
//! ## Degradation
//!
//! The bridge never breaks the terminal. A missing binary, a spawn failure, a
//! crash or a timeout all end in [`RewriteVerdict::Failed`], and the caller
//! runs the ORIGINAL command unchanged. A missing rewrite rule is
//! [`RewriteVerdict::NoRule`] — also "run it unchanged". Only rtk's own deny
//! is [`RewriteVerdict::Refused`].
//!
//! ## The colliding-subcommand guard
//!
//! rtk's own subcommands include names that are also real binaries (`ls`,
//! `test`, `grep`, …). Rewriting `ls` would run *rtk's* `ls` compressor
//! instead of the real `ls`, so a command whose first token is one of
//! [`RTK_COLLIDING_SUBCOMMANDS`] is never rewritten — see
//! [`is_rtk_command`].

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Environment variable that overrides binary discovery.
const RTK_BINARY_ENV: &str = "RTK_BINARY";

/// How long any single rtk invocation may run before the bridge gives up.
///
/// A hung rtk must not hang the agent's turn: the child is killed and the
/// bridge degrades to "run the original command unchanged".
const RTK_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the timeout watcher polls the child for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// rtk's own subcommand names that are also real binaries on a normal host.
///
/// MEASURED on this host (2026-11-12, rtk 1.0.0): these eleven names resolve
/// to rtk's subcommand instead of the system binary, so a command that starts
/// with one of them must be left alone.
pub const RTK_COLLIDING_SUBCOMMANDS: [&str; 11] = [
    "test", "read", "log", "diff", "grep", "env", "init", "json", "find", "ls", "tree",
];

/// Where the rtk binary was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtkLocation {
    /// A binary found on `PATH` by `which::which("rtk")`.
    Path(PathBuf),
    /// The `RTK_BINARY` environment override.
    Env(PathBuf),
}

impl RtkLocation {
    /// The filesystem path of the resolved binary.
    pub fn path(&self) -> &Path {
        match self {
            RtkLocation::Path(path) | RtkLocation::Env(path) => path.as_path(),
        }
    }
}

/// What `rtk rewrite` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteVerdict {
    /// A rewritten command; rtk compresses this one's output.
    Rewritten { command: String },
    /// rtk had no rule for this command; the caller runs it unchanged.
    NoRule,
    /// rtk REFUSED the command (a deny rule matched) — the caller must NOT
    /// run it, because rtk judged it unsafe. The reason is rtk's own
    /// diagnostic.
    Refused { reason: String },
    /// rtk itself failed (could not start, or crashed). The caller must run
    /// the original command unchanged.
    Failed { reason: String },
}

/// Whether a command's FIRST token names one of rtk's own subcommands that
/// collide with a real binary ([`RTK_COLLIDING_SUBCOMMANDS`]).
///
/// Rewriting such a command would run rtk's subcommand — `rtk ls`, `rtk
/// test`, … — instead of the binary the model asked for, so the bridge
/// returns [`RewriteVerdict::NoRule`] for it. Only the first token is
/// inspected: a colliding name further down a chain (`git status && ls`) is
/// still rtk's to rewrite.
pub fn is_rtk_command(cmd: &str) -> bool {
    let first = cmd.split_whitespace().next().unwrap_or_default();
    RTK_COLLIDING_SUBCOMMANDS.contains(&first)
}

/// Resolve the rtk binary.
///
/// Order: the `RTK_BINARY` env override → `which::which("rtk")` → `None`.
/// The shape mirrors [`crate::aft_bridge::resolve_aft_binary`]: an explicit
/// override wins when it points at an existing file, while a `PATH` hit must
/// answer `--version` before it is accepted, so a dead shim is never mistaken
/// for a working binary.
pub fn find_rtk() -> Option<RtkLocation> {
    if let Ok(raw) = std::env::var(RTK_BINARY_ENV) {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(RtkLocation::Env(path));
        }
    }
    let path = which::which("rtk").ok()?;
    if !probe_version(&path) {
        return None;
    }
    Some(RtkLocation::Path(path))
}

/// Ask rtk how it would run this command.
///
/// The command is passed as ONE argv element (`rtk rewrite "<command>"`), never
/// through a shell, so shell metacharacters inside it stay data.
///
/// The verdict never treats a non-zero exit as "the rewrite failed, do not run
/// the command": a missing rule is [`RewriteVerdict::NoRule`] (the caller runs
/// the original command) and a broken rtk is [`RewriteVerdict::Failed`] (also
/// "run the original command"). Only rtk's own deny is
/// [`RewriteVerdict::Refused`].
///
/// A command whose first token is one of rtk's colliding subcommands
/// ([`is_rtk_command`]) is never rewritten: rtk would run its own subcommand
/// instead of the real binary.
pub fn rewrite_command(rtk: &RtkLocation, command: &str) -> RewriteVerdict {
    if is_rtk_command(command) {
        return RewriteVerdict::NoRule;
    }
    match capture(rtk.path(), &["rewrite", command]) {
        Ok(captured) => classify(captured.code, &captured.stdout, &captured.stderr),
        Err(reason) => RewriteVerdict::Failed { reason },
    }
}

/// The pure exit-code → verdict mapping.
///
/// The exit code is the primary signal and stdout is its confirmation:
/// `exit_code` is rtk's exit status, `stdout` the rewritten command it printed
/// (empty when it printed nothing) and `stderr` its diagnostics.
///
/// * `0` / `3` → [`RewriteVerdict::Rewritten`] when stdout confirms a command
///   was printed, [`RewriteVerdict::NoRule`] when it printed nothing (there is
///   nothing to run),
/// * `1` → [`RewriteVerdict::NoRule`] — rtk has no rewrite for this command
///   and printed nothing; the caller runs it unchanged. A command that did
///   come out of stdout is trusted over the exit code and stays
///   [`RewriteVerdict::Rewritten`],
/// * `2` → [`RewriteVerdict::Refused`] — a deny rule matched,
/// * anything else → [`RewriteVerdict::Failed`] — rtk itself misbehaved.
pub fn classify(exit_code: i32, stdout: &str, stderr: &str) -> RewriteVerdict {
    let rewritten = stdout.trim();
    match exit_code {
        2 => RewriteVerdict::Refused {
            reason: deny_reason(stdout, stderr),
        },
        0 | 1 | 3 => {
            if rewritten.is_empty() {
                RewriteVerdict::NoRule
            } else {
                RewriteVerdict::Rewritten {
                    command: rewritten.to_string(),
                }
            }
        }
        _ => RewriteVerdict::Failed {
            reason: format!(
                "rtk rewrite exited with status {exit_code}: {}",
                stderr.trim()
            ),
        },
    }
}

/// The rtk binary's version string, or `None` when it cannot be run.
pub fn rtk_version(rtk: &RtkLocation) -> Option<String> {
    let captured = capture(rtk.path(), &["--version"]).ok()?;
    if captured.code != 0 {
        return None;
    }
    let version = captured.stdout.trim();
    if !version.is_empty() {
        return Some(version.to_string());
    }
    let fallback = captured.stderr.trim();
    if fallback.is_empty() {
        None
    } else {
        Some(fallback.to_string())
    }
}

/// The reason rtk gave for a deny.
///
/// rtk's own diagnostics carry an `rtk:` prefix on stderr; a pipeline rule
/// that denies a command prints its message instead. MEASURED on this host:
/// rtk's stderr was empty in every deny case, so a refusal with no output at
/// all still carries a reason that names that fact rather than inventing a
/// cause.
fn deny_reason(stdout: &str, stderr: &str) -> String {
    let stderr_text = stderr.trim();
    if !stderr_text.is_empty() {
        return stderr_text.to_string();
    }
    let stdout_text = stdout.trim();
    if !stdout_text.is_empty() {
        return stdout_text.to_string();
    }
    "rtk denied the command without giving a reason".to_string()
}

/// Whether `<bin> --version` answers, i.e. the candidate can serve requests.
fn probe_version(bin: &Path) -> bool {
    match capture(bin, &["--version"]) {
        Ok(captured) => captured.code == 0 && !captured.stdout.trim().is_empty(),
        Err(_) => false,
    }
}

/// What a finished rtk invocation produced.
struct Captured {
    /// The process exit status (`-1` when the process died from a signal).
    code: i32,
    /// Everything the process wrote to stdout.
    stdout: String,
    /// Everything the process wrote to stderr.
    stderr: String,
}

/// Run `bin` with `args`, capturing stdout and stderr separately, and kill it
/// if it outlives [`RTK_TIMEOUT`].
fn capture(bin: &Path, args: &[&str]) -> Result<Captured, String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn {}: {err}", bin.display()))?;

    // The pipes are drained on their own threads: a child that fills one
    // 64 KiB pipe buffer while the parent waits would otherwise deadlock.
    let stdout_reader = drain(child.stdout.take());
    let stderr_reader = drain(child.stderr.take());

    let deadline = Instant::now() + RTK_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to wait for {}: {err}", bin.display()));
            }
        }
    };

    let stdout = collect(stdout_reader);
    let stderr = collect(stderr_reader);

    match status {
        Some(status) => Ok(Captured {
            code: status.code().unwrap_or(-1),
            stdout,
            stderr,
        }),
        None => Err(format!(
            "{} timed out after {} seconds",
            bin.display(),
            RTK_TIMEOUT.as_secs()
        )),
    }
}

/// Read a pipe to EOF on its own thread.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Option<std::thread::JoinHandle<String>> {
    pipe.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer);
            String::from_utf8_lossy(&buffer).into_owned()
        })
    })
}

/// Join a reader thread; a panicking reader yields no output rather than
/// taking the bridge down with it.
fn collect(reader: Option<std::thread::JoinHandle<String>>) -> String {
    match reader {
        Some(handle) => handle.join().unwrap_or_default(),
        None => String::new(),
    }
}
