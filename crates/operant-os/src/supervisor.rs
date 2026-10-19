//! The run supervisor (boundary doc §2: replaces one resident Python
//! dispatcher per run).
//!
//! One loop per organization: claim the next stage for one employee
//! (`prepare`), spawn the returned driver command under its env, renew the
//! lease every `lease/3` while it runs, and hand the body in (`finish`).
//! At most `max_concurrent_runs` bodies at once. A supervisor that dies lets
//! every prepared run revert by lease expiry — safety is the kernel's, not
//! ours. The clock (`holon tick`) is fired separately by the caller's cron.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use tokio::process::{Child, Command};

use crate::run::{Prepared, finish, heartbeat, prepare};

/// The OS baseline a body inherits (O-07): everything else — channel
/// tokens, provider keys, HOLON_CHAIR_KEY, the supervisor's HERMES_HOME —
/// stays with the supervisor. The prepared env layers on top of this.
const ENV_BASELINE: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TMPDIR",
    "XDG_RUNTIME_DIR",
];

/// OS-2: what a body's `HOLON_CHAIR_KEY` is pinned to. Holon's chair check
/// reads this override before the operator's key file, and no digest of it
/// matches the stored chair hash, so a body that drops its run token gets no
/// chair authority from the CLI's implicit key lookup. An explicit read of the
/// key file by a same-uid body is out of reach until O-11 isolation lands.
const BODY_CHAIR_KEY: &str = "operant-body-holds-no-chair-key";

/// OS-10: the grace between a body group's SIGTERM and its SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// OS-1: a body is a same-uid child of the supervisor, and a same-uid process
/// can read its parent's `/proc/<pid>/environ` unless that process is
/// non-dumpable. Clearing the flag hides the supervisor's own environment
/// (channel tokens, provider keys) from every body it spawns. `exec` resets the
/// flag, so each body stays dumpable; only the supervisor's `/proc` entries close.
#[cfg(target_os = "linux")]
fn harden_supervisor() -> bool {
    // SAFETY: PR_SET_DUMPABLE takes an integer and changes only this process's flag.
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) == 0 }
}

/// Off Linux there is no dumpable flag, so the caller is told it cannot hold.
#[cfg(not(target_os = "linux"))]
fn harden_supervisor() -> bool {
    false
}

/// The environment a body runs under (O-07, OS-2): the OS baseline, the
/// prepared run env on top, then the chair pin last so nothing prepared can
/// override it.
fn body_env(prepared: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = ENV_BASELINE
        .iter()
        .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_string(), v)))
        .collect();
    env.extend(prepared.iter().map(|(k, v)| (k.clone(), v.clone())));
    env.push(("HOLON_CHAIR_KEY".to_string(), BODY_CHAIR_KEY.to_string()));
    env
}

/// OS-6: a body's process group, killed when the body's future is dropped. A
/// signalled supervisor cancels that future, and a cancelled future cannot
/// await, so this kill is an immediate SIGKILL. `kill_on_drop` alone reaches
/// only the `sh` leader, and the group is what the body's grandchildren share.
struct BodyGroup(Option<u32>);

impl Drop for BodyGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            kill_group_signal(pid, libc::SIGKILL);
        }
    }
}

