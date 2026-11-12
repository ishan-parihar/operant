//! The gateway daemon's state file (RT-523).
//!
//! The delivery ledger (500 entries) and the queue-wait stats live in RAM,
//! so they die with the process: after a restart "how many replies failed?"
//! and "how many turns waited behind another chat?" were unanswerable, and
//! the only evidence of a bad run was the journal.
//!
//! One small JSON file in the operant home carries the numbers, written on
//! every turn end and on shutdown. It is a STATUS file, not a log: it is
//! overwritten, never appended, and a reader that finds it stale can say so
//! (`updated_at`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file name inside the operant home.
pub const STATUS_FILE_NAME: &str = "gateway.status.json";

/// The daemon's state, as another process can read it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayStatus {
    /// When the file was written (RFC 3339, UTC).
    pub updated_at: String,
    /// How many outbound sends were delivered.
    pub deliveries: u64,
    /// How many outbound sends failed.
    pub failed_deliveries: u64,
    /// How many turns had to wait for another chat's turn.
    pub queue_waits: u64,
    /// The total wall time spent waiting, in milliseconds.
    pub queue_wait_total_ms: u64,
    /// The worst single wait, in milliseconds — the tail a user feels.
    pub queue_wait_worst_ms: u64,
    /// How many agent slots the gateway runs (RT-508).
    pub agent_slots: usize,
}

/// Where the state file lives: the operant home (the ONE resolver —
/// `HERMES_HOME` → the sticky profile → `~/.operant`).
pub fn status_file_path() -> PathBuf {
    crate::platform::operant_home().join(STATUS_FILE_NAME)
}

/// Write the state file.
///
/// Best-effort: a status file that cannot be written must never take the
/// daemon down. The write goes to a temporary file in the same directory
/// and is renamed over the target, so a reader never sees half a file.
pub fn write_status_file(path: &Path, status: &GatewayStatus) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(status)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json)?;
    std::fs::rename(&temporary, path)
}

/// Read the state file, if it is there and readable.
pub fn read_status_file(path: &Path) -> Option<GatewayStatus> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RT-523: the daemon's state must survive the process.
    ///
    /// Before this, the delivery ledger and the queue-wait stats were
    /// RAM-only: after a restart nobody could say how many replies had
    /// failed.
    #[test]
    fn the_status_file_carries_the_numbers_across_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.status.json");
        let status = GatewayStatus {
            updated_at: "2026-11-12T10:00:00Z".to_string(),
            deliveries: 7,
            failed_deliveries: 2,
            queue_waits: 3,
            queue_wait_total_ms: 120,
            queue_wait_worst_ms: 80,
            agent_slots: 4,
        };
        write_status_file(&path, &status).expect("write the status");
        // A second reader — a different process — sees the same numbers.
        let read = read_status_file(&path).expect("read the status");
        assert_eq!(read, status);
        assert_eq!(read.failed_deliveries, 2);
        assert_eq!(read.agent_slots, 4);
    }

    /// The write is atomic from a reader's point of view: the file is
    /// renamed over the target, never truncated in place.
    #[test]
    fn the_status_file_is_replaced_not_truncated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gateway.status.json");
        write_status_file(
            &path,
            &GatewayStatus {
                updated_at: "first".to_string(),
                ..Default::default()
            },
        )
        .expect("first write");
        write_status_file(
            &path,
            &GatewayStatus {
                updated_at: "second".to_string(),
                deliveries: 3,
                ..Default::default()
            },
        )
        .expect("second write");
        let read = read_status_file(&path).expect("read");
        assert_eq!(read.updated_at, "second");
        assert_eq!(read.deliveries, 3);
        // No temporary file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
    }
}
