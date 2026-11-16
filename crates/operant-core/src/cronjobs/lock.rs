//! The gateway daemon's file lock, named in ONE place.
//!
//! The gateway daemon holds an exclusive `flock` on
//! `<operant home>/gateway.lock` (`gateway_runner.rs`,
//! `acquire_gateway_daemon_lock`). The scheduled cron dispatcher lives INSIDE
//! that daemon, so the lock's presence is the machine-wide statement "the cron
//! scheduler is running here".
//!
//! `operant cron run <id>` (Z31) builds its own `CronDb`, agent and scheduler
//! and calls `run_job_now` directly, in a second process. The `CronDb`
//! connection mutex is `std::sync::Mutex` — per-process, so it cannot
//! serialize two processes. The file lock can, and it is the one the daemon
//! already holds.
//!
//! The path lives HERE rather than in the CLI so there is exactly one
//! definition of it for the holder (the gateway), the prober (the CLI cron
//! path) and the tests. The ACQUISITION stays in `gateway_runner.rs`: this
//! crate must not own two `flock` implementations that can drift apart.

use std::path::PathBuf;

/// The lock file the gateway daemon holds for the life of the process.
///
/// `<operant home>/gateway.lock`. The file is never unlinked: `flock` protects
/// the open INODE, so removing the path would let a second starter create a
/// NEW inode and lock that one — two holders of "the lock" (RT-526). A
/// leftover file is therefore harmless.
pub fn gateway_daemon_lock_path() -> PathBuf {
    crate::platform::operant_home().join("gateway.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path is derived from the ONE home resolver, not from a second
    /// guess at the OS home (the iter-1137 rule: a store that builds its own
    /// path lands in the default profile's tree while the operator is on
    /// another profile, and the daemon holds a DIFFERENT lock than the CLI
    /// probes).
    #[test]
    fn the_lock_path_is_under_the_resolved_operant_home() {
        let home = crate::platform::operant_home();
        assert_eq!(gateway_daemon_lock_path(), home.join("gateway.lock"));
        assert!(
            gateway_daemon_lock_path().ends_with("gateway.lock"),
            "the file name is what the daemon and the CLI must agree on"
        );
    }
}
