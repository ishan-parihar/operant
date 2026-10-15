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

    let mut manifest =
        String::from("# org-store archive (docs/agentic-os/ phase G — copy, never delete)\n");
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
        let digest = sha256_hex(&src)?;
        std::fs::copy(&src, dest.join(&name))
            .map_err(|e| Error::Archive(format!("copy {name}: {e}")))?;
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
