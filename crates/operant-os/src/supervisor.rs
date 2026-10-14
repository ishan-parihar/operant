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

/// Run ONE prepared body to completion: spawn under the prepared env + the
/// supervisor's own environment (the driver command is a shell line), renew
/// the lease every `lease/3` (min 10s), wait (bounded by the driver's
/// timeout), then hand in with the body's exit code.
///
/// The body inherits the supervisor's environment plus the prepared run env
/// — the kernel's run_env already carries HOLON_RUN_TOKEN, HOLON_RUN_DIR
/// (HOLON_BRIEF_DIR) and HOLON, and the driver's own [env] entries.
pub async fn run_body(
    config_home: &Path,
    bin: Option<&str>,
    prepared: &Prepared,
) -> crate::Result<i32> {
    let token = prepared
        .env
        .get("HOLON_RUN_TOKEN")
        .ok_or_else(|| crate::Error::Protocol("prepared run carries no token".into()))?
        .clone();

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&prepared.command)
        .current_dir(&prepared.cwd);
    // the driver's env (HOLON_*, HERMES_HOME-style entries) — over ours
    for (k, v) in &prepared.env {
        cmd.env(k, v);
    }
    cmd.kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| crate::Error::Protocol(format!("spawn driver: {e}")))?;

    // lease cadence: renew at a third of the lease window, floor 10s — a
    // slow body never lapses, a fast one costs two or three renewals
    let cadence = Duration::from_secs(10);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(prepared.timeout_s.max(1));
    let status: Option<std::process::ExitStatus> = loop {
        tokio::select! {
            st = child.wait() => break Some(st
                .map_err(|e| crate::Error::Protocol(format!("wait driver: {e}")))?),
            _ = tokio::time::sleep(cadence) => {
                if tokio::time::Instant::now() >= deadline {
                    let _ = child.kill().await;
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
