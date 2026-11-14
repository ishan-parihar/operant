//! Point-in-time copies of a config file, so a write that destroys a
//! known-good configuration can be undone.
//!
//! One module owns the on-disk layout, the naming, the retention bound, and
//! the restore guard. Every writer that changes a config calls
//! [`snapshot_before_write`] before it commits, so the guarantee does not
//! depend on which command the operator happened to use.
//!
//! The layout is file-local — `<parent>/backups/config/<name>.<stamp>` — on
//! purpose. A config pinned with `operant -c <path>` has its history next to
//! it, and a secret-bearing file is never copied into an unrelated directory
//! that a different config's retention pass would prune.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many snapshots of one config are kept. The oldest are removed once a
/// write would push the directory past this bound.
pub const KEEP: usize = 5;

/// The directory that holds one config's snapshots: a sibling `backups/config`.
///
/// `None` when the path has no file name (a directory, or a root), because
/// there is nothing to name the snapshots after.
pub fn backup_dir(path: &Path) -> Option<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    Some(parent.join("backups").join("config"))
}

/// A timestamp that sorts lexicographically in time order and is unique
/// within a process: seconds, then nanoseconds.
fn stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since_epoch| since_epoch.as_nanos());
    format!("{nanos}")
}

/// Every snapshot of `path`, newest first.
///
/// The sort key is the timestamp embedded in the file name, which is why
/// [`stamp`] is fixed-width and why nothing else is allowed to share the
/// prefix. A file that does not carry a parseable stamp is skipped rather
/// than ordered against the rest — an operator's own file in this directory
/// must not make the listing lie.
pub fn list(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = backup_dir(path) else {
        return Vec::new();
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{file_name}.");
    let mut found: Vec<(u128, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let stamp = name.strip_prefix(&prefix)?;
            // Fixed-width, so a plain parse orders the same way the digits do.
            let stamp = stamp.parse::<u128>().ok()?;
            Some((stamp, entry.path()))
        })
        .collect();
    found.sort_by(|left, right| right.0.cmp(&left.0));
    found.into_iter().map(|(_, path)| path).collect()
}

/// Copy `original` into the backup directory before a write commits.
///
/// The bytes are the file's CURRENT contents — the version the operator had
/// before this write, which is exactly the version a restore has to bring
/// back. Call it only once the new contents are known to be valid: a
/// snapshot of a write that never happened is a false "known good".
///
/// Best-effort. A failed snapshot must not block a write that is already
/// validated and would otherwise succeed, so every error here is logged and
/// swallowed. The snapshot is written OWNER-ONLY because the file it copies
/// may hold a provider key (`api_key`), and a copy at the umask default
/// would be the one world-readable copy on the machine.
pub fn snapshot_before_write(path: &Path, original: &[u8]) {
    if original.is_empty() {
        // Nothing to recover: an empty file carries no known-good state, and
        // a snapshot of it would be indistinguishable from a failed write.
        return;
    }
    let Some(dir) = backup_dir(path) else {
        return;
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "could not create the config backup directory; the write proceeds without a snapshot"
        );
        return;
    }
    let target = dir.join(format!("{file_name}.{}", stamp()));
    if let Err(error) = crate::fs_secrets::write_secret_file(&target, original) {
        tracing::warn!(
            path = %target.display(),
            error = %error,
            "could not snapshot the config before writing; the write proceeds"
        );
        let _ = std::fs::remove_file(&target);
        return;
    }
    prune(path);
}

/// Drop the oldest snapshots past [`KEEP`].
fn prune(path: &Path) {
    let mut kept = list(path).into_iter();
    for stale in kept.by_ref().skip(KEEP) {
        let _ = std::fs::remove_file(&stale);
    }
}

