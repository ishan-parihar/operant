//! Integration tests for the curator module.
//!
//! Tests backup/archive/restore operations, state serialization,
//! and the curator engine lifecycle.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use operant_core::curator::{CuratorEngine, CuratorState, archiver, backup};
use operant_core::skill_usage::{SkillUsageTracker, UsageTelemetry};

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

    let report = engine.run_review(true, None, false, None).await.unwrap();

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

/// RT-261: a skill that is USED must not be archived by the curator.
///
/// The curator archives an agent-created skill after `archive_after_days` of
/// INACTIVITY, measured from `last_used`. Nothing recorded a use — so
/// `use_count` stayed 0 and `last_used` stayed at creation time — and a skill
/// that was invoked every day was archived like an abandoned one.
#[tokio::test]
async fn a_used_skill_is_not_archived_by_the_curator() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let skill_dir = skills_dir.join("used-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: used-skill\ndescription: a skill in daily use\n---\n\nbody\n",
    )
    .unwrap();

    // The curator's telemetry: an AGENT-CREATED skill whose last use was 40
    // days ago (the default archive window is 30).
    let usage_path = skills_dir.join(".curator").join("usage.json");
    fs::create_dir_all(usage_path.parent().unwrap()).unwrap();
    {
        let mut telemetry = UsageTelemetry::load(&usage_path).unwrap();
        telemetry.record_created("used-skill", true);
        telemetry.get_record_mut("used-skill").unwrap().last_used =
            chrono::Utc::now() - chrono::Duration::days(40);
        telemetry.save(&usage_path).unwrap();
    }

    // The skill is invoked — the production path the TUI's `/skill` and the
    // gateway's skill commands both use.
    let invoked = operant_core::agent::skill_preprocessing::build_skill_invocation_message_in(
        &skills_dir,
        "used-skill",
        "",
        None,
    );
    assert!(
        invoked.is_some(),
        "the skill must resolve and be injected: {invoked:?}"
    );

    // The curator now runs with a ONE-day window: an unused skill would be
    // archived in this run, so the skill's survival is the assertion.
    let state_path = tmp.join("state.json");
    let state = CuratorState {
        stale_after_days: 1,
        archive_after_days: 1,
        ..CuratorState::default()
    };
    fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    let tracker = Arc::new(SkillUsageTracker::new(usage_path));
    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker);
    engine.load_state().await.unwrap();
    let counts = engine.apply_automatic_transitions(None).await.unwrap();

    assert_eq!(
        counts.get("archived").copied().unwrap_or(0),
        0,
        "a skill in daily use must not be archived: {counts:?}"
    );
    assert!(
        skill_dir.exists(),
        "the skill directory must survive the curator's pass"
    );

    cleanup(&tmp);
}

/// RT-267: the automatic curator must protect a skill a cron job references
/// — exactly like the manual review path.
///
/// The two paths implemented the same rule differently: the manual review
/// skipped cron-referenced skills, the automatic transitions had NO cron
/// guard at all — and both read the protection set from a database no cron
/// job ever writes to, so the rule protected nothing either way.
#[tokio::test]
async fn the_automatic_curator_never_archives_a_cron_referenced_skill() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    let skill_dir = skills_dir.join("cron-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "# Cron Skill").unwrap();

    // The usage record: agent-created and long idle (older than the
    // one-day archive window below).
    let usage_path = skills_dir.join(".curator").join("usage.json");
    fs::create_dir_all(usage_path.parent().unwrap()).unwrap();
    {
        let mut telemetry = UsageTelemetry::load(&usage_path).unwrap();
        telemetry.record_created("cron-skill", true);
        telemetry.get_record_mut("cron-skill").unwrap().last_used =
            chrono::Utc::now() - chrono::Duration::days(40);
        telemetry.save(&usage_path).unwrap();
    }

    // A cron job that calls the skill, in the LIVE cron database (the path
    // the core owns — one rule, one file).
    let cron_db = operant_core::cronjobs::CronDb::init(operant_core::cronjobs::db_path(
        &tmp.join("operant.db"),
    ))
    .unwrap();
    cron_db
        .create_job(operant_core::cronjobs::CreateJobParams {
            name: "nightly".into(),
            prompt: "run the cron skill".into(),
            schedule: "0 3 * * *".into(),
            schedule_display: "nightly".into(),
            repeat_times: None,
            deliver: "local".into(),
            origin_platform: None,
            origin_chat_id: None,
            origin_thread_id: None,
            skill: Some("cron-skill".into()),
            skills: None,
            model: None,
            provider: None,
            base_url: None,
            script: None,
            context_from: None,
            enabled_toolsets: None,
            workdir: None,
            no_agent: false,
        })
        .unwrap();

    let state_path = tmp.join("state.json");
    let state = CuratorState {
        stale_after_days: 1,
        archive_after_days: 1,
        ..CuratorState::default()
    };
    fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    let tracker = Arc::new(SkillUsageTracker::new(usage_path));
    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker);
    engine.load_state().await.unwrap();
    let counts = engine
        .apply_automatic_transitions(Some(&cron_db))
        .await
        .unwrap();

    assert_eq!(
        counts.get("archived").copied().unwrap_or(0),
        0,
        "a skill a cron job calls must not be archived: {counts:?}"
    );
    assert_eq!(
        counts.get("skipped_cron").copied().unwrap_or(0),
        1,
        "the cron skip must be counted, not silent: {counts:?}"
    );
    assert!(
        skill_dir.exists(),
        "the skill directory must survive the curator's pass"
    );

    cleanup(&tmp);
}

