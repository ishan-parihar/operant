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

/// Run one `holon` CLI probe and collect its output. ETXTBSY retry: a
/// launcher script just installed (or a bin the operator is redeploying)
/// can be exec'd while the file's write is still settling — a transient
/// tmpfs condition, observed under the parallel test harness. A 100ms
/// pause and one retry is correct for an idempotent read; a second
/// failure is a real failure.
async fn run_holon(
    config_home: &Path,
    bin: Option<&str>,
    args: &[&str],
) -> crate::Result<std::process::Output> {
    let run_once = || async {
        let mut cmd = holon_cmd(config_home, bin);
        cmd.args(args);
        cmd.stdin(std::process::Stdio::null()).output().await
    };
    let mut attempt = run_once().await;
    if let Err(e) = &attempt
        && e.to_string().contains("os error 26")
    {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        attempt = run_once().await;
    }
    attempt
        .map_err(|e| crate::Error::Protocol(format!("holon {}: {e}", args.first().unwrap_or(&""))))
}

/// The Chair's channel refs, as the organization configured them.
/// Degradation, not death: an unreadable config or CLI failure is an empty
/// set — an unverifiable sender is chat, never Chair.
pub async fn chair_channels(config_home: &Path, bin: Option<&str>) -> crate::Result<Vec<String>> {
    // `holon org` is a chair/any-actor read; asking through the CLI keeps
    // one source of truth for the config (no second [chair] parser here)
    let out = run_holon(config_home, bin, &["--json", "chair-channels"]).await?;
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
    let mut all = vec!["--json"];
    all.extend_from_slice(command);
    let out = run_holon(config_home, bin, &all)
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

/// W5 (O-14): a Chair message that names a decision ("approve `<ref>`",
/// "reject `<ref>` …") routes straight to the kernel's own decision
/// commands through `run_as_chair` — the acceptance's exact scenario:
/// "approve `<id>`" from the Chair approves a pending plan; the same
/// text from anyone else is chat (the gateway's `is_chair` gate, before
/// this function is ever reached). Anything else falls through to the
/// classifier as before. The `ref` is passed as a single argv token —
/// untrusted text never reaches argv (O-09): only the decision verb and
/// the reference id, both re-validated by the kernel.
fn chair_decision_command(text: &str) -> Option<Vec<String>> {
    // tokens: a leading "chair," address is tolerated (the Chair may
    // speak to the org, not just at it)
    let body = text
        .strip_prefix("chair,")
        .or_else(|| text.strip_prefix("chair:"))
        .unwrap_or(text)
        .trim();
    let mut parts = body.split_whitespace();
    let verb = parts.next()?;
    let r#ref = parts.next()?;
    // a decision ref is opaque (plan/work/approval id) — one token, no
    // spaces; anything longer is chat, not a command
    if r#ref.contains(char::is_whitespace) || r#ref.len() > 100 {
        return None;
    }
    match verb.to_ascii_lowercase().as_str() {
        "approve" => Some(vec![
            "plan".to_string(),
            "approve".to_string(),
            r#ref.to_string(),
        ]),
        "reject" => Some(vec![
            "plan".to_string(),
            "reject".to_string(),
            r#ref.to_string(),
        ]),
        _ => None,
    }
}

/// W5 (O-14): route one Chair message. A decision verb routes to the
/// kernel's decision commands; everything else is classified (the
/// kernel owns the classifier). A command runs through [`run_as_chair`]
/// (the kernel's own auth still gates it); a note returns the
/// classifier's suggestion so the Chair sees what the organization
/// made of it. Unknown shapes are the kernel's error, surfaced verbatim —
/// the Chair can read them.
pub async fn dispatch_message(
    config_home: &Path,
    bin: Option<&str>,
    text: &str,
) -> crate::Result<String> {
    // O-14: the decision verbs are commands, not classifier traffic —
    // "approve <ref>" approves; the note text after the ref rides --note
    // for reject (the kernel requires a note there)
    if let Some(mut argv) = chair_decision_command(text) {
        let body = text
            .strip_prefix("chair,")
            .or_else(|| text.strip_prefix("chair:"))
            .unwrap_or(text)
            .trim();
        let note = body
            .split_whitespace()
            .skip(2)
            .collect::<Vec<_>>()
            .join(" ");
        if argv[1] == "reject" && !note.is_empty() {
            argv.push("--note".to_string());
            argv.push(note);
        }
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        return run_as_chair(config_home, bin, &refs).await;
    }
    // classify: `title` is a plain positional (the kernel passes it raw
    // — it is never read as a file, so the O-09 file-naming risk does not
    // apply; argv length still bounds it) and `--intent` rides stdin.
    // The Chair's message is both: what it announces and what it wants.
    let title: String = text.chars().take(200).collect();
    // ETXTBSY retry — same transient as run_holon; classify needs stdin
    // piped so it gets its own spawn-with-retry shape.
    let spawn_once = || {
        let mut cmd = holon_cmd(config_home, bin);
        cmd.args(["--json", "classify", &title, "--intent", "-"]);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.spawn()
    };
    let mut spawned = spawn_once();
    if let Err(e) = &spawned
        && e.to_string().contains("os error 26")
    {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        spawned = spawn_once();
    }
    use tokio::io::AsyncWriteExt;
    let mut child = spawned.map_err(|e| crate::Error::Protocol(format!("classify spawn: {e}")))?;
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
