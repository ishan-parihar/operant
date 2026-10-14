//! The supervisor protocol (CONTRACT §11): prepare → heartbeat → finish.
//!
//! Operant is the resident runtime that owns processes. A Holon organization
//! runs on it without a resident Python dispatcher: the supervisor claims
//! through the kernel (`holon dispatch --prepare --json`), runs the returned
//! driver command itself (under the returned env, which carries
//! HOLON_RUN_TOKEN and this run's policy), renews the lease while the body
//! runs (`holon run heartbeat`), and hands the body in (`holon run finish`).
//! A prepared run never finished reverts by lease expiry — the supervisor
//! crashing is safe by construction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;

use crate::{Error, Result};

/// What `holon dispatch --prepare --json` prints under `"prepared"`.
#[derive(Debug, Clone, Deserialize)]
pub struct Prepared {
    pub run: String,
    pub stage: String,
    pub task: String,
    pub employee: String,
    pub driver: String,
    /// The rendered driver command — a shell line; run it with `sh -c`.
    pub command: String,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub timeout_s: u64,
    pub brief_md: PathBuf,
    pub result_path: PathBuf,
    pub policy_json: PathBuf,
    pub lease_until: String,
}

/// How operant invokes the kernel. `[holon] bin` pins a launcher; the PATH
/// `holon` is the fallback. Every protocol call carries `--home` so the
/// supervisor cannot act on the wrong instance.
pub fn holon_cmd(config_home: &Path, bin: Option<&str>) -> Command {
    let launcher = bin
        .map(str::to_string)
        .or_else(|| std::env::var("HOLON_BIN").ok())
        .unwrap_or_else(|| "holon".to_string());
    let mut cmd = Command::new(launcher);
    cmd.arg("--home").arg(config_home).env_remove("HOLON_HOME"); // --home is explicit; never inherit a stray one
    cmd
}

/// Claim one stage for one employee. `Ok(None)` = nothing to do (the kernel
/// prints `{"prepared": null}`).
pub async fn prepare(
    config_home: &Path,
    bin: Option<&str>,
    employee: &str,
    stage: Option<&str>,
    task: Option<&str>,
) -> Result<Option<Prepared>> {
    let mut cmd = holon_cmd(config_home, bin);
    // --json is a kernel-wide flag: argparse wants it BEFORE the subcommand
    cmd.arg("--json");
    cmd.args(["dispatch", "--as", employee]);
    if let Some(stage) = stage {
        cmd.args(["--stage", stage]);
    }
    if let Some(task) = task {
        cmd.args(["--task", task]);
    }
    cmd.args(["--prepare"]);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| Error::Protocol(format!("dispatch --prepare: {e}")))?;
    // the kernel's NOTHING convention (cli.py: rc 3) prints a valid
    // {"prepared": null} — that is "nothing to do", not a failure
    if out.status.code() == Some(3) {
        return Ok(None);
    }
    if !out.status.success() {
        return Err(Error::Protocol(format!(
            "dispatch --prepare exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    #[derive(Deserialize)]
    struct Shell {
        prepared: Option<Prepared>,
    }
    let shell: Shell = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Protocol(format!("dispatch --prepare output: {e}")))?;
    Ok(shell.prepared)
}
/// Renew the lease while the body runs. The token authenticates the
/// supervisor's right to speak for this run.
pub async fn heartbeat(
    config_home: &Path,
    bin: Option<&str>,
    run: &str,
    token: &str,
) -> Result<()> {
    let mut cmd = holon_cmd(config_home, bin);
    cmd.args(["run", "heartbeat", run]);
    cmd.env("HOLON_RUN_TOKEN", token);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| Error::Protocol(format!("run heartbeat: {e}")))?;
    if !out.status.success() {
        return Err(Error::Protocol(format!(
            "run heartbeat exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Hand the finished body in: salvage result.json, submit, run checks and
/// the judge in the kernel's short process.
///
/// Auth: `run finish` is a kernel/chair-actor command — a run token would
/// resolve to the run's employee and be refused. The supervisor's operator
/// environment (chair key in ~/.config/holon/keys, or HOLON_KEY) must carry
/// the right to speak for the kernel; we deliberately do NOT send the token.
pub async fn finish(config_home: &Path, bin: Option<&str>, run: &str, exit: i32) -> Result<()> {
    let mut cmd = holon_cmd(config_home, bin);
    cmd.args(["--json", "run", "finish", run, "--exit", &exit.to_string()]);
    // never propagate a stray run token into the supervisor's own calls —
    // prepare/finish speak as the kernel's operator (chair), not as the run
    cmd.env_remove("HOLON_RUN_TOKEN");
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| Error::Protocol(format!("run finish: {e}")))?;
    if !out.status.success() {
        return Err(Error::Protocol(format!(
            "run finish exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// One tick of the kernel's heartbeat: sweeps, notifications, due
/// schedules. The clock fires this; the supervisor does not.
pub async fn tick(config_home: &Path, bin: Option<&str>) -> Result<()> {
    let mut cmd = holon_cmd(config_home, bin);
    cmd.arg("tick");
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| Error::Protocol(format!("tick: {e}")))?;
    if !out.status.success() {
        return Err(Error::Protocol(format!(
            "tick exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}
