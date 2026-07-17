//! Integration tests for the curator module.
//!
//! Tests backup/archive/restore operations, state serialization,
//! and the curator engine lifecycle.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use operant_core::curator::{CuratorEngine, CuratorState, archiver, backup};
use operant_core::skill_usage::SkillUsageTracker;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "operant_curator_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// CuratorState tests
// ---------------------------------------------------------------------------

#[test]
fn test_curator_state_default() {
    let state = CuratorState::default();
    assert!(state.enabled);
    assert!(!state.paused);
    assert_eq!(state.interval_hours, 24);
    assert_eq!(state.stale_after_days, 14);
    assert_eq!(state.archive_after_days, 30);
    assert_eq!(state.run_count, 0);
    assert!(state.last_run_at.is_none());
    assert!(state.last_run_summary.is_none());
}

#[test]
fn test_curator_state_serialization_roundtrip() {
    let tmp = temp_dir();
    let path = tmp.join("state.json");

    let state = CuratorState {
        enabled: true,
        paused: false,
        interval_hours: 48,
        last_run_at: Some(1234567890),
        last_run_summary: Some("test run".into()),
        run_count: 5,
        last_report_path: Some(PathBuf::from("/tmp/report.md")),
        stale_after_days: 7,
        archive_after_days: 14,
    };

    let json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(&path, &json).unwrap();

    let loaded: CuratorState = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(loaded.enabled, state.enabled);
    assert_eq!(loaded.interval_hours, 48);
    assert_eq!(loaded.last_run_at, Some(1234567890));
    assert_eq!(loaded.run_count, 5);
    assert_eq!(loaded.stale_after_days, 7);
    assert_eq!(loaded.archive_after_days, 14);

    cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// Archiver tests
// ---------------------------------------------------------------------------

#[test]
fn test_archive_and_restore_skill() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let archive_dir = tmp.join("archive");

    // Create a test skill
    let skill_path = skills_dir.join("test-skill");
    fs::create_dir_all(&skill_path).unwrap();
    fs::write(skill_path.join("SKILL.md"), "# Test Skill").unwrap();

    // Archive it
    archiver::archive_skill("test-skill", &skills_dir, &archive_dir).unwrap();
    assert!(!skill_path.exists());
    assert!(archive_dir.join("test-skill").exists());

    // Restore it
    archiver::restore_skill("test-skill", &archive_dir, &skills_dir).unwrap();
    assert!(skill_path.exists());
    assert!(!archive_dir.join("test-skill").exists());

    cleanup(&tmp);
}

#[test]
fn test_list_archived() {
    let tmp = temp_dir();
    let archive_dir = tmp.join("archive");
    fs::create_dir_all(archive_dir.join("skill-a")).unwrap();
    fs::create_dir_all(archive_dir.join("skill-b")).unwrap();

    let list = archiver::list_archived(&archive_dir).unwrap();
    assert_eq!(list, vec!["skill-a", "skill-b"]);

    cleanup(&tmp);
}

#[test]
fn test_list_archived_empty_dir() {
    let tmp = temp_dir();
    let archive_dir = tmp.join("nonexistent");
    let list = archiver::list_archived(&archive_dir).unwrap();
    assert!(list.is_empty());
    cleanup(&tmp);
}

#[test]
fn test_archive_nonexistent_skill_errors() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let archive_dir = tmp.join("archive");
    let result = archiver::archive_skill("does-not-exist", &skills_dir, &archive_dir);
    assert!(result.is_err());
    cleanup(&tmp);
}

#[test]
fn test_prune_archived_empty_dir() {
    let tmp = temp_dir();
    let archive_dir = tmp.join("archive");
    let result = archiver::prune_archived(&archive_dir, 1).unwrap();
    assert!(result.is_empty());
    cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// Backup tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_backup_and_list() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let backup_dir = tmp.join("backups");

    // Create a skill
    let skill_path = skills_dir.join("my-skill");
    fs::create_dir_all(&skill_path).unwrap();
    fs::write(skill_path.join("SKILL.md"), "# My Skill").unwrap();

    // Create backup
    let backup_path = backup::create_backup(&skills_dir, &backup_dir, Some("test")).unwrap();
    assert!(backup_path.exists());
    assert!(
        backup_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".tar.gz")
    );

    // List backups
    let backups = backup::list_backups(&backup_dir).unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0], backup_path);

    cleanup(&tmp);
}

#[test]
fn test_list_backups_empty() {
    let tmp = temp_dir();
    let backup_dir = tmp.join("nonexistent");
    let backups = backup::list_backups(&backup_dir).unwrap();
    assert!(backups.is_empty());
    cleanup(&tmp);
}

