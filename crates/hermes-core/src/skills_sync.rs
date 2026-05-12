//! Skills sync — manifest-based skill synchronization.
//!
//! Manages syncing bundled skills from a manifest into the user's
//! skills directory, with conflict detection and resolution.
//! Ported from `hermes-agent/tools/skills_sync.py`.
//!
//! Manifest format:
//! - V1: `{"skills": [...]}` (flat, no version field)
//! - V2: `{"version": 2, "skills": [...]}` (versioned)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A single entry in the skills manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Skill name (matches SKILL.md frontmatter `name:` field).
    pub name: String,
    /// Source identifier (e.g., "bundled", "hub", "url").
    pub source: String,
    /// Semantic version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
}

/// Manifest describing a collection of skills.
///
/// Supports both V1 (no version field, assumed version 1)
/// and V2 (explicit `version: 2` field) formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version (1 or 2).
    pub version: u8,
    /// List of skill entries.
    pub skills: Vec<ManifestEntry>,
}

/// Report from a sync operation detailing what happened.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    /// Skills that were newly added.
    pub added: Vec<String>,
    /// Skills that were updated to a new version.
    pub updated: Vec<String>,
    /// Skills that were skipped (already up-to-date).
    pub skipped: Vec<String>,
    /// Skills that were removed (exist on disk but not in manifest).
    pub removed: Vec<String>,
    /// Skills with conflicts (e.g., name collision, directory collision).
    pub conflicts: Vec<String>,
}

// ---------------------------------------------------------------------------
// Manifest I/O
// ---------------------------------------------------------------------------

/// Read a manifest from a JSON file.
///
/// Auto-detects V1 (`{"skills": [...]}`) and V2 (`{"version": 2, "skills": [...]}`) formats.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read manifest from {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse manifest at {}", path.display()))?;

    if value.get("version").and_then(|v| v.as_u64()).is_some() {
        // V2 format: {"version": 2, "skills": [...]}
        let manifest: Manifest =
            serde_json::from_value(value).with_context(|| "Failed to parse V2 manifest")?;
        Ok(manifest)
    } else {
        // V1 format: {"skills": [...]} — no version field
        let skills: Vec<ManifestEntry> = serde_json::from_value(
            value
                .get("skills")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Missing 'skills' field in V1 manifest"))?,
        )
        .with_context(|| "Failed to parse V1 manifest skills")?;
        Ok(Manifest { version: 1, skills })
    }
}

/// Write a manifest to a JSON file atomically (temp file + rename).
pub fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("manifest.tmp");
    let content = serde_json::to_string_pretty(manifest)?;
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sync logic
// ---------------------------------------------------------------------------

/// Sync skills from a manifest into the skills directory.
///
/// Scans the manifest and the target directory, comparing versions to decide
/// what to add, update, skip, or flag as a conflict.
///
/// # Arguments
/// * `manifest` — The manifest describing desired skills.
/// * `skills_dir` — The target directory to sync into.
/// * `dry_run` — If true, only report what would happen; don't write files.
///
/// Returns a `SyncReport` detailing every action taken.
pub fn sync_skills(manifest: &Manifest, skills_dir: &Path, dry_run: bool) -> Result<SyncReport> {
    let mut report = SyncReport::default();

    // Ensure skills directory exists (unless dry run)
    if !dry_run {
        std::fs::create_dir_all(skills_dir)?;
    }

    // Index existing skill directories
    let existing: HashMap<String, PathBuf> = if skills_dir.exists() {
        let mut map = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        // Skip hidden directories (.archive, .hub, etc.)
                        if !name.starts_with('.') {
                            map.insert(name.to_string(), path);
                        }
                    }
                }
            }
        }
        map
    } else {
        HashMap::new()
    };

    // Index manifest by name for quick lookup
    let manifest_names: HashSet<&str> = manifest.skills.iter().map(|e| e.name.as_str()).collect();

    // Process each manifest entry
    for entry in &manifest.skills {
        let dest = skills_dir.join(&entry.name);

        // Scenario 1: NEW — skill not in directory
        // Scenario 2: MISSING — skill was deleted by user, not re-added
        if !existing.contains_key(&entry.name) {
            report.added.push(entry.name.clone());
            if !dry_run {
                if let Err(e) = create_skill_directory(&dest, entry) {
                    report
                        .conflicts
                        .push(format!("{}: failed to create: {}", entry.name, e));
                }
            }
            continue;
        }

        // Scenario 3: NAME COLLISION — directory exists but no SKILL.md
        let dir_path = &existing[&entry.name];
        let skill_md = dir_path.join("SKILL.md");
        if !skill_md.exists() {
            report.conflicts.push(format!(
                "{}: directory exists but no SKILL.md found",
                entry.name
            ));
            continue;
        }

        // Scenario 4: DIRECTORY COLLISION — multiple skills map to same dir
        let dir_name = dir_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if dir_name != entry.name {
            report.conflicts.push(format!(
                "{}: directory name mismatch ('{}' vs '{}')",
                entry.name, dir_name, entry.name
            ));
            continue;
        }

        // Read existing version from SKILL.md frontmatter
        let existing_version = extract_version_from_skill(&skill_md);

        match existing_version {
            None => {
                // SKILL.md exists but has no version field — flag as conflict
                report.conflicts.push(format!(
                    "{}: SKILL.md exists but version field missing",
                    entry.name
                ));
            }
            Some(ref ver) if ver != &entry.version => {
                // Scenario 5: DIFFERENT VERSION — update
                report.updated.push(entry.name.clone());
                if !dry_run {
                    if let Err(e) = update_skill_directory(dir_path, entry) {
                        report
                            .conflicts
                            .push(format!("{}: failed to update: {}", entry.name, e));
                    }
                }
            }
            Some(_) => {
                // Scenario 6: SAME VERSION — skip (already in sync)
                report.skipped.push(entry.name.clone());
            }
        }
    }

    // Detect REMOVED: on disk but not in manifest
    for name in existing.keys() {
        if !manifest_names.contains(name.as_str()) {
            report.removed.push(name.clone());
        }
    }

    Ok(report)
}

