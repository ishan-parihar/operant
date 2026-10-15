//! Phase G (docs/agentic-os/): the one-time archive of the org stores —
//! copy, never delete (ORG-LAYER-DISPOSITION.md §4).
//!
//! The org tables live split across several sqlite files that disagree
//! (the disposition's split-brain table). When the organization moves to
//! Holon, the stores are copied to `~/.operant/archive/org-<date>/` with a
//! checksum manifest; the originals stay where they were (their writers go
//! dark in G+1). Idempotent-ish: a second archive on the same date refuses
//! rather than half-mixing two snapshots.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// The org stores the disposition names. Paths are resolved against the
/// operant home the caller passes (the CLI knows where that is).
pub fn org_store_paths(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("database.db"),
        home.join("operant.db"),
        home.join("operant_kanban.db"),
        home.join("kanban.db"),
        home.join("operant_decisions.db"),
        home.join("operant_cron.db"),
        home.join("cron.db"),
    ]
}

fn sha256_hex(path: &Path) -> Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)
        .map_err(|e| Error::Archive(format!("open {}: {e}", path.display())))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| Error::Archive(format!("read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Archive every org store that exists under `home` into
/// `<home>/archive/org-<date>/`, with `MANIFEST.sha256` beside the copies.
/// Returns the archive directory. Missing stores are skipped (listed as such
/// in the manifest — a clean box legitimately has fewer).
pub fn archive_org_stores(home: &Path, date: &str) -> Result<PathBuf> {
    let dest = home.join("archive").join(format!("org-{date}"));
    if dest.exists() {
        return Err(Error::Archive(format!(
            "archive {} already exists — one snapshot per date",
            dest.display()
        )));
    }
    std::fs::create_dir_all(&dest)
        .map_err(|e| Error::Archive(format!("mkdir {}: {e}", dest.display())))?;

    let mut manifest = String::from(
        "# org-store archive (docs/agentic-os/ phase G — copy, never delete)\n# \
         WAL-safe (audit O-24): SQLite stores go through VACUUM INTO — a live\n# \
         -wal file's committed pages are IN the snapshot, and the digest\n# \
         covers the archived copy, not the moving original.\n",
    );
    let mut copied = 0usize;
    for src in org_store_paths(home) {
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !src.exists() {
            manifest.push_str(&format!(
                "{name}: MISSING (never present or already moved)\n"
            ));
            continue;
        }
        let out = dest.join(&name);
        if is_sqlite(&src) {
            vacuum_into(&src, &out).map_err(|e| {
                Error::Archive(format!(
                    "VACUUM INTO {name}: {e} — close the writers or archive from a stopped instance"
                ))
            })?;
        } else {
            std::fs::copy(&src, &out).map_err(|e| Error::Archive(format!("copy {name}: {e}")))?;
        }
        // digest the ARCHIVED copy — the original keeps moving
        let digest = sha256_hex(&out)?;
        manifest.push_str(&format!("{name} {digest}\n"));
        copied += 1;
    }

    // O-24: the org/ tree (the cast's charter files — non-regenerable)
    // and any per-board kanban files. The tree copies whole; the globs
    // catch every operant_kanban_<board>.db the manager may create.
    let org_tree = home.join("org");
    if org_tree.is_dir() {
        let out_tree = dest.join("org");
        copy_tree(&org_tree, &out_tree)?;
        manifest.push_str("org/ tree copied\n");
    }
    let board_files: Vec<std::path::PathBuf> = std::fs::read_dir(home)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("operant_kanban_") && n.ends_with(".db"))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for entry in board_files {
        let name = entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let out = dest.join(&name);
        if is_sqlite(&entry) {
            vacuum_into(&entry, &out)
                .map_err(|e| Error::Archive(format!("VACUUM INTO {name}: {e}")))?;
        } else {
            std::fs::copy(&entry, &out).map_err(|e| Error::Archive(format!("copy {name}: {e}")))?;
        }
        let digest = sha256_hex(&out)?;
        manifest.push_str(&format!("{name} {digest}\n"));
        copied += 1;
    }
    std::fs::write(dest.join("MANIFEST.sha256"), manifest)
        .map_err(|e| Error::Archive(format!("write manifest: {e}")))?;
    if copied == 0 {
        return Err(Error::Archive(format!(
            "no org stores found under {} — nothing to archive",
            home.display()
        )));
    }
    Ok(dest)
}

/// O-24: SQLite's online snapshot — VACUUM INTO writes a consistent copy
/// including every committed page still in the -wal, without a writer lock.
fn vacuum_into(src: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let conn = rusqlite::Connection::open(src).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "wal")
        .map_err(|e| e.to_string())?;
    conn.execute_batch(&format!(
        "VACUUM INTO '{}';",
        dest.display().to_string().replace('\'', "''")
    ))
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// SQLite files begin with the magic string (empty stores and -wal siblings
/// alike; non-db files copy as bytes).
fn is_sqlite(path: &std::path::Path) -> bool {
    let mut magic = [0u8; 16];
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    f.read_exact(&mut magic).is_ok() && &magic == b"SQLite format 3\x00"
}

/// Copy a directory tree recursively.
fn copy_tree(src: &std::path::Path, dest: &std::path::Path) -> Result<(), Error> {
    std::fs::create_dir_all(dest)
        .map_err(|e| Error::Archive(format!("mkdir {}: {e}", dest.display())))?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| Error::Archive(format!("read {}: {e}", src.display())))?
    {
        let entry = entry.map_err(|e| Error::Archive(format!("readdir: {e}")))?;
        let out = dest.join(entry.file_name());
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_tree(&entry.path(), &out)?;
        } else {
            std::fs::copy(entry.path(), &out)
                .map_err(|e| Error::Archive(format!("copy file: {e}")))?;
        }
    }
    Ok(())
}