/// RT-267: an UNREADABLE cron reference set must ABORT the review, not
/// silently mean "nothing is protected".
///
/// The old code logged a warning and carried on with an EMPTY set — which
/// reads as "no skill is protected" — so one transient sqlite error turned
/// into the permanent archival of a skill a cron job still calls.
#[tokio::test]
async fn an_unreadable_cron_reference_set_aborts_the_review() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // A cron db whose `cron_jobs` table lost the `skills` column: the
    // reference query now fails.
    let cron_path = tmp.join("cron.db");
    let cron_db = operant_core::cronjobs::CronDb::init(cron_path.clone()).unwrap();
    {
        let conn = rusqlite::Connection::open(&cron_path).unwrap();
        conn.execute("ALTER TABLE cron_jobs DROP COLUMN skills", [])
            .unwrap();
    }
    assert!(
        cron_db.referenced_skill_names().is_err(),
        "the fixture must make the cron reference read fail"
    );

    let state_path = tmp.join("state.json");
    let tracker = Arc::new(SkillUsageTracker::new(
        skills_dir.join(".curator").join("usage.json"),
    ));
    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker);
    engine.load_state().await.unwrap();

    let result = engine.run_review(false, None, false, Some(&cron_db)).await;

    assert!(
        result.is_err(),
        "an unreadable protection set must abort the review, not archive blind"
    );

    cleanup(&tmp);
}

/// RT-268: the persisted summary must be THIS run's, not the previous one's.
///
/// `run_review` saved `state.json` and only THEN set `last_run_summary`, so
/// the file on disk always carried the previous run's summary (and none at
/// all after the first run) — and `operant status` reads exactly that file
/// (`cmd_status.rs`), while `operant curator status` read the live value:
/// two panels, one run apart.
#[tokio::test]
async fn the_persisted_summary_is_the_current_runs() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let usage_path = skills_dir.join(".curator").join("usage.json");
    fs::create_dir_all(usage_path.parent().unwrap()).unwrap();

    let state_path = tmp.join("state.json");
    let tracker = Arc::new(SkillUsageTracker::new(usage_path.clone()));
    let engine = CuratorEngine::new(skills_dir.clone(), state_path.clone(), tracker.clone());
    engine.load_state().await.unwrap();

    // Run 1: nothing to scan -> the summary says "Scanned 0".
    let first = engine.run_review(false, None, false, None).await.unwrap();
    assert!(first.summary.contains("Scanned 0"), "{}", first.summary);

    // Run 2: one agent-created skill, old enough to archive.
    let skill_dir = skills_dir.join("archived-by-the-curator");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "# Old").unwrap();
    {
        let mut telemetry = UsageTelemetry::load(&usage_path).unwrap();
        telemetry.record_created("archived-by-the-curator", true);
        telemetry
            .get_record_mut("archived-by-the-curator")
            .unwrap()
            .last_used = chrono::Utc::now() - chrono::Duration::days(90);
        telemetry.save(&usage_path).unwrap();
    }
    let second = engine.run_review(false, None, false, None).await.unwrap();
    assert!(second.summary.contains("Archived: 1"), "{}", second.summary);

    // The file on disk must carry run 2's summary.
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    let persisted_summary = persisted["last_run_summary"].as_str().unwrap_or("");
    assert!(
        persisted_summary.contains("Archived: 1"),
        "the persisted summary must be the CURRENT run's, got: {persisted_summary:?}"
    );

    cleanup(&tmp);
}

