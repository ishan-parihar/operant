//! Archive, prune, and restore skills.
//! Moves skill directories between active and archive locations.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Archive a skill by moving it from skills_dir to archive_dir.
pub fn archive_skill(skill_name: &str, skills_dir: &Path, archive_dir: &Path) -> Result<()> {
    let src = skills_dir.join(skill_name);
    if !src.exists() {
        anyhow::bail!("Skill '{}' not found at {}", skill_name, src.display());
    }
    fs::create_dir_all(archive_dir)?;
    let dst = archive_dir.join(skill_name);
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    fs::rename(&src, &dst).with_context(|| format!("Failed to archive skill '{}'", skill_name))?;
    Ok(())
}

/// Restore a skill from archive back to active.
pub fn restore_skill(skill_name: &str, archive_dir: &Path, skills_dir: &Path) -> Result<()> {
    let src = archive_dir.join(skill_name);
    if !src.exists() {
        anyhow::bail!("Archived skill '{}' not found", skill_name);
    }
    let dst = skills_dir.join(skill_name);
    if dst.exists() {
        anyhow::bail!(
            "A skill named '{}' already exists in active skills",
            skill_name
        );
    }
    fs::create_dir_all(skills_dir)?;
    fs::rename(&src, &dst).with_context(|| format!("Failed to restore skill '{}'", skill_name))?;
    Ok(())
}

/// List archived skills, sorted alphabetically.
pub fn list_archived(archive_dir: &Path) -> Result<Vec<String>> {
    if !archive_dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(archive_dir)?;
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    Ok(names)
}

/// Prune archived skills older than `days` (i.e., delete them permanently).
pub fn prune_archived(archive_dir: &Path, days: u64) -> Result<Vec<String>> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now - (days * 86400);

    let mut pruned = Vec::new();
    if !archive_dir.exists() {
        return Ok(pruned);
    }

    for entry in fs::read_dir(archive_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let modified = metadata
            .modified()
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);

        if metadata.is_dir() && modified < cutoff {
            let name = entry.file_name().to_string_lossy().to_string();
            fs::remove_dir_all(entry.path())?;
            pruned.push(name);
        }
    }
    Ok(pruned)
}
