//! Active-session tracking — ported from hermes-agent
//! `hermes_cli/active_sessions.py`.
//!
//! Enforces a `max_concurrent_sessions` cap on NEW gateway turns. Each
//! distinct session holds a lock file under `<data_dir>/sessions/active/`;
//! when the cap is reached, new sessions are refused while existing holders
//! keep their slots (their locks are refreshed on every message, so a busy
//! session can never be evicted by a new one).
//!
//! Locks idle longer than [`STALE_AFTER_SECS`] are pruned on the next check,
//! so a crashed gateway cannot permanently consume a slot. The check is a
//! cheap directory scan — safe to run on every inbound message.

use crate::error::Error;
use crate::platform;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Locks idle for longer than this are considered stale and pruned.
pub const STALE_AFTER_SECS: u64 = 6 * 60 * 60;

/// Tracks currently-active sessions via per-session lock files.
pub struct ActiveSessionTracker {
    dir: PathBuf,
    max: Option<usize>,
}

impl ActiveSessionTracker {
    /// Create a tracker rooted at `dir` with an optional concurrency cap.
    pub fn new(dir: PathBuf, max: Option<usize>) -> Self {
        Self { dir, max }
    }

    /// The default active-session lock directory.
    pub fn default_dir() -> PathBuf {
        platform::operant_data_dir().join("sessions").join("active")
    }

    /// Try to acquire a slot for `session_id`.
    ///
    /// Returns `Ok(true)` when the session may proceed — either it already
    /// holds a lock (refreshed) or the cap is not yet reached. Returns
    /// `Ok(false)` when the cap is reached by *other* sessions. Never fails
    /// on an `AlreadyExists` race: that lock belongs to us.
    pub fn acquire(&self, session_id: &str) -> Result<bool, Error> {
        fs::create_dir_all(&self.dir)?;
        self.prune_stale()?;
        let current = self.current()?;
        let file = sanitize(session_id);

        if current.iter().any(|s| s == &file) {
            // Already holding a slot — refresh the mtime so it is not pruned.
            let _ = fs::write(self.dir.join(&file), now_millis().to_string());
            return Ok(true);
        }
        if let Some(max) = self.max
            && current.len() >= max
        {
            return Ok(false);
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.dir.join(&file))
        {
            Ok(mut f) => {
                let _ = f.write_all(now_millis().to_string().as_bytes());
                Ok(true)
            }
            // Lost a create race — another request for the same session
            // already holds the slot, which still counts as acquired.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(true),
            Err(e) => Err(Error::from(e)),
        }
    }

    /// Release the slot for `session_id`. No-op when not held.
    pub fn release(&self, session_id: &str) {
        let _ = fs::remove_file(self.dir.join(sanitize(session_id)));
    }

    /// Number of currently-active sessions (after pruning stale locks).
    pub fn count(&self) -> Result<usize, Error> {
        self.prune_stale()?;
        Ok(self.current()?.len())
    }

    fn current(&self) -> Result<Vec<String>, Error> {
        let mut names = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                    && let Some(name) = entry.file_name().to_str()
                {
                    names.push(name.to_string());
                }
            }
        }
        Ok(names)
    }

    fn prune_stale(&self) -> Result<(), Error> {
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata()
                    && let Ok(modified) = meta.modified()
                    && let Ok(elapsed) = modified.elapsed()
                    && elapsed.as_secs() > STALE_AFTER_SECS
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Cross-process presence ─────────────────────────────────────────────
//
// `ActiveSessionTracker` above tracks sessions THIS process owns. The
// interrupted-session marker (`crate::session_interrupted`) plus the ember
// gutter need the opposite view: is a given session id live in some OTHER
// process? Without it, a relaunch offers to resume a session that is still
// running behind it, and two processes write the same session concurrently.
//
// # Why /proc and not libc
//
// `libc` is not a dependency of `operant-core` (only `operant-cli` uses it),
// and the plan forbids adding one for this. `libc::kill(pid, 0)` and
// `std::fs::metadata("/proc/<pid>")` are equivalent for the one question we
// ask — "does this pid exist?" — on Linux. The cost is that EPERM and ENOENT
// are indistinguishable here (both surface as `Ok(None)` / absent), so a live
// process owned by another user can be reported dead. That is acceptable: a
// false "dead" costs a redundant resume notice, while a false "live" would
// wedge a session's self-registration, and we deliberately fail toward the
// redundant notice. A non-Linux target has no `/proc`, so `process_is_running`
// reports false and presence simply does not prune there — the registry stays
// correct, just never garbage-collected.

/// Where the presence records live, inside the operant home.
fn presence_dir() -> PathBuf {
    crate::platform::operant_home().join("session-presence")
}

/// One session's owning process, recorded so another process can see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPresence {
    pub session_id: String,
    /// The pid that registered the session.
    ///
    /// Recorded as text and parsed on read: a stale numeric type in a
    /// long-lived format is a migration, and this is a cache.
    pub pid: u32,
}

