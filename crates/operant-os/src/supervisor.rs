//! The run supervisor (boundary doc §2: replaces one resident Python
//! dispatcher per run).
//!
//! One loop per organization: claim the next stage for one employee
//! (`prepare`), spawn the returned driver command under its env, renew the
//! lease every `lease/3` while it runs, and hand the body in (`finish`).
//! At most `max_concurrent_runs` bodies at once. A supervisor that dies lets
//! every prepared run revert by lease expiry — safety is the kernel's, not
//! ours. The clock (`holon tick`) is fired separately by the caller's cron.

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

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
    // which a standing rule forbids. env_clear, then the allowlist, then
    // the prepared env on top (it wins by construction).
    cmd.env_clear();
    for key in ENV_BASELINE {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }
    for (k, v) in &prepared.env {
        cmd.env(k, v);
    }
    cmd.kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| crate::Error::Protocol(format!("spawn driver: {e}")))?;

    // O-12: lease-derived cadence — renew every lease/3 (floor 10s). The
    // kernel's lease window is on the prepared run; a 60-minute execute
    // lease renews every 20 minutes, not every 10 seconds (the audit
    // counted ~360 spurious Python spawns per run at the fixed rate).
    let cadence = lease_third(&prepared.lease_until).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(prepared.timeout_s.max(1));
    let status: Option<std::process::ExitStatus> = loop {
        tokio::select! {
            st = child.wait() => break Some(st
                .map_err(|e| crate::Error::Protocol(format!("wait driver: {e}")))?),
            _ = tokio::time::sleep(cadence) => {
                if tokio::time::Instant::now() >= deadline {
                    // O-12: the whole process group goes — `sh` alone dying
                    // leaves the body's grandchildren running
                    let _ = kill_process_group(&mut child).await;
                    break None;
                }
                if let Err(e) = heartbeat(config_home, bin, &prepared.run, &token).await {
                    tracing::warn!(run = %prepared.run, error = %e, "heartbeat failed (continuing)");
                }
            }
        }
    };
    let exit = match status {
        Some(st) => st.code().unwrap_or(-1),
        // timed out at the driver's own ceiling: report a non-zero exit; the
        // kernel records the breach the same way a killed dispatcher would
        None => 137,
    };
    finish(config_home, bin, &prepared.run, exit).await?;
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

/// O-12: kill the body's whole process group (the spawn used
/// `process_group(0)`, so the child is a group leader).
async fn kill_process_group(child: &mut tokio::process::Child) -> std::io::Result<()> {
    if let Some(pid) = child.id() {
        let _ = kill_group_signal(pid);
    }
    child.kill().await
}

#[cfg(unix)]
fn kill_group_signal(leader_pid: u32) -> bool {
    // SAFETY: killpg with a pid we own (the child we spawned as group
    // leader). Negative pid = process group.
    unsafe { libc::kill(-(leader_pid as i32), libc::SIGTERM) == 0 }
}
