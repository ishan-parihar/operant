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
    // An EXISTING file keeps its mode when it is opened, so a loose file
    // would hold the new content while it is still world-readable. Tighten
    // it BEFORE the write (a no-op for a file that does not exist yet).
    match set_secret_perms(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // Create with mode 0o600 in the SAME open(2). `File::create` applies the
    // umask default (0644 on a stock system), and a chmod that follows the
    // write leaves a window where the secret is world-readable — the R40-1
    // class. `operant-config`'s identically named writer and `operant-cli`'s
    // `.env` writer already create owner-only; this one did not.
    create_owner_only(path)?.write_all(bytes)?;
    set_secret_perms(path)?;
    Ok(())
}

/// Open `path` for writing, creating it OWNER-ONLY on Unix.
///
/// The mode is applied by the same `open(2)` that creates the file, so the
/// file is never world-readable — not even for the instant before the first
/// byte is written. A create-then-chmod sequence cannot close that window:
/// the bytes are already in the file when the chmod runs.
#[cfg(unix)]
fn create_owner_only(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

/// Non-Unix has no POSIX mode bits; the platform default applies.
#[cfg(not(unix))]
fn create_owner_only(path: &Path) -> io::Result<std::fs::File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// Set file mode to 0o600 on Unix when it is looser. On non-Unix, no-op.
///
/// TIGHTEN-ONLY (RT-361): a file already at a tighter mode (e.g. 0o400) is
/// left alone. The old condition was `current != 0o600`, which WIDENED a
/// 0o400 file to 0o600 — the opposite of the function's contract, and a
/// silent permission regression on a secret store. Bypasses umask.
#[cfg(unix)]
pub fn set_secret_perms(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let current = std::fs::metadata(path)?.permissions().mode() & 0o777;
    // Any bit set outside 0o600 means the mode is looser: tighten it.
    if current & !0o600 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn set_secret_perms(_path: &Path) -> io::Result<()> {
    // Non-Unix: leave permissions to the platform / FS defaults.
    Ok(())
}

/// Apply `mode` to `path`, reporting a failure with the consequence the
/// caller names.
///
/// RT-419: a permission write whose result was DISCARDED left the file's
/// posture wrong with nothing said anywhere — an OAuth directory that stayed
/// at the umask default, an installed bridge binary without its execute bit.
/// The mode POLICY stays with the caller (this helper never widens, never
/// fails the caller's operation): it makes the failure observable, which is
/// what the swallow took away.
///
/// `consequence` is the sentence the operator reads — name what is wrong
/// with the file when the chmod did not land.
pub fn set_mode_reporting(path: &Path, mode: u32, consequence: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).inspect_err(|e| {
            tracing::warn!(
                path = %path.display(),
                mode = format!("{mode:o}"),
                error = %e,
                "{consequence}"
            );
        })
    }
    #[cfg(not(unix))]
    {
        // No POSIX mode bits: the platform default applies, and there is
        // nothing to report.
        let _ = (path, mode, consequence);
        Ok(())
    }
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
    /// RT-361: `set_secret_perms` promised "if the file already exists at a
    /// tighter mode (e.g. 0o400), leaves it" while its condition
    /// (`current != 0o600`) WIDENED a 0o400 file to 0o600 — a silent
    /// permission regression on a secret store.
    #[cfg(unix)]
    #[test]
    fn set_secret_perms_tightens_but_never_widens() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");

        // A LOOSER file (0o644) is tightened to 0o600.
        let loose = dir.path().join("loose");
        std::fs::write(&loose, "secret").expect("write");
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        set_secret_perms(&loose).expect("set perms");
        let mode = std::fs::metadata(&loose)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "a looser file must be tightened");

        // A TIGHTER file (0o400) is left exactly as it is.
        let tight = dir.path().join("tight");
        std::fs::write(&tight, "secret").expect("write");
        std::fs::set_permissions(&tight, std::fs::Permissions::from_mode(0o400)).expect("chmod");
        set_secret_perms(&tight).expect("set perms");
        let mode = std::fs::metadata(&tight)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o400, "a tighter file must not be widened: {mode:o}");

        // Exactly 0o600 is a no-op.
        let exact = dir.path().join("exact");
        std::fs::write(&exact, "secret").expect("write");
        std::fs::set_permissions(&exact, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        set_secret_perms(&exact).expect("set perms");
        let mode = std::fs::metadata(&exact)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    use super::*;
    use std::io::Read;

    /// RT-419: the CLASS — a permission write whose result was discarded. Two
    /// production sites swallowed it (the OAuth directory's 0700 and the
    /// downloaded bridge binary's 0755), so both now route through the one
    /// helper. SOURCE contract for the coverage (the failure is IO-shaped, so
    /// no deterministic fixture can force it at those sites), with
    /// concatenation-built needles.
    #[test]
    fn every_swallowed_permission_write_routes_through_the_helper() {
        // The two sites that swallowed it.
        let oauth = include_str!("mcp_oauth.rs");
        assert_eq!(
            oauth.matches("set_mode_reporting(").count(),
            1,
            "the OAuth directory's mode must be set through the helper"
        );
        let aft = include_str!("aft_bridge.rs");
        assert_eq!(
            aft.matches("set_mode_reporting(").count(),
            1,
            "the downloaded binary's mode must be set through the helper"
        );
        // And no production site swallows a permission write any more.
        for src in [oauth, aft] {
            let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
            assert_eq!(
                prod.matches("let _ = std::fs::set_permissions(").count()
                    + prod.matches("let _ = tokio::fs::set_permissions(").count(),
                0,
                "a permission write must not be discarded"
            );
        }
    }

    /// RT-419: the helper's whole point is that a FAILED chmod is visible.
    /// BEHAVIORAL: pointing it at a path that cannot be chmodded (a
    /// non-existent file) must return the error — and the caller decides
    /// what to do with it. The helper never widens and never fails the
    /// caller's operation itself.
    #[cfg(unix)]
    #[test]
    fn set_mode_reporting_surfaces_the_failure_and_applies_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        let path = dir.join("mode-target");
        std::fs::write(&path, "x").expect("write");

        // The happy path: the mode lands.
        set_mode_reporting(&path, 0o700, "unused on the happy path").expect("set mode");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the requested mode must be applied");

        // The failure path: the error is RETURNED (the caller reports it).
        let missing = dir.join("does-not-exist");
        let err = set_mode_reporting(&missing, 0o700, "unused on the failure path")
            .expect_err("a missing path cannot be chmodded");
        assert_eq!(
            err.kind(),
            io::ErrorKind::NotFound,
            "the failure must surface to the caller, not vanish"
        );
    }

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
    fn the_file_is_owner_only_before_a_single_byte_is_written() {
        // The window this pins: a create-then-chmod writer has the file at
        // the umask default (0644 on a stock system) while the secret is
        // already IN it. Creating it owner-only means there is no such
        // instant — the mode is right from the creating open(2).
        let dir = tempdir();
        let path = dir.join("owner-only.toml");
        let file = create_owner_only(&path).expect("create owner-only");
        assert!(path.exists(), "the file is created by the same call");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "expected 0o600, got {mode:o}");
        }
        drop(file);
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