fn presence_path(session_id: &str) -> PathBuf {
    // Deliberately reused sanitize semantics rather than a raw join: a session
    // id containing a path separator would escape the directory.
    let safe = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    presence_dir().join(format!("{safe}.json"))
}

/// Whether a process with this pid is running right now.
///
/// See the /proc tradeoff note at the top of this section.
pub fn process_is_running(pid: u32) -> bool {
    std::fs::metadata(format!("/proc/{pid}")).is_ok()
}

/// Record that this process owns `session_id`.
///
/// Overwrites any existing record for that id: the last writer is the owner,
/// and a second process claiming a session is exactly the state being defended
/// against.
pub fn register_presence(session_id: &str, pid: u32) -> Result<(), String> {
    std::fs::create_dir_all(presence_dir()).map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "session_id": session_id,
        "pid": pid,
    });
    std::fs::write(presence_path(session_id), body.to_string()).map_err(|e| e.to_string())
}

/// Remove the presence record for `session_id`.
///
/// Called on a clean exit; an unclean exit leaves the record for
/// `prune_dead_presence` to reclaim.
pub fn unregister_presence(session_id: &str) -> bool {
    std::fs::remove_file(presence_path(session_id)).is_ok()
}

/// Session ids whose presence record names `pid`.
///
/// Used at exit to release every session this process owned, so an exit path
/// does not have to have tracked its own ids.
pub fn presence_session_ids_for_pid(pid: u32) -> Vec<String> {
    let dir = presence_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if let Some(presence) = presence_from_path(&entry.path())
            && presence.pid == pid
        {
            out.push(presence.session_id);
        }
    }
    out
}

fn presence_from_path(path: &std::path::Path) -> Option<SessionPresence> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    Some(SessionPresence {
        session_id: value.get("session_id")?.as_str()?.to_string(),
        pid: value.get("pid")?.as_u64()? as u32,
    })
}

/// The presence record for a session id, if any.
pub fn session_presence(session_id: &str) -> Option<SessionPresence> {
    presence_from_path(&presence_path(session_id))
}

/// Claim `session_id` for THIS process at boot, pruning other processes' dead
/// records on the way.
///
/// This is the single entry point a live process calls, and it exists so the
/// three acts stay together and cannot drift:
///
/// 1. **prune** — drop records whose pid is gone, so stale claims from
///    SIGKILLed runs do not accumulate and so `session_is_live` never answers
///    true for a session nobody owns.
/// 2. **claim** — record this pid as the owner. A second process claiming the
///    same session overwrites: the last writer is the owner, and refusing
///    instead would wedge every relaunch behind a zombie record.
/// 3. **observe** — the previous owner's claim is gone by definition once step
///    1 has pruned it, which is what makes the interrupted status honest: a
///    relaunch can tell "still running somewhere" from "the process died".
///
/// Returns whether the claim was recorded, so a caller can log a no-op instead
/// of asserting a state it did not create.
pub fn claim_session_here(session_id: &str) -> Result<bool, String> {
    let (pruned, _kept) = prune_dead_presence();
    if pruned > 0 {
        tracing::debug!(
            pruned,
            "Pruned dead presence records before claiming a session"
        );
    }
    register_presence(session_id, std::process::id())?;
    Ok(true)
}

/// Whether `session_id` is live in ANOTHER process.
///
/// True only when a record exists AND its process is running. A record with a
/// dead pid is answered false — that is the interruption this registry
/// exists to surface, and it is what makes the stale-record case the useful
/// one.
pub fn session_is_live(session_id: &str) -> bool {
    session_presence(session_id).is_some_and(|p| process_is_running(p.pid))
}

