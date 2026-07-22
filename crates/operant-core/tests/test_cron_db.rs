//! Integration tests for [`CronDb::referenced_skill_names`] and
//! [`CronDb::rewrite_skill_refs`].

use operant_core::cronjobs::{CreateJobParams, CronDb};
use std::collections::HashMap;
use tempfile::TempDir;

/// Create a temp `CronDb` backed by a unique file in a temp directory.
/// `TempDir` is dropped (and cleaned up) when the returned guard goes out of scope.
fn temp_db() -> (CronDb, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir failed");
    let path = dir.path().join("cron.db");
    let db = CronDb::init(path).expect("CronDb::init failed");
    (db, dir)
}

fn make_job(name: &str, skill: Option<&str>, skills: Option<Vec<&str>>) -> CreateJobParams {
    CreateJobParams {
        name: name.to_string(),
        prompt: "test prompt".to_string(),
        schedule: "0 */2 * * *".to_string(),
        schedule_display: "every 2h".to_string(),
        repeat_times: None,
        deliver: "local".to_string(),
        origin_platform: None,
        origin_chat_id: None,
        origin_thread_id: None,
        skill: skill.map(|s| s.to_string()),
        skills: skills.map(|s| s.into_iter().map(String::from).collect()),
        model: None,
        provider: None,
        base_url: None,
        script: None,
        context_from: None,
        enabled_toolsets: None,
        workdir: None,
        no_agent: false,
    }
}

// ── referenced_skill_names ───────────────────────────────────────────────

#[test]
fn referenced_skill_names_empty_db() {
    let (db, _dir) = temp_db();
    let refs = db.referenced_skill_names().unwrap();
    assert!(refs.is_empty());
}

#[test]
fn referenced_skill_names_collects_both_fields() {
    let (db, _dir) = temp_db();

    // Job with legacy single skill field
    db.create_job(make_job("j1", Some("seo-audit"), None)).unwrap();
    // Job with skills array
    db.create_job(make_job("j2", None, Some(vec!["code-review", "seo-audit"]))).unwrap();
    // Job with both — "seo-audit" appears in both, should dedup
    db.create_job(make_job("j3", Some("seo-audit"), Some(vec!["code-review"]))).unwrap();

    let refs = db.referenced_skill_names().unwrap();
    assert_eq!(refs.len(), 2);
    assert!(refs.contains("seo-audit"));
    assert!(refs.contains("code-review"));
}

#[test]
fn referenced_skill_names_skips_empty_whitespace() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", Some("  "), None)).unwrap();
    db.create_job(make_job("j2", None, Some(vec!["", "  "]))).unwrap();

    let refs = db.referenced_skill_names().unwrap();
    assert!(refs.is_empty());
}

#[test]
fn referenced_skill_names_includes_disabled_jobs() {
    let (db, _dir) = temp_db();
    let id = db.create_job(make_job("j1", Some("old-skill"), None)).unwrap();
    // Disable the job
    let mut updates = HashMap::new();
    updates.insert("enabled".to_string(), Some(serde_json::json!(false)));
    db.update_job(&id, updates).unwrap();

    let refs = db.referenced_skill_names().unwrap();
    assert!(refs.contains("old-skill"), "Disabled job skill should still be returned");
}

// ── rewrite_skill_refs ──────────────────────────────────────────────────

#[test]
fn rewrite_no_jobs_scanned() {
    let (db, _dir) = temp_db();
    let consolidated = HashMap::new();
    let pruned = vec![];
    let report = db.rewrite_skill_refs(&consolidated, &pruned).unwrap();
    assert_eq!(report.jobs_scanned, 0);
    assert_eq!(report.jobs_updated, 0);
}

#[test]
fn rewrite_unchanged_job_not_counted() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", None, Some(vec!["keep-me"]))).unwrap();

    let mut consolidated = HashMap::new();
    consolidated.insert("unrelated".to_string(), "umbrella".to_string());
    let pruned = vec!["also-unrelated".to_string()];

    let report = db.rewrite_skill_refs(&consolidated, &pruned).unwrap();
    assert_eq!(report.jobs_scanned, 1);
    assert_eq!(report.jobs_updated, 0);
}

