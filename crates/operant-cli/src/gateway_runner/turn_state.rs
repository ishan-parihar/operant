//! Turn-state tracking for auto-continue / interruption recovery, split out
//! of `gateway_runner.rs` (2026-10-25 ponytail audit finding 5). One JSON
//! file per channel under `<operant_home>/.turn_state/`, so concurrent turns
//! on different channels do not overwrite each other. Items are
//! `pub(super)` and glob-re-exported by the parent.

// ── Turn state tracking for auto-continue / interruption recovery ────────

/// Persist turn state so interrupted sessions can be detected on restart.
///
/// Writes to `<operant_home>/.turn_state/<channel_id>.json` — one file per
/// channel — so concurrent turns on different channels don't overwrite each
/// other. Previously this wrote a single `.turn_state.json` that got
/// clobbered by whichever channel wrote last, so a crash during channel A's
/// turn would be invisible if channel B completed its turn before the
/// restart.
pub(super) fn save_turn_state(channel_id: &str, status: &str) {
    save_turn_state_in(&operant_core::platform::operant_home(), channel_id, status);
}

/// Terminal close with the machine-readable exit class (iter-691) — the
/// journal's exit_code/exit_reason columns. The platform defaults to
/// telegram like every legacy writer here; the restart-reaper close path
/// calls `save_turn_state_in_for` directly with the recorded platform.
pub(super) fn save_turn_state_exit(
    channel_id: &str,
    status: &str,
    exit_code: operant_core::gateway::lifecycle::TurnExitReason,
    exit_reason: &str,
) {
    save_turn_state_in_for(
        &operant_core::platform::operant_home(),
        channel_id,
        status,
        "telegram",
        Some(exit_code),
        Some(exit_reason),
    );
}

/// Same as `save_turn_state` but writes to a caller-provided base directory.
/// Exposed for tests so they don't pollute the real operant home.
pub(super) fn save_turn_state_in(base_dir: &std::path::Path, channel_id: &str, status: &str) {
    save_turn_state_in_for(base_dir, channel_id, status, "telegram", None, None);
}

/// Persist turn state, recording the originating platform.
///
/// The platform is part of the record because the boot-time interrupted-turn
/// notice has to deliver on the same platform the turn arrived on. Without it,
/// a Discord or Slack channel_id would be notified through Telegram.
///
/// `exit_code`/`exit_reason` (iter-691 — the journal's exit columns) are
/// written on terminal closes: the machine-readable class a retry or
/// breaker can branch on, and the free-form human reason. `None` keeps
/// them absent (open turns, back-compat readers).
pub(super) fn save_turn_state_in_for(
    base_dir: &std::path::Path,
    channel_id: &str,
    status: &str,
    platform: &str,
    exit_code: Option<operant_core::gateway::lifecycle::TurnExitReason>,
    exit_reason: Option<&str>,
) {
    let dir = base_dir.join(".turn_state");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", sanitize_channel_id(channel_id)));
    let ts = chrono::Utc::now().to_rfc3339();
    let mut json = serde_json::json!({
        "channel_id": channel_id,
        "status": status,
        "timestamp": ts,
        "platform": platform,
    });
    if let Some(code) = exit_code {
        json["exit_code"] = serde_json::to_value(code).unwrap_or(serde_json::json!("unknown"));
    }
    if let Some(reason) = exit_reason {
        json["exit_reason"] = serde_json::Value::from(reason);
    }
    let _ = std::fs::write(path, json.to_string());
}

/// Check for interrupted turns on startup and log a warning for each.
///
/// Scans every file under `<operant_home>/.turn_state/` so all concurrent
/// channels are reported, not just whichever one happened to win the
/// last-write race on the old single-file scheme.
/// Scan for turns left `pending` by a previous process and return them
/// WITH their originating platform, so the caller can deliver the
/// interrupted-turn notice on the channel the turn actually arrived on.
///
/// This replaces the old `check_interrupted_turns()`, which logged a warning
/// naming the channel and then discarded the result — leaving the user with
/// silence and no explanation. State files are rewritten with a terminal
/// status ("complete"/"failed") when a turn finishes, so a turn is reported
/// once, not on every boot.
pub fn collect_interrupted_turns() -> Vec<(String, String, String)> {
    check_interrupted_turns_in(&operant_core::platform::operant_home())
}

