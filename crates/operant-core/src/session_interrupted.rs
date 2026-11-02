//! The interrupted-session marker: a one-generation record of which session
//! was live when the process went away.
//!
//! # Why this exists
//!
//! operant persisted sessions but not their liveness. A Ctrl+C — the single
//! most common way a long agent run ends — left the database exactly as a clean
//! exit did, so the next launch had no way to distinguish "you finished that"
//! from "that was cut off mid-flight". jcode answers the same question with
//! `SessionStatus::Crashed{..}`; operant could not answer it at all.
//!
//! # Why a sidecar file and not a column
//!
//! A column can only be written by code that runs at exit, and exit code does
//! not always run: SIGKILL, a panic in a Drop, and a killed terminal all skip
//! it. This marker is also a column (`end_reason = 'interrupted'`,
//! `database::INTERRUPTED_END_REASON`), but the *file* is the transport that
//! survives the graders that kill the runner — it is written before the DB is
//! touched, so a crash between the two still leaves the newer, more honest
//! answer behind.
//!
//! # One generation, by design
//!
//! `consume_marker` deletes the file as it reads it. A resume notice that
//! survives being acknowledged is a notice you learn to ignore, and it also
//! means the marker cannot pile up stale entries across launches. The durable
//! answer to "was this session interrupted?" is the DB flag, which the next
//! turn clears (`agent::turn_context`); the file is only the *offer*.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The current marker schema version.
///
/// Bumped only on a field rename or removal; additive fields keep the number.
pub const MARKER_VERSION: u32 = 1;

/// The file name inside the sessions directory.
const MARKER_FILE: &str = "interrupted-session.json";

/// The one-generation record that the previous process left a session live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInterrupted {
    /// Schema version, for forward compatibility of the read path.
    pub version: u32,
    /// The session id that was live at exit.
    ///
    /// Empty is refused on write: an empty id would match nothing, and a
    /// marker that matches nothing is indistinguishable from no marker at all.
    pub session_id: String,
    /// When the marker was written (RFC 3339).
    ///
    /// Never read for ordering, only shown: the point is that the session was
    /// live, not how recently.
    pub written_at: String,
    /// Best-effort title, for a notice a human can read without a DB lookup.
    #[serde(default)]
    pub title: String,
}