#[test]
fn rewrite_consolidation_mapping() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", None, Some(vec!["seo-audit", "code-review"]))).unwrap();

    let mut consolidated = HashMap::new();
    consolidated.insert("seo-audit".to_string(), "web-quality".to_string());

    let report = db.rewrite_skill_refs(&consolidated, &[]).unwrap();
    assert_eq!(report.jobs_scanned, 1);
    assert_eq!(report.jobs_updated, 1);
    assert_eq!(report.mappings.len(), 1);
    assert_eq!(report.mappings[0].old_skill, "seo-audit");
    assert_eq!(report.mappings[0].new_skill, "web-quality");

    // Verify the job was updated
    let jobs = db.list_jobs(true).unwrap();
    assert_eq!(jobs.len(), 1);
    let skills = jobs[0].skills.as_ref().unwrap();
    assert!(skills.contains(&"web-quality".to_string()));
    assert!(!skills.contains(&"seo-audit".to_string()));
}

#[test]
fn rewrite_pruned_skill_dropped() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", Some("dead-skill"), None)).unwrap();

    let report = db.rewrite_skill_refs(&HashMap::new(), &["dead-skill".to_string()]).unwrap();
    assert_eq!(report.jobs_updated, 1);
    assert_eq!(report.drops.len(), 1);
    assert_eq!(report.drops[0].dropped_skill, "dead-skill");

    let jobs = db.list_jobs(true).unwrap();
    assert!(jobs[0].skills.as_ref().map_or(true, |s| s.is_empty()));
    assert!(jobs[0].skill.is_none(), "skill should be None after all skills pruned");
}

#[test]
fn rewrite_prune_only_no_consolidation() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", None, Some(vec!["skill-a", "skill-b", "skill-c"])))
        .unwrap();

    // Prune one skill, keep two — no consolidation involved
    let report = db
        .rewrite_skill_refs(&HashMap::new(), &["skill-b".to_string()])
        .unwrap();
    assert_eq!(report.jobs_updated, 1);
    assert_eq!(report.drops.len(), 1);
    assert_eq!(report.mappings.len(), 0);

    let jobs = db.list_jobs(true).unwrap();
    let skills = jobs[0].skills.as_ref().unwrap();
    assert_eq!(skills.len(), 2);
    assert!(skills.contains(&"skill-a".to_string()));
    assert!(skills.contains(&"skill-c".to_string()));
    assert!(!skills.contains(&"skill-b".to_string()));
    // skill field should be set to first remaining skill
    assert_eq!(jobs[0].skill.as_deref(), Some("skill-a"));
}

#[test]
fn rewrite_prune_all_from_multi_skill_job() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", None, Some(vec!["a", "b", "c"])))
        .unwrap();

    let report = db
        .rewrite_skill_refs(
            &HashMap::new(),
            &["a".to_string(), "b".to_string(), "c".to_string()],
        )
        .unwrap();
    assert_eq!(report.jobs_updated, 1);
    assert_eq!(report.drops.len(), 3);

    let jobs = db.list_jobs(true).unwrap();
    assert!(jobs[0].skills.as_ref().map_or(true, |s| s.is_empty()));
    assert!(jobs[0].skill.is_none(), "skill should be None after all pruned");
}

#[test]
fn rewrite_dedup_umbrella_already_present() {
    let (db, _dir) = temp_db();
    // Job has both "seo-audit" and the umbrella "web-quality"
    db.create_job(make_job("j1", None, Some(vec!["seo-audit", "web-quality"]))).unwrap();

    let mut consolidated = HashMap::new();
    consolidated.insert("seo-audit".to_string(), "web-quality".to_string());

    let report = db.rewrite_skill_refs(&consolidated, &[]).unwrap();
    assert_eq!(report.jobs_updated, 1);
    // Should have exactly one mapping (no duplicate umbrella added)
    assert_eq!(report.mappings.len(), 1);

    let jobs = db.list_jobs(true).unwrap();
    let skills = jobs[0].skills.as_ref().unwrap();
    assert_eq!(skills.len(), 1, "Should have only web-quality, not duplicate");
    assert_eq!(skills[0], "web-quality");
}