/// Run ONE prepared body to completion: spawn under the prepared env + the
/// OS baseline (the driver command is a shell line), renew
/// the lease every `lease/3` (min 10s), wait (bounded by the driver's
/// timeout), then hand in with the body's exit code.
///
/// The body inherits ONLY the OS baseline (ENV_BASELINE) plus the prepared
/// run env (O-07) — the kernel's run_env carries HOLON_RUN_TOKEN,
/// HOLON_RUN_DIR (HOLON_BRIEF_DIR) and HOLON, and the driver's own [env]
/// entries. The supervisor's secrets never reach it.
pub async fn run_body(
    config_home: &Path,
    bin: Option<&str>,
    prepared: &Prepared,
) -> crate::Result<i32> {
    // O-11: an isolated instance's prepared env carries HOLON_BROKER —
    // the body is meant to run as the agent user through the runner
    // socket. Until the Rust runner exists, supervising such an instance
    // would run the body as the operator (who can read the Chair key and
    // work.db): refuse, with the reason named.
    if prepared.env.contains_key("HOLON_BROKER") {
        return Err(crate::Error::Protocol(
            "this instance runs isolated bodies through its runner socket — the operant \
             supervisor serves it only when the Rust runner lands (audit O-11/O-30)"
                .into(),
        ));
    }

    let token = prepared
        .env
        .get("HOLON_RUN_TOKEN")
        .ok_or_else(|| crate::Error::Protocol("prepared run carries no token".into()))?
        .clone();

    // O-12: the body runs in its own session (setsid) so a timeout kill
    // takes the whole process group — `sh` alone dying leaves grandchildren
    // running (the audit's orphan defect). Its stdout and stderr tee into
    // <brief dir>/driver.log, the evidence file the Python path writes
    // and the kernel's run row expects.
    let brief_dir = prepared
        .brief_md
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| prepared.cwd.clone());
    let driver_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(brief_dir.join("driver.log"))
        .ok();
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&prepared.command)
        .current_dir(&prepared.cwd)
        .process_group(0);
    if let Some(log) = driver_log {
        use std::os::fd::AsRawFd;
        use std::os::fd::FromRawFd;
        let fd = log.as_raw_fd();
        // duplicate the log fd for both streams — the tee is append-only
        let stdout_dup = unsafe { std::os::fd::OwnedFd::from_raw_fd(libc::dup(fd)) };
        let stderr_dup = unsafe { std::os::fd::OwnedFd::from_raw_fd(libc::dup(fd)) };
        cmd.stdout(stdout_dup);
        cmd.stderr(stderr_dup);
    }
    // O-07: a body gets the OS baseline and the kernel's prepared env —
    // NEVER the supervisor's own environment. The gateway's supervisor
    // carries channel tokens and provider keys, and an operator's
    // HOLON_CHAIR_KEY would be a path for the Chair key to reach agents,
    // which a standing rule forbids. env_clear, then body_env: the baseline,
    // the prepared env on top, and the chair pin last (OS-2).
    cmd.env_clear().envs(body_env(&prepared.env));
    cmd.kill_on_drop(true);
    if !harden_supervisor() {
        tracing::warn!(
            "this platform cannot hide the supervisor's environment from its bodies (OS-1)"
        );
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| crate::Error::Protocol(format!("spawn driver: {e}")))?;
    let _group = BodyGroup(child.id());

    // O-12: lease-derived cadence — renew every lease/3 (floor 10s). The
    // kernel's lease window is on the prepared run; a 60-minute execute
    // lease renews every 20 minutes, not every 10 seconds (the audit
    // counted ~360 spurious Python spawns per run at the fixed rate).
    let cadence = lease_third(&prepared.lease_until).await;
    // O-12 (CONTRACT §4): the stop-after-submit poll runs on its OWN short
    // cadence — the kernel's contract says a handed-in body is stopped
    // "within a minute", and piggybacking on the heartbeat cadence made a
    // 20-minute-lease body squat its slot that long after submitting.
    // 15s keeps it under the minute without a Python spawn storm.
    let poll = Duration::from_secs(15);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(prepared.timeout_s.max(1));
    let mut closed: Option<Closed> = None;
    let mut next_heartbeat = tokio::time::Instant::now() + cadence;
    let status: Option<std::process::ExitStatus> = loop {
        let next_poll = tokio::time::Instant::now() + poll;
        tokio::select! {
            st = child.wait() => break Some(st
                .map_err(|e| crate::Error::Protocol(format!("wait driver: {e}")))?),
            _ = tokio::time::sleep_until(next_poll) => {
                if tokio::time::Instant::now() >= deadline {
                    // O-12: the whole process group goes — `sh` alone dying
                    // leaves the body's grandchildren running
                    kill_process_group(&mut child, KILL_GRACE).await;
                    break None;
                }
                // the kernel's status says whether the slot is over: a body
                // that handed its result in itself (`holon submit`, a
                // defer-submit that records and holds verify) has nothing
                // left to do, and a run the kernel closed without a submit
                // (its lease was reclaimed) must not keep a body running
                match run_status(config_home, bin, &prepared.run).await {
                    Ok(state) => {
                        if let Some(by) = closed_by(&state) {
                            match by {
                                Closed::HandedIn => tracing::info!(run = %prepared.run, "run handed in — stopping the body"),
                                Closed::Reclaimed => tracing::warn!(run = %prepared.run, status = %state, "run closed without a submit (lease reclaimed) — stopping the body"),
                            }
                            kill_process_group(&mut child, KILL_GRACE).await;
                            closed = Some(by);
                            break None;
                        }
                    }
                    Err(e) => tracing::debug!(run = %prepared.run, error = %e, "run show failed (continuing)"),
                }
                if tokio::time::Instant::now() >= next_heartbeat {
                    if let Err(e) = heartbeat(config_home, bin, &prepared.run, &token).await {
                        tracing::warn!(run = %prepared.run, error = %e, "heartbeat failed (continuing)");
                    }
                    next_heartbeat = tokio::time::Instant::now() + cadence;
                }
            }
        }
    };
    let exit = match status {
        Some(st) => st.code().unwrap_or(-1),
        // a self-submit is the GOOD stop: the kernel's own submit has the
        // result; report a clean exit, not a timeout kill
        None if closed == Some(Closed::HandedIn) => 0,
        // timed out at the driver's own ceiling, or reclaimed under the body:
        // report a non-zero exit; the kernel records the breach the same way a
        // killed dispatcher would
        None => 137,
    };
    // Invariant 2 (BOUNDARY §5, W8): the finished result validates against
    // the kernel's published schema before the run hands in — a warning
    // here is the drift's first signal (the kernel's own submit remains
    // the authority). The stage comes from the run's policy document.
    if let Some(brief_dir) = prepared.brief_md.parent()
        && let Ok(policy_text) = std::fs::read_to_string(brief_dir.join("policy.json"))
        && let Ok(policy) = serde_json::from_str::<serde_json::Value>(&policy_text)
        && let Some(stage) = policy.get("stage").and_then(|s| s.as_str())
        && let Err(e) = crate::contract::validate_result(brief_dir, stage)
    {
        tracing::warn!(stage, error = %e, "result.json failed its schema check (submit will refuse it)");
    }

    // A body the kernel already closed needs no hand-in: a self-submit moved
    // the run on, and a reclaimed run is closed — `run finish` on either would
    // hit the kernel's TransitionError ("nothing to finish"). The finish call
    // is the DRIVER-path hand-in only.
    if closed.is_none() {
        finish(config_home, bin, &prepared.run, exit).await?;
    }
    Ok(exit)
}