/// Same as `check_interrupted_turns` but reads from a caller-provided base
/// directory. Exposed for tests.
pub(super) fn check_interrupted_turns_in(
    base_dir: &std::path::Path,
) -> Vec<(String, String, String)> {
    let dir = base_dir.join(".turn_state");
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return found,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let state = match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if state.get("status").and_then(|s| s.as_str()) == Some("pending") {
            let channel_id = state["channel_id"].as_str().unwrap_or("").to_string();
            let timestamp = state["timestamp"].as_str().unwrap_or("").to_string();
            // Older state files predate the `platform` field; telegram is the
            // only adapter that recorded turn state until now.
            let platform = state
                .get("platform")
                .and_then(|p| p.as_str())
                .unwrap_or("telegram")
                .to_string();
            tracing::warn!(
                channel_id = %channel_id,
                timestamp = %timestamp,
                "Detected interrupted turn from previous session"
            );
            found.push((channel_id, timestamp, platform));
        }
    }
    found
}

/// Sanitize a channel ID for use as a filename. Channel IDs are typically
/// alphanumeric (Slack channel IDs, Discord channel IDs, etc.) but we
/// defensively strip anything that isn't alphanumeric, dash, or underscore
/// to prevent path traversal if a platform ever sends a hostile ID.
pub(super) fn sanitize_channel_id(channel_id: &str) -> String {
    channel_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    // This suite also pins `resolve_background_target`, which stayed in the
    // parent (it is background-target policy, not turn state).
    use crate::gateway_runner::resolve_background_target;

    #[test]
    fn save_turn_state_writes_one_file_per_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "channel_a", "pending");
        save_turn_state_in(base, "channel_b", "pending");
        save_turn_state_in(base, "channel_a", "complete");

        let dir = base.join(".turn_state");
        let a_path = dir.join("channel_a.json");
        let b_path = dir.join("channel_b.json");
        assert!(a_path.exists(), "channel_a file should exist");
        assert!(b_path.exists(), "channel_b file should exist");

        // channel_a was written pending then complete — final state should
        // be "complete".
        let a_content = std::fs::read_to_string(&a_path).unwrap();
        let a_json: serde_json::Value = serde_json::from_str(&a_content).unwrap();
        assert_eq!(a_json["status"], "complete");
        assert_eq!(a_json["channel_id"], "channel_a");

        // channel_b was only written pending.
        let b_content = std::fs::read_to_string(&b_path).unwrap();
        let b_json: serde_json::Value = serde_json::from_str(&b_content).unwrap();
        assert_eq!(b_json["status"], "pending");
        assert_eq!(b_json["channel_id"], "channel_b");
    }

    #[test]
    fn concurrent_channels_do_not_overwrite_each_other() {
        // This is the core regression test for the old single-file bug:
        // two channels with concurrent pending turns must both be visible
        // to check_interrupted_turns_in, not just whichever one wrote last.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "slack_channel_1", "pending");
        save_turn_state_in(base, "discord_channel_2", "pending");

        let interrupted = check_interrupted_turns_in(base);
        assert_eq!(
            interrupted.len(),
            2,
            "both channels should be reported as interrupted, got {:?}",
            interrupted
        );

        let channel_ids: Vec<&str> = interrupted.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(channel_ids.contains(&"slack_channel_1"));
        assert!(channel_ids.contains(&"discord_channel_2"));
    }

    /// iter-812 — background events (advisor/recap/self-improvement notes)
    /// must route to the SESSION's channel, never the global last-active
    /// slot when both exist and disagree. This is the wrong-thread bug:
    /// a note from session A (topic 42) fired after session B's turn
    /// started, and the old single-slot `current_channel` delivered it
    /// into B's thread (observed live 2026-10-14).
    #[test]
    fn background_events_route_to_the_sessions_own_channel() {
        let sess_a_pin = ("telegram".to_string(), "chat-1".to_string(), Some(42));
        let other_slot = ("telegram".to_string(), "chat-1".to_string(), Some(7));

        // The owning session's pin wins over a disagreeing global slot.
        let target =
            resolve_background_target("gw_a", Some(sess_a_pin.clone()), Some(other_slot.clone()));
        assert_eq!(target, Some(sess_a_pin.clone()), "session pin must win");

        // No pin (entry expired with the turn) — fall back to the slot.
        let target = resolve_background_target("gw_a", None, Some(other_slot.clone()));
        assert_eq!(
            target,
            Some(other_slot.clone()),
            "fallback to the active slot"
        );

        // Legacy emitters with an empty tag: slot only.
        let target = resolve_background_target("", Some(sess_a_pin), Some(other_slot.clone()));
        assert_eq!(
            target,
            Some(("telegram".to_string(), "chat-1".to_string(), Some(7))),
            "empty tag ignores pins"
        );

        // Cold boot: nothing anywhere — drop.
        assert_eq!(resolve_background_target("gw_a", None, None), None);
    }

    /// iter-691 — the restart reaper's CLOSE path. A pending row closed as
    /// `interrupted` (with its exit columns) must never be re-detected on
    /// the next boot, even though the channel notice may never have been
    /// delivered — the close is no longer gated on delivery succeeding.
    #[test]
    fn closed_interrupted_turn_is_not_redictected_and_carries_exit_columns() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "dead_channel", "pending");
        assert_eq!(
            check_interrupted_turns_in(base).len(),
            1,
            "a pending row is detected once"
        );

        // The iter-691 close at detection time: status interrupted + the
        // machine-readable exit columns.
        save_turn_state_in_for(
            base,
            "dead_channel",
            "interrupted",
            "telegram",
            Some(operant_core::gateway::lifecycle::TurnExitReason::Interrupted),
            Some("gateway restarted before the turn finished"),
        );

        assert!(
            check_interrupted_turns_in(base).is_empty(),
            "a closed row must not read as still in-flight after restart"
        );

        let content =
            std::fs::read_to_string(base.join(".turn_state").join("dead_channel.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["status"], "interrupted");
        assert_eq!(
            json["exit_code"], "Interrupted",
            "the exit_code column carries the machine-readable class"
        );
        assert!(
            json["exit_reason"]
                .as_str()
                .unwrap_or("")
                .contains("restarted"),
            "the exit_reason column carries the human reason"
        );
    }

    /// The boot-time notice delivers on the platform the turn arrived on. The
    /// state file now records it, so a Discord turn is not announced through
    /// Telegram.
    #[test]
    fn interrupted_turn_records_originating_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in_for(base, "dc_chan", "pending", "discord", None, None);
        save_turn_state_in_for(base, "tg_chan", "pending", "telegram", None, None);

        let interrupted = check_interrupted_turns_in(base);
        let by_channel: std::collections::HashMap<_, _> = interrupted
            .iter()
            .map(|(c, _, p)| (c.as_str(), p.as_str()))
            .collect();
        assert_eq!(by_channel.get("dc_chan"), Some(&"discord"));
        assert_eq!(by_channel.get("tg_chan"), Some(&"telegram"));
    }

    /// A state file written before the `platform` field existed must still be
    /// reported, with telegram as the fallback.
    #[test]
    fn interrupted_turn_defaults_platform_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        std::fs::create_dir_all(base.join(".turn_state")).unwrap();
        std::fs::write(
            base.join(".turn_state").join("legacy.json"),
            r#"{"channel_id":"legacy","status":"pending","timestamp":"2026-09-29T00:00:00Z"}"#,
        )
        .unwrap();

        let interrupted = check_interrupted_turns_in(base);
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].0, "legacy");
        assert_eq!(
            interrupted[0].2, "telegram",
            "legacy file falls back to telegram"
        );
    }

    #[test]
    fn check_interrupted_turns_skips_complete_status() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "completed_chan", "pending");
        save_turn_state_in(base, "completed_chan", "complete");
        save_turn_state_in(base, "still_pending", "pending");

        let interrupted = check_interrupted_turns_in(base);
        // Only "still_pending" should be reported — "completed_chan" was
        // marked complete after its pending state. This is also what keeps
        // the boot-time notice from re-firing on every restart.
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].0, "still_pending");
    }

    #[test]
    fn check_interrupted_turns_returns_empty_when_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No save_turn_state_in calls — directory doesn't exist.
        let interrupted = check_interrupted_turns_in(tmp.path());
        assert!(interrupted.is_empty());
    }

    #[test]
    fn sanitize_channel_id_strips_path_separators() {
        // Defensive: a hostile platform sending "../../etc/passwd" as a
        // channel ID must not escape the .turn_state directory.
        assert_eq!(sanitize_channel_id("../../etc/passwd"), "______etc_passwd");
        assert_eq!(
            sanitize_channel_id("normal-channel_123"),
            "normal-channel_123"
        );
        assert_eq!(sanitize_channel_id(""), "");
    }
}
