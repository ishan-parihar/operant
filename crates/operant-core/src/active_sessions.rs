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
}
