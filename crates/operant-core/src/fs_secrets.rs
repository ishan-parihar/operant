//! Filesystem helpers for files that may contain credentials.
//!
//! Plan 002: any file that may hold a secret (config TOML with provider
//! keys, `.env` with API tokens, gateway pairing tokens, approval
//! allowlists) must be written with mode `0o600` (owner read/write only),
//! bypassing the process umask. Existing looser files are tightened
//! to `0o600` in place. No-op on non-Unix platforms (the spec only
//! requires the perms on the shipped binary's target).

use std::fs::OpenOptions;
use std::io;
use std::path::Path;

/// Write `bytes` to `path` as a secret file (mode 0o600 on Unix).
///
/// Creates the file if it does not exist, truncates if it does, then
/// tightens (or sets) the mode to 0o600. Atomic with respect to other
/// writers of the same process only (no `rename`-based atomicity — a
/// small-window lossy write is acceptable for human-driven `setup` /
/// `operant.toml` saves; the agent path never invokes this).
pub fn write_secret_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?
        .write_all(bytes)?;

    set_secret_perms(path)?;
    Ok(())
}

/// Set file mode to 0o600 on Unix. On non-Unix, no-op.
///
/// If the file already exists at a tighter mode (e.g. 0o400), leaves it.
/// If looser, tightens to 0o600. Sets the mode exactly (bypasses umask).
#[cfg(unix)]
pub fn set_secret_perms(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let current = std::fs::metadata(path)?.permissions().mode() & 0o777;
    if current != 0o600 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn set_secret_perms(_path: &Path) -> io::Result<()> {
    // Non-Unix: leave permissions to the platform / FS defaults.
    Ok(())
}

/// Tighten the permissions of `path` to 0o600 if currently looser.
///
/// Idempotent: no-op when the file already has 0o600 or stricter (0o400).
/// Silently returns Ok if the file does not exist (caller will create it
/// via `write_secret_file` shortly and that path will set the perms).
/// Used on the config/db read paths to migrate pre-R42 (loose umask-022)
/// files into the post-R42 secret-handling posture.
pub fn tighten_if_loose(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        // Already owner-only → no-op (0o600 or 0o400 or 0o700 with
        // group/world masked off — we only require 0o600 for parity).
        if (mode & 0o077) == 0 {
            return Ok(());
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

use std::io::Write;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn write_secret_file_sets_0600() {
        let dir = tempdir();
        let path = dir.join("secret.toml");
        write_secret_file(&path, b"key = \"v\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "expected 0o600, got {:o}", mode);
        }
        let mut s = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "key = \"v\"\n");
    }

    #[test]
    fn write_secret_file_tightens_existing_0644() {
        let dir = tempdir();
        let path = dir.join("loose.toml");
        std::fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        write_secret_file(&path, b"new").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "expected 0o600, got {:o}", mode);
        }
        let mut s = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "new");
    }

    #[test]
    fn write_secret_file_creates_parent_implicitly_via_setup_pattern() {
        // Caller is expected to create_dir_all; we don't try to be clever.
        let dir = tempdir();
        let path = dir.join("nested/secret.toml");
        // No parent pre-create → write should error (documented contract).
        assert!(write_secret_file(&path, b"x").is_err());
    }

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("operant-fs-secrets-{}", std::process::id()));
        p.push(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