#[test]
fn test_backup_restore_roundtrip() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let backup_dir = tmp.join("backups");
    let restore_dir = tmp.join("restored-skills");

    // Create a skill and a file in it
    let skill_path = skills_dir.join("roundtrip-skill");
    fs::create_dir_all(&skill_path).unwrap();
    fs::write(skill_path.join("SKILL.md"), "# Roundtrip Skill").unwrap();
    fs::write(skill_path.join("script.py"), "print('hello')").unwrap();

    // Create backup
    let backup_path = backup::create_backup(&skills_dir, &backup_dir, Some("roundtrip")).unwrap();

    // Restore into a different location
    let _rollback_path = backup::restore_backup(&backup_path, &restore_dir).unwrap();

    // Verify restored content
    assert!(
        restore_dir
            .join("roundtrip-skill")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        restore_dir
            .join("roundtrip-skill")
            .join("script.py")
            .exists()
    );
    let content = fs::read_to_string(restore_dir.join("roundtrip-skill").join("SKILL.md")).unwrap();
    assert!(content.contains("# Roundtrip Skill"));

    cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// CuratorEngine tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_curator_engine_load_state_default() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let state_path = tmp.join("state.json");
    let usage_path = tmp.join("usage.json");
    let tracker = Arc::new(SkillUsageTracker::new(usage_path));

    let engine = CuratorEngine::new(skills_dir, state_path.clone(), tracker);
    let state = engine.load_state().await.unwrap();

    // Default state should be created
    assert!(state.enabled);
    assert!(!state.paused);
    assert!(state_path.exists());

    cleanup(&tmp);
}

#[tokio::test]
async fn test_curator_engine_load_state_existing() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let state_path = tmp.join("state.json");
    let usage_path = tmp.join("usage.json");

    // Write state to disk
    let initial_state = CuratorState {
        enabled: false,
        paused: true,
        run_count: 42,
        ..CuratorState::default()
    };
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&initial_state).unwrap(),
    )
    .unwrap();

    let tracker = Arc::new(SkillUsageTracker::new(usage_path));
    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker);

    // Load - should exercise load+save cycle, creating archive/backup dirs
    let _state = engine.load_state().await.unwrap();

    // Note: archive_dir is created by archiver::archive_skill, not on construction
    cleanup(&tmp);
}

#[tokio::test]
async fn test_curator_engine_pause_resume() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let state_path = tmp.join("state.json");
    let usage_path = tmp.join("usage.json");
    let tracker = Arc::new(SkillUsageTracker::new(usage_path));

    let engine = CuratorEngine::new(skills_dir, state_path, tracker);
    engine.load_state().await.unwrap();

    // Initially active
    assert!(engine.is_active().await);

    // Pause
    engine.set_paused(true).await.unwrap();
    assert!(!engine.is_active().await);

    // Resume
    engine.set_paused(false).await.unwrap();
    assert!(engine.is_active().await);

    cleanup(&tmp);
}

#[tokio::test]
async fn test_curator_engine_dry_run() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let state_path = tmp.join("state.json");
    let usage_path = tmp.join("usage.json");

    // Create a usage file with an agent-created skill
    let tracker = Arc::new(SkillUsageTracker::new(usage_path.clone()));
    tracker.load().unwrap();
    tracker.mark_agent_created("dry-run-skill");

    // Create the skill directory
    let skill_dir = skills_dir.join("dry-run-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "# Dry Run Skill").unwrap();

    tracker.save().unwrap();

    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker);
    engine.load_state().await.unwrap();

    let report = engine.run_review(true, None).await.unwrap();

    // Dry run should report but not archive
    assert_eq!(report.skills_scanned, 1);
    // Skill should NOT be archived (it was just created, so inactive_days is near 0)
    assert!(report.skills_archived.is_empty());
    assert!(report.skills_stale.is_empty());

    cleanup(&tmp);
}

#[tokio::test]
async fn test_curator_engine_get_state() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let state_path = tmp.join("state.json");
    let usage_path = tmp.join("usage.json");
    let tracker = Arc::new(SkillUsageTracker::new(usage_path));

    let engine = CuratorEngine::new(skills_dir, state_path, tracker);
    engine.load_state().await.unwrap();

    let state = engine.get_state().await;
    assert!(state.enabled);
    assert!(!state.paused);

    cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// CLI workflow integration tests (archive → list → restore → prune lifecycle)