#[test]
fn rewrite_mixed_consolidation_and_prune() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", None, Some(vec!["seo-audit", "dead-skill", "code-review"]))).unwrap();

    let mut consolidated = HashMap::new();
    consolidated.insert("seo-audit".to_string(), "web-quality".to_string());
    let pruned = vec!["dead-skill".to_string()];

    let report = db.rewrite_skill_refs(&consolidated, &pruned).unwrap();
    assert_eq!(report.jobs_updated, 1);
    assert_eq!(report.mappings.len(), 1);
    assert_eq!(report.drops.len(), 1);

    let jobs = db.list_jobs(true).unwrap();
    let skills = jobs[0].skills.as_ref().unwrap();
    assert!(skills.contains(&"web-quality".to_string()));
    assert!(skills.contains(&"code-review".to_string()));
    assert!(!skills.contains(&"seo-audit".to_string()));
    assert!(!skills.contains(&"dead-skill".to_string()));
}

#[test]
fn rewrite_multiple_jobs() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", Some("seo-audit"), None)).unwrap();
    db.create_job(make_job("j2", None, Some(vec!["code-review"]))).unwrap();
    db.create_job(make_job("j3", None, Some(vec!["seo-audit", "code-review"]))).unwrap();

    let mut consolidated = HashMap::new();
    consolidated.insert("seo-audit".to_string(), "web-quality".to_string());
    consolidated.insert("code-review".to_string(), "web-quality".to_string());

    let report = db.rewrite_skill_refs(&consolidated, &[]).unwrap();
    assert_eq!(report.jobs_scanned, 3);
    assert_eq!(report.jobs_updated, 3);

    let jobs = db.list_jobs(true).unwrap();
    for job in &jobs {
        let skills = job.skills.as_ref().unwrap();
        assert!(
            skills.len() <= 1 && skills.first().map_or(true, |s| s == "web-quality"),
            "Expected only web-quality, got {:?}",
            skills
        );
    }
}

#[test]
fn rewrite_legacy_skill_field() {
    let (db, _dir) = temp_db();
    // Job with only the legacy single skill field
    db.create_job(make_job("j1", Some("seo-audit"), None)).unwrap();

    let mut consolidated = HashMap::new();
    consolidated.insert("seo-audit".to_string(), "web-quality".to_string());

    let report = db.rewrite_skill_refs(&consolidated, &[]).unwrap();
    assert_eq!(report.jobs_updated, 1);
    assert_eq!(report.mappings.len(), 1);

    let jobs = db.list_jobs(true).unwrap();
    // The primary skill field should be updated to the umbrella
    assert_eq!(jobs[0].skill.as_deref(), Some("web-quality"));
    let skills = jobs[0].skills.as_ref().unwrap();
    assert!(skills.contains(&"web-quality".to_string()));
}

#[test]
fn rewrite_prune_nonexistent_skill_is_noop() {
    let (db, _dir) = temp_db();
    db.create_job(make_job("j1", None, Some(vec!["a", "b", "c"])))
        .unwrap();

    // Prune a skill that doesn't exist in any job — should be a no-op
    let report = db
        .rewrite_skill_refs(&HashMap::new(), &["nonexistent-d".to_string()])
        .unwrap();
    assert_eq!(report.jobs_scanned, 1);
    assert_eq!(report.jobs_updated, 0, "should not update when prune target doesn't exist");
    assert!(report.drops.is_empty());

    // Verify original skills are unchanged
    let jobs = db.list_jobs(true).unwrap();
    let skills = jobs[0].skills.as_ref().unwrap();
    assert_eq!(skills, &vec!["a".to_string(), "b".to_string(), "c".to_string()]);
}
