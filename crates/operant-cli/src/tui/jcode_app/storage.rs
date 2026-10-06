// Vendored from jcode (crates/jcode-storage/src/lib.rs + crates/jcode-core/src/fs.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — [port-decision] the storage leaf fns jcode_app::logging needs:
// jcode_dir (storage/src/lib.rs:150), logs_dir (:178), ensure_dir (:453), plus
// set_directory_permissions_owner_only (jcode-core/src/fs.rs:22) which ensure_dir
// calls — unix arm verbatim, windows arm [port-excision] (operant targets unix
// TUI hosts; windows_sys is not an operant dependency).
//! [port-excision] the rest of jcode-storage (state dirs, secrets, env files)
//! is not ported; extend here when a renderer reaches for it.
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn jcode_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("JCODE_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    Ok(home.join(".jcode"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(jcode_dir()?.join("logs"))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        set_directory_permissions_owner_only(path)?;
    }
    Ok(())
}

// [port-source] jcode-base/src/storage.rs:24-38 — test-env serialization
// helpers, ported verbatim (upstream gates on `any(test, feature = "test-support")`;
// operant's callers are all #[cfg(test)], so cfg(test) alone is the equivalent
// gate). Deviation: OnceLock+get_or_init → LazyLock per repo rule rs-lazylock
// (initializer is known at declaration time).
#[cfg(test)]
use std::sync::{LazyLock, Mutex, MutexGuard};

#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    &ENV_LOCK
}

#[cfg(test)]
pub(crate) fn lock_test_env() -> MutexGuard<'static, ()> {
    test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// [port-source] jcode-storage/src/lib.rs:218-238 — ported verbatim.
/// Resolve a path under the user's home directory, but sandbox it under
/// `$JCODE_HOME/external/` when `JCODE_HOME` is set.
///
/// This keeps external provider auth files isolated during tests and sandboxed
/// runs without changing default on-disk locations for normal users.
pub(crate) fn user_home_path(relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    if relative.is_absolute() {
        anyhow::bail!(
            "user_home_path expects a relative path, got {}",
            relative.display()
        );
    }

    if let Ok(path) = std::env::var("JCODE_HOME") {
        return Ok(PathBuf::from(path).join("external").join(relative));
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    Ok(home.join(relative))
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
        // [port-excision] upstream windows ACL arm (jcode-core/src/fs.rs:29-:69)
        // dropped — windows_sys is not an operant dependency.
        let _ = path;
        Ok(())
    }
}

// [port-source] jcode-storage/src/lib.rs:516-518 (write_json_fast), :527-535
// (write_json_inner), :537-615 (write_bytes_inner), :626-678
// (StorageRecoveryEvent, read_json, read_json_with_recovery_handler) — ported
// verbatim with two cited deviations:
// 1. upstream's `rand::random()` temp-file nonce is replaced with a
//    nanos-since-epoch nonce (rand is not an operant dependency; the nonce only
//    needs per-process uniqueness for the temp+rename dance).
// 2. the `secret` arms' file-level `jcode_core::fs::set_permissions_owner_only`
//    calls are not ported (jcode-core fs module is out of scope) — the secret
//    parameter is kept verbatim but hardened file permissions land at cutover.
use serde::{Serialize, de::DeserializeOwned};
use std::io::Write;

/// Fast JSON write: atomic rename but no fsync. Good for frequent saves where
/// durability on power loss is not critical (e.g., session saves during tool execution).
/// Data is still safe against process crashes (atomic rename protects against partial writes).
pub fn write_json_fast<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_json_inner(path, value, false, false)
}

fn write_json_inner<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    durable: bool,
    secret: bool,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    write_bytes_inner(path, &bytes, durable, secret)
}

fn write_bytes_inner(path: &Path, bytes: &[u8], durable: bool, secret: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
        // [port-decision] upstream secret arm calls the unported
        // jcode_core::fs::set_directory_permissions_owner_only here; both
        // ported callers pass secret=false. Re-activate at cutover.
        let _ = secret;
    }

    let pid = std::process::id();
    // Deviation: rand::random() → nanos nonce (see header note 1).
    let nonce: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_path = path.with_extension(format!("tmp.{}.{}", pid, nonce));

    let result = (|| -> Result<()> {
        let file = std::fs::File::create(&tmp_path)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(bytes)?;
        let file = writer
            .into_inner()
            .map_err(|e| anyhow::anyhow!("flush failed: {}", e))?;

        if durable {
            file.sync_all()?;
        }

        if path.exists() {
            let bak_path = path.with_extension("bak");
            // Preserve the previous version as .bak without ever leaving the
            // primary path missing. On Unix, rename(tmp, path) atomically
            // replaces the destination, so the backup can be a hard link to
            // the old inode: concurrent readers always see either the old or
            // the new content, never ENOENT. (The old rename-away approach
            // opened a window where the primary did not exist, which made
            // concurrent load-all style readers silently drop entries, e.g.
            // self-dev build requests "disappearing" from the queue.)
            #[cfg(unix)]
            {
                let _ = std::fs::hard_link(path, &bak_path);
            }
        }

        std::fs::rename(&tmp_path, path)?;

        #[cfg(unix)]
        if durable
            && let Some(parent) = path.parent()
            && let Ok(dir) = std::fs::File::open(parent)
        {
            let _ = dir.sync_all();
        }

        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }

    result
}