/// O-12: lease/3 renewal cadence, floor 10s. The kernel stamps ISO-8601
/// UTC; a 15-minute lease renews every 5 minutes, an hour-long execute
/// lease every 20. Unparseable stamps fall back to the floor.
async fn lease_third(lease_until: &str) -> Duration {
    const FLOOR: Duration = Duration::from_secs(10);
    let Some(window_s) = iso_window_seconds(lease_until) else {
        return FLOOR;
    };
    Duration::from_secs((window_s / 3).max(FLOOR.as_secs()))
}

fn iso_window_seconds(stamp: &str) -> Option<u64> {
    let until = chrono::DateTime::parse_from_rfc3339(stamp).ok()?;
    let now: chrono::DateTime<chrono::FixedOffset> = chrono::Utc::now().fixed_offset();
    (until - now).num_seconds().try_into().ok()
}

/// One supervision pass for one employee: claim (if anything is ready), run
/// the body, hand it in. Returns whether a body ran. The caller loops this
/// per employee under its concurrency budget.
pub async fn supervise_one(
    config_home: &Path,
    bin: Option<&str>,
    employee: &str,
    stage: Option<&str>,
) -> crate::Result<bool> {
    match prepare(config_home, bin, employee, stage, None).await? {
        None => Ok(false),
        Some(prepared) => {
            tracing::info!(
                run = %prepared.run,
                stage = %prepared.stage,
                driver = %prepared.driver,
                "supervisor: body prepared — running"
            );
            run_body(config_home, bin, &prepared).await?;
            Ok(true)
        }
    }
}

/// What a run's kernel status says about the body's slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closed {
    /// the body's own submit closed the run: the result is in
    HandedIn,
    /// the kernel closed the run without a submit (lease reclaimed or cancelled)
    Reclaimed,
}

/// `None` while the run is open. OS-7: only a self-submit (`succeeded` or
/// `failed`) is a clean stop. Any other closed status is a reclaim, and so is
/// an unknown one: a body must never outlive a run the kernel has closed.
fn closed_by(status: &str) -> Option<Closed> {
    match status {
        "running" => None,
        "succeeded" | "failed" => Some(Closed::HandedIn),
        _ => Some(Closed::Reclaimed),
    }
}

