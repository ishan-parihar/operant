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
        let mut cmd = holon_cmd(config_home, bin);
        cmd.args(["routine", "fire", &t.id, "--payload", event_text]);
        let out = cmd
            .stdin(std::process::Stdio::null())
            .output()
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