/// Restore a snapshot over its config, through the validated writer.
///
/// The snapshot is re-validated as a config before it replaces anything, so
/// a corrupt or truncated snapshot can never become the live configuration.
/// The live file is itself snapshotted first, so a restore is undoable.
///
/// `target` is the config the caller asked to restore; the snapshot must
/// name the same basename, which is what stops an operator pointing one
/// config's history at a different file.
pub fn restore(snapshot: &Path, target: &Path) -> Result<PathBuf, String> {
    let Some(name) = snapshot.file_name().and_then(|name| name.to_str()) else {
        return Err(format!(
            "'{}' is not a usable snapshot name",
            snapshot.display()
        ));
    };
    let Some(target_name) = target.file_name().and_then(|name| name.to_str()) else {
        return Err(format!("'{}' has no file name", target.display()));
    };
    let Some(stamp) = name.strip_prefix(&format!("{target_name}.")) else {
        return Err(format!(
            "'{}' is not a snapshot of '{}' (expected a name starting '{target_name}.')",
            snapshot.display(),
            target.display()
        ));
    };
    if stamp.parse::<u128>().is_err() {
        return Err(format!(
            "'{}' is not a snapshot of '{}' (the suffix after '{target_name}.' must be the numeric stamp)",
            snapshot.display(),
            target.display()
        ));
    }
    // The snapshot must live in this config's own backup directory: a
    // snapshot of the same basename anywhere else is not this file's history.
    if backup_dir(target).is_none_or(|dir| snapshot.parent() != Some(dir.as_path())) {
        return Err(format!(
            "'{}' is not in '{}'",
            snapshot.display(),
            backup_dir(target).unwrap_or_default().display()
        ));
    }
    let bytes = std::fs::read(snapshot)
        .map_err(|error| format!("read '{}': {error}", snapshot.display()))?;
    // The live file is recoverable before the restore replaces it, whoever
    // wrote it last. The surgical writer's own snapshot cannot cover this:
    // a restore is precisely the path where the current bytes were NOT
    // produced by that writer.
    if let Ok(live) = std::fs::read(target) {
        snapshot_before_write(target, &live);
    }
    let staged = target.with_extension("toml.restore");
    crate::fs_secrets::write_secret_file(&staged, &bytes)
        .map_err(|error| format!("write '{}': {error}", staged.display()))?;
    if let Err(error) = crate::config::parse_config_file_without_recording(&staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!(
            "'{}' is not a loadable config, so nothing was changed ({error})",
            snapshot.display()
        ));
    }
    // The seam carries the target's mode across on the rename.
    crate::config::persist_restored_file(&staged, target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("operant.toml");
        // Valid TOML: the restore path validates its payload through the real
        // loader, so a fixture that does not parse would fail the guard.
        std::fs::write(&path, b"[agent]\nmodel = \"first\"\n").expect("fixture");
        path
    }

    #[test]
    fn a_snapshot_is_recoverable_and_owner_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());
        let before = std::fs::read(&path).expect("read");

        snapshot_before_write(&path, &before);

        let found = list(&path);
        assert_eq!(found.len(), 1, "one snapshot exists");
        assert_eq!(std::fs::read(&found[0]).expect("read snapshot"), before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&found[0])
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "a snapshot may hold a key; got {mode:o}");
        }
    }

    #[test]
    fn the_retention_bound_holds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());

        for index in 0..(KEEP + 3) {
            snapshot_before_write(&path, format!("body {index}\n").as_bytes());
        }

        assert_eq!(list(&path).len(), KEEP, "the bound is KEEP, not KEEP-1");
    }

    #[test]
    fn an_empty_file_is_not_snapshotted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());

        snapshot_before_write(&path, b"");

        assert!(list(&path).is_empty(), "nothing to recover");
    }

    #[test]
    fn the_listing_is_newest_first_and_ignores_foreign_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());
        let backups = backup_dir(&path).expect("backup dir");
        std::fs::create_dir_all(&backups).expect("create");
        std::fs::write(backups.join("operant.toml.100"), b"old").expect("write");
        std::fs::write(backups.join("operant.toml.900"), b"new").expect("write");
        std::fs::write(backups.join("notes.txt"), b"mine").expect("write");

        let found = list(&path);

        assert_eq!(found.len(), 2, "a foreign file is not a snapshot");
        assert_eq!(std::fs::read(&found[0]).expect("read"), b"new");
    }

    #[test]
    fn a_restore_refuses_a_foreign_snapshot_and_changes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());
        let elsewhere = dir.path().join("other.toml");
        std::fs::write(&elsewhere, b"other\n").expect("write");

        let error = restore(&elsewhere, &path).expect_err("wrong basename");

        assert!(error.contains("not a snapshot of"), "got: {error}");
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"[agent]\nmodel = \"first\"\n"
        );
    }

    #[test]
    fn a_restore_refuses_a_snapshot_that_does_not_parse() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());
        let original = std::fs::read(&path).expect("read");
        snapshot_before_write(&path, &original);
        let snapshot = list(&path).into_iter().next().expect("snapshot");
        std::fs::write(&snapshot, b"not = = toml\n").expect("corrupt");

        let error = restore(&snapshot, &path).expect_err("unloadable");

        assert!(error.contains("not a loadable config"), "got: {error}");
        assert_eq!(
            std::fs::read(&path).expect("read"),
            original,
            "the live config is untouched"
        );
    }

    #[test]
    fn a_restore_replaces_the_config_and_keeps_the_pre_restore_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = fixture(dir.path());
        let original = std::fs::read(&path).expect("read");
        snapshot_before_write(&path, &original);
        let snapshot = list(&path).into_iter().next().expect("snapshot");
        std::fs::write(&path, b"[agent]\nmodel = \"second\"\n").expect("overwrite");

        let restored = restore(&snapshot, &path).expect("restore");

        assert_eq!(restored, path);
        assert_eq!(std::fs::read(&path).expect("read"), original);
        let newest = list(&path).into_iter().next().expect("pre-restore copy");
        assert_eq!(
            std::fs::read(&newest).expect("read"),
            b"[agent]\nmodel = \"second\"\n",
            "a restore is undoable"
        );
    }
}
