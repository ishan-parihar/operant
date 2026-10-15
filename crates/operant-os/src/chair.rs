//! The Chair's channel (H8): a message from a `[chair] channels` identity
//! runs as the Chair; anyone else is chat as usual.
//!
//! Operant fronts channels (the gateway); Holon names which identities are
//! the Chair's (`[chair] channels = ["telegram:123456", …]`, its GENOME §6).
//! This module is the verification + dispatch half that lives in the
//! runtime: resolve the sender's `<platform>:<user id>`, check it against
//! the organization's configured Chair channels (read once per message
//! through the kernel accessor — `[chair]` is org config, not operant's),
//! and pass the message's command to `holon` as the Chair. The broker still
//! refuses agents every Chair command — this path is for the owner's own
//! messages arriving on a channel operant fronts, never for a run.

use std::path::Path;

use crate::run::holon_cmd;

/// The Chair's channel refs, as the organization configured them.
/// Degradation, not death: an unreadable config or CLI failure is an empty
/// set — an unverifiable sender is chat, never Chair.
pub async fn chair_channels(config_home: &Path, bin: Option<&str>) -> crate::Result<Vec<String>> {
    // `holon org` is a chair/any-actor read; asking through the CLI keeps
    // one source of truth for the config (no second [chair] parser here)
    let mut cmd = holon_cmd(config_home, bin);
    cmd.args(["--json", "chair-channels"]);
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| crate::Error::Protocol(format!("chair-channels: {e}")))?;
    if !out.status.success() {
        tracing::warn!("chair-channels unavailable — treating sender as chat");
        return Ok(Vec::new());
    }
    #[derive(serde::Deserialize)]
    struct Shell {
        channels: Vec<String>,
    }
    match serde_json::from_slice::<Shell>(&out.stdout) {
        Ok(shell) => Ok(shell.channels),
        Err(e) => {
            tracing::warn!(error = %e, "chair-channels output unreadable — treating sender as chat");
            Ok(Vec::new())
        }
    }
}

/// Whether a sender IS the Chair on this platform. Degradation: an
/// unreadable channel list means nobody is the Chair.
pub async fn is_chair(
    config_home: &Path,
    bin: Option<&str>,
    platform: &str,
    user_id: &str,
) -> crate::Result<bool> {
    let want = format!("{platform}:{user_id}");
    Ok(chair_channels(config_home, bin)
        .await?
        .iter()
        .any(|c| c == &want))
}

/// Run one command as the Chair. Only after [`is_chair`] said yes: the
/// caller owns that check (and the gateway's own identity verification);
/// this function does not re-verify, it dispatches.
pub async fn run_as_chair(
    config_home: &Path,
    bin: Option<&str>,
    command: &[&str],
) -> crate::Result<String> {
    let mut cmd = holon_cmd(config_home, bin);
    cmd.arg("--json");
    cmd.args(command);
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| crate::Error::Protocol(format!("chair command: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(crate::Error::Protocol(format!(
            "chair command refused: {}",
            if stderr.is_empty() { &stdout } else { &stderr }
        )));
    }
    Ok(stdout)
}

/// W5 (O-14): route one Chair message. The kernel owns the classifier —
/// `holon classify --title - --intent -` (stdin text, never argv) says
/// whether this is a command or a note. A command runs through
/// [`run_as_chair`] (the kernel's own auth still gates it); a note
/// returns the classifier's suggestion so the Chair sees what the
/// organization made of it. Unknown shapes are the kernel's error,
/// surfaced verbatim — the Chair can read them.
pub async fn dispatch_message(
    config_home: &Path,
    bin: Option<&str>,
    text: &str,
) -> crate::Result<String> {
    // classify: `title` is a plain positional (the kernel passes it raw
    // — it is never read as a file, so the O-09 file-naming risk does not
    // apply; argv length still bounds it) and `--intent` rides stdin.
    // The Chair's message is both: what it announces and what it wants.
    let title: String = text.chars().take(200).collect();
    let mut cmd = holon_cmd(config_home, bin);
    cmd.args(["--json", "classify", &title, "--intent", "-"]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    use tokio::io::AsyncWriteExt;
    let mut child = cmd
        .spawn()
        .map_err(|e| crate::Error::Protocol(format!("classify spawn: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| crate::Error::Protocol(format!("classify: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(crate::Error::Protocol(format!(
            "classify refused: {}",
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                &stderr
            }
        )));
    }
    Ok(stdout.trim().to_string())
}