// ---------------------------------------------------------------------------

#[test]
fn test_full_archive_lifecycle() {
    // Exercises the exact operations the CLI handlers wire: archive → list → restore
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let archive_dir = tmp.join(".archive");

    // Create two test skills
    fs::create_dir_all(skills_dir.join("skill-alpha")).unwrap();
    fs::write(
        skills_dir.join("skill-alpha").join("SKILL.md"),
        "# Skill Alpha",
    )
    .unwrap();
    fs::create_dir_all(skills_dir.join("skill-beta")).unwrap();
    fs::write(
        skills_dir.join("skill-beta").join("SKILL.md"),
        "# Skill Beta",
    )
    .unwrap();

    // --- List archived (should be empty) ---
    let archived = archiver::list_archived(&archive_dir).unwrap();
    assert!(archived.is_empty());

    // --- Archive both skills ---
    archiver::archive_skill("skill-alpha", &skills_dir, &archive_dir).unwrap();
    archiver::archive_skill("skill-beta", &skills_dir, &archive_dir).unwrap();
    assert!(!skills_dir.join("skill-alpha").exists());
    assert!(archive_dir.join("skill-alpha").exists());
    assert!(!skills_dir.join("skill-beta").exists());

    // --- List archived (should show both) ---
    let archived = archiver::list_archived(&archive_dir).unwrap();
    assert_eq!(archived.len(), 2);
    assert!(archived.contains(&"skill-alpha".to_string()));
    assert!(archived.contains(&"skill-beta".to_string()));

    // --- Restore skill-alpha ---
    archiver::restore_skill("skill-alpha", &archive_dir, &skills_dir).unwrap();
    assert!(skills_dir.join("skill-alpha").exists());
    assert!(!archive_dir.join("skill-alpha").exists());
    // skill-beta should still be archived
    assert!(archive_dir.join("skill-beta").exists());

    // --- List archived (only skill-beta remains) ---
    let archived = archiver::list_archived(&archive_dir).unwrap();
    assert_eq!(archived, vec!["skill-beta"]);

    // --- Restore skill-beta ---
    archiver::restore_skill("skill-beta", &archive_dir, &skills_dir).unwrap();
    let archived = archiver::list_archived(&archive_dir).unwrap();
    assert!(archived.is_empty());

    cleanup(&tmp);
}

#[test]
fn test_prune_keeps_archived_skills_below_threshold() {
    // Exercises the CLI prune handler path: prune_archived with a time threshold.
    // Verifies that recently archived skills are NOT pruned.
    let tmp = temp_dir();
    let archive_dir = tmp.join(".archive");

    // Create a recently archived skill
    fs::create_dir_all(archive_dir.join("recent-skill")).unwrap();
    fs::write(
        archive_dir.join("recent-skill").join("SKILL.md"),
        "# Recent",
    )
    .unwrap();

    // Prune with a low threshold — recent-skill should survive since it was just created
    let pruned = archiver::prune_archived(&archive_dir, 1).unwrap();
    assert!(
        pruned.is_empty(),
        "recently created skill should not be pruned"
    );
    assert!(archive_dir.join("recent-skill").exists());

    cleanup(&tmp);
}

#[test]
fn test_backup_and_rollback_preserves_skills() {
    // Exercises the CLI backup/rollback handler path
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let backup_dir = tmp.join(".backups");

    // Create a skill
    let skill_path = skills_dir.join("important-skill");
    fs::create_dir_all(&skill_path).unwrap();
    fs::write(
        skill_path.join("SKILL.md"),
        "# Important Skill\noriginal content",
    )
    .unwrap();

    // --- Create backup ---
    let backup_path = backup::create_backup(&skills_dir, &backup_dir, Some("pre-change")).unwrap();
    assert!(backup_path.exists());

    // --- Modify the skill ---
    fs::write(
        skill_path.join("SKILL.md"),
        "# Important Skill\nmodified content",
    )
    .unwrap();

    // --- List backups ---
    let backups = backup::list_backups(&backup_dir).unwrap();
    assert_eq!(backups.len(), 1);

    // --- Rollback to backup ---
    let rollback_dir = backup::restore_backup(&backup_path, &skills_dir).unwrap();
    // Rollback dir should exist (previous state was preserved)
    assert!(rollback_dir.exists());

    // Verify original content is restored
    let content = fs::read_to_string(skills_dir.join("important-skill").join("SKILL.md")).unwrap();
    assert!(
        content.contains("original content"),
        "Rollback should restore original content"
    );

    cleanup(&tmp);
}