/// O-12 (CONTRACT §4): the run's kernel status. `run show` is an any-actor
/// read; [`closed_by`] says what the status means for the body's slot.
async fn run_status(config_home: &Path, bin: Option<&str>, run: &str) -> crate::Result<String> {
    let mut cmd = crate::run::holon_cmd(config_home, bin);
    cmd.args(["--json", "run", "show", run]);
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| crate::Error::Protocol(format!("run show: {e}")))?;
    if !out.status.success() {
        return Err(crate::Error::Protocol(format!(
            "run show exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    #[derive(serde::Deserialize)]
    struct Shell {
        run: RunRow,
    }
    #[derive(serde::Deserialize)]
    struct RunRow {
        status: String,
    }
    let shell: Shell = serde_json::from_slice(&out.stdout)
        .map_err(|e| crate::Error::Protocol(format!("run show output: {e}")))?;
    Ok(shell.run.status)
}

/// Stop the body's whole process group (O-12, OS-10): SIGTERM the group, give
/// it `grace` to exit, then SIGKILL the group. A grandchild that traps TERM
/// survives a TERM-only stop, and the SIGKILL the leader received alone never
/// reached it. The group id is the leader's pid, which stays reserved while any
/// member lives, so the group can still be signalled after the leader is reaped.
async fn kill_process_group(child: &mut Child, grace: Duration) {
    let Some(pid) = child.id() else { return };
    kill_group_signal(pid, libc::SIGTERM);
    let _ = tokio::time::timeout(grace, child.wait()).await;
    kill_group_signal(pid, libc::SIGKILL);
    let _ = child.wait().await;
}

/// `kill(-pid, signal)`: the group whose leader is `leader_pid` (the spawn used
/// `process_group(0)`, so the child is that group's leader).
#[cfg(unix)]
fn kill_group_signal(leader_pid: u32, signal: libc::c_int) -> bool {
    // SAFETY: a negative pid addresses a process group; the group is one we spawned.
    unsafe { libc::kill(-(leader_pid as i32), signal) == 0 }
}

/// OS-6: resolves on the first SIGINT, SIGTERM or SIGHUP. The supervisor races
/// this against its whole loop, so a signal is seen wherever the loop is — in a
/// pass, a body, or the idle back-off — and the body in flight is cancelled.
pub async fn shutdown_signal() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = hangup.recv() => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use tokio::io::{AsyncBufReadExt, BufReader};

    use super::*;

    /// True once no live member remains in the group `pgid`. Polls, because a
    /// SIGKILLed member is reaped by its new parent a moment after the signal.
    fn group_gone(pgid: u32) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if !kill_group_signal(pgid, 0) {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_body_cannot_read_its_supervisors_environment() {
        assert!(harden_supervisor(), "PR_SET_DUMPABLE was refused");
        // a body is a child of this process: `$PPID` is the supervisor it reads
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg("cat /proc/$PPID/environ 2>/dev/null | wc -c")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0");
    }

    #[test]
    fn a_body_is_pinned_to_no_chair_key() {
        let prepared = HashMap::from([("HOLON_CHAIR_KEY".to_string(), "leaked".to_string())]);
        let env = body_env(&prepared);
        // the later entry wins in a spawned environment
        let effective = env
            .iter()
            .rev()
            .find(|(k, _)| k == "HOLON_CHAIR_KEY")
            .map(|(_, v)| v.as_str());
        assert_eq!(effective, Some(BODY_CHAIR_KEY));
    }

    #[test]
    fn only_a_self_submit_is_a_clean_stop() {
        assert_eq!(closed_by("running"), None);
        assert_eq!(closed_by("succeeded"), Some(Closed::HandedIn));
        assert_eq!(closed_by("failed"), Some(Closed::HandedIn));
        assert_eq!(closed_by("timeout"), Some(Closed::Reclaimed));
        assert_eq!(closed_by("abandoned"), Some(Closed::Reclaimed));
    }

    #[tokio::test]
    async fn dropping_a_body_kills_its_whole_group() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 60 & echo $!; wait")
            .process_group(0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        // the grandchild is running once its pid is printed
        lines.next_line().await.unwrap();
        drop(BodyGroup(Some(pid)));
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        assert!(group_gone(pid));
    }

    #[tokio::test]
    async fn a_term_trapping_grandchild_dies_with_its_group() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("trap '' TERM; sleep 60 & echo $!; wait")
            .process_group(0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        lines.next_line().await.unwrap();
        // bounded: without the SIGKILL the call waits out the grandchild's own
        // sleep, so the group would only look gone after a minute
        tokio::time::timeout(
            Duration::from_secs(5),
            kill_process_group(&mut child, Duration::from_millis(100)),
        )
        .await
        .expect("the group kill returned within 5s");
        assert!(group_gone(pid));
    }
}