/// RT-377: the doc promises "Creates a backup before any archiving when not in
/// dry-run mode" — the backup used to be taken AFTER the archival loop, so the
/// "pre-archive" snapshot captured the tree the loop had already moved and
/// could not roll back the archival it was named for.
///
/// Two halves, both BEHAVIORAL:
///   1. a FAILING backup means the skill is not archived at all (which is only
///      observable if the backup precedes the move);
///   2. a SUCCESSFUL backup's archive contains the skill — the snapshot really
///      is the pre-archive tree.
#[tokio::test]
async fn a_pre_archive_backup_is_taken_before_the_archival() {
    let tmp = temp_dir();
    let skills_dir = tmp.join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let state_path = tmp.join("state.json");
    let usage_path = tmp.join("usage.json");

    {
        let seed = Arc::new(SkillUsageTracker::new(usage_path.clone()));
        seed.load().unwrap();
        seed.mark_agent_created("old-skill");
        seed.save().unwrap();
    }
    // Age the record past the 30-day default BEFORE anything loads it.
    {
        let mut telemetry = UsageTelemetry::load(&usage_path).unwrap();
        telemetry.get_record_mut("old-skill").unwrap().last_used =
            chrono::Utc::now() - chrono::Duration::days(60);
        telemetry.save(&usage_path).unwrap();
    }
    // The tracker the engine reads is loaded AFTER the aging.
    let tracker = Arc::new(SkillUsageTracker::new(usage_path));
    tracker.load().unwrap();
    let skill_dir = skills_dir.join("old-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "# Old Skill").unwrap();

    // Half 1: the backup cannot be created — `.backups` is a FILE, so
    // create_dir_all fails deterministically.
    fs::write(skills_dir.join(".backups"), "not a directory").unwrap();
    let engine = CuratorEngine::new(skills_dir.clone(), state_path.clone(), tracker.clone());
    engine.load_state().await.unwrap();
    let report = engine.run_review(false, None, false, None).await.unwrap();

    assert!(
        report.errors.iter().any(|e| e.starts_with("Backup failed")),
        "the failed backup must be reported: {:?}",
        report.errors
    );
    assert!(
        report.skills_archived.is_empty(),
        "no archival may run without its backup"
    );
    assert!(
        skill_dir.join("SKILL.md").exists(),
        "the skill must still be on disk — the backup precedes the move"
    );

    // Half 2: with a working backup dir the same review archives the skill AND
    // leaves a pre-archive backup that contains it.
    fs::remove_file(skills_dir.join(".backups")).unwrap();
    // The engine's own tracker was loaded after the first aging and its
    // in-memory copy is what run_review reads, so it still holds the aged
    // record — no re-aging is needed for the second run.
    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker);
    engine.load_state().await.unwrap();
    let report = engine.run_review(false, None, false, None).await.unwrap();

    assert_eq!(report.skills_archived, vec!["old-skill".to_string()]);
    let backups = backup::list_backups(&skills_dir.join(".backups")).unwrap();
    assert_eq!(backups.len(), 1, "exactly one pre-archive backup");
    // The snapshot is the pre-archive tree: the skill is inside it.
    let extract_dir = tmp.join("extract");
    fs::create_dir_all(&extract_dir).unwrap();
    let tar_gz = fs::File::open(&backups[0]).unwrap();
    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(&extract_dir).unwrap();
    assert!(
        extract_dir.join("old-skill").join("SKILL.md").exists(),
        "the backup must hold the pre-archive tree — the skill inside it"
    );

    cleanup(&tmp);
}
