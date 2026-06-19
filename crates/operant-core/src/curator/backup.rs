//! Backup and rollback for curator state.
//! Ported from operant-agent/agent/curator_backup.py.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Create a tar.gz snapshot of the skills directory.
///
/// Returns the path to the created backup archive.
pub fn create_backup(
    skills_dir: &Path,
    backup_dir: &Path,
    reason: Option<&str>,
) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let reason_slug = reason.unwrap_or("manual");
    let filename = format!("curator-backup-{}-{}.tar.gz", timestamp, reason_slug);
    let backup_path = backup_dir.join(&filename);

    fs::create_dir_all(backup_dir)
        .with_context(|| format!("Failed to create backup dir: {}", backup_dir.display()))?;

    let tar_gz = File::create(&backup_path)
        .with_context(|| format!("Failed to create backup file: {}", backup_path.display()))?;
    let encoder = flate2::write::GzEncoder::new(tar_gz, Default::default());
    let mut archive = tar::Builder::new(encoder);

    archive
        .append_dir_all(".", skills_dir)
        .with_context(|| format!("Failed to archive directory: {}", skills_dir.display()))?;

    archive.finish()?;
    Ok(backup_path)
}

/// List available backups, sorted newest-first.
pub fn list_backups(backup_dir: &Path) -> Result<Vec<PathBuf>> {
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(backup_dir)
        .with_context(|| format!("Failed to read backup dir: {}", backup_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tar.gz"))
        .map(|e| e.path())
        .collect();
    entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    Ok(entries)
}

/// Restore skills from a backup archive, returning the path to the
/// previous skills directory (renamed for recovery).
pub fn restore_backup(backup_path: &Path, skills_dir: &Path) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rollback_dir = skills_dir.with_extension(format!("rollback-{}", timestamp));

    // Rename current skills dir to rollback
    if skills_dir.exists() {
        fs::rename(skills_dir, &rollback_dir)
            .context("Failed to rename current skills dir for rollback")?;
    }

    // Extract backup
    let tar_gz = File::open(backup_path)
        .with_context(|| format!("Failed to open backup: {}", backup_path.display()))?;
    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(skills_dir)
        .with_context(|| format!("Failed to extract backup to: {}", skills_dir.display()))?;

    Ok(rollback_dir)
}
