// Vendored from jcode (crates/operant-storage/src/lib.rs + crates/operant-core/src/fs.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — [port-decision] the storage leaf fns operant_app::logging needs:
// operant_dir (storage/src/lib.rs:150), logs_dir (:178), ensure_dir (:453), plus
// set_directory_permissions_owner_only (operant-core/src/fs.rs:22) which ensure_dir
// calls — unix arm verbatim, windows arm [port-excision] (operant targets unix
// TUI hosts; windows_sys is not an operant dependency).
//! [port-excision] the rest of operant-storage (state dirs, secrets, env files)
//! is not ported; extend here when a renderer reaches for it.
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn operant_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("OPERANT_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    Ok(home.join(".operant"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(operant_dir()?.join("logs"))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        set_directory_permissions_owner_only(path)?;
    }
    Ok(())
}

pub fn set_directory_permissions_owner_only(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, perms)
    }
    #[cfg(windows)]
    {
        // [port-excision] upstream windows ACL arm (operant-core/src/fs.rs:29-:69)
        // dropped — windows_sys is not an operant dependency.
        let _ = path;
        Ok(())
    }
}

// [port-source] operant-storage/src/lib.rs:516-518 (write_json_fast), :527-535
// (write_json_inner), :537-615 (write_bytes_inner), :626-678
// (StorageRecoveryEvent, read_json, read_json_with_recovery_handler) — ported
// verbatim with two cited deviations:
// 1. upstream's `rand::random()` temp-file nonce is replaced with a
//    nanos-since-epoch nonce (rand is not an operant dependency; the nonce only
//    needs per-process uniqueness for the temp+rename dance).
// 2. the `secret` arms' file-level `operant_core::fs::set_permissions_owner_only`
//    calls are not ported (operant-core fs module is out of scope) — the secret
//    parameter is kept verbatim but hardened file permissions land at cutover.

// --- active-PID registry (operant-storage/src/active_pids.rs:28-137), the
// --- bounded set the safety expiry path needs. Ported at batch-4 sweep tail.
// --- `streaming_session_ids` / `prune_active_pids_owned_by` and friends stay
// --- unported (presence/desktop consumers, not reached from the TUI layer).
