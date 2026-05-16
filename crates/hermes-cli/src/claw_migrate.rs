//! Migration from OpenClaw (~/.openclaw) to Hermes format.
//! Ported from hermes-agent/hermes_cli/setup.py:_offer_openclaw_migration

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Result of a migration scan.
#[derive(Debug, Serialize, Deserialize)]
pub struct MigrationResult {
    pub migrated: Vec<MigratedItem>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MigratedItem {
    pub source: String,
    pub item_type: String, // "config", "skill", "workspace", "secret"
    pub status: String,    // "migrated", "skipped", "error"
}

/// Detect if an OpenClaw directory exists.
pub fn detect_openclaw(source: Option<&str>) -> Option<PathBuf> {
    let dir = match source {
        Some(path) => PathBuf::from(path),
        None => dirs::home_dir()?.join(".openclaw"),
    };
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// Scan what items exist in an OpenClaw directory.
pub fn scan_openclaw(source: &PathBuf) -> Result<Vec<String>> {
    let mut items = Vec::new();
    if !source.exists() {
        return Ok(items);
    }
    for entry in std::fs::read_dir(source).context("Failed to read OpenClaw dir")? {
        let entry = entry?;
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Ok(name) = entry.file_name().into_string() {
                // Known OpenClaw item types
                match name.as_str() {
                    "skills" | "config" | "workspaces" | "secrets" => items.push(name),
                    _ => {} // skip unknown dirs
                }
            }
        }
    }
    Ok(items)
}

/// Perform a dry-run migration (preview only).
pub fn dry_run_migrate(source: &PathBuf, target_skills_dir: &PathBuf) -> Result<MigrationResult> {
    let items = scan_openclaw(source)?;
    let mut result = MigrationResult {
        migrated: Vec::new(),
        errors: Vec::new(),
    };

    for item in &items {
        let src = source.join(item);
        let _dst = match item.as_str() {
            "skills" => target_skills_dir.join("openclaw-imported"),
            _ => continue, // only skills migration in dry-run
        };
        result.migrated.push(MigratedItem {
            source: src.to_string_lossy().to_string(),
            item_type: item.clone(),
            status: "would-migrate".to_string(),
        });
    }
    Ok(result)
}

/// Perform actual migration — copy OpenClaw skills to Hermes skills dir.
pub fn migrate_skills(
    source: &PathBuf,
    target_skills_dir: &PathBuf,
    overwrite: bool,
) -> Result<MigrationResult> {
    let openclaw_skills = source.join("skills");
    if !openclaw_skills.is_dir() {
        return Ok(MigrationResult {
            migrated: Vec::new(),
            errors: Vec::new(),
        });
    }

    let import_dir = target_skills_dir.join("openclaw-imported");
    std::fs::create_dir_all(&import_dir)?;

    let mut result = MigrationResult {
        migrated: Vec::new(),
        errors: Vec::new(),
    };

    for entry in std::fs::read_dir(&openclaw_skills)? {
        let entry = entry?;
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        let dst = import_dir.join(&name_str);

        if dst.exists() {
            if overwrite {
                std::fs::remove_dir_all(&dst)?;
            } else {
                result.migrated.push(MigratedItem {
                    source: entry.path().to_string_lossy().to_string(),
                    item_type: "skill".to_string(),
                    status: "skipped (exists)".to_string(),
                });
                continue;
            }
        }

        match copy_dir_recursive(&entry.path(), &dst) {
            Ok(()) => result.migrated.push(MigratedItem {
                source: entry.path().to_string_lossy().to_string(),
                item_type: "skill".to_string(),
                status: "migrated".to_string(),
            }),
            Err(e) => result
                .errors
                .push(format!("Failed to migrate '{}': {}", name_str, e)),
        }
    }

    Ok(result)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// Clean up ~/.openclaw directory (make a backup first).
pub fn cleanup_openclaw(source: &PathBuf, dry_run: bool) -> Result<Vec<String>> {
    if !source.exists() {
        return Ok(vec!["No OpenClaw directory found.".to_string()]);
    }

    let backup_path = source.with_extension("openclaw.backup");
    if dry_run {
        return Ok(vec![format!(
            "Would move '{}' to '{}'",
            source.display(),
            backup_path.display()
        )]);
    }

    std::fs::rename(source, &backup_path)?;
    Ok(vec![
        format!(
            "Moved '{}' to '{}'",
            source.display(),
            backup_path.display()
        ),
        format!("Delete the backup with: rm -rf '{}'", backup_path.display()),
    ])
}