/// Drop records whose process is gone, returning `(pruned, kept)`.
///
/// Best-effort: an unreadable entry is neither pruned nor kept, it is skipped,
/// because corrupt presence data must not stop a relaunch.
pub fn prune_dead_presence() -> (usize, usize) {
    let dir = presence_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (0, 0);
    };
    let mut pruned = 0usize;
    let mut kept = 0usize;
    for entry in entries.flatten() {
        let Some(presence) = presence_from_path(&entry.path()) else {
            continue;
        };
        if process_is_running(presence.pid) {
            kept += 1;
        } else {
            let _ = std::fs::remove_file(entry.path());
            pruned += 1;
        }
    }
    (pruned, kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant_activesessions_test_{}_{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn acquires_until_cap_then_refuses_new() {
        let root = temp_root("cap");
        let tracker = ActiveSessionTracker::new(root.clone(), Some(2));
        assert!(tracker.acquire("user1:chan1").unwrap());
        assert!(tracker.acquire("user2:chan2").unwrap());
        // Cap reached — a third distinct session is refused.
        assert!(!tracker.acquire("user3:chan3").unwrap());
        assert_eq!(tracker.count().unwrap(), 2);
    }

    #[test]
    fn existing_holder_keeps_slot_above_cap() {
        let root = temp_root("holder");
        let tracker = ActiveSessionTracker::new(root.clone(), Some(1));
        assert!(tracker.acquire("user1:chan1").unwrap());
        // New session refused, but the holder is still admitted (refreshed).
        assert!(!tracker.acquire("user2:chan2").unwrap());
        assert!(tracker.acquire("user1:chan1").unwrap());
    }

    #[test]
    fn release_frees_a_slot() {
        let root = temp_root("release");
        let tracker = ActiveSessionTracker::new(root.clone(), Some(1));
        assert!(tracker.acquire("user1:chan1").unwrap());
        tracker.release("user1:chan1");
        assert!(tracker.acquire("user2:chan2").unwrap());
    }

    #[test]
    fn no_cap_means_unlimited() {
        let root = temp_root("unlimited");
        let tracker = ActiveSessionTracker::new(root.clone(), None);
        for i in 0..20 {
            assert!(tracker.acquire(&format!("user{i}:chan{i}")).unwrap());
        }
        assert_eq!(tracker.count().unwrap(), 20);
    }

    #[test]
    fn sanitizes_session_keys_for_filenames() {
        let root = temp_root("sanitize");
        let tracker = ActiveSessionTracker::new(root.clone(), None);
        assert!(tracker.acquire("user/with:slashes and spaces!").unwrap());
        assert_eq!(tracker.count().unwrap(), 1);
    }

    // ── Cross-process presence ────────────────────────────────────────
    //
    // These run against a real temp `HERMES_HOME` under the repo's serial env
    // lock: the registry is a directory under the operant home, so two tests
    // racing on it would read each other's files. The lock is the same one
    // `platform::operant_home` readers elsewhere take.

    /// Run `f` with `HERMES_HOME` pointed at a fresh temp dir, then restore.
    fn with_presence_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let _guard = crate::platform::test_env_lock_blocking();
        let home = temp_root("presence_home");
        let original = std::env::var("HERMES_HOME").ok();
        // SAFETY: test-only env mutation under the serial env guard.
        unsafe { std::env::set_var("HERMES_HOME", &home) };
        let out = f(&home);
        match original {
            Some(val) => unsafe { std::env::set_var("HERMES_HOME", val) },
            None => unsafe { std::env::remove_var("HERMES_HOME") },
        }
        let _ = std::fs::remove_dir_all(&home);
        out
    }

    #[test]
    #[serial_test::serial]
    fn a_registered_session_reports_live_then_released_on_unregister() {
        with_presence_home(|_home| {
            let pid = std::process::id();
            register_presence("sess-live", pid).expect("register");
            assert_eq!(session_presence("sess-live").map(|p| p.pid), Some(pid));
            assert!(
                session_is_live("sess-live"),
                "our own pid is running, so this must read live"
            );
            assert!(
                unregister_presence("sess-live"),
                "unregister reports removal"
            );
            assert!(
                session_presence("sess-live").is_none(),
                "released presence must be gone"
            );
            assert!(!session_is_live("sess-live"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn a_dead_pid_is_not_live_and_prune_reclaims_it() {
        with_presence_home(|_home| {
            // pid 999999 is far above the usual pid_max and cannot exist.
            let dead_pid = 999_999u32;
            assert!(!process_is_running(dead_pid), "sanity: that pid is dead");
            register_presence("sess-dead", dead_pid).expect("register");
            assert!(
                !session_is_live("sess-dead"),
                "a stale record must not read as live"
            );
            let (pruned, kept) = prune_dead_presence();
            assert_eq!(pruned, 1, "the dead record must be pruned");
            assert_eq!(kept, 0, "nothing live to keep");
            assert!(session_presence("sess-dead").is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn presence_ids_for_a_pid_lists_only_that_process() {
        with_presence_home(|_home| {
            let mine = std::process::id();
            register_presence("a-mine", mine).expect("register");
            register_presence("b-mine", mine).expect("register");
            register_presence("c-other", 999_998).expect("register");
            let mut mine_ids = presence_session_ids_for_pid(mine);
            mine_ids.sort();
            assert_eq!(mine_ids, vec!["a-mine".to_string(), "b-mine".to_string()]);
        });
    }

    #[test]
    #[serial_test::serial]
    fn a_session_id_with_path_separators_cannot_escape_the_directory() {
        with_presence_home(|home| {
            let mine = std::process::id();
            register_presence("../escaped", mine).expect("register");
            assert!(
                !home
                    .parent()
                    .expect("home has a parent")
                    .join("escaped.json")
                    .exists(),
                "a sanitized id must not write outside the presence dir"
            );
            assert!(
                session_presence("../escaped").is_some(),
                "lookup by the ORIGINAL id must still work"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn registering_twice_keeps_the_last_writer() {
        with_presence_home(|_home| {
            register_presence("sess-race", 999_997).expect("first");
            register_presence("sess-race", std::process::id()).expect("second");
            let presence = session_presence("sess-race").expect("record");
            assert_eq!(presence.pid, std::process::id(), "last writer wins");
        });
    }

    #[test]
    #[serial_test::serial]
    fn claiming_a_session_here_records_this_process_and_survives_the_prune() {
        // The P0 wiring test. iter-1634 shipped this registry with no caller,
        // so every test above passed while nothing in production ever touched
        // it. This one pins the contract the callers now depend on: a fresh
        // claim records THIS process's pid, and the prune that
        // `claim_session_here` performs does not reclaim the claim it just made
        // (that would be the bug that makes liveness silently always-false).
        with_presence_home(|_home| {
            let claimed =
                claim_session_here("claim-mine").expect("claiming this process's session");
            assert!(claimed, "a fresh claim must report success");

            let presence = session_presence("claim-mine").expect("the claim must be recorded");
            assert_eq!(
                presence.pid,
                std::process::id(),
                "the claim must name the claiming process, not an arbitrary pid"
            );
            assert!(
                session_is_live("claim-mine"),
                "this process is running, so its own claim must read live"
            );

            // The prune inside claim must not eat the claim it just wrote.
            let (pruned, kept) = prune_dead_presence();
            assert_eq!(pruned, 0, "a live claim must never be pruned");
            assert_eq!(kept, 1, "this process's claim must survive");
        });
    }

    #[test]
    #[serial_test::serial]
    fn claiming_prunes_a_dead_owner_s_claim_so_the_interrupted_status_is_honest() {
        // The other half of the contract, and the reason P0 exists at all:
        // after a relaunch claims the session, the DEAD owner's record must be
        // gone, so `session_is_live` can tell "a process is still here" from
        // "the process died". If the dead record survived, a relaunch would
        // read its own session as live-somewhere-else forever.
        with_presence_home(|_home| {
            register_presence("dead-owner", 999_994).expect("register the dead owner");
            assert!(!session_is_live("dead-owner"), "sanity: that pid is dead");

            claim_session_here("dead-owner").expect("relaunch claims the same session");
            let presence = session_presence("dead-owner").expect("the relaunch now owns it");
            assert_eq!(
                presence.pid,
                std::process::id(),
                "the relaunch must replace the dead owner, not share the record"
            );
            assert!(
                session_is_live("dead-owner"),
                "the live relaunch reads live"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn releasing_a_claim_makes_the_session_read_dead_immediately() {
        // What the clean and interrupted exit paths both rely on: once this
        // process releases, the session is NOT live, even though this process
        // is still running. Without that, a session would read live forever
        // after its exit and the interrupted status would never be offered.
        with_presence_home(|_home| {
            claim_session_here("release-me").expect("claim");
            assert!(session_is_live("release-me"));
            assert!(
                unregister_presence("release-me"),
                "release must report removal"
            );
            assert!(
                !session_is_live("release-me"),
                "a released session must not read live"
            );
        });
    }
}