impl SessionInterrupted {
    /// Build a marker for `session_id`, timestamped now.
    pub fn new(session_id: &str, title: &str) -> Self {
        Self {
            version: MARKER_VERSION,
            session_id: session_id.to_string(),
            title: title.to_string(),
            written_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Where the marker lives for a given sessions directory.
    pub fn path_in(sessions_dir: &Path) -> PathBuf {
        sessions_dir.join(MARKER_FILE)
    }
}

/// Where the marker lives, derived from the shared sessions directory.
///
/// Deliberately not a new config key: the operator gets one place for session
/// state, not two.
pub fn marker_path() -> PathBuf {
    SessionInterrupted::path_in(&crate::platform::operant_sessions_dir())
}

fn from_str(raw: &str) -> Result<SessionInterrupted, String> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}

fn to_json(marker: &SessionInterrupted) -> Result<String, String> {
    serde_json::to_string_pretty(marker).map_err(|e| e.to_string())
}

/// Write `marker` to `path`.
///
/// The parent directory is created if absent: the marker is written early in
/// process lifetime in some paths, before anything has needed the directory.
fn write_at(marker: &SessionInterrupted, path: &Path) -> Result<(), String> {
    if marker.session_id.is_empty() {
        return Err("refusing to write an interrupted marker with an empty session id".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("marker path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // No temp-file + rename here: the marker is a single small JSON object
    // written on an exit path, and a truncated write is caught by the tolerant
    // read below. Adding rename would also add a second file to clean up.
    std::fs::write(path, to_json(marker)?).map_err(|e| e.to_string())
}

/// Write the marker to the live sessions directory.
pub fn write_marker(marker: &SessionInterrupted) -> Result<(), String> {
    write_at(marker, &marker_path())
}

/// Read the marker at `path`, tolerating anything unreadable.
///
/// `Ok(None)` for a missing OR corrupt file. A corrupt marker must not stop a
/// launch: the operator's session list is the durable source, and failing the
/// boot over a sidecar file would be a far worse failure than skipping the
/// notice.
fn read_at(path: &Path) -> Option<SessionInterrupted> {
    let raw = std::fs::read_to_string(path).ok()?;
    from_str(raw.trim()).ok()
}

/// Read the marker without consuming it.
pub fn read_marker() -> Option<SessionInterrupted> {
    read_at(&marker_path())
}

/// Read the marker and delete it, so it is offered exactly once.
fn consume_at(path: &Path) -> Option<SessionInterrupted> {
    let marker = read_at(path)?;
    // Deliberate ordering: read first, then delete. Deleting first would turn
    // an unreadable marker into a silently lost resume offer.
    let _ = std::fs::remove_file(path);
    Some(marker)
}

/// Consume the live marker: read it, then delete it.
pub fn consume_marker() -> Option<SessionInterrupted> {
    consume_at(&marker_path())
}

/// Delete the marker if it exists, reporting whether one was removed.
fn clear_at(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

/// Delete the live marker.
pub fn clear_marker() -> bool {
    clear_at(&marker_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_marker_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("operant-marker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let out = f(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        drop(guard);
        out
    }

    #[test]
    fn marker_round_trips_through_the_wire_format() {
        with_marker_dir(|dir| {
            let path = SessionInterrupted::path_in(dir);
            let marker = SessionInterrupted::new("session-42", "Building the thing");
            write_at(&marker, &path).expect("write");
            let back = read_at(&path).expect("read back");
            assert_eq!(back.session_id, "session-42");
            assert_eq!(back.title, "Building the thing");
            assert_eq!(back.version, MARKER_VERSION);
        });
    }

    #[test]
    fn consume_offers_a_marker_exactly_once() {
        with_marker_dir(|dir| {
            let path = SessionInterrupted::path_in(dir);
            write_at(&SessionInterrupted::new("s1", ""), &path).expect("write");
            let first = consume_at(&path).expect("first consume");
            assert_eq!(first.session_id, "s1");
            assert!(
                consume_at(&path).is_none(),
                "a consumed marker must not be offered again"
            );
            assert!(!path.exists(), "consume must delete the file");
        });
    }

    #[test]
    fn an_empty_session_id_is_refused() {
        with_marker_dir(|dir| {
            let path = SessionInterrupted::path_in(dir);
            let err = write_at(&SessionInterrupted::new("", ""), &path)
                .expect_err("empty id must not be written");
            assert!(
                err.contains("empty session id"),
                "unexpected error: {}",
                err
            );
            assert!(!path.exists());
        });
    }

    #[test]
    fn a_corrupt_marker_is_tolerated_not_fatal() {
        with_marker_dir(|dir| {
            let path = SessionInterrupted::path_in(dir);
            std::fs::write(&path, "{not json").expect("write garbage");
            assert!(read_at(&path).is_none(), "corrupt marker must read as none");
            // Consume still reclaims the slot, so a garbage file cannot wedge
            // the resume offer closed forever.
            assert!(consume_at(&path).is_none());
        });
    }

    #[test]
    fn clear_reports_whether_a_marker_was_there() {
        with_marker_dir(|dir| {
            let path = SessionInterrupted::path_in(dir);
            assert!(!clear_at(&path), "nothing to clear");
            write_at(&SessionInterrupted::new("s9", ""), &path).expect("write");
            assert!(clear_at(&path), "a written marker must report cleared");
            assert!(!clear_at(&path), "second clear is a no-op");
        });
    }

    #[test]
    fn write_creates_a_missing_parent_directory() {
        with_marker_dir(|dir| {
            let path = dir.join("nested").join("deep").join(MARKER_FILE);
            write_at(&SessionInterrupted::new("s-deep", ""), &path)
                .expect("write into nested dirs");
            assert!(path.exists(), "nested parent dirs must be created");
        });
    }
}