/// Create a skill directory from a manifest entry.
fn create_skill_directory(dest: &Path, entry: &ManifestEntry) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let skill_content = format!(
        "---\nname: {}\ndescription: {}\nversion: {}\nsource: {}\n---\n",
        entry.name, entry.description, entry.version, entry.source
    );
    std::fs::write(dest.join("SKILL.md"), &skill_content)?;
    Ok(())
}

/// Update an existing skill directory with new manifest content.
fn update_skill_directory(dir_path: &Path, entry: &ManifestEntry) -> Result<()> {
    let skill_content = format!(
        "---\nname: {}\ndescription: {}\nversion: {}\nsource: {}\n---\n",
        entry.name, entry.description, entry.version, entry.source
    );
    std::fs::write(dir_path.join("SKILL.md"), &skill_content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reset & query
// ---------------------------------------------------------------------------

/// Force reinstall a bundled skill from the manifest.
///
/// Removes the existing skill directory (if any) and recreates it
/// from the manifest entry.
pub fn reset_bundled_skill(name: &str, manifest: &Manifest, skills_dir: &Path) -> Result<()> {
    let entry = manifest
        .skills
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found in manifest", name))?;

    let dest = skills_dir.join(name);
    // Remove existing directory if present
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    // Recreate from manifest
    create_skill_directory(&dest, entry)
}

/// Get all non-user (bundled) skills from the manifest.
///
/// Filters entries where `source == "bundled"`.
pub fn find_bundled_skills(manifest: &Manifest) -> Vec<&ManifestEntry> {
    manifest
        .skills
        .iter()
        .filter(|e| e.source == "bundled")
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the `version:` field from a SKILL.md YAML frontmatter.
fn extract_version_from_skill(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_open = &trimmed[3..];
    let close_pos = after_open.find("\n---")?;
    let fm_block = &after_open[..close_pos];

    for line in fm_block.lines() {
        let line = line.trim();
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            let value = line[colon_pos + 1..]
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if key == "version" && !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("hermes_sync_test_{}_{}", std::process::id(), count));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn sample_manifest_v1() -> Manifest {
        Manifest {
            version: 1,
            skills: vec![
                ManifestEntry {
                    name: "skill-a".into(),
                    source: "bundled".into(),
                    version: "1.0.0".into(),
                    description: "First test skill".into(),
                },
                ManifestEntry {
                    name: "skill-b".into(),
                    source: "bundled".into(),
                    version: "2.0.0".into(),
                    description: "Second test skill".into(),
                },
            ],
        }
    }

    fn sample_manifest_v2() -> Manifest {
        Manifest {
            version: 2,
            skills: vec![
                ManifestEntry {
                    name: "skill-a".into(),
                    source: "bundled".into(),
                    version: "1.0.0".into(),
                    description: "First test skill".into(),
                },
                ManifestEntry {
                    name: "skill-b".into(),
                    source: "bundled".into(),
                    version: "2.0.0".into(),
                    description: "Second test skill".into(),
                },
            ],
        }
    }

    fn create_skill_at(dir: &Path, name: &str, version: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = format!(
            "---\nname: {}\ndescription: Test\nversion: {}\nsource: bundled\n---\n# {}\n",
            name, version, name
        );
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    // -----------------------------------------------------------------------
    // Manifest I/O tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_manifest_v1() {
        let tmp = test_dir();
        let path = tmp.join("manifest.json");
        let v1 = sample_manifest_v1();
        write_manifest(&path, &v1).unwrap();

        let loaded = read_manifest(&path).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.skills.len(), 2);
        assert_eq!(loaded.skills[0].name, "skill-a");
        assert_eq!(loaded.skills[0].version, "1.0.0");

        cleanup(&tmp);
    }

    #[test]
    fn test_read_manifest_v2() {
        let tmp = test_dir();
        let path = tmp.join("manifest_v2.json");
        let v2 = sample_manifest_v2();
        write_manifest(&path, &v2).unwrap();

        let loaded = read_manifest(&path).unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.skills.len(), 2);

        cleanup(&tmp);
    }

    #[test]
    fn test_read_manifest_missing_file() {
        let tmp = test_dir();
        let path = tmp.join("nonexistent.json");
        let result = read_manifest(&path);
        assert!(result.is_err());
        cleanup(&tmp);
    }

    #[test]
    fn test_read_manifest_auto_detect_v1_no_version_field() {
        let tmp = test_dir();
        let path = tmp.join("auto_v1.json");
        // Write raw V1 JSON (no version field)
        let raw = r#"{"skills": [{"name": "x", "source": "bundled", "version": "1.0", "description": "X"}]}"#;
        std::fs::write(&path, raw).unwrap();
        let manifest = read_manifest(&path).unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.skills.len(), 1);
        cleanup(&tmp);
    }

    #[test]
    fn test_write_manifest_roundtrip() {
        let tmp = test_dir();
        let path = tmp.join("roundtrip.json");
        let original = sample_manifest_v2();
        write_manifest(&path, &original).unwrap();

        let loaded = read_manifest(&path).unwrap();
        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.skills.len(), original.skills.len());
        assert_eq!(loaded.skills[0].name, original.skills[0].name);
        assert_eq!(loaded.skills[0].version, original.skills[0].version);

        cleanup(&tmp);
    }

    // -----------------------------------------------------------------------
    // Sync skills tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sync_skills_adds_new() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manifest = sample_manifest_v1();
        let report = sync_skills(&manifest, &skills_dir, false).unwrap();

        assert_eq!(report.added.len(), 2);
        assert!(report.added.contains(&"skill-a".to_string()));
        assert!(report.added.contains(&"skill-b".to_string()));
        assert!(report.updated.is_empty());
        assert!(report.conflicts.is_empty());

        // Verify files were created
        assert!(skills_dir.join("skill-a").join("SKILL.md").exists());
        assert!(skills_dir.join("skill-b").join("SKILL.md").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_sync_skills_skips_same_version() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Pre-create skill-a with the same version as manifest
        create_skill_at(&skills_dir, "skill-a", "1.0.0");

        let manifest = sample_manifest_v1();
        let report = sync_skills(&manifest, &skills_dir, false).unwrap();

        // skill-a should be skipped (same version), skill-b added
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0], "skill-a");
        assert_eq!(report.added.len(), 1);
        assert_eq!(report.added[0], "skill-b");

        cleanup(&tmp);
    }

    #[test]
    fn test_sync_skills_updates_different_version() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Pre-create skill-a with older version
        create_skill_at(&skills_dir, "skill-a", "0.9.0");

        let manifest = sample_manifest_v1();
        let report = sync_skills(&manifest, &skills_dir, false).unwrap();

        assert_eq!(report.updated.len(), 1);
        assert_eq!(report.updated[0], "skill-a");
        assert_eq!(report.added.len(), 1);
        assert_eq!(report.added[0], "skill-b");

        cleanup(&tmp);
    }

    #[test]
    fn test_sync_skills_dry_run_does_not_write() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");

        let manifest = sample_manifest_v1();
        let report = sync_skills(&manifest, &skills_dir, true).unwrap();

        assert_eq!(report.added.len(), 2);
        // Dry run should NOT create the directory
        assert!(!skills_dir.exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_sync_skills_removed_detection() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create a skill on disk that is NOT in manifest
        create_skill_at(&skills_dir, "orphan-skill", "1.0.0");

        let manifest = Manifest {
            version: 1,
            skills: vec![],
        };
        let report = sync_skills(&manifest, &skills_dir, false).unwrap();

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0], "orphan-skill");

        cleanup(&tmp);
    }

    #[test]
    fn test_sync_skills_conflict_no_skill_md() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create a directory with the same name as a manifest skill but NO SKILL.md
        let clash_dir = skills_dir.join("skill-a");
        std::fs::create_dir_all(&clash_dir).unwrap();
        std::fs::write(clash_dir.join("README.md"), "not a skill").unwrap();

        let manifest = sample_manifest_v1();
        let report = sync_skills(&manifest, &skills_dir, false).unwrap();

        assert!(report.conflicts.iter().any(|c| c.contains("skill-a")));

        cleanup(&tmp);
    }

    #[test]
    fn test_sync_skills_skips_hidden_dirs() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Hidden directory should be ignored by sync
        let hidden = skills_dir.join(".archive");
        std::fs::create_dir_all(&hidden).unwrap();

        let manifest = sample_manifest_v1();
        let report = sync_skills(&manifest, &skills_dir, false).unwrap();

        // Hidden dir should not appear in removed
        assert!(report.removed.is_empty());
        assert_eq!(report.added.len(), 2);

        cleanup(&tmp);
    }

    // -----------------------------------------------------------------------
    // Reset bundled skill tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_reset_bundled_skill_reinstalls() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manifest = sample_manifest_v1();

        // Create a skill with modified content
        create_skill_at(&skills_dir, "skill-a", "0.5.0");

        // Reset should restore to manifest version
        reset_bundled_skill("skill-a", &manifest, &skills_dir).unwrap();

        let skill_md = skills_dir.join("skill-a").join("SKILL.md");
        assert!(skill_md.exists());

        let content = std::fs::read_to_string(&skill_md).unwrap();
        assert!(content.contains("version: 1.0.0"));

        cleanup(&tmp);
    }

    #[test]
    fn test_reset_bundled_skill_not_in_manifest() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manifest = sample_manifest_v1();
        let result = reset_bundled_skill("nonexistent", &manifest, &skills_dir);
        assert!(result.is_err());

        cleanup(&tmp);
    }

    #[test]
    fn test_reset_bundled_skill_creates_new_directory() {
        let tmp = test_dir();
        let skills_dir = tmp.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manifest = sample_manifest_v1();
        // Skill doesn't exist on disk yet
        reset_bundled_skill("skill-a", &manifest, &skills_dir).unwrap();

        assert!(skills_dir.join("skill-a").join("SKILL.md").exists());

        cleanup(&tmp);
    }

    // -----------------------------------------------------------------------
    // find_bundled_skills tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_bundled_skills_filters_by_source() {
        let manifest = Manifest {
            version: 2,
            skills: vec![
                ManifestEntry {
                    name: "bundled-one".into(),
                    source: "bundled".into(),
                    version: "1.0.0".into(),
                    description: String::new(),
                },
                ManifestEntry {
                    name: "hub-one".into(),
                    source: "hub".into(),
                    version: "1.0.0".into(),
                    description: String::new(),
                },
                ManifestEntry {
                    name: "bundled-two".into(),
                    source: "bundled".into(),
                    version: "2.0.0".into(),
                    description: String::new(),
                },
            ],
        };

        let bundled = find_bundled_skills(&manifest);
        assert_eq!(bundled.len(), 2);
        assert!(bundled.iter().any(|e| e.name == "bundled-one"));
        assert!(bundled.iter().any(|e| e.name == "bundled-two"));
    }

    #[test]
    fn test_find_bundled_skills_empty_when_none() {
        let manifest = Manifest {
            version: 2,
            skills: vec![ManifestEntry {
                name: "hub-only".into(),
                source: "hub".into(),
                version: "1.0.0".into(),
                description: String::new(),
            }],
        };

        let bundled = find_bundled_skills(&manifest);
        assert!(bundled.is_empty());
    }

    // -----------------------------------------------------------------------
    // extract_version_from_skill tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_version_from_skill() {
        let tmp = test_dir();
        let skill_md = tmp.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: test\ndescription: Test\nversion: 3.0.0\n---\n# Content\n",
        )
        .unwrap();

        let version = extract_version_from_skill(&skill_md);
        assert_eq!(version, Some("3.0.0".into()));

        cleanup(&tmp);
    }

    #[test]
    fn test_extract_version_missing() {
        let tmp = test_dir();
        let skill_md = tmp.join("SKILL.md");
        std::fs::write(&skill_md, "---\nname: test\n---\n# Content\n").unwrap();

        let version = extract_version_from_skill(&skill_md);
        assert!(version.is_none());

        cleanup(&tmp);
    }

    #[test]
    fn test_extract_version_no_frontmatter() {
        let tmp = test_dir();
        let skill_md = tmp.join("SKILL.md");
        std::fs::write(&skill_md, "# Just content\n").unwrap();

        let version = extract_version_from_skill(&skill_md);
        assert!(version.is_none());

        cleanup(&tmp);
    }
}