pub enum StorageRecoveryEvent<'a> {
    CorruptPrimary {
        path: &'a Path,
        error: &'a serde_json::Error,
    },
    RecoveredFromBackup {
        backup_path: &'a Path,
    },
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    read_json_with_recovery_handler(path, |event| match event {
        StorageRecoveryEvent::CorruptPrimary { path, error } => {
            eprintln!(
                "Corrupt JSON at {}, trying backup: {}",
                path.display(),
                error
            );
        }
        StorageRecoveryEvent::RecoveredFromBackup { backup_path } => {
            eprintln!("Recovered from backup: {}", backup_path.display());
        }
    })
}

pub fn read_json_with_recovery_handler<T, F>(path: &Path, mut on_recovery: F) -> Result<T>
where
    T: DeserializeOwned,
    F: FnMut(StorageRecoveryEvent<'_>),
{
    let data = std::fs::read_to_string(path)?;
    match serde_json::from_str(&data) {
        Ok(val) => Ok(val),
        Err(e) => {
            let bak_path = path.with_extension("bak");
            if bak_path.exists() {
                on_recovery(StorageRecoveryEvent::CorruptPrimary { path, error: &e });
                let bak_data = std::fs::read_to_string(&bak_path)?;
                match serde_json::from_str(&bak_data) {
                    Ok(val) => {
                        on_recovery(StorageRecoveryEvent::RecoveredFromBackup {
                            backup_path: &bak_path,
                        });
                        let _ = std::fs::copy(&bak_path, path);
                        Ok(val)
                    }
                    Err(bak_err) => Err(anyhow::anyhow!(
                        "Corrupt JSON at {} ({}), backup also corrupt ({})",
                        path.display(),
                        e,
                        bak_err
                    )),
                }
            } else {
                Err(anyhow::anyhow!("Corrupt JSON at {}: {}", path.display(), e))
            }
        }
    }
}

// --- active-PID registry (jcode-storage/src/active_pids.rs:28-137), the
// --- bounded set the safety expiry path needs. Ported at batch-4 sweep tail.
// --- `streaming_session_ids` / `prune_active_pids_owned_by` and friends stay
// --- unported (presence/desktop consumers, not reached from the TUI layer).

/// Directory holding one ownership marker per session ID (`~/.jcode/active_pids`).
/// Each file contains the owning process PID, not a client/window PID in server mode.
pub fn active_pids_dir() -> Option<PathBuf> {
    jcode_dir().ok().map(|d| d.join("active_pids"))
}

/// Directory holding per-session "currently streaming" markers.
pub fn streaming_pids_dir() -> Option<std::path::PathBuf> {
    jcode_dir().ok().map(|d| d.join("streaming_pids"))
}

/// Directory holding markers for internal sessions (debug/test sessions and
/// spawned children such as swarm workers).
pub fn internal_pids_dir() -> Option<std::path::PathBuf> {
    jcode_dir().ok().map(|d| d.join("internal_pids"))
}

pub fn set_session_internal(session_id: &str, internal: bool) {
    let Some(dir) = internal_pids_dir() else {
        return;
    };
    if internal {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(session_id), "");
    } else {
        let _ = std::fs::remove_file(dir.join(session_id));
    }
}

pub fn unmark_streaming(session_id: &str) {
    if let Some(dir) = streaming_pids_dir() {
        let _ = std::fs::remove_file(dir.join(session_id));
    }
}

pub fn register_active_pid(session_id: &str, pid: u32) {
    if let Some(dir) = active_pids_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(session_id), pid.to_string());
    }
}

/// Remove the active-PID record for `session_id`, if present.
pub fn unregister_active_pid(session_id: &str) {
    if let Some(dir) = active_pids_dir() {
        let _ = std::fs::remove_file(dir.join(session_id));
    }
    // A closed session is never streaming, and its internal flag is moot.
    unmark_streaming(session_id);
    set_session_internal(session_id, false);
}
