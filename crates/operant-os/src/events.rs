//! Event intake (CONTRACT §12): match what the runtime saw against the
//! organization's event triggers, fire the ones that match.
//!
//! Holon never matches events itself — the runtime that fronts a channel or
//! hosts a webhook does. This is operant's half: read `api_triggers` (rows
//! with `trigger_kind = 'event'`), match each enabled pattern against the
//! event's text, and call `holon routine fire <schedule> --payload`. The
//! kernel enforces the cooldown and the skip-if-open guard; firing twice is
//! the kernel's to refuse, not ours to fear.

use std::path::Path;

use crate::api::HolonApi;
use crate::run::holon_cmd;

/// One matched event → one firing attempt. Returns how many routines fired.
///
/// Pattern semantics are the kernel's `routine add --on event:<pattern>`:
/// a plain substring match (case-insensitive) of the pattern in the event
/// text — deliberately boring; a pattern that needs more becomes a
/// classifier in Holon, not a regex dialect here.
pub async fn match_and_fire(
    config_home: &Path,
    bin: Option<&str>,
    event_text: &str,
) -> crate::Result<usize> {
    let api = HolonApi::open(config_home)?;
    let triggers = api.triggers()?;
    let mut fired = 0usize;
    for t in triggers
        .iter()
        .filter(|t| t.enabled && t.trigger_kind == "event")
    {
        if !event_text
            .to_lowercase()
            .contains(&t.pattern.to_lowercase())
        {
            continue;
        }
        // O-09: untrusted channel text never rides argv — Holon's text
        // arguments read `@path` and `-` as file/stdin as the calling
        // user, so an argv payload of "@/home/.../model.key" would make
        // the kernel read that file into the work's message. STDIN with
        // `-` is the only safe channel for event text.
        let mut cmd = holon_cmd(config_home, bin);
        cmd.args(["routine", "fire", &t.id, "--payload", "-"]);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        use tokio::io::AsyncWriteExt;
        let mut child = cmd
            .spawn()
            .map_err(|e| crate::Error::Protocol(format!("routine fire {}: {e}", t.id)))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(event_text.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| crate::Error::Protocol(format!("routine fire {}: {e}", t.id)))?;
        if out.status.success() {
            fired += 1;
        } else {
            tracing::warn!(
                schedule = %t.id,
                status = ?out.status.code(),
                "routine fire refused (cooldown or open work): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    Ok(fired)
}
