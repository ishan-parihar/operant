//! W4: the resident organs (audit O-12, O-13). The gateway is operant's
//! long-lived process — when it runs, the organization's supervisor loop
//! and clock live in it, not in a CLI one-shot or a Python daemon.
//!
//! - **supervise**: one loop over `[holon] supervise_employees`, each pass
//!   claiming what the kernel has ready. Concurrency is bounded by
//!   `[holon] max_concurrent_runs` (a semaphore shared across passes);
//!   a pass that finds nothing sleeps briefly; an error backs off
//!   exponentially (the CLI loop's hot-retry defect: a missing `holon`
//!   re-spawned it with no sleep).
//! - **clock**: every 2 minutes, `holon tick` — due routines fire, expired
//!   leases sweep, notices compose. The packaged cadence from the plan.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use operant_core::config::AppConfig;

use crate::run::prepare;
use crate::supervisor::run_body;

/// Spawn the configured organs (idempotent; call once at gateway start).
pub fn spawn_if_configured(config: &AppConfig) {
    if !config.holon.enabled {
        return;
    }
    if config.holon.supervise && !config.holon.supervise_employees.is_empty() {
        let home = config.holon.home.clone();
        let bin = config.holon.bin.clone();
        let employees = config.holon.supervise_employees.clone();
        let max = config.holon.max_concurrent_runs.max(1) as usize;
        let bin_s = bin.clone();
        let home_s = home.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!(error = %e, "holon supervise: no tokio runtime");
                    return;
                }
            };
            rt.block_on(async move {
                tracing::info!(
                    employees = ?employees,
                    max,
                    "holon supervise organ: resident supervisor loop running"
                );
                supervise_loop(&home_s, bin_s.as_deref(), &employees, max).await;
            });
        });
    }
    if config.holon.clock {
        let home = config.holon.home.clone();
        let bin = config.holon.bin.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!(error = %e, "holon clock: no tokio runtime");
                    return;
                }
            };
            rt.block_on(async move {
                tracing::info!(home = %home.display(), "holon clock organ: tick every 2 minutes");
                loop {
                    tokio::time::sleep(Duration::from_secs(120)).await;
                    match tick_once(&home, bin.as_deref()).await {
                        Ok(()) => {
                            // Liveness evidence (G3, REMAINING-GAPS): the
                            // doctor reads this stamp to tell a clock organ
                            // that ticks from one that died silently.
                            // Best-effort — a stamp failure must never kill
                            // the cadence.
                            if let Err(e) = stamp_last_tick(&home) {
                                tracing::warn!(error = %e, "holon tick: could not stamp liveness");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "holon tick failed (retrying next cadence)");
                        }
                    }
                }
            });
        });
    }
}

/// One `holon tick`.
async fn tick_once(home: &Path, bin: Option<&str>) -> crate::Result<()> {
    let mut cmd = crate::run::holon_cmd(home, bin);
    cmd.args(["tick"]);
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| crate::Error::Protocol(format!("tick: {e}")))?;
    if !out.status.success() {
        return Err(crate::Error::Protocol(format!(
            "tick exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Write the clock organ's liveness stamp: the unix seconds of the last
/// successful `holon tick`, at `<home>/.clock/last_tick`. The doctor's
/// liveness check reads it (G3). Best-effort by contract — the caller
/// logs a failure and keeps the cadence.
fn stamp_last_tick(home: &Path) -> std::io::Result<()> {
    let dir = home.join(".clock");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("last_tick"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .to_string(),
    )
}

/// The resident supervisor loop: claim what is ready, run it under the
/// concurrency budget, hand it in, repeat. An empty pass sleeps; an
/// error backs off (floor 1s, ceiling 60s, reset on success).
///
/// O-12: `max_concurrent_runs` is a real bound — bodies run as spawned
/// tasks holding semaphore permits, not sequential awaits (the inline
/// await made the semaphore decorative: concurrency was always 1). A
/// pass collects the spawned handles and waits for them before looping,
/// so a claim is never issued while the previous pass's bodies still
/// hold slots the loop would double-count.
async fn supervise_loop(home: &Path, bin: Option<&str>, employees: &[String], max: usize) -> ! {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max));
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut ran = false;
        let mut errored = false;
        let mut handles = Vec::new();
        for employee in employees {
            match prepare(home, bin, employee, None, None).await {
                Ok(Some(prepared)) => {
                    ran = true;
                    backoff = Duration::from_secs(1);
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };
                    let home = home.to_path_buf();
                    let bin = bin.map(str::to_string);
                    handles.push(tokio::spawn(async move {
                        let r = run_body(&home, bin.as_deref(), &prepared).await;
                        drop(permit);
                        if let Err(e) = r {
                            tracing::warn!(run = %prepared.run, error = %e, "supervised body failed");
                        }
                    }));
                }
                Ok(None) => {}
                Err(e) => {
                    errored = true;
                    tracing::warn!(employee = %employee, error = %e, "prepare failed");
                }
            }
        }
        // wait for this pass's bodies before the next claim round: the
        // kernel's own concurrency guard plus our semaphore keep the
        // invariants; the loop itself never stacks unbounded passes
        for h in handles {
            let _ = h.await;
        }
        if errored {
            tracing::warn!(?backoff, "supervise pass errored — backing off");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        } else if !ran {
            tokio::time::sleep(Duration::from_secs(5)).await;
        } else {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
