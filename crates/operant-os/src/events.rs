//! Event intake (CONTRACT §13): match what the runtime saw against the
//! organization's event triggers, fire the ones that match.
//!
//! Holon never matches events itself — the runtime that fronts a channel or
//! hosts a webhook does. This is operant's half: read `api_triggers` (rows
//! with `trigger_kind = 'event'`), match each enabled pattern against the
//! event's text, and call `holon routine fire <schedule> --payload <text>`.
//! The kernel enforces the cooldown and the skip-if-open guard; firing twice
//! is the kernel's to refuse, not ours to fear.

use std::path::Path;

use crate::api::HolonApi;
use crate::run::holon_cmd;

/// One matched event → one firing attempt. Returns how many routines fired.
///
/// Pattern semantics are CONTRACT §13's: the row's pattern is the raw
/// `--on event:<pattern>` value, where the `event:` marker is the trigger
/// KIND's syntax — what is matched against the event text is the pattern
/// after it, as a plain case-insensitive substring. Deliberately boring; a
/// pattern that needs more becomes a classifier in Holon, not a regex
/// dialect here.
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
        // CONTRACT §13: `event:` is the kind marker from `--on event:<pattern>`,
        // not text to find in the event. A row that does not carry the marker
        // does not speak the documented syntax — fail closed (no firing)
        // rather than guessing at what it might have meant. (The 2026-10-17
        // red-team caught this function matching the marker itself: with the
        // raw `event:…` value as the needle, no real channel text could ever
        // contain it and intake never fired.)
        let Some(pattern) = t.pattern.strip_prefix("event:") else {
            continue;
        };
        if !event_text.to_lowercase().contains(&pattern.to_lowercase()) {
            continue;
        }
        // O-09, post-H-06: untrusted channel text rides argv as ONE literal
        // token — exec-style, no shell — which is CONTRACT §13's own form
        // (`holon routine fire <id> --payload 'what the event carried'`).
        // The kernel (8eff8b7) takes --payload literally and NEVER as a
        // file reference, so an "@/home/…/model.key" payload posts as text;
        // the pre-H-06 stdin contract this spoke before ("--payload -")
        // delivered the literal "-" and lost the event text — caught by the
        // 2026-10-17 red-team, pinned by tests/events.rs. A text too large
        // for one argv token (E2BIG at ~128KiB) fails the spawn loudly and
        // degrades to the caller's warn rather than corrupting the payload.
        let mut cmd = holon_cmd(config_home, bin);
        cmd.args(["routine", "fire", &t.id, "--payload", event_text]);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| crate::Error::Protocol(format!("routine fire {}: {e}", t.id)))?;
        // no stdin: the payload is on argv; a piped stdin nobody reads
        // would be a silent contract break again
        child.stdin.take();
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| crate::Error::Protocol(format!("routine fire {}: {e}", t.id)))?;
        // H-13: the kernel reports a cooldown skip as rc 3 (NOTHING) —
        // a skip is NOT a firing; only rc 0 counts (O-15's fix: the
        // counter feeds the operator's "fired once per cooldown" check).
        if out.status.success() {
            fired += 1;
        } else if out.status.code() != Some(3) {
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
